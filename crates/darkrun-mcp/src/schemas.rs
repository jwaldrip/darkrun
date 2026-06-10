//! Published JSON schemas + settings validation.
//!
//! `plugin/schemas/` carries the project-facing schemas: the
//! `.darkrun/settings.yml` contract (`settings.schema.json`) and the
//! per-provider config schemas (`providers/<name>.schema.json`) the settings
//! schema `$ref`s into — Jira/Linear/GitHub-Issues ticketing config,
//! Figma/Pencil/Penpot design config, Notion/Confluence/Google-Docs sources.
//! Editors and the setup skill consume them as files; the engine embeds the
//! same corpus and validates a settings document in-process (the
//! predecessor's AJV path, on the pure-Rust `jsonschema` crate).

use rust_embed::RustEmbed;

/// The embedded `plugin/schemas/` corpus.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../plugin/schemas"]
struct SchemaCorpus;

/// The base URI provider `$ref`s resolve against (the settings schema's `$id`
/// parent), mirroring how the schemas are published.
const SCHEMA_BASE: &str = "https://darkrun.ai/schemas/";

/// Validate a `.darkrun/settings.yml` document (raw YAML) against the
/// published settings schema, resolving provider `$ref`s from the embedded
/// corpus. Returns the list of problems — empty means valid. A missing or
/// malformed schema corpus degrades to "no problems" (validation is a guard,
/// never a wall); a YAML parse failure is itself the problem.
pub fn validate_settings_yaml(raw: &str) -> Vec<String> {
    let doc: serde_json::Value = match serde_yaml::from_str(raw) {
        Ok(v) => v,
        Err(e) => return vec![format!("settings.yml is not valid YAML: {e}")],
    };
    // An empty file parses to null — nothing to validate.
    if doc.is_null() {
        return Vec::new();
    }
    let Some(schema_file) = SchemaCorpus::get("settings.schema.json") else {
        return Vec::new();
    };
    let Ok(schema): Result<serde_json::Value, _> = serde_json::from_slice(schema_file.data.as_ref())
    else {
        return Vec::new();
    };
    // Register every provider schema under the URI the settings schema's
    // relative `$ref`s resolve to (`providers/<name>.schema.json` against the
    // base). Their inner `$id`s are dropped by registering under the resolved
    // URI directly.
    let mut builder = jsonschema::Registry::new();
    for name in SchemaCorpus::iter() {
        let Some(rest) = name.strip_prefix("providers/") else {
            continue;
        };
        let Some(file) = SchemaCorpus::get(&name) else {
            continue;
        };
        let Ok(mut sub): Result<serde_json::Value, _> = serde_json::from_slice(file.data.as_ref())
        else {
            continue;
        };
        if let Some(obj) = sub.as_object_mut() {
            obj.remove("$id");
        }
        let uri = format!("{SCHEMA_BASE}providers/{rest}");
        builder = match builder.add(uri, sub) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
    }
    let registry = match builder.prepare() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let validator = match jsonschema::options().with_registry(&registry).build(&schema) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    validator
        .iter_errors(&doc)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                e.to_string()
            } else {
                format!("{path}: {e}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_settings_doc_validates() {
        let raw = "vcs: git\nhosting: github\nci: github-actions\ndefault_branch: main\n";
        assert!(validate_settings_yaml(raw).is_empty());
    }

    #[test]
    fn unknown_top_level_keys_are_problems() {
        let raw = "hosting: github\njira: {}\n";
        let problems = validate_settings_yaml(raw);
        assert!(!problems.is_empty(), "top-level provider keys are the documented anti-pattern");
        assert!(problems.iter().any(|p| p.contains("jira")), "{problems:?}");
    }

    #[test]
    fn a_bad_hosting_value_is_a_problem() {
        let problems = validate_settings_yaml("hosting: bitbucket\n");
        assert!(problems.iter().any(|p| p.contains("hosting")), "{problems:?}");
    }

    #[test]
    fn provider_config_validates_through_the_ref() {
        // A ticketing provider with a config block the linear schema accepts.
        let ok = "providers:\n  ticketing:\n    type: linear\n    config:\n      project_key: DR\n";
        assert!(validate_settings_yaml(ok).is_empty(), "{:?}", validate_settings_yaml(ok));
        // An invalid enum inside the provider config is caught through the $ref.
        let bad = "providers:\n  ticketing:\n    type: linear\n    config:\n      story_points: sometimes\n";
        let problems = validate_settings_yaml(bad);
        assert!(
            problems.iter().any(|p| p.contains("story_points")),
            "{problems:?}"
        );
    }

    #[test]
    fn malformed_yaml_is_itself_the_problem() {
        let problems = validate_settings_yaml(":\n  - ::bad");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("not valid YAML"));
    }

    #[test]
    fn empty_settings_are_fine() {
        assert!(validate_settings_yaml("").is_empty());
    }
}

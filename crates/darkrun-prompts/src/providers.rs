//! Provider behavior contracts, spliced into rendered prompts.
//!
//! `plugin/providers/<kind>.md` documents are **behavior contracts** for
//! external-system integrations (git, ticketing, spec, knowledge, design).
//! Each declares, in frontmatter, which prompt phases it `splices_into`; when
//! a provider is ACTIVE for the project, its contract body is appended to
//! every rendered prompt whose template key matches a splice point — so the
//! agent carries the integration's rules exactly where they apply, and
//! nowhere else.
//!
//! A provider is active when:
//! - its frontmatter declares `always_on: true` (git — active in any git
//!   repo), or
//! - it appears under `providers.<kind>:` in `.darkrun/settings.yml`.
//!
//! Contracts cascade like prompts: `<repo_root>/.darkrun/providers/<kind>.md`
//! overrides the embedded default.

use std::path::Path;

use rust_embed::RustEmbed;

/// The embedded `plugin/providers/` corpus.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../plugin/providers"]
struct ProviderCorpus;

/// The provider kinds the engine knows about.
const KINDS: &[&str] = &["git", "ticketing", "spec", "knowledge", "design"];

/// One parsed provider doc.
struct ProviderDoc {
    always_on: bool,
    splices_into: Vec<String>,
    body: String,
}

/// Resolve a provider doc's raw source: project override beats embedded.
fn doc_source(repo_root: &Path, kind: &str) -> Option<String> {
    let override_path = repo_root
        .join(".darkrun")
        .join("providers")
        .join(format!("{kind}.md"));
    if let Ok(s) = std::fs::read_to_string(&override_path) {
        return Some(s);
    }
    ProviderCorpus::get(&format!("{kind}.md"))
        .map(|f| String::from_utf8_lossy(f.data.as_ref()).into_owned())
}

/// Parse the tiny provider frontmatter (always_on, splices_into) + body.
/// Line-wise on purpose — the shape is fixed and this crate carries no YAML dep.
fn parse_doc(source: &str) -> Option<ProviderDoc> {
    let rest = source.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (fm, body) = (&rest[..end], &rest[end + 4..]);
    let mut always_on = false;
    let mut splices: Vec<String> = Vec::new();
    let mut in_splices = false;
    for line in fm.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("always_on:") {
            always_on = v.trim() == "true";
            in_splices = false;
        } else if t.starts_with("splices_into:") {
            in_splices = true;
        } else if in_splices && t.starts_with('-') {
            splices.push(t.trim_start_matches('-').trim().to_string());
        } else if !t.starts_with('-') {
            in_splices = false;
        }
    }
    Some(ProviderDoc {
        always_on,
        splices_into: splices,
        body: body.trim().to_string(),
    })
}

/// Whether `.darkrun/settings.yml` configures `providers.<kind>:`. Line-wise:
/// a `providers:` section followed by an indented `<kind>:` entry.
fn configured_in_settings(repo_root: &Path, kind: &str) -> bool {
    let path = repo_root.join(".darkrun").join("settings.yml");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut in_providers = false;
    for line in raw.lines() {
        if line.trim_end() == "providers:" && !line.starts_with(' ') {
            in_providers = true;
            continue;
        }
        if in_providers {
            // Section ends at the next non-indented line.
            if !line.starts_with(' ') && !line.trim().is_empty() {
                in_providers = false;
                continue;
            }
            let t = line.trim();
            if t.strip_prefix(kind)
                .map(|r| r.starts_with(':'))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// The splice key for a template `rel` — its last path segment
/// (`phases/spec` → `spec`, `run/pending_seal` → `pending_seal`).
fn splice_key(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Build the provider-contract block for the prompt at `rel`, or `None` when
/// no active provider splices into it. The block is appended to the rendered
/// prompt by [`crate::render`].
pub(crate) fn provider_block(repo_root: &Path, rel: &str) -> Option<String> {
    let key = splice_key(rel);
    let mut sections: Vec<String> = Vec::new();
    for kind in KINDS {
        let Some(source) = doc_source(repo_root, kind) else {
            continue;
        };
        let Some(doc) = parse_doc(&source) else {
            continue;
        };
        if !doc.splices_into.iter().any(|s| s == key) {
            continue;
        }
        let active = if doc.always_on {
            // git's always-on contract only applies in an actual git repo.
            *kind != "git" || repo_root.join(".git").exists()
        } else {
            configured_in_settings(repo_root, kind)
        };
        if active {
            sections.push(doc.body);
        }
    }
    if sections.is_empty() {
        return None;
    }
    Some(format!(
        "\n\n---\n\n# Provider contracts in effect\n\n\
         The project configures external-system providers whose behavior \
         contracts apply to this phase. Follow them alongside the \
         instructions above.\n\n{}",
        sections.join("\n\n---\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn git_contract_splices_into_spec_in_a_git_repo_only() {
        let dir = tempdir().unwrap();
        // Not a git repo → no block.
        assert!(provider_block(dir.path(), "phases/spec").is_none());
        // A git repo → the always-on git contract applies.
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let block = provider_block(dir.path(), "phases/spec").expect("git contract");
        assert!(block.contains("Git Provider"), "{block}");
        // ...but only at its declared splice points.
        assert!(provider_block(dir.path(), "phases/review").is_none());
    }

    #[test]
    fn configured_provider_splices_and_unconfigured_does_not() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".darkrun")).unwrap();
        // No settings → ticketing inactive.
        assert!(provider_block(dir.path(), "phases/checkpoint").is_none());
        std::fs::write(
            dir.path().join(".darkrun/settings.yml"),
            "hosting: github\nproviders:\n  ticketing:\n    type: linear\n",
        )
        .unwrap();
        let block = provider_block(dir.path(), "phases/checkpoint").expect("ticketing");
        assert!(block.contains("Ticketing Provider"), "{block}");
        // The spec phase gets ticketing too (it splices into spec).
        assert!(provider_block(dir.path(), "phases/spec")
            .unwrap()
            .contains("Ticketing Provider"));
    }

    #[test]
    fn project_override_beats_the_embedded_contract() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join(".darkrun/providers")).unwrap();
        std::fs::write(
            dir.path().join(".darkrun/providers/git.md"),
            "---\nprovider_kind: git\nalways_on: true\nsplices_into:\n  - spec\n---\n\nPROJECT GIT RULES\n",
        )
        .unwrap();
        let block = provider_block(dir.path(), "phases/spec").unwrap();
        assert!(block.contains("PROJECT GIT RULES"), "{block}");
        assert!(!block.contains("Branch architecture"), "override replaces");
    }

    #[test]
    fn settings_section_parsing_respects_indentation() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".darkrun")).unwrap();
        // `design:` OUTSIDE the providers section must not activate design.
        std::fs::write(
            dir.path().join(".darkrun/settings.yml"),
            "providers:\n  spec:\n    type: notion\ndesign: figma\n",
        )
        .unwrap();
        assert!(configured_in_settings(dir.path(), "spec"));
        assert!(!configured_in_settings(dir.path(), "design"));
    }
}

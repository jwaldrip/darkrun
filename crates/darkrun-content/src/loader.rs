//! The content loader: reads the embedded corpus into the [`Factory`] model.
//!
//! The `content/` tree is embedded into the binary at compile time via
//! [`rust-embed`], so the single `darkrun` binary ships its factory corpus with
//! no external files. The loader walks the embedded tree, parses each
//! frontmatter document with `darkrun-core`'s parser, and assembles the typed
//! [`Factory`] model.

use rust_embed::RustEmbed;

use darkrun_core::frontmatter;

use crate::error::{ContentError, Result};
use crate::model::{
    Factory, FactoryFrontmatter, Role, RoleFrontmatter, Station, StationFrontmatter,
};

/// The embedded `content/` tree (workspace-root relative).
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../content"]
struct Content;

/// Read an embedded file as UTF-8 text.
fn read_text(path: &str) -> Result<String> {
    let file = Content::get(path).ok_or_else(|| ContentError::FileNotFound(path.to_string()))?;
    let text = String::from_utf8(file.data.into_owned())
        .map_err(|_| ContentError::FileNotFound(format!("{path} (not valid utf-8)")))?;
    Ok(text)
}

/// List the slugs of every factory embedded in the corpus.
///
/// A factory is any directory under `factories/` that contains a `FACTORY.md`.
pub fn list_factories() -> Vec<String> {
    let mut names: Vec<String> = Content::iter()
        .filter_map(|path| {
            let path = path.as_ref();
            let rest = path.strip_prefix("factories/")?;
            let (slug, tail) = rest.split_once('/')?;
            (tail == "FACTORY.md").then(|| slug.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Load and assemble a factory by slug.
///
/// Parses `FACTORY.md`, then each station's `STATION.md`, then every referenced
/// explorer/worker/reviewer role file, into the typed [`Factory`] model. The
/// returned factory is *not* yet validated — call [`crate::validate::validate`]
/// (or use [`load_validated`]) to enforce structural rules.
pub fn load_factory(name: &str) -> Result<Factory> {
    let factory_md = format!("factories/{name}/FACTORY.md");
    if Content::get(&factory_md).is_none() {
        return Err(ContentError::FactoryNotFound(name.to_string()));
    }

    let (frontmatter, body): (FactoryFrontmatter, String) =
        frontmatter::parse(&read_text(&factory_md)?)?;

    let mut stations = Vec::with_capacity(frontmatter.stations.len());
    for station_slug in &frontmatter.stations {
        stations.push(load_station(name, station_slug)?);
    }

    // Run-level (factory-scope) roles live beside the stations: whole-Run
    // reviewers under `reviewers/`, reflection dimensions under `reflections/`.
    let run_reviewers = load_factory_roles(name, "reviewers", &frontmatter.reviewers)?;
    let reflections = load_factory_roles(name, "reflections", &frontmatter.reflections)?;

    Ok(Factory {
        frontmatter,
        body,
        stations,
        run_reviewers,
        reflections,
    })
}

/// Load a factory and validate it before returning.
pub fn load_validated(name: &str) -> Result<Factory> {
    let factory = load_factory(name)?;
    crate::validate::validate(&factory)?;
    Ok(factory)
}

fn load_station(factory: &str, station: &str) -> Result<Station> {
    let base = format!("factories/{factory}/stations/{station}");
    let (frontmatter, body): (StationFrontmatter, String) =
        frontmatter::parse(&read_text(&format!("{base}/STATION.md"))?)?;

    let explorers = load_roles(&base, "explorers", &frontmatter.explorers)?;
    let workers = load_roles(&base, "workers", &frontmatter.workers)?;
    let reviewers = load_roles(&base, "reviewers", &frontmatter.reviewers)?;

    Ok(Station {
        frontmatter,
        body,
        explorers,
        workers,
        reviewers,
    })
}

/// Load each named role from a station subdirectory, preserving order.
fn load_roles(station_base: &str, subdir: &str, names: &[String]) -> Result<Vec<Role>> {
    let mut roles = Vec::with_capacity(names.len());
    for slug in names {
        let path = format!("{station_base}/{subdir}/{slug}.md");
        let (frontmatter, body): (RoleFrontmatter, String) =
            frontmatter::parse(&read_text(&path)?)?;
        roles.push(Role { frontmatter, body });
    }
    Ok(roles)
}

/// Load each named run-level role from a factory-scope subdirectory
/// (`factories/<factory>/<subdir>/<slug>.md`), preserving declaration order.
///
/// These are the factory-scope analog of [`load_roles`]: whole-Run reviewers
/// and reflection dimensions live beside the stations, not inside one.
fn load_factory_roles(factory: &str, subdir: &str, names: &[String]) -> Result<Vec<Role>> {
    let mut roles = Vec::with_capacity(names.len());
    for slug in names {
        let path = format!("factories/{factory}/{subdir}/{slug}.md");
        let (frontmatter, body): (RoleFrontmatter, String) =
            frontmatter::parse(&read_text(&path)?)?;
        roles.push(Role { frontmatter, body });
    }
    Ok(roles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StationFrontmatter;

    #[test]
    fn list_factories_is_sorted_and_deduped() {
        let names = list_factories();
        assert!(names.contains(&"software".to_string()));
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "list_factories must be sorted");
        sorted.dedup();
        assert_eq!(names.len(), sorted.len(), "list_factories must be deduped");
    }

    #[test]
    fn missing_factory_is_factory_not_found() {
        match load_factory("does-not-exist") {
            Err(ContentError::FactoryNotFound(name)) => assert_eq!(name, "does-not-exist"),
            other => panic!("expected FactoryNotFound, got {other:?}"),
        }
    }

    #[test]
    fn read_text_reports_missing_files() {
        match read_text("factories/software/stations/ghost/STATION.md") {
            Err(ContentError::FileNotFound(p)) => assert!(p.contains("ghost")),
            other => panic!("expected FileNotFound, got {other:?}"),
        }
    }

    #[test]
    fn known_checkpoint_kinds_parse() {
        for kind in ["auto", "ask", "external", "await"] {
            let doc = format!(
                "---\nname: t\nworkers: [a]\ncheckpoint: {kind}\n---\nbody"
            );
            let parsed: Result<(StationFrontmatter, String)> =
                frontmatter::parse(&doc).map_err(Into::into);
            assert!(parsed.is_ok(), "checkpoint `{kind}` should parse");
        }
    }

    #[test]
    fn unknown_checkpoint_kind_is_a_parse_error() {
        // The loader parses each station's `checkpoint:` through the
        // CheckpointKind enum; an unrecognized gate must fail the load rather
        // than silently default.
        let doc = "---\nname: t\nworkers: [a]\ncheckpoint: maybe\n---\nbody";
        let parsed: Result<(StationFrontmatter, String)> =
            frontmatter::parse(doc).map_err(Into::into);
        assert!(
            parsed.is_err(),
            "an unknown checkpoint kind must be rejected at parse time"
        );
    }

    #[test]
    fn missing_required_checkpoint_field_is_a_parse_error() {
        // `checkpoint` is required; a station without it must not load.
        let doc = "---\nname: t\nworkers: [a]\n---\nbody";
        let parsed: Result<(StationFrontmatter, String)> =
            frontmatter::parse(doc).map_err(Into::into);
        assert!(parsed.is_err(), "missing checkpoint must be rejected");
    }

    #[test]
    fn software_loads_run_level_roles_from_the_corpus() {
        // The loader populates run_reviewers / reflections from the factory-scope
        // `reviewers/` and `reflections/` directories beside the stations.
        let f = load_factory("software").expect("load");
        let reviewers: Vec<&str> = f.run_reviewers.iter().map(Role::name).collect();
        let reflections: Vec<&str> = f.reflections.iter().map(Role::name).collect();
        assert_eq!(
            reviewers,
            vec!["integration-auditor", "regression-auditor", "security-auditor"]
        );
        assert_eq!(reflections, vec!["architecture", "process", "quality", "velocity"]);
    }

    #[test]
    fn run_level_role_bodies_are_loaded_verbatim() {
        // Bodies must come through with their instructions intact, not stubbed.
        let f = load_factory("software").expect("load");
        for r in f.run_reviewers.iter().chain(&f.reflections) {
            assert!(r.body.contains('#'), "{} body lost its heading", r.name());
            assert!(r.body.trim().len() > 120, "{} body too thin", r.name());
        }
    }

    #[test]
    fn load_factory_roles_reports_a_missing_factory_scope_file() {
        // A dangling factory-scope reference resolves to a FileNotFound naming
        // the missing path under the factory directory (not a station).
        match load_factory_roles("software", "reflections", &["ghost".to_string()]) {
            Err(ContentError::FileNotFound(p)) => {
                assert!(p.contains("factories/software/reflections/ghost.md"), "{p}");
            }
            other => panic!("expected FileNotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_factory_roles_empty_list_loads_nothing() {
        let roles = load_factory_roles("software", "reviewers", &[]).expect("empty list is ok");
        assert!(roles.is_empty());
    }
}

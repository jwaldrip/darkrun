//! The content loader: resolves a factory from disk into the [`Factory`] model.
//!
//! Resolution is a **cascade**, most-specific-wins, with no code fallback:
//!
//! 1. a project override layer — `<repo_root>/.darkrun/factories/<f>/…` — when a
//!    `repo_root` is supplied, beats
//! 2. the embedded `plugin/factories/<f>/…` corpus (compiled in via `rust-embed`).
//!
//! Crossed with the **inherits chain**: a factory that declares `inherits: <p>`
//! makes the parent *walkable* — any station/role it does not define falls
//! through to `<p>` (and transitively up the chain), so the child overrides what
//! it defines and inherits the rest. The six stations are always taken in the
//! fixed [`Position::FLOW`] order — the spine is a hardcoded mechanic.

use std::fs;
use std::path::{Path, PathBuf};

use rust_embed::RustEmbed;

use darkrun_core::domain::Position;
use darkrun_core::frontmatter;

use crate::error::{ContentError, Result};
use crate::model::{
    Factory, FactoryFrontmatter, Role, RoleFrontmatter, RoleKind, Station, StationFrontmatter,
};

/// The embedded `plugin/factories/` tree. Keys are factory-slug-relative, e.g.
/// `software/FACTORY.md`.
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../plugin/factories"]
struct Content;

/// The maximum inherits-chain depth, a cycle/runaway guard.
const MAX_INHERITS_DEPTH: usize = 16;

/// Resolves factory-relative paths through the project→embedded × inherits-chain
/// cascade. `chain` is the ordered factory slugs to try (child first, then each
/// parent up the `inherits` chain).
struct Resolver {
    project_root: Option<PathBuf>,
    chain: Vec<String>,
}

impl Resolver {
    /// Read a factory-relative path (e.g. `stations/frame/STATION.md`), trying
    /// each factory in the chain — project override before embedded — and
    /// returning the first hit, or `None` if no layer defines it.
    fn read(&self, rel: &str) -> Option<String> {
        for factory in &self.chain {
            let key = format!("{factory}/{rel}");
            if let Some(root) = &self.project_root {
                let path = root.join(".darkrun").join("factories").join(&key);
                if let Ok(text) = fs::read_to_string(&path) {
                    return Some(text);
                }
            }
            if let Some(file) = Content::get(&key) {
                if let Ok(text) = String::from_utf8(file.data.into_owned()) {
                    return Some(text);
                }
            }
        }
        None
    }

    fn require(&self, rel: &str) -> Result<String> {
        self.read(rel)
            .ok_or_else(|| ContentError::FileNotFound(format!("{} (in [{}])", rel, self.chain.join(" → "))))
    }
}

/// Whether a factory `<name>/FACTORY.md` exists in any resolution layer.
fn factory_exists(project_root: Option<&Path>, name: &str) -> bool {
    let key = format!("{name}/FACTORY.md");
    if let Some(root) = project_root {
        if root.join(".darkrun").join("factories").join(&key).is_file() {
            return true;
        }
    }
    Content::get(&key).is_some()
}

/// Read `<factory>/FACTORY.md` from a single factory (project before embedded).
fn read_factory_md(project_root: Option<&Path>, name: &str) -> Result<String> {
    let key = format!("{name}/FACTORY.md");
    if let Some(root) = project_root {
        let path = root.join(".darkrun").join("factories").join(&key);
        if let Ok(text) = fs::read_to_string(&path) {
            return Ok(text);
        }
    }
    let file = Content::get(&key).ok_or_else(|| ContentError::FactoryNotFound(name.to_string()))?;
    String::from_utf8(file.data.into_owned())
        .map_err(|_| ContentError::FileNotFound(format!("{key} (not valid utf-8)")))
}

/// Build the inherits chain for `name` — `[name, parent, grandparent, …]` — by
/// walking each factory's `inherits`. Errors on a cycle or an unknown parent.
fn inherits_chain(project_root: Option<&Path>, name: &str) -> Result<Vec<String>> {
    let mut chain = Vec::new();
    let mut current = name.to_string();
    loop {
        if chain.contains(&current) {
            return Err(ContentError::Invalid {
                factory: name.to_string(),
                message: format!("inherits cycle through `{current}`"),
            });
        }
        if chain.len() >= MAX_INHERITS_DEPTH {
            return Err(ContentError::Invalid {
                factory: name.to_string(),
                message: format!("inherits chain exceeds depth {MAX_INHERITS_DEPTH}"),
            });
        }
        if !factory_exists(project_root, &current) {
            return Err(ContentError::FactoryNotFound(current));
        }
        let (fm, _): (FactoryFrontmatter, String) =
            frontmatter::parse(&read_factory_md(project_root, &current)?)?;
        chain.push(current.clone());
        match fm.inherits {
            Some(parent) => current = parent,
            None => break,
        }
    }
    Ok(chain)
}

/// List the slugs of every factory in the corpus (embedded only — the shipped
/// catalog). A factory is any top-level dir containing a `FACTORY.md`.
pub fn list_factories() -> Vec<String> {
    let mut names: Vec<String> = Content::iter()
        .filter_map(|path| {
            let (slug, tail) = path.as_ref().split_once('/')?;
            (tail == "FACTORY.md").then(|| slug.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Load and assemble a factory by slug from the **embedded** corpus only.
pub fn load_factory(name: &str) -> Result<Factory> {
    load_factory_at(None, name)
}

/// Load a factory through the full cascade: a project override layer at
/// `<repo_root>/.darkrun/factories/` (when `repo_root` is `Some`) beats the
/// embedded corpus, crossed with the factory's `inherits` chain.
pub fn load_factory_at(repo_root: Option<&Path>, name: &str) -> Result<Factory> {
    if !factory_exists(repo_root, name) {
        return Err(ContentError::FactoryNotFound(name.to_string()));
    }
    let chain = inherits_chain(repo_root, name)?;
    let resolver = Resolver {
        project_root: repo_root.map(Path::to_path_buf),
        chain,
    };

    // The child's own FACTORY.md drives the manifest, but each field falls
    // through the inherits chain when the child leaves it empty — so a child
    // factory can specialize one station and inherit the whole run-level roster.
    let (frontmatter, body) = resolve_manifest(repo_root, &resolver.chain)?;

    // The six FSSBPH stations, always, in fixed order — resolved through the
    // chain so a parent can supply a station the child does not.
    let mut stations = Vec::with_capacity(Position::FLOW.len());
    for pos in Position::FLOW {
        stations.push(load_station(&resolver, pos.dir())?);
    }

    let run_reviewers = load_factory_roles(&resolver, "reviewers", &frontmatter.reviewers)?;
    let reflections = load_factory_roles(&resolver, "reflections", &frontmatter.reflections)?;

    Ok(Factory {
        frontmatter,
        body,
        stations,
        run_reviewers,
        reflections,
    })
}

/// Load a factory and validate it before returning (embedded only).
pub fn load_validated(name: &str) -> Result<Factory> {
    load_validated_at(None, name)
}

/// Load through the cascade and validate before returning.
pub fn load_validated_at(repo_root: Option<&Path>, name: &str) -> Result<Factory> {
    let factory = load_factory_at(repo_root, name)?;
    crate::validate::validate(&factory)?;
    Ok(factory)
}

/// Assemble the factory manifest by walking the inherits `chain` (child first).
/// The child's `name`, `inherits`, and `body` are authoritative; every other
/// field falls through to the first ancestor that declares it — so an inheriting
/// factory can leave the run-level roster, model, and category to its parent and
/// only state what it changes. Returns the merged frontmatter and the child body.
fn resolve_manifest(
    repo_root: Option<&Path>,
    chain: &[String],
) -> Result<(FactoryFrontmatter, String)> {
    let mut merged: Option<FactoryFrontmatter> = None;
    let mut child_body = String::new();
    for (i, slug) in chain.iter().enumerate() {
        let (fm, body): (FactoryFrontmatter, String) =
            frontmatter::parse(&read_factory_md(repo_root, slug)?)?;
        if i == 0 {
            child_body = body;
            merged = Some(fm);
            continue;
        }
        // Fill only the fields the child (and nearer ancestors) left empty.
        let m = merged.as_mut().expect("child seeded merged");
        if m.description.is_empty() {
            m.description = fm.description;
        }
        if m.category.is_empty() {
            m.category = fm.category;
        }
        if m.default_model.is_empty() {
            m.default_model = fm.default_model;
        }
        if m.fix_workers.is_empty() {
            m.fix_workers = fm.fix_workers;
        }
        if m.reviewers.is_empty() {
            m.reviewers = fm.reviewers;
        }
        if m.reflections.is_empty() {
            m.reflections = fm.reflections;
        }
        if m.surfaces.is_empty() {
            m.surfaces = fm.surfaces;
        }
    }
    Ok((merged.expect("chain is never empty"), child_body))
}

fn load_station(resolver: &Resolver, station: &str) -> Result<Station> {
    let base = format!("stations/{station}");
    let (frontmatter, body): (StationFrontmatter, String) =
        frontmatter::parse(&resolver.require(&format!("{base}/STATION.md"))?)?;

    let explorers = load_roles(resolver, &base, "explorers", &frontmatter.explorers)?;
    let workers = load_roles(resolver, &base, "workers", &frontmatter.workers)?;
    let reviewers = load_roles(resolver, &base, "reviewers", &frontmatter.reviewers)?;

    Ok(Station {
        frontmatter,
        body,
        explorers,
        workers,
        reviewers,
    })
}

/// Load each named role from a station subdirectory, preserving order. The role
/// kind is inferred from `subdir` (the directory IS the `agent_type`); each role
/// is resolved through the cascade.
fn load_roles(
    resolver: &Resolver,
    station_base: &str,
    subdir: &str,
    names: &[String],
) -> Result<Vec<Role>> {
    let kind = RoleKind::from_dir(subdir)
        .ok_or_else(|| ContentError::FileNotFound(format!("unknown role dir `{subdir}`")))?;
    let mut roles = Vec::with_capacity(names.len());
    for slug in names {
        let rel = format!("{station_base}/{subdir}/{slug}.md");
        let (frontmatter, body): (RoleFrontmatter, String) =
            frontmatter::parse(&resolver.require(&rel)?)?;
        roles.push(Role { frontmatter, body, kind });
    }
    Ok(roles)
}

/// Load each named run-level role from a factory-scope subdirectory
/// (`<subdir>/<slug>.md`), through the cascade, preserving declaration order.
fn load_factory_roles(resolver: &Resolver, subdir: &str, names: &[String]) -> Result<Vec<Role>> {
    let kind = RoleKind::from_dir(subdir)
        .ok_or_else(|| ContentError::FileNotFound(format!("unknown role dir `{subdir}`")))?;
    let mut roles = Vec::with_capacity(names.len());
    for slug in names {
        let rel = format!("{subdir}/{slug}.md");
        let (frontmatter, body): (RoleFrontmatter, String) =
            frontmatter::parse(&resolver.require(&rel)?)?;
        roles.push(Role { frontmatter, body, kind });
    }
    Ok(roles)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn software_walks_the_fixed_flow() {
        let f = load_factory("software").expect("load");
        let names: Vec<&str> = f.stations.iter().map(Station::name).collect();
        assert_eq!(names, vec!["frame", "specify", "shape", "build", "prove", "harden"]);
    }

    #[test]
    fn project_override_beats_embedded() {
        // A project-layer STATION.md overrides the embedded one for the same key.
        let dir = tempfile::tempdir().unwrap();
        let base = dir
            .path()
            .join(".darkrun")
            .join("factories")
            .join("software")
            .join("stations")
            .join("frame");
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("STATION.md"),
            "---\nname: frame\nkills: overridden-risk\nworkers: [framer, challenger, distiller]\nreviewers: [value, feasibility]\nexplorers: [context, value]\ncheckpoint: ask\nlocked_artifact: frame.md\n---\n# Frame (overridden)\n",
        )
        .unwrap();
        let f = load_factory_at(Some(dir.path()), "software").expect("load with override");
        let frame = f.station("frame").unwrap();
        assert_eq!(frame.frontmatter.kills, "overridden-risk");
        // A station NOT overridden still comes from the embedded corpus.
        assert_eq!(f.station("build").unwrap().frontmatter.kills, "implementation-defects");
    }

    #[test]
    fn inherits_makes_the_parent_walkable() {
        // A child factory with only a FACTORY.md (inherits: software) resolves
        // every station/role through the parent.
        let dir = tempfile::tempdir().unwrap();
        let fdir = dir.path().join(".darkrun").join("factories").join("mylib");
        fs::create_dir_all(&fdir).unwrap();
        fs::write(
            fdir.join("FACTORY.md"),
            "---\nname: mylib\ninherits: software\ndefault_model: sonnet\n---\n# mylib\n",
        )
        .unwrap();
        let f = load_factory_at(Some(dir.path()), "mylib").expect("inherits load");
        assert_eq!(f.name(), "mylib");
        // Stations + their rosters come from the parent.
        let names: Vec<&str> = f.stations.iter().map(Station::name).collect();
        assert_eq!(names, vec!["frame", "specify", "shape", "build", "prove", "harden"]);
        assert_eq!(
            f.station("build").unwrap().workers.iter().map(Role::name).collect::<Vec<_>>(),
            vec!["test_author", "builder", "self_reviewer", "reconciler"]
        );
    }

    #[test]
    fn legal_is_a_cross_domain_factory_on_the_same_spine() {
        // The legal factory is orientation-only content — no engine changes — and
        // it walks the identical FSSBPH spine with legal labels and no surface.
        let f = load_validated("legal").expect("legal loads and validates");
        assert_eq!(f.name(), "legal");

        // Same six positions, in the fixed order.
        let names: Vec<&str> = f.stations.iter().map(Station::name).collect();
        assert_eq!(names, vec!["frame", "specify", "shape", "build", "prove", "harden"]);

        // Domain labels ride over the fixed positions.
        let labels: Vec<&str> = f
            .stations
            .iter()
            .map(|s| s.frontmatter.label.as_deref().unwrap_or(s.name()))
            .collect();
        assert_eq!(labels, vec!["Intake", "Position", "Structure", "Draft", "Review", "Execute"]);

        // Proof is human-attested: no measured surface. (Gating is global now —
        // `team` mode opens an external PR at every station, which is how legal's
        // counsel/client attestation is expressed, so there is no per-station
        // checkpoint to assert here.)
        assert!(f.frontmatter.surfaces.is_empty(), "legal declares no software surface");

        // Each station carries a real legal roster (its own role files, not
        // software's) — this is an independent domain corpus.
        let draft_workers: Vec<&str> =
            f.station("build").unwrap().workers.iter().map(Role::name).collect();
        assert_eq!(draft_workers, vec!["clause_drafter", "redline_challenger", "draft_reconciler"]);
    }

    #[test]
    fn a_standalone_factory_missing_its_station_files_errors() {
        let dir = tempfile::tempdir().unwrap();
        // A project factory with a FACTORY.md but none of the six STATION.md
        // files (and not inheriting one that has them) → the first station's
        // require finds no layer with the file.
        let fdir = dir.path().join(".darkrun").join("factories").join("solo");
        fs::create_dir_all(&fdir).unwrap();
        fs::write(fdir.join("FACTORY.md"), "---\nname: solo\n---\n# solo\n").unwrap();
        match load_factory_at(Some(dir.path()), "solo") {
            Err(ContentError::FileNotFound(_)) => {}
            other => panic!("expected a missing-file error, got {other:?}"),
        }
    }

    #[test]
    fn inherits_parent_that_does_not_exist_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let fdir = dir.path().join(".darkrun").join("factories").join("orphan");
        fs::create_dir_all(&fdir).unwrap();
        fs::write(
            fdir.join("FACTORY.md"),
            "---\nname: orphan\ninherits: ghost-parent-xyz\n---\n# orphan\n",
        )
        .unwrap();
        match load_factory_at(Some(dir.path()), "orphan") {
            Err(ContentError::FactoryNotFound(name)) => assert_eq!(name, "ghost-parent-xyz"),
            other => panic!("expected FactoryNotFound for the missing parent, got {other:?}"),
        }
    }

    #[test]
    fn inherits_chain_deeper_than_the_cap_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // f0 -> f1 -> … -> f15 (MAX_INHERITS_DEPTH factories), each inheriting the
        // next. Walking from f0 trips the depth guard before it can resolve f16.
        for i in 0..MAX_INHERITS_DEPTH {
            let name = format!("f{i}");
            let fdir = dir.path().join(".darkrun").join("factories").join(&name);
            fs::create_dir_all(&fdir).unwrap();
            fs::write(
                fdir.join("FACTORY.md"),
                format!("---\nname: {name}\ninherits: f{}\n---\n# {name}\n", i + 1),
            )
            .unwrap();
        }
        match load_factory_at(Some(dir.path()), "f0") {
            Err(ContentError::Invalid { message, .. }) => {
                assert!(message.contains("exceeds depth"), "depth message: {message}")
            }
            other => panic!("expected a depth-cap error, got {other:?}"),
        }
    }

    #[test]
    fn inherits_cycle_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for (a, b) in [("x", "y"), ("y", "x")] {
            let fdir = dir.path().join(".darkrun").join("factories").join(a);
            fs::create_dir_all(&fdir).unwrap();
            fs::write(
                fdir.join("FACTORY.md"),
                format!("---\nname: {a}\ninherits: {b}\n---\n# {a}\n"),
            )
            .unwrap();
        }
        match load_factory_at(Some(dir.path()), "x") {
            Err(ContentError::Invalid { message, .. }) => assert!(message.contains("cycle")),
            other => panic!("expected cycle error, got {other:?}"),
        }
    }
}

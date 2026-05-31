//! Scaffold editable custom artifacts under `.darkrun/factories/` (the
//! `darkrun-scaffold` skill): a Factory, Station, Worker, or Reviewer skeleton
//! the operator fills in. Project-override content the engine reads alongside
//! the embedded corpus.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{McpError, Result};

/// What was scaffolded.
#[derive(Debug, Clone, Serialize)]
pub struct Scaffold {
    /// The artifact kind.
    pub kind: String,
    /// The files written (repo-relative).
    pub written: Vec<String>,
    /// The suggested next step.
    pub next: String,
}

fn write(root: &Path, rel: PathBuf, body: &str, written: &mut Vec<String>) -> Result<()> {
    let path = root.join(&rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(darkrun_core::CoreError::from)?;
    }
    std::fs::write(&path, body).map_err(darkrun_core::CoreError::from)?;
    written.push(rel.to_string_lossy().to_string());
    Ok(())
}

/// Scaffold an artifact. `factory`/`station` are required for the nested kinds.
pub fn scaffold(
    repo_root: &Path,
    kind: &str,
    name: &str,
    factory: Option<&str>,
    station: Option<&str>,
) -> Result<Scaffold> {
    let need = |opt: Option<&str>, what: &str| -> Result<String> {
        opt.filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| McpError::InvalidInput(format!("`{what}` is required to scaffold a {kind}")))
    };
    let base = PathBuf::from(".darkrun").join("factories");
    let mut written = Vec::new();

    let next = match kind.trim().to_ascii_lowercase().as_str() {
        "factory" => {
            let dir = base.join(name);
            write(
                repo_root,
                dir.join("FACTORY.md"),
                &factory_template(name),
                &mut written,
            )?;
            // An empty stations dir to fill in.
            std::fs::create_dir_all(repo_root.join(dir.join("stations")))
                .map_err(darkrun_core::CoreError::from)?;
            format!("Add stations under `.darkrun/factories/{name}/stations/`, then `/darkrun:darkrun-factories` to confirm it registers.")
        }
        "station" => {
            let f = need(factory, "factory")?;
            let dir = base.join(&f).join("stations").join(name);
            write(repo_root, dir.join("STATION.md"), &station_template(name), &mut written)?;
            for sub in ["workers", "reviewers", "explorers"] {
                std::fs::create_dir_all(repo_root.join(dir.join(sub)))
                    .map_err(darkrun_core::CoreError::from)?;
            }
            format!("Add `{name}` to factory `{f}`'s station list (in order), then scaffold its workers/reviewers.")
        }
        "worker" => {
            let f = need(factory, "factory")?;
            let s = need(station, "station")?;
            let rel = base.join(&f).join("stations").join(&s).join("workers").join(format!("{name}.md"));
            write(repo_root, rel, &worker_template(name), &mut written)?;
            format!("Add `{name}` to station `{s}`'s worker list (Workers run Make → Challenge → Resolve).")
        }
        "reviewer" => {
            let f = need(factory, "factory")?;
            let s = need(station, "station")?;
            let rel = base.join(&f).join("stations").join(&s).join("reviewers").join(format!("{name}.md"));
            write(repo_root, rel, &reviewer_template(name), &mut written)?;
            format!("Add `{name}` to station `{s}`'s reviewer list; it runs in the audit phase.")
        }
        other => {
            return Err(McpError::InvalidInput(format!(
                "unknown scaffold kind `{other}` (factory | station | worker | reviewer)"
            )))
        }
    };

    Ok(Scaffold {
        kind: kind.to_string(),
        written,
        next,
    })
}

fn factory_template(name: &str) -> String {
    format!(
        "---\nname: {name}\ndefault_model: sonnet\n---\n\n# {name} factory\n\nWhat this factory produces, and the risk-ordered stations it runs.\n\n## Stations\n\n1. <station> — eliminates <risk class>\n"
    )
}

fn station_template(name: &str) -> String {
    format!(
        "---\nname: {name}\nkills: <the risk class this station eliminates>\nartifact: <the durable artifact it locks>\ncheckpoint: ask\n---\n\n# {name}\n\nWhat this station must achieve, and how its workers and reviewers operate.\n"
    )
}

fn worker_template(name: &str) -> String {
    format!(
        "---\nname: {name}\nmodel: sonnet\n---\n\n# {name}\n\n**Focus** — what this worker is responsible for.\n\n**Produces** — the concrete output it must leave behind.\n\n**Reads** — the inputs it consumes.\n\n**Anti-patterns** — what it must NOT do.\n"
    )
}

fn reviewer_template(name: &str) -> String {
    format!(
        "---\nname: {name}\nmodel: sonnet\n---\n\n# {name}\n\n**Lens** — the single dimension this reviewer judges.\n\n**Checks** — what it verifies against the completion criteria.\n\n**Files** — the findings it emits (and what it MUST NOT flag).\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffolds_a_factory() {
        let dir = tempfile::tempdir().unwrap();
        let s = scaffold(dir.path(), "factory", "sales", None, None).unwrap();
        assert!(s.written.iter().any(|w| w.ends_with("sales/FACTORY.md")));
        assert!(dir.path().join(".darkrun/factories/sales/stations").is_dir());
    }

    #[test]
    fn worker_requires_factory_and_station() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scaffold(dir.path(), "worker", "framer", None, None).is_err());
        assert!(scaffold(dir.path(), "worker", "framer", Some("software"), None).is_err());
        let s = scaffold(dir.path(), "worker", "framer", Some("software"), Some("frame")).unwrap();
        assert!(s.written[0].ends_with("frame/workers/framer.md"));
        let body =
            std::fs::read_to_string(dir.path().join(&s.written[0])).unwrap();
        assert!(body.contains("Anti-patterns"));
    }

    #[test]
    fn unknown_kind_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scaffold(dir.path(), "gizmo", "x", None, None).is_err());
    }
}

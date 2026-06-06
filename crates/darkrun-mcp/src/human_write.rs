//! Operator-delegated file writes (F2).
//!
//! When the operator asks the agent to "save this for me", the agent calls
//! [`human_write`] — a guarded write of a project file on the operator's behalf.
//! It deliberately does **nothing clever** with the run: it just creates the
//! file. If that file is a premise some unit was built on, the next drift sweep
//! witnesses the change and routes a re-orientation feedback automatically — the
//! same human-in-the-loop path any out-of-band edit takes. The write is the only
//! action; drift handling is emergent, not wired here.
//!
//! The guards keep it from being a hole in the engine's write discipline:
//!
//! - the target must resolve **inside the repo root** (no absolute paths, no
//!   `..` escape, no symlinked-parent escape),
//! - it may **not** write engine-managed state under `.darkrun/` (those shapes
//!   have their own typed tools), and
//! - it refuses to follow a symlink at the target itself.

use std::path::{Component, Path, PathBuf};

use crate::error::{McpError, Result};

/// Write `content` to `rel_path` (repo-relative) on the operator's behalf,
/// returning the absolute path written. Guarded per the module docs. Parent
/// directories are created as needed.
pub fn human_write(repo_root: &Path, rel_path: &str, content: &str) -> Result<PathBuf> {
    let rel = Path::new(rel_path.trim());
    if rel.as_os_str().is_empty() {
        return Err(McpError::InvalidInput("write path must not be empty".into()));
    }
    if rel.is_absolute() {
        return Err(McpError::InvalidInput(format!(
            "write path '{rel_path}' must be relative to the project root"
        )));
    }
    // No `..` escape, and normalize away any `.` segments.
    let mut normalized = PathBuf::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => normalized.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(McpError::InvalidInput(format!(
                    "write path '{rel_path}' may not contain '..'"
                )))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(McpError::InvalidInput(format!(
                    "write path '{rel_path}' must be relative to the project root"
                )))
            }
        }
    }

    // Engine-managed state is off-limits — those shapes have typed tools.
    if normalized
        .components()
        .next()
        .map(|c| c.as_os_str() == ".darkrun")
        .unwrap_or(false)
    {
        return Err(McpError::InvalidInput(
            "refusing to write engine state under .darkrun/ — use the darkrun tools for run state".into(),
        ));
    }

    let target = repo_root.join(&normalized);

    // Refuse to follow a symlink at the target itself (don't clobber through a link).
    if target.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(McpError::InvalidInput(format!(
            "refusing to write through the symlink at '{rel_path}'"
        )));
    }

    let parent = target
        .parent()
        .ok_or_else(|| McpError::InvalidInput("write path has no parent directory".into()))?;
    std::fs::create_dir_all(parent).map_err(|e| {
        McpError::InvalidInput(format!("could not create parent directory: {e}"))
    })?;

    // Symlinked-parent escape guard: the real parent must sit inside the real
    // repo root (canonicalize resolves any symlinks in the chain).
    let canon_parent = parent
        .canonicalize()
        .map_err(|e| McpError::InvalidInput(format!("could not resolve parent directory: {e}")))?;
    let canon_root = repo_root
        .canonicalize()
        .map_err(|e| McpError::InvalidInput(format!("could not resolve project root: {e}")))?;
    if !canon_parent.starts_with(&canon_root) {
        return Err(McpError::InvalidInput(format!(
            "write path '{rel_path}' escapes the project root via a symlink"
        )));
    }

    std::fs::write(&target, content)
        .map_err(|e| McpError::InvalidInput(format!("could not write file: {e}")))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tmp")
    }

    #[test]
    fn writes_a_project_file_and_creates_parents() {
        let d = root();
        let p = human_write(d.path(), "knowledge/notes.md", "hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello\n");
        assert!(p.starts_with(d.path()));
    }

    #[test]
    fn rejects_absolute_and_parent_escape() {
        let d = root();
        assert!(human_write(d.path(), "/etc/passwd", "x").is_err());
        assert!(human_write(d.path(), "../outside.txt", "x").is_err());
        assert!(human_write(d.path(), "a/../../b.txt", "x").is_err());
    }

    #[test]
    fn normalizes_curdir_segments_and_writes() {
        let d = root();
        // A `./` segment is skipped by the normalizer (Component::CurDir arm).
        let p = human_write(d.path(), "docs/./notes.md", "ok\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "ok\n");
        assert!(p.ends_with("docs/notes.md"));
    }

    #[test]
    fn surfaces_a_parent_creation_failure() {
        let d = root();
        // A regular file where a parent directory would need to be → create_dir_all
        // fails, exercising the parent-creation error arm.
        std::fs::write(d.path().join("blocker"), "i am a file").unwrap();
        let err = human_write(d.path(), "blocker/child.md", "x").unwrap_err();
        assert!(format!("{err}").contains("could not create parent directory"));
    }

    #[test]
    fn rejects_engine_state_writes() {
        let d = root();
        let err = human_write(d.path(), ".darkrun/r/state.json", "{}").unwrap_err();
        assert!(format!("{err}").contains(".darkrun"));
    }

    #[test]
    fn rejects_symlinked_parent_escape() {
        let d = root();
        let outside = root();
        // A symlinked dir inside the root pointing outside it.
        let link = d.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let err = human_write(d.path(), "escape/evil.txt", "x").unwrap_err();
        assert!(format!("{err}").contains("escapes the project root"));
    }

    /// The whole point of F2: an operator write to a premise some unit was built
    /// on is caught by the next drift sweep as `origin=drift` feedback — no
    /// separate trigger. (Verified through the real sweep, not a stub.)
    #[test]
    fn a_human_write_to_a_premise_triggers_drift() {
        use darkrun_core::domain::{Status, Unit, UnitFrontmatter};
        use darkrun_core::StateStore;
        let d = root();
        let store = StateStore::new(d.path());
        // A completed unit built on a premise file the operator will later edit.
        human_write(d.path(), "design.md", "v1\n").unwrap();
        let unit = Unit {
            slug: "u".into(),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some("build".into()),
                inputs: vec!["design.md".into()],
                ..Default::default()
            },
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).unwrap();
        crate::drift::record_station_witnesses(&store, "r", "build").unwrap();

        // Steady state: no drift.
        crate::drift::sweep(&store, "r").unwrap();
        assert!(crate::feedback::list(&store, "r").unwrap().is_empty());

        // The operator saves a new version via human_write → the next sweep files
        // exactly one drift feedback, no extra wiring.
        human_write(d.path(), "design.md", "v2 — buttons moved\n").unwrap();
        crate::drift::sweep(&store, "r").unwrap();
        let drift_fb: Vec<_> = crate::feedback::list(&store, "r")
            .unwrap()
            .into_iter()
            .filter(|f| matches!(f.origin, darkrun_core::domain::FeedbackOrigin::Drift))
            .collect();
        assert_eq!(drift_fb.len(), 1, "the human write triggered drift automatically");
    }

    #[test]
    fn rejects_empty_path() {
        let d = root();
        assert!(human_write(d.path(), "   ", "x").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_through_a_target_symlink() {
        let d = root();
        // A symlink AT the target path itself must not be clobbered through.
        let link = d.path().join("link.txt");
        std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();
        assert!(human_write(d.path(), "link.txt", "x").is_err());
    }
}

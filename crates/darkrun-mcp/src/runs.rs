//! Run-level helpers: list summaries and archive toggling.
//!
//! These back the `darkrun_run_list` / `darkrun_run_archive` tools. Listing
//! returns a compact summary per run (slug, title, factory, status, active
//! station, archived flag, and the "Mine" authorship predicate) without forcing
//! the caller to read every document.

use std::path::Path;

use darkrun_core::domain::Status;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::{McpError, Result};

/// A compact summary of a run for list views.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RunSummary {
    /// Run slug.
    pub slug: String,
    /// Resolved title.
    pub title: String,
    /// Driving factory.
    pub factory: String,
    /// Lifecycle status.
    pub status: Status,
    /// The active station (write-cache hint).
    pub active_station: String,
    /// Whether this run is archived.
    pub archived: bool,
    /// Whether the current git identity authored any commit on the run's
    /// branch (`darkrun/<slug>`) beyond its base — the engine's "Mine"
    /// predicate. `false` when the project is not a git repo, no identity is
    /// configured, or the branch carries none of the current user's commits.
    pub authored_by_me: bool,
}

/// The run's stable accumulating branch for a slug, used as the authorship head
/// (`darkrun/<slug>/main` — the per-run base every landed station fans into).
fn run_branch(slug: &str) -> String {
    crate::lifecycle::run_main_branch(slug)
}

/// The base branch runs fork from — the shared [`resolve_base_branch`] helper
/// (`default_branch` out of `.darkrun/settings.yml`, defaulting to `main`).
fn base_branch(store: &StateStore) -> String {
    crate::lifecycle::resolve_base_branch(store)
}

/// List every run on disk as a summary, sorted by slug. Archived runs are
/// included unless `include_archived` is false.
///
/// `repo_root` is the git repository root used to compute the per-run "Mine"
/// predicate. The current git identity and base branch are resolved once, so
/// the per-run check is a single revwalk; a non-git project or missing identity
/// degrades cleanly to every run being "not mine".
pub fn list(
    store: &StateStore,
    repo_root: &Path,
    include_archived: bool,
) -> Result<Vec<RunSummary>> {
    let base = base_branch(store);
    let email = darkrun_git::current_identity_email(repo_root)
        .ok()
        .flatten()
        .map(|e| e.to_ascii_lowercase());

    let mut out = Vec::new();
    for slug in store.list_runs()? {
        let run = match store.read_run(&slug) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let archived = run.frontmatter.archived.unwrap_or(false);
        if archived && !include_archived {
            continue;
        }
        let authored_by_me = email.as_deref().is_some_and(|email| {
            darkrun_git::branch_authored_by(repo_root, &base, &run_branch(&run.slug), email)
                .unwrap_or(false)
        });
        out.push(RunSummary {
            slug: run.slug,
            title: run.title,
            factory: run.frontmatter.factory,
            status: run.frontmatter.status,
            active_station: run.frontmatter.active_station,
            archived,
            authored_by_me,
        });
    }
    Ok(out)
}

/// Set (or clear) a run's archived flag. Archiving a run also clears it from
/// the active-run pointer so it stops surfacing as the default.
pub fn set_archived(store: &StateStore, slug: &str, archived: bool) -> Result<()> {
    let mut run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    run.frontmatter.archived = Some(archived);
    store.write_run(&run)?;
    if archived {
        // If this run was the active pointer, drop it.
        if let Ok(Some(active)) = store.active_run() {
            if active == slug {
                store.clear_active_run()?;
            }
        }
    }
    Ok(())
}

/// Set (or clear) a cross-system handle on the run's `external_refs` (G2). The
/// well-known keys are `ticket`, `pr_url` (or `pr`), and `design`; any other
/// key lands in the `other` map. An empty value clears the handle. Returns the
/// updated [`ExternalRefs`].
pub fn set_external_ref(
    store: &StateStore,
    slug: &str,
    key: &str,
    value: &str,
) -> Result<darkrun_core::domain::ExternalRefs> {
    let mut run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    run.frontmatter.external_refs.set(key, value);
    let refs = run.frontmatter.external_refs.clone();
    store.write_run(&run)?;
    Ok(refs)
}

/// Read a run's cross-system handles.
pub fn external_refs(
    store: &StateStore,
    slug: &str,
) -> Result<darkrun_core::domain::ExternalRefs> {
    let run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    Ok(run.frontmatter.external_refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::run_start;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn list_returns_summaries() {
        let (d, store) = store();
        run_start(&store, "a", "software", Some("Alpha".into()), "continuous").unwrap();
        run_start(&store, "b", "software", None, "continuous").unwrap();
        let runs = list(&store, d.path(), true).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].slug, "a");
        assert_eq!(runs[0].title, "Alpha");
        assert_eq!(runs[0].active_station, "frame");
        // A bare tempdir is not a git repo, so nothing is attributable: "Mine"
        // degrades to false rather than erroring.
        assert!(!runs[0].authored_by_me);
    }

    #[test]
    fn archive_hides_run_from_default_list() {
        let (d, store) = store();
        run_start(&store, "a", "software", None, "continuous").unwrap();
        set_archived(&store, "a", true).unwrap();
        assert!(list(&store, d.path(), false).unwrap().is_empty());
        assert_eq!(list(&store, d.path(), true).unwrap().len(), 1);
        assert!(list(&store, d.path(), true).unwrap()[0].archived);
    }

    #[test]
    fn archive_clears_active_pointer() {
        let (_d, store) = store();
        run_start(&store, "a", "software", None, "continuous").unwrap();
        store.set_active_run("a").unwrap();
        set_archived(&store, "a", true).unwrap();
        // Active should no longer resolve to the archived run.
        assert_ne!(store.active_run().unwrap(), Some("a".to_string()));
    }

    #[test]
    fn unarchive_restores() {
        let (d, store) = store();
        run_start(&store, "a", "software", None, "continuous").unwrap();
        set_archived(&store, "a", true).unwrap();
        set_archived(&store, "a", false).unwrap();
        assert_eq!(list(&store, d.path(), false).unwrap().len(), 1);
    }

    #[test]
    fn external_refs_set_read_and_clear() {
        let (_d, store) = store();
        run_start(&store, "a", "software", None, "continuous").unwrap();
        // Empty by default.
        assert!(external_refs(&store, "a").unwrap().is_empty());

        // Well-known keys land in their typed slots; `pr` aliases `pr_url`.
        set_external_ref(&store, "a", "ticket", "JIRA-42").unwrap();
        set_external_ref(&store, "a", "pr", "https://github.com/x/y/pull/7").unwrap();
        set_external_ref(&store, "a", "design", "https://figma.com/abc").unwrap();
        // An unknown key lands in `other`.
        let refs = set_external_ref(&store, "a", "dashboard", "https://grafana/x").unwrap();
        assert_eq!(refs.ticket.as_deref(), Some("JIRA-42"));
        assert_eq!(refs.pr_url.as_deref(), Some("https://github.com/x/y/pull/7"));
        assert_eq!(refs.design.as_deref(), Some("https://figma.com/abc"));
        assert_eq!(refs.other.get("dashboard").map(String::as_str), Some("https://grafana/x"));

        // Survives a read (persisted to the run frontmatter).
        let reread = external_refs(&store, "a").unwrap();
        assert_eq!(reread.ticket.as_deref(), Some("JIRA-42"));

        // An empty value clears a handle.
        set_external_ref(&store, "a", "ticket", "").unwrap();
        assert!(external_refs(&store, "a").unwrap().ticket.is_none());
    }

    #[test]
    fn mine_flag_tracks_branch_authorship() {
        use std::process::Command;

        let dir = tempdir().expect("tmp");
        let root = dir.path();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        // A real repo with the current identity = me@x.io. `.darkrun/` is
        // gitignored so the per-station worktrees the lifecycle forks never
        // clobber the run documents on disk.
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "me@x.io"]);
        git(&["config", "user.name", "Me"]);
        std::fs::write(root.join(".gitignore"), ".darkrun/\n").unwrap();
        std::fs::write(root.join("README.md"), "# x\n").unwrap();
        git(&["add", "README.md", ".gitignore"]);
        git(&["commit", "-q", "-m", "base"]);

        // Register both runs. run_start forks each run's stable branch
        // (darkrun/<slug>/main) off `main` — the authorship head under the
        // hierarchy.
        let store = StateStore::new(root);
        run_start(&store, "mine-run", "software", None, "continuous").unwrap();
        run_start(&store, "their-run", "software", None, "continuous").unwrap();

        // A commit I authored on mine-run's run-main branch.
        git(&["checkout", "-q", "darkrun/mine-run/main"]);
        std::fs::write(root.join("work.txt"), "work\n").unwrap();
        git(&["add", "work.txt"]);
        git(&["commit", "-q", "-m", "work"]);

        // A commit authored by someone else on their-run's run-main branch.
        git(&["checkout", "-q", "darkrun/their-run/main"]);
        git(&["config", "user.email", "other@x.io"]);
        std::fs::write(root.join("theirs.txt"), "theirs\n").unwrap();
        git(&["add", "theirs.txt"]);
        git(&["commit", "-q", "-m", "theirs"]);

        // Back on main with my identity as the "current" one for the list.
        git(&["checkout", "-q", "main"]);
        git(&["config", "user.email", "me@x.io"]);

        let runs = list(&store, root, true).unwrap();
        let by_slug = |slug: &str| runs.iter().find(|r| r.slug == slug).unwrap();
        assert!(by_slug("mine-run").authored_by_me, "I authored mine-run");
        assert!(
            !by_slug("their-run").authored_by_me,
            "their-run is authored by someone else"
        );
    }

    #[test]
    fn archive_missing_run_errors() {
        let (_d, store) = store();
        let err = set_archived(&store, "ghost", true).unwrap_err();
        assert!(matches!(err, McpError::Core(_)));
    }
}

//! darkrun-git — git worktree management for the darkrun factory engine.
//!
//! darkrun executes Stations and Units on isolated git worktrees so the engine
//! never mutates the operator's checked-out branch. This crate provides the
//! local worktree primitives the manager builds on:
//! [`create_worktree`](GitBackend::create_worktree),
//! [`list_worktrees`](GitBackend::list_worktrees),
//! [`remove_worktree`](GitBackend::remove_worktree),
//! [`current_branch`](GitBackend::current_branch), and
//! [`is_clean`](GitBackend::is_clean).
//!
//! Beyond worktree CRUD, darkrun drives a per-run-main / per-station branch
//! hierarchy (`darkrun/<slug>/main` accumulating `darkrun/<slug>/<station>`
//! branches) with staged fan-in merges, guarded by an
//! [`engine_protected_merge`](merge::engine_protected_merge) so a land can never
//! silently revert the engine's `.darkrun/` workflow state. The branch + merge
//! primitives backing that hierarchy are on the [`GitBackend`] trait alongside
//! the worktree ops.
//!
//! Operations go through the [`GitBackend`] trait, implemented by the pure-Rust
//! [`GixBackend`] (gitoxide, in-process — no C dependency, no `git` CLI). [`Git`]
//! is a thin facade over it and is the recommended entry point.

mod authorship;
mod backend;
mod clone;
mod diff;
mod error;
mod gix_backend;
pub mod merge;
pub mod net;
mod push;

use std::path::{Path, PathBuf};

pub use authorship::{
    branch_author, branch_authored_by, current_identity_email, run_authored_by_me,
};
pub use backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
pub use clone::{clone_repo, default_clone_dest, repo_name_from_url};
pub use error::{GitError, Result};
pub use gix_backend::GixBackend;
pub use merge::{engine_protected_merge, is_engine_owned_state_path, ENGINE_STATE_PREFIX};
pub use net::{ensure_noninteractive, network_deadline, with_deadline};

/// Resolve a checkout dir to its PROJECT root: a linked worktree maps to the
/// main repository's working dir (the shared `.git`'s parent); a main checkout
/// maps to itself; a non-git dir passes through. The identity the discovery
/// registry keys projects on — every worktree of a repo is ONE project.
pub fn project_root_of(path: &std::path::Path) -> std::path::PathBuf {
    if let Ok(repo) = gix::open(path) {
        // gix reports the common dir as recorded in the worktree's `commondir`
        // file, which is usually RELATIVE — e.g. `<git_dir>/../..`. Normalize
        // the dot-dots LEXICALLY before comparing or taking a parent:
        // `PathBuf::parent` strips components textually, so on an
        // un-normalized `…/worktrees/<name>/../..` it fabricates a bogus root
        // (`…/worktrees/<name>/..`) — which registered every worktree as its
        // own project instead of grouping it under the repo.
        let git_dir = normalize_dots(repo.git_dir());
        let common = normalize_dots(repo.common_dir().as_ref());
        if git_dir != common {
            if let Some(main) = common.parent() {
                return main.to_path_buf();
            }
        }
        if let Some(wd) = repo.workdir() {
            return wd.to_path_buf();
        }
    }
    path.to_path_buf()
}

/// Lexically resolve `.` / `..` components. No filesystem access and no
/// symlink resolution, so the result stays textually comparable with the raw
/// paths callers hash for project identity.
fn normalize_dots(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
// `has_no_merge_debt` + `is_merge_in_progress` are defined below in this module.

/// The recommended entry point: a [`GitBackend`] facade over a repository.
///
/// `Git` wraps the pure-Rust gitoxide backend ([`GixBackend`]) — no C, no
/// `git` CLI. The conformance suite validates it against the real `git` binary.
pub struct Git {
    inner: Box<dyn GitBackend + Send + Sync>,
    repo_root: PathBuf,
}

impl Git {
    /// Open `repo_root` with the default pure-Rust gitoxide backend.
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        Self::open_gix(repo_root)
    }

    /// Open `repo_root` with the pure-Rust gitoxide backend (in-process, no C,
    /// no `git` CLI) — the sole backend. Implements the full [`GitBackend`]
    /// surface, including the operations gitoxide has no high-level API for
    /// (push/rebase/merge/worktree-create), each built over its plumbing and
    /// conformance-tested against real `git`. Alias of [`Git::open`].
    pub fn open_gix(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref().to_path_buf();
        let inner = GixBackend::open(&root)?;
        Ok(Self {
            inner: Box::new(inner),
            repo_root: root,
        })
    }

    /// The repository root this facade was opened against.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

impl GitBackend for Git {
    fn create_worktree(
        &self,
        name: &str,
        path: &Path,
        opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        self.inner.create_worktree(name, path, opts)
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        self.inner.list_worktrees()
    }

    fn remove_worktree(&self, name: &str, force: bool) -> Result<()> {
        self.inner.remove_worktree(name, force)
    }

    fn current_branch(&self) -> Result<Option<String>> {
        self.inner.current_branch()
    }

    fn is_clean(&self) -> Result<bool> {
        self.inner.is_clean()
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        self.inner.branch_exists(name)
    }

    fn create_branch(&self, name: &str, from_ref: &str) -> Result<()> {
        self.inner.create_branch(name, from_ref)
    }

    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool> {
        self.inner.is_ancestor(maybe_ancestor, descendant)
    }

    fn merge_no_commit(&self, worktree_path: &Path, source_ref: &str) -> Result<MergeOutcome> {
        self.inner.merge_no_commit(worktree_path, source_ref)
    }

    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool> {
        self.inner.merge_in_progress(worktree_path)
    }

    fn checkout_paths(&self, worktree_path: &Path, from_ref: &str, paths: &[String]) -> Result<()> {
        self.inner.checkout_paths(worktree_path, from_ref, paths)
    }

    fn add_paths(&self, worktree_path: &Path, paths: &[String]) -> Result<()> {
        self.inner.add_paths(worktree_path, paths)
    }

    fn add_all_under(&self, worktree_path: &Path, prefix: &str) -> Result<()> {
        self.inner.add_all_under(worktree_path, prefix)
    }

    fn dirty_paths_excluding(
        &self,
        worktree_path: &Path,
        exclude_prefixes: &[&str],
    ) -> Result<Vec<String>> {
        self.inner.dirty_paths_excluding(worktree_path, exclude_prefixes)
    }

    fn changed_paths_between(&self, base_ref: &str, head_ref: &str) -> Result<Vec<String>> {
        self.inner.changed_paths_between(base_ref, head_ref)
    }

    fn status_dirty_under(&self, worktree_path: &Path, prefix: &str) -> Result<bool> {
        self.inner.status_dirty_under(worktree_path, prefix)
    }

    fn checkout_branch(&self, branch: &str) -> Result<()> {
        self.inner.checkout_branch(branch)
    }

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        self.inner.commit(worktree_path, message)
    }

    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>> {
        self.inner.ls_tree(worktree_path, from_ref, prefix)
    }

    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>> {
        self.inner.unresolved_paths(worktree_path)
    }

    fn refs_have_identical_trees(&self, ref_a: &str, ref_b: &str) -> Result<bool> {
        self.inner.refs_have_identical_trees(ref_a, ref_b)
    }

    fn push(&self, worktree_path: &Path, branch: &str) -> Result<()> {
        self.inner.push(worktree_path, branch)
    }

    fn fetch(&self, worktree_path: &Path, branch: &str) -> Result<()> {
        self.inner.fetch(worktree_path, branch)
    }

    fn rebase_onto(&self, worktree_path: &Path, upstream: &str) -> Result<()> {
        self.inner.rebase_onto(worktree_path, upstream)
    }

    fn rebase_abort(&self, worktree_path: &Path) -> Result<()> {
        self.inner.rebase_abort(worktree_path)
    }

    fn create_worktree_detached(
        &self,
        name: &str,
        path: &Path,
        committish: &str,
    ) -> Result<WorktreeInfo> {
        self.inner.create_worktree_detached(name, path, committish)
    }

    fn head_oid(&self, worktree_path: &Path) -> Result<String> {
        self.inner.head_oid(worktree_path)
    }

    fn set_branch_to(&self, name: &str, committish: &str) -> Result<()> {
        self.inner.set_branch_to(name, committish)
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        self.inner.delete_branch(name)
    }

    fn remote_url(&self, name: &str) -> Result<Option<String>> {
        self.inner.remote_url(name)
    }

    fn default_branch(&self) -> Result<Option<String>> {
        self.inner.default_branch()
    }

    fn diff_stat(&self, reference: &str) -> Result<String> {
        self.inner.diff_stat(reference)
    }

    fn diff(&self, reference: &str) -> Result<String> {
        self.inner.diff(reference)
    }
}

/// Whether merging `source` into `target` would discharge no merge debt — i.e.
/// the merge is a pure no-op (mechanic #4, the loop guard).
///
/// True when either:
///
/// - `source` and `target` have **identical trees** (a `--no-ff` merge would
///   mint an empty commit), OR
/// - `source` is already an **ancestor** of `target` ("already up to date" —
///   trees differ only because `target` accreted later commits elsewhere).
///
/// Either case means emitting a merge would re-execute a no-op and re-dispatch
/// the cursor, producing the alternating no-op-merge loop. Callers MUST gate
/// BOTH the cursor's land synthesis AND the in-handler merge on this so the
/// loop can't fire from either side. Read-only; defaults to `false` (there IS
/// debt; do the merge) on any git failure.
pub fn has_no_merge_debt(git: &dyn GitBackend, source: &str, target: &str) -> bool {
    if git.refs_have_identical_trees(source, target).unwrap_or(false) {
        return true;
    }
    git.is_ancestor(source, target).unwrap_or(false)
}

/// The `$GIT_DIR` mid-merge state markers (mechanic #3). Any of these present
/// means the working tree carries a half-applied merge/rebase/cherry-pick/revert
/// whose engine-owned files may hold conflict markers the write guards would
/// otherwise refuse to let the agent touch.
const MERGE_IN_PROGRESS_MARKERS: &[&str] = &[
    "MERGE_HEAD",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "rebase-merge",
    "rebase-apply",
];

/// Whether the working tree at `cwd` is mid-merge (or mid-rebase / cherry-pick /
/// revert), keyed on the full `$GIT_DIR` marker set (mechanic #3).
///
/// This is BROADER than [`GitBackend::merge_in_progress`] (which checks only
/// `MERGE_HEAD`, the precise signal the engine-protected merge's no-op
/// detection depends on). The write-guard path keys ownership / lifecycle /
/// branch-enforcement suspension on THIS predicate so the agent can write the
/// conflicted engine files to resolve any in-flight merge — schema validation
/// stays on regardless. Falls back to `false` (not merging) outside a git repo.
pub fn is_merge_in_progress(cwd: &Path) -> bool {
    // Resolve the git dir in-process (handles linked worktrees, whose markers
    // live under `.git/worktrees/<name>/`). Outside a repo → not merging.
    let Ok(repo) = gix::discover(cwd) else {
        return false;
    };
    let abs = repo.git_dir();
    MERGE_IN_PROGRESS_MARKERS
        .iter()
        .any(|m| abs.join(m).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Build a throwaway repo with one commit on `main`. Returns the tempdir so
    /// the caller keeps it alive for the duration of the test.
    fn init_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "test@darkrun.ai"]);
        git(&["config", "user.name", "darkrun test"]);
        std::fs::write(root.join("README.md"), "# smoke\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        (dir, root)
    }

    fn smoke(open: fn(&Path) -> Result<Git>) {
        let (_dir, root) = init_repo();
        let git = open(&root).expect("open repo");

        // current_branch + is_clean on a fresh repo.
        assert_eq!(git.current_branch().unwrap().as_deref(), Some("main"));
        assert!(git.is_clean().unwrap(), "fresh repo should be clean");

        // A new untracked file makes it dirty.
        std::fs::write(root.join("dirty.txt"), "wip").unwrap();
        assert!(!git.is_clean().unwrap(), "untracked file => dirty");
        std::fs::remove_file(root.join("dirty.txt")).unwrap();
        assert!(git.is_clean().unwrap());

        // list_worktrees sees the primary working tree on its `main` branch.
        let before = git.list_worktrees().unwrap();
        assert!(
            before
                .iter()
                .any(|w| w.branch.as_deref() == Some("main")),
            "primary worktree (on main) should be listed: {before:?}"
        );
        let initial_count = before.len();

        // create_worktree on a brand-new branch.
        let wt_path = root.join("..").join(format!(
            "wt-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let opts = CreateOptions {
            reference: None,
            new_branch: Some("station/frame".to_string()),
        };
        let info = git
            .create_worktree("frame", &wt_path, &opts)
            .expect("create worktree");
        assert_eq!(info.branch.as_deref(), Some("station/frame"));
        assert!(info.path.exists(), "worktree dir should exist on disk");

        // It now appears in the listing.
        let listed = git.list_worktrees().unwrap();
        assert_eq!(listed.len(), initial_count + 1);
        assert!(
            listed
                .iter()
                .any(|w| w.branch.as_deref() == Some("station/frame")),
            "new worktree branch should be listed: {listed:?}"
        );

        // The new worktree's branch differs from the primary checkout, proving
        // the primary tree was untouched.
        assert_eq!(git.current_branch().unwrap().as_deref(), Some("main"));

        // remove_worktree by name cleans it up.
        let name = listed
            .iter()
            .find(|w| w.branch.as_deref() == Some("station/frame"))
            .map(|w| w.name.clone())
            .expect("find created worktree name");
        git.remove_worktree(&name, true).expect("remove worktree");
        assert!(!info.path.exists(), "worktree dir should be gone");

        let after = git.list_worktrees().unwrap();
        assert!(
            !after
                .iter()
                .any(|w| w.branch.as_deref() == Some("station/frame")),
            "removed worktree should be gone: {after:?}"
        );
    }

    #[test]
    fn gix_backend_roundtrip() {
        smoke(|p| Git::open(p));
    }

    /// Run a git command in an arbitrary directory, asserting success.
    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git -C {dir:?} {args:?} failed");
    }

    fn branch_primitives(open: fn(&Path) -> Result<Git>) {
        let (_dir, root) = init_repo();
        let git = open(&root).expect("open repo");

        // branch_exists / create_branch are idempotent.
        assert!(!git.branch_exists("darkrun/r/main").unwrap());
        git.create_branch("darkrun/r/main", "main").unwrap();
        assert!(git.branch_exists("darkrun/r/main").unwrap());
        // Second create is a no-op, not an error.
        git.create_branch("darkrun/r/main", "main").unwrap();

        // Fork a station branch off run-main; both point at the same commit.
        git.create_branch("darkrun/r/build", "darkrun/r/main").unwrap();
        assert!(git
            .is_ancestor("darkrun/r/main", "darkrun/r/build")
            .unwrap());
        assert!(git
            .is_ancestor("darkrun/r/build", "darkrun/r/main")
            .unwrap());
    }

    #[test]
    fn gix_branch_primitives() {
        branch_primitives(|p| Git::open(p));
    }

    /// The engine-protected merge lands a station's agent content onto run-main
    /// while holding the run's `.darkrun/<run>` state to the target side, even
    /// when the station branch carries a stale snapshot of that state.
    fn engine_protected_merge_round_trip(open: fn(&Path) -> Result<Git>) {
        let (_dir, root) = init_repo();

        // Seed run-main with the authoritative engine state + a code file.
        git_in(&root, &["checkout", "-q", "-b", "darkrun/r/main"]);
        std::fs::create_dir_all(root.join(".darkrun/r")).unwrap();
        std::fs::write(root.join(".darkrun/r/state.json"), "{\"v\":\"authoritative\"}\n").unwrap();
        std::fs::write(root.join("code.txt"), "base\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "run-main seed"]);

        // Fork a station branch. On it: change the code (legit, should land) AND
        // clobber the engine state with a stale value (must NOT win).
        git_in(&root, &["checkout", "-q", "-b", "darkrun/r/build"]);
        std::fs::write(root.join("code.txt"), "station work\n").unwrap();
        std::fs::write(root.join(".darkrun/r/state.json"), "{\"v\":\"STALE\"}\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "station work + stale state"]);

        // Meanwhile run-main's state advanced (the manager wrote it). Get back
        // to run-main and bump the authoritative state so the two diverge.
        git_in(&root, &["checkout", "-q", "darkrun/r/main"]);
        std::fs::write(root.join(".darkrun/r/state.json"), "{\"v\":\"advanced\"}\n").unwrap();
        git_in(&root, &["add", "-A"]);
        git_in(&root, &["commit", "-q", "-m", "manager advanced run state"]);

        // Land the station onto run-main through the guard.
        let git = open(&root).expect("open repo");
        let outcome = merge::engine_protected_merge(
            &git,
            &root,
            "darkrun/r/build",
            "r",
            "land build -> run-main",
        )
        .expect("merge");
        assert!(outcome.ok, "merge should resolve cleanly: {outcome:?}");
        assert!(outcome.performed, "a real merge commit should be minted");

        // The station's CODE landed…
        assert_eq!(
            std::fs::read_to_string(root.join("code.txt")).unwrap(),
            "station work\n"
        );
        // …but the engine STATE was held to run-main's authoritative side, not
        // the station's stale snapshot (the BUG-2/3 guard).
        assert_eq!(
            std::fs::read_to_string(root.join(".darkrun/r/state.json")).unwrap(),
            "{\"v\":\"advanced\"}\n"
        );

        // The station branch is now merged into run-main.
        assert!(git
            .is_ancestor("darkrun/r/build", "darkrun/r/main")
            .unwrap());
    }

    #[test]
    fn gix_engine_protected_merge() {
        engine_protected_merge_round_trip(|p| Git::open(p));
    }

    /// An up-to-date merge (source already in target) is a clean no-op.
    #[test]
    fn engine_protected_merge_up_to_date_is_noop() {
        let (_dir, root) = init_repo();
        git_in(&root, &["checkout", "-q", "-b", "darkrun/r/main"]);
        let git = Git::open(&root).expect("open");
        // build forked at the same commit, no new work → already up to date.
        git.create_branch("darkrun/r/build", "darkrun/r/main").unwrap();
        let outcome =
            merge::engine_protected_merge(&git, &root, "darkrun/r/build", "r", "noop").expect("merge");
        assert!(outcome.ok);
        assert!(!outcome.performed, "no commit for an up-to-date merge");
    }

    #[test]
    fn open_rejects_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(matches!(
            Git::open(dir.path()),
            Err(GitError::NotARepo(_))
        ));
    }
}

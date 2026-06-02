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
//! Operations go through the [`GitBackend`] trait. The default
//! [`Libgit2Backend`] drives libgit2 in-process; [`ShellBackend`] is the
//! shell-out fallback that maps each operation to the matching `git` command.
//! [`Git`] is a thin facade that prefers libgit2 and is the recommended entry
//! point.

mod authorship;
mod backend;
mod clone;
mod error;
mod libgit2;
pub mod merge;
mod shell;

use std::path::{Path, PathBuf};

pub use authorship::{
    branch_author, branch_authored_by, current_identity_email, run_authored_by_me,
};
pub use backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
pub use clone::{clone_repo, default_clone_dest, repo_name_from_url};
pub use error::{GitError, Result};
pub use libgit2::Libgit2Backend;
pub use merge::{engine_protected_merge, is_engine_owned_state_path, ENGINE_STATE_PREFIX};
pub use shell::ShellBackend;

/// The recommended entry point: a [`GitBackend`] facade over a repository.
///
/// `Git` wraps the libgit2 backend by default. Use [`Git::open_shell`] to force
/// the shell-out fallback when libgit2 is undesirable in a given environment.
pub struct Git {
    inner: Box<dyn GitBackend + Send + Sync>,
    repo_root: PathBuf,
}

impl Git {
    /// Open `repo_root` with the default (libgit2) backend.
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref().to_path_buf();
        let inner = Libgit2Backend::open(&root)?;
        Ok(Self {
            inner: Box::new(inner),
            repo_root: root,
        })
    }

    /// Open `repo_root` forcing the shell-out (`git` CLI) backend.
    pub fn open_shell(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref().to_path_buf();
        let inner = ShellBackend::open(&root)?;
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

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        self.inner.commit(worktree_path, message)
    }

    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>> {
        self.inner.ls_tree(worktree_path, from_ref, prefix)
    }

    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>> {
        self.inner.unresolved_paths(worktree_path)
    }
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
    fn libgit2_backend_roundtrip() {
        smoke(|p| Git::open(p));
    }

    #[test]
    fn shell_backend_roundtrip() {
        smoke(|p| Git::open_shell(p));
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
    fn libgit2_branch_primitives() {
        branch_primitives(|p| Git::open(p));
    }

    #[test]
    fn shell_branch_primitives() {
        branch_primitives(|p| Git::open_shell(p));
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
    fn libgit2_engine_protected_merge() {
        engine_protected_merge_round_trip(|p| Git::open(p));
    }

    #[test]
    fn shell_engine_protected_merge() {
        engine_protected_merge_round_trip(|p| Git::open_shell(p));
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
        assert!(matches!(
            Git::open_shell(dir.path()),
            Err(GitError::NotARepo(_))
        ));
    }
}

//! Branch authorship — "is this run mine?"
//!
//! A darkrun run does its work on a branch (conventionally `darkrun/<slug>`).
//! A run is considered *mine* when the current git identity authored at least
//! one commit on that branch beyond the base it forked from. These helpers
//! answer that question against a repository, using libgit2 in-process so the
//! engine never shells out.
//!
//! The predicate is deliberately a cheap-but-correct proxy:
//!
//! - [`current_identity_email`] reads the effective `user.email` from the repo
//!   config (local, then global / system), exactly as `git commit` would.
//! - [`branch_authored_by`] walks the commits reachable from `head` but **not**
//!   from `base` — the run's own contribution — and returns `true` as soon as
//!   one of those commits' *author* email matches (case-insensitively). It
//!   stops at the first match, so the common case (the most recent commit is
//!   yours) is one comparison.
//!
//! When `head` cannot be resolved (the branch was never created, or was merged
//! and deleted) the predicate is simply `false`: nothing on disk attributes the
//! work to anyone, so it does not count as mine. When `base` cannot be resolved
//! the walk degrades to "every commit reachable from `head`", which is still a
//! correct (if broader) authorship answer.

use std::path::Path;

use git2::{Oid, Repository, Sort};

use crate::error::{GitError, Result};

/// The effective committer/author email for the repository at `repo_root` —
/// the `user.email` `git commit` would stamp. `None` when no identity is
/// configured (config is missing the key at every level).
pub fn current_identity_email(repo_root: impl AsRef<Path>) -> Result<Option<String>> {
    let repo = Repository::discover(repo_root.as_ref())
        .map_err(|_| GitError::NotARepo(repo_root.as_ref().to_path_buf()))?;
    // `repo.config()` includes the local file plus the global/system snapshots,
    // matching git's own precedence for resolving an identity.
    let cfg = repo.config()?;
    Ok(cfg
        .get_string("user.email")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// Whether `email` authored any commit reachable from `head` but not from
/// `base`, in the repository at `repo_root`.
///
/// `base` is the branch the run forked from (e.g. the project default branch);
/// `head` is the run's working branch (e.g. `darkrun/<slug>`). Both are
/// resolved as revisions (a branch name, tag, or raw oid all work). Matching is
/// case-insensitive on the commit *author* email.
///
/// Returns `false` (not an error) when `head` does not resolve — an absent
/// branch attributes no work to anyone. When `base` does not resolve, the walk
/// covers everything reachable from `head`.
pub fn branch_authored_by(
    repo_root: impl AsRef<Path>,
    base: &str,
    head: &str,
    email: &str,
) -> Result<bool> {
    let repo = Repository::discover(repo_root.as_ref())
        .map_err(|_| GitError::NotARepo(repo_root.as_ref().to_path_buf()))?;

    // No head → no branch → no work to claim.
    let Some(head_oid) = resolve(&repo, head) else {
        return Ok(false);
    };

    let want = email.trim().to_ascii_lowercase();
    if want.is_empty() {
        return Ok(false);
    }

    let mut walk = repo.revwalk()?;
    // Walk in commit-time order; we stop at the first match so ordering only
    // affects which match we find first, not correctness.
    walk.set_sorting(Sort::TIME)?;
    walk.push(head_oid)?;
    // Exclude the base history so we only consider the run's own commits. A
    // missing base just means we consider everything reachable from head.
    if let Some(base_oid) = resolve(&repo, base) {
        walk.hide(base_oid)?;
    }

    for oid in walk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        // Bind the author signature: its borrowed `email()` outlives a chained
        // temporary, so hold the `Signature` for the comparison.
        let author = commit.author();
        if let Some(author_email) = author.email() {
            if author_email.trim().eq_ignore_ascii_case(&want) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The author NAME of the most recent commit on `head` beyond `base` — the owner
/// of the run's branch, for display and author search. `None` when the branch is
/// missing or carries none of its own commits.
pub fn branch_author(
    repo_root: impl AsRef<Path>,
    base: &str,
    head: &str,
) -> Result<Option<String>> {
    let repo = Repository::discover(repo_root.as_ref())
        .map_err(|_| GitError::NotARepo(repo_root.as_ref().to_path_buf()))?;
    let Some(head_oid) = resolve(&repo, head) else {
        return Ok(None);
    };
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TIME)?;
    walk.push(head_oid)?;
    if let Some(base_oid) = resolve(&repo, base) {
        walk.hide(base_oid)?;
    }
    for oid in walk {
        let commit = repo.find_commit(oid?)?;
        let author = commit.author();
        if let Some(name) = author.name() {
            let n = name.trim();
            if !n.is_empty() {
                return Ok(Some(n.to_string()));
            }
        }
    }
    Ok(None)
}

/// Whether the current git identity authored any commit on a run's branch.
///
/// Convenience wrapper over [`current_identity_email`] + [`branch_authored_by`]
/// for the common run-list path: resolve the effective `user.email`, then test
/// the run's `head` branch against `base`. Returns `false` when there is no
/// configured identity (nothing to match) or the branch carries none of its
/// commits.
pub fn run_authored_by_me(
    repo_root: impl AsRef<Path>,
    base: &str,
    head: &str,
) -> Result<bool> {
    let repo_root = repo_root.as_ref();
    let Some(email) = current_identity_email(repo_root)? else {
        return Ok(false);
    };
    branch_authored_by(repo_root, base, head, &email)
}

/// Resolve a revision string (branch, tag, or oid) to a commit oid, or `None`
/// when it does not exist in the repository.
fn resolve(repo: &Repository, rev: &str) -> Option<Oid> {
    let object = repo.revparse_single(rev).ok()?;
    object.peel_to_commit().ok().map(|c| c.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// Build a repo on `main` with one base commit authored by `base@x.io`.
    fn init_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().to_path_buf();
        run_git(&root, &["init", "-q", "-b", "main"]);
        run_git(&root, &["config", "user.email", "base@x.io"]);
        run_git(&root, &["config", "user.name", "Base Dev"]);
        write(&root, "README.md", "# base\n");
        run_git(&root, &["add", "-A"]);
        run_git(&root, &["commit", "-q", "-m", "base"]);
        (dir, root)
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn write(root: &Path, name: &str, body: &str) {
        std::fs::write(root.join(name), body).unwrap();
    }

    /// Branch off `main`, set the committing identity, and land a commit there.
    fn commit_on_branch(root: &Path, branch: &str, email: &str, file: &str) {
        run_git(root, &["checkout", "-q", "-b", branch]);
        run_git(root, &["config", "user.email", email]);
        run_git(root, &["config", "user.name", "Worker"]);
        write(root, file, "work\n");
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-q", "-m", "work"]);
    }

    #[test]
    fn current_identity_reads_config() {
        let (_d, root) = init_repo();
        assert_eq!(
            current_identity_email(&root).unwrap().as_deref(),
            Some("base@x.io")
        );
    }

    #[test]
    fn mine_when_i_authored_a_commit_on_the_branch() {
        let (_d, root) = init_repo();
        commit_on_branch(&root, "darkrun/run-a", "me@x.io", "a.txt");
        assert!(
            branch_authored_by(&root, "main", "darkrun/run-a", "me@x.io").unwrap(),
            "branch with my commit should be mine"
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let (_d, root) = init_repo();
        commit_on_branch(&root, "darkrun/run-a", "Me@X.IO", "a.txt");
        assert!(branch_authored_by(&root, "main", "darkrun/run-a", "me@x.io").unwrap());
    }

    #[test]
    fn not_mine_when_someone_else_authored() {
        let (_d, root) = init_repo();
        commit_on_branch(&root, "darkrun/run-b", "other@x.io", "b.txt");
        assert!(
            !branch_authored_by(&root, "main", "darkrun/run-b", "me@x.io").unwrap(),
            "a branch authored solely by someone else is not mine"
        );
    }

    #[test]
    fn base_commits_do_not_count() {
        // The base commit is authored by base@x.io. Asking whether base@x.io
        // authored anything *beyond* main on a branch that only forks (no new
        // commits) must be false — the shared history is excluded.
        let (_d, root) = init_repo();
        run_git(&root, &["branch", "darkrun/empty"]);
        assert!(
            !branch_authored_by(&root, "main", "darkrun/empty", "base@x.io").unwrap(),
            "commits shared with base are excluded from authorship"
        );
    }

    #[test]
    fn missing_head_is_not_mine() {
        let (_d, root) = init_repo();
        assert!(
            !branch_authored_by(&root, "main", "darkrun/ghost", "me@x.io").unwrap(),
            "an absent branch attributes work to no one"
        );
    }

    #[test]
    fn missing_base_walks_full_head_history() {
        // With a base that does not resolve, the base author's *base* commit is
        // now in scope, so base@x.io counts as having authored on the branch.
        let (_d, root) = init_repo();
        commit_on_branch(&root, "darkrun/run-c", "me@x.io", "c.txt");
        assert!(
            branch_authored_by(&root, "does-not-exist", "darkrun/run-c", "base@x.io").unwrap(),
            "an unresolved base widens the walk to all of head's history"
        );
    }

    #[test]
    fn run_authored_by_me_uses_configured_identity() {
        let (_d, root) = init_repo();
        // Land a commit authored by the repo identity on a run branch, then
        // restore the identity that landed it so it is the "current" one.
        commit_on_branch(&root, "darkrun/run-d", "me@x.io", "d.txt");
        run_git(&root, &["config", "user.email", "me@x.io"]);
        assert!(run_authored_by_me(&root, "main", "darkrun/run-d").unwrap());

        // A different current identity that authored nothing on the branch.
        run_git(&root, &["config", "user.email", "stranger@x.io"]);
        assert!(!run_authored_by_me(&root, "main", "darkrun/run-d").unwrap());
    }

    #[test]
    fn branch_author_returns_the_committer_name() {
        let (_d, root) = init_repo();
        commit_on_branch(&root, "darkrun/run-e", "worker@x.io", "e.txt");
        assert_eq!(
            branch_author(&root, "main", "darkrun/run-e").unwrap().as_deref(),
            Some("Worker")
        );
        // A branch with no commits beyond base names no author.
        run_git(&root, &["checkout", "-q", "main"]);
        run_git(&root, &["branch", "darkrun/empty"]);
        assert_eq!(branch_author(&root, "main", "darkrun/empty").unwrap(), None);
        // A missing head resolves to no author.
        assert_eq!(branch_author(&root, "main", "darkrun/ghost").unwrap(), None);
    }
}

//! Cloning a remote repository.
//!
//! The rest of this crate is deliberately network-free — worktree and status
//! queries against a repo that already exists locally. Cloning is the one
//! operation that reaches the network, driven in-process by gitoxide's
//! pure-Rust clone (reqwest + rustls for HTTPS) — no `git` CLI, no C.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::error::{GitError, Result};
use crate::Git;

/// Map any gix error into our crate error.
fn gix_err(e: impl std::fmt::Display) -> GitError {
    GitError::Gix(e.to_string())
}

/// Clone `url` into `dest`, returning a [`Git`] facade open on the clone.
///
/// `dest` is the working-tree directory to create (e.g. `~/darkrun/<repo>`); it
/// must not already exist as a non-empty directory, and its parent is created
/// if missing. On success the destination holds the checked-out repo and the
/// returned [`Git`] is opened against it.
///
/// Surfaces a [`GitError::Gix`] carrying gitoxide's message when the clone fails
/// (bad URL, auth failure, network down), so callers can show the operator the
/// real reason instead of a generic error.
#[cfg(not(tarpaulin_include))] // clones a repo over the network — irreducible I/O
pub fn clone_repo(url: &str, dest: &Path) -> Result<Git> {
    // Create the parent so a target like `~/darkrun/<repo>` works on a fresh
    // machine where `~/darkrun` doesn't exist yet. gix creates `dest` itself.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|source| GitError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    // Fetch into a fresh repo at `dest`, then check out its main worktree. Under
    // `blocking-network-client` these are synchronous (maybe_async-stripped).
    let interrupt = AtomicBool::new(false);
    let mut fetch = gix::prepare_clone(url, dest).map_err(gix_err)?;
    let (mut checkout, _) = fetch
        .fetch_then_checkout(gix::progress::Discard, &interrupt)
        .map_err(gix_err)?;
    checkout
        .main_worktree(gix::progress::Discard, &interrupt)
        .map_err(gix_err)?;

    // Open the freshly-cloned tree so the caller gets a ready-to-use facade.
    Git::open(dest)
}

/// Derive the default clone target directory name from a git `url`.
///
/// Strips a trailing `.git` and any path/query, yielding the repo's basename —
/// the name `git clone <url>` would itself pick. Used to default the clone
/// destination to `~/darkrun/<name>`. Falls back to `"repo"` for a URL with no
/// recoverable basename.
pub fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    // Split on both `/` (https / path) and `:` (scp-style `git@host:owner/repo`).
    let tail = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed);
    let name = tail.strip_suffix(".git").unwrap_or(tail);
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

/// The default clone destination for `url` under `base` — `<base>/<repo-name>`.
///
/// `base` is the editable clone-root the desktop defaults to `~/darkrun`. The
/// caller may override the returned path before cloning.
pub fn default_clone_dest(base: &Path, url: &str) -> PathBuf {
    base.join(repo_name_from_url(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GitBackend;
    use std::process::Command;
    use tempfile::TempDir;

    #[test]
    fn repo_name_strips_git_suffix_and_path() {
        assert_eq!(repo_name_from_url("https://github.com/acme/store.git"), "store");
        assert_eq!(repo_name_from_url("https://github.com/acme/store"), "store");
        assert_eq!(repo_name_from_url("git@github.com:acme/store.git"), "store");
        assert_eq!(repo_name_from_url("https://example.com/store/"), "store");
        assert_eq!(repo_name_from_url(""), "repo");
    }

    #[test]
    fn default_dest_joins_base_and_name() {
        let dest = default_clone_dest(Path::new("/home/me/darkrun"), "https://x/y/proj.git");
        assert_eq!(dest, PathBuf::from("/home/me/darkrun/proj"));
    }

    #[test]
    fn clone_from_local_source_repo() {
        // Build a throwaway source repo with one commit, then clone it from a
        // local path (no network) and verify the clone opens cleanly.
        let src_dir = TempDir::new().unwrap();
        let src = src_dir.path().to_path_buf();
        let git = |cwd: &Path, args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(cwd)
                .args(args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&src, &["init", "-q", "-b", "main"]);
        git(&src, &["config", "user.email", "test@darkrun.ai"]);
        git(&src, &["config", "user.name", "darkrun test"]);
        std::fs::write(src.join("README.md"), "# src\n").unwrap();
        git(&src, &["add", "-A"]);
        git(&src, &["commit", "-q", "-m", "init"]);

        let work = TempDir::new().unwrap();
        let dest = work.path().join("nested").join("clone");
        let cloned = clone_repo(&src.to_string_lossy(), &dest).expect("clone");

        assert!(dest.join("README.md").exists(), "clone should have content");
        assert_eq!(cloned.repo_root(), dest.as_path());
        assert_eq!(cloned.current_branch().unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn clone_bad_url_surfaces_an_error() {
        let work = TempDir::new().unwrap();
        let dest = work.path().join("nope");
        match clone_repo("/this/path/does/not/exist.git", &dest) {
            Err(GitError::Gix(_)) => {}
            Err(other) => panic!("expected a gix error, got {other:?}"),
            Ok(_) => panic!("clone of a nonexistent source should fail"),
        }
    }
}

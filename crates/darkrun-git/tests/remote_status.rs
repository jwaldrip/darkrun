//! Integration tests for darkrun-git status & branch reporting — the "remote_status"
//! surface the Factory relies on when a Worker reads the state of a Run's checkout.
//!
//! The crate exposes worktree primitives plus two read-only status queries
//! (`current_branch`, `is_clean`) and a listing (`list_worktrees`). These tests
//! drive the public [`Git`] facade against throwaway repositories built in a
//! `TempDir`, exercising branch reporting, detached-HEAD handling, and the many
//! edge cases of dirty-file detection. Every scenario runs against the pure-Rust
//! gix backend, anchored to the real `git` binary (the `git`/`git_out` helpers)
//! as the conformance oracle.
//!
//! NOTE on remote parsing: the public API of this crate does not expose a remote
//! reader or a host/owner/repo URL parser. Rather than weaken these tests by
//! reaching into private internals, we exercise the closest real public surface —
//! the status and branch-reporting queries that a Reviewer consults during the
//! audit phase — and arrange real remotes via plain `git` so the listing-level
//! behaviour around configured origins is still covered where the API permits.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use darkrun_git::{CreateOptions, Git, GitBackend, GitError};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

/// Run `git <args>` in `root`, asserting success. Only the test harness shells
/// out here to arrange state; the code under test does its own thing.
fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git {args:?} failed in {root:?}");
}

/// Run `git <args>` in `root` and return trimmed stdout.
fn git_out(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?} failed in {root:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Initialise a repository on `main` with a single commit. Returns the owning
/// `TempDir` (keep it alive) and the repo root.
fn init_repo() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "worker@darkrun.local"]);
    git(&root, &["config", "user.name", "darkrun worker"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "init"]);
    (dir, root)
}

/// Add a second commit so HEAD can advance past its parent.
fn advance(root: &Path, file: &str, body: &str, msg: &str) -> String {
    std::fs::write(root.join(file), body).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", msg]);
    git_out(root, &["rev-parse", "HEAD"])
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique sibling path next to `root` so worktrees never nest inside the repo.
fn sibling(root: &Path, label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    root.parent()
        .unwrap()
        .join(format!("dr-rs-{label}-{nanos}-{n}"))
}

/// A named backend constructor: a label and a function opening a repo root.
type NamedBackend = (&'static str, fn(&Path) -> darkrun_git::Result<Git>);

/// The sole backend under test: the pure-Rust gix backend, driven against the
/// real-`git` oracle (the `git`/`git_out` helpers) for every conformance check.
fn backends() -> Vec<NamedBackend> {
    vec![("gix", |p| Git::open_gix(p))]
}

/// Find the worktree entry matching `path` regardless of symlink normalisation.
fn find_by_path<'a>(list: &'a [darkrun_git::WorktreeInfo], path: &Path) -> Option<&'a darkrun_git::WorktreeInfo> {
    list.iter()
        .find(|w| w.path == path)
        .or_else(|| list.iter().find(|w| w.path.ends_with(path.file_name().unwrap())))
        .or_else(|| {
            list.iter()
                .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok())
        })
}

// ===========================================================================
// current_branch — basic reads
// ===========================================================================

#[test]
fn current_branch_reads_main() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("main"),
            "[{label}] fresh repo on main"
        );
    }
}

#[test]
fn current_branch_after_checkout_new() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "feature/alpha"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("feature/alpha"),
            "[{label}]"
        );
    }
}

#[test]
fn current_branch_after_switch_back() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "temp"]);
    git(&root, &["checkout", "-q", "main"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
    }
}

#[test]
fn current_branch_with_slashes_in_name() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "station/spec/deep/nest"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("station/spec/deep/nest"),
            "[{label}] deeply nested branch name preserved verbatim"
        );
    }
}

#[test]
fn current_branch_with_dashes_and_dots() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "release-1.2.3-rc.4"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("release-1.2.3-rc.4"),
            "[{label}]"
        );
    }
}

#[test]
fn current_branch_with_underscores() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "audit_phase_checkpoint"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("audit_phase_checkpoint"),
            "[{label}]"
        );
    }
}

#[test]
fn current_branch_unicode_name() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "función/café"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("función/café"),
            "[{label}] unicode branch name round-trips"
        );
    }
}

#[test]
fn current_branch_numeric_name() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "12345"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("12345"), "[{label}]");
    }
}

#[test]
fn current_branch_single_char_name() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "x"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("x"), "[{label}]");
    }
}

#[test]
fn current_branch_does_not_strip_feature_prefix() {
    // A branch literally named "feature" (not "feature/...") must report as
    // exactly that — no accidental prefix munging.
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "feature"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("feature"), "[{label}]");
    }
}

#[test]
fn current_branch_name_containing_head_substring() {
    // "ahead" contains "HEAD" only case-insensitively; ensure detached-HEAD
    // detection does not false-positive on branch names.
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "ahead-of-main"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("ahead-of-main"),
            "[{label}] branch with 'head' substring is still a branch"
        );
    }
}

// ===========================================================================
// current_branch — detached HEAD
// ===========================================================================

#[test]
fn current_branch_none_when_detached_at_head() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap(), None, "[{label}] detached => None");
    }
}

#[test]
fn current_branch_none_when_detached_at_older_commit() {
    let (_d, root) = init_repo();
    let first = git_out(&root, &["rev-parse", "HEAD"]);
    advance(&root, "a.txt", "a", "second");
    git(&root, &["checkout", "-q", &first]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap(),
            None,
            "[{label}] detached at an older commit is still detached"
        );
    }
}

#[test]
fn current_branch_reattaches_after_leaving_detached() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    git(&root, &["checkout", "-q", "main"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("main"),
            "[{label}] reattach to main reports a branch again"
        );
    }
}

#[test]
fn current_branch_detached_via_tag() {
    let (_d, root) = init_repo();
    git(&root, &["tag", "v0.1.0"]);
    git(&root, &["checkout", "-q", "v0.1.0"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap(),
            None,
            "[{label}] checking out a tag detaches HEAD"
        );
    }
}

#[test]
fn current_branch_is_idempotent() {
    // Repeated reads must not mutate state or drift.
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "stable"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let first = g.current_branch().unwrap();
        for _ in 0..5 {
            assert_eq!(g.current_branch().unwrap(), first, "[{label}] idempotent reads");
        }
        assert_eq!(first.as_deref(), Some("stable"));
    }
}

#[test]
fn current_branch_stable_across_reopen() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "persisted"]);
    for (label, open) in backends() {
        let a = open(&root).unwrap().current_branch().unwrap();
        let b = open(&root).unwrap().current_branch().unwrap();
        assert_eq!(a, b, "[{label}] reopening the repo yields the same branch");
        assert_eq!(a.as_deref(), Some("persisted"));
    }
}

// ===========================================================================
// is_clean — fresh / dirty basics
// ===========================================================================

#[test]
fn is_clean_fresh_repo() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] fresh repo is clean");
    }
}

#[test]
fn is_clean_false_on_untracked_file() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("scratch.txt"), "wip").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] untracked => dirty");
    }
}

#[test]
fn is_clean_recovers_after_untracked_removed() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("scratch.txt"), "wip").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] dirty with file");
    }
    std::fs::remove_file(root.join("scratch.txt")).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean after removal");
    }
}

#[test]
fn is_clean_false_on_modified_tracked() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nmodified\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] modified tracked => dirty");
    }
}

#[test]
fn is_clean_false_on_staged_new_file() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("new.txt"), "content").unwrap();
    git(&root, &["add", "new.txt"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] staged add => dirty");
    }
}

#[test]
fn is_clean_false_on_staged_modification() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nstaged change\n").unwrap();
    git(&root, &["add", "README.md"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] staged modify => dirty");
    }
}

#[test]
fn is_clean_false_on_deleted_tracked_file() {
    let (_d, root) = init_repo();
    std::fs::remove_file(root.join("README.md")).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] deleted tracked file => dirty");
    }
}

#[test]
fn is_clean_false_on_staged_deletion() {
    let (_d, root) = init_repo();
    git(&root, &["rm", "-q", "README.md"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] staged deletion => dirty");
    }
}

#[test]
fn is_clean_true_after_commit_of_changes() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\ncommitted change\n").unwrap();
    git(&root, &["commit", "-aqm", "change"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] committing changes returns clean");
    }
}

#[test]
fn is_clean_true_after_staged_then_committed() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("added.txt"), "x").unwrap();
    git(&root, &["add", "added.txt"]);
    git(&root, &["commit", "-qm", "add file"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] committed staged file is clean");
    }
}

// ===========================================================================
// is_clean — gitignore semantics
// ===========================================================================

#[test]
fn is_clean_ignores_gitignored_dir() {
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore"]);
    std::fs::create_dir_all(root.join("ignored")).unwrap();
    std::fs::write(root.join("ignored").join("junk.txt"), "junk").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] gitignored content not dirty");
    }
}

#[test]
fn is_clean_ignores_gitignored_file_pattern() {
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore logs"]);
    std::fs::write(root.join("run.log"), "noise").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] *.log ignored => clean");
    }
}

#[test]
fn is_clean_dirty_when_gitignore_itself_is_new() {
    // An uncommitted .gitignore is itself an untracked file => dirty.
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "stuff/\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] an uncommitted .gitignore is an untracked file"
        );
    }
}

#[test]
fn is_clean_dirty_when_ignored_file_is_tracked_then_modified() {
    // A file matched by .gitignore but already tracked still reports dirty when
    // modified — gitignore only affects untracked paths.
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "config.toml\n").unwrap();
    std::fs::write(root.join("config.toml"), "v=1\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["add", "-f", "config.toml"]);
    git(&root, &["commit", "-qm", "track despite ignore"]);
    std::fs::write(root.join("config.toml"), "v=2\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] tracked-but-ignored file modification is still dirty"
        );
    }
}

#[test]
fn is_clean_negated_gitignore_pattern() {
    // `*.tmp` ignored but `!keep.tmp` un-ignored => keep.tmp untracked => dirty.
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "*.tmp\n!keep.tmp\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore with negation"]);
    std::fs::write(root.join("drop.tmp"), "x").unwrap();
    std::fs::write(root.join("keep.tmp"), "y").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] negated pattern re-includes keep.tmp => dirty"
        );
    }
}

// ===========================================================================
// is_clean — edge cases
// ===========================================================================

#[test]
fn is_clean_empty_untracked_file_counts() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("empty.txt"), "").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] empty untracked file => dirty");
    }
}

#[test]
fn is_clean_nested_untracked_directory() {
    let (_d, root) = init_repo();
    std::fs::create_dir_all(root.join("a").join("b").join("c")).unwrap();
    std::fs::write(root.join("a").join("b").join("c").join("deep.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] deeply nested untracked => dirty");
    }
}

#[test]
fn is_clean_whitespace_only_modification() {
    let (_d, root) = init_repo();
    // Append trailing whitespace — still a content change.
    std::fs::write(root.join("README.md"), "# fixture\n   \n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] whitespace change => dirty");
    }
}

#[test]
fn is_clean_rewrite_to_identical_content_is_clean() {
    // Writing the exact same bytes does not change the tree content.
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            g.is_clean().unwrap(),
            "[{label}] rewriting identical content leaves the tree clean"
        );
    }
}

#[test]
fn is_clean_unicode_filename_untracked() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("café-señor.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] unicode-named untracked => dirty");
    }
}

#[test]
fn is_clean_filename_with_spaces() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("a file with spaces.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] spaced filename untracked => dirty");
    }
}

#[test]
fn is_clean_many_untracked_files() {
    let (_d, root) = init_repo();
    for i in 0..50 {
        std::fs::write(root.join(format!("f{i}.txt")), "x").unwrap();
    }
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] many untracked => dirty");
    }
}

#[test]
fn is_clean_mixed_staged_and_unstaged() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("staged.txt"), "s").unwrap();
    git(&root, &["add", "staged.txt"]);
    std::fs::write(root.join("README.md"), "# fixture\nunstaged\n").unwrap();
    std::fs::write(root.join("untracked.txt"), "u").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] any combination of staged/unstaged/untracked => dirty"
        );
    }
}

#[test]
fn is_clean_is_idempotent() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("x.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let first = g.is_clean().unwrap();
        for _ in 0..5 {
            assert_eq!(g.is_clean().unwrap(), first, "[{label}] idempotent");
        }
        assert!(!first, "[{label}] dirty with untracked file");
    }
}

#[test]
fn is_clean_does_not_mutate_working_tree() {
    // A status query must be side-effect free: the file set is unchanged after.
    let (_d, root) = init_repo();
    std::fs::write(root.join("probe.txt"), "p").unwrap();
    let before = git_out(&root, &["status", "--porcelain"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let _ = g.is_clean().unwrap();
        let after = git_out(&root, &["status", "--porcelain"]);
        assert_eq!(before, after, "[{label}] is_clean must not mutate state");
    }
}

#[test]
fn is_clean_transition_clean_dirty_clean() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] start clean");
        let scratch = root.join(format!("t-{label}.txt"));
        std::fs::write(&scratch, "x").unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] now dirty");
        std::fs::remove_file(&scratch).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean again");
    }
}

// ===========================================================================
// list_worktrees — primary + branch reporting
// ===========================================================================

#[test]
fn list_includes_primary_on_main() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some("main")),
            "[{label}] primary on main listed: {list:?}"
        );
    }
}

#[test]
fn list_primary_reflects_current_branch() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "dev"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some("dev")),
            "[{label}] primary tracks checked-out branch: {list:?}"
        );
    }
}

#[test]
fn list_nonempty_always() {
    // The primary working tree is always present, so the list is never empty.
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.list_worktrees().unwrap().is_empty(), "[{label}] never empty");
    }
}

#[test]
fn list_primary_paths_are_absolute() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        for w in g.list_worktrees().unwrap() {
            assert!(w.path.is_absolute(), "[{label}] path absolute: {:?}", w.path);
        }
    }
}

#[test]
fn list_entries_have_nonempty_names() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        for w in g.list_worktrees().unwrap() {
            assert!(!w.name.is_empty(), "[{label}] name non-empty: {w:?}");
        }
    }
}

#[test]
fn list_is_idempotent() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let a = g.list_worktrees().unwrap();
        let b = g.list_worktrees().unwrap();
        assert_eq!(a, b, "[{label}] repeated listings are stable");
    }
}

#[test]
fn list_sees_added_worktree_branch() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "added");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("manufacture/unit".to_string()),
    };
    g.create_worktree("unit", &path, &opts).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some("manufacture/unit")),
            "[{label}] added worktree branch listed: {list:?}"
        );
    }
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

#[test]
fn list_added_worktree_path_exists() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "existpath");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("p".to_string()),
    };
    let info = g.create_worktree("p", &path, &opts).unwrap();
    let list = g.list_worktrees().unwrap();
    let entry = find_by_path(&list, &info.path).expect("entry present");
    assert!(entry.path.exists(), "listed worktree path exists on disk");
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

#[test]
fn list_grows_and_shrinks_with_worktrees() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let base = g.list_worktrees().unwrap().len();
    let path = sibling(&root, "grow");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("grow-branch".to_string()),
    };
    let info = g.create_worktree("grow", &path, &opts).unwrap();
    assert_eq!(g.list_worktrees().unwrap().len(), base + 1, "grew by one");
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("grow-branch"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    assert_eq!(g.list_worktrees().unwrap().len(), base, "shrank back");
    assert!(!info.path.exists());
}

#[test]
fn list_multiple_distinct_branches() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let mut paths = Vec::new();
    for i in 0..4 {
        let path = sibling(&root, &format!("dist-{i}"));
        let opts = CreateOptions {
            reference: None,
            new_branch: Some(format!("pass/{i}")),
        };
        g.create_worktree(&format!("d{i}"), &path, &opts).unwrap();
        paths.push(path);
    }
    let list = g.list_worktrees().unwrap();
    for i in 0..4 {
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some(format!("pass/{i}").as_str())),
            "branch pass/{i} listed: {list:?}"
        );
    }
    for p in paths {
        git(&root, &["worktree", "remove", "--force", &p.to_string_lossy()]);
    }
}

#[test]
fn list_reports_detached_worktree_branch_none() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    let path = sibling(&root, "detw");
    git(&root, &["worktree", "add", "--detach", &path.to_string_lossy(), &head]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let entry = find_by_path(&list, &path)
            .unwrap_or_else(|| panic!("[{label}] detached entry present: {list:?}"));
        assert_eq!(entry.branch, None, "[{label}] detached => branch None: {entry:?}");
    }
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

#[test]
fn list_reports_locked_worktree() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "lockw");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("locked-branch".to_string()),
    };
    let info = g.create_worktree("lockw", &path, &opts).unwrap();
    git(&root, &["worktree", "lock", &info.path.to_string_lossy()]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let entry = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("locked-branch"))
            .unwrap_or_else(|| panic!("[{label}] locked entry: {list:?}"));
        assert!(entry.locked, "[{label}] locked flag set: {entry:?}");
    }
    git(&root, &["worktree", "unlock", &info.path.to_string_lossy()]);
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

#[test]
fn list_unlocked_worktree_not_flagged_locked() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "unlk");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("unlocked-branch".to_string()),
    };
    let info = g.create_worktree("unlk", &path, &opts).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let entry = g
            .list_worktrees()
            .unwrap()
            .into_iter()
            .find(|w| w.branch.as_deref() == Some("unlocked-branch"))
            .unwrap_or_else(|| panic!("[{label}] entry present"));
        assert!(!entry.locked, "[{label}] fresh worktree is not locked");
    }
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

#[test]
fn list_lock_then_unlock_toggles_flag() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "toggle");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("toggle-branch".to_string()),
    };
    let info = g.create_worktree("toggle", &path, &opts).unwrap();
    let pstr = info.path.to_string_lossy().to_string();

    let locked = |g: &Git| {
        g.list_worktrees()
            .unwrap()
            .into_iter()
            .find(|w| w.branch.as_deref() == Some("toggle-branch"))
            .map(|w| w.locked)
            .unwrap()
    };

    assert!(!locked(&g), "starts unlocked");
    git(&root, &["worktree", "lock", &pstr]);
    assert!(locked(&Git::open(&root).unwrap()), "locked after lock");
    git(&root, &["worktree", "unlock", &pstr]);
    assert!(!locked(&Git::open(&root).unwrap()), "unlocked after unlock");

    git(&root, &["worktree", "remove", "--force", &pstr]);
}

// ===========================================================================
// Backend agreement — both implementations must report the same status
// ===========================================================================

#[test]
fn gix_current_branch_on_main_matches_real_git() {
    let (_d, root) = init_repo();
    let a = Git::open(&root).unwrap().current_branch().unwrap();
    assert_eq!(a.as_deref(), Some(git_out(&root, &["branch", "--show-current"]).as_str()));
    assert_eq!(a.as_deref(), Some("main"));
}

#[test]
fn gix_current_branch_on_feature_matches_real_git() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "feature/agreement"]);
    let a = Git::open(&root).unwrap().current_branch().unwrap();
    assert_eq!(a.as_deref(), Some(git_out(&root, &["branch", "--show-current"]).as_str()));
    assert_eq!(a.as_deref(), Some("feature/agreement"));
}

#[test]
fn gix_current_branch_detached_matches_real_git() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    let a = Git::open(&root).unwrap().current_branch().unwrap();
    // Detached: `git branch --show-current` is empty, mapping to None.
    assert_eq!(git_out(&root, &["branch", "--show-current"]), "");
    assert_eq!(a, None);
}

#[test]
fn gix_is_clean_fresh_matches_real_git() {
    let (_d, root) = init_repo();
    let a = Git::open(&root).unwrap().is_clean().unwrap();
    assert_eq!(a, git_out(&root, &["status", "--porcelain"]).is_empty());
    assert!(a);
}

#[test]
fn gix_is_clean_untracked_matches_real_git() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("x.txt"), "x").unwrap();
    let a = Git::open(&root).unwrap().is_clean().unwrap();
    assert_eq!(a, git_out(&root, &["status", "--porcelain"]).is_empty());
    assert!(!a);
}

#[test]
fn gix_is_clean_modified_matches_real_git() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nmod\n").unwrap();
    let a = Git::open(&root).unwrap().is_clean().unwrap();
    assert_eq!(a, git_out(&root, &["status", "--porcelain"]).is_empty());
    assert!(!a);
}

#[test]
fn gix_is_clean_staged_matches_real_git() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("s.txt"), "s").unwrap();
    git(&root, &["add", "s.txt"]);
    let a = Git::open(&root).unwrap().is_clean().unwrap();
    assert_eq!(a, git_out(&root, &["status", "--porcelain"]).is_empty());
    assert!(!a);
}

#[test]
fn gix_is_clean_ignored_matches_real_git() {
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ig"]);
    std::fs::write(root.join("z.log"), "z").unwrap();
    let a = Git::open(&root).unwrap().is_clean().unwrap();
    assert_eq!(a, git_out(&root, &["status", "--porcelain"]).is_empty());
    assert!(a, "ignored => clean");
}

/// Parse `git worktree list --porcelain` into the sorted set of branch names
/// (with detached entries as None) — the real-git oracle for branch-set checks.
fn oracle_branch_set(root: &Path) -> Vec<Option<String>> {
    let porcelain = git_out(root, &["worktree", "list", "--porcelain"]);
    let mut set: Vec<Option<String>> = Vec::new();
    let mut current: Option<String> = None;
    let mut started = false;
    for line in porcelain.lines() {
        if line.starts_with("worktree ") {
            if started {
                set.push(current.take());
            }
            current = None;
            started = true;
        } else if let Some(r) = line.strip_prefix("branch ") {
            current = Some(r.trim_start_matches("refs/heads/").to_string());
        }
    }
    if started {
        set.push(current);
    }
    set.sort();
    set
}

#[test]
fn gix_branch_set_matches_real_git() {
    let (_d, root) = init_repo();
    let setup = Git::open(&root).unwrap();
    let path = sibling(&root, "bset");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("agree-set".to_string()),
    };
    setup.create_worktree("bset", &path, &opts).unwrap();

    let mut a: Vec<Option<String>> = Git::open(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();
    a.sort();
    assert_eq!(a, oracle_branch_set(&root), "gix branch set matches real git");
    assert!(a.contains(&Some("agree-set".to_string())));
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

#[test]
fn gix_branch_set_with_detached_worktree_matches_real_git() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    let path = sibling(&root, "detset");
    git(&root, &["worktree", "add", "--detach", &path.to_string_lossy(), &head]);

    let mut a: Vec<Option<String>> = Git::open(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();
    a.sort();
    assert_eq!(a, oracle_branch_set(&root), "branch set matches incl. detached");
    // Exactly one detached entry (None) is expected across the set.
    assert!(a.iter().any(|x| x.is_none()), "a detached entry exists");
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

#[test]
fn gix_locked_count_matches_real_git() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "lkcnt");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("lk".to_string()),
    };
    let info = g.create_worktree("lkcnt", &path, &opts).unwrap();
    git(&root, &["worktree", "lock", &info.path.to_string_lossy()]);

    let a = Git::open(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .iter()
        .filter(|w| w.locked)
        .count();
    // Oracle: count `locked` lines in git's porcelain worktree listing.
    let oracle_locked = git_out(&root, &["worktree", "list", "--porcelain"])
        .lines()
        .filter(|l| *l == "locked" || l.starts_with("locked "))
        .count();
    assert_eq!(a, oracle_locked, "gix locked count matches real git");
    assert_eq!(a, 1, "exactly one locked worktree");

    git(&root, &["worktree", "unlock", &info.path.to_string_lossy()]);
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

// ===========================================================================
// Worktree-local status — isolation between checkouts
// ===========================================================================

#[test]
fn worktree_branch_visible_from_inside() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "inside");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("inside-branch".to_string()),
    };
    let info = g.create_worktree("inside", &path, &opts).unwrap();
    for (label, open) in backends() {
        let inner = open(&info.path).unwrap_or_else(|_| Git::open(&info.path).unwrap());
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some("inside-branch"),
            "[{label}] worktree reports its own branch"
        );
    }
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

#[test]
fn worktree_dirty_does_not_affect_primary() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "iso");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("iso-branch".to_string()),
    };
    let info = g.create_worktree("iso", &path, &opts).unwrap();
    std::fs::write(info.path.join("wip.txt"), "scratch").unwrap();

    let inner = Git::open(&info.path).unwrap();
    assert!(!inner.is_clean().unwrap(), "worktree dirty");
    assert!(g.is_clean().unwrap(), "primary stays clean");

    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

#[test]
fn primary_dirty_does_not_affect_worktree() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "iso2");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("iso2-branch".to_string()),
    };
    let info = g.create_worktree("iso2", &path, &opts).unwrap();

    // Dirty the primary only.
    std::fs::write(root.join("primary-wip.txt"), "x").unwrap();
    let inner = Git::open(&info.path).unwrap();
    assert!(!g.is_clean().unwrap(), "primary dirty");
    assert!(inner.is_clean().unwrap(), "worktree stays clean");

    std::fs::remove_file(root.join("primary-wip.txt")).unwrap();
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

#[test]
fn worktree_on_detached_head_reports_none_from_inside() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    let path = sibling(&root, "detinside");
    git(&root, &["worktree", "add", "--detach", &path.to_string_lossy(), &head]);
    let inner = Git::open(&path).unwrap();
    assert_eq!(
        inner.current_branch().unwrap(),
        None,
        "detached worktree reports None from inside"
    );
    git(&root, &["worktree", "remove", "--force", &path.to_string_lossy()]);
}

// ===========================================================================
// Remotes — arranged via plain git; status surface still reads correctly
// ===========================================================================
//
// The crate has no public remote reader, so these assert that configuring an
// origin (github/gitlab/ssh scp-like/https/subgroups/.git-suffix) does NOT
// perturb the status surface the engine relies on. The origin is read back via
// plain `git` to prove the fixture is well-formed, and the public API is
// checked to stay correct in its presence.

/// Add an `origin` remote pointing at `url`.
fn set_origin(root: &Path, url: &str) {
    git(root, &["remote", "add", "origin", url]);
}

#[test]
fn origin_https_github_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "https://github.com/darkrun/factory.git");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "https://github.com/darkrun/factory.git"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
        assert!(g.is_clean().unwrap(), "[{label}] clean with origin set");
    }
}

#[test]
fn origin_ssh_scplike_github_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "git@github.com:darkrun/factory.git");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "git@github.com:darkrun/factory.git"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
        assert!(g.is_clean().unwrap(), "[{label}]");
    }
}

#[test]
fn origin_https_gitlab_subgroups_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "https://gitlab.com/group/subgroup/repo.git");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "https://gitlab.com/group/subgroup/repo.git"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
        assert!(g.is_clean().unwrap(), "[{label}]");
    }
}

#[test]
fn origin_ssh_gitlab_deep_subgroups_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "git@gitlab.com:org/team/sub/repo.git");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "git@gitlab.com:org/team/sub/repo.git"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}]");
    }
}

#[test]
fn origin_without_git_suffix_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "https://github.com/darkrun/factory");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "https://github.com/darkrun/factory"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
    }
}

#[test]
fn origin_with_port_does_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "ssh://git@github.com:2222/darkrun/factory.git");
    assert_eq!(
        git_out(&root, &["remote", "get-url", "origin"]),
        "ssh://git@github.com:2222/darkrun/factory.git"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}]");
    }
}

#[test]
fn missing_origin_does_not_break_status() {
    // No remote at all is the common case for fresh local Runs.
    let (_d, root) = init_repo();
    assert!(
        git_out(&root, &["remote"]).is_empty(),
        "no remotes configured"
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"), "[{label}]");
        assert!(g.is_clean().unwrap(), "[{label}] clean without any origin");
    }
}

#[test]
fn multiple_remotes_do_not_break_status() {
    let (_d, root) = init_repo();
    set_origin(&root, "https://github.com/darkrun/factory.git");
    git(&root, &["remote", "add", "upstream", "git@gitlab.com:darkrun/factory.git"]);
    let remotes = git_out(&root, &["remote"]);
    assert!(remotes.contains("origin"), "origin present");
    assert!(remotes.contains("upstream"), "upstream present");
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean with two remotes");
    }
}

#[test]
fn adding_origin_keeps_repo_clean() {
    // Adding a remote writes to .git/config, not the working tree — status
    // must stay clean.
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean before remote");
    }
    set_origin(&root, "https://github.com/darkrun/factory.git");
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] still clean after adding origin");
    }
}

// ===========================================================================
// Open / error surface
// ===========================================================================

#[test]
fn open_reports_repo_root() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(g.repo_root(), root.as_path(), "[{label}] repo_root");
    }
}

#[test]
fn open_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    for (label, open) in backends() {
        match open(dir.path()) {
            Err(GitError::NotARepo(_)) => {}
            Err(other) => panic!("[{label}] expected NotARepo, got {other:?}"),
            Ok(_) => panic!("[{label}] expected NotARepo error, got Ok"),
        }
    }
}

#[test]
fn open_rejects_empty_dir() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("not-a-repo");
    std::fs::create_dir_all(&nested).unwrap();
    for (label, open) in backends() {
        assert!(open(&nested).is_err(), "[{label}] empty dir is not a repo");
    }
}

#[test]
fn gix_discovers_from_subdirectory() {
    let (_d, root) = init_repo();
    let nested = root.join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    let g = Git::open(&nested).expect("discover upward");
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
}

#[test]
fn not_a_repo_error_carries_path() {
    let dir = TempDir::new().unwrap();
    match Git::open(dir.path()) {
        Err(GitError::NotARepo(p)) => assert_eq!(p, dir.path()),
        Err(other) => panic!("expected NotARepo(path), got {other:?}"),
        Ok(_) => panic!("expected NotARepo error, got Ok"),
    }
}

#[test]
fn not_a_repo_error_displays_path() {
    let dir = TempDir::new().unwrap();
    if let Err(e) = Git::open(dir.path()) {
        let msg = e.to_string();
        assert!(msg.contains("not a git repository"), "message: {msg}");
    } else {
        panic!("expected error");
    }
}

#[test]
fn remove_missing_worktree_errors() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let err = g
            .remove_worktree("nope-not-here", false)
            .expect_err("removing unknown worktree must error");
        assert!(
            matches!(err, GitError::WorktreeNotFound(_)),
            "[{label}] expected WorktreeNotFound, got {err:?}"
        );
    }
}

#[test]
fn worktree_not_found_error_carries_name() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    match g.remove_worktree("phantom-station", false) {
        Err(GitError::WorktreeNotFound(n)) => assert_eq!(n, "phantom-station"),
        other => panic!("expected WorktreeNotFound(name), got {other:?}"),
    }
}

#[test]
fn worktree_not_found_error_displays_name() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    if let Err(e) = g.remove_worktree("ghost", false) {
        assert!(e.to_string().contains("ghost"), "message: {}", e);
    } else {
        panic!("expected error");
    }
}

#[test]
fn create_duplicate_worktree_name_errors_gix() {
    // gix rejects creating a second worktree with an existing name.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let p1 = sibling(&root, "dup1");
    let p2 = sibling(&root, "dup2");
    let opts1 = CreateOptions {
        reference: None,
        new_branch: Some("dup-a".to_string()),
    };
    let info = g.create_worktree("dup", &p1, &opts1).unwrap();
    let opts2 = CreateOptions {
        reference: None,
        new_branch: Some("dup-b".to_string()),
    };
    let err = g.create_worktree("dup", &p2, &opts2).expect_err("duplicate name rejected");
    assert!(
        matches!(err, GitError::WorktreeExists(_)),
        "expected WorktreeExists, got {err:?}"
    );
    git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
}

// ===========================================================================
// Determinism across many state mutations
// ===========================================================================

#[test]
fn status_deterministic_through_mutation_sequence() {
    // Drive a scripted clean/dirty sequence and assert the status query tracks
    // it exactly at every step, for both backends.
    let (_d, root) = init_repo();
    let steps: &[(&str, bool)] = &[
        // (action, expected_clean_after)
        ("start", true),
        ("touch", false),
        ("remove", true),
        ("modify", false),
        ("commit", true),
        ("stage", false),
        ("commit", true),
    ];
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let scratch = root.join(format!("seq-{label}.txt"));
        for (action, expect_clean) in steps {
            match *action {
                "start" => {}
                "touch" => std::fs::write(&scratch, "x").unwrap(),
                "remove" => std::fs::remove_file(&scratch).unwrap(),
                "modify" => std::fs::write(root.join("README.md"), format!("# fixture\n{label}\n")).unwrap(),
                "commit" => git(&root, &["commit", "-aqm", "step"]),
                "stage" => {
                    std::fs::write(&scratch, "y").unwrap();
                    git(&root, &["add", &scratch.to_string_lossy()]);
                }
                _ => unreachable!(),
            }
            assert_eq!(
                g.is_clean().unwrap(),
                *expect_clean,
                "[{label}] after '{action}' expected clean={expect_clean}"
            );
        }
    }
}

#[test]
fn branch_tracks_through_checkout_sequence() {
    let (_d, root) = init_repo();
    let branches = ["main", "spec", "review", "manufacture", "audit", "main"];
    // Pre-create the named branches.
    for b in &branches {
        if *b != "main" {
            git(&root, &["branch", b]);
        }
    }
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        for b in &branches {
            git(&root, &["checkout", "-q", b]);
            assert_eq!(
                g.current_branch().unwrap().as_deref(),
                Some(*b),
                "[{label}] after checkout {b}"
            );
        }
    }
}

// ===========================================================================
// WorktreeInfo struct semantics
// ===========================================================================

#[test]
fn worktree_info_clone_eq() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let list = g.list_worktrees().unwrap();
    let first = &list[0];
    assert_eq!(first.clone(), *first, "WorktreeInfo is Clone + PartialEq");
}

#[test]
fn worktree_info_debug_nonempty() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let list = g.list_worktrees().unwrap();
    let dbg = format!("{:?}", list[0]);
    assert!(dbg.contains("WorktreeInfo"), "Debug renders the type name: {dbg}");
}

#[test]
fn create_options_default_is_empty() {
    let opts = CreateOptions::default();
    assert!(opts.reference.is_none(), "default reference is None");
    assert!(opts.new_branch.is_none(), "default new_branch is None");
}

#[test]
fn create_options_clone() {
    let opts = CreateOptions {
        reference: Some("base".to_string()),
        new_branch: Some("nb".to_string()),
    };
    let c = opts.clone();
    assert_eq!(c.reference.as_deref(), Some("base"));
    assert_eq!(c.new_branch.as_deref(), Some("nb"));
}

#[test]
fn primary_worktree_branch_matches_current_branch() {
    // The branch reported for the primary entry in list_worktrees must equal
    // what current_branch() reports.
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "consistency"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let cur = g.current_branch().unwrap();
        let listed_has = g
            .list_worktrees()
            .unwrap()
            .iter()
            .any(|w| w.branch == cur);
        assert!(listed_has, "[{label}] current branch appears in listing");
    }
}

// ===========================================================================
// Host / owner / repo derivation — fixture-shape verification
// ===========================================================================
//
// The crate exposes no URL parser, so we verify the *fixture* round-trips
// through git's own remote storage and that a reference derivation (the shape a
// future parser would produce) is well-defined for each URL form. This pins the
// inventory of URL shapes the engine must eventually handle (github/gitlab, ssh
// scp-like, https, subgroups, .git suffix, ports) and proves each is a valid
// origin that leaves the status surface intact.

/// Reference derivation of (host, owner, repo) from a remote URL, mirroring the
/// canonical parse the Factory expects. Lives in the test so we can assert the
/// inventory of supported shapes without depending on a not-yet-public parser.
fn derive(url: &str) -> Option<(String, String, String)> {
    // Strip a trailing slash, then a `.git` suffix.
    let u = url.trim_end_matches('/');
    let u = u.strip_suffix(".git").unwrap_or(u);

    let (host, path) = if let Some(rest) = u.strip_prefix("git@") {
        // scp-like: git@host:owner/repo
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    } else if let Some(rest) = u
        .strip_prefix("ssh://")
        .or_else(|| u.strip_prefix("https://"))
        .or_else(|| u.strip_prefix("http://"))
    {
        // Drop optional user@ then split host[:port]/path.
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (hostport, path) = rest.split_once('/')?;
        let host = hostport.split_once(':').map(|(h, _)| h).unwrap_or(hostport);
        (host.to_string(), path.to_string())
    } else {
        return None;
    };

    // owner is everything up to the last segment; repo is the final segment.
    let (owner, repo) = path.rsplit_once('/')?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((host, owner.to_string(), repo.to_string()))
}

#[test]
fn derive_https_github_with_git_suffix() {
    assert_eq!(
        derive("https://github.com/darkrun/factory.git"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_https_github_without_git_suffix() {
    assert_eq!(
        derive("https://github.com/darkrun/factory"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_ssh_scplike_github() {
    assert_eq!(
        derive("git@github.com:darkrun/factory.git"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_ssh_scplike_without_suffix() {
    assert_eq!(
        derive("git@github.com:darkrun/factory"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_gitlab_subgroup_owner_is_path() {
    // GitLab subgroups: owner spans every path segment but the last.
    assert_eq!(
        derive("https://gitlab.com/group/subgroup/repo.git"),
        Some(("gitlab.com".into(), "group/subgroup".into(), "repo".into()))
    );
}

#[test]
fn derive_gitlab_deep_subgroup_ssh() {
    assert_eq!(
        derive("git@gitlab.com:org/team/sub/repo.git"),
        Some(("gitlab.com".into(), "org/team/sub".into(), "repo".into()))
    );
}

#[test]
fn derive_ssh_url_with_port() {
    assert_eq!(
        derive("ssh://git@github.com:2222/darkrun/factory.git"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_https_with_userinfo() {
    assert_eq!(
        derive("https://user@gitlab.com/group/repo.git"),
        Some(("gitlab.com".into(), "group".into(), "repo".into()))
    );
}

#[test]
fn derive_trailing_slash_tolerated() {
    assert_eq!(
        derive("https://github.com/darkrun/factory/"),
        Some(("github.com".into(), "darkrun".into(), "factory".into()))
    );
}

#[test]
fn derive_rejects_missing_owner() {
    // No owner segment — only host and one path part.
    assert_eq!(derive("https://github.com/factory.git"), None);
}

#[test]
fn derive_rejects_unrecognized_scheme() {
    assert_eq!(derive("ftp://example.com/a/b"), None);
}

#[test]
fn derive_rejects_bare_host() {
    assert_eq!(derive("https://github.com/"), None);
}

#[test]
fn derive_is_deterministic() {
    let url = "git@gitlab.com:org/team/sub/repo.git";
    let a = derive(url);
    let b = derive(url);
    assert_eq!(a, b, "derivation is pure");
    assert!(a.is_some());
}

#[test]
fn derive_matches_git_stored_url_roundtrip() {
    // Store each URL via git, read it back, and confirm derivation is stable on
    // the value git actually persists.
    let (_d, root) = init_repo();
    let urls = [
        "https://github.com/darkrun/factory.git",
        "git@github.com:darkrun/factory.git",
        "https://gitlab.com/group/subgroup/repo.git",
        "ssh://git@github.com:2222/darkrun/factory.git",
    ];
    for url in urls {
        // Remove any prior origin without asserting (it may not exist yet).
        let _ = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["remote", "remove", "origin"])
            .status();
        set_origin(&root, url);
        let stored = git_out(&root, &["remote", "get-url", "origin"]);
        assert_eq!(stored, url, "git stores the URL verbatim");
        assert!(derive(&stored).is_some(), "derivable from stored: {stored}");
    }
}

// ===========================================================================
// Additional dirty-detection edge cases
// ===========================================================================

#[test]
fn is_clean_executable_bit_change_is_dirty() {
    // Changing a tracked file's mode (when the platform records it) is a change.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (_d, root) = init_repo();
        // Track a script first.
        std::fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
        git(&root, &["add", "run.sh"]);
        git(&root, &["commit", "-qm", "add script"]);
        let mut perms = std::fs::metadata(root.join("run.sh")).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(root.join("run.sh"), perms).unwrap();
        for (label, open) in backends() {
            let g = open(&root).unwrap();
            assert!(!g.is_clean().unwrap(), "[{label}] mode change => dirty");
        }
    }
}

#[test]
fn is_clean_symlink_untracked_is_dirty() {
    #[cfg(unix)]
    {
        let (_d, root) = init_repo();
        std::os::unix::fs::symlink("README.md", root.join("link")).unwrap();
        for (label, open) in backends() {
            let g = open(&root).unwrap();
            assert!(!g.is_clean().unwrap(), "[{label}] untracked symlink => dirty");
        }
    }
}

#[test]
fn is_clean_renamed_tracked_file_is_dirty() {
    let (_d, root) = init_repo();
    std::fs::rename(root.join("README.md"), root.join("RENAMED.md")).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] rename shows as delete+untracked => dirty"
        );
    }
}

#[test]
fn is_clean_binary_file_untracked_is_dirty() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("blob.bin"), [0u8, 159, 146, 150, 255, 0, 1, 2]).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] untracked binary => dirty");
    }
}

#[test]
fn is_clean_truncating_tracked_file_is_dirty() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] truncation => dirty");
    }
}

#[test]
fn is_clean_after_revert_to_committed_content() {
    // Modify then restore exact committed bytes — back to clean.
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nedit\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] mid-edit dirty");
    }
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] restored content => clean");
    }
}

// ===========================================================================
// create_worktree status/branch reporting variations
// ===========================================================================

#[test]
fn create_from_existing_branch_attaches() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let ref_branch = format!("release/{label}");
        git(&root, &["branch", &ref_branch]);
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("attach-{label}"));
        let opts = CreateOptions {
            reference: Some(ref_branch.clone()),
            new_branch: None,
        };
        let info = g.create_worktree("rel", &path, &opts).unwrap();
        let inner = Git::open(&info.path).unwrap();
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some(ref_branch.as_str()),
            "[{label}] attached to existing branch"
        );
        git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
    }
}

#[test]
fn create_new_branch_off_reference_commit() {
    let (_d, root) = init_repo();
    let base = git_out(&root, &["rev-parse", "HEAD"]);
    advance(&root, "v2.txt", "v2", "v2");
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("offref-{label}"));
        let opts = CreateOptions {
            reference: Some(base.clone()),
            new_branch: Some(format!("topic-{label}")),
        };
        let info = g.create_worktree("topic", &path, &opts).unwrap();
        let wt_head = git_out(&info.path, &["rev-parse", "HEAD"]);
        assert_eq!(wt_head, base, "[{label}] new branch forked from the base commit");
        git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
    }
}

#[test]
fn freshly_created_worktree_is_clean() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("freshclean-{label}"));
        let opts = CreateOptions {
            reference: None,
            new_branch: Some(format!("fresh-{label}")),
        };
        let info = g.create_worktree("fresh", &path, &opts).unwrap();
        let inner = Git::open(&info.path).unwrap();
        assert!(inner.is_clean().unwrap(), "[{label}] new worktree starts clean");
        git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
    }
}

#[test]
fn derive_ssh_https_agree_on_owner_repo() {
    // The same repo addressed two ways must derive identical owner/repo.
    let https = derive("https://github.com/darkrun/factory.git").unwrap();
    let ssh = derive("git@github.com:darkrun/factory.git").unwrap();
    assert_eq!(https.1, ssh.1, "owner agrees across URL forms");
    assert_eq!(https.2, ssh.2, "repo agrees across URL forms");
    assert_eq!(https.0, ssh.0, "host agrees across URL forms");
}

#[test]
fn derive_repo_name_strips_only_one_git_suffix() {
    // A repo literally named "foo.git" stored as "foo.git.git" keeps the inner.
    let (h, o, r) = derive("https://github.com/owner/foo.git.git").unwrap();
    assert_eq!(h, "github.com");
    assert_eq!(o, "owner");
    assert_eq!(r, "foo.git", "only the trailing .git is stripped");
}

#[test]
fn derive_host_is_lowercased_input_preserved() {
    // We don't normalise case; the host comes through as stored.
    let (h, _, _) = derive("https://GitHub.com/owner/repo.git").unwrap();
    assert_eq!(h, "GitHub.com", "host preserved verbatim (no implicit casefold)");
}

#[test]
fn detached_then_branch_status_clean_throughout() {
    // Moving between detached and attached HEAD must not perturb cleanliness.
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean on main");
        git(&root, &["checkout", "-q", &head]);
        assert!(g.is_clean().unwrap(), "[{label}] clean when detached");
        assert_eq!(g.current_branch().unwrap(), None, "[{label}] detached");
        git(&root, &["checkout", "-q", "main"]);
        assert!(g.is_clean().unwrap(), "[{label}] clean after reattach");
    }
}

/// The pure-Rust gitoxide backend reports an unborn HEAD's branch by NAME (like
/// `git branch --show-current` does), not None — the faithful behavior matching
/// the real-git oracle on a commit-less repository.
#[test]
fn empty_repo_gix_unborn_head_is_the_branch_name() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    let g = Git::open(&root).unwrap();
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
    // The oracle agrees: real git reports the unborn branch by name too.
    assert_eq!(git_out(&root, &["branch", "--show-current"]), "main");
}

#[test]
fn empty_repo_is_clean() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] empty repo is clean");
    }
}

#[test]
fn empty_repo_untracked_is_dirty() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("first.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] untracked in empty repo => dirty");
    }
}

#[test]
fn list_after_remove_does_not_show_stale_entry() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let path = sibling(&root, "stale");
    let opts = CreateOptions {
        reference: None,
        new_branch: Some("stale-branch".to_string()),
    };
    let info = g.create_worktree("stale", &path, &opts).unwrap();
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("stale-branch"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert!(
            !list.iter().any(|w| w.branch.as_deref() == Some("stale-branch")),
            "[{label}] removed branch gone from listing: {list:?}"
        );
    }
    assert!(!info.path.exists());
}

#[test]
fn created_worktree_info_reports_branch() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("inforep-{label}"));
        let opts = CreateOptions {
            reference: None,
            new_branch: Some(format!("rep-{label}")),
        };
        let info = g.create_worktree("rep", &path, &opts).unwrap();
        assert_eq!(
            info.branch.as_deref(),
            Some(format!("rep-{label}").as_str()),
            "[{label}] WorktreeInfo.branch reflects the new branch"
        );
        assert!(!info.locked, "[{label}] new worktree not locked");
        git(&root, &["worktree", "remove", "--force", &info.path.to_string_lossy()]);
    }
}

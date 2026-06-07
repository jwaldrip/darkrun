//! Comprehensive integration tests for darkrun-git worktree primitives.
//!
//! Every test builds a throwaway repository in a `TempDir` and drives the
//! public [`Git`] facade (and, where it matters, the concrete
//! [`Libgit2Backend`]/[`ShellBackend`] constructors). The same scenarios run
//! against both backends so the two implementations stay in lock-step: the
//! manager must observe identical behaviour regardless of which backend a given
//! environment selects.
//!
//! Phases covered: spec (option/info shapes), review (open/discover), manufacture
//! (create_worktree), audit (list/remove), tests (current_branch/is_clean), and
//! checkpoint (idempotency/determinism).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use darkrun_git::{
    CreateOptions, Git, GitBackend, GitError, Libgit2Backend, ShellBackend, WorktreeInfo,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Run `git <args>` in `root`, asserting success. Used only by the test harness
/// to arrange repository state; the code under test never shells out here.
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
    git(&root, &["config", "user.email", "test@darkrun.local"]);
    git(&root, &["config", "user.name", "darkrun test"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("README.md"), "# fixture\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "init"]);
    (dir, root)
}

/// Initialise a repo and advance it with a second commit, returning the first
/// commit's full SHA as a stable base revision.
fn init_repo_two_commits() -> (TempDir, PathBuf, String) {
    let (dir, root) = init_repo();
    let base = git_out(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("README.md"), "# fixture\nv2\n").unwrap();
    git(&root, &["commit", "-aqm", "v2"]);
    (dir, root, base)
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A unique sibling path next to `root` so worktrees never nest inside the repo.
fn sibling(root: &Path, label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    root.parent()
        .unwrap()
        .join(format!("dr-wt-{label}-{nanos}-{seq}"))
}

/// A named backend constructor: a label and a function opening a repo root.
type NamedBackend = (&'static str, fn(&Path) -> darkrun_git::Result<Git>);

/// The two backends under test, as named constructors over a repo root.
fn backends() -> Vec<NamedBackend> {
    vec![
        ("libgit2", |p| Git::open(p)),
        ("shell", |p| Git::open_shell(p)),
        // The pure-Rust gitoxide backend runs the full LOCAL conformance matrix
        // (network ops live in the network-only list below until those phases).
        ("gix", |p| Git::open_gix(p)),
    ]
}

/// Backends that implement the network/rebase ops (push/fetch/rebase). The gix
/// backend builds those out in later phases, so it's excluded here for now.
fn backends_with_network() -> Vec<NamedBackend> {
    vec![
        ("libgit2", |p| Git::open(p)),
        ("shell", |p| Git::open_shell(p)),
    ]
}

/// Create a new-branch worktree and return the info, panicking with a labelled
/// message on failure.
fn make_branch_worktree(g: &Git, label: &str, name: &str, branch: &str) -> WorktreeInfo {
    let path = sibling(g.repo_root(), label);
    let opts = CreateOptions {
        reference: None,
        new_branch: Some(branch.to_string()),
    };
    g.create_worktree(name, &path, &opts)
        .unwrap_or_else(|e| panic!("[{label}] create_worktree({name}, {branch}): {e}"))
}

/// Remove a worktree by its on-disk path through plain git, ignoring failures
/// (used purely for cleanup so leftover dirs don't pile up between tests).
fn cleanup(root: &Path, path: &Path) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["worktree", "remove", "--force", &path.to_string_lossy()])
        .status();
    let _ = std::fs::remove_dir_all(path);
}

// ===========================================================================
// SPEC PHASE — option and info data shapes
// ===========================================================================

#[test]
fn create_options_default_is_all_none() {
    let opts = CreateOptions::default();
    assert!(opts.reference.is_none(), "default reference is None");
    assert!(opts.new_branch.is_none(), "default new_branch is None");
}

#[test]
fn create_options_clone_is_equal_field_for_field() {
    let opts = CreateOptions {
        reference: Some("main".into()),
        new_branch: Some("topic".into()),
    };
    let cloned = opts.clone();
    assert_eq!(cloned.reference, opts.reference);
    assert_eq!(cloned.new_branch, opts.new_branch);
}

#[test]
fn create_options_debug_mentions_fields() {
    let opts = CreateOptions {
        reference: Some("base-rev".into()),
        new_branch: Some("new-br".into()),
    };
    let dbg = format!("{opts:?}");
    assert!(dbg.contains("base-rev"), "debug shows reference: {dbg}");
    assert!(dbg.contains("new-br"), "debug shows new_branch: {dbg}");
}

#[test]
fn worktree_info_equality_is_structural() {
    let a = WorktreeInfo {
        name: "w".into(),
        path: PathBuf::from("/tmp/w"),
        branch: Some("main".into()),
        locked: false,
    };
    let b = a.clone();
    assert_eq!(a, b, "identical fields => equal");
    let c = WorktreeInfo {
        branch: Some("other".into()),
        ..a.clone()
    };
    assert_ne!(a, c, "differing branch => not equal");
}

#[test]
fn worktree_info_locked_flag_differentiates() {
    let base = WorktreeInfo {
        name: "w".into(),
        path: PathBuf::from("/tmp/w"),
        branch: None,
        locked: false,
    };
    let locked = WorktreeInfo {
        locked: true,
        ..base.clone()
    };
    assert_ne!(base, locked, "locked flag participates in equality");
}

#[test]
fn worktree_info_debug_includes_name_and_path() {
    let info = WorktreeInfo {
        name: "station-frame".into(),
        path: PathBuf::from("/tmp/station-frame"),
        branch: Some("station/frame".into()),
        locked: true,
    };
    let dbg = format!("{info:?}");
    assert!(dbg.contains("station-frame"), "debug shows name: {dbg}");
    assert!(dbg.contains("station/frame"), "debug shows branch: {dbg}");
}

// ===========================================================================
// REVIEW PHASE — open / discover
// ===========================================================================

#[test]
fn open_reports_repo_root() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap_or_else(|e| panic!("[{label}] open: {e}"));
        assert_eq!(g.repo_root(), root.as_path(), "[{label}] repo_root");
    }
}

#[test]
fn open_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    for (label, open) in backends() {
        match open(dir.path()) {
            Err(GitError::NotARepo(p)) => {
                assert_eq!(p, dir.path(), "[{label}] error carries the rejected path");
            }
            Err(other) => panic!("[{label}] expected NotARepo, got {other:?}"),
            Ok(_) => panic!("[{label}] expected NotARepo error, got Ok"),
        }
    }
}

#[test]
fn open_rejects_nonexistent_path() {
    let missing = std::env::temp_dir().join(format!("dr-missing-{}", SEQ.fetch_add(1, Ordering::Relaxed)));
    for (label, open) in backends() {
        match open(&missing) {
            Err(GitError::NotARepo(_)) => {}
            Err(other) => panic!("[{label}] expected NotARepo for missing path, got {other:?}"),
            Ok(_) => panic!("[{label}] expected NotARepo for missing path, got Ok"),
        }
    }
}

#[test]
fn open_libgit2_discovers_from_subdirectory() {
    // Opening a nested path should walk up to the enclosing repo.
    let (_d, root) = init_repo();
    let nested = root.join("a").join("b");
    std::fs::create_dir_all(&nested).unwrap();
    let g = Git::open(&nested).expect("discover from subdir");
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
}

#[test]
fn open_libgit2_discovers_from_deeply_nested_subdirectory() {
    let (_d, root) = init_repo();
    let nested = root.join("x").join("y").join("z").join("deep");
    std::fs::create_dir_all(&nested).unwrap();
    let g = Libgit2Backend::open(&nested).expect("discover deep");
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
}

#[test]
fn libgit2_backend_open_reports_root() {
    let (_d, root) = init_repo();
    let b = Libgit2Backend::open(&root).unwrap();
    assert_eq!(b.repo_root(), root.as_path());
}

#[test]
fn shell_backend_open_reports_root() {
    let (_d, root) = init_repo();
    let b = ShellBackend::open(&root).unwrap();
    assert_eq!(b.repo_root(), root.as_path());
}

#[test]
fn libgit2_backend_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        Libgit2Backend::open(dir.path()),
        Err(GitError::NotARepo(_))
    ));
}

#[test]
fn shell_backend_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        ShellBackend::open(dir.path()),
        Err(GitError::NotARepo(_))
    ));
}

#[test]
fn open_can_be_called_repeatedly_on_same_repo() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let a = open(&root).unwrap();
        let b = open(&root).unwrap();
        assert_eq!(a.repo_root(), b.repo_root(), "[{label}] stable root");
        assert_eq!(
            a.current_branch().unwrap(),
            b.current_branch().unwrap(),
            "[{label}] two handles agree"
        );
    }
}

// ===========================================================================
// TESTS PHASE — current_branch
// ===========================================================================

#[test]
fn current_branch_reads_head() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("main"),
            "[{label}] branch"
        );
    }
}

#[test]
fn current_branch_tracks_checkout() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "feature/x"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("feature/x"),
            "[{label}] branch after checkout"
        );
    }
}

#[test]
fn current_branch_handles_nested_branch_name() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "a/b/c/deep"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("a/b/c/deep"),
            "[{label}] nested branch shorthand"
        );
    }
}

#[test]
fn current_branch_none_when_detached() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap(),
            None,
            "[{label}] detached HEAD => None"
        );
    }
}

#[test]
fn current_branch_after_switching_back_to_main() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "temp"]);
    git(&root, &["checkout", "-q", "main"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("main"),
            "[{label}] switched back to main"
        );
    }
}

#[test]
fn current_branch_is_idempotent() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let first = g.current_branch().unwrap();
        let second = g.current_branch().unwrap();
        let third = g.current_branch().unwrap();
        assert_eq!(first, second, "[{label}] repeated reads stable");
        assert_eq!(second, third, "[{label}] repeated reads stable");
    }
}

#[test]
fn current_branch_default_branch_name_respected() {
    // A repo initialized on a differently-named default branch reports it.
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "trunk"]);
    git(&root, &["config", "user.email", "t@d.local"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("f"), "x").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "i"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("trunk"),
            "[{label}] custom default branch"
        );
    }
}

#[test]
fn backends_agree_on_current_branch() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "agreement/branch"]);
    let lib = Git::open(&root).unwrap().current_branch().unwrap();
    let sh = Git::open_shell(&root).unwrap().current_branch().unwrap();
    assert_eq!(lib, sh, "both backends report the same branch");
}

#[test]
fn backends_agree_on_detached_branch() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    let lib = Git::open(&root).unwrap().current_branch().unwrap();
    let sh = Git::open_shell(&root).unwrap().current_branch().unwrap();
    assert_eq!(lib, None, "libgit2 detached => None");
    assert_eq!(sh, None, "shell detached => None");
}

// ===========================================================================
// TESTS PHASE — is_clean
// ===========================================================================

#[test]
fn is_clean_on_fresh_repo() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] fresh repo clean");
    }
}

#[test]
fn is_clean_detects_untracked() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("scratch.txt"), "wip").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] untracked => dirty");
    }
    std::fs::remove_file(root.join("scratch.txt")).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean after removal");
    }
}

#[test]
fn is_clean_detects_modified() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nmodified\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] modified tracked => dirty");
    }
}

#[test]
fn is_clean_detects_staged_add() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("new.txt"), "content").unwrap();
    git(&root, &["add", "new.txt"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] staged add => dirty");
    }
}

#[test]
fn is_clean_detects_staged_modification() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nchanged\n").unwrap();
    git(&root, &["add", "README.md"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] staged modify => dirty");
    }
}

#[test]
fn is_clean_detects_deleted_tracked_file() {
    let (_d, root) = init_repo();
    std::fs::remove_file(root.join("README.md")).unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] deleted tracked => dirty");
    }
}

#[test]
fn is_clean_detects_staged_deletion() {
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
    std::fs::write(root.join("added.txt"), "new").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "add file"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] committed => clean again");
    }
}

#[test]
fn is_clean_ignores_gitignored() {
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-q", "-m", "add ignore"]);
    std::fs::create_dir_all(root.join("ignored")).unwrap();
    std::fs::write(root.join("ignored").join("junk.txt"), "junk").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            g.is_clean().unwrap(),
            "[{label}] gitignored content must not count as dirty"
        );
    }
}

#[test]
fn is_clean_ignored_single_file_pattern() {
    let (_d, root) = init_repo();
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    git(&root, &["add", ".gitignore"]);
    git(&root, &["commit", "-qm", "ignore logs"]);
    std::fs::write(root.join("debug.log"), "noise").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] ignored *.log => clean");
    }
    // But a non-ignored sibling does make it dirty.
    std::fs::write(root.join("debug.txt"), "noise").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] non-ignored => dirty");
    }
}

#[test]
fn is_clean_untracked_in_subdirectory() {
    let (_d, root) = init_repo();
    std::fs::create_dir_all(root.join("nested").join("deep")).unwrap();
    std::fs::write(root.join("nested").join("deep").join("f.txt"), "x").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(
            !g.is_clean().unwrap(),
            "[{label}] untracked nested file => dirty"
        );
    }
}

#[test]
fn is_clean_is_idempotent_and_nonmutating() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean #1");
        assert!(g.is_clean().unwrap(), "[{label}] clean #2");
        // The query itself must not dirty the tree.
        assert!(g.is_clean().unwrap(), "[{label}] clean #3");
        assert_eq!(
            git_out(&root, &["status", "--porcelain"]),
            "",
            "[{label}] is_clean left no trace"
        );
    }
}

#[test]
fn backends_agree_on_dirty_state() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("x.txt"), "dirty").unwrap();
    let lib = Git::open(&root).unwrap().is_clean().unwrap();
    let sh = Git::open_shell(&root).unwrap().is_clean().unwrap();
    assert_eq!(lib, sh, "both backends agree the tree is dirty");
    assert!(!lib, "tree is dirty");
}

#[test]
fn backends_agree_on_clean_state() {
    let (_d, root) = init_repo();
    let lib = Git::open(&root).unwrap().is_clean().unwrap();
    let sh = Git::open_shell(&root).unwrap().is_clean().unwrap();
    assert!(lib && sh, "both backends agree the tree is clean");
}

#[test]
fn is_clean_recovers_after_revert() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("README.md"), "# fixture\nmess\n").unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(!g.is_clean().unwrap(), "[{label}] dirty after edit");
    }
    git(&root, &["checkout", "--", "README.md"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] clean after checkout revert");
    }
}

// ===========================================================================
// AUDIT PHASE — list_worktrees
// ===========================================================================

#[test]
fn list_includes_primary_worktree() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some("main")),
            "[{label}] primary worktree on main should be listed: {list:?}"
        );
    }
}

#[test]
fn list_primary_only_has_one_entry_on_fresh_repo() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        assert_eq!(list.len(), 1, "[{label}] only the primary tree: {list:?}");
    }
}

#[test]
fn list_primary_path_is_repo_workdir() {
    let (_d, root) = init_repo();
    let canon_root = root.canonicalize().unwrap();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let primary = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("main"))
            .unwrap_or_else(|| panic!("[{label}] primary present"));
        let canon = primary.path.canonicalize().unwrap();
        assert_eq!(canon, canon_root, "[{label}] primary path == repo root");
    }
}

#[test]
fn list_is_idempotent() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    make_branch_worktree(&g, "idem", "idem", "idem/branch");
    let a = g.list_worktrees().unwrap();
    let b = g.list_worktrees().unwrap();
    assert_eq!(a, b, "list output stable across calls");
}

#[test]
fn list_grows_by_one_per_created_worktree() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let base = g.list_worktrees().unwrap().len();
    let w1 = make_branch_worktree(&g, "g1", "g1", "grow/1");
    assert_eq!(g.list_worktrees().unwrap().len(), base + 1);
    let w2 = make_branch_worktree(&g, "g2", "g2", "grow/2");
    assert_eq!(g.list_worktrees().unwrap().len(), base + 2);
    cleanup(&root, &w1.path);
    cleanup(&root, &w2.path);
}

#[test]
fn list_entries_have_absolute_paths() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let w = make_branch_worktree(&g, "abs", "abs", "abs/branch");
    let list = g.list_worktrees().unwrap();
    for entry in &list {
        assert!(
            entry.path.is_absolute(),
            "every worktree path should be absolute: {entry:?}"
        );
    }
    cleanup(&root, &w.path);
}

#[test]
fn list_reports_locked_worktree() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "locked", "locked", "locked-branch");
    git(&root, &["worktree", "lock", &info.path.to_string_lossy()]);

    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let entry = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("locked-branch"))
            .unwrap_or_else(|| panic!("[{label}] locked entry present: {list:?}"));
        assert!(entry.locked, "[{label}] locked flag set: {entry:?}");
    }

    git(&root, &["worktree", "unlock", &info.path.to_string_lossy()]);
    // After unlock both backends report it unlocked.
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let entry = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("locked-branch"))
            .unwrap();
        assert!(!entry.locked, "[{label}] unlocked flag cleared: {entry:?}");
    }
    cleanup(&root, &info.path);
}

#[test]
fn list_primary_worktree_is_never_locked() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let primary = list
            .iter()
            .find(|w| w.branch.as_deref() == Some("main"))
            .unwrap();
        assert!(!primary.locked, "[{label}] primary not locked");
    }
}

#[test]
fn list_reports_detached_worktree_branch_none() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    let path = sibling(&root, "detached");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--detach",
            &path.to_string_lossy(),
            &head,
        ],
    );
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let list = g.list_worktrees().unwrap();
        let entry = list
            .iter()
            .find(|w| w.path == path || w.path.ends_with("detached"))
            .or_else(|| {
                list.iter()
                    .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok())
            })
            .unwrap_or_else(|| panic!("[{label}] detached entry present: {list:?}"));
        assert_eq!(
            entry.branch, None,
            "[{label}] detached worktree branch is None: {entry:?}"
        );
    }
    cleanup(&root, &path);
}

#[test]
fn multiple_worktrees_all_listed() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let mut created = Vec::new();
    for i in 0..3 {
        let info = make_branch_worktree(&g, &format!("multi-{i}"), &format!("multi-{i}"), &format!("multi/{i}"));
        created.push(info);
    }
    let list = g.list_worktrees().unwrap();
    for i in 0..3 {
        assert!(
            list.iter()
                .any(|w| w.branch.as_deref() == Some(format!("multi/{i}").as_str())),
            "branch multi/{i} should be listed: {list:?}"
        );
    }
    assert!(list.len() >= 4, "at least 4 worktrees: {list:?}");
    for info in &created {
        cleanup(&root, &info.path);
    }
}

#[test]
fn list_worktree_branch_matches_creation() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("match/{label}");
        let info = make_branch_worktree(&g, &format!("m-{label}"), "m", &branch);
        let list = g.list_worktrees().unwrap();
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some(branch.as_str())),
            "[{label}] created branch present in listing: {list:?}"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn backends_agree_on_listing_branch_set() {
    let (_d, root) = init_repo();
    let setup = Git::open(&root).unwrap();
    let info = make_branch_worktree(&setup, "agree", "agree", "agree-branch");

    let mut libgit2_branches: BTreeSet<Option<String>> = Git::open(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();
    let mut shell_branches: BTreeSet<Option<String>> = Git::open_shell(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();
    // BTreeSet already sorted/deduped; compare directly.
    assert_eq!(
        libgit2_branches, shell_branches,
        "both backends should report the same branch set"
    );
    libgit2_branches.clear();
    shell_branches.clear();
    cleanup(&root, &info.path);
}

#[test]
fn list_after_remove_drops_the_entry() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "drop", "drop", "drop-branch");
    assert!(g
        .list_worktrees()
        .unwrap()
        .iter()
        .any(|w| w.branch.as_deref() == Some("drop-branch")));
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("drop-branch"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    assert!(!g
        .list_worktrees()
        .unwrap()
        .iter()
        .any(|w| w.branch.as_deref() == Some("drop-branch")));
    cleanup(&root, &info.path);
}

// ===========================================================================
// MANUFACTURE PHASE — create_worktree
// ===========================================================================

#[test]
fn create_new_branch_worktree() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let initial = g.list_worktrees().unwrap().len();
        let branch = format!("station/frame-{label}");
        let info = make_branch_worktree(&g, &format!("nb-{label}"), "frame", &branch);

        assert_eq!(
            info.branch.as_deref(),
            Some(branch.as_str()),
            "[{label}] reported branch"
        );
        assert!(info.path.exists(), "[{label}] worktree dir exists");
        assert!(!info.locked, "[{label}] freshly created not locked");

        let list = g.list_worktrees().unwrap();
        assert_eq!(list.len(), initial + 1, "[{label}] one more worktree");

        // Primary tree untouched.
        assert_eq!(
            g.current_branch().unwrap().as_deref(),
            Some("main"),
            "[{label}] primary still on main"
        );

        // The worktree itself is on the new branch and clean.
        let inner = Git::open(&info.path).unwrap();
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some(branch.as_str()),
            "[{label}] worktree checked out new branch"
        );
        assert!(inner.is_clean().unwrap(), "[{label}] worktree clean");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_returns_existing_path_on_disk() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let info = make_branch_worktree(&g, &format!("ex-{label}"), "ex", &format!("ex/{label}"));
        assert!(info.path.is_dir(), "[{label}] reported path is a directory");
        assert!(
            info.path.join("README.md").exists(),
            "[{label}] worktree has checked-out content"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_new_branch_actually_creates_branch_ref() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("newref/{label}");
        let info = make_branch_worktree(&g, &format!("nr-{label}"), "nr", &branch);
        let branches = git_out(&root, &["branch", "--list", &branch]);
        assert!(
            branches.contains(&branch),
            "[{label}] branch ref created: {branches:?}"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_from_reference_branch() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let ref_branch = format!("release/{label}");
        git(&root, &["branch", &ref_branch]);
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("ref-{label}"));
        let opts = CreateOptions {
            reference: Some(ref_branch.clone()),
            new_branch: None,
        };
        let info = g
            .create_worktree("rel", &path, &opts)
            .unwrap_or_else(|e| panic!("[{label}] create from ref: {e}"));
        assert!(info.path.exists(), "[{label}] dir exists");
        let inner = Git::open(&info.path).unwrap();
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some(ref_branch.as_str()),
            "[{label}] attached to reference branch"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_new_branch_off_reference() {
    let (_d, root, base) = init_repo_two_commits();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("nboff-{label}"));
        let opts = CreateOptions {
            reference: Some(base.clone()),
            new_branch: Some(format!("topic-{label}")),
        };
        let info = g
            .create_worktree("topic", &path, &opts)
            .unwrap_or_else(|e| panic!("[{label}] create nb off ref: {e}"));
        let inner = Git::open(&info.path).unwrap();
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some(format!("topic-{label}").as_str()),
            "[{label}] on new branch"
        );
        let wt_head = git_out(&info.path, &["rev-parse", "HEAD"]);
        assert_eq!(wt_head, base, "[{label}] new branch forked from base");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_from_a_non_branch_revision_detaches() {
    // A reference that resolves to a commit but is NOT a local branch (a raw
    // SHA) attaches no branch — the worktree is detached at that commit.
    let (_d, root, base) = init_repo_two_commits();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("det-{label}"));
        let opts = CreateOptions { reference: Some(base.clone()), new_branch: None };
        let info = g
            .create_worktree("det", &path, &opts)
            .unwrap_or_else(|e| panic!("[{label}] detached create: {e}"));
        assert!(info.path.exists(), "[{label}] worktree dir exists");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_with_no_options_detaches_at_head() {
    // No reference and no new branch → a fully detached worktree at HEAD.
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("bare-{label}"));
        let info = g
            .create_worktree("bare", &path, &CreateOptions::default())
            .unwrap_or_else(|e| panic!("[{label}] bare create: {e}"));
        assert_eq!(git_out(&info.path, &["rev-parse", "HEAD"]), head, "[{label}] detached at HEAD");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_default_forks_from_head() {
    // No reference + new branch => the new branch starts at current HEAD.
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let info = make_branch_worktree(&g, &format!("hd-{label}"), "hd", &format!("fromhead/{label}"));
        let wt_head = git_out(&info.path, &["rev-parse", "HEAD"]);
        assert_eq!(wt_head, head, "[{label}] new branch starts at HEAD");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_from_tag_reference() {
    let (_d, root) = init_repo();
    git(&root, &["tag", "v1.0"]);
    let tag_sha = git_out(&root, &["rev-parse", "v1.0^{commit}"]);
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("tag-{label}"));
        let opts = CreateOptions {
            reference: Some("v1.0".to_string()),
            new_branch: Some(format!("fromtag/{label}")),
        };
        let info = g
            .create_worktree("t", &path, &opts)
            .unwrap_or_else(|e| panic!("[{label}] create from tag: {e}"));
        let wt_head = git_out(&info.path, &["rev-parse", "HEAD"]);
        assert_eq!(wt_head, tag_sha, "[{label}] forked off tag commit");
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_from_full_sha_reference() {
    let (_d, root, base) = init_repo_two_commits();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("sha-{label}"));
        let opts = CreateOptions {
            reference: Some(base.clone()),
            new_branch: Some(format!("fromsha/{label}")),
        };
        let info = g
            .create_worktree("s", &path, &opts)
            .unwrap_or_else(|e| panic!("[{label}] create from sha: {e}"));
        assert_eq!(
            git_out(&info.path, &["rev-parse", "HEAD"]),
            base,
            "[{label}] checked out the requested sha"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_worktree_with_nested_branch_name() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("unit/{label}/sub/deep");
        let info = make_branch_worktree(&g, &format!("nest-{label}"), "n", &branch);
        let inner = Git::open(&info.path).unwrap();
        assert_eq!(
            inner.current_branch().unwrap().as_deref(),
            Some(branch.as_str()),
            "[{label}] nested branch name preserved"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_duplicate_branch_name_errors() {
    // The same new branch name cannot be created twice.
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("dup/{label}");
        let info = make_branch_worktree(&g, &format!("d1-{label}"), "d1", &branch);
        let path2 = sibling(&root, &format!("d2-{label}"));
        let opts = CreateOptions {
            reference: None,
            new_branch: Some(branch.clone()),
        };
        let err = g
            .create_worktree("d2", &path2, &opts)
            .expect_err(&format!("[{label}] duplicate branch should fail"));
        // Either WorktreeExists, Git2, or a Command failure depending on backend.
        match err {
            GitError::WorktreeExists(_)
            | GitError::Git2(_)
            | GitError::Gix(_)
            | GitError::Command { .. } => {}
            other => panic!("[{label}] unexpected error for dup branch: {other:?}"),
        }
        cleanup(&root, &info.path);
        cleanup(&root, &path2);
    }
}

#[test]
fn create_duplicate_name_libgit2_reports_worktree_exists() {
    // Two worktrees with the same logical name (but different branches/paths):
    // the libgit2 backend rejects on the name collision explicitly.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let p1 = sibling(&root, "name1");
    let p2 = sibling(&root, "name2");
    let o1 = CreateOptions {
        reference: None,
        new_branch: Some("name-a".into()),
    };
    let info = g.create_worktree("collide", &p1, &o1).unwrap();
    let o2 = CreateOptions {
        reference: None,
        new_branch: Some("name-b".into()),
    };
    let err = g
        .create_worktree("collide", &p2, &o2)
        .expect_err("duplicate worktree name should fail");
    assert!(
        matches!(err, GitError::WorktreeExists(ref n) if n == "collide"),
        "expected WorktreeExists(collide), got {err:?}"
    );
    cleanup(&root, &info.path);
    cleanup(&root, &p2);
}

#[test]
fn create_at_existing_nonempty_path_errors() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("occupied-{label}"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("preexisting.txt"), "in the way").unwrap();
        let opts = CreateOptions {
            reference: None,
            new_branch: Some(format!("occ/{label}")),
        };
        let result = g.create_worktree("occ", &path, &opts);
        assert!(
            result.is_err(),
            "[{label}] creating into a non-empty dir should fail"
        );
        cleanup(&root, &path);
    }
}

#[test]
fn create_with_nonexistent_reference_errors() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let path = sibling(&root, &format!("badref-{label}"));
        let opts = CreateOptions {
            reference: Some("no-such-ref-anywhere".to_string()),
            new_branch: Some(format!("br/{label}")),
        };
        let err = g
            .create_worktree("br", &path, &opts)
            .expect_err(&format!("[{label}] bad reference should fail"));
        match err {
            GitError::Git2(_) | GitError::Command { .. } | GitError::Gix(_) => {}
            other => panic!("[{label}] unexpected error for bad ref: {other:?}"),
        }
        cleanup(&root, &path);
    }
}

#[test]
fn create_worktree_is_clean_immediately() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let info = make_branch_worktree(&g, &format!("cln-{label}"), "cln", &format!("cln/{label}"));
        let inner = Git::open(&info.path).unwrap();
        assert!(
            inner.is_clean().unwrap(),
            "[{label}] fresh worktree is clean"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_worktree_does_not_dirty_primary() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        assert!(g.is_clean().unwrap(), "[{label}] primary clean before");
        let info = make_branch_worktree(&g, &format!("noprim-{label}"), "np", &format!("np/{label}"));
        assert!(
            g.is_clean().unwrap(),
            "[{label}] primary still clean after create"
        );
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_multiple_independent_branches() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let mut infos = Vec::new();
    for i in 0..4 {
        infos.push(make_branch_worktree(
            &g,
            &format!("ind-{i}"),
            &format!("ind-{i}"),
            &format!("independent/{i}"),
        ));
    }
    // Each lives on its own branch.
    let mut seen = BTreeSet::new();
    let list = g.list_worktrees().unwrap();
    for i in 0..4 {
        let br = format!("independent/{i}");
        assert!(
            list.iter().any(|w| w.branch.as_deref() == Some(br.as_str())),
            "branch {br} present"
        );
        seen.insert(br);
    }
    assert_eq!(seen.len(), 4, "four distinct branches");
    for info in &infos {
        cleanup(&root, &info.path);
    }
}

#[test]
fn create_worktree_content_isolated_from_primary() {
    // Editing a worktree must not touch the primary checkout's files.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "iso2", "iso2", "iso2/branch");
    std::fs::write(info.path.join("only-here.txt"), "wt-only").unwrap();
    assert!(
        !root.join("only-here.txt").exists(),
        "file created in worktree must not appear in primary tree"
    );
    cleanup(&root, &info.path);
}

// ===========================================================================
// AUDIT PHASE — remove_worktree
// ===========================================================================

#[test]
fn libgit2_remove_worktree_surfaces_a_disk_removal_fault() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, root) = init_repo();
    // libgit2 backend specifically (its remove does an in-process remove_dir_all).
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "rmfault", "gone", "gone-br");
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("gone-br"))
        .map(|w| w.name)
        .expect("created worktree name");
    // Make the worktree dir read-only so removing its contents fails (EACCES).
    std::fs::set_permissions(&info.path, std::fs::Permissions::from_mode(0o555)).unwrap();
    let err = g.remove_worktree(&name, true);
    std::fs::set_permissions(&info.path, std::fs::Permissions::from_mode(0o755)).unwrap();
    cleanup(&root, &info.path);
    assert!(err.is_err(), "an unremovable worktree dir surfaces an error");
}

#[test]
fn remove_worktree_cleans_disk_and_registry() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("gone-{label}");
        let info = make_branch_worktree(&g, &format!("rm-{label}"), "gone", &branch);
        assert!(info.path.exists());

        let listed = g.list_worktrees().unwrap();
        let name = listed
            .iter()
            .find(|w| w.branch.as_deref() == Some(branch.as_str()))
            .map(|w| w.name.clone())
            .unwrap_or_else(|| panic!("[{label}] find created name"));

        g.remove_worktree(&name, true)
            .unwrap_or_else(|e| panic!("[{label}] remove: {e}"));
        assert!(!info.path.exists(), "[{label}] dir removed from disk");

        let after = g.list_worktrees().unwrap();
        assert!(
            !after
                .iter()
                .any(|w| w.branch.as_deref() == Some(branch.as_str())),
            "[{label}] removed worktree gone from registry: {after:?}"
        );
    }
}

#[test]
fn remove_missing_worktree_errors() {
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let err = g
            .remove_worktree("does-not-exist", false)
            .expect_err("removing unknown worktree must error");
        assert!(
            matches!(err, GitError::WorktreeNotFound(ref n) if n == "does-not-exist"),
            "[{label}] expected WorktreeNotFound(does-not-exist), got {err:?}"
        );
    }
}

#[test]
fn remove_missing_worktree_errors_with_force() {
    // force=true must still error for a name that was never registered.
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let err = g
            .remove_worktree("phantom", true)
            .expect_err("forced remove of unknown must error");
        assert!(
            matches!(err, GitError::WorktreeNotFound(_)),
            "[{label}] expected WorktreeNotFound, got {err:?}"
        );
    }
}

#[test]
fn remove_is_idempotent_second_remove_errors() {
    // Removing once succeeds; removing the same name again reports not-found.
    let (_d, root) = init_repo();
    for (label, open) in backends() {
        let g = open(&root).unwrap();
        let branch = format!("idem-rm-{label}");
        let _info = make_branch_worktree(&g, &format!("ir-{label}"), "ir", &branch);
        let name = g
            .list_worktrees()
            .unwrap()
            .into_iter()
            .find(|w| w.branch.as_deref() == Some(branch.as_str()))
            .map(|w| w.name)
            .unwrap();
        g.remove_worktree(&name, true)
            .unwrap_or_else(|e| panic!("[{label}] first remove: {e}"));
        let err = g
            .remove_worktree(&name, true)
            .expect_err(&format!("[{label}] second remove must error"));
        assert!(
            matches!(err, GitError::WorktreeNotFound(_)),
            "[{label}] second remove => WorktreeNotFound, got {err:?}"
        );
    }
}

#[test]
fn remove_dirty_worktree_with_force_succeeds() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "dirtyrm", "dirtyrm", "dirtyrm/branch");
    std::fs::write(info.path.join("uncommitted.txt"), "scratch").unwrap();
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("dirtyrm/branch"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true)
        .expect("forced remove of dirty worktree should succeed");
    assert!(!info.path.exists(), "dirty worktree removed from disk");
}

#[test]
fn remove_lets_branch_name_be_reused() {
    // After removing a worktree and deleting its branch, the same branch name
    // can be created again.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "reuse", "reuse", "reuse/branch");
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("reuse/branch"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    git(&root, &["branch", "-D", "reuse/branch"]);
    // Recreate.
    let info2 = make_branch_worktree(&g, "reuse2", "reuse2", "reuse/branch");
    assert!(info2.path.exists(), "branch name reusable after cleanup");
    cleanup(&root, &info2.path);
    let _ = info;
}

#[test]
fn remove_one_of_many_leaves_others() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let a = make_branch_worktree(&g, "keepa", "keepa", "keep/a");
    let b = make_branch_worktree(&g, "keepb", "keepb", "keep/b");
    let c = make_branch_worktree(&g, "keepc", "keepc", "keep/c");

    let name_b = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("keep/b"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name_b, true).unwrap();

    let list = g.list_worktrees().unwrap();
    assert!(
        list.iter().any(|w| w.branch.as_deref() == Some("keep/a")),
        "keep/a survives"
    );
    assert!(
        !list.iter().any(|w| w.branch.as_deref() == Some("keep/b")),
        "keep/b removed"
    );
    assert!(
        list.iter().any(|w| w.branch.as_deref() == Some("keep/c")),
        "keep/c survives"
    );
    cleanup(&root, &a.path);
    cleanup(&root, &b.path);
    cleanup(&root, &c.path);
}

// ===========================================================================
// CHECKPOINT PHASE — round trips, determinism, cross-backend create/remove
// ===========================================================================

#[test]
fn full_lifecycle_libgit2() {
    lifecycle(|p| Git::open(p), "libgit2");
}

#[test]
fn full_lifecycle_shell() {
    lifecycle(|p| Git::open_shell(p), "shell");
}

/// create -> list -> verify -> remove -> list verifies the entry is gone.
fn lifecycle(open: fn(&Path) -> darkrun_git::Result<Git>, label: &str) {
    let (_d, root) = init_repo();
    let g = open(&root).unwrap();
    let before = g.list_worktrees().unwrap().len();

    let branch = format!("life/{label}");
    let info = make_branch_worktree(&g, &format!("life-{label}"), "life", &branch);
    assert!(info.path.exists(), "[{label}] created on disk");
    assert_eq!(g.list_worktrees().unwrap().len(), before + 1);

    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch.as_str()))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    assert!(!info.path.exists(), "[{label}] removed from disk");
    assert_eq!(
        g.list_worktrees().unwrap().len(),
        before,
        "[{label}] count back to baseline"
    );
}

#[test]
fn worktree_created_by_libgit2_visible_to_shell() {
    let (_d, root) = init_repo();
    let lib = Git::open(&root).unwrap();
    let info = make_branch_worktree(&lib, "x2s", "x2s", "cross/lib-to-shell");

    let sh = Git::open_shell(&root).unwrap();
    let list = sh.list_worktrees().unwrap();
    assert!(
        list.iter()
            .any(|w| w.branch.as_deref() == Some("cross/lib-to-shell")),
        "shell sees libgit2-created worktree: {list:?}"
    );
    cleanup(&root, &info.path);
}

#[test]
fn worktree_created_by_shell_visible_to_libgit2() {
    let (_d, root) = init_repo();
    let sh = Git::open_shell(&root).unwrap();
    let info = make_branch_worktree(&sh, "s2x", "s2x", "cross/shell-to-lib");

    let lib = Git::open(&root).unwrap();
    let list = lib.list_worktrees().unwrap();
    assert!(
        list.iter()
            .any(|w| w.branch.as_deref() == Some("cross/shell-to-lib")),
        "libgit2 sees shell-created worktree: {list:?}"
    );
    cleanup(&root, &info.path);
}

#[test]
fn worktree_created_by_libgit2_removable_by_shell() {
    let (_d, root) = init_repo();
    let lib = Git::open(&root).unwrap();
    let info = make_branch_worktree(&lib, "rmx", "rmx", "rmcross/branch");

    let sh = Git::open_shell(&root).unwrap();
    let name = sh
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("rmcross/branch"))
        .map(|w| w.name)
        .unwrap();
    sh.remove_worktree(&name, true)
        .expect("shell removes libgit2-created worktree");
    assert!(!info.path.exists(), "removed from disk by shell");
}

#[test]
fn worktree_created_by_shell_removable_by_libgit2() {
    let (_d, root) = init_repo();
    let sh = Git::open_shell(&root).unwrap();
    let info = make_branch_worktree(&sh, "rms", "rms", "rmcross2/branch");

    let lib = Git::open(&root).unwrap();
    let name = lib
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("rmcross2/branch"))
        .map(|w| w.name)
        .unwrap();
    lib.remove_worktree(&name, true)
        .expect("libgit2 removes shell-created worktree");
    assert!(!info.path.exists(), "removed from disk by libgit2");
}

#[test]
fn isolated_worktree_dirty_state_is_independent() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "isodirty", "iso", "iso-branch");

    std::fs::write(info.path.join("wip.txt"), "scratch").unwrap();
    let inner = Git::open(&info.path).unwrap();
    assert!(!inner.is_clean().unwrap(), "worktree should be dirty");
    assert!(g.is_clean().unwrap(), "primary tree should stay clean");
    cleanup(&root, &info.path);
}

#[test]
fn primary_dirty_does_not_dirty_worktree() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "primdirty", "pd", "pd-branch");

    // Dirty the primary tree only.
    std::fs::write(root.join("primary-wip.txt"), "scratch").unwrap();
    assert!(!g.is_clean().unwrap(), "primary now dirty");
    let inner = Git::open(&info.path).unwrap();
    assert!(inner.is_clean().unwrap(), "worktree stays clean");

    std::fs::remove_file(root.join("primary-wip.txt")).unwrap();
    cleanup(&root, &info.path);
}

#[test]
fn worktree_branch_independent_of_primary_checkout() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "indbr", "indbr", "indbr/branch");
    // Switch primary to a new branch.
    git(&root, &["checkout", "-q", "-b", "primary-moved"]);
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("primary-moved"));
    let inner = Git::open(&info.path).unwrap();
    assert_eq!(
        inner.current_branch().unwrap().as_deref(),
        Some("indbr/branch"),
        "worktree branch unaffected by primary checkout move"
    );
    cleanup(&root, &info.path);
    git(&root, &["checkout", "-q", "main"]);
}

#[test]
fn create_remove_create_same_name_after_cleanup() {
    // A name can be reused once the prior worktree+branch are fully gone.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "cyc1", "cyc", "cyc/one");
    let name = g
        .list_worktrees()
        .unwrap()
        .into_iter()
        .find(|w| w.branch.as_deref() == Some("cyc/one"))
        .map(|w| w.name)
        .unwrap();
    g.remove_worktree(&name, true).unwrap();
    // Different branch + path, same logical purpose.
    let info2 = make_branch_worktree(&g, "cyc2", "cyc", "cyc/two");
    assert!(info2.path.exists());
    let _ = info;
    cleanup(&root, &info2.path);
}

#[test]
fn list_deterministic_branch_set_across_repeat_opens() {
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let a = make_branch_worktree(&g, "det1", "det1", "det/1");
    let b = make_branch_worktree(&g, "det2", "det2", "det/2");

    let set = |g: &Git| -> BTreeSet<Option<String>> {
        g.list_worktrees().unwrap().into_iter().map(|w| w.branch).collect()
    };
    let s1 = set(&Git::open(&root).unwrap());
    let s2 = set(&Git::open(&root).unwrap());
    let s3 = set(&Git::open_shell(&root).unwrap());
    assert_eq!(s1, s2, "repeated libgit2 opens deterministic");
    assert_eq!(s1, s3, "libgit2 and shell agree on branch set");
    cleanup(&root, &a.path);
    cleanup(&root, &b.path);
}

#[test]
fn error_display_messages_are_descriptive() {
    let not_repo = GitError::NotARepo(PathBuf::from("/nope"));
    assert!(format!("{not_repo}").contains("/nope"), "NotARepo shows path");

    let exists = GitError::WorktreeExists("dup".into());
    assert!(
        format!("{exists}").contains("dup"),
        "WorktreeExists shows name"
    );

    let missing = GitError::WorktreeNotFound("ghost".into());
    assert!(
        format!("{missing}").contains("ghost"),
        "WorktreeNotFound shows name"
    );
}

#[test]
fn current_branch_in_worktree_after_commit_there() {
    // Committing inside a worktree keeps its branch and turns it clean.
    let (_d, root) = init_repo();
    let g = Git::open(&root).unwrap();
    let info = make_branch_worktree(&g, "commit-wt", "cw", "cw/branch");
    std::fs::write(info.path.join("work.txt"), "done").unwrap();
    git(&info.path, &["config", "user.email", "w@d.local"]);
    git(&info.path, &["config", "user.name", "w"]);
    git(&info.path, &["config", "commit.gpgsign", "false"]);
    git(&info.path, &["add", "-A"]);
    git(&info.path, &["commit", "-qm", "wt work"]);

    let inner = Git::open(&info.path).unwrap();
    assert_eq!(
        inner.current_branch().unwrap().as_deref(),
        Some("cw/branch"),
        "branch stable after commit"
    );
    assert!(inner.is_clean().unwrap(), "clean after committing");
    // Primary HEAD did not move.
    assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
    cleanup(&root, &info.path);
}

// ===========================================================================
// MERGE / NETWORK / REBASE — the wrapper's GitBackend delegation, both backends
// ===========================================================================

/// Drive the merge trio, the push/fetch network ops, and rebase through the
/// `Git` wrapper for BOTH backends — covering the wrapper's delegating methods
/// and (for libgit2) its lazy-shell delegation for these ops.
#[test]
fn merge_network_and_rebase_through_the_wrapper() {
    for (label, open) in backends_with_network() {
        let bare = TempDir::new().unwrap();
        git(bare.path(), &["init", "-q", "--bare"]);
        let (_dir, root) = init_repo();
        git(&root, &["remote", "add", "origin", &bare.path().to_string_lossy()]);
        let g = open(&root).unwrap_or_else(|e| panic!("[{label}] open: {e}"));

        // Network ops through the wrapper.
        g.push(&root, "main").unwrap_or_else(|e| panic!("[{label}] push: {e}"));
        g.fetch(&root, "main").unwrap_or_else(|e| panic!("[{label}] fetch: {e}"));

        // A divergent, non-conflicting branch.
        git(&root, &["checkout", "-q", "-b", "side"]);
        std::fs::write(root.join("side.txt"), "s\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "side"]);
        git(&root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("main.txt"), "m\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "main"]);

        assert!(!g.refs_have_identical_trees("main", "side").unwrap(), "[{label}] trees differ");
        // Up-to-date merge → no-op; a real merge stages.
        let noop = g.merge_no_commit(&root, "main").unwrap();
        assert!(noop.ok && !noop.performed, "[{label}] up-to-date is a no-op");
        let merged = g.merge_no_commit(&root, "side").unwrap();
        assert!(merged.ok && merged.performed, "[{label}] real merge performs");
        assert!(g.merge_in_progress(&root).unwrap(), "[{label}] merge staged");
        g.checkout_paths(&root, "HEAD", &["main.txt".into()]).unwrap();
        g.add_paths(&root, &["main.txt".into()]).unwrap();
        g.commit(&root, "merge side").unwrap();
        assert!(
            g.ls_tree(&root, "HEAD", "side.txt").unwrap().iter().any(|p| p == "side.txt"),
            "[{label}] merge brought in side.txt"
        );
        assert!(g.unresolved_paths(&root).unwrap().is_empty(), "[{label}] no conflicts");

        // Rebase a feature branch onto main, then abort (no-op when none in flight).
        git(&root, &["checkout", "-q", "-b", "feat"]);
        std::fs::write(root.join("f.txt"), "f\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-qm", "f"]);
        g.rebase_onto(&root, "main").unwrap_or_else(|e| panic!("[{label}] rebase: {e}"));
        g.rebase_abort(&root).unwrap();
    }
}

// ===========================================================================
// GIX BACKEND (pure Rust) — read conformance vs the reference backends
// ===========================================================================

/// The gix backend's implemented reads must agree with libgit2 on the same repo.
#[test]
fn gix_reads_agree_with_reference() {
    let (_d, root) = init_repo();
    git(&root, &["checkout", "-q", "-b", "gixtest/branch"]);

    let lib = Git::open(&root).unwrap();
    let gx = Git::open_gix(&root).unwrap();

    // current_branch on a branch
    assert_eq!(gx.current_branch().unwrap(), lib.current_branch().unwrap());
    assert_eq!(gx.current_branch().unwrap().as_deref(), Some("gixtest/branch"));

    // branch_exists
    assert!(gx.branch_exists("gixtest/branch").unwrap());
    assert!(!gx.branch_exists("no-such-branch").unwrap());
    assert_eq!(
        gx.branch_exists("gixtest/branch").unwrap(),
        lib.branch_exists("gixtest/branch").unwrap()
    );

    // is_clean: clean → then dirty
    assert!(gx.is_clean().unwrap());
    assert_eq!(gx.is_clean().unwrap(), lib.is_clean().unwrap());
    std::fs::write(root.join("dirty.txt"), "x").unwrap();
    let gx2 = Git::open_gix(&root).unwrap();
    let lib2 = Git::open(&root).unwrap();
    assert!(!gx2.is_clean().unwrap());
    assert_eq!(gx2.is_clean().unwrap(), lib2.is_clean().unwrap());
}

/// A detached HEAD reports no current branch — same as libgit2.
#[test]
fn gix_detached_head_has_no_branch() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-q", &head]);
    let gx = Git::open_gix(&root).unwrap();
    assert_eq!(gx.current_branch().unwrap(), None);
    assert_eq!(Git::open(&root).unwrap().current_branch().unwrap(), None);
}

/// gix ancestry + tree-identity reads agree with libgit2.
#[test]
fn gix_ancestry_and_tree_identity_agree() {
    let (_d, root) = init_repo();
    let c1 = git_out(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "c2"]);
    let c2 = git_out(&root, &["rev-parse", "HEAD"]);

    let gx = Git::open_gix(&root).unwrap();
    let lib = Git::open(&root).unwrap();

    // is_ancestor: c1 → c2 (yes), c2 → c1 (no), self (yes); agrees with libgit2.
    assert!(gx.is_ancestor(&c1, &c2).unwrap());
    assert!(!gx.is_ancestor(&c2, &c1).unwrap());
    assert!(gx.is_ancestor(&c1, &c1).unwrap());
    assert_eq!(gx.is_ancestor(&c1, &c2).unwrap(), lib.is_ancestor(&c1, &c2).unwrap());
    assert_eq!(gx.is_ancestor(&c2, &c1).unwrap(), lib.is_ancestor(&c2, &c1).unwrap());

    // refs_have_identical_trees: same ref identical; distinct commits differ; an
    // unresolvable ref is false (never errors).
    assert!(gx.refs_have_identical_trees(&c2, &c2).unwrap());
    assert!(!gx.refs_have_identical_trees(&c1, &c2).unwrap());
    assert!(!gx.refs_have_identical_trees(&c1, "no-such-ref").unwrap());
    assert_eq!(
        gx.refs_have_identical_trees(&c1, &c2).unwrap(),
        lib.refs_have_identical_trees(&c1, &c2).unwrap()
    );
}

/// gix merge_in_progress agrees with the reference: false on a clean repo, true
/// while a merge with conflicts is unresolved in-tree.
#[test]
fn gix_merge_in_progress_agrees() {
    let (_d, root) = init_repo();
    let gx = Git::open_gix(&root).unwrap();
    let lib = Git::open(&root).unwrap();
    // Clean repo: no merge in progress.
    assert!(!gx.merge_in_progress(&root).unwrap());
    assert_eq!(gx.merge_in_progress(&root).unwrap(), lib.merge_in_progress(&root).unwrap());

    // Force a conflicting merge in-tree (leave MERGE_HEAD set).
    git(&root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(root.join("c.txt"), "side\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "side"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("c.txt"), "main\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "main"]);
    let _ = std::process::Command::new("git").arg("-C").arg(&root)
        .args(["merge", "--no-edit", "side"]).output().unwrap(); // conflicts, leaves MERGE_HEAD
    let gx2 = Git::open_gix(&root).unwrap();
    assert!(gx2.merge_in_progress(&root).unwrap(), "MERGE_HEAD present => in progress");
    assert_eq!(gx2.merge_in_progress(&root).unwrap(), Git::open(&root).unwrap().merge_in_progress(&root).unwrap());
}

/// gix ls_tree (recursive, prefix-filtered) + unresolved_paths agree with the reference.
#[test]
fn gix_ls_tree_and_unresolved_paths_agree() {
    let (_d, root) = init_repo();
    std::fs::create_dir_all(root.join("sub/deep")).unwrap();
    std::fs::write(root.join("sub/a.txt"), "a").unwrap();
    std::fs::write(root.join("sub/deep/b.txt"), "b").unwrap();
    std::fs::write(root.join("top.txt"), "t").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "files"]);

    let gx = Git::open_gix(&root).unwrap();
    let lib = Git::open(&root).unwrap();

    let mut g = gx.ls_tree(&root, "HEAD", "sub").unwrap();
    g.sort();
    let mut l = lib.ls_tree(&root, "HEAD", "sub").unwrap();
    l.sort();
    assert_eq!(g, l, "gix and libgit2 list the same paths under the prefix");
    assert_eq!(g, vec!["sub/a.txt".to_string(), "sub/deep/b.txt".to_string()]);

    // Empty prefix → every tracked path (recursive).
    let all = gx.ls_tree(&root, "HEAD", "").unwrap();
    assert!(all.contains(&"top.txt".to_string()) && all.contains(&"sub/deep/b.txt".to_string()));

    // unresolved_paths: clean → empty; then a real conflict surfaces the path.
    assert!(gx.unresolved_paths(&root).unwrap().is_empty());
    git(&root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(root.join("c.txt"), "side\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "s"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("c.txt"), "main\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "m"]);
    let _ = std::process::Command::new("git").arg("-C").arg(&root)
        .args(["merge", "--no-edit", "side"]).output().unwrap();

    let gx2 = Git::open_gix(&root).unwrap();
    let mut up = gx2.unresolved_paths(&root).unwrap();
    up.sort();
    assert_eq!(up, vec!["c.txt".to_string()]);
    let mut lp = Git::open(&root).unwrap().unresolved_paths(&root).unwrap();
    lp.sort();
    assert_eq!(up, lp, "gix and libgit2 agree on the conflicted path set");
}

/// gix list_worktrees agrees with libgit2 on the branch set (a linked worktree
/// is created via the reference backend; gix must enumerate it identically).
#[test]
fn gix_list_worktrees_agrees() {
    let (_d, root) = init_repo();
    let setup = Git::open(&root).unwrap();
    let info = make_branch_worktree(&setup, "gixwt", "gixwt", "gixwt-branch");

    let gx_branches: BTreeSet<Option<String>> = Git::open_gix(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();
    let lib_branches: BTreeSet<Option<String>> = Git::open(&root)
        .unwrap()
        .list_worktrees()
        .unwrap()
        .into_iter()
        .map(|w| w.branch)
        .collect();

    assert!(
        gx_branches.contains(&Some("gixwt-branch".to_string())),
        "gix sees the linked worktree's branch"
    );
    assert_eq!(gx_branches, lib_branches, "gix and libgit2 agree on the worktree branch set");
    cleanup(&root, &info.path);
}

/// gix create_branch creates a branch at the ref and is idempotent.
#[test]
fn gix_create_branch_agrees() {
    let (_d, root) = init_repo();
    let head = git_out(&root, &["rev-parse", "HEAD"]);
    let gx = Git::open_gix(&root).unwrap();

    gx.create_branch("gixmade/feature", "HEAD").unwrap();
    assert!(gx.branch_exists("gixmade/feature").unwrap());
    assert!(Git::open(&root).unwrap().branch_exists("gixmade/feature").unwrap());
    assert_eq!(git_out(&root, &["rev-parse", "gixmade/feature"]), head, "points at HEAD");

    // Idempotent: a second create is a clean no-op.
    gx.create_branch("gixmade/feature", "HEAD").unwrap();
}

/// The hand-rolled gix write-tree must produce a tree hash IDENTICAL to
/// `git write-tree` (any sort/mode error changes the hash), and the resulting
/// commit must pass `git fsck`.
#[test]
fn gix_commit_produces_a_git_identical_tree() {
    let (_d, root) = init_repo();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("a/b/c.txt"), "deep\n").unwrap();
    std::fs::write(root.join("top.txt"), "top\n").unwrap();
    std::fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    git(&root, &["add", "-A"]);
    let _ = std::process::Command::new("chmod").arg("+x").arg(root.join("run.sh")).status();
    git(&root, &["add", "-A"]);

    // The exact tree git would write for the current index.
    let want_tree = git_out(&root, &["write-tree"]);

    // gix commits the same index via our hand-rolled write-tree.
    Git::open_gix(&root).unwrap().commit(&root, "gix commit").unwrap();

    // HEAD's tree equals git's write-tree, byte-for-byte.
    let got_tree = git_out(&root, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(got_tree, want_tree, "gix-built tree must hash-match git write-tree");
    assert_eq!(git_out(&root, &["log", "-1", "--pretty=%s"]), "gix commit", "message recorded");
    let fsck = std::process::Command::new("git").arg("-C").arg(&root)
        .args(["fsck", "--strict"]).output().unwrap();
    assert!(fsck.status.success(), "git fsck: {}", String::from_utf8_lossy(&fsck.stderr));
}

/// gix add_paths stages worktree content into the index exactly like `git add`,
/// preserving the executable mode, and the staged tree round-trips through git.
#[test]
fn gix_add_paths_stages_like_git() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("foo.txt"), "foo\n").unwrap();
    std::fs::write(root.join("README.md"), "# changed\n").unwrap(); // modify a tracked file
    std::fs::write(root.join("run.sh"), "#!/bin/sh\n").unwrap();
    let _ = std::process::Command::new("chmod").arg("+x").arg(root.join("run.sh")).status();

    Git::open_gix(&root)
        .unwrap()
        .add_paths(&root, &["foo.txt".into(), "README.md".into(), "run.sh".into()])
        .unwrap();

    // All three are staged (in the index), not merely worktree-dirty.
    let staged = git_out(&root, &["diff", "--cached", "--name-only"]);
    for f in ["foo.txt", "README.md", "run.sh"] {
        assert!(staged.contains(f), "{f} staged; got:\n{staged}");
    }
    // The index tree round-trips through git, and content/mode are correct.
    assert!(!git_out(&root, &["write-tree"]).is_empty());
    assert_eq!(git_out(&root, &["cat-file", "blob", ":foo.txt"]), "foo");
    let staged_modes = git_out(&root, &["ls-files", "--stage"]);
    assert!(staged_modes.contains("100755") && staged_modes.contains("run.sh"), "exec mode kept:\n{staged_modes}");
}

/// gix checkout_paths restores a path's content from a ref into both the
/// worktree and the index (the engine-protected restore).
#[test]
fn gix_checkout_paths_restores_from_ref() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("state.md"), "v1\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "v1"]);

    // Diverge the worktree (and stage the divergence).
    std::fs::write(root.join("state.md"), "v2-local\n").unwrap();
    git(&root, &["add", "-A"]);

    // Restore the committed version.
    Git::open_gix(&root)
        .unwrap()
        .checkout_paths(&root, "HEAD", &["state.md".into()])
        .unwrap();

    // Worktree file is back to v1...
    assert_eq!(std::fs::read_to_string(root.join("state.md")).unwrap(), "v1\n");
    // ...and the index now matches HEAD again (nothing staged).
    let staged = git_out(&root, &["diff", "--cached", "--name-only"]);
    assert!(staged.is_empty(), "index restored to HEAD; staged: {staged:?}");
}

/// A gix-built linked worktree is a fully valid git worktree: git recognizes it,
/// the checkout is correct, and `git status` inside it is clean (proving the
/// admin files + index + checked-out files are all mutually consistent).
#[test]
fn gix_create_worktree_is_a_valid_git_worktree() {
    let (_d, root) = init_repo();
    std::fs::create_dir_all(root.join("d")).unwrap();
    std::fs::write(root.join("d/f.txt"), "hello\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "files"]);

    let wt_path = root.join("wt-new");
    let gx = Git::open_gix(&root).unwrap();
    let info = gx
        .create_worktree("wtnew", &wt_path, &CreateOptions {
            reference: None,
            new_branch: Some("feature/x".into()),
        })
        .unwrap();
    assert!(info.path.exists());
    assert_eq!(info.branch.as_deref(), Some("feature/x"));

    // The checked-out content is present.
    assert_eq!(std::fs::read_to_string(wt_path.join("d/f.txt")).unwrap(), "hello\n");

    // git itself accepts the worktree and runs there with a CLEAN status.
    let st = std::process::Command::new("git").arg("-C").arg(&wt_path)
        .args(["status", "--porcelain"]).output().unwrap();
    assert!(st.status.success(), "git status runs in the gix-made worktree");
    assert!(String::from_utf8_lossy(&st.stdout).trim().is_empty(),
        "worktree clean: {}", String::from_utf8_lossy(&st.stdout));

    // HEAD is on the new branch.
    let head = std::process::Command::new("git").arg("-C").arg(&wt_path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "feature/x");

    // gix lists it too, agreeing with libgit2 on the branch set.
    assert!(gx.list_worktrees().unwrap().iter().any(|w| w.branch.as_deref() == Some("feature/x")));

    // remove it.
    gx.remove_worktree("wtnew", true).unwrap();
    assert!(!wt_path.exists(), "worktree dir removed");
    let _ = std::process::Command::new("git").arg("-C").arg(&root).args(["worktree", "prune"]).output();
}

/// A clean gix merge stages the result; committing it yields a real two-parent
/// merge commit that git accepts.
#[test]
fn gix_merge_clean_then_commit_is_a_two_parent_merge() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("base.txt"), "base\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "base"]);
    git(&root, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(root.join("feat.txt"), "feature\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "feat"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("main.txt"), "main\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "main2"]);

    let gx = Git::open_gix(&root).unwrap();
    let out = gx.merge_no_commit(&root, "feature").unwrap();
    assert!(out.ok && out.performed, "clean merge staged: {out:?}");
    assert!(gx.merge_in_progress(&root).unwrap(), "MERGE_HEAD set");
    assert!(gx.unresolved_paths(&root).unwrap().is_empty(), "no conflicts");
    assert_eq!(std::fs::read_to_string(root.join("feat.txt")).unwrap(), "feature\n");

    gx.commit(&root, "merge feature").unwrap();
    let parents = git_out(&root, &["log", "-1", "--pretty=%P"]);
    assert_eq!(parents.split_whitespace().count(), 2, "two-parent merge commit: {parents}");
    assert!(!gx.merge_in_progress(&root).unwrap(), "MERGE_HEAD cleared");
    let fsck = std::process::Command::new("git").arg("-C").arg(&root).args(["fsck", "--strict"]).output().unwrap();
    assert!(fsck.status.success(), "fsck: {}", String::from_utf8_lossy(&fsck.stderr));
    assert!(root.join("feat.txt").exists() && root.join("main.txt").exists(), "both sides present");
}

/// A conflicting gix merge leaves the conflict in-tree: MERGE_HEAD set, the path
/// surfaces via unresolved_paths, and the worktree file carries conflict markers.
#[test]
fn gix_merge_conflict_left_in_tree() {
    let (_d, root) = init_repo();
    std::fs::write(root.join("c.txt"), "base\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "base"]);
    git(&root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(root.join("c.txt"), "side\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "side"]);
    git(&root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("c.txt"), "main\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "main"]);

    let gx = Git::open_gix(&root).unwrap();
    let out = gx.merge_no_commit(&root, "side").unwrap();
    assert!(out.performed && !out.ok, "performed but conflicted: {out:?}");
    assert!(gx.merge_in_progress(&root).unwrap(), "MERGE_HEAD set");
    assert_eq!(gx.unresolved_paths(&root).unwrap(), vec!["c.txt".to_string()]);
    let content = std::fs::read_to_string(root.join("c.txt")).unwrap();
    assert!(content.contains("<<<<<<<") && content.contains(">>>>>>>"), "markers: {content}");
}

/// Merging an ancestor is a no-op (already up to date) — no MERGE_HEAD.
#[test]
fn gix_merge_already_up_to_date_noop() {
    let (_d, root) = init_repo();
    git(&root, &["branch", "old"]);
    std::fs::write(root.join("new.txt"), "new\n").unwrap();
    git(&root, &["add", "-A"]); git(&root, &["commit", "-qm", "ahead"]);
    let gx = Git::open_gix(&root).unwrap();
    let out = gx.merge_no_commit(&root, "old").unwrap();
    assert!(out.ok && !out.performed, "no-op: {out:?}");
    assert!(!gx.merge_in_progress(&root).unwrap(), "no MERGE_HEAD");
}

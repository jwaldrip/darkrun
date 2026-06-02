//! The shell-out [`GitBackend`] fallback.
//!
//! Drives the `git` executable directly. This is the safety net for the
//! libgit2 backend: every operation here maps to the exact `git` command a
//! human would run, so behaviour matches the CLI precisely. Network-touching
//! operations are out of scope for this crate — these are all local worktree
//! and status queries.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::backend::{CreateOptions, GitBackend, MergeOutcome, WorktreeInfo};
use crate::error::{GitError, Result};

/// A [`GitBackend`] that shells out to the `git` executable.
pub struct ShellBackend {
    repo_root: PathBuf,
}

impl ShellBackend {
    /// Bind a shell backend to `repo_root`. Validates that the path is inside a
    /// git working tree, returning [`GitError::NotARepo`] otherwise.
    pub fn open(repo_root: impl AsRef<Path>) -> Result<Self> {
        let root = repo_root.as_ref().to_path_buf();
        let backend = Self { repo_root: root };
        let inside = backend
            .run(&["rev-parse", "--is-inside-work-tree"])
            .map(|out| out.trim() == "true")
            .unwrap_or(false);
        if !inside {
            return Err(GitError::NotARepo(backend.repo_root));
        }
        Ok(backend)
    }

    /// The repository root this backend was bound to.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Run `git <args>` in the repo root, returning trimmed stdout on success.
    fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(args)
            .output()
            .map_err(GitError::BareIo)?;
        if !output.status.success() {
            return Err(GitError::Command {
                args: args.iter().map(|s| s.to_string()).collect(),
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl GitBackend for ShellBackend {
    fn create_worktree(
        &self,
        _name: &str,
        path: &Path,
        opts: &CreateOptions,
    ) -> Result<WorktreeInfo> {
        // git derives the worktree name from the final path component, so the
        // `name` argument is advisory here and we report the path basename
        // back. `git worktree add [-b <new>] <path> [<base>]`.
        let path_str = path.to_string_lossy().to_string();
        let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
        if let Some(new_branch) = &opts.new_branch {
            args.push("-b".into());
            args.push(new_branch.clone());
        }
        args.push(path_str);
        if let Some(reference) = &opts.reference {
            args.push(reference.clone());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.run(&arg_refs)?;

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| _name.to_string());
        let branch = opts
            .new_branch
            .clone()
            .or_else(|| opts.reference.clone());
        Ok(WorktreeInfo {
            name,
            path: path.to_path_buf(),
            branch,
            locked: false,
        })
    }

    fn list_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let raw = self.run(&["worktree", "list", "--porcelain"])?;
        Ok(parse_worktree_list(&raw))
    }

    fn remove_worktree(&self, name: &str, force: bool) -> Result<()> {
        // Resolve the name to a path via the porcelain listing; git's
        // `worktree remove` takes a path, not a logical name.
        let target = self
            .list_worktrees()?
            .into_iter()
            .find(|w| w.name == name || w.path.ends_with(name))
            .ok_or_else(|| GitError::WorktreeNotFound(name.to_string()))?;

        let path_str = target.path.to_string_lossy().to_string();
        let mut args: Vec<&str> = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path_str);
        self.run(&args)?;
        Ok(())
    }

    fn current_branch(&self) -> Result<Option<String>> {
        let out = self.run(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = out.trim();
        // A detached HEAD reports the literal "HEAD".
        if branch.is_empty() || branch == "HEAD" {
            Ok(None)
        } else {
            Ok(Some(branch.to_string()))
        }
    }

    fn is_clean(&self) -> Result<bool> {
        let out = self.run(&["status", "--porcelain"])?;
        Ok(out.trim().is_empty())
    }

    fn branch_exists(&self, name: &str) -> Result<bool> {
        // `rev-parse --verify <name>` exits non-zero when the ref is absent;
        // qualify it as a head so a same-named tag/file can't masquerade.
        let qualified = format!("refs/heads/{name}");
        Ok(self
            .run(&["rev-parse", "--quiet", "--verify", &qualified])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(false))
    }

    fn create_branch(&self, name: &str, from_ref: &str) -> Result<()> {
        // Idempotent: leave an existing branch in place (the lifecycle re-enters
        // a station whose branch already forked).
        if self.branch_exists(name)? {
            return Ok(());
        }
        self.run(&["branch", name, from_ref])?;
        Ok(())
    }

    fn is_ancestor(&self, maybe_ancestor: &str, descendant: &str) -> Result<bool> {
        // `merge-base --is-ancestor` exits 0 when true, 1 when false.
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(["merge-base", "--is-ancestor", maybe_ancestor, descendant])
            .output()
            .map_err(GitError::BareIo)?;
        Ok(output.status.success())
    }

    fn merge_no_commit(&self, worktree_path: &Path, source_ref: &str) -> Result<MergeOutcome> {
        let wt = worktree_path.to_string_lossy().to_string();
        // `--no-commit --no-ff` stages the merge and leaves MERGE_HEAD set when
        // a merge actually started; "Already up to date" exits 0, sets nothing.
        let merge_err = run_in(&wt, &["merge", source_ref, "--no-ff", "--no-commit"]).err();

        if !self.merge_in_progress(worktree_path)? {
            // Either a clean no-op (already up to date) or a hard pre-merge
            // refusal (e.g. dirty tree). If git threw with no merge started,
            // surface it as a non-ok outcome.
            if let Some(err) = merge_err {
                return Ok(MergeOutcome {
                    ok: false,
                    performed: false,
                    conflict_paths: Vec::new(),
                    message: Some(err.to_string()),
                });
            }
            return Ok(MergeOutcome {
                ok: true,
                performed: false,
                conflict_paths: Vec::new(),
                message: None,
            });
        }

        // A merge is staged. The caller (engine_protected_merge) re-asserts
        // engine state, re-scans conflicts, then commits.
        Ok(MergeOutcome {
            ok: true,
            performed: true,
            conflict_paths: Vec::new(),
            message: None,
        })
    }

    fn merge_in_progress(&self, worktree_path: &Path) -> Result<bool> {
        let wt = worktree_path.to_string_lossy().to_string();
        Ok(run_in(&wt, &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"])
            .map(|o| !o.trim().is_empty())
            .unwrap_or(false))
    }

    fn checkout_paths(&self, worktree_path: &Path, from_ref: &str, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let wt = worktree_path.to_string_lossy().to_string();
        let mut args: Vec<&str> = vec!["checkout", from_ref, "--"];
        args.extend(paths.iter().map(|s| s.as_str()));
        // Best-effort per the reference: a no-op checkout on an absent path
        // must not abort the restore sweep.
        let _ = run_in(&wt, &args);
        Ok(())
    }

    fn add_paths(&self, worktree_path: &Path, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let wt = worktree_path.to_string_lossy().to_string();
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(paths.iter().map(|s| s.as_str()));
        let _ = run_in(&wt, &args);
        Ok(())
    }

    fn commit(&self, worktree_path: &Path, message: &str) -> Result<()> {
        let wt = worktree_path.to_string_lossy().to_string();
        run_in(&wt, &["commit", "--no-edit", "-m", message])?;
        Ok(())
    }

    fn ls_tree(&self, worktree_path: &Path, from_ref: &str, prefix: &str) -> Result<Vec<String>> {
        let wt = worktree_path.to_string_lossy().to_string();
        let out = run_in(
            &wt,
            &["ls-tree", "-r", "--name-only", from_ref, "--", prefix],
        )
        .unwrap_or_default();
        Ok(out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn unresolved_paths(&self, worktree_path: &Path) -> Result<Vec<String>> {
        let wt = worktree_path.to_string_lossy().to_string();
        let out = run_in(&wt, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        Ok(out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    }
}

/// Run `git -C <dir> <args>` against an arbitrary working tree (not the
/// backend's bound repo root) — the merge/restore/commit trio operates inside a
/// station/run worktree, not the primary checkout.
fn run_in(dir: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(GitError::BareIo)?;
    if !output.status.success() {
        return Err(GitError::Command {
            args: args.iter().map(|s| s.to_string()).collect(),
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse `git worktree list --porcelain` output into [`WorktreeInfo`] records.
fn parse_worktree_list(raw: &str) -> Vec<WorktreeInfo> {
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut locked = false;
    let mut detached = false;

    let flush = |path: &mut Option<PathBuf>,
                     branch: &mut Option<String>,
                     locked: &mut bool,
                     detached: &mut bool,
                     out: &mut Vec<WorktreeInfo>| {
        if let Some(p) = path.take() {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            out.push(WorktreeInfo {
                name,
                path: p,
                branch: branch.take(),
                locked: *locked,
            });
        }
        *locked = false;
        *detached = false;
    };

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            // A new record begins; flush any in-progress one first.
            flush(&mut path, &mut branch, &mut locked, &mut detached, &mut out);
            path = Some(PathBuf::from(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            // git emits a full ref like `refs/heads/feature`.
            branch = Some(
                rest.trim()
                    .strip_prefix("refs/heads/")
                    .unwrap_or(rest.trim())
                    .to_string(),
            );
        } else if line.starts_with("detached") {
            detached = true;
            branch = None;
        } else if line.starts_with("locked") {
            locked = true;
        } else if line.is_empty() {
            flush(&mut path, &mut branch, &mut locked, &mut detached, &mut out);
        }
    }
    // Trailing record with no blank-line terminator.
    flush(&mut path, &mut branch, &mut locked, &mut detached, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_input_yields_nothing() {
        assert!(parse_worktree_list("").is_empty());
        assert!(parse_worktree_list("\n\n").is_empty());
    }

    #[test]
    fn parse_single_branch_record() {
        let raw = "worktree /repo\nHEAD abc123\nbranch refs/heads/main\n";
        let list = parse_worktree_list(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, PathBuf::from("/repo"));
        assert_eq!(list[0].name, "repo");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert!(!list[0].locked);
    }

    #[test]
    fn parse_strips_refs_heads_prefix() {
        let raw = "worktree /repo\nbranch refs/heads/feature/nested\n";
        let list = parse_worktree_list(raw);
        assert_eq!(list[0].branch.as_deref(), Some("feature/nested"));
    }

    #[test]
    fn parse_detached_has_no_branch() {
        let raw = "worktree /repo/wt\nHEAD deadbeef\ndetached\n";
        let list = parse_worktree_list(raw);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].branch, None);
    }

    #[test]
    fn parse_locked_sets_flag() {
        let raw = "worktree /repo/wt\nbranch refs/heads/x\nlocked\n";
        let list = parse_worktree_list(raw);
        assert!(list[0].locked, "locked flag should be set");
    }

    #[test]
    fn parse_locked_with_reason() {
        // git emits `locked <reason>` when a reason was given.
        let raw = "worktree /repo/wt\nbranch refs/heads/x\nlocked on purpose\n";
        let list = parse_worktree_list(raw);
        assert!(list[0].locked, "locked-with-reason still sets the flag");
    }

    #[test]
    fn parse_multiple_records_separated_by_blank_lines() {
        let raw = "worktree /repo\nbranch refs/heads/main\n\n\
                   worktree /repo/wt-a\nbranch refs/heads/feature/a\n\n\
                   worktree /repo/wt-b\nHEAD cafe\ndetached\nlocked\n";
        let list = parse_worktree_list(raw);
        assert_eq!(list.len(), 3, "three records: {list:?}");

        assert_eq!(list[0].branch.as_deref(), Some("main"));
        assert!(!list[0].locked);

        assert_eq!(list[1].name, "wt-a");
        assert_eq!(list[1].branch.as_deref(), Some("feature/a"));

        assert_eq!(list[2].name, "wt-b");
        assert_eq!(list[2].branch, None, "detached => no branch");
        assert!(list[2].locked);
    }

    #[test]
    fn parse_trailing_record_without_blank_line() {
        // No terminating blank line on the final record — the flush at the end
        // must still emit it.
        let raw = "worktree /a\nbranch refs/heads/one\n\nworktree /b\nbranch refs/heads/two";
        let list = parse_worktree_list(raw);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].branch.as_deref(), Some("two"));
    }

    #[test]
    fn parse_state_resets_between_records() {
        // A locked+detached record must not bleed its flags into the next one.
        let raw = "worktree /a\nHEAD x\ndetached\nlocked\n\n\
                   worktree /b\nbranch refs/heads/clean\n";
        let list = parse_worktree_list(raw);
        assert_eq!(list.len(), 2);
        assert!(list[0].locked);
        assert_eq!(list[0].branch, None);
        // Second record is neither locked nor detached.
        assert!(!list[1].locked, "locked flag must reset");
        assert_eq!(list[1].branch.as_deref(), Some("clean"));
    }
}

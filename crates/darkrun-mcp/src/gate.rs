//! Pre-checkpoint gate review (the `darkrun-gate-review` skill).
//!
//! Computes the current diff and hands back review instructions so the agent
//! can run a deliberate code review before a station's checkpoint locks. The
//! manager stays a pure read; the diff is read in-process via the pure-Rust
//! git backend (no `git` CLI).

use std::path::Path;

use darkrun_git::{Git, GitBackend};
use serde::Serialize;

/// The diff to review and the instructions for reviewing it.
#[derive(Debug, Clone, Serialize)]
pub struct GateReview {
    /// `git diff --stat` against HEAD (the changed-files summary).
    pub stat: String,
    /// The unified diff against HEAD, truncated if very large.
    pub diff: String,
    /// Whether the diff was truncated.
    pub truncated: bool,
    /// How to run the review.
    pub instructions: String,
}

/// The largest diff we inline before truncating (keeps the tool result bounded).
const MAX_DIFF: usize = 60_000;

/// Compute the gate review for the working tree at `repo_root`.
pub fn gate_review(repo_root: &Path) -> GateReview {
    // Read the diff in-process; an unopenable repo yields an empty review.
    let git = Git::open(repo_root).ok();
    let stat = git
        .as_ref()
        .and_then(|g| g.diff_stat("HEAD").ok())
        .unwrap_or_default();
    let full = git
        .as_ref()
        .and_then(|g| g.diff("HEAD").ok())
        .unwrap_or_default();
    let truncated = full.len() > MAX_DIFF;
    let diff = if truncated {
        let mut d: String = full.chars().take(MAX_DIFF).collect();
        d.push_str("\n… [diff truncated — review the remaining files directly with `git diff`]");
        d
    } else {
        full
    };
    let instructions = if stat.trim().is_empty() {
        "No uncommitted changes to review. If the work is already committed, review the \
         station's commits with `git show` / `git diff <base>..HEAD` instead, then decide the \
         checkpoint."
            .to_string()
    } else {
        "Review this diff before the checkpoint locks:\n\
         1. Dispatch the station's Reviewers, each over its own lens (correctness, security, \
         maintainability, regressions). A reviewer reviews — it does not redesign or relitigate \
         the spec.\n\
         2. For each finding, file feedback with `darkrun_feedback_create` and dispatch a \
         fix-worker; re-run the affected checks.\n\
         3. Repeat until the Reviewers come back clean or the operator accepts the remainder.\n\
         4. Then decide the checkpoint with `darkrun_checkpoint_decide`."
            .to_string()
    };
    GateReview {
        stat,
        diff,
        truncated,
        instructions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_review_handles_a_non_git_dir() {
        // In a dir with no git, git output is empty → "no changes" guidance.
        let dir = tempfile::tempdir().unwrap();
        let r = gate_review(dir.path());
        assert!(r.stat.is_empty());
        assert!(!r.truncated);
        assert!(r.instructions.contains("No uncommitted changes"));
    }

    #[test]
    fn gate_review_summarizes_real_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        // Uncommitted change.
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        let r = gate_review(root);
        assert!(r.stat.contains("a.txt"));
        assert!(r.diff.contains("+two"));
        assert!(r.instructions.contains("Dispatch the station's Reviewers"));
    }

    #[test]
    fn gate_review_truncates_a_very_large_diff() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            std::process::Command::new("git").current_dir(root).args(args).output().unwrap();
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("big.txt"), "seed\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        // A working-tree change whose diff comfortably exceeds MAX_DIFF (60k).
        let huge: String = (0..8000).map(|i| format!("line {i} added\n")).collect();
        std::fs::write(root.join("big.txt"), huge).unwrap();

        let r = gate_review(root);
        assert!(r.truncated, "a >60k diff is flagged truncated");
        assert!(r.diff.contains("diff truncated"));
    }
}

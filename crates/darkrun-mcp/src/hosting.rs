//! The hosting client — open a draft PR/MR for a discrete station and detect
//! when a human has merged it.
//!
//! DISCRETE mode resolves each station's Checkpoint on a **human PR/MR merge**
//! rather than in-process. To do that the manager needs two best-effort
//! capabilities against the project's hosting provider:
//!
//! 1. **open** a draft change request (`darkrun/<slug>/<station>` ->
//!    `darkrun/<slug>/main`) when the station reaches its gate, recording a
//!    provider ref on `Station.pr_ref`; and
//! 2. **poll** that ref on each tick to see if it has been merged — the signal
//!    that advances the station.
//!
//! This wraps the `gh` (GitHub) / `glab` (GitLab) CLIs through a small
//! [`Hosting`] seam so the manager stays testable: the CLI implementation
//! ([`CliHosting`]) shells out, while tests inject a mock. Every call is
//! **best-effort** — when no CLI / no hosting is configured the client reports
//! the absence cleanly and the manager falls back to an await gate the operator
//! resolves by hand (it never crashes the tick).

use std::path::{Path, PathBuf};
use std::process::Command;

/// What the manager wants done at a discrete station's gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    /// The PR/MR head branch (`darkrun/<slug>/<station>`).
    pub head: String,
    /// The PR/MR base branch (`darkrun/<slug>/main`).
    pub base: String,
    /// The change-request title.
    pub title: String,
    /// The change-request body (markdown).
    pub body: String,
}

/// A human's review note pulled back off an opened change request — a PR/MR
/// comment or a review verdict. C6 turns each of these into darkrun feedback the
/// fix track addresses, so a reviewer's "please change X" on the PR re-enters the
/// run as work rather than dying on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    /// A provider-stable id for this note (GitHub node id / GitLab note id),
    /// prefixed by kind (`c<id>` issue comment, `r<id>` review). Drives dedup:
    /// the feedback minted from it carries a deterministic id derived from this,
    /// so re-polling the same PR never double-files.
    pub id: String,
    /// The note author's handle (for the feedback body's provenance line).
    pub author: String,
    /// The note's markdown text.
    pub body: String,
    /// Whether this note is a **change request** (a GitHub `CHANGES_REQUESTED`
    /// review) — filed as a blocker, versus a plain comment filed as medium.
    pub change_request: bool,
}

/// The merge state of an opened change request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeState {
    /// Open and not yet merged — the gate still holds.
    Open,
    /// Merged by a human — the gate resolves and the station advances.
    Merged,
    /// Closed without merging — the gate is treated as a hold (no advance).
    Closed,
    /// The ref could not be resolved (transient / unknown) — treated as a hold.
    Unknown,
}

/// The hosting seam: open a draft change request and poll its merge state. The
/// manager depends on this trait, not the CLI, so tests inject a mock.
pub trait Hosting {
    /// Whether a usable hosting client is available (a CLI on PATH + a remote).
    /// When `false` the manager skips the PR path and falls back to an await
    /// gate the operator resolves manually.
    fn available(&self) -> bool;

    /// Open a draft change request for `req`, returning its provider ref (a
    /// number or URL) on success. `None` (best-effort) when the open failed —
    /// the manager then surfaces an await fallback rather than crashing.
    fn open_draft(&self, req: &OpenRequest) -> Option<String>;

    /// Poll the merge state of a previously-opened change request `pr_ref`.
    fn merge_state(&self, pr_ref: &str) -> MergeState;

    /// Whether `pr_ref` is still a DRAFT (not yet marked ready for review).
    /// `Some(true)`/`Some(false)` when known, `None` when unknown or the
    /// provider doesn't report it. Drives the draft→ready transition in the
    /// PR lifecycle (G4); defaults to `None` so a client that can't tell simply
    /// leaves the status at `draft` until it merges.
    fn is_draft(&self, _pr_ref: &str) -> Option<bool> {
        None
    }

    /// Post a markdown `body` as a comment on `pr_ref`, returning `true` on a
    /// confirmed post. Used to attach the station's objective proof to the
    /// change request as a durable, linkable asset (D5). Defaults to `false`
    /// (no-op) so a client that can't comment simply skips the upload.
    fn comment(&self, _pr_ref: &str, _body: &str) -> bool {
        false
    }

    /// Pull the human review notes off `pr_ref` — PR/MR comments and review
    /// verdicts (C6). The discrete poll files each NEW one as `external`-origin
    /// feedback the fix track addresses, so a reviewer's change-request on the
    /// remote re-enters the run as work. Defaults to empty (best-effort) so a
    /// client that can't fetch simply surfaces no remote feedback.
    fn review_comments(&self, _pr_ref: &str) -> Vec<ReviewComment> {
        Vec::new()
    }
}

/// The CLI-backed hosting client: shells `gh` / `glab` against a repo root.
///
/// Provider selection mirrors `darkrun-setup`'s `hosting:` detection — `github`
/// drives `gh`, `gitlab` drives `glab`. An unknown / absent provider yields an
/// [`CliHosting`] that reports [`available`](Hosting::available) `== false`.
pub struct CliHosting {
    repo_root: PathBuf,
    provider: Provider,
}

/// The hosting provider a [`CliHosting`] drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    GitHub,
    GitLab,
    None,
}

impl CliHosting {
    /// Build a CLI hosting client for `repo_root`, resolving the provider from
    /// `.darkrun/settings.yml`'s `hosting:` line (written by `darkrun-setup`),
    /// falling back to the git remote URL when settings are absent.
    pub fn resolve(repo_root: &Path) -> Self {
        let provider = resolve_provider(repo_root);
        Self {
            repo_root: repo_root.to_path_buf(),
            provider,
        }
    }

    /// Run the provider CLI in the repo root, returning trimmed stdout on a
    /// zero exit, or `None` on any failure (missing binary, non-zero exit).
    ///
    /// `gh`/`glab` have no `-C` flag, so the working directory is set with
    /// `current_dir` rather than a CLI argument.
    #[cfg(not(tarpaulin_include))] // spawns gh/glab — irreducible network/process I/O
    fn run(&self, bin: &str, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use std::process::Stdio;
        use std::time::Duration;
        use wait_timeout::ChildExt;

        // `gh`/`glab` make network/API calls (and can prompt for auth) — a hard
        // wall-clock ceiling so an unresponsive host can't wedge a tick. Prompts
        // are suppressed so they fail fast rather than block on input.
        const HOST_TIMEOUT: Duration = Duration::from_secs(60);

        let mut child = Command::new(bin)
            .args(args)
            .current_dir(&self.repo_root)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;

        let status = match child.wait_timeout(HOST_TIMEOUT).ok()? {
            Some(status) => status,
            None => {
                // Unresponsive host — kill and report failure (best-effort: the
                // caller falls back to an await gate).
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        };
        if !status.success() {
            return None;
        }
        let mut stdout = String::new();
        child.stdout.take()?.read_to_string(&mut stdout).ok()?;
        Some(stdout.trim().to_string())
    }
}

impl Hosting for CliHosting {
    fn available(&self) -> bool {
        match self.provider {
            Provider::GitHub => binary_on_path("gh"),
            Provider::GitLab => binary_on_path("glab"),
            Provider::None => false,
        }
    }

    fn open_draft(&self, req: &OpenRequest) -> Option<String> {
        match self.provider {
            Provider::GitHub => self.run(
                "gh",
                &[
                    "pr", "create", "--draft", "--head", &req.head, "--base", &req.base, "--title",
                    &req.title, "--body", &req.body,
                ],
            ),
            Provider::GitLab => self.run(
                "glab",
                &[
                    "mr",
                    "create",
                    "--draft",
                    "--source-branch",
                    &req.head,
                    "--target-branch",
                    &req.base,
                    "--title",
                    &req.title,
                    "--description",
                    &req.body,
                    "--yes",
                ],
            ),
            Provider::None => None,
        }
    }

    #[cfg(not(tarpaulin_include))] // shells out to gh/glab — irreducible network I/O
    fn merge_state(&self, pr_ref: &str) -> MergeState {
        match self.provider {
            Provider::GitHub => {
                // `gh pr view <ref> --json state` → {"state":"MERGED"|"OPEN"|"CLOSED"}.
                let raw = self.run("gh", &["pr", "view", pr_ref, "--json", "state"]);
                match raw {
                    Some(json) => parse_github_state(&json),
                    None => MergeState::Unknown,
                }
            }
            Provider::GitLab => {
                // `glab mr view <ref> -F json` → {"state":"merged"|"opened"|"closed"}.
                let raw = self.run("glab", &["mr", "view", pr_ref, "-F", "json"]);
                match raw {
                    Some(json) => parse_gitlab_state(&json),
                    None => MergeState::Unknown,
                }
            }
            Provider::None => MergeState::Unknown,
        }
    }

    #[cfg(not(tarpaulin_include))] // shells out to gh/glab — irreducible network I/O
    fn is_draft(&self, pr_ref: &str) -> Option<bool> {
        match self.provider {
            Provider::GitHub => {
                // `gh pr view <ref> --json isDraft` → {"isDraft": true|false}.
                let raw = self.run("gh", &["pr", "view", pr_ref, "--json", "isDraft"])?;
                parse_bool_field(&raw, "isDraft")
            }
            Provider::GitLab => {
                // GitLab MRs carry a `draft` boolean in their JSON view.
                let raw = self.run("glab", &["mr", "view", pr_ref, "-F", "json"])?;
                parse_bool_field(&raw, "draft")
            }
            Provider::None => None,
        }
    }

    #[cfg(not(tarpaulin_include))] // shells out to gh/glab — irreducible network I/O
    fn comment(&self, pr_ref: &str, body: &str) -> bool {
        match self.provider {
            Provider::GitHub => self
                .run("gh", &["pr", "comment", pr_ref, "--body", body])
                .is_some(),
            Provider::GitLab => self
                .run("glab", &["mr", "note", pr_ref, "--message", body])
                .is_some(),
            Provider::None => false,
        }
    }

    #[cfg(not(tarpaulin_include))] // shells out to gh/glab — irreducible network I/O
    fn review_comments(&self, pr_ref: &str) -> Vec<ReviewComment> {
        match self.provider {
            Provider::GitHub => {
                // `gh pr view <ref> --json comments,reviews` → issue comments +
                // review verdicts (with `state`). Both carry a stable node `id`.
                match self.run("gh", &["pr", "view", pr_ref, "--json", "comments,reviews"]) {
                    Some(json) => parse_github_review_comments(&json),
                    None => Vec::new(),
                }
            }
            Provider::GitLab => {
                // `glab mr view <ref> -F json` carries the MR's notes when the
                // provider includes them; parse defensively (best-effort).
                match self.run("glab", &["mr", "view", pr_ref, "-F", "json"]) {
                    Some(json) => parse_gitlab_notes(&json),
                    None => Vec::new(),
                }
            }
            Provider::None => Vec::new(),
        }
    }
}

/// Render a JSON value's `id` field as a stable string (GitHub uses string node
/// ids, GitLab uses integer note ids — accept either).
fn json_id(v: &serde_json::Value) -> Option<String> {
    match v.get("id")? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Parse `gh pr view --json comments,reviews` into review notes (C6).
///
/// Issue `comments[]` become plain comments (`c<id>`); `reviews[]` become notes
/// keyed `r<id>`, with `CHANGES_REQUESTED` flagged as a change request and
/// `APPROVED`/`DISMISSED`/`PENDING` reviews with an empty body skipped (no
/// actionable content — an approval isn't work).
fn parse_github_review_comments(json: &str) -> Vec<ReviewComment> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    if let Some(comments) = root.get("comments").and_then(|v| v.as_array()) {
        for c in comments {
            let (Some(id), Some(body)) = (json_id(c), c.get("body").and_then(|b| b.as_str()))
            else {
                continue;
            };
            if body.trim().is_empty() {
                continue;
            }
            out.push(ReviewComment {
                id: format!("c{id}"),
                author: github_author(c),
                body: body.to_string(),
                change_request: false,
            });
        }
    }

    if let Some(reviews) = root.get("reviews").and_then(|v| v.as_array()) {
        for r in reviews {
            let Some(id) = json_id(r) else { continue };
            let state = r
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_ascii_uppercase();
            let body = r.get("body").and_then(|b| b.as_str()).unwrap_or("");
            let is_change = state == "CHANGES_REQUESTED";
            // An approval / dismissal / empty comment carries no work — skip it.
            // A change request with no body still files (the verdict IS the ask).
            if body.trim().is_empty() && !is_change {
                continue;
            }
            let text = if body.trim().is_empty() {
                "Reviewer requested changes (no inline summary).".to_string()
            } else {
                body.to_string()
            };
            out.push(ReviewComment {
                id: format!("r{id}"),
                author: github_author(r),
                body: text,
                change_request: is_change,
            });
        }
    }

    out
}

/// Pull `author.login` from a GitHub comment/review object (default `unknown`).
fn github_author(v: &serde_json::Value) -> String {
    v.get("author")
        .and_then(|a| a.get("login"))
        .and_then(|l| l.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Parse GitLab MR notes out of `glab mr view -F json` (best-effort).
///
/// glab capitalizes struct fields (`Notes`); accept either case. System notes
/// (state changes, label edits) carry `system: true` and are skipped — only a
/// human's discussion becomes feedback. GitLab has no per-note "changes
/// requested" verdict, so every human note files as a plain comment.
fn parse_gitlab_notes(json: &str) -> Vec<ReviewComment> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let notes = root
        .get("Notes")
        .or_else(|| root.get("notes"))
        .and_then(|v| v.as_array());
    let Some(notes) = notes else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for n in notes {
        // Skip system notes (automated activity, not a human comment).
        let system = n
            .get("system")
            .or_else(|| n.get("System"))
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        if system {
            continue;
        }
        let (Some(id), Some(body)) = (
            json_id(n),
            n.get("body")
                .or_else(|| n.get("Body"))
                .and_then(|b| b.as_str()),
        ) else {
            continue;
        };
        if body.trim().is_empty() {
            continue;
        }
        let author = n
            .get("author")
            .or_else(|| n.get("Author"))
            .and_then(|a| a.get("username").or_else(|| a.get("Username")))
            .and_then(|u| u.as_str())
            .unwrap_or("unknown")
            .to_string();
        out.push(ReviewComment {
            id: format!("c{id}"),
            author,
            body: body.to_string(),
            change_request: false,
        });
    }
    out
}

/// Extract a top-level boolean `field` from a flat JSON object body. `None` when
/// the body doesn't parse or the field is absent/non-boolean.
fn parse_bool_field(json: &str, field: &str) -> Option<bool> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get(field)?
        .as_bool()
}

/// Resolve the hosting provider for `repo_root` from `.darkrun/settings.yml`'s
/// `hosting:` line, falling back to the git remote URL.
fn resolve_provider(repo_root: &Path) -> Provider {
    // 1. settings.yml `hosting:` (the canonical, setup-written source).
    let settings = repo_root.join(".darkrun").join("settings.yml");
    if let Ok(raw) = std::fs::read_to_string(&settings) {
        for line in raw.lines() {
            if let Some(value) = line.trim().strip_prefix("hosting:") {
                match value.trim().trim_matches(['"', '\'']).trim() {
                    "github" => return Provider::GitHub,
                    "gitlab" => return Provider::GitLab,
                    "none" | "" => {}
                    _ => {}
                }
            }
        }
    }
    // 2. Fall back to the git remote URL.
    let remote = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if remote.contains("github.com") {
        Provider::GitHub
    } else if remote.contains("gitlab") {
        Provider::GitLab
    } else {
        Provider::None
    }
}

/// Whether a binary is resolvable on `PATH` (a cheap `--version` probe).
#[cfg(not(tarpaulin_include))] // probes PATH by spawning the CLI — irreducible process I/O
fn binary_on_path(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse the merge state out of `gh pr view --json state` output.
fn parse_github_state(json: &str) -> MergeState {
    let lower = json.to_ascii_lowercase();
    if lower.contains("\"merged\"") {
        MergeState::Merged
    } else if lower.contains("\"open\"") {
        MergeState::Open
    } else if lower.contains("\"closed\"") {
        MergeState::Closed
    } else {
        MergeState::Unknown
    }
}

/// Parse the merge state out of `glab mr view -F json` output.
fn parse_gitlab_state(json: &str) -> MergeState {
    let lower = json.to_ascii_lowercase();
    if lower.contains("\"merged\"") {
        MergeState::Merged
    } else if lower.contains("\"opened\"") {
        MergeState::Open
    } else if lower.contains("\"closed\"") {
        MergeState::Closed
    } else {
        MergeState::Unknown
    }
}

/// The outcome of a push-with-NFF-recovery attempt (mechanic #7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// The branch is on origin (pushed, or already up to date).
    Pushed,
    /// Push failed and was not recoverable (or recovery failed). `note` carries
    /// why — best-effort: the caller reports, never crashes the tick.
    Failed { note: String },
}

/// Whether a push-rejection stderr is a GENUINE non-fast-forward (mechanic #7).
///
/// Narrow on purpose: a bare "rejected" also matches protected-branch /
/// pre-receive-hook / permission failures, where rebasing is the WRONG recovery.
/// We only rebase+retry on the three phrasings git uses for a true NFF.
pub fn is_non_fast_forward(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("non-fast-forward")
        || lower.contains("fetch first")
        || lower.contains("behind the remote")
}

/// Push `branch`'s head to origin with non-fast-forward recovery (mechanic #7).
///
/// 1. Try `push origin HEAD:refs/heads/<branch>` from the worktree on `branch`.
/// 2. On a NARROW NFF rejection ([`is_non_fast_forward`]): `fetch origin
///    <branch>` -> `rebase origin/<branch>` -> retry the push once. A rebase
///    failure aborts the rebase and reports — never leaves a half-rebase.
/// 3. On a non-NFF rejection (protected branch / hook / permission), report
///    WITHOUT rebasing — rebasing those would be wrong.
///
/// Best-effort: returns a [`PushOutcome`] and never panics. `worktree_path` is a
/// checkout on `branch` (the engine forks one per station).
pub fn push_head_with_nff_recovery(
    git: &dyn darkrun_git::GitBackend,
    worktree_path: &Path,
    branch: &str,
) -> PushOutcome {
    let first = git.push(worktree_path, branch);
    let Err(err) = first else {
        return PushOutcome::Pushed;
    };
    let stderr = match &err {
        darkrun_git::GitError::Command { stderr, .. } => stderr.clone(),
        other => other.to_string(),
    };
    if !is_non_fast_forward(&stderr) {
        // Protected-branch / hook / permission / other — do NOT rebase.
        return PushOutcome::Failed {
            note: format!("push to origin/{branch} rejected (no rebase): {stderr}"),
        };
    }

    // Genuine NFF: fetch + rebase onto origin/<branch>, then retry once.
    let _ = git.fetch(worktree_path, branch);
    let upstream = format!("origin/{branch}");
    if let Err(e) = git.rebase_onto(worktree_path, &upstream) {
        let _ = git.rebase_abort(worktree_path);
        return PushOutcome::Failed {
            note: format!("non-fast-forward; rebase onto {upstream} failed: {e}"),
        };
    }
    match git.push(worktree_path, branch) {
        Ok(()) => PushOutcome::Pushed,
        Err(e) => PushOutcome::Failed {
            note: format!("non-fast-forward; retry push after rebase failed: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nff_matcher_fires_only_on_genuine_nff() {
        // Genuine NFF phrasings → recover.
        assert!(is_non_fast_forward(
            "! [rejected] main -> main (non-fast-forward)"
        ));
        assert!(is_non_fast_forward("Updates were rejected; fetch first"));
        assert!(is_non_fast_forward(
            "tip of your current branch is behind the remote"
        ));
        // NOT a NFF: a bare 'rejected', protected branch, hook, permission.
        assert!(!is_non_fast_forward("! [remote rejected] main -> main"));
        assert!(!is_non_fast_forward(
            "remote: GH006: Protected branch update failed"
        ));
        assert!(!is_non_fast_forward("pre-receive hook declined"));
        assert!(!is_non_fast_forward("permission denied"));
    }

    #[test]
    fn github_state_parses_merged() {
        assert_eq!(parse_github_state(r#"{"state":"MERGED"}"#), MergeState::Merged);
        assert_eq!(parse_github_state(r#"{"state":"OPEN"}"#), MergeState::Open);
        assert_eq!(parse_github_state(r#"{"state":"CLOSED"}"#), MergeState::Closed);
        assert_eq!(parse_github_state("garbage"), MergeState::Unknown);
    }

    #[test]
    fn gitlab_state_parses_merged() {
        assert_eq!(parse_gitlab_state(r#"{"state":"merged"}"#), MergeState::Merged);
        assert_eq!(parse_gitlab_state(r#"{"state":"opened"}"#), MergeState::Open);
        assert_eq!(parse_gitlab_state(r#"{"state":"closed"}"#), MergeState::Closed);
    }

    #[test]
    fn github_review_comments_split_change_requests_from_comments() {
        let json = r#"{
            "comments": [
                {"id": "IC_1", "author": {"login": "bob"}, "body": "nit: rename this"},
                {"id": "IC_2", "author": {"login": "x"}, "body": "   "}
            ],
            "reviews": [
                {"id": "PRR_9", "author": {"login": "alice"}, "body": "needs a metric", "state": "CHANGES_REQUESTED"},
                {"id": "PRR_8", "author": {"login": "carol"}, "body": "", "state": "APPROVED"},
                {"id": "PRR_7", "author": {"login": "dave"}, "body": "", "state": "CHANGES_REQUESTED"}
            ]
        }"#;
        let got = parse_github_review_comments(json);
        // The empty issue comment and the empty APPROVED review are dropped; the
        // two change requests + the one real comment survive.
        assert_eq!(got.len(), 3, "got {got:?}");
        let cr = got.iter().find(|c| c.id == "rPRR_9").unwrap();
        assert!(cr.change_request);
        assert_eq!(cr.author, "alice");
        assert_eq!(cr.body, "needs a metric");
        // An empty-body change request still files, with a synthetic ask.
        let empty_cr = got.iter().find(|c| c.id == "rPRR_7").unwrap();
        assert!(empty_cr.change_request);
        assert!(empty_cr.body.contains("requested changes"));
        // The issue comment is keyed `c<id>` and not a change request.
        let comment = got.iter().find(|c| c.id == "cIC_1").unwrap();
        assert!(!comment.change_request);
        // The APPROVED review with no body contributes nothing.
        assert!(got.iter().all(|c| c.id != "rPRR_8"));
    }

    #[test]
    fn github_review_comments_empty_on_garbage() {
        assert!(parse_github_review_comments("not json").is_empty());
        assert!(parse_github_review_comments("{}").is_empty());
    }

    #[test]
    fn gitlab_notes_skip_system_and_capitalize_either_case() {
        let json = r#"{
            "Notes": [
                {"id": 11, "system": true, "body": "changed the description", "author": {"username": "gitbot"}},
                {"id": 12, "system": false, "body": "please add a test", "author": {"username": "erin"}},
                {"id": 13, "system": false, "body": "  ", "author": {"username": "x"}}
            ]
        }"#;
        let got = parse_gitlab_notes(json);
        assert_eq!(got.len(), 1, "only the one human, non-empty note survives: {got:?}");
        assert_eq!(got[0].id, "c12");
        assert_eq!(got[0].author, "erin");
        assert!(!got[0].change_request, "gitlab notes file as plain comments");
    }

    #[test]
    fn gitlab_notes_empty_when_absent() {
        assert!(parse_gitlab_notes(r#"{"state":"opened"}"#).is_empty());
        assert!(parse_gitlab_notes("garbage").is_empty());
    }

    #[test]
    fn provider_resolves_from_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".darkrun")).unwrap();
        std::fs::write(
            root.join(".darkrun").join("settings.yml"),
            "hosting: gitlab\ndefault_branch: main\n",
        )
        .unwrap();
        assert_eq!(resolve_provider(root), Provider::GitLab);
    }

    #[test]
    fn provider_none_when_no_hosting_and_no_remote() {
        let dir = tempfile::tempdir().unwrap();
        // No settings, no git remote → None (the await-fallback case).
        assert_eq!(resolve_provider(dir.path()), Provider::None);
    }

    /// A `none`-provider CLI client is never available, so the manager takes the
    /// await fallback rather than attempting a PR.
    #[test]
    fn none_provider_client_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let client = CliHosting::resolve(dir.path());
        assert!(!client.available());
        assert!(client
            .open_draft(&OpenRequest {
                head: "h".into(),
                base: "b".into(),
                title: "t".into(),
                body: "b".into(),
            })
            .is_none());
        assert_eq!(client.merge_state("1"), MergeState::Unknown);
    }

    /// Build a `CliHosting` whose provider is forced via a settings file, in a
    /// throwaway non-git directory so every CLI call fails fast (no repo / no
    /// auth) and returns its best-effort fallback — exercising the GitHub and
    /// GitLab dispatch arms without a real `gh`/`glab` round-trip.
    fn forced(provider: &str) -> (tempfile::TempDir, CliHosting) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".darkrun")).unwrap();
        std::fs::write(
            dir.path().join(".darkrun").join("settings.yml"),
            format!("hosting: {provider}\n"),
        )
        .unwrap();
        let client = CliHosting::resolve(dir.path());
        (dir, client)
    }

    /// Every provider dispatch arm runs and degrades gracefully to its fallback
    /// when the CLI can't act (non-repo dir): `open_draft` -> None, `merge_state`
    /// -> Unknown, `is_draft` -> None, `comment` -> false, `review_comments` ->
    /// empty. Covers the GitHub + GitLab arms regardless of whether the binaries
    /// are installed on the host.
    #[test]
    fn provider_arms_degrade_to_fallback_without_a_live_cli() {
        let req = OpenRequest {
            head: "darkrun/x/frame".into(),
            base: "darkrun/x/main".into(),
            title: "t".into(),
            body: "b".into(),
        };
        for provider in ["github", "gitlab"] {
            let (_d, client) = forced(provider);
            // `available()` probes the CLI binary; either answer is fine, the arm runs.
            let _ = client.available();
            assert!(client.open_draft(&req).is_none(), "{provider} open_draft");
            assert_eq!(client.merge_state("1"), MergeState::Unknown, "{provider} merge_state");
            assert_eq!(client.is_draft("1"), None, "{provider} is_draft");
            assert!(!client.comment("1", "hi"), "{provider} comment");
            assert!(client.review_comments("1").is_empty(), "{provider} review_comments");
        }
    }

    #[test]
    fn json_id_accepts_strings_and_numbers_only() {
        use serde_json::json;
        assert_eq!(json_id(&json!({"id": "abc"})).as_deref(), Some("abc"));
        assert_eq!(json_id(&json!({"id": 42})).as_deref(), Some("42"));
        assert!(json_id(&json!({"id": ""})).is_none()); // empty string
        assert!(json_id(&json!({"id": true})).is_none()); // a bool is not an id
        assert!(json_id(&json!({"other": 1})).is_none()); // absent
    }

    #[test]
    fn parse_bool_field_reads_or_rejects() {
        assert_eq!(parse_bool_field(r#"{"isDraft":true}"#, "isDraft"), Some(true));
        assert_eq!(parse_bool_field(r#"{"isDraft":false}"#, "isDraft"), Some(false));
        assert_eq!(parse_bool_field(r#"{"x":1}"#, "isDraft"), None); // absent
        assert_eq!(parse_bool_field(r#"{"isDraft":"yes"}"#, "isDraft"), None); // non-bool
        assert_eq!(parse_bool_field("not json", "isDraft"), None); // unparseable
    }

    #[test]
    fn gitlab_state_unknown_for_unrecognized() {
        assert_eq!(parse_gitlab_state(r#"{"state":"locked"}"#), MergeState::Unknown);
    }

    #[test]
    fn github_review_comments_skip_empty_and_keep_bodyless_change_requests() {
        let json = r#"{
            "comments": [
                {"id": 1, "body": "real note", "author": {"login": "a"}},
                {"id": 2, "body": "   ", "author": {"login": "b"}},
                {"body": "no id", "author": {"login": "c"}}
            ],
            "reviews": [
                {"id": 10, "state": "APPROVED", "body": "", "author": {"login": "d"}},
                {"id": 11, "state": "CHANGES_REQUESTED", "body": "", "author": {"login": "e"}}
            ]
        }"#;
        let got = parse_github_review_comments(json);
        // The real comment + the bodyless change-request survive; empty/idless drop.
        assert_eq!(got.len(), 2);
        let cr = got.iter().find(|c| c.change_request).expect("a change request files");
        assert!(cr.body.contains("requested changes"));
        assert_eq!(cr.id, "r11");
        assert_eq!(cr.author, "e");
        assert!(got.iter().any(|c| c.id == "c1" && c.author == "a"));
    }

    #[test]
    fn gitlab_notes_skip_idless_and_empty_bodies() {
        let json = r#"{"notes":[
            {"id": 5, "body": "keep me", "author": {"username": "u"}},
            {"body": "no id"},
            {"id": 6, "body": "  "}
        ]}"#;
        let got = parse_gitlab_notes(json);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "c5");
        assert_eq!(got[0].author, "u");
    }

    #[test]
    fn push_head_recovers_from_a_genuine_non_fast_forward() {
        use darkrun_git::Git;
        use std::process::Command;
        let g = |dir: &std::path::Path, args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success(), "git {args:?}");
        };
        let bare = tempfile::tempdir().unwrap();
        g(bare.path(), &["init", "-q", "--bare"]);
        let remote = bare.path().to_string_lossy().to_string();

        // Clone A: seed origin/main with one commit.
        let a = tempfile::tempdir().unwrap();
        g(a.path(), &["init", "-q", "-b", "main"]);
        g(a.path(), &["config", "user.email", "a@x.io"]); g(a.path(), &["config", "user.name", "A"]);
        std::fs::write(a.path().join("a.txt"), "1").unwrap();
        g(a.path(), &["add", "-A"]); g(a.path(), &["commit", "-qm", "base"]);
        g(a.path(), &["remote", "add", "origin", &remote]);
        g(a.path(), &["push", "-q", "origin", "main"]);

        // Clone B from origin, then A advances origin so B is behind (NFF on push).
        let b = tempfile::tempdir().unwrap();
        g(b.path(), &["clone", "-q", &remote, "."]);
        g(b.path(), &["config", "user.email", "b@x.io"]); g(b.path(), &["config", "user.name", "B"]);
        std::fs::write(a.path().join("a.txt"), "2").unwrap();
        g(a.path(), &["commit", "-aqm", "advance"]); g(a.path(), &["push", "-q", "origin", "main"]);

        // B commits on top of the stale base → its push is a genuine non-fast-forward;
        // the recovery fetches + rebases onto origin/main and retries successfully.
        std::fs::write(b.path().join("c.txt"), "x").unwrap();
        g(b.path(), &["add", "-A"]); g(b.path(), &["commit", "-qm", "b-work"]);
        let bgit = Git::open(b.path()).unwrap();
        assert!(matches!(
            push_head_with_nff_recovery(&bgit, b.path(), "main"),
            PushOutcome::Pushed
        ), "a genuine NFF is recovered by fetch+rebase+retry");
    }

    #[test]
    fn push_head_recovery_pushes_then_reports_a_non_nff_failure() {
        use darkrun_git::Git;
        use std::process::Command;
        let g = |dir: &std::path::Path, args: &[&str]| {
            assert!(Command::new("git").arg("-C").arg(dir).args(args).status().unwrap().success(), "git {args:?}");
        };
        // Bare remote + work repo with one commit.
        let bare = tempfile::tempdir().unwrap();
        g(bare.path(), &["init", "-q", "--bare"]);
        let work = tempfile::tempdir().unwrap();
        g(work.path(), &["init", "-q", "-b", "main"]);
        g(work.path(), &["config", "user.email", "t@x.io"]);
        g(work.path(), &["config", "user.name", "T"]);
        std::fs::write(work.path().join("a.txt"), "1").unwrap();
        g(work.path(), &["add", "-A"]); g(work.path(), &["commit", "-qm", "c1"]);
        g(work.path(), &["remote", "add", "origin", &bare.path().to_string_lossy()]);
        let git = Git::open(work.path()).unwrap();
        // Success: pushes HEAD -> origin/main.
        assert!(matches!(push_head_with_nff_recovery(&git, work.path(), "main"), PushOutcome::Pushed));
        // Non-NFF failure: a repo whose origin points nowhere fails without rebase.
        let broken = tempfile::tempdir().unwrap();
        g(broken.path(), &["init", "-q", "-b", "main"]);
        g(broken.path(), &["config", "user.email", "t@x.io"]);
        g(broken.path(), &["config", "user.name", "T"]);
        std::fs::write(broken.path().join("b.txt"), "1").unwrap();
        g(broken.path(), &["add", "-A"]); g(broken.path(), &["commit", "-qm", "c1"]);
        g(broken.path(), &["remote", "add", "origin", "/nope/missing.git"]);
        let bgit = Git::open(broken.path()).unwrap();
        assert!(matches!(push_head_with_nff_recovery(&bgit, broken.path(), "main"), PushOutcome::Failed { .. }));
    }

    #[test]
    fn hosting_trait_defaults_are_conservative_no_ops() {
        // A client that overrides only the required methods inherits the safe
        // defaults for is_draft / comment / review_comments.
        struct Stub;
        impl Hosting for Stub {
            fn available(&self) -> bool { true }
            fn open_draft(&self, _req: &OpenRequest) -> Option<String> { None }
            fn merge_state(&self, _pr_ref: &str) -> MergeState { MergeState::Unknown }
        }
        let s = Stub;
        assert_eq!(s.is_draft("1"), None);
        assert!(!s.comment("1", "hi"));
        assert!(s.review_comments("1").is_empty());
    }

    #[test]
    fn resolve_provider_handles_none_unknown_and_git_remote_fallback() {
        use std::process::Command;
        // `hosting: none` → the none arm; falls through, no remote → None.
        let none_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(none_dir.path().join(".darkrun")).unwrap();
        std::fs::write(none_dir.path().join(".darkrun/settings.yml"), "hosting: none\n").unwrap();
        assert_eq!(resolve_provider(none_dir.path()), Provider::None);
        // An unrecognized token → the `_` arm; same fall-through.
        std::fs::write(none_dir.path().join(".darkrun/settings.yml"), "hosting: bitbucket\n").unwrap();
        assert_eq!(resolve_provider(none_dir.path()), Provider::None);

        // No settings → fall back to the git `origin` remote URL.
        let mk = |url: &str| {
            let d = tempfile::tempdir().unwrap();
            let g = |args: &[&str]| { Command::new("git").current_dir(d.path()).args(args).output().unwrap(); };
            g(&["init", "-q"]);
            g(&["remote", "add", "origin", url]);
            let p = resolve_provider(d.path());
            (d, p)
        };
        let (_gh, gh) = mk("https://github.com/o/r.git");
        assert_eq!(gh, Provider::GitHub);
        let (_gl, gl) = mk("git@gitlab.com:o/r.git");
        assert_eq!(gl, Provider::GitLab);
        let (_other, other) = mk("https://example.com/o/r.git");
        assert_eq!(other, Provider::None);
    }
}

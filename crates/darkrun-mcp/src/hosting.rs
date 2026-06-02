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
    fn run(&self, bin: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(bin)
            .args(args)
            .current_dir(&self.repo_root)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

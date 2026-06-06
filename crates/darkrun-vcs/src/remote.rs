//! Parse repo coordinates (host / owner / repo) from a git remote URL.
//!
//! Handles the two shapes git remotes come in:
//! - scp-like SSH:   `git@github.com:owner/repo.git`
//! - URL form:       `https://github.com/owner/repo.git`, `ssh://git@host/owner/repo`
//!
//! GitLab subgroups are preserved: the "owner" is everything between the host
//! and the final path segment (e.g. `group/subgroup`), and the repo is the last
//! segment. This matches how GitLab project paths nest.

use crate::error::{Result, VcsError};
use crate::provider::Provider;

/// Coordinates identifying a repository on a provider host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCoords {
    /// The host, e.g. `github.com` or `gitlab.com`.
    pub host: String,
    /// The owner path. For GitLab this may contain subgroups, e.g. `group/sub`.
    pub owner: String,
    /// The bare repository name (no `.git`).
    pub repo: String,
}

impl RepoCoords {
    /// Construct coordinates directly.
    pub fn new(host: impl Into<String>, owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    /// `owner/repo`, the slug form GitHub PR APIs and humans use.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The full project path GitLab uses: `owner/repo` including subgroups.
    pub fn project_path(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// Best-effort provider inference from the host.
    pub fn provider(&self) -> Option<Provider> {
        Provider::from_host(&self.host)
    }
}

/// Parse a git remote URL into [`RepoCoords`].
///
/// Accepts scp-like (`git@host:owner/repo.git`) and URL-form
/// (`https://host/owner/repo.git`, `ssh://git@host/owner/repo`) remotes.
pub fn parse_remote_url(url: &str) -> Result<RepoCoords> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(VcsError::RemoteParse(url.to_string()));
    }

    // scp-like form: `[user@]host:path`. Distinguished from a URL by having a
    // `:` that is not followed by `//` and no scheme. Must contain a `:` whose
    // preceding text has no `/` (the host part).
    if !trimmed.contains("://") {
        if let Some((authority, path)) = trimmed.split_once(':') {
            // Reject Windows-ish `C:` and ensure the authority looks like a host.
            if !authority.is_empty() && !authority.contains('/') {
                let host = strip_user(authority);
                return finish(host, path, url);
            }
        }
        return Err(VcsError::RemoteParse(url.to_string()));
    }

    // URL form: strip scheme, then authority, then path.
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .ok_or_else(|| VcsError::RemoteParse(url.to_string()))?;

    let (authority, path) = match after_scheme.split_once('/') {
        Some((a, p)) => (a, p),
        None => return Err(VcsError::RemoteParse(url.to_string())),
    };

    // Authority may carry `user@host:port`; we want just the host.
    let host_port = strip_user(authority);
    let host = host_port.split(':').next().unwrap_or(host_port);
    finish(host, path, url)
}

/// Strip a leading `user@` from an authority component.
fn strip_user(authority: &str) -> &str {
    match authority.rsplit_once('@') {
        Some((_, host)) => host,
        None => authority,
    }
}

/// Split a `owner[/subgroups]/repo[.git]` path into owner + repo.
fn finish(host: &str, path: &str, original: &str) -> Result<RepoCoords> {
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err(VcsError::RemoteParse(original.to_string()));
    }

    let repo = segments[segments.len() - 1];
    let owner = segments[..segments.len() - 1].join("/");

    if host.is_empty() || owner.is_empty() || repo.is_empty() {
        return Err(VcsError::RemoteParse(original.to_string()));
    }

    Ok(RepoCoords::new(host, owner, repo))
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn parses_a_valid_remote_and_rejects_a_pathless_one() {
        let ok = parse_remote_url("git@github.com:owner/repo.git").unwrap();
        assert_eq!(ok.host, "github.com");
        assert_eq!(ok.owner, "owner");
        assert_eq!(ok.repo, "repo");
        // A URL with no repo path → parse error.
        assert!(parse_remote_url("https://github.com").is_err());
        assert!(parse_remote_url("not-a-url").is_err());
    }
}

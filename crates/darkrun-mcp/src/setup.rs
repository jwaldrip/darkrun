//! Project setup detection (the `darkrun-setup` skill): auto-detect VCS,
//! hosting, CI/CD, and the default branch, and optionally write
//! `.darkrun/settings.yml`. The skill confirms with the operator before
//! applying; this tool detects always and writes only when asked.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

/// The detected project environment.
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    /// `git` / `jj` / `none`.
    pub vcs: String,
    /// Hosting provider inferred from the remote (`github` / `gitlab` / …).
    pub hosting: String,
    /// CI system inferred from config files present.
    pub ci: String,
    /// The default branch.
    pub default_branch: String,
    /// Whether `.darkrun/settings.yml` was written this call.
    pub written: bool,
}

fn git(root: &Path, args: &[&str]) -> String {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn hosting_from_remote(remote: &str) -> &'static str {
    let r = remote.to_ascii_lowercase();
    if r.contains("github.com") {
        "github"
    } else if r.contains("gitlab") {
        "gitlab"
    } else if r.contains("bitbucket") {
        "bitbucket"
    } else if remote.is_empty() {
        "none"
    } else {
        "other"
    }
}

/// Detect the project environment. When `apply`, also write
/// `.darkrun/settings.yml` (merging is the caller's concern; this writes the
/// detected fields).
pub fn setup(repo_root: &Path, apply: bool) -> std::io::Result<Settings> {
    let vcs = if repo_root.join(".git").exists() {
        "git"
    } else if repo_root.join(".jj").exists() {
        "jj"
    } else {
        "none"
    };
    let remote = git(repo_root, &["remote", "get-url", "origin"]);
    let hosting = hosting_from_remote(&remote);
    let ci = if repo_root.join(".github/workflows").is_dir() {
        "github-actions"
    } else if repo_root.join(".gitlab-ci.yml").exists() {
        "gitlab-ci"
    } else if repo_root.join(".circleci").is_dir() {
        "circleci"
    } else {
        "none"
    };
    // origin/HEAD → default branch; fall back to the current branch, then main.
    let head_ref = git(repo_root, &["symbolic-ref", "refs/remotes/origin/HEAD"]);
    let default_branch = head_ref
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let cur = git(repo_root, &["branch", "--show-current"]);
            if cur.is_empty() { "main".to_string() } else { cur }
        });

    let mut written = false;
    if apply {
        let yml = format!(
            "vcs: {vcs}\nhosting: {hosting}\nci: {ci}\ndefault_branch: {default_branch}\n"
        );
        let dir = repo_root.join(".darkrun");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("settings.yml"), yml)?;
        written = true;
    }

    Ok(Settings {
        vcs: vcs.to_string(),
        hosting: hosting.to_string(),
        ci: ci.to_string(),
        default_branch,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_git_and_ci_and_writes_when_applied() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();

        let s = setup(root, false).unwrap();
        assert_eq!(s.vcs, "git");
        assert_eq!(s.ci, "github-actions");
        assert!(!s.written);
        assert!(!root.join(".darkrun/settings.yml").exists());

        let s2 = setup(root, true).unwrap();
        assert!(s2.written);
        let yml = std::fs::read_to_string(root.join(".darkrun/settings.yml")).unwrap();
        assert!(yml.contains("vcs: git"));
        assert!(yml.contains("ci: github-actions"));
    }

    #[test]
    fn detects_none_for_a_bare_dir() {
        let dir = tempfile::tempdir().unwrap();
        let s = setup(dir.path(), false).unwrap();
        assert_eq!(s.vcs, "none");
        assert_eq!(s.hosting, "none");
        assert_eq!(s.ci, "none");
    }

    #[test]
    fn hosting_inferred_from_remote_string() {
        assert_eq!(hosting_from_remote("git@github.com:a/b.git"), "github");
        assert_eq!(hosting_from_remote("https://gitlab.com/a/b.git"), "gitlab");
        assert_eq!(hosting_from_remote(""), "none");
    }
}

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
//! This drives the GitHub/GitLab REST APIs in-process (over the pure-Rust
//! [`darkrun_vcs`] HTTP layer — no `gh`/`glab` CLI) behind a small [`Hosting`]
//! seam so the manager stays testable: the API implementation ([`ApiHosting`])
//! makes the calls, while tests inject a mock. Every call is **best-effort** —
//! when no provider / credential / remote is configured the client reports the
//! absence cleanly and the manager falls back to an await gate the operator
//! resolves by hand (it never crashes the tick).

use std::path::Path;

use darkrun_vcs::{HttpRequest, HttpResponse, HttpTransport, Method};

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

/// A pure-Rust synchronous [`HttpTransport`] over `ureq` (rustls, no C). The
/// engine tick calls hosting APIs from a synchronous context inside the tokio
/// runtime, where `reqwest::blocking` would panic — `ureq` has no internal
/// runtime, so it's safe to call there.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqTransport {
    /// Build a transport with a hard per-request wall-clock ceiling so an
    /// unresponsive host can't wedge a tick.
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(30))
            .build();
        Self { agent }
    }
}

impl HttpTransport for UreqTransport {
    #[cfg(not(tarpaulin_include))] // real network I/O
    fn execute(&self, request: HttpRequest) -> darkrun_vcs::Result<HttpResponse> {
        use std::io::Read;
        let mut req = match request.method {
            Method::Get => self.agent.get(&request.url),
            Method::Post => self.agent.post(&request.url),
        };
        for (k, v) in &request.headers {
            req = req.set(k, v);
        }
        let send = match request.body {
            Some(body) => req.send_bytes(&body),
            None => req.call(),
        };
        // ureq surfaces a non-2xx as `Error::Status(code, response)`; capture the
        // status + body for BOTH paths so the caller's error handling works.
        let response = match send {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => return Err(darkrun_vcs::VcsError::Transport(e.to_string())),
        };
        let status = response.status();
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| darkrun_vcs::VcsError::Transport(e.to_string()))?;
        Ok(HttpResponse::new(status, body))
    }
}

/// The API-backed hosting client: drives the provider REST API (GitHub PRs /
/// GitLab MRs) in-process over [`UreqTransport`], with a token from the
/// credential store. Replaces the old `gh`/`glab` CLI shell-outs.
///
/// Provider selection mirrors `darkrun-setup`'s `hosting:` detection. An unknown
/// provider, an unconfigured remote, or a missing credential yields a client
/// that reports [`available`](Hosting::available) `== false`, so the manager
/// falls back to an await gate.
pub struct ApiHosting {
    provider: Option<darkrun_vcs::Provider>,
    coords: Option<darkrun_vcs::RepoCoords>,
    cred: Option<darkrun_vcs::Credential>,
    transport: Box<dyn HttpTransport>,
}

/// The hosting provider an [`ApiHosting`] drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    GitHub,
    GitLab,
    None,
}

impl ApiHosting {
    /// Build an API hosting client for `repo_root` over a real [`UreqTransport`]:
    /// resolve the provider (settings.yml then remote URL), parse the repo
    /// coordinates from `origin`, and load the provider token from the credential
    /// store. Any missing piece simply renders the client unavailable.
    pub fn resolve(repo_root: &Path) -> Self {
        Self::with_transport(repo_root, Box::new(UreqTransport::new()))
    }

    /// [`resolve`](Self::resolve) with an injected transport — the seam tests use
    /// to drive the REST flow over a `MockTransport`.
    pub fn with_transport(repo_root: &Path, transport: Box<dyn HttpTransport>) -> Self {
        let provider = match resolve_provider(repo_root) {
            Provider::GitHub => Some(darkrun_vcs::Provider::GitHub),
            Provider::GitLab => Some(darkrun_vcs::Provider::GitLab),
            Provider::None => None,
        };
        let coords = darkrun_git::Git::open(repo_root)
            .ok()
            .and_then(|g| {
                use darkrun_git::GitBackend;
                g.remote_url("origin").ok().flatten()
            })
            .and_then(|url| darkrun_vcs::parse_remote_url(&url).ok());
        let cred = provider.and_then(|p| {
            darkrun_vcs::CredentialStore::default_path()
                .ok()
                .and_then(|store| store.get(p).ok().flatten())
        });
        Self {
            provider,
            coords,
            cred,
            transport,
        }
    }

    /// Construct directly from parts (the unit-test constructor).
    pub fn from_parts(
        provider: Option<darkrun_vcs::Provider>,
        coords: Option<darkrun_vcs::RepoCoords>,
        cred: Option<darkrun_vcs::Credential>,
        transport: Box<dyn HttpTransport>,
    ) -> Self {
        Self {
            provider,
            coords,
            cred,
            transport,
        }
    }

    /// The provider + coords + credential, present only when fully configured.
    fn ready(
        &self,
    ) -> Option<(
        darkrun_vcs::Provider,
        &darkrun_vcs::RepoCoords,
        &darkrun_vcs::Credential,
    )> {
        Some((self.provider?, self.coords.as_ref()?, self.cred.as_ref()?))
    }

    /// Resolve the GitLab numeric project id for the configured repo.
    fn gitlab_project_id(&self) -> Option<u64> {
        let (_p, coords, cred) = self.ready()?;
        darkrun_vcs::gitlab_resolve_project(self.transport.as_ref(), cred, coords)
            .ok()
            .map(|p| p.id)
    }

    /// Fetch the poll-time view (state + draft) of `pr_ref`.
    fn view(&self, pr_ref: &str) -> Option<darkrun_vcs::ChangeRequestView> {
        let (provider, coords, cred) = self.ready()?;
        let number = parse_ref_number(pr_ref)?;
        let t = self.transport.as_ref();
        match provider {
            darkrun_vcs::Provider::GitHub => {
                darkrun_vcs::github_pull_view(t, cred, coords, number).ok()
            }
            darkrun_vcs::Provider::GitLab => {
                let pid = self.gitlab_project_id()?;
                darkrun_vcs::gitlab_mr_view(t, cred, pid, number).ok()
            }
        }
    }
}

/// Parse the change-request number out of a stored ref (the open-time web URL,
/// e.g. `…/pull/42` or `…/merge_requests/42`, or a bare number).
fn parse_ref_number(pr_ref: &str) -> Option<u64> {
    pr_ref
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .split(['?', '#'])
        .next()?
        .parse()
        .ok()
}

impl Hosting for ApiHosting {
    fn available(&self) -> bool {
        self.ready().is_some()
    }

    fn open_draft(&self, req: &OpenRequest) -> Option<String> {
        let (provider, coords, cred) = self.ready()?;
        let t = self.transport.as_ref();
        let cr = match provider {
            darkrun_vcs::Provider::GitHub => darkrun_vcs::github_create_pull_request_with(
                t, cred, coords, &req.head, &req.base, &req.title, &req.body, true,
            )
            .ok()?,
            darkrun_vcs::Provider::GitLab => {
                let project = darkrun_vcs::gitlab_resolve_project(t, cred, coords).ok()?;
                // GitLab marks a draft MR by a `Draft:` title prefix.
                let title = if req.title.starts_with("Draft:") {
                    req.title.clone()
                } else {
                    format!("Draft: {}", req.title)
                };
                darkrun_vcs::gitlab_create_merge_request(
                    t, cred, project.id, &req.head, &req.base, &title, &req.body,
                )
                .ok()?
            }
        };
        Some(cr.url)
    }

    fn merge_state(&self, pr_ref: &str) -> MergeState {
        match self.view(pr_ref).map(|v| v.state) {
            Some(darkrun_vcs::ChangeRequestState::Open) => MergeState::Open,
            Some(darkrun_vcs::ChangeRequestState::Merged) => MergeState::Merged,
            Some(darkrun_vcs::ChangeRequestState::Closed) => MergeState::Closed,
            None => MergeState::Unknown,
        }
    }

    fn is_draft(&self, pr_ref: &str) -> Option<bool> {
        self.view(pr_ref).map(|v| v.draft)
    }

    fn comment(&self, pr_ref: &str, body: &str) -> bool {
        let Some((provider, coords, cred)) = self.ready() else {
            return false;
        };
        let Some(number) = parse_ref_number(pr_ref) else {
            return false;
        };
        let t = self.transport.as_ref();
        match provider {
            darkrun_vcs::Provider::GitHub => {
                darkrun_vcs::github_create_comment(t, cred, coords, number, body).is_ok()
            }
            darkrun_vcs::Provider::GitLab => match self.gitlab_project_id() {
                Some(pid) => darkrun_vcs::gitlab_create_note(t, cred, pid, number, body).is_ok(),
                None => false,
            },
        }
    }

    fn review_comments(&self, pr_ref: &str) -> Vec<ReviewComment> {
        let Some((provider, coords, cred)) = self.ready() else {
            return Vec::new();
        };
        let Some(number) = parse_ref_number(pr_ref) else {
            return Vec::new();
        };
        let t = self.transport.as_ref();
        let notes = match provider {
            darkrun_vcs::Provider::GitHub => {
                darkrun_vcs::github_review_notes(t, cred, coords, number).unwrap_or_default()
            }
            darkrun_vcs::Provider::GitLab => match self.gitlab_project_id() {
                Some(pid) => darkrun_vcs::gitlab_notes(t, cred, pid, number).unwrap_or_default(),
                None => Vec::new(),
            },
        };
        notes
            .into_iter()
            .map(|n| ReviewComment {
                id: n.id,
                author: n.author,
                body: n.body,
                change_request: n.change_request,
            })
            .collect()
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
    let remote = darkrun_git::Git::open(repo_root)
        .ok()
        .and_then(|g| {
            use darkrun_git::GitBackend;
            g.remote_url("origin").ok().flatten()
        })
        .unwrap_or_default();
    if remote.contains("github.com") {
        Provider::GitHub
    } else if remote.contains("gitlab") {
        Provider::GitLab
    } else {
        Provider::None
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

    /// A GitBackend whose remote primitives are scripted; every other method is
    /// unreachable in these tests. `pushes` is consumed front-to-back (first push,
    /// then the post-rebase retry); `rebase_ok` toggles the rebase result.
    struct PushMock {
        pushes: std::cell::RefCell<Vec<darkrun_git::Result<()>>>,
        rebase_ok: bool,
    }
    impl darkrun_git::GitBackend for PushMock {
        fn push(&self, _: &Path, _: &str) -> darkrun_git::Result<()> {
            self.pushes.borrow_mut().remove(0)
        }
        fn fetch(&self, _: &Path, _: &str) -> darkrun_git::Result<()> { Ok(()) }
        fn rebase_onto(&self, _: &Path, _: &str) -> darkrun_git::Result<()> {
            if self.rebase_ok { Ok(()) } else { Err(darkrun_git::GitError::NotARepo("rebase".into())) }
        }
        fn rebase_abort(&self, _: &Path) -> darkrun_git::Result<()> { Ok(()) }
        // ── Unused by push_head_with_nff_recovery ──
        fn create_worktree(&self, _: &str, _: &Path, _: &darkrun_git::CreateOptions) -> darkrun_git::Result<darkrun_git::WorktreeInfo> { unimplemented!() }
        fn list_worktrees(&self) -> darkrun_git::Result<Vec<darkrun_git::WorktreeInfo>> { unimplemented!() }
        fn remove_worktree(&self, _: &str, _: bool) -> darkrun_git::Result<()> { unimplemented!() }
        fn current_branch(&self) -> darkrun_git::Result<Option<String>> { unimplemented!() }
        fn is_clean(&self) -> darkrun_git::Result<bool> { unimplemented!() }
        fn branch_exists(&self, _: &str) -> darkrun_git::Result<bool> { unimplemented!() }
        fn create_branch(&self, _: &str, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn is_ancestor(&self, _: &str, _: &str) -> darkrun_git::Result<bool> { unimplemented!() }
        fn merge_no_commit(&self, _: &Path, _: &str) -> darkrun_git::Result<darkrun_git::MergeOutcome> { unimplemented!() }
        fn merge_in_progress(&self, _: &Path) -> darkrun_git::Result<bool> { unimplemented!() }
        fn checkout_paths(&self, _: &Path, _: &str, _: &[String]) -> darkrun_git::Result<()> { unimplemented!() }
        fn add_paths(&self, _: &Path, _: &[String]) -> darkrun_git::Result<()> { unimplemented!() }
        fn add_all_under(&self, _: &Path, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn status_dirty_under(&self, _: &Path, _: &str) -> darkrun_git::Result<bool> { unimplemented!() }
        fn checkout_branch(&self, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn commit(&self, _: &Path, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn ls_tree(&self, _: &Path, _: &str, _: &str) -> darkrun_git::Result<Vec<String>> { unimplemented!() }
        fn unresolved_paths(&self, _: &Path) -> darkrun_git::Result<Vec<String>> { unimplemented!() }
        fn refs_have_identical_trees(&self, _: &str, _: &str) -> darkrun_git::Result<bool> { unimplemented!() }
        fn create_worktree_detached(&self, _: &str, _: &Path, _: &str) -> darkrun_git::Result<darkrun_git::WorktreeInfo> { unimplemented!() }
        fn head_oid(&self, _: &Path) -> darkrun_git::Result<String> { unimplemented!() }
        fn set_branch_to(&self, _: &str, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn delete_branch(&self, _: &str) -> darkrun_git::Result<()> { unimplemented!() }
        fn remote_url(&self, _: &str) -> darkrun_git::Result<Option<String>> { unimplemented!() }
        fn default_branch(&self) -> darkrun_git::Result<Option<String>> { unimplemented!() }
        fn diff_stat(&self, _: &str) -> darkrun_git::Result<String> { unimplemented!() }
        fn diff(&self, _: &str) -> darkrun_git::Result<String> { unimplemented!() }
    }

    #[test]
    fn push_recovery_handles_non_nff_rejection_and_rebase_outcomes() {
        let wt = Path::new("/tmp/x");
        // A non-NFF rejection (protected branch / permission) is NOT rebased.
        let m = PushMock {
            pushes: std::cell::RefCell::new(vec![Err(darkrun_git::GitError::NotARepo("denied".into()))]),
            rebase_ok: true,
        };
        assert!(matches!(push_head_with_nff_recovery(&m, wt, "b"), PushOutcome::Failed { .. }));

        // Genuine NFF, but the rebase itself fails → Failed (rebase aborted).
        let m = PushMock {
            pushes: std::cell::RefCell::new(vec![Err(darkrun_git::GitError::WorktreeNotFound("fetch first".into()))]),
            rebase_ok: false,
        };
        assert!(matches!(push_head_with_nff_recovery(&m, wt, "b"), PushOutcome::Failed { .. }));

        // Genuine NFF, rebase succeeds, but the retry push fails → Failed.
        let m = PushMock {
            pushes: std::cell::RefCell::new(vec![
                Err(darkrun_git::GitError::WorktreeNotFound("fetch first".into())),
                Err(darkrun_git::GitError::NotARepo("still rejected".into())),
            ]),
            rebase_ok: true,
        };
        assert!(matches!(push_head_with_nff_recovery(&m, wt, "b"), PushOutcome::Failed { .. }));

        // Genuine NFF, rebase succeeds, retry push succeeds → Pushed.
        let m = PushMock {
            pushes: std::cell::RefCell::new(vec![
                Err(darkrun_git::GitError::WorktreeNotFound("fetch first".into())),
                Ok(()),
            ]),
            rebase_ok: true,
        };
        assert!(matches!(push_head_with_nff_recovery(&m, wt, "b"), PushOutcome::Pushed));
    }

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

    // ── ApiHosting over a MockTransport (the REST flow + mapping) ──────────

    use darkrun_vcs::{Credential, HttpResponse, Method as VcsMethod, MockTransport, RepoCoords};

    fn gh_api(mock: MockTransport) -> ApiHosting {
        ApiHosting::from_parts(
            Some(darkrun_vcs::Provider::GitHub),
            Some(RepoCoords::new("github.com", "o", "r")),
            Some(Credential::new(darkrun_vcs::Provider::GitHub, "tok")),
            Box::new(mock),
        )
    }
    fn json_resp(status: u16, v: serde_json::Value) -> HttpResponse {
        HttpResponse::new(status, serde_json::to_vec(&v).unwrap())
    }
    fn open_req() -> OpenRequest {
        OpenRequest {
            head: "darkrun/x/frame".into(),
            base: "darkrun/x/main".into(),
            title: "Frame".into(),
            body: "body".into(),
        }
    }

    #[test]
    fn api_hosting_unconfigured_is_unavailable_and_safe() {
        let h = ApiHosting::from_parts(None, None, None, Box::new(MockTransport::new()));
        assert!(!h.available());
        assert!(h.open_draft(&open_req()).is_none());
        assert_eq!(h.merge_state("1"), MergeState::Unknown);
        assert_eq!(h.is_draft("1"), None);
        assert!(!h.comment("1", "hi"));
        assert!(h.review_comments("1").is_empty());
    }

    #[test]
    fn parse_ref_number_reads_urls_and_bare_numbers() {
        assert_eq!(parse_ref_number("https://github.com/o/r/pull/42"), Some(42));
        assert_eq!(
            parse_ref_number("https://gitlab.com/o/r/-/merge_requests/7"),
            Some(7)
        );
        assert_eq!(parse_ref_number("13"), Some(13));
        assert_eq!(parse_ref_number(".../pull/9?foo=1"), Some(9));
        assert_eq!(parse_ref_number("not-a-number"), None);
    }

    #[test]
    fn github_open_draft_posts_a_draft_pr_and_returns_its_url() {
        let mock = MockTransport::new();
        mock.expect(
            VcsMethod::Post,
            "https://api.github.com/repos/o/r/pulls",
            json_resp(
                201,
                serde_json::json!({"number": 7, "html_url": "https://github.com/o/r/pull/7"}),
            ),
        );
        let h = gh_api(mock);
        assert_eq!(
            h.open_draft(&open_req()).as_deref(),
            Some("https://github.com/o/r/pull/7")
        );
    }

    #[test]
    fn github_merge_state_maps_merged_open_closed() {
        for (merged, state, want) in [
            (true, "closed", MergeState::Merged),
            (false, "open", MergeState::Open),
            (false, "closed", MergeState::Closed),
        ] {
            let mock = MockTransport::new();
            mock.expect(
                VcsMethod::Get,
                "https://api.github.com/repos/o/r/pulls/7",
                json_resp(200, serde_json::json!({"merged": merged, "state": state})),
            );
            let h = gh_api(mock);
            assert_eq!(h.merge_state("https://github.com/o/r/pull/7"), want);
        }
    }

    #[test]
    fn github_is_draft_reads_the_flag() {
        let mock = MockTransport::new();
        mock.expect(
            VcsMethod::Get,
            "https://api.github.com/repos/o/r/pulls/7",
            json_resp(200, serde_json::json!({"state": "open", "draft": true})),
        );
        assert_eq!(gh_api(mock).is_draft("…/pull/7"), Some(true));
    }

    #[test]
    fn github_comment_posts_and_confirms() {
        let mock = MockTransport::new();
        mock.expect(
            VcsMethod::Post,
            "https://api.github.com/repos/o/r/issues/7/comments",
            json_resp(201, serde_json::json!({"id": 1})),
        );
        assert!(gh_api(mock).comment("…/pull/7", "proof attached"));
    }

    #[test]
    fn github_review_comments_map_comments_and_change_requests() {
        let mock = MockTransport::new();
        mock.expect(
            VcsMethod::Get,
            "https://api.github.com/repos/o/r/issues/7/comments",
            json_resp(
                200,
                serde_json::json!([
                    {"id": 1, "user": {"login": "bob"}, "body": "nit"},
                    {"id": 2, "user": {"login": "x"}, "body": ""}
                ]),
            ),
        );
        mock.expect(
            VcsMethod::Get,
            "https://api.github.com/repos/o/r/pulls/7/reviews",
            json_resp(
                200,
                serde_json::json!([
                    {"id": 9, "user": {"login": "alice"}, "body": "needs a metric", "state": "CHANGES_REQUESTED"},
                    {"id": 8, "user": {"login": "carol"}, "body": "", "state": "APPROVED"}
                ]),
            ),
        );
        let got = gh_api(mock).review_comments("…/pull/7");
        // The empty comment + bodyless APPROVED review drop; the nit + the
        // change-request survive.
        assert_eq!(got.len(), 2, "{got:?}");
        let cr = got.iter().find(|c| c.change_request).unwrap();
        assert_eq!(cr.id, "r9");
        assert_eq!(cr.author, "alice");
        assert!(got.iter().any(|c| c.id == "c1" && !c.change_request));
    }

    #[test]
    fn gitlab_open_draft_resolves_project_then_creates_with_draft_prefix() {
        let mock = MockTransport::new();
        mock.expect(
            VcsMethod::Get,
            "https://gitlab.com/api/v4/projects/o%2Fr",
            json_resp(200, serde_json::json!({"id": 55, "default_branch": "main"})),
        );
        mock.expect(
            VcsMethod::Post,
            "https://gitlab.com/api/v4/projects/55/merge_requests",
            json_resp(
                201,
                serde_json::json!({"iid": 3, "web_url": "https://gitlab.com/o/r/-/merge_requests/3"}),
            ),
        );
        let h = ApiHosting::from_parts(
            Some(darkrun_vcs::Provider::GitLab),
            Some(RepoCoords::new("gitlab.com", "o", "r")),
            Some(Credential::new(darkrun_vcs::Provider::GitLab, "tok")),
            Box::new(mock),
        );
        assert_eq!(
            h.open_draft(&open_req()).as_deref(),
            Some("https://gitlab.com/o/r/-/merge_requests/3")
        );
    }
}

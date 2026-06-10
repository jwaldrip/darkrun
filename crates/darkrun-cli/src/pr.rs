//! `darkrun run pr` — the side-effecting half of the external Checkpoint.
//!
//! The manager stays pure: [`darkrun_mcp::change_request_intent`] reads the run
//! at its external Checkpoint and hands back a [`ChangeRequestIntent`] (title,
//! body, head branch). This module is the impure edge that turns that intent
//! into a real Pull Request (GitHub) / Merge Request (GitLab):
//!
//! 1. resolve the origin remote → [`RepoCoords`] (host/owner/repo) + provider,
//! 2. load the stored [`Credential`] for that provider,
//! 3. POST the change request through `darkrun-vcs`.
//!
//! Git facts (remote URL, current branch) come from a [`RepoFacts`] seam so the
//! whole creation path is testable offline with `MockTransport` + canned facts.

use darkrun_core::StateStore;
use darkrun_mcp::change_request_intent;
use darkrun_vcs::{
    github_create_pull_request, github_get_repo, gitlab_create_merge_request,
    gitlab_resolve_project, parse_remote_url, ChangeRequest, Credential, CredentialStore,
    HttpTransport, Provider, RepoCoords,
};

/// The git facts the PR/MR path needs, behind a seam for testing.
pub trait RepoFacts {
    /// The `origin` (or default) remote URL.
    fn remote_url(&self) -> Result<String, Box<dyn std::error::Error>>;
    /// The branch currently checked out, if any.
    fn current_branch(&self) -> Result<Option<String>, Box<dyn std::error::Error>>;
}

/// `RepoFacts` read in-process via the pure-Rust git backend at a repo root.
pub struct GitCliFacts {
    repo_root: std::path::PathBuf,
    remote: String,
}

impl GitCliFacts {
    /// Facts for `repo_root`, reading the named remote (typically `origin`).
    pub fn new(repo_root: impl Into<std::path::PathBuf>, remote: impl Into<String>) -> Self {
        Self {
            repo_root: repo_root.into(),
            remote: remote.into(),
        }
    }

    /// Open the repo with the pure-Rust git backend (no `git` CLI).
    fn open(&self) -> Result<darkrun_git::Git, Box<dyn std::error::Error>> {
        Ok(darkrun_git::Git::open(&self.repo_root)?)
    }
}

impl RepoFacts for GitCliFacts {
    fn remote_url(&self) -> Result<String, Box<dyn std::error::Error>> {
        use darkrun_git::GitBackend;
        self.open()?
            .remote_url(&self.remote)?
            .ok_or_else(|| format!("remote '{}' is not configured", self.remote).into())
    }

    fn current_branch(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        use darkrun_git::GitBackend;
        // A detached HEAD reports no branch (mirrors `rev-parse --abbrev-ref`).
        Ok(self.open()?.current_branch()?)
    }
}

/// Resolve the provider for a set of coordinates, preferring the host-inferred
/// provider and surfacing a clear error when the host is unrecognized.
fn resolve_provider(coords: &RepoCoords) -> Result<Provider, Box<dyn std::error::Error>> {
    coords.provider().ok_or_else(|| {
        format!(
            "cannot infer a provider from host '{}' — only github.com and gitlab.com are supported",
            coords.host
        )
        .into()
    })
}

/// Open the change request for a run sitting at its external Checkpoint.
///
/// `head_override` pins an explicit source branch; when `None` the run's actual
/// current git branch is used (falling back to the conventional
/// `darkrun/<slug>` when git reports a detached HEAD).
#[allow(clippy::too_many_arguments)]
#[cfg(not(tarpaulin_include))] // network PR/MR creation
pub fn create_for_run(
    transport: &dyn HttpTransport,
    facts: &dyn RepoFacts,
    state_store: &StateStore,
    cred_store: &CredentialStore,
    slug: &str,
    head_override: Option<String>,
    base_override: Option<String>,
) -> Result<ChangeRequest, Box<dyn std::error::Error>> {
    // 1. Coordinates + provider from the origin remote.
    let remote_url = facts.remote_url()?;
    let coords = parse_remote_url(&remote_url)?;
    let provider = resolve_provider(&coords)?;

    // 2. Stored credential for that provider.
    let cred: Credential = cred_store.get(provider)?.ok_or_else(|| {
        format!(
            "no {} credential — run `darkrun auth login --provider {}` first",
            provider.display_name(),
            provider.key()
        )
    })?;

    // 3. The pure intent (title/body/head) from the manager.
    //    Prefer an explicit head; else the live git branch; else the convention.
    let head = match head_override {
        Some(h) => Some(h),
        None => facts.current_branch()?,
    };
    let intent = change_request_intent(state_store, slug, head)?;

    // 4. Open the PR/MR. Each provider resolves the repo exactly once: GitHub
    //    only when it must learn the default branch; GitLab always (it needs the
    //    numeric project id for the MR), reusing that lookup for the base too.
    let cr = match provider {
        Provider::GitHub => {
            let base = match base_override {
                Some(b) => b,
                None => github_get_repo(transport, &cred, &coords)?.default_branch,
            };
            github_create_pull_request(
                transport,
                &cred,
                &coords,
                &intent.head,
                &base,
                &intent.title,
                &intent.body,
            )?
        }
        Provider::GitLab => {
            let project = gitlab_resolve_project(transport, &cred, &coords)?;
            let base = base_override.unwrap_or(project.default_branch);
            gitlab_create_merge_request(
                transport,
                &cred,
                project.id,
                &intent.head,
                &base,
                &intent.title,
                &intent.body,
            )?
        }
    };
    Ok(cr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_core::domain::{CheckpointKind, Status, Unit, UnitFrontmatter};
    use darkrun_mcp::position::{run_start, run_tick, RunAction};
    use darkrun_vcs::{HttpResponse, Method, MockTransport};

    /// Canned git facts for tests.
    struct FakeFacts {
        url: String,
        branch: Option<String>,
    }

    impl RepoFacts for FakeFacts {
        fn remote_url(&self) -> Result<String, Box<dyn std::error::Error>> {
            Ok(self.url.clone())
        }
        fn current_branch(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
            Ok(self.branch.clone())
        }
    }

    fn stores() -> (tempfile::TempDir, StateStore, CredentialStore) {
        let dir = tempfile::tempdir().expect("tmp");
        let state = StateStore::new(dir.path());
        let cred = CredentialStore::at(dir.path().join("credentials"));
        (dir, state, cred)
    }

    /// Drive a run to its `harden` station's external review gate. Every station
    /// gates `ask` by default; the external-review surface is a discrete-mode
    /// concern, so we walk the upstream `ask` gates, park at harden's checkpoint
    /// without approving it, and flip the run discrete so the gate re-derives as
    /// `external`.
    fn drive_to_external_checkpoint(store: &StateStore, slug: &str) {
        run_start(
            store,
            slug,
            "software",
            Some("Add login".into()),
            darkrun_core::domain::Mode::Solo,
            "full",
        )
        .unwrap();
        for station in ["frame", "specify", "shape", "build", "prove", "harden"] {
            // Consume the station's declared inputs so the runtime input-coverage
            // gate is satisfied (the run's distillation is carried forward).
            let inputs = darkrun_mcp::resolve_factory("software")
                .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
                .unwrap_or_default();
            let unit = Unit {
                slug: format!("{station}-u"),
                frontmatter: UnitFrontmatter {
                    status: Status::Completed,
                    station: Some(station.to_string()),
                    inputs,
                    ..Default::default()
                },
                title: "u".into(),
                body: String::new(),
            };
            store.write_unit(slug, &unit).unwrap();
            for _ in 0..14 {
                let tick = run_tick(store, slug).unwrap();
                match &tick.action {
                    // Solo holds the Spec until it's elaborated with the operator;
                    // seal it so the station advances to Review.
                    RunAction::Spec { station: s, .. } if s == station => {
                        darkrun_mcp::position::elaborate_seal(store, slug, station).unwrap();
                    }
                    // Clear the pre-execution operator gate so the wave releases.
                    RunAction::UserGate { station: s, .. } if s == station => {
                        darkrun_mcp::checkpoint_decide(store, slug, true, None).unwrap();
                    }
                    RunAction::Checkpoint { kind, station: s, .. } if s == station => {
                        // Approve the upstream gates; park (don't approve) at harden
                        // so the run holds at its checkpoint for the discrete flip.
                        if matches!(kind, CheckpointKind::Ask) && station != "harden" {
                            darkrun_mcp::checkpoint_decide(store, slug, true, None).unwrap();
                        }
                        break;
                    }
                    _ => {}
                }
            }
            if station == "harden" {
                break;
            }
        }
        // Promote the run to `team` so every gate (including harden's parked one)
        // re-derives as `external` — the per-station PR path.
        let mut state = store.read_state(slug).unwrap().unwrap();
        state.mode = darkrun_core::domain::Mode::Team;
        store.write_state(slug, &state).unwrap();
    }

    #[test]
    fn creates_github_pull_request() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        cred_store
            .save(&Credential::new(Provider::GitHub, "gh-tok"))
            .unwrap();

        let facts = FakeFacts {
            url: "git@github.com:acme/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        // get-repo for the default branch.
        mock.expect(
            Method::Get,
            "https://api.github.com/repos/acme/widgets",
            HttpResponse::new(
                200,
                serde_json::to_vec(&serde_json::json!({
                    "id": 1, "default_branch": "main", "html_url": "https://github.com/acme/widgets"
                }))
                .unwrap(),
            ),
        );
        // create PR.
        mock.expect(
            Method::Post,
            "https://api.github.com/repos/acme/widgets/pulls",
            HttpResponse::new(
                201,
                serde_json::to_vec(&serde_json::json!({
                    "number": 42, "html_url": "https://github.com/acme/widgets/pull/42"
                }))
                .unwrap(),
            ),
        );

        let cr = create_for_run(&mock, &facts, &state, &cred_store, "r", None, None).unwrap();
        assert_eq!(cr.provider, Provider::GitHub);
        assert_eq!(cr.number, 42);
        assert_eq!(cr.url, "https://github.com/acme/widgets/pull/42");

        // The PR POST carried the run's branch + derived title.
        let posts: Vec<_> = mock
            .requests()
            .into_iter()
            .filter(|r| r.method == Method::Post)
            .collect();
        assert_eq!(posts.len(), 1);
        let body: serde_json::Value =
            serde_json::from_slice(posts[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["head"], "darkrun/r");
        assert_eq!(body["base"], "main");
        assert_eq!(body["title"], "Add login");
        assert!(body["body"].as_str().unwrap().contains("harden"));
    }

    #[test]
    fn creates_gitlab_merge_request() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        cred_store
            .save(&Credential::new(Provider::GitLab, "gl-tok"))
            .unwrap();

        let facts = FakeFacts {
            url: "https://gitlab.com/group/sub/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        // resolve project (url-encoded path) — resolved exactly once, reused for
        // both the project id and the default (target) branch.
        mock.expect(
            Method::Get,
            "https://gitlab.com/api/v4/projects/group%2Fsub%2Fwidgets",
            HttpResponse::new(
                200,
                serde_json::to_vec(&serde_json::json!({
                    "id": 99, "default_branch": "trunk", "web_url": "https://gitlab.com/group/sub/widgets"
                }))
                .unwrap(),
            ),
        );
        // create MR.
        mock.expect(
            Method::Post,
            "https://gitlab.com/api/v4/projects/99/merge_requests",
            HttpResponse::new(
                201,
                serde_json::to_vec(&serde_json::json!({
                    "iid": 7, "web_url": "https://gitlab.com/group/sub/widgets/-/merge_requests/7"
                }))
                .unwrap(),
            ),
        );

        let cr = create_for_run(&mock, &facts, &state, &cred_store, "r", None, None).unwrap();
        assert_eq!(cr.provider, Provider::GitLab);
        assert_eq!(cr.number, 7);

        let posts: Vec<_> = mock
            .requests()
            .into_iter()
            .filter(|r| r.method == Method::Post)
            .collect();
        let body: serde_json::Value =
            serde_json::from_slice(posts[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["source_branch"], "darkrun/r");
        assert_eq!(body["target_branch"], "trunk");
    }

    #[test]
    fn base_override_skips_default_branch_lookup() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        cred_store
            .save(&Credential::new(Provider::GitHub, "gh-tok"))
            .unwrap();
        let facts = FakeFacts {
            url: "git@github.com:acme/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        // ONLY the PR POST — no get-repo, because base is overridden.
        mock.expect(
            Method::Post,
            "https://api.github.com/repos/acme/widgets/pulls",
            HttpResponse::new(
                201,
                serde_json::to_vec(&serde_json::json!({
                    "number": 1, "html_url": "https://github.com/acme/widgets/pull/1"
                }))
                .unwrap(),
            ),
        );
        let cr = create_for_run(
            &mock,
            &facts,
            &state,
            &cred_store,
            "r",
            None,
            Some("release".into()),
        )
        .unwrap();
        assert_eq!(cr.number, 1);
        let body: serde_json::Value = serde_json::from_slice(
            mock.requests()[0].body.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(body["base"], "release");
    }

    #[test]
    fn head_override_wins_over_git_branch() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        cred_store
            .save(&Credential::new(Provider::GitHub, "gh-tok"))
            .unwrap();
        let facts = FakeFacts {
            url: "git@github.com:acme/widgets.git".into(),
            branch: Some("some-other-branch".into()),
        };
        let mock = MockTransport::new();
        mock.expect(
            Method::Post,
            "https://api.github.com/repos/acme/widgets/pulls",
            HttpResponse::new(
                201,
                serde_json::to_vec(&serde_json::json!({
                    "number": 1, "html_url": "https://github.com/acme/widgets/pull/1"
                }))
                .unwrap(),
            ),
        );
        create_for_run(
            &mock,
            &facts,
            &state,
            &cred_store,
            "r",
            Some("explicit-head".into()),
            Some("main".into()),
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_slice(
            mock.requests()[0].body.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(body["head"], "explicit-head");
    }

    #[test]
    fn errors_without_credential() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        let facts = FakeFacts {
            url: "git@github.com:acme/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        let err = create_for_run(&mock, &facts, &state, &cred_store, "r", None, None).unwrap_err();
        assert!(err.to_string().contains("auth login"));
    }

    #[test]
    fn errors_when_not_at_external_checkpoint() {
        let (_d, state, cred_store) = stores();
        // Fresh run — sits at Spec, not an external checkpoint.
        run_start(&state, "r", "software", None, darkrun_core::domain::Mode::Solo, "full").unwrap();
        cred_store
            .save(&Credential::new(Provider::GitHub, "gh-tok"))
            .unwrap();
        let facts = FakeFacts {
            url: "git@github.com:acme/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        // No network is reached: provider + credential resolve, but the pure
        // intent derivation rejects the non-external checkpoint before any POST.
        let err = create_for_run(&mock, &facts, &state, &cred_store, "r", None, Some("main".into()))
            .unwrap_err();
        // A non-external run rejects with an "external review gate" error before
        // any POST.
        assert!(err.to_string().to_lowercase().contains("gate"));
        assert!(mock.requests().is_empty());
    }

    #[test]
    fn errors_on_unknown_host() {
        let (_d, state, cred_store) = stores();
        drive_to_external_checkpoint(&state, "r");
        let facts = FakeFacts {
            url: "git@bitbucket.org:acme/widgets.git".into(),
            branch: Some("darkrun/r".into()),
        };
        let mock = MockTransport::new();
        let err = create_for_run(&mock, &facts, &state, &cred_store, "r", None, None).unwrap_err();
        assert!(err.to_string().contains("cannot infer a provider"));
    }
}

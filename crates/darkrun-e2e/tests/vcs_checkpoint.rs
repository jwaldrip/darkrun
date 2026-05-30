//! End-to-end: external-Checkpoint → change-request path.
//!
//! When a Run reaches an **external** Checkpoint, the operator's Pass is handed
//! to a human reviewer through a real change request: a Pull Request on GitHub or
//! a Merge Request on GitLab. These tests exercise that handoff end to end using
//! `darkrun_vcs` with its offline `MockTransport`, so no network is touched.
//!
//! Coverage:
//! - both providers produce a normalized `ChangeRequest` via `create_change_request`,
//! - GitLab's two-step resolve-then-create ordering and the exact requests it issues,
//! - the credential store load that feeds the auth header,
//! - remote-coordinate parsing from representative git remote URLs,
//! - API error handling (422 / 404 / auth) at each stage of the flow.

use darkrun_vcs::{
    create_change_request, github_create_pull_request, github_get_repo,
    gitlab_create_merge_request, gitlab_resolve_project, parse_remote_url, ChangeRequest,
    Credential, CredentialStore, HttpRequest, HttpResponse, Method, MockTransport, Provider,
    RepoCoords, VcsError,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures: the canonical Run-reaches-external-Checkpoint inputs.
// ---------------------------------------------------------------------------

/// The branch a Worker pushed for the Pass under review.
const HEAD: &str = "darkrun/station-build";
/// The base branch the change request targets.
const BASE: &str = "main";
/// The Checkpoint's change-request title.
const TITLE: &str = "Checkpoint: build station output";
/// The Checkpoint's change-request body.
const BODY: &str = "Run reached an external checkpoint and needs human review.";

fn github_coords() -> RepoCoords {
    RepoCoords::new("github.com", "darkrun", "factory")
}

fn gitlab_coords() -> RepoCoords {
    RepoCoords::new("gitlab.com", "darkrun", "factory")
}

fn github_cred() -> Credential {
    Credential::new(Provider::GitHub, "gh-checkpoint-token")
}

fn gitlab_cred() -> Credential {
    Credential::new(Provider::GitLab, "gl-checkpoint-token")
}

fn gh_pulls_url(coords: &RepoCoords) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/pulls",
        coords.owner, coords.repo
    )
}

fn gh_repo_url(coords: &RepoCoords) -> String {
    format!(
        "https://api.github.com/repos/{}/{}",
        coords.owner, coords.repo
    )
}

fn gl_project_url(path_encoded: &str) -> String {
    format!("https://gitlab.com/api/v4/projects/{path_encoded}")
}

fn gl_mr_url(project_id: u64) -> String {
    format!("https://gitlab.com/api/v4/projects/{project_id}/merge_requests")
}

/// A representative GitHub PR-creation success body.
fn gh_pr_body(number: u64, slug: &str) -> String {
    format!(
        r#"{{"number":{number},"html_url":"https://github.com/{slug}/pull/{number}","state":"open"}}"#
    )
}

/// A representative GitLab project-resolve success body.
fn gl_project_body(id: u64, default_branch: &str) -> String {
    format!(
        r#"{{"id":{id},"default_branch":"{default_branch}","web_url":"https://gitlab.com/darkrun/factory"}}"#
    )
}

/// A representative GitLab MR-creation success body.
fn gl_mr_body(iid: u64, id: u64) -> String {
    format!(
        r#"{{"iid":{iid},"id":{id},"web_url":"https://gitlab.com/darkrun/factory/-/merge_requests/{iid}"}}"#
    )
}

/// Find a header value (case-insensitive) on a recorded request.
fn header<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Parse a recorded request body as JSON.
fn body_json(req: &HttpRequest) -> serde_json::Value {
    let bytes = req.body.as_ref().expect("request had a body");
    serde_json::from_slice(bytes).expect("body was JSON")
}

// ===========================================================================
// GitHub: external Checkpoint → Pull Request
// ===========================================================================

#[test]
fn github_checkpoint_creates_pull_request() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(42, "darkrun/factory")),
    );

    let cr = create_change_request(
        &mock,
        Provider::GitHub,
        &github_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .expect("PR created");

    assert_eq!(cr.provider, Provider::GitHub);
    assert_eq!(cr.number, 42);
    assert_eq!(cr.url, "https://github.com/darkrun/factory/pull/42");
}

#[test]
fn github_checkpoint_issues_exactly_one_request() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    create_change_request(
        &mock,
        Provider::GitHub,
        &github_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    // GitHub posts the PR directly — no resolve step.
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn github_checkpoint_posts_to_pulls_endpoint() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(7, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://api.github.com/repos/darkrun/factory/pulls");
}

#[test]
fn github_pr_body_maps_head_base_title_body() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(9, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let body = body_json(&mock.single_request());
    assert_eq!(body["head"], HEAD);
    assert_eq!(body["base"], BASE);
    assert_eq!(body["title"], TITLE);
    assert_eq!(body["body"], BODY);
}

#[test]
fn github_pr_body_has_no_gitlab_fields() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(9, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let body = body_json(&mock.single_request());
    assert!(body.get("source_branch").is_none());
    assert!(body.get("target_branch").is_none());
    assert!(body.get("description").is_none());
}

#[test]
fn github_request_carries_bearer_authorization() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(3, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert_eq!(
        header(&req, "Authorization"),
        Some("Bearer gh-checkpoint-token")
    );
}

#[test]
fn github_request_carries_user_agent() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(3, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    // GitHub rejects API requests without a User-Agent.
    let req = mock.single_request();
    assert!(header(&req, "User-Agent").is_some());
    assert!(!header(&req, "User-Agent").unwrap().is_empty());
}

#[test]
fn github_request_carries_api_version_and_accept() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(3, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert_eq!(
        header(&req, "Accept"),
        Some("application/vnd.github+json")
    );
    assert_eq!(
        header(&req, "X-GitHub-Api-Version"),
        Some("2022-11-28")
    );
}

#[test]
fn github_request_sets_json_content_type() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(3, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert_eq!(header(&req, "Content-Type"), Some("application/json"));
}

#[test]
fn github_pr_accepts_200_status() {
    // GitHub's create endpoint returns 201, but the success check is the whole
    // 2xx range — a 200 must also be accepted.
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(200, gh_pr_body(5, "darkrun/factory")),
    );

    let cr =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();
    assert_eq!(cr.number, 5);
}

#[test]
fn github_pr_large_number_preserved() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(987654, "darkrun/factory")),
    );

    let cr =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();
    assert_eq!(cr.number, 987654);
}

#[test]
fn github_get_repo_returns_repo_info() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gh_repo_url(&coords),
        HttpResponse::new(
            200,
            r#"{"id":555,"default_branch":"trunk","html_url":"https://github.com/darkrun/factory"}"#,
        ),
    );

    let info = github_get_repo(&mock, &github_cred(), &coords).unwrap();
    assert_eq!(info.id, 555);
    assert_eq!(info.default_branch, "trunk");
    assert_eq!(info.web_url, "https://github.com/darkrun/factory");
}

#[test]
fn github_get_repo_is_a_get_to_repo_endpoint() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gh_repo_url(&coords),
        HttpResponse::new(200, r#"{"id":1,"default_branch":"main","html_url":"x"}"#),
    );

    github_get_repo(&mock, &github_cred(), &coords).unwrap();

    let req = mock.single_request();
    assert_eq!(req.method, Method::Get);
    assert_eq!(req.url, "https://api.github.com/repos/darkrun/factory");
    assert!(req.body.is_none());
}

#[test]
fn github_get_repo_defaults_branch_when_absent() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gh_repo_url(&coords),
        HttpResponse::new(200, r#"{"id":1,"html_url":"x"}"#),
    );

    let info = github_get_repo(&mock, &github_cred(), &coords).unwrap();
    assert_eq!(info.default_branch, "main");
}

#[test]
fn github_pr_url_reflects_coords_in_request() {
    // A different repo's coords must change the request URL, not just the body.
    let coords = RepoCoords::new("github.com", "acme", "widgets");
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(11, "acme/widgets")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY).unwrap();
    assert_eq!(
        mock.single_request().url,
        "https://api.github.com/repos/acme/widgets/pulls"
    );
}

// ===========================================================================
// GitLab: external Checkpoint → Merge Request (resolve-then-create)
// ===========================================================================

#[test]
fn gitlab_checkpoint_creates_merge_request() {
    let coords = gitlab_coords();
    let encoded = "darkrun%2Ffactory";
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url(encoded),
        HttpResponse::new(200, gl_project_body(7001, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(13, 7001)),
    );

    let cr = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .expect("MR created");

    assert_eq!(cr.provider, Provider::GitLab);
    assert_eq!(cr.number, 13);
    assert_eq!(
        cr.url,
        "https://gitlab.com/darkrun/factory/-/merge_requests/13"
    );
}

#[test]
fn gitlab_checkpoint_resolves_then_creates_in_order() {
    let coords = gitlab_coords();
    let encoded = "darkrun%2Ffactory";
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url(encoded),
        HttpResponse::new(200, gl_project_body(7001, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(13, 7001)),
    );

    create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "resolve then create");
    // First: GET the project to learn its numeric id.
    assert_eq!(reqs[0].method, Method::Get);
    assert_eq!(reqs[0].url, gl_project_url(encoded));
    // Second: POST the merge request against that id.
    assert_eq!(reqs[1].method, Method::Post);
    assert_eq!(reqs[1].url, gl_mr_url(7001));
}

#[test]
fn gitlab_mr_targets_resolved_project_id() {
    // The MR must be posted against the *resolved* id, not some guess.
    let coords = gitlab_coords();
    let encoded = "darkrun%2Ffactory";
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url(encoded),
        HttpResponse::new(200, gl_project_body(424242, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(424242),
        HttpResponse::new(201, gl_mr_body(1, 424242)),
    );

    create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    let reqs = mock.requests();
    assert_eq!(
        reqs[1].url,
        "https://gitlab.com/api/v4/projects/424242/merge_requests"
    );
}

#[test]
fn gitlab_resolve_project_percent_encodes_path() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(1, "main")),
    );

    let info = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    assert_eq!(info.id, 1);
    // The slash in `darkrun/factory` must arrive percent-encoded.
    assert_eq!(
        mock.single_request().url,
        "https://gitlab.com/api/v4/projects/darkrun%2Ffactory"
    );
}

#[test]
fn gitlab_resolve_project_encodes_subgroup_path() {
    let coords = RepoCoords::new("gitlab.com", "group/subgroup", "factory");
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("group%2Fsubgroup%2Ffactory"),
        HttpResponse::new(200, gl_project_body(9, "main")),
    );

    gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    assert_eq!(
        mock.single_request().url,
        "https://gitlab.com/api/v4/projects/group%2Fsubgroup%2Ffactory"
    );
}

#[test]
fn gitlab_resolve_project_reads_id_and_branch() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(88, "develop")),
    );

    let info = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    assert_eq!(info.id, 88);
    assert_eq!(info.default_branch, "develop");
    assert_eq!(info.web_url, "https://gitlab.com/darkrun/factory");
}

#[test]
fn gitlab_resolve_project_defaults_branch_when_absent() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, r#"{"id":2,"web_url":"x"}"#),
    );

    let info = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    assert_eq!(info.default_branch, "main");
}

#[test]
fn gitlab_resolve_project_missing_id_errors() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, r#"{"default_branch":"main","web_url":"x"}"#),
    );

    let err = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap_err();
    assert!(matches!(err, VcsError::MissingField("id")));
}

#[test]
fn gitlab_mr_body_maps_source_target_title_description() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(13, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();

    let body = body_json(&mock.single_request());
    assert_eq!(body["source_branch"], HEAD);
    assert_eq!(body["target_branch"], BASE);
    assert_eq!(body["title"], TITLE);
    assert_eq!(body["description"], BODY);
}

#[test]
fn gitlab_mr_body_has_no_github_fields() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(13, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();

    let body = body_json(&mock.single_request());
    assert!(body.get("head").is_none());
    assert!(body.get("base").is_none());
    assert!(body.get("body").is_none());
}

#[test]
fn gitlab_mr_request_carries_bearer_and_accept_json() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(1, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert_eq!(
        header(&req, "Authorization"),
        Some("Bearer gl-checkpoint-token")
    );
    assert_eq!(header(&req, "Accept"), Some("application/json"));
}

#[test]
fn gitlab_mr_request_has_no_github_api_version() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(1, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();

    let req = mock.single_request();
    assert!(header(&req, "X-GitHub-Api-Version").is_none());
}

#[test]
fn gitlab_mr_returns_iid_as_number() {
    // GitLab MRs are addressed by their per-project iid, not the global id.
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(57, 999999)),
    );

    let cr =
        gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();
    assert_eq!(cr.number, 57);
}

#[test]
fn gitlab_mr_missing_iid_errors() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, r#"{"id":7001,"web_url":"x"}"#),
    );

    let err = gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY)
        .unwrap_err();
    assert!(matches!(err, VcsError::MissingField("iid")));
}

#[test]
fn gitlab_mr_missing_web_url_errors() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, r#"{"iid":3,"id":7001}"#),
    );

    let err = gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY)
        .unwrap_err();
    assert!(matches!(err, VcsError::MissingField("web_url")));
}

#[test]
fn gitlab_create_change_request_stops_if_resolve_fails() {
    // If the resolve 404s, no MR POST should ever be issued.
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(404, r#"{"message":"404 Project Not Found"}"#),
    );

    let err = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();

    assert!(matches!(err, VcsError::Api { status: 404, .. }));
    // Only the resolve was attempted; no MR creation.
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn gitlab_subgroup_full_flow_creates_mr() {
    let coords = RepoCoords::new("gitlab.com", "darkrun/factories", "station");
    let encoded = "darkrun%2Ffactories%2Fstation";
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url(encoded),
        HttpResponse::new(200, gl_project_body(321, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(321),
        HttpResponse::new(201, gl_mr_body(4, 321)),
    );

    let cr = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();
    assert_eq!(cr.number, 4);
    assert_eq!(mock.requests()[0].url, gl_project_url(encoded));
}

// ===========================================================================
// Credential store: load the token that authorizes the Checkpoint handoff
// ===========================================================================

fn temp_store() -> (TempDir, CredentialStore) {
    let dir = TempDir::new().unwrap();
    let store = CredentialStore::at(dir.path().join(".darkrun").join("credentials"));
    (dir, store)
}

#[test]
fn store_round_trips_github_credential() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();

    let loaded = store.get(Provider::GitHub).unwrap().expect("present");
    assert_eq!(loaded.provider, Provider::GitHub);
    assert_eq!(loaded.access_token, "gh-checkpoint-token");
}

#[test]
fn store_round_trips_gitlab_credential_with_refresh() {
    let (_dir, store) = temp_store();
    let cred = Credential {
        provider: Provider::GitLab,
        access_token: "gl-access".into(),
        refresh_token: Some("gl-refresh".into()),
        expires_in: Some(7200),
        token_type: Some("bearer".into()),
    };
    store.save(&cred).unwrap();

    let loaded = store.get(Provider::GitLab).unwrap().unwrap();
    assert_eq!(loaded, cred);
    assert_eq!(loaded.refresh_token.as_deref(), Some("gl-refresh"));
    assert_eq!(loaded.expires_in, Some(7200));
}

#[test]
fn store_missing_provider_returns_none() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();
    // GitHub saved, GitLab never was.
    assert!(store.get(Provider::GitLab).unwrap().is_none());
}

#[test]
fn store_absent_file_returns_none() {
    let (_dir, store) = temp_store();
    // Nothing saved at all → load yields None, not an error.
    assert!(store.get(Provider::GitHub).unwrap().is_none());
}

#[test]
fn store_holds_both_providers_independently() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();
    store.save(&gitlab_cred()).unwrap();

    assert_eq!(
        store.get(Provider::GitHub).unwrap().unwrap().access_token,
        "gh-checkpoint-token"
    );
    assert_eq!(
        store.get(Provider::GitLab).unwrap().unwrap().access_token,
        "gl-checkpoint-token"
    );
}

#[test]
fn store_save_overwrites_same_provider() {
    let (_dir, store) = temp_store();
    store.save(&Credential::new(Provider::GitHub, "old")).unwrap();
    store.save(&Credential::new(Provider::GitHub, "new")).unwrap();

    assert_eq!(
        store.get(Provider::GitHub).unwrap().unwrap().access_token,
        "new"
    );
}

#[test]
fn store_overwrite_does_not_disturb_other_provider() {
    let (_dir, store) = temp_store();
    store.save(&gitlab_cred()).unwrap();
    store.save(&Credential::new(Provider::GitHub, "v1")).unwrap();
    store.save(&Credential::new(Provider::GitHub, "v2")).unwrap();

    assert_eq!(
        store.get(Provider::GitLab).unwrap().unwrap().access_token,
        "gl-checkpoint-token"
    );
}

#[test]
fn store_remove_deletes_credential() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();
    assert!(store.remove(Provider::GitHub).unwrap());
    assert!(store.get(Provider::GitHub).unwrap().is_none());
}

#[test]
fn store_remove_absent_returns_false() {
    let (_dir, store) = temp_store();
    assert!(!store.remove(Provider::GitHub).unwrap());
}

#[test]
fn store_remove_keeps_other_provider() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();
    store.save(&gitlab_cred()).unwrap();
    store.remove(Provider::GitHub).unwrap();

    assert!(store.get(Provider::GitHub).unwrap().is_none());
    assert!(store.get(Provider::GitLab).unwrap().is_some());
}

#[test]
fn store_list_reports_saved_providers() {
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();
    store.save(&gitlab_cred()).unwrap();

    let mut list = store.list().unwrap();
    list.sort_by_key(|p| p.key());
    assert_eq!(list, vec![Provider::GitHub, Provider::GitLab]);
}

#[test]
fn store_list_empty_when_nothing_saved() {
    let (_dir, store) = temp_store();
    assert!(store.list().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn store_file_is_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, store) = temp_store();
    store.save(&github_cred()).unwrap();

    let mode = std::fs::metadata(store.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "credential file must not be world-readable");
}

#[test]
fn store_loaded_credential_drives_change_request_auth() {
    // Full seam: load the Checkpoint token from disk, then use it to open a PR.
    let (_dir, store) = temp_store();
    store.save(&Credential::new(Provider::GitHub, "from-disk")).unwrap();
    let cred = store.get(Provider::GitHub).unwrap().unwrap();

    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    create_change_request(
        &mock,
        Provider::GitHub,
        &cred,
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    assert_eq!(
        header(&mock.single_request(), "Authorization"),
        Some("Bearer from-disk")
    );
}

#[test]
fn store_path_is_under_darkrun_dir() {
    let (_dir, store) = temp_store();
    assert!(store.path().ends_with("credentials"));
    assert!(store
        .path()
        .to_string_lossy()
        .contains(".darkrun"));
}

// ===========================================================================
// Remote-coords parsing from representative remote URLs
// ===========================================================================

#[test]
fn parse_https_github_remote() {
    let c = parse_remote_url("https://github.com/darkrun/factory.git").unwrap();
    assert_eq!(c.host, "github.com");
    assert_eq!(c.owner, "darkrun");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_https_github_remote_no_git_suffix() {
    let c = parse_remote_url("https://github.com/darkrun/factory").unwrap();
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_scp_github_remote() {
    let c = parse_remote_url("git@github.com:darkrun/factory.git").unwrap();
    assert_eq!(c.host, "github.com");
    assert_eq!(c.owner, "darkrun");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_scp_remote_without_git_suffix() {
    let c = parse_remote_url("git@github.com:darkrun/factory").unwrap();
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_ssh_url_form_remote() {
    let c = parse_remote_url("ssh://git@github.com/darkrun/factory.git").unwrap();
    assert_eq!(c.host, "github.com");
    assert_eq!(c.owner, "darkrun");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_https_gitlab_remote() {
    let c = parse_remote_url("https://gitlab.com/darkrun/factory.git").unwrap();
    assert_eq!(c.host, "gitlab.com");
    assert_eq!(c.slug(), "darkrun/factory");
}

#[test]
fn parse_gitlab_subgroup_remote_https() {
    let c = parse_remote_url("https://gitlab.com/group/subgroup/factory.git").unwrap();
    assert_eq!(c.host, "gitlab.com");
    assert_eq!(c.owner, "group/subgroup");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_gitlab_subgroup_remote_scp() {
    let c = parse_remote_url("git@gitlab.com:group/subgroup/factory.git").unwrap();
    assert_eq!(c.owner, "group/subgroup");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_gitlab_deep_subgroup_remote() {
    let c = parse_remote_url("https://gitlab.com/a/b/c/d/repo.git").unwrap();
    assert_eq!(c.owner, "a/b/c/d");
    assert_eq!(c.repo, "repo");
}

#[test]
fn parse_remote_project_path_for_gitlab_resolve() {
    // The parsed coords feed GitLab's project resolve verbatim.
    let c = parse_remote_url("https://gitlab.com/group/subgroup/factory.git").unwrap();
    assert_eq!(c.project_path(), "group/subgroup/factory");
}

#[test]
fn parse_https_with_port() {
    let c = parse_remote_url("https://gitlab.example.com:8443/team/repo.git").unwrap();
    assert_eq!(c.host, "gitlab.example.com");
    assert_eq!(c.owner, "team");
    assert_eq!(c.repo, "repo");
}

#[test]
fn parse_http_scheme_remote() {
    let c = parse_remote_url("http://github.com/darkrun/factory").unwrap();
    assert_eq!(c.host, "github.com");
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_trims_surrounding_whitespace() {
    let c = parse_remote_url("  https://github.com/darkrun/factory.git\n").unwrap();
    assert_eq!(c.slug(), "darkrun/factory");
}

#[test]
fn parse_trailing_slash_tolerated() {
    let c = parse_remote_url("https://github.com/darkrun/factory/").unwrap();
    assert_eq!(c.repo, "factory");
}

#[test]
fn parse_self_hosted_gitlab_host() {
    let c = parse_remote_url("git@gitlab.internal.example.org:platform/factory.git").unwrap();
    assert_eq!(c.host, "gitlab.internal.example.org");
    assert_eq!(c.owner, "platform");
}

#[test]
fn parse_empty_remote_errors() {
    let err = parse_remote_url("").unwrap_err();
    assert!(matches!(err, VcsError::RemoteParse(_)));
}

#[test]
fn parse_whitespace_only_remote_errors() {
    let err = parse_remote_url("   ").unwrap_err();
    assert!(matches!(err, VcsError::RemoteParse(_)));
}

#[test]
fn parse_remote_missing_repo_segment_errors() {
    // Only an owner, no repo.
    let err = parse_remote_url("https://github.com/darkrun").unwrap_err();
    assert!(matches!(err, VcsError::RemoteParse(_)));
}

#[test]
fn parse_scp_missing_repo_segment_errors() {
    let err = parse_remote_url("git@github.com:darkrun").unwrap_err();
    assert!(matches!(err, VcsError::RemoteParse(_)));
}

#[test]
fn parse_url_form_no_path_errors() {
    let err = parse_remote_url("https://github.com").unwrap_err();
    assert!(matches!(err, VcsError::RemoteParse(_)));
}

#[test]
fn parse_github_coords_infer_provider() {
    let c = parse_remote_url("https://github.com/darkrun/factory.git").unwrap();
    assert_eq!(c.provider(), Some(Provider::GitHub));
}

#[test]
fn parse_gitlab_coords_infer_provider() {
    let c = parse_remote_url("https://gitlab.com/darkrun/factory.git").unwrap();
    assert_eq!(c.provider(), Some(Provider::GitLab));
}

#[test]
fn parsed_coords_drive_full_github_flow() {
    // Parse → create PR, with the request URL derived from the parsed coords.
    let coords = parse_remote_url("git@github.com:acme/service.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(2, "acme/service")),
    );

    let provider = coords.provider().unwrap();
    let cr = create_change_request(
        &mock,
        provider,
        &Credential::new(provider, "tok"),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    assert_eq!(cr.provider, Provider::GitHub);
    assert_eq!(
        mock.single_request().url,
        "https://api.github.com/repos/acme/service/pulls"
    );
}

#[test]
fn parsed_coords_drive_full_gitlab_flow() {
    let coords = parse_remote_url("git@gitlab.com:team/group/service.git").unwrap();
    let encoded = "team%2Fgroup%2Fservice";
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url(encoded),
        HttpResponse::new(200, gl_project_body(99, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(99),
        HttpResponse::new(201, gl_mr_body(8, 99)),
    );

    let provider = coords.provider().unwrap();
    let cr = create_change_request(
        &mock,
        provider,
        &Credential::new(provider, "tok"),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    assert_eq!(cr.provider, Provider::GitLab);
    assert_eq!(cr.number, 8);
    assert_eq!(mock.requests()[0].url, gl_project_url(encoded));
}

// ===========================================================================
// API error handling: 422 / 404 / auth, at each stage of the flow
// ===========================================================================

#[test]
fn github_pr_422_surfaces_api_error() {
    // 422 = validation (e.g. a PR already exists for this head/base).
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(
            422,
            r#"{"message":"A pull request already exists for darkrun:station-build."}"#,
        ),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();

    match err {
        VcsError::Api {
            provider,
            status,
            message,
        } => {
            assert_eq!(provider, "GitHub");
            assert_eq!(status, 422);
            assert!(message.contains("already exists"));
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn github_pr_422_through_create_change_request() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(422, r#"{"message":"Validation Failed"}"#),
    );

    let err = create_change_request(
        &mock,
        Provider::GitHub,
        &github_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();
    assert!(matches!(err, VcsError::Api { status: 422, .. }));
}

#[test]
fn github_pr_404_surfaces_api_error() {
    // 404 = repo not found (or token lacks visibility).
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(404, r#"{"message":"Not Found"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    match err {
        VcsError::Api { status, message, .. } => {
            assert_eq!(status, 404);
            assert_eq!(message, "Not Found");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn github_pr_401_surfaces_auth_error() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(401, r#"{"message":"Bad credentials"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    match err {
        VcsError::Api { status, message, .. } => {
            assert_eq!(status, 401);
            assert_eq!(message, "Bad credentials");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn github_pr_403_rate_limited_surfaces_api_error() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(403, r#"{"message":"API rate limit exceeded"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    assert!(matches!(err, VcsError::Api { status: 403, .. }));
}

#[test]
fn github_get_repo_404_surfaces_api_error() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gh_repo_url(&coords),
        HttpResponse::new(404, r#"{"message":"Not Found"}"#),
    );

    let err = github_get_repo(&mock, &github_cred(), &coords).unwrap_err();
    assert!(matches!(err, VcsError::Api { status: 404, .. }));
}

#[test]
fn github_error_uses_message_field() {
    // The error message must be lifted from the `message` key.
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(422, r#"{"message":"precise reason"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    if let VcsError::Api { message, .. } = err {
        assert_eq!(message, "precise reason");
    } else {
        panic!("expected Api");
    }
}

#[test]
fn github_error_non_json_body_falls_back_to_text() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(500, "internal server error, not json"),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    if let VcsError::Api { status, message, .. } = err {
        assert_eq!(status, 500);
        assert!(message.contains("internal server error"));
    } else {
        panic!("expected Api");
    }
}

#[test]
fn gitlab_resolve_404_surfaces_api_error() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(404, r#"{"message":"404 Project Not Found"}"#),
    );

    let err = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap_err();
    match err {
        VcsError::Api {
            provider,
            status,
            message,
        } => {
            assert_eq!(provider, "GitLab");
            assert_eq!(status, 404);
            assert!(message.contains("Not Found"));
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn gitlab_resolve_401_surfaces_auth_error() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(401, r#"{"message":"401 Unauthorized"}"#),
    );

    let err = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap_err();
    assert!(matches!(err, VcsError::Api { status: 401, .. }));
}

#[test]
fn gitlab_mr_409_after_successful_resolve() {
    // Resolve succeeds, but the MR POST conflicts (e.g. one already open).
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(7001, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(
            409,
            r#"{"message":["Another open merge request already exists"]}"#,
        ),
    );

    let err = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();

    assert!(matches!(err, VcsError::Api { status: 409, .. }));
    // Both calls were made: resolve succeeded, create failed.
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn gitlab_mr_422_surfaces_api_error() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(422, r#"{"error":"branch not found"}"#),
    );

    let err = gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY)
        .unwrap_err();
    match err {
        VcsError::Api { status, message, .. } => {
            assert_eq!(status, 422);
            // GitLab uses `error` for some messages; the extractor falls back to it.
            assert_eq!(message, "branch not found");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[test]
fn gitlab_error_message_array_falls_back_to_text() {
    // GitLab sometimes returns `message` as an array, not a string; the
    // string-only extractor falls back to the raw body text.
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(400, r#"{"message":["a","b"]}"#),
    );

    let err = gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY)
        .unwrap_err();
    if let VcsError::Api { status, message, .. } = err {
        assert_eq!(status, 400);
        assert!(message.contains("\"a\""));
    } else {
        panic!("expected Api");
    }
}

#[test]
fn transport_failure_surfaces_as_transport_error() {
    // No response queued → the mock returns a transport-level error, which must
    // propagate as VcsError::Transport, distinct from an Api error.
    let coords = github_coords();
    let mock = MockTransport::new();

    let err = create_change_request(
        &mock,
        Provider::GitHub,
        &github_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();

    assert!(matches!(err, VcsError::Transport(_)));
}

#[test]
fn gitlab_transport_failure_on_resolve() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    // Nothing queued — the resolve GET fails at the transport layer.
    let err = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();
    assert!(matches!(err, VcsError::Transport(_)));
}

#[test]
fn github_success_body_with_missing_number_errors() {
    // A 2xx with a malformed body must not silently succeed.
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, r#"{"html_url":"https://github.com/x/pull/1"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    assert!(matches!(err, VcsError::MissingField("number")));
}

#[test]
fn github_success_body_with_missing_html_url_errors() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, r#"{"number":1}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    assert!(matches!(err, VcsError::MissingField("html_url")));
}

#[test]
fn github_invalid_json_success_body_errors() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, "not json at all"),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    assert!(matches!(err, VcsError::Json(_)));
}

#[test]
fn api_error_display_includes_provider_and_status() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(422, r#"{"message":"boom"}"#),
    );

    let err =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("GitHub"));
    assert!(text.contains("422"));
    assert!(text.contains("boom"));
}

// ===========================================================================
// Provider metadata used to route the Checkpoint handoff
// ===========================================================================

#[test]
fn provider_keys_are_stable() {
    assert_eq!(Provider::GitHub.key(), "github");
    assert_eq!(Provider::GitLab.key(), "gitlab");
}

#[test]
fn provider_display_names() {
    assert_eq!(Provider::GitHub.display_name(), "GitHub");
    assert_eq!(Provider::GitLab.display_name(), "GitLab");
}

#[test]
fn provider_api_bases() {
    assert_eq!(Provider::GitHub.api_base(), "https://api.github.com");
    assert_eq!(Provider::GitLab.api_base(), "https://gitlab.com/api/v4");
}

#[test]
fn provider_oauth_scopes() {
    assert_eq!(Provider::GitHub.oauth_scope(), "repo");
    assert_eq!(Provider::GitLab.oauth_scope(), "api");
}

#[test]
fn provider_from_key_aliases() {
    assert_eq!(Provider::from_key("gh"), Some(Provider::GitHub));
    assert_eq!(Provider::from_key("github"), Some(Provider::GitHub));
    assert_eq!(Provider::from_key("gl"), Some(Provider::GitLab));
    assert_eq!(Provider::from_key("gitlab"), Some(Provider::GitLab));
    assert_eq!(Provider::from_key("svn"), None);
}

#[test]
fn provider_from_key_is_case_insensitive() {
    assert_eq!(Provider::from_key("GitHub"), Some(Provider::GitHub));
    assert_eq!(Provider::from_key("  GITLAB "), Some(Provider::GitLab));
}

#[test]
fn provider_from_host_github_variants() {
    assert_eq!(Provider::from_host("github.com"), Some(Provider::GitHub));
    assert_eq!(
        Provider::from_host("api.github.com"),
        Some(Provider::GitHub)
    );
}

#[test]
fn provider_from_host_gitlab_variants() {
    assert_eq!(Provider::from_host("gitlab.com"), Some(Provider::GitLab));
    assert_eq!(
        Provider::from_host("gitlab.example.org"),
        Some(Provider::GitLab)
    );
}

#[test]
fn provider_from_host_unknown_is_none() {
    assert_eq!(Provider::from_host("bitbucket.org"), None);
}

#[test]
fn credential_authorization_header_is_bearer() {
    let cred = Credential::new(Provider::GitHub, "abc123");
    assert_eq!(cred.authorization_header(), "Bearer abc123");
}

#[test]
fn credential_new_has_no_refresh_token() {
    let cred = Credential::new(Provider::GitLab, "x");
    assert!(cred.refresh_token.is_none());
    assert!(cred.expires_in.is_none());
}

#[test]
fn coords_slug_and_project_path_match_for_flat_repo() {
    let c = RepoCoords::new("github.com", "darkrun", "factory");
    assert_eq!(c.slug(), "darkrun/factory");
    assert_eq!(c.project_path(), "darkrun/factory");
}

#[test]
fn coords_project_path_includes_subgroups() {
    let c = RepoCoords::new("gitlab.com", "group/sub", "factory");
    assert_eq!(c.project_path(), "group/sub/factory");
}

// ===========================================================================
// Checkpoint payload fidelity: titles/bodies that humans actually write
// ===========================================================================

#[test]
fn github_pr_preserves_multiline_body() {
    let coords = github_coords();
    let body = "Summary line.\n\n- item one\n- item two\n\nReady for review.";
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, body).unwrap();
    let sent = body_json(&mock.single_request());
    assert_eq!(sent["body"], body);
}

#[test]
fn github_pr_preserves_unicode_title() {
    let coords = github_coords();
    let title = "Checkpoint: réviser la station — build ✓";
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, title, BODY).unwrap();
    assert_eq!(body_json(&mock.single_request())["title"], title);
}

#[test]
fn github_pr_preserves_empty_body() {
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, "").unwrap();
    assert_eq!(body_json(&mock.single_request())["body"], "");
}

#[test]
fn gitlab_mr_preserves_body_with_quotes_and_backslashes() {
    let body = r#"Path "C:\runs\factory" needs review; see the "build" station."#;
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(1, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, body).unwrap();
    assert_eq!(body_json(&mock.single_request())["description"], body);
}

#[test]
fn github_pr_preserves_slashes_in_head_branch() {
    // Worker branches commonly carry slashes; they must survive into the body.
    let coords = github_coords();
    let head = "darkrun/station/build/pass-3";
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(1, "darkrun/factory")),
    );

    github_create_pull_request(&mock, &github_cred(), &coords, head, BASE, TITLE, BODY).unwrap();
    assert_eq!(body_json(&mock.single_request())["head"], head);
}

// ===========================================================================
// Additional resolve/create wiring and error coverage
// ===========================================================================

#[test]
fn gitlab_resolve_request_is_a_get_with_no_body() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(1, "main")),
    );

    gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    let req = mock.single_request();
    assert_eq!(req.method, Method::Get);
    assert!(req.body.is_none());
}

#[test]
fn gitlab_resolve_carries_bearer_auth() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(1, "main")),
    );

    gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap();
    assert_eq!(
        header(&mock.single_request(), "Authorization"),
        Some("Bearer gl-checkpoint-token")
    );
}

#[test]
fn gitlab_full_flow_uses_same_token_on_both_calls() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(7001, "main")),
    );
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(1, 7001)),
    );

    create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    let reqs = mock.requests();
    assert_eq!(
        header(&reqs[0], "Authorization"),
        Some("Bearer gl-checkpoint-token")
    );
    assert_eq!(
        header(&reqs[1], "Authorization"),
        Some("Bearer gl-checkpoint-token")
    );
}

#[test]
fn gitlab_mr_post_sets_json_content_type() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(1, 7001)),
    );

    gitlab_create_merge_request(&mock, &gitlab_cred(), 7001, HEAD, BASE, TITLE, BODY).unwrap();
    assert_eq!(
        header(&mock.single_request(), "Content-Type"),
        Some("application/json")
    );
}

#[test]
fn github_pr_500_then_retry_succeeds() {
    // The transport serves queued responses FIFO; a first 500 then a 201 models
    // a caller-side retry. Two separate calls, two recorded requests.
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(500, r#"{"message":"server error"}"#),
    );
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(2, "darkrun/factory")),
    );

    let first =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY);
    assert!(matches!(first, Err(VcsError::Api { status: 500, .. })));

    let second =
        github_create_pull_request(&mock, &github_cred(), &coords, HEAD, BASE, TITLE, BODY)
            .unwrap();
    assert_eq!(second.number, 2);
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn github_get_repo_then_create_pr_full_sequence() {
    // A Run can inspect the repo (e.g. to learn the default base branch) before
    // opening the PR. Both calls flow through one transport, in order.
    let coords = github_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gh_repo_url(&coords),
        HttpResponse::new(
            200,
            r#"{"id":1,"default_branch":"release","html_url":"https://github.com/darkrun/factory"}"#,
        ),
    );
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(3, "darkrun/factory")),
    );

    let info = github_get_repo(&mock, &github_cred(), &coords).unwrap();
    let cr = github_create_pull_request(
        &mock,
        &github_cred(),
        &coords,
        HEAD,
        &info.default_branch,
        TITLE,
        BODY,
    )
    .unwrap();

    assert_eq!(cr.number, 3);
    let reqs = mock.requests();
    assert_eq!(reqs[0].method, Method::Get);
    assert_eq!(reqs[1].method, Method::Post);
    // The PR targeted the discovered default branch.
    assert_eq!(body_json(&reqs[1])["base"], "release");
}

#[test]
fn gitlab_resolve_invalid_json_body_errors() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, "<html>not json</html>"),
    );

    let err = gitlab_resolve_project(&mock, &gitlab_cred(), &coords).unwrap_err();
    assert!(matches!(err, VcsError::Json(_)));
}

#[test]
fn gitlab_403_on_resolve_surfaces_api_error() {
    let coords = gitlab_coords();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(403, r#"{"message":"403 Forbidden"}"#),
    );

    let err = create_change_request(
        &mock,
        Provider::GitLab,
        &gitlab_cred(),
        &coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap_err();
    assert!(matches!(err, VcsError::Api { status: 403, .. }));
}

#[test]
fn change_request_equality_is_structural() {
    let a = ChangeRequest {
        provider: Provider::GitHub,
        url: "https://github.com/x/pull/1".into(),
        number: 1,
    };
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn github_and_gitlab_flows_are_independent_transports() {
    // Two providers, two mocks, two distinct change requests in one test body —
    // confirms neither flow leaks request state into the other.
    let gh_coords = github_coords();
    let gh_mock = MockTransport::new();
    gh_mock.expect(
        Method::Post,
        gh_pulls_url(&gh_coords),
        HttpResponse::new(201, gh_pr_body(10, "darkrun/factory")),
    );

    let gl_coords = gitlab_coords();
    let gl_mock = MockTransport::new();
    gl_mock.expect(
        Method::Get,
        gl_project_url("darkrun%2Ffactory"),
        HttpResponse::new(200, gl_project_body(7001, "main")),
    );
    gl_mock.expect(
        Method::Post,
        gl_mr_url(7001),
        HttpResponse::new(201, gl_mr_body(20, 7001)),
    );

    let gh = create_change_request(
        &gh_mock,
        Provider::GitHub,
        &github_cred(),
        &gh_coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();
    let gl = create_change_request(
        &gl_mock,
        Provider::GitLab,
        &gitlab_cred(),
        &gl_coords,
        HEAD,
        BASE,
        TITLE,
        BODY,
    )
    .unwrap();

    assert_eq!(gh.number, 10);
    assert_eq!(gl.number, 20);
    assert_eq!(gh_mock.requests().len(), 1);
    assert_eq!(gl_mock.requests().len(), 2);
}

#[test]
fn parse_remote_then_store_then_create_full_handoff() {
    // The whole external-Checkpoint handoff in one flow: parse the remote,
    // load the saved credential, and open the change request against it.
    let coords = parse_remote_url("git@github.com:darkrun/factory.git").unwrap();
    let provider = coords.provider().unwrap();

    let (_dir, store) = temp_store();
    store
        .save(&Credential::new(provider, "handoff-token"))
        .unwrap();
    let cred = store.get(provider).unwrap().unwrap();

    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        gh_pulls_url(&coords),
        HttpResponse::new(201, gh_pr_body(99, "darkrun/factory")),
    );

    let cr = create_change_request(
        &mock, provider, &cred, &coords, HEAD, BASE, TITLE, BODY,
    )
    .unwrap();

    assert_eq!(cr.number, 99);
    assert_eq!(cr.url, "https://github.com/darkrun/factory/pull/99");
    assert_eq!(
        header(&mock.single_request(), "Authorization"),
        Some("Bearer handoff-token")
    );
}

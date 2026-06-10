//! GitHub PR / GitLab MR REST client tests (request shape + response parse).

use darkrun_vcs::rest::{
    create_change_request, github_create_pull_request, github_get_repo, github_mark_ready,
    gitlab_create_merge_request, gitlab_mark_ready, gitlab_resolve_project,
};
use darkrun_vcs::transport::{HttpResponse, Method};
use darkrun_vcs::{parse_remote_url, Credential, MockTransport, Provider, RepoCoords, VcsError};

fn gh_cred() -> Credential {
    Credential::new(Provider::GitHub, "gho_tok")
}
fn gl_cred() -> Credential {
    Credential::new(Provider::GitLab, "glpat")
}

#[test]
fn github_create_pr_builds_request_and_parses_response() {
    let coords = parse_remote_url("https://github.com/jwaldrip/darkrun.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        "https://api.github.com/repos/jwaldrip/darkrun/pulls",
        HttpResponse::new(
            201,
            r#"{"number":42,"html_url":"https://github.com/jwaldrip/darkrun/pull/42"}"#,
        ),
    );

    let cr = github_create_pull_request(
        &mock,
        &gh_cred(),
        &coords,
        "feature-branch",
        "main",
        "Add thing",
        "body text",
    )
    .unwrap();

    assert_eq!(cr.provider, Provider::GitHub);
    assert_eq!(cr.number, 42);
    assert_eq!(cr.url, "https://github.com/jwaldrip/darkrun/pull/42");

    let req = mock.single_request();
    let body: serde_json::Value =
        serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["title"], "Add thing");
    assert_eq!(body["head"], "feature-branch");
    assert_eq!(body["base"], "main");
    assert_eq!(body["body"], "body text");

    // GitHub requires auth + user-agent + api-version headers.
    let has = |k: &str, v: &str| req.headers.iter().any(|(hk, hv)| hk == k && hv == v);
    assert!(has("Authorization", "Bearer gho_tok"));
    assert!(has("User-Agent", "darkrun-vcs"));
    assert!(has("X-GitHub-Api-Version", "2022-11-28"));
    assert!(req
        .headers
        .iter()
        .any(|(k, v)| k == "Accept" && v == "application/vnd.github+json"));
}

#[test]
fn github_create_pr_error_is_typed() {
    let coords = RepoCoords::new("github.com", "o", "r");
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        "https://api.github.com/repos/o/r/pulls",
        HttpResponse::new(
            422,
            r#"{"message":"Validation Failed","errors":[{"field":"head"}]}"#,
        ),
    );
    let err =
        github_create_pull_request(&mock, &gh_cred(), &coords, "h", "b", "t", "d").unwrap_err();
    match err {
        VcsError::Api { provider, status, message } => {
            assert_eq!(provider, "GitHub");
            assert_eq!(status, 422);
            assert_eq!(message, "Validation Failed");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn github_get_repo_parses_default_branch() {
    let coords = RepoCoords::new("github.com", "o", "r");
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://api.github.com/repos/o/r",
        HttpResponse::new(
            200,
            r#"{"id":99,"default_branch":"trunk","html_url":"https://github.com/o/r"}"#,
        ),
    );
    let info = github_get_repo(&mock, &gh_cred(), &coords).unwrap();
    assert_eq!(info.id, 99);
    assert_eq!(info.default_branch, "trunk");
    assert_eq!(info.web_url, "https://github.com/o/r");
}

#[test]
fn gitlab_resolve_project_encodes_path() {
    let coords = parse_remote_url("https://gitlab.com/group/subgroup/project.git").unwrap();
    let mock = MockTransport::new();
    // The project path must be URL-encoded into a single path segment.
    mock.expect(
        Method::Get,
        "https://gitlab.com/api/v4/projects/group%2Fsubgroup%2Fproject",
        HttpResponse::new(
            200,
            r#"{"id":777,"default_branch":"main","web_url":"https://gitlab.com/group/subgroup/project"}"#,
        ),
    );
    let info = gitlab_resolve_project(&mock, &gl_cred(), &coords).unwrap();
    assert_eq!(info.id, 777);
    assert_eq!(info.default_branch, "main");
}

#[test]
fn gitlab_create_mr_builds_request_and_parses_response() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        "https://gitlab.com/api/v4/projects/777/merge_requests",
        HttpResponse::new(
            201,
            r#"{"iid":7,"web_url":"https://gitlab.com/group/project/-/merge_requests/7"}"#,
        ),
    );
    let cr = gitlab_create_merge_request(
        &mock, &gl_cred(), 777, "feature", "main", "Title", "desc",
    )
    .unwrap();
    assert_eq!(cr.provider, Provider::GitLab);
    assert_eq!(cr.number, 7);
    assert_eq!(cr.url, "https://gitlab.com/group/project/-/merge_requests/7");

    let req = mock.single_request();
    let body: serde_json::Value =
        serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["source_branch"], "feature");
    assert_eq!(body["target_branch"], "main");
    assert_eq!(body["title"], "Title");
    assert_eq!(body["description"], "desc");
}

#[test]
fn create_change_request_github_dispatches_to_pr() {
    let coords = parse_remote_url("git@github.com:o/r.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        "https://api.github.com/repos/o/r/pulls",
        HttpResponse::new(201, r#"{"number":1,"html_url":"https://github.com/o/r/pull/1"}"#),
    );
    let cr = create_change_request(
        &mock, Provider::GitHub, &gh_cred(), &coords, "h", "main", "t", "b",
    )
    .unwrap();
    assert_eq!(cr.number, 1);
    // Exactly one call: GitHub doesn't need a resolve step.
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn create_change_request_gitlab_resolves_then_creates() {
    let coords = parse_remote_url("https://gitlab.com/group/project.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://gitlab.com/api/v4/projects/group%2Fproject",
        HttpResponse::new(200, r#"{"id":555,"default_branch":"main","web_url":"x"}"#),
    );
    mock.expect(
        Method::Post,
        "https://gitlab.com/api/v4/projects/555/merge_requests",
        HttpResponse::new(
            201,
            r#"{"iid":3,"web_url":"https://gitlab.com/group/project/-/merge_requests/3"}"#,
        ),
    );
    let cr = create_change_request(
        &mock, Provider::GitLab, &gl_cred(), &coords, "feature", "main", "t", "b",
    )
    .unwrap();
    assert_eq!(cr.number, 3);
    // Two calls in order: resolve project, then create MR.
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].method, Method::Get);
    assert_eq!(reqs[1].method, Method::Post);
    assert!(reqs[1].url.contains("/projects/555/merge_requests"));
}

#[test]
fn gitlab_resolve_project_404_is_typed_error() {
    let coords = RepoCoords::new("gitlab.com", "group", "missing");
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://gitlab.com/api/v4/projects/group%2Fmissing",
        HttpResponse::new(404, r#"{"message":"404 Project Not Found"}"#),
    );
    let err = gitlab_resolve_project(&mock, &gl_cred(), &coords).unwrap_err();
    match err {
        VcsError::Api { provider, status, message } => {
            assert_eq!(provider, "GitLab");
            assert_eq!(status, 404);
            assert_eq!(message, "404 Project Not Found");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[test]
fn github_mark_ready_resolves_node_id_then_mutates_via_graphql() {
    let coords = parse_remote_url("https://github.com/jwaldrip/darkrun.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://api.github.com/repos/jwaldrip/darkrun/pulls/42",
        HttpResponse::new(200, r#"{"node_id":"PR_abc123","draft":true,"state":"open"}"#),
    );
    mock.expect(
        Method::Post,
        "https://api.github.com/graphql",
        HttpResponse::new(
            200,
            r#"{"data":{"markPullRequestReadyForReview":{"pullRequest":{"isDraft":false}}}}"#,
        ),
    );

    github_mark_ready(&mock, &gh_cred(), &coords, 42).unwrap();

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "GET node_id then POST graphql");
    let gql: serde_json::Value = serde_json::from_slice(reqs[1].body.as_ref().unwrap()).unwrap();
    assert!(gql["query"]
        .as_str()
        .unwrap()
        .contains("markPullRequestReadyForReview"));
    assert_eq!(gql["variables"]["id"], "PR_abc123");
}

#[test]
fn github_mark_ready_surfaces_graphql_errors() {
    let coords = parse_remote_url("https://github.com/jwaldrip/darkrun.git").unwrap();
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://api.github.com/repos/jwaldrip/darkrun/pulls/42",
        HttpResponse::new(200, r#"{"node_id":"PR_abc123"}"#),
    );
    mock.expect(
        Method::Post,
        "https://api.github.com/graphql",
        HttpResponse::new(200, r#"{"errors":[{"message":"not permitted"}]}"#),
    );
    let err = github_mark_ready(&mock, &gh_cred(), &coords, 42).unwrap_err();
    assert!(format!("{err}").contains("markPullRequestReadyForReview"), "{err}");
}

#[test]
fn gitlab_mark_ready_strips_the_draft_prefix_via_put() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://gitlab.com/api/v4/projects/77/merge_requests/9",
        HttpResponse::new(200, r#"{"title":"Draft: Ship the thing","state":"opened"}"#),
    );
    mock.expect(
        Method::Put,
        "https://gitlab.com/api/v4/projects/77/merge_requests/9",
        HttpResponse::new(200, r#"{"title":"Ship the thing"}"#),
    );

    gitlab_mark_ready(&mock, &gl_cred(), 77, 9).unwrap();

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2, "GET title then PUT stripped title");
    let put: serde_json::Value = serde_json::from_slice(reqs[1].body.as_ref().unwrap()).unwrap();
    assert_eq!(put["title"], "Ship the thing");
}

#[test]
fn gitlab_mark_ready_noops_when_already_ready() {
    let mock = MockTransport::new();
    mock.expect(
        Method::Get,
        "https://gitlab.com/api/v4/projects/77/merge_requests/9",
        HttpResponse::new(200, r#"{"title":"Ship the thing","state":"opened"}"#),
    );
    gitlab_mark_ready(&mock, &gl_cred(), 77, 9).unwrap();
    assert_eq!(mock.requests().len(), 1, "no PUT when the title is un-prefixed");
}

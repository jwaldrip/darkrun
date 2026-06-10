//! REST clients for GitHub Pull Requests and GitLab Merge Requests, plus the
//! unified [`create_change_request`] entry point.
//!
//! Every call goes through the injectable [`HttpTransport`]. The GitHub and
//! GitLab clients build the provider-specific request shape and parse the
//! provider-specific response into a normalized [`ChangeRequest`].

use crate::error::{Result, VcsError};
use crate::oauth::percent_encode;
use crate::provider::{Credential, Provider};
use crate::remote::RepoCoords;
use crate::transport::{HttpRequest, HttpResponse, HttpTransport};

/// A user-agent string. GitHub rejects API requests without one.
const USER_AGENT: &str = "darkrun-vcs";

/// A created change request (a GitHub PR or a GitLab MR), normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    /// The provider that owns this change request.
    pub provider: Provider,
    /// The web URL where a reviewer lands.
    pub url: String,
    /// The provider-assigned number (PR number / MR iid).
    pub number: u64,
}

/// Minimal repository facts returned by the get-repo / resolve-project calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// The provider's numeric id for the repo/project.
    pub id: u64,
    /// The default branch (PR/MR base when the caller does not override).
    pub default_branch: String,
    /// The web URL of the repository.
    pub web_url: String,
}

/// Apply the standard auth + accept headers for `provider` to `request`.
fn authorize(request: HttpRequest, provider: Provider, cred: &Credential) -> HttpRequest {
    let request = request
        .header("Authorization", cred.authorization_header())
        .header("User-Agent", USER_AGENT);
    match provider {
        Provider::GitHub => request
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28"),
        Provider::GitLab => request.header("Accept", "application/json"),
    }
}

/// Turn a non-2xx response into a typed [`VcsError::Api`], extracting the
/// provider's error message where possible.
fn api_error(provider: Provider, response: &HttpResponse) -> VcsError {
    let message = response
        .json::<serde_json::Value>()
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("error_description"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .or_else(|| response.text().ok())
        .unwrap_or_default();
    VcsError::Api {
        provider: provider.display_name(),
        status: response.status,
        message,
    }
}

/// GitHub: `GET /repos/{owner}/{repo}`.
pub fn github_get_repo(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
) -> Result<RepoInfo> {
    let url = format!(
        "{base}/repos/{owner}/{repo}",
        base = Provider::GitHub.api_base(),
        owner = coords.owner,
        repo = coords.repo,
    );
    let request = authorize(HttpRequest::get(url), Provider::GitHub, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    let value: serde_json::Value = response.json()?;
    Ok(RepoInfo {
        id: value.get("id").and_then(|v| v.as_u64()).unwrap_or(0),
        default_branch: value
            .get("default_branch")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string(),
        web_url: value
            .get("html_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// GitHub: `POST /repos/{owner}/{repo}/pulls` — opens a ready (non-draft) PR.
pub fn github_create_pull_request(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<ChangeRequest> {
    github_create_pull_request_with(transport, cred, coords, head, base, title, body, false)
}

/// GitHub: `POST /repos/{owner}/{repo}/pulls`. `draft` opens it as a draft PR.
#[allow(clippy::too_many_arguments)]
pub fn github_create_pull_request_with(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<ChangeRequest> {
    let url = format!(
        "{base}/repos/{owner}/{repo}/pulls",
        base = Provider::GitHub.api_base(),
        owner = coords.owner,
        repo = coords.repo,
    );
    let payload = serde_json::json!({
        "title": title,
        "head": head,
        "base": base,
        "body": body,
        "draft": draft,
    });
    let request =
        authorize(HttpRequest::post(url), Provider::GitHub, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    let value: serde_json::Value = response.json()?;
    Ok(ChangeRequest {
        provider: Provider::GitHub,
        number: value
            .get("number")
            .and_then(|v| v.as_u64())
            .ok_or(VcsError::MissingField("number"))?,
        url: value
            .get("html_url")
            .and_then(|v| v.as_str())
            .ok_or(VcsError::MissingField("html_url"))?
            .to_string(),
    })
}

/// GitLab: resolve a project by its URL-encoded path → [`RepoInfo`].
///
/// `GET /projects/{url-encoded path}`.
pub fn gitlab_resolve_project(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
) -> Result<RepoInfo> {
    let encoded = percent_encode(&coords.project_path());
    let url = format!(
        "{base}/projects/{encoded}",
        base = Provider::GitLab.api_base(),
    );
    let request = authorize(HttpRequest::get(url), Provider::GitLab, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    let value: serde_json::Value = response.json()?;
    Ok(RepoInfo {
        id: value
            .get("id")
            .and_then(|v| v.as_u64())
            .ok_or(VcsError::MissingField("id"))?,
        default_branch: value
            .get("default_branch")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_string(),
        web_url: value
            .get("web_url")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

/// GitLab: `POST /projects/{id}/merge_requests`. A draft MR is marked by a
/// `Draft:` title prefix (GitLab's convention — there is no create-time flag),
/// which callers prepend to `title` themselves.
pub fn gitlab_create_merge_request(
    transport: &dyn HttpTransport,
    cred: &Credential,
    project_id: u64,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<ChangeRequest> {
    let url = format!(
        "{base}/projects/{id}/merge_requests",
        base = Provider::GitLab.api_base(),
        id = project_id,
    );
    let payload = serde_json::json!({
        "source_branch": head,
        "target_branch": base,
        "title": title,
        "description": body,
    });
    let request =
        authorize(HttpRequest::post(url), Provider::GitLab, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    let value: serde_json::Value = response.json()?;
    Ok(ChangeRequest {
        provider: Provider::GitLab,
        number: value
            .get("iid")
            .and_then(|v| v.as_u64())
            .ok_or(VcsError::MissingField("iid"))?,
        url: value
            .get("web_url")
            .and_then(|v| v.as_str())
            .ok_or(VcsError::MissingField("web_url"))?
            .to_string(),
    })
}

/// Create a change request on `provider`, dispatching to the PR or MR client.
///
/// For GitLab this first resolves the project to obtain its numeric id, then
/// opens the merge request against it. For GitHub it posts the pull request
/// directly against `owner/repo`.
#[allow(clippy::too_many_arguments)]
pub fn create_change_request(
    transport: &dyn HttpTransport,
    provider: Provider,
    cred: &Credential,
    coords: &RepoCoords,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<ChangeRequest> {
    match provider {
        Provider::GitHub => {
            github_create_pull_request(transport, cred, coords, head, base, title, body)
        }
        Provider::GitLab => {
            let project = gitlab_resolve_project(transport, cred, coords)?;
            gitlab_create_merge_request(transport, cred, project.id, head, base, title, body)
        }
    }
}

/// The merge state of a change request, normalized across providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRequestState {
    /// Open and not yet merged.
    Open,
    /// Merged.
    Merged,
    /// Closed without merging.
    Closed,
}

/// A change request's poll-time view: its merge state and draft flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeRequestView {
    /// The merge state.
    pub state: ChangeRequestState,
    /// Whether it is still a draft / work-in-progress.
    pub draft: bool,
}

/// A human review note pulled off a change request (a comment or review verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNote {
    /// A provider-stable id, prefixed by kind (`c<id>` comment, `r<id>` review).
    pub id: String,
    /// The note author's handle.
    pub author: String,
    /// The note's markdown body.
    pub body: String,
    /// Whether this is a change-request review (vs a plain comment).
    pub change_request: bool,
}

/// GitHub: `GET /repos/{o}/{r}/pulls/{n}` → merge state + draft flag.
pub fn github_pull_view(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    number: u64,
) -> Result<ChangeRequestView> {
    let url = format!(
        "{base}/repos/{owner}/{repo}/pulls/{number}",
        base = Provider::GitHub.api_base(),
        owner = coords.owner,
        repo = coords.repo,
    );
    let request = authorize(HttpRequest::get(url), Provider::GitHub, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    let v: serde_json::Value = response.json()?;
    let merged = v.get("merged").and_then(|x| x.as_bool()).unwrap_or(false);
    let closed = v.get("state").and_then(|x| x.as_str()) == Some("closed");
    let state = if merged {
        ChangeRequestState::Merged
    } else if closed {
        ChangeRequestState::Closed
    } else {
        ChangeRequestState::Open
    };
    Ok(ChangeRequestView {
        state,
        draft: v.get("draft").and_then(|x| x.as_bool()).unwrap_or(false),
    })
}

/// GitLab: `GET /projects/{id}/merge_requests/{iid}` → merge state + draft flag.
pub fn gitlab_mr_view(
    transport: &dyn HttpTransport,
    cred: &Credential,
    project_id: u64,
    iid: u64,
) -> Result<ChangeRequestView> {
    let url = format!(
        "{base}/projects/{id}/merge_requests/{iid}",
        base = Provider::GitLab.api_base(),
        id = project_id,
    );
    let request = authorize(HttpRequest::get(url), Provider::GitLab, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    let v: serde_json::Value = response.json()?;
    let state = match v.get("state").and_then(|x| x.as_str()) {
        Some("merged") => ChangeRequestState::Merged,
        Some("closed") | Some("locked") => ChangeRequestState::Closed,
        _ => ChangeRequestState::Open,
    };
    // GitLab reports draft via `draft` (and the legacy `work_in_progress`).
    let draft = v.get("draft").and_then(|x| x.as_bool()).or_else(|| {
        v.get("work_in_progress").and_then(|x| x.as_bool())
    });
    Ok(ChangeRequestView {
        state,
        draft: draft.unwrap_or(false),
    })
}

/// GitHub: flip a draft PR to ready-for-review. REST has no draft→ready
/// endpoint — it's GraphQL-only (`markPullRequestReadyForReview`), keyed by
/// the PR's GraphQL node id, so this is two calls: `GET /pulls/{n}` for the
/// `node_id`, then the mutation. (The predecessor learned this the same way.)
pub fn github_mark_ready(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    number: u64,
) -> Result<()> {
    let url = format!(
        "{base}/repos/{owner}/{repo}/pulls/{number}",
        base = Provider::GitHub.api_base(),
        owner = coords.owner,
        repo = coords.repo,
    );
    let request = authorize(HttpRequest::get(url), Provider::GitHub, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    let v: serde_json::Value = response.json()?;
    let node_id = v
        .get("node_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| VcsError::Api {
            provider: "github",
            status: 200,
            message: "pull view carries no node_id".into(),
        })?;
    let payload = serde_json::json!({
        "query": "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { isDraft } } }",
        "variables": { "id": node_id },
    });
    let gql = format!("{}/graphql", Provider::GitHub.api_base());
    let request = authorize(HttpRequest::post(gql), Provider::GitHub, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    let v: serde_json::Value = response.json()?;
    if let Some(errors) = v.get("errors").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            return Err(VcsError::Api {
                provider: "github",
                status: 200,
                message: format!("markPullRequestReadyForReview: {errors:?}"),
            });
        }
    }
    Ok(())
}

/// GitLab: flip a draft MR to ready. GitLab marks drafts by a `Draft:` title
/// prefix — `GET` the MR, strip the prefix, `PUT` the title back. No-op when
/// the title is already un-prefixed.
pub fn gitlab_mark_ready(
    transport: &dyn HttpTransport,
    cred: &Credential,
    project_id: u64,
    iid: u64,
) -> Result<()> {
    let url = format!(
        "{base}/projects/{id}/merge_requests/{iid}",
        base = Provider::GitLab.api_base(),
        id = project_id,
    );
    let request = authorize(HttpRequest::get(url.clone()), Provider::GitLab, cred);
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    let v: serde_json::Value = response.json()?;
    let title = v.get("title").and_then(|x| x.as_str()).unwrap_or_default();
    let stripped = title
        .strip_prefix("Draft:")
        .or_else(|| title.strip_prefix("Draft "))
        .or_else(|| title.strip_prefix("WIP:"))
        .map(str::trim_start);
    let Some(ready_title) = stripped else {
        return Ok(()); // already ready
    };
    let payload = serde_json::json!({ "title": ready_title });
    let request =
        authorize(HttpRequest::put(url), Provider::GitLab, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    Ok(())
}

/// GitHub: `POST /repos/{o}/{r}/issues/{n}/comments` — post a comment.
pub fn github_create_comment(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    number: u64,
    body: &str,
) -> Result<()> {
    let url = format!(
        "{base}/repos/{owner}/{repo}/issues/{number}/comments",
        base = Provider::GitHub.api_base(),
        owner = coords.owner,
        repo = coords.repo,
    );
    let payload = serde_json::json!({ "body": body });
    let request =
        authorize(HttpRequest::post(url), Provider::GitHub, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitHub, &response));
    }
    Ok(())
}

/// GitLab: `POST /projects/{id}/merge_requests/{iid}/notes` — post a note.
pub fn gitlab_create_note(
    transport: &dyn HttpTransport,
    cred: &Credential,
    project_id: u64,
    iid: u64,
    body: &str,
) -> Result<()> {
    let url = format!(
        "{base}/projects/{id}/merge_requests/{iid}/notes",
        base = Provider::GitLab.api_base(),
        id = project_id,
    );
    let payload = serde_json::json!({ "body": body });
    let request =
        authorize(HttpRequest::post(url), Provider::GitLab, cred).json_body(&payload)?;
    let response = transport.execute(request)?;
    if !response.is_success() {
        return Err(api_error(Provider::GitLab, &response));
    }
    Ok(())
}

/// GitHub: the human review notes on a PR — issue comments (`c<id>`) plus review
/// verdicts (`r<id>`), with `CHANGES_REQUESTED` flagged as a change request.
/// Two GETs: `/issues/{n}/comments` and `/pulls/{n}/reviews`.
pub fn github_review_notes(
    transport: &dyn HttpTransport,
    cred: &Credential,
    coords: &RepoCoords,
    number: u64,
) -> Result<Vec<RemoteNote>> {
    let base = Provider::GitHub.api_base();
    let mut out = Vec::new();

    // Issue comments.
    let curl = format!(
        "{base}/repos/{owner}/{repo}/issues/{number}/comments",
        owner = coords.owner,
        repo = coords.repo,
    );
    let resp = transport.execute(authorize(HttpRequest::get(curl), Provider::GitHub, cred))?;
    if !resp.is_success() {
        return Err(api_error(Provider::GitHub, &resp));
    }
    if let serde_json::Value::Array(items) = resp.json::<serde_json::Value>()? {
        for c in items {
            let Some(id) = c.get("id").and_then(|x| x.as_u64()) else {
                continue;
            };
            let body = c.get("body").and_then(|x| x.as_str()).unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            out.push(RemoteNote {
                id: format!("c{id}"),
                author: gh_login(&c),
                body: body.to_string(),
                change_request: false,
            });
        }
    }

    // Review verdicts.
    let rurl = format!(
        "{base}/repos/{owner}/{repo}/pulls/{number}/reviews",
        owner = coords.owner,
        repo = coords.repo,
    );
    let resp = transport.execute(authorize(HttpRequest::get(rurl), Provider::GitHub, cred))?;
    if !resp.is_success() {
        return Err(api_error(Provider::GitHub, &resp));
    }
    if let serde_json::Value::Array(items) = resp.json::<serde_json::Value>()? {
        for r in items {
            let Some(id) = r.get("id").and_then(|x| x.as_u64()) else {
                continue;
            };
            let state = r.get("state").and_then(|x| x.as_str()).unwrap_or_default();
            let change_request = state == "CHANGES_REQUESTED";
            let body = r.get("body").and_then(|x| x.as_str()).unwrap_or_default();
            // A bodyless non-change-request review (plain APPROVED) carries no
            // actionable feedback — skip it.
            if body.is_empty() && !change_request {
                continue;
            }
            out.push(RemoteNote {
                id: format!("r{id}"),
                author: gh_login(&r),
                body: body.to_string(),
                change_request,
            });
        }
    }
    Ok(out)
}

/// GitLab: the human notes on an MR (`GET /merge_requests/{iid}/notes`),
/// skipping system notes. GitLab notes carry no change-request verdict, so all
/// are plain comments.
pub fn gitlab_notes(
    transport: &dyn HttpTransport,
    cred: &Credential,
    project_id: u64,
    iid: u64,
) -> Result<Vec<RemoteNote>> {
    let url = format!(
        "{base}/projects/{id}/merge_requests/{iid}/notes",
        base = Provider::GitLab.api_base(),
        id = project_id,
    );
    let resp = transport.execute(authorize(HttpRequest::get(url), Provider::GitLab, cred))?;
    if !resp.is_success() {
        return Err(api_error(Provider::GitLab, &resp));
    }
    let mut out = Vec::new();
    if let serde_json::Value::Array(items) = resp.json::<serde_json::Value>()? {
        for n in items {
            if n.get("system").and_then(|x| x.as_bool()).unwrap_or(false) {
                continue; // skip auto-generated system notes
            }
            let Some(id) = n.get("id").and_then(|x| x.as_u64()) else {
                continue;
            };
            let body = n.get("body").and_then(|x| x.as_str()).unwrap_or_default();
            if body.is_empty() {
                continue;
            }
            let author = n
                .get("author")
                .and_then(|a| a.get("username"))
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            out.push(RemoteNote {
                id: id.to_string(),
                author,
                body: body.to_string(),
                change_request: false,
            });
        }
    }
    Ok(out)
}

/// The GitHub `user.login` of a comment/review object.
fn gh_login(v: &serde_json::Value) -> String {
    v.get("user")
        .and_then(|u| u.get("login"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

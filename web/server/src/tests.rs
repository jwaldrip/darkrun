//! In-process axum tests for the website host.
//!
//! Every test drives the router through `tower::ServiceExt::oneshot`, so no
//! socket is bound and nothing touches the network: the OAuth token exchange
//! runs against a [`MockTransport`] wrapped to be thread-safe, and the static
//! site is served from a `tempfile` directory.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use darkrun_vcs::{
    HttpRequest, HttpResponse, HttpTransport, Method, MockTransport, Provider, Result as VcsResult,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::broker::{Broker, Clock};
use crate::config::{ProviderCredentials, WebConfig};
use crate::oauth_routes::BrokerPayload;
use crate::state::{SharedTransport, WebState};
use crate::{build_oauth_only, build_router};

/// A `Send + Sync` wrapper over the single-threaded [`MockTransport`].
///
/// darkrun-vcs's mock uses `RefCell` internals and so is not `Sync`; the server
/// state requires `Send + Sync`. A `Mutex` bridges the gap for tests.
struct SyncMock(Mutex<MockTransport>);

impl SyncMock {
    fn new(mock: MockTransport) -> Self {
        Self(Mutex::new(mock))
    }
}

impl HttpTransport for SyncMock {
    fn execute(&self, request: HttpRequest) -> VcsResult<HttpResponse> {
        self.0.lock().expect("mock lock").execute(request)
    }
}

/// A transport that always errors — for the exchange-failure path.
struct FailingTransport;

impl HttpTransport for FailingTransport {
    fn execute(&self, _request: HttpRequest) -> VcsResult<HttpResponse> {
        Err(darkrun_vcs::VcsError::Transport("boom".into()))
    }
}

fn test_config() -> WebConfig {
    WebConfig::new(
        "https://darkrun.ai",
        Some(ProviderCredentials {
            client_id: "gh-client".into(),
            client_secret: "gh-secret".into(),
        }),
        Some(ProviderCredentials {
            client_id: "gl-client".into(),
            client_secret: "gl-secret".into(),
        }),
    )
}

fn state_with(transport: SharedTransport, broker: Broker) -> WebState {
    WebState::new(test_config(), broker, transport)
}

/// A state whose GitHub token exchange returns `tok` once.
fn github_exchange_state(tok: &str, broker: Broker) -> WebState {
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        Provider::GitHub.token_endpoint(),
        HttpResponse::new(
            200,
            serde_json::to_vec(&serde_json::json!({
                "access_token": tok,
                "token_type": "bearer"
            }))
            .unwrap(),
        ),
    );
    state_with(Arc::new(SyncMock::new(mock)), broker)
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ---- /auth/:provider/start --------------------------------------------------

#[tokio::test]
async fn start_redirects_github_with_correct_params() {
    let state = github_exchange_state("unused", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/start?state=nonce-xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let loc = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(loc.starts_with("https://github.com/login/oauth/authorize?"));
    assert!(loc.contains("client_id=gh-client"));
    assert!(loc.contains("state=nonce-xyz"));
    assert!(loc.contains("response_type=code"));
    assert!(loc.contains("scope=repo"));
    // redirect_uri is percent-encoded and provider-scoped.
    assert!(loc.contains("redirect_uri=https%3A%2F%2Fdarkrun.ai%2Fauth%2Fgithub%2Fcallback"));
}

#[tokio::test]
async fn start_redirects_gitlab_with_api_scope() {
    let state = github_exchange_state("unused", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/gitlab/start?state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    let loc = resp.headers().get("location").unwrap().to_str().unwrap();
    assert!(loc.starts_with("https://gitlab.com/oauth/authorize?"));
    assert!(loc.contains("client_id=gl-client"));
    assert!(loc.contains("scope=api"));
}

#[tokio::test]
async fn start_rejects_unknown_provider() {
    let state = github_exchange_state("unused", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/bitbucket/start?state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn start_requires_state() {
    let state = github_exchange_state("unused", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/start?state=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn start_unconfigured_provider_is_unavailable() {
    // Config with no GitLab credentials.
    let cfg = WebConfig::new(
        "https://darkrun.ai",
        Some(ProviderCredentials {
            client_id: "gh".into(),
            client_secret: "s".into(),
        }),
        None,
    );
    let mock = MockTransport::new();
    let state = WebState::new(cfg, Broker::new(), Arc::new(SyncMock::new(mock)));
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/gitlab/start?state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---- /auth/:provider/callback + /auth/broker/:nonce -------------------------

#[tokio::test]
async fn callback_exchanges_and_broker_returns_then_evicts() {
    let broker = Broker::new();
    let state = github_exchange_state("tok-live", broker.clone());
    let app = build_router(state, std::env::temp_dir());

    // Provider calls back with code + state.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?code=abc123&state=nonce-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("return to your terminal") || html.contains("Return to your terminal"));
    assert!(html.contains("GitHub"));

    // CLI claims the token once.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/broker/nonce-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let payload: BrokerPayload = serde_json::from_str(&body_string(resp).await).unwrap();
    assert_eq!(payload.provider, Provider::GitHub);
    assert_eq!(payload.access_token, "tok-live");

    // Second claim is evicted → 404.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/broker/nonce-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn broker_unknown_nonce_is_404() {
    let state = github_exchange_state("tok", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/broker/never-existed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn broker_expired_nonce_is_404() {
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};

    struct FrozenClock(StdMutex<Instant>);
    impl Clock for FrozenClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }
    let clock = Arc::new(FrozenClock(StdMutex::new(Instant::now())));
    let broker = Broker::with_clock(Duration::from_secs(1), clock.clone());

    let state = github_exchange_state("tok", broker.clone());
    let app = build_router(state, std::env::temp_dir());

    // Complete a callback so a token is parked.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?code=abc&state=will-expire")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Advance past the TTL.
    *clock.0.lock().unwrap() += Duration::from_secs(2);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/broker/will-expire")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn callback_missing_code_is_bad_request() {
    let state = github_exchange_state("tok", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?state=nonce-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_provider_error_renders_error_page() {
    let state = github_exchange_state("tok", Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?error=access_denied&error_description=nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let html = body_string(resp).await;
    assert!(html.contains("access_denied"));
}

#[tokio::test]
async fn callback_exchange_failure_is_bad_gateway() {
    let broker = Broker::new();
    let state = state_with(Arc::new(FailingTransport), broker.clone());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?code=abc&state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    // Nothing was parked.
    assert!(broker.is_empty());
}

#[tokio::test]
async fn callback_sends_correct_exchange_request() {
    // Verify the server-side exchange posts to the right endpoint with the code.
    let mock = MockTransport::new();
    mock.expect(
        Method::Post,
        Provider::GitHub.token_endpoint(),
        HttpResponse::new(
            200,
            serde_json::to_vec(&serde_json::json!({ "access_token": "t" })).unwrap(),
        ),
    );
    let sync = Arc::new(SyncMock::new(mock));
    let state = state_with(sync.clone(), Broker::new());
    let app = build_oauth_only(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/callback?code=THE_CODE&state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let reqs = sync.0.lock().unwrap().requests();
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    assert_eq!(req.url, Provider::GitHub.token_endpoint());
    let body: serde_json::Value = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["code"], "THE_CODE");
    assert_eq!(body["client_secret"], "gh-secret");
    assert_eq!(body["redirect_uri"], "https://darkrun.ai/auth/github/callback");
}

// ---- static site + SPA fallback ---------------------------------------------

#[tokio::test]
async fn static_serves_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>HOME</html>").unwrap();
    std::fs::write(dir.path().join("robots.txt"), "User-agent: *").unwrap();

    let state = github_exchange_state("x", Broker::new());
    let app = build_router(state, dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/robots.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("User-agent"));
}

#[tokio::test]
async fn static_fallback_serves_index_for_spa_route() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>SPA-SHELL</html>").unwrap();

    let state = github_exchange_state("x", Broker::new());
    let app = build_router(state, dir.path());

    // A client-side route with no matching file falls back to index.html.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/docs/getting-started")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("SPA-SHELL"));
}

#[tokio::test]
async fn static_serves_index_at_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>ROOT</html>").unwrap();

    let state = github_exchange_state("x", Broker::new());
    let app = build_router(state, dir.path());

    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("ROOT"));
}

#[tokio::test]
async fn oauth_routes_take_precedence_over_static_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html>SPA</html>").unwrap();

    let state = github_exchange_state("x", Broker::new());
    let app = build_router(state, dir.path());

    // /auth/... must hit the OAuth handler, not the SPA fallback.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/github/start?state=n1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
}

#[tokio::test]
async fn callback_unknown_provider_is_rejected() {
    let state = github_exchange_state("tok", Broker::new());
    let app = build_oauth_only(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/bogus/callback?code=c&state=s")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_client_error(), "unknown provider rejected");
}

#[tokio::test]
async fn callback_for_an_unconfigured_provider_is_unavailable() {
    // A config with NO GitLab credentials → a GitLab callback can't proceed.
    let cfg = WebConfig::new(
        "https://darkrun.ai",
        Some(ProviderCredentials { client_id: "gh".into(), client_secret: "s".into() }),
        None,
    );
    let state = WebState::new(cfg, Broker::new(), Arc::new(FailingTransport));
    let app = build_oauth_only(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/auth/gitlab/callback?code=c&state=s")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

//! darkrun-http — the HTTP + WebSocket review server.
//!
//! Bridges the darkrun engine (darkrun-mcp manager) and the desktop review
//! app. The manager registers interactive [`darkrun_api::SessionPayload`]s
//! into a shared [`SessionRegistry`]; the desktop app reads them over REST and
//! subscribes to live updates over a WebSocket. Feedback is read straight off
//! the `.darkrun/` filesystem state via [`darkrun_core::StateStore`].
//!
//! Built on `axum` + `tower`/`tower-http`. The middleware stack applies a
//! permissive CORS layer and a per-IP rate limit (60/min) in remote mode, plus
//! connection and WebSocket-session caps. The routes and transport posture
//! use the factory vocabulary throughout.
//!
//! Entry point: [`serve`] — what darkrun-cli calls to start the server.

mod feedback_doc;
mod handlers;
mod listen;
mod ratelimit;
mod runs;
mod state;
mod ws;

use std::net::SocketAddr;

use axum::{
    extract::FromRef,
    http::{HeaderValue, Method},
    routing::{get, head, post, put},
    Router,
};
use darkrun_core::StateStore;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

pub use ratelimit::RateLimiter;
pub use state::{
    AppState, Limits, Presence, ProofRegistry, SessionRegistry, DEFAULT_BODY_MAX_BYTES,
    DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_WS_SESSIONS, DEFAULT_RATE_LIMIT_PER_MIN, PRESENCE_GRACE_MS,
};

/// The composite router state: the domain [`AppState`] plus the rate-limiter.
///
/// Both are projected out via [`FromRef`] so individual handlers and middleware
/// can extract exactly what they need.
#[derive(Clone)]
pub struct RouterState {
    /// Domain state (sessions, store, limits).
    pub app: AppState,
    /// Per-IP rate-limit bookkeeping.
    pub limiter: RateLimiter,
}

impl FromRef<RouterState> for AppState {
    fn from_ref(s: &RouterState) -> Self {
        s.app.clone()
    }
}

impl FromRef<RouterState> for RateLimiter {
    fn from_ref(s: &RouterState) -> Self {
        s.limiter.clone()
    }
}

/// Build the fully-wired axum [`Router`] for the given application state.
///
/// Exposed (crate-public) so the in-process axum tests can exercise the routes
/// via `tower::ServiceExt::oneshot` without binding a socket.
pub fn build_router(app: AppState) -> Router {
    let remote = app.limits.remote;
    let state = RouterState {
        app,
        limiter: RateLimiter::new(),
    };

    // Permissive CORS only when the server is reachable beyond loopback.
    // Loopback-only deployments need no CORS headers (same-origin desktop app).
    let cors = if remote {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                Method::GET,
                Method::HEAD,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers(Any)
    } else {
        // Echo the request origin back for local same-origin use.
        CorsLayer::new()
            .allow_origin(HeaderValue::from_static("http://127.0.0.1"))
            .allow_methods(Any)
    };

    Router::new()
        .route("/health", get(handlers::health))
        .route("/api/runs", get(runs::list_runs))
        .route("/api/runs/{slug}", get(runs::get_run))
        .route("/api/runs/{slug}/asset/{*path}", get(handlers::get_run_asset))
        .route("/api/session/{id}", get(handlers::get_session))
        .route(
            "/api/session/{id}/heartbeat",
            head(handlers::session_heartbeat),
        )
        .route("/review/{id}/decide", post(handlers::review_decide))
        .route("/question/{id}/answer", post(handlers::question_answer))
        .route("/direction/{id}/select", post(handlers::direction_select))
        .route("/picker/{id}/select", post(handlers::picker_select))
        .route(
            "/visual-review/{id}/annotate",
            post(handlers::visual_review_annotate),
        )
        .route(
            "/api/proof/{run}",
            get(handlers::get_proof).post(handlers::attach_proof),
        )
        .route("/api/advance/{id}", post(handlers::advance))
        .route(
            "/api/unit/{run}/{unit}/reset",
            post(handlers::request_unit_reset),
        )
        .route(
            "/api/feedback/{run}/{station}",
            get(handlers::list_feedback).post(handlers::create_feedback),
        )
        .route(
            "/api/feedback/{run}/{station}/{id}",
            put(handlers::update_feedback).delete(handlers::delete_feedback),
        )
        .route(
            "/api/feedback/{run}/{station}/{id}/replies",
            post(handlers::create_feedback_reply),
        )
        .route("/ws/session/{id}", get(ws::ws_session))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ratelimit::rate_limit_middleware,
        ))
        // Reject oversize request bodies with `413` before a handler runs.
        .layer(RequestBodyLimitLayer::new(state::DEFAULT_BODY_MAX_BYTES))
        .layer(cors)
        .with_state(state)
}

/// Start the review server on `addr`, serving sessions + feedback backed by
/// `store`. Uses [`Limits::default`] (loopback / local mode). The returned
/// future resolves only when the server stops.
///
/// This is the simple fire-and-forget entry point darkrun-cli calls. Callers
/// that need to register/update sessions while the server runs should build an
/// [`AppState`] (cloning its [`SessionRegistry`] first) and use [`build_router`]
/// with [`serve_router`] — both are public for exactly that purpose.
#[cfg(not(tarpaulin_include))] // binds a socket + serves forever — irreducible I/O
pub async fn serve(addr: SocketAddr, store: StateStore) -> std::io::Result<()> {
    serve_with_limits(addr, store, Limits::default()).await
}

/// Like [`serve`], but with explicit [`Limits`].
#[cfg(not(tarpaulin_include))] // delegates into the serve loop — irreducible I/O
pub async fn serve_with_limits(
    addr: SocketAddr,
    store: StateStore,
    limits: Limits,
) -> std::io::Result<()> {
    let app_state = AppState::new(store, limits);
    serve_with_state(addr, app_state).await
}

/// Serve a pre-built [`AppState`] on `addr`.
///
/// The registry-sharing entry point: an embedder (the in-process `darkrun mcp`
/// host) builds ONE [`AppState`] — cloning its [`SessionRegistry`] /
/// [`ProofRegistry`] into its own MCP tool handlers first — and hands the same
/// state here. Because the registries are clonable shared handles, a session
/// the manager upserts is immediately visible to these HTTP/WS handlers without
/// any on-disk bridge.
#[cfg(not(tarpaulin_include))] // builds the router + serves forever — irreducible I/O
pub async fn serve_with_state(addr: SocketAddr, state: AppState) -> std::io::Result<()> {
    let limits = state.limits;
    let router = build_router(state);
    serve_router(addr, router, limits).await
}

/// Bind `addr` and serve a pre-built [`Router`] with the given [`Limits`].
///
/// The escape hatch for the registry-sharing path: build the router yourself
/// (after cloning the [`SessionRegistry`] out of your [`AppState`]) and hand it
/// here. Applies the same connection cap as [`serve`].
///
/// Pass port `0` (e.g. `127.0.0.1:0`) to bind an ephemeral port; embedders that
/// need the actual bound address back should instead [`bind_listener`] first and
/// then [`serve_router_on`], which lets them read the real port before serving.
#[cfg(not(tarpaulin_include))] // binds + serves forever — irreducible I/O
pub async fn serve_router(
    addr: SocketAddr,
    router: Router,
    limits: Limits,
) -> std::io::Result<()> {
    let listener = bind_listener(addr).await?;
    serve_router_on(listener, router, limits).await
}

/// Bind a loopback [`TcpListener`] on `addr`, returning it for the caller to
/// inspect (via [`TcpListener::local_addr`]) before serving.
///
/// Pass port `0` to request an ephemeral port; the kernel assigns a free port
/// the caller can read back and advertise. This is the seam the in-process
/// `darkrun mcp` host uses so it can write the discovery descriptor with the
/// REAL bound port before handing the listener to [`serve_router_on`].
pub async fn bind_listener(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(addr).await
}

/// Serve a pre-built [`Router`] on an already-bound `listener`.
///
/// The counterpart to [`bind_listener`]: callers that bound the socket
/// themselves (to read back an ephemeral port) hand the listener here to start
/// serving. Applies the same connection cap as [`serve`].
#[cfg(not(tarpaulin_include))] // the axum accept/serve loop — runs until aborted; irreducible I/O
pub async fn serve_router_on(
    listener: tokio::net::TcpListener,
    router: Router,
    limits: Limits,
) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    tracing::info!(
        %addr,
        max_connections = limits.max_connections,
        max_ws_sessions = limits.max_ws_sessions,
        "darkrun review server listening"
    );

    // Bound concurrent live connections to `max_connections`. The capped
    // listener hands out a permit per accepted socket and holds it for the
    // socket's lifetime, enforcing the configured max-connections cap. Peer
    // addresses are threaded in via `ConnectInfo` for the rate limiter + WS.
    let capped = listen::CappedListener::new(listener, limits.max_connections);
    axum::serve(
        capped,
        router.into_make_service_with_connect_info::<listen::PeerAddr>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod from_ref_tests {
    use super::*;
    use darkrun_core::StateStore;

    #[test]
    fn router_state_projects_both_substates() {
        // The axum extractors project AppState and RateLimiter out of the
        // composite RouterState via FromRef.
        let tmp = tempfile::tempdir().unwrap();
        let state = RouterState {
            app: AppState::new(StateStore::new(tmp.path()), Limits::default()),
            limiter: RateLimiter::new(),
        };
        let _app: AppState = AppState::from_ref(&state);
        let _limiter: RateLimiter = RateLimiter::from_ref(&state);
    }
}

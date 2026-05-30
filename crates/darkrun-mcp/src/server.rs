//! In-process MCP + HTTP/WS host.
//!
//! [`serve_stdio`] is the function `darkrun-cli` calls to run the manager as an
//! MCP server over stdin/stdout. Crucially it does NOT run alone: on the same
//! tokio runtime it ALSO spawns the axum HTTP/WS review server
//! ([`darkrun_http`]), bound to a loopback port. Both halves share ONE in-memory
//! [`darkrun_http::SessionRegistry`] (and [`darkrun_http::ProofRegistry`]) on a
//! single [`darkrun_http::AppState`], so an interactive session a tool handler
//! raises is immediately visible to the desktop app connected to that port —
//! with no on-disk `session.json` bridge.
//!
//! The bound port is announced to the agent in the MCP server `instructions`
//! string and on stderr, so the desktop app (which reads `DARKRUN_PORT`) knows
//! where to connect. The port is chosen from `DARKRUN_PORT` (or `--addr` passed
//! through by the CLI), defaulting to `127.0.0.1:4317`.

use std::net::SocketAddr;
use std::path::PathBuf;

use rmcp::transport::io::stdio;
use rmcp::ServiceExt;

use darkrun_core::StateStore;
use darkrun_http::{AppState, Limits};

use crate::tools::DarkrunServer;

/// The default loopback address the in-process HTTP/WS server binds.
pub const DEFAULT_ADDR: &str = "127.0.0.1:4317";

/// Resolve the HTTP/WS bind address: the `DARKRUN_PORT` env override (as a bare
/// port on loopback, or a full `host:port`) else [`DEFAULT_ADDR`].
fn resolve_addr() -> SocketAddr {
    let raw = std::env::var("DARKRUN_PORT").ok();
    if let Some(raw) = raw {
        let raw = raw.trim();
        // A bare port (e.g. "4400") binds loopback; a full "host:port" parses
        // directly.
        if let Ok(port) = raw.parse::<u16>() {
            return SocketAddr::from(([127, 0, 0, 1], port));
        }
        if let Ok(addr) = raw.parse::<SocketAddr>() {
            return addr;
        }
    }
    DEFAULT_ADDR.parse().expect("default addr is valid")
}

/// Serve the darkrun MCP server over stdio, rooted at `repo_root`, while also
/// hosting the HTTP/WS review server in-process on the resolved address
/// (`DARKRUN_PORT` or [`DEFAULT_ADDR`]).
///
/// Blocks until the MCP client disconnects. Durable state lives under
/// `<repo_root>/.darkrun`; interactive sessions live only in the shared
/// in-memory registry.
pub async fn serve_stdio(repo_root: impl Into<PathBuf>) -> std::io::Result<()> {
    serve_stdio_on(repo_root, resolve_addr()).await
}

/// Like [`serve_stdio`], but binds the HTTP/WS server to an explicit `addr`.
/// The MCP `instructions` announce the bound port to the agent.
pub async fn serve_stdio_on(
    repo_root: impl Into<PathBuf>,
    addr: SocketAddr,
) -> std::io::Result<()> {
    let repo_root = repo_root.into();

    // One shared AppState: the registries are clonable shared handles, so the
    // MCP tool handlers and the HTTP/WS handlers observe the same sessions and
    // proofs without any disk round-trip.
    let store = StateStore::new(&repo_root);
    let state = AppState::new(store, Limits::default());

    // Spawn the axum HTTP/WS server on the same runtime, sharing the state.
    let http_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = darkrun_http::serve_with_state(addr, http_state).await {
            eprintln!("darkrun: in-process HTTP server on {addr} stopped: {e}");
        }
    });

    // Announce the bound port so the desktop app (DARKRUN_PORT) can connect.
    eprintln!("darkrun: HTTP/WS review server listening on http://{addr}");

    let server = DarkrunServer::with_sessions(repo_root, state.sessions.clone())
        .with_announced_addr(addr);
    let running = server
        .serve(stdio())
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

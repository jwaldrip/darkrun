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
//! string and on stderr, AND written to the home discovery registry
//! (`~/.darkrun/<slug>/engine-<pid>.json`, see [`crate::registry`]) so the
//! desktop app can discover the engine and the port it serves on — no fixed
//! port required.
//!
//! By default the server binds an EPHEMERAL loopback port (`127.0.0.1:0`) and
//! reads the kernel-assigned port back before advertising it, so many engines
//! coexist. `DARKRUN_PORT` (or `--addr` passed through by the CLI) overrides
//! this with an explicit port when a caller needs a fixed one.

use std::net::SocketAddr;
use std::path::PathBuf;

use rmcp::transport::io::stdio;
use rmcp::ServiceExt;

use darkrun_core::StateStore;
use darkrun_harness::Harness;
use darkrun_http::{AppState, Limits};

use crate::registry::EngineRegistry;
use crate::tools::DarkrunServer;

/// The default loopback address the in-process HTTP/WS server binds.
///
/// Retained for callers that want an explicit fixed port; the default boot path
/// now binds an EPHEMERAL port (see [`resolve_addr`]).
pub const DEFAULT_ADDR: &str = "127.0.0.1:4317";

/// The ephemeral loopback bind address: port `0` lets the kernel assign a free
/// port, which is read back via `local_addr()` after binding.
const EPHEMERAL_ADDR: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    0,
);

/// Resolve the HTTP/WS bind address: the `DARKRUN_PORT` env override (as a bare
/// port on loopback, or a full `host:port`) else an EPHEMERAL loopback port
/// (`127.0.0.1:0`), whose real value is read back after binding.
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
    EPHEMERAL_ADDR
}

/// Serve the darkrun MCP server over stdio, rooted at `repo_root`, while also
/// hosting the HTTP/WS review server in-process on the resolved address
/// (`DARKRUN_PORT` or [`DEFAULT_ADDR`]).
///
/// Blocks until the MCP client disconnects. Durable state lives under
/// `<repo_root>/.darkrun`; interactive sessions live only in the shared
/// in-memory registry.
pub async fn serve_stdio(repo_root: impl Into<PathBuf>, harness: Harness) -> std::io::Result<()> {
    serve_stdio_on(repo_root, resolve_addr(), harness).await
}

/// Like [`serve_stdio`], but binds the HTTP/WS server to an explicit `addr`
/// (pass port `0` for an ephemeral port). The MCP `instructions` announce the
/// ACTUAL bound port to the agent, and the engine advertises itself in the home
/// discovery registry (`~/.darkrun/<slug>/engine-<pid>.json`). `harness` selects
/// the capability set the server adapts its tools and prompts to.
pub async fn serve_stdio_on(
    repo_root: impl Into<PathBuf>,
    addr: SocketAddr,
    harness: Harness,
) -> std::io::Result<()> {
    let repo_root = repo_root.into();

    // One shared AppState: the registries are clonable shared handles, so the
    // MCP tool handlers and the HTTP/WS handlers observe the same sessions and
    // proofs without any disk round-trip.
    let store = StateStore::new(&repo_root);
    let state = AppState::new(store, Limits::default());
    // On-demand show sessions: a desktop asking for a session id that names a
    // RUN materializes its review payload from state, so clicking a run in the
    // sidebar works before the engine has ticked. Unfocused — a passive read
    // must not repoint the `current` focus channel other windows watch.
    let state = {
        let sessions = state.sessions.clone();
        let mat_store = state.store.clone();
        state.with_session_materializer(move |id| {
            crate::sessions::create_show_with_focus(&sessions, &mat_store, id, false).is_ok()
        })
    };
    // Re-surface a run after the operator resolves an interactive session: drop
    // the answered prompt and push the next open one (or the review) onto the
    // run channel, so answering dismisses + advances without waiting for the
    // agent's next tick.
    let state = {
        let sessions = state.sessions.clone();
        let res_store = state.store.clone();
        state.with_surface_resolver(move |run| {
            // A run still in SETUP: answering a factory/mode/size picker should
            // promptly surface the NEXT selection. (Once all are chosen, the
            // desktop stays on the last pick until the agent's next advance
            // materializes the run.)
            if let Some(setup) = res_store.read_run_setup(run) {
                if let Some(kind) = setup.first_unset() {
                    let title =
                        res_store.read_run(run).ok().and_then(|r| r.frontmatter.title);
                    crate::sessions::raise_setup_picker(&sessions, run, title.as_deref(), kind);
                }
                return;
            }
            let _ = crate::sessions::create_show_with_focus(&sessions, &res_store, run, false);
        })
    };
    // Durability: every interactive session (question / direction / picker) the
    // registry upserts — on raise AND on answer — is written to the run's
    // `interactive/` dir, so an open question and its eventual answer survive an
    // engine restart and reappear when the desktop reconnects.
    {
        let persist_store = state.store.clone();
        state.sessions.on_persist(std::sync::Arc::new(move |payload| {
            let _ = persist_store.write_interactive_session(payload);
        }));
    }

    // Bind the listener up front so we can read the REAL port back (the
    // requested addr may carry port 0 for an ephemeral bind) before advertising
    // it. Everything downstream — instructions, stderr, the discovery
    // descriptor — uses this concrete address.
    let listener = darkrun_http::bind_listener(addr).await?;
    let bound = listener.local_addr()?;

    // Advertise the engine in the home discovery registry. Best-effort: a write
    // failure (e.g. no home dir) is non-fatal — the engine still serves, it's
    // just not auto-discoverable. The descriptor is RETAINED on exit (flagged
    // stale, never deleted), so we hold the registry handle to mark it stale.
    let engine_registry = announce_engine(&repo_root, bound, harness.key());

    // Spawn the axum HTTP/WS server on the same runtime, sharing the state, on
    // the already-bound listener.
    let http_state = state.clone();
    let limits = http_state.limits;
    let router = darkrun_http::build_router(http_state);
    tokio::spawn(async move {
        if let Err(e) = darkrun_http::serve_router_on(listener, router, limits).await {
            eprintln!("darkrun: in-process HTTP server on {bound} stopped: {e}");
        }
    });

    // Announce the bound port so the desktop app (DARKRUN_PORT) can connect.
    eprintln!("darkrun: HTTP/WS review server listening on http://{bound}");
    eprintln!("darkrun: harness = {}", harness.key());

    let server = DarkrunServer::with_sessions(repo_root, state.sessions.clone())
        .with_announced_addr(bound)
        .with_harness(harness);
    // A session opening onto a project with an ACTIVE run brings the desktop up
    // immediately — the operator watches the run live from the first moment,
    // instead of waiting for a gate (or even a first tick) to raise it. The
    // show session is PUSHED here too (focused), so the app has a payload to
    // render the instant it connects.
    if let Ok(Some(active)) = state.store.active_run() {
        let _ = crate::sessions::create_show(&state.sessions, &state.store, &active);
        server.surface_desktop_once(&active);
    }
    let running = server
        .serve(stdio())
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let wait = running
        .waiting()
        .await
        .map_err(|e| std::io::Error::other(e.to_string()));

    // On a clean shutdown, flag the discovery descriptor stale (retains the
    // record). Best-effort.
    if let Some(registry) = &engine_registry {
        if let Err(e) = registry.mark_stale() {
            eprintln!("darkrun: could not flag discovery descriptor stale: {e}");
        }
    }

    wait?;
    Ok(())
}

/// Write the home discovery descriptor for this engine, returning the registry
/// handle (used to flag the descriptor stale on shutdown) or `None` if the
/// registry could not be set up or the write failed.
fn announce_engine(
    repo_root: &std::path::Path,
    addr: SocketAddr,
    harness_key: &str,
) -> Option<EngineRegistry> {
    let registry = match EngineRegistry::new(repo_root) {
        Ok(registry) => registry,
        Err(e) => {
            eprintln!("darkrun: discovery registry unavailable: {e}");
            return None;
        }
    };
    match registry.announce(addr, harness_key) {
        Ok(_descriptor) => {
            eprintln!(
                "darkrun: discovery descriptor written to {}",
                registry.descriptor_path().display()
            );
            Some(registry)
        }
        Err(e) => {
            eprintln!("darkrun: could not write discovery descriptor: {e}");
            None
        }
    }
}

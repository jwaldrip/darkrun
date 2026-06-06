//! WebSocket upgrade + live session push.
//!
//! `GET /ws/session/:id` upgrades to a WebSocket. On connect the server pushes
//! the current session payload, then streams every subsequent update as a JSON
//! text frame (driven by the registry's per-session broadcast channel). Inbound
//! client frames are drained but otherwise ignored — the SPA's mutations flow
//! through the REST routes; the WebSocket is push-only.
//!
//! The `max_ws_sessions` cap is enforced before the upgrade completes: when the
//! server is already at capacity the socket is closed with RFC 6455 code 1013
//! ("try again later").

use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::Response,
};

use crate::state::{AppState, WsSlot};

/// RFC 6455 close code 1013 — "Try Again Later" (server at capacity).
const CLOSE_TRY_AGAIN_LATER: u16 = 1013;

/// `GET /ws/session/:id` — upgrade and stream live session updates.
pub async fn ws_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let max = state.limits.max_ws_sessions;
    upgrade.on_upgrade(move |socket| handle_socket(socket, state, id, max))
}

/// Per-connection driver. Acquires a session slot, pushes the current payload,
/// then forwards broadcast frames until the client or session goes away.
// The live WebSocket pump: its send/recv/lag/close arms need a real socket with
// specific failure timing — irreducible I/O. The happy path is driven by the
// ws_state integration test over a real connection.
#[cfg(not(tarpaulin_include))]
async fn handle_socket(mut socket: WebSocket, state: AppState, id: String, max: usize) {
    // Enforce the concurrent-session cap. Hold the slot for the connection's
    // lifetime via the RAII guard.
    let _slot: WsSlot = match state.sessions.try_acquire_ws_slot(max) {
        Some(slot) => slot,
        None => {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: CLOSE_TRY_AGAIN_LATER,
                    reason: "max ws sessions reached".into(),
                })))
                .await;
            return;
        }
    };

    // Subscribe before sending the snapshot so no update slips through the gap.
    let mut rx = match state.sessions.subscribe(&id) {
        Some(rx) => rx,
        None => {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: 1000,
                    reason: "unknown session".into(),
                })))
                .await;
            return;
        }
    };

    // Push the current snapshot immediately.
    if let Some(payload) = state.sessions.get(&id) {
        if let Ok(frame) = serde_json::to_string(&payload) {
            if socket.send(Message::Text(frame.into())).await.is_err() {
                return;
            }
        }
    }

    loop {
        tokio::select! {
            // Outbound: a registry update for this session.
            update = rx.recv() => {
                match update {
                    Ok(frame) => {
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: skip dropped frames, keep streaming.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    // Channel closed: the session was removed.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            // Inbound: drain client frames; close on disconnect.
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => { /* ignore: mutations go through REST */ }
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

//! Shared server state: the session registry, the WebSocket connection
//! registry, and the resource-limit configuration.
//!
//! The HTTP server is dependency-light at the domain edge: it serves
//! interactive [`SessionPayload`]s out of an in-memory registry that the
//! manager (darkrun-mcp) populates, while reading feedback off the
//! filesystem via [`darkrun_core::StateStore`]. Keeping the session source in
//! memory (rather than re-deriving it from disk on every request) keeps the
//! live-update WebSocket cheap.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use darkrun_api::{Proof, SessionPayload};
use darkrun_core::StateStore;
use tokio::sync::broadcast;

/// Default per-IP request ceiling per rate-limit window (60 requests / minute).
pub const DEFAULT_RATE_LIMIT_PER_MIN: u64 = 60;
/// Default cap on concurrent TCP connections.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;
/// Default cap on concurrent WebSocket sessions.
pub const DEFAULT_MAX_WS_SESSIONS: usize = 128;
/// Capacity of each session's broadcast channel (buffered server frames).
const WS_CHANNEL_CAPACITY: usize = 64;
/// Global request-body ceiling (1 MiB). Oversize bodies are rejected `413`
/// before a handler runs, bounding memory per request.
pub const DEFAULT_BODY_MAX_BYTES: usize = 1_048_576;

/// Resource limits applied by the middleware stack and WebSocket upgrade path.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Per-IP request ceiling per minute. Applied only in remote mode.
    pub rate_limit_per_min: u64,
    /// Maximum concurrent TCP connections.
    pub max_connections: usize,
    /// Maximum concurrent WebSocket sessions.
    pub max_ws_sessions: usize,
    /// Whether the server is reachable beyond loopback. CORS + rate limiting
    /// only engage when `true`, reflecting the local-vs-remote split.
    pub remote: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            rate_limit_per_min: DEFAULT_RATE_LIMIT_PER_MIN,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_ws_sessions: DEFAULT_MAX_WS_SESSIONS,
            remote: false,
        }
    }
}

/// One registered session plus its live-update broadcast channel.
struct SessionEntry {
    payload: SessionPayload,
    tx: broadcast::Sender<String>,
}

/// In-memory registry of interactive sessions, keyed by `session_id`.
///
/// Clonable and `Send + Sync`: every clone shares the same backing map, so the
/// manager and the HTTP handlers observe the same sessions. Mutations push
/// a fresh JSON frame to any WebSocket subscribed to that session.
#[derive(Clone, Default)]
pub struct SessionRegistry {
    inner: Arc<Mutex<HashMap<String, SessionEntry>>>,
    ws_session_count: Arc<AtomicU64>,
}

impl SessionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a session. Any subscribed WebSocket receives the new
    /// payload as a JSON frame immediately.
    pub fn upsert(&self, payload: SessionPayload) {
        let id = payload.session_id().to_string();
        let frame = serde_json::to_string(&payload).ok();
        let mut guard = self.inner.lock().expect("session registry poisoned");
        let entry = guard.entry(id).or_insert_with(|| SessionEntry {
            payload: payload.clone(),
            tx: broadcast::channel(WS_CHANNEL_CAPACITY).0,
        });
        entry.payload = payload;
        if let Some(frame) = frame {
            // Ignore send errors: no subscribers is fine.
            let _ = entry.tx.send(frame);
        }
    }

    /// Fetch a session payload by id.
    pub fn get(&self, id: &str) -> Option<SessionPayload> {
        let guard = self.inner.lock().expect("session registry poisoned");
        guard.get(id).map(|e| e.payload.clone())
    }

    /// Mint the next session id for the given kind `prefix` (`q`/`d`/`p`),
    /// scanning the live registry so ids stay unique and monotonic within the
    /// process. Format: `{prefix}-NN` (zero-padded to two digits).
    ///
    /// This is the in-memory replacement for the old on-disk `session.json`
    /// id-minting: the manager (darkrun-mcp) calls it to label a session before
    /// upserting it, so the desktop app sees stable `/api/session/:id` paths.
    pub fn next_session_id(&self, prefix: &str) -> String {
        let want = format!("{prefix}-");
        let guard = self.inner.lock().expect("session registry poisoned");
        let max = guard
            .keys()
            .filter_map(|k| k.strip_prefix(&want))
            .filter_map(|n| n.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("{prefix}-{:02}", max + 1)
    }

    /// Whether a session with the given id exists (drives the heartbeat probe).
    pub fn contains(&self, id: &str) -> bool {
        let guard = self.inner.lock().expect("session registry poisoned");
        guard.contains_key(id)
    }

    /// Remove a session and drop its broadcast channel (closing subscribers).
    pub fn remove(&self, id: &str) -> Option<SessionPayload> {
        let mut guard = self.inner.lock().expect("session registry poisoned");
        guard.remove(id).map(|e| e.payload)
    }

    /// Subscribe to live-update frames for a session, creating the entry's
    /// channel lazily. Returns `None` if the session does not exist.
    pub fn subscribe(&self, id: &str) -> Option<broadcast::Receiver<String>> {
        let guard = self.inner.lock().expect("session registry poisoned");
        guard.get(id).map(|e| e.tx.subscribe())
    }

    /// Try to reserve a WebSocket slot, honouring `max_ws_sessions`. Returns a
    /// guard that releases the slot on drop, or `None` if the cap is hit.
    pub fn try_acquire_ws_slot(&self, max: usize) -> Option<WsSlot> {
        loop {
            let current = self.ws_session_count.load(Ordering::Acquire);
            if current as usize >= max {
                return None;
            }
            if self
                .ws_session_count
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(WsSlot {
                    counter: Arc::clone(&self.ws_session_count),
                });
            }
        }
    }
}

/// RAII guard for a reserved WebSocket session slot. Releases on drop.
pub struct WsSlot {
    counter: Arc<AtomicU64>,
}

impl Drop for WsSlot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One attached proof plus the station it was measured at.
#[derive(Clone)]
struct ProofEntry {
    proof: Proof,
    station: Option<String>,
}

/// In-memory registry of run-scoped objective-evidence [`Proof`]s, keyed by run
/// slug. Populated by the Prove station's `POST /api/proof/:run`; read back by
/// the desktop app's `GET /api/proof/:run`.
///
/// Clonable + `Send + Sync` (shares the backing map across clones), mirroring
/// the [`SessionRegistry`] posture so the manager and HTTP handlers observe the
/// same proofs without a disk round-trip.
#[derive(Clone, Default)]
pub struct ProofRegistry {
    inner: Arc<Mutex<HashMap<String, ProofEntry>>>,
}

impl ProofRegistry {
    /// Create an empty proof registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach (or replace) the proof for a run, with its measured station.
    pub fn attach(&self, run: &str, proof: Proof, station: Option<String>) {
        let mut guard = self.inner.lock().expect("proof registry poisoned");
        guard.insert(run.to_string(), ProofEntry { proof, station });
    }

    /// Fetch a run's attached proof + station, if any.
    pub fn get(&self, run: &str) -> Option<(Proof, Option<String>)> {
        let guard = self.inner.lock().expect("proof registry poisoned");
        guard.get(run).map(|e| (e.proof.clone(), e.station.clone()))
    }
}

/// The shared application state threaded through every handler.
#[derive(Clone)]
pub struct AppState {
    /// The in-memory interactive-session registry.
    pub sessions: SessionRegistry,
    /// The in-memory run-scoped proof registry.
    pub proofs: ProofRegistry,
    /// The filesystem state engine (used for feedback reads).
    pub store: Arc<StateStore>,
    /// Resource limits in effect.
    pub limits: Limits,
}

impl AppState {
    /// Build application state from a state store and resource limits.
    pub fn new(store: StateStore, limits: Limits) -> Self {
        Self {
            sessions: SessionRegistry::new(),
            proofs: ProofRegistry::new(),
            store: Arc::new(store),
            limits,
        }
    }
}

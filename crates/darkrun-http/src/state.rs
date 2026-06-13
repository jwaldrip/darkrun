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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
/// How long after the desktop app's last connection drops it is still treated as
/// merely *lost* (a backgrounded tab, a network blip) rather than *closed* — the
/// presence grace window. Within it, the engine should not relaunch the app.
pub const PRESENCE_GRACE_MS: u64 = 15_000;

/// The desktop app's connection presence, with a grace window so a momentary
/// disconnect doesn't read as "closed" (F5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// At least one client is connected right now.
    Live,
    /// Was connected, dropped within the last [`PRESENCE_GRACE_MS`] — likely a
    /// blip or a backgrounded tab; may reattach. Don't relaunch yet.
    Lost,
    /// Was connected and has been gone past the grace window — the app is closed.
    Closed,
    /// No client has ever connected this process — the app hasn't opened yet.
    NeverAttached,
}

impl Presence {
    /// Whether the engine should consider the app present (live or in grace) —
    /// the relaunch decision uses this so a brief drop doesn't respawn the app.
    pub fn is_present(self) -> bool {
        matches!(self, Presence::Live | Presence::Lost)
    }
}

/// Process-wide desktop-presence tracker: records connection transitions with
/// timestamps so [`SessionRegistry::presence`] can apply the grace window.
#[derive(Default)]
struct PresenceTracker {
    ever_connected: AtomicBool,
    /// Epoch-ms when the connection count last fell to zero. `0` = never lost.
    last_lost_ms: AtomicU64,
}

/// Current epoch milliseconds.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
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
    presence: Arc<PresenceTracker>,
    /// Optional durability hook the engine installs: invoked on every upsert
    /// (raise AND answer) with the payload, so interactive sessions persist to
    /// disk without the HTTP answer handlers needing to know how. Shared across
    /// clones (the engine installs it once, after construction).
    persist: Arc<Mutex<Option<PersistHook>>>,
}

/// A durability callback: persist a session payload (e.g. to the run's
/// `interactive/` dir). See [`SessionRegistry::on_persist`].
pub type PersistHook = Arc<dyn Fn(&SessionPayload) + Send + Sync>;

impl SessionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Install the durability hook (see [`PersistHook`]). Shared across every
    /// clone of this registry, so handlers that hold a clone persist too.
    pub fn on_persist(&self, hook: PersistHook) {
        *self.persist.lock().expect("session registry poisoned") = Some(hook);
    }

    /// Run the persist hook for `payload`, if one is installed.
    fn persist(&self, payload: &SessionPayload) {
        let hook = self
            .persist
            .lock()
            .expect("session registry poisoned")
            .clone();
        if let Some(hook) = hook {
            hook(payload);
        }
    }

    /// Insert or replace a session. Any subscribed WebSocket receives the new
    /// payload as a JSON frame immediately.
    pub fn upsert(&self, payload: SessionPayload) {
        let id = payload.session_id().to_string();
        self.upsert_under(&id, payload);
    }

    /// Insert or replace a session under an EXPLICIT id, regardless of the
    /// payload's own `session_id`. The mirror mechanism: a question raised under
    /// `q-NN` is also written under the run slug so a desktop subscribed to the
    /// run's channel renders it live — while the payload still names `q-NN`, so
    /// the operator's answer routes back to the canonical session. The persist
    /// hook fires once, for the payload (not per-mirror), so disk isn't written
    /// twice for one logical session.
    pub fn upsert_under(&self, id: &str, payload: SessionPayload) {
        let frame = serde_json::to_string(&payload).ok();
        // Persist only when storing under the payload's own id (the canonical
        // write) — mirrors are view-only and share the same on-disk record.
        if id == payload.session_id() {
            self.persist(&payload);
        }
        let mut guard = self.inner.lock().expect("session registry poisoned");
        let entry = guard.entry(id.to_string()).or_insert_with(|| SessionEntry {
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

    /// The number of live WebSocket subscribers across all sessions — a proxy
    /// for "is the desktop app connected". `0` means nothing is listening, so the
    /// engine should launch the desktop app.
    pub fn live_connections(&self) -> u64 {
        self.ws_session_count.load(Ordering::Acquire)
    }

    /// The desktop app's connection presence, with a grace window so a momentary
    /// disconnect doesn't read as "closed" (F5). The relaunch decision should use
    /// `presence().is_present()` rather than `live_connections() > 0`, so a
    /// backgrounded tab or a network blip doesn't respawn the app.
    pub fn presence(&self) -> Presence {
        self.presence_at(now_ms())
    }

    /// [`presence`](Self::presence) evaluated at an explicit `now` (epoch ms) —
    /// the clock seam so the grace window is testable without sleeping.
    fn presence_at(&self, now: u64) -> Presence {
        if self.ws_session_count.load(Ordering::Acquire) > 0 {
            return Presence::Live;
        }
        if !self.presence.ever_connected.load(Ordering::Acquire) {
            return Presence::NeverAttached;
        }
        let lost = self.presence.last_lost_ms.load(Ordering::Acquire);
        if lost != 0 && now.saturating_sub(lost) < PRESENCE_GRACE_MS {
            Presence::Lost
        } else {
            Presence::Closed
        }
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
                // The app is (re)attached — mark it ever-connected so a later
                // drop is a *loss*, not "never opened" (F5).
                self.presence.ever_connected.store(true, Ordering::Release);
                return Some(WsSlot {
                    counter: Arc::clone(&self.ws_session_count),
                    presence: Arc::clone(&self.presence),
                });
            }
        }
    }
}

/// RAII guard for a reserved WebSocket session slot. Releases on drop.
pub struct WsSlot {
    counter: Arc<AtomicU64>,
    presence: Arc<PresenceTracker>,
}

impl Drop for WsSlot {
    fn drop(&mut self) {
        // `fetch_sub` returns the PRIOR value; if it was 1 the count just fell to
        // zero — stamp the loss time so the grace window starts (F5).
        if self.counter.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.presence.last_lost_ms.store(now_ms(), Ordering::Release);
        }
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
    /// Optional on-demand session builder the embedding engine installs: given
    /// a session id that MISSES the registry, build it when it names something
    /// real (e.g. a run slug → its show session). Lets a desktop open a run's
    /// review without waiting for the engine to tick first.
    pub materialize_session: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
}

impl AppState {
    /// Build application state from a state store and resource limits.
    pub fn new(store: StateStore, limits: Limits) -> Self {
        Self {
            sessions: SessionRegistry::new(),
            proofs: ProofRegistry::new(),
            store: Arc::new(store),
            limits,
            materialize_session: None,
        }
    }

    /// Install the on-demand session builder (see `materialize_session`).
    pub fn with_session_materializer(
        mut self,
        f: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.materialize_session = Some(Arc::new(f));
        self
    }

    /// Ensure `id` exists in the session registry, building it on demand via
    /// the installed materializer when absent. Returns whether it now exists.
    pub fn ensure_session(&self, id: &str) -> bool {
        if self.sessions.contains(id) {
            return true;
        }
        match &self.materialize_session {
            Some(build) => build(id) && self.sessions.contains(id),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_starts_never_attached() {
        let reg = SessionRegistry::new();
        assert_eq!(reg.presence(), Presence::NeverAttached);
        assert!(!reg.presence().is_present());
    }

    #[test]
    fn presence_is_live_while_a_slot_is_held() {
        let reg = SessionRegistry::new();
        let slot = reg.try_acquire_ws_slot(8).expect("slot");
        assert_eq!(reg.presence(), Presence::Live);
        assert!(reg.presence().is_present());
        drop(slot);
    }

    #[test]
    fn presence_is_lost_within_grace_then_closed_after() {
        let reg = SessionRegistry::new();
        // Connect then drop → the loss time is stamped (~now).
        let slot = reg.try_acquire_ws_slot(8).expect("slot");
        drop(slot);
        let lost_at = reg.presence.last_lost_ms.load(Ordering::Acquire);
        assert!(lost_at > 0, "drop stamps the loss time");

        // Just after the drop: still LOST (within grace) and counts as present.
        let just_after = lost_at + 1;
        assert_eq!(reg.presence_at(just_after), Presence::Lost);
        assert!(reg.presence_at(just_after).is_present());

        // Past the grace window: CLOSED — the app is gone.
        let past_grace = lost_at + PRESENCE_GRACE_MS + 1;
        assert_eq!(reg.presence_at(past_grace), Presence::Closed);
        assert!(!reg.presence_at(past_grace).is_present());
    }

    #[test]
    fn reattaching_returns_to_live() {
        let reg = SessionRegistry::new();
        drop(reg.try_acquire_ws_slot(8).expect("slot")); // connect then lose
        let _slot = reg.try_acquire_ws_slot(8).expect("slot"); // reattach
        assert_eq!(reg.presence(), Presence::Live);
    }
}

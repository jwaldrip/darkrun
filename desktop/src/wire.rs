//! The live wire to the local engine.
//!
//! Two responsibilities:
//!   1. Open `ws://127.0.0.1:PORT/ws/session/:id` and stream every
//!      [`SessionPayload`] frame the server pushes.
//!   2. POST review decisions to `http://127.0.0.1:PORT/review/:id/decide`.
//!
//! The engine speaks plain localhost HTTP/1.1, so the decision POST is a small
//! hand-rolled request over a `tokio` TCP stream rather than a full HTTP client
//! dependency. The WebSocket uses `tokio-tungstenite`.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use darkrun_api::{
    DirectionSelectRequest, FeedbackCreateRequest, FeedbackListResponse, FeedbackUpdateRequest,
    OutputReviewRequest, PickerSelectRequest, QuestionAnswerRequest, ReviewDecisionRequest,
    RunDetailPayload, RunListPayload, SessionPayload,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

/// Default engine port when `DARKRUN_PORT` is unset.
const DEFAULT_PORT: u16 = 7878;
/// Default session id when `DARKRUN_SESSION_ID` is unset.
const DEFAULT_SESSION: &str = "current";

/// Where to find the engine and which session to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnConfig {
    /// Engine host (always loopback in practice).
    pub host: String,
    /// Engine port.
    pub port: u16,
    /// The session id to subscribe to.
    pub session_id: String,
}

impl ConnConfig {
    /// Read the connection config from the environment, falling back to the
    /// loopback defaults so the app still renders a useful "connecting" shell
    /// when launched standalone.
    pub fn from_env() -> Self {
        let port = std::env::var("DARKRUN_PORT")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let session_id = std::env::var("DARKRUN_SESSION_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION.to_string());
        ConnConfig {
            host: "127.0.0.1".to_string(),
            port,
            session_id,
        }
    }

    /// Read the launch config from the environment, reporting whether a session
    /// was *explicitly* pinned via `DARKRUN_SESSION_ID`.
    ///
    /// The desktop app uses `pinned` to decide its opening surface: a pinned
    /// session jumps straight to the live Review (the engine launched us pointed
    /// at a run); an unpinned launch opens the run-browser home screen. The
    /// returned [`ConnConfig`] still carries the [`DEFAULT_SESSION`] id so the
    /// home screen has a base authority/port to fetch the run list from.
    pub fn from_env_pinned() -> (Self, bool) {
        let pinned = std::env::var("DARKRUN_SESSION_ID")
            .ok()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        (Self::from_env(), pinned)
    }

    /// The `host:port` authority.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Rewrite a `file://` image URL pointing inside a run's
    /// `.darkrun/<slug>/assets/` directory into this engine's HTTP asset route,
    /// so the webview (served over a custom protocol, which cannot load
    /// `file://`) can fetch it. Non-`file://` URLs (already `http(s)`/relative)
    /// and paths outside the run's assets dir pass through unchanged.
    pub fn asset_url(&self, run_slug: &str, url: &str) -> String {
        let Some(rest) = url.strip_prefix("file://") else {
            return url.to_string();
        };
        let needle = format!("/.darkrun/{run_slug}/assets/");
        match rest.find(&needle) {
            Some(idx) => {
                let rel = &rest[idx + needle.len()..];
                format!("http://{}/api/runs/{run_slug}/asset/{rel}", self.authority())
            }
            None => url.to_string(),
        }
    }

    /// The WebSocket URL for the session feed.
    pub fn ws_url(&self) -> String {
        format!(
            "ws://{}/ws/session/{}",
            self.authority(),
            self.session_id
        )
    }

    /// The decision POST path (`/review/:id/decide`).
    pub fn decide_path(&self) -> String {
        format!("/review/{}/decide", self.session_id)
    }

    /// The question-answer POST path (`/question/:id/answer`).
    pub fn question_answer_path(&self) -> String {
        format!("/question/{}/answer", self.session_id)
    }

    /// The direction-select POST path (`/direction/:id/select`).
    pub fn direction_select_path(&self) -> String {
        format!("/direction/{}/select", self.session_id)
    }

    /// The picker-select POST path (`/picker/:id/select`).
    pub fn picker_select_path(&self) -> String {
        format!("/picker/{}/select", self.session_id)
    }

    /// The output-annotation POST path (`/visual-review/:id/annotate`) — submits
    /// the pins + comments the operator dropped on an output screenshot.
    pub fn visual_review_annotate_path(&self) -> String {
        format!("/visual-review/{}/annotate", self.session_id)
    }

    /// The proof GET path (`/api/proof/:run`) — reads a run's attached
    /// objective-evidence proof.
    pub fn proof_path(&self, run: &str) -> String {
        format!("/api/proof/{run}")
    }

    /// The feedback-list GET path (`/api/feedback/:run/:station`) — every
    /// feedback item on a station, the data the feedback inbox renders and the
    /// checkpoint counts by severity. (Annotations submitted from the review
    /// surface land here as `user-visual` feedback server-side.)
    pub fn feedback_list_path(&self, run: &str, station: &str) -> String {
        format!("/api/feedback/{run}/{station}")
    }

    /// The feedback-item PUT path (`/api/feedback/:run/:station/:id`) — used by
    /// the inbox's resolve/dismiss chips to transition an item's status.
    pub fn feedback_item_path(&self, run: &str, station: &str, id: &str) -> String {
        format!("/api/feedback/{run}/{station}/{id}")
    }

    /// The run-list GET path (`/api/runs`) — every non-archived run summary.
    pub fn runs_path(&self) -> String {
        "/api/runs".to_string()
    }

    /// The run-detail GET path (`/api/runs/:slug`).
    pub fn run_detail_path(&self, slug: &str) -> String {
        format!("/api/runs/{slug}")
    }

    /// The unit-reset POST path (`/api/unit/:run/:unit/reset`) — the review UI's
    /// "reset this unit" action. Flags a wedged Unit so the engine returns it to
    /// pending (body editable, re-runs from Pass 1) on its next tick.
    pub fn unit_reset_path(&self, run: &str, unit: &str) -> String {
        format!("/api/unit/{run}/{unit}/reset")
    }

    /// Clone this config pointed at a different session id — used when the home
    /// browser opens a specific run's live Review.
    pub fn with_session(&self, session_id: impl Into<String>) -> Self {
        ConnConfig {
            host: self.host.clone(),
            port: self.port,
            session_id: session_id.into(),
        }
    }
}

/// A frame yielded by the live session feed.
#[derive(Debug, Clone)]
pub enum FeedEvent {
    /// A successfully decoded session payload.
    Payload(Box<SessionPayload>),
    /// The connection went down (with a human-readable reason). The UI shows a
    /// reconnect banner; the caller drives any retry.
    Disconnected(String),
}

/// Errors raised while talking to the engine over plain HTTP.
#[derive(Debug)]
pub enum WireError {
    /// The TCP connection failed.
    Connect(std::io::Error),
    /// Reading/writing the socket failed.
    Io(std::io::Error),
    /// The request body could not be serialized.
    Encode(serde_json::Error),
    /// The server returned a non-2xx status.
    Status(u16),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Connect(e) => write!(f, "connect failed: {e}"),
            WireError::Io(e) => write!(f, "io error: {e}"),
            WireError::Encode(e) => write!(f, "encode failed: {e}"),
            WireError::Status(code) => write!(f, "engine returned HTTP {code}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Open the session WebSocket and forward each decoded frame to `on_event`.
///
/// Returns when the socket closes; the caller decides whether to reconnect.
/// Malformed frames are skipped rather than tearing down the stream — the
/// server occasionally interleaves non-session control text.
// The live WebSocket session feed: opens a real socket and loops over server
// frames until close — irreducible I/O (the connect-failure path is tested
// against a dead engine; the message loop needs a live WS server + frame types
// that can't be driven in-process).
#[cfg(not(tarpaulin_include))]
pub async fn run_session_feed<F>(cfg: &ConnConfig, mut on_event: F)
where
    F: FnMut(FeedEvent),
{
    let url = cfg.ws_url();
    let (stream, _resp) = match tokio_tungstenite::connect_async(&url).await {
        Ok(pair) => pair,
        Err(e) => {
            on_event(FeedEvent::Disconnected(format!("connect failed: {e}")));
            return;
        }
    };

    let (_write, mut read) = stream.split();
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                match serde_json::from_str::<SessionPayload>(&text) {
                    Ok(payload) => on_event(FeedEvent::Payload(Box::new(payload))),
                    // Not a session frame (or a schema we don't model) — skip it.
                    Err(_) => continue,
                }
            }
            Ok(Message::Binary(bytes)) => {
                if let Ok(payload) = serde_json::from_slice::<SessionPayload>(&bytes) {
                    on_event(FeedEvent::Payload(Box::new(payload)));
                }
            }
            Ok(Message::Close(frame)) => {
                let reason = frame
                    .map(|f| format!("closed: {} {}", u16::from(f.code), f.reason))
                    .unwrap_or_else(|| "closed by server".to_string());
                on_event(FeedEvent::Disconnected(reason));
                return;
            }
            Ok(_) => { /* ping/pong/frame: ignore, tungstenite auto-pongs */ }
            Err(e) => {
                on_event(FeedEvent::Disconnected(format!("stream error: {e}")));
                return;
            }
        }
    }
    on_event(FeedEvent::Disconnected("stream ended".to_string()));
}

/// POST a review decision to `/review/:id/decide`.
///
/// Hand-rolled HTTP/1.1 over loopback TCP: write the request, read the status
/// line, and surface a typed result. The engine canonicalizes the raw decision
/// string server-side, so `decision` is sent verbatim (`"approved"` or
/// `"changes_requested"`).
pub async fn submit_decision(
    cfg: &ConnConfig,
    req: &ReviewDecisionRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.decide_path(), req).await
}

/// POST the answer to a visual-question session (`/question/:id/answer`).
pub async fn submit_question_answer(
    cfg: &ConnConfig,
    req: &QuestionAnswerRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.question_answer_path(), req).await
}

/// POST the design-direction decision (`/direction/:id/select`).
pub async fn submit_direction_select(
    cfg: &ConnConfig,
    req: &DirectionSelectRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.direction_select_path(), req).await
}

/// POST the picker selection (`/picker/:id/select`).
pub async fn submit_picker_select(
    cfg: &ConnConfig,
    req: &PickerSelectRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.picker_select_path(), req).await
}

/// POST the output-screenshot annotations (`/visual-review/:id/annotate`) — the
/// pins + comments the operator dropped become a piece of feedback server-side.
pub async fn submit_output_review(
    cfg: &ConnConfig,
    req: &OutputReviewRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.visual_review_annotate_path(), req).await
}

/// POST a unit-reset request (`/api/unit/:run/:unit/reset`) — the review UI's
/// rescue for a wedged Unit. The body is empty; the engine flags the Unit and
/// resets it to pending on its next tick (unlocking its body, clearing the Pass
/// budget) while preserving the spec.
pub async fn submit_unit_reset(cfg: &ConnConfig, run: &str, unit: &str) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.unit_reset_path(run, unit), &serde_json::json!({})).await
}

/// GET the project's run list (`/api/runs`) and decode it into a
/// [`RunListPayload`]. The desktop home screen calls this on launch when no
/// session id is pinned, then renders the run browser.
pub async fn fetch_runs(cfg: &ConnConfig) -> Result<RunListPayload, WireError> {
    get_json(&cfg.authority(), &cfg.runs_path()).await
}

/// GET a station's feedback list (`/api/feedback/:run/:station`) — the data the
/// feedback inbox renders and the checkpoint reads to count open annotations by
/// severity. The annotation model surfaces every artifact annotation here as a
/// feedback item, so this is the desktop's read path for
/// `list_annotations_for_work_item`-style counts over a station.
pub async fn fetch_feedback(
    cfg: &ConnConfig,
    run: &str,
    station: &str,
) -> Result<FeedbackListResponse, WireError> {
    get_json(&cfg.authority(), &cfg.feedback_list_path(run, station)).await
}

/// POST a station annotation as a `user-visual` feedback item
/// (`/api/feedback/:run/:station`) — the review surface's `submit_annotation`
/// path for a text/markdown artifact (or a global station note). Image / live
/// HTML artifacts go through [`submit_output_review`] instead, which records the
/// pin geometry. Either way the engine mints an `FB-NN` item the inbox lists and
/// the agent re-references.
pub async fn submit_annotation(
    cfg: &ConnConfig,
    run: &str,
    station: &str,
    req: &FeedbackCreateRequest,
) -> Result<(), WireError> {
    post_json(&cfg.authority(), &cfg.feedback_list_path(run, station), req).await
}

/// PUT a feedback-item status update (`/api/feedback/:run/:station/:id`) — the
/// inbox's resolve / dismiss chips transition an item out of the open set.
pub async fn update_feedback(
    cfg: &ConnConfig,
    run: &str,
    station: &str,
    id: &str,
    req: &FeedbackUpdateRequest,
) -> Result<(), WireError> {
    send_json("PUT", &cfg.authority(), &cfg.feedback_item_path(run, station, id), req).await
}

/// GET the `current` focus session and return the run slug it names, if any.
///
/// `darkrun_show` upserts a Review payload under the `current` session id whose
/// `run_slug` is the run to display; the home screen polls this to navigate when
/// the agent raises a run. `None` when no focus is set or the engine is
/// unreachable (the home then just stays on the run list).
pub async fn fetch_current_focus(cfg: &ConnConfig) -> Option<String> {
    let v: serde_json::Value = get_json(&cfg.authority(), "/api/session/current").await.ok()?;
    if v.get("session_type").and_then(|t| t.as_str()) != Some("review") {
        return None;
    }
    v.get("run_slug")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// GET a single run's detail (`/api/runs/:slug`) and decode it into a
/// [`RunDetailPayload`].
pub async fn fetch_run_detail(
    cfg: &ConnConfig,
    slug: &str,
) -> Result<RunDetailPayload, WireError> {
    get_json(&cfg.authority(), &cfg.run_detail_path(slug)).await
}

/// Hand-rolled HTTP/1.1 JSON GET over loopback TCP. Mirrors [`post_json`]: write
/// the request, read the whole (small) response, check the status, and decode
/// the body into `T`. Used by the run-browser fetches.
async fn get_json<T: DeserializeOwned>(authority: &str, path: &str) -> Result<T, WireError> {
    let request = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         Accept: application/json\r\n\
         Connection: close\r\n\
         \r\n",
    );

    let mut stream = TcpStream::connect(&authority)
        .await
        .map_err(WireError::Connect)?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(WireError::Io)?;
    stream.flush().await.map_err(WireError::Io)?;

    let mut buf = Vec::with_capacity(2048);
    stream.read_to_end(&mut buf).await.map_err(WireError::Io)?;

    let status = parse_status_code(&buf)?;
    if !(200..300).contains(&status) {
        return Err(WireError::Status(status));
    }
    let body = split_body(&buf);
    serde_json::from_slice::<T>(body).map_err(WireError::Encode)
}

/// Split an HTTP/1.1 response into its body — everything after the first blank
/// line (`\r\n\r\n`). Falls back to the whole buffer if no header terminator is
/// found (defensive; the engine always sends well-formed responses).
fn split_body(buf: &[u8]) -> &[u8] {
    const SEP: &[u8] = b"\r\n\r\n";
    buf.windows(SEP.len())
        .position(|w| w == SEP)
        .map(|i| &buf[i + SEP.len()..])
        .unwrap_or(buf)
}

/// Hand-rolled HTTP/1.1 JSON POST over loopback TCP, shared by every decision /
/// answer / selection path. Serializes `req`, writes the request, reads the
/// response, and surfaces a typed result on the status code.
async fn post_json<T: Serialize>(
    authority: &str,
    path: &str,
    req: &T,
) -> Result<(), WireError> {
    send_json("POST", authority, path, req).await
}

/// Hand-rolled HTTP/1.1 JSON request over loopback TCP for an arbitrary method
/// (`POST` / `PUT`), shared by the decision / answer / annotation / feedback-
/// update paths. Serializes `req`, writes the request, reads the response, and
/// surfaces a typed result on the status code.
async fn send_json<T: Serialize>(
    method: &str,
    authority: &str,
    path: &str,
    req: &T,
) -> Result<(), WireError> {
    let body = serde_json::to_vec(req).map_err(WireError::Encode)?;

    let request = format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n",
        len = body.len(),
    );

    let mut stream = TcpStream::connect(&authority)
        .await
        .map_err(WireError::Connect)?;
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(WireError::Io)?;
    stream.write_all(&body).await.map_err(WireError::Io)?;
    stream.flush().await.map_err(WireError::Io)?;

    // Read the whole response (small) and parse the status code off the first line.
    let mut buf = Vec::with_capacity(512);
    stream.read_to_end(&mut buf).await.map_err(WireError::Io)?;
    let status = parse_status_code(&buf)?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(WireError::Status(status))
    }
}

/// Pull the numeric status code out of an HTTP/1.1 response head
/// (`HTTP/1.1 200 OK`).
pub fn parse_status_code(buf: &[u8]) -> Result<u16, WireError> {
    let head = String::from_utf8_lossy(buf);
    let first = head.lines().next().unwrap_or_default();
    first
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| WireError::Io(std::io::Error::other("malformed HTTP status line")))
}

/// A live engine discovered under `~/.darkrun`, projected from the registry's
/// `EngineDescriptor` down to what the desktop needs to connect and label it.
///
/// Mirrors the on-disk descriptor (see `darkrun_mcp::registry::EngineDescriptor`)
/// but flattens the bound `SocketAddr` to just the `port` the desktop dials and
/// keeps the absolute `project_path` (the engine's repo root) for matching a
/// selected run to its engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredEngine {
    /// Absolute repo root the engine was launched against.
    pub project_path: PathBuf,
    /// The loopback port the engine's HTTP/WS server is listening on.
    pub port: u16,
    /// Slug derived from `project_path`; matches the registry directory name.
    pub slug: String,
    /// OS process id of the live engine.
    pub pid: u32,
    /// Harness key the engine adapted to, for display.
    pub harness: String,
    /// RFC3339 timestamp the engine announced itself at.
    pub started_at: String,
}

impl DiscoveredEngine {
    /// Project a registry descriptor into the desktop-facing shape, dropping the
    /// host (always loopback) and keeping only the port.
    fn from_descriptor(d: darkrun_mcp::registry::EngineDescriptor) -> Self {
        DiscoveredEngine {
            project_path: d.repo_root,
            port: d.addr.port(),
            slug: d.slug,
            pid: d.pid,
            harness: d.harness,
            started_at: d.started_at,
        }
    }
}

/// Discover every LIVE engine advertised under `~/.darkrun`.
///
/// Delegates the descriptor lifecycle (active vs `.stale`, pid liveness) to the
/// registry's [`list_live_engines`](darkrun_mcp::registry::list_live_engines), so
/// the desktop and the engine agree exactly on what "live" means. Returns an
/// empty list when the tree doesn't exist (no engine has ever booted).
#[cfg(not(tarpaulin_include))] // reads the real ~/.darkrun registry; list_live_engines_in is tested
pub async fn discover_live_engines() -> io::Result<Vec<DiscoveredEngine>> {
    let descriptors = darkrun_mcp::registry::list_live_engines()?;
    // The registry's pid check is necessary but not sufficient: a dead
    // engine's pid can be REUSED by an unrelated process, leaving a ghost
    // descriptor that looks alive for days. The engine's loopback port is the
    // strong signal — probe it (local, ~ms) and drop descriptors nothing
    // answers on.
    Ok(descriptors
        .into_iter()
        .filter(|d| {
            std::net::TcpStream::connect_timeout(
                &d.addr,
                std::time::Duration::from_millis(250),
            )
            .is_ok()
        })
        .map(DiscoveredEngine::from_descriptor)
        .collect())
}

/// Find the loopback port of the live engine serving `project_path`, if any.
///
/// Matches on the absolute repo root the engine recorded; both sides are assumed
/// absolute (the registry stores an absolute `repo_root`). Returns the first
/// match's port, or `None` when no live engine serves that project.
pub fn find_engine_for_project(engines: &[DiscoveredEngine], project_path: &Path) -> Option<u16> {
    engines
        .iter()
        .find(|e| e.project_path == project_path)
        .map(|e| e.port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_loopback() {
        // Construct directly to avoid env coupling in the test.
        let cfg = ConnConfig {
            host: "127.0.0.1".into(),
            port: 7878,
            session_id: "abc".into(),
        };
        assert_eq!(cfg.ws_url(), "ws://127.0.0.1:7878/ws/session/abc");
        assert_eq!(cfg.decide_path(), "/review/abc/decide");
        assert_eq!(cfg.authority(), "127.0.0.1:7878");
    }

    #[test]
    fn session_submit_paths_match_routes() {
        let cfg = ConnConfig {
            host: "127.0.0.1".into(),
            port: 7878,
            session_id: "s-42".into(),
        };
        assert_eq!(cfg.question_answer_path(), "/question/s-42/answer");
        assert_eq!(cfg.direction_select_path(), "/direction/s-42/select");
        assert_eq!(cfg.picker_select_path(), "/picker/s-42/select");
        assert_eq!(
            cfg.visual_review_annotate_path(),
            "/visual-review/s-42/annotate"
        );
        assert_eq!(cfg.proof_path("my-run"), "/api/proof/my-run");
        assert_eq!(
            cfg.feedback_list_path("my-run", "build"),
            "/api/feedback/my-run/build"
        );
    }

    #[test]
    fn runs_browse_paths_and_with_session() {
        let cfg = ConnConfig {
            host: "127.0.0.1".into(),
            port: 7878,
            session_id: "current".into(),
        };
        assert_eq!(cfg.runs_path(), "/api/runs");
        assert_eq!(cfg.run_detail_path("alpha"), "/api/runs/alpha");
        // with_session swaps only the session id, keeping host/port.
        let pinned = cfg.with_session("alpha");
        assert_eq!(pinned.session_id, "alpha");
        assert_eq!(pinned.authority(), "127.0.0.1:7878");
        assert_eq!(pinned.ws_url(), "ws://127.0.0.1:7878/ws/session/alpha");
    }

    #[test]
    fn split_body_extracts_payload_after_headers() {
        let resp = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"runs\":[]}";
        assert_eq!(split_body(resp), b"{\"runs\":[]}");
    }

    #[test]
    fn split_body_falls_back_to_whole_buffer_without_separator() {
        let resp = b"no-headers-here";
        assert_eq!(split_body(resp), b"no-headers-here");
    }

    #[test]
    fn status_code_parsing() {
        assert_eq!(
            parse_status_code(b"HTTP/1.1 200 OK\r\n\r\n{}").unwrap(),
            200
        );
        assert_eq!(
            parse_status_code(b"HTTP/1.1 404 Not Found\r\n").unwrap(),
            404
        );
        assert!(parse_status_code(b"garbage").is_err());
    }

    // --- Engine discovery -------------------------------------------------

    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    fn discovered(project: &str, port: u16) -> DiscoveredEngine {
        DiscoveredEngine {
            project_path: PathBuf::from(project),
            port,
            slug: format!("slug-{port}"),
            pid: 1234,
            harness: "claude".into(),
            started_at: "2026-05-31T00:00:00+00:00".into(),
        }
    }

    #[test]
    fn from_descriptor_flattens_addr_to_port() {
        let desc = darkrun_mcp::registry::EngineDescriptor {
            pid: 4242,
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5151)),
            repo_root: PathBuf::from("/Users/dev/proj"),
            slug: "proj-deadbeef".into(),
            harness: "claude".into(),
            started_at: "2026-05-31T00:00:00+00:00".into(),
        };
        let eng = DiscoveredEngine::from_descriptor(desc);
        assert_eq!(eng.port, 5151);
        assert_eq!(eng.project_path, PathBuf::from("/Users/dev/proj"));
        assert_eq!(eng.pid, 4242);
        assert_eq!(eng.slug, "proj-deadbeef");
    }

    #[test]
    fn find_engine_for_project_matches_by_path() {
        let engines = vec![discovered("/Users/dev/a", 7001), discovered("/Users/dev/b", 7002)];
        assert_eq!(
            find_engine_for_project(&engines, Path::new("/Users/dev/b")),
            Some(7002)
        );
    }

    #[test]
    fn find_engine_for_project_no_match_returns_none() {
        let engines = vec![discovered("/Users/dev/a", 7001)];
        assert_eq!(
            find_engine_for_project(&engines, Path::new("/Users/dev/missing")),
            None
        );
    }

    #[test]
    fn discovery_filters_to_live_and_excludes_stale() {
        // Mirror `discover_live_engines`'s internals against a temp registry tree:
        // an announced engine for the live pid is returned; a stale-flagged one is
        // not. The registry owns the active-vs-stale + liveness rules.
        let tmp = tempfile::tempdir().unwrap();
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 6161));

        let live = darkrun_mcp::registry::EngineRegistry::with_root(tmp.path(), "/Users/dev/live");
        live.announce(addr, "claude").unwrap();

        let gone = darkrun_mcp::registry::EngineRegistry::with_root(tmp.path(), "/Users/dev/gone");
        gone.announce(addr, "claude").unwrap();
        gone.mark_stale().unwrap();

        let engines: Vec<DiscoveredEngine> =
            darkrun_mcp::registry::list_live_engines_in(tmp.path())
                .unwrap()
                .into_iter()
                .map(DiscoveredEngine::from_descriptor)
                .collect();

        assert_eq!(engines.len(), 1);
        assert_eq!(engines[0].project_path, PathBuf::from("/Users/dev/live"));
        assert_eq!(engines[0].port, 6161);
        // The matcher then resolves the right project's port.
        assert_eq!(
            find_engine_for_project(&engines, Path::new("/Users/dev/live")),
            Some(6161)
        );
    }
}

#[cfg(test)]
mod async_client_tests {
    use super::*;

    /// A config pointing at a port nothing listens on — every client call fails
    /// fast, exercising the request-build + error/None paths without a server.
    fn dead() -> ConnConfig {
        ConnConfig { host: "127.0.0.1".into(), port: 1, session_id: "s".into() }
    }

    #[tokio::test]
    async fn fetchers_and_feed_handle_a_dead_engine() {
        let cfg = dead();
        assert!(fetch_runs(&cfg).await.is_err());
        assert!(fetch_run_detail(&cfg, "r").await.is_err());
        assert!(fetch_current_focus(&cfg).await.is_none());
        assert!(fetch_feedback(&cfg, "r", "frame").await.is_err());
        assert!(submit_unit_reset(&cfg, "r", "u1").await.is_err());

        // The WS feed reports a disconnect and returns (no live socket).
        let mut saw_disconnect = false;
        run_session_feed(&cfg, |ev| {
            if matches!(ev, FeedEvent::Disconnected(_)) {
                saw_disconnect = true;
            }
        })
        .await;
        assert!(saw_disconnect, "a dead engine yields a Disconnected event");
    }

    // ── A loopback fixture server exercising the success paths ───────────────
    // Stands up a TCP listener that speaks just enough HTTP/1.1 to drive the
    // hand-rolled client: it reads each request line, routes on the path, and
    // writes a canned 200 + JSON body, then closes (Connection: close) so the
    // client's read_to_end returns. Covers get_json/post_json/send_json,
    // split_body, parse_status_code, and every fetch_*/submit_* Ok path.

    fn body_for(path: &str) -> String {
        if path == "/api/runs" {
            r#"{"runs":[],"count":0}"#.to_string()
        } else if path.starts_with("/api/runs/") {
            r#"{"slug":"r","title":"T","factory":"software","active_station":"build",
                "status":"active","progress":{"completed":1,"total":3},
                "stations":[],"units":[]}"#
                .to_string()
        } else if path.starts_with("/api/feedback/") {
            r#"{"run":"r","station":"build","count":0,"items":[]}"#.to_string()
        } else if path == "/api/session/current" {
            r#"{"session_type":"review","run_slug":"r1"}"#.to_string()
        } else {
            "{}".to_string()
        }
    }

    async fn serve_canned(n: usize) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..n {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                // Drain the full request (headers + any Content-Length body) so the
                // client finishes writing before we respond — otherwise closing the
                // socket mid-write resets the peer.
                let mut acc = Vec::new();
                let mut chunk = [0u8; 2048];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    acc.extend_from_slice(&chunk[..n]);
                    let head = String::from_utf8_lossy(&acc);
                    if let Some(hdr_end) = acc.windows(4).position(|w| w == b"\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                let l = l.to_ascii_lowercase();
                                l.strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if acc.len() >= hdr_end + 4 + len {
                            break;
                        }
                    }
                }
                let head = String::from_utf8_lossy(&acc);
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let body = body_for(&path);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
                // Drop closes the socket → the client's read_to_end completes.
            }
        });
        port
    }

    #[tokio::test]
    async fn client_success_paths_against_a_fixture_server() {
        // 11 client calls below → the server handles 11 connections.
        let port = serve_canned(11).await;
        let cfg = ConnConfig { host: "127.0.0.1".into(), port, session_id: "s".into() };

        // GETs decode their typed payloads off a real socket.
        let runs = fetch_runs(&cfg).await.expect("runs decode");
        assert_eq!(runs.count, 0);
        let detail = fetch_run_detail(&cfg, "r").await.expect("detail decode");
        assert_eq!(detail.slug, "r");
        let fb = fetch_feedback(&cfg, "r", "build").await.expect("feedback decode");
        assert_eq!(fb.station, "build");
        assert_eq!(fetch_current_focus(&cfg).await.as_deref(), Some("r1"));

        // POST / PUT success paths just check the 2xx status.
        submit_decision(&cfg, &ReviewDecisionRequest {
            decision: "approved".into(), feedback: None, annotations: None,
        }).await.expect("decision ok");
        submit_unit_reset(&cfg, "r", "u1").await.expect("reset ok");
        submit_annotation(&cfg, "r", "build", &FeedbackCreateRequest {
            title: "n".into(), body: "b".into(), origin: None, author: None,
            source_ref: None, anchor: None, inline_anchor: None, resolution: None,
            attachment_data_url: None,
        }).await.expect("annotation ok");
        update_feedback(&cfg, "r", "build", "FB-1", &FeedbackUpdateRequest::default())
            .await.expect("update ok");
        submit_question_answer(&cfg, &QuestionAnswerRequest {
            selected: vec!["a".into()], text: None, annotations: None,
        }).await.expect("answer ok");
        submit_direction_select(&cfg, &DirectionSelectRequest {
            archetype: "editorial".into(), annotations: None,
        }).await.expect("direction ok");
        submit_picker_select(&cfg, &PickerSelectRequest { id: "yes".into() })
            .await.expect("picker ok");
    }

    #[tokio::test]
    async fn focus_is_none_for_a_non_review_session_and_get_surfaces_a_non_2xx() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A server that returns a non-review session for /current and a 404 for runs.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for _ in 0..2 {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let path = String::from_utf8_lossy(&buf[..n])
                    .lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_string();
                let resp = if path == "/api/session/current" {
                    let body = r#"{"session_type":"question","run_slug":"r1"}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                };
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        let cfg = ConnConfig { host: "127.0.0.1".into(), port, session_id: "s".into() };
        // A non-review focus session yields None.
        assert!(fetch_current_focus(&cfg).await.is_none());
        // A 404 GET surfaces a Status error (the non-2xx arm).
        assert!(matches!(fetch_runs(&cfg).await, Err(WireError::Status(404))));
    }

    #[tokio::test]
    async fn parse_and_helpers_cover_their_arms() {
        assert_eq!(parse_status_code(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap(), 204);
        assert!(parse_status_code(b"garbage").is_err());
        assert_eq!(split_body(b"HTTP/1.1 200 OK\r\n\r\nbody"), b"body");
        assert_eq!(split_body(b"no-terminator"), b"no-terminator");

        // The path builders + config helpers.
        let cfg = ConnConfig { host: "127.0.0.1".into(), port: 9, session_id: "sess".into() };
        assert_eq!(cfg.feedback_item_path("r", "build", "FB-1"), "/api/feedback/r/build/FB-1");
        assert!(cfg.ws_url().starts_with("ws://127.0.0.1:9/ws/session/sess"));
        assert_eq!(cfg.with_session("other").session_id, "other");

        // WireError Display covers every variant.
        for e in [
            WireError::Connect(std::io::Error::other("x")),
            WireError::Io(std::io::Error::other("y")),
            WireError::Encode(serde_json::from_str::<i32>("bad").unwrap_err()),
            WireError::Status(503),
        ] {
            assert!(!e.to_string().is_empty());
        }

        // from_env_pinned + find_engine_for_project.
        let (_c, _pinned) = ConnConfig::from_env_pinned();
        let engines = vec![DiscoveredEngine {
            project_path: std::path::PathBuf::from("/repo"),
            port: 7, slug: "s".into(), pid: 1, harness: "claude-code".into(),
            started_at: "t".into(),
        }];
        assert_eq!(find_engine_for_project(&engines, std::path::Path::new("/repo")), Some(7));
        assert_eq!(find_engine_for_project(&engines, std::path::Path::new("/other")), None);
    }
}

#[cfg(test)]
mod asset_url_tests {
    use super::ConnConfig;

    fn cfg() -> ConnConfig {
        ConnConfig { host: "127.0.0.1".into(), port: 59298, session_id: "s".into() }
    }

    #[test]
    fn rewrites_file_urls_under_the_runs_assets_dir() {
        let c = cfg();
        let file = "file:///Users/me/dev/proj/.claude/worktrees/wt/.darkrun/darkrun-sim/assets/options-dark.jpg";
        assert_eq!(
            c.asset_url("darkrun-sim", file),
            "http://127.0.0.1:59298/api/runs/darkrun-sim/asset/options-dark.jpg"
        );
    }

    #[test]
    fn passes_through_non_file_and_out_of_tree_urls() {
        let c = cfg();
        // Already an http(s) url — untouched.
        assert_eq!(c.asset_url("r", "https://img/a.png"), "https://img/a.png");
        // A file:// url NOT under this run's assets dir — untouched (can't serve it).
        let other = "file:///etc/passwd";
        assert_eq!(c.asset_url("r", other), other);
        // Right shape but a different run slug — not this run's asset.
        let elsewhere = "file:///x/.darkrun/other-run/assets/a.png";
        assert_eq!(c.asset_url("r", elsewhere), elsewhere);
    }
}

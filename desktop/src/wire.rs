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

use darkrun_api::{
    DirectionSelectRequest, PickerSelectRequest, QuestionAnswerRequest, ReviewDecisionRequest,
    SessionPayload,
};
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

    /// The `host:port` authority.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
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

/// Hand-rolled HTTP/1.1 JSON POST over loopback TCP, shared by every decision /
/// answer / selection path. Serializes `req`, writes the request, reads the
/// response, and surfaces a typed result on the status code.
async fn post_json<T: Serialize>(
    authority: &str,
    path: &str,
    req: &T,
) -> Result<(), WireError> {
    let body = serde_json::to_vec(req).map_err(WireError::Encode)?;

    let request = format!(
        "POST {path} HTTP/1.1\r\n\
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
}

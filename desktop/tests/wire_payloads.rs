//! Session-payload decoding (what the feed filters on) + decision request
//! serialization + the live `submit_decision` HTTP round-trip over loopback.

use darkrun_api::common::SessionStatus;
use darkrun_api::session::SessionPayload;
use darkrun_api::ReviewDecisionRequest;
use darkrun_desktop::wire::{submit_decision, ConnConfig, WireError};
use serde_json::json;

// ---- SessionPayload decode: the run_session_feed text path accepts only
//      well-formed session frames; everything else is skipped. ----

fn decode(text: &str) -> Option<SessionPayload> {
    serde_json::from_str::<SessionPayload>(text).ok()
}

#[test]
fn decodes_minimal_review_frame() {
    let frame = json!({
        "session_type": "review",
        "session_id": "s1",
        "status": "pending",
    })
    .to_string();
    let p = decode(&frame).expect("review frame should decode");
    assert_eq!(p.session_type(), "review");
    assert_eq!(p.session_id(), "s1");
    match p {
        SessionPayload::Review(r) => assert_eq!(r.status, SessionStatus::Pending),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn decodes_question_frame() {
    let frame = json!({
        "session_type": "question",
        "session_id": "q1",
        "status": "answered",
    })
    .to_string();
    let p = decode(&frame).expect("question frame should decode");
    assert_eq!(p.session_type(), "question");
    assert_eq!(p.session_id(), "q1");
}

#[test]
fn decodes_review_with_units_and_status() {
    let frame = json!({
        "session_type": "review",
        "session_id": "r2",
        "status": "changes_requested",
        "run_slug": "my-run",
        "gate_type": "await",
        "await_active": true,
        "units": [
            { "title": "u1", "status": "active", "pass": 1 },
            { "name": "u2", "criteria": ["x"] }
        ]
    })
    .to_string();
    let p = decode(&frame).expect("decode");
    match p {
        SessionPayload::Review(r) => {
            assert_eq!(r.run_slug.as_deref(), Some("my-run"));
            assert_eq!(r.status, SessionStatus::ChangesRequested);
            assert_eq!(r.units.len(), 2);
            assert_eq!(r.await_active, Some(true));
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn skips_non_session_text_frame() {
    // A control/heartbeat text frame the feed should drop.
    assert!(decode(r#"{"type":"ping"}"#).is_none());
    assert!(decode(r#"{"hello":"world"}"#).is_none());
}

#[test]
fn skips_plain_text_frame() {
    assert!(decode("not json at all").is_none());
    assert!(decode("").is_none());
}

#[test]
fn skips_unknown_session_type() {
    let frame = json!({ "session_type": "wat", "session_id": "x" }).to_string();
    assert!(decode(&frame).is_none());
}

#[test]
fn skips_review_frame_missing_required_fields() {
    // `status` is required (no serde default beyond the enum's own default is
    // applied to a missing field unless annotated) — session_id is required.
    let frame = json!({ "session_type": "review" }).to_string();
    assert!(decode(&frame).is_none());
}

#[test]
fn decodes_review_frame_via_slice_path() {
    // The binary-message path uses from_slice; equivalent decode.
    let frame = json!({
        "session_type": "review",
        "session_id": "b1",
        "status": "approved",
    })
    .to_string();
    let p = serde_json::from_slice::<SessionPayload>(frame.as_bytes()).expect("slice decode");
    assert_eq!(p.session_type(), "review");
    match p {
        SessionPayload::Review(r) => assert_eq!(r.status, SessionStatus::Approved),
        _ => panic!(),
    }
}

#[test]
fn session_type_discriminator_is_snake_case() {
    // review + question decode from the minimal {session_type, session_id,
    // status} shape; the other variants carry additional required fields.
    for (ty, id) in [("review", "a"), ("question", "b")] {
        let frame = json!({
            "session_type": ty,
            "session_id": id,
            "status": "pending"
        })
        .to_string();
        let p = decode(&frame).unwrap_or_else(|| panic!("decode {ty}"));
        assert_eq!(p.session_type(), ty);
        assert_eq!(p.session_id(), id);
    }
}

#[test]
fn decodes_fully_specified_picker_frame() {
    // Picker requires kind/title/prompt/options.
    let frame = json!({
        "session_type": "picker",
        "session_id": "p1",
        "status": "pending",
        "kind": "station",
        "title": "Pick a station",
        "prompt": "Which one?",
        "options": [{ "id": "main", "label": "main" }]
    })
    .to_string();
    let p = decode(&frame).expect("picker should decode with required fields");
    assert_eq!(p.session_type(), "picker");
    assert_eq!(p.session_id(), "p1");
}

#[test]
fn skips_picker_frame_missing_required_fields() {
    // Missing kind/title/prompt/options -> not decodable -> feed skips it.
    let frame = json!({
        "session_type": "picker",
        "session_id": "p2",
        "status": "pending"
    })
    .to_string();
    assert!(decode(&frame).is_none());
}

// ---- ReviewDecisionRequest serialization ----

#[test]
fn decision_request_approved_minimal() {
    let req = ReviewDecisionRequest {
        decision: "approved".to_string(),
        feedback: None,
        annotations: None,
    };
    let v: serde_json::Value = serde_json::to_value(&req).unwrap();
    assert_eq!(v["decision"], "approved");
    // None fields are skipped (skip_serializing_if).
    assert!(v.get("feedback").is_none());
    assert!(v.get("annotations").is_none());
    // Exactly one key on the wire.
    assert_eq!(v.as_object().unwrap().len(), 1);
}

#[test]
fn decision_request_changes_requested() {
    let req = ReviewDecisionRequest {
        decision: "changes_requested".to_string(),
        feedback: Some("needs work".to_string()),
        annotations: None,
    };
    let v: serde_json::Value = serde_json::to_value(&req).unwrap();
    assert_eq!(v["decision"], "changes_requested");
    assert_eq!(v["feedback"], "needs work");
    assert!(v.get("annotations").is_none());
}

#[test]
fn decision_request_round_trips() {
    let req = ReviewDecisionRequest {
        decision: "approved".to_string(),
        feedback: Some("lgtm".to_string()),
        annotations: None,
    };
    let bytes = serde_json::to_vec(&req).unwrap();
    let back: ReviewDecisionRequest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.decision, "approved");
    assert_eq!(back.feedback.as_deref(), Some("lgtm"));
}

#[test]
fn decision_request_decodes_with_defaults_for_optional_fields() {
    // The server may receive a body carrying only `decision`.
    let req: ReviewDecisionRequest =
        serde_json::from_str(r#"{"decision":"approved"}"#).unwrap();
    assert_eq!(req.decision, "approved");
    assert!(req.feedback.is_none());
    assert!(req.annotations.is_none());
}

#[test]
fn decision_request_raw_decision_is_verbatim() {
    // The desktop sends the raw string; canonicalization is server-side. Verify
    // an arbitrary raw value is serialized untouched.
    let req = ReviewDecisionRequest {
        decision: "Approved".to_string(),
        feedback: None,
        annotations: None,
    };
    let v: serde_json::Value = serde_json::to_value(&req).unwrap();
    assert_eq!(v["decision"], "Approved");
}

// ---- submit_decision: live loopback HTTP round-trip ----
//
// Spin a one-shot TCP listener that drains the request and writes a canned
// HTTP/1.1 response, then assert how submit_decision maps the status.

async fn one_shot_server(response: &'static [u8]) -> (ConnConfig, tokio::task::JoinHandle<Vec<u8>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // The client writes the head and the body in two separate writes, so a
        // single read() can return just the first TCP segment (the headers).
        // Loop until we have the full head plus the Content-Length body, so the
        // captured request is complete and the body assertions are not racy.
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            // Once headers are terminated, check whether the declared body has
            // fully arrived; if so we can stop reading.
            if let Some(hdr_end) = buf
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
            {
                let head = String::from_utf8_lossy(&buf[..hdr_end]);
                let content_len = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("Content-Length:")
                            .or_else(|| l.strip_prefix("content-length:"))
                    })
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= hdr_end + content_len {
                    break;
                }
            }
        }
        sock.write_all(response).await.unwrap();
        sock.flush().await.unwrap();
        // Drop closes the connection so read_to_end on the client returns.
        buf
    });
    let cfg = ConnConfig {
        host: "127.0.0.1".to_string(),
        port: addr.port(),
        session_id: "sess".to_string(),
    };
    (cfg, handle)
}

fn approved() -> ReviewDecisionRequest {
    ReviewDecisionRequest {
        decision: "approved".to_string(),
        feedback: None,
        annotations: None,
    }
}

#[tokio::test]
async fn submit_decision_ok_on_200() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
    let res = submit_decision(&cfg, &approved()).await;
    assert!(res.is_ok(), "expected Ok, got {res:?}");
    let req = handle.await.unwrap();
    let req = String::from_utf8_lossy(&req);
    // Verify the hand-rolled request shape.
    assert!(req.starts_with("POST /review/sess/decide HTTP/1.1\r\n"), "{req}");
    assert!(req.contains("Content-Type: application/json\r\n"));
    assert!(req.contains("Connection: close\r\n"));
    assert!(req.contains(&format!("Host: 127.0.0.1:{}\r\n", cfg.port)));
    // Body is the serialized decision.
    assert!(req.contains(r#"{"decision":"approved"}"#), "{req}");
    // Content-Length matches body length.
    assert!(req.contains("Content-Length: 23\r\n"), "{req}");
}

#[tokio::test]
async fn submit_decision_ok_on_204() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 204 No Content\r\n\r\n").await;
    assert!(submit_decision(&cfg, &approved()).await.is_ok());
    handle.await.unwrap();
}

#[tokio::test]
async fn submit_decision_status_err_on_404() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
    let res = submit_decision(&cfg, &approved()).await;
    match res {
        Err(WireError::Status(404)) => {}
        other => panic!("expected Status(404), got {other:?}"),
    }
    handle.await.unwrap();
}

#[tokio::test]
async fn submit_decision_status_err_on_500() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 500 Internal Server Error\r\n\r\n").await;
    match submit_decision(&cfg, &approved()).await {
        Err(WireError::Status(500)) => {}
        other => panic!("expected Status(500), got {other:?}"),
    }
    handle.await.unwrap();
}

#[tokio::test]
async fn submit_decision_status_err_on_409() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 409 Conflict\r\n\r\n").await;
    match submit_decision(&cfg, &approved()).await {
        Err(WireError::Status(409)) => {}
        other => panic!("expected Status(409), got {other:?}"),
    }
    handle.await.unwrap();
}

#[tokio::test]
async fn submit_decision_io_err_on_malformed_response() {
    let (cfg, handle) = one_shot_server(b"garbage-not-http").await;
    match submit_decision(&cfg, &approved()).await {
        Err(WireError::Io(_)) => {}
        other => panic!("expected Io error, got {other:?}"),
    }
    handle.await.unwrap();
}

#[tokio::test]
async fn submit_decision_connect_err_on_dead_port() {
    // Bind+drop to obtain a port nothing is listening on.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let cfg = ConnConfig {
        host: "127.0.0.1".to_string(),
        port,
        session_id: "x".to_string(),
    };
    match submit_decision(&cfg, &approved()).await {
        Err(WireError::Connect(_)) => {}
        other => panic!("expected Connect error, got {other:?}"),
    }
}

#[tokio::test]
async fn submit_decision_sends_changes_requested_body() {
    let (cfg, handle) = one_shot_server(b"HTTP/1.1 200 OK\r\n\r\n").await;
    let req = ReviewDecisionRequest {
        decision: "changes_requested".to_string(),
        feedback: Some("fix it".to_string()),
        annotations: None,
    };
    assert!(submit_decision(&cfg, &req).await.is_ok());
    let sent = handle.await.unwrap();
    let sent = String::from_utf8_lossy(&sent);
    assert!(sent.contains(r#""decision":"changes_requested""#), "{sent}");
    assert!(sent.contains(r#""feedback":"fix it""#), "{sent}");
}

// ---- WireError Display ----

#[test]
fn wire_error_display_status() {
    let e = WireError::Status(503);
    assert_eq!(e.to_string(), "engine returned HTTP 503");
}

#[test]
fn wire_error_display_encode() {
    // Construct a serde error by deserializing into a tight type.
    let serde_err = serde_json::from_str::<u8>("not-a-number").unwrap_err();
    let e = WireError::Encode(serde_err);
    assert!(e.to_string().starts_with("encode failed:"), "{e}");
}

#[test]
fn wire_error_display_io_and_connect() {
    let io = WireError::Io(std::io::Error::other("boom"));
    assert!(io.to_string().starts_with("io error:"), "{io}");
    let conn = WireError::Connect(std::io::Error::other("nope"));
    assert!(conn.to_string().starts_with("connect failed:"), "{conn}");
}

#[test]
fn wire_error_is_std_error() {
    fn assert_err<E: std::error::Error>(_: &E) {}
    assert_err(&WireError::Status(500));
}

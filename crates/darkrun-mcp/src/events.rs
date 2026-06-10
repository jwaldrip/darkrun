//! Engine telemetry: the per-run **session-event stream** plus an optional
//! OTLP export — the predecessor's `logSessionEvent` + `emitTelemetry` pair.
//!
//! Two sinks, one call:
//!
//! - **`events.jsonl`** (always): every engine lifecycle event appends to the
//!   run's journal — `darkrun.run.created`, `darkrun.manager.action`,
//!   `darkrun.gate.entered`, `darkrun.unit.completed`,
//!   `darkrun.unit.scope_violation`, `darkrun.station.dropped`,
//!   `darkrun.run.sealed`, … — a local-first, append-only observability trail
//!   the operator (and the reflection pass) can read without any backend.
//! - **OTLP logs** (when configured): the same event POSTs to an OTLP/HTTP+JSON
//!   logs endpoint, fire-and-forget on a detached thread — never blocks, never
//!   fails the engine. Configured by the standard OTel env, with a
//!   `DARKRUN_`-prefixed override that WINS when present (the predecessor's
//!   lesson: harnesses stopped forwarding bare `OTEL_*` into MCP subprocesses,
//!   so the operator needs a name the engine reads directly):
//!
//!   - `DARKRUN_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT`
//!   - `DARKRUN_OTEL_EXPORTER_OTLP_HEADERS` / `OTEL_EXPORTER_OTLP_HEADERS`
//!     (W3C-baggage style `k=v,k2=v2`; values percent-decoded)
//!
//!   Unset endpoint → no network, the local stream still records everything.

use chrono::Utc;
use darkrun_core::StateStore;

/// Read an env var with the `DARKRUN_`-prefixed override winning.
fn env(name: &str) -> String {
    let prefixed = std::env::var(format!("DARKRUN_{name}")).unwrap_or_default();
    let prefixed = prefixed.trim();
    if !prefixed.is_empty() {
        return prefixed.to_string();
    }
    std::env::var(name).unwrap_or_default().trim().to_string()
}

/// Parse `k=v,k2=v2` OTLP header pairs (percent-decoded values; first `=`
/// splits, later `=` belong to the value — base64 padding survives).
fn parse_headers(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in raw.split(',') {
        let pair = pair.trim();
        let Some(eq) = pair.find('=') else { continue };
        if eq == 0 {
            continue;
        }
        let key = pair[..eq].trim().to_string();
        let value = percent_decode(pair[eq + 1..].trim());
        if !key.is_empty() {
            out.push((key, value));
        }
    }
    out
}

/// Minimal percent-decoding (enough for W3C-baggage header values).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Emit one engine event: append to the run's `events.jsonl` (always) and
/// export to OTLP when an endpoint is configured. `fields` is a flat JSON
/// object of event-specific attributes. Fire-and-forget on both sinks — an
/// event must never fail the engine.
pub fn emit(store: &StateStore, slug: &str, event: &str, fields: serde_json::Value) {
    let at = Utc::now().to_rfc3339();
    let mut entry = serde_json::json!({
        "at": at,
        "event": event,
        "run": slug,
    });
    if let (Some(obj), Some(extra)) = (entry.as_object_mut(), fields.as_object()) {
        for (k, v) in extra {
            obj.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let _ = store.append_journal(slug, "events.jsonl", &entry.to_string());
    otlp_export(event, &entry);
}

/// POST the event to the configured OTLP/HTTP+JSON logs endpoint on a detached
/// thread. No endpoint → no-op. Errors are swallowed (fire-and-forget); the
/// thread carries its own short timeout so a black-holed collector can't pile
/// up threads behind a wedged socket forever.
fn otlp_export(event: &str, entry: &serde_json::Value) {
    let endpoint = env("OTEL_EXPORTER_OTLP_ENDPOINT");
    if endpoint.is_empty() {
        return;
    }
    let url = format!("{}/v1/logs", endpoint.trim_end_matches('/'));
    let headers = parse_headers(&env("OTEL_EXPORTER_OTLP_HEADERS"));
    let body = otlp_log_body(event, entry);
    std::thread::Builder::new()
        .name("darkrun-otlp".into())
        .spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(5))
                .build();
            let mut req = agent.post(&url).set("Content-Type", "application/json");
            for (k, v) in &headers {
                req = req.set(k, v);
            }
            let _ = req.send_string(&body.to_string());
        })
        .ok();
}

/// Shape the event as a minimal OTLP/HTTP+JSON `logs` payload: one resource
/// (`service.name: darkrun`), one log record whose body is the event name and
/// whose attributes are the entry's flat fields.
fn otlp_log_body(event: &str, entry: &serde_json::Value) -> serde_json::Value {
    let nanos = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_default()
        .to_string();
    let attributes: Vec<serde_json::Value> = entry
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| k.as_str() != "event")
                .map(|(k, v)| {
                    let value = match v {
                        serde_json::Value::String(s) => {
                            serde_json::json!({ "stringValue": s })
                        }
                        serde_json::Value::Number(n) if n.is_i64() => {
                            serde_json::json!({ "intValue": n.to_string() })
                        }
                        serde_json::Value::Bool(b) => serde_json::json!({ "boolValue": b }),
                        other => serde_json::json!({ "stringValue": other.to_string() }),
                    };
                    serde_json::json!({ "key": k, "value": value })
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "resourceLogs": [{
            "resource": { "attributes": [
                { "key": "service.name", "value": { "stringValue": "darkrun" } },
                { "key": "service.version",
                  "value": { "stringValue": env!("CARGO_PKG_VERSION") } },
            ]},
            "scopeLogs": [{
                "scope": { "name": "darkrun-engine" },
                "logRecords": [{
                    "timeUnixNano": nanos,
                    "severityText": "INFO",
                    "body": { "stringValue": event },
                    "attributes": attributes,
                }],
            }],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn emit_appends_a_session_event_line() {
        let dir = tempdir().unwrap();
        let store = StateStore::new(dir.path());
        emit(
            &store,
            "r",
            "darkrun.unit.completed",
            serde_json::json!({ "unit": "u1", "station": "build" }),
        );
        let path = store.run_dir("r").join("events.jsonl");
        let raw = std::fs::read_to_string(path).expect("events.jsonl written");
        let line: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        assert_eq!(line["event"], "darkrun.unit.completed");
        assert_eq!(line["run"], "r");
        assert_eq!(line["unit"], "u1");
        assert_eq!(line["station"], "build");
        assert!(line["at"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn events_accumulate_append_only() {
        let dir = tempdir().unwrap();
        let store = StateStore::new(dir.path());
        emit(&store, "r", "a", serde_json::json!({}));
        emit(&store, "r", "b", serde_json::json!({}));
        let raw =
            std::fs::read_to_string(store.run_dir("r").join("events.jsonl")).expect("journal");
        assert_eq!(raw.lines().count(), 2);
    }

    #[test]
    fn header_parsing_is_baggage_style() {
        let h = parse_headers("Authorization=Basic%20dXNlcg==,x-team=core");
        assert_eq!(h[0].0, "Authorization");
        assert_eq!(h[0].1, "Basic dXNlcg==");
        assert_eq!(h[1], ("x-team".into(), "core".into()));
        assert!(parse_headers("").is_empty());
        assert!(parse_headers("=novalue").is_empty());
    }

    #[test]
    fn otlp_body_carries_event_and_attributes() {
        let entry = serde_json::json!({
            "event": "darkrun.gate.entered",
            "run": "r",
            "station": "build",
        });
        let body = otlp_log_body("darkrun.gate.entered", &entry);
        let rec = &body["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
        assert_eq!(rec["body"]["stringValue"], "darkrun.gate.entered");
        let attrs = rec["attributes"].as_array().unwrap();
        assert!(attrs.iter().any(|a| a["key"] == "run"));
        assert!(attrs.iter().all(|a| a["key"] != "event"), "event rides the body");
    }

    #[test]
    fn prefixed_env_wins_over_bare() {
        std::env::set_var("DARKRUN_OTEL_TEST_KEY", "prefixed");
        std::env::set_var("OTEL_TEST_KEY", "bare");
        assert_eq!(env("OTEL_TEST_KEY"), "prefixed");
        std::env::remove_var("DARKRUN_OTEL_TEST_KEY");
        assert_eq!(env("OTEL_TEST_KEY"), "bare");
        std::env::remove_var("OTEL_TEST_KEY");
    }
}

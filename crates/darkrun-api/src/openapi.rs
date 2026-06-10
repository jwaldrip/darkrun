//! OpenAPI document builder.
//!
//! Emits an OpenAPI 3.1 document for the darkrun HTTP surface from two sources
//! of truth that already exist in this crate:
//!
//! - the [`crate::routes::ROUTES`] descriptor table (paths, methods, operation
//!   ids, summaries, tags), and
//! - the `schemars`-generated JSON Schema for every wire type, collected into
//!   `components.schemas`.
//!
//! The document is intentionally hand-assembled rather than pulled from a
//! framework so the contract crate stays dependency-light (`serde` + `schemars`
//! only) and the emitted file is deterministic — a property the parity test
//! relies on.

use schemars::schema_for;
use serde_json::{json, Map, Value};

use crate::routes::{HttpMethod, ROUTES};

/// The OpenAPI document version string emitted in `info.version`.
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Insert the schema for `$ty` into `$schemas` under the clean name `$name`.
macro_rules! collect_schema {
    ($schemas:expr, $name:literal, $ty:ty) => {{
        let schema = schema_for!($ty);
        let value = serde_json::to_value(schema.schema).expect("schema serializes");
        $schemas.insert($name.to_string(), value);
    }};
}

/// The set of named component schemas, keyed by their wire type name.
fn component_schemas() -> Map<String, Value> {
    let mut schemas = Map::new();

    // Session payloads + the discriminated union.
    collect_schema!(schemas, "SessionPayload", crate::session::SessionPayload);
    collect_schema!(
        schemas,
        "ReviewSessionPayload",
        crate::session::ReviewSessionPayload
    );
    collect_schema!(
        schemas,
        "QuestionSessionPayload",
        crate::session::QuestionSessionPayload
    );
    collect_schema!(
        schemas,
        "DirectionSessionPayload",
        crate::session::DirectionSessionPayload
    );
    collect_schema!(
        schemas,
        "PickerSessionPayload",
        crate::session::PickerSessionPayload
    );
    collect_schema!(
        schemas,
        "ViewSessionPayload",
        crate::session::ViewSessionPayload
    );
    collect_schema!(
        schemas,
        "VisualReviewSessionPayload",
        crate::session::VisualReviewSessionPayload
    );
    collect_schema!(
        schemas,
        "ProofSessionPayload",
        crate::session::ProofSessionPayload
    );
    collect_schema!(schemas, "Proof", crate::proof::Proof);
    collect_schema!(schemas, "OutputReviewRequest", crate::output_review::OutputReviewRequest);
    collect_schema!(schemas, "OutputReviewResponse", crate::output_review::OutputReviewResponse);
    collect_schema!(schemas, "ProofAttachRequest", crate::proof::ProofAttachRequest);
    collect_schema!(schemas, "ProofAttachResponse", crate::proof::ProofAttachResponse);
    collect_schema!(schemas, "ProofGetResponse", crate::proof::ProofGetResponse);
    collect_schema!(schemas, "RunPhase", crate::session::RunPhase);
    collect_schema!(schemas, "RunCurrentState", crate::session::RunCurrentState);

    // Review decide.
    collect_schema!(
        schemas,
        "ReviewDecisionRequest",
        crate::review::ReviewDecisionRequest
    );
    collect_schema!(
        schemas,
        "ReviewDecisionResponse",
        crate::review::ReviewDecisionResponse
    );

    // Review-current summary.
    collect_schema!(
        schemas,
        "ReviewCurrentPayload",
        crate::review_current::ReviewCurrentPayload
    );

    // Question answer.
    collect_schema!(
        schemas,
        "QuestionAnswerRequest",
        crate::question::QuestionAnswerRequest
    );
    collect_schema!(
        schemas,
        "QuestionAnswerResponse",
        crate::question::QuestionAnswerResponse
    );

    // Direction + picker select.
    collect_schema!(
        schemas,
        "DirectionSelectRequest",
        crate::direction::DirectionSelectRequest
    );
    collect_schema!(
        schemas,
        "DirectionSelectResponse",
        crate::direction::DirectionSelectResponse
    );
    collect_schema!(
        schemas,
        "PickerSelectRequest",
        crate::direction::PickerSelectRequest
    );
    collect_schema!(
        schemas,
        "PickerSelectResponse",
        crate::direction::PickerSelectResponse
    );

    // Advance.
    collect_schema!(schemas, "AdvanceResponse", crate::advance::AdvanceResponse);

    // Feedback CRUD.
    collect_schema!(schemas, "FeedbackItem", crate::feedback::FeedbackItem);
    collect_schema!(
        schemas,
        "FeedbackListResponse",
        crate::feedback::FeedbackListResponse
    );
    collect_schema!(
        schemas,
        "FeedbackCreateRequest",
        crate::feedback::FeedbackCreateRequest
    );
    collect_schema!(
        schemas,
        "FeedbackCreateResponse",
        crate::feedback::FeedbackCreateResponse
    );
    collect_schema!(
        schemas,
        "FeedbackUpdateRequest",
        crate::feedback::FeedbackUpdateRequest
    );
    collect_schema!(
        schemas,
        "FeedbackUpdateResponse",
        crate::feedback::FeedbackUpdateResponse
    );
    collect_schema!(
        schemas,
        "FeedbackDeleteResponse",
        crate::feedback::FeedbackDeleteResponse
    );
    collect_schema!(
        schemas,
        "FeedbackReplyCreateRequest",
        crate::feedback::FeedbackReplyCreateRequest
    );
    collect_schema!(
        schemas,
        "FeedbackReplyCreateResponse",
        crate::feedback::FeedbackReplyCreateResponse
    );

    // Runs browse.
    collect_schema!(schemas, "RunSummary", crate::runs::RunSummary);
    collect_schema!(schemas, "RunListPayload", crate::runs::RunListPayload);
    collect_schema!(schemas, "RunDetailPayload", crate::runs::RunDetailPayload);

    // Validation envelope.
    collect_schema!(schemas, "ValidationError", crate::common::ValidationError);

    schemas
}

/// The lowercase OpenAPI verb for an HTTP method, or `None` for the WebSocket
/// pseudo-method (which OpenAPI cannot model as a path operation).
fn openapi_verb(method: HttpMethod) -> Option<&'static str> {
    match method {
        HttpMethod::Get => Some("get"),
        HttpMethod::Head => Some("head"),
        HttpMethod::Post => Some("post"),
        HttpMethod::Put => Some("put"),
        HttpMethod::Delete => Some("delete"),
        HttpMethod::Ws => None,
    }
}

/// Build the `paths` object from the route table.
fn paths_object() -> Map<String, Value> {
    let mut paths: Map<String, Value> = Map::new();

    for route in ROUTES {
        let Some(verb) = openapi_verb(route.method) else {
            continue;
        };

        let operation = json!({
            "operationId": route.operation_id,
            "summary": route.summary,
            "tags": [route.tag],
            "responses": {
                "200": { "description": "Success" }
            }
        });

        let entry = paths
            .entry(route.path_template.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(verb.to_string(), operation);
        }
    }

    paths
}

/// Build the full OpenAPI 3.1 document as a [`serde_json::Value`].
pub fn document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "darkrun API",
            "version": API_VERSION,
            "description": "The darkrun software-factory HTTP + WebSocket wire contract."
        },
        "paths": paths_object(),
        "components": {
            "schemas": component_schemas()
        }
    })
}

/// Render the OpenAPI document as pretty-printed JSON.
///
/// Whole-valued floats are normalized to integers (`0.0` → `0`) before
/// serializing: release-please's JSON updater re-serializes `openapi.json`
/// when it bumps `info.version` and normalizes numbers exactly this way, so
/// the canonical file must be a FIXED POINT of that rewrite or every release
/// PR fails the parity test on `minimum: 0.0` vs `minimum: 0`.
pub fn document_json() -> String {
    let mut doc = document();
    normalize_whole_floats(&mut doc);
    serde_json::to_string_pretty(&doc).expect("openapi document serializes")
}

/// Recursively rewrite any f64 with no fractional part into the equivalent
/// integer JSON number (within i64/u64 range).
fn normalize_whole_floats(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if n.as_i64().is_none() && n.as_u64().is_none() && f.fract() == 0.0 {
                    if f >= 0.0 && f <= u64::MAX as f64 {
                        *n = serde_json::Number::from(f as u64);
                    } else if f >= i64::MIN as f64 && f < 0.0 {
                        *n = serde_json::Number::from(f as i64);
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                normalize_whole_floats(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                normalize_whole_floats(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::HttpMethod;

    #[test]
    fn document_has_core_shape() {
        let doc = document();
        assert_eq!(doc["openapi"], "3.1.0");
        assert_eq!(doc["info"]["title"], "darkrun API");
        assert!(doc["paths"].is_object());
        assert!(doc["components"]["schemas"].is_object());
    }

    #[test]
    fn every_non_ws_route_appears_in_paths() {
        let doc = document();
        let paths = doc["paths"].as_object().expect("paths object");

        for route in ROUTES {
            if route.method == HttpMethod::Ws {
                // WebSocket upgrades are not modeled as OpenAPI operations.
                assert!(
                    !paths
                        .get(route.path_template)
                        .and_then(|p| p.get("get"))
                        .map(|op| op["operationId"] == route.operation_id)
                        .unwrap_or(false),
                    "ws route {} should not be a get operation",
                    route.operation_id
                );
                continue;
            }
            let verb = openapi_verb(route.method).expect("non-ws verb");
            let op = &paths[route.path_template][verb];
            assert_eq!(
                op["operationId"], route.operation_id,
                "route {} missing from openapi paths",
                route.operation_id
            );
            assert_eq!(op["tags"][0], route.tag);
        }
    }

    #[test]
    fn operation_ids_are_unique() {
        let doc = document();
        let paths = doc["paths"].as_object().unwrap();
        let mut ids = Vec::new();
        for (_path, item) in paths {
            for (_verb, op) in item.as_object().unwrap() {
                ids.push(op["operationId"].as_str().unwrap().to_string());
            }
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate operationId in openapi");
    }

    #[test]
    fn core_schemas_are_present() {
        let doc = document();
        let schemas = doc["components"]["schemas"].as_object().unwrap();
        for name in ["SessionPayload", "FeedbackItem", "ReviewDecisionRequest"] {
            assert!(
                schemas.contains_key(name),
                "missing component schema: {name}"
            );
        }
    }

    #[test]
    fn document_is_deterministic() {
        assert_eq!(document_json(), document_json());
    }

    /// The checked-in `openapi.json` must match the freshly generated document.
    /// Regenerate with `cargo run -p darkrun-api --bin emit_openapi` when the
    /// wire types change. This is the openapi-parity guard.
    #[test]
    fn openapi_json_is_in_sync() {
        let committed = include_str!("../openapi.json");
        let mut expected = document_json();
        expected.push('\n');
        assert_eq!(
            committed, expected,
            "openapi.json is stale — run `cargo run -p darkrun-api --bin emit_openapi`"
        );
    }
}

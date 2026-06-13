//! HTTP request handlers.
//!
//! The project-specific domain handlers, built around the factory vocabulary
//! and the `.darkrun` state layout:
//!   - `GET    /health`                              — readiness probe.
//!   - `GET    /api/session/:id`                     — interactive session JSON.
//!   - `HEAD   /api/session/:id/heartbeat`           — client-presence ping.
//!   - `POST   /review/:id/decide`                   — record a review decision.
//!   - `POST   /visual-review/:id/annotate`          — annotate an output -> feedback.
//!   - `POST   /api/proof/:run`                       — attach a run's proof.
//!   - `GET    /api/proof/:run`                       — read a run's proof.
//!   - `POST   /api/advance/:id`                     — SPA wake signal past a gate.
//!   - `GET    /api/feedback/:run/:station`          — list feedback for a station.
//!   - `POST   /api/feedback/:run/:station`          — create a feedback item.
//!   - `PUT    /api/feedback/:run/:station/:id`      — update status / closed_by.
//!   - `DELETE /api/feedback/:run/:station/:id`      — delete (409 if still open).
//!   - `POST   /api/feedback/:run/:station/:id/replies` — append a reply.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use darkrun_api::{
    DirectionSelectRequest, DirectionSelectResponse, FeedbackCreateRequest, FeedbackCreateResponse,
    FeedbackDeleteResponse, FeedbackItem, FeedbackListResponse, FeedbackReplyCreateRequest,
    FeedbackReplyCreateResponse, FeedbackStatus, FeedbackUpdateRequest, FeedbackUpdateResponse,
    OutputReviewRequest, OutputReviewResponse, PickerSelectRequest, PickerSelectResponse,
    ProofAttachRequest, ProofAttachResponse, ProofGetResponse, QuestionAnswerRequest,
    QuestionAnswerResponse, ReviewDecision, ReviewDecisionRequest, ReviewDecisionResponse,
    SessionPayload, SessionStatus,
};
use serde_json::json;

use crate::feedback_doc::{self, FeedbackDoc};
use crate::state::AppState;

/// `GET /health` — liveness/readiness probe. Always `200 ok` once the router
/// is serving (the server only mounts routes after binding succeeds, so a
/// reachable `/health` already implies readiness).
pub async fn health() -> Response {
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

/// `GET /api/runs/:slug/asset/*path` — serve a file from the run's
/// `.darkrun/<slug>/assets/` directory (the mockups/screenshots an agent
/// generated for a visual question or design direction). The desktop webview
/// is served over a custom protocol and cannot load `file://` paths, so it
/// rewrites those into this HTTP route.
///
/// Path-safety: the joined path is lexically resolved and MUST stay within the
/// run's assets dir — any `..` that would escape it is a `403`. `404` for a
/// missing file. Read-only; only the assets subtree is reachable.
pub async fn get_run_asset(
    State(state): State<AppState>,
    Path((slug, rest)): Path<(String, String)>,
) -> Response {
    use std::path::{Component, PathBuf};

    let assets_root = state.store.run_dir(&slug).join("assets");
    // Lexically resolve the requested sub-path; reject any escape.
    let mut safe = PathBuf::new();
    for comp in PathBuf::from(&rest).components() {
        match comp {
            Component::Normal(c) => safe.push(c),
            Component::CurDir => {}
            // Anything that could climb out (ParentDir, RootDir, Prefix) is a
            // traversal attempt.
            _ => return (StatusCode::FORBIDDEN, "invalid asset path").into_response(),
        }
    }
    let path = assets_root.join(&safe);
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return not_found("asset", &rest),
    };
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    };
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, mime)],
        bytes,
    )
        .into_response()
}

/// `GET /api/session/:id` — return the interactive session payload as JSON for
/// the desktop app to render. `404` when no such session is registered.
pub async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // A miss may still name something real (a run slug whose show session
    // hasn't been pushed yet) — materialize on demand so the desktop can open
    // a run without waiting for the engine to tick first.
    state.ensure_session(&id);
    match state.sessions.get(&id) {
        Some(payload) => (StatusCode::OK, Json(payload)).into_response(),
        None => not_found("session", &id),
    }
}

/// `HEAD /api/session/:id/heartbeat` — client presence ping. `200` if the
/// session exists, `404` otherwise. No body either way (it is a HEAD route).
pub async fn session_heartbeat(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    if state.ensure_session(&id) {
        StatusCode::OK.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// `POST /review/:id/decide` — record a review decision against a registered
/// review session. The raw `decision` string is canonicalized server-side:
/// only `approved` (case-insensitive) yields [`ReviewDecision::Approved`]. The
/// session's payload is updated in place and pushed to any WebSocket subscriber.
pub async fn review_decide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ReviewDecisionRequest>,
) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    let SessionPayload::Review(mut review) = payload else {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "session is not a review session", "session_id": id })),
        )
            .into_response();
    };

    let decision = ReviewDecision::canonicalize(&req.decision);
    let feedback = req.feedback.clone().unwrap_or_default();

    // Reflect the decision back onto the session payload and re-register it so
    // subscribers see the resolved state.
    review.decision = Some(
        match decision {
            ReviewDecision::Approved => "approved",
            ReviewDecision::ChangesRequested => "changes_requested",
        }
        .to_string(),
    );
    review.feedback = req.feedback;
    review.annotations = req.annotations;
    review.status = match decision {
        ReviewDecision::Approved => SessionStatus::Approved,
        ReviewDecision::ChangesRequested => SessionStatus::ChangesRequested,
    };
    state.sessions.upsert(SessionPayload::Review(review));

    (
        StatusCode::OK,
        Json(ReviewDecisionResponse {
            ok: true,
            decision,
            feedback,
        }),
    )
        .into_response()
}

/// `POST /api/advance/:id` — SPA wake signal. No body. Marks the review session
/// resolved (the engine, on its next tick, walks the cursor past the user gate)
/// and notifies subscribers. `404` when the session is unknown.
pub async fn advance(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    if let SessionPayload::Review(mut review) = payload {
        review.status = SessionStatus::Decided;
        state.sessions.upsert(SessionPayload::Review(review));
    }
    (StatusCode::OK, Json(json!({ "ok": true, "advanced": true }))).into_response()
}

/// `POST /api/unit/:run/:unit/reset` — request a reset of a wedged unit from the
/// desktop review UI. Sets the unit's `reset_requested` flag on disk; the engine
/// performs the actual reset on its next tick (clearing the unit's execution
/// state back to `Pending` so its locked body unlocks and it re-runs from Pass 1).
/// This is the non-MCP, revise-style path to the `darkrun_unit_reset` capability.
/// 404 if the unit is unknown.
pub async fn request_unit_reset(
    State(state): State<AppState>,
    Path((run, unit)): Path<(String, String)>,
) -> Response {
    let store = &state.store;
    let Ok(mut u) = store.read_unit(&run, &unit) else {
        return not_found("unit", &unit);
    };
    if u.frontmatter.reset_requested {
        // Idempotent: a reset is already pending for this unit.
        return (
            StatusCode::OK,
            Json(json!({ "ok": true, "run": run, "unit": unit, "reset_requested": true, "note": "already requested" })),
        )
            .into_response();
    }
    u.frontmatter.reset_requested = true;
    if store.write_unit(&run, &u).is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": "failed to write unit" })),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        Json(json!({
            "ok": true, "run": run, "unit": unit, "reset_requested": true,
            "note": "reset requested — the engine resets this unit to pending on its next tick"
        })),
    )
        .into_response()
}

/// `POST /question/:id/answer` — record the operator's answer to a VISUAL
/// QUESTION session.
///
/// Stores the selected option ids (and optional free text) onto the session's
/// `answer` field, flips the session to `answered`, and pushes the resolved
/// payload to any WebSocket subscriber. `404` when the session is unknown,
/// `409` when the session under that id is not a question session.
pub async fn question_answer(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<QuestionAnswerRequest>,
) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    let SessionPayload::Question(mut question) = payload else {
        return conflict("session is not a question session", &id);
    };

    let answer = req.to_answer();
    question.answer = Some(answer.clone());
    question.status = SessionStatus::Answered;
    state.sessions.upsert(SessionPayload::Question(question));

    (
        StatusCode::OK,
        Json(QuestionAnswerResponse { ok: true, answer }),
    )
        .into_response()
}

/// `POST /direction/:id/select` — record the operator's design DIRECTION: the
/// chosen archetype id plus optional annotations.
///
/// Validates the chosen archetype against the session's `archetypes[].id`
/// (`422` when it names an unknown archetype), records the choice + annotations,
/// flips the session to `decided`, and pushes the update. `404`/`409` mirror
/// the question handler.
pub async fn direction_select(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<DirectionSelectRequest>,
) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    let SessionPayload::Direction(mut direction) = payload else {
        return conflict("session is not a direction session", &id);
    };

    // The chosen archetype must exist among the offered ones (when any were
    // offered). An empty archetype list means the decision is unconstrained.
    if !direction.archetypes.is_empty()
        && !direction.archetypes.iter().any(|a| a.id == req.archetype)
    {
        return unprocessable("unknown archetype id", &req.archetype);
    }

    direction.chosen_archetype = Some(req.archetype.clone());
    direction.annotations = req.annotations;
    direction.status = SessionStatus::Decided;
    state.sessions.upsert(SessionPayload::Direction(direction));

    (
        StatusCode::OK,
        Json(DirectionSelectResponse {
            ok: true,
            archetype: req.archetype,
        }),
    )
        .into_response()
}

/// `POST /picker/:id/select` — choose an option in a blocking picker session.
///
/// Validates the option id against the session's `options[].id` (`422` on an
/// unknown id), records the selection, flips the session to `decided`, and
/// pushes the update. `404`/`409` mirror the other session handlers.
pub async fn picker_select(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PickerSelectRequest>,
) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    let SessionPayload::Picker(mut picker) = payload else {
        return conflict("session is not a picker session", &id);
    };

    if !picker.options.iter().any(|o| o.id == req.id) {
        return unprocessable("unknown option id", &req.id);
    }

    picker.selection = Some(darkrun_api::PickerSelection { id: req.id.clone() });
    picker.status = SessionStatus::Decided;
    state.sessions.upsert(SessionPayload::Picker(picker));

    (
        StatusCode::OK,
        Json(PickerSelectResponse {
            ok: true,
            id: req.id,
        }),
    )
        .into_response()
}

/// `POST /visual-review/:id/annotate` — record the operator's VISUAL REVIEW of
/// an OUTPUT screenshot and emit it as FEEDBACK.
///
/// Records the pin + comment annotations onto the visual-review session (WS
/// push), then mints a `user-visual` feedback item against the run/station the
/// session targets so the fix-worker loop can act on it. `404` when the session
/// is unknown, `409` when it is not a visual-review session, `422` when the
/// annotation carries neither a pin nor a comment (nothing to act on).
pub async fn visual_review_annotate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<OutputReviewRequest>,
) -> Response {
    let Some(payload) = state.sessions.get(&id) else {
        return not_found("session", &id);
    };
    let SessionPayload::VisualReview(mut review) = payload else {
        return conflict("session is not a visual-review session", &id);
    };

    if req.annotations.is_empty() {
        return unprocessable("annotation carries no pins or comments", &id);
    }

    let pins = req.annotations.pins.len();
    let comments = req.annotations.comments.len();

    // The feedback targets the run + station the session names. A visual review
    // with no run slug cannot be routed to a feedback file.
    let Some(run) = review.run_slug.clone() else {
        return unprocessable("visual-review session has no run_slug", &id);
    };
    let station = review.station.clone().unwrap_or_default();

    let existing = state.store.read_feedback_raw(&run).unwrap_or_default();
    let fb_id = feedback_doc::next_id(existing.keys());
    let title = req.title.clone().unwrap_or_else(|| {
        match review.artifact_path.as_deref() {
            Some(path) => format!("Visual review: {path}"),
            None => "Visual review of output".to_string(),
        }
    });
    let doc = FeedbackDoc::new_user(fb_id.clone(), station.clone(), title, req.to_feedback_body());
    if state
        .store
        .write_feedback_raw(&run, &fb_id, &doc.render())
        .is_err()
    {
        return internal_error("failed to persist visual-review feedback");
    }

    // Record the annotations back on the session + flip to decided so any
    // WebSocket subscriber sees the resolved review.
    review.annotations = Some(req.annotations);
    review.status = SessionStatus::Decided;
    state.sessions.upsert(SessionPayload::VisualReview(review));

    (
        StatusCode::CREATED,
        Json(OutputReviewResponse {
            ok: true,
            feedback_id: fb_id,
            pins,
            comments,
        }),
    )
        .into_response()
}

/// `POST /api/proof/:run` — attach a run's objective-evidence [`Proof`].
///
/// Stores the proof in the in-memory proof registry, keyed by run. `422` when
/// the proof's populated block does not match its surface's verification route
/// (e.g. a `web_ui` proof carrying only a bench block).
pub async fn attach_proof(
    State(state): State<AppState>,
    Path(run): Path<String>,
    Json(req): Json<ProofAttachRequest>,
) -> Response {
    let matches = req.proof.block_matches_surface();
    if !matches {
        return unprocessable("proof block does not match surface", req.proof.surface.as_str());
    }
    let surface = req.proof.surface;
    state.proofs.attach(&run, req.proof, req.station.clone());

    (
        StatusCode::CREATED,
        Json(ProofAttachResponse {
            ok: true,
            run,
            surface,
            block_matches_surface: matches,
        }),
    )
        .into_response()
}

/// `GET /api/proof/:run` — return a run's attached objective-evidence proof.
/// `404` when no proof has been attached for the run.
pub async fn get_proof(State(state): State<AppState>, Path(run): Path<String>) -> Response {
    match state.proofs.get(&run) {
        Some((proof, station)) => (
            StatusCode::OK,
            Json(ProofGetResponse { run, station, proof }),
        )
            .into_response(),
        None => not_found("proof", &run),
    }
}

/// `GET /api/feedback/:run/:station` — list feedback items for a run's station.
///
/// Reads the run's feedback sidecar files off `.darkrun/` and returns the
/// parsed items filtered to the requested station. Items with no recorded
/// station are treated as belonging to every station (legacy-tolerant).
pub async fn list_feedback(
    State(state): State<AppState>,
    Path((run, station)): Path<(String, String)>,
) -> Response {
    let raw = state.store.read_feedback_raw(&run).unwrap_or_default();
    let mut items: Vec<FeedbackItem> = raw
        .into_iter()
        .map(|(id, content)| FeedbackDoc::parse(&id, &content))
        .filter(|doc| doc.matches_station(&station))
        .map(|doc| doc.to_item())
        .collect();
    items.sort_by(|a, b| a.feedback_id.cmp(&b.feedback_id));

    (
        StatusCode::OK,
        Json(FeedbackListResponse {
            run,
            station,
            count: items.len(),
            items,
        }),
    )
        .into_response()
}

/// `POST /api/feedback/:run/:station` — create a new feedback item.
///
/// Mints the next `FB-NN` id for the run, stamps `user` as the author
/// (client-supplied author is ignored for the HTTP trust boundary), and writes
/// the markdown-with-frontmatter sidecar. `201` on success, `400` on an empty
/// title or body.
pub async fn create_feedback(
    State(state): State<AppState>,
    Path((run, station)): Path<(String, String)>,
    Json(req): Json<FeedbackCreateRequest>,
) -> Response {
    if req.title.trim().is_empty() || req.body.trim().is_empty() {
        return bad_request("title and body are required");
    }

    let existing = state.store.read_feedback_raw(&run).unwrap_or_default();
    let id = feedback_doc::next_id(existing.keys());

    // `FeedbackDoc::new_user` always stamps `user` as the author: any
    // client-supplied author crosses the trust boundary and is discarded.
    let doc = FeedbackDoc::new_user(
        id.clone(),
        station.clone(),
        req.title.trim().to_string(),
        req.body.trim().to_string(),
    );

    if state
        .store
        .write_feedback_raw(&run, &id, &doc.render())
        .is_err()
    {
        return internal_error("failed to persist feedback");
    }

    (
        StatusCode::CREATED,
        Json(FeedbackCreateResponse {
            feedback_id: id.clone(),
            file: format!("feedback/{id}.md"),
            status: FeedbackStatus::Pending,
            message: format!("created {id}"),
        }),
    )
        .into_response()
}

/// `PUT /api/feedback/:run/:station/:id` — update status / closed_by.
///
/// At least one mutating field must be present (`400` otherwise). `404` when
/// the item does not exist. Returns the list of fields actually changed.
pub async fn update_feedback(
    State(state): State<AppState>,
    Path((run, _station, id)): Path<(String, String, String)>,
    Json(req): Json<FeedbackUpdateRequest>,
) -> Response {
    if req.is_empty() {
        return bad_request("at least one of 'status' / 'closed_by' / 'resolution' must be provided");
    }

    let Some(content) = state
        .store
        .read_feedback_raw(&run)
        .ok()
        .and_then(|mut m| m.remove(&id))
    else {
        return not_found("feedback", &id);
    };

    let mut doc = FeedbackDoc::parse(&id, &content);
    let mut updated = Vec::new();
    if let Some(status) = req.status {
        doc.status = status;
        updated.push("status".to_string());
    }
    if let Some(closed_by) = &req.closed_by {
        doc.closed_by = Some(closed_by.clone());
        updated.push("closed_by".to_string());
    }
    // `resolution` is part of the wire contract but not persisted in the flat
    // frontmatter the engine reads; acknowledge it as a changed field so the
    // SPA's optimistic update reconciles, without inventing on-disk state.
    if req.resolution.is_some() {
        updated.push("resolution".to_string());
    }

    if state
        .store
        .write_feedback_raw(&run, &id, &doc.render())
        .is_err()
    {
        return internal_error("failed to persist feedback");
    }

    (
        StatusCode::OK,
        Json(FeedbackUpdateResponse {
            feedback_id: id.clone(),
            updated_fields: updated,
            message: format!("updated {id}"),
        }),
    )
        .into_response()
}

/// `DELETE /api/feedback/:run/:station/:id` — remove a feedback item.
///
/// Refuses to delete an item that is still `open`/`pending` (`409`) so the
/// fix-worker loop can't lose a live finding out from under it. `404` when the
/// item is unknown.
pub async fn delete_feedback(
    State(state): State<AppState>,
    Path((run, _station, id)): Path<(String, String, String)>,
) -> Response {
    let Some(content) = state
        .store
        .read_feedback_raw(&run)
        .ok()
        .and_then(|mut m| m.remove(&id))
    else {
        return not_found("feedback", &id);
    };

    let doc = FeedbackDoc::parse(&id, &content);
    // Refuse to delete a finding the fix-worker loop still holds the gate on.
    if doc.status.blocks_gate() {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "cannot delete an open feedback item",
                "feedback_id": id,
                "status": doc.status,
            })),
        )
            .into_response();
    }

    let path = state.store.feedback_dir(&run).join(format!("{id}.md"));
    if std::fs::remove_file(&path).is_err() {
        return internal_error("failed to delete feedback");
    }

    (
        StatusCode::OK,
        Json(FeedbackDeleteResponse {
            feedback_id: id.clone(),
            deleted: true,
            message: format!("deleted {id}"),
        }),
    )
        .into_response()
}

/// `POST /api/feedback/:run/:station/:id/replies` — append a reply.
///
/// Optionally transitions the parent to `answered` when `close_as_answered` is
/// set. `400` on an empty body, `404` when the parent is unknown.
pub async fn create_feedback_reply(
    State(state): State<AppState>,
    Path((run, _station, id)): Path<(String, String, String)>,
    Json(req): Json<FeedbackReplyCreateRequest>,
) -> Response {
    if req.body.trim().is_empty() {
        return bad_request("reply body is required");
    }

    let Some(content) = state
        .store
        .read_feedback_raw(&run)
        .ok()
        .and_then(|mut m| m.remove(&id))
    else {
        return not_found("feedback", &id);
    };

    let mut doc = FeedbackDoc::parse(&id, &content);
    let author = req
        .author
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .unwrap_or("user")
        .to_string();
    doc.replies.push(format!("{author}: {}", req.body.trim()));
    let reply_index = doc.replies.len() - 1;

    if req.close_as_answered.unwrap_or(false) {
        doc.status = FeedbackStatus::Answered;
    }

    if state
        .store
        .write_feedback_raw(&run, &id, &doc.render())
        .is_err()
    {
        return internal_error("failed to persist reply");
    }

    (
        StatusCode::CREATED,
        Json(FeedbackReplyCreateResponse {
            feedback_id: id.clone(),
            reply_index,
            status: doc.status,
            message: format!("reply added to {id}"),
        }),
    )
        .into_response()
}

/// Build a uniform `404` JSON envelope.
fn not_found(kind: &str, id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("{kind} not found"), "id": id })),
    )
        .into_response()
}

/// Build a uniform `400` JSON envelope.
fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
}

/// Build a uniform `409` JSON envelope for a session-type mismatch.
fn conflict(msg: &str, id: &str) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": msg, "session_id": id })),
    )
        .into_response()
}

/// Build a uniform `422` JSON envelope for a semantically-invalid selection
/// (e.g. an archetype/option id that doesn't exist on the session).
fn unprocessable(msg: &str, value: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": msg, "value": value })),
    )
        .into_response()
}

/// Build a uniform `500` JSON envelope.
fn internal_error(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
        .into_response()
}

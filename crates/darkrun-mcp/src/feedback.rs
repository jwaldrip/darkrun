//! Typed feedback layer over the core raw-feedback store.
//!
//! `darkrun-core` keeps feedback opaque (a `feedback/*.md` doc with a YAML
//! frontmatter fence over a markdown body). This module gives the MCP tools a
//! typed read/write surface for those documents — create, list, resolve,
//! move, and reject — built on
//! [`StateStore::read_feedback_raw`](darkrun_core::StateStore::read_feedback_raw)
//! and [`write_feedback_raw`](darkrun_core::StateStore::write_feedback_raw).
//!
//! The on-disk shape is intentionally simple YAML frontmatter so that the
//! manager's `feedback_open` walk (in [`crate::position`]) and these tools
//! agree on the same `status:` line.

use chrono::Utc;
use darkrun_core::domain::{
    ClosureReply, Feedback, FeedbackOrigin, FeedbackSeverity, FeedbackStatus,
};
use darkrun_core::StateStore;

use crate::error::{McpError, Result};

/// The terminal statuses — a feedback item in one of these is "settled" and
/// the manager's feedback walk will stop re-dispatching it.
pub const TERMINAL: &[FeedbackStatus] = &[
    FeedbackStatus::Addressed,
    FeedbackStatus::Answered,
    FeedbackStatus::NonActionable,
    FeedbackStatus::Closed,
    FeedbackStatus::Rejected,
];

/// Whether a status is terminal (settled).
pub fn is_terminal(status: FeedbackStatus) -> bool {
    TERMINAL.contains(&status)
}

fn status_str(status: FeedbackStatus) -> &'static str {
    match status {
        FeedbackStatus::Pending => "pending",
        FeedbackStatus::Fixing => "fixing",
        FeedbackStatus::Addressed => "addressed",
        FeedbackStatus::Answered => "answered",
        FeedbackStatus::NonActionable => "non_actionable",
        FeedbackStatus::Escalated => "escalated",
        FeedbackStatus::Closed => "closed",
        FeedbackStatus::Rejected => "rejected",
    }
}

pub fn parse_status(raw: &str) -> Option<FeedbackStatus> {
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "pending" => Some(FeedbackStatus::Pending),
        "fixing" => Some(FeedbackStatus::Fixing),
        "addressed" => Some(FeedbackStatus::Addressed),
        "answered" => Some(FeedbackStatus::Answered),
        "non_actionable" | "nonactionable" => Some(FeedbackStatus::NonActionable),
        "escalated" => Some(FeedbackStatus::Escalated),
        "closed" => Some(FeedbackStatus::Closed),
        "rejected" => Some(FeedbackStatus::Rejected),
        _ => None,
    }
}

fn severity_str(severity: FeedbackSeverity) -> &'static str {
    match severity {
        FeedbackSeverity::Blocker => "blocker",
        FeedbackSeverity::High => "high",
        FeedbackSeverity::Medium => "medium",
        FeedbackSeverity::Low => "low",
    }
}

/// Parse a severity name, accepting the canonical snake_case form.
pub fn parse_severity(raw: &str) -> Option<FeedbackSeverity> {
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "blocker" => Some(FeedbackSeverity::Blocker),
        "high" => Some(FeedbackSeverity::High),
        "medium" => Some(FeedbackSeverity::Medium),
        "low" => Some(FeedbackSeverity::Low),
        _ => None,
    }
}

fn origin_str(origin: FeedbackOrigin) -> &'static str {
    match origin {
        FeedbackOrigin::AdversarialReview => "adversarial_review",
        FeedbackOrigin::RunReview => "run_review",
        FeedbackOrigin::Reflection => "reflection",
        FeedbackOrigin::Discovery => "discovery",
        FeedbackOrigin::Drift => "drift",
        FeedbackOrigin::Operator => "operator",
        FeedbackOrigin::Annotation => "annotation",
        FeedbackOrigin::External => "external",
        FeedbackOrigin::Unspecified => "unspecified",
    }
}

/// Parse a feedback origin token; unknown tokens fall back to `Unspecified`.
pub fn parse_origin(raw: &str) -> FeedbackOrigin {
    match raw.trim().trim_matches('"').to_ascii_lowercase().replace('-', "_").as_str() {
        "adversarial_review" | "review" => FeedbackOrigin::AdversarialReview,
        "run_review" => FeedbackOrigin::RunReview,
        "reflection" => FeedbackOrigin::Reflection,
        "discovery" => FeedbackOrigin::Discovery,
        "drift" => FeedbackOrigin::Drift,
        "operator" => FeedbackOrigin::Operator,
        "annotation" => FeedbackOrigin::Annotation,
        "external" => FeedbackOrigin::External,
        _ => FeedbackOrigin::Unspecified,
    }
}

/// Parse a `[a, b, c]` inline list of role slugs from a frontmatter value.
fn parse_inline_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Serialize a feedback item to its on-disk `feedback/*.md` shape.
fn serialize(fb: &Feedback) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("status: {}\n", status_str(fb.status)));
    out.push_str(&format!("station: {}\n", fb.station));
    if let Some(sev) = fb.severity {
        out.push_str(&format!("severity: {}\n", severity_str(sev)));
    }
    if !matches!(fb.origin, FeedbackOrigin::Unspecified) {
        out.push_str(&format!("origin: {}\n", origin_str(fb.origin)));
    }
    if !fb.invalidates.is_empty() {
        out.push_str(&format!("invalidates: [{}]\n", fb.invalidates.join(", ")));
    }
    if let Some(created) = &fb.created_at {
        out.push_str(&format!("created_at: {created}\n"));
    }
    if let Some(reply) = &fb.closure_reply {
        // Hand-rolled YAML: keep the reply single-line (newlines collapse to
        // spaces) and quoted so it survives the fence round-trip.
        let text = reply.text.replace('\n', " ").replace('"', "'");
        out.push_str(&format!("closure_reply: \"{text}\"\n"));
        if let Some(at) = &reply.at {
            out.push_str(&format!("closure_reply_at: {at}\n"));
        }
    }
    out.push_str("---\n");
    out.push_str(&fb.body);
    if !fb.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Parse a raw `feedback/*.md` document into a typed [`Feedback`].
///
/// Tolerant: a missing `status:` line is treated as `pending`, an absent
/// frontmatter fence still yields a body-only pending item.
fn parse(run: &str, id: &str, raw: &str) -> Feedback {
    let mut status = FeedbackStatus::Pending;
    let mut station = String::new();
    let mut severity = None;
    let mut origin = FeedbackOrigin::Unspecified;
    let mut invalidates = Vec::new();
    let mut created_at = None;
    let mut closure_text: Option<String> = None;
    let mut closure_at: Option<String> = None;

    // Split frontmatter (between the first two `---` fences) from the body.
    let (front, body) = split_frontmatter(raw);
    for line in front.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("status:") {
            if let Some(s) = parse_status(rest) {
                status = s;
            }
        } else if let Some(rest) = line.strip_prefix("station:") {
            station = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("severity:") {
            severity = parse_severity(rest);
        } else if let Some(rest) = line.strip_prefix("origin:") {
            origin = parse_origin(rest);
        } else if let Some(rest) = line.strip_prefix("invalidates:") {
            invalidates = parse_inline_list(rest);
        } else if let Some(rest) = line.strip_prefix("closure_reply_at:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                closure_at = Some(v);
            }
        } else if let Some(rest) = line.strip_prefix("closure_reply:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                closure_text = Some(v);
            }
        } else if let Some(rest) = line.strip_prefix("created_at:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                created_at = Some(v);
            }
        }
    }

    Feedback {
        id: id.to_string(),
        run: run.to_string(),
        station,
        status,
        severity,
        origin,
        body: body.trim_start_matches('\n').to_string(),
        created_at,
        invalidates,
        closure_reply: closure_text.map(|text| ClosureReply { text, at: closure_at }),
    }
}

/// Split a frontmatter document into `(frontmatter, body)`. When there is no
/// leading `---` fence the whole input is the body.
fn split_frontmatter(raw: &str) -> (String, String) {
    let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    if let Some(rest) = trimmed.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let front = rest[..end].to_string();
            let after = &rest[end + 4..];
            let body = after.strip_prefix('\n').unwrap_or(after).to_string();
            return (front, body);
        }
    }
    (String::new(), trimmed.to_string())
}

/// List every feedback item for a run, parsed and sorted by id.
pub fn list(store: &StateStore, run: &str) -> Result<Vec<Feedback>> {
    let raw = store.read_feedback_raw(run)?;
    Ok(raw
        .into_iter()
        .map(|(id, content)| parse(run, &id, &content))
        .collect())
}

/// Read a single feedback item by id.
pub fn get(store: &StateStore, run: &str, id: &str) -> Result<Feedback> {
    let raw = store.read_feedback_raw(run)?;
    raw.get(id)
        .map(|content| parse(run, id, content))
        .ok_or_else(|| McpError::FeedbackNotFound(id.to_string()))
}

/// Allocate the next free `fb-NN` id for a run.
fn next_id(store: &StateStore, run: &str) -> Result<String> {
    let raw = store.read_feedback_raw(run)?;
    let mut max = 0u32;
    for id in raw.keys() {
        if let Some(num) = id.strip_prefix("fb-") {
            if let Ok(n) = num.parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    Ok(format!("fb-{:02}", max + 1))
}

/// Create a new feedback item, returning the persisted record.
pub fn create(
    store: &StateStore,
    run: &str,
    station: &str,
    body: &str,
    severity: Option<FeedbackSeverity>,
) -> Result<Feedback> {
    create_with_origin(store, run, station, body, severity, FeedbackOrigin::Unspecified, vec![])
}

/// Create a feedback item, recording its `origin` (where the finding came from)
/// and the review/approval roles it `invalidates` on close.
#[allow(clippy::too_many_arguments)]
pub fn create_with_origin(
    store: &StateStore,
    run: &str,
    station: &str,
    body: &str,
    severity: Option<FeedbackSeverity>,
    origin: FeedbackOrigin,
    invalidates: Vec<String>,
) -> Result<Feedback> {
    if body.trim().is_empty() {
        return Err(McpError::InvalidInput("feedback body must not be empty".into()));
    }
    let id = next_id(store, run)?;
    let fb = Feedback {
        id: id.clone(),
        run: run.to_string(),
        station: station.to_string(),
        status: FeedbackStatus::Pending,
        severity,
        origin,
        body: body.to_string(),
        created_at: Some(Utc::now().to_rfc3339()),
        invalidates,
        closure_reply: None,
    };
    store.write_feedback_raw(run, &id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(fb)
}

/// Mint feedback from a remote review note (C6), with a **deterministic** id
/// derived from the provider note id (`fb-ext-<sanitized>`) so re-polling the
/// same PR never double-files. Returns `Ok(None)` when a feedback with that id
/// already exists (already ingested) or the body is empty.
///
/// A change request files as a `Blocker` (it gates the merge); a plain comment
/// as `Medium`. Origin is `External`, with no `invalidates` — the human IS the
/// external reviewer, so there's no internal stamp to re-sign; the fix track just
/// addresses the note and closes it.
pub fn create_external(
    store: &StateStore,
    run: &str,
    station: &str,
    external_id: &str,
    author: &str,
    body: &str,
    change_request: bool,
) -> Result<Option<Feedback>> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    let id = external_feedback_id(external_id);
    // Dedup: this note already became feedback on an earlier poll → no-op.
    if store.read_feedback_raw(run)?.contains_key(&id) {
        return Ok(None);
    }
    let severity = if change_request {
        FeedbackSeverity::Blocker
    } else {
        FeedbackSeverity::Medium
    };
    let verb = if change_request { "requested changes" } else { "commented" };
    let full_body = format!("**@{author}** {verb} on the change request:\n\n{}", body.trim());
    let fb = Feedback {
        id: id.clone(),
        run: run.to_string(),
        station: station.to_string(),
        status: FeedbackStatus::Pending,
        severity: Some(severity),
        origin: FeedbackOrigin::External,
        body: full_body,
        created_at: Some(Utc::now().to_rfc3339()),
        invalidates: vec![],
        closure_reply: None,
    };
    store.write_feedback_raw(run, &id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(Some(fb))
}

/// The deterministic feedback id for a remote review note: `fb-ext-<sanitized>`,
/// where the provider id is reduced to a filesystem-safe `[A-Za-z0-9_-]` slug so
/// it can be the `feedback/<id>.md` filename and the dedup key in one.
fn external_feedback_id(external_id: &str) -> String {
    let safe: String = external_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    format!("fb-ext-{safe}")
}

/// Close a feedback item with a resolution reply, and **invalidate** the stamps
/// the finding undercut: every role named in `invalidates` is cleared from each
/// of the target station's units so the gate re-fires and re-signs against the
/// fixed work. This is the loop's self-correction — a closed finding doesn't
/// just flip a status, it re-opens exactly the reviews/approvals it invalidated.
pub fn close_with_reply(
    store: &StateStore,
    run: &str,
    id: &str,
    reply: &str,
) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.status = FeedbackStatus::Closed;
    if !reply.trim().is_empty() {
        fb.closure_reply = Some(ClosureReply {
            text: reply.trim().to_string(),
            at: Some(Utc::now().to_rfc3339()),
        });
    }
    // Clear the invalidated stamps on the target station's units.
    if !fb.invalidates.is_empty() {
        let units = store.read_units(run).unwrap_or_default();
        for mut unit in units {
            if unit.station() != fb.station {
                continue;
            }
            let mut changed = false;
            for role in &fb.invalidates {
                if unit.frontmatter.reviews.remove(role).is_some() {
                    changed = true;
                }
                if unit.frontmatter.approvals.remove(role).is_some() {
                    changed = true;
                }
            }
            if changed {
                store.write_unit(run, &unit)?;
            }
        }
    }
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    // B9: the fix is resolved — land its isolation worktree (if any) back onto
    // the station branch and retire it. Idempotent + best-effort: a feedback that
    // never forked a fix worktree (non-git run, or resolved without code) no-ops.
    crate::lifecycle::land_fix(store, run, &fb.station, id);
    Ok(fb)
}

/// Transition a feedback item to a new status, returning the updated record.
///
/// Settled (terminal) items are immutable — attempting to re-transition one
/// errors. This mirrors the predecessor's "closed/rejected are immutable"
/// rule and keeps the manager's open-feedback walk monotone.
pub fn set_status(
    store: &StateStore,
    run: &str,
    id: &str,
    status: FeedbackStatus,
) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.status = status;
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    // Mid-loop fix durability (the predecessor's fix-chain checkpoint): while a
    // fix is being WORKED, commit + push its worktree branch so the in-flight
    // repair survives a restart / cross-machine pickup. Best-effort no-op when
    // the fix has no worktree (filesystem mode / run-scope inline fix).
    if matches!(fb.status, FeedbackStatus::Fixing) {
        let root = crate::position::cascade_repo_root(store);
        let wt = crate::lifecycle::fix_worktree_path(&root, run, &fb.station, id);
        let branch = crate::lifecycle::fix_branch(run, &fb.station, id);
        crate::commit::checkpoint_worktree(
            store,
            &wt,
            &branch,
            &format!("darkrun: checkpoint fix {id}"),
        );
    }
    Ok(fb)
}

/// Reject a feedback item: stamp it `rejected` and append the reason to the
/// body. Rejection is a terminal transition, so the manager stops
/// re-dispatching it on the next tick.
pub fn reject(store: &StateStore, run: &str, id: &str, reason: &str) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.status = FeedbackStatus::Rejected;
    if !reason.trim().is_empty() {
        if !fb.body.ends_with('\n') {
            fb.body.push('\n');
        }
        fb.body.push_str(&format!("\n---\nRejected: {reason}\n"));
    }
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(fb)
}

/// Set (or re-target) the station a feedback item belongs to — triage
/// placement. Settled items are immutable.
pub fn move_station(store: &StateStore, run: &str, id: &str, to_station: &str) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.station = to_station.to_string();
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(fb)
}

/// Set the review/approval roles a feedback `invalidates` on close — the
/// materiality classification. A drift feedback files with none; the agent
/// names the signed slots the change actually undercut, so closing it re-opens
/// exactly those stamps and the work re-signs against the new premise (a
/// *material* change re-orients; a *cosmetic* one is closed with none set).
/// Settled items are immutable.
pub fn set_targets(
    store: &StateStore,
    run: &str,
    id: &str,
    invalidates: Vec<String>,
) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.invalidates = invalidates;
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(fb)
}

/// Set the severity ranking of a feedback item. Settled items are immutable.
pub fn set_severity(
    store: &StateStore,
    run: &str,
    id: &str,
    severity: FeedbackSeverity,
) -> Result<Feedback> {
    let mut fb = get(store, run, id)?;
    if is_terminal(fb.status) {
        return Err(McpError::FeedbackSettled(id.to_string()));
    }
    fb.severity = Some(severity);
    store.write_feedback_raw(run, id, &serialize(&fb))?;
    let _ = crate::commit::commit_state(store, "darkrun: feedback update");
    Ok(fb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn origin_str_maps_every_origin_including_unspecified() {
        assert_eq!(origin_str(FeedbackOrigin::Unspecified), "unspecified");
        assert_eq!(origin_str(FeedbackOrigin::Drift), "drift");
        assert_eq!(origin_str(FeedbackOrigin::External), "external");
    }

    #[test]
    fn reject_appends_a_nonempty_reason_to_the_body() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "no newline tail", None).unwrap();
        let rejected = reject(&store, "r", &fb.id, "out of scope").unwrap();
        assert_eq!(rejected.status, FeedbackStatus::Rejected);
        assert!(rejected.body.contains("Rejected: out of scope"), "reason appended: {}", rejected.body);
    }

    #[test]
    fn create_allocates_sequential_ids() {
        let (_d, store) = store();
        let a = create(&store, "r", "frame", "first", None).unwrap();
        let b = create(&store, "r", "frame", "second", None).unwrap();
        assert_eq!(a.id, "fb-01");
        assert_eq!(b.id, "fb-02");
        assert_eq!(a.status, FeedbackStatus::Pending);
    }

    #[test]
    fn create_external_is_deterministic_and_deduped() {
        let (_d, store) = store();
        // First ingest of a change-request note files a blocker, external-origin,
        // with an id derived from the provider note id.
        let first = create_external(&store, "r", "frame", "r100", "alice", "fix the metric", true)
            .unwrap()
            .expect("first ingest files feedback");
        assert_eq!(first.id, "fb-ext-r100");
        assert_eq!(first.origin, FeedbackOrigin::External);
        assert_eq!(first.severity, Some(FeedbackSeverity::Blocker));
        assert!(first.body.contains("@alice") && first.body.contains("requested changes"));

        // Re-ingesting the SAME note is a no-op (dedup by deterministic id).
        let again = create_external(&store, "r", "frame", "r100", "alice", "fix the metric", true).unwrap();
        assert!(again.is_none(), "the same note must not double-file");
        assert_eq!(list(&store, "r").unwrap().len(), 1);

        // A plain comment files as medium; an empty body is ignored.
        let note = create_external(&store, "r", "frame", "c200", "bob", "nit", false)
            .unwrap()
            .expect("comment files");
        assert_eq!(note.severity, Some(FeedbackSeverity::Medium));
        assert!(create_external(&store, "r", "frame", "c300", "bob", "   ", false).unwrap().is_none());
        assert_eq!(list(&store, "r").unwrap().len(), 2);
    }

    #[test]
    fn origin_and_invalidates_roundtrip_through_disk() {
        let (_d, store) = store();
        let fb = create_with_origin(
            &store, "r", "build", "the diff regresses the limiter", Some(FeedbackSeverity::High),
            FeedbackOrigin::AdversarialReview, vec!["correctness".into(), "user".into()],
        )
        .unwrap();
        let back = get(&store, "r", &fb.id).unwrap();
        assert_eq!(back.origin, FeedbackOrigin::AdversarialReview);
        assert_eq!(back.invalidates, vec!["correctness".to_string(), "user".to_string()]);
    }

    #[test]
    fn close_with_reply_records_resolution_and_invalidates_stamps() {
        use darkrun_core::domain::Stamp;
        let (_d, store) = store();
        // A build unit signed by two roles.
        let mut unit = crate::units::create(&store, "r", "u1", "build", crate::units::UnitSpec::default()).unwrap();
        let stamp = || Some(Stamp { at: "2026-06-04T00:00:00Z".into() });
        unit.frontmatter.approvals.insert("correctness".into(), stamp());
        unit.frontmatter.approvals.insert("user".into(), stamp());
        store.write_unit("r", &unit).unwrap();

        let fb = create_with_origin(
            &store, "r", "build", "regression", Some(FeedbackSeverity::Blocker),
            FeedbackOrigin::AdversarialReview, vec!["correctness".into()],
        )
        .unwrap();
        let closed = close_with_reply(&store, "r", &fb.id, "rewrote the burst path; added a regression test").unwrap();
        assert_eq!(closed.status, FeedbackStatus::Closed);
        assert_eq!(
            closed.closure_reply.as_ref().map(|r| r.text.as_str()),
            Some("rewrote the burst path; added a regression test")
        );
        // The invalidated role is re-opened; the untouched one survives.
        let back = store.read_unit("r", "u1").unwrap();
        assert!(!back.frontmatter.approvals.contains_key("correctness"));
        assert!(matches!(back.frontmatter.approvals.get("user"), Some(Some(_))));
    }

    #[test]
    fn create_rejects_empty_body() {
        let (_d, store) = store();
        let err = create(&store, "r", "frame", "   ", None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let (_d, store) = store();
        let made = create(
            &store,
            "r",
            "build",
            "the widget overflows",
            Some(FeedbackSeverity::High),
        )
        .unwrap();
        let read = get(&store, "r", &made.id).unwrap();
        assert_eq!(read.station, "build");
        assert_eq!(read.severity, Some(FeedbackSeverity::High));
        assert_eq!(read.body.trim(), "the widget overflows");
        assert!(read.created_at.is_some());
    }

    #[test]
    fn set_status_advances_and_blocks_terminal() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "x", None).unwrap();
        let fixing = set_status(&store, "r", &fb.id, FeedbackStatus::Fixing).unwrap();
        assert_eq!(fixing.status, FeedbackStatus::Fixing);
        let closed = set_status(&store, "r", &fb.id, FeedbackStatus::Addressed).unwrap();
        assert_eq!(closed.status, FeedbackStatus::Addressed);
        // Now terminal → further transitions error.
        let err = set_status(&store, "r", &fb.id, FeedbackStatus::Pending).unwrap_err();
        assert!(matches!(err, McpError::FeedbackSettled(_)));
    }

    /// Predecessor BUG-6: a non-code finding (a question, an out-of-scope
    /// observation, a doc/process change) reached a builder fix-hat that could
    /// only edit files or `reject` — so it looped to the bolt cap, never closing.
    /// darkrun gives every finding a terminal NON-CODE route: `Answered` (a
    /// question resolved by reply) and `NonActionable` (valid but no code fix),
    /// both terminal, settable directly without a build loop.
    #[test]
    fn non_code_findings_have_terminal_routes() {
        let (_d, store) = store();
        // A question, answered with a reply — terminal, no code delta.
        let q = create(&store, "r", "frame", "is this in scope?", None).unwrap();
        let answered = set_status(&store, "r", &q.id, FeedbackStatus::Answered).unwrap();
        assert!(is_terminal(answered.status));

        // An out-of-scope observation — valid, but no actionable fix. Terminal.
        let obs = create(&store, "r", "frame", "noted for later, not this run", None).unwrap();
        let na = set_status(&store, "r", &obs.id, FeedbackStatus::NonActionable).unwrap();
        assert!(is_terminal(na.status));
        // Neither can loop: a terminal item refuses further transitions.
        assert!(set_status(&store, "r", &obs.id, FeedbackStatus::Fixing).is_err());
    }

    #[test]
    fn reject_is_terminal_and_appends_reason() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "bad", None).unwrap();
        let rejected = reject(&store, "r", &fb.id, "stale duplicate").unwrap();
        assert_eq!(rejected.status, FeedbackStatus::Rejected);
        assert!(rejected.body.contains("stale duplicate"));
        assert!(is_terminal(rejected.status));
    }

    #[test]
    fn move_relocates_station() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "x", None).unwrap();
        let moved = move_station(&store, "r", &fb.id, "shape").unwrap();
        assert_eq!(moved.station, "shape");
    }

    #[test]
    fn set_severity_updates_rank() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "x", None).unwrap();
        let ranked = set_severity(&store, "r", &fb.id, FeedbackSeverity::Blocker).unwrap();
        assert_eq!(ranked.severity, Some(FeedbackSeverity::Blocker));
    }

    #[test]
    fn get_missing_errors() {
        let (_d, store) = store();
        let err = get(&store, "r", "fb-99").unwrap_err();
        assert!(matches!(err, McpError::FeedbackNotFound(_)));
    }

    #[test]
    fn list_returns_all_sorted() {
        let (_d, store) = store();
        create(&store, "r", "frame", "a", None).unwrap();
        create(&store, "r", "frame", "b", None).unwrap();
        let all = list(&store, "r").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "fb-01");
        assert_eq!(all[1].id, "fb-02");
    }

    #[test]
    fn every_origin_round_trips_through_disk() {
        use darkrun_core::domain::FeedbackOrigin::*;
        let (_d, store) = store();
        for o in [
            AdversarialReview, RunReview, Reflection, Discovery, Drift, Operator,
            Annotation, External, Unspecified,
        ] {
            let fb = create_with_origin(&store, "r", "frame", "body", None, o, vec![]).unwrap();
            assert_eq!(get(&store, "r", &fb.id).unwrap().origin, o);
        }
        // Unknown origin tokens fall back to Unspecified.
        assert_eq!(parse_origin("not-a-real-origin"), Unspecified);
        assert_eq!(parse_origin("review"), AdversarialReview);
    }

    #[test]
    fn move_reject_and_set_targets() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "finding", None).unwrap();
        // set_targets records the invalidated roles.
        let t = set_targets(&store, "r", &fb.id, vec!["spec".into()]).unwrap();
        assert_eq!(t.invalidates, vec!["spec".to_string()]);
        // move_station relocates it.
        let m = move_station(&store, "r", &fb.id, "specify").unwrap();
        assert_eq!(m.station, "specify");
        // reject with a reason appends to the body and is terminal.
        let r = reject(&store, "r", &fb.id, "stale").unwrap();
        assert_eq!(r.status, FeedbackStatus::Rejected);
        assert!(r.body.contains("Rejected: stale"));
        // A settled item is immutable.
        assert!(move_station(&store, "r", &fb.id, "frame").is_err());
        assert!(set_targets(&store, "r", &fb.id, vec![]).is_err());
    }

    #[test]
    fn split_frontmatter_handles_a_body_only_doc() {
        // No fence → the whole thing is the body, parsed as a pending item.
        let fb = parse("r", "fb-9", "just a body, no frontmatter\n");
        assert_eq!(fb.status, FeedbackStatus::Pending);
        assert!(fb.body.contains("just a body"));
    }

    #[test]
    fn set_severity_updates_and_rejects_settled() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "finding", None).unwrap();
        let up = set_severity(&store, "r", &fb.id, FeedbackSeverity::Blocker).unwrap();
        assert_eq!(up.severity, Some(FeedbackSeverity::Blocker));
        reject(&store, "r", &fb.id, "stale").unwrap();
        assert!(set_severity(&store, "r", &fb.id, FeedbackSeverity::Low).is_err());
    }

    #[test]
    fn unspecified_origin_serializes_and_round_trips() {
        let (_d, store) = store();
        // Drives the origin_token `Unspecified` arm through serialize→disk→parse.
        let fb = create_with_origin(
            &store, "r", "frame", "uncategorized note", None,
            FeedbackOrigin::Unspecified, vec![],
        )
        .unwrap();
        assert_eq!(get(&store, "r", &fb.id).unwrap().origin, FeedbackOrigin::Unspecified);
    }

    #[test]
    fn closing_a_settled_feedback_is_rejected() {
        let (_d, store) = store();
        let fb = create(&store, "r", "frame", "x", None).unwrap();
        close_with_reply(&store, "r", &fb.id, "done").unwrap();
        // A second close on the now-terminal record errors.
        assert!(matches!(
            close_with_reply(&store, "r", &fb.id, "again"),
            Err(McpError::FeedbackSettled(_))
        ));
        // reject on a settled record errors too.
        assert!(matches!(reject(&store, "r", &fb.id, "no"), Err(McpError::FeedbackSettled(_))));
    }

    #[test]
    fn close_clears_invalidated_stamps_only_on_the_target_station() {
        use darkrun_core::domain::{Status, Stamp, Unit, UnitFrontmatter};
        let (_d, store) = store();
        // A feedback on `build` that invalidates the `correctness` review.
        let fb = create_with_origin(
            &store, "r", "build", "regresses", Some(FeedbackSeverity::High),
            FeedbackOrigin::AdversarialReview, vec!["correctness".into()],
        )
        .unwrap();
        // A unit on `build` carrying that review, and a sibling on `frame` (skipped).
        let mut on_target = Unit {
            slug: "u-build".into(),
            frontmatter: UnitFrontmatter { status: Status::Completed, station: Some("build".into()), ..Default::default() },
            title: "u".into(), body: String::new(),
        };
        on_target.frontmatter.reviews.insert("correctness".into(), Some(Stamp { at: "2026-06-04T00:00:00Z".into() }));
        store.write_unit("r", &on_target).unwrap();
        let off_target = Unit {
            slug: "u-frame".into(),
            frontmatter: UnitFrontmatter { status: Status::Completed, station: Some("frame".into()), ..Default::default() },
            title: "u".into(), body: String::new(),
        };
        store.write_unit("r", &off_target).unwrap();

        close_with_reply(&store, "r", &fb.id, "fixed").unwrap();
        // The target-station unit lost its invalidated stamp; the off-station one is untouched.
        let units = store.read_units("r").unwrap();
        let target = units.iter().find(|u| u.slug == "u-build").unwrap();
        assert!(!target.frontmatter.reviews.contains_key("correctness"));
    }
}

//! Admin recovery for wedged Runs (the `darkrun-debug` skill).
//!
//! These ops bypass the manager's normal loop to unstick a Run. `preview_cursor`
//! is read-only; every mutating op requires `confirm` and a `reason` (the skill
//! surfaces the reason to the operator before authorizing the bypass).

use darkrun_core::domain::Status;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::{McpError, Result};

/// The outcome of a debug op.
#[derive(Debug, Clone, Serialize)]
pub struct DebugResult {
    /// The op performed (or previewed).
    pub op: String,
    /// Whether a mutation was applied (false for previews / unconfirmed).
    pub applied: bool,
    /// A human-readable summary.
    pub note: String,
    /// Op-specific payload (e.g. the previewed cursor).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

fn require(confirm: bool, reason: Option<&str>, op: &str) -> Result<String> {
    let reason = reason
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| McpError::InvalidInput(format!("`reason` is required for `{op}`")))?;
    if !confirm {
        return Err(McpError::InvalidInput(format!(
            "`{op}` mutates state — re-call with confirm:true (reason: {reason})"
        )));
    }
    Ok(reason.to_string())
}

/// Read-only: what would the next `run_next` return, given current on-disk state?
pub fn preview_cursor(store: &StateStore, slug: &str) -> Result<DebugResult> {
    let pos = crate::position::derive_position(store, slug)?;
    Ok(DebugResult {
        op: "preview_cursor".to_string(),
        applied: false,
        note: "the cursor the next run_next would derive (no state changed)".to_string(),
        data: Some(serde_json::to_value(&pos).unwrap_or(serde_json::Value::Null)),
    })
}

/// Force a station (and every station before it in the factory order) to
/// `Completed`, so the cursor advances past a gate whose stamps never landed.
pub fn force_station_complete(
    store: &StateStore,
    slug: &str,
    station: &str,
    confirm: bool,
    reason: Option<&str>,
) -> Result<DebugResult> {
    let reason = require(confirm, reason, "force_station_complete")?;
    let run = store.read_run(slug)?;
    let factory = crate::position::resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;
    let order = factory.station_names();
    let target_idx = order
        .iter()
        .position(|s| s == station)
        .ok_or_else(|| McpError::UnknownStation(station.to_string()))?;

    let mut state = store.read_state(slug)?.unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    for name in order.iter().take(target_idx + 1) {
        if let Some(st) = state.stations.get_mut(name) {
            st.status = Status::Completed;
            st.completed_at = Some(now.clone());
        }
    }
    store.write_state(slug, &state)?;
    // A manual unwedge: clear the deadlock guard's history so the recovered run
    // starts from a clean count instead of immediately re-halting.
    crate::deadlock::clear(store, slug);
    Ok(DebugResult {
        op: "force_station_complete".to_string(),
        applied: true,
        note: format!("forced `{station}` and prior stations to Completed ({reason})"),
        data: None,
    })
}

/// Set a manager-protected run field. Currently `mode` (the sizing mode).
pub fn set_run_field(
    store: &StateStore,
    slug: &str,
    field: &str,
    value: &str,
    confirm: bool,
    reason: Option<&str>,
) -> Result<DebugResult> {
    let reason = require(confirm, reason, "set_run_field")?;
    let mut run = store.read_run(slug)?;
    match field {
        "mode" => run.frontmatter.mode = value.to_string(),
        "active_station" => run.frontmatter.active_station = value.to_string(),
        other => {
            return Err(McpError::InvalidInput(format!(
                "field `{other}` is not settable (mode | active_station)"
            )))
        }
    }
    store.write_run(&run)?;
    Ok(DebugResult {
        op: "set_run_field".to_string(),
        applied: true,
        note: format!("set {field} = `{value}` ({reason})"),
        data: None,
    })
}

/// Re-witness every locked artifact to its current content and clear all drift —
/// stops a sweep re-firing on witnesses that are stale but already reconciled.
pub fn reset_drift(
    store: &StateStore,
    slug: &str,
    confirm: bool,
    reason: Option<&str>,
) -> Result<DebugResult> {
    let reason = require(confirm, reason, "reset_drift")?;
    let mut count = 0;
    for w in store.read_witnesses(slug)? {
        if crate::drift::accept(store, slug, &w.path)? {
            count += 1;
        }
    }
    Ok(DebugResult {
        op: "reset_drift".to_string(),
        applied: true,
        note: format!("re-witnessed {count} artifact(s) and cleared drift ({reason})"),
        data: None,
    })
}

/// Set a feedback record's status directly, bypassing lifecycle guards.
pub fn mutate_feedback(
    store: &StateStore,
    slug: &str,
    feedback_id: &str,
    status: &str,
    confirm: bool,
    reason: Option<&str>,
) -> Result<DebugResult> {
    let reason = require(confirm, reason, "mutate_feedback")?;
    let parsed = crate::feedback::parse_status(status)
        .ok_or_else(|| McpError::InvalidInput(format!("invalid feedback status: {status}")))?;
    let fb = crate::feedback::set_status(store, slug, feedback_id, parsed)?;
    Ok(DebugResult {
        op: "mutate_feedback".to_string(),
        applied: true,
        note: format!("set feedback `{feedback_id}` status = `{status}` ({reason})"),
        data: Some(serde_json::to_value(&fb).unwrap_or(serde_json::Value::Null)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::run_start;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = StateStore::new(dir.path());
        (dir, s)
    }

    #[test]
    fn preview_cursor_is_read_only() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, "continuous").unwrap();
        let r = preview_cursor(&store, "r").unwrap();
        assert!(!r.applied);
        assert!(r.data.is_some());
    }

    #[test]
    fn mutating_ops_require_confirm_and_reason() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, "continuous").unwrap();
        // No reason → error.
        assert!(force_station_complete(&store, "r", "frame", true, None).is_err());
        // Reason but no confirm → error.
        assert!(force_station_complete(&store, "r", "frame", false, Some("stuck")).is_err());
        // Both → applies.
        let r = force_station_complete(&store, "r", "frame", true, Some("stuck")).unwrap();
        assert!(r.applied);
        assert_eq!(
            store.read_state("r").unwrap().unwrap().stations["frame"].status,
            Status::Completed
        );
    }

    #[test]
    fn set_run_field_mode() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, "continuous").unwrap();
        let r = set_run_field(&store, "r", "mode", "quick", true, Some("retune")).unwrap();
        assert!(r.applied);
        assert_eq!(store.read_run("r").unwrap().frontmatter.mode, "quick");
        assert!(set_run_field(&store, "r", "nope", "x", true, Some("y")).is_err());
    }
}

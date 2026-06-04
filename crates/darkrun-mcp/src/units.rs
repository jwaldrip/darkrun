//! Unit create/update helpers over the core unit store.
//!
//! The manager decomposes a station's spec into **Units**; these helpers give
//! the MCP tools a typed surface to create a unit, read it, and apply
//! field-scoped corrective updates — mirroring the predecessor's
//! `unit_set`/`unit_list`/`unit_get` triple in factory vocabulary.
//!
//! The forward-only lifecycle rule applies: a unit's structural fields
//! (dependencies, station, type) are only mutable while the unit is `pending`.
//! Status itself can always be advanced. This keeps the dependency DAG stable
//! once a unit starts executing.

use chrono::Utc;
use darkrun_core::domain::{IterationResult, Status, Unit, UnitFrontmatter, UnitIteration};
use darkrun_core::StateStore;

use crate::error::{McpError, Result};

/// Create a new pending unit on a station, returning the persisted record.
pub fn create(
    store: &StateStore,
    run: &str,
    slug: &str,
    station: &str,
    title: Option<String>,
    depends_on: Vec<String>,
) -> Result<Unit> {
    if slug.trim().is_empty() {
        return Err(McpError::InvalidInput("unit slug must not be empty".into()));
    }
    if store.read_unit(run, slug).is_ok() {
        return Err(McpError::InvalidInput(format!(
            "unit '{slug}' already exists"
        )));
    }
    let resolved_title = title.clone().unwrap_or_else(|| slug.to_string());
    let unit = Unit {
        slug: slug.to_string(),
        frontmatter: UnitFrontmatter {
            name: title,
            status: Status::Pending,
            station: Some(station.to_string()),
            depends_on,
            ..Default::default()
        },
        title: resolved_title.clone(),
        body: format!("# {resolved_title}\n"),
    };
    store.write_unit(run, &unit)?;
    Ok(unit)
}

/// Read a single unit by slug.
pub fn get(store: &StateStore, run: &str, slug: &str) -> Result<Unit> {
    store
        .read_unit(run, slug)
        .map_err(|_| McpError::UnitNotFound(slug.to_string()))
}

/// A field-scoped corrective update to a pending unit.
#[derive(Debug, Default, Clone)]
pub struct UnitUpdate {
    /// New status (always permitted — advances the lifecycle).
    pub status: Option<Status>,
    /// New dependency set (pending-only).
    pub depends_on: Option<Vec<String>>,
    /// New worker assignment.
    pub worker: Option<String>,
    /// New declared inputs (pending-only).
    pub inputs: Option<Vec<String>>,
    /// New declared outputs.
    pub outputs: Option<Vec<String>>,
}

/// Apply a corrective update to a unit.
///
/// Structural edits (`depends_on`, `inputs`) require the unit be `pending` —
/// the forward-only rule keeps the DAG stable once execution starts. A status
/// change to `completed`/`active` stamps the matching timestamp.
pub fn update(store: &StateStore, run: &str, slug: &str, upd: UnitUpdate) -> Result<Unit> {
    let mut unit = get(store, run, slug)?;
    let pending = matches!(unit.frontmatter.status, Status::Pending);

    if !pending && (upd.depends_on.is_some() || upd.inputs.is_some()) {
        return Err(McpError::InvalidInput(format!(
            "unit '{slug}' is no longer pending; structural fields are immutable"
        )));
    }

    if let Some(deps) = upd.depends_on {
        unit.frontmatter.depends_on = deps;
    }
    if let Some(inputs) = upd.inputs {
        unit.frontmatter.inputs = inputs;
    }
    if let Some(outputs) = upd.outputs {
        unit.frontmatter.outputs = outputs;
    }
    if let Some(worker) = upd.worker {
        unit.frontmatter.worker = worker;
    }
    if let Some(status) = upd.status {
        let now = Utc::now().to_rfc3339();
        match status {
            Status::Active | Status::InProgress if unit.frontmatter.started_at.is_none() => {
                unit.frontmatter.started_at = Some(now);
            }
            Status::Completed => {
                if unit.frontmatter.started_at.is_none() {
                    unit.frontmatter.started_at = Some(now.clone());
                }
                unit.frontmatter.completed_at = Some(now);
            }
            _ => {}
        }
        unit.frontmatter.status = status;
    }

    store.write_unit(run, &unit)?;
    Ok(unit)
}

/// Record one Pass beat on a unit — append-only. A worker reports whether it
/// `advance`d or `reject`ed, plus a **note**: its handoff to the next worker on
/// advance, or its reason on reject. The note is what the next dispatch reads,
/// and what the operator and the reflection pass see — the loop's story.
///
/// On `advance` the assigned `worker` rolls forward to the next worker the
/// caller names (the engine dispatches it next tick); on `reject` it bounces
/// back to `bounce_to` (the caller resolves the bounce target — typically the
/// nearest build worker). The unit's `pass` count is the iteration length, so it
/// grows by one here automatically.
pub fn record_iteration(
    store: &StateStore,
    run: &str,
    slug: &str,
    worker: &str,
    result: IterationResult,
    note: Option<String>,
    next_worker: Option<String>,
) -> Result<Unit> {
    if worker.trim().is_empty() {
        return Err(McpError::InvalidInput("iteration worker must not be empty".into()));
    }
    let mut unit = get(store, run, slug)?;
    let now = Utc::now().to_rfc3339();
    unit.frontmatter.iterations.push(UnitIteration {
        worker: worker.to_string(),
        started_at: Some(now.clone()),
        completed_at: Some(now.clone()),
        result: Some(result),
        note,
    });
    // The unit leaves Pending the moment its first beat runs.
    if matches!(unit.frontmatter.status, Status::Pending) {
        unit.frontmatter.status = Status::InProgress;
        if unit.frontmatter.started_at.is_none() {
            unit.frontmatter.started_at = Some(now);
        }
    }
    // Roll the active-worker assignment forward (advance) or back (reject).
    if let Some(next) = next_worker {
        unit.frontmatter.worker = next;
    }
    store.write_unit(run, &unit)?;
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store1() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn create_seeds_pending_unit() {
        let (_d, store) = store1();
        let u = create(&store, "r", "u1", "frame", Some("First".into()), vec![]).unwrap();
        assert_eq!(u.frontmatter.status, Status::Pending);
        assert_eq!(u.station(), "frame");
        assert_eq!(u.title, "First");
    }

    #[test]
    fn create_rejects_duplicate() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", None, vec![]).unwrap();
        let err = create(&store, "r", "u1", "frame", None, vec![]).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn create_rejects_empty_slug() {
        let (_d, store) = store1();
        let err = create(&store, "r", " ", "frame", None, vec![]).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn update_advances_status_and_stamps_completion() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", None, vec![]).unwrap();
        let done = update(
            &store,
            "r",
            "u1",
            UnitUpdate {
                status: Some(Status::Completed),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(done.frontmatter.status, Status::Completed);
        assert!(done.frontmatter.completed_at.is_some());
        assert!(done.frontmatter.started_at.is_some());
    }

    #[test]
    fn update_deps_blocked_once_not_pending() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", None, vec![]).unwrap();
        update(
            &store,
            "r",
            "u1",
            UnitUpdate {
                status: Some(Status::Active),
                ..Default::default()
            },
        )
        .unwrap();
        let err = update(
            &store,
            "r",
            "u1",
            UnitUpdate {
                depends_on: Some(vec!["x".into()]),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn update_deps_allowed_while_pending() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", None, vec![]).unwrap();
        let u = update(
            &store,
            "r",
            "u1",
            UnitUpdate {
                depends_on: Some(vec!["dep".into()]),
                worker: Some("builder".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(u.frontmatter.depends_on, vec!["dep".to_string()]);
        assert_eq!(u.frontmatter.worker, "builder");
    }

    #[test]
    fn get_missing_errors() {
        let (_d, store) = store1();
        let err = get(&store, "r", "ghost").unwrap_err();
        assert!(matches!(err, McpError::UnitNotFound(_)));
    }

    #[test]
    fn record_iteration_appends_note_and_derives_pass() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", None, vec![]).unwrap();
        let u = record_iteration(
            &store, "r", "u1", "make", IterationResult::Advance,
            Some("drafted; next: stress the burst path".into()),
            Some("challenge".into()),
        )
        .unwrap();
        assert_eq!(u.pass(), 1);
        assert_eq!(u.frontmatter.status, Status::InProgress);
        assert_eq!(u.active_worker(), "challenge");
        assert_eq!(u.last_note(), Some("drafted; next: stress the burst path"));
        assert!(u.frontmatter.iterations[0].completed_at.is_some());

        // A reject bounces the assignment back and grows the pass count.
        let u2 = record_iteration(
            &store, "r", "u1", "challenge", IterationResult::Reject,
            Some("burst overflows the bucket — bounce to make".into()),
            Some("make".into()),
        )
        .unwrap();
        assert_eq!(u2.pass(), 2);
        assert_eq!(u2.active_worker(), "make");
        assert_eq!(u2.last_note(), Some("burst overflows the bucket — bounce to make"));
    }

    #[test]
    fn record_iteration_rejects_empty_worker() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", None, vec![]).unwrap();
        let err = record_iteration(&store, "r", "u1", " ", IterationResult::Advance, None, None)
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }
}

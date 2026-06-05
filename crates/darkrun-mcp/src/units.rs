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
use darkrun_core::domain::{
    GateResult, GateStatus, IterationResult, Stamp, Status, Unit, UnitFrontmatter, UnitIteration,
};
use darkrun_core::StateStore;

use crate::error::{McpError, Result};

/// Which stamp map a per-role sign-off writes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampKind {
    /// PRE-execute spec review (`reviews` map).
    Review,
    /// POST-execute output approval (`approvals` map).
    Approval,
}

/// After this many env-blocked recordings, a gate is auto-deferred to CI rather
/// than wedging the run — CI is authoritative on the change request.
const GATE_DEFER_AFTER: u32 = 2;

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

/// Record a quality-gate result on a unit — upsert by gate name, accumulating
/// the attempt count. A `pass` satisfies the gate; a `fail` holds Audit; an
/// `env_blocked` that has now been seen `GATE_DEFER_AFTER` times is auto-promoted
/// to `deferred_to_ci` so an unrunnable gate can't wedge the run (CI is the
/// authority). Returns the updated unit.
pub fn record_gate_result(
    store: &StateStore,
    run: &str,
    slug: &str,
    gate: &str,
    status: GateStatus,
    detail: Option<String>,
    nonce: Option<&str>,
) -> Result<Unit> {
    if gate.trim().is_empty() {
        return Err(McpError::InvalidInput("gate name must not be empty".into()));
    }
    let mut unit = get(store, run, slug)?;
    // B5: a gate result is only trustworthy if it came from a real verification
    // dispatch. The unit's station carries a one-time `verifier_nonce` minted
    // when the engine dispatched Manufacture; the caller must echo it. A station
    // with no nonce (legacy/non-gated path) imposes no check.
    let station = unit.station().to_string();
    if let Some(expected) = store
        .read_state(run)
        .ok()
        .flatten()
        .and_then(|s| s.stations.get(&station).and_then(|st| st.verifier_nonce.clone()))
    {
        match nonce {
            Some(n) if n == expected => {}
            _ => {
                return Err(McpError::InvalidInput(format!(
                    "quality-gate result for '{gate}' rejected: missing or wrong verifier nonce — \
                     record gates only from the engine's Manufacture dispatch (which carries the nonce)"
                )));
            }
        }
    }
    let now = Utc::now().to_rfc3339();
    let prior_attempts = unit
        .frontmatter
        .gate_results
        .iter()
        .find(|r| r.name == gate)
        .map(|r| r.attempts)
        .unwrap_or(0);
    let attempts = prior_attempts + 1;
    // Auto-defer a repeatedly env-blocked gate to CI rather than wedge.
    let effective = if matches!(status, GateStatus::EnvBlocked) && attempts >= GATE_DEFER_AFTER {
        GateStatus::DeferredToCi
    } else {
        status
    };
    let result = GateResult {
        name: gate.to_string(),
        status: effective,
        at: Some(now),
        attempts,
        detail,
    };
    unit.frontmatter.gate_results.retain(|r| r.name != gate);
    unit.frontmatter.gate_results.push(result);
    store.write_unit(run, &unit)?;
    Ok(unit)
}

/// The outcome of stamping a review/approval role across a station's units.
#[derive(Debug, Clone, Default)]
pub struct StampOutcome {
    /// Unit slugs stamped this call.
    pub stamped: Vec<String>,
    /// Unit slugs skipped because the reviewer left an open finding on them —
    /// an open finding for this role means the work isn't signed off yet.
    pub skipped: Vec<String>,
}

/// Stamp one review/approval `role` across the given station's units — the
/// **parallel-safe** per-role sign-off. This writes only the one role's stamp
/// and returns; it does **not** walk the cursor, so N reviewer subagents can
/// each stamp their own role concurrently without contending on the tick or
/// tripping the deadlock guard. The parent ticks once after the wave closes.
///
/// A unit with an open feedback finding targeting this `station` is **skipped**
/// (its work isn't signed) — the reviewer should file the finding, not stamp.
pub fn stamp_role(
    store: &StateStore,
    run: &str,
    station: &str,
    role: &str,
    kind: StampKind,
    open_feedback_stations: &[String],
) -> Result<StampOutcome> {
    if role.trim().is_empty() {
        return Err(McpError::InvalidInput("review role must not be empty".into()));
    }
    let station_has_open_finding = open_feedback_stations.iter().any(|s| s == station);
    let now = Utc::now().to_rfc3339();
    let mut outcome = StampOutcome::default();
    for mut unit in store.read_units(run)? {
        if unit.station() != station {
            continue;
        }
        if station_has_open_finding {
            outcome.skipped.push(unit.slug.clone());
            continue;
        }
        let map = match kind {
            StampKind::Review => &mut unit.frontmatter.reviews,
            StampKind::Approval => &mut unit.frontmatter.approvals,
        };
        map.insert(role.to_string(), Some(Stamp { at: now.clone() }));
        store.write_unit(run, &unit)?;
        outcome.stamped.push(unit.slug.clone());
    }
    Ok(outcome)
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
    fn quality_gate_record_satisfies_on_pass_and_defers_blocked_to_ci() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        let mut u = create(&store, "r", "u1", "build", None, vec![]).unwrap();
        u.frontmatter.quality_gates = vec![
            QualityGate { name: "tests".into(), command: "cargo test".into() },
            QualityGate { name: "lint".into(), command: "cargo clippy".into() },
        ];
        store.write_unit("r", &u).unwrap();

        // Unsatisfied until both gates land.
        assert!(!store.read_unit("r", "u1").unwrap().gates_satisfied());
        record_gate_result(&store, "r", "u1", "tests", GateStatus::Pass, None, None).unwrap();
        assert!(!store.read_unit("r", "u1").unwrap().gates_satisfied());

        // lint is env-blocked: first block holds; second auto-defers to CI.
        record_gate_result(&store, "r", "u1", "lint", GateStatus::EnvBlocked, Some("no toolchain".into()), None).unwrap();
        assert!(!store.read_unit("r", "u1").unwrap().gates_satisfied());
        let after = record_gate_result(&store, "r", "u1", "lint", GateStatus::EnvBlocked, None, None).unwrap();
        // Both now satisfied (tests=pass, lint=deferred_to_ci).
        assert!(after.gates_satisfied());
        let lint = after.frontmatter.gate_results.iter().find(|r| r.name == "lint").unwrap();
        assert_eq!(lint.status, GateStatus::DeferredToCi);
        assert_eq!(lint.attempts, 2);
    }

    #[test]
    fn quality_gate_fail_holds_the_unit() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        let mut u = create(&store, "r", "u1", "build", None, vec![]).unwrap();
        u.frontmatter.quality_gates = vec![QualityGate { name: "tests".into(), command: "t".into() }];
        store.write_unit("r", &u).unwrap();
        let after = record_gate_result(&store, "r", "u1", "tests", GateStatus::Fail, None, None).unwrap();
        assert!(!after.gates_satisfied());
        assert_eq!(after.unsatisfied_gates(), vec!["tests"]);
    }

    /// Predecessor BUG-1: the CI-deferral attempt counter was keyed per-UNIT, so
    /// a gate inherited the unit's accumulated count and deferred on its FIRST
    /// failure. darkrun keys attempts PER-GATE-NAME — each gate's counter starts
    /// at its own first appearance, independent of how many times sibling gates
    /// were recorded.
    #[test]
    fn ci_deferral_counter_is_per_gate_not_per_unit() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", None, vec![]).unwrap();

        // Gate A is env-blocked twice → it defers to CI on its 2nd attempt.
        record_gate_result(&store, "r", "u1", "type_check", GateStatus::EnvBlocked, None, None)
            .unwrap();
        let a = record_gate_result(&store, "r", "u1", "type_check", GateStatus::EnvBlocked, None, None)
            .unwrap();
        let ga = a.frontmatter.gate_results.iter().find(|r| r.name == "type_check").unwrap();
        assert_eq!(ga.attempts, 2);
        assert_eq!(ga.status, GateStatus::DeferredToCi);

        // Now gate B is env-blocked for the FIRST time. Even though the unit
        // already has 2 prior gate recordings, B's counter starts at 1 — it must
        // NOT inherit A's count and defer prematurely.
        let b = record_gate_result(&store, "r", "u1", "lint", GateStatus::EnvBlocked, None, None)
            .unwrap();
        let gb = b.frontmatter.gate_results.iter().find(|r| r.name == "lint").unwrap();
        assert_eq!(gb.attempts, 1, "a gate's first failure counts as attempt 1");
        assert_eq!(
            gb.status,
            GateStatus::EnvBlocked,
            "a gate must NOT defer on its first failure just because siblings have a high count"
        );
    }

    #[test]
    fn verifier_nonce_is_required_when_the_station_carries_one() {
        use darkrun_core::domain::Station;
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", None, vec![]).unwrap();
        // The station carries a minted nonce (as it would after Manufacture
        // dispatch). Recording a gate now requires that exact token.
        let mut state = darkrun_core::RunState::default();
        let st = Station {
            station: "build".into(),
            status: Status::InProgress,
            phase: darkrun_core::domain::StationPhase::Manufacture,
            elaborated: false,
            checkpoint: None,
            chosen_checkpoint: None,
            branch: None,
            pr_ref: None,
            pr_status: None,
            pr_ready_at: None,
            pr_merged_at: None,
            verifier_nonce: Some("the-token".into()),
            started_at: None,
            completed_at: None,
        };
        state.stations.insert("build".into(), st);
        store.write_state("r", &state).unwrap();

        // No nonce → rejected.
        let err = record_gate_result(&store, "r", "u1", "tests", GateStatus::Pass, None, None)
            .unwrap_err();
        assert!(format!("{err}").contains("verifier nonce"), "{err}");
        // Wrong nonce → rejected.
        assert!(record_gate_result(&store, "r", "u1", "tests", GateStatus::Pass, None, Some("wrong"))
            .is_err());
        // Correct nonce → accepted.
        let ok = record_gate_result(
            &store, "r", "u1", "tests", GateStatus::Pass, None, Some("the-token"),
        );
        assert!(ok.is_ok(), "correct nonce records the gate: {ok:?}");
    }

    #[test]
    fn stamp_role_signs_station_units_and_skips_open_findings() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", None, vec![]).unwrap();
        create(&store, "r", "u2", "build", None, vec![]).unwrap();
        create(&store, "r", "other", "frame", None, vec![]).unwrap();

        // No open findings → both build units get the correctness review stamp;
        // the frame unit is untouched.
        let out = stamp_role(&store, "r", "build", "correctness", StampKind::Review, &[]).unwrap();
        assert_eq!(out.stamped.len(), 2);
        assert!(out.skipped.is_empty());
        let u1 = store.read_unit("r", "u1").unwrap();
        assert!(matches!(u1.frontmatter.reviews.get("correctness"), Some(Some(_))));
        assert!(store.read_unit("r", "other").unwrap().frontmatter.reviews.is_empty());

        // An open finding on the station → its units are skipped, not stamped.
        let out2 = stamp_role(
            &store, "r", "build", "maintainability", StampKind::Approval,
            &["build".to_string()],
        )
        .unwrap();
        assert!(out2.stamped.is_empty());
        assert_eq!(out2.skipped.len(), 2);
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

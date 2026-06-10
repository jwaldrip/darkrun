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
/// Everything an agent may author on a unit at creation — the predecessor's
/// agent-authorable surface. The BODY is the unit's spec: the completion
/// criteria, scope, and guidance the executing subagent works from (it has no
/// other context). A unit without a real body is a slug, not a definition.
#[derive(Debug, Clone, Default)]
pub struct UnitSpec {
    /// Display title.
    pub title: Option<String>,
    /// The full markdown spec body (goal, completion criteria paired with
    /// verify commands, scope/out-of-scope, files touched).
    pub body: Option<String>,
    /// Sibling unit slugs this one depends on — the ONLY thing the wave
    /// scheduler sequences on.
    pub depends_on: Vec<String>,
    /// Run-relative input paths consumed (file paths, never unit slugs).
    pub inputs: Vec<String>,
    /// Run-relative output paths produced.
    pub outputs: Vec<String>,
    /// Declared objective gates. `None` = undeclared (rejected when the unit
    /// declares outputs); `Some(vec![])` = a DELIBERATE deferral.
    pub quality_gates: Option<Vec<darkrun_core::domain::QualityGate>>,
    /// Optional model tier override (opus / sonnet / haiku).
    pub model: Option<String>,
    /// Free-form unit kind (feature / test / doc / knowledge …).
    pub unit_type: Option<String>,
}

/// Unit-slug shape: lowercase url-safe (`[a-z0-9][a-z0-9._-]*`).
fn slug_ok(slug: &str) -> bool {
    let mut chars = slug.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Input/output path shape: a relative path with no whitespace, colon, or
/// comma — a PATH, never prose and never a unit slug used as a name.
fn path_ok(p: &str) -> bool {
    !p.is_empty() && !p.chars().any(|c| c.is_whitespace() || matches!(c, ':' | ','))
}

/// Whether a declared gate trivially passes — the predecessor's circular-gate
/// guard: a zero-match assertion (`! grep …`), or a prose-substring grep
/// (`complete|done|finished`) pointed at the unit's own output, both of which
/// the implementer can satisfy without the behavior being true.
fn gate_is_trivial(command: &str, outputs: &[String]) -> bool {
    let c = command.trim();
    if c.starts_with("! grep") || c.starts_with("!grep") {
        return true;
    }
    if c.contains("grep")
        && ["complete", "done", "finished"].iter().any(|w| c.contains(w))
        && outputs.iter().any(|o| c.contains(o.as_str()))
    {
        return true;
    }
    false
}

/// Validate a unit's authored shape against its siblings — every rule the
/// predecessor enforced so a thin or self-defeating definition bounces at
/// write time instead of failing the run later.
fn validate_spec(
    slug: &str,
    spec: &UnitSpec,
    siblings: &[Unit],
) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    if !slug_ok(slug) {
        errors.push(format!(
            "slug '{slug}' must be lowercase url-safe ([a-z0-9][a-z0-9._-]*)"
        ));
    }
    // depends_on: no self, every entry resolves to a sibling.
    for d in &spec.depends_on {
        if d == slug {
            errors.push(format!("depends_on must not reference the unit itself ('{d}')"));
        } else if !siblings.iter().any(|u| u.slug == *d) {
            errors.push(format!("depends_on '{d}' does not resolve to a sibling unit"));
        }
    }
    // Cycle detection over the sibling graph + this unit.
    {
        use std::collections::{HashMap, HashSet};
        let mut deps: HashMap<&str, Vec<&str>> = siblings
            .iter()
            .map(|u| {
                (
                    u.slug.as_str(),
                    u.frontmatter.depends_on.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        deps.insert(slug, spec.depends_on.iter().map(String::as_str).collect());
        fn dfs<'a>(
            n: &'a str,
            deps: &HashMap<&'a str, Vec<&'a str>>,
            seen: &mut HashSet<&'a str>,
            stack: &mut HashSet<&'a str>,
        ) -> bool {
            if stack.contains(n) {
                return true;
            }
            if !seen.insert(n) {
                return false;
            }
            stack.insert(n);
            let cyc = deps
                .get(n)
                .map(|ds| ds.iter().any(|d| dfs(d, deps, seen, stack)))
                .unwrap_or(false);
            stack.remove(n);
            cyc
        }
        let mut seen = HashSet::new();
        let mut stack = HashSet::new();
        if dfs(slug, &deps, &mut seen, &mut stack) {
            errors.push("depends_on introduces a dependency cycle".into());
        }
    }
    // Inputs: real paths; not sibling slugs (that's a depends_on, not an
    // input); a sibling-produced path requires the producer in depends_on.
    for i in &spec.inputs {
        if !path_ok(i) {
            errors.push(format!("input '{i}' is not a path (no spaces/colons/commas)"));
            continue;
        }
        if siblings.iter().any(|u| u.slug == *i) {
            errors.push(format!(
                "input '{i}' is a unit slug — declare the producer in depends_on and put \
                 its OUTPUT PATH in inputs"
            ));
            continue;
        }
        if let Some(producer) = siblings
            .iter()
            .find(|u| u.frontmatter.outputs.iter().any(|o| o == i))
        {
            if !spec.depends_on.contains(&producer.slug) {
                errors.push(format!(
                    "input '{i}' is produced by sibling '{}' — add it to depends_on (the \
                     scheduler sequences ONLY on depends_on)",
                    producer.slug
                ));
            }
        }
    }
    for o in &spec.outputs {
        if !path_ok(o) {
            errors.push(format!("output '{o}' is not a path (no spaces/colons/commas)"));
        }
    }
    // Outputs ⇒ gates declared: None bounces; an explicit [] is a deliberate,
    // visible deferral.
    if !spec.outputs.is_empty() && spec.quality_gates.is_none() {
        errors.push(
            "a unit that declares outputs must declare quality_gates — executable checks \
             that prove the criteria (pass an explicit empty list to defer deliberately)"
                .into(),
        );
    }
    if let Some(gates) = &spec.quality_gates {
        for g in gates {
            if g.name.trim().is_empty() || g.command.trim().is_empty() {
                errors.push("every quality gate needs a name and a command".into());
            } else if gate_is_trivial(&g.command, &spec.outputs) {
                errors.push(format!(
                    "gate '{}' trivially passes (zero-match or prose-substring assertion \
                     against the unit's own output) — replace it with a positive, \
                     behavior-driven check",
                    g.name
                ));
            }
        }
    }
    // Model tier: a known tier or nothing (the predecessor sanitized untrusted
    // frontmatter the same way — a typo'd tier silently became "no override").
    if let Some(m) = spec.model.as_deref() {
        if !matches!(m, "opus" | "sonnet" | "haiku") {
            errors.push(format!(
                "model '{m}' is not a tier — use opus (architectural risk), \
                 sonnet (default), or haiku (purely mechanical)"
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(McpError::InvalidInput(errors.join("; ")))
    }
}

pub fn create(
    store: &StateStore,
    run: &str,
    slug: &str,
    station: &str,
    spec: UnitSpec,
) -> Result<Unit> {
    if slug.trim().is_empty() {
        return Err(McpError::InvalidInput("unit slug must not be empty".into()));
    }
    if store.read_unit(run, slug).is_ok() {
        return Err(McpError::InvalidInput(format!(
            "unit '{slug}' already exists"
        )));
    }
    let siblings = store.read_units(run).unwrap_or_default();
    validate_spec(slug, &spec, &siblings)?;
    let resolved_title = spec.title.clone().unwrap_or_else(|| slug.to_string());
    // The body IS the definition: the spec the executing subagent works from.
    // A missing body degrades to the bare heading, but the spec prompt demands
    // the full anatomy and the review phase reads it.
    let body = match spec.body.as_deref().map(str::trim) {
        Some(b) if !b.is_empty() => format!("{b}\n"),
        _ => format!("# {resolved_title}\n"),
    };
    let unit = Unit {
        slug: slug.to_string(),
        frontmatter: UnitFrontmatter {
            name: spec.title,
            status: Status::Pending,
            station: Some(station.to_string()),
            depends_on: spec.depends_on,
            inputs: spec.inputs,
            outputs: spec.outputs,
            quality_gates: spec.quality_gates.unwrap_or_default(),
            model: spec.model,
            unit_type: spec.unit_type.unwrap_or_default(),
            ..Default::default()
        },
        title: resolved_title,
        body,
    };
    store.write_unit(run, &unit)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: create unit {slug}"));
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
    /// New declared outputs. CORRECTIVE-EXEMPT: editable after the unit
    /// starts — adding a missed output changes how the unit is verified, not
    /// what it produced.
    pub outputs: Option<Vec<String>>,
    /// New spec body (pending-only — the definition freezes once work starts).
    pub body: Option<String>,
    /// New quality gates. CORRECTIVE-EXEMPT like outputs: a broken gate
    /// command must be fixable without reopening the unit.
    pub quality_gates: Option<Vec<darkrun_core::domain::QualityGate>>,
    /// New model tier (pending-only).
    pub model: Option<String>,
}

/// Apply a corrective update to a unit.
///
/// Structural edits (`depends_on`, `inputs`) require the unit be `pending` —
/// the forward-only rule keeps the DAG stable once execution starts. A status
/// change to `completed`/`active` stamps the matching timestamp.
pub fn update(store: &StateStore, run: &str, slug: &str, upd: UnitUpdate) -> Result<Unit> {
    let mut unit = get(store, run, slug)?;
    let pending = matches!(unit.frontmatter.status, Status::Pending);

    if !pending
        && (upd.depends_on.is_some()
            || upd.inputs.is_some()
            || upd.body.is_some()
            || upd.model.is_some())
    {
        return Err(McpError::InvalidInput(format!(
            "unit '{slug}' is no longer pending; structural fields are immutable \
             (outputs and quality_gates stay correctable)"
        )));
    }
    if let Some(body) = &upd.body {
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            unit.body = format!("{trimmed}\n");
        }
    }
    if let Some(model) = &upd.model {
        unit.frontmatter.model = Some(model.clone());
    }
    if let Some(gates) = &upd.quality_gates {
        unit.frontmatter.quality_gates = gates.clone();
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

    // Re-validate the authored shape after the edit, against the OTHER units.
    let siblings: Vec<Unit> = store
        .read_units(run)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| u.slug != slug)
        .collect();
    let spec = UnitSpec {
        title: unit.frontmatter.name.clone(),
        body: Some(unit.body.clone()),
        depends_on: unit.frontmatter.depends_on.clone(),
        inputs: unit.frontmatter.inputs.clone(),
        outputs: unit.frontmatter.outputs.clone(),
        quality_gates: Some(unit.frontmatter.quality_gates.clone()),
        model: unit.frontmatter.model.clone(),
        unit_type: Some(unit.frontmatter.unit_type.clone()),
    };
    validate_spec(slug, &spec, &siblings)?;
    store.write_unit(run, &unit)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: update unit {slug}"));
    Ok(unit)
}

/// What a unit reset cleared (or, on a dry run, would clear).
#[derive(Debug, Clone, serde::Serialize)]
pub struct UnitResetPlan {
    /// The run slug.
    pub run: String,
    /// The unit slug that was/would be reset.
    pub unit: String,
    /// The unit's station.
    pub station: String,
    /// The unit's status before the reset (what it's being rescued from).
    pub from_status: String,
    /// Pass iterations cleared.
    pub passes_cleared: u32,
    /// Review + approval stamps cleared.
    pub stamps_cleared: usize,
    /// Gate results cleared.
    pub gates_cleared: usize,
    /// Whether the reset was actually applied (vs a dry run).
    pub confirmed: bool,
    /// Human-readable summary / next step.
    pub note: String,
}

/// Reset a single unit back to a fresh `Pending` state — the per-unit recovery
/// for a wedged or bolt-capped unit.
///
/// A unit's body (its spec) is locked while it executes, so a unit that has run
/// off the rails can't simply be edited and retried. This clears the unit's
/// engine-managed execution state — Pass history (so `pass()` drops to 0),
/// review/approval stamps, input witnesses, gate results, the `revise` flag, and
/// the start/complete timestamps — and flips it to `Pending`, which re-opens the
/// body + structural fields for editing and re-dispatches it from Pass 1.
///
/// **Preserves the unit's identity and spec**: slug, station, dependencies,
/// inputs, outputs, declared `quality_gates`, worker assignment, title, and body.
/// Only the execution *attempt* is wiped, never the unit's definition.
///
/// Dry run by default (reports what it would clear); only mutates when `confirm`
/// is set — mirroring [`crate::reset::reset`]. Idempotent on an already-pending
/// unit (nothing to clear). Resetting a unit other units depend on is the
/// operator's call (the dry-run note flags it); like a station reset, it's an
/// explicit recovery action.
pub fn reset(store: &StateStore, run: &str, slug: &str, confirm: bool) -> Result<UnitResetPlan> {
    let mut unit = get(store, run, slug)?;
    let station = unit.station().to_string();
    let from_status = format!("{:?}", unit.frontmatter.status).to_lowercase();
    let passes = unit.pass();
    let stamps = unit.frontmatter.reviews.len() + unit.frontmatter.approvals.len();
    let gates = unit.frontmatter.gate_results.len();
    // Flag dependents so the operator sees the blast radius before confirming.
    let dependents: Vec<String> = store
        .read_units(run)
        .unwrap_or_default()
        .into_iter()
        .filter(|u| u.slug != slug && u.frontmatter.depends_on.iter().any(|d| d == slug))
        .map(|u| u.slug)
        .collect();

    if confirm {
        unit.frontmatter.status = Status::Pending;
        unit.frontmatter.revise = false;
        unit.frontmatter.reset_requested = false;
        unit.frontmatter.started_at = None;
        unit.frontmatter.completed_at = None;
        unit.frontmatter.iterations.clear();
        unit.frontmatter.reviews.clear();
        unit.frontmatter.approvals.clear();
        unit.frontmatter.input_witnesses.clear();
        unit.frontmatter.gate_results.clear();
        store.write_unit(run, &unit)?;
    }

    let dep_note = if dependents.is_empty() {
        String::new()
    } else {
        format!(" Note: {} unit(s) depend on it ({}) — they may need re-running.", dependents.len(), dependents.join(", "))
    };
    let note = if confirm {
        format!(
            "Reset unit `{slug}` (was {from_status}) to pending — cleared {passes} pass(es), \
             {stamps} stamp(s), {gates} gate result(s). Its body is editable again; the next \
             tick re-dispatches it from Pass 1.{dep_note}"
        )
    } else {
        format!(
            "Dry run: would reset unit `{slug}` (currently {from_status}) to pending — clearing \
             {passes} pass(es), {stamps} stamp(s), {gates} gate result(s). Re-call with \
             confirm:true to apply.{dep_note}"
        )
    };
    Ok(UnitResetPlan {
        run: run.to_string(),
        unit: slug.to_string(),
        station,
        from_status,
        passes_cleared: passes,
        stamps_cleared: stamps,
        gates_cleared: gates,
        confirmed: confirm,
        note,
    })
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

    // Commit early, commit often: the beat stamp publishes on the engine's
    // branch, and the unit's WORKTREE checkpoints (commit + push its branch)
    // so an in-flight Pass loop survives a restart or a cross-machine pickup —
    // the work isn't on the station branch until the terminal land. Both
    // best-effort.
    let _ = crate::commit::commit_state(
        store,
        &format!("darkrun: beat {worker} on {slug} ({result:?})"),
    );
    let station = unit.station().to_string();
    if !station.is_empty() {
        let root = crate::position::cascade_repo_root(store);
        let wt = crate::lifecycle::unit_worktree_path(&root, run, &station, slug);
        let branch = crate::lifecycle::unit_branch(run, &station, slug);
        crate::commit::checkpoint_worktree(
            store,
            &wt,
            &branch,
            &format!("darkrun: checkpoint {slug}"),
        );
    }
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
    // Gate-environment classification: a `fail` whose output reads as a dead
    // dependency (DB down, tool missing, port taken) is not a code defect. Flip
    // it to EnvBlocked *before* the fix loop sees it, so the run routes to a
    // best-effort boot / operator escalation instead of churning fix passes
    // against a broken box. A genuine defect stays `fail`.
    let status = if matches!(status, GateStatus::Fail) {
        let recipe = darkrun_core::boot::read_boot_recipe(store.root()).ok().flatten();
        let tools = recipe
            .as_ref()
            .map(darkrun_core::boot::required_tools)
            .unwrap_or_default();
        let class =
            darkrun_core::gate_env::classify_gate_failure(gate, detail.as_deref().unwrap_or(""), &tools);
        if class.environment {
            GateStatus::EnvBlocked
        } else {
            status
        }
    } else {
        status
    };
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
    let _ = crate::commit::commit_state(store, &format!("darkrun: gate {gate} on {slug}"));
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
    if !outcome.stamped.is_empty() {
        let _ = crate::commit::commit_state(store, &format!("darkrun: stamp {role}"));
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
        let u = create(&store, "r", "u1", "frame", UnitSpec { title: Some("First".into()), ..Default::default() }).unwrap();
        assert_eq!(u.frontmatter.status, Status::Pending);
        assert_eq!(u.station(), "frame");
        assert_eq!(u.title, "First");
    }

    #[test]
    fn create_rejects_duplicate() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", UnitSpec::default()).unwrap();
        let err = create(&store, "r", "u1", "frame", UnitSpec::default()).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn reset_clears_execution_state_but_keeps_the_spec() {
        let (_d, store) = store1();
        // A unit that ran several passes, picked up stamps + a gate result, and
        // wedged InProgress — its body (the spec) is now locked.
        create(&store, "r", "u0", "build", UnitSpec::default()).unwrap();
        let mut u = create(&store, "r", "u1", "build", UnitSpec {
            title: Some("Burst limiter".into()),
            depends_on: vec!["u0".into()],
            ..Default::default()
        }).unwrap();
        u.body = "# Burst limiter\nThe durable spec the operator wants to keep.\n".into();
        u.frontmatter.status = Status::InProgress;
        u.frontmatter.started_at = Some("2026-06-05T00:00:00Z".into());
        u.frontmatter.inputs = vec!["frame/frame.md".into()];
        u.frontmatter.outputs = vec!["src/limiter.rs".into()];
        for n in 0..5 {
            u.frontmatter.iterations.push(UnitIteration {
                worker: format!("w{n}"),
                started_at: None,
                completed_at: None,
                result: Some(IterationResult::Advance),
                note: None,
            });
        }
        u.frontmatter.reviews.insert("correctness".into(), Some(Stamp { at: "2026-06-05T00:00:00Z".into() }));
        u.frontmatter.approvals.insert("user".into(), Some(Stamp { at: "2026-06-05T00:00:00Z".into() }));
        u.frontmatter.input_witnesses.insert("frame/frame.md".into(), "deadbeef".into());
        u.frontmatter.gate_results.push(GateResult {
            name: "tests".into(),
            status: GateStatus::Fail,
            at: None,
            attempts: 3,
            detail: None,
        });
        store.write_unit("r", &u).unwrap();
        assert_eq!(store.read_unit("r", "u1").unwrap().pass(), 5);

        // Dry run reports the blast radius and changes nothing.
        let dry = reset(&store, "r", "u1", false).unwrap();
        assert!(!dry.confirmed);
        assert_eq!(dry.passes_cleared, 5);
        assert_eq!(dry.stamps_cleared, 2);
        assert_eq!(dry.gates_cleared, 1);
        assert_eq!(dry.from_status, "inprogress");
        assert_eq!(store.read_unit("r", "u1").unwrap().pass(), 5, "dry run mutates nothing");

        // Confirmed reset: back to a fresh pending, execution state wiped.
        let done = reset(&store, "r", "u1", true).unwrap();
        assert!(done.confirmed);
        let after = store.read_unit("r", "u1").unwrap();
        assert_eq!(after.frontmatter.status, Status::Pending, "editable again");
        assert_eq!(after.pass(), 0, "pass budget reset");
        assert!(after.frontmatter.reviews.is_empty() && after.frontmatter.approvals.is_empty());
        assert!(after.frontmatter.input_witnesses.is_empty());
        assert!(after.frontmatter.gate_results.is_empty());
        assert!(after.frontmatter.started_at.is_none() && after.frontmatter.completed_at.is_none());
        // The spec + identity survive untouched.
        assert!(after.body.contains("The durable spec the operator wants to keep"));
        assert_eq!(after.title, "Burst limiter");
        assert_eq!(after.frontmatter.depends_on, vec!["u0".to_string()]);
        assert_eq!(after.frontmatter.inputs, vec!["frame/frame.md".to_string()]);
        assert_eq!(after.frontmatter.outputs, vec!["src/limiter.rs".to_string()]);
    }

    #[test]
    fn reset_is_now_pending_so_structural_edits_reopen() {
        let (_d, store) = store1();
        create(&store, "r", "x", "build", UnitSpec::default()).unwrap();
        let mut u = create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        u.frontmatter.status = Status::InProgress;
        store.write_unit("r", &u).unwrap();
        // Structural edit is refused while InProgress…
        let blocked = update(&store, "r", "u1", UnitUpdate { depends_on: Some(vec!["x".into()]), ..Default::default() });
        assert!(blocked.is_err(), "InProgress structural edit must be refused");
        // …and permitted again after a reset.
        reset(&store, "r", "u1", true).unwrap();
        let ok = update(&store, "r", "u1", UnitUpdate { depends_on: Some(vec!["x".into()]), ..Default::default() }).unwrap();
        assert_eq!(ok.frontmatter.depends_on, vec!["x".to_string()]);
    }

    #[test]
    fn create_rejects_empty_slug() {
        let (_d, store) = store1();
        let err = create(&store, "r", " ", "frame", UnitSpec::default()).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn update_advances_status_and_stamps_completion() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "frame", UnitSpec::default()).unwrap();
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
        create(&store, "r", "u1", "frame", UnitSpec::default()).unwrap();
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
        create(&store, "r", "dep", "frame", UnitSpec::default()).unwrap();
        create(&store, "r", "u1", "frame", UnitSpec::default()).unwrap();
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
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
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
        let mut u = create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
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
        let mut u = create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        u.frontmatter.quality_gates = vec![QualityGate { name: "tests".into(), command: "t".into() }];
        store.write_unit("r", &u).unwrap();
        let after = record_gate_result(&store, "r", "u1", "tests", GateStatus::Fail, None, None).unwrap();
        assert!(!after.gates_satisfied());
        assert_eq!(after.unsatisfied_gates(), vec!["tests"]);
    }

    #[test]
    fn a_fail_with_environment_output_auto_flips_to_env_blocked() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        let mut u = create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        u.frontmatter.quality_gates = vec![QualityGate { name: "itest".into(), command: "t".into() }];
        store.write_unit("r", &u).unwrap();

        // A `fail` whose output reads as a dead dependency is reclassified to
        // EnvBlocked (routes to boot/escalate, not the fix loop).
        let after = record_gate_result(
            &store,
            "r",
            "u1",
            "itest",
            GateStatus::Fail,
            Some("Error: connect ECONNREFUSED 127.0.0.1:5432".into()),
            None,
        )
        .unwrap();
        let g = after.frontmatter.gate_results.iter().find(|r| r.name == "itest").unwrap();
        assert_eq!(g.status, GateStatus::EnvBlocked);

        // A `fail` that's a genuine defect stays a `fail`.
        let after2 = record_gate_result(
            &store,
            "r",
            "u1",
            "itest",
            GateStatus::Fail,
            Some("assertion `left == right` failed\n left: 3\n right: 4".into()),
            None,
        )
        .unwrap();
        let g2 = after2.frontmatter.gate_results.iter().find(|r| r.name == "itest").unwrap();
        assert_eq!(g2.status, GateStatus::Fail);
    }

    /// Predecessor BUG-1: the CI-deferral attempt counter was keyed per-UNIT, so
    /// a gate inherited the unit's accumulated count and deferred on its FIRST
    /// failure. darkrun keys attempts PER-GATE-NAME — each gate's counter starts
    /// at its own first appearance, independent of how many times sibling gates
    /// were recorded.
    #[test]
    fn ci_deferral_counter_is_per_gate_not_per_unit() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();

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
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        // The station carries a minted nonce (as it would after Manufacture
        // dispatch). Recording a gate now requires that exact token.
        let mut state = darkrun_core::RunState::default();
        let st = Station {
            station: "build".into(),
            status: Status::InProgress,
            phase: darkrun_core::domain::StationPhase::Manufacture,
            elaborated: false,
            checkpoint: None,
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
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        create(&store, "r", "u2", "build", UnitSpec::default()).unwrap();
        create(&store, "r", "other", "frame", UnitSpec::default()).unwrap();

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
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        let err = record_iteration(&store, "r", "u1", " ", IterationResult::Advance, None, None)
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn reset_notes_dependent_units() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        // u2 depends on u1 → resetting u1 surfaces the dependent note.
        create(&store, "r", "u2", "build", UnitSpec { depends_on: vec!["u1".into()], ..Default::default() }).unwrap();
        let plan = reset(&store, "r", "u1", true).unwrap();
        let note = format!("{plan:?}");
        assert!(note.contains("depend on it") && note.contains("u2"), "{note}");
    }

    #[test]
    fn record_gate_result_rejects_an_empty_gate_name() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        assert!(matches!(
            record_gate_result(&store, "r", "u1", "  ", GateStatus::Pass, None, None),
            Err(McpError::InvalidInput(_))
        ));
    }

    #[test]
    fn stamp_role_rejects_empty_role_and_stamps_approvals() {
        let (_d, store) = store1();
        create(&store, "r", "u1", "build", UnitSpec::default()).unwrap();
        // Empty role is rejected.
        assert!(matches!(
            stamp_role(&store, "r", "build", "  ", StampKind::Approval, &[]),
            Err(McpError::InvalidInput(_))
        ));
        // An Approval stamp lands on the approvals map.
        let out = stamp_role(&store, "r", "build", "user", StampKind::Approval, &[]).unwrap();
        assert_eq!(out.stamped, vec!["u1".to_string()]);
        let u = get(&store, "r", "u1").unwrap();
        assert!(u.frontmatter.approvals.contains_key("user"));
    }

    // ── Spec validators (the predecessor's write-time rules) ───────────────

    #[test]
    fn slug_ok_accepts_url_safe_and_rejects_the_rest() {
        for good in ["u1", "frame-problem", "a.b_c-d", "0start"] {
            assert!(slug_ok(good), "{good} should be a valid slug");
        }
        for bad in ["", "Upper", "has space", "-leads", "uté", "a:b"] {
            assert!(!slug_ok(bad), "{bad} should be rejected");
        }
    }

    #[test]
    fn path_ok_demands_a_real_path_shape() {
        assert!(path_ok("src/limiter.rs"));
        assert!(path_ok("frame/frame.md"));
        for bad in ["", "a path with spaces", "a:b", "a,b"] {
            assert!(!path_ok(bad), "{bad} should be rejected");
        }
    }

    #[test]
    fn gate_is_trivial_catches_circular_assertions() {
        let outs = vec!["docs/report.md".to_string()];
        // Zero-match assertions trivially pass on an empty tree.
        assert!(gate_is_trivial("! grep -rn TODO src/", &outs));
        assert!(gate_is_trivial("!grep broken docs/report.md", &outs));
        // Prose-substring grep against the unit's OWN output: self-certifying.
        assert!(gate_is_trivial("grep -q 'complete' docs/report.md", &outs));
        // A positive behavior-driven check is fine.
        assert!(!gate_is_trivial("cargo test -p api contracts", &outs));
        // Prose grep against someone ELSE's file is not the circular pattern.
        assert!(!gate_is_trivial("grep -q 'done' CHANGELOG.md", &outs));
    }

    #[test]
    fn create_rejects_a_malformed_slug() {
        let (_d, store) = store1();
        let err = create(&store, "r", "Bad Slug", "build", UnitSpec::default()).unwrap_err();
        assert!(format!("{err}").contains("lowercase url-safe"), "{err}");
    }

    #[test]
    fn create_rejects_self_and_unresolved_dependencies() {
        let (_d, store) = store1();
        let selfref = create(&store, "r", "u1", "build", UnitSpec {
            depends_on: vec!["u1".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{selfref}").contains("itself"), "{selfref}");
        let ghost = create(&store, "r", "u2", "build", UnitSpec {
            depends_on: vec!["nope".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{ghost}").contains("does not resolve"), "{ghost}");
    }

    #[test]
    fn update_rejects_a_dependency_cycle() {
        let (_d, store) = store1();
        create(&store, "r", "a", "build", UnitSpec::default()).unwrap();
        create(&store, "r", "b", "build", UnitSpec {
            depends_on: vec!["a".into()],
            ..Default::default()
        })
        .unwrap();
        // a -> b would close the loop b -> a.
        let err = update(&store, "r", "a", UnitUpdate {
            depends_on: Some(vec!["b".into()]),
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{err}").contains("cycle"), "{err}");
    }

    #[test]
    fn create_rejects_a_unit_slug_passed_as_an_input() {
        let (_d, store) = store1();
        create(&store, "r", "producer", "build", UnitSpec::default()).unwrap();
        let err = create(&store, "r", "consumer", "build", UnitSpec {
            inputs: vec!["producer".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{err}").contains("unit slug"), "{err}");
    }

    #[test]
    fn create_demands_the_producer_edge_for_a_sibling_made_input() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        // producer declares the output (gates declared so outputs are legal).
        create(&store, "r", "producer", "build", UnitSpec {
            outputs: vec!["src/api.rs".into()],
            quality_gates: Some(vec![QualityGate {
                name: "tests".into(),
                command: "cargo test -p api".into(),
            }]),
            ..Default::default()
        })
        .unwrap();
        // consumer reads it WITHOUT declaring the edge → bounced.
        let err = create(&store, "r", "consumer", "build", UnitSpec {
            inputs: vec!["src/api.rs".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{err}").contains("depends_on"), "{err}");
        // With the edge declared it lands.
        create(&store, "r", "consumer", "build", UnitSpec {
            inputs: vec!["src/api.rs".into()],
            depends_on: vec!["producer".into()],
            ..Default::default()
        })
        .unwrap();
    }

    #[test]
    fn create_demands_gates_when_outputs_are_declared() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        // Outputs with NO gate declaration → bounced.
        let err = create(&store, "r", "u1", "build", UnitSpec {
            outputs: vec!["src/x.rs".into()],
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{err}").contains("quality_gates"), "{err}");
        // An explicit empty list is a deliberate, visible deferral.
        create(&store, "r", "u1", "build", UnitSpec {
            outputs: vec!["src/x.rs".into()],
            quality_gates: Some(vec![]),
            ..Default::default()
        })
        .unwrap();
        // A trivial circular gate is bounced even when declared.
        let err2 = create(&store, "r", "u2", "build", UnitSpec {
            outputs: vec!["docs/r.md".into()],
            quality_gates: Some(vec![QualityGate {
                name: "done".into(),
                command: "! grep -rn TODO docs/".into(),
            }]),
            ..Default::default()
        })
        .unwrap_err();
        assert!(format!("{err2}").contains("trivially passes"), "{err2}");
    }

    #[test]
    fn create_lands_the_full_spec_on_the_unit_document() {
        use darkrun_core::domain::QualityGate;
        let (_d, store) = store1();
        let body = "# Burst limiter\n\n## Criteria\n- limits bursts -> `cargo test -p limiter` exits 0\n";
        let u = create(&store, "r", "u1", "build", UnitSpec {
            title: Some("Burst limiter".into()),
            body: Some(body.into()),
            inputs: vec!["frame/frame.md".into()],
            outputs: vec!["src/limiter.rs".into()],
            quality_gates: Some(vec![QualityGate {
                name: "tests".into(),
                command: "cargo test -p limiter".into(),
            }]),
            model: Some("opus".into()),
            unit_type: Some("feature".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(u.body.contains("Criteria"), "body is the spec: {}", u.body);
        assert_eq!(u.frontmatter.inputs, vec!["frame/frame.md".to_string()]);
        assert_eq!(u.frontmatter.outputs, vec!["src/limiter.rs".to_string()]);
        assert_eq!(u.frontmatter.quality_gates.len(), 1);
        assert_eq!(u.frontmatter.model.as_deref(), Some("opus"));
        // Re-read from disk: the spec round-trips through the store.
        let back = get(&store, "r", "u1").unwrap();
        assert!(back.body.contains("limits bursts"));
        assert_eq!(back.frontmatter.quality_gates[0].name, "tests");
    }
}

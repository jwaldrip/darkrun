//! Engine-driven prompt wiring tests for the manager.
//!
//! These verify the manager turns each derived `RunAction` into a **rendered,
//! override-resolved** `prompt` string (the engine-driven instructions the
//! agent follows) while keeping the structured `action` intact. They cover:
//!
//! - `run_tick` returns a non-empty `prompt` for every phase action,
//! - the rendered text carries the live context (run / station / kills / units
//!   / worker / reviewers / checkpoint kind),
//! - a project override at `.darkrun/prompts/phases/<x>.md` changes the
//!   rendered output end-to-end *through the manager* (cascade wiring),
//! - overrides of shared partials are honored transitively,
//! - the structured `action` is unchanged by the prompt layer,
//! - `render_prompt` is a pure read (no disk mutation),
//! - run-level actions (sealed) and track actions (fix_feedback / resolve_drift)
//!   render too.

use std::fs;
use std::path::Path;

use darkrun_core::domain::{Mode, Status, StationPhase, Unit, UnitFrontmatter};
use darkrun_core::StateStore;
use darkrun_mcp::position::{
    checkpoint_decide, derive_position, render_prompt, run_start, run_tick, RunAction,
};
use darkrun_mcp::resolve_factory;
use tempfile::TempDir;

// ───────────────────────── helpers ─────────────────────────

/// A run rooted at `repo_root/.darkrun`. We keep the repo root (the tempdir)
/// so tests can drop overrides at `<repo_root>/.darkrun/prompts/...`.
fn fresh(slug: &str) -> (TempDir, StateStore) {
    let dir = TempDir::new().expect("tmp");
    let store = StateStore::new(dir.path());
    run_start(&store, slug, "software", None, Mode::Solo, "full").expect("start");
    (dir, store)
}

/// Drop a project prompt override at `<repo_root>/.darkrun/prompts/<rel>.md`.
fn write_override(repo_root: &Path, rel: &str, body: &str) {
    let path = repo_root
        .join(".darkrun")
        .join("prompts")
        .join(format!("{rel}.md"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

const STATIONS: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

const PHASES: [StationPhase; 6] = [
    StationPhase::Spec,
    StationPhase::Review,
    StationPhase::Manufacture,
    StationPhase::Audit,
    StationPhase::Reflect,
    StationPhase::Checkpoint,
];

fn seed_unit(store: &StateStore, run: &str, station: &str, slug: &str, status: Status) {
    let inputs = darkrun_mcp::resolve_factory("software")
        .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
        .unwrap_or_default();
    let unit = Unit {
        slug: slug.into(),
        frontmatter: UnitFrontmatter {
            status,
            station: Some(station.into()),
            inputs,
            ..Default::default()
        },
        title: slug.into(),
        body: String::new(),
    };
    store.write_unit(run, &unit).expect("write unit");
}

fn set_phase(store: &StateStore, run: &str, station: &str, phase: StationPhase) {
    let mut state = store.read_state(run).expect("state").expect("some");
    let st = state.stations.get_mut(station).expect("station seeded");
    st.phase = phase;
    if !matches!(st.status, Status::Completed) {
        st.status = Status::InProgress;
    }
    store.write_state(run, &state).expect("write state");
}

/// Force the run cursor onto `target`'s `Spec` with all prior stations
/// completed, then stamp the requested phase. Mirrors the manager_phases
/// fixture but trimmed to what these tests need.
fn at_phase(store: &StateStore, run: &str, target: &str, phase: StationPhase) {
    let factory = resolve_factory("software").expect("factory");
    let mut state = store.read_state(run).expect("state").unwrap_or_default();
    let gate = state.mode.gate();
    for station in STATIONS {
        if station == target {
            break;
        }
        let _ = factory.station(station).expect("def");
        state.stations.insert(
            station.to_string(),
            darkrun_core::domain::Station {
                station: station.to_string(),
                status: Status::Completed,
                phase: StationPhase::Checkpoint,
            elaborated: false,
                checkpoint: Some(darkrun_core::domain::Checkpoint {
                    kind: gate,
                    entered_at: None,
                    outcome: Some(darkrun_core::domain::CheckpointOutcome::Advanced),
                }),
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: None,
                completed_at: None,
            },
        );
    }
    let _ = factory.station(target).expect("target def");
    state.stations.insert(
        target.to_string(),
        darkrun_core::domain::Station {
            station: target.to_string(),
            status: Status::InProgress,
            phase,
            elaborated: false,
            checkpoint: Some(darkrun_core::domain::Checkpoint {
                kind: gate,
                entered_at: None,
                outcome: None,
            }),
            branch: None,
            pr_ref: None,
            pr_status: None,
            pr_ready_at: None,
            pr_merged_at: None,
            verifier_nonce: None,
            started_at: None,
            completed_at: None,
        },
    );
    state.active_station = target.to_string();
    store.write_state(run, &state).expect("write state");
}

// ─────────── every phase yields a non-empty rendered prompt ───────────

macro_rules! phase_renders_prompt_test {
    ($name:ident, $station:expr, $phase:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, $phase);
            if matches!($phase, StationPhase::Manufacture) {
                seed_unit(&store, "r", $station, "wu", Status::Pending);
            }
            let t = run_tick(&store, "r").expect("tick");
            let prompt = t.prompt.expect("rendered prompt present");
            assert!(!prompt.trim().is_empty(), "prompt empty for {} {:?}", $station, $phase);
            // The run banner from _shared/announcement.md always shows the run.
            assert!(prompt.contains("`r`"), "prompt missing run slug:\n{prompt}");
            // The station name appears in every phase template.
            assert!(prompt.contains($station), "prompt missing station {}:\n{prompt}", $station);
        }
    };
}

phase_renders_prompt_test!(frame_spec_renders, "frame", StationPhase::Spec);
phase_renders_prompt_test!(frame_review_renders, "frame", StationPhase::Review);
phase_renders_prompt_test!(frame_manufacture_renders, "frame", StationPhase::Manufacture);
phase_renders_prompt_test!(frame_audit_renders, "frame", StationPhase::Audit);
phase_renders_prompt_test!(frame_reflect_renders, "frame", StationPhase::Reflect);
phase_renders_prompt_test!(frame_checkpoint_renders, "frame", StationPhase::Checkpoint);

phase_renders_prompt_test!(specify_spec_renders, "specify", StationPhase::Spec);
phase_renders_prompt_test!(specify_review_renders, "specify", StationPhase::Review);
phase_renders_prompt_test!(specify_manufacture_renders, "specify", StationPhase::Manufacture);
phase_renders_prompt_test!(specify_audit_renders, "specify", StationPhase::Audit);
phase_renders_prompt_test!(specify_reflect_renders, "specify", StationPhase::Reflect);
phase_renders_prompt_test!(specify_checkpoint_renders, "specify", StationPhase::Checkpoint);

phase_renders_prompt_test!(shape_spec_renders, "shape", StationPhase::Spec);
phase_renders_prompt_test!(shape_review_renders, "shape", StationPhase::Review);
phase_renders_prompt_test!(shape_manufacture_renders, "shape", StationPhase::Manufacture);
phase_renders_prompt_test!(shape_audit_renders, "shape", StationPhase::Audit);
phase_renders_prompt_test!(shape_reflect_renders, "shape", StationPhase::Reflect);
phase_renders_prompt_test!(shape_checkpoint_renders, "shape", StationPhase::Checkpoint);

phase_renders_prompt_test!(build_spec_renders, "build", StationPhase::Spec);
phase_renders_prompt_test!(build_review_renders, "build", StationPhase::Review);
phase_renders_prompt_test!(build_manufacture_renders, "build", StationPhase::Manufacture);
phase_renders_prompt_test!(build_audit_renders, "build", StationPhase::Audit);
phase_renders_prompt_test!(build_reflect_renders, "build", StationPhase::Reflect);
phase_renders_prompt_test!(build_checkpoint_renders, "build", StationPhase::Checkpoint);

phase_renders_prompt_test!(prove_spec_renders, "prove", StationPhase::Spec);
phase_renders_prompt_test!(prove_manufacture_renders, "prove", StationPhase::Manufacture);
phase_renders_prompt_test!(prove_reflect_renders, "prove", StationPhase::Reflect);

phase_renders_prompt_test!(harden_spec_renders, "harden", StationPhase::Spec);
phase_renders_prompt_test!(harden_review_renders, "harden", StationPhase::Review);
phase_renders_prompt_test!(harden_checkpoint_renders, "harden", StationPhase::Checkpoint);

// ─────────── context vars: kills / worker / reviewers / kind ───────────

macro_rules! spec_kills_in_prompt_test {
    ($name:ident, $station:expr, $kills:expr) => {
        #[test]
        fn $name() {
            let (_d, store) = fresh("r");
            at_phase(&store, "r", $station, StationPhase::Spec);
            let t = run_tick(&store, "r").expect("tick");
            let prompt = t.prompt.expect("prompt");
            assert!(prompt.contains($kills), "spec prompt for {} missing kills `{}`:\n{prompt}", $station, $kills);
        }
    };
}
spec_kills_in_prompt_test!(frame_spec_kills, "frame", "wrong-thing");
spec_kills_in_prompt_test!(specify_spec_kills, "specify", "ambiguity");
spec_kills_in_prompt_test!(shape_spec_kills, "shape", "expensive-structural-reversal");
spec_kills_in_prompt_test!(build_spec_kills, "build", "implementation-defects");
spec_kills_in_prompt_test!(prove_spec_kills, "prove", "escaped-defects");
spec_kills_in_prompt_test!(harden_spec_kills, "harden", "works-in-dev-dies-in-prod");

#[test]
fn manufacture_prompt_lists_units_and_worker() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "build", StationPhase::Manufacture);
    seed_unit(&store, "r", "build", "alpha", Status::Pending);
    seed_unit(&store, "r", "build", "beta", Status::Pending);
    let t = run_tick(&store, "r").expect("tick");
    // Structured side still carries the wave.
    match &t.action {
        RunAction::Manufacture { worker, units, .. } => {
            assert_eq!(worker, "test_author");
            assert!(units.contains(&"alpha".to_string()));
            assert!(units.contains(&"beta".to_string()));
        }
        other => panic!("expected Manufacture, got {other:?}"),
    }
    // Rendered side surfaces the same worker beat and unit slugs.
    let prompt = t.prompt.expect("prompt");
    assert!(prompt.contains("test_author"), "worker beat missing:\n{prompt}");
    assert!(prompt.contains("alpha"), "unit alpha missing:\n{prompt}");
    assert!(prompt.contains("beta"), "unit beta missing:\n{prompt}");
}

#[test]
fn manufacture_prompt_carries_each_units_full_spec() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "build", StationPhase::Manufacture);
    // A unit with a REAL spec: body, paths, and a quality gate. The dispatch
    // must thread the whole definition — the worker subagent has no other
    // context, so a slug-only dispatch is how thin work happens.
    // (It consumes the station's declared inputs so the decomposition is valid.)
    let station_inputs = darkrun_mcp::resolve_factory("software")
        .and_then(|f| f.station("build").map(|d| d.inputs.clone()))
        .unwrap_or_default();
    let unit = Unit {
        slug: "alpha".into(),
        frontmatter: UnitFrontmatter {
            status: Status::Pending,
            station: Some("build".into()),
            inputs: station_inputs,
            outputs: vec!["src/limiter.rs".into()],
            quality_gates: vec![darkrun_core::domain::QualityGate {
                name: "tests".into(),
                command: "cargo test -p limiter".into(),
            }],
            ..Default::default()
        },
        title: "Burst limiter".into(),
        body: "# Burst limiter\n\n## Criteria\n- bursts above N are limited \
               -> `cargo test -p limiter` exits 0\n\n## Out of scope\n- distributed limits\n"
            .into(),
    };
    store.write_unit("r", &unit).expect("write unit");
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    assert!(prompt.contains("Burst limiter"), "spec title missing:\n{prompt}");
    assert!(
        prompt.contains("bursts above N are limited"),
        "spec body (the contract) missing:\n{prompt}"
    );
    assert!(prompt.contains("Out of scope"), "scope boundary missing:\n{prompt}");
    assert!(
        prompt.contains("cargo test -p limiter"),
        "quality-gate command missing:\n{prompt}"
    );
    assert!(prompt.contains("src/limiter.rs"), "output path missing:\n{prompt}");
}

#[test]
fn review_prompt_lists_reviewers() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "build", StationPhase::Review);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    // build's reviewers are correctness + maintainability (roster partial).
    assert!(prompt.contains("correctness"), "reviewer missing:\n{prompt}");
    assert!(prompt.contains("maintainability"), "reviewer missing:\n{prompt}");
}

#[test]
fn checkpoint_prompt_branches_on_kind() {
    // `ask` gate (frame) → human-approval copy.
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Checkpoint);
    let t = run_tick(&store, "r").expect("tick");
    let ask = t.prompt.expect("prompt");
    assert!(ask.contains("a human must approve"), "ask copy missing:\n{ask}");

    // `auto` gate (a downgraded build gate) → no-human copy.
    let (_d2, store2) = fresh("r2");
    at_phase(&store2, "r2", "build", StationPhase::Checkpoint);
    let mut s = store2.read_state("r2").unwrap().unwrap();
    s.mode = Mode::Dark;
    store2.write_state("r2", &s).unwrap();
    let t2 = run_tick(&store2, "r2").expect("tick");
    let auto = t2.prompt.expect("prompt");
    assert!(auto.contains("no human in the loop"), "auto copy missing:\n{auto}");
    assert!(!auto.contains("a human must approve"));
}

#[test]
fn checkpoint_prompt_external_for_discrete_station() {
    // Every station gates `ask` by default; the external-review copy renders for
    // a discrete run (where the gate resolves External).
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "harden", StationPhase::Checkpoint);
    let mut s = store.read_state("r").unwrap().unwrap();
    s.mode = Mode::Team;
    store.write_state("r", &s).unwrap();
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    assert!(prompt.contains("external"), "external gate copy missing:\n{prompt}");
}

#[test]
fn spec_prompt_lists_on_record_units() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Spec);
    seed_unit(&store, "r", "frame", "already-here", Status::Pending);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    // The spec template lists units already on record.
    assert!(prompt.contains("already-here"), "on-record unit missing:\n{prompt}");
}

#[test]
fn reflect_prompt_is_a_retrospective() {
    // The Reflect phase (5th, before Checkpoint) renders a retrospective that
    // captures learnings feeding the run-level reflections.
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Reflect);
    let t = run_tick(&store, "r").expect("tick");
    // Structured side carries the Reflect action.
    assert!(
        matches!(&t.action, RunAction::Reflect { station, .. } if station == "frame"),
        "expected Reflect, got {:?}",
        t.action
    );
    let prompt = t.prompt.expect("reflect prompt").to_lowercase();
    // The rendered prompt frames an autonomous retrospective.
    assert!(prompt.contains("reflect"), "reflect prompt missing reflect framing");
    assert!(
        prompt.contains("retrospective") || prompt.contains("learnings") || prompt.contains("reflection"),
        "reflect prompt should read as a retrospective:\n{prompt}"
    );
}

#[test]
fn audit_prompt_folds_in_the_tests_work() {
    // Audit now covers what the old Tests phase did: it both verifies against
    // the spec AND runs the quality checks. The rendered audit prompt must
    // surface the checks/tests work, not just reviewer judgment.
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Audit);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("audit prompt").to_lowercase();
    assert!(prompt.contains("check") || prompt.contains("test"), "audit prompt should fold in the checks/tests work:\n{prompt}");
}

// ─────────── override cascade END-TO-END through the manager ───────────

#[test]
fn project_override_changes_rendered_prompt_through_manager() {
    let (dir, store) = fresh("r");
    // Override the spec phase template. The render must pick this up because the
    // manager resolves the cascade against repo_root = <.darkrun>/.. .
    write_override(
        dir.path(),
        "phases/spec",
        "CUSTOM SPEC for {{ station }} killing {{ kills }}",
    );
    at_phase(&store, "r", "frame", StationPhase::Spec);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    assert!(
        prompt.starts_with("CUSTOM SPEC for frame killing wrong-thing"),
        "override not honored through manager:\n{prompt}"
    );
    // Embedded default copy is gone.
    assert!(!prompt.contains("run the explorers in parallel"), "embedded default leaked:\n{prompt}");
    // The structured action is untouched by the override.
    assert!(matches!(&t.action, RunAction::Spec { station, .. } if station == "frame"));
}

#[test]
fn override_of_shared_partial_is_honored_via_include_through_manager() {
    let (dir, store) = fresh("r");
    // Override only the shared contracts partial; the top-level spec template is
    // still embedded but pulls the override in through `{% include %}`.
    write_override(dir.path(), "_shared/contracts", "MY CUSTOM CONTRACT BLOCK");
    at_phase(&store, "r", "frame", StationPhase::Spec);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    assert!(prompt.contains("MY CUSTOM CONTRACT BLOCK"), "partial override not honored:\n{prompt}");
    assert!(!prompt.contains("source of truth"), "embedded contract leaked:\n{prompt}");
}

#[test]
fn override_uses_live_context_vars() {
    let (dir, store) = fresh("r");
    write_override(
        dir.path(),
        "phases/manufacture",
        "BEAT={{ worker }} UNITS={% for u in units %}[{{ u }}]{% endfor %}",
    );
    at_phase(&store, "r", "build", StationPhase::Manufacture);
    seed_unit(&store, "r", "build", "one", Status::Pending);
    seed_unit(&store, "r", "build", "two", Status::Pending);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    assert!(prompt.contains("BEAT=test_author"), "worker var missing:\n{prompt}");
    assert!(prompt.contains("[one]"), "unit one missing:\n{prompt}");
    assert!(prompt.contains("[two]"), "unit two missing:\n{prompt}");
}

#[test]
fn removing_override_falls_back_to_embedded_through_manager() {
    let (dir, store) = fresh("r");
    let rel_path = dir.path().join(".darkrun/prompts/phases/spec.md");
    write_override(dir.path(), "phases/spec", "TEMP OVERRIDE {{ station }}");
    at_phase(&store, "r", "frame", StationPhase::Spec);
    let t1 = run_tick(&store, "r").expect("tick");
    assert!(t1.prompt.unwrap().contains("TEMP OVERRIDE frame"));

    // Remove the override and re-derive at the same phase → embedded default.
    fs::remove_file(&rel_path).unwrap();
    set_phase(&store, "r", "frame", StationPhase::Spec);
    let t2 = run_tick(&store, "r").expect("tick");
    let prompt = t2.prompt.expect("prompt");
    assert!(prompt.contains("run the explorers in parallel"), "did not fall back to embedded:\n{prompt}");
}

#[test]
fn spec_prompt_runs_discovery_and_elaboration_in_tandem() {
    // Mirrors the predecessor's elaborate_loop: when a station opens, the agent
    // dispatches the explorers IN PARALLEL while it frames the problem — discovery
    // and elaboration run in tandem, not sequentially.
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Spec);
    let t = run_tick(&store, "r").expect("tick");
    let prompt = t.prompt.expect("prompt");
    // The frame station's explorers are listed for the parallel fan-out.
    assert!(prompt.contains("context"), "explorer `context` not surfaced:\n{prompt}");
    // The dynamic is explicitly tandem + parallel, not "explore then decompose".
    assert!(prompt.contains("in tandem"), "tandem dynamic missing:\n{prompt}");
    assert!(
        prompt.contains("run the explorers in parallel"),
        "parallel explorer fan-out missing:\n{prompt}"
    );
}

// ─────────── structured action is preserved alongside the prompt ───────────

#[test]
fn structured_action_unchanged_by_prompt_layer() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "specify", StationPhase::Review);
    let t = run_tick(&store, "r").expect("tick");
    // Both halves present: structured action AND rendered prompt.
    assert!(t.prompt.is_some(), "prompt should be present");
    match &t.action {
        RunAction::Review { run, station, reviewers } => {
            assert_eq!(run, "r");
            assert_eq!(station, "specify");
            assert_eq!(reviewers, &vec!["testability".to_string(), "completeness".to_string()]);
        }
        other => panic!("expected Review, got {other:?}"),
    }
    // The position is still carried too.
    assert!(t.position.action.is_some());
}

// ─────────── render_prompt is a pure read (no disk mutation) ───────────

#[test]
fn render_prompt_does_not_mutate_disk() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Review);
    let action = derive_position(&store, "r").unwrap().action.unwrap();
    let before = store.read_state("r").unwrap().unwrap();
    let _ = render_prompt(&store, "r", &action).expect("render");
    let _ = render_prompt(&store, "r", &action).expect("render again");
    let after = store.read_state("r").unwrap().unwrap();
    assert_eq!(before.active_station, after.active_station);
    assert_eq!(before.stations["frame"].phase, after.stations["frame"].phase);
}

#[test]
fn render_prompt_is_deterministic() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "build", StationPhase::Manufacture);
    seed_unit(&store, "r", "build", "u", Status::Pending);
    let action = derive_position(&store, "r").unwrap().action.unwrap();
    let a = render_prompt(&store, "r", &action).unwrap();
    let b = render_prompt(&store, "r", &action).unwrap();
    assert_eq!(a, b, "same action + disk → same rendered prompt");
}

// ─────────── run-level + track actions render ───────────

#[test]
fn sealed_action_renders_run_completion_prompt() {
    let (_d, store) = fresh("r");
    // Mark every station complete → sealed.
    let factory = resolve_factory("software").unwrap();
    let mut state = store.read_state("r").unwrap().unwrap();
    for s in &factory.stations {
        state.stations.insert(
            s.name.clone(),
            darkrun_core::domain::Station {
                station: s.name.clone(),
                status: Status::Completed,
                phase: StationPhase::Checkpoint,
            elaborated: false,
                checkpoint: None,
                branch: None,
                pr_ref: None,
                pr_status: None,
                pr_ready_at: None,
                pr_merged_at: None,
                verifier_nonce: None,
                started_at: None,
                completed_at: None,
            },
        );
    }
    store.write_state("r", &state).unwrap();
    // The whole-run review gates before seal; sign the run reviewers off.
    for r in &factory.run_reviewers {
        darkrun_mcp::position::run_review_stamp(&store, "r", r).expect("run review stamp");
    }
    let t = run_tick(&store, "r").expect("tick");
    assert!(matches!(&t.action, RunAction::Sealed { .. }));
    let prompt = t.prompt.expect("sealed prompt");
    assert!(!prompt.trim().is_empty(), "sealed prompt empty");
    assert!(prompt.contains("`r`"), "sealed prompt missing run slug:\n{prompt}");
}

#[test]
fn fix_feedback_action_renders_track_prompt() {
    let (_d, store) = fresh("r");
    store
        .write_feedback_raw("r", "fb-7", "---\nstatus: pending\n---\nbusted\n")
        .expect("fb");
    let t = run_tick(&store, "r").expect("tick");
    match &t.action {
        RunAction::FixFeedback { feedback_id, .. } => assert_eq!(feedback_id, "fb-7"),
        other => panic!("expected FixFeedback, got {other:?}"),
    }
    let prompt = t.prompt.expect("fix prompt");
    assert!(prompt.contains("fb-7"), "feedback id missing from track prompt:\n{prompt}");
}


// ─────────── noop action renders the mid-wave hold prompt ───────────

#[test]
fn noop_action_renders_message_prompt() {
    let (_d, store) = fresh("r");
    at_phase(&store, "r", "frame", StationPhase::Manufacture);
    // A dispatched, in-flight unit → mid-wave noop. (A dangling dep would be a
    // UnitsInvalid decomposition error.)
    let unit = Unit {
        slug: "inflight".into(),
        frontmatter: UnitFrontmatter {
            status: Status::InProgress,
            station: Some("frame".into()),
            ..Default::default()
        },
        title: "inflight".into(),
        body: String::new(),
    };
    store.write_unit("r", &unit).expect("write unit");
    let t = run_tick(&store, "r").expect("tick");
    assert!(matches!(&t.action, RunAction::Noop { .. }));
    let prompt = t.prompt.expect("noop prompt");
    assert!(!prompt.trim().is_empty(), "noop prompt empty");
    // The noop template echoes the message.
    assert!(prompt.contains("Mid-wave"), "noop message missing:\n{prompt}");
}

// ─────────── full walk: every tick carries a prompt ───────────

#[test]
fn every_tick_through_a_station_carries_a_prompt() {
    let (_d, store) = fresh("r");
    seed_unit(&store, "r", "frame", "u", Status::Completed);
    // Walk frame: spec → review → user_gate → audit → reflect → checkpoint. Each
    // tick must carry a non-empty rendered prompt alongside its structured action.
    for _ in 0..10 {
        let t = run_tick(&store, "r").expect("tick");
        let prompt = t.prompt.expect("prompt on every tick");
        assert!(!prompt.trim().is_empty(), "empty prompt for action {:?}", t.action);
        if matches!(t.action, RunAction::UserGate { .. }) {
            // Clear the pre-execution operator gate so the walk continues.
            checkpoint_decide(&store, "r", true, None).expect("clear gate");
            continue;
        }
        if matches!(t.action, RunAction::Checkpoint { .. }) {
            break;
        }
    }
    // Approve the ask gate and confirm the next station's spec prompt renders.
    let decided = checkpoint_decide(&store, "r", true, None).expect("approve");
    assert!(decided.prompt.expect("prompt").contains("specify"));
}

// ─────────── coverage sanity: PHASES constant is exercised ───────────

#[test]
fn phases_constant_is_complete() {
    assert_eq!(PHASES.len(), 6);
    assert_eq!(PHASES[0], StationPhase::Spec);
    assert_eq!(PHASES[5], StationPhase::Checkpoint);
}

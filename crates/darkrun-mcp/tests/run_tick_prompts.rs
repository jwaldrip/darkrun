//! End-to-end: `run_tick` renders the active phase's engine-driven prompt and
//! a project `.darkrun/prompts` override changes that prompt end-to-end.
//!
//! These exercise the seam between the manager (`darkrun-mcp`) and the prompt
//! engine (`darkrun-prompts`): the manager derives a structured `RunAction`,
//! maps it to a template key, builds a live `PromptContext` from the resolved
//! station, and renders it through the override cascade — returning the markdown
//! on `TickResult::prompt` alongside the structured `action`.

use std::fs;
use std::path::Path;

use darkrun_core::domain::{Status, Unit, UnitFrontmatter};
use darkrun_core::StateStore;
use darkrun_mcp::{checkpoint_decide, run_start, run_tick, RunAction, TickResult};
use tempfile::TempDir;

/// A store whose repo root is the tempdir, so `.darkrun/prompts` overrides
/// resolve against `<tempdir>/.darkrun/prompts`.
fn fresh() -> (TempDir, StateStore) {
    let dir = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(dir.path());
    (dir, store)
}

/// Drop a project override at `<repo_root>/.darkrun/prompts/<rel>.md`.
fn write_override(repo_root: &Path, rel: &str, body: &str) {
    let path = repo_root
        .join(".darkrun")
        .join("prompts")
        .join(format!("{rel}.md"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

/// Pull the rendered prompt off a tick, asserting it is present and non-empty —
/// every action in the current vocabulary maps to a template, so the manager
/// must always render one.
fn prompt(tick: &TickResult) -> &str {
    let p = tick
        .prompt
        .as_deref()
        .expect("every emitted action renders a prompt");
    assert!(!p.trim().is_empty(), "rendered prompt was empty");
    p
}

#[test]
fn run_tick_renders_a_prompt_for_every_phase() {
    let (dir, store) = fresh();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // ── Spec ────────────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("spec tick");
    assert!(matches!(t.action, RunAction::Spec { .. }));
    let p = prompt(&t);
    assert!(p.contains("# Spec — `frame`"), "spec heading missing:\n{p}");
    // Spec walks elaborate → explore.
    assert!(p.contains("elaborate") && p.contains("explore"), "spec beats:\n{p}");
    // The announcement partial rendered with the live run/station.
    assert!(p.contains("`r`") && p.contains("`frame`"));

    // ── Review ───────────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("review tick");
    assert!(matches!(t.action, RunAction::Review { .. }));
    let p = prompt(&t);
    assert!(p.contains("# Review"), "review heading:\n{p}");
    // Review walks spec → adversarial → brief → user.
    for beat in ["adversarial", "brief", "user"] {
        assert!(p.contains(beat), "review beat `{beat}` missing:\n{p}");
    }
    // The frame station's reviewers come from the resolved station def.
    assert!(p.contains("Reviewers"), "roster reviewers missing:\n{p}");

    // Decompose a unit so Manufacture has something wave-ready.
    write_unit(&store, "r", "u1", Status::Pending);

    // ── Manufacture ─────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("manufacture tick");
    let worker = match &t.action {
        RunAction::Manufacture { worker, units, .. } => {
            assert_eq!(units, &vec!["u1".to_string()]);
            worker.clone()
        }
        other => panic!("expected Manufacture, got {other:?}"),
    };
    let p = prompt(&t);
    assert!(p.contains("# Manufacture"), "manufacture heading:\n{p}");
    // The current worker beat is interpolated, and the make→challenge→resolve
    // loop is named.
    assert!(p.contains(&worker), "worker beat `{worker}` missing:\n{p}");
    for beat in ["make", "challenge", "resolve"] {
        assert!(p.contains(beat), "pass-loop beat `{beat}` missing:\n{p}");
    }
    assert!(p.contains("`u1`"), "wave-ready unit missing:\n{p}");

    // Lock the unit → next tick audits.
    write_unit(&store, "r", "u1", Status::Completed);

    // ── Audit ────────────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("audit tick");
    assert!(matches!(t.action, RunAction::Audit { .. }));
    let p = prompt(&t);
    assert!(p.contains("# Audit"), "audit heading:\n{p}");
    // Audit folds tests in: it walks spec → adversarial and runs the checks.
    assert!(p.contains("spec") && p.contains("adversarial"), "audit beats:\n{p}");
    assert!(
        p.to_lowercase().contains("check") || p.to_lowercase().contains("test"),
        "audit must mention running the checks/tests:\n{p}"
    );

    // ── Reflect ──────────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("reflect tick");
    assert!(matches!(t.action, RunAction::Reflect { .. }));
    let p = prompt(&t);
    assert!(p.contains("# Reflect"), "reflect heading:\n{p}");
    assert!(p.contains("agentic"), "reflect agentic beat missing:\n{p}");

    // ── Checkpoint ───────────────────────────────────────────────────────────
    let t = run_tick(&store, "r").expect("checkpoint tick");
    assert!(matches!(t.action, RunAction::Checkpoint { .. }));
    let p = prompt(&t);
    assert!(p.contains("# Checkpoint"), "checkpoint heading:\n{p}");
    // Checkpoint walks brief → user; frame's gate is `ask`.
    assert!(p.contains("brief") && p.contains("user"), "checkpoint beats:\n{p}");
    assert!(p.contains("a human must approve"), "ask-gate copy missing:\n{p}");
    // Regression: the old "passed ... and tests" line must be gone (Tests is
    // folded into audit; the phase before checkpoint is reflect).
    assert!(
        !p.contains("audit, and tests"),
        "checkpoint still references the removed Tests phase:\n{p}"
    );

    drop(dir);
}

#[test]
fn project_override_changes_run_tick_output_end_to_end() {
    let (dir, store) = fresh();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Baseline: the embedded spec template renders the default heading.
    let baseline = run_tick(&store, "r").expect("baseline tick");
    let base_prompt = prompt(&baseline);
    assert!(base_prompt.contains("# Spec — `frame`"));
    assert!(!base_prompt.contains("CUSTOM-SPEC-OVERRIDE"));

    // Restart cleanly so the next tick is Spec again, then drop an override that
    // still consumes the live context (`station`) to prove rendering, not just
    // pass-through, runs against the override.
    let (dir2, store2) = fresh();
    run_start(&store2, "r2", "software", None, "continuous").expect("start2");
    write_override(
        dir2.path(),
        "phases/spec",
        "CUSTOM-SPEC-OVERRIDE for {{ station }} killing {{ kills }}",
    );

    let overridden = run_tick(&store2, "r2").expect("override tick");
    let over_prompt = prompt(&overridden);
    assert!(
        over_prompt.starts_with("CUSTOM-SPEC-OVERRIDE for frame"),
        "override did not take effect end-to-end:\n{over_prompt}"
    );
    // The embedded default is fully replaced.
    assert!(!over_prompt.contains("# Spec — `frame`"));
    // The live context still rendered through the override.
    assert!(over_prompt.contains("killing"));

    drop((dir, dir2));
}

#[test]
fn override_of_shared_partial_flows_through_run_tick() {
    let (dir, store) = fresh();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Override only the shared contracts partial; the top-level spec template is
    // still embedded and includes it — the cascade must honor the override
    // transitively through `{% include %}`.
    write_override(dir.path(), "_shared/contracts", "SHARED-PARTIAL-OVERRIDE");

    let t = run_tick(&store, "r").expect("spec tick");
    let p = prompt(&t);
    assert!(p.contains("# Spec — `frame`"), "top-level still embedded:\n{p}");
    assert!(
        p.contains("SHARED-PARTIAL-OVERRIDE"),
        "included partial override did not flow through:\n{p}"
    );

    drop(dir);
}

#[test]
fn checkpoint_decide_retick_carries_rendered_prompt() {
    let (dir, store) = fresh();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Reject the checkpoint → files feedback, which preempts the run track. The
    // re-tick must still render the fix-feedback track prompt.
    let res = checkpoint_decide(&store, "r", false, Some("needs rework".into())).expect("decide");
    assert!(matches!(res.action, RunAction::FixFeedback { .. }));
    let p = prompt(&res);
    assert!(p.contains("# Fix Feedback"), "fix-feedback prompt missing:\n{p}");

    drop(dir);
}

#[test]
fn sealed_run_renders_the_sealed_prompt() {
    let (dir, store) = fresh();
    run_start(&store, "r", "software", None, "continuous").expect("start");

    // Walk every station to completion by approving each auto-or-asked gate.
    // The software factory has 6 stations; drive each through its phases.
    for _ in 0..6 {
        // Spec → Review → (Manufacture w/ a unit) → Audit → Reflect → Checkpoint.
        let station = active_station(&store, "r");
        run_tick(&store, "r").expect("spec"); // Spec → Review
        run_tick(&store, "r").expect("review"); // Review → Manufacture
        write_unit(&store, "r", &format!("{station}-u"), Status::Completed);
        run_tick(&store, "r").expect("audit"); // all units locked → Audit → Reflect
        run_tick(&store, "r").expect("reflect"); // Reflect → Checkpoint
        run_tick(&store, "r").expect("checkpoint"); // gate holds (ask) or auto-advances
        // Approve whatever gate is holding to advance to the next station.
        let _ = checkpoint_decide(&store, "r", true, None);
    }

    // Every station locked → the run is sealed, and the sealed prompt renders.
    let t = run_tick(&store, "r").expect("sealed tick");
    assert!(matches!(t.action, RunAction::Sealed { .. }), "expected Sealed, got {:?}", t.action);
    let p = prompt(&t);
    assert!(p.contains("Sealed"), "sealed prompt missing:\n{p}");

    drop(dir);
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn write_unit(store: &StateStore, run: &str, slug: &str, status: Status) {
    let unit = Unit {
        slug: slug.to_string(),
        frontmatter: UnitFrontmatter {
            status,
            station: Some(active_station(store, run)),
            ..Default::default()
        },
        title: slug.to_string(),
        body: String::new(),
    };
    store.write_unit(run, &unit).expect("write unit");
}

fn active_station(store: &StateStore, run: &str) -> String {
    store
        .read_state(run)
        .expect("state")
        .expect("some")
        .active_station
}

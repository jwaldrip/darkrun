//! External-Checkpoint → change-request intent (the PR/MR handoff).
//!
//! The **external** Checkpoint kind hands a Station's work off to a human via a
//! Pull Request (GitHub) or Merge Request (GitLab). The manager stays a pure
//! read: it never touches the network. This module derives the *intent* — the
//! title, body, and branch facts — that the side-effecting CLI/tool layer feeds
//! to `darkrun-vcs::create_change_request`.
//!
//! Separation of concerns:
//!
//! - **here (pure):** read run/state, confirm the active station's checkpoint is
//!   `external`, and assemble a [`ChangeRequestIntent`] describing what to open.
//! - **CLI (side-effecting):** resolve the git remote, load the stored
//!   credential, and POST the PR/MR.

use darkrun_core::StateStore;
use serde::{Deserialize, Serialize};

use crate::error::{McpError, Result};
use crate::position::derive_position;
use crate::position::RunAction;

use crate::lifecycle::{run_main_branch, station_branch};

/// A fully-derived description of the change request an external Checkpoint
/// wants opened. Pure data — no network, no credentials, no git remote.
///
/// Under the branch hierarchy a DISCRETE station PR runs station-branch
/// (`darkrun/<slug>/<station>`) -> run-main (`darkrun/<slug>/main`) as a DRAFT;
/// the run-completion PR runs run-main -> the provider default. The CLI combines
/// this with the repo's parsed remote coordinates and the stored credential to
/// actually open the PR/MR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequestIntent {
    /// The run this change request delivers.
    pub run: String,
    /// The station whose external Checkpoint triggered the handoff.
    pub station: String,
    /// The source branch carrying the work (the PR/MR head) — the station
    /// branch for a discrete station PR.
    pub head: String,
    /// The branch the change request targets (the PR/MR base) — run-main for a
    /// discrete station PR. Empty when the CLI should resolve the provider
    /// default branch (the run-completion PR).
    #[serde(default)]
    pub base: String,
    /// Whether to open the change request as a draft (discrete station PRs do).
    #[serde(default)]
    pub draft: bool,
    /// The change-request title.
    pub title: String,
    /// The change-request body (markdown).
    pub body: String,
}

impl ChangeRequestIntent {
    /// The run's stable base branch: `darkrun/<slug>/main`.
    pub fn run_main_branch(slug: &str) -> String {
        run_main_branch(slug)
    }

    /// A station's working branch: `darkrun/<slug>/<station>`.
    pub fn station_branch(slug: &str, station: &str) -> String {
        station_branch(slug, station)
    }
}

/// Derive the [`ChangeRequestIntent`] for a run sitting at an external
/// Checkpoint.
///
/// Returns [`McpError::InvalidInput`] when the run's active station is not
/// currently gated by an `external` Checkpoint, so the CLI can refuse to open a
/// PR/MR out of band. `head_override` lets the caller pin an explicit source
/// branch (e.g. the actual current git branch); when `None` the conventional
/// `darkrun/<slug>` branch is used.
pub fn change_request_intent(
    store: &StateStore,
    slug: &str,
    head_override: Option<String>,
) -> Result<ChangeRequestIntent> {
    let run = store.read_run(slug)?;
    let factory = crate::position::resolve_factory_for(store, &run.frontmatter.factory)
        .ok_or_else(|| McpError::UnknownFactory(run.frontmatter.factory.clone()))?;

    // The active station is whatever the cursor currently sits on.
    let position = derive_position(store, slug)?;
    let station = match &position.action {
        // An external gate surfaces as ExternalReviewRequested — exactly the
        // action this PR/MR handoff serves.
        Some(RunAction::ExternalReviewRequested { station, .. }) => station.clone(),
        Some(action) => {
            return Err(McpError::InvalidInput(format!(
                "run '{slug}' is not at an external review gate (current action: {})",
                action_name(action)
            )));
        }
        None => {
            return Err(McpError::InvalidInput(format!(
                "run '{slug}' has no pending action (mid-wave)"
            )));
        }
    };

    // The `ExternalReviewRequested` action above is the authoritative external
    // gate signal — `derive_position` emits it only when the station's EFFECTIVE
    // checkpoint kind is `External` (full-discrete runs, or a factory that
    // declares it). We deliberately don't re-check the raw factory kind here: the
    // software factory gates `ask` by default, and discrete mode promotes that to
    // external via `effective_checkpoint_kind` — which the action already reflects.
    let def = factory
        .station(&station)
        .ok_or_else(|| McpError::UnknownStation(station.clone()))?;

    // Under the hierarchy a station's external Checkpoint opens a draft PR from
    // its own branch into the run's stable base: darkrun/<slug>/<station> ->
    // darkrun/<slug>/main. `head_override` still pins an explicit source branch
    // (e.g. the live git branch) for the caller that knows better.
    let head =
        head_override.unwrap_or_else(|| ChangeRequestIntent::station_branch(slug, &station));
    let base = ChangeRequestIntent::run_main_branch(slug);
    let title = run.title.clone();
    let body = build_body(&run.title, &station, &def.kills, &run.body);

    Ok(ChangeRequestIntent {
        run: slug.to_string(),
        station,
        head,
        base,
        draft: true,
        title,
        body,
    })
}

/// Build the change-request body from the run's facts. Keeps it short and
/// deterministic so the same disk yields the same body.
fn build_body(title: &str, station: &str, kills: &str, run_body: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {title}\n\n"));
    out.push_str(&format!(
        "Opened by the darkrun **{station}** station's external Checkpoint.\n\n"
    ));
    if !kills.is_empty() {
        out.push_str(&format!("Risk eliminated: `{kills}`.\n\n"));
    }
    let trimmed = run_body.trim();
    if !trimmed.is_empty() {
        out.push_str("---\n\n");
        out.push_str(trimmed);
        out.push('\n');
    }
    out
}

/// A short label for the action kind, for error messages.
fn action_name(action: &RunAction) -> &'static str {
    match action {
        RunAction::Spec { .. } => "spec",
        RunAction::Review { .. } => "review",
        RunAction::Manufacture { .. } => "manufacture",
        RunAction::Audit { .. } => "audit",
        RunAction::Reflect { .. } => "reflect",
        RunAction::UserGate { .. } => "user_gate",
        RunAction::Checkpoint { .. } => "checkpoint",
        RunAction::FixFeedback { .. } => "fix_feedback",
        RunAction::FeedbackQuestion { .. } => "feedback_question",
        RunAction::UnitsInvalid { .. } => "units_invalid",
        RunAction::Escalate { .. } => "escalate",
        RunAction::BestEffortBoot { .. } => "best_effort_boot",
        RunAction::EscalateToUser { .. } => "escalate_to_user",
        RunAction::SafeRepair { .. } => "safe_repair",
        RunAction::ReviseUnitSpecs { .. } => "revise_unit_specs",
        RunAction::RunReview { .. } => "run_review",
        RunAction::ExternalReviewRequested { .. } => "external_review_requested",
        RunAction::PendingSeal { .. } => "pending_seal",
        RunAction::Sealed { .. } => "sealed",
        RunAction::MergeConflict { .. } => "merge_conflict",
        RunAction::SaveWip { .. } => "save_wip",
        RunAction::Noop { .. } => "noop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{checkpoint_decide, elaborate_seal, run_start, run_tick};
    use darkrun_core::domain::{CheckpointKind, Mode, Status, Unit, UnitFrontmatter};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    /// Drive a run to the `harden` station's checkpoint, then mark the run
    /// discrete so the gate resolves `external` (the PR path). Every station now
    /// gates `ask` by default; the external-review surface is a discrete-mode
    /// concern, which `effective_checkpoint_kind` forces.
    fn drive_to_harden_checkpoint(store: &StateStore, slug: &str) {
        run_start(store, slug, "software", Some("Ship the thing".into()), Mode::Solo, "full")
            .expect("start");
        // Walk every upstream station's `ask` gate and approve it.
        for station in ["frame", "specify", "shape", "build", "prove"] {
            walk_station(store, slug, station);
        }
        // harden: Spec -> Review -> UserGate -> Manufacture(empty -> Audit) ->
        // Reflect -> Checkpoint.
        walk_station_to_checkpoint(store, slug, "harden");
        // Flip the run to Team so harden's held gate re-derives as External.
        let mut state = store.read_state(slug).unwrap().unwrap();
        state.mode = Mode::Team;
        store.write_state(slug, &state).unwrap();
    }

    /// Walk one station's phases and clear its gate (auto stations clear on the
    /// Checkpoint tick; ask stations need an approval).
    fn walk_station(store: &StateStore, slug: &str, station: &str) {
        seed_one_unit(store, slug, station);
        // up to a dozen ticks is plenty to clear a station.
        for _ in 0..16 {
            let tick = run_tick(store, slug).expect("tick");
            // Clear the pre-execution operator gate so manufacture releases.
            if matches!(&tick.action, RunAction::UserGate { station: s, .. } if s == station) {
                checkpoint_decide(store, slug, true, None).expect("clear gate");
                continue;
            }
            if let RunAction::Checkpoint { kind, .. } = &tick.action {
                if matches!(kind, CheckpointKind::Ask) {
                    checkpoint_decide(store, slug, true, None).expect("approve");
                }
                // auto already advanced inside run_tick.
                break;
            }
            if let RunAction::Spec { station: s, .. } = &tick.action {
                if s != station {
                    break; // advanced past it
                }
                // Solo holds the Spec until the elaboration is sealed.
                elaborate_seal(store, slug, station).expect("seal");
                continue;
            }
        }
    }

    /// Walk one station up to (and stopping at) its gate, without deciding. The
    /// gate surfaces as a local `Checkpoint` or, for an external station, as
    /// `ExternalReviewRequested`.
    fn walk_station_to_checkpoint(store: &StateStore, slug: &str, station: &str) {
        seed_one_unit(store, slug, station);
        for _ in 0..16 {
            let tick = run_tick(store, slug).expect("tick");
            // Clear the pre-execution operator gate so the walk reaches the
            // post-execution checkpoint / external review gate.
            if matches!(&tick.action, RunAction::UserGate { station: s, .. } if s == station) {
                crate::position::checkpoint_decide(store, slug, true, None).expect("clear gate");
                continue;
            }
            // Solo holds the Spec until the elaboration is sealed.
            if matches!(&tick.action, RunAction::Spec { station: s, .. } if s == station) {
                elaborate_seal(store, slug, station).expect("seal");
                continue;
            }
            let at_gate = matches!(&tick.action, RunAction::Checkpoint { station: s, .. } if s == station)
                || matches!(&tick.action, RunAction::ExternalReviewRequested { station: s, .. } if s == station);
            if at_gate {
                return;
            }
        }
        panic!("never reached {station} gate");
    }

    /// Give a station one completed unit so Manufacture clears to Audit. The unit
    /// consumes the station's declared inputs so the runtime input-coverage gate
    /// is satisfied (the run's distillation is carried forward).
    fn seed_one_unit(store: &StateStore, slug: &str, station: &str) {
        let inputs = crate::factory::resolve_factory("software")
            .and_then(|f| f.station(station).map(|d| d.inputs.clone()))
            .unwrap_or_default();
        let unit = Unit {
            slug: format!("{station}-u1"),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some(station.to_string()),
                inputs,
                ..Default::default()
            },
            title: "u1".into(),
            body: String::new(),
        };
        store.write_unit(slug, &unit).expect("write unit");
    }

    #[test]
    fn intent_derived_at_external_checkpoint() {
        let (_d, store) = store();
        drive_to_harden_checkpoint(&store, "r");
        let intent = change_request_intent(&store, "r", None).expect("intent");
        assert_eq!(intent.run, "r");
        assert_eq!(intent.station, "harden");
        // Hierarchy: the station's draft PR runs station-branch -> run-main.
        assert_eq!(intent.head, "darkrun/r/harden");
        assert_eq!(intent.base, "darkrun/r/main");
        assert!(intent.draft, "discrete station PRs open as draft");
        assert_eq!(intent.title, "Ship the thing");
        assert!(intent.body.contains("harden"));
        assert!(intent.body.contains("works-in-dev-dies-in-prod"));
    }

    #[test]
    fn head_override_is_respected() {
        let (_d, store) = store();
        drive_to_harden_checkpoint(&store, "r");
        let intent =
            change_request_intent(&store, "r", Some("feature/login".into())).expect("intent");
        assert_eq!(intent.head, "feature/login");
    }

    #[test]
    fn rejects_non_external_checkpoint() {
        let (_d, store) = store();
        // frame's checkpoint is `ask`, not `external`.
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        seed_one_unit(&store, "r", "frame");
        for _ in 0..12 {
            let tick = run_tick(&store, "r").expect("tick");
            if matches!(&tick.action, RunAction::Checkpoint { .. }) {
                break;
            }
        }
        let err = change_request_intent(&store, "r", None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn rejects_when_not_at_checkpoint() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").expect("start");
        // Fresh run sits at Spec, not a Checkpoint.
        let err = change_request_intent(&store, "r", None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn hierarchy_branch_helpers() {
        assert_eq!(
            ChangeRequestIntent::run_main_branch("my-run"),
            "darkrun/my-run/main"
        );
        assert_eq!(
            ChangeRequestIntent::station_branch("my-run", "harden"),
            "darkrun/my-run/harden"
        );
    }

    #[test]
    fn action_name_maps_every_variant() {
        use crate::position::RunAction as A;
        use darkrun_core::domain::SealKind;
        let r = || "r".to_string();
        let s = || "frame".to_string();
        let cases: Vec<(A, &str)> = vec![
            (A::Spec { run: r(), station: s(), kills: "x".into() }, "spec"),
            (A::Review { run: r(), station: s(), reviewers: vec![] }, "review"),
            (A::Manufacture { run: r(), station: s(), worker: "w".into(), units: vec![] }, "manufacture"),
            (A::Audit { run: r(), station: s(), reviewers: vec![] }, "audit"),
            (A::Reflect { run: r(), station: s() }, "reflect"),
            (A::UserGate { run: r(), station: s() }, "user_gate"),
            (A::Checkpoint { run: r(), station: s(), kind: CheckpointKind::Ask }, "checkpoint"),
            (A::FixFeedback { run: r(), station: s(), feedback_id: "fb-1".into() }, "fix_feedback"),
            (A::FeedbackQuestion { run: r(), station: s(), feedback_id: "fb-1".into() }, "feedback_question"),
            (A::UnitsInvalid { run: r(), station: s(), problem: "p".into(), units: vec![] }, "units_invalid"),
            (A::Escalate { run: r(), station: s(), reason: "x".into() }, "escalate"),
            (A::SafeRepair { run: r(), station: s(), reason: "x".into() }, "safe_repair"),
            (A::ReviseUnitSpecs { run: r(), station: s(), units: vec![] }, "revise_unit_specs"),
            (A::ExternalReviewRequested { run: r(), station: s(), target: "t".into() }, "external_review_requested"),
            (A::RunReview { run: r(), reviewers: vec![] }, "run_review"),
            (A::PendingSeal { run: r(), kind: SealKind::External }, "pending_seal"),
            (A::Sealed { run: r() }, "sealed"),
            (A::MergeConflict { run: r(), station: s(), branch: "b".into(), conflict_paths: vec![] }, "merge_conflict"),
            (A::Noop { run: r(), message: "m".into() }, "noop"),
        ];
        for (action, name) in cases {
            assert_eq!(action_name(&action), name);
        }
    }

    #[test]
    fn change_request_intent_errors_off_an_external_gate() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // A fresh run sits at frame's pre-execution gate, not an external review
        // gate → opening a change request is refused (InvalidInput).
        assert!(change_request_intent(&store, "r", None).is_err());
    }

    #[test]
    fn change_request_intent_errors_mid_wave_with_no_pending_action() {
        use darkrun_core::domain::StationPhase;
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        // Force frame into Manufacture with an in-flight (InProgress) unit: no
        // wave-ready work and not all complete → derive_position yields no action.
        let mut state = store.read_state("r").unwrap().unwrap();
        state.stations.get_mut("frame").unwrap().phase = StationPhase::Manufacture;
        store.write_state("r", &state).unwrap();
        let unit = Unit {
            slug: "frame-u".into(),
            frontmatter: UnitFrontmatter { status: Status::InProgress, station: Some("frame".into()), ..Default::default() },
            title: "u".into(),
            body: String::new(),
        };
        store.write_unit("r", &unit).unwrap();

        let err = change_request_intent(&store, "r", None).expect_err("mid-wave has no action");
        assert!(format!("{err}").contains("no pending action"), "got {err}");
    }
}

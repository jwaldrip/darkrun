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

use darkrun_core::domain::CheckpointKind;
use darkrun_core::StateStore;
use serde::{Deserialize, Serialize};

use crate::error::{McpError, Result};
use crate::factory::resolve_factory;
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
    let factory = resolve_factory(&run.frontmatter.factory)
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

    let def = factory
        .station(&station)
        .ok_or_else(|| McpError::UnknownStation(station.clone()))?;
    if def.checkpoint != CheckpointKind::External {
        return Err(McpError::InvalidInput(format!(
            "station '{station}' has a {:?} checkpoint, not external",
            def.checkpoint
        )));
    }

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
        RunAction::ResolveDrift { .. } => "resolve_drift",
        RunAction::UnitsInvalid { .. } => "units_invalid",
        RunAction::Escalate { .. } => "escalate",
        RunAction::SafeRepair { .. } => "safe_repair",
        RunAction::ReviseUnitSpecs { .. } => "revise_unit_specs",
        RunAction::ExternalReviewRequested { .. } => "external_review_requested",
        RunAction::PendingSeal { .. } => "pending_seal",
        RunAction::Sealed { .. } => "sealed",
        RunAction::MergeConflict { .. } => "merge_conflict",
        RunAction::Noop { .. } => "noop",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::{checkpoint_decide, run_start, run_tick};
    use darkrun_core::domain::{Status, Unit, UnitFrontmatter};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    /// Drive a run all the way to the `harden` station's external Checkpoint.
    fn drive_to_harden_checkpoint(store: &StateStore, slug: &str) {
        run_start(store, slug, "software", Some("Ship the thing".into()), "continuous")
            .expect("start");
        // Walk every station. frame/specify/shape gate `ask`; build/prove gate
        // `auto`; harden gates `external`.
        for station in ["frame", "specify", "shape", "build", "prove"] {
            walk_station(store, slug, station);
        }
        // harden: Spec -> Review -> Manufacture(empty -> Audit) -> Reflect -> Checkpoint
        walk_station_to_checkpoint(store, slug, "harden");
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
            let at_gate = matches!(&tick.action, RunAction::Checkpoint { station: s, .. } if s == station)
                || matches!(&tick.action, RunAction::ExternalReviewRequested { station: s, .. } if s == station);
            if at_gate {
                return;
            }
        }
        panic!("never reached {station} gate");
    }

    /// Give a station one completed unit so Manufacture clears to Audit.
    fn seed_one_unit(store: &StateStore, slug: &str, station: &str) {
        let unit = Unit {
            slug: format!("{station}-u1"),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some(station.to_string()),
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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
        run_start(&store, "r", "software", None, "continuous").expect("start");
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
}

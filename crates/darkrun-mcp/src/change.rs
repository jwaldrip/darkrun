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

/// The branch prefix the engine forks run work onto, mirroring the worktree
/// naming convention (`darkrun/<slug>`).
const BRANCH_PREFIX: &str = "darkrun";

/// A fully-derived description of the change request an external Checkpoint
/// wants opened. Pure data — no network, no credentials, no git remote.
///
/// The CLI combines this with the repo's parsed remote coordinates (the base
/// branch and provider come from there) and the stored credential to actually
/// open the PR/MR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRequestIntent {
    /// The run this change request delivers.
    pub run: String,
    /// The station whose external Checkpoint triggered the handoff.
    pub station: String,
    /// The source branch carrying the run's work (the PR/MR head).
    pub head: String,
    /// The change-request title.
    pub title: String,
    /// The change-request body (markdown).
    pub body: String,
}

impl ChangeRequestIntent {
    /// The default head branch for a run slug: `darkrun/<slug>`.
    pub fn default_branch(slug: &str) -> String {
        format!("{BRANCH_PREFIX}/{slug}")
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
        Some(RunAction::Checkpoint { station, .. }) => station.clone(),
        Some(action) => {
            return Err(McpError::InvalidInput(format!(
                "run '{slug}' is not at a Checkpoint (current action: {})",
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

    let head = head_override.unwrap_or_else(|| ChangeRequestIntent::default_branch(slug));
    let title = run.title.clone();
    let body = build_body(&run.title, &station, &def.kills, &run.body);

    Ok(ChangeRequestIntent {
        run: slug.to_string(),
        station,
        head,
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
        RunAction::Checkpoint { .. } => "checkpoint",
        RunAction::FixFeedback { .. } => "fix_feedback",
        RunAction::ResolveDrift { .. } => "resolve_drift",
        RunAction::Sealed { .. } => "sealed",
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
        for _ in 0..12 {
            let tick = run_tick(store, slug).expect("tick");
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

    /// Walk one station up to (and stopping at) its Checkpoint, without deciding.
    fn walk_station_to_checkpoint(store: &StateStore, slug: &str, station: &str) {
        seed_one_unit(store, slug, station);
        for _ in 0..12 {
            let tick = run_tick(store, slug).expect("tick");
            if matches!(&tick.action, RunAction::Checkpoint { station: s, .. } if s == station) {
                return;
            }
        }
        panic!("never reached {station} checkpoint");
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
        assert_eq!(intent.head, "darkrun/r");
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
    fn default_branch_uses_prefix() {
        assert_eq!(ChangeRequestIntent::default_branch("my-run"), "darkrun/my-run");
    }
}

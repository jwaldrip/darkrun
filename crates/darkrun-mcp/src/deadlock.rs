//! Cross-tick deadlock guard.
//!
//! darkrun's [`run_tick`](crate::position::run_tick) is **single-shot** — the
//! agent performs the returned action then re-ticks via `darkrun_tick`. So
//! the deadlock shape is the INTER-tick one the predecessor hit: the cursor
//! returns the same action (or alternates between two) across consecutive ticks
//! with **no progress**, wedging the agent forever — including the worst case
//! where the agent has *already satisfied the requirements* but the tick still
//! won't advance.
//!
//! This guard pairs each derived action with a cheap **progress fingerprint**
//! (active station + unit count + completed count + total Pass count + run
//! status). A tick that makes any real progress changes the fingerprint and
//! resets the counter; only a genuinely stuck loop accumulates. Once the same
//! (action + fingerprint) repeats past [`HALT_THRESHOLD`], or two signatures
//! alternate past the churn window, the wedged action is swapped for an
//! [`RunAction::Escalate`] carrying a diagnostic + recovery path, so a stuck run
//! surfaces to a human instead of spinning.
//!
//! History is per-run, persisted to `.darkrun/<slug>/deadlock.json` so it
//! survives an MCP restart mid-loop (a reconnect can't wipe the count); a history
//! untouched for [`STALE_AGE_SECS`] is treated as fresh so a returning session
//! never inherits a stale halt.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use darkrun_core::domain::Status;
use darkrun_core::StateStore;

use crate::position::{action_tag, RunAction};

/// Same (action + no-progress) signature seen MORE than this many consecutive
/// times → halt. Mirrors the predecessor's `HALT_THRESHOLD`.
const HALT_THRESHOLD: u32 = 4;
/// Churn window: at most this many DISTINCT signatures across the recent window
/// (an A↔B alternation), once at least [`CHURN_MIN_TICKS`] have accumulated.
const CHURN_MAX_DISTINCT: usize = 2;
const CHURN_WINDOW: usize = 10;
const CHURN_MIN_TICKS: usize = 8;
/// A history untouched this long is treated as fresh — a next-day session never
/// inherits a stale halt while a rapid reconnect-loop keeps accumulating.
const STALE_AGE_SECS: u64 = 60 * 60;

/// Per-run no-progress history, persisted to `.darkrun/<slug>/deadlock.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct History {
    /// The current repeating (action + fingerprint) signature.
    signature: String,
    /// Consecutive ticks at `signature` with no progress.
    count: u32,
    /// Unix seconds of the last update (drives staleness).
    updated_at: u64,
    /// Recent signatures, bounded to [`CHURN_WINDOW`], for the churn detector.
    recent: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Actions that LEGITIMATELY repeat while awaiting an external party (the human
/// at a gate, an external PR/merge, the agent resolving conflicts in-tree, or an
/// already-fired escalation) — never a wedge, so they're exempt from the halt.
fn is_exempt(action: &RunAction) -> bool {
    matches!(
        action,
        RunAction::UserGate { .. }
            | RunAction::Checkpoint { .. }
            | RunAction::PendingSeal { .. }
            | RunAction::ExternalReviewRequested { .. }
            | RunAction::Sealed { .. }
            | RunAction::MergeConflict { .. }
            | RunAction::Escalate { .. }
            | RunAction::FeedbackQuestion { .. }
    )
}

/// A cheap fingerprint of run progress: anything the agent advancing the run
/// changes (decompose → unit count; a Pass → the Pass sum; a completion → the
/// done count or run status; a station land → the active station). Identical
/// across ticks ⇒ the agent made no progress.
fn progress_fingerprint(store: &StateStore, slug: &str) -> String {
    let units = store.read_units(slug).unwrap_or_default();
    let total = units.len();
    let done = units
        .iter()
        .filter(|u| matches!(u.status(), Status::Completed))
        .count();
    let pass_sum: u32 = units.iter().map(|u| u.pass()).sum();
    let (active, status) = store
        .read_run(slug)
        .map(|r| {
            (
                r.frontmatter.active_station,
                format!("{:?}", r.frontmatter.status),
            )
        })
        .unwrap_or_default();
    // Drift + feedback so a fix/drift loop that actually RESOLVES something (an
    // entry cleared, a status advanced) counts as progress and resets the guard;
    // only a loop that resolves nothing accumulates toward the halt.
    let drift_n = crate::drift::open_drift_count(store, slug);
    let mut fb: Vec<String> = crate::feedback::list(store, slug)
        .unwrap_or_default()
        .iter()
        .map(|f| format!("{}:{:?}", f.id, f.status))
        .collect();
    fb.sort();
    format!(
        "st={active};units={total};done={done};pass={pass_sum};status={status};drift={drift_n};fb=[{}]",
        fb.join(",")
    )
}

/// The wedge signature for a tick: the action's tag + station + the progress
/// fingerprint. (Tag+station, not the full serialization, so a varying prompt or
/// reason string can't mask a true no-progress loop.)
fn signature(store: &StateStore, slug: &str, action: &RunAction) -> String {
    let station = action_station(action).unwrap_or("");
    format!(
        "{}@{}|{}",
        action_tag(action),
        station,
        progress_fingerprint(store, slug)
    )
}

/// The station an action targets, if any (for the signature).
fn action_station(action: &RunAction) -> Option<&str> {
    match action {
        RunAction::Spec { station, .. }
        | RunAction::Review { station, .. }
        | RunAction::Manufacture { station, .. }
        | RunAction::Audit { station, .. }
        | RunAction::Reflect { station, .. }
        | RunAction::UserGate { station, .. }
        | RunAction::Checkpoint { station, .. }
        | RunAction::Escalate { station, .. }
        | RunAction::MergeConflict { station, .. } => Some(station),
        _ => None,
    }
}

fn history_path(store: &StateStore, slug: &str) -> std::path::PathBuf {
    store.run_dir(slug).join("deadlock.json")
}

fn load(store: &StateStore, slug: &str) -> History {
    let path = history_path(store, slug);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return History::default();
    };
    let Ok(h) = serde_json::from_str::<History>(&raw) else {
        return History::default();
    };
    // Stale (untouched > STALE_AGE_SECS) → start fresh.
    if now_secs().saturating_sub(h.updated_at) > STALE_AGE_SECS {
        return History::default();
    }
    h
}

fn save(store: &StateStore, slug: &str, h: &History) {
    let path = history_path(store, slug);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(raw) = serde_json::to_string_pretty(h) {
        let _ = std::fs::write(&path, raw);
    }
}

/// The `Escalate` action a halt produces — surfaces the wedge + recovery path.
fn halt_action(slug: &str, action: &RunAction, count: u32, reason_kind: &str) -> RunAction {
    let station = action_station(action).unwrap_or("").to_string();
    RunAction::Escalate {
        run: slug.to_string(),
        station,
        reason: format!(
            "deadlock guard: the run returned `{}` {reason_kind} across {count} ticks with no \
             progress — it is wedged (the requirements may already be satisfied but the cursor is \
             not advancing). Escalating to a human. Inspect/recover with the `darkrun-debug` skill.",
            action_tag(action)
        ),
    }
}

/// Record this tick's action and decide whether the run is deadlocked.
///
/// Returns `Some(escalate_action)` to REPLACE the wedged action when a halt
/// threshold is crossed (and the action isn't a legitimate external-await),
/// else `None` (proceed with the derived action). Best-effort: a persistence
/// failure never blocks a tick.
pub fn check(store: &StateStore, slug: &str, action: &RunAction) -> Option<RunAction> {
    if is_exempt(action) {
        // Reset the history so a long gate wait doesn't carry a stale count into
        // the next real action.
        save(store, slug, &History::default());
        return None;
    }

    let sig = signature(store, slug, action);
    let mut h = load(store, slug);

    if h.signature == sig {
        h.count = h.count.saturating_add(1);
    } else {
        h.signature = sig.clone();
        h.count = 1;
    }
    h.updated_at = now_secs();
    h.recent.push(sig.clone());
    if h.recent.len() > CHURN_WINDOW {
        let drop = h.recent.len() - CHURN_WINDOW;
        h.recent.drain(0..drop);
    }

    // Same-signature halt.
    let same_halt = h.count > HALT_THRESHOLD;
    // Churn halt: the window is full of an A↔B (≤ CHURN_MAX_DISTINCT) alternation,
    // and it's not a single repeated signature (that's the same-signature case).
    let distinct: std::collections::BTreeSet<&str> =
        h.recent.iter().map(String::as_str).collect();
    let churn_halt = h.recent.len() >= CHURN_MIN_TICKS
        && distinct.len() >= 2
        && distinct.len() <= CHURN_MAX_DISTINCT;

    let verdict = if same_halt {
        Some(halt_action(slug, action, h.count, "repeatedly"))
    } else if churn_halt {
        Some(halt_action(slug, action, h.recent.len() as u32, "alternating"))
    } else {
        None
    };

    save(store, slug, &h);
    verdict
}

/// Clear a run's deadlock history (after a successful recovery / decision so the
/// next live action starts from a clean count).
pub fn clear(store: &StateStore, slug: &str) {
    let _ = std::fs::remove_file(history_path(store, slug));
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_core::domain::{Run, RunFrontmatter};
    use tempfile::TempDir;

    fn store_with_run(slug: &str) -> (TempDir, StateStore) {
        let dir = TempDir::new().unwrap();
        let store = StateStore::new(dir.path());
        let run = Run {
            slug: slug.to_string(),
            title: slug.to_string(),
            body: String::new(),
            frontmatter: RunFrontmatter {
                factory: "software".into(),
                active_station: "frame".into(),
                ..Default::default()
            },
        };
        store.write_run(&run).unwrap();
        (dir, store)
    }

    fn spec(slug: &str) -> RunAction {
        RunAction::Spec {
            run: slug.into(),
            station: "frame".into(),
            kills: String::new(),
        }
    }

    #[test]
    fn same_action_no_progress_halts_after_threshold() {
        let (_d, store) = store_with_run("r");
        let a = spec("r");
        // The first HALT_THRESHOLD ticks proceed (count climbs to the threshold).
        for _ in 0..HALT_THRESHOLD {
            assert!(check(&store, "r", &a).is_none());
        }
        // The next identical, no-progress tick is the wedge → escalate.
        let halt = check(&store, "r", &a).expect("halt");
        assert!(matches!(halt, RunAction::Escalate { .. }));
    }

    #[test]
    fn exempt_actions_never_halt() {
        let (_d, store) = store_with_run("r");
        let gate = RunAction::Checkpoint {
            run: "r".into(),
            station: "frame".into(),
            kind: darkrun_core::domain::CheckpointKind::Ask,
        };
        for _ in 0..(HALT_THRESHOLD + 5) {
            assert!(check(&store, "r", &gate).is_none());
        }
    }

    #[test]
    fn churn_between_two_actions_halts() {
        let (_d, store) = store_with_run("r");
        // Two distinct actions on the SAME station with no progress between them —
        // an A↔B alternation. After the churn window it's a wedge.
        let a = RunAction::Spec {
            run: "r".into(),
            station: "frame".into(),
            kills: String::new(),
        };
        let b = RunAction::Review {
            run: "r".into(),
            station: "frame".into(),
            reviewers: vec![],
        };
        let mut halted = false;
        for i in 0..CHURN_WINDOW {
            let action = if i % 2 == 0 { &a } else { &b };
            if check(&store, "r", action).is_some() {
                halted = true;
                break;
            }
        }
        assert!(halted, "an A↔B no-progress alternation must halt");
    }

    #[test]
    fn progress_resets_the_counter() {
        let (_d, store) = store_with_run("r");
        let a = spec("r");
        for _ in 0..HALT_THRESHOLD {
            assert!(check(&store, "r", &a).is_none());
        }
        // Make progress: advance the run's active station — the fingerprint
        // changes, so the next identical action is a NEW signature (count resets).
        let mut run = store.read_run("r").unwrap();
        run.frontmatter.active_station = "specify".into();
        store.write_run(&run).unwrap();
        let a2 = RunAction::Spec {
            run: "r".into(),
            station: "specify".into(),
            kills: String::new(),
        };
        assert!(check(&store, "r", &a2).is_none());
    }

    #[test]
    fn action_station_reads_the_station_off_every_carrying_variant() {
        assert_eq!(
            action_station(&RunAction::UserGate { run: "r".into(), station: "frame".into() }),
            Some("frame")
        );
        assert_eq!(
            action_station(&RunAction::Escalate {
                run: "r".into(), station: "build".into(), reason: "x".into(),
            }),
            Some("build")
        );
        assert_eq!(
            action_station(&RunAction::MergeConflict {
                run: "r".into(), station: "harden".into(), branch: "b".into(), conflict_paths: vec![],
            }),
            Some("harden")
        );
        assert_eq!(
            action_station(&RunAction::Checkpoint {
                run: "r".into(), station: "prove".into(),
                kind: darkrun_core::domain::CheckpointKind::Ask,
            }),
            Some("prove")
        );
        // A variant with no station → None.
        assert_eq!(action_station(&RunAction::Sealed { run: "r".into() }), None);
    }

    #[test]
    fn corrupt_history_file_is_treated_as_a_fresh_start() {
        let (_d, store) = store_with_run("r");
        // Plant an unparseable deadlock.json; load() must swallow it and reset.
        let path = history_path(&store, "r");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not valid json").unwrap();
        // The tick proceeds (no carried count) rather than erroring.
        assert!(check(&store, "r", &spec("r")).is_none());
    }

    #[test]
    fn recent_window_drains_past_the_churn_bound() {
        let (_d, store) = store_with_run("r");
        let a = spec("r");
        // Ticking the same action well past CHURN_WINDOW grows then drains the
        // bounded `recent` history (it stays capped, never unbounded).
        for _ in 0..(CHURN_WINDOW + 4) {
            let _ = check(&store, "r", &a);
        }
        let h = load(&store, "r");
        assert!(h.recent.len() <= CHURN_WINDOW, "recent stays bounded: {}", h.recent.len());
    }
}

//! Integration tests for the run-browser view logic: `run_status_tone` (the
//! status string → badge tone mapping) and the `RunCardData` view-model, both
//! exercised through the crate's public prelude without a renderer.

use darkrun_ui::prelude::{run_status_tone, Phase, RunCardData, Tone};

#[test]
fn completed_maps_to_ok() {
    assert_eq!(run_status_tone("completed"), Tone::Ok);
}

#[test]
fn active_and_in_progress_map_to_info() {
    assert_eq!(run_status_tone("active"), Tone::Info);
    assert_eq!(run_status_tone("in_progress"), Tone::Info);
}

#[test]
fn blocked_maps_to_danger() {
    assert_eq!(run_status_tone("blocked"), Tone::Danger);
}

#[test]
fn pending_maps_to_warn() {
    assert_eq!(run_status_tone("pending"), Tone::Warn);
}

#[test]
fn unknown_status_is_neutral() {
    for s in ["", "unknown", "wat", "archived"] {
        assert_eq!(run_status_tone(s), Tone::Neutral, "{s}");
    }
}

#[test]
fn mapping_is_case_sensitive() {
    // Wire statuses are lowercase snake_case; uppercase variants fall through.
    assert_eq!(run_status_tone("ACTIVE"), Tone::Neutral);
    assert_eq!(run_status_tone("Completed"), Tone::Neutral);
}

#[test]
fn the_status_buckets_are_distinct() {
    assert_ne!(run_status_tone("completed"), run_status_tone("blocked"));
    assert_ne!(run_status_tone("blocked"), run_status_tone("active"));
    assert_ne!(run_status_tone("active"), run_status_tone("pending"));
}

#[test]
fn mapping_is_deterministic() {
    for s in ["active", "completed", "blocked", "pending", "xyz"] {
        assert_eq!(run_status_tone(s), run_status_tone(s));
    }
}

#[test]
fn card_data_round_trips_fields_and_is_eq() {
    let data = RunCardData {
        slug: "rate-limit".into(),
        title: "Rate limit the public API".into(),
        factory: "software".into(),
        active_station: "build".into(),
        phase: Some(Phase::Manufacture),
        status: "active".into(),
        completed: 3,
        total: 6,
    };
    assert_eq!(data.clone(), data);
    assert_eq!(data.slug, "rate-limit");
    assert_eq!(data.completed, 3);
    assert_eq!(data.total, 6);
    assert_eq!(data.phase, Some(Phase::Manufacture));
}

#[test]
fn card_data_phase_is_optional() {
    let data = RunCardData {
        slug: "between".into(),
        title: "Between stations".into(),
        factory: "software".into(),
        active_station: "frame".into(),
        phase: None,
        status: "pending".into(),
        completed: 0,
        total: 4,
    };
    assert_eq!(data.phase, None);
}

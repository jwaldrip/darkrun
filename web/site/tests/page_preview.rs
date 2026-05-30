//! Integration tests for the `/preview` fixture page: the representative
//! Question and Direction payloads it renders, and the route registration that
//! makes it reachable + listed for the static generator.

use darkrun_api::common::SessionStatus;
use darkrun_site::pages::preview::{sample_direction, sample_question};
use darkrun_site::route::Route;

// ---------------------------------------------------------------------------
// route registration
// ---------------------------------------------------------------------------

#[test]
fn preview_route_is_listed_in_all_paths() {
    assert!(
        Route::all_paths().iter().any(|p| p == "/preview"),
        "the /preview fixture must be in Route::all_paths so the generator pre-renders it"
    );
}

#[test]
fn preview_path_is_unique() {
    let count = Route::all_paths().iter().filter(|p| *p == "/preview").count();
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// question fixture
// ---------------------------------------------------------------------------

#[test]
fn question_fixture_is_a_well_formed_pending_question() {
    let q = sample_question();
    assert_eq!(q.session_id, "preview-question");
    assert_eq!(q.status, SessionStatus::Pending);
    assert!(!q.prompt.is_empty());
    assert!(q.title.is_some());
    assert!(q.context.is_some());
    // It is unanswered so the view renders the interactive (not read-only) shell.
    assert!(q.answer.is_none());
}

#[test]
fn question_fixture_has_at_least_two_distinct_options() {
    let q = sample_question();
    assert!(q.options.len() >= 2);
    let mut ids: Vec<&str> = q.options.iter().map(|o| o.id.as_str()).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "option ids must be unique");
}

#[test]
fn question_fixture_options_carry_labels() {
    for o in sample_question().options {
        assert!(!o.id.is_empty());
        assert!(!o.label.is_empty());
    }
}

#[test]
fn question_fixture_is_single_select_by_default() {
    assert!(!sample_question().multi_select);
}

// ---------------------------------------------------------------------------
// direction fixture
// ---------------------------------------------------------------------------

#[test]
fn direction_fixture_is_a_well_formed_pending_direction() {
    let d = sample_direction();
    assert_eq!(d.session_id, "preview-direction");
    assert_eq!(d.status, SessionStatus::Pending);
    assert!(!d.prompt.is_empty());
    assert!(d.title.is_some());
}

#[test]
fn direction_fixture_has_distinct_archetypes_with_descriptions() {
    let d = sample_direction();
    assert!(d.archetypes.len() >= 2);
    for a in &d.archetypes {
        assert!(!a.id.is_empty());
        assert!(!a.label.is_empty());
        assert!(!a.description.is_empty(), "archetype {} needs a description", a.id);
    }
    let mut ids: Vec<&str> = d.archetypes.iter().map(|a| a.id.as_str()).collect();
    let total = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), total, "archetype ids must be unique");
}

#[test]
fn direction_fixture_chooses_an_existing_archetype() {
    let d = sample_direction();
    let chosen = d.chosen_archetype.as_deref().expect("a chosen archetype");
    assert!(
        d.archetypes.iter().any(|a| a.id == chosen),
        "chosen_archetype {chosen} must be one of the archetypes"
    );
}

#[test]
fn direction_fixture_pins_are_normalized_and_noted() {
    let d = sample_direction();
    let ann = d.annotations.as_ref().expect("annotations present");
    assert!(!ann.pins.is_empty(), "fixture should show pins for the screenshot");
    for p in &ann.pins {
        assert!((0.0..=1.0).contains(&p.x), "pin x {} out of range", p.x);
        assert!((0.0..=1.0).contains(&p.y), "pin y {} out of range", p.y);
        assert!(!p.note.is_empty());
    }
}

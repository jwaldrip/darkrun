//! Integration tests for the interactive session views' public surface — the
//! prop-data builders ([`OptionCard`], [`ArchetypeCard`], [`PickerItem`]) and the
//! pure selection + pin-placement logic ([`SelectionModel`], [`place_pin`]) that
//! the [`QuestionView`], [`DirectionView`], and [`PickerView`] components drive.
//!
//! These exercise the crate through its `prelude` without instantiating a
//! renderer — the same renderer-free contract as `flow.rs` and `logic.rs`.
//!
//! Coverage:
//! - option / archetype / picker card builders and their optional fields
//! - single- vs multi-select state machine, order preservation, seeding
//! - pin placement: normalization, clamping, percentage rendering, degenerate
//!   dimensions, and the pixel -> normalized -> css-percent round trip
//! - the realistic flows a parent component runs: answering a multi-select
//!   question, choosing + annotating a design direction, picking one option

use darkrun_ui::prelude::*;

// ===========================================================================
// OptionCard builder
// ===========================================================================

#[test]
fn option_card_label_only_has_no_extras() {
    let o = OptionCard::new("a", "Option A");
    assert_eq!(o.id, "a");
    assert_eq!(o.label, "Option A");
    assert!(o.image_url.is_none());
    assert!(o.description.is_none());
}

#[test]
fn option_card_with_image_and_description() {
    let o = OptionCard::new("a", "A")
        .with_image("https://img/a.png")
        .with_description("the first one");
    assert_eq!(o.image_url.as_deref(), Some("https://img/a.png"));
    assert_eq!(o.description.as_deref(), Some("the first one"));
}

#[test]
fn option_card_is_clone_eq() {
    let o = OptionCard::new("a", "A").with_image("x");
    assert_eq!(o.clone(), o);
    assert_ne!(OptionCard::new("a", "A"), OptionCard::new("b", "A"));
}

// ===========================================================================
// ArchetypeCard builder
// ===========================================================================

#[test]
fn archetype_card_carries_all_required_fields() {
    let a = ArchetypeCard::new("minimal", "Minimal", "https://img/m.png", "clean & spare");
    assert_eq!(a.id, "minimal");
    assert_eq!(a.label, "Minimal");
    assert_eq!(a.image_url, "https://img/m.png");
    assert_eq!(a.description, "clean & spare");
}

#[test]
fn archetype_card_is_clone_eq() {
    let a = ArchetypeCard::new("x", "X", "u", "d");
    assert_eq!(a.clone(), a);
    assert_ne!(
        ArchetypeCard::new("x", "X", "u", "d"),
        ArchetypeCard::new("y", "X", "u", "d"),
    );
}

// ===========================================================================
// PickerItem builder
// ===========================================================================

#[test]
fn picker_item_label_only() {
    let p = PickerItem::new("sw", "software-factory");
    assert_eq!(p.id, "sw");
    assert_eq!(p.label, "software-factory");
    assert!(p.description.is_none());
    assert!(p.secondary.is_none());
}

#[test]
fn picker_item_with_description_and_secondary() {
    let p = PickerItem::new("sw", "software")
        .with_description("ship code")
        .with_secondary("default");
    assert_eq!(p.description.as_deref(), Some("ship code"));
    assert_eq!(p.secondary.as_deref(), Some("default"));
}

// ===========================================================================
// SelectMode
// ===========================================================================

#[test]
fn select_mode_from_multi_flag() {
    assert_eq!(SelectMode::from_multi(true), SelectMode::Multi);
    assert_eq!(SelectMode::from_multi(false), SelectMode::Single);
}

#[test]
fn select_mode_is_multi_predicate() {
    assert!(SelectMode::Multi.is_multi());
    assert!(!SelectMode::Single.is_multi());
}

// ===========================================================================
// SelectionModel — single select
// ===========================================================================

#[test]
fn single_select_replaces_choice() {
    let mut m = SelectionModel::new(SelectMode::Single);
    m.toggle("a");
    m.toggle("b");
    assert_eq!(m.selected(), ["b".to_string()]);
    assert!(!m.is_selected("a"));
    assert!(m.is_selected("b"));
}

#[test]
fn single_select_toggle_same_clears_and_disables_submit() {
    let mut m = SelectionModel::new(SelectMode::Single);
    m.toggle("a");
    assert!(!m.is_empty());
    m.toggle("a");
    assert!(m.is_empty()); // a submit bar keyed on !is_empty() would now disable
}

// ===========================================================================
// SelectionModel — multi select
// ===========================================================================

#[test]
fn multi_select_accumulates_and_preserves_order() {
    let mut m = SelectionModel::new(SelectMode::Multi);
    for id in ["c", "a", "b"] {
        m.toggle(id);
    }
    assert_eq!(
        m.selected(),
        ["c".to_string(), "a".to_string(), "b".to_string()]
    );
    assert_eq!(m.count(), 3);
}

#[test]
fn multi_select_deselect_keeps_remaining_order() {
    let mut m = SelectionModel::new(SelectMode::Multi);
    for id in ["a", "b", "c"] {
        m.toggle(id);
    }
    m.toggle("a");
    assert_eq!(m.selected(), ["b".to_string(), "c".to_string()]);
}

#[test]
fn seeded_from_prior_answer_round_trips() {
    // A re-opened answered question seeds the model from the stored selection.
    let m = SelectionModel::from_selected(
        SelectMode::Multi,
        ["opt-1".to_string(), "opt-3".to_string()],
    );
    assert!(m.is_selected("opt-1"));
    assert!(m.is_selected("opt-3"));
    assert!(!m.is_selected("opt-2"));
    assert_eq!(m.count(), 2);
}

#[test]
fn seeded_single_select_keeps_only_last() {
    let m = SelectionModel::from_selected(
        SelectMode::Single,
        ["a".to_string(), "b".to_string()],
    );
    assert_eq!(m.selected(), ["b".to_string()]);
}

// ===========================================================================
// Pin placement
// ===========================================================================

#[test]
fn place_pin_center_is_half_half() {
    let p = place_pin(80.0, 45.0, 160.0, 90.0, "center");
    assert!((p.x - 0.5).abs() < 1e-9);
    assert!((p.y - 0.5).abs() < 1e-9);
    assert_eq!(p.note, "center");
}

#[test]
fn place_pin_clamps_to_box() {
    let p = place_pin(1000.0, -50.0, 100.0, 100.0, "corner");
    assert_eq!(p.x, 1.0);
    assert_eq!(p.y, 0.0);
}

#[test]
fn place_pin_zero_dimensions_are_safe() {
    let p = place_pin(10.0, 10.0, 0.0, 0.0, "x");
    assert!(p.x.is_finite() && p.y.is_finite());
    assert_eq!(p.x, 0.0);
    assert_eq!(p.y, 0.0);
}

#[test]
fn pin_point_renders_css_percentages() {
    let p = PinPoint::new(0.25, 0.75, "n");
    assert_eq!(p.left_pct(), "25.0000%");
    assert_eq!(p.top_pct(), "75.0000%");
}

#[test]
fn pin_pixel_to_percent_round_trip() {
    // place a pin at 3/4 across a 200px-wide stage -> renders at 75%.
    let p = place_pin(150.0, 0.0, 200.0, 120.0, "q");
    assert_eq!(p.left_pct(), "75.0000%");
}

// ===========================================================================
// Realistic parent flows
// ===========================================================================

#[test]
fn answering_a_multi_select_question_builds_the_answer_ids() {
    // The QuestionView parent owns a SelectionModel; toggling cards mutates it,
    // and the submit handler reads `selected()` as the answer.
    let options = [
        OptionCard::new("warm", "Warm palette").with_image("u1"),
        OptionCard::new("cool", "Cool palette").with_image("u2"),
        OptionCard::new("mono", "Monochrome").with_description("greyscale"),
    ];
    let mut model = SelectionModel::new(SelectMode::from_multi(true));
    // Operator picks two.
    model.toggle(&options[0].id);
    model.toggle(&options[2].id);
    assert_eq!(model.selected(), ["warm".to_string(), "mono".to_string()]);
    // The submit bar is enabled while a selection exists.
    assert!(!model.is_empty());
}

#[test]
fn choosing_and_annotating_a_direction() {
    let archetypes = [
        ArchetypeCard::new("editorial", "Editorial", "u1", "serif, generous whitespace"),
        ArchetypeCard::new("brutalist", "Brutalist", "u2", "raw, high-contrast"),
    ];
    // Single-choice archetype selection reuses the single-select model.
    let mut choice = SelectionModel::new(SelectMode::Single);
    choice.toggle(&archetypes[1].id);
    assert_eq!(choice.selected(), ["brutalist".to_string()]);

    // Two pins dropped on the chosen preview at known offsets.
    let stage_w = 520.0;
    let stage_h = 390.0;
    let p1 = place_pin(130.0, 97.5, stage_w, stage_h, "tighten the header");
    let p2 = place_pin(390.0, 292.5, stage_w, stage_h, "more contrast here");
    assert_eq!(p1.left_pct(), "25.0000%");
    assert_eq!(p1.top_pct(), "25.0000%");
    assert_eq!(p2.left_pct(), "75.0000%");
    assert_eq!(p2.top_pct(), "75.0000%");

    let pins = [p1, p2];
    assert_eq!(pins.len(), 2);
    assert_eq!(pins[0].note, "tighten the header");
}

#[test]
fn picking_one_option_is_single_select() {
    let items = [
        PickerItem::new("software", "software-factory").with_secondary("default"),
        PickerItem::new("design", "design-factory"),
    ];
    let mut sel = SelectionModel::new(SelectMode::Single);
    sel.toggle(&items[0].id);
    assert_eq!(sel.selected(), ["software".to_string()]);
    // Re-pick switches the single selection.
    sel.toggle(&items[1].id);
    assert_eq!(sel.selected(), ["design".to_string()]);
}

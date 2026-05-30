//! The boundary mapping: `darkrun-api` wire enums + opaque parser `Value`s ->
//! the `darkrun-ui` design-system kinds the components consume.
//!
//! `darkrun-ui` deliberately has no `darkrun-core`/`darkrun-api` dependency, so
//! every domain->UI translation lives here, one small function each. The unit
//! and criteria payloads are loose `serde_json::Value`s by design; we probe a
//! handful of conventional keys and degrade gracefully when one is absent.

use darkrun_api::common::{GateType, SessionStatus};
use darkrun_api::session::{
    DirectionArchetype, DirectionPin, PickerOption, QuestionOption, RunPhase,
};
use darkrun_ui::components::factory::CheckpointKind;
use darkrun_ui::components::session_views::{ArchetypeCard, OptionCard, PickerItem};
use darkrun_ui::kinds::{Phase, Tone};
use darkrun_ui::selection::PinPoint;
use serde_json::Value;

/// Map the wire [`RunPhase`] onto the UI [`Phase`].
pub fn phase(p: RunPhase) -> Phase {
    match p {
        RunPhase::Spec => Phase::Spec,
        RunPhase::Review => Phase::Review,
        RunPhase::Manufacture => Phase::Manufacture,
        RunPhase::Audit => Phase::Audit,
        RunPhase::Reflect => Phase::Reflect,
        RunPhase::Checkpoint => Phase::Checkpoint,
    }
}

/// Map the wire [`GateType`] onto the UI [`CheckpointKind`].
pub fn checkpoint_kind(g: GateType) -> CheckpointKind {
    match g {
        GateType::Auto => CheckpointKind::Auto,
        GateType::Ask => CheckpointKind::Ask,
        GateType::External => CheckpointKind::External,
        GateType::Await => CheckpointKind::Await,
    }
}

/// Map a session lifecycle status onto a badge [`Tone`].
pub fn status_tone(s: SessionStatus) -> Tone {
    match s {
        SessionStatus::Pending => Tone::Warn,
        SessionStatus::Decided => Tone::Info,
        SessionStatus::Answered => Tone::Info,
        SessionStatus::Approved => Tone::Ok,
        SessionStatus::ChangesRequested => Tone::Danger,
    }
}

/// A flattened, display-ready Unit pulled out of the opaque parser `Value`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnitView {
    /// Display title (falls back to slug, then `"unit"`).
    pub title: String,
    /// Optional unit type chip.
    pub unit_type: Option<String>,
    /// Status label, lowercased.
    pub status_label: String,
    /// Status tone derived from the label.
    pub tone: Tone,
    /// Pass counter, when present.
    pub pass: u32,
    /// Completion criteria lines.
    pub criteria: Vec<String>,
}

/// Probe a `Value` object for the first present string among `keys`.
pub fn first_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

/// Map a free-form status token onto a [`Tone`]. Unknown tokens read neutral.
pub fn label_tone(label: &str) -> Tone {
    match label.trim().to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" | "merged" | "passed" | "approved" => Tone::Ok,
        "active" | "in_progress" | "in-progress" | "running" | "manufacturing" => Tone::Info,
        "blocked" | "failed" | "error" | "rejected" | "changes_requested" => Tone::Danger,
        "pending" | "queued" | "waiting" | "review" => Tone::Warn,
        _ => Tone::Neutral,
    }
}

/// Pull completion-criteria lines out of a unit `Value`. Accepts either a list
/// of strings or a list of objects carrying a `text`/`description`/`label`
/// field — whichever the parser emitted.
pub fn extract_criteria(unit: &Value) -> Vec<String> {
    for key in ["criteria", "completion_criteria", "acceptance", "checks"] {
        if let Some(arr) = unit.get(key).and_then(Value::as_array) {
            let lines: Vec<String> = arr
                .iter()
                .filter_map(|item| match item {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(_) => {
                        first_str(item, &["text", "description", "label", "name", "criterion"])
                    }
                    _ => None,
                })
                .filter(|s| !s.trim().is_empty())
                .collect();
            if !lines.is_empty() {
                return lines;
            }
        }
    }
    Vec::new()
}

/// Flatten one opaque unit `Value` into a [`UnitView`].
pub fn unit_view(unit: &Value) -> UnitView {
    let title = first_str(unit, &["title", "name", "slug", "id"])
        .unwrap_or_else(|| "unit".to_string());
    let unit_type = first_str(unit, &["unit_type", "type", "kind"]);
    let status_label = first_str(unit, &["status", "state"])
        .unwrap_or_else(|| "pending".to_string())
        .to_ascii_lowercase();
    let tone = label_tone(&status_label);
    let pass = unit
        .get("pass")
        .or_else(|| unit.get("passes"))
        .or_else(|| unit.get("visit"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    UnitView {
        title,
        unit_type,
        status_label,
        tone,
        pass,
        criteria: extract_criteria(unit),
    }
}

// ---------------------------------------------------------------------------
// Interactive-session payload mapping: wire option/archetype/pin types -> the
// darkrun-ui prop data the QuestionView / DirectionView / PickerView consume.
// ---------------------------------------------------------------------------

/// Map a wire [`QuestionOption`] onto a UI [`OptionCard`].
pub fn option_card(o: &QuestionOption) -> OptionCard {
    OptionCard {
        id: o.id.clone(),
        label: o.label.clone(),
        image_url: o.image_url.clone(),
        description: o.description.clone(),
    }
}

/// Map every option on a question payload into UI cards.
pub fn option_cards(opts: &[QuestionOption]) -> Vec<OptionCard> {
    opts.iter().map(option_card).collect()
}

/// Map a wire [`DirectionArchetype`] onto a UI [`ArchetypeCard`].
pub fn archetype_card(a: &DirectionArchetype) -> ArchetypeCard {
    ArchetypeCard {
        id: a.id.clone(),
        label: a.label.clone(),
        image_url: a.image_url.clone(),
        description: a.description.clone(),
    }
}

/// Map every archetype on a direction payload into UI cards.
pub fn archetype_cards(archs: &[DirectionArchetype]) -> Vec<ArchetypeCard> {
    archs.iter().map(archetype_card).collect()
}

/// Map a wire [`PickerOption`] onto a UI [`PickerItem`]. `secondary` is a bool
/// flag on the wire ("show all" grouping); it carries no display text, so it is
/// not projected into the item's `secondary` slot.
pub fn picker_item(o: &PickerOption) -> PickerItem {
    PickerItem {
        id: o.id.clone(),
        label: o.label.clone(),
        description: o.description.clone(),
        secondary: None,
    }
}

/// Map every option on a picker payload into UI items.
pub fn picker_items(opts: &[PickerOption]) -> Vec<PickerItem> {
    opts.iter().map(picker_item).collect()
}

/// Map a wire [`DirectionPin`] (already-normalized `0..1`) onto a UI
/// [`PinPoint`]. The constructor re-clamps defensively.
pub fn pin_point(p: &DirectionPin) -> PinPoint {
    PinPoint::new(p.x, p.y, p.note.clone())
}

/// Map every pin on a direction's annotations into UI pin points.
pub fn pin_points(pins: &[DirectionPin]) -> Vec<PinPoint> {
    pins.iter().map(pin_point).collect()
}

/// Project a UI [`PinPoint`] back onto a wire [`DirectionPin`] for submission.
pub fn pin_to_wire(p: &PinPoint) -> DirectionPin {
    DirectionPin {
        x: p.x,
        y: p.y,
        note: p.note.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn phase_and_gate_round_trip() {
        assert_eq!(phase(RunPhase::Manufacture), Phase::Manufacture);
        assert_eq!(checkpoint_kind(GateType::Await), CheckpointKind::Await);
    }

    #[test]
    fn status_tones_split_approve_vs_changes() {
        assert_eq!(status_tone(SessionStatus::Approved), Tone::Ok);
        assert_eq!(status_tone(SessionStatus::ChangesRequested), Tone::Danger);
        assert_eq!(status_tone(SessionStatus::Pending), Tone::Warn);
    }

    #[test]
    fn unit_view_reads_titles_type_and_pass() {
        let u = json!({
            "title": "Wire the importer",
            "type": "feature",
            "status": "Active",
            "pass": 2
        });
        let view = unit_view(&u);
        assert_eq!(view.title, "Wire the importer");
        assert_eq!(view.unit_type.as_deref(), Some("feature"));
        assert_eq!(view.status_label, "active");
        assert_eq!(view.tone, Tone::Info);
        assert_eq!(view.pass, 2);
    }

    #[test]
    fn unit_view_falls_back_to_slug_then_default() {
        let with_slug = json!({ "slug": "alpha" });
        assert_eq!(unit_view(&with_slug).title, "alpha");
        let bare = json!({});
        let v = unit_view(&bare);
        assert_eq!(v.title, "unit");
        assert_eq!(v.status_label, "pending");
        assert!(v.criteria.is_empty());
    }

    #[test]
    fn criteria_accepts_strings_and_objects() {
        let strings = json!({ "criteria": ["builds green", "tests pass"] });
        assert_eq!(
            unit_view(&strings).criteria,
            vec!["builds green".to_string(), "tests pass".to_string()]
        );
        let objects = json!({
            "completion_criteria": [
                { "text": "API wired" },
                { "description": "Docs updated" }
            ]
        });
        assert_eq!(
            unit_view(&objects).criteria,
            vec!["API wired".to_string(), "Docs updated".to_string()]
        );
    }

    #[test]
    fn option_card_carries_image_and_description() {
        let o = QuestionOption {
            id: "warm".into(),
            label: "Warm palette".into(),
            image_url: Some("https://img/warm.png".into()),
            description: Some("amber + rust".into()),
        };
        let card = option_card(&o);
        assert_eq!(card.id, "warm");
        assert_eq!(card.label, "Warm palette");
        assert_eq!(card.image_url.as_deref(), Some("https://img/warm.png"));
        assert_eq!(card.description.as_deref(), Some("amber + rust"));
    }

    #[test]
    fn option_card_without_image_is_placeholder_ready() {
        let o = QuestionOption {
            id: "plain".into(),
            label: "Plain".into(),
            image_url: None,
            description: None,
        };
        let card = option_card(&o);
        assert!(card.image_url.is_none());
        assert!(card.description.is_none());
    }

    #[test]
    fn option_cards_maps_all() {
        let opts = vec![
            QuestionOption {
                id: "a".into(),
                label: "A".into(),
                image_url: None,
                description: None,
            },
            QuestionOption {
                id: "b".into(),
                label: "B".into(),
                image_url: Some("u".into()),
                description: None,
            },
        ];
        let cards = option_cards(&opts);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, "a");
        assert_eq!(cards[1].image_url.as_deref(), Some("u"));
    }

    #[test]
    fn archetype_card_carries_required_fields() {
        let a = DirectionArchetype {
            id: "editorial".into(),
            label: "Editorial".into(),
            image_url: "https://img/ed.png".into(),
            description: "serif, airy".into(),
        };
        let card = archetype_card(&a);
        assert_eq!(card.id, "editorial");
        assert_eq!(card.label, "Editorial");
        assert_eq!(card.image_url, "https://img/ed.png");
        assert_eq!(card.description, "serif, airy");
    }

    #[test]
    fn picker_item_maps_label_and_description() {
        let o = PickerOption {
            id: "sw".into(),
            label: "software-factory".into(),
            description: Some("ship code".into()),
            secondary: Some(true),
        };
        let item = picker_item(&o);
        assert_eq!(item.id, "sw");
        assert_eq!(item.label, "software-factory");
        assert_eq!(item.description.as_deref(), Some("ship code"));
        // The wire `secondary` bool is a grouping flag, not display text.
        assert!(item.secondary.is_none());
    }

    #[test]
    fn pin_point_round_trips_through_wire() {
        let wire = DirectionPin { x: 0.25, y: 0.75, note: "tighten".into() };
        let ui = pin_point(&wire);
        assert_eq!(ui.x, 0.25);
        assert_eq!(ui.y, 0.75);
        assert_eq!(ui.note, "tighten");
        let back = pin_to_wire(&ui);
        assert_eq!(back.x, 0.25);
        assert_eq!(back.y, 0.75);
        assert_eq!(back.note, "tighten");
    }

    #[test]
    fn pin_point_clamps_out_of_range_wire_values() {
        // A malformed pin (out of 0..1) is clamped on import.
        let wire = DirectionPin { x: 1.5, y: -0.2, note: "bad".into() };
        let ui = pin_point(&wire);
        assert_eq!(ui.x, 1.0);
        assert_eq!(ui.y, 0.0);
    }

    #[test]
    fn pin_points_maps_all() {
        let pins = vec![
            DirectionPin { x: 0.1, y: 0.2, note: "a".into() },
            DirectionPin { x: 0.3, y: 0.4, note: "b".into() },
        ];
        let mapped = pin_points(&pins);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[1].note, "b");
    }
}

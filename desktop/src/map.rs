//! The boundary mapping: `darkrun-api` wire enums + opaque parser `Value`s ->
//! the `darkrun-ui` design-system kinds the components consume.
//!
//! `darkrun-ui` deliberately has no `darkrun-core`/`darkrun-api` dependency, so
//! every domain->UI translation lives here, one small function each. The unit
//! and criteria payloads are loose `serde_json::Value`s by design; we probe a
//! handful of conventional keys and degrade gracefully when one is absent.


use darkrun_api::common::{FeedbackSeverity, GateType, SessionStatus};
use darkrun_api::feedback::FeedbackItem;
use darkrun_api::proof::{Proof, Surface};
use darkrun_api::session::{
    DirectionArchetype, DirectionPin, PickerOption, QuestionOption, RunCurrentState, RunPhase,
    StationStateInfo, ViewArtifact, ViewArtifactKind,
};
use darkrun_ui::components::factory::CheckpointKind;
use darkrun_ui::components::feedback::{FeedbackEntry, Severity};
use darkrun_ui::components::station_strip::{StationItem, StationStatus};
use darkrun_ui::components::proof_panel::{
    AuditRow, BenchStat, ProofMetricKind, ProofView, VitalMetric,
};
use darkrun_ui::components::run_list::RunCardData;
use darkrun_ui::components::session_views::{ArchetypeCard, OptionCard, PickerItem};
use darkrun_ui::components::view_artifacts::ArtifactEntry;
use darkrun_ui::graph::layout::GraphEdge;
use darkrun_ui::graph::view::UnitGraphNode;
use darkrun_ui::kinds::{Phase, Tone};
use darkrun_ui::selection::PinPoint;
use darkrun_ui::view::{
    classify_vital, format_latency_ms, format_samples, format_throughput, format_vital,
    ArtifactKind,
};
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

// ---------------------------------------------------------------------------
// Station strip + phase subheader projection. The review payload now carries
// `station_states` (an ordered per-factory map) AND a `current_state.phase`, so
// the TOP-level assembly line (the station strip) and the SECONDARY phase
// subheader scoped to the current station are both driven straight off the wire.
// ---------------------------------------------------------------------------

/// Decide one station's [`StationStatus`] from its wire snapshot relative to the
/// run's active station.
///
/// `merged_into_main` is the only authoritative predicate — a merged station is
/// done. The active station (matching `current_station`) reads `Current`;
/// everything else is `Pending`. We also honor an explicit `status` token when
/// it canonicalizes to done/current so a stale `merged_into_main` never blanks a
/// finished station.
fn station_status(
    info: &StationStateInfo,
    index: usize,
    active_index: Option<usize>,
) -> StationStatus {
    // A merged station is authoritatively Done regardless of position.
    if info.merged_into_main {
        return StationStatus::Done;
    }
    if let Some(tok) = info.status.as_deref() {
        if matches!(StationStatus::parse(tok), StationStatus::Done) {
            return StationStatus::Done;
        }
    }
    // Otherwise the index-relative ordering comes from the SHARED
    // `darkrun_core::derive::station_status` — the same pure logic the engine and
    // HTTP run — so the desktop strip can't disagree with the other surfaces.
    match darkrun_core::derive::station_status(index, active_index) {
        darkrun_core::domain::Status::Completed => StationStatus::Done,
        darkrun_core::domain::Status::Active => StationStatus::Current,
        _ => StationStatus::Pending,
    }
}

/// Project the review payload's ordered `station_states` into the strip's
/// [`StationItem`] list — the assembly line at the TOP of the review.
///
/// `station_states` is an ordered slice in FACTORY order (the engine builds it
/// from the factory's declared station list), so iteration preserves the station
/// line order — it is NOT sorted alphabetically. `feedback_stations` flags which
/// stations carry open feedback so the strip can ride an amber dot on them.
pub fn station_items(
    station_states: &[StationStateInfo],
    current_state: Option<&RunCurrentState>,
    feedback_stations: &[String],
) -> Vec<StationItem> {
    let current = current_state.map(|s| s.station.as_str()).filter(|s| !s.is_empty());
    let active_index = current.and_then(|c| station_states.iter().position(|s| s.station == c));
    station_states
        .iter()
        .enumerate()
        .map(|(i, info)| {
            let status = station_status(info, i, active_index);
            let has_feedback = feedback_stations.iter().any(|s| s == &info.station);
            StationItem { name: info.station.clone(), status, has_feedback }
        })
        .collect()
}

/// The active phase for the current station's subheader, resolved from
/// `current_state.phase` (the now-live pipeline). `None` leaves the phase strip
/// all-pending.
pub fn station_phase(current_state: Option<&RunCurrentState>) -> Option<Phase> {
    current_state.and_then(|s| s.phase).map(phase)
}

// ---------------------------------------------------------------------------
// Feedback inbox projection: wire `FeedbackItem`s -> the severity-grouped
// `FeedbackEntry` rows the inbox renders, and the open-count the checkpoint reads.
// ---------------------------------------------------------------------------

/// Map a wire [`FeedbackSeverity`] onto the UI [`Severity`]. An unclassified
/// item (no severity yet) reads as a [`Severity::Should`] so it surfaces as
/// actionable rather than a silent nit.
pub fn feedback_severity(s: Option<FeedbackSeverity>) -> Severity {
    match s {
        Some(FeedbackSeverity::Blocker) => Severity::Must,
        Some(FeedbackSeverity::High) => Severity::Should,
        Some(FeedbackSeverity::Medium) => Severity::Should,
        Some(FeedbackSeverity::Low) => Severity::Nit,
        None => Severity::Should,
    }
}

/// Project one wire [`FeedbackItem`] into a UI [`FeedbackEntry`]. A non-blocking
/// status (closed / addressed / answered / …) renders the row dimmed.
pub fn feedback_entry(item: &FeedbackItem) -> FeedbackEntry {
    let locator = item
        .source_ref
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if item.title.is_empty() {
                item.feedback_id.clone()
            } else {
                item.title.clone()
            }
        });
    let comment = if item.body.is_empty() {
        item.title.clone()
    } else {
        item.body.clone()
    };
    // The persisted inline anchor surfaces as a quoted span on the row, so the
    // exact text a comment was anchored to is visible wherever the feedback is.
    let anchor = item
        .inline_anchor
        .as_ref()
        .map(|a| {
            let mut quote: String = a.selected_text.chars().take(64).collect();
            if a.selected_text.chars().count() > 64 {
                quote.push('\u{2026}');
            }
            format!("\u{201c}{quote}\u{201d} \u{00b7} {}", a.location)
        })
        .unwrap_or_default();
    FeedbackEntry {
        id: item.feedback_id.clone(),
        severity: feedback_severity(item.severity),
        locator,
        anchor,
        comment,
        author: item.author.clone(),
        resolved: !item.status.blocks_gate(),
    }
}

/// Project every feedback item into UI entries (severity-grouped at render).
pub fn feedback_entries(items: &[FeedbackItem]) -> Vec<FeedbackEntry> {
    items.iter().map(feedback_entry).collect()
}

/// Whether a feedback item is an OPEN blocker-or-high (a `must`/`should` that
/// holds the checkpoint's clean Approve). Drives the severity-driven primary.
pub fn feedback_blocks_checkpoint(item: &FeedbackItem) -> bool {
    if !item.status.blocks_gate() {
        return false;
    }
    matches!(feedback_severity(item.severity), Severity::Must | Severity::Should)
}

/// Project a `darkrun-api` [`RunSummary`] into the UI [`RunCardData`] view-model
/// the run browser renders. The wire `phase` is a display string (the
/// `StationPhase` serde name); unknown / absent phases leave the pipeline strip
/// all-pending. The status string passes straight through — the UI maps it onto
/// a badge tone via `run_status_tone`.
pub fn run_card(summary: &darkrun_api::RunSummary) -> RunCardData {
    RunCardData {
        slug: summary.slug.clone(),
        title: summary.title.clone(),
        factory: summary.factory.clone(),
        active_station: summary.active_station.clone(),
        phase: summary.phase.as_deref().and_then(Phase::from_name),
        status: summary.status.clone(),
        completed: summary.progress.completed,
        total: summary.progress.total,
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

/// Middle-ellipsize `s` to at most `max` characters (graph node labels).
pub fn ellipsize(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2 + keep % 2;
    let tail = keep / 2;
    let h: String = s.chars().take(head).collect();
    let t: String = s.chars().skip(n - tail).collect();
    format!("{h}\u{2026}{t}")
}

/// Project the review payload's raw unit documents into the [`UnitGraph`]'s
/// nodes + edges — the dependency DAG at the top of the Units tab.
///
/// The wire unit is a serialized `Unit` (`{slug, frontmatter: {...}, title}`),
/// so fields are probed FLAT first, then under `frontmatter` (status and
/// `depends_on` live there). Edges keep only endpoints that resolve to a known
/// unit, so a stale dependency never draws a dangling arrow.
pub fn unit_graph(units: &[Value]) -> (Vec<UnitGraphNode>, Vec<GraphEdge>) {
    let nested = |u: &Value, k: &str| -> Option<Value> {
        u.get(k)
            .cloned()
            .or_else(|| u.get("frontmatter").and_then(|f| f.get(k)).cloned())
    };
    let id_of = |u: &Value| {
        first_str(u, &["slug", "id"])
            .or_else(|| first_str(u, &["title", "name"]))
            .unwrap_or_else(|| "unit".to_string())
    };

    let mut nodes = Vec::new();
    let mut ids: std::collections::BTreeSet<String> = Default::default();
    for u in units {
        let id = id_of(u);
        // Node labels are the SLUG, ellipsized — real unit titles are sentences
        // and a graph node is a handle, not a paragraph (the row below carries
        // the full title).
        let label = ellipsize(&id, 18);
        let status = nested(u, "status")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "pending".to_string())
            .to_ascii_lowercase();
        ids.insert(id.clone());
        nodes.push(UnitGraphNode::new(id, label).with_tone(label_tone(&status)));
    }

    let mut edges = Vec::new();
    for u in units {
        let to = id_of(u);
        let deps = nested(u, "depends_on").and_then(|v| v.as_array().cloned()).unwrap_or_default();
        for d in deps {
            if let Some(from) = d.as_str() {
                if ids.contains(from) && from != to {
                    edges.push(GraphEdge { from: from.to_string(), to: to.clone() });
                }
            }
        }
    }
    (nodes, edges)
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
        image_url_light: o.image_url_light.clone(),
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
        image_url_light: a.image_url_light.clone(),
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

// ---------------------------------------------------------------------------
// View artifact mapping: wire ViewArtifact -> the ArtifactEntry the artifact
// browser consumes.
// ---------------------------------------------------------------------------

/// Map a wire [`ViewArtifactKind`] onto the UI [`ArtifactKind`].
pub fn artifact_kind(k: ViewArtifactKind) -> ArtifactKind {
    match k {
        ViewArtifactKind::File => ArtifactKind::File,
        ViewArtifactKind::Image => ArtifactKind::Image,
        ViewArtifactKind::Screenshot => ArtifactKind::Screenshot,
        ViewArtifactKind::Markdown => ArtifactKind::Markdown,
        ViewArtifactKind::Json => ArtifactKind::Json,
    }
}

/// Map a wire [`ViewArtifact`] onto a UI [`ArtifactEntry`].
pub fn artifact_entry(a: &ViewArtifact) -> ArtifactEntry {
    ArtifactEntry {
        id: a.id.clone(),
        path: a.path.clone(),
        kind: artifact_kind(a.kind),
        label: a.label.clone(),
        thumbnail_url: a.thumbnail_url.clone(),
        url: a.url.clone(),
        body: None,
    }
}

/// Map every artifact on a view payload into UI entries.
pub fn artifact_entries(arts: &[ViewArtifact]) -> Vec<ArtifactEntry> {
    arts.iter().map(artifact_entry).collect()
}

// ---------------------------------------------------------------------------
// Proof mapping: wire Proof (surface-tagged web/bench blocks) -> the ProofView
// the proof panel renders, with the numbers pre-formatted + classified.
// ---------------------------------------------------------------------------

/// The canonical order web vitals are surfaced in the panel.
const VITAL_ORDER: [&str; 5] = ["lcp", "fcp", "ttfb", "inp", "cls"];

/// Which display block a surface routes its proof through.
pub fn proof_metric_kind(s: Surface) -> ProofMetricKind {
    if s.is_visual() {
        ProofMetricKind::Web
    } else if s.is_bench() {
        ProofMetricKind::Bench
    } else {
        ProofMetricKind::Terminal
    }
}

/// Map a wire [`Proof`] onto the display-ready [`ProofView`], pre-formatting and
/// classifying every number so the panel stays a thin renderer.
pub fn proof_view(proof: &Proof) -> ProofView {
    let kind = proof_metric_kind(proof.surface);
    let block_matches_surface = proof.block_matches_surface();

    let mut vitals = Vec::new();
    let mut audits = Vec::new();
    let mut screenshot_url = None;
    if let Some(web) = &proof.web {
        // Known vitals first, in canonical order; then any extras the engine
        // emitted, in their stable BTreeMap order.
        for key in VITAL_ORDER {
            if let Some(value) = web.vitals.get(key) {
                vitals.push(vital_metric(key, *value));
            }
        }
        for (key, value) in &web.vitals {
            if !VITAL_ORDER.contains(&key.as_str()) {
                vitals.push(vital_metric(key, *value));
            }
        }
        audits = web
            .audits
            .iter()
            .map(|a| AuditRow {
                name: a.name.clone(),
                value: a.value.clone(),
                pass: a.pass,
            })
            .collect();
        screenshot_url = web.screenshot_url.clone();
    }

    let mut bench = Vec::new();
    if let Some(b) = &proof.bench {
        if let Some(v) = b.p50 {
            bench.push(BenchStat { label: "p50".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.p95 {
            bench.push(BenchStat { label: "p95".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.p99 {
            bench.push(BenchStat { label: "p99".to_string(), display: format_latency_ms(v) });
        }
        if let Some(v) = b.throughput {
            bench.push(BenchStat {
                label: "throughput".to_string(),
                display: format_throughput(v),
            });
        }
        if let Some(n) = b.samples {
            bench.push(BenchStat { label: "samples".to_string(), display: format_samples(n) });
        }
    }

    ProofView {
        surface: proof.surface.as_str().to_string(),
        kind,
        vitals,
        audits,
        screenshot_url,
        bench,
        block_matches_surface,
    }
}

/// Build one classified, pre-formatted [`VitalMetric`] from a raw vital.
fn vital_metric(key: &str, value: f64) -> VitalMetric {
    VitalMetric {
        key: key.to_string(),
        value,
        display: format_vital(key, value),
        verdict: classify_vital(key, value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_api::common::{AuthorType, FeedbackOrigin, FeedbackStatus};
    use serde_json::json;

    #[test]
    fn phase_and_gate_round_trip() {
        assert_eq!(phase(RunPhase::Manufacture), Phase::Manufacture);
        assert_eq!(checkpoint_kind(GateType::Await), CheckpointKind::Await);
    }

    fn station_info(name: &str, merged: bool, status: Option<&str>) -> StationStateInfo {
        StationStateInfo {
            station: name.to_string(),
            merged_into_main: merged,
            status: status.map(str::to_string),
            phase: None,
            started_at: None,
            completed_at: None,
            gate_entered_at: None,
            gate_outcome: None,
        }
    }

    #[test]
    fn station_items_marks_done_current_pending() {
        // An ordered Vec (factory order), NOT alphabetical — order is preserved.
        let states = vec![
            station_info("01-frame", true, None),
            station_info("02-build", false, None),
            station_info("03-harden", false, None),
        ];
        let cur = RunCurrentState {
            station: "02-build".into(),
            ..Default::default()
        };
        let items = station_items(&states, Some(&cur), &["02-build".to_string()]);
        assert_eq!(items.len(), 3);
        // The Vec order is the station line order.
        assert_eq!(items[0].status, StationStatus::Done);
        assert_eq!(items[1].status, StationStatus::Current);
        assert!(items[1].has_feedback);
        assert_eq!(items[2].status, StationStatus::Pending);
        assert!(!items[2].has_feedback);
    }

    #[test]
    fn station_status_honors_explicit_done_token() {
        // A not-yet-merged station whose status reads "done" still shows done
        // (the explicit-done token path returns before the index-relative logic,
        // so the index/active here are immaterial).
        let info = station_info("x", false, Some("completed"));
        assert_eq!(station_status(&info, 2, Some(0)), StationStatus::Done);
    }

    #[test]
    fn feedback_severity_maps_and_defaults() {
        assert_eq!(feedback_severity(Some(FeedbackSeverity::Blocker)), Severity::Must);
        assert_eq!(feedback_severity(Some(FeedbackSeverity::High)), Severity::Should);
        assert_eq!(feedback_severity(Some(FeedbackSeverity::Medium)), Severity::Should);
        assert_eq!(feedback_severity(Some(FeedbackSeverity::Low)), Severity::Nit);
        // Unclassified surfaces as actionable, not a silent nit.
        assert_eq!(feedback_severity(None), Severity::Should);
    }

    fn feedback_item(id: &str, sev: Option<FeedbackSeverity>, status: FeedbackStatus) -> FeedbackItem {
        FeedbackItem {
            feedback_id: id.into(),
            title: "t".into(),
            body: "b".into(),
            status,
            origin: FeedbackOrigin::UserVisual,
            severity: sev,
            author: "you".into(),
            author_type: AuthorType::Human,
            created_at: "2026-05-31T00:00:00Z".into(),
            visit: 1,
            source_ref: Some("payment.rs".into()),
            closed_by: None,
            resolution: None,
            replies: vec![],
            inline_anchor: None,
            scope: None,
            iterations: vec![],
            closure_reply: None,
            closure_reply_unread: None,
        }
    }

    #[test]
    fn feedback_entry_resolves_closed_items() {
        let open = feedback_item("FB-01", Some(FeedbackSeverity::Blocker), FeedbackStatus::Pending);
        let closed = feedback_item("FB-02", Some(FeedbackSeverity::Low), FeedbackStatus::Closed);
        let e_open = feedback_entry(&open);
        assert_eq!(e_open.severity, Severity::Must);
        assert!(!e_open.resolved);
        assert_eq!(e_open.locator, "payment.rs");
        assert!(feedback_entry(&closed).resolved);
    }

    #[test]
    fn feedback_blocks_checkpoint_only_open_high_or_blocker() {
        let blocker = feedback_item("a", Some(FeedbackSeverity::Blocker), FeedbackStatus::Pending);
        let nit = feedback_item("b", Some(FeedbackSeverity::Low), FeedbackStatus::Pending);
        let closed_blocker =
            feedback_item("c", Some(FeedbackSeverity::Blocker), FeedbackStatus::Closed);
        assert!(feedback_blocks_checkpoint(&blocker));
        assert!(!feedback_blocks_checkpoint(&nit));
        assert!(!feedback_blocks_checkpoint(&closed_blocker));
    }

    #[test]
    fn status_tones_split_approve_vs_changes() {
        assert_eq!(status_tone(SessionStatus::Approved), Tone::Ok);
        assert_eq!(status_tone(SessionStatus::ChangesRequested), Tone::Danger);
        assert_eq!(status_tone(SessionStatus::Pending), Tone::Warn);
    }

    #[test]
    fn unit_graph_probes_nested_frontmatter_and_filters_dangling_edges() {
        let units = vec![
            serde_json::json!({"slug":"a","title":"A","frontmatter":{"status":"completed","depends_on":[]}}),
            serde_json::json!({"slug":"b","title":"B","frontmatter":{"status":"in_progress","depends_on":["a","ghost"]}}),
        ];
        let (nodes, edges) = unit_graph(&units);
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1, "the dangling `ghost` edge is dropped");
        assert_eq!(edges[0].from, "a");
        assert_eq!(edges[0].to, "b");
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
            image_url_light: None,
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
            image_url_light: None,
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
                image_url_light: None,
                description: None,
            },
            QuestionOption {
                id: "b".into(),
                label: "B".into(),
                image_url: Some("u".into()),
                image_url_light: None,
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
            image_url_light: None,
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

    // --- view artifact mapping ---------------------------------------------

    #[test]
    fn artifact_kind_maps_every_wire_kind() {
        assert_eq!(artifact_kind(ViewArtifactKind::File), ArtifactKind::File);
        assert_eq!(artifact_kind(ViewArtifactKind::Image), ArtifactKind::Image);
        assert_eq!(
            artifact_kind(ViewArtifactKind::Screenshot),
            ArtifactKind::Screenshot
        );
        assert_eq!(
            artifact_kind(ViewArtifactKind::Markdown),
            ArtifactKind::Markdown
        );
        assert_eq!(artifact_kind(ViewArtifactKind::Json), ArtifactKind::Json);
    }

    #[test]
    fn artifact_entry_carries_paths_and_urls() {
        let wire = ViewArtifact {
            id: "a1".into(),
            path: "out/home.png".into(),
            kind: ViewArtifactKind::Screenshot,
            label: "Home".into(),
            thumbnail_url: Some("/thumb/a1".into()),
            url: Some("/fetch/a1".into()),
        };
        let entry = artifact_entry(&wire);
        assert_eq!(entry.id, "a1");
        assert_eq!(entry.path, "out/home.png");
        assert_eq!(entry.kind, ArtifactKind::Screenshot);
        assert!(entry.kind.is_reviewable());
        assert_eq!(entry.thumbnail_url.as_deref(), Some("/thumb/a1"));
        assert_eq!(entry.url.as_deref(), Some("/fetch/a1"));
    }

    #[test]
    fn artifact_entries_maps_all() {
        let arts = vec![
            ViewArtifact {
                id: "a".into(),
                path: "x".into(),
                kind: ViewArtifactKind::Markdown,
                label: "X".into(),
                thumbnail_url: None,
                url: None,
            },
            ViewArtifact {
                id: "b".into(),
                path: "y".into(),
                kind: ViewArtifactKind::Json,
                label: "Y".into(),
                thumbnail_url: None,
                url: None,
            },
        ];
        let entries = artifact_entries(&arts);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].kind, ArtifactKind::Json);
    }

    // --- proof mapping -----------------------------------------------------

    #[test]
    fn proof_metric_kind_routes_by_surface() {
        assert_eq!(proof_metric_kind(Surface::WebUi), ProofMetricKind::Web);
        assert_eq!(proof_metric_kind(Surface::Desktop), ProofMetricKind::Web);
        assert_eq!(proof_metric_kind(Surface::Library), ProofMetricKind::Bench);
        assert_eq!(proof_metric_kind(Surface::Cli), ProofMetricKind::Terminal);
    }

    #[test]
    fn proof_view_web_orders_vitals_and_formats() {
        use darkrun_api::proof::{AuditResult, WebProof};
        use std::collections::BTreeMap;
        let mut vitals = BTreeMap::new();
        // Insert out of canonical order to prove ordering.
        vitals.insert("cls".to_string(), 0.02);
        vitals.insert("lcp".to_string(), 1200.0);
        vitals.insert("ttfb".to_string(), 640.0);
        let proof = Proof::web(
            Surface::WebUi,
            WebProof {
                vitals,
                audits: vec![AuditResult {
                    name: "contrast".into(),
                    value: "4.8:1".into(),
                    pass: true,
                }],
                screenshot_url: Some("/shot.png".into()),
            },
        );
        let view = proof_view(&proof);
        assert_eq!(view.surface, "web_ui");
        assert_eq!(view.kind, ProofMetricKind::Web);
        // Canonical order: lcp before ttfb before cls.
        let keys: Vec<&str> = view.vitals.iter().map(|v| v.key.as_str()).collect();
        assert_eq!(keys, vec!["lcp", "ttfb", "cls"]);
        assert_eq!(view.vitals[0].display, "1.20 s");
        assert_eq!(view.vitals[0].verdict, darkrun_ui::view::VitalVerdict::Good);
        assert_eq!(view.audits.len(), 1);
        assert_eq!(view.screenshot_url.as_deref(), Some("/shot.png"));
        assert!(view.bench.is_empty());
        assert!(view.block_matches_surface);
    }

    #[test]
    fn proof_view_bench_formats_percentiles_and_throughput() {
        use darkrun_api::proof::BenchProof;
        let proof = Proof::bench(
            Surface::Library,
            BenchProof {
                p50: Some(0.5),
                p95: Some(1.2),
                p99: Some(2.0),
                throughput: Some(50_000.0),
                samples: Some(1_000),
            },
        );
        let view = proof_view(&proof);
        assert_eq!(view.kind, ProofMetricKind::Bench);
        let labels: Vec<&str> = view.bench.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, vec!["p50", "p95", "p99", "throughput", "samples"]);
        assert_eq!(view.bench[3].display, "50.0k ops/s");
        assert_eq!(view.bench[4].display, "1,000");
        assert!(view.vitals.is_empty());
    }

    #[test]
    fn proof_view_flags_block_mismatch() {
        use darkrun_api::proof::BenchProof;
        // A visual surface carrying only a bench block does not match its route.
        let proof = Proof {
            surface: Surface::WebUi,
            web: None,
            bench: Some(BenchProof::default()),
        };
        let view = proof_view(&proof);
        assert!(!view.block_matches_surface);
        assert_eq!(view.kind, ProofMetricKind::Web);
    }

    #[test]
    fn phase_maps_every_run_phase() {
        assert_eq!(phase(RunPhase::Spec), Phase::Spec);
        assert_eq!(phase(RunPhase::Review), Phase::Review);
        assert_eq!(phase(RunPhase::Manufacture), Phase::Manufacture);
        assert_eq!(phase(RunPhase::Audit), Phase::Audit);
        assert_eq!(phase(RunPhase::Reflect), Phase::Reflect);
        assert_eq!(phase(RunPhase::Checkpoint), Phase::Checkpoint);
    }

    #[test]
    fn status_tone_covers_decided_and_answered() {
        assert_eq!(status_tone(SessionStatus::Decided), Tone::Info);
        assert_eq!(status_tone(SessionStatus::Answered), Tone::Info);
    }

    #[test]
    fn label_tone_unknown_token_is_neutral() {
        assert_eq!(label_tone("whatever-unmapped"), Tone::Neutral);
        assert_eq!(label_tone("done"), Tone::Ok);
        assert_eq!(label_tone("blocked"), Tone::Danger);
        assert_eq!(label_tone("queued"), Tone::Warn);
    }

    #[test]
    fn station_status_derive_completed_arm_for_a_passed_station() {
        // A not-yet-merged, no-token station BEFORE the active index resolves to
        // Done via the shared derive (the Completed arm).
        let info = station_info("early", false, None);
        assert_eq!(station_status(&info, 0, Some(2)), StationStatus::Done);
        // And the Pending arm for a station after the active index.
        assert_eq!(station_status(&info, 3, Some(1)), StationStatus::Pending);
    }

    #[test]
    fn feedback_entry_falls_back_to_id_and_title() {
        let mut item = feedback_item("FB-09", None, FeedbackStatus::Pending);
        // No source_ref and an empty title → the locator falls back to the id.
        item.source_ref = None;
        item.title = String::new();
        item.body = String::new();
        let e = feedback_entry(&item);
        assert_eq!(e.locator, "FB-09");
        assert_eq!(e.comment, ""); // empty body → empty title
        // A present title (still no source_ref) becomes the locator + comment.
        item.title = "Layout drifts".into();
        let e2 = feedback_entry(&item);
        assert_eq!(e2.locator, "Layout drifts");
        assert_eq!(e2.comment, "Layout drifts");
    }

    #[test]
    fn run_card_projects_summary_fields() {
        let summary = darkrun_api::RunSummary {
            slug: "checkout".into(),
            title: "Checkout flow".into(),
            factory: "software".into(),
            active_station: "build".into(),
            phase: Some("manufacture".into()),
            status: "active".into(),
            progress: darkrun_api::runs::StationProgress { completed: 2, total: 6 },
            started_at: None,
            authored_by_me: true,
            author: None,
        };
        let card = run_card(&summary);
        assert_eq!(card.slug, "checkout");
        assert_eq!(card.title, "Checkout flow");
        assert_eq!(card.factory, "software");
        assert_eq!(card.active_station, "build");
        assert_eq!(card.phase, Some(Phase::Manufacture));
        assert_eq!(card.completed, 2);
        assert_eq!(card.total, 6);
    }

    #[test]
    fn extract_criteria_skips_non_string_non_object_items() {
        // A mixed array — a number is dropped via the `_ => None` arm.
        let unit = json!({ "criteria": [42, "keep this", { "text": "and this" }] });
        assert_eq!(
            extract_criteria(&unit),
            vec!["keep this".to_string(), "and this".to_string()]
        );
    }

    #[test]
    fn proof_view_appends_extra_non_canonical_vitals() {
        use darkrun_api::proof::WebProof;
        use std::collections::BTreeMap;
        let mut vitals = BTreeMap::new();
        vitals.insert("lcp".to_string(), 1000.0);
        vitals.insert("tbt".to_string(), 120.0); // not in VITAL_ORDER → extra arm
        let proof = Proof::web(
            Surface::WebUi,
            WebProof { vitals, audits: vec![], screenshot_url: None },
        );
        let view = proof_view(&proof);
        let keys: Vec<&str> = view.vitals.iter().map(|v| v.key.as_str()).collect();
        // Canonical lcp first, then the extra appended after.
        assert_eq!(keys, vec!["lcp", "tbt"]);
    }
}

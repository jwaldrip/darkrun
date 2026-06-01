//! The live Review screen — the assembly-line IA (mockup section A/E/E·f/F).
//!
//! [`ReviewApp`] opens the session WebSocket, holds the latest
//! [`ReviewSessionPayload`] in a signal, and renders the review surface:
//!   - the [`StationStrip`] at the TOP — the prominent assembly line, driven off
//!     the payload's ordered `station_states`,
//!   - a compact phase subheader ([`StationPipeline`]) scoped to the current
//!     station, now live off `current_state.phase`,
//!   - a [`TabBar`] (Units / Outputs / Knowledge / Feedback / Overview) over the
//!     station body, each unit/output row carrying view + review(annotate)
//!     affordances and a feedback count,
//!   - a Feedback inbox listing every station annotation grouped by severity,
//!     reachable from a persistent header button,
//!   - the annotate surface ([`AnnotateToolbar`] + overlay + [`CommentPanel`])
//!     opened on an artifact, submitting via the wire, and
//!   - a single severity-driven [`CheckpointBar`] rendered ONLY at an active
//!     review/final gate, whose primary action darkens to Request-changes when
//!     open `should`/`must` annotations exist.
//!
//! Only the `Review` session variant is rendered in full; the other variants
//! (question / direction / picker / view) show a compact placeholder so an
//! unexpected payload never blanks the screen.

use darkrun_api::common::{FeedbackOrigin, FeedbackStatus};
use darkrun_api::feedback::FeedbackItem;
use darkrun_api::session::{
    DirectionAnnotations, DirectionSessionPayload, OutputArtifact, PickerSessionPayload,
    ProofSessionPayload, QuestionSessionPayload, ReviewSessionPayload, ViewSessionPayload,
    VisualReviewAnnotations, VisualReviewPin, VisualReviewSessionPayload,
};
use darkrun_api::{
    DirectionSelectRequest, FeedbackCreateRequest, OutputReviewRequest, PickerSelectRequest,
    QuestionAnswerRequest, ReviewDecisionRequest, SessionPayload,
};
use darkrun_ui::prelude::*;

use crate::map;
use crate::wire::{self, ConnConfig};

/// Connection state shown in the header so the operator always knows whether the
/// feed is live.
#[derive(Debug, Clone, PartialEq)]
enum Link {
    /// Dialing the WebSocket.
    Connecting,
    /// A payload has arrived.
    Live,
    /// The socket dropped; carries the reason.
    Down(String),
}

/// The result of the most recent decision POST, surfaced under the checkpoint.
#[derive(Debug, Clone, PartialEq)]
enum Decision {
    /// No decision submitted yet.
    Idle,
    /// A POST is in flight.
    Sending,
    /// The engine accepted the decision.
    Sent(String),
    /// The POST failed.
    Failed(String),
}

/// The root review component: owns the feed and renders the active payload.
#[component]
pub fn ReviewApp(cfg: ConnConfig) -> Element {
    let mut payload = use_signal(|| None::<SessionPayload>);
    let mut link = use_signal(|| Link::Connecting);
    let decision = use_signal(|| Decision::Idle);

    // Drive the session feed for the lifetime of the component. Each frame
    // updates the payload signal; a drop flips the link to Down.
    let feed_cfg = cfg.clone();
    use_future(move || {
        let cfg = feed_cfg.clone();
        async move {
            wire::run_session_feed(&cfg, move |event| match event {
                wire::FeedEvent::Payload(p) => {
                    payload.set(Some(*p));
                    link.set(Link::Live);
                }
                wire::FeedEvent::Disconnected(reason) => {
                    link.set(Link::Down(reason));
                }
            })
            .await;
        }
    });

    let shell = "padding:24px;display:flex;flex-direction:column;gap:16px;\
                 max-width:880px;margin:0 auto;";
    // Translucent surface so content blurs *through* the sticky header.
    let header_style = format!(
        "display:flex;align-items:center;justify-content:space-between;gap:12px;\
         position:sticky;top:0;z-index:10;padding:12px 0;\
         backdrop-filter:blur(8px);background:{base}ee;\
         border-bottom:1px solid {border};",
        base = tokens::SURFACE_BASE,
        border = tokens::BORDER,
    );

    rsx! {
        div { style: "{shell}",
            header {
                style: "{header_style}",
                Wordmark { variant: WordmarkVariant::OutlinedSolidRun, size: 28.0 }
                LinkBadge { link: link.read().clone() }
            }
            match payload.read().clone() {
                Some(SessionPayload::Review(review)) => review_body(cfg.clone(), review, decision),
                Some(SessionPayload::Question(q)) => question_session(cfg.clone(), q),
                Some(SessionPayload::Direction(d)) => direction_session(cfg.clone(), d),
                Some(SessionPayload::Picker(p)) => picker_session(cfg.clone(), p),
                Some(SessionPayload::View(v)) => view_session(cfg.clone(), v),
                Some(SessionPayload::VisualReview(vr)) => visual_review_session(cfg.clone(), vr),
                Some(SessionPayload::Proof(pr)) => proof_session(pr),
                None => rsx! {
                    Card {
                        p { style: "color:var(--dr-text-muted);",
                            "Waiting for the engine to push a session…"
                        }
                    }
                },
            }
        }
    }
}

/// A small connection-status badge for the header.
#[component]
fn LinkBadge(link: Link) -> Element {
    let (tone, label) = match &link {
        Link::Connecting => (Tone::Warn, "connecting".to_string()),
        Link::Live => (Tone::Ok, "live".to_string()),
        Link::Down(_) => (Tone::Danger, "offline".to_string()),
    };
    rsx! {
        Badge { tone, filled: true, "{label}" }
    }
}

/// Which artifact the operator is annotating, captured when a unit/output row's
/// "review" affordance is pressed. Carries enough to drive the annotate surface
/// (the toolbar's surface kind, the artifact label/path, and a screenshot URL
/// for the visual case).
#[derive(Debug, Clone, PartialEq)]
struct AnnotateTarget {
    /// Display label of the artifact.
    label: String,
    /// Run-relative path / locator.
    path: String,
    /// The work-item id (unit slug / output name) the annotation hangs on.
    work_id: String,
    /// Whether this is a visual surface (image / live HTML) or a text surface.
    visual: bool,
    /// Screenshot / image URL for a visual surface.
    screenshot_url: Option<String>,
}

/// The fully-rendered review surface — the assembly-line IA.
///
/// A plain function (not a `#[component]`) because the wire payload types don't
/// derive `PartialEq`, which the component macro requires of its props. It owns
/// the surface-local UI signals (active tab, the open annotate target, whether
/// the feedback inbox is open) and the fetched station feedback, then renders the
/// station strip, the phase subheader, the tabbed body, and the (severity-driven)
/// checkpoint bar.
fn review_body(
    cfg: ConnConfig,
    review: ReviewSessionPayload,
    decision: Signal<Decision>,
) -> Element {
    // --- Header context -----------------------------------------------------
    let station = review
        .station
        .clone()
        .or_else(|| review.current_state.as_ref().map(|s| s.station.clone()))
        .filter(|s| !s.is_empty());
    let active_phase = map::station_phase(review.current_state.as_ref());
    let title = review
        .run_slug
        .clone()
        .unwrap_or_else(|| "darkrun review".to_string());
    let run_slug = review.run_slug.clone();

    // --- Live station feedback (the inbox data + the checkpoint counts) ------
    // Fetched off the feedback HTTP route for the current station; the annotation
    // model surfaces every artifact annotation here as a feedback item.
    let feedback = use_signal(Vec::<FeedbackItem>::new);
    {
        let cfg = cfg.clone();
        let run = run_slug.clone();
        let st = station.clone();
        let mut feedback = feedback;
        use_future(move || {
            let cfg = cfg.clone();
            let run = run.clone();
            let st = st.clone();
            async move {
                if let (Some(run), Some(st)) = (run, st) {
                    if let Ok(resp) = wire::fetch_feedback(&cfg, &run, &st).await {
                        feedback.set(resp.items);
                    }
                }
            }
        });
    }
    let feedback_items = feedback.read().clone();
    let feedback_entries = map::feedback_entries(&feedback_items);
    let open_blockers = feedback_items
        .iter()
        .filter(|f| map::feedback_blocks_checkpoint(f))
        .count();
    let open_total = feedback_items
        .iter()
        .filter(|f| f.status.blocks_gate())
        .count();

    // The station strip: ordered station_states, with the current station's
    // open feedback flagged as the amber dot.
    let feedback_stations: Vec<String> = if open_total > 0 {
        station.clone().into_iter().collect()
    } else {
        Vec::new()
    };
    let stations = map::station_items(
        &review.station_states,
        review.current_state.as_ref(),
        &feedback_stations,
    );

    // --- Units + outputs (the tab bodies) -----------------------------------
    let units: Vec<map::UnitView> = review.units.iter().map(map::unit_view).collect();
    let outputs = review.output_artifacts.clone();
    let knowledge = review.knowledge_files.clone();
    let unit_outputs = review.unit_outputs.clone();

    // --- Surface-local UI state --------------------------------------------
    let active_tab = use_signal(|| "units".to_string());
    let annotate_target = use_signal(|| None::<AnnotateTarget>);
    let inbox_open = use_signal(|| false);

    // The tab strip, with the Feedback tab carrying the open-annotation count
    // (danger-red when any blocker/high is open).
    let tabs = build_tabs(units.len(), outputs.len(), knowledge.len(), open_total);
    let active = active_tab.read().clone();

    let mut tab_sig = active_tab;
    let mut inbox_sig = inbox_open;
    let inbox_is_open = *inbox_open.read();

    // The active gate predicate: only render the checkpoint at an actual
    // review/final gate that is currently blocking on a decision.
    let gate_open = review.await_active.unwrap_or(false);

    rsx! {
        // ── The assembly line (TOP) ────────────────────────────────────────
        ReviewHeader {
            title: title.clone(),
            station: station.clone(),
            phase: active_phase,
            status: map::status_tone(review.status),
            status_label: format!("{:?}", review.status).to_lowercase(),
            stations: stations.clone(),
            feedback_count: open_total as u32,
            feedback_alert: open_blockers > 0,
            on_open_feedback: move |_| inbox_sig.set(!inbox_is_open),
        }

        // ── The feedback inbox (severity-grouped), toggled from the header ──
        if inbox_is_open {
            {feedback_inbox_panel(cfg.clone(), run_slug.clone(), station.clone(), feedback, feedback_entries.clone())}
        }

        // ── The annotate surface, when an artifact is under review ──────────
        if let Some(target) = annotate_target.read().clone() {
            {annotate_panel(cfg.clone(), run_slug.clone(), station.clone(), target, annotate_target)}
        }

        // ── The tabbed station body ─────────────────────────────────────────
        Card {
            TabBar {
                tabs,
                active: active.clone(),
                on_select: move |id| tab_sig.set(id),
            }
            div { style: "margin-top:14px;",
                {tab_body(&active, &units, &outputs, &knowledge, &unit_outputs, &feedback_entries, &review, annotate_target, inbox_open)}
            }
        }

        // ── The single, severity-driven checkpoint control set ──────────────
        if gate_open {
            {checkpoint_section(cfg, review, decision, open_blockers)}
        }
    }
}

/// Build the review tab strip. The Feedback tab carries the open-annotation
/// count; it reads danger-red when any blocker/high is open.
fn build_tabs(units: usize, outputs: usize, knowledge: usize, feedback: usize) -> Vec<TabItem> {
    let feedback_tab = if feedback > 0 {
        TabItem::with_alert_count("feedback", "Feedback", feedback as u32)
    } else {
        TabItem::new("feedback", "Feedback")
    };
    vec![
        TabItem::with_count("units", "Units", units as u32),
        TabItem::with_count("outputs", "Outputs", outputs as u32),
        TabItem::with_count("knowledge", "Knowledge", knowledge as u32),
        feedback_tab,
        TabItem::new("overview", "Overview"),
    ]
}

/// The review header: the wordmark-free station strip + the compact phase
/// subheader scoped to the current station, plus the persistent feedback button.
#[component]
fn ReviewHeader(
    title: String,
    station: Option<String>,
    phase: Option<Phase>,
    status: Tone,
    status_label: String,
    stations: Vec<StationItem>,
    feedback_count: u32,
    feedback_alert: bool,
    on_open_feedback: EventHandler<MouseEvent>,
) -> Element {
    let title_style = format!(
        "font-family:{sans};font-size:15px;font-weight:700;color:{text};",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let sub_style = format!(
        "display:flex;align-items:center;gap:10px;margin-top:10px;\
         font-family:{mono};font-size:12px;color:{muted};",
        mono = tokens::FONT_MONO,
        muted = tokens::TEXT_MUTED,
    );
    let (fb_bg, fb_fg) = if feedback_alert {
        ("#f8514922", "#f5a3a3")
    } else {
        (tokens::SURFACE_OVERLAY, tokens::TEXT_MUTED)
    };
    let fb_btn = format!(
        "background:{fb_bg};color:{fb_fg};border:1px solid {border};\
         font-family:{sans};font-size:12px;border-radius:6px;padding:5px 11px;\
         cursor:pointer;display:flex;align-items:center;gap:6px;",
        border = tokens::BORDER_STRONG,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        Card {
            div {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;",
                span { style: "{title_style}", "{title}" }
                div { style: "display:flex;align-items:center;gap:8px;",
                    button {
                        class: "dr-feedback-open",
                        style: "{fb_btn}",
                        onclick: move |evt| on_open_feedback.call(evt),
                        "Feedback"
                        span {
                            style: format!(
                                "font-family:{};border-radius:999px;padding:0 6px;\
                                 background:{};color:{};",
                                tokens::FONT_MONO,
                                if feedback_alert { "#f8514933" } else { tokens::SURFACE_BASE },
                                fb_fg,
                            ),
                            "{feedback_count}"
                        }
                    }
                    Badge { tone: status, filled: true, "{status_label}" }
                }
            }
            // The assembly line — the prominent progress.
            div { style: "margin-top:14px;",
                StationStrip { stations }
            }
            // The phase subheader, scoped to the current station.
            div { style: "{sub_style}",
                if let Some(st) = station.clone() {
                    span { "station: {st}" }
                }
                StationPipeline { dots: strip_for(phase), labels: true }
            }
        }
    }
}

/// Render the body for the active tab.
#[allow(clippy::too_many_arguments)]
fn tab_body(
    active: &str,
    units: &[map::UnitView],
    outputs: &[OutputArtifact],
    knowledge: &[darkrun_api::session::KnowledgeFile],
    unit_outputs: &std::collections::BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>>,
    feedback: &[FeedbackEntry],
    review: &ReviewSessionPayload,
    annotate_target: Signal<Option<AnnotateTarget>>,
    inbox_open: Signal<bool>,
) -> Element {
    match active {
        "outputs" => output_tab(outputs, feedback, annotate_target),
        "knowledge" => knowledge_tab(knowledge),
        "feedback" => feedback_tab(feedback, inbox_open),
        "overview" => overview_tab(review),
        // Default to the units tab.
        _ => unit_tab(units, unit_outputs, feedback, annotate_target),
    }
}

/// A count of open feedback rows targeting a given work item, by locator match.
fn feedback_count_for(feedback: &[FeedbackEntry], needle: &str) -> usize {
    feedback
        .iter()
        .filter(|e| !e.resolved && (e.locator == needle || e.locator.contains(needle)))
        .count()
}

/// The Units tab: each unit row with its completion criteria, declared output
/// previews folded in (the unit's dependencies), plus a review(annotate)
/// affordance and a feedback count.
fn unit_tab(
    units: &[map::UnitView],
    unit_outputs: &std::collections::BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>>,
    feedback: &[FeedbackEntry],
    annotate_target: Signal<Option<AnnotateTarget>>,
) -> Element {
    if units.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No units in this review." }
        };
    }
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:10px;",
            for unit in units.iter() {
                {
                    let unit = unit.clone();
                    let previews = unit_outputs.get(&unit.title).cloned().unwrap_or_default();
                    let mut target = annotate_target;
                    let label = unit.title.clone();
                    let work_id = unit.title.clone();
                    let fb_n = feedback_count_for(feedback, &unit.title);
                    rsx! {
                        div { style: "display:flex;flex-direction:column;gap:6px;",
                            div { style: "display:flex;align-items:center;gap:8px;",
                                div { style: "flex:1;min-width:0;",
                                    UnitRow {
                                        title: unit.title.clone(),
                                        unit_type: unit.unit_type.clone(),
                                        status: unit.tone,
                                        status_label: unit.status_label.clone(),
                                        pass: unit.pass,
                                    }
                                }
                                if fb_n > 0 {
                                    Badge { tone: Tone::Warn, "{fb_n}" }
                                }
                                {row_actions(move |_| {
                                    target.set(Some(AnnotateTarget {
                                        label: label.clone(),
                                        path: work_id.clone(),
                                        work_id: work_id.clone(),
                                        visual: false,
                                        screenshot_url: None,
                                    }));
                                })}
                            }
                            if !unit.criteria.is_empty() {
                                ul { style: criteria_list(),
                                    for line in unit.criteria.iter() {
                                        li { style: "margin:2px 0;", "{line}" }
                                    }
                                }
                            }
                            // Declared outputs are the unit's dependencies — folded
                            // into the unit row rather than a separate DAG panel.
                            if !previews.is_empty() {
                                div { style: "margin-left:28px;display:flex;flex-direction:column;gap:4px;",
                                    for prev in previews.iter() {
                                        div {
                                            style: "display:flex;align-items:center;gap:8px;\
                                                    font-family:var(--dr-font-mono);font-size:11px;\
                                                    color:var(--dr-text-faint);",
                                            Badge { tone: if prev.exists { Tone::Ok } else { Tone::Warn }, "out" }
                                            span { "{prev.name}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The Outputs tab: declared deliverables, each with view + review(annotate)
/// affordances. Visual artifacts (image/html) open the spatial annotate surface;
/// the rest open the text surface.
fn output_tab(
    outputs: &[OutputArtifact],
    feedback: &[FeedbackEntry],
    annotate_target: Signal<Option<AnnotateTarget>>,
) -> Element {
    if outputs.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No declared outputs." }
        };
    }
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:8px;",
            for out in outputs.iter() {
                {
                    let out = out.clone();
                    let mut target = annotate_target;
                    let visual = output_is_visual(&out);
                    let label = out.name.clone();
                    let path = out.run_relative_path.clone().unwrap_or_else(|| out.name.clone());
                    let url = out.relative_path.clone();
                    let fb_n = feedback_count_for(feedback, &out.name);
                    rsx! {
                        div {
                            style: "display:flex;align-items:center;gap:10px;\
                                    font-family:var(--dr-font-mono);font-size:12px;\
                                    border:1px solid var(--dr-border);border-radius:6px;\
                                    padding:8px 10px;background:var(--dr-surface-raised);",
                            Badge { tone: Tone::Neutral, "{output_kind(&out)}" }
                            span { style: "flex:1;color:var(--dr-text);", "{out.name}" }
                            if fb_n > 0 {
                                Badge { tone: Tone::Warn, "{fb_n}" }
                            }
                            if !out.station.is_empty() {
                                span { style: "color:var(--dr-text-faint);", "{out.station}" }
                            }
                            {row_actions(move |_| {
                                target.set(Some(AnnotateTarget {
                                    label: label.clone(),
                                    path: path.clone(),
                                    work_id: label.clone(),
                                    visual,
                                    screenshot_url: url.clone(),
                                }));
                            })}
                        }
                    }
                }
            }
        }
    }
}

/// A small `view` + `review` action pair for a unit/output row. `on_review`
/// fires the annotate affordance; `view` is a passive inline hint for now.
fn row_actions(on_review: impl FnMut(MouseEvent) + 'static) -> Element {
    let chip = format!(
        "font-size:11px;color:{muted};border:1px solid {border};\
         border-radius:5px;padding:3px 9px;cursor:pointer;background:transparent;",
        muted = tokens::TEXT_MUTED,
        border = tokens::BORDER_STRONG,
    );
    rsx! {
        button {
            class: "dr-row-review",
            style: "{chip}",
            onclick: on_review,
            "review"
        }
    }
}

/// The Knowledge tab: the run's surfaced knowledge files.
fn knowledge_tab(knowledge: &[darkrun_api::session::KnowledgeFile]) -> Element {
    if knowledge.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No knowledge files surfaced." }
        };
    }
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:12px;",
            for kf in knowledge.iter() {
                div {
                    div {
                        style: "font-family:var(--dr-font-mono);font-size:12px;\
                                color:var(--dr-text);margin-bottom:4px;",
                        "{kf.name}"
                    }
                    pre {
                        style: "margin:0;white-space:pre-wrap;font-family:var(--dr-font-mono);\
                                font-size:11.5px;color:var(--dr-text-muted);\
                                background:var(--dr-surface-base);border:1px solid var(--dr-border);\
                                border-radius:6px;padding:10px;max-height:240px;overflow:auto;",
                        "{kf.content}"
                    }
                }
            }
        }
    }
}

/// The Feedback tab: the consolidated, severity-grouped inbox of every station
/// annotation. A persistent header button mirrors this; both render the same data.
fn feedback_tab(feedback: &[FeedbackEntry], inbox_open: Signal<bool>) -> Element {
    let mut inbox = inbox_open;
    if feedback.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No feedback on this station yet." }
        };
    }
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:8px;",
            {feedback_inbox(feedback.to_vec(), None::<EventHandler<(String, FeedbackAction)>>)}
            div { style: "margin-top:4px;",
                Button {
                    variant: ButtonVariant::Ghost,
                    on_click: move |_| inbox.set(true),
                    "open inbox panel"
                }
            }
        }
    }
}

/// The Overview tab: the run-scope reflection + a per-station status digest.
fn overview_tab(review: &ReviewSessionPayload) -> Element {
    let reflection = review.reflection.clone();
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:12px;",
            if let Some(r) = reflection {
                if !r.is_empty() {
                    div {
                        div { style: section_title(), "Reflection" }
                        p {
                            style: "margin:6px 0 0;font-size:12.5px;color:var(--dr-text-muted);\
                                    white-space:pre-wrap;",
                            "{r}"
                        }
                    }
                }
            }
            div {
                div { style: section_title(), "Stations" }
                div { style: "display:flex;flex-direction:column;gap:6px;margin-top:8px;",
                    for info in review.station_states.values() {
                        div {
                            style: "display:flex;align-items:center;gap:8px;\
                                    font-family:var(--dr-font-mono);font-size:12px;",
                            Badge {
                                tone: if info.merged_into_main { Tone::Ok } else { Tone::Neutral },
                                if info.merged_into_main { "merged" } else { "open" }
                            }
                            span { style: "flex:1;color:var(--dr-text);", "{info.station}" }
                            if let Some(ph) = info.phase.clone() {
                                span { style: "color:var(--dr-text-faint);", "{ph}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The feedback inbox panel, surfaced under the header when the operator opens
/// it. Resolve / dismiss chips PUT the feedback status back over the wire; a
/// successful write re-fetches the list so the count updates.
fn feedback_inbox_panel(
    cfg: ConnConfig,
    run: Option<String>,
    station: Option<String>,
    feedback: Signal<Vec<FeedbackItem>>,
    entries: Vec<FeedbackEntry>,
) -> Element {
    let on_action = {
        let cfg = cfg.clone();
        let run = run.clone();
        let station = station.clone();
        move |(id, action): (String, FeedbackAction)| {
            // Only resolve/dismiss mutate; jump is a no-op surface action.
            let new_status = match action {
                FeedbackAction::Resolve => Some(FeedbackStatus::Addressed),
                FeedbackAction::Dismiss => Some(FeedbackStatus::NonActionable),
                FeedbackAction::Jump => None,
            };
            let (Some(status), Some(run), Some(station)) =
                (new_status, run.clone(), station.clone())
            else {
                return;
            };
            let cfg = cfg.clone();
            let mut feedback = feedback;
            spawn(async move {
                let req = darkrun_api::FeedbackUpdateRequest {
                    status: Some(status),
                    ..Default::default()
                };
                if wire::update_feedback(&cfg, &run, &station, &id, &req).await.is_ok() {
                    if let Ok(resp) = wire::fetch_feedback(&cfg, &run, &station).await {
                        feedback.set(resp.items);
                    }
                }
            });
        }
    };
    rsx! {
        Card {
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:10px;",
                h2 { style: section_title(), "Feedback inbox" }
                Badge { tone: Tone::Neutral, "{entries.len()}" }
            }
            if entries.is_empty() {
                p { style: "color:var(--dr-text-muted);", "No feedback on this station yet." }
            } else {
                {feedback_inbox(entries, Some(EventHandler::new(on_action)))}
            }
        }
    }
}

/// The annotate surface: the toolbar + overlay + comment panel over the artifact
/// under review. Submits via the wire — image/html artifacts through the
/// visual-review annotate path (pin geometry), text artifacts through the
/// annotation→feedback create path. Mirrors `annotation-variants` (text + image).
fn annotate_panel(
    cfg: ConnConfig,
    run: Option<String>,
    station: Option<String>,
    target: AnnotateTarget,
    mut annotate_target: Signal<Option<AnnotateTarget>>,
) -> Element {
    rsx! {
        AnnotateSurface {
            cfg,
            run,
            station,
            label: target.label.clone(),
            path: target.path.clone(),
            work_id: target.work_id.clone(),
            visual: target.visual,
            screenshot_url: target.screenshot_url.clone(),
            on_close: move |_| annotate_target.set(None),
        }
    }
}

/// The live annotate surface — owns the active tool, the placed pins, and the
/// comment draft, and POSTs the annotation on submit.
#[component]
fn AnnotateSurface(
    cfg: ConnConfig,
    run: Option<String>,
    station: Option<String>,
    label: String,
    path: String,
    work_id: String,
    visual: bool,
    screenshot_url: Option<String>,
    on_close: EventHandler<MouseEvent>,
) -> Element {
    let kind = if visual { SurfaceKind::Visual } else { SurfaceKind::Text };
    let default_tool = if visual { AnnotateTool::Pin } else { AnnotateTool::Select };
    let mut tool = use_signal(|| default_tool);
    let mut pins = use_signal(Vec::<PinPoint>::new);
    let mut comments = use_signal(Vec::<String>::new);
    let submit = use_signal(|| Submit::Idle);

    let place = move |(x, y, w, h): (f64, f64, f64, f64)| {
        let note = format!("pin {}", pins.read().len() + 1);
        let pt = if w > 0.0 && h > 0.0 {
            place_pin(x, y, w, h, note)
        } else {
            PinPoint::new(x, y, note)
        };
        pins.write().push(pt);
    };

    let do_submit = {
        let cfg = cfg.clone();
        let run = run.clone();
        let station = station.clone();
        let label = label.clone();
        let path = path.clone();
        move |typed: String| {
            let cfg = cfg.clone();
            let run = run.clone();
            let station = station.clone();
            let label = label.clone();
            let path = path.clone();
            let mut submit = submit;
            // Capture the comment typed in the panel before reading the thread so
            // the user's text ships with the annotation, not just the pins/counts.
            let typed = typed.trim();
            if !typed.is_empty() {
                comments.write().push(typed.to_string());
            }
            let pin_list = pins.read().clone();
            let comment_list = comments.read().clone();
            spawn(async move {
                submit.set(Submit::Sending);
                let result = if visual {
                    // Visual artifact → record the pin geometry over the
                    // screenshot via the visual-review annotate path.
                    let pins = pin_list
                        .iter()
                        .map(|p| VisualReviewPin { x: p.x, y: p.y, note: p.note.clone() })
                        .collect();
                    let req = OutputReviewRequest {
                        annotations: VisualReviewAnnotations { pins, comments: comment_list.clone() },
                        title: Some(label.clone()),
                    };
                    wire::submit_output_review(&cfg, &req).await
                } else {
                    // Text artifact → submit the annotation as a feedback item.
                    let (Some(run), Some(station)) = (run.clone(), station.clone()) else {
                        submit.set(Submit::Failed("no run/station to attach to".into()));
                        return;
                    };
                    let body = if comment_list.is_empty() {
                        "(no comment)".to_string()
                    } else {
                        comment_list.join("\n")
                    };
                    let req = FeedbackCreateRequest {
                        title: format!("review: {label}"),
                        body,
                        origin: Some(FeedbackOrigin::UserVisual),
                        author: None,
                        source_ref: Some(path.clone()),
                        anchor: None,
                        inline_anchor: None,
                        resolution: None,
                        attachment_data_url: None,
                    };
                    wire::submit_annotation(&cfg, &run, &station, &req).await
                };
                match result {
                    Ok(()) => submit.set(Submit::Sent(format!(
                        "annotation recorded ({} pins · {} comments)",
                        pin_list.len(),
                        comment_list.len(),
                    ))),
                    Err(e) => submit.set(Submit::Failed(e.to_string())),
                }
            });
        }
    };

    let thread: Vec<ThreadComment> = comments
        .read()
        .iter()
        .enumerate()
        .map(|(i, c)| ThreadComment::new(i + 1, c.clone()))
        .collect();
    let active_tool = *tool.read();
    let pin_points = pins.read().clone();
    let stage_dims = use_signal(|| (0.0_f64, 0.0_f64));

    rsx! {
        Card {
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:10px;",
                Badge { tone: Tone::Info, if visual { "annotate · visual" } else { "annotate · text" } }
                span {
                    style: "flex:1;font-family:var(--dr-font-mono);font-size:12px;color:var(--dr-text-muted);",
                    "{label}"
                }
                Button { variant: ButtonVariant::Ghost, on_click: move |evt| on_close.call(evt), "close" }
            }
            div { style: "display:flex;gap:8px;margin-bottom:10px;",
                AnnotateToolbar {
                    kind,
                    active: active_tool,
                    on_pick: move |t| tool.set(t),
                }
            }
            div { style: "display:flex;gap:16px;align-items:flex-start;",
                // The artifact stage — a click drops a pin when a spatial tool is active.
                {annotate_stage(visual, screenshot_url.clone(), pin_points, stage_dims, place)}
                div { style: "flex:1;min-width:0;",
                    CommentPanel {
                        comments: thread,
                        placeholder: "comment on this artifact…".to_string(),
                        on_submit: do_submit,
                    }
                    SubmitStatus { state: submit.read().clone() }
                    div {
                        style: "margin-top:6px;font-family:var(--dr-font-mono);\
                                font-size:11px;color:var(--dr-text-faint);",
                        "annotating: {path}"
                    }
                }
            }
        }
    }
}

/// The artifact stage the annotate surface paints over — the screenshot (visual)
/// or a text placeholder. Forwards a click's pixel offset + the stage box so the
/// caller can normalize the pin.
fn annotate_stage(
    visual: bool,
    screenshot_url: Option<String>,
    pins: Vec<PinPoint>,
    _stage_dims: Signal<(f64, f64)>,
    mut on_place: impl FnMut((f64, f64, f64, f64)) + 'static,
) -> Element {
    let stage = format!(
        "position:relative;flex:0 0 360px;min-height:220px;border-radius:8px;\
         border:1px solid {border};background:{base};overflow:hidden;\
         {cursor}",
        border = tokens::BORDER,
        base = tokens::SURFACE_BASE,
        cursor = if visual { "cursor:crosshair;" } else { "" },
    );
    rsx! {
        div {
            class: "dr-annotate-stage",
            style: "{stage}",
            onclick: move |evt| {
                if !visual {
                    return;
                }
                let coords = evt.element_coordinates();
                // Width/height are read from the event's element rect when
                // available; fall back to the fixed flex-basis so the pin still
                // lands somewhere sensible.
                on_place((coords.x, coords.y, 360.0, 220.0));
            },
            if visual {
                if let Some(url) = screenshot_url {
                    img {
                        src: "{url}",
                        style: "width:100%;display:block;pointer-events:none;",
                    }
                } else {
                    div {
                        style: "display:flex;align-items:center;justify-content:center;\
                                height:220px;color:var(--dr-text-faint);font-size:12px;",
                        "drop pins to point at the surface"
                    }
                }
                for (i, pt) in pins.iter().enumerate() {
                    PinMarker { point: pt.clone(), number: i + 1 }
                }
            } else {
                div {
                    style: "padding:14px;color:var(--dr-text-muted);font-size:12px;\
                            font-family:var(--dr-font-mono);",
                    "Text artifact — select a span and leave a comment. The annotation \
                     anchors to this artifact and ships to the agent as feedback."
                }
            }
        }
    }
}

/// Whether an output artifact opens the visual (spatial) annotate surface.
fn output_is_visual(out: &OutputArtifact) -> bool {
    use darkrun_api::session::OutputArtifactType::*;
    matches!(out.artifact_type, Html | Image | Video)
}

/// The single, severity-driven checkpoint control set, rendered only at an
/// active review/final gate.
///
/// `open_blockers` is the count of open `must`/`should` annotations on the
/// station. When any are open the primary darkens to Request-changes (you can't
/// cleanly approve over a blocker); a clean / nits-only station keeps Approve
/// primary. This is the ONE decision control — the old duplicate (the bar's
/// advance/hold AND a separate approve row) is gone.
fn checkpoint_section(
    cfg: ConnConfig,
    review: ReviewSessionPayload,
    decision: Signal<Decision>,
    open_blockers: usize,
) -> Element {
    let kind = review
        .gate_type
        .map(map::checkpoint_kind)
        .unwrap_or(CheckpointKind::Ask);
    let approve_label = review
        .approve_action
        .as_ref()
        .map(|a| a.label.clone())
        .unwrap_or_else(|| "Approve".to_string());
    let prompt = review
        .gate_context
        .clone()
        .or_else(|| review.target.clone())
        .unwrap_or_else(|| "Checkpoint reached — approve or request changes.".to_string());

    // A global station note shipped with Request-changes.
    let note = use_signal(String::new);

    let post = {
        let cfg = cfg.clone();
        move |raw: &'static str, feedback: Option<String>| {
            let cfg = cfg.clone();
            let mut decision = decision;
            spawn(async move {
                decision.set(Decision::Sending);
                let req = ReviewDecisionRequest {
                    decision: raw.to_string(),
                    feedback,
                    annotations: None,
                };
                match wire::submit_decision(&cfg, &req).await {
                    Ok(()) => decision.set(Decision::Sent(raw.to_string())),
                    Err(e) => decision.set(Decision::Failed(e.to_string())),
                }
            });
        }
    };

    let sending = matches!(*decision.read(), Decision::Sending);
    // Severity-driven primary: open blockers darken Approve + promote changes.
    let blocked = open_blockers > 0;
    let changes_note = note.read().clone();
    let changes_payload = if changes_note.trim().is_empty() {
        None
    } else {
        Some(changes_note)
    };

    let approve_click = post.clone();
    let changes_click = post.clone();
    let bar_advance = post.clone();
    let bar_hold = post;
    let changes_payload_bar = changes_payload.clone();

    let mut note_sig = note;

    rsx! {
        div { style: "display:flex;flex-direction:column;gap:10px;",
            CheckpointBar {
                kind,
                prompt,
                on_advance: move |_| if !blocked { bar_advance("approved", None) },
                on_hold: move |_| bar_hold("changes_requested", changes_payload_bar.clone()),
            }
            // One global station note ships with Request-changes.
            textarea {
                style: format!(
                    "width:100%;box-sizing:border-box;min-height:54px;padding:9px 12px;\
                     border-radius:6px;border:1px solid {border};background:{base};\
                     color:{text};font-family:{sans};font-size:13px;resize:vertical;",
                    border = tokens::BORDER,
                    base = tokens::SURFACE_BASE,
                    text = tokens::TEXT,
                    sans = tokens::FONT_SANS,
                ),
                placeholder: "Station note (ships with Request changes)…",
                oninput: move |evt| note_sig.set(evt.value()),
            }
            div { style: "display:flex;align-items:center;gap:10px;",
                Button {
                    variant: if blocked { ButtonVariant::Secondary } else { ButtonVariant::Primary },
                    tone: Tone::Ok,
                    disabled: sending || blocked,
                    on_click: move |_| approve_click("approved", None),
                    "{approve_label}"
                }
                Button {
                    variant: if blocked { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                    tone: Tone::Danger,
                    disabled: sending,
                    on_click: move |_| changes_click("changes_requested", changes_payload.clone()),
                    "Request changes"
                }
                if blocked {
                    Badge { tone: Tone::Danger, "{open_blockers} open blocking" }
                }
                DecisionStatus { decision: decision.read().clone(), gate_open: true }
            }
        }
    }
}

/// A small status line reflecting the last decision POST.
#[component]
fn DecisionStatus(decision: Decision, gate_open: bool) -> Element {
    let (tone, text) = match &decision {
        Decision::Idle if !gate_open => {
            (Tone::Neutral, "gate is not currently blocking".to_string())
        }
        Decision::Idle => return rsx! {},
        Decision::Sending => (Tone::Info, "submitting…".to_string()),
        Decision::Sent(d) => (Tone::Ok, format!("recorded: {d}")),
        Decision::Failed(e) => (Tone::Danger, format!("failed: {e}")),
    };
    rsx! {
        Badge { tone, "{text}" }
    }
}

// ===========================================================================
// Interactive sessions: question / direction / picker.
//
// Each wire payload is decoded off the same WS feed as a review. The wire types
// do not derive `PartialEq` (a Dioxus prop requirement), so a thin plain
// function extracts the `PartialEq` view-model data + scalars and hands them to a
// real `#[component]` that owns the local selection/annotation signals and POSTs
// the result back over the existing decision path.
// ===========================================================================

/// The submit-state machine shared by every interactive session, mirroring the
/// review [`Decision`] but generic over what was submitted.
#[derive(Debug, Clone, PartialEq)]
enum Submit {
    /// Nothing submitted yet.
    Idle,
    /// A POST is in flight.
    Sending,
    /// The engine accepted the submission (carries a short summary).
    Sent(String),
    /// The POST failed (carries the reason).
    Failed(String),
}

/// A small status line reflecting the last interactive-session submission.
#[component]
fn SubmitStatus(state: Submit) -> Element {
    let (tone, text) = match &state {
        Submit::Idle => return rsx! {},
        Submit::Sending => (Tone::Info, "submitting…".to_string()),
        Submit::Sent(s) => (Tone::Ok, s.clone()),
        Submit::Failed(e) => (Tone::Danger, format!("failed: {e}")),
    };
    rsx! {
        div { style: "margin-top:10px;",
            Badge { tone, "{text}" }
        }
    }
}

/// Extract the question payload's `PartialEq` data and render the session.
fn question_session(cfg: ConnConfig, q: QuestionSessionPayload) -> Element {
    let answered = matches!(
        q.status,
        darkrun_api::common::SessionStatus::Answered
            | darkrun_api::common::SessionStatus::Approved
    );
    let seed = q.answer.as_ref().map(|a| a.selected.clone()).unwrap_or_default();
    rsx! {
        QuestionSession {
            cfg,
            prompt: q.prompt.clone(),
            context: q.context.clone(),
            title: q.title.clone(),
            options: map::option_cards(&q.options),
            multi_select: q.multi_select,
            image_urls: q.image_urls.clone(),
            seed_selected: seed,
            answered,
        }
    }
}

/// The live visual-question session: owns the selection model and submits the
/// chosen option ids to `/question/:id/answer`.
#[component]
fn QuestionSession(
    cfg: ConnConfig,
    prompt: String,
    context: Option<String>,
    title: Option<String>,
    options: Vec<OptionCard>,
    multi_select: bool,
    image_urls: Vec<String>,
    seed_selected: Vec<String>,
    answered: bool,
) -> Element {
    let mode = SelectMode::from_multi(multi_select);
    let mut selected = use_signal(|| {
        SelectionModel::from_selected(mode, seed_selected.clone())
            .selected()
            .to_vec()
    });
    let submit = use_signal(|| Submit::Idle);

    let toggle = move |id: String| {
        let mut model = SelectionModel::from_selected(mode, selected.read().clone());
        model.toggle(&id);
        selected.set(model.selected().to_vec());
    };

    let do_submit = {
        let cfg = cfg.clone();
        move |_| {
            let cfg = cfg.clone();
            let mut submit = submit;
            let chosen = selected.read().clone();
            spawn(async move {
                submit.set(Submit::Sending);
                let req = QuestionAnswerRequest {
                    selected: chosen.clone(),
                    text: None,
                    annotations: None,
                };
                match wire::submit_question_answer(&cfg, &req).await {
                    Ok(()) => submit.set(Submit::Sent(format!(
                        "answer recorded ({} selected)",
                        chosen.len()
                    ))),
                    Err(e) => submit.set(Submit::Failed(e.to_string())),
                }
            });
        }
    };

    let sending = matches!(*submit.read(), Submit::Sending);
    rsx! {
        QuestionView {
            prompt,
            context,
            title,
            options,
            multi_select,
            image_urls,
            selected: selected.read().clone(),
            answered: answered || sending,
            on_toggle: toggle,
            on_submit: do_submit,
        }
        SubmitStatus { state: submit.read().clone() }
    }
}

/// Extract the direction payload's `PartialEq` data and render the session.
fn direction_session(cfg: ConnConfig, d: DirectionSessionPayload) -> Element {
    let decided = matches!(
        d.status,
        darkrun_api::common::SessionStatus::Decided
            | darkrun_api::common::SessionStatus::Approved
    );
    let seed_pins = d
        .annotations
        .as_ref()
        .map(|a| map::pin_points(&a.pins))
        .unwrap_or_default();
    let seed_comments = d
        .annotations
        .as_ref()
        .map(|a| a.comments.clone())
        .unwrap_or_default();
    rsx! {
        DirectionSession {
            cfg,
            prompt: d.prompt.clone(),
            context: d.context.clone(),
            title: d.title.clone(),
            archetypes: map::archetype_cards(&d.archetypes),
            seed_chosen: d.chosen_archetype.clone(),
            seed_pins,
            seed_comments,
            decided,
        }
    }
}

/// The live design-direction session: owns the chosen archetype, the pin set,
/// and the comment list; submits the decision to `/direction/:id/select`.
#[component]
fn DirectionSession(
    cfg: ConnConfig,
    prompt: String,
    context: Option<String>,
    title: Option<String>,
    archetypes: Vec<ArchetypeCard>,
    seed_chosen: Option<String>,
    seed_pins: Vec<PinPoint>,
    seed_comments: Vec<String>,
    decided: bool,
) -> Element {
    let mut chosen = use_signal(|| seed_chosen.clone());
    let mut pins = use_signal(|| seed_pins.clone());
    let mut comments = use_signal(|| seed_comments.clone());
    let submit = use_signal(|| Submit::Idle);

    let choose = move |id: String| {
        // Switching archetypes resets annotations — pins are relative to the
        // chosen preview, so they would be meaningless on a different image.
        let same = chosen.read().as_deref() == Some(id.as_str());
        chosen.set(Some(id));
        if !same {
            pins.set(Vec::new());
        }
    };

    let place = move |(x, y, w, h): (f64, f64, f64, f64)| {
        // The stage forwards the click offset; when it cannot resolve its own
        // box it passes (0,0) dims, in which case the offset is already the
        // normalized value. Either way `place_pin` clamps into 0..1.
        let pt = if w > 0.0 && h > 0.0 {
            place_pin(x, y, w, h, format!("pin {}", pins.read().len() + 1))
        } else {
            PinPoint::new(x, y, format!("pin {}", pins.read().len() + 1))
        };
        pins.write().push(pt);
    };

    let comment = move |text: String| {
        comments.write().push(text);
    };

    let do_submit = {
        let cfg = cfg.clone();
        move |_| {
            let cfg = cfg.clone();
            let mut submit = submit;
            let archetype = chosen.read().clone();
            let pin_list: Vec<_> = pins.read().iter().map(map::pin_to_wire).collect();
            let comment_list = comments.read().clone();
            let Some(archetype) = archetype else {
                submit.set(Submit::Failed("choose an archetype first".to_string()));
                return;
            };
            spawn(async move {
                submit.set(Submit::Sending);
                let annotations = if pin_list.is_empty() && comment_list.is_empty() {
                    None
                } else {
                    Some(DirectionAnnotations {
                        pins: pin_list,
                        screenshot: None,
                        comments: comment_list,
                    })
                };
                let req = DirectionSelectRequest { archetype: archetype.clone(), annotations };
                match wire::submit_direction_select(&cfg, &req).await {
                    Ok(()) => submit.set(Submit::Sent(format!("direction recorded: {archetype}"))),
                    Err(e) => submit.set(Submit::Failed(e.to_string())),
                }
            });
        }
    };

    let sending = matches!(*submit.read(), Submit::Sending);
    rsx! {
        DirectionView {
            prompt,
            context,
            title,
            archetypes,
            chosen: chosen.read().clone(),
            pins: pins.read().clone(),
            comments: comments.read().clone(),
            decided: decided || sending,
            on_choose: choose,
            on_place_pin: place,
            on_comment: comment,
            on_submit: do_submit,
        }
        SubmitStatus { state: submit.read().clone() }
    }
}

/// Extract the picker payload's `PartialEq` data and render the session.
fn picker_session(cfg: ConnConfig, p: PickerSessionPayload) -> Element {
    let decided = p.selection.is_some()
        || matches!(
            p.status,
            darkrun_api::common::SessionStatus::Decided
                | darkrun_api::common::SessionStatus::Approved
        );
    let seed = p.selection.as_ref().map(|s| s.id.clone());
    rsx! {
        PickerSession {
            cfg,
            title: Some(p.title.clone()),
            prompt: p.prompt.clone(),
            options: map::picker_items(&p.options),
            seed_selected: seed,
            decided,
        }
    }
}

/// The live picker session: owns the single selection and submits it to
/// `/picker/:id/select`.
#[component]
fn PickerSession(
    cfg: ConnConfig,
    title: Option<String>,
    prompt: String,
    options: Vec<PickerItem>,
    seed_selected: Option<String>,
    decided: bool,
) -> Element {
    let mut selected = use_signal(|| seed_selected.clone());
    let submit = use_signal(|| Submit::Idle);

    let select = {
        let cfg = cfg.clone();
        move |id: String| {
            let cfg = cfg.clone();
            let mut submit = submit;
            selected.set(Some(id.clone()));
            spawn(async move {
                submit.set(Submit::Sending);
                let req = PickerSelectRequest { id: id.clone() };
                match wire::submit_picker_select(&cfg, &req).await {
                    Ok(()) => submit.set(Submit::Sent(format!("selected: {id}"))),
                    Err(e) => submit.set(Submit::Failed(e.to_string())),
                }
            });
        }
    };

    let sending = matches!(*submit.read(), Submit::Sending);
    rsx! {
        PickerView {
            title,
            prompt,
            options,
            selected: selected.read().clone(),
            decided: decided || sending,
            on_select: select,
        }
        SubmitStatus { state: submit.read().clone() }
    }
}

// ===========================================================================
// View / visual-review / proof sessions.
//
// The view session is a non-blocking ARTIFACT BROWSER; focusing a screenshot
// artifact reveals the inline OutputReview annotator, which POSTs its pins +
// comments to the output-annotation route. The standalone visual-review session
// renders the same annotator over a single screenshot. The proof session renders
// the surface-routed NUMBERS in the ProofPanel.
// ===========================================================================

/// Extract the view payload's `PartialEq` data and render the artifact browser.
fn view_session(cfg: ConnConfig, v: ViewSessionPayload) -> Element {
    let run_slug = if v.run_slug.is_empty() {
        None
    } else {
        Some(v.run_slug.clone())
    };
    rsx! {
        ViewSession {
            cfg,
            run_slug,
            station: v.station.clone(),
            artifacts: map::artifact_entries(&v.artifacts),
            seed_focus: v.artifact.clone(),
        }
    }
}

/// The live artifact browser: owns the focused artifact + the inline output
/// review it spawns when a screenshot is reviewed.
#[component]
fn ViewSession(
    cfg: ConnConfig,
    run_slug: Option<String>,
    station: Option<String>,
    artifacts: Vec<ArtifactEntry>,
    seed_focus: Option<String>,
) -> Element {
    let mut focused = use_signal(|| seed_focus.clone());
    // The id of the artifact currently being visually reviewed, if any.
    let mut reviewing = use_signal(|| None::<String>);

    let focus = move |id: String| {
        focused.set(Some(id));
    };
    let review = move |id: String| {
        reviewing.set(Some(id));
    };

    // The screenshot artifact under review, resolved from the browser list.
    let review_entry = reviewing
        .read()
        .clone()
        .and_then(|id| artifacts.iter().find(|a| a.id == id).cloned());

    rsx! {
        ViewArtifacts {
            run_slug: run_slug.clone(),
            station: station.clone(),
            artifacts: artifacts.clone(),
            focused: focused.read().clone(),
            on_focus: focus,
            on_review: review,
        }
        if let Some(entry) = review_entry {
            OutputReviewSession {
                cfg,
                run_slug,
                station,
                artifact_label: Some(entry.label.clone()),
                artifact_path: Some(entry.path.clone()),
                screenshot_url: entry.url.clone().or(entry.thumbnail_url.clone()),
                prompt: None,
            }
        }
    }
}

/// Extract the visual-review payload's `PartialEq` data and render the annotator.
fn visual_review_session(cfg: ConnConfig, vr: VisualReviewSessionPayload) -> Element {
    rsx! {
        OutputReviewSession {
            cfg,
            run_slug: vr.run_slug.clone(),
            station: vr.station.clone(),
            artifact_label: vr.artifact_id.clone(),
            artifact_path: vr.artifact_path.clone(),
            screenshot_url: vr.screenshot_url.clone(),
            prompt: vr.prompt.clone(),
        }
    }
}

/// The live output-review session: owns the pin set + comment list over an output
/// screenshot and POSTs them to `/visual-review/:id/annotate`.
#[component]
fn OutputReviewSession(
    cfg: ConnConfig,
    run_slug: Option<String>,
    station: Option<String>,
    artifact_label: Option<String>,
    artifact_path: Option<String>,
    screenshot_url: Option<String>,
    prompt: Option<String>,
) -> Element {
    let mut pins = use_signal(Vec::<PinPoint>::new);
    let mut comments = use_signal(Vec::<String>::new);
    let submit = use_signal(|| Submit::Idle);

    let place = move |(x, y, w, h): (f64, f64, f64, f64)| {
        let note = format!("pin {}", pins.read().len() + 1);
        let pt = if w > 0.0 && h > 0.0 {
            place_pin(x, y, w, h, note)
        } else {
            PinPoint::new(x, y, note)
        };
        pins.write().push(pt);
    };
    let comment = move |text: String| {
        comments.write().push(text);
    };

    let do_submit = {
        let cfg = cfg.clone();
        let label = artifact_label.clone();
        move |_| {
            let cfg = cfg.clone();
            let label = label.clone();
            let mut submit = submit;
            let pin_list: Vec<VisualReviewPin> = pins
                .read()
                .iter()
                .map(|p| VisualReviewPin { x: p.x, y: p.y, note: p.note.clone() })
                .collect();
            let comment_list = comments.read().clone();
            spawn(async move {
                submit.set(Submit::Sending);
                let req = OutputReviewRequest {
                    annotations: VisualReviewAnnotations {
                        pins: pin_list.clone(),
                        comments: comment_list.clone(),
                    },
                    title: label,
                };
                match wire::submit_output_review(&cfg, &req).await {
                    Ok(()) => submit.set(Submit::Sent(format!(
                        "feedback recorded ({} pins · {} comments)",
                        pin_list.len(),
                        comment_list.len()
                    ))),
                    Err(e) => submit.set(Submit::Failed(e.to_string())),
                }
            });
        }
    };

    let sending = matches!(*submit.read(), Submit::Sending);
    let submitted = matches!(*submit.read(), Submit::Sent(_));
    rsx! {
        OutputReview {
            run_slug,
            station,
            artifact_label,
            screenshot_url,
            prompt,
            pins: pins.read().clone(),
            comments: comments.read().clone(),
            submitted: submitted || sending,
            on_place_pin: place,
            on_comment: comment,
            on_submit: do_submit,
        }
        SubmitStatus { state: submit.read().clone() }
        if let Some(path) = artifact_path {
            div {
                style: "margin-top:6px;font-family:var(--dr-font-mono);\
                        font-size:11px;color:var(--dr-text-faint);",
                "annotating: {path}"
            }
        }
    }
}

/// Render the proof session's surface-routed objective NUMBERS in the panel.
fn proof_session(pr: ProofSessionPayload) -> Element {
    rsx! {
        ProofPanel { proof: map::proof_view(&pr.proof) }
    }
}

/// Shared section-heading style.
fn section_title() -> String {
    "margin:0;font-family:var(--dr-font-sans);font-size:13px;font-weight:700;\
     color:var(--dr-text);text-transform:uppercase;letter-spacing:0.04em;"
        .to_string()
}

/// Shared completion-criteria list style.
fn criteria_list() -> String {
    "margin:0 0 0 28px;padding:0;font-family:var(--dr-font-sans);\
     font-size:12px;color:var(--dr-text-muted);"
        .to_string()
}

/// A short label for an output artifact's render kind.
fn output_kind(out: &OutputArtifact) -> &'static str {
    use darkrun_api::session::OutputArtifactType::*;
    match out.artifact_type {
        Markdown => "md",
        Html => "html",
        Image => "img",
        Video => "video",
        Code => "code",
        File => "file",
    }
}

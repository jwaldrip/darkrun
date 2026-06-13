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

/// The feedback button's color rules — theme-keyed, literal colors, no custom
/// properties. Dark keeps the soft pink-on-red-tint pair the design settled
/// on; light pairs a deep red with a rose tint (WCAG AA in both: 7.5:1 / 5.7:1).
const FB_BTN_CSS: &str = r#"
.dr-feedback-open{ background:var(--dr-surface-overlay); color:var(--dr-text-muted); }
.dr-feedback-open>span{ background:var(--dr-surface-base); }
.dr-feedback-open[data-alert="true"]{ background:#f8514922; color:#f5a3a3; }
.dr-feedback-open[data-alert="true"]>span{ background:#f8514933; }
:root[data-theme="light"] .dr-feedback-open[data-alert="true"]{ background:#f9dedc; color:#a8201a; }
:root[data-theme="light"] .dr-feedback-open[data-alert="true"]>span{ background:#f3c8c5; }
@media (prefers-color-scheme: light){
  :root:not([data-theme="dark"]) .dr-feedback-open[data-alert="true"]{ background:#f9dedc; color:#a8201a; }
  :root:not([data-theme="dark"]) .dr-feedback-open[data-alert="true"]>span{ background:#f3c8c5; }
}
"#;

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

    // Fill the main pane (the shell already provides the chrome + gutters); a
    // generous max-width keeps long lines readable without leaving a centered
    // moat of padding on a wide window.
    // Full-bleed: the review fills the window's width (the shell's gutters are
    // the only margin) — no centered moat on wide displays.
    let shell = "padding:14px 18px;display:flex;flex-direction:column;gap:12px;\
                 width:100%;box-sizing:border-box;";

    rsx! {
        div { style: "{shell}",
            // The wordmark + app chrome live in the shell toolbar now, so the
            // review pane shows only its content plus a slim connection indicator.
            div { style: "display:flex;justify-content:flex-end;align-items:center;min-height:0;",
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
    /// The artifact's TEXT content for a text surface (the real body the
    /// reviewer selects spans of). `None` falls back to a placeholder.
    text: Option<String>,
}

/// One mark painted over the text surface: a persisted anchor from earlier
/// feedback, or an in-flight selection — numbered in display order.
#[derive(Debug, Clone, PartialEq)]
struct TextMark {
    /// The anchored span.
    selected_text: String,
    /// Zero-based paragraph index.
    paragraph: u32,
    /// The tool that made it (`None` for a persisted anchor → highlight style).
    tool: Option<AnnotateTool>,
    /// Content drifted since save (persisted anchors only).
    stale: bool,
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
            {feedback_inbox_panel(cfg.clone(), run_slug.clone(), station.clone(), feedback, feedback_entries.clone(), &outputs, active_tab, annotate_target, inbox_open)}
        }

        // ── The annotate surface, when an artifact is under review ──────────
        if let Some(target) = annotate_target.read().clone() {
            {
                // Earlier feedback anchored to THIS artifact re-renders as
                // visible marks, stale-flagged when the text has drifted since
                // the anchor was saved (content-hash mismatch).
                let current_sha = target
                    .text
                    .as_deref()
                    .map(|t| darkrun_core::hash_bytes(t.as_bytes()));
                let persisted: Vec<TextMark> = feedback
                    .read()
                    .iter()
                    .filter_map(|item| item.inline_anchor.as_ref())
                    .filter(|a| a.file_path.as_deref() == Some(target.path.as_str()))
                    .map(|a| TextMark {
                        selected_text: a.selected_text.clone(),
                        paragraph: a.paragraph,
                        tool: None,
                        stale: match (&a.content_sha, &current_sha) {
                            (Some(saved), Some(now)) => saved != now,
                            _ => false,
                        },
                    })
                    .collect();
                annotate_panel(cfg.clone(), run_slug.clone(), station.clone(), target, persisted, annotate_target)
            }
        }

        // ── The tabbed station body ─────────────────────────────────────────
        Card {
            TabBar {
                tabs,
                active: active.clone(),
                on_select: move |id| tab_sig.set(id),
            }
            div { style: "margin-top:14px;",
                {
                    // The SAME jump/resolve/dismiss handler the inbox uses —
                    // the Feedback tab's chips act identically.
                    let fb_action = EventHandler::new(feedback_action_handler(
                        cfg.clone(),
                        run_slug.clone(),
                        station.clone(),
                        feedback,
                        outputs.clone(),
                        active_tab,
                        annotate_target,
                        inbox_open,
                    ));
                    tab_body(&active, &units, &outputs, &knowledge, &unit_outputs, &feedback_entries, &review, annotate_target, inbox_open, fb_action)
                }
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
        text = tokens::var::TEXT,
    );
    let sub_style = format!(
        "display:flex;align-items:center;gap:10px;margin-top:10px;\
         font-family:{mono};font-size:12px;color:{muted};",
        mono = tokens::FONT_MONO,
        muted = tokens::var::TEXT_MUTED,
    );
    // The button's COLORS live in `FB_BTN_CSS` (keyed on `data-alert` + the
    // theme), not inline — dark keeps the soft pink-on-red-tint pair; light
    // gets a deep red on a rose tint. Both clear WCAG AA. Inline = layout only.
    let fb_btn = format!(
        "appearance:none;-webkit-appearance:none;border:1px solid {border};\
         font-family:{sans};font-size:12px;border-radius:6px;padding:5px 11px;\
         cursor:pointer;display:flex;align-items:center;gap:6px;",
        border = tokens::var::BORDER_STRONG,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        Card {
            div {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;",
                span { style: "{title_style}", "{title}" }
                div { style: "display:flex;align-items:center;gap:8px;",
                    style { "{FB_BTN_CSS}" }
                    button {
                        class: "dr-feedback-open",
                        "data-alert": if feedback_alert { "true" } else { "false" },
                        style: "{fb_btn}",
                        onclick: move |evt| on_open_feedback.call(evt),
                        "Feedback"
                        span {
                            style: format!(
                                "font-family:{};border-radius:999px;padding:0 6px;",
                                tokens::FONT_MONO,
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
            // The phase subheader, scoped to the current station: the label on
            // its own row, centered like the pipeline beneath it.
            if let Some(st) = station.clone() {
                div { style: "{sub_style}justify-content:center;", span { "station: {st}" } }
            }
            div { style: "display:flex;justify-content:center;margin-top:6px;",
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
    fb_action: EventHandler<(String, FeedbackAction)>,
) -> Element {
    match active {
        "outputs" => output_tab(outputs, feedback, annotate_target),
        "knowledge" => knowledge_tab(knowledge),
        "feedback" => feedback_tab(feedback, inbox_open, fb_action),
        "overview" => overview_tab(review),
        // Default to the units tab.
        _ => unit_tab(units, &review.units, unit_outputs, feedback, annotate_target),
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
    raw_units: &[serde_json::Value],
    unit_outputs: &std::collections::BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>>,
    feedback: &[FeedbackEntry],
    annotate_target: Signal<Option<AnnotateTarget>>,
) -> Element {
    if units.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No units in this review." }
        };
    }
    // The dependency DAG leads the tab (per the approved mockup): the existing
    // darkrun-ui UnitGraph over the station's units, status-toned, full-width.
    // A single-unit station skips the graph (no topology to show).
    let (graph_nodes, graph_edges) = map::unit_graph(raw_units);
    let show_graph = graph_nodes.len() >= 2;
    let legend = [
        ("completed", "var(--dr-status-ok)"),
        ("in progress", "var(--dr-status-info)"),
        ("pending", "var(--dr-text-faint)"),
        ("blocked", "var(--dr-status-danger)"),
    ];
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:10px;",
            if show_graph {
                div { style: "display:flex;flex-direction:column;gap:8px;margin-bottom:4px;",
                    div { style: "display:flex;align-items:center;justify-content:space-between;",
                        span {
                            style: "font-family:var(--dr-font-mono,monospace);font-size:11px;\
                                    letter-spacing:0.08em;text-transform:uppercase;color:var(--dr-accent);",
                            "dependency graph"
                        }
                        span { style: "display:inline-flex;gap:14px;",
                            for (name, color) in legend.iter() {
                                span {
                                    style: "font-family:var(--dr-font-mono,monospace);font-size:11px;\
                                            color:var(--dr-text-muted);display:inline-flex;\
                                            align-items:center;gap:5px;",
                                    span {
                                        style: format!(
                                            "width:9px;height:9px;border-radius:2px;border:1.5px solid {color};\
                                             background:var(--dr-surface-overlay);"
                                        ),
                                    }
                                    "{name}"
                                }
                            }
                        }
                    }
                    UnitGraph { units: graph_nodes.clone(), edges: graph_edges.clone() }
                }
            }
            for unit in units.iter() {
                {
                    let unit = unit.clone();
                    let previews = unit_outputs.get(&unit.title).cloned().unwrap_or_default();
                    let mut target = annotate_target;
                    let label = unit.title.clone();
                    let work_id = unit.title.clone();
                    let fb_n = feedback_count_for(feedback, &unit.title);
                    // The unit's real markdown body — the text the reviewer
                    // selects spans of (matched by the same title resolution
                    // `unit_view` uses, so the row and its text agree).
                    let body_text = raw_units
                        .iter()
                        .find(|u| {
                            map::first_str(u, &["title", "name", "slug", "id"]).as_deref()
                                == Some(unit.title.as_str())
                        })
                        .and_then(|u| u.get("body").and_then(|b| b.as_str()))
                        .map(str::to_string);
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
                                        text: body_text.clone(),
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
                                    text: None,
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
        muted = tokens::var::TEXT_MUTED,
        border = tokens::var::BORDER_STRONG,
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
fn feedback_tab(
    feedback: &[FeedbackEntry],
    inbox_open: Signal<bool>,
    fb_action: EventHandler<(String, FeedbackAction)>,
) -> Element {
    let mut inbox = inbox_open;
    if feedback.is_empty() {
        return rsx! {
            p { style: "color:var(--dr-text-muted);", "No feedback on this station yet." }
        };
    }
    rsx! {
        div { style: "display:flex;flex-direction:column;gap:8px;",
            {feedback_inbox(feedback.to_vec(), Some(fb_action))}
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
                    for info in &review.station_states {
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

/// Build the jump / resolve / dismiss handler shared by the inbox panel AND
/// the Feedback tab — both surfaces act on the same records identically.
/// Feedback items are read FRESH from the signal per action, so a row acted on
/// after a refetch still resolves.
#[allow(clippy::too_many_arguments)]
fn feedback_action_handler(
    cfg: ConnConfig,
    run: Option<String>,
    station: Option<String>,
    feedback: Signal<Vec<FeedbackItem>>,
    outputs: Vec<OutputArtifact>,
    active_tab: Signal<String>,
    annotate_target: Signal<Option<AnnotateTarget>>,
    inbox_open: Signal<bool>,
) -> impl FnMut((String, FeedbackAction)) + 'static {
    let mut active_tab = active_tab;
    let mut annotate_target = annotate_target;
    let mut inbox_open = inbox_open;
    move |(id, action): (String, FeedbackAction)| {
        let items = feedback.read().clone();
        // Jump is a surface action: focus the anchored artifact instead of
        // mutating the record.
        if action == FeedbackAction::Jump {
            if let Some(target) = jump_target(&items, &id, &outputs) {
                // Switch to the owning tab, open the annotate surface on the
                // anchored artifact, and close the inbox so it's in view.
                active_tab.set(if target.visual {
                    "outputs".to_string()
                } else {
                    "units".to_string()
                });
                annotate_target.set(Some(target));
                inbox_open.set(false);
            }
            return;
        }
        // Resolve / dismiss mutate the record's status.
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
}

/// The feedback inbox panel, surfaced under the header when the operator opens
/// it. Resolve / dismiss chips PUT the feedback status back over the wire; a
/// successful write re-fetches the list so the count updates. The jump chip
/// focuses the artifact the annotation is anchored to — it switches to the
/// owning tab and opens that artifact's annotate surface at the anchor.
#[allow(clippy::too_many_arguments)]
fn feedback_inbox_panel(
    cfg: ConnConfig,
    run: Option<String>,
    station: Option<String>,
    feedback: Signal<Vec<FeedbackItem>>,
    entries: Vec<FeedbackEntry>,
    outputs: &[OutputArtifact],
    active_tab: Signal<String>,
    annotate_target: Signal<Option<AnnotateTarget>>,
    inbox_open: Signal<bool>,
) -> Element {
    // Snapshot the outputs so the Jump resolver can match a feedback locator to
    // a declared output (and reuse its visual/path/url) without borrowing.
    let outputs = outputs.to_vec();
    let on_action = feedback_action_handler(
        cfg.clone(),
        run.clone(),
        station.clone(),
        feedback,
        outputs,
        active_tab,
        annotate_target,
        inbox_open,
    );
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

/// Resolve a feedback id to the artifact it's anchored to, building the
/// [`AnnotateTarget`] the Jump chip opens. The item's `source_ref` (locator) is
/// matched against the declared outputs first — a match reuses the output's
/// visual class + path + screenshot URL so the surface opens correctly; anything
/// else falls back to a text target keyed on the locator (the unit/output id).
fn jump_target(
    items: &[FeedbackItem],
    id: &str,
    outputs: &[OutputArtifact],
) -> Option<AnnotateTarget> {
    let item = items.iter().find(|i| i.feedback_id == id)?;
    // The locator is the back-reference to the origin artifact; fall back to the
    // title so a Jump still lands on *something* the operator can recognize.
    let locator = item
        .source_ref
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| (!item.title.is_empty()).then(|| item.title.clone()))?;

    // Prefer an exact output match (gives us the visual class + screenshot URL),
    // then a contains match so a `payment.rs:42` locator still finds `payment.rs`.
    if let Some(out) = outputs
        .iter()
        .find(|o| o.name == locator || o.run_relative_path.as_deref() == Some(locator.as_str()))
        .or_else(|| outputs.iter().find(|o| locator.contains(&o.name)))
    {
        let visual = output_is_visual(out);
        return Some(AnnotateTarget {
            label: out.name.clone(),
            path: out.run_relative_path.clone().unwrap_or_else(|| out.name.clone()),
            work_id: out.name.clone(),
            visual,
            screenshot_url: out.relative_path.clone(),
            text: None,
        });
    }

    // No declared output — anchor a text surface on the locator directly.
    Some(AnnotateTarget {
        label: locator.clone(),
        path: locator.clone(),
        work_id: locator,
        visual: false,
        screenshot_url: None,
        text: None,
    })
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
    persisted: Vec<TextMark>,
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
            text: target.text.clone(),
            persisted,
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
    text: Option<String>,
    persisted: Vec<TextMark>,
    on_close: EventHandler<MouseEvent>,
) -> Element {
    let kind = if visual { SurfaceKind::Visual } else { SurfaceKind::Text };
    let default_tool = if visual { AnnotateTool::Pin } else { AnnotateTool::Select };
    let tool = use_signal(|| default_tool);
    // The placed visual marks (pin/rect/arrow/path/highlight) over the surface.
    let mut marks = use_signal(Vec::<VisualMark>::new);
    let mut comments = use_signal(Vec::<String>::new);
    let submit = use_signal(|| Submit::Idle);
    // The reviewer's in-flight TEXT selections (this session). Persisted anchors
    // from earlier feedback render alongside, numbered first.
    let mut text_marks = use_signal(Vec::<TextMark>::new);
    // Capture the webview's real text selection when a text tool is active —
    // the span is located in the artifact's paragraphs and painted immediately,
    // so the selection is REPRESENTED, not implied.
    let sel_text = text.clone();
    let on_select = use_callback(move |selected: String| {
        let selected = selected.trim().to_string();
        if selected.is_empty() {
            return;
        }
        let active = *tool.read();
        if active == AnnotateTool::Cursor {
            return;
        }
        let paragraph = sel_text
            .as_deref()
            .map(|t| paragraph_of(t, &selected))
            .unwrap_or(0);
        text_marks.write().push(TextMark {
            selected_text: selected,
            paragraph,
            tool: Some(active),
            stale: false,
        });
    });

    // Capture a completed gesture for the active tool. The stage forwards the
    // gesture as normalized `0..1` geometry (start point, end point, and a
    // freehand point list for the pen) so this only has to wrap the matching
    // [`VisualMark`].
    let place = move |gesture: Gesture| {
        let active = *tool.read();
        let n = marks.read().len() + 1;
        let mark = match active {
            AnnotateTool::Box => {
                let r = gesture.norm_rect(format!("box {n}"));
                Some(VisualMark::Rect { rect: r })
            }
            AnnotateTool::Highlight => {
                let r = gesture.norm_rect(format!("highlight {n}"));
                Some(VisualMark::Highlight { rect: r })
            }
            AnnotateTool::Arrow => Some(VisualMark::Arrow {
                from: PinPoint::new(gesture.start.0, gesture.start.1, String::new()),
                to: PinPoint::new(gesture.end.0, gesture.end.1, format!("arrow {n}")),
            }),
            AnnotateTool::Pen => {
                let pts: Vec<PinPoint> = gesture
                    .path
                    .iter()
                    .map(|(x, y)| PinPoint::new(*x, *y, String::new()))
                    .collect();
                // A stroke needs at least two points to be a path; a stray tap
                // degrades to a pin so the click still lands a mark.
                if pts.len() >= 2 {
                    let mut pts = pts;
                    if let Some(last) = pts.last_mut() {
                        last.note = format!("pen {n}");
                    }
                    Some(VisualMark::Path { points: pts })
                } else {
                    Some(VisualMark::Pin {
                        point: PinPoint::new(gesture.start.0, gesture.start.1, format!("pin {n}")),
                    })
                }
            }
            // Pin (and any other/neutral spatial tool) drops a point.
            _ => Some(VisualMark::Pin {
                point: PinPoint::new(gesture.start.0, gesture.start.1, format!("pin {n}")),
            }),
        };
        if let Some(mark) = mark {
            marks.write().push(mark);
        }
    };

    let do_submit = {
        let cfg = cfg.clone();
        let run = run.clone();
        let station = station.clone();
        let label = label.clone();
        let path = path.clone();
        let text_for_submit_outer = text.clone();
        move |draft: CommentDraft| {
            let text_for_submit = text_for_submit_outer.clone();
            let cfg = cfg.clone();
            let run = run.clone();
            let station = station.clone();
            let label = label.clone();
            let path = path.clone();
            let active = *tool.read();
            let mut submit = submit;
            // Capture the comment typed in the panel before reading the thread so
            // the user's text ships with the annotation, not just the marks/counts.
            let typed = draft.comment.trim();
            if !typed.is_empty() {
                comments.write().push(typed.to_string());
            }
            // The `suggest` tool authored a replacement; `strike` is a deletion —
            // both ride the annotation's suggestion slot (a diff on the span).
            let suggestion = if active == AnnotateTool::Strike {
                // A strike marks the span for removal — model it as a suggestion
                // with an empty replacement, consistent with how `suggest` was
                // wired (resolution → inline fix), so the agent deletes the span.
                Some(String::new())
            } else {
                let s = draft.suggestion.trim().to_string();
                (!s.is_empty()).then_some(s)
            };
            let mark_list = marks.read().clone();
            let comment_list = comments.read().clone();
            // The newest user-made text mark anchors this feedback; the hash of
            // the artifact at save time drives later stale detection.
            let anchor_mark = text_marks
                .read()
                .iter()
                .rev()
                .find(|m| m.tool.is_some())
                .cloned();
            let content_sha = text_for_submit
                .as_deref()
                .map(|t| darkrun_core::hash_bytes(t.as_bytes()));
            spawn(async move {
                submit.set(Submit::Sending);
                let result = if visual {
                    // Visual artifact → record each mark's shape + normalized
                    // geometry over the screenshot. Each mark maps to the wire's
                    // `ImageShape` (pin/rect/arrow/path/highlight); the legacy pin
                    // channel carries the anchor point + a structured note so the
                    // exact geometry ships to the agent.
                    let pins = mark_list
                        .iter()
                        .map(visual_mark_to_pin)
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
                    let mut body = if comment_list.is_empty() {
                        "(no comment)".to_string()
                    } else {
                        comment_list.join("\n")
                    };
                    // A suggestion (or a strike's deletion) rides in the body as a
                    // fenced replacement and flips the resolution to a single
                    // inline fix the agent applies — the annotation's `suggestion`
                    // slot, on the wire. `strike` ships an empty replacement, i.e.
                    // "remove this span".
                    let resolution = match &suggestion {
                        Some(repl) => {
                            if active == AnnotateTool::Strike {
                                body.push_str("\n\nstrike: remove the selected span.");
                            }
                            body.push_str(&format!("\n\n```suggestion\n{repl}\n```"));
                            Some(darkrun_api::common::FeedbackResolution::InlineFix)
                        }
                        None => None,
                    };
                    // Persist the span the reviewer marked: the anchor carries
                    // the exact selected text, its paragraph, and a content hash
                    // so the mark re-renders (and stales) on later views.
                    let inline_anchor = anchor_mark.map(|m| {
                        darkrun_api::feedback::FeedbackInlineAnchor {
                            selected_text: m.selected_text.clone(),
                            paragraph: m.paragraph,
                            location: format!("paragraph {}", m.paragraph + 1),
                            comment_id: None,
                            file_path: Some(path.clone()),
                            content_sha: content_sha.clone(),
                        }
                    });
                    let req = FeedbackCreateRequest {
                        title: format!("review: {label}"),
                        body,
                        origin: Some(FeedbackOrigin::UserVisual),
                        author: None,
                        source_ref: Some(path.clone()),
                        anchor: None,
                        inline_anchor,
                        resolution,
                        attachment_data_url: None,
                    };
                    wire::submit_annotation(&cfg, &run, &station, &req).await
                };
                match result {
                    Ok(()) => submit.set(Submit::Sent(format!(
                        "annotation recorded ({} marks · {} comments)",
                        mark_list.len(),
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
    let placed = marks.read().clone();

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
                    on_pick: move |t| {
                        let mut tool = tool;
                        tool.set(t);
                    },
                }
            }
            div { style: "display:flex;gap:16px;align-items:flex-start;",
                // The artifact stage — the active tool's gesture lands the
                // matching shape (pin/box/arrow/pen/highlight).
                {
                    // Persisted anchors (earlier feedback) lead the numbering;
                    // this session's selections follow.
                    let mut display_marks = persisted.clone();
                    display_marks.extend(text_marks.read().iter().cloned());
                    annotate_stage(
                        visual,
                        active_tool,
                        screenshot_url.clone(),
                        placed,
                        text.clone(),
                        display_marks,
                        on_select,
                        place,
                    )
                }
                div { style: "flex:1;min-width:0;",
                    CommentPanel {
                        comments: thread,
                        placeholder: "comment on this artifact…".to_string(),
                        suggest: !visual && active_tool == AnnotateTool::Suggest,
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

/// The stage's fixed pixel box — the flex-basis width and min-height the gesture
/// math normalizes against. Kept here so the click→`0..1` mapping is one source.
const STAGE_W: f64 = 360.0;
const STAGE_H: f64 = 220.0;

/// A completed pointer gesture over the visual stage, in normalized `0..1` space.
///
/// `start`/`end` bracket a click or a drag (equal for a single click); `path` is
/// the freehand point list the pen accumulates. The active tool decides which of
/// these it consumes — a [`VisualMark`] is built in the surface's `place`.
#[derive(Debug, Clone, Default)]
struct Gesture {
    /// The gesture's start point (drag tail / click), `0..1`.
    start: (f64, f64),
    /// The gesture's end point (drag head / click), `0..1`.
    end: (f64, f64),
    /// The freehand stroke points, in draw order, `0..1` (pen only).
    path: Vec<(f64, f64)>,
}

impl Gesture {
    /// The drag rectangle as a normalized [`NormBox`], origin-normalized so the
    /// drag direction doesn't matter.
    fn norm_rect(&self, note: impl Into<String>) -> NormBox {
        NormBox::from_corners(self.start.0, self.start.1, self.end.0, self.end.1, note)
    }
}

/// Normalize a stage pixel offset into the `0..1` box.
fn norm_xy(px: f64, py: f64) -> (f64, f64) {
    ((px / STAGE_W).clamp(0.0, 1.0), (py / STAGE_H).clamp(0.0, 1.0))
}

/// The artifact stage the annotate surface paints over — the screenshot (visual)
/// or a text placeholder. Captures the active tool's gesture (a click for `pin`,
/// a drag for `box`/`highlight`/`arrow`, a tracked stroke for `pen`) and forwards
/// it normalized; renders the matching overlay for every placed mark.
#[allow(clippy::too_many_arguments)]
fn annotate_stage(
    visual: bool,
    active: AnnotateTool,
    screenshot_url: Option<String>,
    marks: Vec<VisualMark>,
    text: Option<String>,
    text_marks: Vec<TextMark>,
    on_select: Callback<String>,
    mut on_place: impl FnMut(Gesture) + 'static,
) -> Element {
    // The in-flight gesture: the press origin and (for the pen) the accumulating
    // stroke. `None` origin means the pointer is up.
    let mut origin = use_signal(|| None::<(f64, f64)>);
    let mut stroke = use_signal(Vec::<(f64, f64)>::new);

    let stage = format!(
        "position:relative;flex:0 0 360px;min-height:220px;border-radius:8px;\
         border:1px solid {border};background:{base};overflow:hidden;\
         {cursor}",
        border = tokens::var::BORDER,
        base = tokens::var::SURFACE_BASE,
        cursor = if visual { "cursor:crosshair;" } else { "" },
    );
    let is_pen = active == AnnotateTool::Pen;
    rsx! {
        div {
            class: "dr-annotate-stage",
            style: "{stage}",
            onmousedown: move |evt| {
                if !visual {
                    return;
                }
                let c = evt.element_coordinates();
                let p = norm_xy(c.x, c.y);
                origin.set(Some(p));
                stroke.set(vec![p]);
            },
            onmousemove: move |evt| {
                // Only the pen needs the intermediate points; other tools resolve
                // from the press origin + the release point.
                if !visual || !is_pen || origin.read().is_none() {
                    return;
                }
                let c = evt.element_coordinates();
                stroke.write().push(norm_xy(c.x, c.y));
            },
            onmouseup: move |evt| {
                if !visual {
                    // TEXT surface: read the webview's real selection and hand
                    // it up — the span becomes a visible, numbered mark.
                    spawn(async move {
                        let sel = dioxus::document::eval(
                            "return (window.getSelection() ? window.getSelection().toString() : '');",
                        )
                        .join::<String>()
                        .await
                        .unwrap_or_default();
                        if !sel.trim().is_empty() {
                            on_select.call(sel);
                            // Collapse the native selection — the painted mark
                            // is now the representation.
                            let _ = dioxus::document::eval(
                                "window.getSelection() && window.getSelection().removeAllRanges();",
                            );
                        }
                    });
                    return;
                }
                let Some(start) = *origin.read() else { return };
                let c = evt.element_coordinates();
                let end = norm_xy(c.x, c.y);
                let mut path = stroke.read().clone();
                path.push(end);
                on_place(Gesture { start, end, path });
                origin.set(None);
                stroke.set(Vec::new());
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
                        "draw on the surface to point at it"
                    }
                }
                // Render every placed mark with its matching overlay primitive.
                for (i, mark) in marks.iter().enumerate() {
                    {render_mark(mark, i + 1)}
                }
            } else if let Some(body) = text.as_deref() {
                {render_text_with_marks(body, &text_marks)}
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

/// The zero-based paragraph (blank-line-separated) containing `needle`'s first
/// occurrence, falling back to a whole-text scan (paragraph 0) when the span
/// crosses a boundary or isn't found.
fn paragraph_of(text: &str, needle: &str) -> u32 {
    let mut offset = 0usize;
    for (i, para) in text.split("\n\n").enumerate() {
        if para.contains(needle) {
            return i as u32;
        }
        offset += para.len() + 2;
        let _ = offset;
    }
    0
}

/// The inline style for a painted text mark, by tool + staleness. Every mark is
/// accent-tinted; the tool varies the decoration; a stale (content-drifted)
/// anchor goes amber with a dashed underline.
fn mark_span_style(tool: Option<AnnotateTool>, stale: bool) -> String {
    if stale {
        return "background:color-mix(in srgb, var(--dr-status-warn) 22%, transparent);\
                border-bottom:2px dashed var(--dr-status-warn);border-radius:3px;\
                padding:0 2px;".to_string();
    }
    let base = "background:color-mix(in srgb, var(--dr-accent) 24%, transparent);\
                border-radius:3px;padding:0 2px;";
    let deco = match tool {
        Some(AnnotateTool::Strike) => "text-decoration:line-through;",
        Some(AnnotateTool::Suggest) => "border-bottom:2px dotted var(--dr-accent);",
        Some(AnnotateTool::Select) => "border-bottom:2px solid var(--dr-accent);",
        // Highlight, persisted anchors, and anything else: the tinted span.
        _ => "",
    };
    format!("{base}{deco}")
}

/// Render the artifact's text with every [`TextMark`] painted in place: each
/// paragraph splits at its marks' spans, wrapping them in numbered, tinted
/// spans (the predecessor's inline-comments rendering, in Dioxus). Marks whose
/// span no longer appears in their paragraph (drifted text) are listed under
/// the body as stale chips, so an anchored comment is never silently invisible.
fn render_text_with_marks(body: &str, marks: &[TextMark]) -> Element {
    let paragraphs: Vec<&str> = body.split("\n\n").collect();
    let mut unmatched: Vec<(usize, &TextMark)> = Vec::new();

    let para_block = paragraphs.iter().enumerate().map(|(pi, para)| {
        // The marks anchored to this paragraph, in mark order.
        let mut segments: Vec<(String, Option<(usize, &TextMark)>)> = Vec::new();
        let mut rest: &str = para;
        let mut consumed = 0usize;
        let mut local: Vec<(usize, &TextMark)> = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.paragraph as usize == pi)
            .collect();
        // Paint in textual order so splits never overlap.
        local.sort_by_key(|(_, m)| para.find(m.selected_text.as_str()).unwrap_or(usize::MAX));
        for (n, mark) in local {
            match rest.find(mark.selected_text.as_str()) {
                Some(at) => {
                    segments.push((rest[..at].to_string(), None));
                    segments.push((mark.selected_text.clone(), Some((n, mark))));
                    rest = &rest[at + mark.selected_text.len()..];
                    consumed += at + mark.selected_text.len();
                    let _ = consumed;
                }
                None => unmatched.push((n, mark)),
            }
        }
        segments.push((rest.to_string(), None));
        rsx! {
            p { style: "margin:0 0 10px;",
                for (seg, mark) in segments.into_iter() {
                    if let Some((n, m)) = mark {
                        span { style: mark_span_style(m.tool, m.stale),
                            "{seg}"
                            sup {
                                style: "font-size:9px;font-weight:700;color:var(--dr-on-accent);\
                                        background:var(--dr-accent);border-radius:3px;\
                                        padding:0 3px;margin-left:2px;",
                                "{n + 1}"
                            }
                        }
                    } else {
                        span { "{seg}" }
                    }
                }
            }
        }
    }).collect::<Vec<_>>();

    rsx! {
        div {
            class: "dr-annotate-text",
            style: "padding:14px;color:var(--dr-text);font-size:12px;line-height:1.7;\
                    font-family:var(--dr-font-mono);white-space:pre-wrap;overflow:auto;\
                    max-height:420px;user-select:text;cursor:text;",
            for block in para_block.into_iter() {
                {block}
            }
            if !unmatched.is_empty() {
                div { style: "margin-top:10px;display:flex;flex-direction:column;gap:5px;",
                    for (n, m) in unmatched.into_iter() {
                        span {
                            style: "font-family:var(--dr-font-mono);font-size:11px;\
                                    color:var(--dr-status-warn);",
                            "#{n + 1} \u{201c}{m.selected_text}\u{201d} — the anchored text \
                             has changed since this was annotated"
                        }
                    }
                }
            }
        }
    }
}

/// Paint one placed [`VisualMark`] with the overlay primitive matching its shape.
fn render_mark(mark: &VisualMark, number: usize) -> Element {
    match mark {
        VisualMark::Pin { point } => rsx! { PinMarker { point: point.clone(), number } },
        VisualMark::Rect { rect } => rsx! {
            BoxMarker { x: rect.x, y: rect.y, w: rect.w, h: rect.h, number }
        },
        VisualMark::Highlight { rect } => rsx! {
            HighlightMarker { x: rect.x, y: rect.y, w: rect.w, h: rect.h, number }
        },
        VisualMark::Arrow { from, to } => rsx! {
            ArrowMarker { from: from.clone(), to: to.clone(), number }
        },
        VisualMark::Path { points } => rsx! {
            PathMarker { points: points.clone(), number }
        },
    }
}

/// Map a placed [`VisualMark`] onto the wire's typed [`PixelMark`]/[`ImageShape`]
/// geometry, paired with the `0..1` render box. This is the load-bearing
/// pin→`ImageShape` routing: `pin`→point, `box`→rect, `highlight`→highlight rect,
/// `arrow`→from/to, `pen`→path. The render dimensions are the fixed stage box.
fn mark_to_anchor(mark: &VisualMark) -> darkrun_api::Anchor {
    use darkrun_api::{ImageShape, NormPoint, NormRect, PixelMark};
    let pt = |p: &PinPoint| NormPoint { x: p.x, y: p.y };
    let mark = match mark {
        VisualMark::Pin { point } => PixelMark {
            shape: ImageShape::Pin,
            point: Some(pt(point)),
            rect: None,
            arrow_from: None,
            arrow_to: None,
            path: Vec::new(),
            render_w: STAGE_W as u32,
            render_h: STAGE_H as u32,
        },
        VisualMark::Rect { rect } => PixelMark {
            shape: ImageShape::Rect,
            point: None,
            rect: Some(NormRect { x: rect.x, y: rect.y, w: rect.w, h: rect.h }),
            arrow_from: None,
            arrow_to: None,
            path: Vec::new(),
            render_w: STAGE_W as u32,
            render_h: STAGE_H as u32,
        },
        VisualMark::Highlight { rect } => PixelMark {
            shape: ImageShape::Highlight,
            point: None,
            rect: Some(NormRect { x: rect.x, y: rect.y, w: rect.w, h: rect.h }),
            arrow_from: None,
            arrow_to: None,
            path: Vec::new(),
            render_w: STAGE_W as u32,
            render_h: STAGE_H as u32,
        },
        VisualMark::Arrow { from, to } => PixelMark {
            shape: ImageShape::Arrow,
            point: None,
            rect: None,
            arrow_from: Some(pt(from)),
            arrow_to: Some(pt(to)),
            path: Vec::new(),
            render_w: STAGE_W as u32,
            render_h: STAGE_H as u32,
        },
        VisualMark::Path { points } => PixelMark {
            shape: ImageShape::Path,
            point: None,
            rect: None,
            arrow_from: None,
            arrow_to: None,
            path: points.iter().map(pt).collect(),
            render_w: STAGE_W as u32,
            render_h: STAGE_H as u32,
        },
    };
    darkrun_api::Anchor::Image { mark }
}

/// Project a placed [`VisualMark`] onto the legacy [`VisualReviewPin`] channel:
/// the anchor point carries the representative coordinate, and the note carries
/// the shape slug plus the serialized [`mark_to_anchor`] geometry so the exact
/// `ImageShape` ships to the agent even though the pin channel is point-only.
fn visual_mark_to_pin(mark: &VisualMark) -> VisualReviewPin {
    let anchor = mark_to_anchor(mark);
    let geometry = serde_json::to_string(&anchor).unwrap_or_default();
    let pt = mark.anchor_point();
    let base = mark.note();
    let note = if base.is_empty() {
        format!("[{}] {}", mark.shape_slug(), geometry)
    } else {
        format!("{base} [{}] {}", mark.shape_slug(), geometry)
    };
    VisualReviewPin { x: pt.x, y: pt.y, note }
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
                    border = tokens::var::BORDER,
                    base = tokens::var::SURFACE_BASE,
                    text = tokens::var::TEXT,
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
    // Answer the payload's OWN session, not the channel we're subscribed to:
    // a question raised under `q-NN` is mirrored onto the run channel, so the
    // desktop renders it while subscribed to the run slug — but the answer must
    // POST to `/question/q-NN/answer` for the engine to record + read it back.
    let cfg = cfg.with_session(q.session_id.clone());
    let answered = matches!(
        q.status,
        darkrun_api::common::SessionStatus::Answered
            | darkrun_api::common::SessionStatus::Approved
    );
    let seed = q.answer.as_ref().map(|a| a.selected.clone()).unwrap_or_default();
    // Rewrite any file:// mockup / reference urls into the engine's HTTP asset
    // route so the webview can load them (it cannot read file://).
    let run = q.run_slug.clone().unwrap_or_default();
    let mut options = map::option_cards(&q.options);
    for o in &mut options {
        o.image_url = o.image_url.as_deref().map(|u| cfg.asset_url(&run, u));
        o.image_url_light = o.image_url_light.as_deref().map(|u| cfg.asset_url(&run, u));
    }
    let image_urls: Vec<String> = q.image_urls.iter().map(|u| cfg.asset_url(&run, u)).collect();
    rsx! {
        QuestionSession {
            cfg,
            prompt: q.prompt.clone(),
            context: q.context.clone(),
            title: q.title.clone(),
            options,
            multi_select: q.multi_select,
            image_urls,
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
    // Decide against the payload's own session (see `question_session`).
    let cfg = cfg.with_session(d.session_id.clone());
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
    // file:// mockup urls -> engine HTTP asset route (see `question_session`).
    let run = d.run_slug.clone().unwrap_or_default();
    let mut archetypes = map::archetype_cards(&d.archetypes);
    for a in &mut archetypes {
        a.image_url = cfg.asset_url(&run, &a.image_url);
        a.image_url_light = a.image_url_light.as_deref().map(|u| cfg.asset_url(&run, u));
    }
    rsx! {
        DirectionSession {
            cfg,
            prompt: d.prompt.clone(),
            context: d.context.clone(),
            title: d.title.clone(),
            archetypes,
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
    // Select against the payload's own session (see `question_session`).
    let cfg = cfg.with_session(p.session_id.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_api::common::{AuthorType, FeedbackOrigin, FeedbackStatus};
    use darkrun_api::session::{OutputArtifact, OutputArtifactType};

    fn item(id: &str, source_ref: Option<&str>, title: &str) -> FeedbackItem {
        FeedbackItem {
            feedback_id: id.into(),
            title: title.into(),
            body: "b".into(),
            status: FeedbackStatus::Pending,
            origin: FeedbackOrigin::UserVisual,
            severity: None,
            author: "you".into(),
            author_type: AuthorType::Human,
            created_at: "2026-05-31T00:00:00Z".into(),
            visit: 1,
            source_ref: source_ref.map(Into::into),
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

    fn output(name: &str, ty: OutputArtifactType) -> OutputArtifact {
        OutputArtifact {
            station: "build".into(),
            name: name.into(),
            artifact_type: ty,
            language: None,
            directory: None,
            content: None,
            relative_path: Some(format!("/api/output/{name}")),
            run_relative_path: Some(format!("outputs/{name}")),
        }
    }

    #[test]
    fn jump_matches_a_visual_output_and_carries_its_screenshot() {
        let items = vec![item("FB-01", Some("dashboard.png"), "review: dashboard")];
        let outputs = vec![output("dashboard.png", OutputArtifactType::Image)];
        let target = jump_target(&items, "FB-01", &outputs).expect("resolves");
        assert!(target.visual, "an image output opens the visual surface");
        assert_eq!(target.work_id, "dashboard.png");
        assert_eq!(target.path, "outputs/dashboard.png");
        assert_eq!(target.screenshot_url.as_deref(), Some("/api/output/dashboard.png"));
    }

    #[test]
    fn jump_matches_a_text_output_on_the_text_surface() {
        let items = vec![item("FB-02", Some("payment.rs"), "review: payment")];
        let outputs = vec![output("payment.rs", OutputArtifactType::Code)];
        let target = jump_target(&items, "FB-02", &outputs).expect("resolves");
        assert!(!target.visual, "a code output stays on the text surface");
        assert_eq!(target.work_id, "payment.rs");
    }

    #[test]
    fn jump_finds_the_output_when_the_locator_carries_a_line_suffix() {
        let items = vec![item("FB-03", Some("payment.rs:42-44"), "review")];
        let outputs = vec![output("payment.rs", OutputArtifactType::Code)];
        let target = jump_target(&items, "FB-03", &outputs).expect("resolves via contains");
        assert_eq!(target.work_id, "payment.rs");
    }

    #[test]
    fn jump_falls_back_to_a_text_target_for_an_unmatched_locator() {
        // No declared output matches — a unit annotation anchors on the locator.
        let items = vec![item("FB-04", Some("auth-flow"), "review: auth-flow")];
        let target = jump_target(&items, "FB-04", &[]).expect("resolves to text");
        assert!(!target.visual);
        assert_eq!(target.work_id, "auth-flow");
        assert_eq!(target.label, "auth-flow");
    }

    #[test]
    fn jump_falls_back_to_the_title_when_no_source_ref() {
        let items = vec![item("FB-05", None, "loose note")];
        let target = jump_target(&items, "FB-05", &[]).expect("resolves to title");
        assert_eq!(target.label, "loose note");
    }

    #[test]
    fn jump_returns_none_for_an_unknown_id() {
        let items = vec![item("FB-06", Some("x"), "t")];
        assert!(jump_target(&items, "FB-99", &[]).is_none());
    }

    // --- visual mark → ImageShape routing ----------------------------------

    #[test]
    fn pin_mark_maps_to_a_point_anchor() {
        let mark = VisualMark::Pin { point: PinPoint::new(0.4, 0.6, "pin 1") };
        let darkrun_api::Anchor::Image { mark: pm } = mark_to_anchor(&mark) else {
            panic!("pin → image anchor");
        };
        assert_eq!(pm.shape, darkrun_api::ImageShape::Pin);
        let p = pm.point.expect("a pin carries a point");
        assert!((p.x - 0.4).abs() < 1e-9 && (p.y - 0.6).abs() < 1e-9);
        assert!(pm.rect.is_none() && pm.path.is_empty());
        assert_eq!(pm.render_w, STAGE_W as u32);
    }

    #[test]
    fn box_and_highlight_map_to_their_rect_shapes() {
        let r = NormBox::new(0.1, 0.2, 0.3, 0.25, "box 1");
        let darkrun_api::Anchor::Image { mark: pm } =
            mark_to_anchor(&VisualMark::Rect { rect: r.clone() })
        else {
            panic!("rect anchor");
        };
        assert_eq!(pm.shape, darkrun_api::ImageShape::Rect);
        let rect = pm.rect.expect("a rect carries a rect");
        assert!((rect.w - 0.3).abs() < 1e-9 && (rect.h - 0.25).abs() < 1e-9);

        let darkrun_api::Anchor::Image { mark: hm } =
            mark_to_anchor(&VisualMark::Highlight { rect: r })
        else {
            panic!("highlight anchor");
        };
        // Highlight is rect-shaped but tagged distinctly so the agent knows it
        // was a soft sweep, not a hard box.
        assert_eq!(hm.shape, darkrun_api::ImageShape::Highlight);
        assert!(hm.rect.is_some());
    }

    #[test]
    fn arrow_mark_carries_tail_and_head() {
        let mark = VisualMark::Arrow {
            from: PinPoint::new(0.1, 0.1, ""),
            to: PinPoint::new(0.8, 0.7, "arrow 1"),
        };
        let darkrun_api::Anchor::Image { mark: pm } = mark_to_anchor(&mark) else {
            panic!("arrow anchor");
        };
        assert_eq!(pm.shape, darkrun_api::ImageShape::Arrow);
        let from = pm.arrow_from.expect("tail");
        let to = pm.arrow_to.expect("head");
        assert!((from.x - 0.1).abs() < 1e-9);
        assert!((to.x - 0.8).abs() < 1e-9 && (to.y - 0.7).abs() < 1e-9);
    }

    #[test]
    fn pen_mark_carries_the_full_path() {
        let mark = VisualMark::Path {
            points: vec![
                PinPoint::new(0.1, 0.1, ""),
                PinPoint::new(0.2, 0.3, ""),
                PinPoint::new(0.4, 0.5, "pen 1"),
            ],
        };
        let darkrun_api::Anchor::Image { mark: pm } = mark_to_anchor(&mark) else {
            panic!("path anchor");
        };
        assert_eq!(pm.shape, darkrun_api::ImageShape::Path);
        assert_eq!(pm.path.len(), 3);
        assert!((pm.path[2].y - 0.5).abs() < 1e-9);
    }

    #[test]
    fn visual_pin_anchor_point_and_note_carry_the_shape() {
        // The legacy pin channel keeps the representative point and embeds the
        // shape slug + serialized geometry in the note, so a rect still ships its
        // full geometry to the agent.
        let mark = VisualMark::Rect { rect: NormBox::new(0.2, 0.25, 0.4, 0.3, "box 1") };
        let pin = visual_mark_to_pin(&mark);
        // Anchor point is the rect's top-left.
        assert!((pin.x - 0.2).abs() < 1e-9 && (pin.y - 0.25).abs() < 1e-9);
        assert!(pin.note.contains("[rect]"), "note tags the shape: {}", pin.note);
        assert!(pin.note.contains("box 1"));
        // The embedded geometry round-trips back to an image anchor.
        let json_start = pin.note.find('{').expect("embeds json");
        let anchor: darkrun_api::Anchor =
            serde_json::from_str(&pin.note[json_start..]).expect("geometry parses");
        assert_eq!(anchor.artifact_type(), darkrun_api::ArtifactType::Image);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::wire::ConnConfig;
    use darkrun_api::session::{
        DirectionSessionPayload, PickerSessionPayload, QuestionSessionPayload, ReviewSessionPayload,
    };

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn review_app_loading_state_renders() {
        fn App() -> Element {
            rsx! { ReviewApp { cfg: ConnConfig::from_env() } }
        }
        let _ = render(App);
    }

    #[test]
    fn review_body_renders_with_a_payload() {
        fn App() -> Element {
            let decision = use_signal(|| Decision::Idle);
            review_body(
                ConnConfig::from_env(),
                ReviewSessionPayload { session_id: "s".into(), ..Default::default() },
                decision,
            )
        }
        let _ = render(App);
    }

    #[test]
    fn question_direction_picker_sessions_render() {
        fn AppQ() -> Element {
            question_session(
                ConnConfig::from_env(),
                QuestionSessionPayload { session_id: "s".into(), prompt: "Pick".into(), ..Default::default() },
            )
        }
        let _ = render(AppQ);

        fn AppD() -> Element {
            direction_session(
                ConnConfig::from_env(),
                DirectionSessionPayload { session_id: "s".into(), prompt: "Choose".into(), ..Default::default() },
            )
        }
        let _ = render(AppD);

        fn AppP() -> Element {
            picker_session(
                ConnConfig::from_env(),
                PickerSessionPayload {
                    session_id: "s".into(),
                    status: Default::default(),
                    run_slug: None,
                    kind: darkrun_api::session::PickerKind::Confirm,
                    title: "Confirm".into(),
                    prompt: "Yes?".into(),
                    options: vec![],
                    selection: None,
                },
            )
        }
        let _ = render(AppP);
    }
}

#[cfg(test)]
mod review_state_render_tests {
    use super::*;
    use crate::wire::ConnConfig;
    use darkrun_api::common::GateType;
    use darkrun_api::session::ReviewSessionPayload;

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn populated(gate: GateType) -> ReviewSessionPayload {
        ReviewSessionPayload {
            session_id: "s".into(),
            run_slug: Some("r".into()),
            gate_type: Some(gate),
            station: Some("build".into()),
            units: vec![
                serde_json::json!({"slug":"u1","title":"Burst limiter","status":"completed","unit_type":"code"}),
                serde_json::json!({"slug":"u2","title":"Tests","status":"in_progress"}),
            ],
            criteria: vec![serde_json::json!({"text":"limiter caps at N"})],
            reflection: Some("learned the burst path".into()),
            ..Default::default()
        }
    }

    #[test]
    fn review_body_renders_populated_across_decision_states() {
        // Idle (default action surface).
        fn Idle() -> Element {
            let d = use_signal(|| Decision::Idle);
            review_body(ConnConfig::from_env(), populated(GateType::Ask), d)
        }
        let _ = render(Idle);
        // Sending (in-flight).
        fn Sending() -> Element {
            let d = use_signal(|| Decision::Sending);
            review_body(ConnConfig::from_env(), populated(GateType::External), d)
        }
        let _ = render(Sending);
        // Sent + Failed (terminal decision banners).
        fn Sent() -> Element {
            let d = use_signal(|| Decision::Sent("approved".into()));
            review_body(ConnConfig::from_env(), populated(GateType::Auto), d)
        }
        let _ = render(Sent);
        fn Failed() -> Element {
            let d = use_signal(|| Decision::Failed("network".into()));
            review_body(ConnConfig::from_env(), populated(GateType::Await), d)
        }
        let _ = render(Failed);
    }
}

#[cfg(test)]
mod subcomponent_render_tests {
    use super::*;
    use crate::wire::ConnConfig;

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn decision_and_submit_status_render_every_state() {
        fn App() -> Element {
            rsx! {
                DecisionStatus { decision: Decision::Idle, gate_open: true }
                DecisionStatus { decision: Decision::Sending, gate_open: true }
                DecisionStatus { decision: Decision::Sent("approved".to_string()), gate_open: false }
                DecisionStatus { decision: Decision::Failed("net".to_string()), gate_open: true }
                SubmitStatus { state: Submit::Idle }
                SubmitStatus { state: Submit::Sending }
                SubmitStatus { state: Submit::Sent("ok".to_string()) }
                SubmitStatus { state: Submit::Failed("err".to_string()) }
            }
        }
        let _ = render(App);
    }

    #[test]
    fn review_header_and_annotate_surface_render() {
        fn App() -> Element {
            rsx! {
                ReviewHeader {
                    title: "Ship it".to_string(),
                    station: Some("build".to_string()),
                    phase: Some(Phase::Manufacture),
                    status: Tone::Info,
                    status_label: "in progress".to_string(),
                    stations: vec![StationItem::new("build", StationStatus::Current)],
                    feedback_count: 3,
                    feedback_alert: true,
                    on_open_feedback: move |_| {},
                }
                AnnotateSurface {
                    cfg: ConnConfig::from_env(),
                    label: "home.png".to_string(),
                    path: "build/home.png".to_string(),
                    work_id: "a2".to_string(),
                    visual: true,
                    screenshot_url: Some("/shot.png".to_string()),
                    text: None,
                    persisted: vec![],
                    on_close: move |_| {},
                }
            }
        }
        let _ = render(App);
    }

    #[test]
    fn question_session_component_renders_answered_and_open() {
        fn App() -> Element {
            rsx! {
                QuestionSession {
                    cfg: ConnConfig::from_env(),
                    prompt: "Pick one".to_string(),
                    options: vec![OptionCard::new("a", "A"), OptionCard::new("b", "B")],
                    multi_select: true,
                    image_urls: vec!["http://img/1.png".to_string()],
                    seed_selected: vec!["a".to_string()],
                    answered: true,
                }
            }
        }
        let _ = render(App);
    }

    #[test]
    fn remaining_session_components_render() {
        fn App() -> Element {
            rsx! {
                DirectionSession {
                    cfg: ConnConfig::from_env(),
                    prompt: "Choose".to_string(),
                    archetypes: vec![ArchetypeCard::new("x", "X", "u", "d")],
                    seed_pins: vec![PinPoint::new(0.2, 0.3, "n")],
                    seed_comments: vec!["c".to_string()],
                    decided: true,
                }
                OutputReviewSession {
                    cfg: ConnConfig::from_env(),
                    artifact_label: Some("home.png".to_string()),
                    screenshot_url: Some("/s.png".to_string()),
                }
                PickerSession {
                    cfg: ConnConfig::from_env(),
                    prompt: "Confirm".to_string(),
                    options: vec![PickerItem::new("y", "Yes")],
                    decided: true,
                }
                ViewSession {
                    cfg: ConnConfig::from_env(),
                    artifacts: vec![ArtifactEntry::new("a1", "build/x.html", ArtifactKind::File, "x")],
                }
            }
        }
        let _ = render(App);
    }
}

#[cfg(test)]
mod tab_render_tests {
    use super::*;
    use darkrun_api::session::ReviewSessionPayload;
    use std::collections::BTreeMap;

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn body(active: &'static str) -> Element {
        let at = use_signal(|| None::<AnnotateTarget>);
        let io = use_signal(|| false);
        let review = ReviewSessionPayload::default();
        let unit_outputs: BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>> = BTreeMap::new();
        tab_body(active, &[], &[], &[], &unit_outputs, &[], &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
    }

    #[test]
    fn build_tabs_includes_feedback_tab_when_present() {
        let with = build_tabs(2, 1, 1, 3);
        let without = build_tabs(0, 0, 0, 0);
        assert!(with.len() >= without.len());
    }

    #[test]
    fn tab_body_renders_each_tab() {
        fn Units() -> Element { body("units") }
        fn Outputs() -> Element { body("outputs") }
        fn Knowledge() -> Element { body("knowledge") }
        fn Feedback() -> Element { body("feedback") }
        for f in [Units as fn() -> Element, Outputs, Knowledge, Feedback] {
            let _ = render(f);
        }
    }

    #[test]
    fn tab_body_renders_populated_units_and_knowledge() {
        use darkrun_api::session::KnowledgeFile;
        fn UnitsPop() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let io = use_signal(|| false);
            let review = ReviewSessionPayload::default();
            let uo: BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>> = BTreeMap::new();
            let units = vec![
                crate::map::unit_view(&serde_json::json!({"slug":"u1","title":"Burst limiter","status":"completed","unit_type":"code"})),
                crate::map::unit_view(&serde_json::json!({"slug":"u2","title":"Tests","status":"in_progress"})),
            ];
            tab_body("units", &units, &[], &[], &uo, &[], &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
        }
        let mut dom = VirtualDom::new(UnitsPop); dom.rebuild_in_place(); let _ = dioxus_ssr::render(&dom);
        fn KnowPop() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let io = use_signal(|| false);
            let review = ReviewSessionPayload::default();
            let uo: BTreeMap<String, Vec<darkrun_api::session::UnitOutputPreview>> = BTreeMap::new();
            let know = vec![KnowledgeFile { name: "notes.md".into(), content: "# notes\nbody".into() }];
            tab_body("knowledge", &[], &[], &know, &uo, &[], &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
        }
        let mut dom2 = VirtualDom::new(KnowPop); dom2.rebuild_in_place(); let _ = dioxus_ssr::render(&dom2);
    }

    #[test]
    fn tab_body_renders_populated_outputs() {
        use darkrun_api::session::{OutputArtifact, OutputArtifactType, UnitOutputPreview, UnitOutputType};
        fn OutPop() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let io = use_signal(|| false);
            let review = ReviewSessionPayload::default();
            let mut uo: BTreeMap<String, Vec<UnitOutputPreview>> = BTreeMap::new();
            uo.insert("u1".into(), vec![UnitOutputPreview {
                path: "src/x.rs".into(), name: "x.rs".into(), output_type: UnitOutputType::File,
                url: "/o/x".into(), preview_body: Some("fn x() {}".into()), size_bytes: Some(42), exists: true,
            }]);
            let outputs = vec![
                OutputArtifact { station: "build".into(), name: "page.html".into(), artifact_type: OutputArtifactType::Html, language: None, directory: None, content: Some("<h1>hi</h1>".into()), relative_path: Some("build/page.html".into()), run_relative_path: None },
                OutputArtifact { station: "build".into(), name: "shot.png".into(), artifact_type: OutputArtifactType::Image, language: None, directory: None, content: None, relative_path: None, run_relative_path: Some("build/shot.png".into()) },
            ];
            tab_body("outputs", &[], &outputs, &[], &uo, &[], &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
        }
        let mut dom = VirtualDom::new(OutPop); dom.rebuild_in_place(); let _ = dioxus_ssr::render(&dom);
    }
}

#[cfg(test)]
mod panel_render_tests {
    use super::*;
    use darkrun_api::common::{
        AuthorType, FeedbackOrigin, FeedbackStatus, GateType, SessionStatus,
    };
    use darkrun_api::session::{
        ApproveAction, ApproveActionKind, OutputArtifact, OutputArtifactType,
        ReviewSessionPayload,
    };

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn fb_item(id: &str, source_ref: Option<&str>, title: &str) -> FeedbackItem {
        FeedbackItem {
            feedback_id: id.into(),
            title: title.into(),
            body: "b".into(),
            status: FeedbackStatus::Pending,
            origin: FeedbackOrigin::UserVisual,
            severity: None,
            author: "you".into(),
            author_type: AuthorType::Human,
            created_at: "2026-05-31T00:00:00Z".into(),
            visit: 1,
            source_ref: source_ref.map(Into::into),
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

    fn out(name: &str, ty: OutputArtifactType) -> OutputArtifact {
        OutputArtifact {
            station: "build".into(),
            name: name.into(),
            artifact_type: ty,
            language: None,
            directory: None,
            content: None,
            relative_path: Some(format!("/api/output/{name}")),
            run_relative_path: Some(format!("outputs/{name}")),
        }
    }

    // ── Pure helpers (no DOM) ───────────────────────────────────────────────

    #[test]
    fn output_kind_labels_every_artifact_type() {
        let cases = [
            (OutputArtifactType::Markdown, "md"),
            (OutputArtifactType::Html, "html"),
            (OutputArtifactType::Image, "img"),
            (OutputArtifactType::Video, "video"),
            (OutputArtifactType::Code, "code"),
            (OutputArtifactType::File, "file"),
        ];
        for (ty, label) in cases {
            assert_eq!(output_kind(&out("x", ty)), label);
        }
    }

    #[test]
    fn output_is_visual_only_for_rendered_surfaces() {
        assert!(output_is_visual(&out("a.png", OutputArtifactType::Image)));
        assert!(output_is_visual(&out("a.html", OutputArtifactType::Html)));
        assert!(output_is_visual(&out("a.mp4", OutputArtifactType::Video)));
        assert!(!output_is_visual(&out("a.rs", OutputArtifactType::Code)));
        assert!(!output_is_visual(&out("a.md", OutputArtifactType::Markdown)));
        assert!(!output_is_visual(&out("a.txt", OutputArtifactType::File)));
    }

    #[test]
    fn style_helpers_emit_non_empty_css() {
        assert!(section_title().contains("font-weight:700"));
        assert!(criteria_list().contains("margin"));
    }

    #[test]
    fn norm_xy_clamps_into_the_unit_square() {
        let (x, y) = norm_xy(-50.0, -50.0);
        assert_eq!((x, y), (0.0, 0.0));
        let (x, y) = norm_xy(1.0e9, 1.0e9);
        assert_eq!((x, y), (1.0, 1.0));
        let (x, _) = norm_xy(STAGE_W / 2.0, 0.0);
        assert!((x - 0.5).abs() < 1.0e-9);
    }

    #[test]
    fn feedback_count_for_counts_open_locator_matches() {
        let entries = map::feedback_entries(&[
            fb_item("FB-1", Some("payment.rs"), "a"),
            fb_item("FB-2", Some("payment.rs:42"), "b"),
            fb_item("FB-3", Some("other.rs"), "c"),
        ]);
        assert_eq!(feedback_count_for(&entries, "payment.rs"), 2);
        assert_eq!(feedback_count_for(&entries, "missing"), 0);
    }

    // ── DOM render: the conditional panels review_body gates behind signals ──

    #[test]
    fn checkpoint_section_renders_clean_and_blocked() {
        fn Clean() -> Element {
            let decision = use_signal(|| Decision::Idle);
            let mut review = ReviewSessionPayload::default();
            review.gate_type = Some(GateType::Ask);
            review.gate_context = Some("Approve the build?".into());
            review.approve_action = Some(ApproveAction {
                label: "Ship it".into(),
                kind: ApproveActionKind::OpenPr,
            });
            checkpoint_section(ConnConfig::from_env(), review, decision, 0)
        }
        fn Blocked() -> Element {
            let decision = use_signal(|| Decision::Sending);
            // No gate_type / approve_action → exercises the fallback labels.
            let review = ReviewSessionPayload::default();
            checkpoint_section(ConnConfig::from_env(), review, decision, 3)
        }
        assert!(render(Clean).contains("Ship it"));
        let blocked = render(Blocked);
        assert!(blocked.contains("3 open blocking"));
    }

    #[test]
    fn feedback_inbox_panel_renders_empty_and_populated() {
        fn Empty() -> Element {
            let feedback = use_signal(Vec::<FeedbackItem>::new);
            let active_tab = use_signal(|| "units".to_string());
            let annotate_target = use_signal(|| None::<AnnotateTarget>);
            let inbox_open = use_signal(|| true);
            feedback_inbox_panel(
                ConnConfig::from_env(),
                Some("run-1".into()),
                Some("build".into()),
                feedback,
                vec![],
                &[],
                active_tab,
                annotate_target,
                inbox_open,
            )
        }
        fn Full() -> Element {
            let items = vec![fb_item("FB-1", Some("home.png"), "review: home")];
            let entries = map::feedback_entries(&items);
            let feedback = use_signal(move || items.clone());
            let active_tab = use_signal(|| "units".to_string());
            let annotate_target = use_signal(|| None::<AnnotateTarget>);
            let inbox_open = use_signal(|| true);
            let outputs = vec![out("home.png", OutputArtifactType::Image)];
            feedback_inbox_panel(
                ConnConfig::from_env(),
                Some("run-1".into()),
                Some("build".into()),
                feedback,
                entries,
                &outputs,
                active_tab,
                annotate_target,
                inbox_open,
            )
        }
        assert!(render(Empty).contains("No feedback on this station yet."));
        assert!(render(Full).contains("Feedback inbox"));
    }

    #[test]
    fn annotate_panel_mounts_the_surface() {
        fn App() -> Element {
            let annotate_target = use_signal(|| None::<AnnotateTarget>);
            let target = AnnotateTarget {
                text: None,
                label: "home.png".into(),
                path: "build/home.png".into(),
                work_id: "home.png".into(),
                visual: true,
                screenshot_url: Some("/api/output/home.png".into()),
            };
            annotate_panel(
                ConnConfig::from_env(),
                Some("run-1".into()),
                Some("build".into()),
                target,
                vec![],
                annotate_target,
            )
        }
        let _ = render(App);
    }

    #[test]
    fn annotate_stage_renders_visual_with_marks_and_text() {
        fn Visual() -> Element {
            let marks = vec![
                VisualMark::Pin { point: PinPoint::new(0.2, 0.3, "") },
                VisualMark::Rect { rect: NormBox::new(0.1, 0.1, 0.2, 0.2, "") },
                VisualMark::Highlight { rect: NormBox::new(0.3, 0.3, 0.1, 0.1, "") },
                VisualMark::Arrow {
                    from: PinPoint::new(0.1, 0.1, ""),
                    to: PinPoint::new(0.5, 0.5, ""),
                },
                VisualMark::Path {
                    points: vec![PinPoint::new(0.1, 0.1, ""), PinPoint::new(0.2, 0.2, "")],
                },
            ];
            annotate_stage(true, AnnotateTool::Pin, Some("/s.png".into()), marks, None, vec![], Callback::new(|_: String| {}), |_| {})
        }
        fn VisualNoShot() -> Element {
            annotate_stage(true, AnnotateTool::Pen, None, vec![], None, vec![], Callback::new(|_: String| {}), |_| {})
        }
        fn Text() -> Element {
            annotate_stage(false, AnnotateTool::Select, None, vec![], Some("Alpha beta.\n\nGamma delta.".into()), vec![TextMark { selected_text: "beta".into(), paragraph: 0, tool: Some(AnnotateTool::Select), stale: false }], Callback::new(|_: String| {}), |_| {})
        }
        fn TextNoBody() -> Element {
            annotate_stage(false, AnnotateTool::Select, None, vec![], None, vec![], Callback::new(|_: String| {}), |_| {})
        }
        let _ = render(Visual);
        assert!(render(VisualNoShot).contains("draw on the surface"));
        // A REAL text body renders with its mark painted in place (numbered),
        // not the placeholder…
        let text = render(Text);
        assert!(text.contains("Alpha"), "body renders: {text}");
        assert!(text.contains("beta"), "the marked span renders: {text}");
        assert!(text.contains("<sup"), "the mark number badge renders: {text}");
        assert!(text.contains("Gamma delta"), "later paragraphs render: {text}");
        assert!(!text.contains("Text artifact —"), "no placeholder when body present");
        // …and the placeholder only appears when there is no body to show.
        assert!(render(TextNoBody).contains("Text artifact"));
    }

    #[test]
    fn render_mark_paints_each_overlay_primitive() {
        fn App() -> Element {
            rsx! {
                {render_mark(&VisualMark::Pin { point: PinPoint::new(0.2, 0.3, "") }, 1)}
                {render_mark(&VisualMark::Rect { rect: NormBox::new(0.1, 0.1, 0.2, 0.2, "") }, 2)}
                {render_mark(&VisualMark::Highlight { rect: NormBox::new(0.2, 0.2, 0.1, 0.1, "") }, 3)}
                {render_mark(&VisualMark::Arrow { from: PinPoint::new(0.0, 0.0, ""), to: PinPoint::new(0.4, 0.4, "") }, 4)}
                {render_mark(&VisualMark::Path { points: vec![PinPoint::new(0.1, 0.1, "")] }, 5)}
            }
        }
        let _ = render(App);
    }

    #[test]
    fn overview_and_feedback_tabs_render_populated() {
        fn Overview() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let io = use_signal(|| false);
            let mut review = ReviewSessionPayload::default();
            review.reflection = Some("Shipped the limiter; revisit the retry budget.".into());
            let st = |station: &str, phase: Option<&str>, merged: bool| {
                darkrun_api::session::StationStateInfo {
                    station: station.into(),
                    merged_into_main: merged,
                    status: None,
                    phase: phase.map(Into::into),
                    started_at: None,
                    completed_at: None,
                    gate_entered_at: None,
                    gate_outcome: None,
                }
            };
            review.station_states = vec![
                st("build", Some("manufacture"), true),
                st("prove", None, false),
            ];
            tab_body("overview", &[], &[], &[], &Default::default(), &[], &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
        }
        fn Feedback() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let io = use_signal(|| false);
            let review = ReviewSessionPayload::default();
            let entries = map::feedback_entries(&[fb_item("FB-1", Some("home.png"), "review: home")]);
            tab_body("feedback", &[], &[], &[], &Default::default(), &entries, &review, at, io, EventHandler::new(|_: (String, FeedbackAction)| {}))
        }
        assert!(render(Overview).contains("Reflection"));
        assert!(render(Feedback).contains("open inbox panel"));
    }

    #[test]
    fn unit_and_output_tabs_render_rows_with_actions() {
        fn Units() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let units = vec![
                map::unit_view(&serde_json::json!({
                    "slug": "u1", "title": "Burst limiter", "status": "completed",
                    "criteria": ["rejects over budget", "emits a 429"]
                })),
            ];
            let entries = map::feedback_entries(&[fb_item("FB-1", Some("Burst limiter"), "x")]);
            unit_tab(&units, &[], &Default::default(), &entries, at)
        }
        fn Outputs() -> Element {
            let at = use_signal(|| None::<AnnotateTarget>);
            let outputs = vec![
                out("page.html", OutputArtifactType::Html),
                out("notes.md", OutputArtifactType::Markdown),
            ];
            let entries = map::feedback_entries(&[fb_item("FB-1", Some("page.html"), "x")]);
            output_tab(&outputs, &entries, at)
        }
        let u = render(Units);
        assert!(u.contains("review"), "the row review action renders");
        let o = render(Outputs);
        assert!(o.contains("review"));
    }

    #[test]
    fn question_session_open_state_renders() {
        // The not-yet-answered branch (answered=false) of the wrapper extractor.
        fn App() -> Element {
            let q = darkrun_api::session::QuestionSessionPayload {
                status: SessionStatus::Pending,
                ..Default::default()
            };
            question_session(ConnConfig::from_env(), q)
        }
        let _ = render(App);
    }
}

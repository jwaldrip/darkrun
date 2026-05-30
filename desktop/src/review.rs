//! The live Review screen.
//!
//! [`ReviewApp`] opens the session WebSocket, holds the latest
//! [`ReviewSessionPayload`] in a signal, and renders:
//!   - the darkrun wordmark + a [`FactoryCard`] header with the live station
//!     pipeline,
//!   - the unit list (each [`UnitRow`]) with its completion criteria,
//!   - declared output deliverables,
//!   - the unit dependency DAG behind the shared [`UnitGraph`] viz, and
//!   - a [`CheckpointBar`] whose approve / request-changes actions POST a
//!     decision back to the engine.
//!
//! Only the `Review` session variant is rendered in full; the other variants
//! (question / direction / picker / view) show a compact placeholder so an
//! unexpected payload never blanks the screen.

use darkrun_api::session::{
    DirectionAnnotations, DirectionSessionPayload, OutputArtifact, PickerSessionPayload,
    QuestionSessionPayload, ReviewSessionPayload,
};
use darkrun_api::{
    DirectionSelectRequest, PickerSelectRequest, QuestionAnswerRequest, ReviewDecisionRequest,
    SessionPayload,
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

    rsx! {
        div { style: "{shell}",
            header {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;",
                Wordmark { variant: WordmarkVariant::Filled, size: 28.0 }
                LinkBadge { link: link.read().clone() }
            }
            match payload.read().clone() {
                Some(SessionPayload::Review(review)) => review_body(cfg.clone(), review, decision),
                Some(SessionPayload::Question(q)) => question_session(cfg.clone(), q),
                Some(SessionPayload::Direction(d)) => direction_session(cfg.clone(), d),
                Some(SessionPayload::Picker(p)) => picker_session(cfg.clone(), p),
                Some(other) => rsx! {
                    Card {
                        Badge { tone: Tone::Neutral, "session: {other.session_type()}" }
                        p { style: "margin-top:8px;color:var(--dr-text-muted);",
                            "This session type isn't rendered by the review app."
                        }
                    }
                },
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

/// The fully-rendered review payload.
///
/// A plain function (not a `#[component]`) because the wire payload types don't
/// derive `PartialEq`, which the component macro requires of its props.
fn review_body(
    cfg: ConnConfig,
    review: ReviewSessionPayload,
    decision: Signal<Decision>,
) -> Element {
    // Header: factory + active station + live phase pipeline.
    let factory = review
        .current_state
        .as_ref()
        .map(|s| s.factory.clone())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "software-factory".to_string());
    let station = review
        .station
        .clone()
        .or_else(|| review.current_state.as_ref().map(|s| s.station.clone()))
        .filter(|s| !s.is_empty());
    let active_phase = review
        .current_state
        .as_ref()
        .and_then(|s| s.phase)
        .map(map::phase);
    let title = review
        .run_slug
        .clone()
        .unwrap_or_else(|| "darkrun review".to_string());
    let header_tone = map::status_tone(review.status);

    // Units flattened out of the opaque parser payload.
    let units: Vec<map::UnitView> = review.units.iter().map(map::unit_view).collect();

    rsx! {
        FactoryCard {
            title,
            factory,
            station: station.clone(),
            phase: active_phase,
            status: header_tone,
            status_label: format!("{:?}", review.status).to_lowercase(),
        }

        UnitList { units: units.clone() }

        if !units.is_empty() {
            UnitDag { units: units.clone() }
        }

        {output_list(review.output_artifacts.clone())}

        {checkpoint_section(cfg, review, decision)}
    }
}

/// The unit list with per-unit completion criteria.
#[component]
fn UnitList(units: Vec<map::UnitView>) -> Element {
    if units.is_empty() {
        return rsx! {
            Card {
                p { style: "color:var(--dr-text-muted);", "No units in this review." }
            }
        };
    }
    rsx! {
        Card {
            h2 { style: section_title(), "Units" }
            div { style: "display:flex;flex-direction:column;gap:10px;margin-top:10px;",
                for unit in units.iter() {
                    div { style: "display:flex;flex-direction:column;gap:6px;",
                        UnitRow {
                            title: unit.title.clone(),
                            unit_type: unit.unit_type.clone(),
                            status: unit.tone,
                            status_label: unit.status_label.clone(),
                            pass: unit.pass,
                        }
                        if !unit.criteria.is_empty() {
                            ul { style: criteria_list(),
                                for line in unit.criteria.iter() {
                                    li { style: "margin:2px 0;", "{line}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The unit dependency DAG, rendered behind the shared `UnitGraph` viz.
///
/// The opaque unit payload carries no edge schema we can rely on, so the graph
/// shows the units as an ordered manufacturing line (each unit depends on the
/// previous) — a faithful default until the wire carries explicit dependencies.
#[component]
fn UnitDag(units: Vec<map::UnitView>) -> Element {
    let nodes: Vec<UnitGraphNode> = units
        .iter()
        .enumerate()
        .map(|(i, u)| UnitGraphNode::new(format!("u{i}"), u.title.clone()).with_tone(u.tone))
        .collect();
    let edges: Vec<GraphEdge> = (1..units.len())
        .map(|i| GraphEdge::new(format!("u{}", i - 1), format!("u{i}")))
        .collect();
    rsx! {
        Card {
            h2 { style: section_title(), "Assembly line" }
            div { style: "margin-top:10px;overflow:auto;",
                UnitGraph { units: nodes, edges }
            }
        }
    }
}

/// The declared-output deliverables list.
fn output_list(outputs: Vec<OutputArtifact>) -> Element {
    if outputs.is_empty() {
        return rsx! {};
    }
    rsx! {
        Card {
            h2 { style: section_title(), "Outputs" }
            div { style: "display:flex;flex-direction:column;gap:8px;margin-top:10px;",
                for out in outputs.iter() {
                    div {
                        style: "display:flex;align-items:center;gap:10px;\
                                font-family:var(--dr-font-mono);font-size:12px;",
                        Badge { tone: Tone::Neutral, "{output_kind(out)}" }
                        span { style: "flex:1;color:var(--dr-text);", "{out.name}" }
                        if !out.station.is_empty() {
                            span { style: "color:var(--dr-text-faint);", "{out.station}" }
                        }
                    }
                }
            }
        }
    }
}

/// The checkpoint bar plus approve / request-changes wiring.
fn checkpoint_section(
    cfg: ConnConfig,
    review: ReviewSessionPayload,
    decision: Signal<Decision>,
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

    // A decision is only meaningful while the gate is actually blocking.
    let gate_open = review.await_active.unwrap_or(true);

    let post = {
        let cfg = cfg.clone();
        move |raw: &'static str| {
            let cfg = cfg.clone();
            let mut decision = decision;
            spawn(async move {
                decision.set(Decision::Sending);
                let req = ReviewDecisionRequest {
                    decision: raw.to_string(),
                    feedback: None,
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

    // One owned clone of the POST closure per click target.
    let bar_approve = post.clone();
    let bar_hold = post.clone();
    let btn_approve = post.clone();
    let btn_changes = post;

    rsx! {
        div { style: "display:flex;flex-direction:column;gap:10px;",
            CheckpointBar {
                kind,
                prompt,
                on_advance: move |_| bar_approve("approved"),
                on_hold: move |_| bar_hold("changes_requested"),
            }
            // Explicit, labelled decision buttons mirror the server's
            // approve-action label and the request-changes path.
            div { style: "display:flex;align-items:center;gap:10px;",
                Button {
                    variant: ButtonVariant::Primary,
                    tone: Tone::Ok,
                    disabled: sending || !gate_open,
                    on_click: move |_| btn_approve("approved"),
                    "{approve_label}"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    tone: Tone::Danger,
                    disabled: sending || !gate_open,
                    on_click: move |_| btn_changes("changes_requested"),
                    "Request changes"
                }
                DecisionStatus { decision: decision.read().clone(), gate_open }
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

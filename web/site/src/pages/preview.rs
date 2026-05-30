//! `/preview` — a static fixture/preview of the interactive session views.
//!
//! Renders a representative VISUAL QUESTION and DESIGN DIRECTION built from the
//! real `darkrun-api` session payload types so the new `darkrun-ui`
//! [`QuestionView`] and [`DirectionView`] components can be viewed (and
//! screenshotted) in the browser without a running engine. The page is a
//! preview only — there is no live feed and the submit/annotation handlers are
//! intentionally unwired; a banner makes that explicit.

use darkrun_api::session::{
    DirectionAnnotations, DirectionArchetype, DirectionPin, DirectionSessionPayload, QuestionOption,
    QuestionSessionPayload,
};
use darkrun_ui::prelude::*;

use crate::pages::review::ScaffoldNote;
use crate::ui::SectionHead;

/// `/preview` — the session-view fixture gallery.
#[component]
pub fn Preview() -> Element {
    let question = sample_question();
    let direction = sample_direction();

    // Map the wire payloads into the darkrun-ui prop data (the same boundary the
    // desktop app crosses), then render them read-only.
    let q_options: Vec<OptionCard> = question
        .options
        .iter()
        .map(|o| OptionCard {
            id: o.id.clone(),
            label: o.label.clone(),
            image_url: o.image_url.clone(),
            description: o.description.clone(),
        })
        .collect();
    let q_selected = question
        .answer
        .as_ref()
        .map(|a| a.selected.clone())
        .unwrap_or_default();

    let d_archetypes: Vec<ArchetypeCard> = direction
        .archetypes
        .iter()
        .map(|a| ArchetypeCard {
            id: a.id.clone(),
            label: a.label.clone(),
            image_url: a.image_url.clone(),
            description: a.description.clone(),
        })
        .collect();
    let d_pins: Vec<PinPoint> = direction
        .annotations
        .as_ref()
        .map(|ann| {
            ann.pins
                .iter()
                .map(|p| PinPoint::new(p.x, p.y, p.note.clone()))
                .collect()
        })
        .unwrap_or_default();
    let d_comments = direction
        .annotations
        .as_ref()
        .map(|ann| ann.comments.clone())
        .unwrap_or_default();

    rsx! {
        SectionHead {
            kicker: "fixture".to_string(),
            title: "Session preview".to_string(),
            lead: Some(
                "A static preview of the interactive session views the agent poses mid-run: a \
                 visual question and a design direction, built from the real darkrun-api payload \
                 types. Preview only — no live feed is attached."
                    .to_string(),
            ),
        }

        ScaffoldNote {
            text: "Fixture: representative QuestionSessionPayload + DirectionSessionPayload \
                   rendered read-only. Submit and annotation actions are unwired here."
                .to_string(),
        }

        div { style: "display:flex;flex-direction:column;gap:32px;margin-top:8px;",
            section {
                "data-fixture": "question",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, tokens::TEXT,
                    ),
                    "Visual question"
                }
                QuestionView {
                    prompt: question.prompt.clone(),
                    context: question.context.clone(),
                    title: question.title.clone(),
                    options: q_options,
                    multi_select: question.multi_select,
                    image_urls: question.image_urls.clone(),
                    selected: q_selected,
                    answered: false,
                }
            }

            section {
                "data-fixture": "direction",
                h2 {
                    style: format!(
                        "font-family:{};font-size:18px;color:{};margin:0 0 12px;",
                        tokens::FONT_SANS, tokens::TEXT,
                    ),
                    "Design direction"
                }
                DirectionView {
                    prompt: direction.prompt.clone(),
                    context: direction.context.clone(),
                    title: direction.title.clone(),
                    archetypes: d_archetypes,
                    chosen: direction.chosen_archetype.clone(),
                    pins: d_pins,
                    comments: d_comments,
                    decided: false,
                }
            }
        }
    }
}

/// A representative visual-question payload built from the real API types.
pub fn sample_question() -> QuestionSessionPayload {
    QuestionSessionPayload {
        session_id: "preview-question".to_string(),
        status: darkrun_api::common::SessionStatus::Pending,
        title: Some("dashboard hero treatment".to_string()),
        prompt: "Which hero layout reads best for the operator dashboard?".to_string(),
        context: Some(
            "Generated three options from the brand tokens. Pick the one that best frames the \
             live run status."
                .to_string(),
        ),
        options: vec![
            QuestionOption {
                id: "split".to_string(),
                label: "Split status rail".to_string(),
                image_url: None,
                description: Some("Run status pinned left, output stream right.".to_string()),
            },
            QuestionOption {
                id: "stacked".to_string(),
                label: "Stacked timeline".to_string(),
                image_url: None,
                description: Some("Phases as a vertical timeline with inline previews.".to_string()),
            },
            QuestionOption {
                id: "grid".to_string(),
                label: "Station grid".to_string(),
                image_url: None,
                description: Some("Every station as a card in a responsive grid.".to_string()),
            },
        ],
        multi_select: false,
        answer: None,
        image_urls: Vec::new(),
    }
}

/// A representative design-direction payload built from the real API types,
/// including two pre-placed annotation pins on the chosen archetype.
pub fn sample_direction() -> DirectionSessionPayload {
    DirectionSessionPayload {
        session_id: "preview-direction".to_string(),
        status: darkrun_api::common::SessionStatus::Pending,
        title: Some("control surface aesthetic".to_string()),
        run_slug: Some("operator-console".to_string()),
        prompt: "Pick the design direction for the operator console.".to_string(),
        context: Some(
            "Three archetypes generated from the dark brand. Choose one and drop pins where you \
             want changes."
                .to_string(),
        ),
        archetypes: vec![
            DirectionArchetype {
                id: "instrument".to_string(),
                label: "Instrument panel".to_string(),
                image_url: String::new(),
                description: "Dense, telemetry-first: gauges, sparklines, monospace readouts."
                    .to_string(),
            },
            DirectionArchetype {
                id: "editorial".to_string(),
                label: "Editorial calm".to_string(),
                image_url: String::new(),
                description: "Generous whitespace, one focal metric, quiet supporting detail."
                    .to_string(),
            },
            DirectionArchetype {
                id: "terminal".to_string(),
                label: "Terminal".to_string(),
                image_url: String::new(),
                description: "Pure mono, log-stream forward, minimal chrome.".to_string(),
            },
        ],
        chosen_archetype: Some("instrument".to_string()),
        annotations: Some(DirectionAnnotations {
            pins: vec![
                DirectionPin {
                    x: 0.25,
                    y: 0.3,
                    note: "tighten the header density".to_string(),
                },
                DirectionPin {
                    x: 0.7,
                    y: 0.62,
                    note: "more contrast on the active gauge".to_string(),
                },
            ],
            screenshot: None,
            comments: vec!["Lean into the instrument metaphor across the whole console.".to_string()],
        }),
    }
}

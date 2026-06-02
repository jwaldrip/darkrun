//! `/review` — how review works.
//!
//! This is **not** the live review surface, and it deliberately does not connect
//! to a running engine. Review happens in the **darkrun desktop app**: a local,
//! dark-brand window that streams the live session over `ws://127.0.0.1:PORT`
//! and never takes over your browser. Remote / web review is a later thing.
//!
//! The page explains that split and shows a representative review layout (built
//! from the real `darkrun-api` types) so the desktop app's surface is legible
//! before you launch it. The `status_tone` mapping is shared with the rest of
//! the site and kept here.

use darkrun_api::review_current::{
    FeedbackSummary, ReviewCurrentPayload, ReviewCurrentStation, ReviewCurrentUnit,
};
use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::ui::SectionHead;

/// `/review` — the "how review works" explainer.
#[component]
pub fn Review() -> Element {
    let payload = sample_payload();
    let phase = payload.phase.as_deref().and_then(Phase::from_name);

    rsx! {
        SectionHead {
            kicker: "how review works".to_string(),
            title: "Review runs in the desktop app".to_string(),
            lead: Some(
                "Review is a local surface. The darkrun desktop app opens a dark window, \
                 connects to the engine on your machine, and streams the live session \u{2014} \
                 it never takes over your browser."
                    .to_string(),
            ),
        }

        DesktopNote {}

        div { style: "margin-top:28px;",
            h2 {
                style: format!(
                    "font-family:{};font-size:18px;color:{};margin:0 0 6px;",
                    tokens::FONT_SANS, theme::TEXT,
                ),
                "What the desktop surface shows"
            }
            p {
                style: format!(
                    "font-family:{};font-size:14px;color:{};margin:0 0 18px;max-width:62ch;",
                    tokens::FONT_SANS, theme::TEXT_MUTED,
                ),
                "A representative review, rendered from the real session types. In the app \
                 this is live: the station pipeline, the units and their criteria, declared \
                 outputs, and an approve / request-changes checkpoint."
            }
        }

        FactoryCard {
            title: format!("run: {}", payload.run),
            factory: "software".to_string(),
            station: payload.station.clone(),
            phase,
            status: Tone::Info,
            status_label: "in review".to_string(),
        }

        div { style: "margin-top:24px;",
            h2 {
                style: format!("font-family:{};font-size:18px;color:{};margin:0 0 10px;", tokens::FONT_SANS, theme::TEXT),
                "Units"
            }
            div { style: "display:flex;flex-direction:column;gap:8px;",
                for unit in payload.units.iter() {
                    UnitRow {
                        title: unit.title.clone(),
                        unit_type: Some("unit".to_string()),
                        status: status_tone(&unit.status),
                        status_label: unit.status.clone(),
                        pass: 1,
                    }
                }
            }
        }

        div { style: "margin-top:24px;",
            FeedbackCounts {
                pending: payload.feedback_summary.pending,
                addressed: payload.feedback_summary.addressed,
                closed: payload.feedback_summary.closed,
                rejected: payload.feedback_summary.rejected,
            }
        }

        div { style: "margin-top:24px;",
            CheckpointBar { kind: CheckpointKind::Ask, prompt: "Advance the station, or hold for changes?".to_string() }
        }
    }
}

/// The note that frames where review actually happens: the local desktop app
/// now, remote / web review later. Dark-brand, terse.
#[component]
pub fn DesktopNote() -> Element {
    let wrap = format!(
        "border:1px solid {border};border-left:3px solid {accent};border-radius:8px;\
         padding:14px 16px;background:{overlay};",
        border = theme::BORDER,
        accent = theme::ACCENT,
        overlay = theme::SURFACE_OVERLAY,
    );
    let line = format!(
        "font-family:{sans};font-size:14px;color:{text};margin:0;",
        sans = tokens::FONT_SANS,
        text = theme::TEXT,
    );
    let sub = format!(
        "font-family:{sans};font-size:13px;color:{muted};margin:8px 0 0;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:8px;",
                Badge { tone: Tone::Accent, filled: true, "desktop app" }
                Badge { tone: Tone::Neutral, "remote review: coming later" }
            }
            p { style: "{line}",
                "Run "
                code {
                    style: format!(
                        "font-family:{};color:{};", tokens::FONT_MONO, theme::ACCENT,
                    ),
                    "darkrun serve"
                }
                " on your machine, then open the desktop app. It lists your runs and opens \
                 any one into its live review \u{2014} all over loopback, nothing leaves your box."
            }
            p { style: "{sub}",
                "Reviewing from the web (or another machine) is on the roadmap, not shipped. \
                 For now the browser is for reading the docs; the app is for driving the work."
            }
        }
    }
}

/// The feedback summary, rendered as tone-coded badges from the real counts.
///
/// Takes primitive counts rather than the `darkrun-api` struct directly because
/// the wire type does not implement `PartialEq` (a Dioxus prop requirement).
#[component]
fn FeedbackCounts(pending: u32, addressed: u32, closed: u32, rejected: u32) -> Element {
    rsx! {
        div { style: "display:flex;gap:10px;flex-wrap:wrap;align-items:center;",
            span {
                style: format!(
                    "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{};",
                    tokens::FONT_MONO, theme::TEXT_FAINT,
                ),
                "feedback"
            }
            Badge { tone: Tone::Warn, "{pending} pending" }
            Badge { tone: Tone::Info, "{addressed} addressed" }
            Badge { tone: Tone::Ok, "{closed} closed" }
            Badge { tone: Tone::Danger, "{rejected} rejected" }
        }
    }
}

/// A small banner marking a not-yet-live scaffold. Kept for reuse by `/browse`.
#[component]
pub fn ScaffoldNote(text: String) -> Element {
    let style = format!(
        "border:1px dashed {border};border-radius:8px;padding:10px 12px;margin:0 0 20px;\
         font-family:{mono};font-size:12px;color:{muted};background:{raised};",
        border = theme::BORDER_STRONG,
        mono = tokens::FONT_MONO,
        muted = theme::TEXT_MUTED,
        raised = theme::SURFACE_RAISED,
    );
    rsx! {
        div { style: "{style}", "{text}" }
    }
}

/// Map a display status string onto a UI tone.
pub fn status_tone(status: &str) -> Tone {
    match status {
        "approved" | "locked" | "done" | "passed" => Tone::Ok,
        "blocked" | "failed" | "rejected" => Tone::Danger,
        "in_review" | "review" | "active" => Tone::Info,
        "pending" | "queued" => Tone::Warn,
        _ => Tone::Neutral,
    }
}

/// A representative payload built from the real `darkrun-api` types.
fn sample_payload() -> ReviewCurrentPayload {
    ReviewCurrentPayload {
        run: "rate-limit-public-api".to_string(),
        station: Some("build".to_string()),
        phase: Some("audit".to_string()),
        units: vec![
            ReviewCurrentUnit {
                slug: "limiter-core".to_string(),
                title: "Token-bucket limiter".to_string(),
                status: "in_review".to_string(),
            },
            ReviewCurrentUnit {
                slug: "limiter-middleware".to_string(),
                title: "Axum middleware layer".to_string(),
                status: "pending".to_string(),
            },
            ReviewCurrentUnit {
                slug: "limiter-config".to_string(),
                title: "Per-route config parsing".to_string(),
                status: "approved".to_string(),
            },
        ],
        feedback_summary: FeedbackSummary {
            pending: 2,
            addressed: 1,
            closed: 4,
            rejected: 0,
        },
        stations: vec![
            ReviewCurrentStation {
                name: "frame".to_string(),
                status: "locked".to_string(),
                phase: None,
                iteration: Some(1),
                visits: Some(1),
            },
            ReviewCurrentStation {
                name: "build".to_string(),
                status: "active".to_string(),
                phase: Some("audit".to_string()),
                iteration: Some(2),
                visits: Some(1),
            },
        ],
    }
}

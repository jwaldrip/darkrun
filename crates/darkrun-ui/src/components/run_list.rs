//! [`RunList`] / [`RunCard`] — the run-browser surface.
//!
//! Given a `Vec` of [`RunCardData`] summaries (slug, title, factory, active
//! station, phase, status, station progress), render a dark-brand grid of cards.
//! Each card carries a mini [`StationPipeline`] for its active station and a
//! status badge; clicking a card fires `on_select(slug)` so the host (the
//! desktop app or the website) can open that run's live Review.
//!
//! The crate stays renderer-agnostic and `darkrun-core`-free: the host maps its
//! domain enums into the small [`RunCardData`] view-model + a [`Tone`] at the
//! boundary. The pure mapping [`run_status_tone`] lives here so the status →
//! badge-tone decision is unit-testable without a renderer.

use dioxus::prelude::*;

use crate::components::pipeline::{strip_for, StationPipeline};
use crate::components::primitives::{Badge, Card};
use crate::kinds::{Phase, Tone};
use crate::tokens;

/// The view-model for one run row, projected from a `darkrun-api` `RunSummary`
/// at the host boundary. Everything is plain data so the component derives
/// `PartialEq` and never needs the wire crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCardData {
    /// The run slug — the stable id the host opens on select.
    pub slug: String,
    /// Resolved display title (host falls back to the slug).
    pub title: String,
    /// The factory driving the run (e.g. `software`).
    pub factory: String,
    /// The station the run currently sits on.
    pub active_station: String,
    /// The active phase within that station, driving the pipeline strip.
    pub phase: Option<Phase>,
    /// Lifecycle status display string (e.g. `active`, `completed`).
    pub status: String,
    /// Stations that have reached completion.
    pub completed: u32,
    /// Total stations the run walks.
    pub total: u32,
}

/// Map a run's lifecycle status string onto a badge [`Tone`].
///
/// Mirrors `darkrun-core`'s `Status` snake_case wire strings. Unknown values
/// fall back to [`Tone::Neutral`] so a new status never blanks a badge.
pub fn run_status_tone(status: &str) -> Tone {
    match status {
        "completed" => Tone::Ok,
        "active" | "in_progress" => Tone::Info,
        "blocked" => Tone::Danger,
        "pending" => Tone::Warn,
        _ => Tone::Neutral,
    }
}

/// A grid of [`RunCard`]s. Clicking a card fires `on_select` with its slug.
///
/// When `runs` is empty an `empty` slot is rendered if provided, otherwise a
/// default muted message. The host supplies the empty state so it can tailor the
/// copy (e.g. "start `darkrun serve`").
#[component]
pub fn RunList(
    /// The run summaries to render.
    runs: Vec<RunCardData>,
    /// Fired with the slug of the clicked run.
    on_select: EventHandler<String>,
    /// Optional custom empty-state element shown when `runs` is empty.
    #[props(default)]
    empty: Option<Element>,
) -> Element {
    if runs.is_empty() {
        return match empty {
            Some(el) => el,
            None => rsx! {
                Card {
                    p { style: "color:var(--dr-text-muted);margin:0;", "No runs yet." }
                }
            },
        };
    }

    let grid = "display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
                gap:14px;";
    rsx! {
        div { class: "dr-run-list", style: "{grid}",
            for run in runs.iter() {
                RunCard { run: run.clone(), on_select: move |slug| on_select.call(slug) }
            }
        }
    }
}

/// A single run card: identity + factory/station meta, a mini station pipeline,
/// a progress chip, and a status badge. The whole card is a click target that
/// fires `on_select(slug)`.
#[component]
pub fn RunCard(run: RunCardData, on_select: EventHandler<String>) -> Element {
    let accent = run.phase.map(|p| p.hue().base.to_string());
    let tone = run_status_tone(&run.status);

    let header = "display:flex;align-items:flex-start;justify-content:space-between;\
                  gap:10px;margin-bottom:8px;";
    let title_style = format!(
        "font-family:{sans};font-size:14px;font-weight:700;color:{text};\
         overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0;",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let meta_style = format!(
        "display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:10px;\
         font-family:{mono};font-size:11px;color:{muted};",
        mono = tokens::FONT_MONO,
        muted = tokens::TEXT_MUTED,
    );
    let progress_style = format!(
        "font-family:{mono};font-size:11px;color:{faint};margin-top:8px;",
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );

    let slug = run.slug.clone();
    rsx! {
        div {
            class: "dr-run-card",
            style: "cursor:pointer;",
            "data-slug": "{run.slug}",
            role: "button",
            tabindex: "0",
            onclick: move |_| on_select.call(slug.clone()),
            Card { accent,
                div { style: "{header}",
                    span { style: "{title_style}", title: "{run.title}", "{run.title}" }
                    Badge { tone, filled: true, "{run.status}" }
                }
                div { style: "{meta_style}",
                    Badge { tone: Tone::Neutral, "{run.factory}" }
                    span { "station: {run.active_station}" }
                }
                StationPipeline { dots: strip_for(run.phase), size: 14.0 }
                div { style: "{progress_style}",
                    "stations {run.completed} / {run.total}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_tone_maps_known_statuses() {
        assert_eq!(run_status_tone("completed"), Tone::Ok);
        assert_eq!(run_status_tone("active"), Tone::Info);
        assert_eq!(run_status_tone("in_progress"), Tone::Info);
        assert_eq!(run_status_tone("blocked"), Tone::Danger);
        assert_eq!(run_status_tone("pending"), Tone::Warn);
    }

    #[test]
    fn status_tone_unknown_is_neutral() {
        assert_eq!(run_status_tone(""), Tone::Neutral);
        assert_eq!(run_status_tone("wat"), Tone::Neutral);
        // Case-sensitive: wire strings are lowercase snake_case.
        assert_eq!(run_status_tone("Active"), Tone::Neutral);
    }

    #[test]
    fn card_data_is_eq_and_clonable() {
        let a = RunCardData {
            slug: "alpha".into(),
            title: "Alpha".into(),
            factory: "software".into(),
            active_station: "frame".into(),
            phase: Some(Phase::Spec),
            status: "active".into(),
            completed: 2,
            total: 6,
        };
        assert_eq!(a.clone(), a);
    }
}

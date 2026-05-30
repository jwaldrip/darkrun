//! [`RunWalkthrough`] — the standout: a stepper that walks a Run through the
//! stations and the six phases.
//!
//! A cursor rides the station pipeline while the phase ring highlights the active
//! phase; each tick narrates what is happening, tied to the real phase-machine
//! semantics ("frame · spec → explore — gather context…"). It advances by Step
//! buttons, and is reduced-motion friendly by construction: the component never
//! animates on its own. Auto-play is the *caller's* job — the design-system crate
//! deliberately does not own a wall-clock timer (it stays renderer-agnostic and
//! dependency-light). A consumer that wants Play drives the `tick`/`on_tick`
//! controlled props from its own timer (the website has one; the desktop app
//! has tokio); reduced-motion consumers simply never start that timer.
//!
//! The component works in two modes:
//! - **uncontrolled** (default): it owns an internal tick signal; Step buttons
//!   move it. Good for a static, click-to-advance walkthrough.
//! - **controlled**: pass `tick` and `on_tick`; the component renders the given
//!   tick and reports requested moves, so a parent timer can auto-advance it.

use dioxus::prelude::*;

use crate::components::phase_machine::PhaseMachine;
use crate::components::primitives::{Badge, Button, ButtonVariant};
use crate::components::station_flow::StationFlow;
use crate::flow::{walkthrough_steps, FlowStation, WalkStep};
use crate::kinds::Tone;
use crate::tokens;

/// Drive a Run walkthrough across a pipeline of stations.
///
/// The tick list comes from [`walkthrough_steps`]: every station expands into its
/// six phases, with Manufacture further expanding into the Make / Challenge /
/// Resolve pass (8 ticks per station).
#[component]
pub fn RunWalkthrough(
    /// The stations to walk, in pipeline order.
    stations: Vec<FlowStation>,
    /// Controlled tick. When `Some`, the component renders this tick and does not
    /// keep its own; pair with `on_tick` to advance from a parent timer.
    #[props(default)]
    tick: Option<usize>,
    /// Reports a requested tick (from a Step/reset button). Set this with `tick`
    /// to run in controlled mode.
    #[props(default)]
    on_tick: Option<EventHandler<usize>>,
    /// Initial tick for uncontrolled mode (clamped into range).
    #[props(default = 0)]
    start: usize,
) -> Element {
    let slugs: Vec<String> = stations.iter().map(|s| s.slug.clone()).collect();
    let steps: Vec<WalkStep> = walkthrough_steps(&slugs);
    let total = steps.len();

    if total == 0 {
        return rsx! {
            div {
                style: format!(
                    "font-family:{};color:{};padding:16px;",
                    tokens::FONT_MONO, tokens::TEXT_MUTED,
                ),
                "No stations to walk."
            }
        };
    }

    let controlled = tick.is_some();
    let internal = use_signal(|| start.min(total - 1));
    let at = match tick {
        Some(t) => t.min(total - 1),
        None => internal().min(total - 1),
    };

    // A single place to request a move: report it (controlled) and/or update the
    // internal signal (uncontrolled). Clamped into range. `Copy` so every button
    // handler can capture it independently (signals + EventHandler are `Copy`).
    let go = move |target: usize| {
        let clamped = target.min(total - 1);
        if let Some(h) = &on_tick {
            h.call(clamped);
        }
        if !controlled {
            let mut sig = internal;
            sig.set(clamped);
        }
    };

    let cur = &steps[at];
    let active_station = cur.station_index;
    let active_phase = cur.phase;
    let active_beat = cur.beat;
    let narration = cur.narration();

    let panel = format!(
        "display:flex;flex-direction:column;gap:14px;font-family:{sans};",
        sans = tokens::FONT_SANS,
    );
    let stage = "display:flex;gap:16px;flex-wrap:wrap;align-items:flex-start;";
    let narration_style = format!(
        "font-family:{mono};font-size:13px;color:{text};background:{surface};\
         border:1px solid {border};border-left:3px solid {accent};border-radius:8px;\
         padding:10px 14px;transition:opacity .2s ease;",
        mono = tokens::FONT_MONO,
        text = tokens::TEXT,
        surface = tokens::SURFACE_RAISED,
        border = tokens::BORDER,
        accent = active_phase.hue().base,
    );
    let controls = "display:flex;align-items:center;gap:8px;flex-wrap:wrap;";
    let counter_style = format!(
        "font-family:{mono};font-size:12px;color:{faint};margin-left:auto;",
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );

    rsx! {
        div { class: "dr-run-walkthrough", "data-tick": "{at}", "data-controlled": "{controlled}", style: "{panel}",
            // The two synchronized views: the pipeline and the phase ring.
            div { style: "{stage}",
                div { style: "flex:1;min-width:280px;",
                    StationFlow { stations: stations.clone(), active: Some(active_station) }
                }
                div {
                    PhaseMachine { active: Some(active_phase), active_beat, size: 280.0 }
                }
            }

            // Narration tied to the real phase-machine semantics. aria-live so a
            // screen reader announces each tick without motion.
            div {
                class: "dr-walk-narration",
                "aria-live": "polite",
                style: "{narration_style}",
                span { style: format!("color:{};", active_phase.hue().base), "{tokens::GLYPH_ACTIVE} " }
                "{narration}"
            }

            // Controls — step-driven, reduced-motion friendly.
            div { style: "{controls}",
                Button {
                    variant: ButtonVariant::Secondary,
                    disabled: at == 0,
                    on_click: move |_| if at > 0 { go(at - 1) },
                    "◀ step"
                }
                Button {
                    variant: ButtonVariant::Secondary,
                    disabled: at >= total - 1,
                    on_click: move |_| if at < total - 1 { go(at + 1) },
                    "step ▶"
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    tone: Tone::Neutral,
                    on_click: move |_| go(0),
                    "reset"
                }
                Badge { tone: Tone::Neutral, "{active_phase.name()}" }
                span { style: "{counter_style}", "tick {at + 1} / {total}" }
            }
        }
    }
}

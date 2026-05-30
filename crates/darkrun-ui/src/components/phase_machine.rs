//! [`PhaseMachine`] — an SVG ring of the within-station phase loop.
//!
//! The six phases (spec → review → manufacture → audit → reflect → checkpoint)
//! sit evenly around a ring, each in its own hue and labeled with the
//! universal-slot beat it performs. Every phase expands into named sub-steps
//! ([`crate::flow::phase_beats`]); the active phase's sub-step strip follows the
//! active node, and the center caption names the active phase and its beats. An
//! optional `active` phase emphasizes one node; hover surfaces the beat text.
//! Geometry comes from the pure [`crate::flow::phase_ring_points`].

use dioxus::prelude::*;

use crate::flow::{phase_beats, phase_label, phase_ring_points, PassBeat};
use crate::kinds::Phase;
use crate::tokens;

/// Render the within-station phase machine as an SVG ring.
#[component]
pub fn PhaseMachine(
    /// The phase currently emphasized, if any.
    #[props(default)]
    active: Option<Phase>,
    /// Overall SVG size (square), in pixels.
    #[props(default = 320.0)]
    size: f64,
    /// When set, the active Pass beat is highlighted inside the Manufacture node.
    #[props(default)]
    active_beat: Option<PassBeat>,
    /// Stretch the SVG to fill its container width (aspect preserved) instead of
    /// capping at its natural size.
    #[props(default = false)]
    full_width: bool,
) -> Element {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let ring_r = size * 0.34;
    let node_r = size * 0.085;
    let points = phase_ring_points(cx, cy, ring_r);

    // The phase labels sit OUTSIDE the ring and the Make → Challenge → Resolve
    // strip extends to the right of the Manufacture node, so a square `0 0 size
    // size` box clips them on both edges. Pad the viewBox: enough each side for
    // the longest label ("Checkpoint"/"Manufacture"), with extra on the right
    // for the strip. width/height grow with it; `max-width:100%` keeps it fitting
    // the card.
    // Symmetric horizontal padding so the ring stays centered (cx = size/2 is the
    // viewBox center); generous enough for the longest labels AND the right-side
    // make/challenge/resolve strip.
    let pad_x = size * 0.52;
    let pad_y = size * 0.13;
    let vb_w = size + pad_x * 2.0;
    let vb_h = size + pad_y * 2.0;
    let view_box = format!("{:.1} {:.1} {:.1} {:.1}", -pad_x, -pad_y, vb_w, vb_h);
    // `width:100%` fills the container (content stays centered via the viewBox's
    // default preserveAspectRatio); `max-width:100%` keeps natural size otherwise.
    let width_rule = if full_width { "width:100%" } else { "max-width:100%" };
    let svg_style = format!(
        "background:{surface};border:1px solid {border};border-radius:10px;\
         display:block;{width_rule};height:auto;font-family:{mono};margin:0 auto;",
        surface = tokens::SURFACE_RAISED,
        border = tokens::BORDER,
        mono = tokens::FONT_MONO,
    );

    // Pre-compute the polyline that connects the nodes in a closed loop.
    let loop_path = {
        let mut d = String::new();
        for (i, (_, x, y)) in points.iter().enumerate() {
            d.push_str(if i == 0 { "M" } else { "L" });
            d.push_str(&format!(" {x:.2} {y:.2} "));
        }
        d.push('Z');
        d
    };

    rsx! {
        svg {
            class: "dr-phase-machine",
            width: "{vb_w}", height: "{vb_h}",
            view_box: "{view_box}",
            xmlns: "http://www.w3.org/2000/svg",
            style: "{svg_style}",
            role: "img",
            "aria-label": "within-station phase machine",

            defs {
                marker {
                    id: "dr-phase-arrow",
                    view_box: "0 0 10 10",
                    ref_x: "5", ref_y: "5",
                    marker_width: "6", marker_height: "6",
                    orient: "auto",
                    path { d: "M0,0 L10,5 L0,10 z", fill: tokens::BORDER_STRONG }
                }
            }

            // The loop the phases ride. Dashed to read as a cycle, not a chain.
            path {
                d: "{loop_path}",
                fill: "none",
                stroke: tokens::BORDER_STRONG,
                stroke_width: "1.5",
                stroke_dasharray: "4 5",
            }

            // Center caption: names the machine, and — when a phase is active —
            // that phase plus the beats it walks (so the caption follows the
            // active node for all six phases, not just Manufacture).
            {
                let (title, subtitle) = match active {
                    Some(phase) => {
                        let beats = phase_beats(phase)
                            .iter()
                            .map(|b| b.label())
                            .collect::<Vec<_>>()
                            .join(" → ");
                        (phase_label(phase).to_uppercase(), beats)
                    }
                    None => ("PHASE MACHINE".to_string(), "spec → … → checkpoint".to_string()),
                };
                rsx! {
                    text {
                        x: "{cx}", y: "{cy - 6.0}",
                        fill: tokens::TEXT_MUTED, font_size: "11",
                        text_anchor: "middle", letter_spacing: "0.08em",
                        "{title}"
                    }
                    text {
                        x: "{cx}", y: "{cy + 12.0}",
                        fill: tokens::TEXT_FAINT, font_size: "9",
                        text_anchor: "middle",
                        "{subtitle}"
                    }
                }
            }

            // Phase nodes.
            for (phase, x, y) in points.iter() {
                {
                    let phase = *phase;
                    let hue = phase.hue();
                    let is_active = active == Some(phase);
                    let fill = if is_active { hue.base } else { tokens::SURFACE_OVERLAY };
                    let glyph_color = if is_active { hue.on } else { hue.base };
                    let stroke_w = if is_active { "3" } else { "1.5" };
                    // label sits just outside the node, pushed away from center
                    let dx = x - cx;
                    let dy = y - cy;
                    let len = (dx * dx + dy * dy).sqrt().max(1.0);
                    let lx = x + dx / len * (node_r + 10.0);
                    let ly = y + dy / len * (node_r + 10.0);
                    let anchor = if dx.abs() < 1.0 { "middle" } else if dx > 0.0 { "start" } else { "end" };
                    let is_manufacture = phase == Phase::Manufacture;
                    // Tooltip: the phase's named sub-step beats, in order.
                    let beat_title = phase_beats(phase)
                        .iter()
                        .map(|b| b.label())
                        .collect::<Vec<_>>()
                        .join(" → ");
                    rsx! {
                        g {
                            class: "dr-phase-node",
                            "data-phase": phase.name(),
                            "data-active": "{is_active}",
                            title { "{beat_title}" }
                            circle {
                                cx: "{x}", cy: "{y}", r: "{node_r}",
                                fill: "{fill}", stroke: "{hue.base}", stroke_width: "{stroke_w}",
                            }
                            text {
                                x: "{x}", y: "{y + 4.0}",
                                fill: "{glyph_color}", font_size: "13", font_weight: "700",
                                text_anchor: "middle",
                                if is_manufacture { "⚙" } else { "{tokens::GLYPH_ACTIVE}" }
                            }
                            text {
                                x: "{lx}", y: "{ly}",
                                fill: if is_active { tokens::TEXT } else { tokens::TEXT_MUTED },
                                font_size: "11", font_weight: if is_active { "700" } else { "500" },
                                text_anchor: "{anchor}",
                                "{phase_label(phase)}"
                            }
                        }
                    }
                }
            }

            // The active phase's sub-step strip. It follows the active node:
            // the named beats of whatever phase is active are listed beside it,
            // with the live beat highlighted. With no active phase it falls back
            // to the Manufacture node's Make/Challenge/Resolve pass so the ring
            // still reads as a worker loop at rest.
            {
                // Which phase's beats to show, and where its node sits.
                let strip_phase = active.unwrap_or(Phase::Manufacture);
                let node = points.iter().find(|(p, _, _)| *p == strip_phase);
                if let Some((_, nx, ny)) = node {
                    let beats = phase_beats(strip_phase);
                    let n = beats.len() as f64;
                    let hue = strip_phase.hue();
                    // Lean the strip to whichever side the node is on so it never
                    // crosses the center caption; anchor the text to match. Nodes
                    // dead-center-top/bottom default to the right.
                    let on_right = *nx >= cx - 1.0;
                    let strip_x =
                        if on_right { *nx + node_r + 6.0 } else { *nx - node_r - 6.0 };
                    let anchor = if on_right { "start" } else { "end" };
                    // Vertically center the strip on the node.
                    let strip_top = *ny - (n - 1.0) * 6.5;
                    rsx! {
                        g {
                            class: "dr-beat-strip",
                            "data-phase": strip_phase.name(),
                            for (i, beat) in beats.iter().enumerate() {
                                {
                                    let by = strip_top + i as f64 * 13.0;
                                    // The live beat: for Manufacture honor the
                                    // typed `active_beat`; otherwise highlight all
                                    // beats of the active phase in its hue.
                                    let on = match active_beat {
                                        Some(ab) if strip_phase == Phase::Manufacture => {
                                            *beat == ab.as_beat()
                                        }
                                        _ => active == Some(strip_phase),
                                    };
                                    let color = if on { hue.base } else { tokens::TEXT_FAINT };
                                    let weight = if on { "700" } else { "500" };
                                    rsx! {
                                        text {
                                            x: "{strip_x}", y: "{by}",
                                            fill: "{color}", font_size: "9", font_weight: "{weight}",
                                            text_anchor: "{anchor}",
                                            "data-beat": beat.label(),
                                            "{tokens::GLYPH_DONE} {beat.label()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    rsx! {}
                }
            }
        }
    }
}

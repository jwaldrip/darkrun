//! [`StationFlow`] — an interactive SVG of a factory's station pipeline.
//!
//! Each station is a phase-hued node carrying its progress glyph (● ◉ ○) and a
//! checkpoint mark; adjacent nodes are joined by a connector that lights up once
//! its upstream station is done. Hover raises a node, click navigates. The whole
//! thing is positioned by the pure [`crate::flow::layout_flow`] pass, so it draws
//! identically on native (WebView) and wasm (browser) with no JS.

use dioxus::prelude::*;

use crate::components::factory::CheckpointKind;
use crate::flow::{layout_flow, FlowOptions, FlowStation};
use crate::kinds::Step;
use crate::tokens;

/// A short mark for the checkpoint kind, drawn under the node.
fn checkpoint_mark(kind: CheckpointKind) -> &'static str {
    match kind {
        CheckpointKind::Auto => "auto",
        CheckpointKind::Ask => "ask",
        CheckpointKind::External => "ext",
        CheckpointKind::Await => "await",
    }
}

/// Render a factory's station pipeline as interactive SVG.
#[component]
pub fn StationFlow(
    /// The stations, in pipeline order.
    stations: Vec<FlowStation>,
    /// The currently-active station index (drives done/active/pending). `None`
    /// leaves every station pending.
    #[props(default)]
    active: Option<usize>,
    /// Fired with the station slug when a node is clicked.
    #[props(default)]
    on_select: Option<EventHandler<String>>,
    /// Stretch the SVG to fill its container width (content stays centered via the
    /// viewBox) instead of capping at natural size.
    #[props(default = false)]
    full_width: bool,
) -> Element {
    let opts = FlowOptions::default();
    let layout = layout_flow(&stations, active, &opts);

    let mut hovered = use_signal(|| Option::<usize>::None);

    let view_box = format!("0 0 {} {}", layout.width, layout.height);
    let width_rule = if full_width { "width:100%" } else { "max-width:100%" };
    let svg_style = format!(
        "background:{surface};border:1px solid {border};border-radius:10px;\
         display:block;{width_rule};height:auto;font-family:{mono};margin:0 auto;",
        surface = tokens::SURFACE_RAISED,
        border = tokens::BORDER,
        mono = tokens::FONT_MONO,
    );

    rsx! {
        svg {
            class: "dr-station-flow",
            width: "{layout.width}",
            height: "{layout.height}",
            view_box: "{view_box}",
            xmlns: "http://www.w3.org/2000/svg",
            style: "{svg_style}",
            role: "img",
            "aria-label": "factory station pipeline",

            // Connectors first so nodes paint over them.
            for conn in layout.connectors.iter() {
                {
                    let stroke = if conn.flowed { tokens::ACCENT } else { tokens::BORDER_STRONG };
                    let w = if conn.flowed { "2.5" } else { "1.5" };
                    rsx! {
                        line {
                            x1: "{conn.x1}", y1: "{conn.y}",
                            x2: "{conn.x2}", y2: "{conn.y}",
                            stroke: "{stroke}", stroke_width: "{w}",
                            stroke_linecap: "round",
                        }
                    }
                }
            }

            // Station nodes.
            for s in layout.stations.iter() {
                {
                    let is_hover = hovered() == Some(s.index);
                    let dim = matches!(s.step, Step::Pending);
                    let ring = if dim && !is_hover { tokens::BORDER_STRONG } else { s.hue.base };
                    let fill = if matches!(s.step, Step::Active) {
                        s.hue.base
                    } else {
                        tokens::SURFACE_OVERLAY
                    };
                    let glyph_color = if matches!(s.step, Step::Active) {
                        s.hue.on
                    } else if dim {
                        tokens::TEXT_FAINT
                    } else {
                        s.hue.base
                    };
                    let r = if is_hover { s.r + 3.0 } else { s.r };
                    let stroke_w = if is_hover || matches!(s.step, Step::Active) { "2.5" } else { "1.5" };
                    let label_color = if dim { tokens::TEXT_FAINT } else { tokens::TEXT };
                    let label_y = s.cy + s.r + 16.0;
                    let mark_y = s.cy + s.r + 28.0;
                    let idx = s.index;
                    let slug_click = s.slug.clone();
                    let on_select = on_select;
                    rsx! {
                        g {
                            class: "dr-flow-node",
                            "data-slug": "{s.slug}",
                            "data-step": match s.step {
                                Step::Done => "done",
                                Step::Active => "active",
                                Step::Pending => "pending",
                            },
                            style: "cursor:pointer;",
                            onmouseenter: move |_| hovered.set(Some(idx)),
                            onmouseleave: move |_| hovered.set(None),
                            onclick: move |_| {
                                if let Some(h) = &on_select {
                                    h.call(slug_click.clone());
                                }
                            },
                            title { "{s.label}" }
                            circle {
                                cx: "{s.cx}", cy: "{s.cy}", r: "{r}",
                                fill: "{fill}", stroke: "{ring}", stroke_width: "{stroke_w}",
                            }
                            text {
                                x: "{s.cx}", y: "{s.cy + 5.0}",
                                fill: "{glyph_color}", font_size: "18",
                                text_anchor: "middle",
                                "{s.glyph}"
                            }
                            text {
                                x: "{s.cx}", y: "{label_y}",
                                fill: "{label_color}", font_size: "12", font_weight: "600",
                                text_anchor: "middle",
                                "{s.label}"
                            }
                            text {
                                x: "{s.cx}", y: "{mark_y}",
                                fill: tokens::TEXT_FAINT, font_size: "9",
                                text_anchor: "middle",
                                "◇ {checkpoint_mark(s.checkpoint)}"
                            }
                        }
                    }
                }
            }
        }
    }
}

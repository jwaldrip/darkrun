//! [`UnitGraph`] — an SVG rendering of a unit dependency DAG.
//!
//! The component is pure presentation: it takes nodes + edges, runs them through
//! a [`GraphLayout`] (default [`LayeredLayout`]), and emits inline SVG. No JS
//! graph library, no canvas — just positioned `<rect>`/`<path>` elements that
//! render identically on native (WebView) and wasm (browser).

use dioxus::prelude::*;

use crate::graph::layout::{
    GraphEdge, GraphLayout, GraphNode, LayeredLayout, LayoutOptions,
};
use crate::kinds::Tone;
use crate::tokens;

/// A node plus an optional tone, so callers can color a unit by status.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitGraphNode {
    /// The underlying graph node.
    pub node: GraphNode,
    /// Optional status tone for the node outline/label.
    pub tone: Option<Tone>,
}

impl UnitGraphNode {
    /// Construct an untinted node.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { node: GraphNode::new(id, label), tone: None }
    }

    /// Set the node's status tone.
    pub fn with_tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }
}

/// Render a unit dependency graph as SVG using the layered layout.
#[component]
pub fn UnitGraph(
    /// The units to place.
    units: Vec<UnitGraphNode>,
    /// Dependency edges (`from` depends-on -> `to` dependent).
    edges: Vec<GraphEdge>,
) -> Element {
    let opts = LayoutOptions::default();
    let nodes: Vec<GraphNode> = units.iter().map(|u| u.node.clone()).collect();
    let result = LayeredLayout.layout(&nodes, &edges, &opts);

    let tone_for = |id: &str| {
        units
            .iter()
            .find(|u| u.node.id == id)
            .and_then(|u| u.tone)
            .map(|t| t.color_var())
            .unwrap_or(tokens::var::ACCENT)
    };

    let view_box = format!("0 0 {} {}", result.width, result.height);
    // Crisp 1:1 rendering: the drawing never UPSCALES (billboarded text), only
    // shrinks when wider than its container. The graph panel itself spans the
    // full pane width; a small graph simply sits left within it.
    let svg_style = format!(
        "background:{surface};border:1px solid {border};border-radius:8px;\
         display:block;max-width:100%;height:auto;font-family:{mono};",
        surface = tokens::var::SURFACE_RAISED,
        border = tokens::var::BORDER,
        mono = tokens::FONT_MONO,
    );

    rsx! {
        svg {
            class: "dr-unit-graph",
            width: "{result.width}",
            height: "{result.height}",
            view_box: "{view_box}",
            preserve_aspect_ratio: "xMinYMid meet",
            xmlns: "http://www.w3.org/2000/svg",
            style: "{svg_style}",
            role: "img",
            "aria-label": "unit dependency graph",

            // Arrowhead marker, accent-tinted.
            defs {
                marker {
                    id: "dr-arrow",
                    view_box: "0 0 10 10",
                    ref_x: "9",
                    ref_y: "5",
                    marker_width: "7",
                    marker_height: "7",
                    orient: "auto-start-reverse",
                    path { d: "M0,0 L10,5 L0,10 z", fill: tokens::var::BORDER_STRONG }
                }
            }

            // Edges first so nodes paint on top.
            for edge in result.edges.iter() {
                {
                    // A simple cubic with horizontal control handles reads as a
                    // left-to-right flow without a routing library.
                    let dx = (edge.x2 - edge.x1).abs().max(24.0) * 0.5;
                    let d = format!(
                        "M {x1} {y1} C {c1x} {y1}, {c2x} {y2}, {x2} {y2}",
                        x1 = edge.x1,
                        y1 = edge.y1,
                        c1x = edge.x1 + dx,
                        c2x = edge.x2 - dx,
                        x2 = edge.x2,
                        y2 = edge.y2,
                    );
                    rsx! {
                        path {
                            d: "{d}",
                            fill: "none",
                            stroke: tokens::var::BORDER_STRONG,
                            stroke_width: "1.5",
                            marker_end: "url(#dr-arrow)",
                        }
                    }
                }
            }

            // Nodes.
            for node in result.nodes.iter() {
                {
                    let color = tone_for(&node.id);
                    let label_y = node.cy() + 4.0;
                    rsx! {
                        g { class: "dr-graph-node", "data-id": "{node.id}",
                            rect {
                                x: "{node.x}",
                                y: "{node.y}",
                                width: "{node.width}",
                                height: "{node.height}",
                                rx: "6",
                                fill: tokens::var::SURFACE_OVERLAY,
                                stroke: "{color}",
                                stroke_width: "1.5",
                            }
                            text {
                                x: "{node.cx()}",
                                y: "{label_y}",
                                fill: tokens::var::TEXT,
                                font_size: "12",
                                text_anchor: "middle",
                                "{node.label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

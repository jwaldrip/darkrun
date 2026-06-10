//! Layered DAG layout — a small, dependency-free Sugiyama-ish placement.
//!
//! The layout is pure (no Dioxus, no rendering) so it is trivially testable and
//! can be swapped: it sits behind the [`GraphLayout`] trait. The default
//! [`LayeredLayout`] assigns each node a layer by longest-path from a root,
//! orders nodes within a layer, and emits absolute pixel positions plus routed
//! edges.

use std::collections::{BTreeMap, HashMap, HashSet};

/// An input node: a stable id plus a display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Stable identifier (e.g. a unit slug). Referenced by edges.
    pub id: String,
    /// Human label rendered inside the node.
    pub label: String,
}

impl GraphNode {
    /// Construct a node from id and label.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into() }
    }
}

/// A directed dependency edge `from -> to` (the `to` node depends on `from`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// Source node id (the dependency).
    pub from: String,
    /// Target node id (the dependent).
    pub to: String,
}

impl GraphEdge {
    /// Construct an edge.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self { from: from.into(), to: to.into() }
    }
}

/// Tunable geometry for a layout pass.
#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    /// Node box width in pixels.
    pub node_width: f64,
    /// Node box height in pixels.
    pub node_height: f64,
    /// Horizontal gap between layers.
    pub layer_gap: f64,
    /// Vertical gap between nodes in a layer.
    pub node_gap: f64,
    /// Outer padding around the whole diagram.
    pub padding: f64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            node_width: 132.0,
            node_height: 40.0,
            layer_gap: 56.0,
            node_gap: 18.0,
            padding: 16.0,
        }
    }
}

/// A placed node with absolute top-left pixel coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedNode {
    /// The source node id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Layer index (0 = roots).
    pub layer: usize,
    /// Top-left x.
    pub x: f64,
    /// Top-left y.
    pub y: f64,
    /// Box width.
    pub width: f64,
    /// Box height.
    pub height: f64,
}

impl PlacedNode {
    /// Center x of the node box.
    pub fn cx(&self) -> f64 {
        self.x + self.width / 2.0
    }
    /// Center y of the node box.
    pub fn cy(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

/// A routed edge between two placed node centers.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Start point x (source right edge).
    pub x1: f64,
    /// Start point y.
    pub y1: f64,
    /// End point x (target left edge).
    pub x2: f64,
    /// End point y.
    pub y2: f64,
}

/// The result of a layout pass: placed nodes, routed edges, and the canvas size.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutResult {
    /// Placed nodes in input order.
    pub nodes: Vec<PlacedNode>,
    /// Routed edges (only those whose endpoints both resolved).
    pub edges: Vec<PlacedEdge>,
    /// Total canvas width.
    pub width: f64,
    /// Total canvas height.
    pub height: f64,
}

/// A pluggable graph layout strategy. The SVG renderer depends only on this
/// trait, so the layout can grow (e.g. crossing minimization) without touching
/// the view.
pub trait GraphLayout {
    /// Place `nodes` connected by `edges` into pixel space.
    fn layout(
        &self,
        nodes: &[GraphNode],
        edges: &[GraphEdge],
        opts: &LayoutOptions,
    ) -> LayoutResult;
}

/// The default layered layout: longest-path layering, stable within-layer order.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayeredLayout;

impl LayeredLayout {
    /// Assign each node a layer = the longest dependency path reaching it.
    ///
    /// Cycles (which a well-formed unit DAG should not contain) are tolerated:
    /// any node not resolved by the relaxation passes is pinned to layer 0 so
    /// the function always terminates and never panics.
    fn assign_layers(
        nodes: &[GraphNode],
        edges: &[GraphEdge],
    ) -> HashMap<String, usize> {
        let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        // Edges that reference unknown ids are ignored.
        let valid: Vec<&GraphEdge> = edges
            .iter()
            .filter(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()))
            .collect();

        let mut layer: HashMap<String, usize> =
            nodes.iter().map(|n| (n.id.clone(), 0usize)).collect();

        // Relax: layer(to) = max(layer(to), layer(from)+1). At most |nodes|
        // passes are needed for a DAG; the bound also stops any accidental cycle.
        let max_passes = nodes.len();
        for _ in 0..max_passes {
            let mut changed = false;
            for e in &valid {
                let from_layer = layer[&e.from];
                let want = from_layer + 1;
                let entry = layer.get_mut(&e.to).expect("node present");
                if *entry < want {
                    *entry = want;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        layer
    }
}

impl GraphLayout for LayeredLayout {
    fn layout(
        &self,
        nodes: &[GraphNode],
        edges: &[GraphEdge],
        opts: &LayoutOptions,
    ) -> LayoutResult {
        if nodes.is_empty() {
            return LayoutResult {
                width: opts.padding * 2.0,
                height: opts.padding * 2.0,
                ..Default::default()
            };
        }

        let layers = Self::assign_layers(nodes, edges);

        // Group node ids by layer, preserving input order within each layer.
        let mut by_layer: BTreeMap<usize, Vec<&GraphNode>> = BTreeMap::new();
        for n in nodes {
            by_layer.entry(layers[&n.id]).or_default().push(n);
        }

        // Label-aware node sizing: a node is as wide as its (monospace) label
        // needs, floored at `opts.node_width` — long labels never overflow
        // their box. ~7.2px/char at the renderer's 12px mono + side padding.
        let node_w = |n: &GraphNode| -> f64 {
            (n.label.chars().count() as f64 * 7.2 + 24.0).max(opts.node_width)
        };
        // Each layer's column is as wide as its widest node.
        let layer_count = by_layer.keys().next_back().map(|m| m + 1).unwrap_or(1);
        let mut col_w: Vec<f64> = vec![opts.node_width; layer_count];
        for (&layer_idx, layer_nodes) in &by_layer {
            for node in layer_nodes {
                col_w[layer_idx] = col_w[layer_idx].max(node_w(node));
            }
        }
        // Column x origins: padding + the accumulated widths + gaps before it.
        let mut col_x: Vec<f64> = Vec::with_capacity(layer_count);
        let mut acc = opts.padding;
        for w in &col_w {
            col_x.push(acc);
            acc += w + opts.layer_gap;
        }

        // Place: layers march left->right (x by column), nodes stack top->bottom.
        let mut placed: Vec<PlacedNode> = Vec::with_capacity(nodes.len());
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut max_y: f64 = 0.0;
        for (&layer_idx, layer_nodes) in &by_layer {
            let x = col_x[layer_idx];
            for (row, node) in layer_nodes.iter().enumerate() {
                let y = opts.padding
                    + row as f64 * (opts.node_height + opts.node_gap);
                index.insert(node.id.clone(), placed.len());
                placed.push(PlacedNode {
                    id: node.id.clone(),
                    label: node.label.clone(),
                    layer: layer_idx,
                    x,
                    y,
                    width: node_w(node),
                    height: opts.node_height,
                });
                max_y = max_y.max(y + opts.node_height);
            }
        }

        let width = opts.padding * 2.0
            + col_w.iter().sum::<f64>()
            + (layer_count.saturating_sub(1)) as f64 * opts.layer_gap;
        let height = max_y + opts.padding;

        // Route edges from source right edge to target left edge.
        let mut routed: Vec<PlacedEdge> = Vec::new();
        for e in edges {
            let (Some(&fi), Some(&ti)) =
                (index.get(&e.from), index.get(&e.to))
            else {
                continue;
            };
            let f = &placed[fi];
            let t = &placed[ti];
            routed.push(PlacedEdge {
                from: e.from.clone(),
                to: e.to.clone(),
                x1: f.x + f.width,
                y1: f.cy(),
                x2: t.x,
                y2: t.cy(),
            });
        }

        LayoutResult { nodes: placed, edges: routed, width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str) -> GraphNode {
        GraphNode::new(id, id)
    }

    #[test]
    fn empty_graph_has_padding_only_canvas() {
        let r = LayeredLayout.layout(&[], &[], &LayoutOptions::default());
        assert!(r.nodes.is_empty());
        assert!(r.edges.is_empty());
        assert_eq!(r.width, 32.0);
    }

    #[test]
    fn chain_increments_layers() {
        let nodes = vec![n("a"), n("b"), n("c")];
        let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "c")];
        let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
        let layer = |id: &str| r.nodes.iter().find(|p| p.id == id).unwrap().layer;
        assert_eq!(layer("a"), 0);
        assert_eq!(layer("b"), 1);
        assert_eq!(layer("c"), 2);
        // x strictly increases with layer.
        let xa = r.nodes.iter().find(|p| p.id == "a").unwrap().x;
        let xc = r.nodes.iter().find(|p| p.id == "c").unwrap().x;
        assert!(xc > xa);
    }

    #[test]
    fn diamond_uses_longest_path() {
        // a -> b -> d, a -> c -> d, plus a -> d directly. d must sit past b/c.
        let nodes = vec![n("a"), n("b"), n("c"), n("d")];
        let edges = vec![
            GraphEdge::new("a", "b"),
            GraphEdge::new("a", "c"),
            GraphEdge::new("b", "d"),
            GraphEdge::new("c", "d"),
            GraphEdge::new("a", "d"),
        ];
        let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
        let layer = |id: &str| r.nodes.iter().find(|p| p.id == id).unwrap().layer;
        assert_eq!(layer("a"), 0);
        assert_eq!(layer("b"), 1);
        assert_eq!(layer("c"), 1);
        assert_eq!(layer("d"), 2);
        assert_eq!(r.edges.len(), 5);
    }

    #[test]
    fn unknown_edge_endpoints_are_dropped() {
        let nodes = vec![n("a")];
        let edges = vec![GraphEdge::new("a", "ghost")];
        let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
        assert_eq!(r.nodes.len(), 1);
        assert!(r.edges.is_empty());
    }

    #[test]
    fn cycle_terminates_without_panic() {
        let nodes = vec![n("a"), n("b")];
        let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "a")];
        let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
        assert_eq!(r.nodes.len(), 2);
    }
}

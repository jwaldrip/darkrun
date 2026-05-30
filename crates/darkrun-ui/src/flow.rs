//! Pure layout + semantics for the factory pipeline and the within-station phase
//! machine. No Dioxus, no rendering — just the math and the domain mapping the
//! [`crate::components::station_flow`], [`crate::components::phase_machine`], and
//! [`crate::components::walkthrough`] components draw. Keeping it here makes the
//! geometry and the step sequencing trivially testable on native, and lets the
//! SVG views stay thin.

use crate::components::factory::CheckpointKind;
use crate::kinds::{Phase, Step};
use crate::tokens::{self, Hue};

// ===========================================================================
// Station pipeline layout (StationFlow)
// ===========================================================================

/// One station in a factory pipeline as the flow view needs it: a stable slug,
/// a human label, the gate that ends it, and the risk class it eliminates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowStation {
    /// Stable slug (e.g. `build`). Used for navigation + `data-` hooks.
    pub slug: String,
    /// Display label (defaults to the slug, title-cased by the view).
    pub label: String,
    /// The checkpoint gate that closes this station.
    pub checkpoint: CheckpointKind,
    /// The class of risk this station is designed to eliminate, if known.
    pub risk: Option<String>,
}

impl FlowStation {
    /// Construct a station with slug == label and no risk note.
    pub fn new(slug: impl Into<String>, checkpoint: CheckpointKind) -> Self {
        let slug = slug.into();
        Self { label: slug.clone(), slug, checkpoint, risk: None }
    }

    /// Set the display label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set the eliminated risk class.
    pub fn with_risk(mut self, risk: impl Into<String>) -> Self {
        self.risk = Some(risk.into());
        self
    }
}

/// Tunable geometry for the horizontal station pipeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowOptions {
    /// Radius of each station node circle.
    pub node_radius: f64,
    /// Center-to-center horizontal spacing between stations.
    pub node_gap: f64,
    /// Outer padding around the diagram.
    pub padding: f64,
    /// Vertical room reserved under each node for its label.
    pub label_height: f64,
}

impl Default for FlowOptions {
    fn default() -> Self {
        Self { node_radius: 22.0, node_gap: 104.0, padding: 28.0, label_height: 34.0 }
    }
}

/// A placed station: its center, glyph, hue, and progress state.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedStation {
    /// Source slug.
    pub slug: String,
    /// Display label.
    pub label: String,
    /// Zero-based index in the pipeline.
    pub index: usize,
    /// Center x.
    pub cx: f64,
    /// Center y.
    pub cy: f64,
    /// Node radius.
    pub r: f64,
    /// Progress glyph (● done, ◉ active, ○ pending).
    pub glyph: char,
    /// Progress state.
    pub step: Step,
    /// The phase hue assigned to this station's slot (drives the node color).
    pub hue: Hue,
    /// The checkpoint kind that ends the station.
    pub checkpoint: CheckpointKind,
    /// The risk class this station eliminates, if known.
    pub risk: Option<String>,
}

/// A connector segment between two adjacent placed stations.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowConnector {
    /// Start x (right edge of the left node).
    pub x1: f64,
    /// Shared center y.
    pub y: f64,
    /// End x (left edge of the right node).
    pub x2: f64,
    /// True once the *upstream* station is done — the line reads as "flowed".
    pub flowed: bool,
}

/// The full placed pipeline: stations, connectors, and canvas size.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FlowLayout {
    /// Placed stations in pipeline order.
    pub stations: Vec<PlacedStation>,
    /// Connectors between adjacent stations (`stations.len().saturating_sub(1)`).
    pub connectors: Vec<FlowConnector>,
    /// Canvas width.
    pub width: f64,
    /// Canvas height.
    pub height: f64,
}

/// The phase hue a station owns, by its position in the pipeline.
///
/// Stations cycle the six phase hues so a long pipeline still reads as a colored
/// assembly line. The first six stations map 1:1 onto the canonical phase order
/// (`spec`-grey, `review`-blue, …), the seventh wraps back to grey, and so on.
pub fn station_hue(index: usize) -> Hue {
    Phase::ALL[index % Phase::ALL.len()].hue()
}

/// Lay the stations out left-to-right. `active` selects the current node: every
/// station before it is `Done`, it is `Active`, the rest `Pending`. `active =
/// None` leaves the whole pipeline pending.
pub fn layout_flow(
    stations: &[FlowStation],
    active: Option<usize>,
    opts: &FlowOptions,
) -> FlowLayout {
    if stations.is_empty() {
        return FlowLayout {
            width: opts.padding * 2.0,
            height: opts.padding * 2.0 + opts.node_radius * 2.0 + opts.label_height,
            ..Default::default()
        };
    }

    let cy = opts.padding + opts.node_radius;
    let placed: Vec<PlacedStation> = stations
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let step = match active {
                Some(a) if i < a => Step::Done,
                Some(a) if i == a => Step::Active,
                _ => Step::Pending,
            };
            PlacedStation {
                slug: s.slug.clone(),
                label: s.label.clone(),
                index: i,
                cx: opts.padding + opts.node_radius + i as f64 * opts.node_gap,
                cy,
                r: opts.node_radius,
                glyph: step.glyph(),
                step,
                hue: station_hue(i),
                checkpoint: s.checkpoint,
                risk: s.risk.clone(),
            }
        })
        .collect();

    let connectors: Vec<FlowConnector> = placed
        .windows(2)
        .map(|w| FlowConnector {
            x1: w[0].cx + w[0].r,
            y: cy,
            x2: w[1].cx - w[1].r,
            flowed: w[0].step == Step::Done,
        })
        .collect();

    let last = placed.last().expect("non-empty");
    let width = last.cx + opts.node_radius + opts.padding;
    let height = opts.padding * 2.0 + opts.node_radius * 2.0 + opts.label_height;

    FlowLayout { stations: placed, connectors, width, height }
}

// ===========================================================================
// Phase-machine semantics (PhaseMachine + walkthrough narration)
// ===========================================================================

/// The universal-slot beat each within-station phase performs. This is the
/// load-bearing mapping between the six phases and the methodology vocabulary:
/// Explore -> Decompose -> Pass-loop(Make/Challenge/Resolve) -> Review ->
/// Checkpoint -> Lock.
pub fn phase_beat(phase: Phase) -> &'static str {
    match phase {
        Phase::Spec => "explore — gather context, decompose into units",
        Phase::Review => "review the spec — challenge scope before any output",
        Phase::Manufacture => "manufacture — the Make / Challenge / Resolve pass loop",
        Phase::Audit => "audit the output against the spec",
        Phase::Tests => "prove — run the quality gates",
        Phase::Checkpoint => "checkpoint — fire the gate, then lock the artifact",
    }
}

/// The phase's display label — the phase name itself (distinct from station
/// names like "Specify"/"Prove" so the ring never collides with the pipeline).
pub fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Spec => "Spec",
        Phase::Review => "Review",
        Phase::Manufacture => "Manufacture",
        Phase::Audit => "Audit",
        Phase::Tests => "Tests",
        Phase::Checkpoint => "Checkpoint",
    }
}

/// The three beats of a Manufacture Pass, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassBeat {
    /// Make: produce the candidate output.
    Make,
    /// Challenge: attack it, find the weakness.
    Challenge,
    /// Resolve: fix what the challenge surfaced.
    Resolve,
}

impl PassBeat {
    /// All three beats in order.
    pub const ALL: [PassBeat; 3] = [PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve];

    /// The lowercase label.
    pub fn label(self) -> &'static str {
        match self {
            PassBeat::Make => "make",
            PassBeat::Challenge => "challenge",
            PassBeat::Resolve => "resolve",
        }
    }

    /// One-line description of what the beat does.
    pub fn beat(self) -> &'static str {
        match self {
            PassBeat::Make => "produce the candidate output",
            PassBeat::Challenge => "attack it — find the weakest seam",
            PassBeat::Resolve => "fix what the challenge surfaced",
        }
    }
}

/// Place the six phases evenly around a ring of the given radius, starting at
/// twelve o'clock and proceeding clockwise. Returns `(phase, x, y)` per phase.
///
/// `cx`/`cy` are the ring center; `r` the placement radius. The math is pure so
/// the ring view and its tests share one source of truth.
pub fn phase_ring_points(cx: f64, cy: f64, r: f64) -> Vec<(Phase, f64, f64)> {
    let n = Phase::ALL.len() as f64;
    Phase::ALL
        .into_iter()
        .enumerate()
        .map(|(i, phase)| {
            // -PI/2 puts phase 0 at the top; positive step goes clockwise.
            let theta = -std::f64::consts::FRAC_PI_2 + (i as f64 / n) * std::f64::consts::TAU;
            (phase, cx + r * theta.cos(), cy + r * theta.sin())
        })
        .collect()
}

// ===========================================================================
// Run walkthrough step sequencing
// ===========================================================================

/// One tick of a run walkthrough: a station, the phase within it, and — when the
/// phase is Manufacture — which Pass beat is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStep {
    /// Index of the station in the pipeline.
    pub station_index: usize,
    /// The station's slug.
    pub station_slug: String,
    /// The phase active at this tick.
    pub phase: Phase,
    /// The Pass beat, present only while `phase == Manufacture`.
    pub beat: Option<PassBeat>,
}

impl WalkStep {
    /// The narration line for this tick, e.g.
    /// `"build · manufacture → challenge: attack it — find the weakest seam"`.
    pub fn narration(&self) -> String {
        match self.beat {
            Some(b) => format!(
                "{station} · manufacture → {beat}: {desc}",
                station = self.station_slug,
                beat = b.label(),
                desc = b.beat(),
            ),
            None => format!(
                "{station} · {phase} → {beat}",
                station = self.station_slug,
                phase = self.phase.name(),
                beat = phase_beat(self.phase),
            ),
        }
    }
}

/// Build the full ordered list of walkthrough ticks for a pipeline of `stations`
/// slugs. Each station expands into its six phases; the Manufacture phase further
/// expands into the three Pass beats (Make/Challenge/Resolve), so each station is
/// `5 + 3 = 8` ticks and the whole run is `stations * 8`.
pub fn walkthrough_steps(stations: &[String]) -> Vec<WalkStep> {
    let mut steps = Vec::with_capacity(stations.len() * (Phase::ALL.len() + PassBeat::ALL.len() - 1));
    for (station_index, slug) in stations.iter().enumerate() {
        for phase in Phase::ALL {
            if phase == Phase::Manufacture {
                for beat in PassBeat::ALL {
                    steps.push(WalkStep {
                        station_index,
                        station_slug: slug.clone(),
                        phase,
                        beat: Some(beat),
                    });
                }
            } else {
                steps.push(WalkStep {
                    station_index,
                    station_slug: slug.clone(),
                    phase,
                    beat: None,
                });
            }
        }
    }
    steps
}

/// The number of ticks one station contributes to a walkthrough (8: five plain
/// phases plus the three-beat Manufacture pass).
pub const TICKS_PER_STATION: usize = Phase::ALL.len() + PassBeat::ALL.len() - 1;

/// Resolve a checkpoint kind to the hue used to tint its badge. Auto reads as
/// success-green; ask/await as caution; external as info.
pub fn checkpoint_hue(kind: CheckpointKind) -> &'static str {
    match kind {
        CheckpointKind::Auto => tokens::STATUS_OK,
        CheckpointKind::Ask => tokens::STATUS_WARN,
        CheckpointKind::Await => tokens::STATUS_WARN,
        CheckpointKind::External => tokens::STATUS_INFO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(slug: &str) -> FlowStation {
        FlowStation::new(slug, CheckpointKind::Auto)
    }

    #[test]
    fn empty_flow_has_padding_canvas() {
        let r = layout_flow(&[], None, &FlowOptions::default());
        assert!(r.stations.is_empty());
        assert!(r.connectors.is_empty());
        assert_eq!(r.width, 56.0);
    }

    #[test]
    fn stations_advance_by_node_gap() {
        let opts = FlowOptions::default();
        let r = layout_flow(&[st("a"), st("b"), st("c")], None, &opts);
        assert_eq!(r.stations.len(), 3);
        assert_eq!(r.stations[1].cx - r.stations[0].cx, opts.node_gap);
        assert_eq!(r.stations[2].cx - r.stations[1].cx, opts.node_gap);
        // all share a baseline
        assert_eq!(r.stations[0].cy, r.stations[2].cy);
    }

    #[test]
    fn connectors_join_adjacent_edges() {
        let opts = FlowOptions::default();
        let r = layout_flow(&[st("a"), st("b")], None, &opts);
        assert_eq!(r.connectors.len(), 1);
        let c = &r.connectors[0];
        assert_eq!(c.x1, r.stations[0].cx + r.stations[0].r);
        assert_eq!(c.x2, r.stations[1].cx - r.stations[1].r);
        assert!(c.x2 > c.x1);
    }

    #[test]
    fn active_marks_done_active_pending() {
        let r = layout_flow(&[st("a"), st("b"), st("c"), st("d")], Some(2), &FlowOptions::default());
        assert_eq!(r.stations[0].step, Step::Done);
        assert_eq!(r.stations[1].step, Step::Done);
        assert_eq!(r.stations[2].step, Step::Active);
        assert_eq!(r.stations[3].step, Step::Pending);
        // glyphs follow the step
        assert_eq!(r.stations[2].glyph, tokens::GLYPH_ACTIVE);
        assert_eq!(r.stations[0].glyph, tokens::GLYPH_DONE);
        assert_eq!(r.stations[3].glyph, tokens::GLYPH_PENDING);
    }

    #[test]
    fn connector_flowed_tracks_upstream_done() {
        let r = layout_flow(&[st("a"), st("b"), st("c")], Some(1), &FlowOptions::default());
        // a is done -> a->b connector flowed; b is active -> b->c not yet.
        assert!(r.connectors[0].flowed);
        assert!(!r.connectors[1].flowed);
    }

    #[test]
    fn station_hue_cycles_phase_hues() {
        assert_eq!(station_hue(0), Phase::Spec.hue());
        assert_eq!(station_hue(5), Phase::Checkpoint.hue());
        assert_eq!(station_hue(6), Phase::Spec.hue()); // wraps
        assert_eq!(station_hue(7), Phase::Review.hue());
    }

    #[test]
    fn none_active_leaves_all_pending() {
        let r = layout_flow(&[st("a"), st("b")], None, &FlowOptions::default());
        assert!(r.stations.iter().all(|s| s.step == Step::Pending));
        assert!(!r.connectors[0].flowed);
    }

    #[test]
    fn ring_points_start_at_top_and_go_clockwise() {
        let pts = phase_ring_points(100.0, 100.0, 50.0);
        assert_eq!(pts.len(), 6);
        // phase 0 at top: x == cx, y < cy
        assert!((pts[0].1 - 100.0).abs() < 1e-9);
        assert!(pts[0].2 < 100.0);
        // next point clockwise -> x increases (moves right)
        assert!(pts[1].1 > 100.0);
        // all points sit on the radius
        for (_, x, y) in &pts {
            let d = ((x - 100.0).powi(2) + (y - 100.0).powi(2)).sqrt();
            assert!((d - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn ring_points_preserve_phase_order() {
        let pts = phase_ring_points(0.0, 0.0, 10.0);
        let phases: Vec<Phase> = pts.iter().map(|(p, _, _)| *p).collect();
        assert_eq!(phases, Phase::ALL.to_vec());
    }

    #[test]
    fn pass_beats_are_three_in_order() {
        assert_eq!(PassBeat::ALL, [PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve]);
        assert_eq!(PassBeat::Make.label(), "make");
        assert_eq!(PassBeat::Resolve.label(), "resolve");
    }

    #[test]
    fn every_phase_has_a_nonempty_beat_and_label() {
        for p in Phase::ALL {
            assert!(!phase_beat(p).is_empty());
            assert!(!phase_label(p).is_empty());
        }
    }

    #[test]
    fn walkthrough_expands_each_station_into_eight_ticks() {
        let steps = walkthrough_steps(&["frame".into(), "build".into()]);
        assert_eq!(TICKS_PER_STATION, 8);
        assert_eq!(steps.len(), 16);
        // first station occupies the first 8 ticks
        assert!(steps[..8].iter().all(|s| s.station_index == 0));
        assert!(steps[8..].iter().all(|s| s.station_index == 1));
    }

    #[test]
    fn walkthrough_manufacture_carries_three_beats() {
        let steps = walkthrough_steps(&["build".into()]);
        let manu: Vec<&WalkStep> = steps.iter().filter(|s| s.phase == Phase::Manufacture).collect();
        assert_eq!(manu.len(), 3);
        assert_eq!(manu[0].beat, Some(PassBeat::Make));
        assert_eq!(manu[1].beat, Some(PassBeat::Challenge));
        assert_eq!(manu[2].beat, Some(PassBeat::Resolve));
        // non-manufacture ticks carry no beat
        assert!(steps.iter().filter(|s| s.phase != Phase::Manufacture).all(|s| s.beat.is_none()));
    }

    #[test]
    fn walkthrough_phase_order_within_a_station() {
        let steps = walkthrough_steps(&["frame".into()]);
        // Collapse the manufacture beats back to a single phase to check order.
        let mut seen: Vec<Phase> = Vec::new();
        for s in &steps {
            if seen.last() != Some(&s.phase) {
                seen.push(s.phase);
            }
        }
        assert_eq!(seen, Phase::ALL.to_vec());
    }

    #[test]
    fn walkthrough_empty_is_empty() {
        assert!(walkthrough_steps(&[]).is_empty());
    }

    #[test]
    fn narration_for_manufacture_names_the_beat() {
        let steps = walkthrough_steps(&["build".into()]);
        let challenge = steps.iter().find(|s| s.beat == Some(PassBeat::Challenge)).unwrap();
        let n = challenge.narration();
        assert!(n.contains("build"));
        assert!(n.contains("manufacture"));
        assert!(n.contains("challenge"));
    }

    #[test]
    fn narration_for_plain_phase_uses_phase_beat() {
        let steps = walkthrough_steps(&["frame".into()]);
        let spec = steps.iter().find(|s| s.phase == Phase::Spec).unwrap();
        let n = spec.narration();
        assert!(n.contains("frame"));
        assert!(n.contains("spec"));
        assert!(n.contains(phase_beat(Phase::Spec)));
    }

    #[test]
    fn checkpoint_hue_maps_each_kind() {
        assert_eq!(checkpoint_hue(CheckpointKind::Auto), tokens::STATUS_OK);
        assert_eq!(checkpoint_hue(CheckpointKind::Ask), tokens::STATUS_WARN);
        assert_eq!(checkpoint_hue(CheckpointKind::Await), tokens::STATUS_WARN);
        assert_eq!(checkpoint_hue(CheckpointKind::External), tokens::STATUS_INFO);
    }

    #[test]
    fn flow_station_builder_sets_fields() {
        let s = FlowStation::new("build", CheckpointKind::Ask)
            .with_label("Build")
            .with_risk("wrong implementation");
        assert_eq!(s.slug, "build");
        assert_eq!(s.label, "Build");
        assert_eq!(s.checkpoint, CheckpointKind::Ask);
        assert_eq!(s.risk.as_deref(), Some("wrong implementation"));
    }
}

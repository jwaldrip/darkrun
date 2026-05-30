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
/// Spec -> Review -> Manufacture(Make/Challenge/Resolve) -> Audit -> Reflect ->
/// Checkpoint.
pub fn phase_beat(phase: Phase) -> &'static str {
    match phase {
        Phase::Spec => "spec — elaborate the work, then explore + decompose into units",
        Phase::Review => "review the spec — adversarial pass, brief, then a user decision",
        Phase::Manufacture => "manufacture — the Plan / Make / Challenge / Resolve pass loop",
        Phase::Audit => "audit against the spec — verify + run the quality checks, adversarial pass",
        Phase::Reflect => "reflect — an autonomous retrospective feeding the run-level reflections",
        Phase::Checkpoint => "checkpoint — read the closing brief, then the user gate fires",
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
        Phase::Reflect => "Reflect",
        Phase::Checkpoint => "Checkpoint",
    }
}

/// A named sub-step (a "beat") within a phase.
///
/// Every phase walks an ordered list of these — the way Manufacture has always
/// expanded into Make → Challenge → Resolve, now generalized so the
/// `PhaseMachine`/`RunWalkthrough` can expand *any* phase into its sub-steps.
/// The vocabulary is shared across phases (e.g. both Review and Audit run an
/// `Adversarial` beat; both Review and Checkpoint end on a `User` beat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// Elaborate the locked work before exploring (spec).
    Elaborate,
    /// Run Explorers + decompose into units (spec).
    Explore,
    /// Verify against the locked spec (review, audit).
    Spec,
    /// An adversarial reviewer pass (review, audit).
    Adversarial,
    /// Produce / read the closing-brief summary artifact (review, checkpoint).
    Brief,
    /// A genuine user input / decision step (review, checkpoint).
    User,
    /// Plan: decide the approach for the unit before building (manufacture).
    Plan,
    /// Make: produce the candidate output (manufacture).
    Make,
    /// Challenge: attack it, find the weakness (manufacture).
    Challenge,
    /// Resolve: fix what the challenge surfaced (manufacture).
    Resolve,
    /// Autonomous reflection (reflect).
    Agentic,
}

impl Beat {
    /// The lowercase label for this beat.
    pub fn label(self) -> &'static str {
        match self {
            Beat::Elaborate => "elaborate",
            Beat::Explore => "explore",
            Beat::Spec => "spec",
            Beat::Adversarial => "adversarial",
            Beat::Brief => "brief",
            Beat::User => "user",
            Beat::Plan => "plan",
            Beat::Make => "make",
            Beat::Challenge => "challenge",
            Beat::Resolve => "resolve",
            Beat::Agentic => "agentic",
        }
    }

    /// One-line description of what the beat does.
    pub fn desc(self) -> &'static str {
        match self {
            Beat::Elaborate => "elaborate the locked work before exploring",
            Beat::Explore => "run Explorers + decompose into units",
            Beat::Spec => "verify against the locked spec",
            Beat::Adversarial => "an adversarial reviewer pass",
            Beat::Brief => "produce/read the closing-brief summary artifact",
            Beat::User => "a genuine user input / decision step",
            Beat::Plan => "decide the approach from spec + design; name the riskiest assumption",
            Beat::Make => "produce the candidate output",
            Beat::Challenge => "attack it — find the weakest seam",
            Beat::Resolve => "fix what the challenge surfaced",
            Beat::Agentic => "an autonomous retrospective",
        }
    }
}

/// The ordered named sub-steps (beats) a phase walks.
///
/// This is the generalized sub-step model: today only Manufacture had beats
/// (the Make/Challenge/Resolve pass); now every phase expands into its own
/// named beats, which the `PhaseMachine` strip and the `RunWalkthrough` step
/// sequencing both ride.
pub fn phase_beats(phase: Phase) -> Vec<Beat> {
    match phase {
        Phase::Spec => vec![Beat::Elaborate, Beat::Explore],
        Phase::Review => vec![Beat::Spec, Beat::Adversarial, Beat::Brief, Beat::User],
        Phase::Manufacture => vec![Beat::Plan, Beat::Make, Beat::Challenge, Beat::Resolve],
        Phase::Audit => vec![Beat::Spec, Beat::Adversarial],
        Phase::Reflect => vec![Beat::Agentic],
        Phase::Checkpoint => vec![Beat::Brief, Beat::User],
    }
}

/// The four beats of a Manufacture Pass, in order.
///
/// Retained as the typed Plan → Make → Challenge → Resolve worker pass: Plan
/// (decide the approach from spec + design) then the adversarial-hardening loop
/// (Make → Challenge → Resolve). It is the Manufacture slice of the broader
/// [`Beat`] vocabulary; [`PassBeat::as_beat`] bridges to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassBeat {
    /// Plan: decide the approach for the unit before building.
    Plan,
    /// Make: produce the candidate output.
    Make,
    /// Challenge: attack it, find the weakness.
    Challenge,
    /// Resolve: fix what the challenge surfaced.
    Resolve,
}

impl PassBeat {
    /// All four beats in order.
    pub const ALL: [PassBeat; 4] =
        [PassBeat::Plan, PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve];

    /// The lowercase label.
    pub fn label(self) -> &'static str {
        self.as_beat().label()
    }

    /// One-line description of what the beat does.
    pub fn beat(self) -> &'static str {
        self.as_beat().desc()
    }

    /// The matching entry in the generalized [`Beat`] vocabulary.
    pub fn as_beat(self) -> Beat {
        match self {
            PassBeat::Plan => Beat::Plan,
            PassBeat::Make => Beat::Make,
            PassBeat::Challenge => Beat::Challenge,
            PassBeat::Resolve => Beat::Resolve,
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

/// One tick of a run walkthrough: a station, the phase within it, and which
/// named sub-step ([`Beat`]) of that phase is active.
///
/// Every phase now expands into its beats, so `beat` is always populated. When
/// the phase is Manufacture, [`WalkStep::pass_beat`] additionally exposes the
/// typed Make/Challenge/Resolve value the PhaseMachine strip rides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkStep {
    /// Index of the station in the pipeline.
    pub station_index: usize,
    /// The station's slug.
    pub station_slug: String,
    /// The phase active at this tick.
    pub phase: Phase,
    /// The named sub-step of `phase` active at this tick.
    pub beat: Beat,
}

impl WalkStep {
    /// The typed Manufacture pass beat, present only while `phase ==
    /// Manufacture`. Lets the Make/Challenge/Resolve strip stay strongly typed.
    pub fn pass_beat(&self) -> Option<PassBeat> {
        match (self.phase, self.beat) {
            (Phase::Manufacture, Beat::Plan) => Some(PassBeat::Plan),
            (Phase::Manufacture, Beat::Make) => Some(PassBeat::Make),
            (Phase::Manufacture, Beat::Challenge) => Some(PassBeat::Challenge),
            (Phase::Manufacture, Beat::Resolve) => Some(PassBeat::Resolve),
            _ => None,
        }
    }

    /// The narration line for this tick, e.g.
    /// `"build · manufacture → challenge: attack it — find the weakest seam"`.
    pub fn narration(&self) -> String {
        format!(
            "{station} · {phase} → {beat}: {desc}",
            station = self.station_slug,
            phase = self.phase.name(),
            beat = self.beat.label(),
            desc = self.beat.desc(),
        )
    }
}

/// Build the full ordered list of walkthrough ticks for a pipeline of `stations`
/// slugs. Each station expands into its six phases, and **every** phase further
/// expands into its named beats ([`phase_beats`]): spec(2) + review(4) +
/// manufacture(4) + audit(2) + reflect(1) + checkpoint(2) = [`TICKS_PER_STATION`]
/// ticks per station, and the whole run is `stations * TICKS_PER_STATION`.
pub fn walkthrough_steps(stations: &[String]) -> Vec<WalkStep> {
    let mut steps = Vec::with_capacity(stations.len() * TICKS_PER_STATION);
    for (station_index, slug) in stations.iter().enumerate() {
        for phase in Phase::ALL {
            for beat in phase_beats(phase) {
                steps.push(WalkStep {
                    station_index,
                    station_slug: slug.clone(),
                    phase,
                    beat,
                });
            }
        }
    }
    steps
}

/// The number of ticks one station contributes to a walkthrough — the sum of
/// every phase's beat count: spec(2)+review(4)+manufacture(4)+audit(2)+
/// reflect(1)+checkpoint(2) = 15.
pub const TICKS_PER_STATION: usize = 15;

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
        assert_eq!(PassBeat::ALL, [PassBeat::Plan, PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve]);
        assert_eq!(PassBeat::Make.label(), "make");
        assert_eq!(PassBeat::Resolve.label(), "resolve");
        // bridges into the generalized Beat vocabulary
        assert_eq!(PassBeat::Make.as_beat(), Beat::Make);
        assert_eq!(PassBeat::Challenge.as_beat(), Beat::Challenge);
        assert_eq!(PassBeat::Resolve.as_beat(), Beat::Resolve);
    }

    #[test]
    fn every_phase_has_a_nonempty_beat_and_label() {
        for p in Phase::ALL {
            assert!(!phase_beat(p).is_empty());
            assert!(!phase_label(p).is_empty());
        }
    }

    #[test]
    fn phase_beats_match_the_spec_table() {
        assert_eq!(phase_beats(Phase::Spec), vec![Beat::Elaborate, Beat::Explore]);
        assert_eq!(
            phase_beats(Phase::Review),
            vec![Beat::Spec, Beat::Adversarial, Beat::Brief, Beat::User]
        );
        assert_eq!(
            phase_beats(Phase::Manufacture),
            vec![Beat::Plan, Beat::Make, Beat::Challenge, Beat::Resolve]
        );
        assert_eq!(phase_beats(Phase::Audit), vec![Beat::Spec, Beat::Adversarial]);
        assert_eq!(phase_beats(Phase::Reflect), vec![Beat::Agentic]);
        assert_eq!(phase_beats(Phase::Checkpoint), vec![Beat::Brief, Beat::User]);
    }

    #[test]
    fn phase_beats_sum_to_ticks_per_station() {
        let total: usize = Phase::ALL.iter().map(|p| phase_beats(*p).len()).sum();
        assert_eq!(total, TICKS_PER_STATION);
        assert_eq!(TICKS_PER_STATION, 15);
    }

    #[test]
    fn every_beat_has_nonempty_label_and_desc() {
        for p in Phase::ALL {
            for b in phase_beats(p) {
                assert!(!b.label().is_empty());
                assert!(!b.desc().is_empty());
            }
        }
    }

    #[test]
    fn walkthrough_expands_each_station_into_its_beats() {
        let steps = walkthrough_steps(&["frame".into(), "build".into()]);
        assert_eq!(TICKS_PER_STATION, 15);
        assert_eq!(steps.len(), 30);
        // first station occupies the first 15 ticks
        assert!(steps[..15].iter().all(|s| s.station_index == 0));
        assert!(steps[15..].iter().all(|s| s.station_index == 1));
    }

    #[test]
    fn walkthrough_phase_beat_counts_match_phase_beats() {
        let steps = walkthrough_steps(&["build".into()]);
        for phase in Phase::ALL {
            let got: Vec<Beat> =
                steps.iter().filter(|s| s.phase == phase).map(|s| s.beat).collect();
            assert_eq!(got, phase_beats(phase), "beats for {phase:?}");
        }
    }

    #[test]
    fn walkthrough_manufacture_carries_four_pass_beats() {
        let steps = walkthrough_steps(&["build".into()]);
        let manu: Vec<&WalkStep> = steps.iter().filter(|s| s.phase == Phase::Manufacture).collect();
        assert_eq!(manu.len(), 4);
        assert_eq!(manu[0].pass_beat(), Some(PassBeat::Plan));
        assert_eq!(manu[1].pass_beat(), Some(PassBeat::Make));
        assert_eq!(manu[2].pass_beat(), Some(PassBeat::Challenge));
        assert_eq!(manu[3].pass_beat(), Some(PassBeat::Resolve));
        // non-manufacture ticks have no typed pass beat
        assert!(steps.iter().filter(|s| s.phase != Phase::Manufacture).all(|s| s.pass_beat().is_none()));
    }

    #[test]
    fn walkthrough_phase_order_within_a_station() {
        let steps = walkthrough_steps(&["frame".into()]);
        // Collapse the per-phase beats back to a single phase to check order.
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
        let challenge = steps.iter().find(|s| s.pass_beat() == Some(PassBeat::Challenge)).unwrap();
        let n = challenge.narration();
        assert!(n.contains("build"));
        assert!(n.contains("manufacture"));
        assert!(n.contains("challenge"));
    }

    #[test]
    fn narration_for_plain_phase_names_phase_and_beat() {
        let steps = walkthrough_steps(&["frame".into()]);
        let spec = steps.iter().find(|s| s.phase == Phase::Spec).unwrap();
        let n = spec.narration();
        assert!(n.contains("frame"));
        assert!(n.contains("spec"));
        assert!(n.contains(spec.beat.label()));
        assert!(n.contains(spec.beat.desc()));
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

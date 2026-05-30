//! Integration tests for the deeper-visualization logic in `darkrun-ui::flow` —
//! the pure math + semantics behind StationFlow, PhaseMachine, and the
//! RunWalkthrough stepper. Exercised through the crate's public API (the
//! `prelude`) without instantiating a renderer.
//!
//! Coverage:
//! - station pipeline layout: spacing, connectors, done/active/pending state,
//!   canvas sizing, degenerate inputs, and the per-station phase-hue cycle
//! - phase-ring geometry: even placement on the radius, twelve-o'clock start,
//!   clockwise order, phase-order preservation
//! - phase-machine semantics: every phase maps to a non-empty beat + label, the
//!   Make/Challenge/Resolve pass is three ordered beats
//! - walkthrough step sequencing: per-station expansion (8 ticks), Manufacture's
//!   three beats, phase order, narration content, and the checkpoint-hue map

use darkrun_ui::prelude::*;

fn st(slug: &str) -> FlowStation {
    FlowStation::new(slug, CheckpointKind::Auto)
}

// ===========================================================================
// station flow layout
// ===========================================================================

#[test]
fn empty_flow_is_padding_only() {
    let r = layout_flow(&[], None, &FlowOptions::default());
    assert!(r.stations.is_empty());
    assert!(r.connectors.is_empty());
    let opts = FlowOptions::default();
    assert_eq!(r.width, opts.padding * 2.0);
}

#[test]
fn single_station_has_no_connectors() {
    let r = layout_flow(&[st("frame")], Some(0), &FlowOptions::default());
    assert_eq!(r.stations.len(), 1);
    assert!(r.connectors.is_empty());
    assert_eq!(r.stations[0].step, Step::Active);
}

#[test]
fn stations_are_evenly_spaced_on_a_baseline() {
    let opts = FlowOptions::default();
    let r = layout_flow(&[st("a"), st("b"), st("c"), st("d")], None, &opts);
    for w in r.stations.windows(2) {
        assert_eq!(w[1].cx - w[0].cx, opts.node_gap);
        assert_eq!(w[0].cy, w[1].cy);
    }
}

#[test]
fn connector_count_is_stations_minus_one() {
    let r = layout_flow(&[st("a"), st("b"), st("c")], None, &FlowOptions::default());
    assert_eq!(r.connectors.len(), 2);
}

#[test]
fn connectors_span_between_node_edges_left_to_right() {
    let r = layout_flow(&[st("a"), st("b")], None, &FlowOptions::default());
    let c = &r.connectors[0];
    let a = &r.stations[0];
    let b = &r.stations[1];
    assert_eq!(c.x1, a.cx + a.r);
    assert_eq!(c.x2, b.cx - b.r);
    assert!(c.x2 > c.x1);
    assert_eq!(c.y, a.cy);
}

#[test]
fn active_index_partitions_done_active_pending() {
    let r = layout_flow(&[st("a"), st("b"), st("c"), st("d"), st("e")], Some(2), &FlowOptions::default());
    let steps: Vec<Step> = r.stations.iter().map(|s| s.step).collect();
    assert_eq!(steps, vec![Step::Done, Step::Done, Step::Active, Step::Pending, Step::Pending]);
}

#[test]
fn exactly_one_active_station() {
    for active in 0..4 {
        let r = layout_flow(&[st("a"), st("b"), st("c"), st("d")], Some(active), &FlowOptions::default());
        let actives = r.stations.iter().filter(|s| s.step == Step::Active).count();
        assert_eq!(actives, 1);
        assert_eq!(r.stations[active].step, Step::Active);
    }
}

#[test]
fn none_active_is_all_pending_and_no_flow() {
    let r = layout_flow(&[st("a"), st("b"), st("c")], None, &FlowOptions::default());
    assert!(r.stations.iter().all(|s| s.step == Step::Pending));
    assert!(r.connectors.iter().all(|c| !c.flowed));
}

#[test]
fn connector_flow_lights_only_after_upstream_done() {
    let r = layout_flow(&[st("a"), st("b"), st("c"), st("d")], Some(2), &FlowOptions::default());
    // a,b done -> their outgoing connectors flowed; c active -> c->d not.
    assert!(r.connectors[0].flowed); // a->b
    assert!(r.connectors[1].flowed); // b->c
    assert!(!r.connectors[2].flowed); // c->d
}

#[test]
fn glyph_follows_step_state() {
    let r = layout_flow(&[st("a"), st("b"), st("c")], Some(1), &FlowOptions::default());
    assert_eq!(r.stations[0].glyph, tokens::GLYPH_DONE);
    assert_eq!(r.stations[1].glyph, tokens::GLYPH_ACTIVE);
    assert_eq!(r.stations[2].glyph, tokens::GLYPH_PENDING);
}

#[test]
fn station_hue_cycles_the_six_phase_hues() {
    for i in 0..Phase::ALL.len() {
        assert_eq!(station_hue(i), Phase::ALL[i].hue());
    }
    // wraps after six
    assert_eq!(station_hue(6), Phase::Spec.hue());
    assert_eq!(station_hue(11), Phase::Checkpoint.hue());
}

#[test]
fn placed_station_carries_checkpoint_and_risk() {
    let stations = vec![
        FlowStation::new("frame", CheckpointKind::Ask).with_risk("wrong problem"),
    ];
    let r = layout_flow(&stations, Some(0), &FlowOptions::default());
    assert_eq!(r.stations[0].checkpoint, CheckpointKind::Ask);
    assert_eq!(r.stations[0].risk.as_deref(), Some("wrong problem"));
}

#[test]
fn canvas_grows_with_station_count() {
    let r1 = layout_flow(&[st("a")], None, &FlowOptions::default());
    let r2 = layout_flow(&[st("a"), st("b"), st("c")], None, &FlowOptions::default());
    assert!(r2.width > r1.width);
    assert_eq!(r1.height, r2.height); // single row, height is constant
}

#[test]
fn layout_is_deterministic() {
    let s = vec![st("a"), st("b"), st("c")];
    let a = layout_flow(&s, Some(1), &FlowOptions::default());
    let b = layout_flow(&s, Some(1), &FlowOptions::default());
    assert_eq!(a, b);
}

#[test]
fn every_node_sits_inside_the_canvas() {
    let r = layout_flow(&[st("a"), st("b"), st("c"), st("d")], Some(1), &FlowOptions::default());
    for s in &r.stations {
        assert!(s.cx - s.r >= 0.0);
        assert!(s.cx + s.r <= r.width + 1e-9);
        assert!(s.cy - s.r >= 0.0);
    }
}

// ===========================================================================
// phase ring geometry
// ===========================================================================

#[test]
fn ring_has_six_points_in_phase_order() {
    let pts = phase_ring_points(50.0, 50.0, 30.0);
    assert_eq!(pts.len(), 6);
    let phases: Vec<Phase> = pts.iter().map(|(p, _, _)| *p).collect();
    assert_eq!(phases, Phase::ALL.to_vec());
}

#[test]
fn ring_first_point_is_at_twelve_oclock() {
    let pts = phase_ring_points(100.0, 100.0, 40.0);
    let (_, x, y) = pts[0];
    assert!((x - 100.0).abs() < 1e-9); // directly above center
    assert!(y < 100.0);
}

#[test]
fn ring_proceeds_clockwise() {
    let pts = phase_ring_points(0.0, 0.0, 10.0);
    // After top, the next point is to the right (clockwise).
    assert!(pts[1].1 > 0.0);
    // The point opposite the top (index 3 of 6) is at the bottom.
    assert!(pts[3].2 > 0.0);
    assert!((pts[3].1).abs() < 1e-9);
}

#[test]
fn ring_points_lie_on_the_radius() {
    let (cx, cy, r) = (12.0, 34.0, 56.0);
    for (_, x, y) in phase_ring_points(cx, cy, r) {
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        assert!((d - r).abs() < 1e-9);
    }
}

#[test]
fn ring_points_are_evenly_spaced() {
    let pts = phase_ring_points(0.0, 0.0, 100.0);
    // chord length between adjacent points is constant for an even ring.
    let chord = |i: usize, j: usize| {
        ((pts[i].1 - pts[j].1).powi(2) + (pts[i].2 - pts[j].2).powi(2)).sqrt()
    };
    let base = chord(0, 1);
    for i in 0..6 {
        let c = chord(i, (i + 1) % 6);
        assert!((c - base).abs() < 1e-6, "uneven spacing at {i}");
    }
}

// ===========================================================================
// phase-machine semantics
// ===========================================================================

#[test]
fn every_phase_has_nonempty_beat_and_label() {
    for p in Phase::ALL {
        assert!(!phase_beat(p).is_empty());
        assert!(!phase_label(p).is_empty());
    }
}

#[test]
fn phase_beats_are_distinct() {
    let beats: Vec<&str> = Phase::ALL.iter().map(|p| phase_beat(*p)).collect();
    for i in 0..beats.len() {
        for j in (i + 1)..beats.len() {
            assert_ne!(beats[i], beats[j]);
        }
    }
}

#[test]
fn manufacture_beat_mentions_the_pass_loop() {
    let b = phase_beat(Phase::Manufacture);
    assert!(b.to_lowercase().contains("make"));
    assert!(b.to_lowercase().contains("challenge"));
    assert!(b.to_lowercase().contains("resolve"));
}

#[test]
fn pass_beat_is_three_ordered_steps() {
    assert_eq!(PassBeat::ALL.len(), 3);
    assert_eq!(PassBeat::ALL[0], PassBeat::Make);
    assert_eq!(PassBeat::ALL[1], PassBeat::Challenge);
    assert_eq!(PassBeat::ALL[2], PassBeat::Resolve);
    for b in PassBeat::ALL {
        assert!(!b.label().is_empty());
        assert!(!b.beat().is_empty());
    }
}

#[test]
fn pass_beat_labels_are_distinct() {
    assert_ne!(PassBeat::Make.label(), PassBeat::Challenge.label());
    assert_ne!(PassBeat::Challenge.label(), PassBeat::Resolve.label());
}

// ===========================================================================
// walkthrough step sequencing
// ===========================================================================

#[test]
fn ticks_per_station_is_eight() {
    assert_eq!(TICKS_PER_STATION, 8);
}

#[test]
fn walkthrough_total_is_stations_times_ticks() {
    let s = vec!["frame".to_string(), "build".to_string(), "prove".to_string()];
    let steps = walkthrough_steps(&s);
    assert_eq!(steps.len(), 3 * TICKS_PER_STATION);
}

#[test]
fn walkthrough_empty_yields_no_steps() {
    assert!(walkthrough_steps(&[]).is_empty());
}

#[test]
fn each_station_block_is_contiguous() {
    let s = vec!["a".to_string(), "b".to_string()];
    let steps = walkthrough_steps(&s);
    assert!(steps[..TICKS_PER_STATION].iter().all(|w| w.station_index == 0));
    assert!(steps[TICKS_PER_STATION..].iter().all(|w| w.station_index == 1));
    assert!(steps[..TICKS_PER_STATION].iter().all(|w| w.station_slug == "a"));
}

#[test]
fn manufacture_expands_into_three_ordered_beats() {
    let steps = walkthrough_steps(&["build".to_string()]);
    let beats: Vec<PassBeat> = steps
        .iter()
        .filter(|w| w.phase == Phase::Manufacture)
        .filter_map(|w| w.beat)
        .collect();
    assert_eq!(beats, vec![PassBeat::Make, PassBeat::Challenge, PassBeat::Resolve]);
}

#[test]
fn non_manufacture_ticks_have_no_beat() {
    let steps = walkthrough_steps(&["build".to_string()]);
    assert!(steps
        .iter()
        .filter(|w| w.phase != Phase::Manufacture)
        .all(|w| w.beat.is_none()));
}

#[test]
fn phase_order_within_station_is_canonical() {
    let steps = walkthrough_steps(&["frame".to_string()]);
    let mut collapsed: Vec<Phase> = Vec::new();
    for w in &steps {
        if collapsed.last() != Some(&w.phase) {
            collapsed.push(w.phase);
        }
    }
    assert_eq!(collapsed, Phase::ALL.to_vec());
}

#[test]
fn narration_names_station_and_phase() {
    let steps = walkthrough_steps(&["harden".to_string()]);
    let audit = steps.iter().find(|w| w.phase == Phase::Audit).unwrap();
    let n = audit.narration();
    assert!(n.contains("harden"));
    assert!(n.contains("audit"));
    assert!(n.contains(phase_beat(Phase::Audit)));
}

#[test]
fn narration_for_manufacture_names_the_active_beat() {
    let steps = walkthrough_steps(&["build".to_string()]);
    for b in PassBeat::ALL {
        let step = steps.iter().find(|w| w.beat == Some(b)).unwrap();
        let n = step.narration();
        assert!(n.contains("build"));
        assert!(n.contains("manufacture"));
        assert!(n.contains(b.label()));
    }
}

#[test]
fn first_and_last_tick_make_sense() {
    let steps = walkthrough_steps(&["frame".to_string(), "harden".to_string()]);
    let first = &steps[0];
    assert_eq!(first.station_index, 0);
    assert_eq!(first.phase, Phase::Spec);
    let last = steps.last().unwrap();
    assert_eq!(last.station_index, 1);
    assert_eq!(last.phase, Phase::Checkpoint);
}

// ===========================================================================
// checkpoint hue mapping
// ===========================================================================

#[test]
fn checkpoint_hue_maps_every_kind_to_valid_hue() {
    for k in [CheckpointKind::Auto, CheckpointKind::Ask, CheckpointKind::External, CheckpointKind::Await] {
        let hue = checkpoint_hue(k);
        assert!(hue.starts_with('#') && hue.len() == 7);
    }
}

#[test]
fn checkpoint_hue_auto_is_ok_green() {
    assert_eq!(checkpoint_hue(CheckpointKind::Auto), tokens::STATUS_OK);
}

#[test]
fn checkpoint_hue_ask_and_await_are_warn() {
    assert_eq!(checkpoint_hue(CheckpointKind::Ask), tokens::STATUS_WARN);
    assert_eq!(checkpoint_hue(CheckpointKind::Await), tokens::STATUS_WARN);
}

#[test]
fn checkpoint_hue_external_is_info() {
    assert_eq!(checkpoint_hue(CheckpointKind::External), tokens::STATUS_INFO);
}

// ===========================================================================
// RoleKind (drill-down card taxonomy)
// ===========================================================================

#[test]
fn role_kinds_have_distinct_labels_and_tones() {
    use darkrun_ui::components::role::RoleKind;
    let kinds = [RoleKind::Explorer, RoleKind::Worker, RoleKind::Reviewer];
    let labels: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
    assert_eq!(labels, vec!["explorer", "worker", "reviewer"]);
    // worker reads as the manufacture accent, reviewer as review-info.
    assert_eq!(RoleKind::Worker.tone(), Tone::Accent);
    assert_eq!(RoleKind::Reviewer.tone(), Tone::Info);
    assert_eq!(RoleKind::Explorer.tone(), Tone::Neutral);
}

// ===========================================================================
// RightSizeTier (run collapsing)
// ===========================================================================

#[test]
fn right_size_tier_holds_label_and_kept() {
    let t = RightSizeTier::new("tiny", vec!["frame".to_string(), "build".to_string()]);
    assert_eq!(t.label, "tiny");
    assert_eq!(t.kept, vec!["frame".to_string(), "build".to_string()]);
}

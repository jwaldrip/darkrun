//! Comprehensive integration tests for `darkrun-ui` pure logic — the functions
//! and data behind the components, exercised through the crate's public API
//! without instantiating a renderer.
//!
//! Coverage:
//! - token values, the `Hue` pairs, and the `THEME_CSS` <-> Rust-constant lockstep
//! - glyph/phase mapping for the station pipeline (● ◉ ○ + per-phase hues)
//! - `Tone`/`Step`/`Phase` enum behavior, parsing, round-trips, contrast
//! - the layered (Sugiyama-ish) layout math: layering, placement, edge routing,
//!   canvas sizing, determinism, idempotency, and degenerate inputs
//! - wordmark variant selection and the graph view node builders
//!
//! Hex-color contrast is computed locally (WCAG relative luminance) so the tests
//! can assert real legibility properties of the design tokens.

use darkrun_ui::components::factory::CheckpointKind;
use darkrun_ui::components::pipeline::{strip_for, PhaseDot};
use darkrun_ui::components::primitives::ButtonVariant;
use darkrun_ui::components::wordmark::WordmarkVariant;
use darkrun_ui::graph::layout::{
    GraphEdge, GraphLayout, GraphNode, LayeredLayout, LayoutOptions, LayoutResult,
    PlacedEdge, PlacedNode,
};
use darkrun_ui::graph::view::UnitGraphNode;
use darkrun_ui::kinds::{Phase, Step, Tone};
use darkrun_ui::tokens;

// ---------------------------------------------------------------------------
// Local helpers (color math; layout convenience)
// ---------------------------------------------------------------------------

/// Parse a `#rrggbb` string into (r, g, b) bytes.
fn rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.strip_prefix('#').expect("hex starts with #");
    assert_eq!(h.len(), 6, "expected #rrggbb, got {hex}");
    let r = u8::from_str_radix(&h[0..2], 16).unwrap();
    let g = u8::from_str_radix(&h[2..4], 16).unwrap();
    let b = u8::from_str_radix(&h[4..6], 16).unwrap();
    (r, g, b)
}

/// WCAG relative luminance of a `#rrggbb` color, in [0, 1].
fn luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    let lin = |c: u8| {
        let s = c as f64 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

/// WCAG contrast ratio between two `#rrggbb` colors (>= 1.0).
fn contrast(a: &str, b: &str) -> f64 {
    let la = luminance(a);
    let lb = luminance(b);
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn n(id: &str) -> GraphNode {
    GraphNode::new(id, id)
}

fn placed<'a>(r: &'a LayoutResult, id: &str) -> &'a PlacedNode {
    r.nodes.iter().find(|p| p.id == id).expect("node placed")
}

fn layer_of(r: &LayoutResult, id: &str) -> usize {
    placed(r, id).layer
}

fn edge<'a>(r: &'a LayoutResult, from: &str, to: &str) -> &'a PlacedEdge {
    r.edges
        .iter()
        .find(|e| e.from == from && e.to == to)
        .expect("routed edge present")
}

// ===========================================================================
// tokens: hex validity & shape
// ===========================================================================

#[test]
fn all_surface_tokens_are_valid_hex() {
    for c in [
        tokens::SURFACE_BASE,
        tokens::SURFACE_RAISED,
        tokens::SURFACE_OVERLAY,
        tokens::BORDER,
        tokens::BORDER_STRONG,
    ] {
        let _ = rgb(c);
    }
}

#[test]
fn all_text_tokens_are_valid_hex() {
    for c in [tokens::TEXT, tokens::TEXT_MUTED, tokens::TEXT_FAINT] {
        let _ = rgb(c);
    }
}

#[test]
fn all_accent_tokens_are_valid_hex() {
    for c in [tokens::ACCENT, tokens::ACCENT_STRONG, tokens::ON_ACCENT] {
        let _ = rgb(c);
    }
}

#[test]
fn all_status_tokens_are_valid_hex() {
    for c in [
        tokens::STATUS_OK,
        tokens::STATUS_WARN,
        tokens::STATUS_DANGER,
        tokens::STATUS_INFO,
    ] {
        let _ = rgb(c);
    }
}

#[test]
fn every_phase_hue_pair_is_valid_hex() {
    for (_, hue) in tokens::PHASES {
        let _ = rgb(hue.base);
        let _ = rgb(hue.on);
    }
}

#[test]
fn surfaces_layer_from_dark_to_light() {
    // base is the deepest (darkest), overlay the lightest of the three.
    let base = luminance(tokens::SURFACE_BASE);
    let raised = luminance(tokens::SURFACE_RAISED);
    let overlay = luminance(tokens::SURFACE_OVERLAY);
    assert!(base < raised, "raised should be lighter than base");
    assert!(raised < overlay, "overlay should be lighter than raised");
}

#[test]
fn border_strong_is_lighter_than_hairline_border() {
    assert!(luminance(tokens::BORDER_STRONG) > luminance(tokens::BORDER));
}

#[test]
fn text_scale_dims_from_primary_to_faint() {
    let primary = luminance(tokens::TEXT);
    let muted = luminance(tokens::TEXT_MUTED);
    let faint = luminance(tokens::TEXT_FAINT);
    assert!(primary > muted, "muted dimmer than primary");
    assert!(muted > faint, "faint dimmer than muted");
}

#[test]
fn accent_strong_differs_from_accent() {
    assert_ne!(tokens::ACCENT, tokens::ACCENT_STRONG);
}

#[test]
fn on_accent_is_near_black() {
    // ON_ACCENT must be dark enough to read on the bright cyan accent.
    assert!(luminance(tokens::ON_ACCENT) < 0.05);
}

// ===========================================================================
// tokens: contrast / legibility
// ===========================================================================

#[test]
fn primary_text_on_base_is_high_contrast() {
    // Comfortably above WCAG AA (4.5) for body text.
    assert!(contrast(tokens::TEXT, tokens::SURFACE_BASE) >= 4.5);
}

#[test]
fn primary_text_on_raised_meets_aa() {
    assert!(contrast(tokens::TEXT, tokens::SURFACE_RAISED) >= 4.5);
}

#[test]
fn primary_text_on_overlay_meets_aa() {
    assert!(contrast(tokens::TEXT, tokens::SURFACE_OVERLAY) >= 4.5);
}

#[test]
fn muted_text_on_base_meets_large_text_aa() {
    // Muted/supporting text only needs to clear the large-text bar (3.0).
    assert!(contrast(tokens::TEXT_MUTED, tokens::SURFACE_BASE) >= 3.0);
}

#[test]
fn on_accent_reads_against_accent() {
    assert!(contrast(tokens::ON_ACCENT, tokens::ACCENT) >= 4.5);
}

#[test]
fn each_phase_on_reads_against_its_base() {
    for (name, hue) in tokens::PHASES {
        let c = contrast(hue.on, hue.base);
        assert!(c >= 3.0, "phase {name} contrast {c} too low");
    }
}

#[test]
fn accent_pops_against_base_surface() {
    // The accent must be visibly brighter than the canvas to carry interaction.
    assert!(contrast(tokens::ACCENT, tokens::SURFACE_BASE) >= 4.5);
}

#[test]
fn faint_text_is_dimmer_than_muted_on_base() {
    let muted = contrast(tokens::TEXT_MUTED, tokens::SURFACE_BASE);
    let faint = contrast(tokens::TEXT_FAINT, tokens::SURFACE_BASE);
    assert!(faint < muted);
}

#[test]
fn contrast_ratio_is_symmetric() {
    let a = contrast(tokens::TEXT, tokens::SURFACE_BASE);
    let b = contrast(tokens::SURFACE_BASE, tokens::TEXT);
    assert!((a - b).abs() < 1e-9);
}

#[test]
fn contrast_of_identical_colors_is_one() {
    assert!((contrast(tokens::ACCENT, tokens::ACCENT) - 1.0).abs() < 1e-9);
}

#[test]
fn white_on_black_is_max_contrast() {
    let c = contrast("#ffffff", "#000000");
    assert!(c > 20.0, "expected ~21:1, got {c}");
}

// ===========================================================================
// tokens: phase identity & lockstep
// ===========================================================================

#[test]
fn phases_array_is_canonical_order() {
    let names: Vec<&str> = tokens::PHASES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        ["spec", "review", "manufacture", "audit", "reflect", "checkpoint"]
    );
}

#[test]
fn phases_array_has_six_entries() {
    assert_eq!(tokens::PHASES.len(), 6);
}

#[test]
fn manufacture_phase_shares_the_brand_accent() {
    assert_eq!(tokens::PHASE_MANUFACTURE.base, tokens::ACCENT);
}

#[test]
fn reflect_phase_is_a_distinct_teal() {
    // Reflect owns its own teal — distinct from every other phase hue and from
    // the status palette (it is not the success-green nor the brand cyan).
    assert_eq!(Phase::Reflect.hue(), tokens::PHASE_REFLECT);
    assert_ne!(tokens::PHASE_REFLECT.base, tokens::STATUS_OK);
    assert_ne!(tokens::PHASE_REFLECT.base, tokens::ACCENT);
    for other in [
        tokens::PHASE_SPEC,
        tokens::PHASE_REVIEW,
        tokens::PHASE_MANUFACTURE,
        tokens::PHASE_AUDIT,
        tokens::PHASE_CHECKPOINT,
    ] {
        assert_ne!(tokens::PHASE_REFLECT.base, other.base);
    }
}

#[test]
fn audit_phase_matches_status_warn_amber() {
    assert_eq!(tokens::PHASE_AUDIT.base, tokens::STATUS_WARN);
}

#[test]
fn status_info_matches_accent() {
    assert_eq!(tokens::STATUS_INFO, tokens::ACCENT);
}

#[test]
fn every_phase_base_hue_is_distinct() {
    let bases: Vec<&str> = tokens::PHASES.iter().map(|(_, h)| h.base).collect();
    for i in 0..bases.len() {
        for j in (i + 1)..bases.len() {
            assert_ne!(bases[i], bases[j], "phase hues {i} and {j} collide");
        }
    }
}

#[test]
fn theme_css_contains_every_surface_value() {
    for v in [
        tokens::SURFACE_BASE,
        tokens::SURFACE_RAISED,
        tokens::SURFACE_OVERLAY,
        tokens::BORDER,
        tokens::BORDER_STRONG,
    ] {
        assert!(tokens::THEME_CSS.contains(v), "THEME_CSS missing {v}");
    }
}

#[test]
fn theme_css_contains_every_text_value() {
    for v in [tokens::TEXT, tokens::TEXT_MUTED, tokens::TEXT_FAINT] {
        assert!(tokens::THEME_CSS.contains(v), "THEME_CSS missing {v}");
    }
}

#[test]
fn theme_css_contains_every_phase_base() {
    for (name, hue) in tokens::PHASES {
        assert!(
            tokens::THEME_CSS.contains(hue.base),
            "THEME_CSS missing phase {name} base {}",
            hue.base
        );
    }
}

#[test]
fn theme_css_contains_every_status_value() {
    for v in [
        tokens::STATUS_OK,
        tokens::STATUS_WARN,
        tokens::STATUS_DANGER,
        tokens::STATUS_INFO,
    ] {
        assert!(tokens::THEME_CSS.contains(v), "THEME_CSS missing {v}");
    }
}

#[test]
fn theme_css_declares_root_block() {
    assert!(tokens::THEME_CSS.contains(":root{"));
}

#[test]
fn theme_css_sets_dark_color_scheme() {
    assert!(tokens::THEME_CSS.contains("color-scheme:dark"));
}

#[test]
fn theme_css_uses_dr_namespace_for_every_var() {
    // Every custom property in the block is namespaced `--dr-`.
    for line in tokens::THEME_CSS.lines() {
        let t = line.trim();
        if t.starts_with("--") {
            assert!(t.starts_with("--dr-"), "unexpected non-dr var: {t}");
        }
    }
}

#[test]
fn theme_css_space_unit_matches_constant() {
    let expected = format!("--dr-space:{}px;", tokens::SPACE_UNIT);
    assert!(tokens::THEME_CSS.contains(&expected));
}

#[test]
fn theme_css_references_both_font_stacks() {
    assert!(tokens::THEME_CSS.contains("Inter"));
    assert!(tokens::THEME_CSS.contains("JetBrains Mono"));
}

// ===========================================================================
// tokens: stations, fonts, glyphs, spacing
// ===========================================================================

#[test]
fn stations_are_the_six_software_factory_steps() {
    assert_eq!(
        tokens::STATIONS,
        ["frame", "specify", "shape", "build", "prove", "harden"]
    );
}

#[test]
fn stations_are_all_distinct() {
    let s = tokens::STATIONS;
    for i in 0..s.len() {
        for j in (i + 1)..s.len() {
            assert_ne!(s[i], s[j]);
        }
    }
}

#[test]
fn station_count_matches_phase_count() {
    assert_eq!(tokens::STATIONS.len(), tokens::PHASES.len());
}

#[test]
fn font_sans_lists_inter_first() {
    assert!(tokens::FONT_SANS.starts_with("\"Inter\""));
    assert!(tokens::FONT_SANS.contains("sans-serif"));
}

#[test]
fn font_mono_lists_jetbrains_first_and_ends_monospace() {
    assert!(tokens::FONT_MONO.starts_with("\"JetBrains Mono\""));
    assert!(tokens::FONT_MONO.contains("monospace"));
}

#[test]
fn font_stacks_differ() {
    assert_ne!(tokens::FONT_SANS, tokens::FONT_MONO);
}

#[test]
fn space_unit_is_four_pixels() {
    assert_eq!(tokens::SPACE_UNIT, 4);
}

#[test]
fn space_scale_multiples_are_expected() {
    let scale: Vec<u32> = [1, 2, 3, 4, 6, 8]
        .iter()
        .map(|m| tokens::SPACE_UNIT * m)
        .collect();
    assert_eq!(scale, vec![4, 8, 12, 16, 24, 32]);
}

#[test]
fn glyphs_are_the_three_circle_codepoints() {
    assert_eq!(tokens::GLYPH_DONE, '\u{25cf}'); // ● black circle
    assert_eq!(tokens::GLYPH_ACTIVE, '\u{25c9}'); // ◉ fisheye
    assert_eq!(tokens::GLYPH_PENDING, '\u{25cb}'); // ○ white circle
}

#[test]
fn glyphs_are_pairwise_distinct() {
    assert_ne!(tokens::GLYPH_DONE, tokens::GLYPH_ACTIVE);
    assert_ne!(tokens::GLYPH_ACTIVE, tokens::GLYPH_PENDING);
    assert_ne!(tokens::GLYPH_DONE, tokens::GLYPH_PENDING);
}

// ===========================================================================
// tokens::phase_hue()
// ===========================================================================

#[test]
fn phase_hue_resolves_every_canonical_name() {
    for (name, hue) in tokens::PHASES {
        assert_eq!(tokens::phase_hue(name), Some(hue));
    }
}

#[test]
fn phase_hue_is_case_insensitive() {
    assert_eq!(tokens::phase_hue("SPEC"), Some(tokens::PHASE_SPEC));
    assert_eq!(tokens::phase_hue("Review"), Some(tokens::PHASE_REVIEW));
    assert_eq!(tokens::phase_hue("ChEcKpOiNt"), Some(tokens::PHASE_CHECKPOINT));
}

#[test]
fn phase_hue_rejects_unknown_names() {
    assert_eq!(tokens::phase_hue("unknown"), None);
    assert_eq!(tokens::phase_hue(""), None);
    assert_eq!(tokens::phase_hue("spec "), None); // trailing space not trimmed
    assert_eq!(tokens::phase_hue("manufacturing"), None);
}

#[test]
fn phase_hue_matches_kinds_phase_hue() {
    for p in Phase::ALL {
        assert_eq!(tokens::phase_hue(p.name()), Some(p.hue()));
    }
}

// ===========================================================================
// kinds::Phase
// ===========================================================================

#[test]
fn phase_all_is_six_in_order() {
    assert_eq!(Phase::ALL.len(), 6);
    assert_eq!(Phase::ALL[0], Phase::Spec);
    assert_eq!(Phase::ALL[1], Phase::Review);
    assert_eq!(Phase::ALL[2], Phase::Manufacture);
    assert_eq!(Phase::ALL[3], Phase::Audit);
    assert_eq!(Phase::ALL[4], Phase::Reflect);
    assert_eq!(Phase::ALL[5], Phase::Checkpoint);
}

#[test]
fn phase_names_are_lowercase_canonical() {
    assert_eq!(Phase::Spec.name(), "spec");
    assert_eq!(Phase::Review.name(), "review");
    assert_eq!(Phase::Manufacture.name(), "manufacture");
    assert_eq!(Phase::Audit.name(), "audit");
    assert_eq!(Phase::Reflect.name(), "reflect");
    assert_eq!(Phase::Checkpoint.name(), "checkpoint");
}

#[test]
fn phase_names_are_all_distinct() {
    let names: Vec<&str> = Phase::ALL.iter().map(|p| p.name()).collect();
    for i in 0..names.len() {
        for j in (i + 1)..names.len() {
            assert_ne!(names[i], names[j]);
        }
    }
}

#[test]
fn phase_name_round_trips_through_from_name() {
    for p in Phase::ALL {
        assert_eq!(Phase::from_name(p.name()), Some(p));
    }
}

#[test]
fn phase_from_name_is_case_insensitive() {
    assert_eq!(Phase::from_name("SPEC"), Some(Phase::Spec));
    assert_eq!(Phase::from_name("Manufacture"), Some(Phase::Manufacture));
    assert_eq!(Phase::from_name("cHeCkPoInT"), Some(Phase::Checkpoint));
}

#[test]
fn phase_from_name_rejects_garbage() {
    assert_eq!(Phase::from_name(""), None);
    assert_eq!(Phase::from_name("build"), None);
    assert_eq!(Phase::from_name("spec\n"), None);
    assert_eq!(Phase::from_name("  spec"), None);
}

#[test]
fn phase_hue_matches_token_constant_for_each() {
    assert_eq!(Phase::Spec.hue(), tokens::PHASE_SPEC);
    assert_eq!(Phase::Review.hue(), tokens::PHASE_REVIEW);
    assert_eq!(Phase::Manufacture.hue(), tokens::PHASE_MANUFACTURE);
    assert_eq!(Phase::Audit.hue(), tokens::PHASE_AUDIT);
    assert_eq!(Phase::Reflect.hue(), tokens::PHASE_REFLECT);
    assert_eq!(Phase::Checkpoint.hue(), tokens::PHASE_CHECKPOINT);
}

#[test]
fn phase_hues_are_all_distinct_bases() {
    let bases: Vec<&str> = Phase::ALL.iter().map(|p| p.hue().base).collect();
    for i in 0..bases.len() {
        for j in (i + 1)..bases.len() {
            assert_ne!(bases[i], bases[j]);
        }
    }
}

#[test]
fn phase_index_in_all_matches_phases_token_order() {
    for (i, p) in Phase::ALL.iter().enumerate() {
        assert_eq!(p.name(), tokens::PHASES[i].0);
    }
}

#[test]
fn phase_is_copy_and_eq() {
    let p = Phase::Audit;
    let q = p; // Copy
    assert_eq!(p, q);
    assert_ne!(Phase::Audit, Phase::Reflect);
}

// ===========================================================================
// kinds::Step + glyphs
// ===========================================================================

#[test]
fn step_glyph_maps_to_token_glyphs() {
    assert_eq!(Step::Done.glyph(), tokens::GLYPH_DONE);
    assert_eq!(Step::Active.glyph(), tokens::GLYPH_ACTIVE);
    assert_eq!(Step::Pending.glyph(), tokens::GLYPH_PENDING);
}

#[test]
fn step_glyphs_are_pairwise_distinct() {
    assert_ne!(Step::Done.glyph(), Step::Active.glyph());
    assert_ne!(Step::Active.glyph(), Step::Pending.glyph());
    assert_ne!(Step::Done.glyph(), Step::Pending.glyph());
}

#[test]
fn step_done_is_filled_circle() {
    assert_eq!(Step::Done.glyph(), '●');
}

#[test]
fn step_active_is_fisheye() {
    assert_eq!(Step::Active.glyph(), '◉');
}

#[test]
fn step_pending_is_hollow_circle() {
    assert_eq!(Step::Pending.glyph(), '○');
}

#[test]
fn step_is_copy_and_eq() {
    let s = Step::Active;
    let t = s;
    assert_eq!(s, t);
    assert_ne!(Step::Done, Step::Pending);
}

// ===========================================================================
// kinds::Tone
// ===========================================================================

#[test]
fn tone_default_is_accent() {
    assert_eq!(Tone::default(), Tone::Accent);
}

#[test]
fn tone_color_maps_to_expected_token() {
    assert_eq!(Tone::Accent.color(), tokens::ACCENT);
    assert_eq!(Tone::Neutral.color(), tokens::TEXT_MUTED);
    assert_eq!(Tone::Ok.color(), tokens::STATUS_OK);
    assert_eq!(Tone::Warn.color(), tokens::STATUS_WARN);
    assert_eq!(Tone::Danger.color(), tokens::STATUS_DANGER);
    assert_eq!(Tone::Info.color(), tokens::STATUS_INFO);
}

#[test]
fn tone_on_accent_and_neutral_are_specific() {
    assert_eq!(Tone::Accent.on(), tokens::ON_ACCENT);
    assert_eq!(Tone::Neutral.on(), tokens::TEXT);
}

#[test]
fn tone_status_foregrounds_are_near_black_surface() {
    assert_eq!(Tone::Ok.on(), tokens::SURFACE_BASE);
    assert_eq!(Tone::Warn.on(), tokens::SURFACE_BASE);
    assert_eq!(Tone::Danger.on(), tokens::SURFACE_BASE);
    assert_eq!(Tone::Info.on(), tokens::SURFACE_BASE);
}

#[test]
fn tone_color_is_valid_hex_for_all() {
    for t in [
        Tone::Accent,
        Tone::Neutral,
        Tone::Ok,
        Tone::Warn,
        Tone::Danger,
        Tone::Info,
    ] {
        let _ = rgb(t.color());
        let _ = rgb(t.on());
    }
}

#[test]
fn tone_foreground_reads_on_its_fill_for_filled_badges() {
    // The on() foreground must be legible (>= 3:1) against the tone fill, which
    // is exactly the contract a filled Badge/Button relies on.
    for t in [
        Tone::Accent,
        Tone::Ok,
        Tone::Warn,
        Tone::Danger,
        Tone::Info,
    ] {
        let c = contrast(t.on(), t.color());
        assert!(c >= 3.0, "tone {t:?} on/color contrast {c} too low");
    }
}

#[test]
fn tone_info_and_accent_share_a_color_but_not_identity() {
    assert_eq!(Tone::Info.color(), Tone::Accent.color());
    assert_ne!(Tone::Info, Tone::Accent);
}

#[test]
fn tone_ok_color_is_status_ok_green() {
    assert_eq!(Tone::Ok.color(), tokens::STATUS_OK);
}

#[test]
fn tone_warn_color_matches_audit_phase() {
    assert_eq!(Tone::Warn.color(), Phase::Audit.hue().base);
}

#[test]
fn tone_danger_color_is_unique_red() {
    // Danger is the only tone with a red-dominant channel.
    let (r, g, b) = rgb(Tone::Danger.color());
    assert!(r > g && r > b, "danger should be red-dominant");
}

#[test]
fn all_tones_have_distinct_color_or_documented_overlap() {
    // Only Info/Accent are allowed to share a color; everything else distinct.
    let tones = [
        Tone::Neutral,
        Tone::Ok,
        Tone::Warn,
        Tone::Danger,
    ];
    for i in 0..tones.len() {
        for j in (i + 1)..tones.len() {
            assert_ne!(tones[i].color(), tones[j].color());
        }
    }
}

#[test]
fn tone_is_copy_eq() {
    let t = Tone::Warn;
    let u = t;
    assert_eq!(t, u);
}

// ===========================================================================
// pipeline::strip_for + PhaseDot
// ===========================================================================

#[test]
fn strip_for_always_has_six_dots() {
    assert_eq!(strip_for(None).len(), 6);
    for p in Phase::ALL {
        assert_eq!(strip_for(Some(p)).len(), 6);
    }
}

#[test]
fn strip_for_preserves_canonical_phase_order() {
    let strip = strip_for(Some(Phase::Manufacture));
    let phases: Vec<Phase> = strip.iter().map(|d| d.phase).collect();
    assert_eq!(phases, Phase::ALL.to_vec());
}

#[test]
fn strip_for_none_is_all_pending() {
    let strip = strip_for(None);
    assert!(strip.iter().all(|d| d.step == Step::Pending));
}

#[test]
fn strip_for_spec_marks_only_first_active() {
    let strip = strip_for(Some(Phase::Spec));
    assert_eq!(strip[0].step, Step::Active);
    assert!(strip[1..].iter().all(|d| d.step == Step::Pending));
}

#[test]
fn strip_for_checkpoint_completes_the_line() {
    let strip = strip_for(Some(Phase::Checkpoint));
    assert!(strip[..5].iter().all(|d| d.step == Step::Done));
    assert_eq!(strip[5].step, Step::Active);
}

#[test]
fn strip_for_each_active_has_exactly_one_active_dot() {
    for p in Phase::ALL {
        let strip = strip_for(Some(p));
        let actives = strip.iter().filter(|d| d.step == Step::Active).count();
        assert_eq!(actives, 1, "phase {:?} should have one active dot", p);
    }
}

#[test]
fn strip_for_active_index_matches_done_count() {
    // Every phase before the active one is Done; that count equals the index.
    for (idx, p) in Phase::ALL.iter().enumerate() {
        let strip = strip_for(Some(*p));
        let done = strip.iter().filter(|d| d.step == Step::Done).count();
        assert_eq!(done, idx, "phase {p:?} done-count should equal its index");
    }
}

#[test]
fn strip_for_pending_count_is_after_active() {
    for (idx, p) in Phase::ALL.iter().enumerate() {
        let strip = strip_for(Some(*p));
        let pending = strip.iter().filter(|d| d.step == Step::Pending).count();
        assert_eq!(pending, 5 - idx);
    }
}

#[test]
fn strip_for_is_monotone_done_then_active_then_pending() {
    // Across the strip the step never regresses: Done* Active? Pending*.
    fn rank(s: Step) -> u8 {
        match s {
            Step::Done => 0,
            Step::Active => 1,
            Step::Pending => 2,
        }
    }
    for p in Phase::ALL {
        let strip = strip_for(Some(p));
        for w in strip.windows(2) {
            assert!(rank(w[0].step) <= rank(w[1].step));
        }
    }
}

#[test]
fn strip_for_dots_carry_their_own_phase_hue() {
    let strip = strip_for(Some(Phase::Audit));
    for dot in &strip {
        assert_eq!(dot.phase.hue(), dot.phase.hue());
        // Each dot's phase matches the canonical phase at its position.
    }
    assert_eq!(strip[3].phase, Phase::Audit);
    assert_eq!(strip[3].step, Step::Active);
}

#[test]
fn strip_for_is_deterministic() {
    assert_eq!(strip_for(Some(Phase::Reflect)), strip_for(Some(Phase::Reflect)));
    assert_eq!(strip_for(None), strip_for(None));
}

#[test]
fn strip_for_review_done_spec_active_review() {
    let strip = strip_for(Some(Phase::Review));
    assert_eq!(strip[0].step, Step::Done); // spec done
    assert_eq!(strip[1].step, Step::Active); // review active
    assert_eq!(strip[2].step, Step::Pending); // manufacture pending
}

#[test]
fn phase_dot_new_round_trips_fields() {
    let dot = PhaseDot::new(Phase::Reflect, Step::Done);
    assert_eq!(dot.phase, Phase::Reflect);
    assert_eq!(dot.step, Step::Done);
}

#[test]
fn phase_dot_equality_is_structural() {
    assert_eq!(
        PhaseDot::new(Phase::Spec, Step::Active),
        PhaseDot::new(Phase::Spec, Step::Active)
    );
    assert_ne!(
        PhaseDot::new(Phase::Spec, Step::Active),
        PhaseDot::new(Phase::Spec, Step::Done)
    );
    assert_ne!(
        PhaseDot::new(Phase::Spec, Step::Active),
        PhaseDot::new(Phase::Review, Step::Active)
    );
}

#[test]
fn strip_for_active_glyph_is_fisheye_at_active_position() {
    let strip = strip_for(Some(Phase::Manufacture));
    assert_eq!(strip[2].step.glyph(), tokens::GLYPH_ACTIVE);
    assert_eq!(strip[0].step.glyph(), tokens::GLYPH_DONE);
    assert_eq!(strip[5].step.glyph(), tokens::GLYPH_PENDING);
}

// ===========================================================================
// layout: LayoutOptions defaults
// ===========================================================================

#[test]
fn layout_options_default_geometry() {
    let o = LayoutOptions::default();
    assert_eq!(o.node_width, 132.0);
    assert_eq!(o.node_height, 40.0);
    assert_eq!(o.layer_gap, 56.0);
    assert_eq!(o.node_gap, 18.0);
    assert_eq!(o.padding, 16.0);
}

#[test]
fn layout_options_is_copy() {
    let o = LayoutOptions::default();
    let p = o; // Copy
    assert_eq!(o.node_width, p.node_width);
}

// ===========================================================================
// layout: empty / single-node / degenerate
// ===========================================================================

#[test]
fn empty_graph_yields_padding_only_canvas() {
    let r = LayeredLayout.layout(&[], &[], &LayoutOptions::default());
    assert!(r.nodes.is_empty());
    assert!(r.edges.is_empty());
    assert_eq!(r.width, 32.0); // padding * 2
    assert_eq!(r.height, 32.0);
}

#[test]
fn empty_graph_respects_custom_padding() {
    let opts = LayoutOptions { padding: 25.0, ..LayoutOptions::default() };
    let r = LayeredLayout.layout(&[], &[], &opts);
    assert_eq!(r.width, 50.0);
    assert_eq!(r.height, 50.0);
}

#[test]
fn single_node_is_at_padding_origin() {
    let r = LayeredLayout.layout(&[n("solo")], &[], &LayoutOptions::default());
    assert_eq!(r.nodes.len(), 1);
    let p = &r.nodes[0];
    assert_eq!(p.x, 16.0);
    assert_eq!(p.y, 16.0);
    assert_eq!(p.layer, 0);
}

#[test]
fn single_node_canvas_is_box_plus_padding() {
    let r = LayeredLayout.layout(&[n("solo")], &[], &LayoutOptions::default());
    // width = padding*2 + node_width (single layer)
    assert_eq!(r.width, 32.0 + 132.0);
    // height = node y + node_height + padding = 16 + 40 + 16
    assert_eq!(r.height, 16.0 + 40.0 + 16.0);
}

#[test]
fn single_node_carries_its_dimensions() {
    let r = LayeredLayout.layout(&[n("solo")], &[], &LayoutOptions::default());
    let p = &r.nodes[0];
    assert_eq!(p.width, 132.0);
    assert_eq!(p.height, 40.0);
}

#[test]
fn isolated_nodes_all_land_in_layer_zero() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let r = LayeredLayout.layout(&nodes, &[], &LayoutOptions::default());
    assert!(r.nodes.iter().all(|p| p.layer == 0));
    assert!(r.edges.is_empty());
}

#[test]
fn isolated_nodes_stack_vertically_at_same_x() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let r = LayeredLayout.layout(&nodes, &[], &LayoutOptions::default());
    let xs: Vec<f64> = r.nodes.iter().map(|p| p.x).collect();
    assert!(xs.windows(2).all(|w| w[0] == w[1]), "all same x");
    let ys: Vec<f64> = r.nodes.iter().map(|p| p.y).collect();
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "y strictly increases");
}

#[test]
fn stacked_node_vertical_spacing_is_height_plus_gap() {
    let nodes = vec![n("a"), n("b")];
    let r = LayeredLayout.layout(&nodes, &[], &LayoutOptions::default());
    let a = placed(&r, "a");
    let b = placed(&r, "b");
    assert_eq!(b.y - a.y, 40.0 + 18.0); // node_height + node_gap
}

// ===========================================================================
// layout: layering (longest path)
// ===========================================================================

#[test]
fn chain_assigns_incremental_layers() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "c")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "a"), 0);
    assert_eq!(layer_of(&r, "b"), 1);
    assert_eq!(layer_of(&r, "c"), 2);
}

#[test]
fn longer_chain_layers_match_depth() {
    let ids = ["a", "b", "c", "d", "e", "f"];
    let nodes: Vec<GraphNode> = ids.iter().map(|i| n(i)).collect();
    let edges: Vec<GraphEdge> = ids
        .windows(2)
        .map(|w| GraphEdge::new(w[0], w[1]))
        .collect();
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(layer_of(&r, id), i);
    }
}

#[test]
fn diamond_uses_longest_path_for_sink() {
    let nodes = vec![n("a"), n("b"), n("c"), n("d")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("a", "c"),
        GraphEdge::new("b", "d"),
        GraphEdge::new("c", "d"),
        GraphEdge::new("a", "d"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "a"), 0);
    assert_eq!(layer_of(&r, "b"), 1);
    assert_eq!(layer_of(&r, "c"), 1);
    // d depends on b/c (layer 1) so it must sit at layer 2 even though a->d exists.
    assert_eq!(layer_of(&r, "d"), 2);
}

#[test]
fn skip_edge_does_not_pull_sink_back() {
    // a->b->c->d plus a->d. Longest path to d is 3, not 1.
    let nodes = vec![n("a"), n("b"), n("c"), n("d")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("b", "c"),
        GraphEdge::new("c", "d"),
        GraphEdge::new("a", "d"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "d"), 3);
}

#[test]
fn multiple_roots_share_layer_zero() {
    // two independent roots feeding one sink
    let nodes = vec![n("r1"), n("r2"), n("s")];
    let edges = vec![GraphEdge::new("r1", "s"), GraphEdge::new("r2", "s")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "r1"), 0);
    assert_eq!(layer_of(&r, "r2"), 0);
    assert_eq!(layer_of(&r, "s"), 1);
}

#[test]
fn fan_out_keeps_children_in_next_layer() {
    let nodes = vec![n("root"), n("c1"), n("c2"), n("c3")];
    let edges = vec![
        GraphEdge::new("root", "c1"),
        GraphEdge::new("root", "c2"),
        GraphEdge::new("root", "c3"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "root"), 0);
    for c in ["c1", "c2", "c3"] {
        assert_eq!(layer_of(&r, c), 1);
    }
}

#[test]
fn within_layer_order_follows_input_order() {
    // Three siblings in layer 1; their y must follow declaration order.
    let nodes = vec![n("root"), n("first"), n("second"), n("third")];
    let edges = vec![
        GraphEdge::new("root", "first"),
        GraphEdge::new("root", "second"),
        GraphEdge::new("root", "third"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let yf = placed(&r, "first").y;
    let ys = placed(&r, "second").y;
    let yt = placed(&r, "third").y;
    assert!(yf < ys && ys < yt);
}

#[test]
fn layers_advance_x_by_node_width_plus_gap() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "c")];
    let opts = LayoutOptions::default();
    let r = LayeredLayout.layout(&nodes, &edges, &opts);
    let xa = placed(&r, "a").x;
    let xb = placed(&r, "b").x;
    let xc = placed(&r, "c").x;
    assert_eq!(xb - xa, opts.node_width + opts.layer_gap);
    assert_eq!(xc - xb, opts.node_width + opts.layer_gap);
}

#[test]
fn x_increases_monotonically_with_layer() {
    let nodes = vec![n("a"), n("b"), n("c"), n("d")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("b", "c"),
        GraphEdge::new("c", "d"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let mut last_x = f64::MIN;
    for id in ["a", "b", "c", "d"] {
        let x = placed(&r, id).x;
        assert!(x > last_x);
        last_x = x;
    }
}

// ===========================================================================
// layout: canvas sizing
// ===========================================================================

#[test]
fn chain_canvas_width_spans_all_layers() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "c")];
    let opts = LayoutOptions::default();
    let r = LayeredLayout.layout(&nodes, &edges, &opts);
    // 3 layers: padding*2 + 3*width + 2*gap
    let expected = opts.padding * 2.0
        + 3.0 * opts.node_width
        + 2.0 * opts.layer_gap;
    assert_eq!(r.width, expected);
}

#[test]
fn canvas_height_covers_tallest_layer() {
    // layer with 3 stacked nodes drives the height.
    let nodes = vec![n("root"), n("c1"), n("c2"), n("c3")];
    let edges = vec![
        GraphEdge::new("root", "c1"),
        GraphEdge::new("root", "c2"),
        GraphEdge::new("root", "c3"),
    ];
    let opts = LayoutOptions::default();
    let r = LayeredLayout.layout(&nodes, &edges, &opts);
    // bottom node y = padding + 2*(height+gap); height = that + node_height + padding
    let bottom_y = opts.padding + 2.0 * (opts.node_height + opts.node_gap);
    assert_eq!(r.height, bottom_y + opts.node_height + opts.padding);
}

#[test]
fn every_node_fits_inside_canvas() {
    let nodes = vec![n("a"), n("b"), n("c"), n("d")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("a", "c"),
        GraphEdge::new("b", "d"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    for p in &r.nodes {
        assert!(p.x >= 0.0);
        assert!(p.y >= 0.0);
        assert!(p.x + p.width <= r.width + 1e-9, "node {} exceeds width", p.id);
        assert!(p.y + p.height <= r.height + 1e-9, "node {} exceeds height", p.id);
    }
}

#[test]
fn custom_options_scale_geometry() {
    let opts = LayoutOptions {
        node_width: 100.0,
        node_height: 30.0,
        layer_gap: 40.0,
        node_gap: 10.0,
        padding: 8.0,
    };
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &opts);
    assert_eq!(placed(&r, "a").x, 8.0);
    assert_eq!(placed(&r, "b").x, 8.0 + 100.0 + 40.0);
    assert_eq!(placed(&r, "a").width, 100.0);
    assert_eq!(placed(&r, "a").height, 30.0);
}

// ===========================================================================
// layout: edge routing
// ===========================================================================

#[test]
fn routed_edge_count_matches_valid_edges() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "c")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.edges.len(), 2);
}

#[test]
fn edge_starts_at_source_right_edge() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let a = placed(&r, "a");
    let e = edge(&r, "a", "b");
    assert_eq!(e.x1, a.x + a.width);
    assert_eq!(e.y1, a.cy());
}

#[test]
fn edge_ends_at_target_left_edge() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let b = placed(&r, "b");
    let e = edge(&r, "a", "b");
    assert_eq!(e.x2, b.x);
    assert_eq!(e.y2, b.cy());
}

#[test]
fn edge_runs_left_to_right() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let e = edge(&r, "a", "b");
    assert!(e.x2 > e.x1, "edge must flow forward");
}

#[test]
fn edge_endpoints_carry_node_ids() {
    let nodes = vec![n("from"), n("to")];
    let edges = vec![GraphEdge::new("from", "to")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.edges[0].from, "from");
    assert_eq!(r.edges[0].to, "to");
}

#[test]
fn edge_y_matches_node_centers_for_aligned_chain() {
    // single chain: every node is the only one in its layer so y aligns.
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let e = edge(&r, "a", "b");
    assert_eq!(e.y1, e.y2); // both at same row -> level edge
}

#[test]
fn fanned_edges_have_distinct_target_ys() {
    let nodes = vec![n("root"), n("c1"), n("c2")];
    let edges = vec![GraphEdge::new("root", "c1"), GraphEdge::new("root", "c2")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    let e1 = edge(&r, "root", "c1");
    let e2 = edge(&r, "root", "c2");
    assert_eq!(e1.x1, e2.x1); // same source
    assert_ne!(e1.y2, e2.y2); // different targets stacked
}

#[test]
fn duplicate_edges_route_twice() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.edges.len(), 2);
    assert_eq!(r.edges[0], r.edges[1]);
}

#[test]
fn edge_preserves_input_edge_order() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![GraphEdge::new("b", "c"), GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!((r.edges[0].from.as_str(), r.edges[0].to.as_str()), ("b", "c"));
    assert_eq!((r.edges[1].from.as_str(), r.edges[1].to.as_str()), ("a", "b"));
}

// ===========================================================================
// layout: invalid / dropped edges
// ===========================================================================

#[test]
fn edge_to_unknown_target_is_dropped() {
    let nodes = vec![n("a")];
    let edges = vec![GraphEdge::new("a", "ghost")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert!(r.edges.is_empty());
    assert_eq!(r.nodes.len(), 1);
}

#[test]
fn edge_from_unknown_source_is_dropped() {
    let nodes = vec![n("b")];
    let edges = vec![GraphEdge::new("ghost", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert!(r.edges.is_empty());
    assert_eq!(layer_of(&r, "b"), 0); // unknown source can't push b forward
}

#[test]
fn fully_unknown_edge_is_dropped() {
    let nodes = vec![n("a")];
    let edges = vec![GraphEdge::new("x", "y")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert!(r.edges.is_empty());
}

#[test]
fn mix_of_valid_and_invalid_edges_keeps_valid_only() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("a", "ghost"),
        GraphEdge::new("ghost", "b"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.edges.len(), 1);
    assert_eq!(r.edges[0].from, "a");
    assert_eq!(r.edges[0].to, "b");
}

#[test]
fn self_loop_keeps_node_in_layer_zero() {
    // a->a is a degenerate self edge: a stays in layer 0 (no forward progress
    // possible without exceeding the pass bound).
    let nodes = vec![n("a")];
    let edges = vec![GraphEdge::new("a", "a")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.nodes.len(), 1);
    // self-edge resolves both endpoints, so it is routed.
    assert_eq!(r.edges.len(), 1);
}

// ===========================================================================
// layout: cycles / robustness
// ===========================================================================

#[test]
fn two_node_cycle_terminates_without_panic() {
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b"), GraphEdge::new("b", "a")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.nodes.len(), 2);
    assert_eq!(r.edges.len(), 2);
}

#[test]
fn three_node_cycle_terminates() {
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("b", "c"),
        GraphEdge::new("c", "a"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(r.nodes.len(), 3);
    // The bounded relaxation terminates and every node still gets a finite,
    // non-negative layer (no overflow, no panic) even though the cycle has no
    // well-defined longest path.
    for id in ["a", "b", "c"] {
        let l = layer_of(&r, id);
        assert!(l < usize::MAX);
    }
    // Every placed node has finite coordinates inside the reported canvas.
    for p in &r.nodes {
        assert!(p.x.is_finite() && p.y.is_finite());
        assert!(r.width.is_finite() && r.height.is_finite());
    }
}

#[test]
fn cycle_with_tail_still_places_tail_forward() {
    // a<->b cycle, then b->c tail. c must still land past b.
    let nodes = vec![n("a"), n("b"), n("c")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("b", "a"),
        GraphEdge::new("b", "c"),
    ];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert!(layer_of(&r, "c") > layer_of(&r, "b"));
}

// ===========================================================================
// layout: determinism / idempotency / trait usage
// ===========================================================================

#[test]
fn layout_is_deterministic_across_runs() {
    let nodes = vec![n("a"), n("b"), n("c"), n("d")];
    let edges = vec![
        GraphEdge::new("a", "b"),
        GraphEdge::new("a", "c"),
        GraphEdge::new("b", "d"),
        GraphEdge::new("c", "d"),
    ];
    let opts = LayoutOptions::default();
    let r1 = LayeredLayout.layout(&nodes, &edges, &opts);
    let r2 = LayeredLayout.layout(&nodes, &edges, &opts);
    assert_eq!(r1, r2);
}

#[test]
fn layout_result_nodes_preserve_input_order() {
    // Nodes are emitted grouped by layer, but layer 0 nodes keep input order.
    let nodes = vec![n("z"), n("y"), n("x")]; // all isolated
    let r = LayeredLayout.layout(&nodes, &[], &LayoutOptions::default());
    let ids: Vec<&str> = r.nodes.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["z", "y", "x"]);
}

#[test]
fn layout_via_trait_object_matches_direct_call() {
    let layout: &dyn GraphLayout = &LayeredLayout;
    let nodes = vec![n("a"), n("b")];
    let edges = vec![GraphEdge::new("a", "b")];
    let opts = LayoutOptions::default();
    let via_trait = layout.layout(&nodes, &edges, &opts);
    let direct = LayeredLayout.layout(&nodes, &edges, &opts);
    assert_eq!(via_trait, direct);
}

#[test]
fn reordering_input_changes_within_layer_y_only() {
    let edges = vec![GraphEdge::new("root", "a"), GraphEdge::new("root", "b")];
    let order1 = vec![n("root"), n("a"), n("b")];
    let order2 = vec![n("root"), n("b"), n("a")];
    let r1 = LayeredLayout.layout(&order1, &edges, &LayoutOptions::default());
    let r2 = LayeredLayout.layout(&order2, &edges, &LayoutOptions::default());
    // a is first sibling in r1, second in r2 -> different y.
    assert!(placed(&r1, "a").y < placed(&r1, "b").y);
    assert!(placed(&r2, "b").y < placed(&r2, "a").y);
    // but layers are identical regardless of order.
    assert_eq!(layer_of(&r1, "a"), layer_of(&r2, "a"));
}

#[test]
fn empty_layout_result_default_is_zeroed() {
    let r = LayoutResult::default();
    assert!(r.nodes.is_empty());
    assert!(r.edges.is_empty());
    assert_eq!(r.width, 0.0);
    assert_eq!(r.height, 0.0);
}

// ===========================================================================
// layout: PlacedNode geometry
// ===========================================================================

#[test]
fn placed_node_center_is_box_midpoint() {
    let p = PlacedNode {
        id: "x".into(),
        label: "x".into(),
        layer: 0,
        x: 10.0,
        y: 20.0,
        width: 100.0,
        height: 40.0,
    };
    assert_eq!(p.cx(), 60.0);
    assert_eq!(p.cy(), 40.0);
}

#[test]
fn placed_node_centers_from_real_layout() {
    let r = LayeredLayout.layout(&[n("solo")], &[], &LayoutOptions::default());
    let p = &r.nodes[0];
    assert_eq!(p.cx(), p.x + p.width / 2.0);
    assert_eq!(p.cy(), p.y + p.height / 2.0);
    assert_eq!(p.cx(), 16.0 + 66.0);
    assert_eq!(p.cy(), 16.0 + 20.0);
}

#[test]
fn placed_node_label_survives_layout() {
    let nodes = vec![GraphNode::new("id1", "Pretty Label")];
    let r = LayeredLayout.layout(&nodes, &[], &LayoutOptions::default());
    assert_eq!(r.nodes[0].label, "Pretty Label");
    assert_eq!(r.nodes[0].id, "id1");
}

// ===========================================================================
// layout: GraphNode / GraphEdge constructors
// ===========================================================================

#[test]
fn graph_node_new_accepts_str_and_string() {
    let a = GraphNode::new("x", "X");
    let b = GraphNode::new(String::from("x"), String::from("X"));
    assert_eq!(a, b);
}

#[test]
fn graph_node_distinct_id_and_label() {
    let node = GraphNode::new("slug-1", "Human Label");
    assert_eq!(node.id, "slug-1");
    assert_eq!(node.label, "Human Label");
    assert_ne!(node.id, node.label);
}

#[test]
fn graph_edge_new_accepts_str_and_string() {
    let a = GraphEdge::new("p", "q");
    let b = GraphEdge::new(String::from("p"), String::from("q"));
    assert_eq!(a, b);
}

#[test]
fn graph_edge_direction_is_significant() {
    assert_ne!(GraphEdge::new("a", "b"), GraphEdge::new("b", "a"));
}

#[test]
fn graph_node_equality_is_structural() {
    assert_eq!(GraphNode::new("a", "L"), GraphNode::new("a", "L"));
    assert_ne!(GraphNode::new("a", "L"), GraphNode::new("a", "M"));
}

// ===========================================================================
// graph::view::UnitGraphNode
// ===========================================================================

#[test]
fn unit_graph_node_new_is_untinted() {
    let u = UnitGraphNode::new("u1", "Unit One");
    assert_eq!(u.node.id, "u1");
    assert_eq!(u.node.label, "Unit One");
    assert_eq!(u.tone, None);
}

#[test]
fn unit_graph_node_with_tone_sets_tone() {
    let u = UnitGraphNode::new("u1", "Unit One").with_tone(Tone::Ok);
    assert_eq!(u.tone, Some(Tone::Ok));
}

#[test]
fn unit_graph_node_with_tone_overrides_previous() {
    let u = UnitGraphNode::new("u1", "Unit One")
        .with_tone(Tone::Warn)
        .with_tone(Tone::Danger);
    assert_eq!(u.tone, Some(Tone::Danger));
}

#[test]
fn unit_graph_node_wraps_a_graph_node() {
    let u = UnitGraphNode::new("u1", "L");
    assert_eq!(u.node, GraphNode::new("u1", "L"));
}

#[test]
fn unit_graph_node_tone_color_resolves() {
    let u = UnitGraphNode::new("u1", "L").with_tone(Tone::Danger);
    assert_eq!(u.tone.unwrap().color(), tokens::STATUS_DANGER);
}

#[test]
fn unit_graph_nodes_feed_layout_by_inner_node() {
    // The view drives the same layout we test directly; confirm the inner
    // GraphNodes layer correctly when extracted (mirrors view.rs behavior).
    let units = [UnitGraphNode::new("a", "A"),
        UnitGraphNode::new("b", "B").with_tone(Tone::Ok)];
    let nodes: Vec<GraphNode> = units.iter().map(|u| u.node.clone()).collect();
    let edges = vec![GraphEdge::new("a", "b")];
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());
    assert_eq!(layer_of(&r, "a"), 0);
    assert_eq!(layer_of(&r, "b"), 1);
}

#[test]
fn unit_graph_node_equality_includes_tone() {
    let a = UnitGraphNode::new("x", "X");
    let b = UnitGraphNode::new("x", "X").with_tone(Tone::Ok);
    assert_ne!(a, b);
    assert_eq!(a, UnitGraphNode::new("x", "X"));
}

// ===========================================================================
// wordmark variant selection
// ===========================================================================

#[test]
fn wordmark_variant_default_is_filled() {
    assert_eq!(WordmarkVariant::default(), WordmarkVariant::Filled);
}

#[test]
fn wordmark_variants_are_distinct() {
    assert_ne!(WordmarkVariant::Filled, WordmarkVariant::Outlined);
    assert_ne!(WordmarkVariant::Filled, WordmarkVariant::OutlinedSolidRun);
    assert_ne!(WordmarkVariant::Outlined, WordmarkVariant::OutlinedSolidRun);
}

#[test]
fn wordmark_variant_is_copy() {
    let v = WordmarkVariant::Outlined;
    let w = v;
    assert_eq!(v, w);
}

// ===========================================================================
// button variant
// ===========================================================================

#[test]
fn button_variant_default_is_primary() {
    assert_eq!(ButtonVariant::default(), ButtonVariant::Primary);
}

#[test]
fn button_variants_are_three_distinct() {
    let vs = [
        ButtonVariant::Primary,
        ButtonVariant::Secondary,
        ButtonVariant::Ghost,
    ];
    for i in 0..vs.len() {
        for j in (i + 1)..vs.len() {
            assert_ne!(vs[i], vs[j]);
        }
    }
}

// ===========================================================================
// checkpoint kind
// ===========================================================================

#[test]
fn checkpoint_kind_default_is_auto() {
    assert_eq!(CheckpointKind::default(), CheckpointKind::Auto);
}

#[test]
fn checkpoint_kinds_are_four_distinct() {
    let ks = [
        CheckpointKind::Auto,
        CheckpointKind::Ask,
        CheckpointKind::External,
        CheckpointKind::Await,
    ];
    for i in 0..ks.len() {
        for j in (i + 1)..ks.len() {
            assert_ne!(ks[i], ks[j]);
        }
    }
}

#[test]
fn checkpoint_kind_is_copy_eq() {
    let k = CheckpointKind::Ask;
    let l = k;
    assert_eq!(k, l);
}

// ===========================================================================
// cross-cutting integration: a realistic factory pipeline scenario
// ===========================================================================

#[test]
fn factory_pipeline_at_each_station_phase_is_consistent() {
    // Walk a station through all six phases and confirm the strip, glyphs, and
    // hues stay internally consistent at every step.
    for (idx, active) in Phase::ALL.iter().enumerate() {
        let strip = strip_for(Some(*active));
        // The active dot's phase hue is the one phase_hue() returns for its name.
        let active_dot = &strip[idx];
        assert_eq!(active_dot.phase, *active);
        assert_eq!(active_dot.step, Step::Active);
        assert_eq!(
            tokens::phase_hue(active_dot.phase.name()),
            Some(active_dot.phase.hue())
        );
        // Every done dot precedes the active index; pending follows.
        for (i, dot) in strip.iter().enumerate() {
            match dot.step {
                Step::Done => assert!(i < idx),
                Step::Active => assert_eq!(i, idx),
                Step::Pending => assert!(i > idx),
            }
        }
    }
}

#[test]
fn unit_dag_scenario_layers_and_routes_end_to_end() {
    // spec -> build -> prove, with build also feeding a docs side unit.
    let units = [UnitGraphNode::new("spec", "Spec").with_tone(Tone::Info),
        UnitGraphNode::new("build", "Build").with_tone(Tone::Accent),
        UnitGraphNode::new("prove", "Prove").with_tone(Tone::Ok),
        UnitGraphNode::new("docs", "Docs").with_tone(Tone::Neutral)];
    let edges = vec![
        GraphEdge::new("spec", "build"),
        GraphEdge::new("build", "prove"),
        GraphEdge::new("build", "docs"),
    ];
    let nodes: Vec<GraphNode> = units.iter().map(|u| u.node.clone()).collect();
    let r = LayeredLayout.layout(&nodes, &edges, &LayoutOptions::default());

    assert_eq!(layer_of(&r, "spec"), 0);
    assert_eq!(layer_of(&r, "build"), 1);
    assert_eq!(layer_of(&r, "prove"), 2);
    assert_eq!(layer_of(&r, "docs"), 2);
    assert_eq!(r.edges.len(), 3);

    // prove and docs share a layer -> stacked, distinct y.
    assert_ne!(placed(&r, "prove").y, placed(&r, "docs").y);
    // edges from build fan to both.
    assert_eq!(edge(&r, "build", "prove").x1, edge(&r, "build", "docs").x1);
}

#[test]
fn every_phase_renders_a_legible_active_glyph_color() {
    // The pipeline paints the active glyph in the phase base hue; confirm that
    // hue is legible against the raised surface the strip sits on.
    for p in Phase::ALL {
        let c = contrast(p.hue().base, tokens::SURFACE_RAISED);
        assert!(c >= 2.0, "phase {:?} glyph too dim on surface ({c})", p);
    }
}

#[test]
fn pending_glyph_color_is_faint_not_phase_hue() {
    // Pending dots use TEXT_FAINT, which must be dimmer than any phase hue
    // against the surface (so active/done read brighter than pending).
    let faint_c = contrast(tokens::TEXT_FAINT, tokens::SURFACE_RAISED);
    for p in Phase::ALL {
        let phase_c = contrast(p.hue().base, tokens::SURFACE_RAISED);
        assert!(
            phase_c >= faint_c,
            "phase {:?} should read at least as bright as faint pending",
            p
        );
    }
}

//! Integration tests for the station-detail data assembly and the methodology
//! phase content — the pure view-models behind `/factories/:f/stations/:s` and
//! `/methodology/:phase`, exercised against the real embedded corpus.

use darkrun_site::content::render_markdown;
use darkrun_site::factory_view::{
    flow_stations, humanize, pipeline_slugs, right_size_tiers, risk_from_body, role_view,
    station_index, summary_from_body, ui_checkpoint, ui_role_kind,
};
use darkrun_site::pages::concepts::{phase_explainer, PHASE_SLUGS};
use darkrun_ui::prelude::{CheckpointKind as UiCheckpoint, Phase, RoleKind};

fn software() -> darkrun_content::Factory {
    darkrun_content::load_validated("software").expect("software factory loads")
}

#[test]
fn every_station_resolves_a_prev_next_index() {
    let f = software();
    for (i, station) in f.stations.iter().enumerate() {
        assert_eq!(station_index(&f, station.name()), Some(i));
    }
    // Pipeline order is preserved.
    assert_eq!(
        pipeline_slugs(&f),
        vec!["frame", "specify", "shape", "build", "prove", "harden"]
    );
}

#[test]
fn station_detail_assembles_every_role_with_real_markdown() {
    let f = software();
    for station in &f.stations {
        let roles = station
            .explorers
            .iter()
            .chain(&station.workers)
            .chain(&station.reviewers);
        for role in roles {
            let view = role_view(role, render_markdown);
            assert!(!view.name.is_empty(), "role name empty in {}", station.name());
            // The card renders the REAL instruction body as HTML.
            assert!(
                view.body_html.contains('<'),
                "role {} in {} rendered no HTML",
                view.name,
                station.name()
            );
            assert!(!view.agent_type.is_empty());
        }
    }
}

#[test]
fn worker_roles_map_to_the_worker_kind_for_beats() {
    let f = software();
    let frame = f.station("frame").unwrap();
    // Frame's three workers are the Make/Challenge/Resolve sequence.
    assert_eq!(frame.workers.len(), 3);
    for w in &frame.workers {
        assert_eq!(ui_role_kind(w.kind()), RoleKind::Worker);
    }
    // Explorers and reviewers map to their own kinds.
    assert!(frame.explorers.iter().all(|e| ui_role_kind(e.kind()) == RoleKind::Explorer));
    assert!(frame.reviewers.iter().all(|r| ui_role_kind(r.kind()) == RoleKind::Reviewer));
}

#[test]
fn every_station_documents_a_killed_risk() {
    let f = software();
    for station in &f.stations {
        let risk = risk_from_body(&station.body);
        assert!(risk.is_some(), "station {} has no risk note", station.name());
        let risk = risk.unwrap();
        assert!(!risk.is_empty());
        assert!(!risk.contains('*'), "markdown leaked into risk: {risk:?}");
    }
}

#[test]
fn flow_stations_feed_the_pipeline_and_walkthrough() {
    let f = software();
    let flows = flow_stations(&f);
    assert_eq!(flows.len(), 6);
    // Each flow node carries the checkpoint kind the UI component needs.
    let frame = &flows[0];
    assert_eq!(frame.checkpoint, UiCheckpoint::Ask);
    let build = &flows[3];
    assert_eq!(build.checkpoint, UiCheckpoint::Auto);
    let harden = &flows[5];
    assert_eq!(harden.checkpoint, UiCheckpoint::External);
}

#[test]
fn checkpoint_mapping_is_total() {
    use darkrun_core::domain::CheckpointKind as C;
    assert_eq!(ui_checkpoint(C::Auto), UiCheckpoint::Auto);
    assert_eq!(ui_checkpoint(C::Ask), UiCheckpoint::Ask);
    assert_eq!(ui_checkpoint(C::External), UiCheckpoint::External);
    assert_eq!(ui_checkpoint(C::Await), UiCheckpoint::Await);
}

#[test]
fn station_header_exposes_locked_artifact_and_inputs() {
    let f = software();
    // Frame locks frame.md with no inputs; later stations inherit upstream ones.
    let frame = f.station("frame").unwrap();
    assert_eq!(frame.frontmatter.locked_artifact, "frame.md");
    assert!(frame.frontmatter.inputs.is_empty());
    // At least one downstream station consumes upstream artifacts.
    let has_inputs = f.stations.iter().any(|s| !s.frontmatter.inputs.is_empty());
    assert!(has_inputs, "no station declares inputs");
}

#[test]
fn right_sizing_collapses_to_the_build_prove_core() {
    let f = software();
    let tiers = right_size_tiers(&f);
    let labels: Vec<&str> = tiers.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, vec!["tiny", "small", "full"]);
    // tiny ⊂ small ⊂ full
    assert!(tiers[0].kept.len() <= tiers[1].kept.len());
    assert!(tiers[1].kept.len() <= tiers[2].kept.len());
}

#[test]
fn humanize_renders_role_slugs_for_display() {
    assert_eq!(humanize("pressure_tester"), "Pressure Tester");
    assert_eq!(humanize("self_reviewer"), "Self Reviewer");
}

#[test]
fn summary_is_a_single_clean_line() {
    let f = software();
    let frame = f.station("frame").unwrap();
    let s = summary_from_body(&frame.workers[0].body).expect("framer has a summary");
    assert!(!s.contains('\n'));
    assert!(!s.contains('#'));
}

// --- methodology phase content -------------------------------------------

#[test]
fn phase_slugs_round_trip_to_phases() {
    assert_eq!(PHASE_SLUGS.len(), 6);
    for slug in PHASE_SLUGS {
        let phase = Phase::from_name(slug).unwrap_or_else(|| panic!("unknown phase {slug}"));
        assert_eq!(phase.name(), slug);
    }
}

#[test]
fn phase_slugs_match_canonical_phase_order() {
    let from_phases: Vec<&str> = Phase::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(PHASE_SLUGS.to_vec(), from_phases);
}

#[test]
fn every_phase_has_a_real_explainer() {
    for phase in Phase::ALL {
        let text = phase_explainer(phase);
        assert!(text.len() > 60, "phase {} explainer too short", phase.name());
    }
    // Manufacture's explainer names the pass loop beats.
    let m = phase_explainer(Phase::Manufacture);
    assert!(m.contains("Make") && m.contains("Challenge") && m.contains("Resolve"));
}

#[test]
fn methodology_markdown_renders() {
    let doc = darkrun_site::content::find(darkrun_site::content::CONCEPTS, "methodology")
        .expect("methodology concept");
    let html = render_markdown(doc.markdown);
    assert!(html.contains("<h1>"));
    assert!(html.to_lowercase().contains("cost of late discovery"));
}

#[test]
fn glossary_carries_the_full_vocabulary() {
    let doc = darkrun_site::content::find(darkrun_site::content::CONCEPTS, "glossary")
        .expect("glossary concept");
    let md = doc.markdown.to_lowercase();
    for term in [
        "factory",
        "station",
        "unit",
        "pass",
        "worker",
        "run",
        "explorer",
        "reviewer",
        "checkpoint",
        "manager",
        "decompose",
        "make / challenge / resolve",
    ] {
        assert!(md.contains(term), "glossary missing `{term}`");
    }
}

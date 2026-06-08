//! Integration tests for the pure logic behind the `/factories` pages:
//! the phase-index mapping and the corpus data the tiles + detail view read out
//! of `darkrun-content`.

use darkrun_site::pages::factories::phase_for_index;
use darkrun_ui::prelude::Phase;

#[test]
fn phase_for_index_maps_the_first_six_positions() {
    for (i, expected) in Phase::ALL.iter().enumerate() {
        assert_eq!(phase_for_index(i), Some(*expected));
    }
}

#[test]
fn phase_for_index_is_none_past_the_six_phases() {
    assert_eq!(phase_for_index(6), None);
    assert_eq!(phase_for_index(7), None);
    assert_eq!(phase_for_index(usize::MAX), None);
}

#[test]
fn phase_for_index_zero_is_spec() {
    assert_eq!(phase_for_index(0), Some(Phase::Spec));
}

#[test]
fn phase_for_index_five_is_checkpoint() {
    assert_eq!(phase_for_index(5), Some(Phase::Checkpoint));
}

#[test]
fn the_software_factory_is_listed() {
    assert!(darkrun_content::list_factories().contains(&"software".to_string()));
}

#[test]
fn every_listed_factory_loads_and_validates() {
    // The index tile path calls load_validated for each slug; all must succeed.
    for slug in darkrun_content::list_factories() {
        assert!(
            darkrun_content::load_validated(&slug).is_ok(),
            "factory {slug} failed validation"
        );
    }
}

#[test]
fn factory_detail_surfaces_panel_reads_declared_surfaces() {
    // The detail page renders a surface badge per declared surface.
    let software = darkrun_content::load_validated("software").unwrap();
    assert_eq!(software.frontmatter.surfaces.len(), 8);
    // software offers the library/api surfaces a library run classifies into.
    assert!(software.frontmatter.surfaces.iter().any(|s| s == "library"));
    assert!(software.frontmatter.surfaces.iter().any(|s| s == "api"));
}

#[test]
fn loading_an_unknown_factory_is_an_error() {
    // The FactoryTile/FactoryDetail error arm renders this message.
    let err = darkrun_content::load_validated("no-such-factory").unwrap_err();
    let msg = err.to_string();
    assert!(!msg.is_empty());
    assert!(msg.to_lowercase().contains("factory") || msg.to_lowercase().contains("not found"));
}

#[test]
fn software_factory_exposes_the_fields_the_tile_renders() {
    let f = darkrun_content::load_validated("software").unwrap();
    // Tile reads description, category, and station count.
    assert!(!f.frontmatter.description.is_empty());
    assert!(!f.stations.is_empty());
    assert_eq!(f.name(), "software");
}

#[test]
fn software_factory_has_six_stations_each_mappable_to_a_phase() {
    let f = darkrun_content::load_validated("software").unwrap();
    assert_eq!(f.stations.len(), 6);
    for (i, _station) in f.stations.iter().enumerate() {
        // The detail view's accent stripe uses phase_for_index; all six resolve.
        assert!(phase_for_index(i).is_some(), "station {i} has no phase");
    }
}

#[test]
fn detail_view_renders_factory_body_to_html() {
    let f = darkrun_content::load_validated("software").unwrap();
    let html = darkrun_site::content::render_markdown(&f.body);
    assert!(!html.trim().is_empty());
}

#[test]
fn each_station_exposes_named_roles_for_the_roster_rows() {
    let f = darkrun_content::load_validated("software").unwrap();
    for station in &f.stations {
        let total = station.explorers.len() + station.workers.len() + station.reviewers.len();
        // Every station the corpus ships has at least one role to render.
        assert!(total > 0, "station {} has no roles", station.name());
        for r in station.explorers.iter().chain(&station.workers).chain(&station.reviewers) {
            assert!(!r.name().is_empty());
        }
    }
}

#[test]
fn station_names_are_non_empty_and_capitalizable() {
    let f = darkrun_content::load_validated("software").unwrap();
    for station in &f.stations {
        assert!(!station.name().is_empty());
    }
}


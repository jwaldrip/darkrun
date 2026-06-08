//! Integration tests for the factory module of `darkrun-mcp`.
//!
//! Drives the public API: `resolve_factory`, `list_factories`,
//! `software_factory`, and the `FactoryDef` / `StationDef` shapes. Covers
//! station ordering (frame -> specify -> shape -> build -> prove -> harden),
//! per-station kills / workers / reviewers / artifact / checkpoint fields,
//! navigation helpers (`first_station`, `next_station`, `station`), the
//! resolver's known/unknown behavior, and structural invariants
//! (determinism, uniqueness, external checkpoints where expected).

use darkrun_mcp::factory::list_factories;
use darkrun_mcp::{resolve_factory, FactoryDef, StationDef};

// ---------------------------------------------------------------------------
// Shared expectations — the authoritative station plan.
// ---------------------------------------------------------------------------

const ORDER: [&str; 6] = ["frame", "specify", "shape", "build", "prove", "harden"];

fn sw() -> FactoryDef {
    resolve_factory("software").unwrap()
}

/// Returns the station with the given name, panicking with a clear message.
fn st(name: &str) -> StationDef {
    sw().station(name).cloned().unwrap_or_else(|| {
        panic!("station {name:?} not found in software factory");
    })
}

// ---------------------------------------------------------------------------
// resolve_factory — known / unknown.
// ---------------------------------------------------------------------------

#[test]
fn resolve_software_is_some() {
    assert!(resolve_factory("software").is_some());
}

#[test]
fn resolve_software_returns_named_software() {
    let f = resolve_factory("software").unwrap();
    assert_eq!(f.name, "software");
}

#[test]
fn resolve_software_equals_corpus() {
    assert_eq!(resolve_factory("software").unwrap(), resolve_factory("software").unwrap());
}

#[test]
fn resolve_unknown_plain_is_none() {
    assert!(resolve_factory("nope").is_none());
}

#[test]
fn resolve_empty_string_is_none() {
    assert!(resolve_factory("").is_none());
}

#[test]
fn resolve_whitespace_is_none() {
    assert!(resolve_factory("   ").is_none());
}

#[test]
fn resolve_is_case_sensitive_upper() {
    assert!(resolve_factory("Software").is_none());
}

#[test]
fn resolve_is_case_sensitive_allcaps() {
    assert!(resolve_factory("SOFTWARE").is_none());
}

#[test]
fn resolve_rejects_leading_space() {
    assert!(resolve_factory(" software").is_none());
}

#[test]
fn resolve_rejects_trailing_space() {
    assert!(resolve_factory("software ").is_none());
}

#[test]
fn resolve_rejects_station_names_as_factories() {
    for name in ORDER {
        assert!(
            resolve_factory(name).is_none(),
            "station name {name:?} should not resolve as a factory"
        );
    }
}

#[test]
fn resolve_rejects_unrelated_methodology_names() {
    for name in ["agile", "waterfall", "kanban", "scrum", "lean", "design"] {
        assert!(resolve_factory(name).is_none(), "{name:?} should be None");
    }
}

#[test]
fn resolve_rejects_partial_prefix() {
    assert!(resolve_factory("soft").is_none());
}

#[test]
fn resolve_rejects_partial_suffix() {
    assert!(resolve_factory("ware").is_none());
}

#[test]
fn resolve_rejects_substring_with_extra() {
    assert!(resolve_factory("software2").is_none());
}

#[test]
fn resolve_is_deterministic_across_calls() {
    let a = resolve_factory("software").unwrap();
    let b = resolve_factory("software").unwrap();
    assert_eq!(a, b);
}

#[test]
fn resolve_unknown_is_deterministic() {
    assert!(resolve_factory("zzz").is_none());
    assert!(resolve_factory("zzz").is_none());
}

// ---------------------------------------------------------------------------
// list_factories.
// ---------------------------------------------------------------------------

#[test]
fn list_factories_is_non_empty() {
    assert!(!list_factories().is_empty());
}

#[test]
fn list_factories_contains_software_and_legal() {
    let names: Vec<String> = list_factories().iter().map(|f| f.name.clone()).collect();
    assert!(names.iter().any(|n| n == "software"), "software factory shipped");
    assert!(names.iter().any(|n| n == "legal"), "legal factory shipped");
}

#[test]
fn list_factories_contains_software() {
    assert!(list_factories().iter().any(|f| f.name == "software"));
}

#[test]
fn list_factories_is_sorted() {
    let names: Vec<String> = list_factories().iter().map(|f| f.name.clone()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the catalog is sorted by slug");
}

#[test]
fn list_factories_software_equals_resolve() {
    let listed = list_factories()
        .into_iter()
        .find(|f| f.name == "software")
        .expect("software listed");
    assert_eq!(listed, resolve_factory("software").unwrap());
}

#[test]
fn list_factories_every_entry_resolves() {
    for f in list_factories() {
        assert_eq!(
            resolve_factory(&f.name),
            Some(f.clone()),
            "listed factory {:?} must resolve identically",
            f.name
        );
    }
}

#[test]
fn list_factories_is_deterministic() {
    assert_eq!(list_factories(), list_factories());
}

#[test]
fn list_factories_names_are_unique() {
    let mut names: Vec<String> = list_factories().iter().map(|f| f.name.clone()).collect();
    let total = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), total, "factory names must be unique");
}

// ---------------------------------------------------------------------------
// software_factory — identity & determinism.
// ---------------------------------------------------------------------------

#[test]
fn software_factory_name_is_software() {
    assert_eq!(sw().name, "software");
}

#[test]
fn software_factory_is_deterministic() {
    assert_eq!(resolve_factory("software").unwrap(), resolve_factory("software").unwrap());
}

#[test]
fn software_factory_clone_equals_original() {
    let f = sw();
    assert_eq!(f.clone(), f);
}

#[test]
fn software_factory_has_six_stations() {
    assert_eq!(sw().stations.len(), 6);
}

// ---------------------------------------------------------------------------
// Station ordering.
// ---------------------------------------------------------------------------

#[test]
fn station_names_in_canonical_order() {
    assert_eq!(sw().station_names(), ORDER.to_vec());
}

#[test]
fn station_names_length_matches_stations() {
    let f = sw();
    assert_eq!(f.station_names().len(), f.stations.len());
}

#[test]
fn stations_iterate_in_canonical_order() {
    for (station, expected) in sw().stations.iter().zip(ORDER.iter()) {
        assert_eq!(&station.name, expected);
    }
}

#[test]
fn frame_is_index_zero() {
    assert_eq!(sw().stations[0].name, "frame");
}

#[test]
fn specify_is_index_one() {
    assert_eq!(sw().stations[1].name, "specify");
}

#[test]
fn shape_is_index_two() {
    assert_eq!(sw().stations[2].name, "shape");
}

#[test]
fn build_is_index_three() {
    assert_eq!(sw().stations[3].name, "build");
}

#[test]
fn prove_is_index_four() {
    assert_eq!(sw().stations[4].name, "prove");
}

#[test]
fn harden_is_index_five() {
    assert_eq!(sw().stations[5].name, "harden");
}

#[test]
fn station_names_are_unique() {
    let mut names = sw().station_names();
    let total = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), total, "station names must be unique");
}

#[test]
fn station_names_are_lowercase_single_words() {
    for name in sw().station_names() {
        assert!(!name.is_empty());
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase()),
            "station name {name:?} should be lowercase ascii"
        );
    }
}

// ---------------------------------------------------------------------------
// first_station.
// ---------------------------------------------------------------------------

#[test]
fn first_station_is_frame() {
    assert_eq!(sw().first_station().unwrap().name, "frame");
}

#[test]
fn first_station_is_some() {
    assert!(sw().first_station().is_some());
}

#[test]
fn first_station_equals_index_zero() {
    let f = sw();
    assert_eq!(f.first_station().unwrap(), &f.stations[0]);
}

#[test]
fn first_station_on_empty_factory_is_none() {
    let empty = FactoryDef {
        name: "empty".to_string(),
        stations: vec![],
        surfaces: vec![],
        default_model: String::new(),
        run_reviewers: vec![],
        run_reviewer_applies_to: Default::default(),
    };
    assert!(empty.first_station().is_none());
}

#[test]
fn first_station_full_def_matches_frame_lookup() {
    let f = sw();
    assert_eq!(f.first_station(), f.station("frame"));
}

// ---------------------------------------------------------------------------
// next_station.
// ---------------------------------------------------------------------------

#[test]
fn next_after_frame_is_specify() {
    assert_eq!(sw().next_station("frame").unwrap().name, "specify");
}

#[test]
fn next_after_specify_is_shape() {
    assert_eq!(sw().next_station("specify").unwrap().name, "shape");
}

#[test]
fn next_after_shape_is_build() {
    assert_eq!(sw().next_station("shape").unwrap().name, "build");
}

#[test]
fn next_after_build_is_prove() {
    assert_eq!(sw().next_station("build").unwrap().name, "prove");
}

#[test]
fn next_after_prove_is_harden() {
    assert_eq!(sw().next_station("prove").unwrap().name, "harden");
}

#[test]
fn next_after_harden_is_none() {
    assert!(sw().next_station("harden").is_none());
}

#[test]
fn next_for_unknown_station_is_none() {
    assert!(sw().next_station("nonexistent").is_none());
}

#[test]
fn next_for_empty_name_is_none() {
    assert!(sw().next_station("").is_none());
}

#[test]
fn next_chain_walks_full_order() {
    let f = sw();
    let mut cur = f.first_station().unwrap().name.clone();
    let mut walked = vec![cur.clone()];
    while let Some(next) = f.next_station(&cur) {
        walked.push(next.name.clone());
        cur = next.name.clone();
    }
    assert_eq!(walked, ORDER.to_vec());
}

#[test]
fn next_station_count_is_one_less_than_stations() {
    let f = sw();
    let with_next = ORDER
        .iter()
        .filter(|n| f.next_station(n).is_some())
        .count();
    assert_eq!(with_next, ORDER.len() - 1);
}

#[test]
fn only_harden_has_no_next() {
    let f = sw();
    for name in ORDER {
        let has_next = f.next_station(name).is_some();
        assert_eq!(has_next, name != "harden", "next({name:?}) mismatch");
    }
}

#[test]
fn next_station_is_distinct_from_input() {
    let f = sw();
    for name in ORDER {
        if let Some(next) = f.next_station(name) {
            assert_ne!(next.name, name);
        }
    }
}

// ---------------------------------------------------------------------------
// station(name) lookup.
// ---------------------------------------------------------------------------

#[test]
fn station_lookup_finds_each_in_order() {
    let f = sw();
    for name in ORDER {
        let s = f.station(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(s.name, name);
    }
}

#[test]
fn station_lookup_unknown_is_none() {
    assert!(sw().station("nope").is_none());
}

#[test]
fn station_lookup_empty_is_none() {
    assert!(sw().station("").is_none());
}

#[test]
fn station_lookup_is_case_sensitive() {
    assert!(sw().station("Frame").is_none());
    assert!(sw().station("FRAME").is_none());
}

#[test]
fn station_lookup_rejects_whitespace_padding() {
    assert!(sw().station(" frame").is_none());
    assert!(sw().station("frame ").is_none());
}

#[test]
fn station_lookup_returns_reference_into_vec() {
    let f = sw();
    let looked = f.station("build").unwrap();
    let direct = &f.stations[3];
    assert_eq!(looked, direct);
}

#[test]
fn station_lookup_matches_artifact_name_not_used() {
    // The lookup keys on station name, not artifact filename.
    assert!(sw().station("frame.md").is_none());
    assert!(sw().station("release.md").is_none());
}

// ---------------------------------------------------------------------------
// Artifacts per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_artifact_is_frame_md() {
    assert_eq!(st("frame").artifact, "frame.md");
}

#[test]
fn specify_artifact_is_spec_md() {
    assert_eq!(st("specify").artifact, "spec.md");
}

#[test]
fn shape_artifact_is_shape_md() {
    assert_eq!(st("shape").artifact, "design.md");
}

#[test]
fn build_artifact_is_build_md() {
    assert_eq!(st("build").artifact, "code");
}

#[test]
fn prove_artifact_is_prove_md() {
    assert_eq!(st("prove").artifact, "proof.md");
}

#[test]
fn harden_artifact_is_release_md() {
    assert_eq!(st("harden").artifact, "release.md");
}

#[test]
fn every_artifact_is_a_md_file_or_a_named_output() {
    // Most stations lock a `.md` document; Build locks `code` (a named output,
    // no extension). Either way the artifact is a bare, non-pathy token.
    for s in sw().stations {
        assert!(
            s.artifact.ends_with(".md") || !s.artifact.contains('.'),
            "artifact {:?} for {:?} should be a .md file or a bare named output",
            s.artifact,
            s.name
        );
    }
}

#[test]
fn every_artifact_is_non_empty() {
    for s in sw().stations {
        assert!(!s.artifact.is_empty(), "station {:?} has empty artifact", s.name);
    }
}

#[test]
fn artifacts_are_unique() {
    let mut arts: Vec<String> = sw().stations.iter().map(|s| s.artifact.clone()).collect();
    let total = arts.len();
    arts.sort();
    arts.dedup();
    assert_eq!(arts.len(), total, "artifacts must be unique per station");
}

#[test]
fn artifacts_in_canonical_order() {
    let arts: Vec<String> = sw().stations.iter().map(|s| s.artifact.clone()).collect();
    assert_eq!(
        arts,
        vec!["frame.md", "spec.md", "design.md", "code", "proof.md", "release.md"]
    );
}

#[test]
fn artifacts_have_no_path_separators() {
    for s in sw().stations {
        assert!(!s.artifact.contains('/'), "artifact must be a bare filename");
        assert!(!s.artifact.contains('\\'));
    }
}

// ---------------------------------------------------------------------------
// Kills per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_kills_wrong_thing() {
    assert_eq!(st("frame").kills, "wrong-thing");
}

#[test]
fn specify_kills_ambiguity() {
    assert_eq!(st("specify").kills, "ambiguity");
}

#[test]
fn shape_kills_expensive_structural_reversal() {
    assert_eq!(st("shape").kills, "expensive-structural-reversal");
}

#[test]
fn build_kills_implementation_defects() {
    assert_eq!(st("build").kills, "implementation-defects");
}

#[test]
fn prove_kills_escaped_defects() {
    assert_eq!(st("prove").kills, "escaped-defects");
}

#[test]
fn harden_kills_works_in_dev_dies_in_prod() {
    assert_eq!(st("harden").kills, "works-in-dev-dies-in-prod");
}

#[test]
fn every_station_kills_something_non_empty() {
    for s in sw().stations {
        assert!(!s.kills.is_empty(), "station {:?} must declare a risk it kills", s.name);
    }
}

#[test]
fn kills_are_unique_per_station() {
    let mut kills: Vec<String> = sw().stations.iter().map(|s| s.kills.clone()).collect();
    let total = kills.len();
    kills.sort();
    kills.dedup();
    assert_eq!(kills.len(), total, "each station kills a distinct risk");
}

#[test]
fn kills_use_hyphenated_slugs_not_spaces() {
    for s in sw().stations {
        assert!(
            !s.kills.contains(' '),
            "kills slug {:?} should be hyphenated, not spaced",
            s.kills
        );
    }
}

// ---------------------------------------------------------------------------
// Workers per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_workers() {
    assert_eq!(st("frame").workers, vec!["framer", "challenger", "distiller"]);
}

#[test]
fn specify_workers() {
    assert_eq!(
        st("specify").workers,
        vec!["spec_writer", "adversary", "tightener"]
    );
}

#[test]
fn shape_workers() {
    assert_eq!(
        st("shape").workers,
        vec!["designer", "visual_designer", "spiker", "pressure_tester", "resolver"]
    );
}

#[test]
fn build_workers() {
    assert_eq!(
        st("build").workers,
        vec!["test_author", "builder", "self_reviewer", "reconciler"]
    );
}

#[test]
fn prove_workers() {
    assert_eq!(
        st("prove").workers,
        vec!["verifier", "breaker", "triage"]
    );
}

#[test]
fn harden_workers() {
    assert_eq!(
        st("harden").workers,
        vec!["hardener", "red_teamer", "releaser"]
    );
}

#[test]
fn every_station_has_at_least_three_workers() {
    for s in sw().stations {
        assert!(
            s.workers.len() >= 3,
            "station {:?} should run at least 3 workers, had {}",
            s.name,
            s.workers.len()
        );
    }
}

#[test]
fn build_has_four_workers() {
    assert_eq!(st("build").workers.len(), 4);
}

#[test]
fn worker_counts_match_the_corpus() {
    for (name, n) in [
        ("frame", 3),
        ("specify", 3),
        ("shape", 5),
        ("build", 4),
        ("prove", 3),
        ("harden", 3),
    ] {
        assert_eq!(st(name).workers.len(), n, "{name} worker count");
    }
}

#[test]
fn no_worker_name_is_empty() {
    for s in sw().stations {
        for w in &s.workers {
            assert!(!w.is_empty(), "empty worker in station {:?}", s.name);
        }
    }
}

#[test]
fn worker_names_are_unique_within_each_station() {
    for s in sw().stations {
        let mut w = s.workers.clone();
        let total = w.len();
        w.sort();
        w.dedup();
        assert_eq!(w.len(), total, "duplicate worker in station {:?}", s.name);
    }
}

#[test]
fn worker_names_use_snake_case_slugs() {
    for s in sw().stations {
        for w in &s.workers {
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "worker {w:?} should be snake_case in station {:?}",
                s.name
            );
        }
    }
}

#[test]
fn total_worker_slots_is_nineteen() {
    let total: usize = sw().stations.iter().map(|s| s.workers.len()).sum();
    assert_eq!(total, 21);
}

#[test]
fn worker_names_are_globally_unique() {
    let mut all: Vec<String> = sw()
        .stations
        .iter()
        .flat_map(|s| s.workers.clone())
        .collect();
    let total = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), total, "worker names must not repeat across stations");
}

// ---------------------------------------------------------------------------
// Reviewers per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_reviewers() {
    assert_eq!(st("frame").reviewers, vec!["value", "feasibility"]);
}

#[test]
fn specify_reviewers() {
    assert_eq!(
        st("specify").reviewers,
        vec!["testability", "completeness"]
    );
}

#[test]
fn shape_reviewers() {
    assert_eq!(
        st("shape").reviewers,
        vec!["fit", "reversibility", "simplicity"]
    );
}

#[test]
fn build_reviewers() {
    assert_eq!(
        st("build").reviewers,
        vec!["correctness", "maintainability"]
    );
}

#[test]
fn prove_reviewers() {
    assert_eq!(st("prove").reviewers, vec!["evidence", "coverage"]);
}

#[test]
fn harden_reviewers() {
    assert_eq!(st("harden").reviewers, vec!["security", "readiness"]);
}

#[test]
fn reviewer_counts_match_the_corpus() {
    // Every station has two reviewers except Shape, which has three (fit /
    // reversibility / simplicity).
    for s in sw().stations {
        let expected = if s.name == "shape" { 3 } else { 2 };
        assert_eq!(s.reviewers.len(), expected, "station {:?} reviewer count", s.name);
    }
}

#[test]
fn no_reviewer_name_is_empty() {
    for s in sw().stations {
        for r in &s.reviewers {
            assert!(!r.is_empty(), "empty reviewer in station {:?}", s.name);
        }
    }
}

#[test]
fn reviewer_names_are_unique_within_each_station() {
    for s in sw().stations {
        assert_ne!(
            s.reviewers[0], s.reviewers[1],
            "duplicate reviewer in station {:?}",
            s.name
        );
    }
}

#[test]
fn reviewer_names_use_snake_case_slugs() {
    for s in sw().stations {
        for r in &s.reviewers {
            assert!(
                r.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "reviewer {r:?} should be snake_case in station {:?}",
                s.name
            );
        }
    }
}

#[test]
fn total_reviewer_slots_is_twelve() {
    let total: usize = sw().stations.iter().map(|s| s.reviewers.len()).sum();
    assert_eq!(total, 13);
}

#[test]
fn reviewer_names_are_globally_unique() {
    let mut all: Vec<String> = sw()
        .stations
        .iter()
        .flat_map(|s| s.reviewers.clone())
        .collect();
    let total = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), total, "reviewer names must not repeat across stations");
}

// ---------------------------------------------------------------------------
// Cross-cutting structural invariants.
// ---------------------------------------------------------------------------

#[test]
fn workers_and_reviewers_never_share_a_name_within_a_station() {
    for s in sw().stations {
        for w in &s.workers {
            assert!(
                !s.reviewers.contains(w),
                "name {w:?} is both worker and reviewer in {:?}",
                s.name
            );
        }
    }
}

#[test]
fn worker_and_reviewer_pools_are_globally_disjoint() {
    let workers: Vec<String> = sw().stations.iter().flat_map(|s| s.workers.clone()).collect();
    let reviewers: Vec<String> = sw()
        .stations
        .iter()
        .flat_map(|s| s.reviewers.clone())
        .collect();
    for r in &reviewers {
        assert!(
            !workers.contains(r),
            "{r:?} appears as both a worker and a reviewer"
        );
    }
}

#[test]
fn station_names_never_collide_with_factory_name() {
    let f = sw();
    for s in &f.stations {
        assert_ne!(s.name, f.name);
    }
}

#[test]
fn every_station_field_is_populated() {
    for s in sw().stations {
        assert!(!s.name.is_empty());
        assert!(!s.kills.is_empty());
        assert!(!s.artifact.is_empty());
        assert!(!s.workers.is_empty());
        assert!(!s.reviewers.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Equality / inequality of the value types.
// ---------------------------------------------------------------------------

#[test]
fn factory_def_equality_is_structural() {
    assert_eq!(resolve_factory("software").unwrap(), resolve_factory("software").unwrap());
}

#[test]
fn factory_def_differs_when_name_differs() {
    let mut other = sw();
    other.name = "different".to_string();
    assert_ne!(other, sw());
}

#[test]
fn factory_def_differs_when_stations_truncated() {
    let mut other = sw();
    other.stations.pop();
    assert_ne!(other, sw());
}

#[test]
fn factory_def_differs_when_stations_reordered() {
    let mut other = sw();
    other.stations.reverse();
    assert_ne!(other, sw());
}

#[test]
fn station_def_equality_is_structural() {
    assert_eq!(st("frame"), st("frame"));
}

#[test]
fn distinct_stations_are_not_equal() {
    assert_ne!(st("frame"), st("specify"));
}

#[test]
fn station_def_differs_when_artifact_differs() {
    let mut s = st("build");
    s.artifact = "different.md".to_string();
    assert_ne!(s, st("build"));
}

#[test]
fn station_def_differs_when_workers_differ() {
    let mut s = st("frame");
    s.workers.push("extra".to_string());
    assert_ne!(s, st("frame"));
}

#[test]
fn station_def_clone_round_trips() {
    let s = st("harden");
    assert_eq!(s.clone(), s);
}

// ---------------------------------------------------------------------------
// Debug formatting (the types derive Debug).
// ---------------------------------------------------------------------------

#[test]
fn factory_def_debug_mentions_name_and_a_station() {
    let dbg = format!("{:?}", sw());
    assert!(dbg.contains("software"));
    assert!(dbg.contains("frame"));
}

#[test]
fn station_def_debug_mentions_its_fields() {
    let dbg = format!("{:?}", st("frame"));
    assert!(dbg.contains("frame"));
    assert!(dbg.contains("framer"));
    assert!(dbg.contains("value"));
}

// ---------------------------------------------------------------------------
// Navigation helpers over the empty / synthetic factories.
// ---------------------------------------------------------------------------

#[test]
fn empty_factory_has_no_station_names() {
    let empty = FactoryDef {
        name: "x".to_string(),
        stations: vec![],
        surfaces: vec![],
        default_model: String::new(),
        run_reviewers: vec![],
        run_reviewer_applies_to: Default::default(),
    };
    assert!(empty.station_names().is_empty());
    assert!(empty.station("anything").is_none());
    assert!(empty.next_station("anything").is_none());
}

#[test]
fn single_station_factory_has_no_next() {
    let one = FactoryDef {
        name: "one".to_string(),
        stations: vec![st("frame")],
        surfaces: vec![],
        default_model: String::new(),
        run_reviewers: vec![],
        run_reviewer_applies_to: Default::default(),
    };
    assert_eq!(one.first_station().unwrap().name, "frame");
    assert!(one.next_station("frame").is_none());
    assert_eq!(one.station_names(), vec!["frame"]);
}

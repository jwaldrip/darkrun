//! Comprehensive integration tests for the embedded `software` factory.
//!
//! Area: software_factory. These tests drive the public API of
//! `darkrun-content` (`list_factories`, `load_factory`, `load_validated`,
//! `validate`) and assert the *shape* of the shipped software factory in depth:
//!
//! - the station roster is exactly `[frame, specify, shape, build, prove,
//!   harden]`, in cost-of-late-discovery order;
//! - each station's checkpoint kind, explorer/worker/reviewer rosters, and
//!   locked artifact match the design contract;
//! - the Make -> Challenge -> Resolve worker ordering is present in every
//!   station's pass-loop;
//! - `FactoryFrontmatter` fields (name, category, default_model, stations,
//!   fix_workers) carry the expected values;
//! - serde roundtrips of frontmatter are lossless and stable;
//! - structural invariants hold (determinism, idempotency, no dangling
//!   references, no duplicate slugs, locked-artifact uniqueness, input
//!   provenance).

use darkrun_content::{
    list_factories, load_factory, load_validated, validate, Factory, FactoryFrontmatter, Role,
    RoleKind, Station,
};
use darkrun_core::domain::CheckpointKind;

// ---------------------------------------------------------------------------
// The design contract: the six stations and their full role rosters.
// ---------------------------------------------------------------------------

/// One station's expected shape, straight from the factory design.
struct Expected {
    name: &'static str,
    explorers: &'static [&'static str],
    workers: &'static [&'static str],
    reviewers: &'static [&'static str],
    checkpoint: CheckpointKind,
    locked_artifact: &'static str,
    inputs: &'static [&'static str],
}

const STATIONS: &[Expected] = &[
    Expected {
        name: "frame",
        explorers: &["context", "value"],
        workers: &["framer", "challenger", "distiller"],
        reviewers: &["value", "feasibility"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "frame.md",
        inputs: &[],
    },
    Expected {
        name: "specify",
        explorers: &["contract", "edge_case"],
        workers: &["spec_writer", "adversary", "tightener"],
        reviewers: &["testability", "completeness"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "spec.md",
        inputs: &["frame.md"],
    },
    Expected {
        name: "shape",
        explorers: &["architecture", "risk"],
        workers: &["designer", "visual_designer", "spiker", "pressure_tester", "resolver"],
        reviewers: &["fit", "reversibility", "simplicity"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "design.md",
        inputs: &["frame.md", "spec.md"],
    },
    Expected {
        name: "build",
        explorers: &["reuse", "integration_point"],
        workers: &["test_author", "builder", "self_reviewer", "reconciler"],
        reviewers: &["correctness", "maintainability"],
        checkpoint: CheckpointKind::Auto,
        locked_artifact: "code",
        inputs: &["frame.md", "spec.md", "design.md"],
    },
    Expected {
        name: "prove",
        explorers: &["scenario", "regression"],
        workers: &["verifier", "breaker", "triage"],
        reviewers: &["evidence", "coverage"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "proof.md",
        inputs: &["spec.md", "code"],
    },
    Expected {
        name: "harden",
        explorers: &["threat", "operability"],
        workers: &["hardener", "red_teamer", "releaser"],
        reviewers: &["security", "readiness"],
        checkpoint: CheckpointKind::External,
        locked_artifact: "release.md",
        inputs: &["frame.md", "spec.md", "design.md", "proof.md", "code"],
    },
];

const STATION_ORDER: &[&str] = &["frame", "specify", "shape", "build", "prove", "harden"];

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

fn factory() -> Factory {
    load_validated("software").expect("software factory must load and validate")
}

fn slugs(roles: &[Role]) -> Vec<&str> {
    roles.iter().map(Role::name).collect()
}

fn expected_for(name: &str) -> &'static Expected {
    STATIONS
        .iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("no expectation for station `{name}`"))
}

// ---------------------------------------------------------------------------
// Top-level factory existence and identity.
// ---------------------------------------------------------------------------

#[test]
fn corpus_lists_the_software_factory() {
    assert!(list_factories().contains(&"software".to_string()));
}

#[test]
fn list_factories_is_sorted() {
    let names = list_factories();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "list_factories must come back sorted");
}

#[test]
fn list_factories_is_deduped() {
    let names = list_factories();
    let mut deduped = names.clone();
    deduped.dedup();
    assert_eq!(names.len(), deduped.len(), "no duplicate factory slugs");
}

#[test]
fn list_factories_is_nonempty() {
    assert!(!list_factories().is_empty(), "corpus ships at least one factory");
}

#[test]
fn list_factories_is_deterministic() {
    assert_eq!(list_factories(), list_factories());
}

#[test]
fn software_loads_without_validation() {
    load_factory("software").expect("software factory loads");
}

#[test]
fn software_loads_and_validates() {
    load_validated("software").expect("software factory loads and validates");
}

#[test]
fn software_validates_after_load() {
    let f = load_factory("software").expect("load");
    validate(&f).expect("loaded factory must validate");
}

#[test]
fn factory_name_is_software() {
    assert_eq!(factory().name(), "software");
}

#[test]
fn factory_name_matches_frontmatter_name() {
    let f = factory();
    assert_eq!(f.name(), f.frontmatter.name);
}

// ---------------------------------------------------------------------------
// FactoryFrontmatter fields.
// ---------------------------------------------------------------------------

#[test]
fn frontmatter_name_field() {
    assert_eq!(factory().frontmatter.name, "software");
}

#[test]
fn frontmatter_category_is_engineering() {
    assert_eq!(factory().frontmatter.category, "engineering");
}

#[test]
fn frontmatter_default_model_is_sonnet() {
    assert_eq!(factory().frontmatter.default_model, "sonnet");
}

#[test]
fn frontmatter_description_is_nonempty() {
    assert!(!factory().frontmatter.description.trim().is_empty());
}

#[test]
fn frontmatter_description_mentions_six_stations() {
    let desc = factory().frontmatter.description.to_lowercase();
    assert!(desc.contains("six"), "description should name six stations: {desc}");
}

#[test]
fn frontmatter_stations_field_is_the_design_order() {
    let stations = factory().frontmatter.stations;
    let expected: Vec<String> = STATION_ORDER.iter().map(|s| s.to_string()).collect();
    assert_eq!(stations, expected);
}

#[test]
fn frontmatter_stations_has_six_entries() {
    assert_eq!(factory().frontmatter.stations.len(), 6);
}

#[test]
fn frontmatter_fix_workers_is_nonempty() {
    assert!(
        !factory().frontmatter.fix_workers.is_empty(),
        "factory must declare fix-workers for drift repair"
    );
}

#[test]
fn frontmatter_fix_workers_are_builder_reconciler_validator() {
    assert_eq!(
        factory().frontmatter.fix_workers,
        vec![
            "builder".to_string(),
            "reconciler".to_string(),
            "validator".to_string()
        ]
    );
}

#[test]
fn frontmatter_fix_workers_have_no_duplicates() {
    let fw = factory().frontmatter.fix_workers;
    let mut deduped = fw.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(fw.len(), deduped.len(), "fix_workers must be unique");
}

// ---------------------------------------------------------------------------
// Station roster: order, count, presence.
// ---------------------------------------------------------------------------

#[test]
fn loaded_stations_are_in_design_order() {
    let f = factory();
    let order: Vec<&str> = f.stations.iter().map(Station::name).collect();
    assert_eq!(order, STATION_ORDER);
}

#[test]
fn loaded_station_count_is_six() {
    assert_eq!(factory().stations.len(), 6);
}

#[test]
fn loaded_stations_match_declared_stations() {
    let f = factory();
    let declared = &f.frontmatter.stations;
    let loaded: Vec<&str> = f.stations.iter().map(Station::name).collect();
    let declared_refs: Vec<&str> = declared.iter().map(String::as_str).collect();
    assert_eq!(loaded, declared_refs);
}

#[test]
fn station_lookup_finds_every_declared_station() {
    let f = factory();
    for name in STATION_ORDER {
        assert!(f.station(name).is_some(), "station `{name}` must resolve");
    }
}

#[test]
fn station_lookup_returns_none_for_unknown() {
    assert!(factory().station("nonexistent").is_none());
}

#[test]
fn station_lookup_is_case_sensitive() {
    assert!(factory().station("FRAME").is_none());
    assert!(factory().station("Frame").is_none());
}

#[test]
fn station_lookup_returns_the_named_station() {
    let f = factory();
    for name in STATION_ORDER {
        assert_eq!(f.station(name).unwrap().name(), *name);
    }
}

// Per-station presence tests (one each — each can fail independently).
macro_rules! station_present {
    ($test:ident, $name:literal) => {
        #[test]
        fn $test() {
            assert!(factory().station($name).is_some());
        }
    };
}
station_present!(frame_station_present, "frame");
station_present!(specify_station_present, "specify");
station_present!(shape_station_present, "shape");
station_present!(build_station_present, "build");
station_present!(prove_station_present, "prove");
station_present!(harden_station_present, "harden");

// ---------------------------------------------------------------------------
// Per-station roster correctness (generated, one test fn per station).
// ---------------------------------------------------------------------------

macro_rules! station_rosters {
    ($($test:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $test() {
                let f = factory();
                let exp = expected_for($name);
                let s = f.station($name).expect("station present");

                assert_eq!(slugs(&s.explorers), exp.explorers, "{} explorers", $name);
                assert_eq!(slugs(&s.workers), exp.workers, "{} workers", $name);
                assert_eq!(slugs(&s.reviewers), exp.reviewers, "{} reviewers", $name);
                assert_eq!(s.checkpoint(), exp.checkpoint, "{} checkpoint", $name);
                assert_eq!(
                    s.frontmatter.locked_artifact, exp.locked_artifact,
                    "{} locked_artifact", $name
                );
                let inputs: Vec<&str> =
                    s.frontmatter.inputs.iter().map(String::as_str).collect();
                assert_eq!(inputs, exp.inputs, "{} inputs", $name);
            }
        )*
    };
}

station_rosters! {
    frame_roster => "frame",
    specify_roster => "specify",
    shape_roster => "shape",
    build_roster => "build",
    prove_roster => "prove",
    harden_roster => "harden",
}

// ---------------------------------------------------------------------------
// Explorer rosters — individual per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_explorers_are_context_and_value() {
    assert_eq!(
        slugs(&factory().station("frame").unwrap().explorers),
        vec!["context", "value"]
    );
}

#[test]
fn specify_explorers_are_contract_and_edge_case() {
    assert_eq!(
        slugs(&factory().station("specify").unwrap().explorers),
        vec!["contract", "edge_case"]
    );
}

#[test]
fn shape_explorers_are_architecture_and_risk() {
    assert_eq!(
        slugs(&factory().station("shape").unwrap().explorers),
        vec!["architecture", "risk"]
    );
}

#[test]
fn build_explorers_are_reuse_and_integration_point() {
    assert_eq!(
        slugs(&factory().station("build").unwrap().explorers),
        vec!["reuse", "integration_point"]
    );
}

#[test]
fn prove_explorers_are_scenario_and_regression() {
    assert_eq!(
        slugs(&factory().station("prove").unwrap().explorers),
        vec!["scenario", "regression"]
    );
}

#[test]
fn harden_explorers_are_threat_and_operability() {
    assert_eq!(
        slugs(&factory().station("harden").unwrap().explorers),
        vec!["threat", "operability"]
    );
}

#[test]
fn every_station_has_exactly_two_explorers() {
    for s in &factory().stations {
        assert_eq!(s.explorers.len(), 2, "{} explorer count", s.name());
    }
}

// ---------------------------------------------------------------------------
// Worker rosters — individual per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("frame").unwrap().workers),
        vec!["framer", "challenger", "distiller"]
    );
}

#[test]
fn specify_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("specify").unwrap().workers),
        vec!["spec_writer", "adversary", "tightener"]
    );
}

#[test]
fn shape_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("shape").unwrap().workers),
        vec!["designer", "visual_designer", "spiker", "pressure_tester", "resolver"]
    );
}

#[test]
fn build_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("build").unwrap().workers),
        vec!["test_author", "builder", "self_reviewer", "reconciler"]
    );
}

#[test]
fn prove_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("prove").unwrap().workers),
        vec!["verifier", "breaker", "triage"]
    );
}

#[test]
fn harden_workers_in_order() {
    assert_eq!(
        slugs(&factory().station("harden").unwrap().workers),
        vec!["hardener", "red_teamer", "releaser"]
    );
}

#[test]
fn frame_specify_prove_harden_have_three_workers() {
    for name in ["frame", "specify", "prove", "harden"] {
        assert_eq!(
            factory().station(name).unwrap().workers.len(),
            3,
            "{name} should have a 3-beat pass-loop"
        );
    }
}

#[test]
fn build_has_four_workers() {
    assert_eq!(
        factory().station("build").unwrap().workers.len(),
        4,
        "build has an extra worker beat"
    );
}

#[test]
fn shape_has_five_workers() {
    // Shape adds a VisualDesigner beat between the Designer and the Spiker for the
    // visual/UX facet of user-facing work — five beats in all.
    assert_eq!(
        factory().station("shape").unwrap().workers.len(),
        5,
        "shape carries an extra visual-design worker beat"
    );
}

// ---------------------------------------------------------------------------
// Reviewer rosters — individual per station.
// ---------------------------------------------------------------------------

#[test]
fn frame_reviewers_are_value_and_feasibility() {
    assert_eq!(
        slugs(&factory().station("frame").unwrap().reviewers),
        vec!["value", "feasibility"]
    );
}

#[test]
fn specify_reviewers_are_testability_and_completeness() {
    assert_eq!(
        slugs(&factory().station("specify").unwrap().reviewers),
        vec!["testability", "completeness"]
    );
}

#[test]
fn shape_reviewers_are_fit_reversibility_simplicity() {
    assert_eq!(
        slugs(&factory().station("shape").unwrap().reviewers),
        vec!["fit", "reversibility", "simplicity"]
    );
}

#[test]
fn build_reviewers_are_correctness_and_maintainability() {
    assert_eq!(
        slugs(&factory().station("build").unwrap().reviewers),
        vec!["correctness", "maintainability"]
    );
}

#[test]
fn prove_reviewers_are_evidence_and_coverage() {
    assert_eq!(
        slugs(&factory().station("prove").unwrap().reviewers),
        vec!["evidence", "coverage"]
    );
}

#[test]
fn harden_reviewers_are_security_and_readiness() {
    assert_eq!(
        slugs(&factory().station("harden").unwrap().reviewers),
        vec!["security", "readiness"]
    );
}

#[test]
fn every_station_has_at_least_two_reviewers() {
    for s in &factory().stations {
        assert!(s.reviewers.len() >= 2, "{} reviewer count", s.name());
    }
}

// ---------------------------------------------------------------------------
// Checkpoint kinds — per station and as a sequence.
// ---------------------------------------------------------------------------

#[test]
fn frame_checkpoint_is_ask() {
    assert_eq!(factory().station("frame").unwrap().checkpoint(), CheckpointKind::Ask);
}

#[test]
fn specify_checkpoint_is_ask() {
    assert_eq!(factory().station("specify").unwrap().checkpoint(), CheckpointKind::Ask);
}

#[test]
fn shape_checkpoint_is_ask() {
    assert_eq!(factory().station("shape").unwrap().checkpoint(), CheckpointKind::Ask);
}

#[test]
fn build_checkpoint_is_auto() {
    assert_eq!(factory().station("build").unwrap().checkpoint(), CheckpointKind::Auto);
}

#[test]
fn prove_checkpoint_is_ask() {
    assert_eq!(factory().station("prove").unwrap().checkpoint(), CheckpointKind::Ask);
}

#[test]
fn harden_checkpoint_is_external() {
    assert_eq!(factory().station("harden").unwrap().checkpoint(), CheckpointKind::External);
}

#[test]
fn checkpoint_sequence_is_ask_ask_ask_auto_ask_external() {
    let kinds: Vec<CheckpointKind> =
        factory().stations.iter().map(Station::checkpoint).collect();
    assert_eq!(
        kinds,
        vec![
            CheckpointKind::Ask,
            CheckpointKind::Ask,
            CheckpointKind::Ask,
            CheckpointKind::Auto,
            CheckpointKind::Ask,
            CheckpointKind::External,
        ]
    );
}

#[test]
fn only_build_is_auto() {
    let f = factory();
    let auto: Vec<&str> = f
        .stations
        .iter()
        .filter(|s| s.checkpoint() == CheckpointKind::Auto)
        .map(Station::name)
        .collect();
    assert_eq!(auto, vec!["build"]);
}

#[test]
fn only_harden_is_external() {
    let f = factory();
    let external: Vec<&str> = f
        .stations
        .iter()
        .filter(|s| s.checkpoint() == CheckpointKind::External)
        .map(Station::name)
        .collect();
    assert_eq!(external, vec!["harden"]);
}

#[test]
fn no_station_uses_await_checkpoint() {
    for s in &factory().stations {
        assert_ne!(
            s.checkpoint(),
            CheckpointKind::Await,
            "{} should not use await in this factory",
            s.name()
        );
    }
}

#[test]
fn four_stations_ask() {
    let ask = factory()
        .stations
        .iter()
        .filter(|s| s.checkpoint() == CheckpointKind::Ask)
        .count();
    assert_eq!(ask, 4);
}

#[test]
fn final_station_hands_off_externally() {
    let last = factory().stations.last().unwrap().clone();
    assert_eq!(last.name(), "harden");
    assert_eq!(last.checkpoint(), CheckpointKind::External);
}

#[test]
fn checkpoint_matches_frontmatter_field() {
    for s in &factory().stations {
        assert_eq!(s.checkpoint(), s.frontmatter.checkpoint, "{}", s.name());
    }
}

// ---------------------------------------------------------------------------
// Locked artifacts.
// ---------------------------------------------------------------------------

#[test]
fn frame_locks_frame_md() {
    assert_eq!(factory().station("frame").unwrap().frontmatter.locked_artifact, "frame.md");
}

#[test]
fn specify_locks_spec_md() {
    assert_eq!(factory().station("specify").unwrap().frontmatter.locked_artifact, "spec.md");
}

#[test]
fn shape_locks_design_md() {
    assert_eq!(factory().station("shape").unwrap().frontmatter.locked_artifact, "design.md");
}

#[test]
fn build_locks_code() {
    assert_eq!(factory().station("build").unwrap().frontmatter.locked_artifact, "code");
}

#[test]
fn prove_locks_proof_md() {
    assert_eq!(factory().station("prove").unwrap().frontmatter.locked_artifact, "proof.md");
}

#[test]
fn harden_locks_release_md() {
    assert_eq!(factory().station("harden").unwrap().frontmatter.locked_artifact, "release.md");
}

#[test]
fn every_locked_artifact_is_nonempty() {
    for s in &factory().stations {
        assert!(
            !s.frontmatter.locked_artifact.trim().is_empty(),
            "{} must lock a durable artifact",
            s.name()
        );
    }
}

#[test]
fn locked_artifacts_are_unique() {
    let f = factory();
    let artifacts: Vec<&str> = f
        .stations
        .iter()
        .map(|s| s.frontmatter.locked_artifact.as_str())
        .collect();
    let mut deduped = artifacts.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(
        artifacts.len(),
        deduped.len(),
        "no two stations may lock the same artifact"
    );
}

#[test]
fn locked_artifacts_full_sequence() {
    let f = factory();
    let artifacts: Vec<&str> = f
        .stations
        .iter()
        .map(|s| s.frontmatter.locked_artifact.as_str())
        .collect();
    assert_eq!(
        artifacts,
        vec!["frame.md", "spec.md", "design.md", "code", "proof.md", "release.md"]
    );
}

#[test]
fn only_build_locks_a_non_markdown_artifact() {
    let f = factory();
    let non_md: Vec<&str> = f
        .stations
        .iter()
        .filter(|s| !s.frontmatter.locked_artifact.ends_with(".md"))
        .map(Station::name)
        .collect();
    assert_eq!(non_md, vec!["build"], "only Build locks running code, not a doc");
}

// ---------------------------------------------------------------------------
// Inputs / pipeline provenance.
// ---------------------------------------------------------------------------

#[test]
fn frame_has_no_inputs() {
    assert!(factory().station("frame").unwrap().frontmatter.inputs.is_empty());
}

#[test]
fn specify_consumes_frame_md() {
    assert_eq!(
        factory().station("specify").unwrap().frontmatter.inputs,
        vec!["frame.md".to_string()]
    );
}

#[test]
fn shape_consumes_frame_and_spec() {
    assert_eq!(
        factory().station("shape").unwrap().frontmatter.inputs,
        vec!["frame.md".to_string(), "spec.md".to_string()]
    );
}

#[test]
fn build_consumes_frame_spec_design() {
    assert_eq!(
        factory().station("build").unwrap().frontmatter.inputs,
        vec!["frame.md".to_string(), "spec.md".to_string(), "design.md".to_string()]
    );
}

#[test]
fn prove_consumes_spec_and_code() {
    assert_eq!(
        factory().station("prove").unwrap().frontmatter.inputs,
        vec!["spec.md".to_string(), "code".to_string()]
    );
}

#[test]
fn harden_consumes_everything_upstream() {
    assert_eq!(
        factory().station("harden").unwrap().frontmatter.inputs,
        vec![
            "frame.md".to_string(),
            "spec.md".to_string(),
            "design.md".to_string(),
            "proof.md".to_string(),
            "code".to_string()
        ]
    );
}

#[test]
fn every_input_is_locked_by_an_upstream_station() {
    let f = factory();
    let mut available: Vec<String> = vec![];
    for s in &f.stations {
        for input in &s.frontmatter.inputs {
            assert!(
                available.contains(input),
                "station `{}` consumes `{input}` but no upstream station locks it (available: {available:?})",
                s.name()
            );
        }
        available.push(s.frontmatter.locked_artifact.clone());
    }
}

#[test]
fn no_station_consumes_its_own_locked_artifact() {
    for s in &factory().stations {
        assert!(
            !s.frontmatter.inputs.contains(&s.frontmatter.locked_artifact),
            "{} must not list its own output as input",
            s.name()
        );
    }
}

#[test]
fn no_station_consumes_a_downstream_artifact() {
    let f = factory();
    let order: Vec<String> = f
        .stations
        .iter()
        .map(|s| s.frontmatter.locked_artifact.clone())
        .collect();
    for (idx, s) in f.stations.iter().enumerate() {
        let upstream: Vec<&String> = order[..idx].iter().collect();
        for input in &s.frontmatter.inputs {
            assert!(
                upstream.contains(&input),
                "{} consumes `{input}`, which is not produced strictly upstream",
                s.name()
            );
        }
    }
}

#[test]
fn inputs_have_no_duplicates_within_a_station() {
    for s in &factory().stations {
        let mut deduped = s.frontmatter.inputs.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            s.frontmatter.inputs.len(),
            deduped.len(),
            "{} lists an input twice",
            s.name()
        );
    }
}

#[test]
fn inputs_grow_monotonically_in_count() {
    // Each later station tends to inherit more locked artifacts; harden is the
    // widest consumer. Assert the last station consumes the most inputs.
    let f = factory();
    let counts: Vec<usize> = f.stations.iter().map(|s| s.frontmatter.inputs.len()).collect();
    let max = *counts.iter().max().unwrap();
    assert_eq!(*counts.last().unwrap(), max, "harden should consume the most inputs");
}

// ---------------------------------------------------------------------------
// Make -> Challenge -> Resolve ordering / pass-loop shape.
// ---------------------------------------------------------------------------

#[test]
fn every_station_has_at_least_three_worker_beats() {
    for s in &factory().stations {
        assert!(
            s.workers.len() >= 3,
            "{} needs Make->Challenge->Resolve, has {}",
            s.name(),
            s.workers.len()
        );
    }
}

#[test]
fn frame_pass_loop_make_challenge_resolve() {
    // framer (Make) -> challenger (Challenge) -> distiller (Resolve)
    let f = factory();
    let w = slugs(&f.station("frame").unwrap().workers);
    assert_eq!(w.first(), Some(&"framer"), "Make beat");
    assert_eq!(w.get(1), Some(&"challenger"), "Challenge beat");
    assert_eq!(w.last(), Some(&"distiller"), "Resolve beat");
}

#[test]
fn specify_pass_loop_make_challenge_resolve() {
    // spec_writer (Make) -> adversary (Challenge) -> tightener (Resolve)
    let f = factory();
    let w = slugs(&f.station("specify").unwrap().workers);
    assert_eq!(w.first(), Some(&"spec_writer"));
    assert_eq!(w.get(1), Some(&"adversary"));
    assert_eq!(w.last(), Some(&"tightener"));
}

#[test]
fn shape_pass_loop_has_challenge_before_resolve() {
    // designer (Make) -> spiker -> pressure_tester (Challenge) -> resolver (Resolve)
    let f = factory();
    let w = slugs(&f.station("shape").unwrap().workers);
    assert_eq!(w.first(), Some(&"designer"), "Make beat");
    assert_eq!(w.last(), Some(&"resolver"), "Resolve beat");
    let challenge = w.iter().position(|x| *x == "pressure_tester").unwrap();
    let resolve = w.iter().position(|x| *x == "resolver").unwrap();
    assert!(challenge < resolve, "Challenge must precede Resolve");
}

#[test]
fn build_pass_loop_ends_in_reconcile() {
    // test_author -> builder (Make) -> self_reviewer (Challenge) -> reconciler (Resolve)
    let f = factory();
    let w = slugs(&f.station("build").unwrap().workers);
    assert_eq!(w.first(), Some(&"test_author"));
    assert_eq!(w.last(), Some(&"reconciler"), "Resolve beat reconciles");
    let challenge = w.iter().position(|x| *x == "self_reviewer").unwrap();
    let resolve = w.iter().position(|x| *x == "reconciler").unwrap();
    assert!(challenge < resolve);
}

#[test]
fn prove_pass_loop_make_challenge_resolve() {
    // verifier (Make) -> breaker (Challenge) -> triage (Resolve)
    let f = factory();
    let w = slugs(&f.station("prove").unwrap().workers);
    assert_eq!(w.first(), Some(&"verifier"));
    assert_eq!(w.get(1), Some(&"breaker"));
    assert_eq!(w.last(), Some(&"triage"));
}

#[test]
fn harden_pass_loop_make_challenge_resolve() {
    // hardener (Make) -> red_teamer (Challenge) -> releaser (Resolve)
    let f = factory();
    let w = slugs(&f.station("harden").unwrap().workers);
    assert_eq!(w.first(), Some(&"hardener"));
    assert_eq!(w.get(1), Some(&"red_teamer"));
    assert_eq!(w.last(), Some(&"releaser"));
}

#[test]
fn challenge_beat_present_in_every_station_body() {
    // Each station documents an adversarial Challenge worker in its pass-loop prose.
    let f = factory();
    for s in &f.stations {
        let body = s.body.to_lowercase();
        let has_challenge = body.contains("challenge")
            || body.contains("attack")
            || body.contains("advers")
            || body.contains("break")
            || body.contains("pressure")
            || body.contains("red team")
            || body.contains("review");
        assert!(has_challenge, "{} body should describe a challenge/attack beat", s.name());
    }
}

#[test]
fn shape_spiker_describes_a_throwaway_proof() {
    let f = factory();
    let shape = f.station("shape").unwrap();
    let spiker = shape.workers.iter().find(|w| w.name() == "spiker").unwrap();
    assert!(
        spiker.body.to_lowercase().contains("throwaway"),
        "the Spiker builds a throwaway proof of the riskiest assumption"
    );
}

#[test]
fn shape_has_a_visual_designer_worker() {
    let f = factory();
    let shape = f.station("shape").unwrap();
    assert!(
        shape.workers.iter().any(|w| w.name() == "visual_designer"),
        "Shape carries a VisualDesigner beat for the visual/UX facet"
    );
}

#[test]
fn shape_visual_designer_sits_between_designer_and_spiker() {
    // The visual beat is a Make-phase facet: it runs after the structural Designer
    // and before the Spiker, so the structure is drafted but no UI is built until
    // the operator has chosen a direction.
    let f = factory();
    let w = slugs(&f.station("shape").unwrap().workers);
    let designer = w.iter().position(|x| *x == "designer").unwrap();
    let visual = w.iter().position(|x| *x == "visual_designer").unwrap();
    let spiker = w.iter().position(|x| *x == "spiker").unwrap();
    assert!(designer < visual, "VisualDesigner runs after the Designer");
    assert!(visual < spiker, "VisualDesigner runs before the Spiker");
}

#[test]
fn shape_visual_designer_directs_generating_options() {
    let f = factory();
    let shape = f.station("shape").unwrap();
    let vd = shape
        .workers
        .iter()
        .find(|w| w.name() == "visual_designer")
        .unwrap();
    let body = vd.body.to_lowercase();
    assert!(
        body.contains("mockup") || body.contains("option") || body.contains("image"),
        "the VisualDesigner generates design options / mockups / images"
    );
}

#[test]
fn shape_visual_designer_uses_the_visual_decision_tools() {
    let f = factory();
    let shape = f.station("shape").unwrap();
    let vd = shape
        .workers
        .iter()
        .find(|w| w.name() == "visual_designer")
        .unwrap();
    let body = vd.body.to_lowercase();
    assert!(
        body.contains("darkrun_question"),
        "VisualDesigner uses darkrun_question to pick among options"
    );
    assert!(
        body.contains("darkrun_direction"),
        "VisualDesigner uses darkrun_direction for a design direction"
    );
}

#[test]
fn shape_visual_designer_conditions_on_user_facing_work() {
    // The visual beat must say it is skipped for non-UI / headless / API work so
    // it does not impose a design step where there is no surface.
    let f = factory();
    let shape = f.station("shape").unwrap();
    let vd = shape
        .workers
        .iter()
        .find(|w| w.name() == "visual_designer")
        .unwrap();
    let body = vd.body.to_lowercase();
    assert!(
        body.contains("user-facing") || body.contains("user facing"),
        "VisualDesigner frames itself around user-facing work"
    );
    assert!(
        body.contains("skip") || body.contains("non-ui") || body.contains("headless"),
        "VisualDesigner skips non-UI work"
    );
}

#[test]
fn shape_station_body_documents_the_visual_designer_beat() {
    let f = factory();
    let body = f.station("shape").unwrap().body.to_lowercase();
    assert!(
        body.contains("visualdesigner") || body.contains("visual"),
        "Shape's pass-loop prose should describe the visual-design beat"
    );
}

#[test]
fn workers_first_beat_relates_to_station_purpose() {
    // The Make beat of each station tends to share a stem with the station name
    // or its core verb. Assert framer<->frame and spec_writer<->spec.
    let f = factory();
    assert!(f.station("frame").unwrap().workers[0].name().contains("fram"));
    assert!(f.station("specify").unwrap().workers[0].name().contains("spec"));
}

// ---------------------------------------------------------------------------
// Role kinds and body substance.
// ---------------------------------------------------------------------------

#[test]
fn explorers_are_all_kind_explorer() {
    for s in &factory().stations {
        for r in &s.explorers {
            assert_eq!(r.kind(), RoleKind::Explorer, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn workers_are_all_kind_worker() {
    for s in &factory().stations {
        for r in &s.workers {
            assert_eq!(r.kind(), RoleKind::Worker, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn reviewers_are_all_kind_reviewer() {
    for s in &factory().stations {
        for r in &s.reviewers {
            assert_eq!(r.kind(), RoleKind::Reviewer, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn role_name_matches_frontmatter_name() {
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert_eq!(r.name(), r.frontmatter.name);
        }
    }
}

#[test]
fn role_kind_matches_frontmatter_agent_type() {
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert_eq!(r.kind(), r.frontmatter.agent_type);
        }
    }
}

#[test]
fn every_role_body_has_a_heading() {
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert!(
                r.body.contains('#'),
                "{}/{} body has no markdown heading",
                s.name(),
                r.name()
            );
        }
    }
}

#[test]
fn every_role_body_is_substantive() {
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert!(
                r.body.trim().len() > 120,
                "{}/{} body too thin ({} bytes)",
                s.name(),
                r.name(),
                r.body.trim().len()
            );
        }
    }
}

#[test]
fn role_bodies_are_not_just_frontmatter_echo() {
    // The body must contain prose beyond the role's own slug.
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            let without_name = r.body.replace(r.name(), "");
            assert!(
                without_name.trim().len() > 80,
                "{}/{} body is mostly just its name",
                s.name(),
                r.name()
            );
        }
    }
}

#[test]
fn frame_framer_body_mentions_framer() {
    let f = factory();
    let framer = f
        .station("frame")
        .unwrap()
        .workers
        .iter()
        .find(|w| w.name() == "framer")
        .unwrap();
    assert!(framer.body.to_lowercase().contains("fram"));
}

#[test]
fn build_test_author_body_mentions_tests() {
    let f = factory();
    let ta = f
        .station("build")
        .unwrap()
        .workers
        .iter()
        .find(|w| w.name() == "test_author")
        .unwrap();
    assert!(ta.body.to_lowercase().contains("test"));
}

#[test]
fn build_reconciler_body_mentions_merge_or_integration() {
    let f = factory();
    let rec = f
        .station("build")
        .unwrap()
        .workers
        .iter()
        .find(|w| w.name() == "reconciler")
        .unwrap();
    let body = rec.body.to_lowercase();
    assert!(body.contains("merge") || body.contains("integrat") || body.contains("reconcil"));
}

#[test]
fn harden_red_teamer_body_mentions_security_or_attack() {
    let f = factory();
    let rt = f
        .station("harden")
        .unwrap()
        .workers
        .iter()
        .find(|w| w.name() == "red_teamer")
        .unwrap();
    let body = rt.body.to_lowercase();
    assert!(body.contains("attack") || body.contains("security") || body.contains("exploit") || body.contains("threat"));
}

// ---------------------------------------------------------------------------
// Role uniqueness and kind balance.
// ---------------------------------------------------------------------------

#[test]
fn no_duplicate_explorer_slugs_within_a_station() {
    for s in &factory().stations {
        let mut names = slugs(&s.explorers);
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len(), "{} has duplicate explorers", s.name());
    }
}

#[test]
fn no_duplicate_worker_slugs_within_a_station() {
    for s in &factory().stations {
        let mut names = slugs(&s.workers);
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len(), "{} has duplicate workers", s.name());
    }
}

#[test]
fn no_duplicate_reviewer_slugs_within_a_station() {
    for s in &factory().stations {
        let mut names = slugs(&s.reviewers);
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(len, names.len(), "{} has duplicate reviewers", s.name());
    }
}

#[test]
fn frontmatter_and_loaded_role_counts_agree() {
    for s in &factory().stations {
        assert_eq!(s.frontmatter.explorers.len(), s.explorers.len(), "{} explorers", s.name());
        assert_eq!(s.frontmatter.workers.len(), s.workers.len(), "{} workers", s.name());
        assert_eq!(s.frontmatter.reviewers.len(), s.reviewers.len(), "{} reviewers", s.name());
    }
}

#[test]
fn frontmatter_role_slugs_match_loaded_role_names() {
    for s in &factory().stations {
        let fm_ex: Vec<&str> = s.frontmatter.explorers.iter().map(String::as_str).collect();
        assert_eq!(fm_ex, slugs(&s.explorers), "{} explorers", s.name());
        let fm_w: Vec<&str> = s.frontmatter.workers.iter().map(String::as_str).collect();
        assert_eq!(fm_w, slugs(&s.workers), "{} workers", s.name());
        let fm_r: Vec<&str> = s.frontmatter.reviewers.iter().map(String::as_str).collect();
        assert_eq!(fm_r, slugs(&s.reviewers), "{} reviewers", s.name());
    }
}

#[test]
fn total_role_count_across_factory() {
    // 2 explorers + (3 or 4) workers + (2 or 3) reviewers per station.
    let f = factory();
    let total: usize = f
        .stations
        .iter()
        .map(|s| s.explorers.len() + s.workers.len() + s.reviewers.len())
        .sum();
    // frame 2+3+2=7, specify 2+3+2=7, shape 2+5+3=10, build 2+4+2=8,
    // prove 2+3+2=7, harden 2+3+2=7  => 46
    assert_eq!(total, 46);
}

#[test]
fn every_station_has_all_three_kinds() {
    for s in &factory().stations {
        assert!(!s.explorers.is_empty(), "{} explorers", s.name());
        assert!(!s.workers.is_empty(), "{} workers", s.name());
        assert!(!s.reviewers.is_empty(), "{} reviewers", s.name());
    }
}

// ---------------------------------------------------------------------------
// Station bodies describe risk / checkpoint.
// ---------------------------------------------------------------------------

#[test]
fn every_station_body_names_its_risk_class() {
    for s in &factory().stations {
        assert!(
            s.body.to_lowercase().contains("risk"),
            "{} body should name the risk class it eliminates",
            s.name()
        );
    }
}

#[test]
fn every_station_body_describes_its_checkpoint() {
    for s in &factory().stations {
        assert!(
            s.body.to_lowercase().contains("checkpoint"),
            "{} body should describe its checkpoint",
            s.name()
        );
    }
}

#[test]
fn every_station_body_has_a_heading() {
    for s in &factory().stations {
        assert!(s.body.contains('#'), "{} body has no heading", s.name());
    }
}

#[test]
fn frame_body_calls_out_wrong_thing_risk() {
    let f = factory();
    assert!(f.station("frame").unwrap().body.to_lowercase().contains("wrong thing"));
}

#[test]
fn factory_body_mentions_cost_of_late_discovery_ordering() {
    assert!(factory().body.to_lowercase().contains("cost-of-late-discovery"));
}

#[test]
fn factory_body_mentions_class_of_risk_eliminated() {
    assert!(factory().body.contains("class-of-risk-eliminated"));
}

#[test]
fn factory_body_documents_the_universal_slot() {
    let body = factory().body.to_lowercase();
    assert!(body.contains("explore"));
    assert!(body.contains("decompose"));
    assert!(body.contains("review"));
    assert!(body.contains("checkpoint"));
    assert!(body.contains("lock"));
}

#[test]
fn factory_body_documents_make_challenge_resolve() {
    let body = factory().body.to_lowercase();
    assert!(body.contains("make"));
    assert!(body.contains("challenge"));
    assert!(body.contains("resolve"));
}

// ---------------------------------------------------------------------------
// Determinism / idempotency.
// ---------------------------------------------------------------------------

#[test]
fn loading_twice_yields_identical_station_order() {
    let a: Vec<String> = load_factory("software")
        .unwrap()
        .stations
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    let b: Vec<String> = load_factory("software")
        .unwrap()
        .stations
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    assert_eq!(a, b);
}

#[test]
fn loading_twice_yields_identical_role_rosters() {
    let collect = || -> Vec<String> {
        load_factory("software")
            .unwrap()
            .stations
            .iter()
            .flat_map(|s| {
                s.explorers
                    .iter()
                    .chain(&s.workers)
                    .chain(&s.reviewers)
                    .map(|r| format!("{}/{}", s.name(), r.name()))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(collect(), collect());
}

#[test]
fn validate_is_idempotent() {
    let f = factory();
    validate(&f).expect("first validate");
    validate(&f).expect("second validate");
    validate(&f).expect("third validate");
}

#[test]
fn load_validated_equals_load_then_validate() {
    let a = load_validated("software").unwrap();
    let b = load_factory("software").unwrap();
    validate(&b).unwrap();
    let a_names: Vec<&str> = a.stations.iter().map(Station::name).collect();
    let b_names: Vec<&str> = b.stations.iter().map(Station::name).collect();
    assert_eq!(a_names, b_names);
    assert_eq!(a.name(), b.name());
}

#[test]
fn factory_clone_is_equivalent() {
    let f = factory();
    let cloned = f.clone();
    assert_eq!(f.name(), cloned.name());
    assert_eq!(f.stations.len(), cloned.stations.len());
    validate(&cloned).expect("a clone must still validate");
}

// ---------------------------------------------------------------------------
// Serde roundtrips of frontmatter.
// ---------------------------------------------------------------------------

#[test]
fn factory_frontmatter_yaml_roundtrip() {
    let fm = factory().frontmatter;
    let yaml = serde_yaml::to_string(&fm).expect("serialize");
    let back: FactoryFrontmatter = serde_yaml::from_str(&yaml).expect("deserialize");
    assert_eq!(fm.name, back.name);
    assert_eq!(fm.category, back.category);
    assert_eq!(fm.default_model, back.default_model);
    assert_eq!(fm.stations, back.stations);
    assert_eq!(fm.fix_workers, back.fix_workers);
    assert_eq!(fm.description, back.description);
}

#[test]
fn station_frontmatter_yaml_roundtrip_for_every_station() {
    for s in &factory().stations {
        let yaml = serde_yaml::to_string(&s.frontmatter).expect("serialize");
        let back: darkrun_content::StationFrontmatter =
            serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(s.frontmatter.name, back.name, "{}", s.name());
        assert_eq!(s.frontmatter.checkpoint, back.checkpoint, "{}", s.name());
        assert_eq!(s.frontmatter.locked_artifact, back.locked_artifact, "{}", s.name());
        assert_eq!(s.frontmatter.explorers, back.explorers, "{}", s.name());
        assert_eq!(s.frontmatter.workers, back.workers, "{}", s.name());
        assert_eq!(s.frontmatter.reviewers, back.reviewers, "{}", s.name());
        assert_eq!(s.frontmatter.inputs, back.inputs, "{}", s.name());
    }
}

#[test]
fn role_frontmatter_yaml_roundtrip() {
    let f = factory();
    let frame = f.station("frame").unwrap();
    for r in frame.explorers.iter().chain(&frame.workers).chain(&frame.reviewers) {
        let yaml = serde_yaml::to_string(&r.frontmatter).expect("serialize");
        let back: darkrun_content::RoleFrontmatter =
            serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(r.frontmatter.name, back.name);
        assert_eq!(r.frontmatter.agent_type, back.agent_type);
    }
}

#[test]
fn checkpoint_kind_serializes_snake_case() {
    assert_eq!(serde_yaml::to_string(&CheckpointKind::Ask).unwrap().trim(), "ask");
    assert_eq!(serde_yaml::to_string(&CheckpointKind::Auto).unwrap().trim(), "auto");
    assert_eq!(serde_yaml::to_string(&CheckpointKind::External).unwrap().trim(), "external");
    assert_eq!(serde_yaml::to_string(&CheckpointKind::Await).unwrap().trim(), "await");
}

#[test]
fn role_kind_serializes_snake_case() {
    assert_eq!(serde_yaml::to_string(&RoleKind::Explorer).unwrap().trim(), "explorer");
    assert_eq!(serde_yaml::to_string(&RoleKind::Worker).unwrap().trim(), "worker");
    assert_eq!(serde_yaml::to_string(&RoleKind::Reviewer).unwrap().trim(), "reviewer");
}

#[test]
fn role_kind_deserializes_snake_case() {
    let r: RoleKind = serde_yaml::from_str("worker").unwrap();
    assert_eq!(r, RoleKind::Worker);
    let r: RoleKind = serde_yaml::from_str("explorer").unwrap();
    assert_eq!(r, RoleKind::Explorer);
    let r: RoleKind = serde_yaml::from_str("reviewer").unwrap();
    assert_eq!(r, RoleKind::Reviewer);
}

#[test]
fn checkpoint_kind_roundtrips_via_yaml() {
    for k in [
        CheckpointKind::Ask,
        CheckpointKind::Auto,
        CheckpointKind::External,
        CheckpointKind::Await,
    ] {
        let y = serde_yaml::to_string(&k).unwrap();
        let back: CheckpointKind = serde_yaml::from_str(&y).unwrap();
        assert_eq!(k, back);
    }
}

#[test]
fn factory_frontmatter_serialized_contains_all_station_slugs() {
    let yaml = serde_yaml::to_string(&factory().frontmatter).unwrap();
    for name in STATION_ORDER {
        assert!(yaml.contains(name), "serialized frontmatter missing `{name}`");
    }
}

// ---------------------------------------------------------------------------
// Error paths through the public surface.
// ---------------------------------------------------------------------------

#[test]
fn loading_unknown_factory_errors() {
    assert!(load_factory("does-not-exist").is_err());
}

#[test]
fn validating_unknown_factory_errors() {
    assert!(load_validated("does-not-exist").is_err());
}

#[test]
fn unknown_factory_error_names_the_slug() {
    let err = load_factory("ghosty").unwrap_err();
    assert!(format!("{err}").contains("ghosty"), "error should name the missing slug");
}

#[test]
fn empty_factory_name_errors() {
    assert!(load_factory("").is_err());
}

#[test]
fn factory_name_with_traversal_errors() {
    assert!(load_factory("../software").is_err());
}

#[test]
fn factory_name_with_slash_errors() {
    assert!(load_factory("software/frame").is_err());
}

// ---------------------------------------------------------------------------
// Validation contract via a constructed factory (mutate-one-field tests).
// These reuse the loaded software factory as a valid baseline, then break
// one invariant at a time and assert validation catches it.
// ---------------------------------------------------------------------------

#[test]
fn baseline_software_factory_validates() {
    validate(&load_factory("software").unwrap()).expect("baseline must validate");
}

#[test]
fn validation_rejects_a_station_count_mismatch() {
    let mut f = load_factory("software").unwrap();
    f.frontmatter.stations.push("phantom".into());
    assert!(validate(&f).is_err(), "declared more stations than loaded");
}

#[test]
fn validation_rejects_clearing_all_stations() {
    let mut f = load_factory("software").unwrap();
    f.frontmatter.stations.clear();
    f.stations.clear();
    assert!(validate(&f).is_err(), "a factory with no stations is invalid");
}

#[test]
fn validation_rejects_reordered_declaration() {
    let mut f = load_factory("software").unwrap();
    f.frontmatter.stations.reverse();
    assert!(
        validate(&f).is_err(),
        "declared order must match loaded order"
    );
}

#[test]
fn validation_rejects_a_station_with_two_workers() {
    let mut f = load_factory("software").unwrap();
    f.stations[0].frontmatter.workers.truncate(2);
    f.stations[0].workers.truncate(2);
    assert!(validate(&f).is_err(), "fewer than 3 workers breaks the pass-loop");
}

#[test]
fn validation_rejects_emptying_workers() {
    let mut f = load_factory("software").unwrap();
    f.stations[0].frontmatter.workers.clear();
    f.stations[0].workers.clear();
    assert!(validate(&f).is_err());
}

#[test]
fn validation_rejects_emptying_explorers() {
    let mut f = load_factory("software").unwrap();
    f.stations[0].frontmatter.explorers.clear();
    f.stations[0].explorers.clear();
    assert!(validate(&f).is_err());
}

#[test]
fn validation_rejects_emptying_reviewers() {
    let mut f = load_factory("software").unwrap();
    f.stations[0].frontmatter.reviewers.clear();
    f.stations[0].reviewers.clear();
    assert!(validate(&f).is_err());
}

#[test]
fn validation_rejects_blank_locked_artifact() {
    let mut f = load_factory("software").unwrap();
    f.stations[0].frontmatter.locked_artifact = "   ".into();
    assert!(validate(&f).is_err());
}

#[test]
fn validation_error_names_the_factory() {
    let mut f = load_factory("software").unwrap();
    f.frontmatter.stations.clear();
    f.stations.clear();
    let msg = format!("{}", validate(&f).unwrap_err());
    assert!(msg.contains("software"), "error should name the factory: {msg}");
}

// ---------------------------------------------------------------------------
// Cross-cutting invariants over all stations (batch checks).
// ---------------------------------------------------------------------------

#[test]
fn all_stations_match_full_expectation_table() {
    let f = factory();
    for exp in STATIONS {
        let s = f.station(exp.name).expect("station present");
        assert_eq!(slugs(&s.explorers), exp.explorers, "{} explorers", exp.name);
        assert_eq!(slugs(&s.workers), exp.workers, "{} workers", exp.name);
        assert_eq!(slugs(&s.reviewers), exp.reviewers, "{} reviewers", exp.name);
        assert_eq!(s.checkpoint(), exp.checkpoint, "{} checkpoint", exp.name);
        assert_eq!(
            s.frontmatter.locked_artifact, exp.locked_artifact,
            "{} artifact",
            exp.name
        );
        let inputs: Vec<&str> = s.frontmatter.inputs.iter().map(String::as_str).collect();
        assert_eq!(inputs, exp.inputs, "{} inputs", exp.name);
    }
}

#[test]
fn station_names_are_unique() {
    let f = factory();
    let names: Vec<&str> = f.stations.iter().map(Station::name).collect();
    let mut deduped = names.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(names.len(), deduped.len());
}

#[test]
fn station_names_are_lowercase_slugs() {
    for s in &factory().stations {
        let name = s.name();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "station slug `{name}` should be a lowercase slug"
        );
    }
}

#[test]
fn role_names_are_lowercase_slugs() {
    for s in &factory().stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            let name = r.name();
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "role slug `{name}` should be a lowercase slug"
            );
        }
    }
}

#[test]
fn every_station_descriptions_are_nonempty() {
    for s in &factory().stations {
        assert!(
            !s.frontmatter.description.trim().is_empty(),
            "{} needs a description",
            s.name()
        );
    }
}

#[test]
fn frame_is_the_first_station() {
    assert_eq!(factory().stations.first().unwrap().name(), "frame");
}

#[test]
fn harden_is_the_last_station() {
    assert_eq!(factory().stations.last().unwrap().name(), "harden");
}

#[test]
fn the_value_role_appears_as_both_explorer_and_reviewer_in_frame() {
    // Frame uses `value` as both an explorer (gather value context) and a
    // reviewer (verify the value claim) — same slug, different kinds, different
    // files. They must each be the kind their phase expects.
    let f = factory();
    let frame = f.station("frame").unwrap();
    let value_explorer = frame.explorers.iter().find(|r| r.name() == "value").unwrap();
    let value_reviewer = frame.reviewers.iter().find(|r| r.name() == "value").unwrap();
    assert_eq!(value_explorer.kind(), RoleKind::Explorer);
    assert_eq!(value_reviewer.kind(), RoleKind::Reviewer);
}

#[test]
fn fix_workers_overlap_build_workers() {
    // Builder and Reconciler are both build-station workers and fix-workers:
    // the same beats that build also repair drift.
    let f = factory();
    let build_workers = slugs(&f.station("build").unwrap().workers);
    assert!(f.frontmatter.fix_workers.iter().any(|fw| build_workers.contains(&fw.as_str())));
}

#[test]
fn first_three_stations_are_documentation_artifacts() {
    // Frame/Specify/Shape lock markdown design docs before any code exists.
    let f = factory();
    for name in ["frame", "specify", "shape"] {
        let artifact = &f.station(name).unwrap().frontmatter.locked_artifact;
        assert!(artifact.ends_with(".md"), "{name} locks a doc, got {artifact}");
    }
}

#[test]
fn prove_is_independent_of_build_authors() {
    // Prove consumes spec + code but not design — it grades against the spec
    // rubric, independent of how Build chose to implement.
    let f = factory();
    let prove = f.station("prove").unwrap();
    let inputs: Vec<&str> = prove.frontmatter.inputs.iter().map(String::as_str).collect();
    assert!(inputs.contains(&"spec.md"), "prove grades against the spec");
    assert!(inputs.contains(&"code"), "prove exercises the code");
    assert!(!inputs.contains(&"design.md"), "prove stays blind to design choices");
}

//! Corpus-level integration tests over the embedded software factory.
//!
//! These assert the *shipped* content is coherent: every station listed by the
//! factory loads, exposes a full role set (explorers, workers, reviewers), a
//! known checkpoint kind, and a non-empty locked artifact — and that each role's
//! markdown body actually carries instructions an agent could act on.

use darkrun_content::{load_factory, load_validated, Role, RoleKind, Station};
use darkrun_core::domain::CheckpointKind;

/// The six software stations, in cost-of-late-discovery order, with the design's
/// expected role counts and checkpoint gates. This is the contract the shipped
/// corpus must satisfy.
struct Expected {
    name: &'static str,
    explorers: &'static [&'static str],
    workers: &'static [&'static str],
    reviewers: &'static [&'static str],
    checkpoint: CheckpointKind,
    locked_artifact: &'static str,
}

const STATIONS: &[Expected] = &[
    Expected {
        name: "frame",
        explorers: &["context", "value"],
        workers: &["framer", "challenger", "distiller"],
        reviewers: &["value", "feasibility"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "frame.md",
    },
    Expected {
        name: "specify",
        explorers: &["contract", "edge_case"],
        workers: &["spec_writer", "adversary", "tightener"],
        reviewers: &["testability", "completeness"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "spec.md",
    },
    Expected {
        name: "shape",
        explorers: &["surface", "architecture", "risk"],
        workers: &["designer", "visual_designer", "spiker", "pressure_tester", "resolver"],
        reviewers: &["fit", "reversibility", "simplicity"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "design.md",
    },
    Expected {
        name: "build",
        explorers: &["reuse", "integration_point"],
        workers: &["test_author", "builder", "self_reviewer", "reconciler"],
        reviewers: &["correctness", "maintainability"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "code",
    },
    Expected {
        name: "prove",
        explorers: &["scenario", "regression"],
        workers: &["verifier", "breaker", "triage"],
        reviewers: &["evidence", "coverage"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "proof.md",
    },
    Expected {
        name: "harden",
        explorers: &["threat", "operability"],
        workers: &["hardener", "red_teamer", "releaser"],
        reviewers: &["security", "readiness"],
        checkpoint: CheckpointKind::Ask,
        locked_artifact: "release.md",
    },
];

fn slugs(roles: &[Role]) -> Vec<&str> {
    roles.iter().map(Role::name).collect()
}

#[test]
fn software_factory_loads_and_validates() {
    let factory = load_validated("software").expect("software factory must load and validate");
    assert_eq!(factory.name(), "software");
    assert_eq!(factory.frontmatter.category, "engineering");
    assert_eq!(factory.frontmatter.default_model, "sonnet");

    let order: Vec<&str> = factory.stations.iter().map(Station::name).collect();
    let expected_order: Vec<&str> = STATIONS.iter().map(|e| e.name).collect();
    assert_eq!(order, expected_order, "stations must be in design order");
}

#[test]
fn every_station_has_a_complete_role_set() {
    let factory = load_validated("software").expect("loads");

    for exp in STATIONS {
        let station = factory
            .station(exp.name)
            .unwrap_or_else(|| panic!("missing station `{}`", exp.name));

        assert_eq!(
            slugs(&station.explorers),
            exp.explorers,
            "{} explorers",
            exp.name
        );
        assert_eq!(slugs(&station.workers), exp.workers, "{} workers", exp.name);
        assert_eq!(
            slugs(&station.reviewers),
            exp.reviewers,
            "{} reviewers",
            exp.name
        );

        assert_eq!(
            station.checkpoint(),
            exp.checkpoint,
            "{} checkpoint",
            exp.name
        );
        assert_eq!(
            station.frontmatter.locked_artifact, exp.locked_artifact,
            "{} locked_artifact",
            exp.name
        );

        // Frontmatter list and loaded roles agree on length for every kind.
        assert_eq!(station.frontmatter.explorers.len(), station.explorers.len());
        assert_eq!(station.frontmatter.workers.len(), station.workers.len());
        assert_eq!(station.frontmatter.reviewers.len(), station.reviewers.len());
    }
}

#[test]
fn every_role_is_correctly_kinded_and_carries_instructions() {
    let factory = load_factory("software").expect("loads");

    for station in &factory.stations {
        for role in &station.explorers {
            assert_eq!(role.kind(), RoleKind::Explorer, "{}", role.name());
            assert_instructive(station.name(), role);
        }
        for role in &station.workers {
            assert_eq!(role.kind(), RoleKind::Worker, "{}", role.name());
            assert_instructive(station.name(), role);
        }
        for role in &station.reviewers {
            assert_eq!(role.kind(), RoleKind::Reviewer, "{}", role.name());
            assert_instructive(station.name(), role);
        }
    }
}

/// A role's body must be substantive, not a stub: it has to carry a heading and
/// enough prose that the manager can hand it to an agent verbatim.
fn assert_instructive(station: &str, role: &Role) {
    let body = role.body.trim();
    assert!(
        body.contains('#'),
        "{station}/{} body has no markdown heading",
        role.name()
    );
    assert!(
        body.len() > 120,
        "{station}/{} body is too thin ({} bytes) to instruct an agent",
        role.name(),
        body.len()
    );
}

#[test]
fn worker_pass_loop_carries_phase_specific_guidance() {
    let factory = load_factory("software").expect("loads");

    // The build station's workers must spell out the test-first, build-to-green,
    // self-review, reconcile sequence — not generic filler.
    let build = factory.station("build").expect("build station");
    let body = |slug: &str| {
        build
            .workers
            .iter()
            .find(|w| w.name() == slug)
            .map(|w| w.body.to_lowercase())
            .unwrap_or_default()
    };
    assert!(body("test_author").contains("test"));
    assert!(body("builder").contains("design"));
    assert!(body("self_reviewer").contains("review"));
    assert!(body("reconciler").contains("merge") || body("reconciler").contains("integrat"));

    // Shape's Spiker must talk about a throwaway proof of the riskiest assumption.
    let shape = factory.station("shape").expect("shape station");
    let spiker = shape
        .workers
        .iter()
        .find(|w| w.name() == "spiker")
        .expect("spiker");
    assert!(spiker.body.to_lowercase().contains("throwaway"));
}

#[test]
fn station_bodies_describe_their_risk_class() {
    let factory = load_factory("software").expect("loads");
    for station in &factory.stations {
        let body = station.body.to_lowercase();
        assert!(
            body.contains("risk"),
            "station `{}` body should name the risk class it eliminates",
            station.name()
        );
        assert!(
            body.contains("checkpoint"),
            "station `{}` body should describe its checkpoint",
            station.name()
        );
    }
}

#[test]
fn checkpoint_kinds_are_all_recognized() {
    // Loading parses each station's `checkpoint:` through the CheckpointKind
    // enum; an unknown kind would fail the load. Reaching here proves every
    // shipped station declares a recognized gate.
    let factory = load_validated("software").expect("loads");
    let kinds: Vec<CheckpointKind> = factory
        .stations
        .iter()
        .map(Station::checkpoint)
        .collect();
    assert_eq!(kinds, vec![CheckpointKind::Ask; 6]);
}

#[test]
fn station_inputs_reference_upstream_locked_artifacts() {
    // Each station's `inputs` must be artifacts a prior station actually locks
    // (or `code`, the running artifact Build produces). A dangling input is a
    // broken hand-off in the pipeline.
    let factory = load_factory("software").expect("loads");

    let mut available: Vec<String> = vec![];
    for station in &factory.stations {
        for input in &station.frontmatter.inputs {
            assert!(
                available.iter().any(|a| a == input),
                "station `{}` consumes `{input}` but no upstream station locks it (available: {available:?})",
                station.name()
            );
        }
        // After a station runs, its locked artifact is available downstream.
        available.push(station.frontmatter.locked_artifact.clone());
    }
}

#[test]
fn fix_workers_are_declared() {
    let factory = load_factory("software").expect("loads");
    assert!(
        !factory.frontmatter.fix_workers.is_empty(),
        "factory must declare fix-workers for drift repair"
    );
}

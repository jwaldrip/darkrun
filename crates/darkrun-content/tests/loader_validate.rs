//! Comprehensive integration coverage for the darkrun-content loader and
//! validator, area `loader_validate`.
//!
//! Three surfaces are driven:
//!  - the corpus loader (`list_factories`, `load_factory`, `load_validated`)
//!    over the *shipped* software factory — real role sets, bodies,
//!    the input/locked-artifact hand-off DAG, determinism, idempotency;
//!  - the structural validator `validate()` driven against in-memory `Factory`
//!    values assembled through the crate's public model fields, mutating exactly
//!    one rule at a time to exercise every passing branch and every failure
//!    branch with its boundary conditions;
//!  - `RoleKind` serde — snake_case roundtrips through serde_yaml and
//!    the core frontmatter parser, the same path the loader walks.

use darkrun_content::{
    list_factories, load_factory, load_validated, ContentError, Factory, FactoryFrontmatter, Role,
    RoleFrontmatter, RoleKind, Station, StationFrontmatter,
};

// ---------------------------------------------------------------------------
// Builders — assemble structurally valid model values, then mutate one field.
// ---------------------------------------------------------------------------

/// A role with a markdown body substantial enough to look like real content.
fn role(name: &str, kind: RoleKind) -> Role {
    Role {
        frontmatter: RoleFrontmatter {
            name: name.to_string(),
            agent_type: None,
            model: None,
                interpretation: None,
                role: None,
                applies_to: vec![],
        },
        body: format!("# {name}\n\nThis role carries enough prose to instruct an agent verbatim."),
        kind,
    }
}

fn role_with_model(name: &str, kind: RoleKind, model: &str) -> Role {
    let mut r = role(name, kind);
    r.frontmatter.model = Some(model.to_string());
    r
}

/// A minimal structurally-valid single-station factory. Each failure test
/// mutates exactly one field of a fresh copy of this baseline.
fn valid_station() -> Station {
    Station {
        frontmatter: StationFrontmatter {
            name: "s1".into(),
            description: "a station".into(),
            kills: "a-risk".into(),
            label: None, optional: false,
            explorers: vec!["e1".into()],
            workers: vec!["w1".into(), "w2".into(), "w3".into()],
            fix_workers: vec![],
            reviewers: vec!["r1".into()],
            locked_artifact: "out.md".into(),
            inputs: vec![],
            inputs_waived: vec![],
        },
        body: "# s1\n\nstation body".into(),
        explorers: vec![role("e1", RoleKind::Explorer)],
        workers: vec![
            role("w1", RoleKind::Worker),
            role("w2", RoleKind::Worker),
            role("w3", RoleKind::Worker),
        ],
        reviewers: vec![role("r1", RoleKind::Reviewer)],
    }
}

fn valid_factory() -> Factory {
    Factory {
        frontmatter: FactoryFrontmatter {
            name: "demo".into(),
            description: "demo factory".into(),
            category: "engineering".into(),
            default_model: "sonnet".into(),
            inherits: None,
            stations: vec!["s1".into()],
            fix_workers: vec![],
            reviewers: vec![],
            reflections: vec![],
            surfaces: vec![],
        },
        body: "# demo".into(),
        stations: vec![valid_station()],
        run_reviewers: vec![],
        reflections: vec![],
    }
}

/// A second valid station named `s2` whose role slugs do not collide with `s1`.
/// Waives the baseline `out.md` artifact every station locks, so a chain of these
/// satisfies cross-station input coverage (no silent drop) without each needing
/// to consume the prior's output.
fn valid_station_named(name: &str) -> Station {
    let mut s = valid_station();
    s.frontmatter.name = name.to_string();
    s.frontmatter.inputs_waived = vec!["out.md".into()];
    s.body = format!("# {name}");
    s
}

#[test]
fn cross_station_coverage_rejects_a_silent_drop_and_accepts_a_waiver() {
    // s1 locks out.md; s2 neither consumes nor waives it → silent drop, rejected.
    let mut f = valid_factory();
    f.frontmatter.stations = vec!["s1".into(), "s2".into()];
    let mut s2 = valid_station();
    s2.frontmatter.name = "s2".into();
    s2.frontmatter.inputs = vec![]; // does not carry out.md
    s2.frontmatter.inputs_waived = vec![]; // and does not waive it
    f.stations = vec![valid_station(), s2];
    let msg = match darkrun_content::validate(&f) {
        Err(ContentError::Invalid { message, .. }) => message,
        other => panic!("expected Invalid, got {other:?}"),
    };
    assert!(msg.contains("silently drops upstream artifact `out.md`"), "{msg}");

    // Waiving it explicitly is accepted (conscious drop, not silent).
    f.stations[1].frontmatter.inputs_waived = vec!["out.md".into()];
    assert!(darkrun_content::validate(&f).is_ok());

    // Carrying it forward as an input is also accepted.
    f.stations[1].frontmatter.inputs_waived = vec![];
    f.stations[1].frontmatter.inputs = vec!["out.md".into()];
    assert!(darkrun_content::validate(&f).is_ok());
}

#[test]
fn software_prove_waives_frame_and_design_for_coverage() {
    // The shipped software factory carries the distillation forward; prove
    // consciously waives frame.md + design.md (it verifies against spec + code).
    let f = load_validated("software").expect("software validates with coverage");
    let prove = f.station("prove").unwrap();
    assert!(prove.frontmatter.inputs_waived.contains(&"frame.md".to_string()));
    assert!(prove.frontmatter.inputs_waived.contains(&"design.md".to_string()));
}

/// A two-station factory, both stations structurally valid.
fn valid_two_station_factory() -> Factory {
    let mut f = valid_factory();
    f.frontmatter.stations = vec!["s1".into(), "s2".into()];
    f.stations = vec![valid_station(), valid_station_named("s2")];
    f
}

/// Extract the `Invalid` message, panicking on any other outcome.
fn message(factory: &Factory) -> String {
    match darkrun_content::validate(factory) {
        Err(ContentError::Invalid { message, .. }) => message,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

/// Extract the factory slug carried by an `Invalid` error.
fn invalid_factory_slug(factory: &Factory) -> String {
    match darkrun_content::validate(factory) {
        Err(ContentError::Invalid { factory, .. }) => factory,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// ===========================================================================
// SECTION 1 — list_factories
// ===========================================================================

#[test]
fn list_factories_includes_software() {
    assert!(list_factories().contains(&"software".to_string()));
}

#[test]
fn list_factories_is_non_empty() {
    assert!(!list_factories().is_empty());
}

#[test]
fn list_factories_is_sorted() {
    let names = list_factories();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "list_factories must be sorted ascending");
}

#[test]
fn list_factories_is_deduped() {
    let names = list_factories();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(names.len(), unique.len(), "list_factories must not repeat a slug");
}

#[test]
fn list_factories_is_deterministic_across_calls() {
    assert_eq!(list_factories(), list_factories());
}

#[test]
fn list_factories_repeated_calls_agree_three_times() {
    let a = list_factories();
    let b = list_factories();
    let c = list_factories();
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn list_factories_entries_are_non_empty_slugs() {
    for slug in list_factories() {
        assert!(!slug.is_empty(), "factory slug must be non-empty");
        assert!(!slug.contains('/'), "factory slug must not contain a path separator: {slug}");
    }
}

#[test]
fn list_factories_slugs_have_no_whitespace() {
    for slug in list_factories() {
        assert!(
            !slug.chars().any(char::is_whitespace),
            "factory slug `{slug}` should not contain whitespace"
        );
    }
}

#[test]
fn every_listed_factory_loads() {
    for slug in list_factories() {
        load_factory(&slug).unwrap_or_else(|e| panic!("listed factory `{slug}` failed to load: {e:?}"));
    }
}

#[test]
fn every_listed_factory_validates() {
    for slug in list_factories() {
        load_validated(&slug)
            .unwrap_or_else(|e| panic!("listed factory `{slug}` failed to validate: {e:?}"));
    }
}

#[test]
fn every_listed_factory_name_matches_its_slug() {
    for slug in list_factories() {
        let f = load_factory(&slug).expect("load");
        assert_eq!(f.name(), slug, "loaded factory name must equal its directory slug");
    }
}

// ===========================================================================
// SECTION 2 — load_factory: shipped software factory
// ===========================================================================

#[test]
fn software_loads() {
    assert!(load_factory("software").is_ok());
}

#[test]
fn software_name_is_software() {
    assert_eq!(load_factory("software").unwrap().name(), "software");
}

#[test]
fn software_frontmatter_name_matches_accessor() {
    let f = load_factory("software").unwrap();
    assert_eq!(f.frontmatter.name, f.name());
}

#[test]
fn software_category_is_engineering() {
    assert_eq!(load_factory("software").unwrap().frontmatter.category, "engineering");
}

#[test]
fn software_default_model_is_sonnet() {
    assert_eq!(load_factory("software").unwrap().frontmatter.default_model, "sonnet");
}

#[test]
fn software_has_six_stations() {
    assert_eq!(load_factory("software").unwrap().stations.len(), 6);
}

#[test]
fn software_station_order_matches_design() {
    let f = load_factory("software").unwrap();
    let order: Vec<&str> = f.stations.iter().map(Station::name).collect();
    assert_eq!(order, vec!["frame", "specify", "shape", "build", "prove", "harden"]);
}

#[test]
fn software_declared_station_order_matches_loaded_order() {
    let f = load_factory("software").unwrap();
    let declared: Vec<&str> = f.frontmatter.stations.iter().map(String::as_str).collect();
    let loaded: Vec<&str> = f.stations.iter().map(Station::name).collect();
    assert_eq!(declared, loaded, "loader must preserve declared station order");
}

#[test]
fn software_body_describes_cost_of_late_discovery() {
    let f = load_factory("software").unwrap();
    assert!(f.body.contains("cost-of-late-discovery"));
}

#[test]
fn software_body_names_the_class_of_risk() {
    let f = load_factory("software").unwrap();
    assert!(f.body.contains("class-of-risk-eliminated"));
}

#[test]
fn software_body_is_substantial() {
    let f = load_factory("software").unwrap();
    assert!(f.body.trim().len() > 200, "factory overview should be substantial");
}

#[test]
fn software_declares_fix_workers() {
    let f = load_factory("software").unwrap();
    assert!(!f.frontmatter.fix_workers.is_empty(), "factory must declare fix-workers");
}

#[test]
fn software_fix_workers_are_unique() {
    let f = load_factory("software").unwrap();
    let mut fw = f.frontmatter.fix_workers.clone();
    let len = fw.len();
    fw.sort();
    fw.dedup();
    assert_eq!(fw.len(), len, "fix-workers should not repeat");
}

#[test]
fn software_load_is_deterministic() {
    let a = load_factory("software").unwrap();
    let b = load_factory("software").unwrap();
    let names_a: Vec<&str> = a.stations.iter().map(Station::name).collect();
    let names_b: Vec<&str> = b.stations.iter().map(Station::name).collect();
    assert_eq!(names_a, names_b);
    assert_eq!(a.body, b.body);
}

// ===========================================================================
// SECTION 3 — station lookup
// ===========================================================================

#[test]
fn station_lookup_finds_each_declared_station() {
    let f = load_factory("software").unwrap();
    for slug in ["frame", "specify", "shape", "build", "prove", "harden"] {
        assert!(f.station(slug).is_some(), "station `{slug}` must be findable");
    }
}

#[test]
fn station_lookup_returns_none_for_unknown() {
    let f = load_factory("software").unwrap();
    assert!(f.station("nonexistent").is_none());
}

#[test]
fn station_lookup_is_case_sensitive() {
    let f = load_factory("software").unwrap();
    assert!(f.station("Frame").is_none(), "lookup must be case-sensitive");
}

#[test]
fn station_lookup_returns_matching_name() {
    let f = load_factory("software").unwrap();
    let s = f.station("shape").unwrap();
    assert_eq!(s.name(), "shape");
}

#[test]
fn station_lookup_empty_string_is_none() {
    let f = load_factory("software").unwrap();
    assert!(f.station("").is_none());
}

// ===========================================================================
// SECTION 4 — per-station role sets (the shipped contract)
// ===========================================================================

struct Expected {
    name: &'static str,
    explorers: &'static [&'static str],
    workers: &'static [&'static str],
    reviewers: &'static [&'static str],
    locked_artifact: &'static str,
}

const STATIONS: &[Expected] = &[
    Expected {
        name: "frame",
        explorers: &["context", "value"],
        workers: &["framer", "challenger", "distiller"],
        reviewers: &["value", "feasibility"],
        locked_artifact: "frame.md",
    },
    Expected {
        name: "specify",
        explorers: &["contract", "edge_case"],
        workers: &["spec_writer", "adversary", "tightener"],
        reviewers: &["testability", "completeness"],
        locked_artifact: "spec.md",
    },
    Expected {
        name: "shape",
        explorers: &["surface", "architecture", "risk"],
        workers: &["designer", "visual_designer", "spiker", "pressure_tester", "resolver"],
        reviewers: &["fit", "reversibility", "simplicity"],
        locked_artifact: "design.md",
    },
    Expected {
        name: "build",
        explorers: &["reuse", "integration_point"],
        workers: &["test_author", "builder", "self_reviewer", "reconciler"],
        reviewers: &["correctness", "maintainability"],
        locked_artifact: "code",
    },
    Expected {
        name: "prove",
        explorers: &["scenario", "regression"],
        workers: &["verifier", "breaker", "triage"],
        reviewers: &["evidence", "coverage"],
        locked_artifact: "proof.md",
    },
    Expected {
        name: "harden",
        explorers: &["threat", "operability"],
        workers: &["hardener", "red_teamer", "releaser"],
        reviewers: &["security", "readiness"],
        locked_artifact: "release.md",
    },
];

fn slugs(roles: &[Role]) -> Vec<&str> {
    roles.iter().map(Role::name).collect()
}

#[test]
fn every_station_loads_its_declared_explorers() {
    let f = load_factory("software").unwrap();
    for exp in STATIONS {
        let s = f.station(exp.name).unwrap();
        assert_eq!(slugs(&s.explorers), exp.explorers, "{} explorers", exp.name);
    }
}

#[test]
fn every_station_loads_its_declared_workers() {
    let f = load_factory("software").unwrap();
    for exp in STATIONS {
        let s = f.station(exp.name).unwrap();
        assert_eq!(slugs(&s.workers), exp.workers, "{} workers", exp.name);
    }
}

#[test]
fn every_station_loads_its_declared_reviewers() {
    let f = load_factory("software").unwrap();
    for exp in STATIONS {
        let s = f.station(exp.name).unwrap();
        assert_eq!(slugs(&s.reviewers), exp.reviewers, "{} reviewers", exp.name);
    }
}

#[test]
fn every_station_has_expected_locked_artifact() {
    let f = load_factory("software").unwrap();
    for exp in STATIONS {
        let s = f.station(exp.name).unwrap();
        assert_eq!(s.frontmatter.locked_artifact, exp.locked_artifact, "{} artifact", exp.name);
    }
}

#[test]
fn every_station_frontmatter_and_loaded_explorers_agree_in_count() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert_eq!(
            s.frontmatter.explorers.len(),
            s.explorers.len(),
            "{} explorer count mismatch",
            s.name()
        );
    }
}

#[test]
fn every_station_frontmatter_and_loaded_workers_agree_in_count() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert_eq!(s.frontmatter.workers.len(), s.workers.len(), "{} worker count", s.name());
    }
}

#[test]
fn every_station_frontmatter_and_loaded_reviewers_agree_in_count() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert_eq!(s.frontmatter.reviewers.len(), s.reviewers.len(), "{} reviewer count", s.name());
    }
}

#[test]
fn every_station_loaded_role_name_matches_its_reference() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for (decl, role) in s.frontmatter.explorers.iter().zip(s.explorers.iter()) {
            assert_eq!(decl, role.name(), "{} explorer ref/name mismatch", s.name());
        }
        for (decl, role) in s.frontmatter.workers.iter().zip(s.workers.iter()) {
            assert_eq!(decl, role.name(), "{} worker ref/name mismatch", s.name());
        }
        for (decl, role) in s.frontmatter.reviewers.iter().zip(s.reviewers.iter()) {
            assert_eq!(decl, role.name(), "{} reviewer ref/name mismatch", s.name());
        }
    }
}

#[test]
fn every_station_has_at_least_one_explorer() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(!s.explorers.is_empty(), "{} has no explorers", s.name());
    }
}

#[test]
fn every_station_has_at_least_three_workers() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(s.workers.len() >= 3, "{} has fewer than 3 workers", s.name());
    }
}

#[test]
fn every_station_has_at_least_one_reviewer() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(!s.reviewers.is_empty(), "{} has no reviewers", s.name());
    }
}

#[test]
fn every_station_locks_a_non_empty_artifact() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(!s.frontmatter.locked_artifact.trim().is_empty(), "{} artifact empty", s.name());
    }
}

#[test]
fn locked_artifacts_are_distinct_across_stations() {
    let f = load_factory("software").unwrap();
    let mut arts: Vec<&str> = f.stations.iter().map(|s| s.frontmatter.locked_artifact.as_str()).collect();
    let len = arts.len();
    arts.sort_unstable();
    arts.dedup();
    assert_eq!(arts.len(), len, "each station should lock a distinct artifact");
}

// ===========================================================================
// SECTION 5 — role kinds and bodies (markdown body present)
// ===========================================================================

#[test]
fn every_explorer_is_kinded_explorer() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in &s.explorers {
            assert_eq!(r.kind(), RoleKind::Explorer, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn every_worker_is_kinded_worker() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in &s.workers {
            assert_eq!(r.kind(), RoleKind::Worker, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn every_reviewer_is_kinded_reviewer() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in &s.reviewers {
            assert_eq!(r.kind(), RoleKind::Reviewer, "{}/{}", s.name(), r.name());
        }
    }
}

#[test]
fn role_kind_accessor_matches_frontmatter_agent_type() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in &s.explorers { assert_eq!(r.kind(), RoleKind::Explorer); }
        for r in &s.workers { assert_eq!(r.kind(), RoleKind::Worker); }
        for r in &s.reviewers { assert_eq!(r.kind(), RoleKind::Reviewer); }
    }
}

#[test]
fn role_name_accessor_matches_frontmatter_name() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert_eq!(r.name(), r.frontmatter.name);
        }
    }
}

#[test]
fn every_role_body_is_non_empty() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert!(!r.body.trim().is_empty(), "{}/{} body empty", s.name(), r.name());
        }
    }
}

#[test]
fn every_role_body_has_a_markdown_heading() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        for r in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            assert!(r.body.contains('#'), "{}/{} body has no heading", s.name(), r.name());
        }
    }
}

#[test]
fn every_role_body_is_substantial() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
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
fn every_station_body_is_non_empty() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(!s.body.trim().is_empty(), "{} body empty", s.name());
    }
}

#[test]
fn every_station_body_names_its_risk_class() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(s.body.to_lowercase().contains("risk"), "{} body omits risk", s.name());
    }
}

#[test]
fn every_station_body_describes_its_checkpoint() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(s.body.to_lowercase().contains("checkpoint"), "{} body omits checkpoint", s.name());
    }
}

#[test]
fn frame_station_distiller_is_terminal_worker() {
    let f = load_factory("software").unwrap();
    let frame = f.station("frame").unwrap();
    assert_eq!(frame.workers.last().unwrap().name(), "distiller");
}

#[test]
fn shape_spiker_body_mentions_throwaway() {
    let f = load_factory("software").unwrap();
    let shape = f.station("shape").unwrap();
    let spiker = shape.workers.iter().find(|w| w.name() == "spiker").unwrap();
    assert!(spiker.body.to_lowercase().contains("throwaway"));
}

#[test]
fn build_test_author_body_mentions_test() {
    let f = load_factory("software").unwrap();
    let build = f.station("build").unwrap();
    let ta = build.workers.iter().find(|w| w.name() == "test_author").unwrap();
    assert!(ta.body.to_lowercase().contains("test"));
}

#[test]
fn harden_releaser_is_terminal_worker() {
    let f = load_factory("software").unwrap();
    let harden = f.station("harden").unwrap();
    assert_eq!(harden.workers.last().unwrap().name(), "releaser");
}

// ===========================================================================
// SECTION 6 — input / locked-artifact hand-off DAG (shipped corpus)
// ===========================================================================

#[test]
fn station_inputs_reference_upstream_locked_artifacts() {
    let f = load_factory("software").unwrap();
    let mut available: Vec<String> = vec![];
    for s in &f.stations {
        for input in &s.frontmatter.inputs {
            assert!(
                available.iter().any(|a| a == input),
                "station `{}` consumes `{input}` not yet locked (available: {available:?})",
                s.name()
            );
        }
        available.push(s.frontmatter.locked_artifact.clone());
    }
}

#[test]
fn first_station_consumes_no_inputs() {
    let f = load_factory("software").unwrap();
    let first = &f.stations[0];
    assert!(first.frontmatter.inputs.is_empty(), "the opening station should have no inputs");
}

#[test]
fn no_station_consumes_its_own_locked_artifact() {
    let f = load_factory("software").unwrap();
    for s in &f.stations {
        assert!(
            !s.frontmatter.inputs.contains(&s.frontmatter.locked_artifact),
            "{} consumes the artifact it locks",
            s.name()
        );
    }
}

// ===========================================================================
// SECTION 7 — error paths: missing factory / files
// ===========================================================================

#[test]
fn missing_factory_is_factory_not_found() {
    match load_factory("nonexistent") {
        Err(ContentError::FactoryNotFound(name)) => assert_eq!(name, "nonexistent"),
        other => panic!("expected FactoryNotFound, got {other:?}"),
    }
}

#[test]
fn missing_factory_via_load_validated_is_factory_not_found() {
    assert!(matches!(load_validated("nope"), Err(ContentError::FactoryNotFound(_))));
}

#[test]
fn empty_factory_name_is_factory_not_found() {
    assert!(matches!(load_factory(""), Err(ContentError::FactoryNotFound(_))));
}

#[test]
fn factory_name_with_slash_is_not_found() {
    assert!(matches!(load_factory("software/frame"), Err(ContentError::FactoryNotFound(_))));
}

#[test]
fn factory_name_with_traversal_is_not_found() {
    assert!(matches!(load_factory("../software"), Err(ContentError::FactoryNotFound(_))));
}

#[test]
fn factory_not_found_error_carries_the_requested_name() {
    match load_factory("ghost-factory") {
        Err(ContentError::FactoryNotFound(n)) => assert_eq!(n, "ghost-factory"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn factory_not_found_display_mentions_name() {
    let err = load_factory("phantom").unwrap_err();
    assert!(err.to_string().contains("phantom"), "display: {err}");
}

// ===========================================================================
// SECTION 8 — validate(): the baseline passes
// ===========================================================================

#[test]
fn baseline_factory_validates() {
    darkrun_content::validate(&valid_factory()).expect("baseline must validate");
}

#[test]
fn two_station_baseline_validates() {
    darkrun_content::validate(&valid_two_station_factory()).expect("two-station baseline must validate");
}

#[test]
fn baseline_with_exactly_three_workers_validates() {
    // Three is the lower boundary for Make->Challenge->Resolve.
    let f = valid_factory();
    assert_eq!(f.stations[0].workers.len(), 3);
    darkrun_content::validate(&f).expect("exactly three workers is valid");
}

#[test]
fn baseline_with_four_workers_validates() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers.push("w4".into());
    f.stations[0].workers.push(role("w4", RoleKind::Worker));
    darkrun_content::validate(&f).expect("four workers is valid");
}

#[test]
fn baseline_with_many_explorers_validates() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers = vec!["e1".into(), "e2".into(), "e3".into()];
    f.stations[0].explorers = vec![
        role("e1", RoleKind::Explorer),
        role("e2", RoleKind::Explorer),
        role("e3", RoleKind::Explorer),
    ];
    darkrun_content::validate(&f).expect("multiple explorers valid");
}

#[test]
fn baseline_with_many_reviewers_validates() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.reviewers = vec!["r1".into(), "r2".into()];
    f.stations[0].reviewers = vec![
        role("r1", RoleKind::Reviewer),
        role("r2", RoleKind::Reviewer),
    ];
    darkrun_content::validate(&f).expect("multiple reviewers valid");
}

#[test]
fn baseline_validate_is_idempotent() {
    let f = valid_factory();
    darkrun_content::validate(&f).expect("first");
    darkrun_content::validate(&f).expect("second");
    darkrun_content::validate(&f).expect("third");
}

#[test]
fn validate_does_not_mutate_the_factory() {
    let f = valid_factory();
    let before: Vec<String> = f.frontmatter.stations.clone();
    darkrun_content::validate(&f).expect("valid");
    assert_eq!(f.frontmatter.stations, before);
}

#[test]
fn role_with_model_override_validates() {
    let mut f = valid_factory();
    f.stations[0].workers[0] = role_with_model("w1", RoleKind::Worker, "opus");
    darkrun_content::validate(&f).expect("a model override must not break validation");
    assert_eq!(f.stations[0].workers[0].frontmatter.model.as_deref(), Some("opus"));
}

// ===========================================================================
// SECTION 9 — validate(): empty / mismatched stations
// ===========================================================================

#[test]
fn rejects_factory_with_no_stations() {
    let mut f = valid_factory();
    f.frontmatter.stations.clear();
    f.stations.clear();
    assert!(message(&f).contains("no stations"));
}

#[test]
fn no_stations_error_carries_factory_slug() {
    let mut f = valid_factory();
    f.frontmatter.stations.clear();
    f.stations.clear();
    assert_eq!(invalid_factory_slug(&f), "demo");
}

#[test]
fn frontmatter_stations_is_ignored_by_validation() {
    // The station spine is the fixed Position::FLOW (resolved by the loader);
    // `frontmatter.stations` is vestigial and the validator does not check the
    // loaded stations against it. A mismatched declaration validates fine so
    // long as the loaded stations are each coherent.
    let mut f = valid_factory();
    f.frontmatter.stations = vec!["whatever".into(), "we".into(), "like".into()];
    assert!(darkrun_content::validate(&f).is_ok());
}

#[test]
fn no_stations_error_carries_factory_slug_too() {
    // A genuinely empty factory still errors and carries its slug.
    let mut f = valid_factory();
    f.stations.clear();
    assert_eq!(invalid_factory_slug(&f), "demo");
}

// ===========================================================================
// SECTION 10 — validate(): worker-count rules (Make->Challenge->Resolve)
// ===========================================================================

#[test]
fn rejects_station_with_no_workers() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers.clear();
    f.stations[0].workers.clear();
    assert!(message(&f).contains("no workers"));
}

#[test]
fn no_workers_message_names_the_station() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers.clear();
    f.stations[0].workers.clear();
    assert!(message(&f).contains("station `s1`"));
}

#[test]
fn rejects_one_worker() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers = vec!["w1".into()];
    f.stations[0].workers = vec![role("w1", RoleKind::Worker)];
    assert!(message(&f).contains("at least 3 workers"));
}

#[test]
fn rejects_two_workers() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers = vec!["w1".into(), "w2".into()];
    f.stations[0].workers = vec![role("w1", RoleKind::Worker), role("w2", RoleKind::Worker)];
    let msg = message(&f);
    assert!(msg.contains("at least 3 workers"), "{msg}");
    assert!(msg.contains("has 2"), "{msg}");
}

#[test]
fn empty_workers_checked_before_count_floor() {
    // Zero workers trips the "no workers" rule, not the "< 3" rule.
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers.clear();
    f.stations[0].workers.clear();
    let msg = message(&f);
    assert!(msg.contains("no workers"), "{msg}");
    assert!(!msg.contains("at least 3"), "{msg}");
}

// ===========================================================================
// SECTION 11 — validate(): missing explorers / reviewers / locked artifact
// ===========================================================================

#[test]
fn rejects_station_with_no_explorers() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers.clear();
    f.stations[0].explorers.clear();
    assert!(message(&f).contains("no explorers"));
}

#[test]
fn no_explorers_message_mentions_explore_phase() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers.clear();
    f.stations[0].explorers.clear();
    assert!(message(&f).contains("Explore phase"));
}

#[test]
fn rejects_station_with_no_reviewers() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.reviewers.clear();
    f.stations[0].reviewers.clear();
    assert!(message(&f).contains("no reviewers"));
}

#[test]
fn no_reviewers_message_mentions_review_phase() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.reviewers.clear();
    f.stations[0].reviewers.clear();
    assert!(message(&f).contains("Review phase"));
}

#[test]
fn rejects_blank_locked_artifact() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.locked_artifact = "   ".into();
    assert!(message(&f).contains("no locked_artifact"));
}

#[test]
fn rejects_empty_locked_artifact() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.locked_artifact = String::new();
    assert!(message(&f).contains("no locked_artifact"));
}

#[test]
fn rejects_tab_only_locked_artifact() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.locked_artifact = "\t\n".into();
    assert!(message(&f).contains("no locked_artifact"));
}

#[test]
fn accepts_locked_artifact_with_surrounding_meaningful_text() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.locked_artifact = "code".into();
    darkrun_content::validate(&f).expect("non-blank artifact is fine");
}

// ===========================================================================
// SECTION 12 — validate(): duplicate role references
// ===========================================================================

#[test]
fn rejects_duplicate_worker_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers = vec!["w1".into(), "w1".into(), "w2".into()];
    f.stations[0].workers = vec![
        role("w1", RoleKind::Worker),
        role("w1", RoleKind::Worker),
        role("w2", RoleKind::Worker),
    ];
    assert!(message(&f).contains("worker `w1` more than once"));
}

#[test]
fn rejects_duplicate_explorer_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers = vec!["e1".into(), "e1".into()];
    f.stations[0].explorers = vec![
        role("e1", RoleKind::Explorer),
        role("e1", RoleKind::Explorer),
    ];
    assert!(message(&f).contains("explorer `e1` more than once"));
}

#[test]
fn rejects_duplicate_reviewer_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.reviewers = vec!["r1".into(), "r1".into()];
    f.stations[0].reviewers = vec![
        role("r1", RoleKind::Reviewer),
        role("r1", RoleKind::Reviewer),
    ];
    assert!(message(&f).contains("reviewer `r1` more than once"));
}

#[test]
fn duplicate_at_non_adjacent_positions_is_rejected() {
    let mut f = valid_factory();
    // w1 at index 0 and index 2 — non-adjacent duplicate.
    f.stations[0].frontmatter.workers = vec!["w1".into(), "w2".into(), "w1".into(), "w3".into()];
    f.stations[0].workers = vec![
        role("w1", RoleKind::Worker),
        role("w2", RoleKind::Worker),
        role("w1", RoleKind::Worker),
        role("w3", RoleKind::Worker),
    ];
    assert!(message(&f).contains("worker `w1` more than once"));
}

#[test]
fn distinct_slugs_across_kinds_are_allowed() {
    // The same slug may appear as both an explorer and a reviewer (different
    // kinds) — duplicate detection is per-kind, not global.
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers = vec!["value".into()];
    f.stations[0].explorers = vec![role("value", RoleKind::Explorer)];
    f.stations[0].frontmatter.reviewers = vec!["value".into()];
    f.stations[0].reviewers = vec![role("value", RoleKind::Reviewer)];
    darkrun_content::validate(&f).expect("same slug across distinct kinds is allowed");
}

// ===========================================================================
// SECTION 13 — validate(): dangling references (count mismatch)
// ===========================================================================

#[test]
fn rejects_dangling_reviewer_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.reviewers.push("ghost".into());
    let msg = message(&f);
    assert!(msg.contains("declared 2"), "{msg}");
    assert!(msg.contains("Reviewer"), "{msg}");
}

#[test]
fn rejects_dangling_explorer_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.explorers.push("ghost".into());
    let msg = message(&f);
    assert!(msg.contains("declared 2"), "{msg}");
    assert!(msg.contains("Explorer"), "{msg}");
}

#[test]
fn rejects_dangling_worker_reference() {
    let mut f = valid_factory();
    f.stations[0].frontmatter.workers.push("ghost".into());
    let msg = message(&f);
    assert!(msg.contains("declared 4"), "{msg}");
    assert!(msg.contains("Worker"), "{msg}");
}

#[test]
fn rejects_loaded_role_without_reference() {
    // More loaded roles than declared references.
    let mut f = valid_factory();
    f.stations[0].reviewers.push(role("extra", RoleKind::Reviewer));
    let msg = message(&f);
    assert!(msg.contains("loaded 2"), "{msg}");
}

// ===========================================================================
// SECTION 14 — validate(): role slug / kind mismatch
// ===========================================================================

#[test]
fn rejects_explorer_slug_mismatch() {
    let mut f = valid_factory();
    f.stations[0].explorers[0] = role("wrong", RoleKind::Explorer);
    let msg = message(&f);
    assert!(msg.contains("Explorer `e1` defines name `wrong`"), "{msg}");
}

#[test]
fn rejects_worker_slug_mismatch() {
    let mut f = valid_factory();
    f.stations[0].workers[1] = role("wrong", RoleKind::Worker);
    let msg = message(&f);
    assert!(msg.contains("Worker `w2` defines name `wrong`"), "{msg}");
}

#[test]
fn rejects_reviewer_slug_mismatch() {
    let mut f = valid_factory();
    f.stations[0].reviewers[0] = role("wrong", RoleKind::Reviewer);
    let msg = message(&f);
    assert!(msg.contains("Reviewer `r1` defines name `wrong`"), "{msg}");
}

#[test]
fn rejects_worker_tagged_as_reviewer() {
    let mut f = valid_factory();
    f.stations[0].workers[0] = role("w1", RoleKind::Reviewer);
    let msg = message(&f);
    assert!(msg.contains("Worker `w1` declares agent_type Reviewer"), "{msg}");
}

#[test]
fn rejects_worker_tagged_as_explorer() {
    let mut f = valid_factory();
    f.stations[0].workers[2] = role("w3", RoleKind::Explorer);
    let msg = message(&f);
    assert!(msg.contains("Worker `w3` declares agent_type Explorer"), "{msg}");
}

#[test]
fn rejects_explorer_tagged_as_worker() {
    let mut f = valid_factory();
    f.stations[0].explorers[0] = role("e1", RoleKind::Worker);
    let msg = message(&f);
    assert!(msg.contains("Explorer `e1` declares agent_type Worker"), "{msg}");
}

#[test]
fn rejects_reviewer_tagged_as_explorer() {
    let mut f = valid_factory();
    f.stations[0].reviewers[0] = role("r1", RoleKind::Explorer);
    let msg = message(&f);
    assert!(msg.contains("Reviewer `r1` declares agent_type Explorer"), "{msg}");
}

#[test]
fn slug_mismatch_checked_before_kind_mismatch() {
    // A role with both a wrong name and a wrong kind reports the name first.
    let mut f = valid_factory();
    f.stations[0].workers[0] = role("wrong", RoleKind::Reviewer);
    let msg = message(&f);
    assert!(msg.contains("defines name `wrong`"), "{msg}");
    assert!(!msg.contains("agent_type"), "{msg}");
}

// ===========================================================================
// SECTION 15 — validate(): per-station error attribution in multi-station factories
// ===========================================================================

#[test]
fn validation_error_names_the_offending_station_in_a_chain() {
    let mut f = valid_two_station_factory();
    // Break only the second station.
    f.stations[1].frontmatter.workers.clear();
    f.stations[1].workers.clear();
    let msg = message(&f);
    assert!(msg.contains("station `s2`"), "{msg}");
}

#[test]
fn first_failing_station_reported_first() {
    let mut f = valid_two_station_factory();
    // Break both; the first station's error should surface.
    f.stations[0].frontmatter.reviewers.clear();
    f.stations[0].reviewers.clear();
    f.stations[1].frontmatter.explorers.clear();
    f.stations[1].explorers.clear();
    let msg = message(&f);
    assert!(msg.contains("station `s1`"), "{msg}");
}

#[test]
fn all_good_stations_in_a_chain_validate() {
    let mut f = valid_factory();
    f.frontmatter.stations = vec!["a".into(), "b".into(), "c".into()];
    f.stations = vec![
        valid_station_named("a"),
        valid_station_named("b"),
        valid_station_named("c"),
    ];
    darkrun_content::validate(&f).expect("three good stations validate");
}

#[test]
fn invalid_error_factory_field_is_consistent_across_stations() {
    let mut f = valid_two_station_factory();
    f.frontmatter.name = "myfactory".into();
    f.stations[1].frontmatter.reviewers.clear();
    f.stations[1].reviewers.clear();
    assert_eq!(invalid_factory_slug(&f), "myfactory");
}

// ===========================================================================
// SECTION 16 — load_validated wires loader + validator
// ===========================================================================

#[test]
fn load_validated_returns_same_shape_as_load_factory() {
    let loaded = load_factory("software").unwrap();
    let validated = load_validated("software").unwrap();
    let a: Vec<&str> = loaded.stations.iter().map(Station::name).collect();
    let b: Vec<&str> = validated.stations.iter().map(Station::name).collect();
    assert_eq!(a, b);
    assert_eq!(loaded.name(), validated.name());
}

#[test]
fn load_validated_software_is_idempotent() {
    let a = load_validated("software").unwrap();
    let b = load_validated("software").unwrap();
    assert_eq!(a.frontmatter.default_model, b.frontmatter.default_model);
    assert_eq!(a.stations.len(), b.stations.len());
}

#[test]
fn load_validated_preserves_role_bodies() {
    let f = load_validated("software").unwrap();
    let frame = f.station("frame").unwrap();
    assert!(frame.workers.iter().any(|w| w.body.to_lowercase().contains("framer")));
}

// ===========================================================================
// SECTION 17 — RoleKind serde (the loader's parse path)
// ===========================================================================

fn parse_role_kind(yaml: &str) -> Result<RoleKind, serde_yaml::Error> {
    serde_yaml::from_str(yaml)
}

#[test]
fn role_kind_explorer_deserializes_from_snake_case() {
    assert_eq!(parse_role_kind("explorer").unwrap(), RoleKind::Explorer);
}

#[test]
fn role_kind_worker_deserializes_from_snake_case() {
    assert_eq!(parse_role_kind("worker").unwrap(), RoleKind::Worker);
}

#[test]
fn role_kind_reviewer_deserializes_from_snake_case() {
    assert_eq!(parse_role_kind("reviewer").unwrap(), RoleKind::Reviewer);
}

#[test]
fn role_kind_rejects_titlecase() {
    assert!(parse_role_kind("Explorer").is_err());
}

#[test]
fn role_kind_rejects_uppercase() {
    assert!(parse_role_kind("WORKER").is_err());
}

#[test]
fn role_kind_rejects_unknown_variant() {
    assert!(parse_role_kind("manager").is_err());
}

#[test]
fn role_kind_rejects_empty_string() {
    assert!(parse_role_kind("\"\"").is_err());
}

#[test]
fn role_kind_explorer_roundtrips() {
    let yaml = serde_yaml::to_string(&RoleKind::Explorer).unwrap();
    assert_eq!(parse_role_kind(&yaml).unwrap(), RoleKind::Explorer);
}

#[test]
fn role_kind_worker_roundtrips() {
    let yaml = serde_yaml::to_string(&RoleKind::Worker).unwrap();
    assert_eq!(parse_role_kind(&yaml).unwrap(), RoleKind::Worker);
}

#[test]
fn role_kind_reviewer_roundtrips() {
    let yaml = serde_yaml::to_string(&RoleKind::Reviewer).unwrap();
    assert_eq!(parse_role_kind(&yaml).unwrap(), RoleKind::Reviewer);
}

#[test]
fn role_kind_serializes_as_snake_case() {
    assert_eq!(serde_yaml::to_string(&RoleKind::Explorer).unwrap().trim(), "explorer");
    assert_eq!(serde_yaml::to_string(&RoleKind::Worker).unwrap().trim(), "worker");
    assert_eq!(serde_yaml::to_string(&RoleKind::Reviewer).unwrap().trim(), "reviewer");
}

#[test]
fn role_kind_all_three_are_distinct() {
    assert_ne!(RoleKind::Explorer, RoleKind::Worker);
    assert_ne!(RoleKind::Worker, RoleKind::Reviewer);
    assert_ne!(RoleKind::Explorer, RoleKind::Reviewer);
}

#[test]
fn role_kind_is_copy_and_equatable() {
    let k = RoleKind::Worker;
    let copy = k; // Copy
    assert_eq!(k, copy);
}

// --- RoleFrontmatter serde through the same path the loader uses ---

#[test]
fn role_frontmatter_parses_minimal_yaml() {
    let yaml = "name: framer\nagent_type: worker";
    let fm: RoleFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "framer");
    assert_eq!(fm.agent_type, Some(RoleKind::Worker));
    assert_eq!(fm.model, None, "model defaults to None when omitted");
}

#[test]
fn role_frontmatter_parses_with_model_override() {
    let yaml = "name: framer\nagent_type: worker\nmodel: opus";
    let fm: RoleFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.model.as_deref(), Some("opus"));
}

#[test]
fn role_frontmatter_rejects_missing_name() {
    let yaml = "agent_type: worker";
    assert!(serde_yaml::from_str::<RoleFrontmatter>(yaml).is_err());
}

#[test]
fn role_frontmatter_allows_missing_agent_type() {
    // `agent_type` is deprecated — the kind is inferred from the role's
    // directory — so a role file with only `name` parses fine.
    let yaml = "name: framer";
    let fm = serde_yaml::from_str::<RoleFrontmatter>(yaml).expect("parses without agent_type");
    assert_eq!(fm.agent_type, None);
}

#[test]
fn role_frontmatter_rejects_unknown_agent_type() {
    let yaml = "name: framer\nagent_type: overseer";
    assert!(serde_yaml::from_str::<RoleFrontmatter>(yaml).is_err());
}

// --- StationFrontmatter serde ---

#[test]
fn station_frontmatter_parses_with_all_fields() {
    let yaml = "name: frame\ndescription: d\nexplorers: [a]\nworkers: [b, c, d]\nreviewers: [e]\nlocked_artifact: frame.md\ninputs: [spec.md]";
    let fm: StationFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "frame");
    assert_eq!(fm.workers.len(), 3);
    assert_eq!(fm.inputs, vec!["spec.md".to_string()]);
}

#[test]
fn station_frontmatter_defaults_empty_role_lists() {
    let yaml = "name: frame";
    let fm: StationFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert!(fm.explorers.is_empty());
    assert!(fm.workers.is_empty());
    assert!(fm.reviewers.is_empty());
    assert!(fm.inputs.is_empty());
    assert_eq!(fm.locked_artifact, "");
}

#[test]
fn station_frontmatter_requires_name() {
    let yaml = "description: d";
    assert!(serde_yaml::from_str::<StationFrontmatter>(yaml).is_err());
}

// --- FactoryFrontmatter serde ---

#[test]
fn factory_frontmatter_parses_minimal() {
    let yaml = "name: demo\nstations: [a, b]";
    let fm: FactoryFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.name, "demo");
    assert_eq!(fm.stations, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(fm.default_model, "");
    assert!(fm.fix_workers.is_empty());
}

#[test]
fn factory_frontmatter_stations_is_optional() {
    // `stations` is vestigial — the engine walks the fixed Position::FLOW — so a
    // FACTORY.md without it parses (and defaults to empty).
    let yaml = "name: demo";
    let fm = serde_yaml::from_str::<FactoryFrontmatter>(yaml).expect("parses without stations");
    assert!(fm.stations.is_empty());
    assert_eq!(fm.inherits, None);
}

#[test]
fn factory_frontmatter_requires_name() {
    let yaml = "stations: [a]";
    assert!(serde_yaml::from_str::<FactoryFrontmatter>(yaml).is_err());
}

#[test]
fn factory_frontmatter_roundtrips_through_yaml() {
    let f = valid_factory();
    let yaml = serde_yaml::to_string(&f.frontmatter).unwrap();
    let back: FactoryFrontmatter = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(back.name, f.frontmatter.name);
    assert_eq!(back.stations, f.frontmatter.stations);
    assert_eq!(back.default_model, f.frontmatter.default_model);
}

#[test]
fn station_frontmatter_roundtrips_through_yaml() {
    let s = valid_station();
    let yaml = serde_yaml::to_string(&s.frontmatter).unwrap();
    let back: StationFrontmatter = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(back.name, s.frontmatter.name);
    assert_eq!(back.workers, s.frontmatter.workers);
    assert_eq!(back.locked_artifact, s.frontmatter.locked_artifact);
}

#[test]
fn role_frontmatter_roundtrips_through_yaml() {
    let r = role_with_model("framer", RoleKind::Worker, "opus");
    let yaml = serde_yaml::to_string(&r.frontmatter).unwrap();
    let back: RoleFrontmatter = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(back.name, "framer");
    // `agent_type` is deprecated and not serialized; the kind lives on the Role.
    assert_eq!(back.agent_type, None);
    assert_eq!(r.kind(), RoleKind::Worker);
    assert_eq!(back.model.as_deref(), Some("opus"));
}


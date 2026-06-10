//! Comprehensive coverage for run-level (factory-scope) roles, area `run_level`.
//!
//! These exercise the additive factory-scope lifecycle: whole-Run **reviewers**
//! (cross-station auditors that run AFTER the final station) and **reflections**
//! (backward-looking dimensions evaluated at Run completion). Three surfaces are
//! driven:
//!
//!  - the shipped `software` factory's run reviewers and reflections — their
//!    slugs, kinds, bodies, models, and the FACTORY.md frontmatter that lists
//!    them;
//!  - `FactoryFrontmatter`'s new `reviewers` / `reflections` fields — serde
//!    defaults (additive: omission yields empty), parse, and roundtrip;
//!  - `validate()` over in-memory factories that exercise the run-level rules:
//!    dangling refs (count mismatch), slug typos, mis-kinded files, and
//!    duplicate references, each mutated one at a time off a valid baseline.

use darkrun_content::{
    load_factory, load_validated, validate, ContentError, Factory, FactoryFrontmatter, Role,
    RoleFrontmatter, RoleKind, Station, StationFrontmatter,
};

// ---------------------------------------------------------------------------
// The shipped run-level contract.
// ---------------------------------------------------------------------------

const RUN_REVIEWERS: &[&str] = &[
    "integration-auditor",
    "regression-auditor",
    "security-auditor",
    "accessibility-auditor",
    "runtime-verifier",
];
const REFLECTIONS: &[&str] = &["architecture", "process", "quality", "velocity"];

fn factory() -> Factory {
    load_validated("software").expect("software factory must load and validate")
}

fn slugs(roles: &[Role]) -> Vec<&str> {
    roles.iter().map(Role::name).collect()
}

// ---------------------------------------------------------------------------
// In-memory builders for validation tests (one-field-at-a-time mutation).
// ---------------------------------------------------------------------------

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
        body: format!("# {name}\n\nEnough prose to instruct an agent verbatim end to end."),
        kind,
    }
}

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

/// A baseline factory that declares one run reviewer and one reflection, both
/// resolved. Failure tests break exactly one run-level field of a fresh copy.
fn valid_run_level_factory() -> Factory {
    Factory {
        frontmatter: FactoryFrontmatter {
            name: "demo".into(),
            description: "demo factory".into(),
            category: "engineering".into(),
            default_model: "sonnet".into(),
            inherits: None,
            stations: vec!["s1".into()],
            fix_workers: vec![],
            reviewers: vec!["audit".into()],
            reflections: vec!["learn".into()],
            surfaces: vec![],
        },
        body: "# demo".into(),
        stations: vec![valid_station()],
        run_reviewers: vec![role("audit", RoleKind::Reviewer)],
        reflections: vec![role("learn", RoleKind::Reflection)],
    }
}

fn message(factory: &Factory) -> String {
    match validate(factory) {
        Err(ContentError::Invalid { message, .. }) => message,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// ===========================================================================
// SECTION 1 — the shipped software factory's run reviewers
// ===========================================================================

#[test]
fn software_declares_its_run_reviewers() {
    assert_eq!(factory().frontmatter.reviewers, RUN_REVIEWERS);
}

#[test]
fn software_loads_all_run_reviewers() {
    assert_eq!(slugs(&factory().run_reviewers), RUN_REVIEWERS);
}

#[test]
fn accessibility_auditor_is_surface_scoped() {
    // E6: a run reviewer can carry an applies_to surface scope.
    let f = factory();
    let a11y = f.run_reviewer("accessibility-auditor").expect("loaded");
    assert_eq!(a11y.frontmatter.applies_to, vec!["web_ui", "desktop", "mobile"]);
    // An unscoped reviewer has an empty applies_to.
    let integ = f.run_reviewer("integration-auditor").expect("loaded");
    assert!(integ.frontmatter.applies_to.is_empty());
}

#[test]
fn run_reviewer_frontmatter_and_loaded_count_agree() {
    let f = factory();
    assert_eq!(f.frontmatter.reviewers.len(), f.run_reviewers.len());
}

#[test]
fn every_run_reviewer_is_kinded_reviewer() {
    for r in &factory().run_reviewers {
        assert_eq!(r.kind(), RoleKind::Reviewer, "{}", r.name());
    }
}

#[test]
fn run_reviewer_lookup_finds_each() {
    let f = factory();
    for name in RUN_REVIEWERS {
        assert!(f.run_reviewer(name).is_some(), "run reviewer `{name}` must resolve");
    }
}

#[test]
fn run_reviewer_lookup_returns_none_for_unknown() {
    assert!(factory().run_reviewer("does-not-exist").is_none());
}

#[test]
fn run_reviewer_lookup_is_not_confused_with_reflection() {
    // A reflection slug must not resolve through the run-reviewer accessor.
    let f = factory();
    assert!(f.run_reviewer("architecture").is_none());
}

#[test]
fn run_reviewer_name_matches_frontmatter_name() {
    for r in &factory().run_reviewers {
        assert_eq!(r.name(), r.frontmatter.name);
    }
}

#[test]
fn run_reviewer_bodies_have_a_heading() {
    for r in &factory().run_reviewers {
        assert!(r.body.contains('#'), "{} body has no heading", r.name());
    }
}

#[test]
fn run_reviewer_bodies_are_substantial() {
    for r in &factory().run_reviewers {
        assert!(
            r.body.trim().len() > 120,
            "{} body too thin ({} bytes)",
            r.name(),
            r.body.trim().len()
        );
    }
}

#[test]
fn run_reviewer_bodies_describe_a_whole_run_scope() {
    // The defining trait of a run reviewer is that it judges the whole Run, not
    // one station — the body should say so.
    for r in &factory().run_reviewers {
        let body = r.body.to_lowercase();
        assert!(
            body.contains("whole") || body.contains("entire") || body.contains("end-to-end")
                || body.contains("cross-station") || body.contains("across"),
            "{} body should frame a whole-Run scope",
            r.name()
        );
    }
}

#[test]
fn run_reviewer_bodies_mention_running_after_the_final_station() {
    for r in &factory().run_reviewers {
        let body = r.body.to_lowercase();
        assert!(
            body.contains("after") || body.contains("final") || body.contains("last")
                || body.contains("complete") || body.contains("integrated"),
            "{} body should place itself after the final station",
            r.name()
        );
    }
}

#[test]
fn integration_auditor_talks_about_seams_or_consistency() {
    let f = factory();
    let r = f.run_reviewer("integration-auditor").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("seam") || body.contains("consisten") || body.contains("drift"));
}

#[test]
fn regression_auditor_talks_about_regressions() {
    let f = factory();
    let r = f.run_reviewer("regression-auditor").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("regress") || body.contains("broke") || body.contains("collateral"));
}

#[test]
fn security_auditor_talks_about_threats_or_attackers() {
    let f = factory();
    let r = f.run_reviewer("security-auditor").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("threat") || body.contains("attacker") || body.contains("vulnerab") || body.contains("security"));
}

#[test]
fn run_reviewer_slugs_are_unique() {
    let f = factory();
    let mut names = slugs(&f.run_reviewers);
    let len = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), len, "run reviewer slugs must be unique");
}

#[test]
fn run_reviewer_slugs_are_lowercase_with_hyphens() {
    for r in &factory().run_reviewers {
        let name = r.name();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '_'),
            "run reviewer slug `{name}` should be a lowercase slug"
        );
    }
}

#[test]
fn run_reviewers_carry_a_model() {
    // Each authored run reviewer pins a model in frontmatter (per the content spec).
    for r in &factory().run_reviewers {
        assert!(
            r.frontmatter.model.is_some(),
            "{} should declare a model",
            r.name()
        );
    }
}

// ===========================================================================
// SECTION 2 — the shipped software factory's reflections
// ===========================================================================

#[test]
fn software_declares_four_reflections() {
    assert_eq!(factory().frontmatter.reflections, REFLECTIONS);
}

#[test]
fn software_loads_all_reflections() {
    assert_eq!(slugs(&factory().reflections), REFLECTIONS);
}

#[test]
fn reflection_frontmatter_and_loaded_count_agree() {
    let f = factory();
    assert_eq!(f.frontmatter.reflections.len(), f.reflections.len());
}

#[test]
fn every_reflection_is_kinded_reflection() {
    for r in &factory().reflections {
        assert_eq!(r.kind(), RoleKind::Reflection, "{}", r.name());
    }
}

#[test]
fn reflection_lookup_finds_each() {
    let f = factory();
    for name in REFLECTIONS {
        assert!(f.reflection(name).is_some(), "reflection `{name}` must resolve");
    }
}

#[test]
fn reflection_lookup_returns_none_for_unknown() {
    assert!(factory().reflection("does-not-exist").is_none());
}

#[test]
fn reflection_lookup_is_not_confused_with_run_reviewer() {
    let f = factory();
    assert!(f.reflection("security-auditor").is_none());
}

#[test]
fn reflection_name_matches_frontmatter_name() {
    for r in &factory().reflections {
        assert_eq!(r.name(), r.frontmatter.name);
    }
}

#[test]
fn reflection_bodies_have_a_heading() {
    for r in &factory().reflections {
        assert!(r.body.contains('#'), "{} body has no heading", r.name());
    }
}

#[test]
fn reflection_bodies_are_substantial() {
    for r in &factory().reflections {
        assert!(
            r.body.trim().len() > 120,
            "{} body too thin ({} bytes)",
            r.name(),
            r.body.trim().len()
        );
    }
}

#[test]
fn reflection_bodies_frame_a_backward_look() {
    // A reflection looks back over the finished Run — the body should say so.
    for r in &factory().reflections {
        let body = r.body.to_lowercase();
        assert!(
            body.contains("look back") || body.contains("backward") || body.contains("learning")
                || body.contains("reflect") || body.contains("next run"),
            "{} body should frame a backward-looking reflection",
            r.name()
        );
    }
}

#[test]
fn reflection_bodies_say_they_do_not_gate() {
    // Reflections produce learnings, not verdicts — at least name that they
    // don't block.
    for r in &factory().reflections {
        let body = r.body.to_lowercase();
        assert!(
            body.contains("not a gate") || body.contains("never block") || body.contains("not a verdict")
                || body.contains("learning") || body.contains("not to grade"),
            "{} body should distinguish itself from a gate",
            r.name()
        );
    }
}

#[test]
fn architecture_reflection_talks_about_debt_or_structure() {
    let f = factory();
    let r = f.reflection("architecture").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("debt") || body.contains("abstraction") || body.contains("dependency") || body.contains("structur"));
}

#[test]
fn process_reflection_talks_about_friction_or_checkpoints() {
    let f = factory();
    let r = f.reflection("process").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("friction") || body.contains("checkpoint") || body.contains("hand-off") || body.contains("transition"));
}

#[test]
fn quality_reflection_talks_about_reviewers_or_gates() {
    let f = factory();
    let r = f.reflection("quality").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("reviewer") || body.contains("gate") || body.contains("coverage") || body.contains("finding"));
}

#[test]
fn velocity_reflection_talks_about_effort_or_throughput() {
    let f = factory();
    let r = f.reflection("velocity").unwrap();
    let body = r.body.to_lowercase();
    assert!(body.contains("effort") || body.contains("throughput") || body.contains("pass") || body.contains("blocker"));
}

#[test]
fn reflection_slugs_are_unique() {
    let f = factory();
    let mut names = slugs(&f.reflections);
    let len = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), len, "reflection slugs must be unique");
}

#[test]
fn reflection_slugs_are_lowercase() {
    for r in &factory().reflections {
        let name = r.name();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '_'),
            "reflection slug `{name}` should be a lowercase slug"
        );
    }
}

#[test]
fn reflections_carry_a_model() {
    for r in &factory().reflections {
        assert!(r.frontmatter.model.is_some(), "{} should declare a model", r.name());
    }
}

#[test]
fn no_reflection_slug_collides_with_a_run_reviewer_slug() {
    let f = factory();
    for refl in slugs(&f.reflections) {
        assert!(
            !slugs(&f.run_reviewers).contains(&refl),
            "reflection `{refl}` collides with a run reviewer slug"
        );
    }
}

// ===========================================================================
// SECTION 3 — RoleKind::Reflection serde
// ===========================================================================

#[test]
fn reflection_kind_serializes_snake_case() {
    assert_eq!(serde_yaml::to_string(&RoleKind::Reflection).unwrap().trim(), "reflection");
}

#[test]
fn reflection_kind_deserializes_snake_case() {
    let k: RoleKind = serde_yaml::from_str("reflection").unwrap();
    assert_eq!(k, RoleKind::Reflection);
}

#[test]
fn reflection_kind_roundtrips() {
    let y = serde_yaml::to_string(&RoleKind::Reflection).unwrap();
    let back: RoleKind = serde_yaml::from_str(&y).unwrap();
    assert_eq!(back, RoleKind::Reflection);
}

#[test]
fn reflection_kind_is_distinct_from_the_other_kinds() {
    assert_ne!(RoleKind::Reflection, RoleKind::Reviewer);
    assert_ne!(RoleKind::Reflection, RoleKind::Worker);
    assert_ne!(RoleKind::Reflection, RoleKind::Explorer);
}

#[test]
fn reflection_kind_rejects_titlecase() {
    assert!(serde_yaml::from_str::<RoleKind>("Reflection").is_err());
}

#[test]
fn role_frontmatter_parses_reflection_agent_type() {
    let yaml = "name: architecture\nagent_type: reflection";
    let fm: RoleFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.agent_type, Some(RoleKind::Reflection));
}

// ===========================================================================
// SECTION 4 — FactoryFrontmatter: additive defaults + roundtrip
// ===========================================================================

#[test]
fn frontmatter_reviewers_default_empty_when_omitted() {
    // Additive: a factory that predates these fields still parses.
    let yaml = "name: demo\nstations: [a, b]";
    let fm: FactoryFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert!(fm.reviewers.is_empty(), "reviewers must default empty");
}

#[test]
fn frontmatter_reflections_default_empty_when_omitted() {
    let yaml = "name: demo\nstations: [a, b]";
    let fm: FactoryFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert!(fm.reflections.is_empty(), "reflections must default empty");
}

#[test]
fn frontmatter_parses_run_reviewers_and_reflections() {
    let yaml = "name: demo\nstations: [a]\nreviewers: [x, y]\nreflections: [p]";
    let fm: FactoryFrontmatter = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(fm.reviewers, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(fm.reflections, vec!["p".to_string()]);
}

#[test]
fn frontmatter_run_level_fields_roundtrip() {
    let fm = factory().frontmatter;
    let yaml = serde_yaml::to_string(&fm).unwrap();
    let back: FactoryFrontmatter = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(fm.reviewers, back.reviewers);
    assert_eq!(fm.reflections, back.reflections);
}

#[test]
fn serialized_frontmatter_contains_run_reviewer_slugs() {
    let yaml = serde_yaml::to_string(&factory().frontmatter).unwrap();
    for name in RUN_REVIEWERS {
        assert!(yaml.contains(name), "serialized frontmatter missing run reviewer `{name}`");
    }
}

#[test]
fn serialized_frontmatter_contains_reflection_slugs() {
    let yaml = serde_yaml::to_string(&factory().frontmatter).unwrap();
    for name in REFLECTIONS {
        assert!(yaml.contains(name), "serialized frontmatter missing reflection `{name}`");
    }
}

#[test]
fn existing_frontmatter_fields_survive_alongside_run_level() {
    // Additive guarantee: the new fields did not displace the old ones.
    let fm = factory().frontmatter;
    assert_eq!(fm.name, "software");
    assert_eq!(fm.category, "engineering");
    assert!(!fm.stations.is_empty());
    assert!(!fm.fix_workers.is_empty());
}

// ===========================================================================
// SECTION 5 — validate(): run-level baseline passes
// ===========================================================================

#[test]
fn run_level_baseline_validates() {
    validate(&valid_run_level_factory()).expect("baseline with run-level roles must validate");
}

#[test]
fn empty_run_level_lists_validate() {
    // A factory may legitimately declare no run reviewers and no reflections.
    let mut f = valid_run_level_factory();
    f.frontmatter.reviewers.clear();
    f.frontmatter.reflections.clear();
    f.run_reviewers.clear();
    f.reflections.clear();
    validate(&f).expect("a factory with no run-level roles is still valid");
}

#[test]
fn run_level_validate_is_idempotent() {
    let f = valid_run_level_factory();
    validate(&f).expect("first");
    validate(&f).expect("second");
    validate(&f).expect("third");
}

#[test]
fn multiple_run_reviewers_and_reflections_validate() {
    let mut f = valid_run_level_factory();
    f.frontmatter.reviewers = vec!["a".into(), "b".into(), "c".into()];
    f.run_reviewers = vec![
        role("a", RoleKind::Reviewer),
        role("b", RoleKind::Reviewer),
        role("c", RoleKind::Reviewer),
    ];
    f.frontmatter.reflections = vec!["p".into(), "q".into()];
    f.reflections = vec![
        role("p", RoleKind::Reflection),
        role("q", RoleKind::Reflection),
    ];
    validate(&f).expect("several run-level roles validate");
}

#[test]
fn shipped_software_factory_validates_with_run_level_roles() {
    validate(&load_factory("software").unwrap()).expect("shipped corpus must validate");
}

// ===========================================================================
// SECTION 6 — validate(): dangling run-level references
// ===========================================================================

#[test]
fn rejects_dangling_run_reviewer_reference() {
    let mut f = valid_run_level_factory();
    f.frontmatter.reviewers.push("ghost".into());
    let msg = message(&f);
    assert!(msg.contains("declared 2"), "{msg}");
    assert!(msg.contains("Reviewer"), "{msg}");
}

#[test]
fn rejects_dangling_reflection_reference() {
    let mut f = valid_run_level_factory();
    f.frontmatter.reflections.push("ghost".into());
    let msg = message(&f);
    assert!(msg.contains("declared 2"), "{msg}");
    assert!(msg.contains("Reflection"), "{msg}");
}

#[test]
fn rejects_loaded_run_reviewer_without_reference() {
    let mut f = valid_run_level_factory();
    f.run_reviewers.push(role("extra", RoleKind::Reviewer));
    let msg = message(&f);
    assert!(msg.contains("loaded 2"), "{msg}");
}

#[test]
fn rejects_loaded_reflection_without_reference() {
    let mut f = valid_run_level_factory();
    f.reflections.push(role("extra", RoleKind::Reflection));
    let msg = message(&f);
    assert!(msg.contains("loaded 2"), "{msg}");
}

// ===========================================================================
// SECTION 7 — validate(): slug / kind mismatch in run-level roles
// ===========================================================================

#[test]
fn rejects_run_reviewer_slug_mismatch() {
    let mut f = valid_run_level_factory();
    f.run_reviewers[0] = role("wrong", RoleKind::Reviewer);
    let msg = message(&f);
    assert!(msg.contains("Reviewer `audit` defines name `wrong`"), "{msg}");
}

#[test]
fn rejects_reflection_slug_mismatch() {
    let mut f = valid_run_level_factory();
    f.reflections[0] = role("wrong", RoleKind::Reflection);
    let msg = message(&f);
    assert!(msg.contains("Reflection `learn` defines name `wrong`"), "{msg}");
}

#[test]
fn rejects_run_reviewer_tagged_as_reflection() {
    let mut f = valid_run_level_factory();
    f.run_reviewers[0] = role("audit", RoleKind::Reflection);
    let msg = message(&f);
    assert!(msg.contains("Reviewer `audit` declares agent_type Reflection"), "{msg}");
}

#[test]
fn rejects_reflection_tagged_as_reviewer() {
    let mut f = valid_run_level_factory();
    f.reflections[0] = role("learn", RoleKind::Reviewer);
    let msg = message(&f);
    assert!(msg.contains("Reflection `learn` declares agent_type Reviewer"), "{msg}");
}

#[test]
fn rejects_reflection_tagged_as_worker() {
    let mut f = valid_run_level_factory();
    f.reflections[0] = role("learn", RoleKind::Worker);
    let msg = message(&f);
    assert!(msg.contains("Reflection `learn` declares agent_type Worker"), "{msg}");
}

// ===========================================================================
// SECTION 8 — validate(): duplicate run-level references
// ===========================================================================

#[test]
fn rejects_duplicate_run_reviewer_reference() {
    let mut f = valid_run_level_factory();
    f.frontmatter.reviewers = vec!["audit".into(), "audit".into()];
    f.run_reviewers = vec![
        role("audit", RoleKind::Reviewer),
        role("audit", RoleKind::Reviewer),
    ];
    assert!(message(&f).contains("run reviewer `audit` more than once"));
}

#[test]
fn rejects_duplicate_reflection_reference() {
    let mut f = valid_run_level_factory();
    f.frontmatter.reflections = vec!["learn".into(), "learn".into()];
    f.reflections = vec![
        role("learn", RoleKind::Reflection),
        role("learn", RoleKind::Reflection),
    ];
    assert!(message(&f).contains("reflection `learn` more than once"));
}

#[test]
fn run_reviewer_and_reflection_may_share_a_slug() {
    // Duplicate detection is per-list: the same slug can name both a run reviewer
    // and a reflection (different kinds, different files).
    let mut f = valid_run_level_factory();
    f.frontmatter.reviewers = vec!["shared".into()];
    f.run_reviewers = vec![role("shared", RoleKind::Reviewer)];
    f.frontmatter.reflections = vec!["shared".into()];
    f.reflections = vec![role("shared", RoleKind::Reflection)];
    validate(&f).expect("same slug across the two run-level lists is allowed");
}

// ===========================================================================
// SECTION 9 — error attribution names the factory
// ===========================================================================

#[test]
fn run_level_error_names_the_factory() {
    let mut f = valid_run_level_factory();
    f.frontmatter.name = "myfactory".into();
    f.frontmatter.reflections.push("ghost".into());
    let err = validate(&f).unwrap_err();
    match err {
        ContentError::Invalid { factory, .. } => assert_eq!(factory, "myfactory"),
        other => panic!("expected Invalid, got {other:?}"),
    }
}

// ===========================================================================
// SECTION 10 — determinism / idempotency of the shipped run-level corpus
// ===========================================================================

#[test]
fn loading_twice_yields_identical_run_reviewers() {
    let a: Vec<String> = load_factory("software")
        .unwrap()
        .run_reviewers
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    let b: Vec<String> = load_factory("software")
        .unwrap()
        .run_reviewers
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    assert_eq!(a, b);
}

#[test]
fn loading_twice_yields_identical_reflections() {
    let a: Vec<String> = load_factory("software")
        .unwrap()
        .reflections
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    let b: Vec<String> = load_factory("software")
        .unwrap()
        .reflections
        .iter()
        .map(|r| r.name().to_string())
        .collect();
    assert_eq!(a, b);
}

#[test]
fn cloning_preserves_run_level_roles() {
    let f = factory();
    let cloned = f.clone();
    assert_eq!(slugs(&f.run_reviewers), slugs(&cloned.run_reviewers));
    assert_eq!(slugs(&f.reflections), slugs(&cloned.reflections));
    validate(&cloned).expect("a clone must still validate");
}

// ===========================================================================
// SECTION 11 — FACTORY.md prose documents the run-level lifecycle
// ===========================================================================

#[test]
fn factory_body_documents_run_reviewers() {
    let body = factory().body.to_lowercase();
    assert!(body.contains("run reviewer") || body.contains("whole-run"));
}

#[test]
fn factory_body_documents_reflections() {
    let body = factory().body.to_lowercase();
    assert!(body.contains("reflection"));
}

#[test]
fn factory_body_says_reflections_do_not_gate() {
    let body = factory().body.to_lowercase();
    assert!(
        body.contains("never block") || body.contains("not a verdict") || body.contains("learning"),
        "factory body should distinguish reflections from gates"
    );
}

#[test]
fn run_reviewer_role_frontmatter_yaml_roundtrip() {
    for r in &factory().run_reviewers {
        let yaml = serde_yaml::to_string(&r.frontmatter).unwrap();
        let back: RoleFrontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(r.frontmatter.name, back.name);
        assert_eq!(r.frontmatter.agent_type, back.agent_type);
        assert_eq!(r.frontmatter.model, back.model);
    }
}

#[test]
fn reflection_role_frontmatter_yaml_roundtrip() {
    for r in &factory().reflections {
        let yaml = serde_yaml::to_string(&r.frontmatter).unwrap();
        let back: RoleFrontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(r.frontmatter.name, back.name);
        assert_eq!(r.frontmatter.agent_type, back.agent_type);
        assert_eq!(r.frontmatter.model, back.model);
    }
}

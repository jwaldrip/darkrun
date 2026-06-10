//! Structural validation of a loaded [`Factory`].
//!
//! Loading proves the files *parse*; validation proves they are *coherent*:
//! every station references roles that exist, each role's declared `name` and
//! `agent_type` match where it is referenced, the Make/Challenge/Resolve worker
//! sequence is present, and each station declares a checkpoint and a locked
//! artifact.

use crate::error::{ContentError, Result};
use crate::model::{Factory, Role, RoleKind, Station};

/// Validate a loaded factory, returning the first structural error found.
pub fn validate(factory: &Factory) -> Result<()> {
    let slug = factory.name().to_string();
    let invalid = |message: String| ContentError::Invalid {
        factory: slug.clone(),
        message,
    };

    // The station spine is the fixed `Position::FLOW`, resolved by the loader —
    // not the factory's frontmatter (`stations` is vestigial). Validate each
    // loaded station's internal coherence; their identity/order is the engine's
    // invariant, not a per-factory declaration.
    if factory.stations.is_empty() {
        return Err(invalid("factory loaded no stations".into()));
    }

    for station in &factory.stations {
        validate_station(&slug, station)?;
    }

    validate_run_level(&invalid, factory)?;
    validate_surfaces(&invalid, factory)?;
    validate_input_coverage(&invalid, factory)?;

    Ok(())
}

/// Cross-station input coverage — the run's distillation must not be *silently*
/// lost. Walking the fixed flow, every upstream station's `locked_artifact` must
/// appear in a later station's `inputs` (carried forward) **or** its
/// `inputs_waived` (consciously dropped). An upstream artifact that is neither is
/// a silent gap: the station would work from scratch, re-inventing what the run
/// already distilled. (A station may also `inputs_waive` an artifact it never
/// needs — that's the explicit opt-out.)
fn validate_input_coverage(
    invalid: &impl Fn(String) -> ContentError,
    factory: &Factory,
) -> Result<()> {
    let mut upstream: Vec<String> = Vec::new();
    for station in &factory.stations {
        let fm = &station.frontmatter;
        for art in &upstream {
            let carried = fm.inputs.iter().any(|i| i == art);
            let waived = fm.inputs_waived.iter().any(|w| w == art);
            if !carried && !waived {
                return Err(invalid(format!(
                    "station `{}` silently drops upstream artifact `{art}` — \
                     declare it in `inputs` (carry the distillation forward) or \
                     `inputs_waived` (consciously not needed)",
                    station.name()
                )));
            }
        }
        let art = fm.locked_artifact.trim();
        if !art.is_empty() && !upstream.iter().any(|a| a == art) {
            upstream.push(art.to_string());
        }
    }
    Ok(())
}

/// Validate the factory's declared delivery surfaces. Surfaces are per-factory
/// data, but each one must name a delivery surface the engine knows how to route
/// for verification — an unknown token (a typo, an unsupported surface) is a hard
/// error so a run can never classify into a surface Prove/Audit cannot measure.
/// No surface may be declared twice.
fn validate_surfaces(
    invalid: &impl Fn(String) -> ContentError,
    factory: &Factory,
) -> Result<()> {
    reject_duplicate_slugs(invalid, "surface", &factory.frontmatter.surfaces)?;
    for token in &factory.frontmatter.surfaces {
        if darkrun_core::domain::Surface::parse(token).is_none() {
            return Err(invalid(format!("declares unknown surface `{token}`")));
        }
    }
    Ok(())
}

/// Validate the factory-scope (run-level) roles: whole-Run reviewers that run
/// after the final station, and reflection dimensions evaluated at completion.
///
/// Like a station's roles, each declared slug must resolve to a loaded role
/// whose `name` matches and whose `agent_type` is the kind its list expects —
/// a dangling reference (length mismatch), a slug typo, or a mis-kinded file is
/// rejected. No two entries within a list may share a slug.
fn validate_run_level(
    invalid: &impl Fn(String) -> ContentError,
    factory: &Factory,
) -> Result<()> {
    reject_duplicate_slugs(invalid, "run reviewer", &factory.frontmatter.reviewers)?;
    reject_duplicate_slugs(invalid, "reflection", &factory.frontmatter.reflections)?;

    check_roles(
        invalid,
        &factory.frontmatter.reviewers,
        &factory.run_reviewers,
        RoleKind::Reviewer,
    )?;
    check_roles(
        invalid,
        &factory.frontmatter.reflections,
        &factory.reflections,
        RoleKind::Reflection,
    )?;

    Ok(())
}

fn validate_station(factory: &str, station: &Station) -> Result<()> {
    let name = station.name().to_string();
    let invalid = |message: String| ContentError::Invalid {
        factory: factory.to_string(),
        message: format!("station `{name}`: {message}"),
    };

    // A station must do work: at least one worker beat.
    if station.frontmatter.workers.is_empty() {
        return Err(invalid("declares no workers".into()));
    }

    // The pass-loop needs at least Make -> Challenge -> Resolve (3 beats), and
    // exactly one terminal Resolve worker.
    if station.workers.len() < 3 {
        return Err(invalid(format!(
            "needs at least 3 workers for Make->Challenge->Resolve, has {}",
            station.workers.len()
        )));
    }

    // The Explore phase needs Explorers; without one the station gathers no
    // context before decomposing.
    if station.frontmatter.explorers.is_empty() {
        return Err(invalid("declares no explorers for the Explore phase".into()));
    }

    // The Review phase needs Reviewers; without one no independent party verifies
    // the workers' output before the checkpoint.
    if station.frontmatter.reviewers.is_empty() {
        return Err(invalid("declares no reviewers for the Review phase".into()));
    }

    // A station must lock a durable artifact.
    if station.frontmatter.locked_artifact.trim().is_empty() {
        return Err(invalid("declares no locked_artifact".into()));
    }

    // No two declared roles within a kind may share a slug — a duplicate
    // reference is almost always a copy-paste mistake and makes the run loop
    // ambiguous about which definition to apply.
    reject_duplicate_slugs(&invalid, "explorer", &station.frontmatter.explorers)?;
    reject_duplicate_slugs(&invalid, "worker", &station.frontmatter.workers)?;
    reject_duplicate_slugs(&invalid, "reviewer", &station.frontmatter.reviewers)?;

    // Each referenced role must match its declared slug and kind.
    check_roles(&invalid, &station.frontmatter.explorers, &station.explorers, RoleKind::Explorer)?;
    check_roles(&invalid, &station.frontmatter.workers, &station.workers, RoleKind::Worker)?;
    check_roles(&invalid, &station.frontmatter.reviewers, &station.reviewers, RoleKind::Reviewer)?;

    Ok(())
}

/// Reject a kind's reference list when it names the same slug twice.
fn reject_duplicate_slugs(
    invalid: &impl Fn(String) -> ContentError,
    kind: &str,
    declared: &[String],
) -> Result<()> {
    for (i, slug) in declared.iter().enumerate() {
        if declared[..i].iter().any(|earlier| earlier == slug) {
            return Err(invalid(format!("declares {kind} `{slug}` more than once")));
        }
    }
    Ok(())
}

/// Verify that each loaded role matches its declared slug and the expected kind.
fn check_roles(
    invalid: &impl Fn(String) -> ContentError,
    declared: &[String],
    loaded: &[Role],
    expected: RoleKind,
) -> Result<()> {
    if declared.len() != loaded.len() {
        return Err(invalid(format!(
            "declared {} {:?} roles but loaded {}",
            declared.len(),
            expected,
            loaded.len()
        )));
    }
    for (slug, role) in declared.iter().zip(loaded.iter()) {
        if role.name() != slug {
            return Err(invalid(format!(
                "{expected:?} `{slug}` defines name `{}`",
                role.name()
            )));
        }
        if role.kind() != expected {
            return Err(invalid(format!(
                "{expected:?} `{slug}` declares agent_type {:?}",
                role.kind()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Factory, FactoryFrontmatter, RoleFrontmatter, StationFrontmatter,
    };

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
            body: format!("# {name}\n\ninstructions"),
            kind,
        }
    }

    /// A minimal, structurally valid single-station factory used as the baseline
    /// each failure test mutates one field of.
    fn valid_factory() -> Factory {
        let station = Station {
            frontmatter: StationFrontmatter {
                name: "s1".into(),
                description: String::new(),
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
            body: "# s1".into(),
            explorers: vec![role("e1", RoleKind::Explorer)],
            workers: vec![
                role("w1", RoleKind::Worker),
                role("w2", RoleKind::Worker),
                role("w3", RoleKind::Worker),
            ],
            reviewers: vec![role("r1", RoleKind::Reviewer)],
        };
        Factory {
            frontmatter: FactoryFrontmatter {
                name: "demo".into(),
                description: String::new(),
                category: String::new(),
                default_model: "sonnet".into(),
                inherits: None,
                stations: vec!["s1".into()],
                fix_workers: vec![],
                reviewers: vec![],
                reflections: vec![],
                surfaces: vec![],
            },
            body: "# demo".into(),
            stations: vec![station],
            run_reviewers: vec![],
            reflections: vec![],
        }
    }

    fn message(factory: &Factory) -> String {
        match validate(factory) {
            Err(ContentError::Invalid { message, .. }) => message,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn baseline_is_valid() {
        validate(&valid_factory()).expect("the baseline factory must validate");
    }

    #[test]
    fn rejects_factory_with_no_stations() {
        let mut f = valid_factory();
        f.frontmatter.stations.clear();
        f.stations.clear();
        assert!(message(&f).contains("no stations"));
    }

    #[test]
    fn frontmatter_stations_is_not_validated_against_loaded() {
        // `frontmatter.stations` is vestigial — the spine is Position::FLOW.
        // A mismatched declaration validates fine as long as the loaded stations
        // are each coherent.
        let mut f = valid_factory();
        f.frontmatter.stations = vec!["anything".into(), "goes".into()];
        assert!(validate(&f).is_ok());
    }

    #[test]
    fn rejects_station_with_no_workers() {
        let mut f = valid_factory();
        f.stations[0].frontmatter.workers.clear();
        f.stations[0].workers.clear();
        assert!(message(&f).contains("no workers"));
    }

    #[test]
    fn rejects_too_few_workers_for_pass_loop() {
        let mut f = valid_factory();
        f.stations[0].frontmatter.workers.pop();
        f.stations[0].workers.pop();
        assert!(message(&f).contains("at least 3 workers"));
    }

    #[test]
    fn rejects_station_with_no_explorers() {
        let mut f = valid_factory();
        f.stations[0].frontmatter.explorers.clear();
        f.stations[0].explorers.clear();
        assert!(message(&f).contains("no explorers"));
    }

    #[test]
    fn rejects_station_with_no_reviewers() {
        let mut f = valid_factory();
        f.stations[0].frontmatter.reviewers.clear();
        f.stations[0].reviewers.clear();
        assert!(message(&f).contains("no reviewers"));
    }

    #[test]
    fn rejects_missing_locked_artifact() {
        let mut f = valid_factory();
        f.stations[0].frontmatter.locked_artifact = "   ".into();
        assert!(message(&f).contains("no locked_artifact"));
    }

    #[test]
    fn rejects_duplicate_role_reference() {
        let mut f = valid_factory();
        // Reference the same worker slug twice.
        f.stations[0].frontmatter.workers = vec!["w1".into(), "w1".into(), "w2".into()];
        f.stations[0].workers = vec![
            role("w1", RoleKind::Worker),
            role("w1", RoleKind::Worker),
            role("w2", RoleKind::Worker),
        ];
        assert!(message(&f).contains("worker `w1` more than once"));
    }

    #[test]
    fn rejects_dangling_role_reference_count_mismatch() {
        let mut f = valid_factory();
        // Declared a reviewer the loader never resolved (length mismatch stands
        // in for a dangling reference, since the loader would otherwise fail).
        f.stations[0]
            .frontmatter
            .reviewers
            .push("ghost".into());
        let msg = message(&f);
        assert!(msg.contains("declared 2"), "{msg}");
        assert!(msg.contains("Reviewer"), "{msg}");
    }

    #[test]
    fn rejects_role_slug_mismatch() {
        let mut f = valid_factory();
        // The loaded explorer defines a different name than the reference.
        f.stations[0].explorers[0] = role("wrong", RoleKind::Explorer);
        let msg = message(&f);
        assert!(msg.contains("Explorer `e1` defines name `wrong`"), "{msg}");
    }

    #[test]
    fn rejects_role_kind_mismatch() {
        let mut f = valid_factory();
        // A file referenced as a worker is actually tagged as a reviewer.
        f.stations[0].workers[0] = role("w1", RoleKind::Reviewer);
        let msg = message(&f);
        assert!(msg.contains("Worker `w1` declares agent_type Reviewer"), "{msg}");
    }

    // --- run-level (factory-scope) reviewers + reflections ---

    /// Attach one resolved run reviewer and one resolved reflection to the
    /// baseline so the run-level branch has something to validate.
    fn factory_with_run_level() -> Factory {
        let mut f = valid_factory();
        f.frontmatter.reviewers = vec!["audit".into()];
        f.run_reviewers = vec![role("audit", RoleKind::Reviewer)];
        f.frontmatter.reflections = vec!["learn".into()];
        f.reflections = vec![role("learn", RoleKind::Reflection)];
        f
    }

    #[test]
    fn run_level_baseline_validates() {
        validate(&factory_with_run_level()).expect("run-level baseline must validate");
    }

    #[test]
    fn empty_run_level_lists_are_valid() {
        // The default (no run reviewers, no reflections) must still validate.
        validate(&valid_factory()).expect("a factory with no run-level roles is valid");
    }

    #[test]
    fn rejects_dangling_run_reviewer() {
        let mut f = factory_with_run_level();
        f.frontmatter.reviewers.push("ghost".into());
        let msg = message(&f);
        assert!(msg.contains("declared 2"), "{msg}");
        assert!(msg.contains("Reviewer"), "{msg}");
    }

    #[test]
    fn rejects_dangling_reflection() {
        let mut f = factory_with_run_level();
        f.frontmatter.reflections.push("ghost".into());
        let msg = message(&f);
        assert!(msg.contains("declared 2"), "{msg}");
        assert!(msg.contains("Reflection"), "{msg}");
    }

    #[test]
    fn rejects_run_reviewer_kind_mismatch() {
        let mut f = factory_with_run_level();
        f.run_reviewers[0] = role("audit", RoleKind::Reflection);
        let msg = message(&f);
        assert!(msg.contains("Reviewer `audit` declares agent_type Reflection"), "{msg}");
    }

    #[test]
    fn rejects_reflection_kind_mismatch() {
        let mut f = factory_with_run_level();
        f.reflections[0] = role("learn", RoleKind::Reviewer);
        let msg = message(&f);
        assert!(msg.contains("Reflection `learn` declares agent_type Reviewer"), "{msg}");
    }

    #[test]
    fn rejects_run_reviewer_slug_mismatch() {
        let mut f = factory_with_run_level();
        f.run_reviewers[0] = role("wrong", RoleKind::Reviewer);
        let msg = message(&f);
        assert!(msg.contains("Reviewer `audit` defines name `wrong`"), "{msg}");
    }

    #[test]
    fn rejects_duplicate_run_reviewer() {
        let mut f = factory_with_run_level();
        f.frontmatter.reviewers = vec!["audit".into(), "audit".into()];
        f.run_reviewers = vec![
            role("audit", RoleKind::Reviewer),
            role("audit", RoleKind::Reviewer),
        ];
        assert!(message(&f).contains("run reviewer `audit` more than once"));
    }

    #[test]
    fn rejects_unknown_surface() {
        let mut f = valid_factory();
        f.frontmatter.surfaces = vec!["library".into(), "hologram".into()];
        assert!(message(&f).contains("unknown surface `hologram`"));
    }

    #[test]
    fn rejects_duplicate_surface() {
        let mut f = valid_factory();
        f.frontmatter.surfaces = vec!["library".into(), "library".into()];
        assert!(message(&f).contains("surface `library` more than once"));
    }

    #[test]
    fn accepts_known_surfaces() {
        let mut f = valid_factory();
        f.frontmatter.surfaces = vec!["web_ui".into(), "cli".into(), "data".into()];
        validate(&f).expect("known surfaces validate");
    }

    #[test]
    fn rejects_duplicate_reflection() {
        let mut f = factory_with_run_level();
        f.frontmatter.reflections = vec!["learn".into(), "learn".into()];
        f.reflections = vec![
            role("learn", RoleKind::Reflection),
            role("learn", RoleKind::Reflection),
        ];
        assert!(message(&f).contains("reflection `learn` more than once"));
    }
}

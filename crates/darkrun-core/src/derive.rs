//! The **shared, pure phase derivation** — one source of truth consumed by every
//! surface (the engine cursor, the HTTP browse endpoints, the desktop app, and the
//! website). Mirrors the predecessor's `packages/shared/derived-stage-state.ts`.
//!
//! A station's phase/status is a **pure function of its units' on-disk
//! frontmatter** — never a stored snapshot. Because the engine, HTTP, and desktop
//! all call these same functions over the same `Unit` documents, they can never
//! disagree about where a run is. There is no `state.json` to drift.
//!
//! The role lists (`review_roles`, `approval_roles`) are mode-shaped by the caller
//! (autopilot trims the `user` role) so this module stays a pure derivation.

use crate::domain::{IterationResult, StationPhase, Status, Unit};

/// Whether a unit has every required review role stamped (the PRE-execute gate).
fn reviews_signed(unit: &Unit, review_roles: &[String]) -> bool {
    review_roles
        .iter()
        .all(|role| matches!(unit.frontmatter.reviews.get(role), Some(Some(_))))
}

/// Whether a unit has every required approval role stamped (the POST-execute gate).
fn approvals_signed(unit: &Unit, approval_roles: &[String]) -> bool {
    approval_roles
        .iter()
        .all(|role| matches!(unit.frontmatter.approvals.get(role), Some(Some(_))))
}

/// Whether a unit's Pass loop is complete: its LAST iteration `advance`d on the
/// station's last worker. With no declared workers, any `advance` qualifies
/// (research-style stations that only produce artifacts).
fn pass_loop_done(unit: &Unit, workers: &[String]) -> bool {
    let Some(last) = unit.frontmatter.iterations.last() else {
        return false;
    };
    if last.result != Some(IterationResult::Advance) {
        return false;
    }
    match workers.last() {
        Some(terminal) => &last.worker == terminal,
        None => true,
    }
}

/// Derive a station's [`StationPhase`] from its units — the pure cursor-walk
/// signal, shared by every surface.
///
/// Order is load-bearing (review BEFORE execute): a not-yet-spec-signed unit has
/// empty iterations and would otherwise mislabel as `Manufacture`.
///
/// - `elaboration_verified`: `Some(true)` verified, `Some(false)` present-unverified,
///   `None` artifact missing. Skipped entirely under `autopilot`.
pub fn derive_station_phase(
    units: &[Unit],
    workers: &[String],
    review_roles: &[String],
    approval_roles: &[String],
    elaboration_verified: Option<bool>,
    autopilot: bool,
) -> StationPhase {
    // 1. Elaborate gate (Spec phase). Skipped under autopilot.
    if !autopilot {
        if elaboration_verified == Some(false) {
            return StationPhase::Spec;
        }
        if elaboration_verified.is_none() && units.is_empty() {
            return StationPhase::Spec;
        }
    }
    // 2. Decompose pending → still Spec.
    if units.is_empty() {
        return StationPhase::Spec;
    }
    // 3. Review: any unit missing a required review role.
    if units.iter().any(|u| !reviews_signed(u, review_roles)) {
        return StationPhase::Review;
    }
    // 4. Manufacture: any unit whose Pass loop isn't done.
    if !workers.is_empty() && units.iter().any(|u| !pass_loop_done(u, workers)) {
        return StationPhase::Manufacture;
    }
    // 5. Audit/gate: any unit missing a required approval role (post-execute
    //    reviewers + quality gates sign here; Reflect/observations is a sub-step
    //    the cursor handles after the gate is signed).
    if units.iter().any(|u| !approvals_signed(u, approval_roles)) {
        return StationPhase::Audit;
    }
    // 6. All signed — the Checkpoint gate fires (awaiting the station→run-main merge).
    StationPhase::Checkpoint
}

/// Whether every unit in a station is fully signed (all reviews + approvals +
/// Pass loops done) — the predecessor's `isStageComplete`. A station with no units
/// is NOT complete (it still owes decomposition).
pub fn station_units_complete(
    units: &[Unit],
    workers: &[String],
    review_roles: &[String],
    approval_roles: &[String],
) -> bool {
    !units.is_empty()
        && units.iter().all(|u| {
            reviews_signed(u, review_roles)
                && pass_loop_done(u, workers)
                && approvals_signed(u, approval_roles)
        })
}

/// The lifecycle [`Status`] of a station relative to the active one: `Completed`
/// (before the active), `Active` (the active station), `Pending` (after).
pub fn station_status(index: usize, active_index: Option<usize>) -> Status {
    match active_index {
        Some(active) if index < active => Status::Completed,
        Some(active) if index == active => Status::Active,
        _ => Status::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Stamp, UnitFrontmatter, UnitIteration};

    fn unit(slug: &str) -> Unit {
        Unit {
            slug: slug.into(),
            frontmatter: UnitFrontmatter::default(),
            title: slug.into(),
            body: String::new(),
        }
    }
    fn signed() -> Option<Stamp> {
        Some(Stamp { at: "2026-06-02T00:00:00Z".into() })
    }
    fn roles(rs: &[&str]) -> Vec<String> {
        rs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_units_is_spec() {
        assert_eq!(
            derive_station_phase(&[], &[], &[], &[], Some(true), false),
            StationPhase::Spec
        );
    }

    #[test]
    fn unverified_elaboration_is_spec_unless_autopilot() {
        let us = [unit("a")];
        assert_eq!(
            derive_station_phase(&us, &roles(&["w"]), &[], &[], Some(false), false),
            StationPhase::Spec
        );
        // autopilot skips the elaborate gate → falls to review (no review roles → execute…)
        assert_ne!(
            derive_station_phase(&us, &roles(&["w"]), &[], &[], Some(false), true),
            StationPhase::Spec
        );
    }

    #[test]
    fn missing_review_is_review_then_manufacture_then_audit_then_checkpoint() {
        let review_roles = roles(&["spec"]);
        let approval_roles = roles(&["user"]);
        let workers = roles(&["make", "resolve"]);

        // 3. No review stamp → Review.
        let mut a = unit("a");
        assert_eq!(
            derive_station_phase(
                std::slice::from_ref(&a), &workers, &review_roles, &approval_roles, Some(true), false
            ),
            StationPhase::Review
        );

        // 4. Review signed, Pass loop not done → Manufacture.
        a.frontmatter.reviews.insert("spec".into(), signed());
        assert_eq!(
            derive_station_phase(
                std::slice::from_ref(&a), &workers, &review_roles, &approval_roles, Some(true), false
            ),
            StationPhase::Manufacture
        );

        // 4b. Last iteration advanced but NOT on the terminal worker → still Manufacture.
        a.frontmatter.iterations.push(UnitIteration {
            worker: "make".into(), result: Some(IterationResult::Advance), ..Default::default()
        });
        assert_eq!(
            derive_station_phase(
                std::slice::from_ref(&a), &workers, &review_roles, &approval_roles, Some(true), false
            ),
            StationPhase::Manufacture
        );

        // 5. Terminal worker advanced, approval missing → Audit.
        a.frontmatter.iterations.push(UnitIteration {
            worker: "resolve".into(), result: Some(IterationResult::Advance), ..Default::default()
        });
        assert_eq!(
            derive_station_phase(
                std::slice::from_ref(&a), &workers, &review_roles, &approval_roles, Some(true), false
            ),
            StationPhase::Audit
        );

        // 6. Approval signed → Checkpoint, and the station is complete.
        a.frontmatter.approvals.insert("user".into(), signed());
        assert_eq!(
            derive_station_phase(
                std::slice::from_ref(&a), &workers, &review_roles, &approval_roles, Some(true), false
            ),
            StationPhase::Checkpoint
        );
        assert!(station_units_complete(
            std::slice::from_ref(&a), &workers, &review_roles, &approval_roles
        ));
    }

    #[test]
    fn station_status_orders_relative_to_active() {
        assert_eq!(station_status(0, Some(2)), Status::Completed);
        assert_eq!(station_status(2, Some(2)), Status::Active);
        assert_eq!(station_status(3, Some(2)), Status::Pending);
        assert_eq!(station_status(0, None), Status::Pending);
    }

    #[test]
    fn pass_loop_done_edge_arms_and_missing_elaboration_is_spec() {
        use crate::domain::IterationResult;
        // A rejected last iteration is not "done".
        let mut a = unit("a");
        a.frontmatter.iterations.push(UnitIteration {
            worker: "make".into(), result: Some(IterationResult::Reject), ..Default::default()
        });
        assert!(!pass_loop_done(&a, &roles(&["make"])));
        // An advance with NO declared workers qualifies (artifact-only stations).
        let mut b = unit("b");
        b.frontmatter.iterations.push(UnitIteration {
            worker: "make".into(), result: Some(IterationResult::Advance), ..Default::default()
        });
        assert!(pass_loop_done(&b, &[]));
        // Elaboration unknown (None) + no units yet → the Spec decompose gate.
        assert_eq!(
            derive_station_phase(&[], &roles(&["make"]), &[], &[], None, false),
            StationPhase::Spec
        );
    }
}

//! Drift — the input-premise immune system.
//!
//! Drift is **about inputs, not outputs**. When a station locks,
//! [`record_station_witnesses`] hashes every completed unit's declared **input**
//! files — the *premises* the unit was built and signed against — into that
//! unit's `input_witnesses`. Outputs are deliberately NOT witnessed: an output
//! is the mutable product, downstream of the signature and allowed to evolve;
//! witnessing it would make a later unit's edit register as drift on the earlier
//! producer and spin a fix loop that never converges.
//!
//! Each tick [`sweep`] re-hashes every input premise. A premise that moved (or
//! vanished) means the work resting on it may need to **re-orient** — so the
//! sweep restamps the witness to the current content (it fires once), re-anchors
//! any annotations on the artifact, and files an `origin = drift` **feedback**
//! item against the affected unit's station. Drift is feedback: there is no
//! separate drift hold. The agent then classifies the change as cosmetic (close,
//! no-op) or material (invalidate the unit's signed slots → re-sign against the
//! new premise). This is how an out-of-band change — a human editing a design
//! file by hand, another agent moving an upstream artifact — gets taken into
//! regard as the run iterates.
//!
//! **Baton exemption.** A premise that is also produced by the *same station*
//! (a file a unit both consumes and produces — an in-place edit, or a shared
//! baton several units append to) is exempt: its own in-loop writes must not
//! re-fire drift. Only a premise produced *upstream* (a different station, or an
//! external hand) is a real re-orientation signal.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use darkrun_core::domain::{FeedbackOrigin, FeedbackSeverity, Unit};
use darkrun_core::{hash_file, StateStore};

use crate::error::Result;

/// The repo root the run's artifact paths are relative to — the parent of the
/// `.darkrun/` state root.
fn repo_root(store: &StateStore) -> PathBuf {
    store
        .root()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| store.root().to_path_buf())
}

/// The artifact basename — the last path segment — so `specify/spec.md` and
/// `spec.md` compare equal (premises and outputs are written in either form).
fn artifact_basename(s: &str) -> String {
    s.trim().rsplit('/').next().unwrap_or(s.trim()).to_string()
}

/// How a witnessed premise drifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PremiseDrift {
    /// The premise file's content changed.
    Mutated,
    /// The premise file was deleted.
    Deleted,
}

impl PremiseDrift {
    fn as_str(self) -> &'static str {
        match self {
            PremiseDrift::Mutated => "mutated",
            PremiseDrift::Deleted => "deleted",
        }
    }
}

/// The cap on open drift feedback before the sweep stops filing new ones — a
/// circuit breaker so a widely-consumed premise moving (or a directory rename)
/// can't bury the run under a flood the operator can't act on. The witness is
/// still restamped past the cap; only the feedback is suppressed. Overridable
/// via `DARKRUN_DRIFT_CASCADE_CAP`.
fn cascade_cap() -> usize {
    std::env::var("DARKRUN_DRIFT_CASCADE_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}

/// Snapshot the premises of a just-completed station: hash every declared
/// **input** of its completed units into that unit's `input_witnesses` (the
/// premise the unit was built on). Called when a station locks. A later
/// [`sweep`] flags a premise that moved upstream.
///
/// Outputs are intentionally not witnessed (see the module docs).
pub fn record_station_witnesses(store: &StateStore, run: &str, station: &str) -> Result<()> {
    let root = repo_root(store);
    let units = store.read_units(run)?;
    for u in units.iter().filter(|u| {
        u.station() == station && matches!(u.status(), darkrun_core::domain::Status::Completed)
    }) {
        let mut unit = u.clone();
        let mut changed = false;
        for input in &u.frontmatter.inputs {
            if let Some(hash) = hash_file(&root.join(input)) {
                unit.frontmatter.input_witnesses.insert(input.clone(), hash);
                changed = true;
            }
        }
        if changed {
            store.write_unit(run, &unit)?;
        }
    }
    Ok(())
}

/// The set of output basenames produced by each station's units — the
/// baton-exempt set. A premise whose basename is produced by its *own* station
/// is the station's own output evolving in-loop, not an upstream change.
fn produced_basenames_by_station(units: &[Unit]) -> HashMap<String, HashSet<String>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    for u in units {
        let set = out.entry(u.station().to_string()).or_default();
        for o in &u.frontmatter.outputs {
            set.insert(artifact_basename(o));
        }
    }
    out
}

/// Re-hash every input premise; for any that moved or vanished, restamp the
/// witness to current, re-anchor the artifact's annotations, and file one
/// `origin = drift` feedback against the affected unit's station. Pure over disk
/// — same files, same result. Run at the top of each tick.
pub fn sweep(store: &StateStore, run: &str) -> Result<()> {
    let root = repo_root(store);
    let cap = cascade_cap();
    let units = store.read_units(run)?;
    let produced = produced_basenames_by_station(&units);
    let mut open_drift = open_drift_feedback_count(store, run)?;

    for unit in &units {
        if unit.frontmatter.input_witnesses.is_empty() {
            continue;
        }
        let station = unit.station().to_string();
        let exempt = produced.get(&station);
        let mut updated = unit.clone();
        let mut witnesses_changed = false;
        let mut drifts: Vec<(String, PremiseDrift)> = Vec::new();

        for (path, witnessed) in unit.frontmatter.input_witnesses.clone() {
            // Baton exemption: a premise the same station also produces is its
            // own in-loop write, never a re-orientation signal.
            if exempt
                .map(|set| set.contains(&artifact_basename(&path)))
                .unwrap_or(false)
            {
                continue;
            }
            match hash_file(&root.join(&path)) {
                Some(current) if current == witnessed => { /* steady — no drift */ }
                Some(current) => {
                    // Restamp to current so the same change fires exactly once;
                    // the feedback now owns the unresolved re-orientation.
                    updated.frontmatter.input_witnesses.insert(path.clone(), current);
                    witnesses_changed = true;
                    drifts.push((path, PremiseDrift::Mutated));
                }
                None => {
                    // A vanished premise: drop the witness so it can't re-fire.
                    updated.frontmatter.input_witnesses.remove(&path);
                    witnesses_changed = true;
                    drifts.push((path, PremiseDrift::Deleted));
                }
            }
        }

        if witnesses_changed {
            store.write_unit(run, &updated)?;
        }

        for (path, kind) in drifts {
            // Keep annotations valid across the change (text re-anchors;
            // image/pdf regions re-crop from the new bytes).
            if matches!(kind, PremiseDrift::Mutated) {
                if let Ok(bytes) = fs::read(root.join(&path)) {
                    let _ = crate::annotation::reanchor_artifact_version(store, &root, run, &path, &bytes);
                }
            }
            // Dedup against an already-open drift feedback for this premise, then
            // cascade-cap. Restamp has already run either way.
            let marker = drift_marker(kind, &path);
            if drift_feedback_is_open(store, run, &marker)? {
                continue;
            }
            if open_drift >= cap {
                continue;
            }
            let body = render_drift_body(kind, &path, &updated, &marker);
            crate::feedback::create_with_origin(
                store,
                run,
                &station,
                &body,
                Some(FeedbackSeverity::High),
                FeedbackOrigin::Drift,
                vec![],
            )?;
            open_drift += 1;
        }
    }
    Ok(())
}

/// Accept an out-of-band premise change as the new truth, everywhere it is
/// witnessed: re-stamp the premise on every unit that consumes it, re-anchor its
/// annotations, and close any open drift feedback for it (a *cosmetic*
/// resolution — the new premise is absorbed without re-orienting downstream
/// work; for a *material* change the agent invalidates the unit's slots
/// instead). Returns `false` if `path` isn't witnessed or is unreadable.
pub fn accept(store: &StateStore, run: &str, path: &str) -> Result<bool> {
    let root = repo_root(store);
    let current = hash_file(&root.join(path));
    let mut found = false;
    for u in store.read_units(run)? {
        if !u.frontmatter.input_witnesses.contains_key(path) {
            continue;
        }
        let mut unit = u.clone();
        match &current {
            Some(hash) => {
                unit.frontmatter.input_witnesses.insert(path.to_string(), hash.clone());
            }
            None => {
                unit.frontmatter.input_witnesses.remove(path);
            }
        }
        store.write_unit(run, &unit)?;
        found = true;
    }
    if found {
        if let Ok(bytes) = fs::read(root.join(path)) {
            crate::annotation::reanchor_artifact_version(store, &root, run, path, &bytes)?;
        }
        close_open_drift_feedback_for(store, run, path, "premise change accepted")?;
        let _ = crate::commit::commit_state(store, &format!("darkrun: drift accept {path}"));
    }
    Ok(found)
}

/// Re-witness every unit's input premises to their current content and close all
/// open drift feedback — the operator's "rebaseline everything" reset, for a
/// run whose witnesses are stale but already reconciled. Returns the number of
/// premises re-witnessed.
pub fn rebaseline_all(store: &StateStore, run: &str) -> Result<usize> {
    let root = repo_root(store);
    let mut count = 0;
    for u in store.read_units(run)? {
        if u.frontmatter.input_witnesses.is_empty() {
            continue;
        }
        let mut unit = u.clone();
        let paths: Vec<String> = u.frontmatter.input_witnesses.keys().cloned().collect();
        for path in paths {
            match hash_file(&root.join(&path)) {
                Some(hash) => {
                    unit.frontmatter.input_witnesses.insert(path.clone(), hash);
                }
                None => {
                    unit.frontmatter.input_witnesses.remove(&path);
                }
            }
            count += 1;
        }
        store.write_unit(run, &unit)?;
    }
    // Close every open drift feedback — the reset reconciles them all.
    for fb in crate::feedback::list(store, run)? {
        if matches!(fb.origin, FeedbackOrigin::Drift) && !crate::feedback::is_terminal(fb.status) {
            let _ = crate::feedback::close_with_reply(store, run, &fb.id, "rebaselined");
        }
    }
    Ok(count)
}

/// The hidden marker embedded at the top of a drift feedback body so re-sweeps
/// dedup against an already-open item for the same premise + kind. An HTML
/// comment, so it doesn't render in the surfaced markdown.
fn drift_marker(kind: PremiseDrift, path: &str) -> String {
    format!("<!--drift:{}:{}-->", kind.as_str(), path)
}

/// Whether an open `origin = drift` feedback already carries `marker`.
fn drift_feedback_is_open(store: &StateStore, run: &str, marker: &str) -> Result<bool> {
    Ok(crate::feedback::list(store, run)?.into_iter().any(|fb| {
        matches!(fb.origin, FeedbackOrigin::Drift)
            && !crate::feedback::is_terminal(fb.status)
            && fb.body.contains(marker)
    }))
}

/// The number of open `origin = drift` feedback items — the cascade-cap counter.
fn open_drift_feedback_count(store: &StateStore, run: &str) -> Result<usize> {
    Ok(crate::feedback::list(store, run)?
        .into_iter()
        .filter(|fb| {
            matches!(fb.origin, FeedbackOrigin::Drift) && !crate::feedback::is_terminal(fb.status)
        })
        .count())
}

/// The number of open drift feedback items (cascade counter), exposed for the
/// deadlock guard's progress signature.
pub fn open_drift_count(store: &StateStore, run: &str) -> usize {
    open_drift_feedback_count(store, run).unwrap_or(0)
}

/// Close every open drift feedback that references `path`, with a reply.
fn close_open_drift_feedback_for(
    store: &StateStore,
    run: &str,
    path: &str,
    reply: &str,
) -> Result<()> {
    let needle = format!(":{path}-->");
    for fb in crate::feedback::list(store, run)? {
        if matches!(fb.origin, FeedbackOrigin::Drift)
            && !crate::feedback::is_terminal(fb.status)
            && fb.body.contains(&needle)
        {
            let _ = crate::feedback::close_with_reply(store, run, &fb.id, reply);
        }
    }
    Ok(())
}

/// The surfaced body of a drift feedback: the dedup marker, what moved, the
/// signed slots it may have undercut, and how to classify it.
fn render_drift_body(kind: PremiseDrift, path: &str, unit: &Unit, marker: &str) -> String {
    let signed: Vec<&str> = unit
        .frontmatter
        .reviews
        .iter()
        .chain(unit.frontmatter.approvals.iter())
        .filter(|(_, stamp)| stamp.is_some())
        .map(|(role, _)| role.as_str())
        .collect();
    let slots = if signed.is_empty() {
        "none signed yet".to_string()
    } else {
        signed.join(", ")
    };
    let what = match kind {
        PremiseDrift::Mutated => format!("changed on disk: `{path}`"),
        PremiseDrift::Deleted => format!("been deleted: `{path}`"),
    };
    format!(
        "{marker}\n# Premise drift — `{path}`\n\n\
         An input premise unit `{unit_slug}` was built and signed against has {what}. \
         The change came from outside this unit's own work (an upstream station, or an \
         out-of-band edit — a human or another agent). The work resting on this premise \
         may need to re-orient.\n\n\
         **Signed slots that rested on it:** {slots}.\n\n\
         **Classify and act:**\n\
         - **Cosmetic** — the change doesn't move this unit's result or how it's judged. \
         Close this feedback; nothing re-opens.\n\
         - **Material** — it changes what this unit must build, or the acceptance criteria \
         it's judged against. Set the undercut roles with `darkrun_feedback_set_targets` \
         (the signed slots above), then close: the gate re-fires and the work re-signs \
         against the new premise — re-orienting around the change.\n",
        unit_slug = unit.slug,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_core::domain::{Status, Unit, UnitFrontmatter};

    fn store() -> (tempfile::TempDir, StateStore, PathBuf) {
        let dir = tempfile::TempDir::new().expect("tmp");
        let root = dir.path().to_path_buf();
        let store = StateStore::new(&root);
        (dir, store, root)
    }

    fn unit(slug: &str, station: &str, inputs: &[&str], outputs: &[&str]) -> Unit {
        Unit {
            slug: slug.into(),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some(station.into()),
                inputs: inputs.iter().map(|s| s.to_string()).collect(),
                outputs: outputs.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        }
    }

    fn open_drift_bodies(store: &StateStore, run: &str) -> Vec<String> {
        crate::feedback::list(store, run)
            .unwrap()
            .into_iter()
            .filter(|f| matches!(f.origin, FeedbackOrigin::Drift) && !crate::feedback::is_terminal(f.status))
            .map(|f| f.body)
            .collect()
    }

    #[test]
    fn upstream_premise_change_files_one_drift_feedback_restamped_once() {
        let (_d, store, root) = store();
        // shape produces design.md; build consumes it as a premise.
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u-build", "build", &["design.md"], &["code"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();

        // No change yet → no drift.
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 0, "steady state files nothing");

        // A human edits design.md by hand → one drift feedback, restamped once.
        std::fs::write(root.join("design.md"), b"v2-buttons-moved").unwrap();
        sweep(&store, "r").unwrap();
        let bodies = open_drift_bodies(&store, "r");
        assert_eq!(bodies.len(), 1, "exactly one drift feedback");
        assert!(bodies[0].contains("design.md"));
        assert!(matches!(
            crate::feedback::list(&store, "r").unwrap()[0].origin,
            FeedbackOrigin::Drift
        ));

        // Re-sweeping does NOT pile on more (restamp fired once; dedup holds).
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1, "fires once, not every tick");
    }

    #[test]
    fn two_units_on_the_same_premise_file_one_deduped_drift() {
        let (_d, store, root) = store();
        // Two build units BOTH consume design.md as a premise.
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u-a", "build", &["design.md"], &["a"])).unwrap();
        store.write_unit("r", &unit("u-b", "build", &["design.md"], &["b"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();

        // One hand edit to the shared premise → both units detect drift, but they
        // share a marker so only ONE feedback is filed (the second dedups).
        std::fs::write(root.join("design.md"), b"v2-changed").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(
            open_drift_bodies(&store, "r").len(),
            1,
            "a shared premise files exactly one drift, not one per consumer"
        );
    }

    #[test]
    fn in_place_edit_is_baton_exempt_and_never_self_drifts() {
        let (_d, store, root) = store();
        // A unit that BOTH consumes and produces the same file (in-place refactor).
        std::fs::write(root.join("code.rs"), b"v1").unwrap();
        store.write_unit("r", &unit("u", "build", &["code.rs"], &["code.rs"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();

        // The unit's own work changes the file. It must NOT drift itself.
        std::fs::write(root.join("code.rs"), b"v2-refactored").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 0, "input==output is baton-exempt");
    }

    #[test]
    fn downstream_edit_to_a_produced_file_does_not_drift_the_producer() {
        let (_d, store, root) = store();
        // shape produces design.md (output only — no input witness on it).
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u-shape", "shape", &[], &["design.md"])).unwrap();
        record_station_witnesses(&store, "r", "shape").unwrap();

        // Something downstream rewrites design.md. The producer must not drift —
        // outputs are not witnessed.
        std::fs::write(root.join("design.md"), b"v2").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 0, "outputs are not drift");
    }

    #[test]
    fn deleted_premise_files_drift_then_accept_rebaselines() {
        let (_d, store, root) = store();
        std::fs::write(root.join("spec.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u", "build", &["spec.md"], &["code"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();

        std::fs::write(root.join("spec.md"), b"v2").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1);

        // Accept the change → the drift feedback closes and the witness rebaselines.
        assert!(accept(&store, "r", "spec.md").unwrap());
        assert_eq!(open_drift_bodies(&store, "r").len(), 0, "accept closes the drift feedback");
        // A further sweep stays quiet (witness == current).
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 0);
    }

    #[test]
    fn material_classification_reopens_the_units_signed_stamp() {
        use darkrun_core::domain::Stamp;
        let (_d, store, root) = store();
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        // A build unit signed off (review `correctness`) resting on design.md.
        let mut u = unit("u", "build", &["design.md"], &["code"]);
        u.frontmatter
            .reviews
            .insert("correctness".into(), Some(Stamp { at: "t0".into() }));
        store.write_unit("r", &u).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();

        // The design moves out-of-band → one drift feedback.
        std::fs::write(root.join("design.md"), b"v2-buttons-moved").unwrap();
        sweep(&store, "r").unwrap();
        let fb = crate::feedback::list(&store, "r")
            .unwrap()
            .into_iter()
            .find(|f| matches!(f.origin, FeedbackOrigin::Drift))
            .expect("a drift feedback");
        // Its body names the signed slot to consider.
        assert!(fb.body.contains("correctness"));

        // The agent classifies it MATERIAL: the moved design changes what this
        // unit must build, so it invalidates the correctness review.
        crate::feedback::set_targets(&store, "r", &fb.id, vec!["correctness".into()]).unwrap();
        crate::feedback::close_with_reply(&store, "r", &fb.id, "re-orient to new layout").unwrap();

        // The stamp is re-opened → the gate re-fires → the work re-signs against
        // the new premise. That is the re-orientation.
        let back = store.read_unit("r", "u").unwrap();
        assert!(
            !back.frontmatter.reviews.contains_key("correctness"),
            "material drift re-opened the signed slot"
        );
    }

    /// Predecessor drift INFINITE LOOP regression (its 5.0.2/5.0.3): a
    /// witnessed artifact changed by a sanctioned fix, re-baselined and the drift
    /// FB closed, yet `run_next` re-fired the same `drift_detected` forever —
    /// because (A) it diffed via `git log` on a worktree-prefixed path that found
    /// no commits, and (B) closing with `target_invalidates` never cleared the
    /// `approvals.<role>` witness. darkrun must be structurally immune: drift is
    /// CONTENT-HASH based (no git, no commits field) and restamps on detect, so
    /// closing the feedback breaks the loop. This proves it doesn't re-fire.
    #[test]
    fn a_resolved_premise_drift_does_not_re_fire_the_predecessor_loop() {
        let (_d, store, root) = store();
        std::fs::write(root.join("tokens.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u", "design", &["tokens.md"], &["design.md"])).unwrap();
        record_station_witnesses(&store, "r", "design").unwrap();

        // A sanctioned fix extends the premise (the predecessor's FB-033).
        std::fs::write(root.join("tokens.md"), b"v2-expanded-token-catalog").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1, "drift fires once");

        // Re-sweeping does NOT pile on — restamp-on-detect already captured the
        // new content, so the same change can't re-fire (the predecessor's loop).
        sweep(&store, "r").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1, "no re-fire across ticks");

        // The agent closes the drift feedback (cosmetic or after re-orienting).
        let fb = crate::feedback::list(&store, "r")
            .unwrap()
            .into_iter()
            .find(|f| matches!(f.origin, FeedbackOrigin::Drift))
            .unwrap();
        crate::feedback::close_with_reply(&store, "r", &fb.id, "token expansion accepted").unwrap();

        // After closure, the loop is broken: no further sweep re-opens it.
        sweep(&store, "r").unwrap();
        assert!(
            open_drift_bodies(&store, "r").is_empty(),
            "a closed premise drift never re-fires — the predecessor's infinite loop cannot occur"
        );
    }

    #[test]
    fn cascade_cap_bounds_open_drift_feedback() {
        let (_d, store, root) = store();
        std::env::set_var("DARKRUN_DRIFT_CASCADE_CAP", "2");
        // Five distinct premises across five units, all drift at once.
        for i in 0..5 {
            let p = format!("in{i}.md");
            std::fs::write(root.join(&p), b"v1").unwrap();
            store
                .write_unit("r", &unit(&format!("u{i}"), "build", &[&p], &["code"]))
                .unwrap();
        }
        record_station_witnesses(&store, "r", "build").unwrap();
        for i in 0..5 {
            std::fs::write(root.join(format!("in{i}.md")), b"v2").unwrap();
        }
        sweep(&store, "r").unwrap();
        assert!(
            open_drift_bodies(&store, "r").len() <= 2,
            "cascade cap suppresses the flood"
        );
        std::env::remove_var("DARKRUN_DRIFT_CASCADE_CAP");
    }

    #[test]
    fn rebaseline_all_rewitnesses_and_closes_open_drift() {
        let (_d, store, root) = store();
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u-build", "build", &["design.md"], &["code"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();
        // Drift it.
        std::fs::write(root.join("design.md"), b"v2").unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1);

        // rebaseline_all re-witnesses every premise and closes the drift.
        let n = rebaseline_all(&store, "r").unwrap();
        assert!(n >= 1, "re-witnessed at least one premise");
        assert_eq!(open_drift_bodies(&store, "r").len(), 0, "drift closed by rebaseline");
        // Steady again: the witness now matches the current content.
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 0);
    }

    #[test]
    fn a_deleted_premise_files_a_deleted_drift_and_drops_the_witness() {
        let (_d, store, root) = store();
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u-build", "build", &["design.md"], &["code"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();
        // Delete the premise → one drift, witness dropped so it can't re-fire.
        std::fs::remove_file(root.join("design.md")).unwrap();
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1);
        assert!(open_drift_bodies(&store, "r")[0].contains("design.md"));
        sweep(&store, "r").unwrap();
        assert_eq!(open_drift_bodies(&store, "r").len(), 1, "deleted-premise drift fires once");
    }

    #[test]
    fn premise_drift_labels() {
        assert_eq!(PremiseDrift::Mutated.as_str(), "mutated");
        assert_eq!(PremiseDrift::Deleted.as_str(), "deleted");
    }

    #[test]
    fn accept_on_a_deleted_premise_drops_the_witness() {
        let (_d, store, root) = store();
        std::fs::write(root.join("spec.md"), b"v1").unwrap();
        store.write_unit("r", &unit("u", "build", &["spec.md"], &["code"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();
        std::fs::remove_file(root.join("spec.md")).unwrap();
        // Accept directly (no sweep — a sweep would itself drop the deleted-premise
        // witness): the witness is still present, so accept hits the None arm and
        // removes it.
        assert!(accept(&store, "r", "spec.md").unwrap());
        let units = store.read_units("r").unwrap();
        assert!(
            !units[0].frontmatter.input_witnesses.contains_key("spec.md"),
            "the deleted premise's witness is dropped, not re-stamped"
        );
        // An unwitnessed path returns false (nothing to accept).
        assert!(!accept(&store, "r", "never-witnessed.md").unwrap());
    }

    #[test]
    fn rebaseline_all_skips_unwitnessed_units_and_drops_deleted_premises() {
        let (_d, store, root) = store();
        std::fs::write(root.join("design.md"), b"v1").unwrap();
        // One unit consumes design.md (witnessed); a second consumes nothing.
        store.write_unit("r", &unit("u-build", "build", &["design.md"], &["code"])).unwrap();
        store.write_unit("r", &unit("u-free", "frame", &[], &["frame.md"])).unwrap();
        record_station_witnesses(&store, "r", "build").unwrap();
        // Delete the premise so rebaseline drops (rather than re-stamps) its witness.
        std::fs::remove_file(root.join("design.md")).unwrap();
        let n = rebaseline_all(&store, "r").unwrap();
        assert_eq!(n, 1, "only the one witnessed premise is rebaselined");
        let units = store.read_units("r").unwrap();
        for u in units {
            assert!(
                u.frontmatter.input_witnesses.is_empty(),
                "the deleted witness is dropped; the unwitnessed unit is untouched"
            );
        }
    }
}

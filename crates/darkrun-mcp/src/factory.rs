//! Factory definitions — the ordered station plan a Run walks.
//!
//! Given a factory name, resolves its ordered stations, each station's
//! checkpoint kind, and the universal worker/reviewer slot.
//!
//! The plan's six stations are the fixed `Position::FLOW` spine (a hardcoded
//! invariant); each station's *orientation* — its risk class, locked artifact,
//! checkpoint, and role rosters — is loaded from the on-disk
//! `plugin/factories/<name>/` corpus via `darkrun-content`. There is no inline
//! factory definition in code: [`resolve_factory`] reads the corpus or returns
//! `None`.

use darkrun_core::domain::{CheckpointKind, Position};

/// A resolved station within a factory plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationDef {
    /// Stable station key (e.g. `"frame"`) — one of the six FSSBPH positions.
    pub name: String,
    /// Domain-facing display name shown over the fixed position (legal →
    /// `Intake`). `None` → the UI shows the position name.
    pub label: Option<String>,
    /// Human-readable summary of the risk this station eliminates.
    pub kills: String,
    /// The durable artifact the station locks on completion.
    pub artifact: String,
    /// The checkpoint gate that ends the station (the default path).
    pub checkpoint: CheckpointKind,
    /// Alternative gate paths the operator may pick (a compound gate). Empty →
    /// a single fixed gate.
    pub checkpoint_options: Vec<CheckpointKind>,
    /// The Explorers dispatched in the Spec phase — they gather context in
    /// **tandem** with the elaboration framing (discovery + elaboration run in
    /// parallel, mirroring the predecessor's `elaborate_loop`), before decompose.
    pub explorers: Vec<String>,
    /// The ordered Workers run in the Pass loop (Make -> Challenge -> Resolve...).
    pub workers: Vec<String>,
    /// The Reviewers that verify the station's output in the Review phase.
    pub reviewers: Vec<String>,
    /// Per-role model overrides, keyed by role name (any explorer/worker/
    /// reviewer that declared a `model:` in its definition). Resolved at
    /// dispatch over the factory default; absent → use the factory default.
    pub role_models: std::collections::BTreeMap<String, String>,
    /// Per-reviewer review posture (`lens` / `strict`), keyed by reviewer name,
    /// for reviewers that declared an `interpretation:`. Injected into the
    /// Review/Audit dispatch framing.
    pub role_interpretations: std::collections::BTreeMap<String, String>,
    /// Per-worker pass-loop role (`plan` / `build` / `verify`), keyed by worker
    /// name. Drives reject-bounce: a reject returns to the nearest preceding
    /// `build` worker. Absent entries default to `build`.
    pub worker_roles: std::collections::BTreeMap<String, String>,
    /// The upstream artifacts this station carries forward — its declared
    /// `inputs`. Validated for *template* coverage at content-load (every
    /// upstream `locked_artifact` is here or consciously waived); the cursor
    /// additionally enforces that the *run's units* actually consume each of
    /// these at decomposition, so the distillation isn't dropped at runtime.
    pub inputs: Vec<String>,
    /// Per-reviewer surface scope (`applies_to`), keyed by reviewer name, for
    /// reviewers that declared one. A reviewer with a scope fires only when the
    /// run's classified surface is in it; absent → fires always (E6).
    pub role_applies_to: std::collections::BTreeMap<String, Vec<String>>,
}

/// A resolved factory: an ordered list of stations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactoryDef {
    /// Factory key (e.g. `"software"`).
    pub name: String,
    /// Ordered stations, cost-of-late-discovery first.
    pub stations: Vec<StationDef>,
    /// The delivery surfaces this factory can produce, declared as data in
    /// `FACTORY.md`. The Shape station classifies the run into one of these; the
    /// classification routes how Prove/Audit verify. Empty → no surface stage.
    pub surfaces: Vec<String>,
    /// The factory's default model — the floor a role/station/unit override sits
    /// above. Empty → the harness/agent default.
    pub default_model: String,
    /// Whole-Run reviewer slugs — the cross-station auditors that judge the
    /// integrated run AFTER the final station locks, before it seals. Each gates.
    pub run_reviewers: Vec<String>,
    /// Per-run-reviewer surface scope (`applies_to`), keyed by reviewer name. A
    /// run reviewer with a scope joins the whole-run review only when the run's
    /// classified surface is in it; absent → it always fires (E6).
    pub run_reviewer_applies_to: std::collections::BTreeMap<String, Vec<String>>,
}

impl FactoryDef {
    /// Ordered station names.
    pub fn station_names(&self) -> Vec<String> {
        self.stations.iter().map(|s| s.name.clone()).collect()
    }

    /// Look up a station by name.
    pub fn station(&self, name: &str) -> Option<&StationDef> {
        self.stations.iter().find(|s| s.name == name)
    }

    /// The first station in the plan (the entry point of a fresh run).
    pub fn first_station(&self) -> Option<&StationDef> {
        self.stations.first()
    }

    /// The station that follows `name`, or `None` when `name` is last.
    pub fn next_station(&self, name: &str) -> Option<&StationDef> {
        let idx = self.stations.iter().position(|s| s.name == name)?;
        self.stations.get(idx + 1)
    }

    /// Whether `surface` is one of the factory's declared delivery surfaces.
    /// Tokens are compared through the canonical [`Surface`](darkrun_core::domain::Surface)
    /// parse so `web-ui`/`web_ui` spellings agree. A factory that declares no
    /// surfaces offers no classification, so every token is rejected.
    pub fn offers_surface(&self, surface: &str) -> bool {
        let want = match darkrun_core::domain::Surface::parse(surface) {
            Some(s) => s,
            None => return false,
        };
        self.surfaces
            .iter()
            .filter_map(|d| darkrun_core::domain::Surface::parse(d))
            .any(|d| d == want)
    }
}

impl FactoryDef {
    /// Build a `FactoryDef` from a loaded on-disk [`darkrun_content::Factory`].
    ///
    /// The stations are taken in the fixed `Position::FLOW` order, not from the
    /// factory's frontmatter — the spine is a hardcoded invariant. Each station's
    /// orientation (kills/label/artifact/checkpoint and the role rosters) comes
    /// from its `STATION.md`.
    fn from_content(f: &darkrun_content::Factory) -> FactoryDef {
        let stations = Position::FLOW
            .iter()
            .filter_map(|pos| f.station(pos.dir()))
            .map(StationDef::from_content)
            .collect();
        let mut run_reviewer_applies_to = std::collections::BTreeMap::new();
        for r in &f.run_reviewers {
            if !r.frontmatter.applies_to.is_empty() {
                run_reviewer_applies_to.insert(r.name().to_string(), r.frontmatter.applies_to.clone());
            }
        }
        FactoryDef {
            name: f.name().to_string(),
            stations,
            surfaces: f.frontmatter.surfaces.clone(),
            default_model: f.frontmatter.default_model.clone(),
            run_reviewers: f.frontmatter.reviewers.clone(),
            run_reviewer_applies_to,
        }
    }
}

impl StationDef {
    /// Build a `StationDef` from a loaded on-disk [`darkrun_content::Station`].
    fn from_content(s: &darkrun_content::Station) -> StationDef {
        // Collect every role's declared model override (explorers + workers +
        // reviewers), keyed by role name, so dispatch can resolve per-role.
        let mut role_models = std::collections::BTreeMap::new();
        let mut role_interpretations = std::collections::BTreeMap::new();
        let mut role_applies_to = std::collections::BTreeMap::new();
        let mut worker_roles = std::collections::BTreeMap::new();
        for role in s.explorers.iter().chain(&s.workers).chain(&s.reviewers) {
            if let Some(model) = &role.frontmatter.model {
                if !model.trim().is_empty() {
                    role_models.insert(role.name().to_string(), model.clone());
                }
            }
            if let Some(interp) = &role.frontmatter.interpretation {
                if !interp.trim().is_empty() {
                    role_interpretations.insert(role.name().to_string(), interp.clone());
                }
            }
            if !role.frontmatter.applies_to.is_empty() {
                role_applies_to
                    .insert(role.name().to_string(), role.frontmatter.applies_to.clone());
            }
        }
        for w in &s.workers {
            if let Some(r) = &w.frontmatter.role {
                if !r.trim().is_empty() {
                    worker_roles.insert(w.name().to_string(), r.clone());
                }
            }
        }
        StationDef {
            name: s.name().to_string(),
            label: s.frontmatter.label.clone(),
            kills: s.frontmatter.kills.clone(),
            artifact: s.frontmatter.locked_artifact.clone(),
            checkpoint: s.checkpoint(),
            checkpoint_options: s.frontmatter.checkpoint_options.clone(),
            explorers: s.frontmatter.explorers.clone(),
            workers: s.frontmatter.workers.clone(),
            reviewers: s.frontmatter.reviewers.clone(),
            role_models,
            role_interpretations,
            worker_roles,
            inputs: s.frontmatter.inputs.clone(),
            role_applies_to,
        }
    }
}

/// Resolve a factory by name from the embedded corpus. Returns `None` for an
/// unknown or structurally-invalid factory.
///
/// The source of truth is the on-disk `plugin/factories/<name>/` content — there
/// is no inline definition in code. The six FSSBPH stations are walked in their
/// fixed `Position::FLOW` order. For project-override resolution, use
/// [`resolve_factory_at`].
pub fn resolve_factory(name: &str) -> Option<FactoryDef> {
    darkrun_content::load_validated(name)
        .ok()
        .map(|f| FactoryDef::from_content(&f))
}

/// Resolve a factory through the full cascade rooted at `repo_root`: a project
/// override at `<repo_root>/.darkrun/factories/<name>/` beats the embedded
/// corpus, crossed with the factory's `inherits` chain.
pub fn resolve_factory_at(repo_root: &std::path::Path, name: &str) -> Option<FactoryDef> {
    darkrun_content::load_validated_at(Some(repo_root), name)
        .ok()
        .map(|f| FactoryDef::from_content(&f))
}

/// Every factory available in this build, resolved from the corpus.
pub fn list_factories() -> Vec<FactoryDef> {
    darkrun_content::list_factories()
        .into_iter()
        .filter_map(|name| resolve_factory(&name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_factory_has_six_ordered_stations() {
        let f = resolve_factory("software").expect("software resolves from the corpus");
        assert_eq!(
            f.station_names(),
            vec!["frame", "specify", "shape", "build", "prove", "harden"]
        );
        assert_eq!(f.first_station().unwrap().name, "frame");
        assert_eq!(f.next_station("frame").unwrap().name, "specify");
        assert!(f.next_station("harden").is_none());
    }

    #[test]
    fn software_station_orientation_loads_from_disk() {
        let f = resolve_factory("software").unwrap();
        let build = f.station("build").unwrap();
        assert_eq!(build.kills, "implementation-defects");
        assert_eq!(build.artifact, "code");
        assert_eq!(build.checkpoint, CheckpointKind::Ask);
        assert_eq!(build.workers, vec!["test_author", "builder", "self_reviewer", "reconciler"]);
    }

    #[test]
    fn resolve_unknown_factory_is_none() {
        assert!(resolve_factory("nope").is_none());
        assert!(resolve_factory("software").is_some());
    }

    #[test]
    fn software_declares_the_full_surface_set() {
        let f = resolve_factory("software").unwrap();
        assert_eq!(f.surfaces.len(), 8);
        assert!(f.offers_surface("web_ui"));
        assert!(f.offers_surface("web-ui"), "tolerant spelling agrees");
        assert!(f.offers_surface("library"));
        assert!(!f.offers_surface("hologram"));
    }

    #[test]
    fn factory_and_role_models_resolve_from_the_corpus() {
        let f = resolve_factory("software").unwrap();
        // The factory default model flows onto the def.
        assert_eq!(f.default_model, "sonnet");
        // A role that declares a `model:` is captured per-role for dispatch.
        let shape = f.station("shape").unwrap();
        assert_eq!(shape.role_models.get("spiker").map(String::as_str), Some("sonnet"));
        // A reviewer that declares an `interpretation:` is captured too.
        let build = f.station("build").unwrap();
        assert_eq!(build.role_interpretations.get("correctness").map(String::as_str), Some("strict"));
        // A worker that declares a pass-loop `role:` is captured for reject-routing.
        assert_eq!(build.worker_roles.get("self_reviewer").map(String::as_str), Some("verify"));
        // shape offers a compound gate (ask default, external alternative).
        let shape = f.station("shape").unwrap();
        assert_eq!(shape.checkpoint, CheckpointKind::Ask);
        assert_eq!(
            shape.checkpoint_options,
            vec![CheckpointKind::Ask, CheckpointKind::External]
        );
        // A single-gate station carries no options.
        assert!(f.station("frame").unwrap().checkpoint_options.is_empty());
        // A station's declared inputs flow onto the def (for the runtime
        // input-coverage gate). frame is the first station — no upstream inputs.
        assert!(f.station("frame").unwrap().inputs.is_empty());
        assert_eq!(
            f.station("build").unwrap().inputs,
            vec!["frame.md", "spec.md", "design.md"]
        );
    }

    #[test]
    fn libdev_narrows_surfaces_to_library_and_api() {
        let f = resolve_factory("libdev").unwrap();
        assert!(f.offers_surface("library"));
        assert!(f.offers_surface("api"));
        // A library has no UI — it cannot classify as a visual surface.
        assert!(!f.offers_surface("web_ui"));
        assert!(!f.offers_surface("desktop"));
    }
}

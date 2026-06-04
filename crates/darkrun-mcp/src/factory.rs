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
    /// The checkpoint gate that ends the station.
    pub checkpoint: CheckpointKind,
    /// The Explorers dispatched in the Spec phase — they gather context in
    /// **tandem** with the elaboration framing (discovery + elaboration run in
    /// parallel, mirroring the predecessor's `elaborate_loop`), before decompose.
    pub explorers: Vec<String>,
    /// The ordered Workers run in the Pass loop (Make -> Challenge -> Resolve...).
    pub workers: Vec<String>,
    /// The Reviewers that verify the station's output in the Review phase.
    pub reviewers: Vec<String>,
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
        FactoryDef {
            name: f.name().to_string(),
            stations,
            surfaces: f.frontmatter.surfaces.clone(),
        }
    }
}

impl StationDef {
    /// Build a `StationDef` from a loaded on-disk [`darkrun_content::Station`].
    fn from_content(s: &darkrun_content::Station) -> StationDef {
        StationDef {
            name: s.name().to_string(),
            label: s.frontmatter.label.clone(),
            kills: s.frontmatter.kills.clone(),
            artifact: s.frontmatter.locked_artifact.clone(),
            checkpoint: s.checkpoint(),
            explorers: s.frontmatter.explorers.clone(),
            workers: s.frontmatter.workers.clone(),
            reviewers: s.frontmatter.reviewers.clone(),
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
    fn libdev_narrows_surfaces_to_library_and_api() {
        let f = resolve_factory("libdev").unwrap();
        assert!(f.offers_surface("library"));
        assert!(f.offers_surface("api"));
        // A library has no UI — it cannot classify as a visual surface.
        assert!(!f.offers_surface("web_ui"));
        assert!(!f.offers_surface("desktop"));
    }
}

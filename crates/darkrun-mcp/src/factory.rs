//! Factory definitions — the ordered station plan a Run walks.
//!
//! Given a factory name, resolves its ordered stations, each station's
//! checkpoint kind, and the universal worker/reviewer slot.
//!
//! For this first vertical slice the **software factory** is defined inline
//! here as the authoritative fallback. Once `darkrun-content` lands its
//! embedded corpus, [`resolve_factory`] can prefer the embedded definition
//! and fall back to this table — the manager only consumes the
//! [`FactoryDef`] shape, so swapping the source is transparent.

use darkrun_core::domain::CheckpointKind;

/// A resolved station within a factory plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationDef {
    /// Stable station key (e.g. `"frame"`).
    pub name: String,
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
}

fn station(
    name: &str,
    kills: &str,
    artifact: &str,
    checkpoint: CheckpointKind,
    explorers: &[&str],
    workers: &[&str],
    reviewers: &[&str],
) -> StationDef {
    StationDef {
        name: name.to_string(),
        kills: kills.to_string(),
        artifact: artifact.to_string(),
        checkpoint,
        explorers: explorers.iter().map(|s| s.to_string()).collect(),
        workers: workers.iter().map(|s| s.to_string()).collect(),
        reviewers: reviewers.iter().map(|s| s.to_string()).collect(),
    }
}

/// The software factory — the redesigned station plan ordered by
/// cost-of-late-discovery: Frame -> Specify -> Shape -> Build -> Prove -> Harden.
pub fn software_factory() -> FactoryDef {
    use CheckpointKind::Ask;
    FactoryDef {
        name: "software".to_string(),
        stations: vec![
            station(
                "frame",
                "wrong-thing",
                "frame.md",
                Ask,
                &["context", "value"],
                &["framer", "challenger", "distiller"],
                &["value", "feasibility"],
            ),
            station(
                "specify",
                "ambiguity",
                "spec.md",
                Ask,
                &["contract", "edge_case"],
                &["spec_writer", "adversary", "tightener"],
                &["completeness", "testability"],
            ),
            station(
                "shape",
                "expensive-structural-reversal",
                "shape.md",
                Ask,
                &["architecture", "risk", "surface"],
                &["architect", "risk_challenger", "simplifier"],
                &["soundness", "reversibility"],
            ),
            station(
                "build",
                "implementation-defects",
                "build.md",
                Ask,
                &["integration_point", "reuse"],
                &["test_author", "builder", "self_reviewer", "reconciler"],
                &["correctness", "maintainability"],
            ),
            station(
                "prove",
                "escaped-defects",
                "prove.md",
                Ask,
                &["regression", "scenario"],
                &["scenario_author", "prover", "regressor"],
                &["coverage", "evidence"],
            ),
            station(
                "harden",
                "works-in-dev-dies-in-prod",
                "release.md",
                Ask,
                &["operability", "threat"],
                &["hardener", "red_teamer", "releaser"],
                &["security", "readiness"],
            ),
        ],
    }
}

/// Resolve a factory by name. Returns `None` for unknown factories.
///
/// Only the software factory ships in this slice; other factories resolve to
/// `None` until `darkrun-content` provides them.
pub fn resolve_factory(name: &str) -> Option<FactoryDef> {
    match name {
        "software" => Some(software_factory()),
        _ => None,
    }
}

/// Every factory available in this build.
pub fn list_factories() -> Vec<FactoryDef> {
    vec![software_factory()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_factory_has_six_ordered_stations() {
        let f = software_factory();
        assert_eq!(
            f.station_names(),
            vec!["frame", "specify", "shape", "build", "prove", "harden"]
        );
        assert_eq!(f.first_station().unwrap().name, "frame");
        assert_eq!(f.next_station("frame").unwrap().name, "specify");
        assert!(f.next_station("harden").is_none());
    }

    #[test]
    fn resolve_unknown_factory_is_none() {
        assert!(resolve_factory("nope").is_none());
        assert!(resolve_factory("software").is_some());
    }
}

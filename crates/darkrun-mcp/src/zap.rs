//! Zero-ceremony single-task execution (the `darkrun-zap` skill).
//!
//! Zap resolves a factory + station and hands back that station's Worker
//! sequence and a run/verify/commit procedure — no Run record, no decomposition,
//! no manager tick. The agent drives the returned procedure itself.

use serde::Serialize;

use crate::factory::{list_factories, resolve_factory};

/// A resolved zap: the station's Worker loop and the procedure to run it.
#[derive(Debug, Clone, Serialize)]
pub struct Zap {
    /// The task to execute.
    pub task: String,
    /// The resolved factory.
    pub factory: String,
    /// The resolved station.
    pub station: String,
    /// The risk class the station eliminates.
    pub kills: String,
    /// The station's Workers, in Pass-loop order.
    pub workers: Vec<String>,
    /// The procedure the agent follows.
    pub message: String,
}

/// A zap that couldn't resolve its factory or station, with the valid choices.
#[derive(Debug, Clone, Serialize)]
pub struct ZapError {
    /// `zap_factory_not_found` or `zap_station_not_found`.
    pub error: String,
    /// Valid factory names (set when the factory didn't resolve).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub valid_factories: Vec<String>,
    /// Valid station names (set when the station didn't resolve).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub valid_stations: Vec<String>,
}

/// Resolve a zap. Defaults to the software factory's build station — the
/// build-class station where small work lands.
pub fn zap(
    task: &str,
    factory: Option<&str>,
    station: Option<&str>,
) -> std::result::Result<Zap, ZapError> {
    let factory_name = factory.unwrap_or("software");
    // Zap is stateless (no run, no project context) — embedded resolution.
    let factory = resolve_factory(factory_name).ok_or_else(|| ZapError {
        error: "zap_factory_not_found".to_string(),
        valid_factories: list_factories().iter().map(|f| f.name.clone()).collect(),
        valid_stations: Vec::new(),
    })?;

    let station_name = default_station(&factory, station);
    // Clone the fields we need out of the station def so the immutable borrow of
    // `factory` ends before we move `factory.name` into the result.
    let (station, kills, workers) = {
        let def = factory.station(&station_name).ok_or_else(|| ZapError {
            error: "zap_station_not_found".to_string(),
            valid_factories: Vec::new(),
            valid_stations: factory.station_names(),
        })?;
        (def.name.clone(), def.kills.clone(), def.workers.clone())
    };

    let worker_line = worker_line(&workers);
    let message = format!(
        "Zap: run `{task}` straight through the **{station}** station's Worker loop — \
         stateless, no Run, nothing under `.darkrun/`.\n\n\
         Worker sequence: {worker_line}.\n\n\
         Procedure:\n\
         1. Preflight — confirm a clean enough working tree to attribute the change.\n\
         2. Make → Challenge → Resolve: produce the change, adversarially attack it \
         (edge cases, missing handling), then reconcile. Build the real thing.\n\
         3. Verify — run this station's checks (tests / type-check / lint / build) \
         completely. A partial run is not a pass.\n\
         4. Commit only on PASS. If verification fails, fix and re-verify; if it can't \
         converge, stop and report rather than committing broken work.\n\n\
         This station eliminates **{kills}**.",
        task = task,
        station = station,
        worker_line = worker_line,
        kills = kills,
    );

    Ok(Zap {
        task: task.to_string(),
        factory: factory.name,
        station,
        kills,
        workers,
        message,
    })
}

/// The station a zap targets: an explicit one, else `build` when the factory
/// has it, else the factory's first station. (The non-`build` fallbacks support
/// factories whose pipeline doesn't include a build station.)
fn default_station(factory: &crate::factory::FactoryDef, station: Option<&str>) -> String {
    station.map(str::to_string).unwrap_or_else(|| {
        if factory.station("build").is_some() {
            "build".to_string()
        } else {
            factory.first_station().map(|s| s.name.clone()).unwrap_or_default()
        }
    })
}

/// The worker-sequence line: the station's workers joined, or a generic note
/// when a station declares none.
fn worker_line(workers: &[String]) -> String {
    if workers.is_empty() {
        "the station's Worker loop".to_string()
    } else {
        workers.join(" → ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_station_falls_back_and_worker_line_handles_empty() {
        use crate::factory::{FactoryDef, StationDef};
        let station = StationDef {
            name: "frame".into(), label: None, optional: false, kills: "wrong-thing".into(), artifact: "o.md".into(),
            explorers: vec![],
            workers: vec![], fix_workers: vec![], reviewers: vec![], role_models: Default::default(),
            role_interpretations: Default::default(), worker_roles: Default::default(),
            inputs: vec![], role_applies_to: Default::default(),
        };
        let def = FactoryDef {
            name: "no-build".into(), stations: vec![station], surfaces: vec![],
            default_model: "sonnet".into(), run_reviewers: vec![], run_reviewer_applies_to: Default::default(),
        };
        // A factory with no `build` station falls back to the first station.
        assert_eq!(default_station(&def, None), "frame");
        // An explicit station always wins.
        assert_eq!(default_station(&def, Some("specify")), "specify");
        // A factory with no stations at all → empty.
        let empty = FactoryDef {
            name: "e".into(), stations: vec![], surfaces: vec![], default_model: String::new(),
            run_reviewers: vec![], run_reviewer_applies_to: Default::default(),
        };
        assert_eq!(default_station(&empty, None), "");
        // worker_line: empty → the generic loop note; populated → joined.
        assert_eq!(worker_line(&[]), "the station's Worker loop");
        assert_eq!(worker_line(&["a".into(), "b".into()]), "a → b");
    }

    #[test]
    fn defaults_to_software_build_station() {
        let z = zap("fix the padding", None, None).expect("zap");
        assert_eq!(z.factory, "software");
        assert_eq!(z.station, "build");
        assert!(!z.workers.is_empty());
        assert!(z.message.contains("fix the padding"));
        assert!(z.message.contains("Worker sequence"));
    }

    #[test]
    fn unknown_factory_returns_valid_list() {
        let e = zap("x", Some("nope"), None).unwrap_err();
        assert_eq!(e.error, "zap_factory_not_found");
        assert!(e.valid_factories.contains(&"software".to_string()));
    }

    #[test]
    fn unknown_station_returns_valid_list() {
        let e = zap("x", Some("software"), Some("nope")).unwrap_err();
        assert_eq!(e.error, "zap_station_not_found");
        assert!(e.valid_stations.contains(&"build".to_string()));
    }

    #[test]
    fn explicit_station_resolves() {
        let z = zap("tighten the api", Some("software"), Some("shape")).expect("zap");
        assert_eq!(z.station, "shape");
    }
}

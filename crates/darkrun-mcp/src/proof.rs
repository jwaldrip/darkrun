//! Surface classification + objective-proof helpers.
//!
//! These back the surface/proof MCP tools that make darkrun's verification
//! **objective measurement** rather than an agent reading code and asserting
//! quality. The flow is:
//!
//! 1. **Shape classifies the surface.** Once the run's deliverable is known
//!    (library / api / web-ui / tui / cli / desktop / mobile / data), the agent
//!    records it via [`set_surface`] — persisted onto the run frontmatter.
//! 2. **Prove/Audit route by surface.** The surface decides which measurement
//!    applies — a headless-browser [`WebProof`] for visual surfaces, a
//!    criterion [`BenchProof`] for bench surfaces, a terminal snapshot for
//!    cli/tui — and the measured proof is attached via [`attach_proof`].
//! 3. **The view/review reads the proof back** with [`get_proof`].
//!
//! The proof is stored as a JSON document at `.darkrun/<run>/proof.json`,
//! keyed by station so each station that measures can attach independently.
//! The [`Surface`] on the wire mirrors [`darkrun_core::domain::Surface`]; the
//! two enums share their snake_case tokens, so conversion goes through
//! [`Surface::as_str`](darkrun_core::domain::Surface::as_str) /
//! [`Surface::parse`](darkrun_core::domain::Surface::parse).

use std::collections::BTreeMap;

use darkrun_api::proof::{Proof, ProofAttachResponse, ProofGetResponse, Surface as ApiSurface};
use darkrun_core::domain::Surface as CoreSurface;
use darkrun_core::StateStore;
use serde::{Deserialize, Serialize};

use crate::error::{McpError, Result};

/// The on-disk proof store: per-station attached proofs plus an optional
/// run-level (unscoped) proof. Serialized to `.darkrun/<run>/proof.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ProofStore {
    /// The run-level proof (attached without a station scope).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<Proof>,
    /// Proofs attached scoped to a named station.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    stations: BTreeMap<String, Proof>,
}

/// Convert a `darkrun-core` surface to its `darkrun-api` mirror (lossless —
/// the two enums share their wire tokens).
fn api_surface(core: CoreSurface) -> std::result::Result<ApiSurface, McpError> {
    serde_json::from_value(serde_json::Value::String(core.as_str().to_string()))
        .map_err(McpError::Json)
}

/// Read the proof store for a run, defaulting to empty when none is attached.
fn read_store(store: &StateStore, slug: &str) -> Result<ProofStore> {
    let path = store.run_dir(slug).join("proof.json");
    if !path.exists() {
        return Ok(ProofStore::default());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| McpError::InvalidInput(format!("reading proof.json: {e}")))?;
    serde_json::from_str(&raw).map_err(McpError::Json)
}

/// Persist the proof store for a run.
fn write_store(store: &StateStore, slug: &str, ps: &ProofStore) -> Result<()> {
    let dir = store.run_dir(slug);
    std::fs::create_dir_all(&dir)
        .map_err(|e| McpError::InvalidInput(format!("creating run dir: {e}")))?;
    let path = dir.join("proof.json");
    let json = serde_json::to_string_pretty(ps).map_err(McpError::Json)?;
    std::fs::write(&path, json)
        .map_err(|e| McpError::InvalidInput(format!("writing proof.json: {e}")))?;
    Ok(())
}

/// The structured result of reading/setting a run's surface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SurfaceResult {
    /// The run slug.
    pub run: String,
    /// The classified surface token (`web_ui`, `library`, …), or `None` when
    /// the run has not been classified yet.
    pub surface: Option<String>,
    /// Whether this surface is verified through a headless browser
    /// (web-ui / desktop / mobile).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_visual: Option<bool>,
    /// Whether this surface is verified through criterion benches + a load
    /// harness (library / api / data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_bench: Option<bool>,
    /// Whether this surface is verified through a terminal/output snapshot
    /// (tui / cli).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_terminal: Option<bool>,
    /// The verification route the surface selects — `web`, `bench`, or
    /// `terminal` — or `None` when unclassified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<&'static str>,
}

impl SurfaceResult {
    fn from_surface(run: &str, surface: Option<CoreSurface>) -> Self {
        match surface {
            Some(s) => SurfaceResult {
                run: run.to_string(),
                surface: Some(s.as_str().to_string()),
                is_visual: Some(s.is_visual()),
                is_bench: Some(s.is_bench()),
                is_terminal: Some(s.is_terminal()),
                route: Some(route_for(s)),
            },
            None => SurfaceResult {
                run: run.to_string(),
                surface: None,
                is_visual: None,
                is_bench: None,
                is_terminal: None,
                route: None,
            },
        }
    }
}

/// The verification route a surface selects.
pub fn route_for(surface: CoreSurface) -> &'static str {
    if surface.is_visual() {
        "web"
    } else if surface.is_bench() {
        "bench"
    } else {
        "terminal"
    }
}

/// Classify (set) the run's surface. Parses the token tolerantly
/// (`web-ui`/`webui`/`lib` all normalize) and persists it onto the run
/// frontmatter — this is what the Shape station calls once it has classified
/// the deliverable.
pub fn set_surface(store: &StateStore, slug: &str, raw_surface: &str) -> Result<SurfaceResult> {
    let surface = CoreSurface::parse(raw_surface)
        .ok_or_else(|| McpError::InvalidInput(format!("unknown surface: {raw_surface}")))?;
    let mut run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;

    // The classification must be one the run's factory actually offers — a
    // library factory cannot classify a run as `web_ui`. Surfaces are per-factory
    // declared data; this is where that declaration is enforced. (Resolution is
    // best-effort: a factory that declares no surfaces — or won't resolve — does
    // not constrain the classification, preserving the prior open behavior.)
    if let Some(def) = crate::position::resolve_factory_for(store, &run.frontmatter.factory) {
        if !def.surfaces.is_empty() && !def.offers_surface(raw_surface) {
            return Err(McpError::InvalidInput(format!(
                "the {} factory does not offer the `{}` surface (offers: {})",
                def.name,
                surface.as_str(),
                def.surfaces.join(", ")
            )));
        }
    }

    run.set_surface(surface);
    store.write_run(&run)?;
    Ok(SurfaceResult::from_surface(slug, Some(surface)))
}

/// Read the run's currently classified surface, if any.
pub fn get_surface(store: &StateStore, slug: &str) -> Result<SurfaceResult> {
    let run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    Ok(SurfaceResult::from_surface(slug, run.surface()))
}

/// Attach an objective [`Proof`] to a run, optionally scoped to a station.
///
/// The proof's surface must match the run's classified surface (set at Shape) —
/// you cannot attach a web proof to a bench run. The response carries
/// [`block_matches_surface`](Proof::block_matches_surface): the proof must
/// carry the measurement block its surface routes to (a visual surface needs a
/// `web` block, a bench surface a `bench` block) before it counts as evidence.
pub fn attach_proof(
    store: &StateStore,
    slug: &str,
    proof: Proof,
    station: Option<String>,
) -> Result<ProofAttachResponse> {
    let run = store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;

    // The run must have been classified before a proof can be routed.
    let classified = run.surface().ok_or_else(|| {
        McpError::InvalidInput(format!(
            "run '{slug}' has no classified surface — Shape must record one before Prove can attach a proof"
        ))
    })?;
    let expected = api_surface(classified)?;
    if proof.surface != expected {
        return Err(McpError::InvalidInput(format!(
            "proof surface '{}' does not match the run's classified surface '{}'",
            proof.surface.as_str(),
            expected.as_str()
        )));
    }

    let block_matches_surface = proof.block_matches_surface();
    let mut ps = read_store(store, slug)?;
    match &station {
        Some(s) => {
            ps.stations.insert(s.clone(), proof.clone());
        }
        None => ps.run = Some(proof.clone()),
    }
    write_store(store, slug, &ps)?;

    Ok(ProofAttachResponse {
        ok: true,
        run: slug.to_string(),
        surface: proof.surface,
        block_matches_surface,
    })
}

/// Read a run's attached proof — the station-scoped proof when `station` is
/// given (falling back to the run-level proof), or the run-level proof
/// otherwise. Errors when no matching proof has been attached.
pub fn get_proof(
    store: &StateStore,
    slug: &str,
    station: Option<String>,
) -> Result<ProofGetResponse> {
    // Confirm the run exists first for a clean error.
    store
        .read_run(slug)
        .map_err(|_| McpError::Core(darkrun_core::CoreError::RunNotFound(slug.to_string())))?;
    let ps = read_store(store, slug)?;
    let (resolved_station, proof) = match &station {
        Some(s) => match ps.stations.get(s) {
            Some(p) => (Some(s.clone()), p.clone()),
            None => match ps.run {
                Some(p) => (None, p),
                None => {
                    return Err(McpError::InvalidInput(format!(
                        "no proof attached for run '{slug}' (station '{s}' or run-level)"
                    )))
                }
            },
        },
        None => match ps.run {
            Some(p) => (None, p),
            None => {
                return Err(McpError::InvalidInput(format!(
                    "no run-level proof attached for run '{slug}'"
                )))
            }
        },
    };
    Ok(ProofGetResponse {
        run: slug.to_string(),
        station: resolved_station,
        proof,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::run_start;
    use darkrun_api::proof::{AuditResult, BenchProof, WebProof};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    fn started(store: &StateStore, slug: &str) {
        run_start(store, slug, "software", None, "continuous").unwrap();
    }

    #[test]
    fn surface_is_none_until_classified() {
        let (_d, store) = store();
        started(&store, "r");
        let res = get_surface(&store, "r").unwrap();
        assert_eq!(res.surface, None);
        assert_eq!(res.route, None);
        assert_eq!(res.is_visual, None);
    }

    #[test]
    fn set_surface_classifies_and_persists() {
        let (_d, store) = store();
        started(&store, "r");
        let res = set_surface(&store, "r", "web-ui").unwrap();
        assert_eq!(res.surface.as_deref(), Some("web_ui"));
        assert_eq!(res.is_visual, Some(true));
        assert_eq!(res.is_bench, Some(false));
        assert_eq!(res.route, Some("web"));

        // Survives a re-read through the run frontmatter.
        let reread = get_surface(&store, "r").unwrap();
        assert_eq!(reread.surface.as_deref(), Some("web_ui"));
        assert_eq!(store.read_run("r").unwrap().surface(), Some(CoreSurface::WebUi));
    }

    #[test]
    fn set_surface_tolerates_spellings() {
        let (_d, store) = store();
        started(&store, "r");
        assert_eq!(
            set_surface(&store, "r", "lib").unwrap().surface.as_deref(),
            Some("library")
        );
        assert_eq!(
            set_surface(&store, "r", "WEBUI").unwrap().surface.as_deref(),
            Some("web_ui")
        );
    }

    #[test]
    fn set_surface_rejects_unknown_token() {
        let (_d, store) = store();
        started(&store, "r");
        let err = set_surface(&store, "r", "telepathy").unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn surface_routes_for_each_class() {
        assert_eq!(route_for(CoreSurface::WebUi), "web");
        assert_eq!(route_for(CoreSurface::Desktop), "web");
        assert_eq!(route_for(CoreSurface::Mobile), "web");
        assert_eq!(route_for(CoreSurface::Library), "bench");
        assert_eq!(route_for(CoreSurface::Api), "bench");
        assert_eq!(route_for(CoreSurface::Data), "bench");
        assert_eq!(route_for(CoreSurface::Cli), "terminal");
        assert_eq!(route_for(CoreSurface::Tui), "terminal");
    }

    #[test]
    fn attach_requires_a_classified_surface() {
        let (_d, store) = store();
        started(&store, "r");
        let proof = Proof::bench(ApiSurface::Library, BenchProof::default());
        let err = attach_proof(&store, "r", proof, None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn attach_rejects_surface_mismatch() {
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "library").unwrap();
        // The run is a library (bench), but a web proof is offered.
        let proof = Proof::web(ApiSurface::WebUi, WebProof::default());
        let err = attach_proof(&store, "r", proof, None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn attach_bench_proof_and_read_back() {
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "api").unwrap();
        let proof = Proof::bench(
            ApiSurface::Api,
            BenchProof {
                p50: Some(1.0),
                p95: Some(2.5),
                p99: Some(4.0),
                throughput: Some(12_000.0),
                samples: Some(500),
            },
        );
        let resp = attach_proof(&store, "r", proof, Some("prove".into())).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.surface, ApiSurface::Api);
        assert!(resp.block_matches_surface);

        let got = get_proof(&store, "r", Some("prove".into())).unwrap();
        assert_eq!(got.run, "r");
        assert_eq!(got.station.as_deref(), Some("prove"));
        assert_eq!(got.proof.bench.unwrap().p95, Some(2.5));
    }

    #[test]
    fn attach_web_proof_carries_block_match() {
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "web-ui").unwrap();
        let mut vitals = BTreeMap::new();
        vitals.insert("lcp".to_string(), 1100.0);
        let proof = Proof::web(
            ApiSurface::WebUi,
            WebProof {
                vitals,
                audits: vec![AuditResult {
                    name: "contrast".into(),
                    value: "5.1:1".into(),
                    pass: true,
                }],
                screenshot_url: Some("/shot/home.png".into()),
            },
        );
        let resp = attach_proof(&store, "r", proof, None).unwrap();
        assert!(resp.block_matches_surface);
        let got = get_proof(&store, "r", None).unwrap();
        let web = got.proof.web.unwrap();
        assert_eq!(web.vitals.get("lcp"), Some(&1100.0));
        assert!(web.all_audits_pass());
    }

    #[test]
    fn block_mismatch_is_surfaced_not_rejected() {
        // A visual surface carrying no web block is *recorded* but flagged —
        // the attach succeeds so the agent can see exactly what's missing.
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "desktop").unwrap();
        let proof = Proof {
            surface: ApiSurface::Desktop,
            web: None,
            bench: None,
        };
        let resp = attach_proof(&store, "r", proof, None).unwrap();
        assert!(resp.ok);
        assert!(!resp.block_matches_surface, "missing web block must be flagged");
    }

    #[test]
    fn get_proof_falls_back_to_run_level() {
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "cli").unwrap();
        // CLI: a terminal surface carries a screenshot-only proof.
        let proof = Proof {
            surface: ApiSurface::Cli,
            web: Some(WebProof {
                screenshot_url: Some("/snap/out.txt".into()),
                ..Default::default()
            }),
            bench: None,
        };
        attach_proof(&store, "r", proof, None).unwrap();
        // Asking for a station with no scoped proof falls back to run-level.
        let got = get_proof(&store, "r", Some("prove".into())).unwrap();
        assert_eq!(got.station, None);
        assert_eq!(
            got.proof.web.unwrap().screenshot_url.as_deref(),
            Some("/snap/out.txt")
        );
    }

    #[test]
    fn get_proof_errors_when_absent() {
        let (_d, store) = store();
        started(&store, "r");
        let err = get_proof(&store, "r", None).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn station_scoped_proofs_are_independent() {
        let (_d, store) = store();
        started(&store, "r");
        set_surface(&store, "r", "data").unwrap();
        attach_proof(
            &store,
            "r",
            Proof::bench(ApiSurface::Data, BenchProof { p50: Some(1.0), ..Default::default() }),
            Some("prove".into()),
        )
        .unwrap();
        attach_proof(
            &store,
            "r",
            Proof::bench(ApiSurface::Data, BenchProof { p50: Some(9.0), ..Default::default() }),
            Some("harden".into()),
        )
        .unwrap();
        assert_eq!(
            get_proof(&store, "r", Some("prove".into())).unwrap().proof.bench.unwrap().p50,
            Some(1.0)
        );
        assert_eq!(
            get_proof(&store, "r", Some("harden".into())).unwrap().proof.bench.unwrap().p50,
            Some(9.0)
        );
    }

    #[test]
    fn surface_on_missing_run_errors() {
        let (_d, store) = store();
        let err = get_surface(&store, "ghost").unwrap_err();
        assert!(matches!(err, McpError::Core(_)));
    }
}

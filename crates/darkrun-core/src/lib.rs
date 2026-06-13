//! darkrun-core — the domain model + filesystem state engine for darkrun.
//!
//! darkrun models software delivery as a **Factory** of ordered **Stations**.
//! A top-level execution is a **Run**; inside every Station the engine walks a
//! universal slot: `Explore -> Decompose -> Pass-loop(Make->Challenge->Resolve)
//! -> Review -> Checkpoint -> Lock`. Hierarchy: Factory > Station > Unit > Pass.
//!
//! This crate owns the domain + state concepts for the engine, expressed in
//! the factory vocabulary, with state living entirely on the
//! filesystem under `.darkrun/`.
//!
//! Modules:
//! - [`annotation`] — annotation storage, the text re-anchor pass, and the
//!   open-ask severity aggregation that steers the checkpoint.
//! - [`domain`]      — the factory domain types (Run, Station, Unit, Pass, ...).
//! - [`frontmatter`] — YAML-frontmatter + markdown-body parsing.
//! - [`state`]       — the [`state::StateStore`] filesystem engine.
//! - [`locks`]       — advisory mkdir locks with stale-holder recovery.
//! - [`dag`]         — the unit dependency graph (topo order, waves, ready-set).
//! - [`error`]       — the crate error type.

pub mod annotation;
pub mod boot;
pub mod dag;
pub mod derive;
pub mod domain;
pub mod error;
pub mod frontmatter;
pub mod gate_env;
pub mod locks;
pub mod state;
pub mod witness;

pub use annotation::{
    checkpoint_button_state, count_open_by_severity, flag_scene_changed, pixel_region,
    reanchor_annotation, reanchor_text, region_out_of_bounds, scene_changed, CheckpointButton,
    OpenSeverityCounts, ReAnchor,
};
pub use dag::Dag;
pub use derive::{derive_station_phase, station_status, station_units_complete};
pub use error::{CoreError, Result};
pub use locks::{LockGuard, LockManager};
pub use state::{
    run_is_complete, RunState, StateStore, StationStatus, LEGACY_VERSION, SCHEMA_VERSION,
    SCHEMA_VERSION_LEGACY,
};
pub use witness::{hash_bytes, hash_file};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Run, RunFrontmatter, Status, Surface, Unit, UnitFrontmatter};

    fn unit(slug: &str, status: Status, deps: &[&str]) -> Unit {
        Unit {
            slug: slug.to_string(),
            frontmatter: UnitFrontmatter {
                status,
                depends_on: deps.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            title: slug.to_string(),
            body: String::new(),
        }
    }

    #[test]
    fn frontmatter_roundtrip() {
        let fm = RunFrontmatter {
            title: Some("Ship the thing".into()),
            factory: "software".into(),
            mode: crate::domain::Mode::Solo,
            active_station: "frame".into(),
            status: Status::Active,
            ..Default::default()
        };
        let body = "# Ship the thing\n\nDeliver the vertical slice.\n";
        let serialized = frontmatter::serialize(&fm, body).expect("serialize");

        let (parsed, parsed_body) =
            frontmatter::parse::<RunFrontmatter>(&serialized).expect("parse");
        assert_eq!(parsed.title.as_deref(), Some("Ship the thing"));
        assert_eq!(parsed.factory, "software");
        assert_eq!(parsed.active_station, "frame");
        assert_eq!(parsed.status, Status::Active);
        assert!(parsed_body.contains("Deliver the vertical slice."));
    }

    #[test]
    fn surface_defaults_to_none_and_roundtrips() {
        // Default frontmatter carries no surface, and serde omits the field.
        let fm = RunFrontmatter {
            factory: "software".into(),
            ..Default::default()
        };
        assert_eq!(fm.surface, None);
        let yaml = serde_yaml::to_string(&fm).expect("yaml");
        assert!(!yaml.contains("surface"), "absent surface must not serialize");

        // A classified surface serializes as its snake_case token and parses back.
        let fm = RunFrontmatter {
            factory: "software".into(),
            surface: Some(Surface::WebUi),
            ..Default::default()
        };
        let body = "# Web app\n";
        let serialized = frontmatter::serialize(&fm, body).expect("serialize");
        assert!(serialized.contains("surface: web_ui"));
        let (parsed, _) = frontmatter::parse::<RunFrontmatter>(&serialized).expect("parse");
        assert_eq!(parsed.surface, Some(Surface::WebUi));
    }

    #[test]
    fn surface_helpers_classify_verification_route() {
        // Visual surfaces route to the headless browser.
        for s in [Surface::WebUi, Surface::Desktop, Surface::Mobile] {
            assert!(s.is_visual() && !s.is_bench() && !s.is_terminal());
        }
        // Bench surfaces route to criterion + the load harness.
        for s in [Surface::Library, Surface::Api, Surface::Data] {
            assert!(s.is_bench() && !s.is_visual() && !s.is_terminal());
        }
        // Terminal surfaces route to a terminal/output snapshot.
        for s in [Surface::Tui, Surface::Cli] {
            assert!(s.is_terminal() && !s.is_visual() && !s.is_bench());
        }
    }

    #[test]
    fn surface_parse_tolerates_spellings() {
        assert_eq!(Surface::parse("web-ui"), Some(Surface::WebUi));
        assert_eq!(Surface::parse("WebUI"), Some(Surface::WebUi));
        assert_eq!(Surface::parse(" web_ui "), Some(Surface::WebUi));
        assert_eq!(Surface::parse("lib"), Some(Surface::Library));
        assert_eq!(Surface::parse("CLI"), Some(Surface::Cli));
        assert_eq!(Surface::parse("telepathy"), None);
        // as_str round-trips through serde tokens.
        for s in [
            Surface::Library,
            Surface::Api,
            Surface::WebUi,
            Surface::Tui,
            Surface::Cli,
            Surface::Desktop,
            Surface::Mobile,
            Surface::Data,
        ] {
            assert_eq!(Surface::parse(s.as_str()), Some(s));
            let json = serde_json::to_value(s).unwrap();
            assert_eq!(json, serde_json::json!(s.as_str()));
        }
    }

    #[test]
    fn run_surface_get_set_helpers() {
        let mut run = Run {
            slug: "r".into(),
            frontmatter: RunFrontmatter {
                factory: "software".into(),
                ..Default::default()
            },
            title: "R".into(),
            body: String::new(),
        };
        assert_eq!(run.surface(), None);
        run.set_surface(Surface::Cli);
        assert_eq!(run.surface(), Some(Surface::Cli));
        assert_eq!(run.frontmatter.surface, Some(Surface::Cli));
    }

    #[test]
    fn frontmatter_body_only_errors() {
        let err = frontmatter::parse::<RunFrontmatter>("no fence here").unwrap_err();
        assert!(matches!(err, CoreError::MissingFrontmatter));
    }

    #[test]
    fn frontmatter_split_extracts_body() {
        let doc = frontmatter::split("---\nfactory: software\n---\n# Title\nbody\n");
        assert!(doc.frontmatter.contains("factory: software"));
        assert!(doc.body.contains("# Title"));
        assert_eq!(frontmatter::first_heading(&doc.body).as_deref(), Some("Title"));
    }

    #[test]
    fn state_store_run_and_unit_roundtrip() {
        let tmp = tempfile::tempdir().expect("tmp");
        let store = StateStore::new(tmp.path());

        let run = Run {
            slug: "my-run".into(),
            frontmatter: RunFrontmatter {
                factory: "software".into(),
                active_station: "frame".into(),
                status: Status::Active,
                ..Default::default()
            },
            title: "My Run".into(),
            body: "# My Run\n".into(),
        };
        store.write_run(&run).expect("write run");

        let loaded = store.read_run("my-run").expect("read run");
        assert_eq!(loaded.frontmatter.factory, "software");
        assert_eq!(loaded.frontmatter.status, Status::Active);

        let u = unit("u1", Status::Pending, &[]);
        store.write_unit("my-run", &u).expect("write unit");
        let units = store.read_units("my-run").expect("read units");
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].slug, "u1");

        assert_eq!(store.list_runs().expect("list"), vec!["my-run".to_string()]);
    }

    #[test]
    fn lock_acquire_release_and_reacquire() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mgr = LockManager::new(tmp.path());

        let guard = mgr.acquire("run-frame", "test").expect("acquire");
        assert!(guard.path().exists());
        guard.release();

        // Releasing removes the dir, so a re-acquire succeeds immediately.
        let again = mgr.acquire("run-frame", "test").expect("reacquire");
        assert!(again.path().exists());
    }

    #[test]
    fn lock_with_lock_releases_after_closure() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mgr = LockManager::new(tmp.path());
        let result = mgr.with_lock("x", "t", || 42).expect("with_lock");
        assert_eq!(result, 42);
        // Lock dir is gone after the closure.
        assert!(!mgr.root().join("x").exists());
    }

    #[test]
    fn lock_stale_holder_is_reclaimed() {
        use std::fs;
        let tmp = tempfile::tempdir().expect("tmp");
        let mgr = LockManager::new(tmp.path());

        // Manually plant a wedged lock dir (no holder.json) with an old mtime
        // by creating it and asserting the stale path via a dead pid.
        let lock_dir = mgr.root().join("wedged");
        fs::create_dir_all(&lock_dir).expect("mkdir");
        // Write a holder with a pid that cannot be alive (pid 0 is never a
        // signalable user process; kill(0,0) targets the process group, so
        // use i32::MAX which is effectively never a live pid here).
        let holder = serde_json::json!({ "pid": i32::MAX, "at": 0u64, "tag": "dead" });
        fs::write(lock_dir.join("holder.json"), holder.to_string()).expect("write");

        // Backdate the dir mtime well past the stale threshold.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        filetime::set_file_mtime(&lock_dir, filetime::FileTime::from_system_time(old))
            .expect("backdate mtime");

        assert!(mgr.is_stale(&lock_dir), "old dir with dead holder is stale");

        // Acquiring the same name should steal the stale lock and succeed.
        let guard = mgr.acquire("wedged", "fresh").expect("steal stale");
        assert!(guard.path().exists());
    }

    #[test]
    fn dag_topological_order() {
        let units = vec![
            unit("a", Status::Completed, &[]),
            unit("b", Status::Pending, &["a"]),
            unit("c", Status::Pending, &["a"]),
            unit("d", Status::Pending, &["b", "c"]),
        ];
        let dag = Dag::build(&units);
        assert!(dag.unresolved.is_empty());
        let order = dag.topological_sort().expect("topo");
        let pos = |s: &str| order.iter().position(|x| x == s).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));

        let waves = dag.waves().expect("waves");
        assert_eq!(waves[&0], vec!["a".to_string()]);
        assert_eq!(waves[&1], vec!["b".to_string(), "c".to_string()]);
        assert_eq!(waves[&2], vec!["d".to_string()]);

        // a is completed, so b and c are ready; d is not (deps pending).
        let ready: Vec<&str> = dag.ready_units(&units).iter().map(|u| u.slug.as_str()).collect();
        assert_eq!(ready, vec!["b", "c"]);
    }

    #[test]
    fn dag_detects_cycle() {
        let units = vec![
            unit("a", Status::Pending, &["b"]),
            unit("b", Status::Pending, &["a"]),
        ];
        let dag = Dag::build(&units);
        let err = dag.topological_sort().unwrap_err();
        assert!(matches!(err, CoreError::CyclicDependency(_)));
    }

    #[test]
    fn dag_collects_unresolved_deps() {
        let units = vec![unit("a", Status::Pending, &["ghost"])];
        let dag = Dag::build(&units);
        assert_eq!(dag.unresolved.len(), 1);
        assert_eq!(dag.unresolved[0].dep, "ghost");
    }
}

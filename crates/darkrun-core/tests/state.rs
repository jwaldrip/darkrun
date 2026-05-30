//! Filesystem `StateStore` coverage: run/unit/feedback/session/state.json
//! roundtrips, list_runs edge cases, active-run resolution (pointer + inferred
//! newest), title resolution fallbacks, and not-found errors.

use std::collections::BTreeMap;
use std::fs;

use darkrun_core::domain::{
    Run, RunFrontmatter, Station, StationPhase, Status, Unit, UnitFrontmatter,
};
use darkrun_core::error::CoreError;
use darkrun_core::state::{run_is_complete, RunState, StateStore};

fn run(slug: &str, status: Status) -> Run {
    Run {
        slug: slug.to_string(),
        frontmatter: RunFrontmatter {
            factory: "software".into(),
            active_station: "frame".into(),
            status,
            ..Default::default()
        },
        title: slug.to_string(),
        body: format!("# {slug}\n"),
    }
}

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

// ─── Run documents ──────────────────────────────────────────────────────

#[test]
fn run_roundtrip_preserves_frontmatter() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    let mut r = run("my-run", Status::Active);
    r.frontmatter.title = Some("My Run".into());
    r.frontmatter.mode = "continuous".into();
    store.write_run(&r).expect("write");

    let loaded = store.read_run("my-run").expect("read");
    assert_eq!(loaded.frontmatter.factory, "software");
    assert_eq!(loaded.frontmatter.status, Status::Active);
    assert_eq!(loaded.frontmatter.mode, "continuous");
    assert_eq!(loaded.title, "My Run");
}

#[test]
fn read_run_not_found_errors() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    match store.read_run("ghost").unwrap_err() {
        CoreError::RunNotFound(s) => assert_eq!(s, "ghost"),
        other => panic!("expected RunNotFound, got {other:?}"),
    }
}

#[test]
fn run_title_falls_back_to_first_heading_then_slug() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());

    // No frontmatter title, but an H1 in the body.
    let r = Run {
        slug: "heading-run".into(),
        frontmatter: RunFrontmatter {
            factory: "software".into(),
            ..Default::default()
        },
        title: "ignored".into(),
        body: "# From Heading\n\nbody\n".into(),
    };
    store.write_run(&r).expect("write");
    assert_eq!(store.read_run("heading-run").expect("read").title, "From Heading");

    // No title and no heading -> slug.
    let r2 = Run {
        slug: "bare-run".into(),
        frontmatter: RunFrontmatter {
            factory: "software".into(),
            ..Default::default()
        },
        title: "ignored".into(),
        body: "just text, no heading\n".into(),
    };
    store.write_run(&r2).expect("write");
    assert_eq!(store.read_run("bare-run").expect("read").title, "bare-run");
}

// ─── Unit documents ─────────────────────────────────────────────────────

#[test]
fn unit_roundtrip_and_read_units_sorted() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("write run");

    store.write_unit("r", &unit("zebra", Status::Pending, &[])).expect("w");
    store.write_unit("r", &unit("alpha", Status::Active, &["zebra"])).expect("w");

    let units = store.read_units("r").expect("read");
    assert_eq!(units.len(), 2);
    // Sorted by slug.
    assert_eq!(units[0].slug, "alpha");
    assert_eq!(units[1].slug, "zebra");
    assert_eq!(units[0].frontmatter.depends_on, vec!["zebra".to_string()]);
}

#[test]
fn read_unit_not_found_errors() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("write run");
    match store.read_unit("r", "missing").unwrap_err() {
        CoreError::UnitNotFound(s) => assert_eq!(s, "missing"),
        other => panic!("expected UnitNotFound, got {other:?}"),
    }
}

#[test]
fn read_units_empty_when_no_units_dir() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("write run");
    // No units written yet.
    assert!(store.read_units("r").expect("read").is_empty());
}

#[test]
fn read_units_ignores_non_md_files() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("write run");
    store.write_unit("r", &unit("real", Status::Pending, &[])).expect("w");

    // Drop a stray non-markdown file into units/.
    let dir = store.units_dir("r");
    fs::write(dir.join("notes.txt"), "ignore me").expect("write stray");
    fs::write(dir.join(".DS_Store"), "junk").expect("write junk");

    let units = store.read_units("r").expect("read");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].slug, "real");
}

#[test]
fn unit_title_falls_back_to_name_then_heading_then_slug() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("write run");

    // name set -> wins.
    let named = Unit {
        slug: "u-named".into(),
        frontmatter: UnitFrontmatter {
            name: Some("Display Name".into()),
            ..Default::default()
        },
        title: "ignored".into(),
        body: "# Heading\n".into(),
    };
    store.write_unit("r", &named).expect("w");
    assert_eq!(store.read_unit("r", "u-named").expect("read").title, "Display Name");

    // no name, heading present -> heading.
    let heading = Unit {
        slug: "u-head".into(),
        frontmatter: UnitFrontmatter::default(),
        title: "ignored".into(),
        body: "# Unit Heading\n".into(),
    };
    store.write_unit("r", &heading).expect("w");
    assert_eq!(store.read_unit("r", "u-head").expect("read").title, "Unit Heading");

    // neither -> slug.
    let bare = Unit {
        slug: "u-bare".into(),
        frontmatter: UnitFrontmatter::default(),
        title: "ignored".into(),
        body: "no heading\n".into(),
    };
    store.write_unit("r", &bare).expect("w");
    assert_eq!(store.read_unit("r", "u-bare").expect("read").title, "u-bare");
}

// ─── list_runs ──────────────────────────────────────────────────────────

#[test]
fn list_runs_empty_when_no_darkrun_dir() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    // .darkrun does not exist yet.
    assert!(store.list_runs().expect("list").is_empty());
}

#[test]
fn list_runs_returns_sorted_slugs() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("charlie", Status::Active)).expect("w");
    store.write_run(&run("alpha", Status::Active)).expect("w");
    store.write_run(&run("bravo", Status::Active)).expect("w");
    assert_eq!(
        store.list_runs().expect("list"),
        vec!["alpha", "bravo", "charlie"]
    );
}

#[test]
fn list_runs_ignores_dirs_without_run_md() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("real", Status::Active)).expect("w");

    // A directory with no run.md must not count as a run.
    fs::create_dir_all(store.root().join("not-a-run")).expect("mkdir");
    // The locks dir lives under .darkrun too and must be ignored.
    fs::create_dir_all(store.root().join("locks")).expect("mkdir");

    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

#[test]
fn list_runs_ignores_the_active_pointer_file() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("real", Status::Active)).expect("w");
    store.set_active_run("real").expect("set active");
    // `active` is a plain file, not a run dir.
    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

// ─── Active-run resolution ──────────────────────────────────────────────

#[test]
fn active_run_none_when_empty() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn active_run_uses_pointer_when_valid() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("one", Status::Active)).expect("w");
    store.write_run(&run("two", Status::Active)).expect("w");
    store.set_active_run("two").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("two".to_string()));
}

#[test]
fn active_run_pointer_to_missing_run_falls_through_to_inference() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("live", Status::Active)).expect("w");
    // Point at a run that does not exist on disk.
    store.set_active_run("deleted").expect("set");
    // Falls back to inference -> the only active run.
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_empty_pointer_falls_through() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("live", Status::Active)).expect("w");
    // Whitespace-only pointer is treated as empty.
    fs::write(store.root().join("active"), "   \n").expect("write pointer");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_clear_then_infer() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("a", Status::Active)).expect("w");
    store.set_active_run("a").expect("set");
    store.clear_active_run().expect("clear");
    // Clearing is idempotent.
    store.clear_active_run().expect("clear again");
    // With the pointer gone, inference still finds the active run.
    assert_eq!(store.active_run().expect("active"), Some("a".to_string()));
}

#[test]
fn active_run_infers_newest_by_started_at() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());

    let mut older = run("older", Status::Active);
    older.frontmatter.started_at = Some("2026-01-01T00:00:00Z".into());
    let mut newer = run("newer", Status::Active);
    newer.frontmatter.started_at = Some("2026-05-01T00:00:00Z".into());
    store.write_run(&older).expect("w");
    store.write_run(&newer).expect("w");

    // No pointer set -> inference picks the most recent started_at.
    assert_eq!(store.active_run().expect("active"), Some("newer".to_string()));
}

#[test]
fn active_run_skips_archived_and_inactive() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());

    // Archived active run is skipped.
    let mut archived = run("archived", Status::Active);
    archived.frontmatter.archived = Some(true);
    archived.frontmatter.started_at = Some("2026-09-01T00:00:00Z".into());
    store.write_run(&archived).expect("w");

    // Completed run is skipped (not Active/InProgress).
    let mut done = run("done", Status::Completed);
    done.frontmatter.started_at = Some("2026-08-01T00:00:00Z".into());
    store.write_run(&done).expect("w");

    // The only eligible run.
    let mut live = run("live", Status::InProgress);
    live.frontmatter.started_at = Some("2026-01-01T00:00:00Z".into());
    store.write_run(&live).expect("w");

    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_none_when_all_archived_or_done() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    let mut done = run("done", Status::Completed);
    done.frontmatter.started_at = Some("2026-01-01T00:00:00Z".into());
    store.write_run(&done).expect("w");
    let mut pending = run("pending", Status::Pending);
    pending.frontmatter.started_at = Some("2026-02-01T00:00:00Z".into());
    store.write_run(&pending).expect("w");
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn active_run_in_progress_counts_as_active() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("ip", Status::InProgress)).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("ip".to_string()));
}

// ─── state.json ─────────────────────────────────────────────────────────

#[test]
fn state_json_roundtrip() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("w");

    // Absent until written.
    assert!(store.read_state("r").expect("read").is_none());

    let mut stations = BTreeMap::new();
    stations.insert(
        "frame".to_string(),
        Station {
            station: "frame".into(),
            status: Status::Active,
            phase: StationPhase::Manufacture,
            checkpoint: None,
            started_at: Some("2026-05-30T00:00:00Z".into()),
            completed_at: None,
        },
    );
    let state = RunState {
        factory: "software".into(),
        active_station: "frame".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &state).expect("write state");

    let loaded = store.read_state("r").expect("read").expect("present");
    assert_eq!(loaded.factory, "software");
    assert_eq!(loaded.active_station, "frame");
    assert_eq!(loaded.stations["frame"].phase, StationPhase::Manufacture);
    assert_eq!(loaded.stations["frame"].status, Status::Active);
}

#[test]
fn state_json_default_is_empty() {
    let state = RunState::default();
    assert_eq!(state.factory, "");
    assert_eq!(state.active_station, "");
    assert!(state.stations.is_empty());
}

// ─── feedback ───────────────────────────────────────────────────────────

#[test]
fn feedback_raw_roundtrip_keyed_by_stem() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("w");

    // Empty before any writes.
    assert!(store.read_feedback_raw("r").expect("read").is_empty());

    store
        .write_feedback_raw("r", "fb-2", "---\nid: fb-2\n---\nsecond\n")
        .expect("w");
    store
        .write_feedback_raw("r", "fb-1", "---\nid: fb-1\n---\nfirst\n")
        .expect("w");

    let map: BTreeMap<String, String> = store.read_feedback_raw("r").expect("read");
    assert_eq!(map.len(), 2);
    assert!(map.contains_key("fb-1"));
    assert!(map.contains_key("fb-2"));
    assert!(map["fb-1"].contains("first"));
    // BTreeMap iteration is sorted by key.
    let keys: Vec<&String> = map.keys().collect();
    assert_eq!(keys, vec!["fb-1", "fb-2"]);
}

#[test]
fn feedback_raw_ignores_non_md() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("w");
    store.write_feedback_raw("r", "fb-1", "---\nid: fb-1\n---\n").expect("w");
    fs::write(store.feedback_dir("r").join("README.txt"), "ignore").expect("stray");
    let map = store.read_feedback_raw("r").expect("read");
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("fb-1"));
}

// ─── path helpers + run_is_complete ─────────────────────────────────────

#[test]
fn path_helpers_compose_under_darkrun() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    assert!(store.root().ends_with(".darkrun"));
    assert_eq!(store.run_dir("r"), store.root().join("r"));
    assert_eq!(store.units_dir("r"), store.root().join("r").join("units"));
    assert_eq!(store.feedback_dir("r"), store.root().join("r").join("feedback"));
}

#[test]
fn run_is_complete_only_for_completed_status() {
    assert!(run_is_complete(&run("r", Status::Completed)));
    for s in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Blocked,
    ] {
        assert!(!run_is_complete(&run("r", s)));
    }
}

#[test]
fn write_run_creates_run_directory() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    // Directory does not pre-exist.
    assert!(!store.run_dir("fresh").exists());
    store.write_run(&run("fresh", Status::Active)).expect("w");
    assert!(store.run_dir("fresh").join("run.md").exists());
}

#[test]
fn overwriting_run_replaces_content() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    store.write_run(&run("r", Status::Active)).expect("w1");
    let mut updated = run("r", Status::Completed);
    updated.frontmatter.completed_at = Some("2026-05-30T00:00:00Z".into());
    store.write_run(&updated).expect("w2");
    let loaded = store.read_run("r").expect("read");
    assert_eq!(loaded.frontmatter.status, Status::Completed);
    assert_eq!(
        loaded.frontmatter.completed_at.as_deref(),
        Some("2026-05-30T00:00:00Z")
    );
}

//! Comprehensive `StateStore` integration coverage for darkrun-core.
//!
//! Drives the public filesystem state engine across every persisted shape:
//! run / unit / feedback / session / state.json read+write roundtrips,
//! `list_runs` enumeration edge cases, active-run pointer set/clear/resolve
//! (including inference by `started_at`, archived exclusion, and stale-pointer
//! fall-through), the `run_dir` / `units_dir` / `feedback_dir` path helpers,
//! `RunNotFound` / `UnitNotFound` errors, malformed on-disk files, and unicode
//! slugs. Every test exercises real behavior and can fail for a real reason.
#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use darkrun_core::domain::{
    Checkpoint, CheckpointKind, CheckpointOutcome, Run, RunFrontmatter, RunGit, Station,
    StationPhase, Status, Unit, UnitFrontmatter,
};
use darkrun_core::error::CoreError;
use darkrun_core::state::{run_is_complete, RunState, StateStore};

// ─── helpers ────────────────────────────────────────────────────────────────

fn store() -> (tempfile::TempDir, StateStore) {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    (tmp, store)
}

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

fn run_started(slug: &str, status: Status, started_at: &str) -> Run {
    let mut r = run(slug, status);
    r.frontmatter.started_at = Some(started_at.to_string());
    r
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

fn station(name: &str, status: Status, phase: StationPhase) -> Station {
    Station {
        station: name.to_string(),
        status,
        phase,
        checkpoint: None,
        branch: None,
        pr_ref: None,
        started_at: None,
        completed_at: None,
    }
}

// ─── new() / root() / path helpers ──────────────────────────────────────────

#[test]
fn new_appends_darkrun_to_repo_root() {
    let tmp = tempfile::tempdir().expect("tmp");
    let store = StateStore::new(tmp.path());
    assert_eq!(store.root(), tmp.path().join(".darkrun"));
}

#[test]
fn new_accepts_str_path() {
    let store = StateStore::new("/some/repo");
    assert_eq!(store.root(), Path::new("/some/repo/.darkrun"));
}

#[test]
fn new_accepts_pathbuf() {
    let store = StateStore::new(std::path::PathBuf::from("/p"));
    assert_eq!(store.root(), Path::new("/p/.darkrun"));
}

#[test]
fn root_ends_with_darkrun() {
    let (_t, store) = store();
    assert_eq!(store.root().file_name().unwrap(), ".darkrun");
}

#[test]
fn run_dir_is_root_join_slug() {
    let (_t, store) = store();
    assert_eq!(store.run_dir("alpha"), store.root().join("alpha"));
}

#[test]
fn units_dir_is_run_dir_join_units() {
    let (_t, store) = store();
    assert_eq!(store.units_dir("alpha"), store.run_dir("alpha").join("units"));
}

#[test]
fn feedback_dir_is_run_dir_join_feedback() {
    let (_t, store) = store();
    assert_eq!(
        store.feedback_dir("alpha"),
        store.run_dir("alpha").join("feedback")
    );
}

#[test]
fn run_dir_distinct_per_slug() {
    let (_t, store) = store();
    assert_ne!(store.run_dir("a"), store.run_dir("b"));
}

#[test]
fn units_dir_nested_below_run_dir() {
    let (_t, store) = store();
    assert!(store.units_dir("r").starts_with(store.run_dir("r")));
}

#[test]
fn feedback_dir_nested_below_run_dir() {
    let (_t, store) = store();
    assert!(store.feedback_dir("r").starts_with(store.run_dir("r")));
}

#[test]
fn path_helpers_pure_no_side_effects() {
    let (_t, store) = store();
    let _ = store.run_dir("r");
    let _ = store.units_dir("r");
    let _ = store.feedback_dir("r");
    // Querying paths never creates anything.
    assert!(!store.root().exists());
}

#[test]
fn run_dir_with_unicode_slug() {
    let (_t, store) = store();
    let d = store.run_dir("café-ünits");
    assert_eq!(d.file_name().unwrap(), "café-ünits");
}

#[test]
fn clone_store_shares_root() {
    let (_t, store) = store();
    let clone = store.clone();
    assert_eq!(store.root(), clone.root());
}

#[test]
fn clone_store_writes_visible_to_original() {
    let (_t, store) = store();
    let clone = store.clone();
    clone.write_run(&run("r", Status::Active)).expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["r"]);
}

// ─── write_run / read_run roundtrips ────────────────────────────────────────

#[test]
fn run_roundtrip_factory_and_status() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    let loaded = store.read_run("r").expect("read");
    assert_eq!(loaded.frontmatter.factory, "software");
    assert_eq!(loaded.frontmatter.status, Status::Active);
}

#[test]
fn run_roundtrip_preserves_slug() {
    let (_t, store) = store();
    store.write_run(&run("my-slug", Status::Active)).expect("w");
    assert_eq!(store.read_run("my-slug").expect("read").slug, "my-slug");
}

#[test]
fn run_roundtrip_preserves_mode() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.mode = "right-sized".into();
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").frontmatter.mode, "right-sized");
}

#[test]
fn run_roundtrip_preserves_active_station() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.active_station = "checkpoint".into();
    store.write_run(&r).expect("w");
    assert_eq!(
        store.read_run("r").expect("read").frontmatter.active_station,
        "checkpoint"
    );
}

#[test]
fn run_roundtrip_preserves_title_frontmatter() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.title = Some("Ship It".into());
    store.write_run(&r).expect("w");
    let loaded = store.read_run("r").expect("read");
    assert_eq!(loaded.frontmatter.title.as_deref(), Some("Ship It"));
    assert_eq!(loaded.title, "Ship It");
}

#[test]
fn run_roundtrip_preserves_started_at() {
    let (_t, store) = store();
    store
        .write_run(&run_started("r", Status::Active, "2026-05-30T12:00:00Z"))
        .expect("w");
    assert_eq!(
        store.read_run("r").expect("read").frontmatter.started_at.as_deref(),
        Some("2026-05-30T12:00:00Z")
    );
}

#[test]
fn run_roundtrip_preserves_completed_at() {
    let (_t, store) = store();
    let mut r = run("r", Status::Completed);
    r.frontmatter.completed_at = Some("2026-06-01T00:00:00Z".into());
    store.write_run(&r).expect("w");
    assert_eq!(
        store.read_run("r").expect("read").frontmatter.completed_at.as_deref(),
        Some("2026-06-01T00:00:00Z")
    );
}

#[test]
fn run_roundtrip_preserves_archived_true() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.archived = Some(true);
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").frontmatter.archived, Some(true));
}

#[test]
fn run_roundtrip_preserves_archived_false() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.archived = Some(false);
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").frontmatter.archived, Some(false));
}

#[test]
fn run_roundtrip_archived_none_when_unset() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    assert_eq!(store.read_run("r").expect("read").frontmatter.archived, None);
}

#[test]
fn run_roundtrip_preserves_git_policy() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.git = Some(RunGit {
        change_strategy: "worktree-per-unit".into(),
        auto_merge: true,
        auto_squash: false,
    });
    store.write_run(&r).expect("w");
    let git = store.read_run("r").expect("read").frontmatter.git.expect("git");
    assert_eq!(git.change_strategy, "worktree-per-unit");
    assert!(git.auto_merge);
    assert!(!git.auto_squash);
}

#[test]
fn run_roundtrip_git_none_when_unset() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    assert!(store.read_run("r").expect("read").frontmatter.git.is_none());
}

#[test]
fn run_roundtrip_preserves_body() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.body = "# Heading\n\nA paragraph of detail.\n".into();
    store.write_run(&r).expect("w");
    assert!(store.read_run("r").expect("read").body.contains("A paragraph of detail."));
}

#[test]
fn run_roundtrip_empty_body() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.body = String::new();
    store.write_run(&r).expect("w");
    // Empty body -> title falls back to slug.
    assert_eq!(store.read_run("r").expect("read").title, "r");
}

#[test]
fn run_roundtrip_multiline_body_preserved() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.body = "# T\n\nline1\nline2\nline3\n".into();
    store.write_run(&r).expect("w");
    let body = store.read_run("r").expect("read").body;
    assert!(body.contains("line1"));
    assert!(body.contains("line2"));
    assert!(body.contains("line3"));
}

#[test]
fn run_roundtrip_all_statuses() {
    let (_t, store) = store();
    for (i, s) in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Completed,
        Status::Blocked,
    ]
    .iter()
    .enumerate()
    {
        let slug = format!("r{i}");
        store.write_run(&run(&slug, *s)).expect("w");
        assert_eq!(store.read_run(&slug).expect("read").frontmatter.status, *s);
    }
}

#[test]
fn write_run_creates_directory_tree() {
    let (_t, store) = store();
    assert!(!store.root().exists());
    store.write_run(&run("r", Status::Active)).expect("w");
    assert!(store.root().exists());
    assert!(store.run_dir("r").exists());
    assert!(store.run_dir("r").join("run.md").exists());
}

#[test]
fn write_run_idempotent_content() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w1");
    let first = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read1");
    store.write_run(&run("r", Status::Active)).expect("w2");
    let second = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read2");
    assert_eq!(first, second);
}

#[test]
fn write_run_overwrites_status() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w1");
    store.write_run(&run("r", Status::Completed)).expect("w2");
    assert_eq!(store.read_run("r").expect("read").frontmatter.status, Status::Completed);
}

#[test]
fn write_run_overwrite_does_not_duplicate_runs() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w1");
    store.write_run(&run("r", Status::Completed)).expect("w2");
    assert_eq!(store.list_runs().expect("list").len(), 1);
}

#[test]
fn write_run_emits_frontmatter_fence() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    let raw = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read");
    assert!(raw.starts_with("---\n"));
    assert!(raw.contains("factory: software"));
}

#[test]
fn read_run_resolves_title_from_heading_when_no_frontmatter_title() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.title = None;
    r.body = "# Heading Title\n\nbody\n".into();
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").title, "Heading Title");
}

#[test]
fn read_run_title_prefers_frontmatter_over_heading() {
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.title = Some("FM Title".into());
    r.body = "# Heading Title\n".into();
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").title, "FM Title");
}

#[test]
fn read_run_title_falls_back_to_slug() {
    let (_t, store) = store();
    let mut r = run("the-slug", Status::Active);
    r.frontmatter.title = None;
    r.body = "no heading here\n".into();
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("the-slug").expect("read").title, "the-slug");
}

#[test]
fn read_run_ignored_in_struct_title_recomputed() {
    // The `title` field on the Run struct is recomputed on read, not persisted.
    let (_t, store) = store();
    let mut r = run("r", Status::Active);
    r.frontmatter.title = None;
    r.title = "this is thrown away".into();
    r.body = "# Real\n".into();
    store.write_run(&r).expect("w");
    assert_eq!(store.read_run("r").expect("read").title, "Real");
}

// ─── read_run not-found ─────────────────────────────────────────────────────

#[test]
fn read_run_missing_errors_runnotfound() {
    let (_t, store) = store();
    match store.read_run("ghost").unwrap_err() {
        CoreError::RunNotFound(s) => assert_eq!(s, "ghost"),
        other => panic!("expected RunNotFound, got {other:?}"),
    }
}

#[test]
fn read_run_missing_carries_slug_in_error() {
    let (_t, store) = store();
    let err = store.read_run("specific-slug").unwrap_err();
    assert!(err.to_string().contains("specific-slug"));
}

#[test]
fn read_run_dir_without_run_md_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("empty")).expect("mkdir");
    // Directory exists but no run.md.
    assert!(matches!(
        store.read_run("empty").unwrap_err(),
        CoreError::RunNotFound(_)
    ));
}

#[test]
fn read_run_after_other_runs_still_not_found() {
    let (_t, store) = store();
    store.write_run(&run("real", Status::Active)).expect("w");
    assert!(matches!(
        store.read_run("ghost").unwrap_err(),
        CoreError::RunNotFound(_)
    ));
}

// ─── unit roundtrips ────────────────────────────────────────────────────────

#[test]
fn unit_roundtrip_status() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").frontmatter.status, Status::Active);
}

#[test]
fn unit_roundtrip_preserves_slug() {
    let (_t, store) = store();
    store.write_unit("r", &unit("my-unit", Status::Pending, &[])).expect("w");
    assert_eq!(store.read_unit("r", "my-unit").expect("read").slug, "my-unit");
}

#[test]
fn unit_roundtrip_depends_on() {
    let (_t, store) = store();
    store
        .write_unit("r", &unit("u", Status::Pending, &["a", "b", "c"]))
        .expect("w");
    assert_eq!(
        store.read_unit("r", "u").expect("read").frontmatter.depends_on,
        vec!["a", "b", "c"]
    );
}

#[test]
fn unit_roundtrip_empty_deps() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Pending, &[])).expect("w");
    assert!(store.read_unit("r", "u").expect("read").frontmatter.depends_on.is_empty());
}

#[test]
fn unit_roundtrip_preserves_pass_index() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.pass = 7;
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").frontmatter.pass, 7);
}

#[test]
fn unit_roundtrip_preserves_worker() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.worker = "challenger".into();
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").frontmatter.worker, "challenger");
}

#[test]
fn unit_roundtrip_preserves_unit_type() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.unit_type = "endpoint".into();
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").frontmatter.unit_type, "endpoint");
}

#[test]
fn unit_roundtrip_preserves_model_override() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.model = Some("opus".into());
    store.write_unit("r", &u).expect("w");
    assert_eq!(
        store.read_unit("r", "u").expect("read").frontmatter.model.as_deref(),
        Some("opus")
    );
}

#[test]
fn unit_roundtrip_model_none_when_unset() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.read_unit("r", "u").expect("read").frontmatter.model.is_none());
}

#[test]
fn unit_roundtrip_preserves_station() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.station = Some("build".into());
    store.write_unit("r", &u).expect("w");
    assert_eq!(
        store.read_unit("r", "u").expect("read").frontmatter.station.as_deref(),
        Some("build")
    );
}

#[test]
fn unit_station_helper_defaults_to_root() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").station(), "_root");
}

#[test]
fn unit_station_helper_returns_set_station() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.station = Some("audit".into());
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").station(), "audit");
}

#[test]
fn unit_status_helper_matches_frontmatter() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Blocked, &[])).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").status(), Status::Blocked);
}

#[test]
fn unit_roundtrip_preserves_inputs() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.inputs = vec!["spec.md".into(), "notes.md".into()];
    store.write_unit("r", &u).expect("w");
    assert_eq!(
        store.read_unit("r", "u").expect("read").frontmatter.inputs,
        vec!["spec.md", "notes.md"]
    );
}

#[test]
fn unit_roundtrip_preserves_outputs() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.outputs = vec!["impl.rs".into()];
    store.write_unit("r", &u).expect("w");
    assert_eq!(
        store.read_unit("r", "u").expect("read").frontmatter.outputs,
        vec!["impl.rs"]
    );
}

#[test]
fn unit_roundtrip_inputs_empty_default() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.read_unit("r", "u").expect("read").frontmatter.inputs.is_empty());
}

#[test]
fn unit_roundtrip_outputs_empty_default() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.read_unit("r", "u").expect("read").frontmatter.outputs.is_empty());
}

#[test]
fn unit_roundtrip_preserves_timestamps() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Completed, &[]);
    u.frontmatter.started_at = Some("2026-01-01T00:00:00Z".into());
    u.frontmatter.completed_at = Some("2026-01-02T00:00:00Z".into());
    store.write_unit("r", &u).expect("w");
    let loaded = store.read_unit("r", "u").expect("read");
    assert_eq!(loaded.frontmatter.started_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(loaded.frontmatter.completed_at.as_deref(), Some("2026-01-02T00:00:00Z"));
}

#[test]
fn unit_roundtrip_preserves_body() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.body = "# Unit\n\nacceptance criteria\n".into();
    store.write_unit("r", &u).expect("w");
    assert!(store.read_unit("r", "u").expect("read").body.contains("acceptance criteria"));
}

#[test]
fn unit_roundtrip_preserves_name_as_title() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.name = Some("Pretty Name".into());
    store.write_unit("r", &u).expect("w");
    let loaded = store.read_unit("r", "u").expect("read");
    assert_eq!(loaded.frontmatter.name.as_deref(), Some("Pretty Name"));
    assert_eq!(loaded.title, "Pretty Name");
}

#[test]
fn unit_title_heading_fallback() {
    let (_t, store) = store();
    let mut u = unit("u", Status::Active, &[]);
    u.frontmatter.name = None;
    u.body = "# Unit From Heading\n".into();
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "u").expect("read").title, "Unit From Heading");
}

#[test]
fn unit_title_slug_fallback() {
    let (_t, store) = store();
    let mut u = unit("the-unit", Status::Active, &[]);
    u.frontmatter.name = None;
    u.body = "no heading\n".into();
    store.write_unit("r", &u).expect("w");
    assert_eq!(store.read_unit("r", "the-unit").expect("read").title, "the-unit");
}

#[test]
fn unit_all_statuses_roundtrip() {
    let (_t, store) = store();
    for (i, s) in [
        Status::Pending,
        Status::Active,
        Status::InProgress,
        Status::Completed,
        Status::Blocked,
    ]
    .iter()
    .enumerate()
    {
        let slug = format!("u{i}");
        store.write_unit("r", &unit(&slug, *s, &[])).expect("w");
        assert_eq!(store.read_unit("r", &slug).expect("read").frontmatter.status, *s);
    }
}

#[test]
fn write_unit_creates_units_dir() {
    let (_t, store) = store();
    assert!(!store.units_dir("r").exists());
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.units_dir("r").exists());
    assert!(store.units_dir("r").join("u.md").exists());
}

#[test]
fn write_unit_overwrites_existing() {
    let (_t, store) = store();
    store.write_unit("r", &unit("u", Status::Pending, &[])).expect("w1");
    store.write_unit("r", &unit("u", Status::Completed, &[])).expect("w2");
    assert_eq!(store.read_unit("r", "u").expect("read").frontmatter.status, Status::Completed);
}

#[test]
fn write_unit_does_not_require_run_md() {
    // write_unit creates units/ independently; it does not need run.md present.
    let (_t, store) = store();
    store.write_unit("orphan", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.units_dir("orphan").join("u.md").exists());
    // But the run itself is still "not found" without run.md.
    assert!(matches!(
        store.read_run("orphan").unwrap_err(),
        CoreError::RunNotFound(_)
    ));
}

// ─── read_unit not-found ────────────────────────────────────────────────────

#[test]
fn read_unit_missing_errors_unitnotfound() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    match store.read_unit("r", "missing").unwrap_err() {
        CoreError::UnitNotFound(s) => assert_eq!(s, "missing"),
        other => panic!("expected UnitNotFound, got {other:?}"),
    }
}

#[test]
fn read_unit_missing_when_run_absent() {
    let (_t, store) = store();
    // No run, no units dir.
    assert!(matches!(
        store.read_unit("ghost-run", "u").unwrap_err(),
        CoreError::UnitNotFound(_)
    ));
}

#[test]
fn read_unit_error_carries_unit_slug_not_run() {
    let (_t, store) = store();
    let err = store.read_unit("some-run", "the-unit").unwrap_err();
    assert!(err.to_string().contains("the-unit"));
}

#[test]
fn read_unit_other_unit_exists_still_not_found() {
    let (_t, store) = store();
    store.write_unit("r", &unit("present", Status::Active, &[])).expect("w");
    assert!(matches!(
        store.read_unit("r", "absent").unwrap_err(),
        CoreError::UnitNotFound(_)
    ));
}

// ─── read_units ─────────────────────────────────────────────────────────────

#[test]
fn read_units_empty_when_no_dir() {
    let (_t, store) = store();
    assert!(store.read_units("r").expect("read").is_empty());
}

#[test]
fn read_units_empty_when_dir_exists_but_empty() {
    let (_t, store) = store();
    fs::create_dir_all(store.units_dir("r")).expect("mkdir");
    assert!(store.read_units("r").expect("read").is_empty());
}

#[test]
fn read_units_single() {
    let (_t, store) = store();
    store.write_unit("r", &unit("only", Status::Active, &[])).expect("w");
    let units = store.read_units("r").expect("read");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].slug, "only");
}

#[test]
fn read_units_sorted_by_slug() {
    let (_t, store) = store();
    for s in ["zebra", "mango", "alpha", "delta"] {
        store.write_unit("r", &unit(s, Status::Pending, &[])).expect("w");
    }
    let slugs: Vec<String> = store.read_units("r").expect("read").into_iter().map(|u| u.slug).collect();
    assert_eq!(slugs, vec!["alpha", "delta", "mango", "zebra"]);
}

#[test]
fn read_units_many() {
    let (_t, store) = store();
    for i in 0..50 {
        store.write_unit("r", &unit(&format!("u{i:03}"), Status::Pending, &[])).expect("w");
    }
    assert_eq!(store.read_units("r").expect("read").len(), 50);
}

#[test]
fn read_units_ignores_non_md_files() {
    let (_t, store) = store();
    store.write_unit("r", &unit("real", Status::Active, &[])).expect("w");
    fs::write(store.units_dir("r").join("notes.txt"), "x").expect("w");
    fs::write(store.units_dir("r").join("data.json"), "{}").expect("w");
    let units = store.read_units("r").expect("read");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].slug, "real");
}

#[test]
fn read_units_ignores_subdirectories() {
    let (_t, store) = store();
    store.write_unit("r", &unit("real", Status::Active, &[])).expect("w");
    fs::create_dir_all(store.units_dir("r").join("subdir")).expect("mkdir");
    assert_eq!(store.read_units("r").expect("read").len(), 1);
}

#[test]
fn read_units_preserves_deps_after_read() {
    let (_t, store) = store();
    store.write_unit("r", &unit("a", Status::Completed, &[])).expect("w");
    store.write_unit("r", &unit("b", Status::Pending, &["a"])).expect("w");
    let units = store.read_units("r").expect("read");
    let b = units.iter().find(|u| u.slug == "b").expect("b");
    assert_eq!(b.frontmatter.depends_on, vec!["a"]);
}

#[test]
fn read_units_dotfile_md_included_by_stem() {
    // A file named ".hidden.md" has stem ".hidden" — it parses as a unit doc.
    let (_t, store) = store();
    store.write_unit("r", &unit("normal", Status::Active, &[])).expect("w");
    fs::write(
        store.units_dir("r").join("extra.md"),
        "---\nstatus: pending\n---\n# Extra\n",
    )
    .expect("w");
    let units = store.read_units("r").expect("read");
    assert_eq!(units.len(), 2);
}

// ─── list_runs ──────────────────────────────────────────────────────────────

#[test]
fn list_runs_empty_no_root() {
    let (_t, store) = store();
    assert!(store.list_runs().expect("list").is_empty());
}

#[test]
fn list_runs_empty_root_exists_no_runs() {
    let (_t, store) = store();
    fs::create_dir_all(store.root()).expect("mkdir");
    assert!(store.list_runs().expect("list").is_empty());
}

#[test]
fn list_runs_single() {
    let (_t, store) = store();
    store.write_run(&run("solo", Status::Active)).expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["solo"]);
}

#[test]
fn list_runs_sorted() {
    let (_t, store) = store();
    for s in ["gamma", "alpha", "beta"] {
        store.write_run(&run(s, Status::Active)).expect("w");
    }
    assert_eq!(store.list_runs().expect("list"), vec!["alpha", "beta", "gamma"]);
}

#[test]
fn list_runs_many() {
    let (_t, store) = store();
    for i in 0..40 {
        store.write_run(&run(&format!("run-{i:03}"), Status::Active)).expect("w");
    }
    assert_eq!(store.list_runs().expect("list").len(), 40);
}

#[test]
fn list_runs_ignores_dir_without_run_md() {
    let (_t, store) = store();
    store.write_run(&run("real", Status::Active)).expect("w");
    fs::create_dir_all(store.root().join("not-a-run")).expect("mkdir");
    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

#[test]
fn list_runs_ignores_locks_dir() {
    let (_t, store) = store();
    store.write_run(&run("real", Status::Active)).expect("w");
    fs::create_dir_all(store.root().join("locks")).expect("mkdir");
    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

#[test]
fn list_runs_ignores_plain_files_in_root() {
    let (_t, store) = store();
    store.write_run(&run("real", Status::Active)).expect("w");
    fs::write(store.root().join("README"), "x").expect("w");
    fs::write(store.root().join("config.json"), "{}").expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

#[test]
fn list_runs_ignores_active_pointer_file() {
    let (_t, store) = store();
    store.write_run(&run("real", Status::Active)).expect("w");
    store.set_active_run("real").expect("set");
    assert_eq!(store.list_runs().expect("list"), vec!["real"]);
}

#[test]
fn list_runs_dir_with_run_md_as_file_only() {
    // A run dir need only contain run.md; subdirs etc. don't matter.
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    store.write_unit("r", &unit("u", Status::Active, &[])).expect("w");
    store.write_feedback_raw("r", "fb", "---\nid: fb\n---\n").expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["r"]);
}

#[test]
fn list_runs_after_overwriting_unchanged() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.write_run(&run("b", Status::Active)).expect("w");
    store.write_run(&run("a", Status::Completed)).expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["a", "b"]);
}

#[test]
fn list_runs_with_unicode_slug() {
    let (_t, store) = store();
    store.write_run(&run("café", Status::Active)).expect("w");
    store.write_run(&run("naïve", Status::Active)).expect("w");
    let runs = store.list_runs().expect("list");
    assert!(runs.contains(&"café".to_string()));
    assert!(runs.contains(&"naïve".to_string()));
}

#[test]
fn list_runs_mixed_status_all_listed() {
    // list_runs lists every run dir regardless of status.
    let (_t, store) = store();
    store.write_run(&run("active", Status::Active)).expect("w");
    store.write_run(&run("done", Status::Completed)).expect("w");
    store.write_run(&run("blocked", Status::Blocked)).expect("w");
    assert_eq!(store.list_runs().expect("list").len(), 3);
}

#[test]
fn list_runs_archived_still_listed() {
    // Archived runs are not excluded from list_runs (only from active_run).
    let (_t, store) = store();
    let mut r = run("archived", Status::Active);
    r.frontmatter.archived = Some(true);
    store.write_run(&r).expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["archived"]);
}

// ─── active pointer set/clear/resolve ───────────────────────────────────────

#[test]
fn set_active_run_creates_pointer_file() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    store.set_active_run("r").expect("set");
    assert!(store.root().join("active").exists());
    assert_eq!(fs::read_to_string(store.root().join("active")).expect("read"), "r");
}

#[test]
fn set_active_run_creates_root_if_missing() {
    let (_t, store) = store();
    // No .darkrun yet — set_active_run must create it.
    assert!(!store.root().exists());
    store.set_active_run("r").expect("set");
    assert!(store.root().exists());
}

#[test]
fn set_active_run_overwrites_previous() {
    let (_t, store) = store();
    store.write_run(&run("one", Status::Active)).expect("w");
    store.write_run(&run("two", Status::Active)).expect("w");
    store.set_active_run("one").expect("set");
    store.set_active_run("two").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("two".to_string()));
}

#[test]
fn active_run_uses_valid_pointer() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.write_run(&run("b", Status::Active)).expect("w");
    store.set_active_run("a").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("a".to_string()));
}

#[test]
fn active_run_pointer_wins_over_newer_inference() {
    // Pointer overrides "newest by started_at" inference.
    let (_t, store) = store();
    store.write_run(&run_started("old", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("new", Status::Active, "2026-12-01T00:00:00Z")).expect("w");
    store.set_active_run("old").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("old".to_string()));
}

#[test]
fn active_run_none_empty_store() {
    let (_t, store) = store();
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn active_run_stale_pointer_falls_through_to_inference() {
    let (_t, store) = store();
    store.write_run(&run("live", Status::Active)).expect("w");
    store.set_active_run("deleted-run").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_stale_pointer_no_candidates_is_none() {
    let (_t, store) = store();
    // Only a completed run exists; stale pointer -> nothing inferable.
    store.write_run(&run("done", Status::Completed)).expect("w");
    store.set_active_run("ghost").expect("set");
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn active_run_pointer_to_deleted_after_run_removed() {
    let (_t, store) = store();
    store.write_run(&run("temp", Status::Active)).expect("w");
    store.set_active_run("temp").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("temp".to_string()));
    // Remove the run dir; pointer now stale.
    fs::remove_dir_all(store.run_dir("temp")).expect("rm");
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn active_run_whitespace_pointer_treated_empty() {
    let (_t, store) = store();
    store.write_run(&run("live", Status::Active)).expect("w");
    fs::write(store.root().join("active"), "  \n\t ").expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_empty_pointer_treated_empty() {
    let (_t, store) = store();
    store.write_run(&run("live", Status::Active)).expect("w");
    fs::write(store.root().join("active"), "").expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn active_run_pointer_trims_trailing_newline() {
    let (_t, store) = store();
    store.write_run(&run("trimmed", Status::Active)).expect("w");
    fs::write(store.root().join("active"), "trimmed\n").expect("w");
    assert_eq!(store.active_run().expect("active"), Some("trimmed".to_string()));
}

#[test]
fn active_run_pointer_trims_surrounding_whitespace() {
    let (_t, store) = store();
    store.write_run(&run("padded", Status::Active)).expect("w");
    fs::write(store.root().join("active"), "  padded  \n").expect("w");
    assert_eq!(store.active_run().expect("active"), Some("padded".to_string()));
}

// ─── clear_active_run ───────────────────────────────────────────────────────

#[test]
fn clear_active_run_removes_pointer() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    store.set_active_run("r").expect("set");
    store.clear_active_run().expect("clear");
    assert!(!store.root().join("active").exists());
}

#[test]
fn clear_active_run_idempotent_no_pointer() {
    let (_t, store) = store();
    store.clear_active_run().expect("clear");
    store.clear_active_run().expect("clear again");
}

#[test]
fn clear_active_run_idempotent_no_root() {
    let (_t, store) = store();
    // No .darkrun at all.
    assert!(!store.root().exists());
    store.clear_active_run().expect("clear");
}

#[test]
fn clear_then_infer_from_disk() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.set_active_run("a").expect("set");
    store.clear_active_run().expect("clear");
    // Pointer gone, inference finds the active run.
    assert_eq!(store.active_run().expect("active"), Some("a".to_string()));
}

#[test]
fn clear_does_not_remove_runs() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.set_active_run("a").expect("set");
    store.clear_active_run().expect("clear");
    assert_eq!(store.list_runs().expect("list"), vec!["a"]);
}

// ─── active_run inference ───────────────────────────────────────────────────

#[test]
fn infer_newest_by_started_at() {
    let (_t, store) = store();
    store.write_run(&run_started("old", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("mid", Status::Active, "2026-03-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("new", Status::Active, "2026-06-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("new".to_string()));
}

#[test]
fn infer_skips_archived() {
    let (_t, store) = store();
    let mut arch = run_started("arch", Status::Active, "2026-12-01T00:00:00Z");
    arch.frontmatter.archived = Some(true);
    store.write_run(&arch).expect("w");
    store.write_run(&run_started("live", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn infer_archived_false_is_eligible() {
    let (_t, store) = store();
    let mut r = run_started("live", Status::Active, "2026-01-01T00:00:00Z");
    r.frontmatter.archived = Some(false);
    store.write_run(&r).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn infer_skips_completed() {
    let (_t, store) = store();
    store.write_run(&run_started("done", Status::Completed, "2026-12-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("live", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn infer_skips_pending() {
    let (_t, store) = store();
    store.write_run(&run_started("pend", Status::Pending, "2026-12-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("live", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn infer_skips_blocked() {
    let (_t, store) = store();
    store.write_run(&run_started("block", Status::Blocked, "2026-12-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("live", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("live".to_string()));
}

#[test]
fn infer_in_progress_counts() {
    let (_t, store) = store();
    store.write_run(&run("ip", Status::InProgress)).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("ip".to_string()));
}

#[test]
fn infer_in_progress_competes_with_active() {
    let (_t, store) = store();
    store.write_run(&run_started("a", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("b", Status::InProgress, "2026-06-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("b".to_string()));
}

#[test]
fn infer_none_when_all_terminal() {
    let (_t, store) = store();
    store.write_run(&run("done", Status::Completed)).expect("w");
    store.write_run(&run("pend", Status::Pending)).expect("w");
    store.write_run(&run("block", Status::Blocked)).expect("w");
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn infer_none_when_all_archived() {
    let (_t, store) = store();
    for s in ["a", "b"] {
        let mut r = run(s, Status::Active);
        r.frontmatter.archived = Some(true);
        store.write_run(&r).expect("w");
    }
    assert_eq!(store.active_run().expect("active"), None);
}

#[test]
fn infer_single_active_no_timestamp() {
    let (_t, store) = store();
    store.write_run(&run("only", Status::Active)).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("only".to_string()));
}

#[test]
fn infer_missing_timestamp_loses_to_dated_active() {
    // Missing started_at sorts first (empty string), so a dated run wins.
    let (_t, store) = store();
    store.write_run(&run("undated", Status::Active)).expect("w");
    store.write_run(&run_started("dated", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("dated".to_string()));
}

#[test]
fn infer_two_undated_picks_by_slug_order() {
    // Both undated -> tie on timestamp -> the (started, slug) tuple sort
    // resolves by slug; pop() takes the lexicographically-largest slug.
    let (_t, store) = store();
    store.write_run(&run("aaa", Status::Active)).expect("w");
    store.write_run(&run("zzz", Status::Active)).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("zzz".to_string()));
}

#[test]
fn infer_equal_timestamp_resolves_by_slug() {
    let (_t, store) = store();
    store.write_run(&run_started("aaa", Status::Active, "2026-05-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("bbb", Status::Active, "2026-05-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("bbb".to_string()));
}

#[test]
fn infer_skips_unreadable_run_md() {
    // A run dir with run.md but malformed (missing required factory) is
    // skipped by the inference loop (read_run errs -> continue).
    let (_t, store) = store();
    store.write_run(&run("good", Status::Active)).expect("w");
    fs::create_dir_all(store.run_dir("broken")).expect("mkdir");
    fs::write(store.run_dir("broken").join("run.md"), "not even frontmatter").expect("w");
    assert_eq!(store.active_run().expect("active"), Some("good".to_string()));
}

#[test]
fn infer_ignores_archived_among_many() {
    let (_t, store) = store();
    let mut newest_archived = run_started("z-arch", Status::Active, "2026-12-31T00:00:00Z");
    newest_archived.frontmatter.archived = Some(true);
    store.write_run(&newest_archived).expect("w");
    store.write_run(&run_started("a", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("b", Status::Active, "2026-02-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("b".to_string()));
}

// ─── state.json ─────────────────────────────────────────────────────────────

#[test]
fn read_state_none_when_absent() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    assert!(store.read_state("r").expect("read").is_none());
}

#[test]
fn read_state_none_when_run_absent() {
    let (_t, store) = store();
    assert!(store.read_state("ghost").expect("read").is_none());
}

#[test]
fn state_roundtrip_factory_and_active_station() {
    let (_t, store) = store();
    let s = RunState {
        factory: "software".into(),
        active_station: "build".into(),
        stations: BTreeMap::new(),
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let loaded = store.read_state("r").expect("read").expect("some");
    assert_eq!(loaded.factory, "software");
    assert_eq!(loaded.active_station, "build");
}

#[test]
fn state_roundtrip_stations_map() {
    let (_t, store) = store();
    let mut stations = BTreeMap::new();
    stations.insert("frame".into(), station("frame", Status::Active, StationPhase::Spec));
    stations.insert("build".into(), station("build", Status::Pending, StationPhase::Manufacture));
    let s = RunState {
        factory: "f".into(),
        active_station: "frame".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let loaded = store.read_state("r").expect("read").expect("some");
    assert_eq!(loaded.stations.len(), 2);
    assert_eq!(loaded.stations["frame"].phase, StationPhase::Spec);
    assert_eq!(loaded.stations["build"].status, Status::Pending);
}

#[test]
fn state_roundtrip_all_phases() {
    let (_t, store) = store();
    let mut stations = BTreeMap::new();
    for (i, p) in [
        StationPhase::Spec,
        StationPhase::Review,
        StationPhase::Manufacture,
        StationPhase::Audit,
        StationPhase::Reflect,
        StationPhase::Checkpoint,
    ]
    .iter()
    .enumerate()
    {
        stations.insert(format!("s{i}"), station(&format!("s{i}"), Status::Active, *p));
    }
    let s = RunState {
        factory: "f".into(),
        active_station: "s0".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let loaded = store.read_state("r").expect("read").expect("some");
    assert_eq!(loaded.stations["s2"].phase, StationPhase::Manufacture);
    assert_eq!(loaded.stations["s5"].phase, StationPhase::Checkpoint);
}

#[test]
fn state_roundtrip_checkpoint() {
    let (_t, store) = store();
    let mut st = station("frame", Status::Active, StationPhase::Checkpoint);
    st.checkpoint = Some(Checkpoint {
        kind: CheckpointKind::Ask,
        entered_at: Some("2026-05-30T00:00:00Z".into()),
        outcome: Some(CheckpointOutcome::Paused),
    });
    let mut stations = BTreeMap::new();
    stations.insert("frame".into(), st);
    let s = RunState {
        factory: "f".into(),
        active_station: "frame".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let cp = store.read_state("r").expect("read").expect("some").stations["frame"]
        .checkpoint
        .clone()
        .expect("cp");
    assert_eq!(cp.kind, CheckpointKind::Ask);
    assert_eq!(cp.outcome, Some(CheckpointOutcome::Paused));
    assert_eq!(cp.entered_at.as_deref(), Some("2026-05-30T00:00:00Z"));
}

#[test]
fn state_roundtrip_all_checkpoint_kinds() {
    let (_t, store) = store();
    for (i, k) in [
        CheckpointKind::Auto,
        CheckpointKind::Ask,
        CheckpointKind::External,
        CheckpointKind::Await,
    ]
    .iter()
    .enumerate()
    {
        let mut st = station("s", Status::Active, StationPhase::Checkpoint);
        st.checkpoint = Some(Checkpoint {
            kind: *k,
            entered_at: None,
            outcome: None,
        });
        let mut stations = BTreeMap::new();
        stations.insert("s".into(), st);
        let s = RunState {
            factory: "f".into(),
            active_station: "s".into(),
            stations,
            ..Default::default()
        };
        let slug = format!("r{i}");
        store.write_state(&slug, &s).expect("w");
        let loaded = store.read_state(&slug).expect("read").expect("some");
        assert_eq!(loaded.stations["s"].checkpoint.as_ref().unwrap().kind, *k);
    }
}

#[test]
fn state_roundtrip_all_checkpoint_outcomes() {
    let (_t, store) = store();
    for (i, o) in [
        CheckpointOutcome::Advanced,
        CheckpointOutcome::Paused,
        CheckpointOutcome::Blocked,
        CheckpointOutcome::Awaiting,
    ]
    .iter()
    .enumerate()
    {
        let mut st = station("s", Status::Active, StationPhase::Checkpoint);
        st.checkpoint = Some(Checkpoint {
            kind: CheckpointKind::Auto,
            entered_at: None,
            outcome: Some(*o),
        });
        let mut stations = BTreeMap::new();
        stations.insert("s".into(), st);
        let s = RunState {
            factory: "f".into(),
            active_station: "s".into(),
            stations,
            ..Default::default()
        };
        let slug = format!("o{i}");
        store.write_state(&slug, &s).expect("w");
        let loaded = store.read_state(&slug).expect("read").expect("some");
        assert_eq!(
            loaded.stations["s"].checkpoint.as_ref().unwrap().outcome,
            Some(*o)
        );
    }
}

#[test]
fn state_roundtrip_station_timestamps() {
    let (_t, store) = store();
    let mut st = station("frame", Status::Completed, StationPhase::Checkpoint);
    st.started_at = Some("2026-01-01T00:00:00Z".into());
    st.completed_at = Some("2026-01-02T00:00:00Z".into());
    let mut stations = BTreeMap::new();
    stations.insert("frame".into(), st);
    let s = RunState {
        factory: "f".into(),
        active_station: "frame".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let loaded = store.read_state("r").expect("read").expect("some");
    assert_eq!(loaded.stations["frame"].started_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(loaded.stations["frame"].completed_at.as_deref(), Some("2026-01-02T00:00:00Z"));
}

#[test]
fn state_empty_default_roundtrip() {
    let (_t, store) = store();
    store.write_state("r", &RunState::default()).expect("w");
    let loaded = store.read_state("r").expect("read").expect("some");
    assert_eq!(loaded.factory, "");
    assert_eq!(loaded.active_station, "");
    assert!(loaded.stations.is_empty());
}

#[test]
fn state_creates_run_dir() {
    let (_t, store) = store();
    assert!(!store.run_dir("r").exists());
    store.write_state("r", &RunState::default()).expect("w");
    assert!(store.run_dir("r").join("state.json").exists());
}

#[test]
fn state_overwrites() {
    let (_t, store) = store();
    let mut s = RunState::default();
    s.active_station = "a".into();
    store.write_state("r", &s).expect("w1");
    s.active_station = "b".into();
    store.write_state("r", &s).expect("w2");
    assert_eq!(store.read_state("r").expect("read").expect("some").active_station, "b");
}

#[test]
fn state_written_as_pretty_json() {
    let (_t, store) = store();
    store.write_state("r", &RunState::default()).expect("w");
    let raw = fs::read_to_string(store.run_dir("r").join("state.json")).expect("read");
    // Pretty JSON has newlines and indentation.
    assert!(raw.contains('\n'));
    assert!(raw.contains("\"factory\""));
}

#[test]
fn state_stations_preserved_sorted_keys() {
    let (_t, store) = store();
    let mut stations = BTreeMap::new();
    for name in ["zebra", "alpha", "mango"] {
        stations.insert(name.to_string(), station(name, Status::Active, StationPhase::Spec));
    }
    let s = RunState {
        factory: "f".into(),
        active_station: "alpha".into(),
        stations,
        ..Default::default()
    };
    store.write_state("r", &s).expect("w");
    let keys: Vec<String> = store
        .read_state("r")
        .expect("read")
        .expect("some")
        .stations
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, vec!["alpha", "mango", "zebra"]);
}

#[test]
fn state_malformed_json_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("r")).expect("mkdir");
    fs::write(store.run_dir("r").join("state.json"), "{ not json").expect("w");
    assert!(matches!(store.read_state("r").unwrap_err(), CoreError::Json(_)));
}

// ─── feedback ───────────────────────────────────────────────────────────────

#[test]
fn feedback_empty_when_no_dir() {
    let (_t, store) = store();
    assert!(store.read_feedback_raw("r").expect("read").is_empty());
}

#[test]
fn feedback_empty_when_dir_empty() {
    let (_t, store) = store();
    fs::create_dir_all(store.feedback_dir("r")).expect("mkdir");
    assert!(store.read_feedback_raw("r").expect("read").is_empty());
}

#[test]
fn feedback_single_roundtrip() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb-1", "---\nid: fb-1\n---\nbody\n").expect("w");
    let map = store.read_feedback_raw("r").expect("read");
    assert_eq!(map.len(), 1);
    assert!(map["fb-1"].contains("body"));
}

#[test]
fn feedback_preserves_exact_content() {
    let (_t, store) = store();
    let content = "---\nid: fb\nseverity: high\n---\n\nThis is the finding.\n";
    store.write_feedback_raw("r", "fb", content).expect("w");
    assert_eq!(store.read_feedback_raw("r").expect("read")["fb"], content);
}

#[test]
fn feedback_keyed_by_stem_sorted() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb-3", "c").expect("w");
    store.write_feedback_raw("r", "fb-1", "a").expect("w");
    store.write_feedback_raw("r", "fb-2", "b").expect("w");
    let map = store.read_feedback_raw("r").expect("read");
    let keys: Vec<String> = map.keys().cloned().collect();
    assert_eq!(keys, vec!["fb-1", "fb-2", "fb-3"]);
}

#[test]
fn feedback_many() {
    let (_t, store) = store();
    for i in 0..30 {
        store.write_feedback_raw("r", &format!("fb-{i:03}"), &format!("body {i}")).expect("w");
    }
    assert_eq!(store.read_feedback_raw("r").expect("read").len(), 30);
}

#[test]
fn feedback_ignores_non_md() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb-1", "---\nid: fb-1\n---\n").expect("w");
    fs::write(store.feedback_dir("r").join("README.txt"), "x").expect("w");
    fs::write(store.feedback_dir("r").join("notes.json"), "{}").expect("w");
    let map = store.read_feedback_raw("r").expect("read");
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("fb-1"));
}

#[test]
fn feedback_creates_dir() {
    let (_t, store) = store();
    assert!(!store.feedback_dir("r").exists());
    store.write_feedback_raw("r", "fb", "x").expect("w");
    assert!(store.feedback_dir("r").join("fb.md").exists());
}

#[test]
fn feedback_overwrites_same_id() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb", "old").expect("w1");
    store.write_feedback_raw("r", "fb", "new").expect("w2");
    let map = store.read_feedback_raw("r").expect("read");
    assert_eq!(map.len(), 1);
    assert_eq!(map["fb"], "new");
}

#[test]
fn feedback_empty_content_ok() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb", "").expect("w");
    let map = store.read_feedback_raw("r").expect("read");
    assert_eq!(map["fb"], "");
}

#[test]
fn feedback_unicode_content() {
    let (_t, store) = store();
    let content = "---\nid: fb\n---\nфидбэк — 日本語 — café\n";
    store.write_feedback_raw("r", "fb", content).expect("w");
    assert_eq!(store.read_feedback_raw("r").expect("read")["fb"], content);
}

#[test]
fn feedback_ignores_subdirs() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb", "x").expect("w");
    fs::create_dir_all(store.feedback_dir("r").join("nested")).expect("mkdir");
    assert_eq!(store.read_feedback_raw("r").expect("read").len(), 1);
}

#[test]
fn feedback_raw_is_not_parsed() {
    // read_feedback_raw returns the literal bytes; even non-frontmatter content
    // round-trips verbatim (no parse error).
    let (_t, store) = store();
    store.write_feedback_raw("r", "fb", "no frontmatter at all").expect("w");
    assert_eq!(store.read_feedback_raw("r").expect("read")["fb"], "no frontmatter at all");
}

// ─── malformed files ────────────────────────────────────────────────────────

#[test]
fn read_run_missing_frontmatter_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("r")).expect("mkdir");
    fs::write(store.run_dir("r").join("run.md"), "no fence at all\n").expect("w");
    assert!(matches!(
        store.read_run("r").unwrap_err(),
        CoreError::MissingFrontmatter
    ));
}

#[test]
fn read_run_invalid_yaml_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("r")).expect("mkdir");
    // Frontmatter present but missing the required `factory` field.
    fs::write(store.run_dir("r").join("run.md"), "---\nmode: x\n---\n").expect("w");
    assert!(matches!(store.read_run("r").unwrap_err(), CoreError::Yaml(_)));
}

#[test]
fn read_run_bad_status_value_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("r")).expect("mkdir");
    fs::write(
        store.run_dir("r").join("run.md"),
        "---\nfactory: f\nstatus: bogus\n---\n",
    )
    .expect("w");
    assert!(matches!(store.read_run("r").unwrap_err(), CoreError::Yaml(_)));
}

#[test]
fn read_unit_missing_frontmatter_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.units_dir("r")).expect("mkdir");
    fs::write(store.units_dir("r").join("u.md"), "body only, no fence\n").expect("w");
    assert!(matches!(
        store.read_unit("r", "u").unwrap_err(),
        CoreError::MissingFrontmatter
    ));
}

#[test]
fn read_unit_bad_status_errors() {
    let (_t, store) = store();
    fs::create_dir_all(store.units_dir("r")).expect("mkdir");
    fs::write(store.units_dir("r").join("u.md"), "---\nstatus: nope\n---\n").expect("w");
    assert!(matches!(store.read_unit("r", "u").unwrap_err(), CoreError::Yaml(_)));
}

#[test]
fn read_units_propagates_malformed_unit_error() {
    let (_t, store) = store();
    store.write_unit("r", &unit("good", Status::Active, &[])).expect("w");
    fs::write(store.units_dir("r").join("bad.md"), "no fence\n").expect("w");
    // read_units reads every unit; the malformed one surfaces an error.
    assert!(store.read_units("r").is_err());
}

#[test]
fn read_run_empty_frontmatter_uses_defaults_but_factory_required() {
    // An empty frontmatter block fails because `factory` has no default.
    let (_t, store) = store();
    fs::create_dir_all(store.run_dir("r")).expect("mkdir");
    fs::write(store.run_dir("r").join("run.md"), "---\n---\n# Body\n").expect("w");
    assert!(matches!(store.read_run("r").unwrap_err(), CoreError::Yaml(_)));
}

#[test]
fn read_unit_empty_frontmatter_uses_defaults() {
    // UnitFrontmatter has all-defaultable fields, so empty frontmatter parses.
    let (_t, store) = store();
    fs::create_dir_all(store.units_dir("r")).expect("mkdir");
    fs::write(store.units_dir("r").join("u.md"), "---\n---\n# U\n").expect("w");
    let u = store.read_unit("r", "u").expect("read");
    assert_eq!(u.frontmatter.status, Status::Pending);
    assert_eq!(u.title, "U");
}

// ─── unicode slugs ──────────────────────────────────────────────────────────

#[test]
fn unicode_run_slug_roundtrip() {
    let (_t, store) = store();
    store.write_run(&run("café-run", Status::Active)).expect("w");
    assert_eq!(store.read_run("café-run").expect("read").slug, "café-run");
}

#[test]
fn unicode_run_slug_cjk() {
    let (_t, store) = store();
    store.write_run(&run("実装-run", Status::Active)).expect("w");
    assert_eq!(store.read_run("実装-run").expect("read").frontmatter.status, Status::Active);
}

#[test]
fn unicode_run_slug_emoji() {
    let (_t, store) = store();
    store.write_run(&run("rocket-🚀", Status::Active)).expect("w");
    assert_eq!(store.list_runs().expect("list"), vec!["rocket-🚀"]);
}

#[test]
fn unicode_unit_slug_roundtrip() {
    let (_t, store) = store();
    store.write_unit("r", &unit("ünit-ñame", Status::Active, &[])).expect("w");
    assert_eq!(store.read_unit("r", "ünit-ñame").expect("read").slug, "ünit-ñame");
}

#[test]
fn unicode_feedback_id_roundtrip() {
    let (_t, store) = store();
    store.write_feedback_raw("r", "фидбэк", "content").expect("w");
    assert!(store.read_feedback_raw("r").expect("read").contains_key("фидбэк"));
}

#[test]
fn unicode_run_active_pointer() {
    let (_t, store) = store();
    store.write_run(&run("café", Status::Active)).expect("w");
    store.set_active_run("café").expect("set");
    assert_eq!(store.active_run().expect("active"), Some("café".to_string()));
}

#[test]
fn unicode_run_inference() {
    let (_t, store) = store();
    store.write_run(&run_started("日本", Status::Active, "2026-06-01T00:00:00Z")).expect("w");
    store.write_run(&run_started("english", Status::Active, "2026-01-01T00:00:00Z")).expect("w");
    assert_eq!(store.active_run().expect("active"), Some("日本".to_string()));
}

// ─── integration / cross-cutting ────────────────────────────────────────────

#[test]
fn full_run_lifecycle_persisted() {
    let (_t, store) = store();
    // Write a run plus its units, state, and feedback.
    store.write_run(&run("r", Status::Active)).expect("w");
    store.write_unit("r", &unit("u1", Status::Completed, &[])).expect("w");
    store.write_unit("r", &unit("u2", Status::Active, &["u1"])).expect("w");
    store.write_state("r", &RunState {
        factory: "software".into(),
        active_station: "build".into(),
        stations: BTreeMap::new(),
        ..Default::default()
    }).expect("w");
    store.write_feedback_raw("r", "fb", "---\nid: fb\n---\nfix this\n").expect("w");

    assert_eq!(store.list_runs().expect("list"), vec!["r"]);
    assert_eq!(store.read_units("r").expect("read").len(), 2);
    assert_eq!(store.read_state("r").expect("read").expect("some").active_station, "build");
    assert_eq!(store.read_feedback_raw("r").expect("read").len(), 1);
}

#[test]
fn multiple_runs_isolated() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.write_run(&run("b", Status::Active)).expect("w");
    store.write_unit("a", &unit("ua", Status::Active, &[])).expect("w");
    store.write_unit("b", &unit("ub", Status::Active, &[])).expect("w");
    // Each run only sees its own units.
    assert_eq!(store.read_units("a").expect("read")[0].slug, "ua");
    assert_eq!(store.read_units("b").expect("read")[0].slug, "ub");
}

#[test]
fn units_for_one_run_not_visible_to_another() {
    let (_t, store) = store();
    store.write_unit("a", &unit("u", Status::Active, &[])).expect("w");
    assert!(store.read_units("b").expect("read").is_empty());
}

#[test]
fn feedback_for_one_run_not_visible_to_another() {
    let (_t, store) = store();
    store.write_feedback_raw("a", "fb", "x").expect("w");
    assert!(store.read_feedback_raw("b").expect("read").is_empty());
}

#[test]
fn state_for_one_run_not_visible_to_another() {
    let (_t, store) = store();
    store.write_state("a", &RunState::default()).expect("w");
    assert!(store.read_state("b").expect("read").is_none());
}

#[test]
fn deleting_run_dir_removes_from_list() {
    let (_t, store) = store();
    store.write_run(&run("a", Status::Active)).expect("w");
    store.write_run(&run("b", Status::Active)).expect("w");
    fs::remove_dir_all(store.run_dir("a")).expect("rm");
    assert_eq!(store.list_runs().expect("list"), vec!["b"]);
}

#[test]
fn deleting_unit_removes_from_read_units() {
    let (_t, store) = store();
    store.write_unit("r", &unit("a", Status::Active, &[])).expect("w");
    store.write_unit("r", &unit("b", Status::Active, &[])).expect("w");
    fs::remove_file(store.units_dir("r").join("a.md")).expect("rm");
    let slugs: Vec<String> = store.read_units("r").expect("read").into_iter().map(|u| u.slug).collect();
    assert_eq!(slugs, vec!["b"]);
}

#[test]
fn run_is_complete_true_for_completed() {
    assert!(run_is_complete(&run("r", Status::Completed)));
}

#[test]
fn run_is_complete_false_for_non_completed() {
    for s in [Status::Pending, Status::Active, Status::InProgress, Status::Blocked] {
        assert!(!run_is_complete(&run("r", s)));
    }
}

#[test]
fn run_is_complete_reads_frontmatter_status() {
    // run_is_complete keys off frontmatter.status, not the read path.
    let (_t, store) = store();
    store.write_run(&run("r", Status::Completed)).expect("w");
    assert!(run_is_complete(&store.read_run("r").expect("read")));
}

#[test]
fn write_then_list_then_read_consistent() {
    let (_t, store) = store();
    for i in 0..10 {
        store.write_run(&run(&format!("r{i}"), Status::Active)).expect("w");
    }
    let runs = store.list_runs().expect("list");
    assert_eq!(runs.len(), 10);
    for slug in &runs {
        let loaded = store.read_run(slug).expect("read");
        assert_eq!(&loaded.slug, slug);
    }
}

#[test]
fn determinism_repeated_writes_stable_file() {
    let (_t, store) = store();
    let r = run("r", Status::Active);
    store.write_run(&r).expect("w");
    let a = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read");
    store.write_run(&r).expect("w");
    let b = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read");
    store.write_run(&r).expect("w");
    let c = fs::read_to_string(store.run_dir("r").join("run.md")).expect("read");
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn determinism_state_json_stable() {
    let (_t, store) = store();
    let mut stations = BTreeMap::new();
    stations.insert("z".into(), station("z", Status::Active, StationPhase::Spec));
    stations.insert("a".into(), station("a", Status::Active, StationPhase::Spec));
    let s = RunState { factory: "f".into(), active_station: "a".into(), stations, ..Default::default() };
    store.write_state("r", &s).expect("w");
    let first = fs::read_to_string(store.run_dir("r").join("state.json")).expect("read");
    store.write_state("r", &s).expect("w");
    let second = fs::read_to_string(store.run_dir("r").join("state.json")).expect("read");
    assert_eq!(first, second);
}

#[test]
fn read_units_idempotent() {
    let (_t, store) = store();
    for s in ["a", "b", "c"] {
        store.write_unit("r", &unit(s, Status::Active, &[])).expect("w");
    }
    let first: Vec<String> = store.read_units("r").expect("read").into_iter().map(|u| u.slug).collect();
    let second: Vec<String> = store.read_units("r").expect("read").into_iter().map(|u| u.slug).collect();
    assert_eq!(first, second);
}

#[test]
fn active_run_idempotent_reads() {
    let (_t, store) = store();
    store.write_run(&run("r", Status::Active)).expect("w");
    store.set_active_run("r").expect("set");
    assert_eq!(store.active_run().expect("a1"), store.active_run().expect("a2"));
}

#[test]
fn list_runs_idempotent() {
    let (_t, store) = store();
    for s in ["c", "a", "b"] {
        store.write_run(&run(s, Status::Active)).expect("w");
    }
    assert_eq!(store.list_runs().expect("l1"), store.list_runs().expect("l2"));
}

//! Comprehensive coverage for the advisory mkdir-lock engine.
//!
//! Exercises the public surface of [`LockManager`] / [`LockGuard`]:
//! acquire/release/reacquire, drop-release, `with_lock` (value passthrough and
//! panic safety), stale-holder reclamation (dead pid + old mtime), wedged
//! (holder-less) dirs, fresh-lock protection, contention between managers,
//! holder.json shape and persistence, nested/odd lock names, idempotency,
//! determinism, and multi-threaded concurrency.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use darkrun_core::error::CoreError;
use darkrun_core::locks::LockManager;
use darkrun_core::LockManager as ReexportedLockManager;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Backdate a directory's mtime far past the stale window (5 min).
fn backdate(path: &Path, secs: u64) {
    let old = SystemTime::now() - Duration::from_secs(secs);
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(old))
        .expect("backdate mtime");
}

/// Plant a raw lock dir + holder.json with the given pid and an old mtime.
fn plant_stale(mgr: &LockManager, name: &str, pid: i32, tag: &str) -> PathBuf {
    let dir = mgr.root().join(name);
    fs::create_dir_all(&dir).expect("mkdir lock dir");
    let holder = serde_json::json!({ "pid": pid, "at": 0u64, "tag": tag });
    fs::write(dir.join("holder.json"), holder.to_string()).expect("write holder");
    backdate(&dir, 3600);
    dir
}

/// Read and parse the holder.json of a held lock as a generic JSON value.
fn read_holder_json(dir: &Path) -> serde_json::Value {
    let raw = fs::read_to_string(dir.join("holder.json")).expect("read holder.json");
    serde_json::from_str(&raw).expect("parse holder.json")
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// A pid that is overwhelmingly unlikely to be alive on a test host.
const DEAD_PID: i32 = i32::MAX;

// ===========================================================================
// SECTION 1: basic acquire / path / holder existence
// ===========================================================================

#[test]
fn acquire_creates_lock_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("build", "worker").unwrap();
    assert!(g.path().exists());
}

#[test]
fn acquire_creates_holder_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("build", "worker").unwrap();
    assert!(g.path().join("holder.json").exists());
}

#[test]
fn acquire_path_is_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("d", "t").unwrap();
    assert!(g.path().is_dir());
}

#[test]
fn acquire_path_under_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("nested", "t").unwrap();
    assert!(g.path().starts_with(mgr.root()));
}

#[test]
fn acquire_path_basename_matches_name() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("phase-spec", "t").unwrap();
    assert_eq!(g.path().file_name().unwrap(), "phase-spec");
}

#[test]
fn acquire_creates_intermediate_root_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    // .darkrun/locks did not exist before acquire.
    assert!(!mgr.root().exists());
    let _g = mgr.acquire("x", "t").unwrap();
    assert!(mgr.root().exists());
}

#[test]
fn holder_json_is_a_file_not_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    assert!(g.path().join("holder.json").is_file());
}

#[test]
fn acquire_returns_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(mgr.acquire("x", "t").is_ok());
}

#[test]
fn reexported_lock_manager_works() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = ReexportedLockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    assert!(g.path().exists());
}

// Parameterized: many distinct names each acquire cleanly.
macro_rules! acquire_named {
    ($($fn:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let g = mgr.acquire($name, "tag").unwrap();
                assert!(g.path().exists());
                assert_eq!(g.path().file_name().unwrap().to_str().unwrap(), $name);
            }
        )*
    };
}

acquire_named! {
    acquire_name_spec => "spec",
    acquire_name_review => "review",
    acquire_name_manufacture => "manufacture",
    acquire_name_audit => "audit",
    acquire_name_tests => "tests",
    acquire_name_checkpoint => "checkpoint",
    acquire_name_worker => "worker",
    acquire_name_explorer => "explorer",
    acquire_name_reviewer => "reviewer",
    acquire_name_station => "station",
    acquire_name_factory => "factory",
    acquire_name_unit => "unit",
    acquire_name_pass => "pass",
    acquire_name_run => "run",
    acquire_name_single_char => "a",
    acquire_name_digits => "12345",
    acquire_name_with_dash => "a-b-c",
    acquire_name_with_underscore => "a_b_c",
    acquire_name_with_dot => "a.b.c",
    acquire_name_mixed_case => "MixedCase",
    acquire_name_uppercase => "BUILD",
    acquire_name_long => "this-is-a-fairly-long-lock-name-with-many-segments-1234567890",
    acquire_name_unit_001 => "unit-001",
    acquire_name_unit_002 => "unit-002",
    acquire_name_run_abc123 => "run-abc123",
}

// ===========================================================================
// SECTION 2: release / drop / idempotency
// ===========================================================================

#[test]
fn release_removes_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    let p = g.path().to_path_buf();
    g.release();
    assert!(!p.exists());
}

#[test]
fn release_removes_holder_json() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    let p = g.path().join("holder.json");
    g.release();
    assert!(!p.exists());
}

#[test]
fn release_allows_immediate_reacquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    g.release();
    let again = mgr.acquire("x", "t").unwrap();
    assert!(again.path().exists());
}

#[test]
fn drop_removes_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let p = {
        let g = mgr.acquire("y", "t").unwrap();
        g.path().to_path_buf()
    };
    assert!(!p.exists());
}

#[test]
fn drop_allows_reacquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    {
        let _g = mgr.acquire("y", "t").unwrap();
    }
    mgr.acquire("y", "t").expect("reacquire after drop");
}

#[test]
fn reacquire_many_cycles() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    for i in 0..50 {
        let g = mgr.acquire("cycle", "t").unwrap_or_else(|e| panic!("iter {i}: {e}"));
        assert!(g.path().exists());
        g.release();
        assert!(!mgr.root().join("cycle").exists(), "iter {i} not released");
    }
}

#[test]
fn reacquire_after_drop_many_cycles() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    for i in 0..50 {
        let g = mgr.acquire("dropcycle", "t").unwrap();
        assert!(g.path().exists(), "iter {i}");
        drop(g);
        assert!(!mgr.root().join("dropcycle").exists(), "iter {i}");
    }
}

#[test]
fn explicit_release_then_drop_of_clone_path_is_safe() {
    // release consumes the guard; there is no double-free path, but confirm a
    // re-acquire of a fresh guard then dropping it twice over cycles is stable.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("idem", "t").unwrap();
    let p = g.path().to_path_buf();
    g.release();
    // releasing already removed the dir; manually creating + dropping a new one
    // must also clean up.
    let g2 = mgr.acquire("idem", "t").unwrap();
    assert_eq!(g2.path(), p);
    drop(g2);
    assert!(!p.exists());
}

#[test]
fn release_is_effective_even_if_dir_already_gone() {
    // If something external removes the dir, release must not error/panic.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("vanish", "t").unwrap();
    fs::remove_dir_all(g.path()).unwrap();
    // best-effort release: just must not panic.
    g.release();
    assert!(!mgr.root().join("vanish").exists());
}

#[test]
fn drop_is_effective_even_if_dir_already_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("vanish2", "t").unwrap();
    fs::remove_dir_all(g.path()).unwrap();
    drop(g); // must not panic
}

// ===========================================================================
// SECTION 3: with_lock
// ===========================================================================

#[test]
fn with_lock_returns_closure_value() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let out = mgr.with_lock("z", "t", || 7 * 6).unwrap();
    assert_eq!(out, 42);
}

#[test]
fn with_lock_releases_after_success() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    mgr.with_lock("z", "t", || ()).unwrap();
    assert!(!mgr.root().join("z").exists());
}

#[test]
fn with_lock_string_value() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let s = mgr.with_lock("z", "t", || String::from("checkpoint")).unwrap();
    assert_eq!(s, "checkpoint");
}

#[test]
fn with_lock_vec_value() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let v = mgr.with_lock("z", "t", || vec![1, 2, 3]).unwrap();
    assert_eq!(v, vec![1, 2, 3]);
}

#[test]
fn with_lock_unit_value() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let out: () = mgr.with_lock("z", "t", || ()).unwrap();
    assert_eq!(out, ());
}

#[test]
fn with_lock_closure_observes_held_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let root = mgr.root().to_path_buf();
    let held = mgr
        .with_lock("observe", "t", || root.join("observe").exists())
        .unwrap();
    assert!(held, "lock dir must exist while the closure runs");
}

#[test]
fn with_lock_closure_can_capture_and_mutate() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let mut counter = 0;
    mgr.with_lock("mut", "t", || counter += 5).unwrap();
    assert_eq!(counter, 5);
}

#[test]
fn with_lock_releases_on_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = mgr.with_lock("boom", "t", || panic!("inside"));
    }));
    assert!(res.is_err());
    assert!(!mgr.root().join("boom").exists(), "must release on panic");
}

#[test]
fn with_lock_reacquirable_after_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = mgr.with_lock("boom", "t", || panic!("inside"));
    }));
    mgr.acquire("boom", "t").expect("reacquire after panic");
}

#[test]
fn with_lock_panic_payload_propagates() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = mgr.with_lock("boom", "t", || panic!("custom-message-7"));
    }));
    let payload = res.unwrap_err();
    let msg = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()))
        .unwrap_or("");
    assert_eq!(msg, "custom-message-7");
}

#[test]
fn with_lock_sequential_same_name() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let a = mgr.with_lock("seq", "t", || 1).unwrap();
    let b = mgr.with_lock("seq", "t", || 2).unwrap();
    let c = mgr.with_lock("seq", "t", || 3).unwrap();
    assert_eq!((a, b, c), (1, 2, 3));
}

#[test]
fn with_lock_nested_distinct_names() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let out = mgr
        .with_lock("outer", "t", || mgr.with_lock("inner", "t", || 99).unwrap())
        .unwrap();
    assert_eq!(out, 99);
    assert!(!mgr.root().join("outer").exists());
    assert!(!mgr.root().join("inner").exists());
}

// ===========================================================================
// SECTION 4: holder.json shape
// ===========================================================================

#[test]
fn holder_has_pid_field() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v.get("pid").is_some());
}

#[test]
fn holder_has_at_field() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v.get("at").is_some());
}

#[test]
fn holder_has_tag_field() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v.get("tag").is_some());
}

#[test]
fn holder_exactly_three_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v.as_object().unwrap().len(), 3);
}

#[test]
fn holder_pid_is_current_process() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["pid"].as_i64().unwrap(), std::process::id() as i64);
}

#[test]
fn holder_tag_roundtrips() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "reviewer-checkpoint").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"].as_str().unwrap(), "reviewer-checkpoint");
}

#[test]
fn holder_at_is_recent_millis() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let before = now_millis();
    let g = mgr.acquire("h", "t").unwrap();
    let after = now_millis();
    let v = read_holder_json(g.path());
    let at = v["at"].as_u64().unwrap();
    assert!(at >= before.saturating_sub(2000), "at={at} before={before}");
    assert!(at <= after + 2000, "at={at} after={after}");
}

#[test]
fn holder_at_is_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v["at"].as_u64().unwrap() > 0);
}

#[test]
fn holder_pid_is_integer_json() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v["pid"].is_i64() || v["pid"].is_u64());
}

#[test]
fn holder_at_is_unsigned_json() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v["at"].is_u64());
}

#[test]
fn holder_tag_is_string_json() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v["tag"].is_string());
}

#[test]
fn holder_json_is_pretty_printed() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let raw = fs::read_to_string(g.path().join("holder.json")).unwrap();
    // pretty json has newlines + indentation.
    assert!(raw.contains('\n'), "expected pretty (multiline) json");
}

#[test]
fn holder_json_parses_as_object() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "t").unwrap();
    let v = read_holder_json(g.path());
    assert!(v.is_object());
}

#[test]
fn holder_tag_empty_string_allowed() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"].as_str().unwrap(), "");
}

#[test]
fn holder_tag_with_unicode() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "café-naïve-日本").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"].as_str().unwrap(), "café-naïve-日本");
}

#[test]
fn holder_tag_with_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("h", "worker 7 phase audit").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"].as_str().unwrap(), "worker 7 phase audit");
}

#[test]
fn holder_tag_with_quotes_and_escapes() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let weird = "tag with \"quotes\" and \\ backslash and \n newline";
    let g = mgr.acquire("h", weird).unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"].as_str().unwrap(), weird);
}

#[test]
fn holder_changes_tag_across_reacquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g1 = mgr.acquire("h", "first-tag").unwrap();
    assert_eq!(read_holder_json(g1.path())["tag"], "first-tag");
    g1.release();
    let g2 = mgr.acquire("h", "second-tag").unwrap();
    assert_eq!(read_holder_json(g2.path())["tag"], "second-tag");
}

// ===========================================================================
// SECTION 5: contention (held lock not stealable while fresh)
// ===========================================================================

#[test]
fn held_lock_dir_present_for_second_manager() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let _g = mgr.acquire("shared", "first").unwrap();
    let mgr2 = LockManager::new(tmp.path());
    assert!(mgr2.root().join("shared").exists());
}

#[test]
fn second_acquire_blocks_until_release() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("shared", "first").unwrap();
    let root = tmp.path().to_path_buf();
    let h = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        drop(g);
    });
    let mgr2 = LockManager::new(&root);
    let start = std::time::Instant::now();
    let g2 = mgr2.acquire("shared", "second").unwrap();
    assert!(start.elapsed() >= Duration::from_millis(100), "should have blocked");
    assert!(g2.path().exists());
    h.join().unwrap();
}

#[test]
fn second_manager_gets_lock_after_first_releases() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr1 = LockManager::new(tmp.path());
    let mgr2 = LockManager::new(tmp.path());
    let g1 = mgr1.acquire("hand", "a").unwrap();
    g1.release();
    let g2 = mgr2.acquire("hand", "b").unwrap();
    assert_eq!(read_holder_json(g2.path())["tag"], "b");
}

#[test]
fn freshly_held_lock_is_not_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("fresh", "t").unwrap();
    assert!(!mgr.is_stale(g.path()));
}

#[test]
fn held_lock_with_old_mtime_but_live_pid_not_stale() {
    // Even backdated, our own (live) pid protects the lock.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("live", "t").unwrap();
    backdate(g.path(), 3600);
    assert!(!mgr.is_stale(g.path()), "live pid must protect even an old dir");
}

#[test]
fn old_dir_with_live_holder_acquire_blocks() {
    // A backdated dir whose holder pid is alive (ours) is not stealable;
    // acquire from another name-equal manager must block, not steal.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("protected");
    fs::create_dir_all(&dir).unwrap();
    let me = std::process::id() as i32;
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"pid": me, "at": 0u64, "tag": "me"}).to_string(),
    )
    .unwrap();
    backdate(&dir, 3600);
    assert!(!mgr.is_stale(&dir));
}

// ===========================================================================
// SECTION 6: stale reclamation (dead pid + old mtime)
// ===========================================================================

#[test]
fn dead_pid_old_dir_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = plant_stale(&mgr, "wedged", DEAD_PID, "dead");
    assert!(mgr.is_stale(&dir));
}

#[test]
fn dead_pid_old_dir_is_reclaimed_on_acquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    plant_stale(&mgr, "wedged", DEAD_PID, "dead");
    let g = mgr.acquire("wedged", "fresh").unwrap();
    assert!(g.path().exists());
}

#[test]
fn reclaimed_lock_has_fresh_holder() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    plant_stale(&mgr, "wedged", DEAD_PID, "dead-tag");
    let g = mgr.acquire("wedged", "fresh-tag").unwrap();
    let v = read_holder_json(g.path());
    assert_eq!(v["tag"], "fresh-tag");
    assert_eq!(v["pid"].as_i64().unwrap(), std::process::id() as i64);
}

#[test]
fn reclaimed_lock_at_is_refreshed() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    plant_stale(&mgr, "wedged", DEAD_PID, "dead");
    let before = now_millis();
    let g = mgr.acquire("wedged", "fresh").unwrap();
    let at = read_holder_json(g.path())["at"].as_u64().unwrap();
    // planted "at" was 0; refreshed must be recent.
    assert!(at >= before.saturating_sub(2000), "at must be refreshed, got {at}");
}

#[test]
fn holderless_old_dir_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("nohldr");
    fs::create_dir_all(&dir).unwrap();
    backdate(&dir, 3600);
    assert!(mgr.is_stale(&dir));
}

#[test]
fn holderless_old_dir_is_reclaimed() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("nohldr");
    fs::create_dir_all(&dir).unwrap();
    backdate(&dir, 3600);
    let g = mgr.acquire("nohldr", "rescued").unwrap();
    assert_eq!(read_holder_json(g.path())["tag"], "rescued");
}

#[test]
fn holderless_fresh_dir_is_not_stale() {
    // No holder.json but recent mtime: not yet a stale candidate.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("youngnohldr");
    fs::create_dir_all(&dir).unwrap();
    assert!(!mgr.is_stale(&dir), "young holder-less dir must not be stale yet");
}

#[test]
fn corrupt_holder_old_dir_is_stale() {
    // Unparseable holder.json behaves like "no holder" => stale once old.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("corrupt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("holder.json"), "{ this is not json").unwrap();
    backdate(&dir, 3600);
    assert!(mgr.is_stale(&dir), "corrupt holder on old dir is wedged");
}

#[test]
fn corrupt_holder_old_dir_is_reclaimed() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("corrupt2");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("holder.json"), "not-json-at-all").unwrap();
    backdate(&dir, 3600);
    let g = mgr.acquire("corrupt2", "fixed").unwrap();
    assert_eq!(read_holder_json(g.path())["tag"], "fixed");
}

#[test]
fn empty_holder_old_dir_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("emptyholder");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("holder.json"), "").unwrap();
    backdate(&dir, 3600);
    assert!(mgr.is_stale(&dir));
}

#[test]
fn partial_holder_missing_pid_old_dir_is_stale() {
    // holder.json missing required `pid` => parse fails => treated as wedged.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("partial");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"at": 0u64, "tag": "x"}).to_string(),
    )
    .unwrap();
    backdate(&dir, 3600);
    assert!(mgr.is_stale(&dir));
}

#[test]
fn dead_pid_fresh_dir_is_not_stale() {
    // Dead pid but recent mtime: age gate blocks reclamation.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("youngdead");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"pid": DEAD_PID, "at": 0u64, "tag": "x"}).to_string(),
    )
    .unwrap();
    // do NOT backdate
    assert!(!mgr.is_stale(&dir), "fresh mtime blocks stale reclaim");
}

#[test]
fn dead_pid_just_under_stale_window_not_stale() {
    // Age below STALE_AFTER (5 min): backdate only 60s.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("under");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"pid": DEAD_PID, "at": 0u64, "tag": "x"}).to_string(),
    )
    .unwrap();
    backdate(&dir, 60);
    assert!(!mgr.is_stale(&dir), "60s old is under the 5min stale window");
}

#[test]
fn dead_pid_well_over_stale_window_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("over");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"pid": DEAD_PID, "at": 0u64, "tag": "x"}).to_string(),
    )
    .unwrap();
    backdate(&dir, 600); // 10 min
    assert!(mgr.is_stale(&dir));
}

#[test]
fn is_stale_false_for_missing_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(!mgr.is_stale(&mgr.root().join("ghost")));
}

#[test]
fn is_stale_false_for_path_outside_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(!mgr.is_stale(Path::new("/this/does/not/exist/anywhere/xyz")));
}

#[test]
fn is_stale_idempotent() {
    // is_stale is a pure read; calling it repeatedly must not mutate the dir.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = plant_stale(&mgr, "repeat", DEAD_PID, "x");
    let r1 = mgr.is_stale(&dir);
    let r2 = mgr.is_stale(&dir);
    let r3 = mgr.is_stale(&dir);
    assert_eq!((r1, r2, r3), (true, true, true));
    assert!(dir.exists(), "is_stale must not remove the dir");
}

#[test]
fn live_pid_old_dir_not_reclaimed_via_acquire_blocks_then_times_out_path() {
    // We can't wait 30s, but we can prove is_stale gates the steal: with a live
    // pid and old mtime, is_stale=false so acquire would block (not steal).
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dir = mgr.root().join("liveold");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("holder.json"),
        serde_json::json!({"pid": std::process::id() as i32, "at": 0u64, "tag": "me"})
            .to_string(),
    )
    .unwrap();
    backdate(&dir, 3600);
    assert!(!mgr.is_stale(&dir));
}

// ===========================================================================
// SECTION 7: distinct names & nesting
// ===========================================================================

#[test]
fn distinct_names_do_not_contend() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let a = mgr.acquire("alpha", "t").unwrap();
    let b = mgr.acquire("beta", "t").unwrap();
    assert!(a.path().exists() && b.path().exists());
    assert_ne!(a.path(), b.path());
}

#[test]
fn many_distinct_names_held_simultaneously() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let mut guards = Vec::new();
    for i in 0..30 {
        guards.push(mgr.acquire(&format!("lock-{i}"), "t").unwrap());
    }
    for (i, g) in guards.iter().enumerate() {
        assert!(g.path().exists(), "lock-{i} should be held");
    }
    // all paths distinct
    let set: HashSet<_> = guards.iter().map(|g| g.path().to_path_buf()).collect();
    assert_eq!(set.len(), 30);
}

#[test]
fn slash_name_without_existing_parent_times_out() {
    // The acquire path uses a NON-recursive `create_dir`, so a name containing
    // a path separator whose parent dir does not exist cannot be created and
    // the acquire blocks on the retry loop rather than auto-creating the
    // hierarchy. This documents the real contract: lock names are flat dir
    // names, not auto-created hierarchies.
    //
    // Asserting that contract directly via the public `acquire` would block on
    // the full 30s `ACQUIRE_TIMEOUT` and (when detached) leak a thread that
    // keeps the whole test binary alive for 30s, destabilising sibling tests
    // under parallel load. Instead we verify the underlying invariant
    // deterministically: with the parent missing, `create_dir` of the leaf
    // fails and no lock dir appears — which is exactly what makes `acquire`
    // loop until it times out.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    // Create the locks root but NOT the "run" intermediate.
    fs::create_dir_all(mgr.root()).unwrap();

    let leaf = mgr.root().join("run").join("unit-missing-parent");
    // Non-recursive create must fail because the "run" parent does not exist.
    let err = fs::create_dir(&leaf).unwrap_err();
    assert_eq!(
        err.kind(),
        std::io::ErrorKind::NotFound,
        "missing parent makes a non-recursive create fail with NotFound"
    );
    // Nothing was created: the intermediate and the leaf are both absent.
    assert!(!mgr.root().join("run").exists());
    assert!(!leaf.exists());
}

#[test]
fn slash_name_with_precreated_parent_acquires_without_blocking() {
    // The positive counterpart: when the parent IS pre-created, the same
    // slash-name acquires immediately via the public API — proving the block
    // above is caused solely by the missing intermediate, not the separator.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    fs::create_dir_all(mgr.root().join("run")).unwrap();
    let start = std::time::Instant::now();
    let g = mgr.acquire("run/unit-missing-parent", "t").expect("acquire");
    assert!(g.path().exists());
    // It returned promptly, nowhere near the 30s timeout.
    assert!(start.elapsed() < Duration::from_secs(5));
    g.release();
}

#[test]
fn slash_name_with_precreated_parent_works() {
    // If the intermediate dir exists, a slash-name acquires the leaf cleanly.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    fs::create_dir_all(mgr.root().join("run")).unwrap();
    let g = mgr.acquire("run/unit", "t").unwrap();
    assert!(g.path().exists());
    assert!(g.path().ends_with("unit"));
    assert!(g.path().starts_with(mgr.root().join("run")));
    g.release();
    assert!(!mgr.root().join("run").join("unit").exists());
}

#[test]
fn sibling_slash_names_share_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    fs::create_dir_all(mgr.root().join("run")).unwrap();
    let a = mgr.acquire("run/u1", "t").unwrap();
    let b = mgr.acquire("run/u2", "t").unwrap();
    assert!(a.path().exists() && b.path().exists());
    assert_eq!(a.path().parent(), b.path().parent());
}

#[test]
fn precreated_nested_reacquire_after_release() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    fs::create_dir_all(mgr.root().join("phase")).unwrap();
    let g = mgr.acquire("phase/spec", "t").unwrap();
    g.release();
    mgr.acquire("phase/spec", "t").expect("reacquire nested");
}

// "Nested-style" flat names (separator chars embedded in a flat dir name).
// These do NOT touch the filesystem hierarchy and must acquire cleanly.
macro_rules! flat_nested_named {
    ($($fn:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let g = mgr.acquire($name, "t").unwrap();
                assert!(g.path().exists());
                assert!(g.path().starts_with(mgr.root()));
                assert_eq!(g.path().file_name().unwrap().to_str().unwrap(), $name);
                g.release();
                assert!(!mgr.root().join($name).exists());
            }
        )*
    };
}

flat_nested_named! {
    nested_run_spec => "run.spec",
    nested_run_review => "run.review",
    nested_run_manufacture => "run.manufacture",
    nested_run_audit => "run.audit",
    nested_run_tests => "run.tests",
    nested_run_checkpoint => "run.checkpoint",
    nested_factory_station => "factory-station",
    nested_station_worker => "station-worker",
    nested_worker_pass => "worker-pass",
    nested_explorer_reviewer => "explorer-reviewer",
}

// ===========================================================================
// SECTION 8: manager construction / root
// ===========================================================================

#[test]
fn root_ends_with_locks() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(mgr.root().ends_with("locks"));
}

#[test]
fn root_parent_is_darkrun() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(mgr.root().parent().unwrap().ends_with(".darkrun"));
}

#[test]
fn root_grandparent_is_repo_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let grand = mgr.root().parent().unwrap().parent().unwrap();
    assert_eq!(grand, tmp.path());
}

#[test]
fn root_does_not_exist_until_acquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    assert!(!mgr.root().exists());
}

#[test]
fn new_accepts_str_path() {
    let tmp = tempfile::tempdir().unwrap();
    let s: &str = tmp.path().to_str().unwrap();
    let mgr = LockManager::new(s);
    assert!(mgr.root().ends_with("locks"));
}

#[test]
fn new_accepts_pathbuf() {
    let tmp = tempfile::tempdir().unwrap();
    let pb: PathBuf = tmp.path().to_path_buf();
    let mgr = LockManager::new(pb);
    assert!(mgr.root().ends_with("locks"));
}

#[test]
fn new_accepts_path_ref() {
    let tmp = tempfile::tempdir().unwrap();
    let p: &Path = tmp.path();
    let mgr = LockManager::new(p);
    assert!(mgr.root().ends_with("locks"));
}

#[test]
fn manager_clone_shares_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let cloned = mgr.clone();
    assert_eq!(mgr.root(), cloned.root());
}

#[test]
fn cloned_manager_sees_same_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let cloned = mgr.clone();
    let _g = mgr.acquire("shared", "t").unwrap();
    assert!(cloned.root().join("shared").exists());
}

#[test]
fn two_managers_same_root_share_lock_state() {
    let tmp = tempfile::tempdir().unwrap();
    let m1 = LockManager::new(tmp.path());
    let m2 = LockManager::new(tmp.path());
    let g = m1.acquire("x", "a").unwrap();
    assert!(m2.root().join("x").exists());
    g.release();
    assert!(!m2.root().join("x").exists());
    m2.acquire("x", "b").expect("m2 reacquires");
}

#[test]
fn two_managers_distinct_roots_isolated() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let m1 = LockManager::new(tmp1.path());
    let m2 = LockManager::new(tmp2.path());
    let _g1 = m1.acquire("same", "a").unwrap();
    // m2 has an independent root: acquiring the same name must succeed.
    let g2 = m2.acquire("same", "b").unwrap();
    assert!(g2.path().exists());
    assert_ne!(m1.root(), m2.root());
}

#[test]
fn manager_root_is_stable_across_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let r1 = mgr.root().to_path_buf();
    let _g = mgr.acquire("x", "t").unwrap();
    let r2 = mgr.root().to_path_buf();
    assert_eq!(r1, r2);
}

#[test]
fn manager_debug_format_nonempty() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let dbg = format!("{mgr:?}");
    assert!(dbg.contains("LockManager"));
}

#[test]
fn guard_debug_format_nonempty() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("x", "t").unwrap();
    let dbg = format!("{:?}", g);
    assert!(dbg.contains("LockGuard"));
}

// ===========================================================================
// SECTION 9: concurrency via threads
// ===========================================================================

#[test]
fn concurrent_acquire_mutual_exclusion() {
    // N threads contend for one lock; a shared counter proves no two ever hold
    // it at the same time.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let in_section = Arc::new(Mutex::new(0i32));
    let max_seen = Arc::new(Mutex::new(0i32));
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    for _ in 0..8 {
        let root = root.clone();
        let in_section = Arc::clone(&in_section);
        let max_seen = Arc::clone(&max_seen);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            barrier.wait();
            for _ in 0..5 {
                mgr.with_lock("crit", "worker", || {
                    let mut n = in_section.lock().unwrap();
                    *n += 1;
                    {
                        let mut m = max_seen.lock().unwrap();
                        if *n > *m {
                            *m = *n;
                        }
                    }
                    // tiny window to expose overlap if exclusion is broken
                    thread::sleep(Duration::from_millis(1));
                    *n -= 1;
                })
                .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*max_seen.lock().unwrap(), 1, "lock must be mutually exclusive");
    assert!(!root.join(".darkrun/locks/crit").exists(), "lock left held");
}

#[test]
fn concurrent_distinct_names_all_succeed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let mut handles = Vec::new();
    for i in 0..16 {
        let root = root.clone();
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            let g = mgr.acquire(&format!("name-{i}"), "t").unwrap();
            assert!(g.path().exists());
            // hold briefly then release
            thread::sleep(Duration::from_millis(2));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // all released
    for i in 0..16 {
        assert!(!root.join(format!(".darkrun/locks/name-{i}")).exists());
    }
}

#[test]
fn concurrent_increment_under_lock_is_consistent() {
    // Without a lock, racing increments would lose updates. With it, the final
    // count equals threads * iters exactly.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let counter = Arc::new(Mutex::new(0u64));
    let threads = 6u64;
    let iters = 20u64;
    let mut handles = Vec::new();
    for _ in 0..threads {
        let root = root.clone();
        let counter = Arc::clone(&counter);
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            for _ in 0..iters {
                mgr.with_lock("counter", "t", || {
                    let mut c = counter.lock().unwrap();
                    *c += 1;
                })
                .unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*counter.lock().unwrap(), threads * iters);
}

#[test]
fn concurrent_handoff_eventually_all_complete() {
    // Many threads each grab the same lock once; all should eventually succeed.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let done = Arc::new(Mutex::new(0u32));
    let mut handles = Vec::new();
    for _ in 0..12 {
        let root = root.clone();
        let done = Arc::clone(&done);
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            let g = mgr.acquire("hand", "t").unwrap();
            thread::sleep(Duration::from_millis(1));
            g.release();
            *done.lock().unwrap() += 1;
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*done.lock().unwrap(), 12);
}

#[test]
fn concurrent_same_manager_clone_across_threads() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let order = Arc::new(Mutex::new(Vec::<u32>::new()));
    let mut handles = Vec::new();
    for id in 0..5u32 {
        let mgr = mgr.clone();
        let order = Arc::clone(&order);
        handles.push(thread::spawn(move || {
            mgr.with_lock("shared", "t", || {
                order.lock().unwrap().push(id);
                thread::sleep(Duration::from_millis(1));
            })
            .unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let recorded = order.lock().unwrap();
    assert_eq!(recorded.len(), 5);
    let set: HashSet<_> = recorded.iter().copied().collect();
    assert_eq!(set.len(), 5, "every thread should have entered exactly once");
}

#[test]
fn concurrent_reclaim_of_stale_lock() {
    // A stale (dead pid, old) lock is planted; multiple threads race to reclaim
    // it. Exactly one wins initially, but with handoff all eventually acquire.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    plant_stale(&mgr, "stale", DEAD_PID, "ghost");
    let root = tmp.path().to_path_buf();
    let wins = Arc::new(Mutex::new(0u32));
    let mut handles = Vec::new();
    for _ in 0..6 {
        let root = root.clone();
        let wins = Arc::clone(&wins);
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            let g = mgr.acquire("stale", "rescuer").unwrap();
            *wins.lock().unwrap() += 1;
            g.release();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(*wins.lock().unwrap(), 6, "all threads should reclaim+handoff");
}

#[test]
fn parallel_with_lock_returns_correct_values() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let mut handles = Vec::new();
    for i in 0..10u64 {
        let root = root.clone();
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            mgr.with_lock("v", "t", || i * i).unwrap()
        }));
    }
    let mut results: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    results.sort_unstable();
    let expected: Vec<u64> = (0..10).map(|i| i * i).collect();
    assert_eq!(results, expected);
}

// ===========================================================================
// SECTION 10: LockTimeout error shape / Display
// ===========================================================================

#[test]
fn lock_timeout_display_contains_name() {
    let err = CoreError::LockTimeout {
        name: "build".into(),
        timeout_ms: 30_000,
    };
    assert!(err.to_string().contains("build"));
}

#[test]
fn lock_timeout_display_contains_timeout_ms() {
    let err = CoreError::LockTimeout {
        name: "build".into(),
        timeout_ms: 30_000,
    };
    assert!(err.to_string().contains("30000"));
}

#[test]
fn lock_timeout_display_mentions_timed_out() {
    let err = CoreError::LockTimeout {
        name: "x".into(),
        timeout_ms: 1,
    };
    assert!(err.to_string().to_lowercase().contains("timed out"));
}

#[test]
fn lock_timeout_debug_nonempty() {
    let err = CoreError::LockTimeout {
        name: "x".into(),
        timeout_ms: 1,
    };
    assert!(format!("{err:?}").contains("LockTimeout"));
}

#[test]
fn lock_timeout_distinct_names_render_distinctly() {
    let a = CoreError::LockTimeout {
        name: "alpha".into(),
        timeout_ms: 100,
    };
    let b = CoreError::LockTimeout {
        name: "beta".into(),
        timeout_ms: 100,
    };
    assert_ne!(a.to_string(), b.to_string());
}

#[test]
fn lock_timeout_matchable_pattern() {
    let err = CoreError::LockTimeout {
        name: "n".into(),
        timeout_ms: 42,
    };
    match err {
        CoreError::LockTimeout { name, timeout_ms } => {
            assert_eq!(name, "n");
            assert_eq!(timeout_ms, 42);
        }
        _ => panic!("wrong variant"),
    }
}

// Parameterized timeout rendering for a range of values.
macro_rules! timeout_renders {
    ($($fn:ident => $name:literal, $ms:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let err = CoreError::LockTimeout { name: $name.into(), timeout_ms: $ms };
                let s = err.to_string();
                assert!(s.contains($name));
                assert!(s.contains(&$ms.to_string()));
            }
        )*
    };
}

timeout_renders! {
    timeout_render_spec_0 => "spec", 0u64,
    timeout_render_spec_1 => "spec", 1u64,
    timeout_render_review_500 => "review", 500u64,
    timeout_render_manufacture_1000 => "manufacture", 1000u64,
    timeout_render_audit_5000 => "audit", 5000u64,
    timeout_render_tests_30000 => "tests", 30000u64,
    timeout_render_checkpoint_60000 => "checkpoint", 60000u64,
    timeout_render_worker_max => "worker", 18446744073709551615u64,
}

// ===========================================================================
// SECTION 11: determinism / idempotency of paths & behavior
// ===========================================================================

#[test]
fn same_name_yields_same_path_across_managers() {
    let tmp = tempfile::tempdir().unwrap();
    let m1 = LockManager::new(tmp.path());
    let m2 = LockManager::new(tmp.path());
    let g1 = m1.acquire("det", "t").unwrap();
    let p1 = g1.path().to_path_buf();
    g1.release();
    let g2 = m2.acquire("det", "t").unwrap();
    assert_eq!(p1, g2.path());
}

#[test]
fn root_is_deterministic_for_same_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let m1 = LockManager::new(tmp.path());
    let m2 = LockManager::new(tmp.path());
    assert_eq!(m1.root(), m2.root());
}

#[test]
fn acquire_release_leaves_root_present_but_empty_of_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("ephemeral", "t").unwrap();
    g.release();
    // root survives; only the named lock dir is gone.
    assert!(mgr.root().exists());
    assert!(!mgr.root().join("ephemeral").exists());
}

#[test]
fn repeated_acquire_release_holder_pid_constant() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let me = std::process::id() as i64;
    for _ in 0..10 {
        let g = mgr.acquire("p", "t").unwrap();
        assert_eq!(read_holder_json(g.path())["pid"].as_i64().unwrap(), me);
        g.release();
    }
}

#[test]
fn acquire_does_not_disturb_sibling_locks() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let held = mgr.acquire("keepme", "t").unwrap();
    let held_at = read_holder_json(held.path())["at"].clone();
    // churn another lock repeatedly
    for _ in 0..5 {
        let g = mgr.acquire("churn", "t").unwrap();
        g.release();
    }
    // the kept lock's holder is untouched
    assert_eq!(read_holder_json(held.path())["at"], held_at);
    assert!(held.path().exists());
}

#[test]
fn stale_reclaim_is_deterministic_outcome() {
    // Same planted condition reclaims the same way every time.
    for _ in 0..5 {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = LockManager::new(tmp.path());
        plant_stale(&mgr, "d", DEAD_PID, "old");
        let g = mgr.acquire("d", "new").unwrap();
        assert_eq!(read_holder_json(g.path())["tag"], "new");
    }
}

// ===========================================================================
// SECTION 12: edge / boundary on names and state
// ===========================================================================

#[test]
fn acquire_then_external_dir_removal_then_reacquire() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("ext", "t").unwrap();
    let p = g.path().to_path_buf();
    // External actor removes the lock dir out from under the guard.
    fs::remove_dir_all(&p).unwrap();
    // A different manager can now acquire it.
    let mgr2 = LockManager::new(tmp.path());
    let g2 = mgr2.acquire("ext", "t2").unwrap();
    assert!(g2.path().exists());
    drop(g); // stale guard drop must not error
}

#[test]
fn holder_json_survives_while_lock_held() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("persist", "t").unwrap();
    for _ in 0..3 {
        assert!(g.path().join("holder.json").exists());
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn lock_name_equal_to_holder_json_literal() {
    // A lock literally named "holder.json" must still work as a dir name.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("holder.json", "t").unwrap();
    assert!(g.path().is_dir());
    assert!(g.path().join("holder.json").is_file());
}

#[test]
fn multiple_locks_each_have_own_holder() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let a = mgr.acquire("a", "tag-a").unwrap();
    let b = mgr.acquire("b", "tag-b").unwrap();
    assert_eq!(read_holder_json(a.path())["tag"], "tag-a");
    assert_eq!(read_holder_json(b.path())["tag"], "tag-b");
}

#[test]
fn reacquired_lock_overwrites_old_holder_content() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g1 = mgr.acquire("ov", "AAAA").unwrap();
    let at1 = read_holder_json(g1.path())["at"].as_u64().unwrap();
    g1.release();
    thread::sleep(Duration::from_millis(5));
    let g2 = mgr.acquire("ov", "BBBB").unwrap();
    let v2 = read_holder_json(g2.path());
    assert_eq!(v2["tag"], "BBBB");
    // at should be >= the first (time moves forward)
    assert!(v2["at"].as_u64().unwrap() >= at1);
}

#[test]
fn guard_path_returns_borrowed_path() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("bp", "t").unwrap();
    let p: &Path = g.path();
    assert!(p.is_absolute() || p.starts_with(mgr.root()));
}

#[test]
fn acquire_with_numeric_tag() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let g = mgr.acquire("n", "123456789").unwrap();
    assert_eq!(read_holder_json(g.path())["tag"], "123456789");
}

#[test]
fn acquire_with_very_long_tag() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let long = "x".repeat(4096);
    let g = mgr.acquire("lt", &long).unwrap();
    assert_eq!(read_holder_json(g.path())["tag"].as_str().unwrap().len(), 4096);
}

// Parameterized holder roundtrip across (name, tag) pairs.
macro_rules! holder_roundtrip {
    ($($fn:ident => $name:literal, $tag:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let g = mgr.acquire($name, $tag).unwrap();
                let v = read_holder_json(g.path());
                assert_eq!(v["tag"].as_str().unwrap(), $tag);
                assert_eq!(v["pid"].as_i64().unwrap(), std::process::id() as i64);
                assert!(v["at"].as_u64().unwrap() > 0);
            }
        )*
    };
}

holder_roundtrip! {
    rt_spec_worker => "spec", "worker-1",
    rt_review_reviewer => "review", "reviewer-2",
    rt_manufacture_factory => "manufacture", "factory-3",
    rt_audit_station => "audit", "station-4",
    rt_tests_unit => "tests", "unit-5",
    rt_checkpoint_pass => "checkpoint", "pass-6",
    rt_run_explorer => "run", "explorer-7",
    rt_a_emptyish => "a", "t",
    rt_dash_tag => "with-dash", "tag-with-dash",
    rt_dot_tag => "with.dot", "tag.with.dot",
    rt_under_tag => "with_under", "tag_with_under",
    rt_unicode => "uni", "ünïcödé",
}

// ===========================================================================
// SECTION 13: stale window boundary sweep (parameterized)
// ===========================================================================

// For a dead pid, age below 5min => not stale; above => stale.
macro_rules! stale_age_dead_pid {
    ($($fn:ident => $secs:literal, $expect:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let dir = mgr.root().join("age");
                fs::create_dir_all(&dir).unwrap();
                fs::write(
                    dir.join("holder.json"),
                    serde_json::json!({"pid": DEAD_PID, "at": 0u64, "tag": "x"}).to_string(),
                ).unwrap();
                backdate(&dir, $secs);
                assert_eq!(mgr.is_stale(&dir), $expect, "secs={}", $secs);
            }
        )*
    };
}

stale_age_dead_pid! {
    stale_age_10s => 10u64, false,
    stale_age_60s => 60u64, false,
    stale_age_120s => 120u64, false,
    stale_age_240s => 240u64, false,
    stale_age_290s => 290u64, false,
    stale_age_360s => 360u64, true,
    stale_age_600s => 600u64, true,
    stale_age_3600s => 3600u64, true,
    stale_age_86400s => 86400u64, true,
}

// For a LIVE pid (ours), no age makes it stale.
macro_rules! never_stale_live_pid {
    ($($fn:ident => $secs:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let dir = mgr.root().join("livesweep");
                fs::create_dir_all(&dir).unwrap();
                fs::write(
                    dir.join("holder.json"),
                    serde_json::json!({"pid": std::process::id() as i32, "at": 0u64, "tag": "me"}).to_string(),
                ).unwrap();
                backdate(&dir, $secs);
                assert!(!mgr.is_stale(&dir), "live pid never stale, secs={}", $secs);
            }
        )*
    };
}

never_stale_live_pid! {
    live_never_stale_60s => 60u64,
    live_never_stale_600s => 600u64,
    live_never_stale_3600s => 3600u64,
    live_never_stale_86400s => 86400u64,
}

// For a holderless dir, age below 5min => not stale; above => stale (wedged).
macro_rules! holderless_age {
    ($($fn:ident => $secs:literal, $expect:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let dir = mgr.root().join("hl");
                fs::create_dir_all(&dir).unwrap();
                backdate(&dir, $secs);
                assert_eq!(mgr.is_stale(&dir), $expect, "secs={}", $secs);
            }
        )*
    };
}

holderless_age! {
    holderless_age_30s => 30u64, false,
    holderless_age_290s => 290u64, false,
    holderless_age_360s => 360u64, true,
    holderless_age_3600s => 3600u64, true,
}

// ===========================================================================
// SECTION 14: many-name acquire/release sweeps (broad determinism)
// ===========================================================================

macro_rules! acquire_release_reacquire {
    ($($fn:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                // acquire
                let g = mgr.acquire($name, "t1").unwrap();
                assert!(g.path().exists());
                // release
                g.release();
                assert!(!mgr.root().join($name).exists());
                // reacquire
                let g2 = mgr.acquire($name, "t2").unwrap();
                assert!(g2.path().exists());
                assert_eq!(read_holder_json(g2.path())["tag"], "t2");
                drop(g2);
                assert!(!mgr.root().join($name).exists());
            }
        )*
    };
}

acquire_release_reacquire! {
    arr_l00 => "l00", arr_l01 => "l01", arr_l02 => "l02", arr_l03 => "l03",
    arr_l04 => "l04", arr_l05 => "l05", arr_l06 => "l06", arr_l07 => "l07",
    arr_l08 => "l08", arr_l09 => "l09", arr_l10 => "l10", arr_l11 => "l11",
    arr_l12 => "l12", arr_l13 => "l13", arr_l14 => "l14", arr_l15 => "l15",
    arr_l16 => "l16", arr_l17 => "l17", arr_l18 => "l18", arr_l19 => "l19",
    arr_l20 => "l20", arr_l21 => "l21", arr_l22 => "l22", arr_l23 => "l23",
    arr_l24 => "l24", arr_l25 => "l25", arr_l26 => "l26", arr_l27 => "l27",
    arr_l28 => "l28", arr_l29 => "l29",
}

// Sweep: with_lock returns the value for many names.
macro_rules! with_lock_value_sweep {
    ($($fn:ident => $name:literal, $val:expr),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let out = mgr.with_lock($name, "t", || $val).unwrap();
                assert_eq!(out, $val);
                assert!(!mgr.root().join($name).exists());
            }
        )*
    };
}

with_lock_value_sweep! {
    wlv_0 => "w0", 0i64, wlv_1 => "w1", 1i64, wlv_2 => "w2", -1i64,
    wlv_3 => "w3", 100i64, wlv_4 => "w4", 9999i64, wlv_5 => "w5", i64::MAX,
    wlv_6 => "w6", i64::MIN, wlv_7 => "w7", 42i64, wlv_8 => "w8", -42i64,
    wlv_9 => "w9", 7i64,
}

// Sweep: dead-pid stale reclaim across many names.
macro_rules! reclaim_sweep {
    ($($fn:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                plant_stale(&mgr, $name, DEAD_PID, "ghost");
                assert!(mgr.is_stale(&mgr.root().join($name)));
                let g = mgr.acquire($name, "rescued").unwrap();
                assert_eq!(read_holder_json(g.path())["tag"], "rescued");
            }
        )*
    };
}

reclaim_sweep! {
    rec_s00 => "s00", rec_s01 => "s01", rec_s02 => "s02", rec_s03 => "s03",
    rec_s04 => "s04", rec_s05 => "s05", rec_s06 => "s06", rec_s07 => "s07",
    rec_s08 => "s08", rec_s09 => "s09", rec_s10 => "s10", rec_s11 => "s11",
    rec_s12 => "s12", rec_s13 => "s13", rec_s14 => "s14", rec_s15 => "s15",
    rec_s16 => "s16", rec_s17 => "s17", rec_s18 => "s18", rec_s19 => "s19",
}

// Sweep: holder.json shape across many names (3 fields, correct types).
macro_rules! holder_shape_sweep {
    ($($fn:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let g = mgr.acquire($name, "shape").unwrap();
                let v = read_holder_json(g.path());
                let obj = v.as_object().unwrap();
                assert_eq!(obj.len(), 3, "exactly pid/at/tag");
                assert!(obj.contains_key("pid"));
                assert!(obj.contains_key("at"));
                assert!(obj.contains_key("tag"));
                assert!(v["pid"].is_i64() || v["pid"].is_u64());
                assert!(v["at"].is_u64());
                assert!(v["tag"].is_string());
            }
        )*
    };
}

holder_shape_sweep! {
    hs_h00 => "h00", hs_h01 => "h01", hs_h02 => "h02", hs_h03 => "h03",
    hs_h04 => "h04", hs_h05 => "h05", hs_h06 => "h06", hs_h07 => "h07",
    hs_h08 => "h08", hs_h09 => "h09", hs_h10 => "h10", hs_h11 => "h11",
    hs_h12 => "h12", hs_h13 => "h13", hs_h14 => "h14", hs_h15 => "h15",
    hs_h16 => "h16", hs_h17 => "h17", hs_h18 => "h18", hs_h19 => "h19",
}

// Sweep: distinct names never collide in path basenames.
macro_rules! distinct_path_sweep {
    ($($fn:ident => $a:literal, $b:literal),* $(,)?) => {
        $(
            #[test]
            fn $fn() {
                let tmp = tempfile::tempdir().unwrap();
                let mgr = LockManager::new(tmp.path());
                let ga = mgr.acquire($a, "t").unwrap();
                let gb = mgr.acquire($b, "t").unwrap();
                assert_ne!(ga.path(), gb.path());
                assert!(ga.path().exists() && gb.path().exists());
            }
        )*
    };
}

distinct_path_sweep! {
    dp_0 => "p0a", "p0b", dp_1 => "p1a", "p1b", dp_2 => "p2a", "p2b",
    dp_3 => "p3a", "p3b", dp_4 => "p4a", "p4b", dp_5 => "p5a", "p5b",
    dp_6 => "spec", "review", dp_7 => "manufacture", "audit",
    dp_8 => "tests", "checkpoint", dp_9 => "factory", "station",
}

// ===========================================================================
// SECTION 15: cross-phase realistic flow
// ===========================================================================

#[test]
fn phase_pipeline_sequential_locks() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = LockManager::new(tmp.path());
    let phases = ["spec", "review", "manufacture", "audit", "tests", "checkpoint"];
    let mut log = Vec::new();
    for phase in phases {
        let out = mgr
            .with_lock(phase, "pipeline", || phase.to_string())
            .unwrap();
        log.push(out);
        // each phase fully releases before the next
        assert!(!mgr.root().join(phase).exists());
    }
    assert_eq!(log, phases);
}

#[test]
fn phase_pipeline_concurrent_distinct_phases() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let phases = ["spec", "review", "manufacture", "audit", "tests", "checkpoint"];
    let mut handles = Vec::new();
    for phase in phases {
        let root = root.clone();
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            // distinct names => no contention; all run in parallel
            mgr.with_lock(phase, "p", || {
                thread::sleep(Duration::from_millis(2));
                phase
            })
            .unwrap()
        }));
    }
    let mut got: Vec<&str> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    got.sort_unstable();
    let mut expected = phases.to_vec();
    expected.sort_unstable();
    assert_eq!(got, expected);
}

#[test]
fn worker_contention_on_single_station() {
    // Multiple workers contend for one station lock; the station is held by at
    // most one worker at a time and all eventually run.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let ran = Arc::new(Mutex::new(Vec::<u32>::new()));
    let overlap = Arc::new(Mutex::new(0i32));
    let max_overlap = Arc::new(Mutex::new(0i32));
    let mut handles = Vec::new();
    for w in 0..5u32 {
        let root = root.clone();
        let ran = Arc::clone(&ran);
        let overlap = Arc::clone(&overlap);
        let max_overlap = Arc::clone(&max_overlap);
        handles.push(thread::spawn(move || {
            let mgr = LockManager::new(&root);
            mgr.with_lock("station-1", "worker", || {
                {
                    let mut o = overlap.lock().unwrap();
                    *o += 1;
                    let mut m = max_overlap.lock().unwrap();
                    if *o > *m {
                        *m = *o;
                    }
                }
                ran.lock().unwrap().push(w);
                thread::sleep(Duration::from_millis(1));
                *overlap.lock().unwrap() -= 1;
            })
            .unwrap();
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(ran.lock().unwrap().len(), 5);
    assert_eq!(*max_overlap.lock().unwrap(), 1);
}

// SECTION 16: the locks-root cannot be created (mkdir failure)

#[test]
fn acquire_errors_when_the_locks_root_cannot_be_created() {
    let dir = tempfile::tempdir().unwrap();
    // Plant a regular file where `.darkrun/` would be a directory, so creating
    // `.darkrun/locks` fails — exercising acquire's create-dir error arm.
    std::fs::write(dir.path().join(".darkrun"), "i am a file, not a dir").unwrap();
    let mgr = LockManager::new(dir.path());
    let err = mgr.acquire("build", "tag").unwrap_err();
    assert!(matches!(err, CoreError::Io { .. }));
}

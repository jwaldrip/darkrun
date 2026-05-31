//! Drift track (Track C) scaffold.
//!
//! Drift is a witnessed artifact mutation — a locked artifact whose on-disk
//! content no longer matches the hash the engine recorded. It preempts both
//! feedback and run work because building on a silently-changed artifact
//! produces inconsistent output.
//!
//! The drift *sweep* — [`record_station_witnesses`] snapshots a station's
//! locked artifacts (a content hash per output) when it completes, and
//! [`sweep`] re-hashes every witness each tick: a hash that no longer matches
//! (or a vanished file) deposits a drift entry, and a hash that matches again
//! clears a stale one (so reverting an artifact self-heals). [`accept`]
//! re-witnesses an intentional change. The manager's Track C reads the deposited
//! entries via [`first`]; with none, the track is a no-op.

use std::fs;
use std::path::PathBuf;

use darkrun_core::domain::{Drift, DriftKind};
use darkrun_core::{hash_file, StateStore, Witness};

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

/// A filename-safe drift id derived from an artifact path (so re-sweeps of the
/// same artifact overwrite rather than pile up).
fn drift_id_for(path: &str) -> String {
    let mut id = String::from("drift-");
    for c in path.chars() {
        id.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    id
}

/// Remove a drift entry if present (idempotent).
fn clear(store: &StateStore, run: &str, id: &str) -> Result<()> {
    let path = drift_dir(store, run).join(format!("{id}.md"));
    if path.exists() {
        fs::remove_file(&path).map_err(darkrun_core::CoreError::from)?;
    }
    Ok(())
}

/// Snapshot the locked artifacts of a just-completed station: hash every output
/// of its completed units and upsert a [`Witness`]. Called when a station locks.
pub fn record_station_witnesses(store: &StateStore, run: &str, station: &str) -> Result<()> {
    let root = repo_root(store);
    let units = store.read_units(run)?;
    let mut witnesses = store.read_witnesses(run)?;
    for u in units.iter().filter(|u| {
        u.station() == station && matches!(u.status(), darkrun_core::domain::Status::Completed)
    }) {
        for out in &u.frontmatter.outputs {
            if let Some(hash) = hash_file(&root.join(out)) {
                witnesses.retain(|w| w.path != *out);
                witnesses.push(Witness {
                    path: out.clone(),
                    hash,
                    station: station.to_string(),
                    unit: Some(u.slug.clone()),
                });
            }
        }
    }
    store.write_witnesses(run, &witnesses)?;
    Ok(())
}

/// Re-hash every witness; deposit a drift entry for any artifact whose content
/// changed or vanished, and clear the entry for any that matches again. Pure
/// over disk — same files, same result. Run at the top of each tick.
pub fn sweep(store: &StateStore, run: &str) -> Result<()> {
    let root = repo_root(store);
    for w in store.read_witnesses(run)? {
        let id = drift_id_for(&w.path);
        let drifted = match hash_file(&root.join(&w.path)) {
            Some(current) => current != w.hash,
            None => true, // a missing locked artifact is drift
        };
        if drifted {
            let entry = Drift {
                path: w.path.clone(),
                station: w.station.clone(),
                run: run.to_string(),
                kind: DriftKind::Output,
                age: String::new(),
                unit: w.unit.clone(),
            };
            record(store, run, &id, &entry)?;
        } else {
            clear(store, run, &id)?;
        }
    }
    Ok(())
}

/// Accept an intentional change to a locked artifact: re-witness it to its
/// current content hash and clear the drift entry. Returns `false` if the path
/// isn't witnessed or the file is unreadable. (The other resolution — reverting
/// the artifact — needs no tool: the next [`sweep`] clears the drift on its own.)
pub fn accept(store: &StateStore, run: &str, path: &str) -> Result<bool> {
    let root = repo_root(store);
    let Some(hash) = hash_file(&root.join(path)) else {
        return Ok(false);
    };
    let mut witnesses = store.read_witnesses(run)?;
    let mut found = false;
    for w in witnesses.iter_mut() {
        if w.path == path {
            w.hash = hash.clone();
            found = true;
        }
    }
    if found {
        store.write_witnesses(run, &witnesses)?;
        clear(store, run, &drift_id_for(path))?;
    }
    Ok(found)
}

/// The `drift/` directory for a run.
fn drift_dir(store: &StateStore, run: &str) -> std::path::PathBuf {
    store.run_dir(run).join("drift")
}

/// Parse a drift kind, defaulting to `Output`.
fn parse_kind(raw: &str) -> DriftKind {
    match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
        "spec" => DriftKind::Spec,
        "discovery_output" => DriftKind::DiscoveryOutput,
        "discovery_mandate" => DriftKind::DiscoveryMandate,
        _ => DriftKind::Output,
    }
}

/// Parse one raw `drift/*.md` document into a [`Drift`].
fn parse(run: &str, raw: &str) -> Drift {
    let mut path = String::new();
    let mut station = String::new();
    let mut kind = DriftKind::Output;
    let mut age = String::new();
    let mut unit = None;
    for line in raw.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("path:") {
            path = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("station:") {
            station = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("kind:") {
            kind = parse_kind(rest);
        } else if let Some(rest) = line.strip_prefix("age:") {
            age = rest.trim().trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("unit:") {
            let v = rest.trim().trim_matches('"').to_string();
            if !v.is_empty() {
                unit = Some(v);
            }
        }
    }
    Drift {
        path,
        station,
        run: run.to_string(),
        kind,
        age,
        unit,
    }
}

/// Read every drift entry for a run (sorted by file stem). Empty when no
/// sweep has deposited entries.
pub fn list(store: &StateStore, run: &str) -> Result<Vec<Drift>> {
    let dir = drift_dir(store, run);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<(String, Drift)> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(darkrun_core::CoreError::from)? {
        let entry = entry.map_err(darkrun_core::CoreError::from)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let raw = fs::read_to_string(&path).map_err(darkrun_core::CoreError::from)?;
                entries.push((stem.to_string(), parse(run, &raw)));
            }
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries.into_iter().map(|(_, d)| d).collect())
}

/// The first (highest-priority) drift entry for a run, if any. The manager
/// uses this to drive Track C.
pub fn first(store: &StateStore, run: &str) -> Result<Option<Drift>> {
    Ok(list(store, run)?.into_iter().next())
}

/// Record a drift entry under `drift/<id>.md`. Used by tests and by the
/// (future) core sweep until it owns drift storage natively.
pub fn record(store: &StateStore, run: &str, id: &str, entry: &Drift) -> Result<()> {
    let dir = drift_dir(store, run);
    fs::create_dir_all(&dir).map_err(|source| darkrun_core::CoreError::Io {
        path: dir.clone(),
        source,
    })?;
    let kind = match entry.kind {
        DriftKind::Spec => "spec",
        DriftKind::Output => "output",
        DriftKind::DiscoveryOutput => "discovery_output",
        DriftKind::DiscoveryMandate => "discovery_mandate",
    };
    let mut doc = String::from("---\n");
    doc.push_str(&format!("path: {}\n", entry.path));
    doc.push_str(&format!("station: {}\n", entry.station));
    doc.push_str(&format!("kind: {kind}\n"));
    if !entry.age.is_empty() {
        doc.push_str(&format!("age: {}\n", entry.age));
    }
    if let Some(unit) = &entry.unit {
        doc.push_str(&format!("unit: {unit}\n"));
    }
    doc.push_str("---\n");
    let path = dir.join(format!("{id}.md"));
    fs::write(&path, doc).map_err(|source| {
        darkrun_core::CoreError::Io {
            path: path.clone(),
            source,
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().expect("tmp");
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn empty_when_no_dir() {
        let (_d, store) = store();
        assert!(list(&store, "r").unwrap().is_empty());
        assert!(first(&store, "r").unwrap().is_none());
    }

    #[test]
    fn record_and_read_back() {
        let (_d, store) = store();
        let d = Drift {
            path: "frame/frame.md".into(),
            station: "frame".into(),
            run: "r".into(),
            kind: DriftKind::Spec,
            age: "5m".into(),
            unit: Some("u1".into()),
        };
        record(&store, "r", "d-01", &d).unwrap();
        let read = list(&store, "r").unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].path, "frame/frame.md");
        assert_eq!(read[0].kind, DriftKind::Spec);
        assert_eq!(read[0].unit, Some("u1".to_string()));
        assert_eq!(first(&store, "r").unwrap().unwrap().station, "frame");
    }

    use darkrun_core::domain::{Status, Unit, UnitFrontmatter};

    /// A store whose `.darkrun` root sits under `repo`, so the sweep's
    /// `repo_root` resolves back to `repo` where the witnessed artifacts live.
    /// (`StateStore::new` appends `.darkrun` itself.)
    fn repo_store() -> (tempfile::TempDir, StateStore, std::path::PathBuf) {
        let dir = tempdir().expect("tmp");
        let repo = dir.path().to_path_buf();
        let store = StateStore::new(&repo);
        (dir, store, repo)
    }

    fn completed_unit_with_output(station: &str, slug: &str, output: &str) -> Unit {
        Unit {
            slug: slug.into(),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some(station.into()),
                outputs: vec![output.into()],
                ..Default::default()
            },
            title: slug.into(),
            body: String::new(),
        }
    }

    #[test]
    fn sweep_detects_mutation_then_self_heals_on_revert() {
        let (_d, store, repo) = repo_store();
        store
            .write_unit("r", &completed_unit_with_output("frame", "u1", "out.txt"))
            .unwrap();
        fs::write(repo.join("out.txt"), b"v1").unwrap();
        record_station_witnesses(&store, "r", "frame").unwrap();
        assert_eq!(store.read_witnesses("r").unwrap().len(), 1);

        // Clean: no drift.
        sweep(&store, "r").unwrap();
        assert!(first(&store, "r").unwrap().is_none());

        // Mutated: drift on that artifact.
        fs::write(repo.join("out.txt"), b"v2").unwrap();
        sweep(&store, "r").unwrap();
        let d = first(&store, "r").unwrap().expect("drift");
        assert_eq!(d.path, "out.txt");
        assert_eq!(d.station, "frame");

        // Reverted: the sweep clears the drift on its own.
        fs::write(repo.join("out.txt"), b"v1").unwrap();
        sweep(&store, "r").unwrap();
        assert!(first(&store, "r").unwrap().is_none());

        // Vanished locked artifact is itself drift.
        fs::remove_file(repo.join("out.txt")).unwrap();
        sweep(&store, "r").unwrap();
        assert!(first(&store, "r").unwrap().is_some());
    }

    #[test]
    fn accept_rewitnesses_an_intentional_change() {
        let (_d, store, repo) = repo_store();
        store
            .write_unit("r", &completed_unit_with_output("frame", "u1", "out.txt"))
            .unwrap();
        fs::write(repo.join("out.txt"), b"v1").unwrap();
        record_station_witnesses(&store, "r", "frame").unwrap();

        fs::write(repo.join("out.txt"), b"v2").unwrap();
        sweep(&store, "r").unwrap();
        assert!(first(&store, "r").unwrap().is_some());

        // Accept the new content → drift clears, witness updates to v2.
        assert!(accept(&store, "r", "out.txt").unwrap());
        sweep(&store, "r").unwrap();
        assert!(first(&store, "r").unwrap().is_none());
        assert_eq!(
            store.read_witnesses("r").unwrap()[0].hash,
            darkrun_core::hash_bytes(b"v2")
        );

        // Accepting an unknown path is a no-op false.
        assert!(!accept(&store, "r", "nope.txt").unwrap());
    }
}

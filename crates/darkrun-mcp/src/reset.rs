//! Reset a station or a whole Run (the `darkrun-reset` skill). Destructive, so
//! it's a dry run by default — it reports exactly what *would* be wiped — and
//! only deletes when `confirm` is set.

use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::Result;

/// What a reset wiped (or, on a dry run, would wipe).
#[derive(Debug, Clone, Serialize)]
pub struct ResetPlan {
    /// `station` or `run`.
    pub scope: String,
    /// The run slug.
    pub run: String,
    /// The station (empty for a run-scope reset).
    pub station: String,
    /// The units that were/would be removed.
    pub units: Vec<String>,
    /// Whether the wipe was actually performed (vs a dry run).
    pub confirmed: bool,
    /// Human-readable summary / next step.
    pub note: String,
}

/// Reset a station (re-enter it from Spec) or, with no station, the whole Run.
/// Performs nothing unless `confirm` is true.
pub fn reset(
    store: &StateStore,
    slug: &str,
    station: Option<&str>,
    confirm: bool,
) -> Result<ResetPlan> {
    let units_dir = store.units_dir(slug);
    match station {
        Some(station) => {
            let units: Vec<String> = store
                .read_units(slug)?
                .into_iter()
                .filter(|u| u.station() == station)
                .map(|u| u.slug)
                .collect();
            if confirm {
                for u in &units {
                    let path = units_dir.join(format!("{u}.md"));
                    if path.exists() {
                        std::fs::remove_file(&path).map_err(darkrun_core::CoreError::from)?;
                    }
                }
                // Drop the station's state entry so the next tick re-enters it
                // at its Spec phase.
                if let Some(mut state) = store.read_state(slug)? {
                    state.stations.remove(station);
                    store.write_state(slug, &state)?;
                }
            }
            if confirm {
                let _ = crate::commit::commit_state(
                    store,
                    &format!("darkrun: reset station {station}"),
                );
            }
            let note = if confirm {
                format!("Wiped station `{station}` ({} unit(s)). Call darkrun_advance — the next tick re-enters it at Spec.", units.len())
            } else {
                format!("Dry run: would wipe station `{station}` and {} unit(s). Re-call with confirm:true to apply.", units.len())
            };
            Ok(ResetPlan {
                scope: "station".to_string(),
                run: slug.to_string(),
                station: station.to_string(),
                units,
                confirmed: confirm,
                note,
            })
        }
        None => {
            let units: Vec<String> = store.read_units(slug)?.into_iter().map(|u| u.slug).collect();
            if confirm {
                let dir = store.run_dir(slug);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).map_err(darkrun_core::CoreError::from)?;
                }
            }
            if confirm {
                let _ = crate::commit::commit_state(store, &format!("darkrun: reset run {slug}"));
            }
            let note = if confirm {
                format!("Wiped the entire run `{slug}` ({} unit(s) and all state).", units.len())
            } else {
                format!("Dry run: would wipe the ENTIRE run `{slug}` ({} unit(s) and all state). Re-call with confirm:true to apply.", units.len())
            };
            Ok(ResetPlan {
                scope: "run".to_string(),
                run: slug.to_string(),
                station: String::new(),
                units,
                confirmed: confirm,
                note,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::run_start;
    use darkrun_core::domain::{Mode, Status, Unit, UnitFrontmatter};

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    fn seed_unit(store: &StateStore, slug: &str, station: &str, unit: &str) {
        let u = Unit {
            slug: unit.into(),
            frontmatter: UnitFrontmatter {
                status: Status::Completed,
                station: Some(station.into()),
                ..Default::default()
            },
            title: unit.into(),
            body: String::new(),
        };
        store.write_unit(slug, &u).unwrap();
    }

    #[test]
    fn station_dry_run_then_confirmed_wipe() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        seed_unit(&store, "r", "frame", "frame-u");
        seed_unit(&store, "r", "specify", "specify-u");

        // Dry run touches nothing.
        let plan = reset(&store, "r", Some("frame"), false).unwrap();
        assert_eq!(plan.scope, "station");
        assert_eq!(plan.units, vec!["frame-u".to_string()]);
        assert!(!plan.confirmed);
        assert!(store.read_unit("r", "frame-u").is_ok());

        // Confirmed wipe removes only frame's unit.
        let plan = reset(&store, "r", Some("frame"), true).unwrap();
        assert!(plan.confirmed);
        assert!(store.read_unit("r", "frame-u").is_err());
        assert!(store.read_unit("r", "specify-u").is_ok());
    }

    #[test]
    fn run_scope_confirmed_removes_the_run_dir() {
        let (_d, store) = store();
        run_start(&store, "r", "software", None, Mode::Solo, "full").unwrap();
        seed_unit(&store, "r", "frame", "frame-u");
        assert!(store.run_dir("r").exists());

        let plan = reset(&store, "r", None, false).unwrap();
        assert_eq!(plan.scope, "run");
        assert!(store.run_dir("r").exists()); // dry run

        reset(&store, "r", None, true).unwrap();
        assert!(!store.run_dir("r").exists());
    }
}

//! Runs browse API — the wire contract for the `GET /api/runs` list and
//! `GET /api/runs/:slug` detail endpoints the desktop app (and any browse
//! surface) reads to render a project's runs without pulling every document.
//!
//! [`RunSummary`] is the compact per-run row for the list view: identity
//! (slug / title / factory), live position (active station + phase), lifecycle
//! status, and a station-progress fraction (completed / total). The list is
//! returned as a [`RunListPayload`]; a single run's expanded view — every
//! station and the units that sit on the active station — is a
//! [`RunDetailPayload`].
//!
//! Dependency-light by design (only `serde` + `schemars`): the HTTP server
//! projects these out of the on-disk `.darkrun/` state; nothing here imports
//! the domain crate, so a downstream consumer can deserialize a browse
//! response without pulling the engine in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Station-progress fraction for a run: how many stations have completed out of
/// the total the run walks. Surfaced as a small object so the SPA can render a
/// `3 / 6` chip (or a bar) without recomputing it client-side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StationProgress {
    /// Stations that have reached `completed`.
    pub completed: u32,
    /// Total stations the run walks.
    pub total: u32,
}

/// A compact summary of a single run for the browse list.
///
/// Mirrors the engine's `run_list`-style projection: identity, the live
/// position (`active_station` + `phase`), lifecycle `status`, station
/// `progress`, and the `started_at` timestamp. `title` is always resolved
/// (falling back to the slug when no explicit title was set); `phase` is
/// nullable for a run sitting between stations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunSummary {
    /// The run slug — the stable id used in every run-scoped route.
    pub slug: String,
    /// Resolved display title (falls back to the slug).
    pub title: String,
    /// The factory driving the run (e.g. `software`).
    pub factory: String,
    /// The station the run currently sits on.
    pub active_station: String,
    /// The active phase within the active station, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Lifecycle status (display string, e.g. `active` / `completed`).
    pub status: String,
    /// Station progress (completed / total).
    pub progress: StationProgress,
    /// RFC3339 start timestamp, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

/// `GET /api/runs` response body: every (non-archived) run on the project as a
/// summary, sorted by slug.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunListPayload {
    /// The run summaries, sorted by slug.
    pub runs: Vec<RunSummary>,
    /// Convenience count of `runs`.
    pub count: usize,
}

impl RunListPayload {
    /// Build a list payload from summaries, stamping the `count` from the
    /// vector length so the two can never drift.
    pub fn new(runs: Vec<RunSummary>) -> Self {
        let count = runs.len();
        RunListPayload { runs, count }
    }
}

/// A station row in a run's detail view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunDetailStation {
    /// Station name (e.g. `frame`, `build`).
    pub name: String,
    /// Lifecycle status (display string).
    pub status: String,
    /// Current phase within the station, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// RFC3339 start timestamp, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// RFC3339 completion timestamp, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// A unit row in a run's detail view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunDetailUnit {
    /// The unit slug.
    pub slug: String,
    /// The unit title.
    pub title: String,
    /// Lifecycle status (display string).
    pub status: String,
    /// The station the unit belongs to, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<String>,
}

/// `GET /api/runs/:slug` response body: a single run's expanded view — its
/// identity, the live position, every station it walks, and the units that
/// sit on the active station.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunDetailPayload {
    /// The run slug.
    pub slug: String,
    /// Resolved display title.
    pub title: String,
    /// The factory driving the run.
    pub factory: String,
    /// The station the run currently sits on.
    pub active_station: String,
    /// The active phase, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Lifecycle status (display string).
    pub status: String,
    /// Station progress (completed / total).
    pub progress: StationProgress,
    /// RFC3339 start timestamp, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Every station the run walks, in declared order.
    pub stations: Vec<RunDetailStation>,
    /// The units on the active station.
    pub units: Vec<RunDetailUnit>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary() -> RunSummary {
        RunSummary {
            slug: "alpha".into(),
            title: "Alpha".into(),
            factory: "software".into(),
            active_station: "frame".into(),
            phase: Some("spec".into()),
            status: "active".into(),
            progress: StationProgress {
                completed: 2,
                total: 6,
            },
            started_at: Some("2026-05-30T00:00:00Z".into()),
        }
    }

    #[test]
    fn summary_roundtrips_and_carries_fields() {
        let s = summary();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["slug"], "alpha");
        assert_eq!(v["title"], "Alpha");
        assert_eq!(v["factory"], "software");
        assert_eq!(v["active_station"], "frame");
        assert_eq!(v["phase"], "spec");
        assert_eq!(v["status"], "active");
        assert_eq!(v["progress"]["completed"], 2);
        assert_eq!(v["progress"]["total"], 6);
        assert_eq!(v["started_at"], "2026-05-30T00:00:00Z");

        let back: RunSummary = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn summary_omits_none_optionals() {
        let s = RunSummary {
            phase: None,
            started_at: None,
            ..summary()
        };
        let v = serde_json::to_value(&s).unwrap();
        assert!(v.get("phase").is_none());
        assert!(v.get("started_at").is_none());
        // Required fields stay present.
        assert!(v.get("status").is_some());
        assert!(v.get("progress").is_some());
    }

    #[test]
    fn list_payload_stamps_count() {
        let payload = RunListPayload::new(vec![summary(), summary()]);
        assert_eq!(payload.count, 2);
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["runs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn empty_list_payload_roundtrips() {
        let payload = RunListPayload::new(vec![]);
        assert_eq!(payload.count, 0);
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["count"], 0);
        assert!(v["runs"].as_array().unwrap().is_empty());
        let back: RunListPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn detail_payload_roundtrips() {
        let payload = RunDetailPayload {
            slug: "alpha".into(),
            title: "Alpha".into(),
            factory: "software".into(),
            active_station: "frame".into(),
            phase: Some("manufacture".into()),
            status: "active".into(),
            progress: StationProgress {
                completed: 1,
                total: 3,
            },
            started_at: Some("2026-05-30T00:00:00Z".into()),
            stations: vec![
                RunDetailStation {
                    name: "frame".into(),
                    status: "active".into(),
                    phase: Some("manufacture".into()),
                    started_at: Some("2026-05-30T00:00:00Z".into()),
                    completed_at: None,
                },
                RunDetailStation {
                    name: "build".into(),
                    status: "pending".into(),
                    phase: None,
                    started_at: None,
                    completed_at: None,
                },
            ],
            units: vec![RunDetailUnit {
                slug: "u-1".into(),
                title: "First Unit".into(),
                status: "active".into(),
                station: Some("frame".into()),
            }],
        };
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["slug"], "alpha");
        assert_eq!(v["stations"][0]["name"], "frame");
        assert_eq!(v["stations"][1]["status"], "pending");
        assert!(v["stations"][1].get("phase").is_none());
        assert_eq!(v["units"][0]["slug"], "u-1");
        assert_eq!(v["units"][0]["station"], "frame");

        let back: RunDetailPayload = serde_json::from_value(v).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn detail_unit_omits_none_station() {
        let u = RunDetailUnit {
            slug: "u".into(),
            title: "U".into(),
            status: "pending".into(),
            station: None,
        };
        let v = serde_json::to_value(&u).unwrap();
        assert!(v.get("station").is_none());
    }

    #[test]
    fn progress_defaults_to_zero() {
        let p = StationProgress::default();
        assert_eq!(p.completed, 0);
        assert_eq!(p.total, 0);
        let v = serde_json::to_value(p).unwrap();
        assert_eq!(v, json!({ "completed": 0, "total": 0 }));
    }

    #[test]
    fn schemas_generate_with_titles() {
        for (val, title) in [
            (
                serde_json::to_value(schemars::schema_for!(RunSummary)).unwrap(),
                "RunSummary",
            ),
            (
                serde_json::to_value(schemars::schema_for!(RunListPayload)).unwrap(),
                "RunListPayload",
            ),
            (
                serde_json::to_value(schemars::schema_for!(RunDetailPayload)).unwrap(),
                "RunDetailPayload",
            ),
        ] {
            assert_eq!(val["title"], title);
        }
    }
}

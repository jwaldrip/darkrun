//! Runs browse handlers — `GET /api/runs` and `GET /api/runs/:slug`.
//!
//! These project the on-disk `.darkrun/` state into the
//! [`darkrun_api`] browse payloads, mirroring darkrun-mcp's `run_list`-style
//! listing but reading straight off [`darkrun_core::StateStore`] so the HTTP
//! crate stays free of an engine dependency:
//!
//! - `GET /api/runs` — every non-archived run as a [`RunSummary`], sorted by
//!   slug, wrapped in a [`RunListPayload`].
//! - `GET /api/runs/:slug` — a single run's [`RunDetailPayload`]: identity,
//!   live position, every station it walks, and the units sitting on the active
//!   station. `404` when the run is unknown.
//!
//! The display strings (`status`, `phase`) come from the domain enums' serde
//! representation, so they stay in lockstep with the wire contract without a
//! hand-maintained match.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use darkrun_api::{
    RunDetailPayload, RunDetailStation, RunDetailUnit, RunListPayload, RunSummary, StationProgress,
};
use darkrun_core::domain::{Run, Station, StationPhase, Status, Unit};
use darkrun_core::state::RunState;
use serde_json::json;

use crate::state::AppState;

/// Render a `serde`-enum value (e.g. [`Status`]) to its wire string. Falls back
/// to an empty string if the value did not serialize to a bare JSON string —
/// which the domain enums never do.
fn wire_string<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The display string for a station phase.
fn phase_string(phase: StationPhase) -> String {
    wire_string(&phase)
}

/// Compute station progress (completed / total) from a run's derived state.
/// `None` when no state has been written yet — the run is counted as having no
/// stations.
fn progress_from_state(state: Option<&RunState>) -> StationProgress {
    let Some(state) = state else {
        return StationProgress::default();
    };
    let total = state.stations.len() as u32;
    let completed = state
        .stations
        .values()
        .filter(|s| s.status == Status::Completed)
        .count() as u32;
    StationProgress { completed, total }
}

/// The active station's phase, if the run's state records one for it.
fn active_phase(run: &Run, state: Option<&RunState>) -> Option<String> {
    let station = &run.frontmatter.active_station;
    state
        .and_then(|s| s.stations.get(station))
        .map(|s| phase_string(s.phase))
}

/// Project a [`Run`] (+ its derived state, if present) into a [`RunSummary`].
fn summarize(run: &Run, state: Option<&RunState>) -> RunSummary {
    RunSummary {
        slug: run.slug.clone(),
        title: run.title.clone(),
        factory: run.frontmatter.factory.clone(),
        active_station: run.frontmatter.active_station.clone(),
        phase: active_phase(run, state),
        status: wire_string(&run.frontmatter.status),
        progress: progress_from_state(state),
        started_at: run.frontmatter.started_at.clone(),
    }
}

/// Project a single derived [`Station`] into a detail row.
fn detail_station(station: &Station) -> RunDetailStation {
    RunDetailStation {
        name: station.station.clone(),
        status: wire_string(&station.status),
        phase: Some(phase_string(station.phase)),
        started_at: station.started_at.clone(),
        completed_at: station.completed_at.clone(),
    }
}

/// Project a [`Unit`] into a detail row.
fn detail_unit(unit: &Unit) -> RunDetailUnit {
    RunDetailUnit {
        slug: unit.slug.clone(),
        title: unit.title.clone(),
        status: wire_string(&unit.frontmatter.status),
        station: unit.frontmatter.station.clone(),
    }
}

/// `GET /api/runs` — list the project's runs as summaries, sorted by slug.
///
/// Archived runs are omitted (mirroring the engine's default list view). Runs
/// whose document fails to parse are skipped rather than failing the whole
/// list, so one corrupt sidecar never blanks the browse view.
pub async fn list_runs(State(state): State<AppState>) -> Response {
    let store = &state.store;
    let mut summaries = Vec::new();

    if let Ok(slugs) = store.list_runs() {
        for slug in slugs {
            let Ok(run) = store.read_run(&slug) else {
                continue;
            };
            if run.frontmatter.archived.unwrap_or(false) {
                continue;
            }
            let state = store.read_state(&slug).ok().flatten();
            summaries.push(summarize(&run, state.as_ref()));
        }
    }

    // `list_runs` already returns slugs sorted, but re-sort defensively so the
    // wire order is a guaranteed property regardless of the store's ordering.
    summaries.sort_by(|a, b| a.slug.cmp(&b.slug));

    (StatusCode::OK, Json(RunListPayload::new(summaries))).into_response()
}

/// `GET /api/runs/:slug` — a single run's detail: identity, live position,
/// every station it walks, and the units on the active station. `404` when no
/// such run exists.
pub async fn get_run(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    let store = &state.store;
    let Ok(run) = store.read_run(&slug) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "run not found", "id": slug })),
        )
            .into_response();
    };

    let state = store.read_state(&slug).ok().flatten();

    // Stations in declared order: the derived state's BTreeMap is keyed by
    // station name, but the per-station `started_at` (when present) gives the
    // true walk order. Sort by start time, falling back to name for stations
    // not yet started so the ordering is total and deterministic.
    let mut stations: Vec<RunDetailStation> = state
        .as_ref()
        .map(|s| s.stations.values().map(detail_station).collect())
        .unwrap_or_default();
    stations.sort_by(|a, b| match (&a.started_at, &b.started_at) {
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.name.cmp(&b.name)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name),
    });

    // Units on the active station only.
    let active = &run.frontmatter.active_station;
    let mut units: Vec<RunDetailUnit> = store
        .read_units(&slug)
        .unwrap_or_default()
        .iter()
        .filter(|u| u.station() == active)
        .map(detail_unit)
        .collect();
    units.sort_by(|a, b| a.slug.cmp(&b.slug));

    let payload = RunDetailPayload {
        slug: run.slug.clone(),
        title: run.title.clone(),
        factory: run.frontmatter.factory.clone(),
        active_station: run.frontmatter.active_station.clone(),
        phase: active_phase(&run, state.as_ref()),
        status: wire_string(&run.frontmatter.status),
        progress: progress_from_state(state.as_ref()),
        started_at: run.frontmatter.started_at.clone(),
        stations,
        units,
    };

    (StatusCode::OK, Json(payload)).into_response()
}

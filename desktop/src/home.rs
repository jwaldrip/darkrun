//! The run-browser HOME screen.
//!
//! On launch with **no** pinned `DARKRUN_SESSION_ID`, the app opens here instead
//! of the live Review. [`HomeApp`] GETs `http://127.0.0.1:{port}/api/runs` over
//! the existing hand-rolled wire client, projects each [`darkrun_api::RunSummary`]
//! into a [`RunCardData`], and renders the shared [`RunList`].
//!
//! Clicking a run resolves its session — preferring the run's own `:slug` route
//! and falling back to the run slug — and swaps the surface to the live
//! [`crate::review::ReviewApp`] connected to that session. A back action returns
//! to the browser.
//!
//! The screen degrades gracefully: while the list is loading it shows a spinner
//! line; when the engine is unreachable it shows a clear "start `darkrun serve`"
//! message; an empty (but reachable) engine shows a friendly "no runs yet" note.

use darkrun_ui::prelude::*;

use crate::map;
use crate::review::ReviewApp;
use crate::wire::{self, ConnConfig};

/// The load state of the run list.
#[derive(Clone, PartialEq)]
enum Load {
    /// The `/api/runs` GET is in flight.
    Loading,
    /// The list loaded (possibly empty).
    Loaded(Vec<RunCardData>),
    /// The GET failed — the engine is likely not running. Carries the reason.
    Failed(String),
}

/// The home surface: either the run browser or, once a run is opened, that run's
/// live Review with a back affordance.
#[component]
pub fn HomeApp(cfg: ConnConfig) -> Element {
    // The session id of the run currently opened into Review, if any.
    let opened = use_signal(|| None::<String>);

    let shell = "padding:24px;display:flex;flex-direction:column;gap:16px;\
                 max-width:880px;margin:0 auto;";

    // When a run is opened, render its live Review pointed at that session.
    if let Some(session) = opened.read().clone() {
        let run_cfg = cfg.with_session(session);
        let mut opened = opened;
        return rsx! {
            div { style: "{shell}",
                div { style: "display:flex;",
                    Button {
                        variant: ButtonVariant::Ghost,
                        tone: Tone::Neutral,
                        on_click: move |_| opened.set(None),
                        "\u{2190} all runs"
                    }
                }
            }
            ReviewApp { cfg: run_cfg }
        };
    }

    rsx! {
        div { style: "{shell}",
            header {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;",
                Wordmark { variant: WordmarkVariant::Filled, size: 28.0 }
                Badge { tone: Tone::Neutral, "runs" }
            }
            RunBrowser { cfg: cfg.clone(), opened }
        }
    }
}

/// Loads `/api/runs` and renders the [`RunList`]. A button on each card opens
/// the run by writing its session id into `opened`.
#[component]
fn RunBrowser(cfg: ConnConfig, opened: Signal<Option<String>>) -> Element {
    let mut state = use_signal(|| Load::Loading);

    // Fetch the run list once on mount.
    let fetch_cfg = cfg.clone();
    use_future(move || {
        let cfg = fetch_cfg.clone();
        async move {
            state.set(Load::Loading);
            match wire::fetch_runs(&cfg).await {
                Ok(payload) => {
                    let cards: Vec<RunCardData> = payload.runs.iter().map(map::run_card).collect();
                    state.set(Load::Loaded(cards));
                }
                Err(e) => state.set(Load::Failed(e.to_string())),
            }
        }
    });

    let mut opened = opened;
    let open = move |slug: String| {
        // The run slug is the session id the engine serves the live feed under.
        opened.set(Some(slug));
    };

    let current = state.read().clone();
    match current {
        Load::Loading => rsx! {
            Card {
                p { style: "color:var(--dr-text-muted);margin:0;",
                    "Loading runs from the local engine\u{2026}"
                }
            }
        },
        Load::Failed(reason) => rsx! { EngineDown { reason } },
        Load::Loaded(cards) => rsx! {
            RunList {
                runs: cards,
                on_select: open,
                empty: rsx! { NoRuns {} },
            }
        },
    }
}

/// Shown when `/api/runs` could not be reached — the engine is almost certainly
/// not running. Tells the operator exactly how to start it.
#[component]
fn EngineDown(reason: String) -> Element {
    rsx! {
        Card { accent: Tone::Danger.color().to_string(),
            h2 {
                style: "margin:0 0 8px;font-family:var(--dr-font-sans);\
                        font-size:15px;font-weight:700;color:var(--dr-text);",
                "No engine running"
            }
            p { style: "margin:0 0 10px;color:var(--dr-text-muted);font-size:13px;",
                "Couldn't reach the local engine to list runs. Start it, then this \
                 screen will fill in."
            }
            pre {
                style: "margin:0;padding:10px 12px;border-radius:6px;\
                        background:var(--dr-surface-raised);border:1px solid var(--dr-border);\
                        font-family:var(--dr-font-mono);font-size:13px;color:var(--dr-accent);",
                "darkrun serve"
            }
            p {
                style: "margin:10px 0 0;font-family:var(--dr-font-mono);\
                        font-size:11px;color:var(--dr-text-faint);",
                "{reason}"
            }
        }
    }
}

/// Shown when the engine is reachable but has no runs yet.
#[component]
fn NoRuns() -> Element {
    rsx! {
        Card {
            h2 {
                style: "margin:0 0 8px;font-family:var(--dr-font-sans);\
                        font-size:15px;font-weight:700;color:var(--dr-text);",
                "No runs yet"
            }
            p { style: "margin:0;color:var(--dr-text-muted);font-size:13px;",
                "The engine is up but hasn't started any runs. Kick one off and it'll \
                 show up here."
            }
        }
    }
}

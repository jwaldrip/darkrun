//! `/browse` — the web viewer for a *published / remote* workspace.
//!
//! The website browses runs read-only: point it at a repository and it renders
//! that workspace's runs, stations, units, and artifacts in the browser. There
//! is deliberately **no local-folder picker here** — picking and opening a local
//! workspace (and reviewing it) is the **desktop app's** job. The agent never
//! opens a browser; the desktop app is the only interactive surface it drives.
//!
//! The engine's HTTP/WS contract (`darkrun_api::ROUTES`) is listed as reference.

use darkrun_api::{HttpMethod, ROUTES};
use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::ui::SectionHead;

/// `/browse` — view a published/remote darkrun workspace in the browser.
#[component]
pub fn Browse() -> Element {
    rsx! {
        SectionHead {
            kicker: "browse a workspace".to_string(),
            title: "Browse a published workspace".to_string(),
            lead: Some(
                "View a darkrun workspace's runs, stations, units, and artifacts read-only, \
                 right in your browser. Point at a repository to render its published workspace."
                    .to_string(),
            ),
        }

        RemoteRepo {}

        DesktopForLocal {}

        // Reference: the live-engine contract the desktop review app speaks.
        div { style: "margin-top:32px;",
            h2 {
                style: format!(
                    "font-family:{};font-size:18px;color:{};margin:0 0 6px;",
                    tokens::FONT_SANS, theme::TEXT,
                ),
                "The engine contract"
            }
            p {
                style: format!(
                    "font-family:{};font-size:14px;color:{};margin:0 0 18px;max-width:62ch;",
                    tokens::FONT_SANS, theme::TEXT_MUTED,
                ),
                "For reference: the HTTP / WS routes a running engine exposes. The desktop app \
                 reads "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, theme::ACCENT),
                    "GET /api/runs"
                }
                " and streams each run over the session socket."
            }
        }

        div { style: "display:flex;flex-direction:column;gap:6px;",
            for spec in ROUTES.iter() {
                RouteRow {
                    method: format!("{:?}", spec.method),
                    is_ws: spec.method == HttpMethod::Ws,
                    path: spec.path_template.to_string(),
                    summary: spec.summary.to_string(),
                    tag: spec.tag.to_string(),
                }
            }
        }
    }
}

/// The remote-repository entry: paste a repo URL to render its published workspace.
#[component]
fn RemoteRepo() -> Element {
    let card = format!(
        "border:1px solid {border};border-radius:12px;padding:18px 20px;background:{raised};",
        border = theme::BORDER,
        raised = theme::SURFACE_RAISED,
    );
    let input = format!(
        "flex:1;font-family:{mono};font-size:13px;color:{text};background:{surface};\
         border:1px solid {border};border-radius:8px;padding:9px 12px;",
        mono = tokens::FONT_MONO,
        text = theme::TEXT,
        surface = theme::SURFACE_BASE,
        border = theme::BORDER,
    );
    let btn = format!(
        "font-family:{sans};font-size:13px;font-weight:600;color:{on};background:{accent};\
         border:none;border-radius:8px;padding:9px 16px;cursor:pointer;",
        sans = tokens::FONT_SANS,
        on = theme::ON_ACCENT,
        accent = theme::ACCENT,
    );
    rsx! {
        div { style: "{card}",
            div {
                style: format!(
                    "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{};margin-bottom:10px;",
                    tokens::FONT_MONO, theme::TEXT_FAINT,
                ),
                "browse a remote repository"
            }
            div { style: "display:flex;gap:10px;align-items:center;",
                input {
                    style: "{input}",
                    r#type: "text",
                    placeholder: "github.com/org/repo or gitlab.com/group/project",
                    "data-darkrun-remote": "true",
                }
                button { style: "{btn}", "data-darkrun-remote-go": "true", "Browse" }
            }
            p {
                style: format!("font-family:{};font-size:13px;color:{};margin:12px 0 0;", tokens::FONT_SANS, theme::TEXT_MUTED),
                "Renders the repo's "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, theme::ACCENT),
                    ".darkrun/"
                }
                " workspace read-only \u{2014} a shareable link to a run's shape."
            }
        }
    }
}

/// A note pointing local browsing and review at the desktop app.
#[component]
fn DesktopForLocal() -> Element {
    let wrap = format!(
        "margin-top:18px;border:1px solid {border};border-left:3px solid {accent};border-radius:8px;\
         padding:12px 16px;background:{overlay};",
        border = theme::BORDER,
        accent = theme::ACCENT,
        overlay = theme::SURFACE_OVERLAY,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:8px;",
                Badge { tone: Tone::Accent, filled: true, "desktop app" }
                Badge { tone: Tone::Neutral, "your local runs" }
            }
            p {
                style: format!("font-family:{};font-size:13px;color:{};margin:0;", tokens::FONT_SANS, theme::TEXT_MUTED),
                "To browse and review your own runs, open the darkrun desktop app \u{2014} run "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, theme::ACCENT),
                    "darkrun serve"
                }
                ". It picks your local workspace and opens any run into its live review on your \
                 machine. The web browse here is read-only and never touches your local files."
            }
        }
    }
}

/// One route descriptor row.
#[component]
fn RouteRow(method: String, is_ws: bool, path: String, summary: String, tag: String) -> Element {
    let tone = if is_ws { Tone::Accent } else { Tone::Info };
    let row = format!(
        "display:flex;align-items:center;gap:12px;padding:8px 12px;\
         border:1px solid {border};border-radius:8px;background:{raised};",
        border = theme::BORDER,
        raised = theme::SURFACE_RAISED,
    );
    rsx! {
        div { style: "{row}",
            span { style: "min-width:58px;", Badge { tone, filled: true, "{method}" } }
            code {
                style: format!("font-family:{};font-size:13px;color:{};min-width:240px;", tokens::FONT_MONO, theme::TEXT),
                "{path}"
            }
            span {
                style: format!("font-family:{};font-size:13px;color:{};flex:1;", tokens::FONT_SANS, theme::TEXT_MUTED),
                "{summary}"
            }
            Badge { tone: Tone::Neutral, "{tag}" }
        }
    }
}

//! `/browse` — the web workspace explorer.
//!
//! Browsing runs in the **website**, not the desktop app: you open a project's
//! `.darkrun/` workspace and explore its runs, stations, units, and artifacts —
//! read-only, in the browser. (Review — the interactive annotate/decide surface
//! — stays in the desktop app so it never takes over your browser.)
//!
//! Two entry points, mirroring how a workspace reaches the browser:
//! - **Local folder** — pick a directory containing a `.darkrun/` folder; the
//!   files are read client-side, nothing leaves your machine.
//! - **Remote repository** — paste a repo URL to browse its workspace.
//!
//! The engine's HTTP/WS contract (`darkrun_api::ROUTES`) is listed as reference
//! for the live-engine path.

use darkrun_api::{HttpMethod, ROUTES};
use darkrun_ui::prelude::*;

use crate::ui::SectionHead;

/// `/browse` — open a darkrun workspace and explore its runs, in the browser.
#[component]
pub fn Browse() -> Element {
    rsx! {
        SectionHead {
            kicker: "browse a workspace".to_string(),
            title: "Browse a darkrun workspace".to_string(),
            lead: Some(
                "Explore runs, stations, units, and artifacts from any darkrun workspace \u{2014} \
                 read-only, right in your browser. Open a local project folder or point at a \
                 remote repository."
                    .to_string(),
            ),
        }

        // Entry point 1 — a local project folder, read client-side.
        Dropzone {}

        // Entry point 2 — a remote repository.
        RemoteRepo {}

        ClientSideNote {}

        // Reference: the live-engine contract the desktop review app speaks.
        div { style: "margin-top:32px;",
            h2 {
                style: format!(
                    "font-family:{};font-size:18px;color:{};margin:0 0 6px;",
                    tokens::FONT_SANS, tokens::TEXT,
                ),
                "The engine contract"
            }
            p {
                style: format!(
                    "font-family:{};font-size:14px;color:{};margin:0 0 18px;max-width:62ch;",
                    tokens::FONT_SANS, tokens::TEXT_MUTED,
                ),
                "For reference: the HTTP / WS routes a running engine exposes. A live browse \
                 reads "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, tokens::ACCENT),
                    "GET /api/runs"
                }
                "; the desktop review app streams each run over the session socket."
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

/// The local-folder dropzone (open a directory containing a `.darkrun/` folder).
#[component]
fn Dropzone() -> Element {
    let zone = format!(
        "border:1px dashed {border};border-radius:12px;padding:40px 24px;text-align:center;\
         background:{raised};cursor:pointer;",
        border = tokens::BORDER_STRONG,
        raised = tokens::SURFACE_RAISED,
    );
    rsx! {
        // The picker hook reads the chosen directory client-side via the File
        // System Access API (`data-darkrun-pick`); the workspace reader renders
        // the discovered runs in place.
        div { style: "{zone}", "data-darkrun-pick": "true",
            div {
                style: format!("font-family:{};font-size:32px;color:{};margin-bottom:10px;", tokens::FONT_SANS, tokens::TEXT_FAINT),
                "▢"
            }
            div {
                style: format!("font-family:{};font-size:16px;color:{};", tokens::FONT_SANS, tokens::TEXT),
                "Drop a project folder here or click to browse"
            }
            div {
                style: format!("font-family:{};font-size:13px;color:{};margin-top:6px;", tokens::FONT_SANS, tokens::TEXT_MUTED),
                "Select a directory containing a "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, tokens::ACCENT),
                    ".darkrun/"
                }
                " folder."
            }
        }
    }
}

/// The remote-repository entry: paste a repo URL to browse its workspace.
#[component]
fn RemoteRepo() -> Element {
    let card = format!(
        "margin-top:20px;border:1px solid {border};border-radius:12px;padding:18px 20px;background:{raised};",
        border = tokens::BORDER,
        raised = tokens::SURFACE_RAISED,
    );
    let input = format!(
        "flex:1;font-family:{mono};font-size:13px;color:{text};background:{surface};\
         border:1px solid {border};border-radius:8px;padding:9px 12px;",
        mono = tokens::FONT_MONO,
        text = tokens::TEXT,
        surface = tokens::SURFACE_BASE,
        border = tokens::BORDER,
    );
    let btn = format!(
        "font-family:{sans};font-size:13px;font-weight:600;color:{on};background:{accent};\
         border:none;border-radius:8px;padding:9px 16px;cursor:pointer;",
        sans = tokens::FONT_SANS,
        on = tokens::ON_ACCENT,
        accent = tokens::ACCENT,
    );
    rsx! {
        div { style: "{card}",
            div {
                style: format!(
                    "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;color:{};margin-bottom:10px;",
                    tokens::FONT_MONO, tokens::TEXT_FAINT,
                ),
                "or browse a remote repository"
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
        }
    }
}

/// A note framing browse as a read-only, client-side surface.
#[component]
fn ClientSideNote() -> Element {
    let wrap = format!(
        "margin-top:18px;border:1px solid {border};border-left:3px solid {accent};border-radius:8px;\
         padding:12px 16px;background:{overlay};",
        border = tokens::BORDER,
        accent = tokens::ACCENT,
        overlay = tokens::SURFACE_OVERLAY,
    );
    rsx! {
        div { style: "{wrap}",
            div { style: "display:flex;align-items:center;gap:8px;margin-bottom:8px;",
                Badge { tone: Tone::Accent, filled: true, "in the browser" }
                Badge { tone: Tone::Neutral, "read-only" }
            }
            p {
                style: format!("font-family:{};font-size:13px;color:{};margin:0;", tokens::FONT_SANS, tokens::TEXT_MUTED),
                "Browsing reads the workspace files client-side \u{2014} a local folder never \
                 leaves your machine. Browse is for reading a run's shape; driving the work \
                 (approve / request-changes) happens in the desktop review app."
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
        border = tokens::BORDER,
        raised = tokens::SURFACE_RAISED,
    );
    rsx! {
        div { style: "{row}",
            span { style: "min-width:58px;", Badge { tone, filled: true, "{method}" } }
            code {
                style: format!("font-family:{};font-size:13px;color:{};min-width:240px;", tokens::FONT_MONO, tokens::TEXT),
                "{path}"
            }
            span {
                style: format!("font-family:{};font-size:13px;color:{};flex:1;", tokens::FONT_SANS, tokens::TEXT_MUTED),
                "{summary}"
            }
            Badge { tone: Tone::Neutral, "{tag}" }
        }
    }
}

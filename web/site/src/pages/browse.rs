//! `/browse` — where browsing runs actually happens.
//!
//! This page is **not** the live run browser. You browse and open your runs in
//! the **darkrun desktop app**, which lists them from the local engine's
//! `GET /api/runs` and opens any one into its live review — locally, without
//! taking over your browser. Remote / web browsing is a later thing.
//!
//! As reference, the page still lists the engine's real HTTP/WS contract
//! (`darkrun_api::ROUTES`) so the surface the desktop app talks to is visible.

use darkrun_api::{HttpMethod, ROUTES};
use darkrun_ui::prelude::*;

use crate::pages::review::DesktopNote;
use crate::ui::SectionHead;

/// `/browse` — the desktop run-browser explainer + the live contract reference.
#[component]
pub fn Browse() -> Element {
    rsx! {
        SectionHead {
            kicker: "how browsing works".to_string(),
            title: "Browse runs in the desktop app".to_string(),
            lead: Some(
                "Your active and past runs live in the darkrun desktop app. It lists them \
                 from the local engine and opens any one into its live review \u{2014} on your \
                 machine, not in your browser."
                    .to_string(),
            ),
        }

        DesktopNote {}

        div { style: "margin-top:28px;",
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
                "For reference: the HTTP / WS routes the desktop app calls. The run browser \
                 reads "
                code {
                    style: format!("font-family:{};color:{};", tokens::FONT_MONO, tokens::ACCENT),
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

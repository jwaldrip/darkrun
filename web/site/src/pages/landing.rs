//! The landing page: the outlined-wordmark hero, the station line, the phase
//! legend, and entry points into the factory corpus and docs.

use darkrun_ui::prelude::*;

use crate::route::Route;
use crate::ui::{PhaseLegend, SectionHead};

/// `/` — the front door.
#[component]
pub fn Landing() -> Element {
    let hero = "display:flex;flex-direction:column;align-items:flex-start;gap:20px;\
                padding:48px 0 56px;";
    let tagline = format!(
        "font-family:{sans};font-size:34px;font-weight:700;line-height:1.15;\
         letter-spacing:-0.02em;color:{text};margin:0;max-width:18ch;",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    let sub = format!(
        "font-family:{sans};font-size:18px;color:{muted};margin:0;max-width:60ch;",
        sans = tokens::FONT_SANS,
        muted = tokens::TEXT_MUTED,
    );
    let cta = "display:flex;gap:12px;flex-wrap:wrap;margin-top:4px;";

    rsx! {
        section { style: "{hero}",
            Wordmark { variant: WordmarkVariant::Outlined, size: 64.0 }
            h1 { style: "{tagline}", "An agentic assembly line for your business." }
            p { style: "{sub}",
                "darkrun is a dark factory harness: it runs your agents lights-out as an ordered "
                "line of stations that take work from raw intent to a shipped, hardened outcome. "
                "You drive the line. The manager keeps every station honest."
            }
            div { style: "{cta}",
                Link { to: Route::Docs {},
                    Button { variant: ButtonVariant::Primary, "Read the docs" }
                }
                Link { to: Route::Factories {},
                    Button { variant: ButtonVariant::Secondary, "Browse factories" }
                }
            }
        }

        // The station line: the six software-factory stations as a pipeline.
        section { style: "margin:8px 0 40px;",
            SectionHead {
                kicker: "the line".to_string(),
                title: "Six stations, in cost-of-late-discovery order".to_string(),
                lead: Some(
                    "Frame -> Specify -> Shape -> Build -> Prove -> Harden. Each station retires \
                     one class of risk before the next begins."
                        .to_string(),
                ),
            }
            div { class: "dr-grid",
                for (i, name) in tokens::STATIONS.iter().enumerate() {
                    StationCard { index: i, name: name.to_string() }
                }
            }
        }

        // The phase machine every station runs.
        section { style: "margin:8px 0 40px;",
            SectionHead {
                kicker: "every station".to_string(),
                title: "One phase machine, six beats".to_string(),
                lead: Some(
                    "spec -> review -> manufacture -> audit -> tests -> checkpoint. The same loop \
                     runs in Frame and in Harden; only the workers and the locked artifact change."
                        .to_string(),
                ),
            }
            PhaseLegend {}
        }
    }
}

/// One station tile on the landing line.
#[component]
fn StationCard(index: usize, name: String) -> Element {
    let n = format!("{:02}", index + 1);
    let card = format!(
        "background:{raised};border:1px solid {border};border-radius:10px;padding:16px;",
        raised = tokens::SURFACE_RAISED,
        border = tokens::BORDER,
    );
    let num = format!(
        "font-family:{mono};font-size:12px;color:{accent};",
        mono = tokens::FONT_MONO,
        accent = tokens::ACCENT,
    );
    let title = format!(
        "font-family:{sans};font-size:18px;font-weight:700;color:{text};\
         text-transform:capitalize;margin:6px 0 0;",
        sans = tokens::FONT_SANS,
        text = tokens::TEXT,
    );
    rsx! {
        div { style: "{card}",
            div { style: "{num}", "station {n}" }
            div { style: "{title}", "{name}" }
        }
    }
}

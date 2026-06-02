//! The landing page: the outlined-wordmark hero, the station line, the phase
//! legend, and entry points into the factory corpus and docs.

use darkrun_ui::prelude::*;

use crate::ui::theme;

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
        text = theme::TEXT,
    );
    let sub = format!(
        "font-family:{sans};font-size:18px;color:{muted};margin:0;max-width:60ch;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    let cta = "display:flex;gap:12px;flex-wrap:wrap;margin-top:4px;";

    rsx! {
        section { style: "{hero}",
            Wordmark { variant: WordmarkVariant::OutlinedSolidRun, size: 64.0, interactive: true }
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

        // The software factory's line: its own declared stations, in pipeline
        // order. This is one factory's recipe, not a fixed universal six.
        section { style: "margin:8px 0 40px;",
            SectionHead {
                kicker: "the software factory".to_string(),
                title: "Its assembly line, in cost-of-late-discovery order".to_string(),
                lead: Some(
                    "Frame -> Specify -> Shape -> Build -> Prove -> Harden. Each station retires \
                     one class of risk before the next begins. This is the software factory's line; \
                     every factory declares its own — the station names and count are the recipe, \
                     not the law."
                        .to_string(),
                ),
            }
            div { class: "dr-grid",
                for (i, name) in software_stations().iter().enumerate() {
                    StationCard { index: i, name: name.clone() }
                }
            }
        }

        // The phase machine: the universal part. Every station in every factory
        // runs this loop, ordered by the cost of discovering a defect late.
        section { style: "margin:8px 0 40px;",
            SectionHead {
                kicker: "every factory, every station".to_string(),
                title: "One phase machine, ordered by cost-of-late-discovery".to_string(),
                lead: Some(
                    "spec -> review -> manufacture -> audit -> tests -> checkpoint. This loop is \
                     what every factory shares: the same machine runs in each station, and stations \
                     are sequenced so the cheapest risks die first. The line's length and labels \
                     vary by factory; the machine and the ordering principle do not."
                        .to_string(),
                ),
            }
            PhaseLegend {}
        }
    }
}

/// The software factory's own declared station names, in pipeline order.
///
/// Sourced from the embedded corpus so the landing line is genuinely *that
/// factory's* recipe rather than a hardcoded universal. Falls back to the
/// `tokens::STATIONS` defaults if the factory cannot be loaded, so the hero
/// never blanks.
fn software_stations() -> Vec<String> {
    match darkrun_content::load_validated("software") {
        Ok(factory) => factory.stations.iter().map(|s| s.name().to_string()).collect(),
        Err(_) => tokens::STATIONS.iter().map(|s| s.to_string()).collect(),
    }
}

/// One station tile on the landing line.
#[component]
fn StationCard(index: usize, name: String) -> Element {
    let n = format!("{:02}", index + 1);
    let card = format!(
        "background:{raised};border:1px solid {border};border-radius:10px;padding:16px;",
        raised = theme::SURFACE_RAISED,
        border = theme::BORDER,
    );
    let num = format!(
        "font-family:{mono};font-size:12px;color:{accent};",
        mono = tokens::FONT_MONO,
        accent = theme::ACCENT,
    );
    let title = format!(
        "font-family:{sans};font-size:18px;font-weight:700;color:{text};\
         text-transform:capitalize;margin:6px 0 0;",
        sans = tokens::FONT_SANS,
        text = theme::TEXT,
    );
    rsx! {
        div { style: "{card}",
            div { style: "{num}", "station {n}" }
            div { style: "{title}", "{name}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_stations_come_from_the_corpus() {
        // The landing line renders the software factory's *own* declared
        // stations, not the hardcoded token defaults, so adding/reordering a
        // station in the corpus flows through to the hero.
        let from_corpus = software_stations();
        let declared: Vec<String> = darkrun_content::load_validated("software")
            .expect("software factory loads")
            .stations
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        assert_eq!(from_corpus, declared);
        assert!(!from_corpus.is_empty());
    }
}

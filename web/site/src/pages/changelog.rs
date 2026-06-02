//! `/changelog` — a simple reverse-chronological release log, rendered from a
//! small in-crate data table.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::ui::SectionHead;

/// A single changelog entry.
struct Release {
    version: &'static str,
    date: &'static str,
    notes: &'static [&'static str],
}

const RELEASES: &[Release] = &[
    Release {
        version: "0.1.0",
        date: "2026-05-30",
        notes: &[
            "First public cut of the software factory: Frame -> Specify -> Shape -> Build -> Prove -> Harden.",
            "The six-phase station machine: spec -> review -> manufacture -> audit -> tests -> checkpoint.",
            "Embedded factory corpus, the local engine, and the desktop review app.",
        ],
    },
];

/// `/changelog` — the release log.
#[component]
pub fn Changelog() -> Element {
    rsx! {
        SectionHead {
            kicker: "releases".to_string(),
            title: "Changelog".to_string(),
            lead: Some("What shipped, newest first.".to_string()),
        }
        div { style: "display:flex;flex-direction:column;gap:16px;",
            for release in RELEASES {
                Card {
                    div {
                        style: format!(
                            "display:flex;align-items:baseline;gap:10px;margin-bottom:8px;font-family:{};",
                            tokens::FONT_SANS,
                        ),
                        span {
                            style: format!("font-size:18px;font-weight:700;color:{};", theme::TEXT),
                            "v{release.version}"
                        }
                        span {
                            style: format!("font-family:{};font-size:12px;color:{};", tokens::FONT_MONO, theme::TEXT_FAINT),
                            "{release.date}"
                        }
                    }
                    ul {
                        style: format!("margin:0;padding-left:18px;color:{};font-family:{};font-size:14px;line-height:1.7;", theme::TEXT_MUTED, tokens::FONT_SANS),
                        for note in release.notes {
                            li { "{note}" }
                        }
                    }
                }
            }
        }
    }
}

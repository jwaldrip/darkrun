//! Smaller pages: `/paper`, `/templates`, and the catch-all `NotFound`.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::route::Route;
use crate::ui::SectionHead;

/// `/paper` — the methodology paper landing.
#[component]
pub fn Paper() -> Element {
    rsx! {
        SectionHead {
            kicker: "the paper".to_string(),
            title: "The cost of late discovery".to_string(),
            lead: Some(
                "The argument behind darkrun: order the line so every mistake is paid for while it \
                 is still cheap to fix."
                    .to_string(),
            ),
        }
        article { class: "dr-prose",
            p {
                "The full write-up walks through the six stations, the phase machine, and the "
                "checkpoint model. Until the long-form paper lands, the "
            }
            p {
                Link { to: Route::Methodology {}, "methodology" }
                " and "
                Link { to: Route::Lifecycles {}, "lifecycles" }
                " pages carry the core argument."
            }
        }
    }
}

/// `/templates` — factory templates, sourced from the embedded corpus.
#[component]
pub fn Templates() -> Element {
    let slugs = darkrun_content::list_factories();
    rsx! {
        SectionHead {
            kicker: "templates".to_string(),
            title: "Factory templates".to_string(),
            lead: Some(
                "Start from a shipped factory. Each is a complete methodology you can run as-is or \
                 fork for your own line."
                    .to_string(),
            ),
        }
        div { class: "dr-grid",
            for slug in slugs {
                Link {
                    to: Route::FactoryDetail { slug: slug.clone() },
                    style: "text-decoration:none;display:block;",
                    Card {
                        span {
                            style: format!(
                                "font-family:{};font-size:16px;font-weight:700;color:{};text-transform:capitalize;",
                                tokens::FONT_SANS, theme::TEXT,
                            ),
                            "{slug}"
                        }
                    }
                }
            }
        }
    }
}

/// The catch-all 404 page for any unmatched path.
#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = format!("/{}", segments.join("/"));
    rsx! {
        SectionHead {
            kicker: "404".to_string(),
            title: "Off the line".to_string(),
            lead: Some(format!("There is nothing at {path}.")),
        }
        Link { to: Route::Landing {}, Button { variant: ButtonVariant::Primary, "Back to the factory" } }
    }
}

//! The guide pages: `/start-here`, `/how-it-works`, `/big-picture`,
//! `/workflows`, and `/about`.
//!
//! These are the prose-forward onboarding and explainer pages. Each one renders
//! a single embedded markdown document from the [`GUIDES`](crate::content::GUIDES)
//! corpus through the shared [`Prose`] component, with a small kicker header and
//! a footer strip linking across the other guides — the same
//! render-markdown-by-slug pattern the concept pages use.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::content::{self, Doc, GUIDES};
use crate::route::Route;
use crate::ui::{Prose, SectionHead};

/// Render a guide page by slug, with a cross-link footer, or a small fallback if
/// the slug is unknown.
fn guide(slug: &str) -> Element {
    match content::find(GUIDES, slug) {
        Some(doc) => rsx! {
            div { style: "margin-bottom:8px;",
                span {
                    style: format!(
                        "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.08em;color:{};",
                        tokens::FONT_MONO, theme::TEXT_FAINT,
                    ),
                    "guide"
                }
            }
            Prose { doc: *doc }
            GuideNav { current: slug.to_string() }
        },
        None => rsx! {
            SectionHead {
                kicker: "not found".to_string(),
                title: "No such guide".to_string(),
                lead: Some(format!("There is no guide at /{slug}.")),
            }
            Link { to: Route::StartHere {}, Button { variant: ButtonVariant::Secondary, "Start here" } }
        },
    }
}

/// A bottom strip linking across every other guide, so the prose pages form a
/// connected set instead of dead ends.
#[component]
fn GuideNav(current: String) -> Element {
    let wrap = format!(
        "margin-top:40px;border-top:1px solid {};padding-top:20px;\
         display:flex;gap:10px;flex-wrap:wrap;",
        theme::BORDER,
    );
    rsx! {
        div { style: "{wrap}",
            for doc in GUIDES {
                if doc.slug != current {
                    Link { to: guide_route(doc),
                        Button { variant: ButtonVariant::Secondary, "{doc.title}" }
                    }
                }
            }
        }
    }
}

/// Map a guide doc to its top-level route.
fn guide_route(doc: &Doc) -> Route {
    match doc.slug {
        "start-here" => Route::StartHere {},
        "how-it-works" => Route::HowItWorks {},
        "big-picture" => Route::BigPicture {},
        "workflows" => Route::Workflows {},
        _ => Route::About {},
    }
}

/// `/start-here` — onboarding entry: what darkrun is, install, first Run.
#[component]
pub fn StartHere() -> Element {
    guide("start-here")
}

/// `/how-it-works` — the engine model.
#[component]
pub fn HowItWorks() -> Element {
    guide("how-it-works")
}

/// `/big-picture` — the why/thesis, the most prose-forward page.
#[component]
pub fn BigPicture() -> Element {
    guide("big-picture")
}

/// `/workflows` — the practical workflow + command catalog.
#[component]
pub fn Workflows() -> Element {
    guide("workflows")
}

/// `/about` — what darkrun is, the rewrite story, the philosophy, the license.
#[component]
pub fn About() -> Element {
    guide("about")
}

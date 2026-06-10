//! `/blog` and `/blog/:slug` — the post index and individual posts, rendered
//! from the embedded blog corpus.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::content::{self, POSTS};
use crate::route::Route;
use crate::search::use_json_ld;
use crate::seo;
use crate::ui::{Prose, SectionHead};

/// `/blog` — the post index, newest first.
#[component]
pub fn Blog() -> Element {
    rsx! {
        SectionHead {
            kicker: "writing".to_string(),
            title: "Blog".to_string(),
            lead: Some("Notes on running an agent as a factory.".to_string()),
        }
        div { style: "display:flex;flex-direction:column;gap:12px;",
            for post in POSTS {
                Link {
                    to: Route::Post { slug: post.slug.to_string() },
                    style: "text-decoration:none;display:block;",
                    Card {
                        span {
                            style: format!("font-family:{};font-size:17px;font-weight:700;color:{};", tokens::FONT_SANS, theme::TEXT),
                            "{post.title}"
                        }
                        if !post.date.is_empty() {
                            span {
                                style: format!("font-family:{};font-size:12px;color:{};margin-left:8px;", tokens::FONT_MONO, theme::TEXT_MUTED),
                                "{post.date}"
                            }
                        }
                        p {
                            style: format!("font-family:{};font-size:14px;color:{};margin:6px 0 0;", tokens::FONT_SANS, theme::TEXT_MUTED),
                            "{post.summary}"
                        }
                    }
                }
            }
        }
    }
}

/// `/blog/:slug` — a single post.
#[component]
pub fn Post(slug: String) -> Element {
    use_json_ld(
        content::find(POSTS, &slug)
            .map(|p| seo::json_ld_article(p, &format!("/blog/{slug}")))
            .unwrap_or_default(),
    );
    match content::find(POSTS, &slug) {
        Some(post) => rsx! {
            div { style: "margin-bottom:8px;",
                Link { to: Route::Blog {},
                    span {
                        style: format!("font-family:{};font-size:13px;color:{};", tokens::FONT_MONO, theme::ACCENT),
                        "\u{2190} all posts"
                    }
                }
            }
            if !post.date.is_empty() {
                div {
                    style: format!("font-family:{};font-size:12px;color:{};margin-bottom:4px;", tokens::FONT_MONO, theme::TEXT_MUTED),
                    "{post.date}"
                }
            }
            Prose { doc: *post }
        },
        None => rsx! {
            SectionHead {
                kicker: "not found".to_string(),
                title: "No such post".to_string(),
                lead: Some(format!("There is no post at /blog/{slug}.")),
            }
            Link { to: Route::Blog {}, Button { variant: ButtonVariant::Secondary, "All posts" } }
        },
    }
}

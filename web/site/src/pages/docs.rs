//! `/docs` and `/docs/:slug` — markdown documentation with a sidebar, rendered
//! from the embedded docs corpus via `pulldown-cmark`.

use darkrun_ui::prelude::*;

use crate::ui::theme;

use crate::content::{self, DOCS};
use crate::route::Route;
use crate::search::{use_json_ld, SearchBox};
use crate::seo;
use crate::ui::{Prose, SectionHead};

/// `/docs` — the documentation index. Renders the first doc inline beside the
/// sidebar so the landing of `/docs` is never empty.
#[component]
pub fn Docs() -> Element {
    let first = DOCS.first().copied();
    rsx! {
        DocsLayout { active: first.map(|d| d.slug.to_string()),
            if let Some(doc) = first {
                Prose { doc }
            } else {
                SectionHead { kicker: "docs".to_string(), title: "Documentation".to_string(), lead: None }
            }
        }
    }
}

/// `/docs/:slug` — a single documentation page beside the sidebar.
#[component]
pub fn DocPage(slug: String) -> Element {
    let doc = content::find(DOCS, &slug).copied();
    use_json_ld(
        doc.map(|d| seo::json_ld_article(&d, &format!("/docs/{slug}")))
            .unwrap_or_default(),
    );
    rsx! {
        DocsLayout { active: Some(slug.clone()),
            match doc {
                Some(doc) => rsx! { Prose { doc } },
                None => rsx! {
                    SectionHead {
                        kicker: "not found".to_string(),
                        title: "No such doc".to_string(),
                        lead: Some(format!("There is no documentation page at /docs/{slug}.")),
                    }
                    Link { to: Route::Docs {}, Button { variant: ButtonVariant::Secondary, "All docs" } }
                },
            }
        }
    }
}

/// Two-column docs frame: a sticky sidebar of links plus the page body.
#[component]
fn DocsLayout(active: Option<String>, children: Element) -> Element {
    let frame = "display:grid;grid-template-columns:200px 1fr;gap:32px;align-items:start;";
    let side = format!(
        "position:sticky;top:80px;display:flex;flex-direction:column;gap:4px;\
         border-right:1px solid {border};padding-right:16px;",
        border = theme::BORDER,
    );
    rsx! {
        div { style: "{frame}",
            nav { style: "{side}",
                SearchBox {}
                span {
                    style: format!(
                        "font-family:{};font-size:11px;text-transform:uppercase;letter-spacing:0.08em;color:{};margin-bottom:6px;",
                        tokens::FONT_MONO, theme::TEXT_FAINT,
                    ),
                    "docs"
                }
                for doc in DOCS {
                    SidebarLink {
                        slug: doc.slug.to_string(),
                        title: doc.title.to_string(),
                        active: active.as_deref() == Some(doc.slug),
                    }
                }
            }
            div { {children} }
        }
    }
}

/// One sidebar entry; the active page is accent-colored.
#[component]
fn SidebarLink(slug: String, title: String, active: bool) -> Element {
    let color = if active { theme::ACCENT } else { theme::TEXT_MUTED };
    let weight = if active { "600" } else { "400" };
    let style = format!(
        "font-family:{sans};font-size:14px;color:{color};font-weight:{weight};\
         text-decoration:none;padding:3px 0;",
        sans = tokens::FONT_SANS,
    );
    rsx! {
        Link { to: Route::DocPage { slug: slug.clone() }, class: "dr-navlink", style: "{style}", "{title}" }
    }
}

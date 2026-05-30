//! The site shell: the header (wordmark + nav), the routed page outlet, and the
//! footer. Every route renders inside this layout.

use darkrun_ui::prelude::*;

use crate::route::Route;
use crate::ui;

/// The chrome wrapped around every page: a sticky header with the outlined
/// wordmark and primary nav, the routed `Outlet`, and a footer.
#[component]
pub fn Shell() -> Element {
    let header = format!(
        "position:sticky;top:0;z-index:10;display:flex;align-items:center;\
         justify-content:space-between;gap:24px;padding:14px 24px;\
         background:{base}ee;border-bottom:1px solid {border};\
         backdrop-filter:blur(8px);",
        base = tokens::SURFACE_BASE,
        border = tokens::BORDER,
    );
    let nav = "display:flex;align-items:center;gap:18px;flex-wrap:wrap;";
    let main = "max-width:980px;margin:0 auto;padding:40px 24px 80px;";
    let footer = format!(
        "max-width:980px;margin:0 auto;padding:32px 24px 48px;\
         border-top:1px solid {border};display:flex;gap:24px;flex-wrap:wrap;\
         justify-content:space-between;align-items:center;\
         font-family:{mono};font-size:12px;color:{faint};",
        border = tokens::BORDER,
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );
    let page_bg = format!(
        "min-height:100vh;background:{base};color:{text};font-family:{sans};",
        base = tokens::SURFACE_BASE,
        text = tokens::TEXT,
        sans = tokens::FONT_SANS,
    );

    rsx! {
        // The dark-only theme custom properties, inlined once for the SPA.
        style { "{tokens::THEME_CSS}" }
        style { "{ui::GLOBAL_CSS}" }
        div { style: "{page_bg}",
            header { style: "{header}",
                Link { to: Route::Landing {},
                    Wordmark { variant: WordmarkVariant::Outlined, size: 22.0 }
                }
                nav { style: "{nav}",
                    NavLink { to: Route::Factories {}, label: "Factories" }
                    NavLink { to: Route::Docs {}, label: "Docs" }
                    NavLink { to: Route::Methodology {}, label: "Methodology" }
                    NavLink { to: Route::Blog {}, label: "Blog" }
                    NavLink { to: Route::Review {}, label: "Review" }
                }
            }
            main { style: "{main}", Outlet::<Route> {} }
            footer { style: "{footer}",
                span { "darkrun \u{00b7} the dark factory harness" }
                div { style: "display:flex;gap:18px;",
                    NavLink { to: Route::Changelog {}, label: "Changelog" }
                    NavLink { to: Route::Privacy {}, label: "Privacy" }
                    NavLink { to: Route::Terms {}, label: "Terms" }
                }
            }
        }
    }
}

/// A header/footer nav link with a muted-to-accent hover, driven by the router.
#[component]
fn NavLink(to: Route, label: &'static str) -> Element {
    let style = format!(
        "font-family:{sans};font-size:14px;color:{muted};text-decoration:none;",
        sans = tokens::FONT_SANS,
        muted = tokens::TEXT_MUTED,
    );
    rsx! {
        Link { to, class: "dr-navlink", style: "{style}", "{label}" }
    }
}

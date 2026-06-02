//! The site shell: the header (wordmark + nav), the routed page outlet, and the
//! footer. Every route renders inside this layout.

use darkrun_ui::prelude::*;

use crate::route::Route;
use crate::theme_toggle::ThemeToggle;
use crate::ui;
use crate::ui::theme;

/// The chrome wrapped around every page: a sticky header with the outlined
/// wordmark and primary nav, the routed `Outlet`, and a footer.
#[component]
pub fn Shell() -> Element {
    // The base color with ~93% alpha so the blur shows through. `color-mix`
    // keeps it theme-aware (a hex-alpha suffix can't ride on a `var()`).
    let header = format!(
        "position:sticky;top:0;z-index:10;display:flex;align-items:center;\
         justify-content:space-between;gap:24px;padding:14px 24px;\
         background:color-mix(in srgb, {base} 93%, transparent);\
         border-bottom:1px solid {border};backdrop-filter:blur(8px);",
        base = theme::SURFACE_BASE,
        border = theme::BORDER,
    );
    let nav = "display:flex;align-items:center;gap:18px;flex-wrap:wrap;";
    let main = "max-width:980px;margin:0 auto;padding:40px 24px 80px;";
    let footer = format!(
        "max-width:980px;margin:0 auto;padding:32px 24px 48px;\
         border-top:1px solid {border};display:flex;gap:24px;flex-wrap:wrap;\
         justify-content:space-between;align-items:center;\
         font-family:{mono};font-size:12px;color:{faint};",
        border = theme::BORDER,
        mono = tokens::FONT_MONO,
        faint = theme::TEXT_FAINT,
    );
    let page_bg = format!(
        "min-height:100vh;background:{base};color:{text};font-family:{sans};",
        base = theme::SURFACE_BASE,
        text = theme::TEXT,
        sans = tokens::FONT_SANS,
    );

    rsx! {
        // The theme custom properties (dark default + light via prefers-color-scheme
        // + the [data-theme] override), inlined once for the SPA.
        style { "{tokens::THEME_CSS}" }
        style { "{ui::GLOBAL_CSS}" }
        div { style: "{page_bg}",
            header { style: "{header}",
                Link { to: Route::Landing {},
                    Wordmark { variant: WordmarkVariant::OutlinedSolidRun, size: 22.0, interactive: true }
                }
                nav { style: "{nav}",
                    NavLink { to: Route::StartHere {}, label: "Start here" }
                    NavLink { to: Route::HowItWorks {}, label: "How it works" }
                    NavLink { to: Route::Factories {}, label: "Factories" }
                    NavLink { to: Route::Docs {}, label: "Docs" }
                    NavLink { to: Route::Methodology {}, label: "Methodology" }
                    NavLink { to: Route::Blog {}, label: "Blog" }
                    ThemeToggle {}
                }
            }
            main { style: "{main}", Outlet::<Route> {} }
            footer { style: "{footer}",
                span { "darkrun \u{00b7} the dark factory harness" }
                div { style: "display:flex;gap:18px;flex-wrap:wrap;",
                    NavLink { to: Route::BigPicture {}, label: "Big picture" }
                    NavLink { to: Route::Workflows {}, label: "Workflows" }
                    NavLink { to: Route::About {}, label: "About" }
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
        muted = theme::TEXT_MUTED,
    );
    rsx! {
        Link { to, class: "dr-navlink", style: "{style}", "{label}" }
    }
}

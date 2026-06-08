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
                    GithubLink {}
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

/// The repository URL, linked from the header GitHub mark.
const REPO_URL: &str = "https://github.com/darkrun-ai/darkrun";

/// The GitHub mark in the header, linking out to the repo. External link, so a
/// plain anchor (not the router) with `target=_blank` + `rel=noopener`.
#[component]
fn GithubLink() -> Element {
    let style = format!("display:inline-flex;align-items:center;color:{muted};", muted = theme::TEXT_MUTED);
    rsx! {
        a {
            class: "dr-navlink",
            href: REPO_URL,
            target: "_blank",
            rel: "noopener noreferrer",
            style: "{style}",
            "aria-label": "darkrun on GitHub",
            title: "darkrun on GitHub",
            svg {
                width: "20",
                height: "20",
                view_box: "0 0 16 16",
                fill: "currentColor",
                "aria-hidden": "true",
                path {
                    d: "M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8z",
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

//! The System / Light / Dark theme toggle.
//!
//! darkrun follows the system appearance by default and accepts a manual override.
//! This control is the website's surface for that override: a small segmented
//! control that maps each [`ThemeChoice`] onto the root `[data-theme]` attribute
//! (via [`darkrun_ui::theme::apply_script`]) and persists the choice in
//! `localStorage["darkrun-theme"]` so it survives reloads. `index.html` reads the
//! same key on first paint, so there is no flash of the wrong theme.
//!
//! Everything goes through `document::eval` because the SPA runs in the browser:
//! there is no direct DOM handle in the component, so we hand small JS snippets to
//! the renderer. The choice signal seeds from the persisted value on mount.

use darkrun_ui::prelude::*;

use crate::ui::theme;

/// The localStorage key the toggle and `index.html` share.
const STORAGE_KEY: &str = "darkrun-theme";

/// Run a fire-and-forget JS snippet in the browser. The site only ever pushes
/// state out (set attribute, write storage), so the result is ignored.
fn run_js(script: String) {
    let _ = document::eval(&script);
}

/// Apply a [`ThemeChoice`]: set/remove the root `[data-theme]`, persist the label,
/// and keep the `theme-color` meta in sync so mobile chrome matches.
fn apply_and_persist(choice: ThemeChoice) {
    let attr = apply_script(choice);
    let label = choice.label();
    // Dark when pinned dark, or when System resolves to a dark system preference.
    let theme_color = format!(
        "(function(){{\
           var pinned='{label}';\
           var dark = pinned==='dark' || (pinned==='system' && window.matchMedia && \
             window.matchMedia('(prefers-color-scheme: dark)').matches);\
           var m=document.querySelector('meta[name=\"theme-color\"]');\
           if(m)m.setAttribute('content',dark?'#07090c':'#f3f6f9');\
         }})();"
    );
    run_js(format!(
        "{attr}try{{localStorage.setItem('{STORAGE_KEY}','{label}');}}catch(e){{}}{theme_color}"
    ));
}

/// The System / Light / Dark segmented control. Defaults to whatever the user
/// last picked (read from localStorage on mount); falls back to System.
#[component]
pub fn ThemeToggle() -> Element {
    let mut choice = use_signal(|| ThemeChoice::System);

    // Seed the control from the persisted choice once, after mount. The attribute
    // itself is already applied by index.html's inline script; this only syncs the
    // signal so the right segment renders active.
    use_effect(move || {
        spawn(async move {
            if let Ok(label) = document::eval(&format!(
                "return (localStorage.getItem('{STORAGE_KEY}') || 'system');"
            ))
            .join::<String>()
            .await
            {
                choice.set(ThemeChoice::from_label(&label));
            }
        });
    });

    let wrap = format!(
        "display:inline-flex;align-items:center;gap:2px;\
         border:1px solid {border};border-radius:999px;padding:2px;\
         background:{raised};",
        border = theme::BORDER,
        raised = theme::SURFACE_RAISED,
    );

    rsx! {
        div { style: "{wrap}", role: "group", "aria-label": "Theme",
            for opt in ThemeChoice::ALL {
                {
                    let active = choice() == opt;
                    let seg = format!(
                        "appearance:none;border:0;cursor:pointer;\
                         font-family:{mono};font-size:11px;letter-spacing:0.02em;\
                         padding:4px 10px;border-radius:999px;line-height:1;\
                         color:{fg};background:{bg};",
                        mono = tokens::FONT_MONO,
                        fg = if active { theme::ON_ACCENT } else { theme::TEXT_MUTED },
                        bg = if active { theme::ACCENT } else { "transparent" },
                    );
                    rsx! {
                        button {
                            style: "{seg}",
                            "aria-pressed": if active { "true" } else { "false" },
                            title: "{opt.display_label()} theme",
                            onclick: move |_| {
                                choice.set(opt);
                                apply_and_persist(opt);
                            },
                            "{opt.display_label()}"
                        }
                    }
                }
            }
        }
    }
}

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

        // The desktop review app: where the human stands on the line. The shot
        // shows a real design decision rendered as a picture per option.
        section { style: "margin:8px 0 48px;",
            SectionHead {
                kicker: "the desktop app".to_string(),
                title: "Where you and the agent collaborate".to_string(),
                lead: Some(
                    "The desktop app is the visual interface between you and the agent — the \
                     control room for the line. The agent surfaces every checkpoint, review, \
                     and design direction as something you can see and act on; you decide, \
                     annotate, and steer. A few of its surfaces:"
                        .to_string(),
                ),
            }
            DesktopSlideshow {}
        }

        // Why you should use it: the value, not the feature list.
        section { style: "margin:8px 0 48px;",
            SectionHead {
                kicker: "why darkrun".to_string(),
                title: "Spend your attention where it's load-bearing".to_string(),
                lead: Some(
                    "Agents are fast and tireless; your judgment is the scarce input. \
                     darkrun spends it at the gates and nowhere else."
                        .to_string(),
                ),
            }
            div { class: "dr-grid",
                for (title , body) in why_points() {
                    ValueCard { title, body }
                }
            }
        }

        // How to get started quickly: the agent path, three lines.
        section { style: "margin:8px 0 48px;",
            SectionHead {
                kicker: "get started".to_string(),
                title: "Three lines, inside your agent".to_string(),
                lead: Some(
                    "darkrun installs as a plugin and runs where your agent already lives. \
                     Add it, then describe the work — the manager walks the line and you \
                     review in the desktop app."
                        .to_string(),
                ),
            }
            Quickstart {}
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

/// A manual carousel of the desktop app's surfaces — one feature per slide,
/// driven by prev/next + dots. No auto-advance (no timer): the visitor steps
/// through it, which also keeps the SSG pre-render deterministic.
#[component]
fn DesktopSlideshow() -> Element {
    // (feature label, caption, dark image, light image). `asset!` needs literal
    // paths. Both variants render; the shared `.dr-themed-*` CSS (in
    // darkrun_ui::tokens::THEME_CSS) shows the one matching the site theme —
    // the same render-both-let-CSS-pick mechanism the wordmark uses.
    let slides = [
        (
            "The run review",
            "The main surface. The station line shows where the run is; the tabs hold the work under review; and this is where you approve, request changes, or leave feedback.",
            asset!("/assets/desktop-run-review.png"),
            asset!("/assets/desktop-run-review-light.png"),
        ),
        (
            "Decisions",
            "When a call is yours to make, the agent draws each option — you pick from a diagram, not a wall of prose.",
            asset!("/assets/desktop-review.png"),
            asset!("/assets/desktop-review-light.png"),
        ),
        (
            "Design directions",
            "Choose a design archetype from real mockups, then annotate what to change.",
            asset!("/assets/desktop-direction.png"),
            asset!("/assets/desktop-direction-light.png"),
        ),
        (
            "Projects & runs",
            "Every repo's runs in one place — open a review or add a project.",
            asset!("/assets/desktop-browser.png"),
            asset!("/assets/desktop-browser-light.png"),
        ),
    ];
    let n = slides.len();
    let mut idx = use_signal(|| 0usize);
    let cur = idx();
    let label = slides[cur].0;
    let caption = slides[cur].1;
    let dark = &slides[cur].2;
    let light = &slides[cur].3;

    // No `display` here — the `.dr-themed-*` CSS classes toggle which variant
    // shows per the active theme, and an inline `display` would outrank them.
    // The screenshots carry their own transparent, rounded window corners (baked
    // into the PNG alpha), so we add neither border nor border-radius here — a
    // CSS rounding wouldn't match the window's corner radius and would leave a
    // mismatched edge. `drop-shadow` (not `box-shadow`) follows the alpha shape,
    // so the shadow hugs the rounded corners instead of a square box.
    let frame = "width:100%;height:auto;\
                 filter:drop-shadow(0 10px 30px rgba(0,0,0,0.32));"
        .to_string();
    let navbtn = format!(
        "appearance:none;cursor:pointer;background:{raised};border:1px solid {border};\
         color:{text};border-radius:999px;width:30px;height:30px;line-height:1;font-size:16px;",
        raised = theme::SURFACE_RAISED,
        border = theme::BORDER,
        text = theme::TEXT,
    );
    let cap = format!(
        "margin-top:10px;text-align:center;font-family:{sans};font-size:14px;color:{muted};",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    let chip = format!(
        "font-family:{mono};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;\
         color:{accent};margin-right:8px;",
        mono = tokens::FONT_MONO,
        accent = theme::ACCENT,
    );

    rsx! {
        figure { style: "margin:0;",
            img { class: "dr-themed-dark", src: "{dark}", alt: "darkrun desktop app — {label}", loading: "lazy", style: "{frame}" }
            img { class: "dr-themed-light", src: "{light}", alt: "darkrun desktop app — {label}", loading: "lazy", style: "{frame}" }
            div {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:12px;",
                button {
                    style: "{navbtn}", "aria-label": "previous surface",
                    onclick: move |_| idx.set((cur + n - 1) % n),
                    "\u{2039}"
                }
                div { style: "display:flex;align-items:center;gap:7px;",
                    for i in 0..n {
                        // Active state via a toggled CLASS (.dr-dot / .is-active in
                        // GLOBAL_CSS). NO `key` here: the list order is fixed, and a
                        // keyed reuse left the rendered width stale (the pill stuck on
                        // the first dot) even though the class attribute updated.
                        button {
                            class: if i == cur { "dr-dot is-active" } else { "dr-dot" },
                            "aria-label": "go to surface {i + 1}",
                            "aria-current": if i == cur { "true" } else { "false" },
                            onclick: move |_| idx.set(i),
                        }
                    }
                }
                button {
                    style: "{navbtn}", "aria-label": "next surface",
                    onclick: move |_| idx.set((cur + 1) % n),
                    "\u{203a}"
                }
            }
            figcaption { style: "{cap}",
                span { style: "{chip}", "{label}" }
                "{caption}"
            }
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

/// The three reasons to use it — value, not features.
fn why_points() -> [(String, String); 3] {
    [
        (
            "Checkpoints, not babysitting".to_string(),
            "Your attention goes to the gates, not the keystrokes. The run does the work; \
             you decide where it actually counts."
                .to_string(),
        ),
        (
            "Risk dies early".to_string(),
            "Stations run in cost-of-late-discovery order. The cheap risks die first, before \
             they get expensive to undo."
                .to_string(),
        ),
        (
            "Shipped, not just done".to_string(),
            "Every run ends hardened — proven against its spec and signed off at the release \
             gate, not left at \"works on my machine\"."
                .to_string(),
        ),
    ]
}

/// One value card in the "why" grid.
#[component]
fn ValueCard(title: String, body: String) -> Element {
    let card = format!(
        "background:{raised};border:1px solid {border};border-radius:10px;padding:18px;",
        raised = theme::SURFACE_RAISED,
        border = theme::BORDER,
    );
    let head = format!(
        "font-family:{sans};font-size:17px;font-weight:700;color:{text};margin:0 0 8px;",
        sans = tokens::FONT_SANS,
        text = theme::TEXT,
    );
    let body_style = format!(
        "font-family:{sans};font-size:14px;line-height:1.5;color:{muted};margin:0;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    rsx! {
        div { style: "{card}",
            h3 { style: "{head}", "{title}" }
            p { style: "{body_style}", "{body}" }
        }
    }
}

/// The quickstart: the agent install + first run, as a terminal block.
#[component]
fn Quickstart() -> Element {
    let block = format!(
        "background:{sink};border:1px solid {border};border-radius:10px;padding:18px 20px;\
         font-family:{mono};font-size:13.5px;line-height:1.7;color:{text};overflow-x:auto;",
        sink = theme::SURFACE_BASE,
        border = theme::BORDER,
        mono = tokens::FONT_MONO,
        text = theme::TEXT,
    );
    let comment = format!("color:{};", theme::TEXT_MUTED);
    let accent = format!("color:{};", theme::ACCENT);
    let note = format!(
        "font-family:{sans};font-size:13px;color:{muted};margin:12px 2px 0;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    rsx! {
        div { style: "{block}",
            div { style: "{comment}", "# in Claude Code: add the plugin" }
            div { span { style: "{accent}", "/plugin" } " marketplace add jwaldrip/darkrun" }
            div { span { style: "{accent}", "/plugin" } " install darkrun" }
            div { style: "height:8px;" }
            div { style: "{comment}", "# then describe the work" }
            div { span { style: "{accent}", "/darkrun:darkrun-new" } " \"add rate limiting to the public API\"" }
        }
        p { style: "{note}",
            "That's it. The manager scaffolds a right-sized run and walks the line; you show up "
            "at the checkpoints in the desktop app. Prefer headless? "
            "Read "
            Link { to: Route::Docs {}, "the docs" }
            " for the CLI path."
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

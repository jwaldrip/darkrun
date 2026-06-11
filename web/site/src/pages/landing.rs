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

        // The terminal status line: the run's position in one line, rendered
        // here by the REAL engine renderer against seeded state.
        section { style: "margin:8px 0 48px;",
            SectionHead {
                kicker: "the status line".to_string(),
                title: "The whole run, one terminal line".to_string(),
                lead: Some(
                    "While the desktop is the control room, the status line rides your \
                     terminal: position, phase, and a live second line of unit, feedback, \
                     or reviewer chips. These are real renders, not mockups:"
                        .to_string(),
                ),
            }
            StatuslineDemo {}
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
            "The main surface, live as the engine ticks. The station line shows where the run is; the unit dependency graph shows the wave the factory is building; the tabs hold outputs, knowledge, and feedback.",
            asset!("/assets/desktop-run-review.png"),
            asset!("/assets/desktop-run-review-light.png"),
        ),
        (
            "The approval gate",
            "At a checkpoint the run stops and hands you the decision: complete the station to advance, or request changes — which route back as drift, no restart.",
            asset!("/assets/desktop-approval.png"),
            asset!("/assets/desktop-approval-light.png"),
        ),
        (
            "Design directions",
            "Choose a design archetype from real mockups, then annotate what to change.",
            asset!("/assets/desktop-direction.png"),
            asset!("/assets/desktop-direction-light.png"),
        ),
        (
            "Annotate & steer",
            "Pick a direction and mark it up — drop pins on the preview and leave comments. The agent inherits not just which way to go, but exactly what to adjust.",
            asset!("/assets/desktop-annotate.png"),
            asset!("/assets/desktop-annotate-light.png"),
        ),
        (
            "Projects & runs",
            "Every repo's runs in one place — open a review or add a project.",
            asset!("/assets/desktop-browser.png"),
            asset!("/assets/desktop-browser-light.png"),
        ),
        // The status line gets its own live-render demo section below, so it
        // no longer rides this slideshow as a screenshot.
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

/// Claude Code's boxed session-start banner, composed for the demo's fiction
/// (the `checkout-flow` run in `~/dev/acme-checkout`). Pure presentation
/// chrome — the status line beneath it stays a real engine render.
///
/// Composed programmatically because terminal text is a character grid: the
/// left cell centers per line ({:^}), the right cell pads flush ({:<}), so the
/// column divider and borders land on exact columns instead of hand-counted
/// spaces.
fn cc_header_html(light: bool) -> String {
    // The box spans the terminal like the real banner does: at the banner's
    // 12px mono (~7.2px/char) the demo panel's ~851px inner width holds 116
    // columns with a hair of slack — wider would put a scrollbar on the panel.
    const TOTAL: usize = 116;
    const LW: usize = 40; // left cell inner width (centered)
    const RW: usize = TOTAL - LW - 7; // right cell inner width (flush left)
    // The whole box is clay, like the terminal renders it: border and title
    // in the dimmer weight, the section headings bold-bright. The light
    // palette deepens the clay and flips the text the way Claude Code's own
    // light theme does on a white terminal.
    let (border, clay, dim, faint, text) = if light {
        ("#b1664c", "#c15f3c", "#57606a", "#8b949e", "#1f2328")
    } else {
        ("#b1664c", "#d97757", "#9aa4ad", "#646d76", "#e6edf3")
    };
    const B: &str = "font-weight:700;";
    const I: &str = "font-style:italic;";
    let span = |c: &str, extra: &str, s: &str| {
        format!("<span style=\"color:{c};{extra}\">{s}</span>")
    };

    // (text, color, extra css) per cell row. Logo offsets survive the
    // centering: {:^} biases the extra space right, keeping the three lines'
    // relative columns exactly as the terminal draws them.
    let left: [(&str, &str, &str); 10] = [
        ("", dim, ""),
        ("Welcome back Jason!", text, B),
        ("", dim, ""),
        ("\u{2590}\u{259b}\u{2588}\u{2588}\u{2588}\u{259c}\u{258c}", clay, ""),
        ("\u{259d}\u{259c}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{259b}\u{2598}", clay, ""),
        ("\u{2598}\u{2598} \u{259d}\u{259d}", clay, ""),
        ("", dim, ""),
        ("Fable 5 with high effort \u{b7} Claude Max", dim, ""),
        ("~/dev/acme-checkout", faint, ""),
        ("", dim, ""),
    ];
    let rule: String = "\u{2500}".repeat(RW);
    let right: [(&str, &str, &str); 10] = [
        ("Tips for getting started", clay, B),
        ("Run /init to create a CLAUDE.md file with instructions for Claude", dim, ""),
        (&rule, faint, ""),
        ("What's new", clay, B),
        ("Fixed Fable 5 model names with a `[1m]` suffix not being normalized", dim, ""),
        ("\u{2014} Fable 5 includes 1M context, so the suffix is now stripped", dim, ""),
        ("Fixed a spurious \"sandbox dependencies missing\" startup warning on", dim, ""),
        ("Windows when sandbox was enabled in settings", dim, ""),
        ("Sub-agents can now spawn their own sub-agents (up to 5 levels deep)", dim, ""),
        ("/release-notes for more", dim, I),
    ];

    let title = "Claude Code v2.1.173";
    let mut lines = Vec::with_capacity(left.len() + 2);
    lines.push(format!(
        "{open}{name}{fill}",
        open = span(border, "", "\u{256d}\u{2500}\u{2500}\u{2500} "),
        name = span(clay, "", title),
        fill = span(
            border,
            "",
            &format!(" {}\u{256e}", "\u{2500}".repeat(TOTAL - title.len() - 7)),
        ),
    ));
    for ((lt, lc, lx), (rt, rc, rx)) in left.iter().zip(right.iter()) {
        lines.push(format!(
            "{v} {l} {v} {r} {v}",
            v = span(border, "", "\u{2502}"),
            l = span(lc, lx, &format!("{lt:^LW$}")),
            r = span(rc, rx, &format!("{rt:<RW$}")),
        ));
    }
    lines.push(span(
        border,
        "",
        &format!("\u{2570}{}\u{256f}", "\u{2500}".repeat(TOTAL - 2)),
    ));
    lines.join("\n")
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

/// How a given harness installs darkrun.
#[derive(Clone, Copy, PartialEq)]
enum Hkind {
    /// Claude Code: the `/plugin` marketplace commands.
    Plugin,
    /// A one-line `<cli> mcp add` command (the `detail` is the CLI binary).
    Cli,
    /// An MCP config file (the `detail` is the config path).
    Mcp,
}

/// The harness catalog: (label, harness id, install kind, detail). `detail` is
/// the CLI binary for `Cli` and the config-file path for `Mcp`.
fn harnesses() -> [(&'static str, &'static str, Hkind, &'static str); 7] {
    [
        ("Claude Code", "claude-code", Hkind::Plugin, ""),
        ("Codex", "codex", Hkind::Cli, "codex"),
        ("Gemini CLI", "gemini-cli", Hkind::Cli, "gemini"),
        ("Cursor", "cursor", Hkind::Mcp, ".cursor/mcp.json"),
        ("Windsurf", "windsurf", Hkind::Mcp, "~/.codeium/windsurf/mcp_config.json"),
        ("OpenCode", "opencode", Hkind::Mcp, "opencode.json"),
        ("Kiro", "kiro", Hkind::Mcp, ".kiro/agents/darkrun.yaml"),
    ]
}

/// The quickstart: pick your harness, get the right install + first run.
#[component]
fn Quickstart() -> Element {
    let list = harnesses();
    let mut sel = use_signal(|| 0usize);
    let (label, id, kind, path) = list[sel()];

    // The install + first-run snippet for the selected harness.
    let code = match kind {
        Hkind::Plugin => format!(
            "# in {label}: add the plugin\n\
             /plugin marketplace add darkrun-ai/darkrun\n\
             /plugin install darkrun\n\n\
             # then describe the work\n\
             /darkrun:darkrun-new \"add rate limiting to the public API\""
        ),
        Hkind::Cli => format!(
            "# in {label}: register the MCP server\n\
             {path} mcp add darkrun -- npx -y darkrun mcp --harness {id}\n\n\
             # then ask your agent to start a darkrun run\n\
             \"start a darkrun run: add rate limiting to the public API\""
        ),
        Hkind::Mcp => format!(
            "# add darkrun as an MCP server in {path}\n\
             npx -y darkrun mcp --harness {id}\n\n\
             # then ask your agent to start a darkrun run\n\
             \"start a darkrun run: add rate limiting to the public API\""
        ),
    };

    // The block is a terminal, like the statusline demo's panel: black on the
    // dark theme, white on the light one (.dr-qs-block, theme-keyed rules).
    let block = format!(
        "border-radius:10px;padding:18px 20px;\
         font-family:{mono};font-size:13.5px;line-height:1.7;overflow-x:auto;\
         white-space:pre;margin:0;",
        mono = tokens::FONT_MONO,
    );
    const QS_CSS: &str = r#"
.dr-qs-block{background:#0b0e13;border:1px solid #232a33;color:#c9d1d9;}
:root[data-theme="light"] .dr-qs-block{background:#ffffff;border-color:#d0d7de;color:#1f2328;}
@media (prefers-color-scheme: light){
  :root:not([data-theme="dark"]) .dr-qs-block{background:#ffffff;border-color:#d0d7de;color:#1f2328;}
}
"#;
    let seg_wrap = format!(
        "display:inline-flex;flex-wrap:wrap;gap:3px;border:1px solid {border};\
         border-radius:999px;padding:3px;background:{raised};",
        border = theme::BORDER,
        raised = theme::SURFACE_RAISED,
    );
    let row_label = format!(
        "font-family:{mono};font-size:11px;text-transform:uppercase;letter-spacing:0.06em;\
         color:{muted};",
        mono = tokens::FONT_MONO,
        muted = theme::TEXT_MUTED,
    );
    let note = format!(
        "font-family:{sans};font-size:13px;color:{muted};margin:12px 2px 0;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );

    rsx! {
        div {
            style: "display:flex;align-items:center;gap:10px;margin-bottom:12px;flex-wrap:wrap;",
            span { style: "{row_label}", "harness" }
            // Radio-style segmented control. `.dr-theme-seg` + aria-pressed reuses
            // the theme-picker pill styling and updates reliably (no inline-style
            // diffing quirk); no `key` so positions update in place.
            div { role: "radiogroup", "aria-label": "harness", style: "{seg_wrap}",
                for (j , h) in list.iter().enumerate() {
                    button {
                        class: "dr-theme-seg",
                        role: "radio",
                        "aria-checked": if j == sel() { "true" } else { "false" },
                        "aria-pressed": if j == sel() { "true" } else { "false" },
                        onclick: move |_| sel.set(j),
                        "{h.0}"
                    }
                }
            }
        }
        style { "{QS_CSS}" }
        pre { class: "dr-qs-block", style: "{block}", "{code}" }
        p { style: "{note}",
            "The manager scaffolds a right-sized run and walks the line; you review in the "
            "desktop app (Claude Code) or inline. Full per-harness setup and capabilities in "
            Link { to: Route::DocPage { slug: "other-harnesses".to_string() }, "Other harnesses" }
            "."
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

/// The terminal panel's theme-keyed chrome: a dark terminal on the dark site
/// theme, a light (Terminal.app-style) one on the light theme. The statusline
/// fragments themselves are theme-safe — chips carry their own boxes, hues are
/// palette-fixed, and the slug/pips paint in default-fg, which `.dr-sl-line`'s
/// `color` supplies per theme exactly like a real terminal would. Only the
/// banner needs a per-theme render (`cc_header_html(light)`), toggled via the
/// shared `.dr-themed-*` classes.
const SL_CSS: &str = r#"
.dr-sl-term{background:#0b0e13;border:1px solid #232a33;border-radius:10px;padding:14px 16px 12px;overflow-x:auto;}
.dr-sl-banner{margin:0 0 12px;font:12px/1.2 'JetBrains Mono','SF Mono',Menlo,monospace;white-space:pre;}
.dr-sl-line{margin:0;font:13px/1.7 'JetBrains Mono','SF Mono',Menlo,monospace;color:#c9d1d9;white-space:pre;}
.dr-sl-prompt{display:flex;align-items:baseline;gap:9px;border:1px solid #2f3742;border-radius:8px;padding:8px 12px;margin-bottom:7px;font:13px/1.5 'JetBrains Mono','SF Mono',Menlo,monospace;}
.dr-sl-mark{color:#8b949e;}
.dr-sl-hint{color:#58606a;font-style:italic;}
:root[data-theme="light"] .dr-sl-term{background:#ffffff;border-color:#d0d7de;}
:root[data-theme="light"] .dr-sl-line{color:#1f2328;}
:root[data-theme="light"] .dr-sl-prompt{border-color:#d0d7de;}
:root[data-theme="light"] .dr-sl-mark{color:#57606a;}
:root[data-theme="light"] .dr-sl-hint{color:#8b949e;}
@media (prefers-color-scheme: light){
  :root:not([data-theme="dark"]) .dr-sl-term{background:#ffffff;border-color:#d0d7de;}
  :root:not([data-theme="dark"]) .dr-sl-line{color:#1f2328;}
  :root:not([data-theme="dark"]) .dr-sl-prompt{border-color:#d0d7de;}
  :root:not([data-theme="dark"]) .dr-sl-mark{color:#57606a;}
  :root:not([data-theme="dark"]) .dr-sl-hint{color:#8b949e;}
}
"#;

/// The status line, rendered by the REAL engine renderer. The fragments are
/// generated from seeded run state by `gen_statusline_demo_html` (darkrun-cli)
/// and committed at `content/statusline-demo.html` — this component never
/// hand-fakes a chip. The terminal panel follows the site theme (see
/// [`SL_CSS`]): dark terminal on dark, light terminal on light.
#[component]
fn StatuslineDemo() -> Element {
    const FRAGMENTS: &str = include_str!("../../content/statusline-demo.html");
    // Pull one fragment per `<!--scenario:KEY-->` marker.
    let frag = |key: &str| -> &'static str {
        let marker = format!("<!--scenario:{key}-->");
        FRAGMENTS
            .find(&marker)
            .map(|at| {
                let rest = &FRAGMENTS[at + marker.len()..];
                let end = rest.find("<!--scenario:").unwrap_or(rest.len());
                rest[..end].trim()
            })
            .unwrap_or("")
    };
    let scenarios: Vec<(&str, &str, &str, &str)> = {
        let mut out = Vec::new();
        let metas = [
            (
                "manufacture",
                "The pool, live",
                "Second line: one chip per in-flight unit, one pip per worker beat — green advanced, yellow in-flight, red bounced.",
            ),
            (
                "feedback",
                "Feedback preempts",
                "Open feedback takes the line over — severity-tinted chips (! blocker, ~ medium) until the fix track clears them.",
            ),
            (
                "gated",
                "Parked at your gate",
                "Π is the doorway: the run is holding for your decision at the checkpoint, not failing.",
            ),
        ];
        for (key, title, caption) in metas {
            let dark = frag(key);
            if !dark.is_empty() {
                // The light variant deepens the accent hues for a white
                // terminal; fall back to the dark render if absent.
                let light_frag = frag(&format!("{key}-light"));
                let light = if light_frag.is_empty() { dark } else { light_frag };
                out.push((title, caption, dark, light));
            }
        }
        out
    };
    let n = scenarios.len();
    // Manual stepping, like the desktop slideshow above — the visitor drives.
    let mut idx = use_signal(|| 0usize);
    let cur = idx().min(n.saturating_sub(1));
    let Some((title, caption, html_dark, html_light)) = scenarios.get(cur).copied() else {
        return rsx! {};
    };

    // Claude Code's boxed session-start banner. Terminal text is a character
    // grid: the box is composed column-exact in `cc_header_html`, and the pre
    // gets a tight line-height (`.dr-sl-banner`) so the logo's block glyphs
    // stack the way a terminal cell grid renders them. One render per theme,
    // toggled by the shared `.dr-themed-*` classes.
    let header_dark = cc_header_html(false);
    let header_light = cc_header_html(true);
    let head = format!(
        "display:flex;align-items:center;gap:7px;margin-bottom:10px;",
    );
    let dot = |c: &str| format!("width:10px;height:10px;border-radius:50%;background:{c};");
    let cap = format!(
        "display:flex;align-items:baseline;gap:10px;margin-top:10px;\
         font-family:{sans};font-size:13px;color:{muted};",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    let cap_title = format!(
        "font-weight:700;color:{text};white-space:nowrap;",
        text = theme::TEXT,
    );
    let navbtn = format!(
        "appearance:none;cursor:pointer;background:{raised};border:1px solid {border};\
         color:{text};border-radius:999px;width:30px;height:30px;line-height:1;font-size:16px;",
        raised = theme::SURFACE_RAISED,
        border = theme::BORDER,
        text = theme::TEXT,
    );
    rsx! {
        div {
            style { "{SL_CSS}" }
            div { class: "dr-sl-term",
                div { style: "{head}",
                    span { style: dot("#ff5f57") }
                    span { style: dot("#febc2e") }
                    span { style: dot("#28c840") }
                }
                pre {
                    class: "dr-sl-banner dr-themed-dark",
                    dangerous_inner_html: "{header_dark}",
                }
                pre {
                    class: "dr-sl-banner dr-themed-light",
                    dangerous_inner_html: "{header_light}",
                }
                div { class: "dr-sl-prompt",
                    span { class: "dr-sl-mark", ">" }
                    span { class: "dr-sl-hint", "Try \"darkrun resume\"" }
                }
                pre {
                    class: "dr-sl-line dr-themed-dark",
                    dangerous_inner_html: "{html_dark}",
                }
                pre {
                    class: "dr-sl-line dr-themed-light",
                    dangerous_inner_html: "{html_light}",
                }
            }
            div { style: "{cap}",
                span { style: "{cap_title}", "{title}" }
                span { "{caption}" }
            }
            // The same left/right stepper as the desktop slideshow above.
            div {
                style: "display:flex;align-items:center;justify-content:space-between;gap:12px;margin-top:12px;",
                button {
                    style: "{navbtn}", "aria-label": "previous scenario",
                    onclick: move |_| idx.set((cur + n - 1) % n),
                    "\u{2039}"
                }
                div { style: "display:flex;align-items:center;gap:7px;",
                    for i in 0..n {
                        // Same classed dots as the desktop slideshow: the active
                        // scenario is the wide accent pill (.dr-dot/.is-active).
                        button {
                            class: if i == cur { "dr-dot is-active" } else { "dr-dot" },
                            "aria-label": "show scenario {i + 1}",
                            "aria-current": if i == cur { "true" } else { "false" },
                            onclick: move |_| idx.set(i),
                        }
                    }
                }
                button {
                    style: "{navbtn}", "aria-label": "next scenario",
                    onclick: move |_| idx.set((cur + 1) % n),
                    "\u{203a}"
                }
            }
        }
    }
}

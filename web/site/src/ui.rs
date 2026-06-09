//! Small presentational helpers shared across pages: global CSS, section
//! headers, prose rendering, and a phase legend. These sit on top of the
//! `darkrun-ui` design system and add only website-specific layout.

use darkrun_ui::prelude::*;

use crate::content::Doc;

/// Theme-aware color references for inline styles.
///
/// The shared `darkrun_ui::tokens` color constants are the **dark** hex values —
/// fine for SVG fills and computed geometry, but baked into a string they never
/// flip when the theme changes. The site instead points its inline styles at the
/// `--dr-*` custom properties from [`darkrun_ui::tokens::THEME_CSS`], which the
/// light/dark blocks (and the `[data-theme]` override) redefine. Using these in a
/// `style` string means every surface, border, and text color tracks the active
/// theme automatically.
///
/// Fonts and spacing don't change with the theme, so those keep using the plain
/// `tokens::FONT_*` constants directly.
pub mod theme {
    /// Near-black / paper-white canvas.
    pub const SURFACE_BASE: &str = "var(--dr-surface-base)";
    /// The default panel surface.
    pub const SURFACE_RAISED: &str = "var(--dr-surface-raised)";
    /// A card/inset surface.
    pub const SURFACE_OVERLAY: &str = "var(--dr-surface-overlay)";
    /// A hairline border between surfaces.
    pub const BORDER: &str = "var(--dr-border)";
    /// A stronger border for focus and active edges.
    pub const BORDER_STRONG: &str = "var(--dr-border-strong)";
    /// Primary text.
    pub const TEXT: &str = "var(--dr-text)";
    /// Secondary / supporting text.
    pub const TEXT_MUTED: &str = "var(--dr-text-muted)";
    /// Dimmed text for metadata.
    pub const TEXT_FAINT: &str = "var(--dr-text-faint)";
    /// The brand accent (cyan in dark, teal-blue in light).
    pub const ACCENT: &str = "var(--dr-accent)";
    /// A pressed/active variant of the accent.
    pub const ACCENT_STRONG: &str = "var(--dr-accent-strong)";
    /// A foreground that reads on top of the accent.
    pub const ON_ACCENT: &str = "var(--dr-on-accent)";
    /// Caution / awaiting a decision.
    pub const STATUS_WARN: &str = "var(--dr-status-warn)";
}

/// Global website CSS layered on top of [`darkrun_ui::tokens::THEME_CSS`]:
/// link hovers, the markdown `.dr-prose` typography, and a couple of resets.
/// Everything resolves against the `--dr-*` custom properties, so it tracks the
/// active theme (system default, or the `[data-theme]` override) automatically.
pub const GLOBAL_CSS: &str = r#"
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; }
a { color: inherit; }
.dr-navlink:hover { color: var(--dr-accent) !important; }
.dr-prose { color: var(--dr-text); line-height: 1.7; font-size: 16px; }
.dr-prose h1 { font-size: 30px; line-height: 1.2; margin: 0 0 16px; letter-spacing: -0.01em; }
.dr-prose h2 { font-size: 22px; margin: 32px 0 12px; color: var(--dr-text); }
.dr-prose h3 { font-size: 18px; margin: 24px 0 8px; }
.dr-prose p { margin: 0 0 16px; color: var(--dr-text-muted); }
.dr-prose ul, .dr-prose ol { margin: 0 0 16px; padding-left: 22px; color: var(--dr-text-muted); }
.dr-prose li { margin: 4px 0; }
.dr-prose strong { color: var(--dr-text); }
.dr-prose a { color: var(--dr-accent); text-decoration: none; }
.dr-prose a:hover { text-decoration: underline; }
.dr-prose code {
  font-family: var(--dr-font-mono); font-size: 13px;
  background: var(--dr-surface-overlay); border: 1px solid var(--dr-border);
  border-radius: 4px; padding: 1px 5px; color: var(--dr-accent);
}
.dr-prose pre {
  background: var(--dr-surface-raised); border: 1px solid var(--dr-border);
  border-radius: 8px; padding: 14px 16px; overflow-x: auto; margin: 0 0 16px;
}
.dr-prose pre code { background: none; border: none; padding: 0; color: var(--dr-text); }
.dr-prose table { border-collapse: collapse; width: 100%; margin: 0 0 16px; font-size: 14px; }
.dr-prose th, .dr-prose td { border: 1px solid var(--dr-border); padding: 8px 12px; text-align: left; }
.dr-prose th { background: var(--dr-surface-raised); color: var(--dr-text); }
.dr-grid { display: grid; gap: 16px; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); }

/* Markdown directive shortcodes (see content::preprocess_directives). */
.dr-md-callout {
  margin: 16px 0; padding: 12px 16px;
  background: var(--dr-surface-raised);
  border: 1px solid var(--dr-border);
  border-left: 3px solid var(--dr-accent);
  border-radius: 8px;
}
.dr-md-callout > :first-child { margin-top: 0; }
.dr-md-callout > :last-child { margin-bottom: 0; }
.dr-md-callout-info { border-left-color: var(--dr-accent); }
.dr-md-callout-warn {
  border-left-color: var(--dr-status-warn);
  background: color-mix(in srgb, var(--dr-status-warn) 7%, var(--dr-surface-raised));
}
.dr-md-callout-ok {
  border-left-color: var(--dr-status-ok);
  background: color-mix(in srgb, var(--dr-status-ok) 7%, var(--dr-surface-raised));
}
.dr-md-callout-danger {
  border-left-color: var(--dr-status-danger);
  background: color-mix(in srgb, var(--dr-status-danger) 7%, var(--dr-surface-raised));
}
.dr-md-keypoints {
  margin: 16px 0; padding: 14px 18px;
  background: var(--dr-surface-overlay);
  border: 1px solid var(--dr-border);
  border-radius: 8px;
}
.dr-md-keypoints-title {
  font-family: var(--dr-font-mono); font-size: 12px;
  letter-spacing: 0.08em; text-transform: uppercase;
  color: var(--dr-accent); margin-bottom: 8px;
}
.dr-md-keypoints ul { margin: 0; }
.dr-md-keypoints li { margin: 2px 0; }
.dr-md-keypoints > :last-child { margin-bottom: 0; }
.dr-md-columns, .dr-md-grid {
  display: grid; gap: 12px 24px; margin: 16px 0;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
}
.dr-md-columns ul, .dr-md-grid ul { margin: 0; padding-left: 0; list-style: none; }
.dr-md-columns li, .dr-md-grid li { margin: 0; }
.dr-md-steps { margin: 16px 0; padding-left: 0; list-style: none; counter-reset: dr-step; }
.dr-md-steps ol { margin: 0; padding-left: 0; list-style: none; counter-reset: dr-step; }
.dr-md-steps li { counter-increment: dr-step; position: relative; padding-left: 32px; margin: 8px 0; }
.dr-md-steps li::before {
  content: counter(dr-step); position: absolute; left: 0; top: 0;
  width: 22px; height: 22px; border-radius: 50%;
  background: var(--dr-surface-raised); border: 1px solid var(--dr-border);
  color: var(--dr-accent); font-family: var(--dr-font-mono); font-size: 12px;
  display: inline-flex; align-items: center; justify-content: center;
}
.dr-theme-seg { appearance: none; border: 0; cursor: pointer; font-family: var(--dr-font-mono); font-size: 11px; letter-spacing: 0.02em; padding: 4px 10px; border-radius: 999px; line-height: 1; white-space: nowrap; font-weight: 400; color: var(--dr-text); background-color: transparent; transition: background-color .15s ease, color .15s ease; }
.dr-theme-seg[aria-pressed="true"] { font-weight: 600; color: var(--dr-on-accent); background-color: var(--dr-accent); }

/* Slideshow position dots. The active surface is a wide accent pill; the rest
   are muted dots. Driven by a toggled class (not an inline style string) so the
   active state actually updates on navigation. */
.dr-dot { height: 8px; width: 8px; border: 0; border-radius: 999px; padding: 0; cursor: pointer;
  background: var(--dr-text-muted); }
.dr-dot.is-active { width: 24px; background: var(--dr-accent); }
"#;

/// A page section header: an eyebrow kicker, a title, and an optional lead.
#[component]
pub fn SectionHead(kicker: String, title: String, lead: Option<String>) -> Element {
    let kicker_style = format!(
        "font-family:{mono};font-size:12px;letter-spacing:0.08em;text-transform:uppercase;\
         color:{accent};margin-bottom:8px;",
        mono = tokens::FONT_MONO,
        accent = theme::ACCENT,
    );
    let title_style = format!(
        "font-family:{sans};font-size:28px;font-weight:700;letter-spacing:-0.01em;\
         color:{text};margin:0;",
        sans = tokens::FONT_SANS,
        text = theme::TEXT,
    );
    let lead_style = format!(
        "font-family:{sans};font-size:16px;color:{muted};margin:10px 0 0;max-width:64ch;",
        sans = tokens::FONT_SANS,
        muted = theme::TEXT_MUTED,
    );
    rsx! {
        div { style: "margin-bottom:24px;",
            div { style: "{kicker_style}", "{kicker}" }
            h1 { style: "{title_style}", "{title}" }
            if let Some(lead) = lead {
                p { style: "{lead_style}", "{lead}" }
            }
        }
    }
}

/// Render a [`Doc`]'s markdown body as a `.dr-prose` block.
///
/// The markdown is rendered to HTML at build time (or in the browser, once) and
/// injected with `dangerous_inner_html`. The source is our own embedded corpus,
/// not user input, so there is no untrusted-HTML concern.
#[component]
pub fn Prose(doc: Doc) -> Element {
    let html = doc.to_html();
    rsx! {
        article { class: "dr-prose", dangerous_inner_html: "{html}" }
    }
}

/// The six-phase legend strip: every phase with its hue and glyph, in order.
/// Reused on the landing page and the methodology page.
#[component]
pub fn PhaseLegend() -> Element {
    let wrap = "display:flex;gap:10px;flex-wrap:wrap;margin:8px 0 0;";
    rsx! {
        div { style: "{wrap}",
            for phase in Phase::ALL {
                {
                    let hue = phase.hue_var();
                    let chip = format!(
                        "display:inline-flex;align-items:center;gap:6px;\
                         font-family:{mono};font-size:12px;color:{base};\
                         border:1px solid {border};border-radius:999px;padding:4px 10px;",
                        mono = tokens::FONT_MONO,
                        base = hue.base,
                        border = theme::BORDER,
                    );
                    rsx! {
                        span { style: "{chip}",
                            span { "{tokens::GLYPH_ACTIVE}" }
                            "{phase.name()}"
                        }
                    }
                }
            }
        }
    }
}

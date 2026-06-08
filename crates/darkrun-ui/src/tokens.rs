//! darkrun design tokens — the single source of truth for color, type, and
//! spacing across the desktop app and the website.
//!
//! darkrun follows the **system appearance** (`prefers-color-scheme`) and also
//! accepts a manual override. The dark theme is the brand default: a near-black
//! base, surfaces layered toward the viewer, and a single cool-cyan accent that
//! carries interaction. The light theme mirrors it with a paper-white base and a
//! higher-contrast teal-blue accent (the dark cyan is too light on white). Each
//! station phase owns a hue in both themes so a pipeline reads at a glance.
//!
//! Two representations stay in lockstep:
//! - the Rust constants here (for SVG fills, inline styles, computed layout), and
//! - the [`THEME_CSS`] custom-property block (for class-based component styling).
//!
//! The Rust constants are the **dark** set (used wherever a value must be computed
//! before the browser resolves a custom property — SVG fills, inline styles). A
//! parallel **light** set ([`LIGHT_*`] / [`*_LIGHT`]) is available for renderers
//! that need to pick a theme explicitly. Class-based components consume the
//! `--dr-*` custom properties and theme automatically.
//!
//! When a token changes, change it in both places. The [`tokens::tests`] module
//! guards the obvious drift.

/// A hue paired with a readable foreground, expressed as CSS color strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hue {
    /// The primary color (fills, text, strokes).
    pub base: &'static str,
    /// A foreground that reads against [`Hue::base`] when used as a background.
    pub on: &'static str,
}

/// Near-black canvas — the deepest layer, behind everything.
pub const SURFACE_BASE: &str = "#07090c";
/// The default panel surface, one step toward the viewer.
pub const SURFACE_RAISED: &str = "#0e1217";
/// A card/inset surface, two steps up.
pub const SURFACE_OVERLAY: &str = "#161b22";
/// A hairline border between surfaces.
pub const BORDER: &str = "#222a33";
/// A stronger border for focus and active edges.
pub const BORDER_STRONG: &str = "#33404d";

/// Primary text on dark surfaces.
pub const TEXT: &str = "#e6edf3";
/// Secondary / supporting text.
pub const TEXT_MUTED: &str = "#9aa7b4";
/// Dimmed text for metadata and disabled states.
pub const TEXT_FAINT: &str = "#5b6773";

/// The cool-cyan brand accent (xterm 81 territory).
pub const ACCENT: &str = "#5fd7ff";
/// A pressed/active variant of the accent.
pub const ACCENT_STRONG: &str = "#33c5f5";
/// A foreground that reads on top of the accent.
pub const ON_ACCENT: &str = "#04141b";

// --- Phase hues -----------------------------------------------------------
// spec=grey review=blue manufacture=cyan audit=amber reflect=teal checkpoint=magenta

/// `spec` phase — neutral grey: the work is still being framed.
pub const PHASE_SPEC: Hue = Hue { base: "#8b98a5", on: "#0b0e12" };
/// `review` phase — blue: the spec is under examination.
pub const PHASE_REVIEW: Hue = Hue { base: "#5b8def", on: "#04101f" };
/// `manufacture` phase — cyan: output is being made (shares the brand accent family).
pub const PHASE_MANUFACTURE: Hue = Hue { base: "#5fd7ff", on: "#04141b" };
/// `audit` phase — amber: output is being checked against spec (folds in the old
/// quality-gate/tests work).
pub const PHASE_AUDIT: Hue = Hue { base: "#f0b429", on: "#1a1200" };
/// `reflect` phase — teal: the autonomous retrospective. A cool blue-green that
/// is distinct from both the review-blue and the manufacture-cyan around it.
pub const PHASE_REFLECT: Hue = Hue { base: "#2dd4bf", on: "#04201c" };
/// `checkpoint` phase — magenta: the gate fires.
pub const PHASE_CHECKPOINT: Hue = Hue { base: "#d160e8", on: "#1b0420" };

// --- Status hues ----------------------------------------------------------

/// Success / completed.
pub const STATUS_OK: &str = "#3fb950";
/// Caution / awaiting a decision.
pub const STATUS_WARN: &str = "#f0b429";
/// Blocked / failed.
pub const STATUS_DANGER: &str = "#f85149";
/// Informational / in progress.
pub const STATUS_INFO: &str = "#5fd7ff";

// --- Light theme constants ------------------------------------------------
// The light mirror of the set above, for renderers that must pick a theme
// explicitly (the class-based components flip automatically via the custom
// properties). Light derives a higher-contrast accent — the dark cyan washes
// out on white — and an ink variant for hairline borders/labels on accent.

/// Paper-white canvas — the base layer in the light theme.
pub const SURFACE_BASE_LIGHT: &str = "#f3f6f9";
/// The default panel surface in the light theme.
pub const SURFACE_RAISED_LIGHT: &str = "#ffffff";
/// A card/inset surface in the light theme.
pub const SURFACE_OVERLAY_LIGHT: &str = "#eef2f6";
/// A recessed/sink surface in the light theme (sidebar, wells).
pub const SURFACE_SINK_LIGHT: &str = "#e7edf3";
/// A hairline border in the light theme.
pub const BORDER_LIGHT: &str = "#dce3ea";
/// A stronger border for focus and active edges in the light theme.
pub const BORDER_STRONG_LIGHT: &str = "#c2ccd6";

/// Primary text on light surfaces.
pub const TEXT_LIGHT: &str = "#0e1217";
/// Secondary / supporting text in the light theme.
pub const TEXT_MUTED_LIGHT: &str = "#566370";
/// Dimmed text for metadata and disabled states in the light theme.
pub const TEXT_FAINT_LIGHT: &str = "#8a97a4";

/// The teal-blue brand accent in the light theme (higher contrast on white).
pub const ACCENT_LIGHT: &str = "#0e9fd6";
/// A pressed/active variant of the light accent (used for ink on light fills).
pub const ACCENT_STRONG_LIGHT: &str = "#0b7fae";
/// A foreground that reads on top of the light accent.
pub const ON_ACCENT_LIGHT: &str = "#ffffff";

/// `spec` phase — neutral grey, light theme.
pub const PHASE_SPEC_LIGHT: Hue = Hue { base: "#6b7884", on: "#ffffff" };
/// `review` phase — blue, light theme.
pub const PHASE_REVIEW_LIGHT: Hue = Hue { base: "#3b6fd0", on: "#ffffff" };
/// `manufacture` phase — teal-blue, light theme (shares the light accent).
pub const PHASE_MANUFACTURE_LIGHT: Hue = Hue { base: "#0e9fd6", on: "#ffffff" };
/// `audit` phase — amber, light theme.
pub const PHASE_AUDIT_LIGHT: Hue = Hue { base: "#b9791a", on: "#ffffff" };
/// `reflect` phase — teal, light theme.
pub const PHASE_REFLECT_LIGHT: Hue = Hue { base: "#11a392", on: "#ffffff" };
/// `checkpoint` phase — magenta, light theme.
pub const PHASE_CHECKPOINT_LIGHT: Hue = Hue { base: "#b443cf", on: "#ffffff" };

/// Success / completed, light theme.
pub const STATUS_OK_LIGHT: &str = "#2e9e43";
/// Caution / awaiting a decision, light theme.
pub const STATUS_WARN_LIGHT: &str = "#b9791a";
/// Blocked / failed, light theme.
pub const STATUS_DANGER_LIGHT: &str = "#d83c33";
/// Informational / in progress, light theme.
pub const STATUS_INFO_LIGHT: &str = "#0e9fd6";

// --- Type & spacing -------------------------------------------------------

/// The geometric sans used for UI chrome.
pub const FONT_SANS: &str =
    "\"Inter\", \"Segoe UI\", system-ui, -apple-system, sans-serif";
/// The monospace used for code, labels, and station glyphs.
pub const FONT_MONO: &str =
    "\"JetBrains Mono\", \"SF Mono\", \"Cascadia Code\", ui-monospace, monospace";

/// The base spacing unit, in pixels. The scale is `SPACE_UNIT * {1,2,3,4,6,8}`.
pub const SPACE_UNIT: u32 = 4;

/// The phase glyph strip motif: ● filled (done), ◉ current, ○ pending.
pub const GLYPH_DONE: char = '\u{25cf}';
/// The glyph for the active station/phase.
pub const GLYPH_ACTIVE: char = '\u{25c9}';
/// The glyph for a not-yet-reached station/phase.
pub const GLYPH_PENDING: char = '\u{25cb}';

/// The complete theme as custom-property blocks.
///
/// Mount this once (e.g. in a `<style>` tag or a linked stylesheet) and every
/// component class below resolves against it. The variable names mirror the
/// Rust constants so the two never diverge silently.
///
/// Theming model (locked):
/// - The **dark** tokens are the default `:root`, so dark is the brand baseline.
/// - A `@media (prefers-color-scheme: light)` block applies the **light** tokens,
///   so the app/site follow the system appearance automatically.
/// - `:root[data-theme="light"]` / `:root[data-theme="dark"]` are the **manual
///   override**. Because an attribute selector on `:root` has higher specificity
///   than the media query's `:root`, the override wins regardless of system
///   preference. Setting `data-theme` to `light`/`dark` pins the theme; removing
///   the attribute returns to "System" (the media query / dark default).
///
/// The wordmark drives its per-theme look off `--dr-wm-dark-*` / `--dr-wm-run`,
/// which the light blocks redefine — so the wordmark flips with the theme without
/// hard-coding either side.
pub const THEME_CSS: &str = r#":root{
  --dr-surface-base:#07090c;
  --dr-surface-raised:#0e1217;
  --dr-surface-overlay:#161b22;
  --dr-surface-sink:#04060a;
  --dr-border:#222a33;
  --dr-border-strong:#33404d;
  --dr-text:#e6edf3;
  --dr-text-muted:#9aa7b4;
  --dr-text-faint:#5b6773;
  --dr-accent:#5fd7ff;
  --dr-accent-strong:#33c5f5;
  --dr-on-accent:#04141b;
  --dr-phase-spec:#8b98a5;
  --dr-phase-review:#5b8def;
  --dr-phase-manufacture:#5fd7ff;
  --dr-phase-audit:#f0b429;
  --dr-phase-reflect:#2dd4bf;
  --dr-phase-checkpoint:#d160e8;
  --dr-status-ok:#3fb950;
  --dr-status-warn:#f0b429;
  --dr-status-danger:#f85149;
  --dr-status-info:#5fd7ff;
  --dr-font-sans:"Inter","Segoe UI",system-ui,-apple-system,sans-serif;
  --dr-font-mono:"JetBrains Mono","SF Mono","Cascadia Code",ui-monospace,monospace;
  --dr-space:4px;
  --dr-radius:8px;
  --dr-radius-sm:5px;
  /* Wordmark, dark theme: "dark" outlined cyan over a base fill (stroke painted
     under the fill), "run" solid white. */
  --dr-wm-dark-fill:var(--dr-surface-base);
  --dr-wm-dark-stroke:var(--dr-accent);
  --dr-wm-dark-stroke-width:1.5px;
  --dr-wm-run:var(--dr-text);
  /* "run" is SOLID WHITE (text fill) with a cyan outline matching "dark", so it
     reads filled — not hollow — against the dark canvas. */
  --dr-wm-run-fill:var(--dr-text);
  --dr-wm-run-stroke:var(--dr-accent);
  --dr-wm-run-stroke-width:1.5px;
  color-scheme:dark;
}
/* The light tokens, factored out so both the media query and the manual override
   can apply them without duplication. */
@media (prefers-color-scheme:light){
  :root{
    --dr-surface-base:#f3f6f9;
    --dr-surface-raised:#ffffff;
    --dr-surface-overlay:#eef2f6;
    --dr-surface-sink:#e7edf3;
    --dr-border:#dce3ea;
    --dr-border-strong:#c2ccd6;
    --dr-text:#0e1217;
    --dr-text-muted:#566370;
    --dr-text-faint:#8a97a4;
    --dr-accent:#0e9fd6;
    --dr-accent-strong:#0b7fae;
    --dr-on-accent:#ffffff;
    --dr-phase-spec:#6b7884;
    --dr-phase-review:#3b6fd0;
    --dr-phase-manufacture:#0e9fd6;
    --dr-phase-audit:#b9791a;
    --dr-phase-reflect:#11a392;
    --dr-phase-checkpoint:#b443cf;
    --dr-status-ok:#2e9e43;
    --dr-status-warn:#b9791a;
    --dr-status-danger:#d83c33;
    --dr-status-info:#0e9fd6;
    /* Wordmark, light theme: "dark" SOLID BLACK (--dr-text) with no stroke,
       "run" the teal accent. */
    --dr-wm-dark-fill:var(--dr-text);
    --dr-wm-dark-stroke:transparent;
    --dr-wm-dark-stroke-width:0;
    --dr-wm-run:var(--dr-accent);
    /* "run" is solid teal in light (matches the manual [data-theme=light]). */
    --dr-wm-run-fill:var(--dr-accent);
    --dr-wm-run-stroke:transparent;
    --dr-wm-run-stroke-width:0;
    color-scheme:light;
  }
}
/* Manual override — wins over the media query (attribute selector on :root has
   higher specificity than the bare :root the media query targets). */
:root[data-theme="dark"]{
  --dr-surface-base:#07090c;
  --dr-surface-raised:#0e1217;
  --dr-surface-overlay:#161b22;
  --dr-surface-sink:#04060a;
  --dr-border:#222a33;
  --dr-border-strong:#33404d;
  --dr-text:#e6edf3;
  --dr-text-muted:#9aa7b4;
  --dr-text-faint:#5b6773;
  --dr-accent:#5fd7ff;
  --dr-accent-strong:#33c5f5;
  --dr-on-accent:#04141b;
  --dr-phase-spec:#8b98a5;
  --dr-phase-review:#5b8def;
  --dr-phase-manufacture:#5fd7ff;
  --dr-phase-audit:#f0b429;
  --dr-phase-reflect:#2dd4bf;
  --dr-phase-checkpoint:#d160e8;
  --dr-status-ok:#3fb950;
  --dr-status-warn:#f0b429;
  --dr-status-danger:#f85149;
  --dr-status-info:#5fd7ff;
  --dr-wm-dark-fill:var(--dr-surface-base);
  --dr-wm-dark-stroke:var(--dr-accent);
  --dr-wm-dark-stroke-width:1.5px;
  --dr-wm-run:var(--dr-text);
  /* "run" is SOLID WHITE (text fill) with a cyan outline matching "dark", so it
     reads filled — not hollow — against the dark canvas. */
  --dr-wm-run-fill:var(--dr-text);
  --dr-wm-run-stroke:var(--dr-accent);
  --dr-wm-run-stroke-width:1.5px;
  color-scheme:dark;
}
:root[data-theme="light"]{
  --dr-surface-base:#f3f6f9;
  --dr-surface-raised:#ffffff;
  --dr-surface-overlay:#eef2f6;
  --dr-surface-sink:#e7edf3;
  --dr-border:#dce3ea;
  --dr-border-strong:#c2ccd6;
  --dr-text:#0e1217;
  --dr-text-muted:#566370;
  --dr-text-faint:#8a97a4;
  --dr-accent:#0e9fd6;
  --dr-accent-strong:#0b7fae;
  --dr-on-accent:#ffffff;
  --dr-phase-spec:#6b7884;
  --dr-phase-review:#3b6fd0;
  --dr-phase-manufacture:#0e9fd6;
  --dr-phase-audit:#b9791a;
  --dr-phase-reflect:#11a392;
  --dr-phase-checkpoint:#b443cf;
  --dr-status-ok:#2e9e43;
  --dr-status-warn:#b9791a;
  --dr-status-danger:#d83c33;
  --dr-status-info:#0e9fd6;
  --dr-wm-dark-fill:var(--dr-text);
  --dr-wm-dark-stroke:transparent;
  --dr-wm-dark-stroke-width:0;
  --dr-wm-run:var(--dr-accent);
  /* Light: "run" stays solid teal (no outline) — nothing to match against a
     solid-black "dark". */
  --dr-wm-run-fill:var(--dr-accent);
  --dr-wm-run-stroke:transparent;
  --dr-wm-run-stroke-width:0;
  color-scheme:light;
}
html,body{
  background:var(--dr-surface-base);
  color:var(--dr-text);
  font-family:var(--dr-font-sans);
}
/* Theme-aware wordmark (static variants). The "dark" segment paints its stroke
   under the fill so the outer outline stays clean; in light the stroke collapses
   to 0 / transparent and the fill becomes solid --dr-text. "run" follows
   --dr-wm-run (white in dark, teal accent in light). Both flip automatically. */
.dr-wordmark-themed .dr-wordmark-dark{
  color:var(--dr-wm-dark-fill);
  paint-order:stroke;
  -webkit-text-stroke:var(--dr-wm-dark-stroke-width) var(--dr-wm-dark-stroke);
  text-stroke:var(--dr-wm-dark-stroke-width) var(--dr-wm-dark-stroke);
}
.dr-wordmark-themed .dr-wordmark-run{
  color:var(--dr-wm-run-fill);
  paint-order:stroke;
  -webkit-text-stroke:var(--dr-wm-run-stroke-width) var(--dr-wm-run-stroke);
  text-stroke:var(--dr-wm-run-stroke-width) var(--dr-wm-run-stroke);
}
/* Lights-out wordmark (interactive site logo).
   DARK theme: the "dark" glyphs sit near-black with a cyan stroke painted under
   the fill — invisible-until-lit on the near-black header — then glow on hover and
   flicker out on blur (the "lights out" motif).
   LIGHT theme: there is nothing to hide on a paper background, so it renders as the
   static brand wordmark — solid black "dark", no stroke, no glow/flicker. */
.dr-wordmark-anim .dr-wordmark-dark{
  paint-order:stroke;
  color:#07090c; text-shadow:none;
  -webkit-text-stroke:1.5px #5fd7ff; text-stroke:1.5px #5fd7ff;
}
.dr-wordmark-anim[data-anim="lit"] .dr-wordmark-dark{
  color:#5fd7ff; text-shadow:0 0 22px #5fd7ffcc, 0 0 6px #5fd7ff;
  transition:color .12s ease, text-shadow .12s ease;
}
.dr-wordmark-anim[data-anim="flicker"] .dr-wordmark-dark{ animation:dr-lightsout 1.3s ease both; }
@keyframes dr-lightsout{
  0%   { color:#5fd7ff; text-shadow:0 0 22px #5fd7ffcc, 0 0 6px #5fd7ff; }
  18%  { color:#5fd7ff; text-shadow:0 0 20px #5fd7ffbb; }
  24%  { color:#07090c; text-shadow:none; }
  30%  { color:#5fd7ff; text-shadow:0 0 16px #5fd7ff99; }
  36%  { color:#07090c; text-shadow:none; }
  44%  { color:#3a8aa0; text-shadow:0 0 6px #5fd7ff44; }
  50%  { color:#07090c; text-shadow:none; }
  58%  { color:#1c4a58; text-shadow:none; }
  64%  { color:#07090c; text-shadow:none; }
  100% { color:#07090c; text-shadow:none; }
}
/* Light theme: solid black glyphs, stroke + glow + flicker all suppressed. Both
   the media query (System on a light OS) and the manual [data-theme="light"]
   override neutralize the lights-out treatment. */
.dr-wordmark-anim.dr-light-static .dr-wordmark-dark,
:root[data-theme="light"] .dr-wordmark-anim .dr-wordmark-dark{
  color:var(--dr-text) !important;
  -webkit-text-stroke:0 !important; text-stroke:0 !important;
  text-shadow:none !important; animation:none !important;
}
@media (prefers-color-scheme:light){
  :root:not([data-theme="dark"]) .dr-wordmark-anim .dr-wordmark-dark{
    color:var(--dr-text) !important;
    -webkit-text-stroke:0 !important; text-stroke:0 !important;
    text-shadow:none !important; animation:none !important;
  }
}
@media (prefers-reduced-motion:reduce){
  .dr-wordmark-anim[data-anim="flicker"] .dr-wordmark-dark{ animation:none; color:#07090c; }
  .dr-wordmark-anim[data-anim="lit"] .dr-wordmark-dark{ transition:none; }
}
"#;

/// CSS custom-property references (`var(--dr-*)`) — the **theme-aware twins** of
/// the hex constants above.
///
/// Use these wherever a value is handed straight to the DOM/SVG as a color
/// (inline `style:` strings, SVG `fill`/`stroke`) so it flips automatically when
/// the active theme changes. Reach for the **hex constants** only where a value
/// must be computed before the browser resolves a custom property — alpha math
/// (`"{accent}33"`), `<canvas>`, or a `.icns`/raster pipeline.
///
/// A `Hue` whose `base`/`on` reference these properties is produced by
/// [`Hue::var`]; the per-phase var hues come from [`crate::kinds::Phase::hue_var`].
pub mod var {
    /// Near-black canvas — the deepest layer (themed).
    pub const SURFACE_BASE: &str = "var(--dr-surface-base)";
    /// The default panel surface (themed).
    pub const SURFACE_RAISED: &str = "var(--dr-surface-raised)";
    /// A card/inset surface (themed).
    pub const SURFACE_OVERLAY: &str = "var(--dr-surface-overlay)";
    /// A recessed/sink surface — sidebars, wells (themed).
    pub const SURFACE_SINK: &str = "var(--dr-surface-sink)";
    /// A hairline border (themed).
    pub const BORDER: &str = "var(--dr-border)";
    /// A stronger border for focus and active edges (themed).
    pub const BORDER_STRONG: &str = "var(--dr-border-strong)";
    /// Primary text (themed).
    pub const TEXT: &str = "var(--dr-text)";
    /// Secondary / supporting text (themed).
    pub const TEXT_MUTED: &str = "var(--dr-text-muted)";
    /// Dimmed text for metadata and disabled states (themed).
    pub const TEXT_FAINT: &str = "var(--dr-text-faint)";
    /// The brand accent (themed: cool-cyan dark, teal-blue light).
    pub const ACCENT: &str = "var(--dr-accent)";
    /// A pressed/active variant of the accent (themed).
    pub const ACCENT_STRONG: &str = "var(--dr-accent-strong)";
    /// A foreground that reads on top of the accent — and, more generally, the
    /// readable ink to drop on **any** vivid hue used as a fill (near-black in
    /// dark, white in light), so it doubles as the phase/status `on` color.
    pub const ON_ACCENT: &str = "var(--dr-on-accent)";

    /// `spec` phase base (themed).
    pub const PHASE_SPEC: &str = "var(--dr-phase-spec)";
    /// `review` phase base (themed).
    pub const PHASE_REVIEW: &str = "var(--dr-phase-review)";
    /// `manufacture` phase base (themed).
    pub const PHASE_MANUFACTURE: &str = "var(--dr-phase-manufacture)";
    /// `audit` phase base (themed).
    pub const PHASE_AUDIT: &str = "var(--dr-phase-audit)";
    /// `reflect` phase base (themed).
    pub const PHASE_REFLECT: &str = "var(--dr-phase-reflect)";
    /// `checkpoint` phase base (themed).
    pub const PHASE_CHECKPOINT: &str = "var(--dr-phase-checkpoint)";

    /// Success / completed (themed).
    pub const STATUS_OK: &str = "var(--dr-status-ok)";
    /// Caution / awaiting a decision (themed).
    pub const STATUS_WARN: &str = "var(--dr-status-warn)";
    /// Blocked / failed (themed).
    pub const STATUS_DANGER: &str = "var(--dr-status-danger)";
    /// Informational / in progress (themed).
    pub const STATUS_INFO: &str = "var(--dr-status-info)";

    /// The themed var twin of a phase name's base color, case-insensitive.
    /// `None` for an unknown phase (mirrors [`super::phase_hue`]).
    pub fn phase(name: &str) -> Option<&'static str> {
        Some(match () {
            _ if name.eq_ignore_ascii_case("spec") => PHASE_SPEC,
            _ if name.eq_ignore_ascii_case("review") => PHASE_REVIEW,
            _ if name.eq_ignore_ascii_case("manufacture") => PHASE_MANUFACTURE,
            _ if name.eq_ignore_ascii_case("audit") => PHASE_AUDIT,
            _ if name.eq_ignore_ascii_case("reflect") => PHASE_REFLECT,
            _ if name.eq_ignore_ascii_case("checkpoint") => PHASE_CHECKPOINT,
            _ => return None,
        })
    }
}

/// The six station phases, in canonical order, each with its name and hue.
///
/// Mirrors `darkrun_core::domain::StationPhase` but lives here so the UI crate
/// stays dependency-light (no core dependency, wasm-safe). Order is load-bearing:
/// `spec -> review -> manufacture -> audit -> reflect -> checkpoint`.
pub const PHASES: [(&str, Hue); 6] = [
    ("spec", PHASE_SPEC),
    ("review", PHASE_REVIEW),
    ("manufacture", PHASE_MANUFACTURE),
    ("audit", PHASE_AUDIT),
    ("reflect", PHASE_REFLECT),
    ("checkpoint", PHASE_CHECKPOINT),
];

/// The six software-factory stations, in order: the assembly line itself.
pub const STATIONS: [&str; 6] = [
    "frame", "specify", "shape", "build", "prove", "harden",
];

/// Resolve a phase name (case-insensitive) to its hue, if known.
pub fn phase_hue(name: &str) -> Option<Hue> {
    PHASES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_css_defines_every_rust_token() {
        // Every primary color constant must appear in the CSS block so the two
        // representations cannot silently drift apart.
        for value in [
            SURFACE_BASE,
            SURFACE_RAISED,
            SURFACE_OVERLAY,
            BORDER,
            BORDER_STRONG,
            TEXT,
            TEXT_MUTED,
            TEXT_FAINT,
            ACCENT,
            ACCENT_STRONG,
            ON_ACCENT,
            PHASE_SPEC.base,
            PHASE_REVIEW.base,
            PHASE_MANUFACTURE.base,
            PHASE_AUDIT.base,
            PHASE_REFLECT.base,
            PHASE_CHECKPOINT.base,
            // The light mirror must also appear (in the media query + override).
            SURFACE_BASE_LIGHT,
            SURFACE_RAISED_LIGHT,
            SURFACE_OVERLAY_LIGHT,
            BORDER_LIGHT,
            BORDER_STRONG_LIGHT,
            TEXT_LIGHT,
            TEXT_MUTED_LIGHT,
            TEXT_FAINT_LIGHT,
            ACCENT_LIGHT,
            ACCENT_STRONG_LIGHT,
            ON_ACCENT_LIGHT,
            PHASE_SPEC_LIGHT.base,
            PHASE_REVIEW_LIGHT.base,
            PHASE_MANUFACTURE_LIGHT.base,
            PHASE_AUDIT_LIGHT.base,
            PHASE_REFLECT_LIGHT.base,
            PHASE_CHECKPOINT_LIGHT.base,
        ] {
            assert!(
                THEME_CSS.contains(value),
                "THEME_CSS is missing token value {value}"
            );
        }
    }

    #[test]
    fn theme_css_defines_the_three_theme_scopes() {
        // Dark default, the media-query light block, and both manual overrides.
        assert!(THEME_CSS.contains("@media (prefers-color-scheme:light)"));
        assert!(THEME_CSS.contains(":root[data-theme=\"light\"]"));
        assert!(THEME_CSS.contains(":root[data-theme=\"dark\"]"));
    }

    #[test]
    fn manufacture_light_shares_the_light_accent() {
        assert_eq!(PHASE_MANUFACTURE_LIGHT.base, ACCENT_LIGHT);
    }

    #[test]
    fn phase_hue_is_case_insensitive_and_ordered() {
        assert_eq!(phase_hue("spec"), Some(PHASE_SPEC));
        assert_eq!(phase_hue("CHECKPOINT"), Some(PHASE_CHECKPOINT));
        assert_eq!(phase_hue("unknown"), None);
        assert_eq!(PHASES.len(), 6);
        assert_eq!(PHASES[0].0, "spec");
        assert_eq!(PHASES[5].0, "checkpoint");
    }

    #[test]
    fn manufacture_shares_the_brand_accent() {
        assert_eq!(PHASE_MANUFACTURE.base, ACCENT);
    }
}

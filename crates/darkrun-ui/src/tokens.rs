//! darkrun design tokens — the single source of truth for color, type, and
//! spacing across the desktop app and the website.
//!
//! darkrun is **dark only**: there is no light theme. The base is near-black,
//! surfaces are layered toward the viewer, and a single cool-cyan accent carries
//! interaction. Each station phase owns a hue so a pipeline reads at a glance.
//!
//! Two representations stay in lockstep:
//! - the Rust constants here (for SVG fills, inline styles, computed layout), and
//! - the [`THEME_CSS`] custom-property block (for class-based component styling).
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

/// The complete dark theme as a `:root { --token: value }` block.
///
/// Mount this once (e.g. in a `<style>` tag or a linked stylesheet) and every
/// component class below resolves against it. The variable names mirror the
/// Rust constants so the two never diverge silently.
pub const THEME_CSS: &str = r#":root{
  --dr-surface-base:#07090c;
  --dr-surface-raised:#0e1217;
  --dr-surface-overlay:#161b22;
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
}
html,body{
  background:var(--dr-surface-base);
  color:var(--dr-text);
  font-family:var(--dr-font-sans);
  color-scheme:dark;
}
"#;

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
        ] {
            assert!(
                THEME_CSS.contains(value),
                "THEME_CSS is missing token value {value}"
            );
        }
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

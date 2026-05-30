//! Small, `Copy` enums shared by components: visual tones and the phase taxonomy.
//!
//! These deliberately do NOT depend on `darkrun-core`. The UI crate stays
//! self-contained and wasm-light; callers map their domain enums into these at
//! the boundary (a one-line `match`).

use crate::tokens::{self, Hue};

/// The six station phases. Mirrors `darkrun_core::domain::StationPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Specify the work.
    Spec,
    /// Review the spec.
    Review,
    /// Manufacture the output.
    Manufacture,
    /// Audit against the spec (folds in the old quality-gate / tests work).
    Audit,
    /// Reflect — the autonomous retrospective feeding the run-level reflections.
    Reflect,
    /// The checkpoint gate.
    Checkpoint,
}

impl Phase {
    /// All phases in canonical order.
    pub const ALL: [Phase; 6] = [
        Phase::Spec,
        Phase::Review,
        Phase::Manufacture,
        Phase::Audit,
        Phase::Reflect,
        Phase::Checkpoint,
    ];

    /// The lowercase canonical name (`"spec"`, `"checkpoint"`, ...).
    pub fn name(self) -> &'static str {
        match self {
            Phase::Spec => "spec",
            Phase::Review => "review",
            Phase::Manufacture => "manufacture",
            Phase::Audit => "audit",
            Phase::Reflect => "reflect",
            Phase::Checkpoint => "checkpoint",
        }
    }

    /// The hue this phase owns.
    pub fn hue(self) -> Hue {
        match self {
            Phase::Spec => tokens::PHASE_SPEC,
            Phase::Review => tokens::PHASE_REVIEW,
            Phase::Manufacture => tokens::PHASE_MANUFACTURE,
            Phase::Audit => tokens::PHASE_AUDIT,
            Phase::Reflect => tokens::PHASE_REFLECT,
            Phase::Checkpoint => tokens::PHASE_CHECKPOINT,
        }
    }

    /// Parse a phase name (case-insensitive). Unknown names yield `None`.
    pub fn from_name(name: &str) -> Option<Phase> {
        Phase::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(name))
    }
}

/// The visual progress state of a step in a pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Already completed — rendered with the ● glyph, full hue.
    Done,
    /// The current step — rendered with the ◉ glyph, full hue, emphasized.
    Active,
    /// Not yet reached — rendered with the ○ glyph, dimmed.
    Pending,
}

impl Step {
    /// The station-pipeline glyph for this step (● ◉ ○).
    pub fn glyph(self) -> char {
        match self {
            Step::Done => tokens::GLYPH_DONE,
            Step::Active => tokens::GLYPH_ACTIVE,
            Step::Pending => tokens::GLYPH_PENDING,
        }
    }
}

/// A semantic tone applied to badges, buttons, and bars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// The cool-cyan brand accent — the default emphasis.
    #[default]
    Accent,
    /// Neutral / muted — low emphasis chrome.
    Neutral,
    /// Success / completed.
    Ok,
    /// Caution / awaiting a decision.
    Warn,
    /// Blocked / failed.
    Danger,
    /// Informational.
    Info,
}

impl Tone {
    /// The primary color for this tone.
    pub fn color(self) -> &'static str {
        match self {
            Tone::Accent => tokens::ACCENT,
            Tone::Neutral => tokens::TEXT_MUTED,
            Tone::Ok => tokens::STATUS_OK,
            Tone::Warn => tokens::STATUS_WARN,
            Tone::Danger => tokens::STATUS_DANGER,
            Tone::Info => tokens::STATUS_INFO,
        }
    }

    /// A foreground that reads on top of [`Tone::color`] used as a fill.
    pub fn on(self) -> &'static str {
        match self {
            Tone::Accent => tokens::ON_ACCENT,
            Tone::Neutral => tokens::TEXT,
            // The status hues are bright enough to take a near-black foreground.
            _ => tokens::SURFACE_BASE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_names_round_trip() {
        for p in Phase::ALL {
            assert_eq!(Phase::from_name(p.name()), Some(p));
        }
        assert_eq!(Phase::from_name("MANUFACTURE"), Some(Phase::Manufacture));
        assert_eq!(Phase::from_name("nope"), None);
    }

    #[test]
    fn step_glyphs_are_distinct() {
        assert_ne!(Step::Done.glyph(), Step::Active.glyph());
        assert_ne!(Step::Active.glyph(), Step::Pending.glyph());
    }

    #[test]
    fn tone_default_is_accent() {
        assert_eq!(Tone::default(), Tone::Accent);
        assert_eq!(Tone::Ok.color(), tokens::STATUS_OK);
    }
}

//! Small domain chips/badges: [`CheckpointBadge`], [`RiskChip`], and the
//! [`RightSizeStrip`]. Each is a thin, inline-styled element resolving against
//! the dark tokens — composable into station detail views.

use dioxus::prelude::*;

use crate::components::factory::CheckpointKind;
use crate::flow::checkpoint_hue;
use crate::tokens;

/// A badge for a station's checkpoint gate, hued by kind (auto=green,
/// ask/await=amber, external=info-cyan) and prefixed with a gate glyph.
#[component]
pub fn CheckpointBadge(
    /// The gate kind.
    kind: CheckpointKind,
    /// When true, render filled rather than outlined.
    #[props(default = false)]
    filled: bool,
) -> Element {
    let label = match kind {
        CheckpointKind::Auto => "auto",
        CheckpointKind::Ask => "ask",
        CheckpointKind::External => "external",
        CheckpointKind::Await => "await",
    };
    let color = checkpoint_hue(kind);
    let style = if filled {
        format!(
            "background:{color};color:{on};border:1px solid {color};",
            on = tokens::SURFACE_BASE,
        )
    } else {
        format!("background:transparent;color:{color};border:1px solid {color};")
    };
    let style = format!(
        "{style}display:inline-flex;align-items:center;gap:5px;\
         font-family:{mono};font-size:11px;font-weight:600;line-height:1;\
         padding:3px 8px;border-radius:999px;white-space:nowrap;",
        mono = tokens::FONT_MONO,
    );
    rsx! {
        span { class: "dr-checkpoint-badge", "data-kind": label, style: "{style}",
            span { "◇" }
            "checkpoint:{label}"
        }
    }
}

/// A chip naming the class of risk a station eliminates — the reason the station
/// exists. Rendered in danger-red as a struck-through "killed risk" marker.
#[component]
pub fn RiskChip(
    /// The risk class label (e.g. "wrong problem framed").
    risk: String,
) -> Element {
    let style = format!(
        "display:inline-flex;align-items:center;gap:5px;\
         font-family:{mono};font-size:11px;line-height:1;\
         color:{danger};background:transparent;\
         border:1px dashed {danger};border-radius:6px;padding:3px 8px;white-space:nowrap;",
        mono = tokens::FONT_MONO,
        danger = tokens::STATUS_DANGER,
    );
    let strike = format!(
        "text-decoration:line-through;text-decoration-color:{danger};color:{muted};",
        danger = tokens::STATUS_DANGER,
        muted = tokens::TEXT_MUTED,
    );
    rsx! {
        span { class: "dr-risk-chip", title: "risk this station eliminates", style: "{style}",
            span { style: "font-weight:700;", "kills" }
            span { style: "{strike}", "{risk}" }
        }
    }
}

/// One entry in a [`RightSizeStrip`]: a run size label and the stations it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RightSizeTier {
    /// The size label (e.g. "tiny", "small", "full").
    pub label: String,
    /// Station slugs that survive at this size, in pipeline order.
    pub kept: Vec<String>,
}

impl RightSizeTier {
    /// Construct a tier.
    pub fn new(label: impl Into<String>, kept: Vec<String>) -> Self {
        Self { label: label.into(), kept }
    }
}

/// Shows how a run auto-right-sizes: for each size tier, which stations stay and
/// which collapse away. The full pipeline is the reference row; smaller tiers
/// render dropped stations as faint, struck-through chips.
#[component]
pub fn RightSizeStrip(
    /// The full pipeline, in order — the reference set.
    full: Vec<String>,
    /// Size tiers from smallest to largest (the last typically equals `full`).
    tiers: Vec<RightSizeTier>,
) -> Element {
    let wrap = format!(
        "display:flex;flex-direction:column;gap:8px;font-family:{mono};",
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div { class: "dr-rightsize-strip", style: "{wrap}",
            for tier in tiers.iter() {
                {
                    let label = tier.label.clone();
                    let kept = tier.kept.clone();
                    let row = "display:flex;align-items:center;gap:8px;flex-wrap:wrap;";
                    let label_style = format!(
                        "min-width:56px;font-size:11px;text-transform:uppercase;\
                         letter-spacing:0.06em;color:{accent};font-weight:700;",
                        accent = tokens::ACCENT,
                    );
                    rsx! {
                        div { style: "{row}", "data-tier": "{tier.label}",
                            span { style: "{label_style}", "{label}" }
                            for slug in full.iter() {
                                {
                                    let present = kept.iter().any(|k| k == slug);
                                    let chip = if present {
                                        format!(
                                            "color:{text};border:1px solid {border};background:{surface};",
                                            text = tokens::TEXT,
                                            border = tokens::BORDER_STRONG,
                                            surface = tokens::SURFACE_OVERLAY,
                                        )
                                    } else {
                                        format!(
                                            "color:{faint};border:1px dashed {border};background:transparent;\
                                             text-decoration:line-through;",
                                            faint = tokens::TEXT_FAINT,
                                            border = tokens::BORDER,
                                        )
                                    };
                                    let chip = format!(
                                        "{chip}font-size:11px;padding:2px 8px;border-radius:5px;white-space:nowrap;"
                                    );
                                    rsx! {
                                        span {
                                            style: "{chip}",
                                            "data-present": "{present}",
                                            "{slug}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}


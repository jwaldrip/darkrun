//! [`ExpandableRole`] — a collapsible card rendering a Role's real instruction
//! body, plus [`ArtifactCard`] for a station's locked artifact.
//!
//! The card shows a one-line summary, the agent_type/model chips, and a kind
//! badge (explorer / worker / reviewer — and, for a worker, its Make / Challenge
//! / Resolve beat). Expanding reveals the role's full markdown instructions.
//!
//! The UI crate stays markdown-renderer-agnostic: the caller renders the role
//! body to HTML with whatever the site already uses and passes it as `body_html`
//! (injected into a `.dr-prose` block), or supplies a `body` slot of pre-built
//! `Element`s. The corpus is our own embedded content, never user input.

use dioxus::prelude::*;

use crate::components::primitives::Badge;
use crate::flow::PassBeat;
use crate::kinds::Tone;
use crate::tokens;

/// The kind of role a card describes, mirroring `darkrun_content::RoleKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    /// Gathers context in the Explore phase.
    Explorer,
    /// Performs a beat of a Pass (Make / Challenge / Resolve).
    Worker,
    /// Verifies output independently in the Review phase.
    Reviewer,
}

impl RoleKind {
    /// The lowercase label.
    pub fn label(self) -> &'static str {
        match self {
            RoleKind::Explorer => "explorer",
            RoleKind::Worker => "worker",
            RoleKind::Reviewer => "reviewer",
        }
    }

    /// The tone this kind paints its badge in.
    pub fn tone(self) -> Tone {
        match self {
            // explorers read as the "spec/explore" neutral, workers as the
            // manufacture accent, reviewers as the review-blue info.
            RoleKind::Explorer => Tone::Neutral,
            RoleKind::Worker => Tone::Accent,
            RoleKind::Reviewer => Tone::Info,
        }
    }
}

/// A collapsible card for one role.
///
/// Provide exactly one body source: `body_html` (pre-rendered markdown, injected
/// into a prose block) or the `children` slot. If both are empty the card still
/// renders its header.
#[component]
pub fn ExpandableRole(
    /// Role slug / name.
    name: String,
    /// The kind (explorer / worker / reviewer) — drives the badge.
    kind: RoleKind,
    /// The agent type string shown as a chip (e.g. "worker", "reviewer").
    #[props(default)]
    agent_type: Option<String>,
    /// The model the role runs on, shown as a chip when present.
    #[props(default)]
    model: Option<String>,
    /// For a worker, which Pass beat it performs (Make / Challenge / Resolve).
    #[props(default)]
    beat: Option<PassBeat>,
    /// A one-line summary shown collapsed (e.g. the first instruction line).
    #[props(default)]
    summary: Option<String>,
    /// Pre-rendered markdown HTML for the role body. Injected into `.dr-prose`.
    #[props(default)]
    body_html: Option<String>,
    /// Start expanded.
    #[props(default = false)]
    open: bool,
    /// Optional custom body slot (used when `body_html` is not supplied).
    #[props(default)]
    children: Element,
) -> Element {
    let mut expanded = use_signal(|| open);

    let tone = kind.tone();
    let rail = tone.color();
    let card = format!(
        "background:{surface};border:1px solid {border};border-left:3px solid {rail};\
         border-radius:8px;overflow:hidden;",
        surface = tokens::SURFACE_OVERLAY,
        border = tokens::BORDER,
    );
    let header = format!(
        "display:flex;align-items:center;gap:8px;padding:10px 12px;cursor:pointer;\
         font-family:{sans};",
        sans = tokens::FONT_SANS,
    );
    let name_style = format!(
        "font-size:14px;font-weight:700;color:{text};",
        text = tokens::TEXT,
    );
    let summary_style = format!(
        "flex:1;min-width:0;font-size:12px;color:{muted};overflow:hidden;\
         text-overflow:ellipsis;white-space:nowrap;",
        muted = tokens::TEXT_MUTED,
    );
    let caret = if expanded() { "▾" } else { "▸" };
    let caret_style = format!("color:{};font-size:12px;width:12px;", tokens::TEXT_FAINT);
    let body_wrap = format!(
        "padding:16px 20px 18px;border-top:1px solid {border};",
        border = tokens::BORDER,
    );

    rsx! {
        div { class: "dr-role-card", "data-kind": kind.label(), "data-open": "{expanded()}", style: "{card}",
            div {
                class: "dr-role-header",
                style: "{header}",
                onclick: move |_| {
                    let now = expanded();
                    expanded.set(!now);
                },
                span { style: "{caret_style}", "{caret}" }
                span { style: "{name_style}", "{name}" }
                Badge { tone, filled: true, "{kind.label()}" }
                if let Some(b) = beat {
                    Badge { tone: Tone::Accent, "beat: {b.label()}" }
                }
                if !expanded() {
                    if let Some(s) = summary.clone() {
                        span { style: "{summary_style}", "{s}" }
                    }
                }
                div { style: "margin-left:auto;display:flex;gap:6px;align-items:center;",
                    if let Some(a) = agent_type.clone() {
                        Badge { tone: Tone::Neutral, "{a}" }
                    }
                    if let Some(m) = model.clone() {
                        Badge { tone: Tone::Info, "{m}" }
                    }
                }
            }
            if expanded() {
                div { class: "dr-role-body", style: "{body_wrap}",
                    if let Some(html) = body_html.clone() {
                        article { class: "dr-prose", dangerous_inner_html: "{html}" }
                    } else {
                        {children}
                    }
                }
            }
        }
    }
}

/// A card for a station's locked artifact: the durable thing the station produces
/// and locks. Renders a kind chip and the artifact name in mono.
#[component]
pub fn ArtifactCard(
    /// The artifact name (e.g. "frame.md", "code").
    name: String,
    /// Optional one-line description of what is locked.
    #[props(default)]
    description: Option<String>,
    /// Whether the artifact is currently locked (drives the lock glyph + hue).
    #[props(default = true)]
    locked: bool,
) -> Element {
    let hue = if locked { tokens::PHASE_CHECKPOINT.base } else { tokens::TEXT_FAINT };
    let card = format!(
        "display:flex;align-items:center;gap:10px;padding:10px 12px;\
         background:{surface};border:1px solid {border};border-left:3px solid {hue};\
         border-radius:8px;",
        surface = tokens::SURFACE_OVERLAY,
        border = tokens::BORDER,
    );
    let glyph = if locked { "🔒" } else { "🔓" };
    let name_style = format!(
        "font-family:{mono};font-size:13px;font-weight:600;color:{text};",
        mono = tokens::FONT_MONO,
        text = tokens::TEXT,
    );
    let desc_style = format!(
        "font-family:{sans};font-size:12px;color:{muted};",
        sans = tokens::FONT_SANS,
        muted = tokens::TEXT_MUTED,
    );
    rsx! {
        div { class: "dr-artifact-card", "data-locked": "{locked}", style: "{card}",
            span { style: "font-size:14px;", "{glyph}" }
            div { style: "display:flex;flex-direction:column;gap:2px;min-width:0;",
                span { style: "{name_style}", "{name}" }
                if let Some(d) = description {
                    span { style: "{desc_style}", "{d}" }
                }
            }
            span { style: "margin-left:auto;",
                Badge { tone: Tone::Neutral, "artifact" }
            }
        }
    }
}

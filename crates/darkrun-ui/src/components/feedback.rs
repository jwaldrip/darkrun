//! The feedback inbox: [`FeedbackRow`] + the [`FeedbackInbox`] list container.
//!
//! This is the one place to read **all** feedback on a station — every
//! annotation from every artifact, grouped by severity, with jump / resolve /
//! dismiss action chips. It is the same data the checkpoint counts; this is just
//! where the operator triages it before approving or sending the station back.
//!
//! Severity drives the visual weight and the grouping: `must` (blocker) is
//! danger-red, `should` (high) is warn-amber, `nit` is faint. The pure
//! `severity → tone` and the count-by-severity decisions live here so they are
//! unit-testable without a renderer.
//!
//! The crate stays renderer-agnostic and `darkrun-core`-free: the host projects
//! `list_annotations_for_work_item` rows into a `Vec` of [`FeedbackEntry`] at the
//! boundary.

use dioxus::prelude::*;

use crate::kinds::Tone;
use crate::tokens;

/// An annotation's severity — the wire's `ask.severity`.
///
/// Order is load-bearing for grouping: `Must` (blocker) first, then `Should`
/// (high), then `Nit`. Mirrors the annotation model; `nit` never blocks a
/// checkpoint, `should`/`must` flip the primary action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Blocker — must be resolved before a clean approve.
    Must,
    /// High — should be addressed; flips the checkpoint primary.
    Should,
    /// Nit — never blocks.
    Nit,
}

impl Severity {
    /// All severities in grouping order (blocker → high → nit).
    pub const ALL: [Severity; 3] = [Severity::Must, Severity::Should, Severity::Nit];

    /// Parse the wire's severity string (case-insensitive). Unknown values fall
    /// back to [`Severity::Nit`] so a new value never blocks unexpectedly.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "must" | "blocker" => Severity::Must,
            "should" | "high" => Severity::Should,
            _ => Severity::Nit,
        }
    }

    /// The lowercase wire slug.
    pub fn slug(self) -> &'static str {
        match self {
            Severity::Must => "must",
            Severity::Should => "should",
            Severity::Nit => "nit",
        }
    }

    /// The group header shown above this severity's rows.
    pub fn group_label(self) -> &'static str {
        match self {
            Severity::Must => "BLOCKER · MUST",
            Severity::Should => "HIGH · SHOULD",
            Severity::Nit => "NIT",
        }
    }

    /// The semantic [`Tone`] (and thus color) for the severity dot and counts.
    pub fn tone(self) -> Tone {
        match self {
            Severity::Must => Tone::Danger,
            Severity::Should => Tone::Warn,
            Severity::Nit => Tone::Neutral,
        }
    }

    /// The severity-dot color. `Nit` reads as faint rather than the muted-text
    /// neutral so the three dots stay visually distinct.
    pub fn dot_color(self) -> &'static str {
        match self {
            Severity::Must => tokens::STATUS_DANGER,
            Severity::Should => tokens::STATUS_WARN,
            Severity::Nit => tokens::TEXT_FAINT,
        }
    }
}

/// One feedback row's view-model, projected from an annotation at the host
/// boundary. Everything is plain data so the component derives `PartialEq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackEntry {
    /// Stable annotation id — the key the host resolves/dismisses against.
    pub id: String,
    /// Severity, driving the dot, grouping, and count.
    pub severity: Severity,
    /// The artifact/anchor locator (e.g. `payment.rs`).
    pub locator: String,
    /// The fine anchor within the locator (e.g. `:43-44`, `· region 2`).
    pub anchor: String,
    /// The comment text.
    pub comment: String,
    /// Who left it (e.g. `you`, `agent`).
    pub author: String,
    /// When true the row renders dimmed (resolved/dismissed/addressed).
    pub resolved: bool,
}

impl FeedbackEntry {
    /// Construct an open (un-resolved) feedback entry.
    pub fn new(
        id: impl Into<String>,
        severity: Severity,
        locator: impl Into<String>,
        anchor: impl Into<String>,
        comment: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            severity,
            locator: locator.into(),
            anchor: anchor.into(),
            comment: comment.into(),
            author: author.into(),
            resolved: false,
        }
    }
}

/// Count the open (un-resolved) entries by severity, returned in
/// [`Severity::ALL`] order as `(severity, count)`. This is exactly the data the
/// checkpoint bar shows (`2 blocker · 1 high · 3 nit`).
pub fn counts_by_severity(entries: &[FeedbackEntry]) -> [(Severity, usize); 3] {
    Severity::ALL.map(|sev| {
        let n = entries
            .iter()
            .filter(|e| !e.resolved && e.severity == sev)
            .count();
        (sev, n)
    })
}

/// Which action a feedback row's chip fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackAction {
    /// Jump to the annotation's anchor on its artifact.
    Jump,
    /// Resolve it (the underlying ask is satisfied).
    Resolve,
    /// Dismiss it without action.
    Dismiss,
}

/// One feedback row: a severity dot, the locator + anchor, the comment, the
/// author, and the small action chips. Fires `on_action` with the row's id and
/// the chosen [`FeedbackAction`].
///
/// A plain function (not `#[component]`) because the wire-projected
/// [`FeedbackEntry`] derives `PartialEq` but the `on_action` payload tuple does
/// not — matching the existing `review.rs` pattern of preferring plain functions
/// when a prop type lacks `PartialEq`.
pub fn feedback_row(
    entry: FeedbackEntry,
    on_action: Option<EventHandler<(String, FeedbackAction)>>,
) -> Element {
    let dim = entry.resolved;
    let opacity = if dim { "0.6" } else { "1" };
    let row_style = format!(
        "display:flex;align-items:center;gap:10px;padding:8px 10px;\
         border:1px solid {border};border-radius:7px;background:{surface};opacity:{opacity};",
        border = tokens::BORDER,
        surface = tokens::SURFACE_RAISED,
    );
    let dot_style = format!(
        "width:9px;height:9px;border-radius:50%;flex:none;background:{};",
        entry.severity.dot_color(),
    );
    let loc_style = format!(
        "font-family:{mono};font-size:12px;color:{text};white-space:nowrap;",
        mono = tokens::FONT_MONO,
        text = tokens::TEXT,
    );
    let anc_style = format!(
        "color:{faint};margin-left:3px;",
        faint = tokens::TEXT_FAINT,
    );
    let comment_style = format!(
        "flex:1;font-size:12.5px;color:{muted};min-width:0;",
        muted = tokens::TEXT_MUTED,
    );
    let who_style = format!(
        "font-family:{mono};font-size:10.5px;color:{faint};",
        mono = tokens::FONT_MONO,
        faint = tokens::TEXT_FAINT,
    );
    let acts_style = "display:flex;gap:6px;";

    // Resolved rows expose no live actions; open rows get jump/resolve/dismiss.
    let actions: Vec<(&str, FeedbackAction)> = if dim {
        Vec::new()
    } else {
        vec![
            ("jump", FeedbackAction::Jump),
            ("resolve", FeedbackAction::Resolve),
            ("dismiss", FeedbackAction::Dismiss),
        ]
    };

    rsx! {
        div {
            class: "dr-feedback-row",
            "data-severity": entry.severity.slug(),
            "data-resolved": "{dim}",
            style: "{row_style}",
            span { class: "dr-feedback-sev", style: "{dot_style}" }
            span { class: "dr-feedback-loc", style: "{loc_style}",
                "{entry.locator}"
                if !entry.anchor.is_empty() {
                    span { style: "{anc_style}", "{entry.anchor}" }
                }
            }
            span { class: "dr-feedback-comment", style: "{comment_style}", "{entry.comment}" }
            span { class: "dr-feedback-who", style: "{who_style}", "{entry.author}" }
            span { class: "dr-feedback-acts", style: "{acts_style}",
                for (label, action) in actions.into_iter() {
                    {
                        let id = entry.id.clone();
                        let handler = on_action;
                        let chip = format!(
                            "font-size:11px;color:{muted};border:1px solid {border};\
                             border-radius:5px;padding:2px 7px;cursor:pointer;background:transparent;",
                            muted = tokens::TEXT_MUTED,
                            border = tokens::BORDER_STRONG,
                        );
                        rsx! {
                            span {
                                class: "dr-feedback-action",
                                "data-action": match action {
                                    FeedbackAction::Jump => "jump",
                                    FeedbackAction::Resolve => "resolve",
                                    FeedbackAction::Dismiss => "dismiss",
                                },
                                style: "{chip}",
                                onclick: move |_| {
                                    if let Some(h) = &handler {
                                        h.call((id.clone(), action));
                                    }
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The feedback inbox: the consolidated, severity-grouped list of every
/// annotation on a station. Each non-empty severity gets a mono group header
/// (`BLOCKER · MUST`, `HIGH · SHOULD`, `NIT`) followed by its rows.
///
/// A plain function (not `#[component]`) so it can fan the non-`PartialEq`
/// `on_action` handler down into [`feedback_row`].
pub fn feedback_inbox(
    entries: Vec<FeedbackEntry>,
    on_action: Option<EventHandler<(String, FeedbackAction)>>,
) -> Element {
    let wrap = "display:flex;flex-direction:column;gap:6px;";
    rsx! {
        div { class: "dr-feedback-inbox", style: "{wrap}",
            for sev in Severity::ALL {
                {
                    let group: Vec<FeedbackEntry> = entries
                        .iter()
                        .filter(|e| e.severity == sev)
                        .cloned()
                        .collect();
                    let header_style = format!(
                        "font-family:{mono};font-size:10.5px;letter-spacing:0.08em;\
                         margin-top:6px;color:{faint};",
                        mono = tokens::FONT_MONO,
                        faint = tokens::TEXT_FAINT,
                    );
                    rsx! {
                        if !group.is_empty() {
                            div {
                                class: "dr-feedback-group",
                                "data-severity": sev.slug(),
                                style: "{header_style}",
                                "{sev.group_label()}"
                            }
                            for entry in group.into_iter() {
                                {feedback_row(entry, on_action)}
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_from_str_is_lenient() {
        assert_eq!(Severity::parse("must"), Severity::Must);
        assert_eq!(Severity::parse("BLOCKER"), Severity::Must);
        assert_eq!(Severity::parse("should"), Severity::Should);
        assert_eq!(Severity::parse("high"), Severity::Should);
        assert_eq!(Severity::parse("nit"), Severity::Nit);
        assert_eq!(Severity::parse("anything"), Severity::Nit);
    }

    #[test]
    fn severity_order_is_blocker_first() {
        assert_eq!(Severity::ALL[0], Severity::Must);
        assert_eq!(Severity::ALL[2], Severity::Nit);
    }

    #[test]
    fn counts_ignore_resolved_and_group_by_severity() {
        let mut resolved_nit =
            FeedbackEntry::new("4", Severity::Nit, "x", "", "c", "agent");
        resolved_nit.resolved = true;
        let entries = vec![
            FeedbackEntry::new("1", Severity::Must, "a", ":1", "c", "you"),
            FeedbackEntry::new("2", Severity::Must, "b", ":2", "c", "you"),
            FeedbackEntry::new("3", Severity::Should, "c", ":3", "c", "you"),
            resolved_nit,
        ];
        let counts = counts_by_severity(&entries);
        assert_eq!(counts[0], (Severity::Must, 2));
        assert_eq!(counts[1], (Severity::Should, 1));
        // The resolved nit is excluded from the open count.
        assert_eq!(counts[2], (Severity::Nit, 0));
    }

    #[test]
    fn dot_colors_are_distinct_per_severity() {
        assert_eq!(Severity::Must.dot_color(), tokens::STATUS_DANGER);
        assert_eq!(Severity::Should.dot_color(), tokens::STATUS_WARN);
        assert_eq!(Severity::Nit.dot_color(), tokens::TEXT_FAINT);
    }
}

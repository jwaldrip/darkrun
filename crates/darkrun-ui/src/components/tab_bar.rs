//! [`TabBar`] — the review surface tabs (Units / Outputs / Knowledge / Feedback /
//! Overview) with an active underline and optional count badges.
//!
//! Matches the predecessor's review-tabs model: a row under a hairline border,
//! the active tab carrying an accent underline, and each tab able to show a small
//! mono count pill. The Feedback tab's count reads in danger-red when it carries
//! open blockers, so the operator sees pending feedback at a glance.

use dioxus::prelude::*;

use crate::tokens;

/// One tab's view-model: its id, label, and optional count badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabItem {
    /// Stable id fired on select (e.g. `outputs`).
    pub id: String,
    /// Display label (e.g. `Outputs`).
    pub label: String,
    /// Optional count badge; `None` shows no pill.
    pub count: Option<u32>,
    /// When true the count pill reads in danger-red (e.g. open blockers).
    pub alert: bool,
}

impl TabItem {
    /// A tab with no count badge.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), count: None, alert: false }
    }

    /// A tab with a neutral count badge.
    pub fn with_count(id: impl Into<String>, label: impl Into<String>, count: u32) -> Self {
        Self { id: id.into(), label: label.into(), count: Some(count), alert: false }
    }

    /// A tab with a danger-red count badge (e.g. the Feedback tab with blockers).
    pub fn with_alert_count(id: impl Into<String>, label: impl Into<String>, count: u32) -> Self {
        Self { id: id.into(), label: label.into(), count: Some(count), alert: true }
    }
}

/// The review tab bar. The tab whose id equals `active` is underlined and bright;
/// the rest are muted. Fires `on_select` with the chosen tab id.
#[component]
pub fn TabBar(
    /// The tabs, in display order.
    tabs: Vec<TabItem>,
    /// The active tab's id.
    active: String,
    /// Fired with the selected tab's id.
    #[props(default)]
    on_select: Option<EventHandler<String>>,
) -> Element {
    let row = format!(
        "display:flex;gap:4px;border-bottom:1px solid {border};margin-bottom:2px;",
        border = tokens::var::BORDER,
    );
    rsx! {
        div {
            class: "dr-tab-bar",
            role: "tablist",
            style: "{row}",
            for tab in tabs.iter() {
                {
                    let on = tab.id == active;
                    let id = tab.id.clone();
                    let label = tab.label.clone();
                    let count = tab.count;
                    let alert = tab.alert;
                    let handler = on_select;
                    let (color, border_color, weight) = if on {
                        (tokens::var::TEXT, tokens::var::ACCENT, "600")
                    } else {
                        (tokens::var::TEXT_MUTED, "transparent", "400")
                    };
                    let tab_style = format!(
                        "padding:8px 14px;font-size:13px;color:{color};\
                         border-bottom:2px solid {border_color};font-weight:{weight};\
                         cursor:pointer;display:flex;align-items:center;gap:7px;\
                         background:transparent;border-top:none;border-left:none;border-right:none;\
                         font-family:{sans};",
                        sans = tokens::FONT_SANS,
                    );
                    rsx! {
                        button {
                            class: "dr-tab",
                            role: "tab",
                            "data-tab": "{id}",
                            "data-active": "{on}",
                            "aria-selected": "{on}",
                            style: "{tab_style}",
                            onclick: move |_| {
                                if let Some(h) = &handler {
                                    h.call(id.clone());
                                }
                            },
                            "{label}"
                            if let Some(n) = count {
                                {
                                    let pill = if alert {
                                        "background:var(--dr-alert-chip-bg);color:var(--dr-alert-chip-fg);"
                                            .to_string()
                                    } else {
                                        format!(
                                            "background:{surface};color:{faint};",
                                            surface = tokens::var::SURFACE_OVERLAY,
                                            faint = tokens::var::TEXT_FAINT,
                                        )
                                    };
                                    let pill = format!(
                                        "{pill}font-family:{mono};font-size:11px;\
                                         border-radius:999px;padding:1px 7px;line-height:1.4;",
                                        mono = tokens::FONT_MONO,
                                    );
                                    let shown = count_display(n);
                                    rsx! {
                                        span { class: "dr-tab-count", style: "{pill}", "{shown}" }
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

/// Past 99 the exact number stops informing and starts stretching the tab
/// bar — saturate the pill at `99+`.
fn count_display(n: u32) -> String {
    if n > 99 { "99+".to_string() } else { n.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_constructors_set_counts() {
        let plain = TabItem::new("overview", "Overview");
        assert_eq!(plain.count, None);
        assert!(!plain.alert);

        let counted = TabItem::with_count("outputs", "Outputs", 4);
        assert_eq!(counted.count, Some(4));
        assert!(!counted.alert);

        let alert = TabItem::with_alert_count("feedback", "Feedback", 6);
        assert_eq!(alert.count, Some(6));
        assert!(alert.alert);
    }

    #[test]
    fn ids_round_trip_through_labels() {
        let t = TabItem::with_count("knowledge", "Knowledge", 2);
        assert_eq!(t.id, "knowledge");
        assert_eq!(t.label, "Knowledge");
    }

    #[test]
    fn count_pill_saturates_at_99() {
        assert_eq!(count_display(0), "0");
        assert_eq!(count_display(99), "99");
        assert_eq!(count_display(100), "99+");
        assert_eq!(count_display(4729), "99+");
    }
}

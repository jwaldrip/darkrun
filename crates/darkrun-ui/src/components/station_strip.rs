//! [`StationStrip`] — the TOP-level assembly-line stepper.
//!
//! This is the prominent progress indicator in the review IA: an ordered row of
//! a factory's stations (e.g. `frame → specify → shape → build → prove → harden`),
//! each a numbered marker styled by status. Done stations fill green with a check,
//! the current station is a glowing cyan diamond, and pending stations are
//! outlined numbers. Connectors between markers are color-coded by the left
//! station's status, and a station carrying open feedback flags an amber dot.
//!
//! The existing [`StationPipeline`](crate::components::pipeline::StationPipeline)
//! (the ●◉○ phase glyph strip) becomes the SECONDARY subheader scoped to the
//! current station; this strip is the line itself.
//!
//! The crate stays renderer-agnostic and `darkrun-core`-free: the host maps the
//! engine's `station_states` (ordered, each with a status) into a `Vec` of
//! [`StationItem`] at the boundary. The pure `status → marker` decision lives in
//! [`StationStatus`] so it is unit-testable without a renderer.

use dioxus::prelude::*;

use crate::tokens;

/// The visual state of one station on the assembly line.
///
/// Mirrors the engine's per-station `status` (`done`/`current`/`pending`), kept
/// here as a small `Copy` enum so the UI crate stays self-contained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationStatus {
    /// Completed — filled green marker with a check glyph, solid connector.
    Done,
    /// The station the run currently sits on — glowing cyan diamond marker.
    Current,
    /// Not yet reached — outlined number marker, muted connector.
    Pending,
}

impl StationStatus {
    /// Parse the engine's status string (case-insensitive). Unknown values fall
    /// back to [`StationStatus::Pending`] so a new status never blanks a marker.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "done" | "complete" | "completed" => StationStatus::Done,
            "current" | "active" | "in_progress" => StationStatus::Current,
            _ => StationStatus::Pending,
        }
    }

    /// The `data-status` slug emitted on the marker for styling/testing hooks.
    pub fn slug(self) -> &'static str {
        match self {
            StationStatus::Done => "done",
            StationStatus::Current => "current",
            StationStatus::Pending => "pending",
        }
    }
}

/// One station's display state on the strip: its name, status, and whether it
/// carries open feedback (which flags the amber dot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationItem {
    /// The station's display name (e.g. `Build`).
    pub name: String,
    /// Done / current / pending.
    pub status: StationStatus,
    /// When true, an amber feedback dot rides the marker.
    pub has_feedback: bool,
}

impl StationItem {
    /// Construct a station item with no feedback flag.
    pub fn new(name: impl Into<String>, status: StationStatus) -> Self {
        Self { name: name.into(), status, has_feedback: false }
    }

    /// Construct a station item, flagging it as carrying open feedback.
    pub fn with_feedback(name: impl Into<String>, status: StationStatus) -> Self {
        Self { name: name.into(), status, has_feedback: true }
    }
}

/// Build a strip from `(name, status)` pairs — the common case where the host
/// already projected the engine's ordered `station_states`.
pub fn strip_from(stations: impl IntoIterator<Item = (String, StationStatus)>) -> Vec<StationItem> {
    stations
        .into_iter()
        .map(|(name, status)| StationItem::new(name, status))
        .collect()
}

/// The TOP-level station stepper — the assembly line.
///
/// Renders the ordered `stations` as numbered markers joined by color-coded
/// connectors. Pass `on_select` to make a station clickable (fires its zero-based
/// index); the host opens that station's detail. Markers are 1-indexed in their
/// labels but the callback carries the slice index.
#[component]
pub fn StationStrip(
    /// The factory's stations, in line order.
    stations: Vec<StationItem>,
    /// Optional click handler, fired with the station's zero-based index.
    #[props(default)]
    on_select: Option<EventHandler<usize>>,
) -> Element {
    let root = "display:flex;align-items:flex-start;justify-content:center;gap:0;padding:6px 0 2px;";
    let count = stations.len();
    rsx! {
        div {
            class: "dr-station-strip",
            style: "{root}",
            role: "list",
            "aria-label": "assembly line stations",
            for (i, st) in stations.iter().enumerate() {
                {
                    let status = st.status;
                    let name = st.name.clone();
                    let has_feedback = st.has_feedback;
                    let is_last = i + 1 == count;
                    let clickable = on_select.is_some();
                    let handler = on_select;

                    // The marker face: green check (done), cyan diamond (current),
                    // outlined number (pending). The diamond rotates 45deg and the
                    // inner number counter-rotates so it reads upright.
                    let (mark_core, label_color) = match status {
                        StationStatus::Done => (
                            format!(
                                "background:{ok};color:{on};border:2px solid {ok};",
                                ok = tokens::STATUS_OK,
                                on = "#04140a",
                            ),
                            tokens::TEXT,
                        ),
                        StationStatus::Current => (
                            format!(
                                "color:{accent};border:2px solid {accent};\
                                 box-shadow:0 0 0 4px #5fd7ff22;transform:rotate(45deg);",
                                accent = tokens::ACCENT,
                            ),
                            tokens::TEXT,
                        ),
                        StationStatus::Pending => (
                            format!(
                                "color:{faint};border:2px solid {border};",
                                faint = tokens::TEXT_FAINT,
                                border = tokens::BORDER_STRONG,
                            ),
                            tokens::TEXT_FAINT,
                        ),
                    };
                    let mark_style = format!(
                        "{mark_core}width:34px;height:34px;border-radius:50%;\
                         display:flex;align-items:center;justify-content:center;\
                         font-family:{mono};font-size:13px;font-weight:700;z-index:2;\
                         background-color:{base};box-sizing:border-box;",
                        mono = tokens::FONT_MONO,
                        // current keeps a base fill behind the glow; done overrides above.
                        base = if matches!(status, StationStatus::Done) {
                            tokens::STATUS_OK
                        } else {
                            tokens::SURFACE_BASE
                        },
                    );

                    // The connector to the next station, hued by this station's status.
                    let conn_bg = match status {
                        StationStatus::Done => tokens::STATUS_OK.to_string(),
                        StationStatus::Current => format!(
                            "linear-gradient(90deg,{accent},{border})",
                            accent = tokens::ACCENT,
                            border = tokens::BORDER_STRONG,
                        ),
                        StationStatus::Pending => tokens::BORDER_STRONG.to_string(),
                    };
                    let conn_style = format!(
                        "position:absolute;top:16px;left:50%;width:100%;height:3px;z-index:1;\
                         background:{conn_bg};"
                    );

                    let item_style = format!(
                        "display:flex;flex-direction:column;align-items:center;gap:7px;\
                         position:relative;min-width:118px;{cursor}",
                        cursor = if clickable { "cursor:pointer;" } else { "" },
                    );
                    let label_style = format!(
                        "font-family:{sans};font-size:12px;color:{label_color};",
                        sans = tokens::FONT_SANS,
                    );
                    let inner_style = if matches!(status, StationStatus::Current) {
                        "transform:rotate(-45deg);display:block;"
                    } else {
                        "display:block;"
                    };
                    let glyph = match status {
                        StationStatus::Done => "✓".to_string(),
                        _ => format!("{}", i + 1),
                    };

                    rsx! {
                        div {
                            class: "dr-station",
                            role: "listitem",
                            "data-status": status.slug(),
                            "data-station": "{name}",
                            style: "{item_style}",
                            onclick: move |_| {
                                if let Some(h) = &handler {
                                    h.call(i);
                                }
                            },
                            if !is_last {
                                span { class: "dr-station-conn", style: "{conn_style}" }
                            }
                            if has_feedback {
                                span {
                                    class: "dr-station-fbdot",
                                    title: "pending feedback",
                                    style: "position:absolute;top:-2px;right:34px;width:10px;height:10px;\
                                            border-radius:50%;background:{tokens::STATUS_WARN};\
                                            border:2px solid {tokens::SURFACE_BASE};z-index:3;",
                                }
                            }
                            div { class: "dr-station-mark", style: "{mark_style}",
                                span { style: "{inner_style}", "{glyph}" }
                            }
                            div { class: "dr-station-lbl", style: "{label_style}", "{name}" }
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
    fn status_from_str_is_lenient() {
        assert_eq!(StationStatus::parse("done"), StationStatus::Done);
        assert_eq!(StationStatus::parse("COMPLETED"), StationStatus::Done);
        assert_eq!(StationStatus::parse("current"), StationStatus::Current);
        assert_eq!(StationStatus::parse("active"), StationStatus::Current);
        assert_eq!(StationStatus::parse("pending"), StationStatus::Pending);
        // Unknown falls back to pending rather than blanking the marker.
        assert_eq!(StationStatus::parse("whatever"), StationStatus::Pending);
    }

    #[test]
    fn status_slugs_are_distinct() {
        assert_eq!(StationStatus::Done.slug(), "done");
        assert_eq!(StationStatus::Current.slug(), "current");
        assert_eq!(StationStatus::Pending.slug(), "pending");
    }

    #[test]
    fn strip_from_preserves_order_and_names() {
        let strip = strip_from([
            ("Frame".to_string(), StationStatus::Done),
            ("Build".to_string(), StationStatus::Current),
            ("Harden".to_string(), StationStatus::Pending),
        ]);
        assert_eq!(strip.len(), 3);
        assert_eq!(strip[0].name, "Frame");
        assert_eq!(strip[1].status, StationStatus::Current);
        assert_eq!(strip[2].name, "Harden");
        assert!(!strip[0].has_feedback);
    }

    #[test]
    fn with_feedback_flags_the_dot() {
        let st = StationItem::with_feedback("Build", StationStatus::Current);
        assert!(st.has_feedback);
        assert_eq!(st.status, StationStatus::Current);
    }
}

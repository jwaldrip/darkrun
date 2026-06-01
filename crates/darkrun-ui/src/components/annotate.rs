//! The annotate surface primitives: [`AnnotateToolbar`] (the tool palette) plus
//! the overlay elements ([`PinMarker`], [`BoxMarker`]) and the [`CommentPanel`]
//! side panel.
//!
//! These are deliberately **presentational**: they take the current tool / the
//! placed marks / the thread as plain data and emit callbacks. The placement math
//! (pixel → normalized `0..1`) lives in [`crate::selection`] and the wire/I-O
//! lives in the host — these components only paint.
//!
//! The palette adapts to the artifact class. Visual artifacts (image / live HTML)
//! get `cursor / pin / box / arrow / pen / highlight`; text artifacts get
//! `cursor / select / highlight / strike / suggest`. Mirrors the
//! `annotation-variants` mockup.

use dioxus::prelude::*;

use crate::selection::PinPoint;
use crate::tokens;

/// The tool palette varies by the artifact class being annotated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    /// An image / live-HTML render — spatial tools (pin/box/arrow/pen/highlight).
    Visual,
    /// A text artifact (code / markdown / spec) — span tools (select/highlight/strike/suggest).
    Text,
}

/// One annotation tool. The variant set is shared; [`SurfaceKind`] picks which
/// subset is offered. `Cursor` is the neutral select/idle tool on both surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotateTool {
    /// Neutral pointer — no mark on click.
    Cursor,
    /// Drop a numbered pin at a point (visual).
    Pin,
    /// Drag a rectangle region (visual).
    Box,
    /// Draw an arrow (visual).
    Arrow,
    /// Freehand pen (visual).
    Pen,
    /// Highlight a region/span (visual & text).
    Highlight,
    /// Select a text span (text).
    Select,
    /// Strike a span (text).
    Strike,
    /// Propose a replacement diff on a span (text).
    Suggest,
}

impl AnnotateTool {
    /// The lowercase slug emitted for styling/testing hooks.
    pub fn slug(self) -> &'static str {
        match self {
            AnnotateTool::Cursor => "cursor",
            AnnotateTool::Pin => "pin",
            AnnotateTool::Box => "box",
            AnnotateTool::Arrow => "arrow",
            AnnotateTool::Pen => "pen",
            AnnotateTool::Highlight => "highlight",
            AnnotateTool::Select => "select",
            AnnotateTool::Strike => "strike",
            AnnotateTool::Suggest => "suggest",
        }
    }

    /// The glyph shown on the palette button.
    pub fn glyph(self) -> &'static str {
        match self {
            AnnotateTool::Cursor => "↖",
            AnnotateTool::Pin => "📍",
            AnnotateTool::Box => "▭",
            AnnotateTool::Arrow => "↗",
            AnnotateTool::Pen => "✐",
            AnnotateTool::Highlight => "▤",
            AnnotateTool::Select => "⌶",
            AnnotateTool::Strike => "S̶",
            AnnotateTool::Suggest => "±",
        }
    }

    /// The tools offered for a given surface, in palette order. The leading
    /// `Cursor` is the neutral tool; the rest are surface-specific.
    pub fn for_surface(kind: SurfaceKind) -> &'static [AnnotateTool] {
        match kind {
            SurfaceKind::Visual => &[
                AnnotateTool::Cursor,
                AnnotateTool::Pin,
                AnnotateTool::Box,
                AnnotateTool::Arrow,
                AnnotateTool::Pen,
                AnnotateTool::Highlight,
            ],
            SurfaceKind::Text => &[
                AnnotateTool::Cursor,
                AnnotateTool::Select,
                AnnotateTool::Highlight,
                AnnotateTool::Strike,
                AnnotateTool::Suggest,
            ],
        }
    }
}

/// The tool palette. Renders the surface's tools with the `active` one filled in
/// the accent. Fires `on_pick` with the chosen tool.
#[component]
pub fn AnnotateToolbar(
    /// Which surface's toolset to offer.
    kind: SurfaceKind,
    /// The currently selected tool.
    active: AnnotateTool,
    /// Fired when a tool is picked.
    #[props(default)]
    on_pick: Option<EventHandler<AnnotateTool>>,
) -> Element {
    let palette = format!(
        "display:inline-flex;gap:4px;align-items:center;background:{surface};\
         border:1px solid {border};border-radius:9px;padding:4px;",
        surface = tokens::SURFACE_OVERLAY,
        border = tokens::BORDER,
    );
    rsx! {
        div {
            class: "dr-annotate-toolbar",
            role: "toolbar",
            "aria-label": "annotation tools",
            style: "{palette}",
            for tool in AnnotateTool::for_surface(kind).iter().copied() {
                {
                    let on = tool == active;
                    let handler = on_pick;
                    let core = if on {
                        format!(
                            "background:{accent};color:{on_accent};",
                            accent = tokens::ACCENT,
                            on_accent = tokens::ON_ACCENT,
                        )
                    } else {
                        format!("background:transparent;color:{};", tokens::TEXT_MUTED)
                    };
                    let btn = format!(
                        "{core}width:30px;height:28px;border-radius:6px;border:none;\
                         display:flex;align-items:center;justify-content:center;\
                         font-size:14px;cursor:pointer;"
                    );
                    rsx! {
                        button {
                            class: "dr-annotate-tool",
                            "data-tool": tool.slug(),
                            "data-active": "{on}",
                            title: tool.slug(),
                            style: "{btn}",
                            onclick: move |_| {
                                if let Some(h) = &handler {
                                    h.call(tool);
                                }
                            },
                            "{tool.glyph()}"
                        }
                    }
                }
            }
        }
    }
}

/// A numbered pin overlay, positioned by a normalized [`PinPoint`].
///
/// Absolutely positioned within the artifact stage (the parent must be
/// `position:relative`); the transform centers the pin on its point.
#[component]
pub fn PinMarker(
    /// Where the pin sits, in `0..1` coords.
    point: PinPoint,
    /// The 1-based number shown in the pin.
    number: usize,
) -> Element {
    let style = format!(
        "position:absolute;left:{left};top:{top};width:24px;height:24px;border-radius:50%;\
         background:{accent};color:{on};font-size:12px;font-weight:700;\
         display:flex;align-items:center;justify-content:center;\
         box-shadow:0 2px 8px #000a;transform:translate(-50%,-50%);",
        left = point.left_pct(),
        top = point.top_pct(),
        accent = tokens::ACCENT,
        on = tokens::ON_ACCENT,
    );
    rsx! {
        div { class: "dr-annotate-pin", "data-n": "{number}", style: "{style}", "{number}" }
    }
}

/// A box-region overlay, positioned and sized by normalized `0..1` coords.
///
/// `(x, y)` is the top-left corner and `(w, h)` the size, each a fraction of the
/// stage. The numbered tag rides the top-left corner.
#[component]
pub fn BoxMarker(
    /// Left edge, `0..1`.
    x: f64,
    /// Top edge, `0..1`.
    y: f64,
    /// Width, `0..1`.
    w: f64,
    /// Height, `0..1`.
    h: f64,
    /// The 1-based number shown on the tag.
    number: usize,
) -> Element {
    let style = format!(
        "position:absolute;left:{left:.4}%;top:{top:.4}%;width:{width:.4}%;height:{height:.4}%;\
         border:2px solid {accent};border-radius:4px;background:#5fd7ff14;box-sizing:border-box;",
        left = x * 100.0,
        top = y * 100.0,
        width = w * 100.0,
        height = h * 100.0,
        accent = tokens::ACCENT,
    );
    let tag = format!(
        "position:absolute;top:-11px;left:-2px;background:{accent};color:{on};\
         font-family:{mono};font-size:10px;font-weight:700;border-radius:4px;padding:0 5px;",
        accent = tokens::ACCENT,
        on = tokens::ON_ACCENT,
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div { class: "dr-annotate-box", "data-n": "{number}", style: "{style}",
            span { class: "dr-annotate-box-tag", style: "{tag}", "{number}" }
        }
    }
}

/// One comment in the thread side panel: a number badge + the comment text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadComment {
    /// The 1-based number tying the comment to its overlay mark.
    pub number: usize,
    /// The comment body.
    pub text: String,
}

impl ThreadComment {
    /// Construct a thread comment.
    pub fn new(number: usize, text: impl Into<String>) -> Self {
        Self { number, text: text.into() }
    }
}

/// The comment side panel: the thread of comments tied to the placed marks, plus
/// a draft input. The input is *controlled* by a panel-local draft signal, so the
/// typed text is captured and handed up on submit.
///
/// Fires `on_submit` with the current (trimmed) draft text. The host threads that
/// string into the annotation it records. The draft clears after a submit.
#[component]
pub fn CommentPanel(
    /// The thread, in mark order.
    comments: Vec<ThreadComment>,
    /// Placeholder for the draft input.
    #[props(default = "comment on the last pin…".to_string())]
    placeholder: String,
    /// Fired when the submit button is pressed, carrying the typed draft text.
    #[props(default)]
    on_submit: Option<EventHandler<String>>,
) -> Element {
    let mut draft = use_signal(String::new);
    let wrap = "display:flex;flex-direction:column;gap:8px;min-width:0;";
    let h2 = format!(
        "margin:0;font-size:13px;font-weight:700;color:{text};\
         text-transform:uppercase;letter-spacing:0.04em;",
        text = tokens::TEXT,
    );
    let input = format!(
        "width:100%;box-sizing:border-box;padding:9px 12px;border-radius:6px;\
         border:1px solid {border};background:{base};color:{text};\
         font-family:{sans};font-size:13px;",
        border = tokens::BORDER,
        base = tokens::SURFACE_BASE,
        text = tokens::TEXT,
        sans = tokens::FONT_SANS,
    );
    let submit = format!(
        "background:{accent};color:{on};border:1px solid {accent};\
         font-family:{sans};font-size:13px;font-weight:600;\
         padding:7px 14px;border-radius:6px;cursor:pointer;",
        accent = tokens::ACCENT,
        on = tokens::ON_ACCENT,
        sans = tokens::FONT_SANS,
    );
    rsx! {
        div { class: "dr-comment-panel", style: "{wrap}",
            div { class: "dr-comment-h2", style: "{h2}", "Comments" }
            for c in comments.iter() {
                {
                    let cmt = format!(
                        "font-size:12.5px;color:{muted};padding:7px 0;\
                         border-bottom:1px solid {border};display:flex;gap:8px;",
                        muted = tokens::TEXT_MUTED,
                        border = tokens::BORDER,
                    );
                    let num = format!("color:{accent};font-weight:700;", accent = tokens::ACCENT);
                    rsx! {
                        div { class: "dr-comment", style: "{cmt}",
                            span { style: "{num}", "{c.number}" }
                            span { "{c.text}" }
                        }
                    }
                }
            }
            input {
                class: "dr-comment-input",
                style: "{input}",
                placeholder: "{placeholder}",
                value: "{draft}",
                oninput: move |evt| draft.set(evt.value()),
            }
            div { style: "display:flex;gap:8px;margin-top:4px;",
                button {
                    class: "dr-comment-submit",
                    style: "{submit}",
                    onclick: move |_| {
                        if let Some(h) = &on_submit {
                            h.call(draft.read().trim().to_string());
                        }
                        draft.set(String::new());
                    },
                    "Submit feedback"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_surface_offers_spatial_tools() {
        let tools = AnnotateTool::for_surface(SurfaceKind::Visual);
        assert_eq!(tools[0], AnnotateTool::Cursor);
        assert!(tools.contains(&AnnotateTool::Pin));
        assert!(tools.contains(&AnnotateTool::Box));
        // Text-only tools are absent from the visual palette.
        assert!(!tools.contains(&AnnotateTool::Suggest));
        assert!(!tools.contains(&AnnotateTool::Select));
    }

    #[test]
    fn text_surface_offers_span_tools() {
        let tools = AnnotateTool::for_surface(SurfaceKind::Text);
        assert_eq!(tools[0], AnnotateTool::Cursor);
        assert!(tools.contains(&AnnotateTool::Select));
        assert!(tools.contains(&AnnotateTool::Suggest));
        // Highlight is shared across both surfaces.
        assert!(tools.contains(&AnnotateTool::Highlight));
        // Spatial-only tools are absent from the text palette.
        assert!(!tools.contains(&AnnotateTool::Pin));
        assert!(!tools.contains(&AnnotateTool::Arrow));
    }

    #[test]
    fn tool_slugs_and_glyphs_are_set() {
        for tool in [
            AnnotateTool::Cursor,
            AnnotateTool::Pin,
            AnnotateTool::Box,
            AnnotateTool::Suggest,
        ] {
            assert!(!tool.slug().is_empty());
            assert!(!tool.glyph().is_empty());
        }
    }

    #[test]
    fn thread_comment_holds_number_and_text() {
        let c = ThreadComment::new(2, "total is misaligned");
        assert_eq!(c.number, 2);
        assert_eq!(c.text, "total is misaligned");
    }
}

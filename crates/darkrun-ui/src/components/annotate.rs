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

    /// The `ImageShape` slug a *visual* tool draws + submits, matching the
    /// wire's `pin`/`rect`/`arrow`/`path`/`highlight` vocabulary. `None` for
    /// non-drawing tools (`Cursor`) and text-surface tools, which don't carry a
    /// pixel shape. The host uses this to route a placed [`crate::selection::VisualMark`]
    /// onto the correct anchor shape.
    pub fn image_shape(self) -> Option<&'static str> {
        match self {
            AnnotateTool::Pin => Some("pin"),
            AnnotateTool::Box => Some("rect"),
            AnnotateTool::Arrow => Some("arrow"),
            AnnotateTool::Pen => Some("path"),
            AnnotateTool::Highlight => Some("highlight"),
            AnnotateTool::Cursor
            | AnnotateTool::Select
            | AnnotateTool::Strike
            | AnnotateTool::Suggest => None,
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
        surface = tokens::var::SURFACE_OVERLAY,
        border = tokens::var::BORDER,
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
                    // Every tool carries a VISIBLE label (no hover-guessing), and
                    // the active tool is fully accent-filled — glyph, name, and
                    // pill all in the accent so the current mode is unmistakable.
                    // Kept compact (26px pills, 11px mono labels) so the palette
                    // never outweighs the artifact it annotates.
                    let core = if on {
                        format!(
                            "background:{accent};color:{on_accent};font-weight:600;",
                            accent = tokens::var::ACCENT,
                            on_accent = tokens::var::ON_ACCENT,
                        )
                    } else {
                        format!("background:transparent;color:{};", tokens::var::TEXT_MUTED)
                    };
                    let btn = format!(
                        "{core}height:26px;border-radius:6px;border:none;\
                         display:inline-flex;align-items:center;gap:6px;\
                         padding:0 9px;cursor:pointer;white-space:nowrap;"
                    );
                    let label_style = format!(
                        "font-family:{mono};font-size:11px;letter-spacing:0.02em;line-height:1;",
                        mono = tokens::FONT_MONO,
                    );
                    rsx! {
                        button {
                            class: "dr-annotate-tool",
                            "data-tool": tool.slug(),
                            "data-active": "{on}",
                            "aria-pressed": "{on}",
                            title: tool.slug(),
                            style: "{btn}",
                            onclick: move |_| {
                                if let Some(h) = &handler {
                                    h.call(tool);
                                }
                            },
                            span { style: "font-size:13px;line-height:1;", "{tool.glyph()}" }
                            span { style: "{label_style}", "{tool.slug()}" }
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
        accent = tokens::var::ACCENT,
        on = tokens::var::ON_ACCENT,
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
        accent = tokens::var::ACCENT,
    );
    let tag = format!(
        "position:absolute;top:-11px;left:-2px;background:{accent};color:{on};\
         font-family:{mono};font-size:10px;font-weight:700;border-radius:4px;padding:0 5px;",
        accent = tokens::var::ACCENT,
        on = tokens::var::ON_ACCENT,
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div { class: "dr-annotate-box", "data-n": "{number}", style: "{style}",
            span { class: "dr-annotate-box-tag", style: "{tag}", "{number}" }
        }
    }
}

/// An arrow overlay, drawn from a normalized tail to a normalized head.
///
/// Renders as a full-stage SVG (`viewBox 0 0 100 100`, so coords are percent)
/// laid absolutely over the artifact stage; the parent must be
/// `position:relative`. A `<line>` carries the shaft and a `<marker>` paints the
/// arrowhead at the head end. `pointer-events:none` so it never eats stage clicks.
#[component]
pub fn ArrowMarker(
    /// Tail point, `0..1`.
    from: PinPoint,
    /// Head point, `0..1`.
    to: PinPoint,
    /// The 1-based number shown at the tail.
    number: usize,
) -> Element {
    // A per-marker arrowhead id so multiple arrows don't collide in one DOM.
    let head_id = format!("dr-arrowhead-{number}");
    let (x1, y1) = (from.x * 100.0, from.y * 100.0);
    let (x2, y2) = (to.x * 100.0, to.y * 100.0);
    let wrap = "position:absolute;inset:0;width:100%;height:100%;\
                pointer-events:none;overflow:visible;";
    rsx! {
        svg {
            class: "dr-annotate-arrow",
            "data-n": "{number}",
            style: "{wrap}",
            view_box: "0 0 100 100",
            preserve_aspect_ratio: "none",
            defs {
                marker {
                    id: "{head_id}",
                    "viewBox": "0 0 10 10",
                    "refX": "8",
                    "refY": "5",
                    "markerWidth": "6",
                    "markerHeight": "6",
                    "orient": "auto-start-reverse",
                    path { d: "M0,0 L10,5 L0,10 z", fill: tokens::var::ACCENT }
                }
            }
            line {
                x1: "{x1}",
                y1: "{y1}",
                x2: "{x2}",
                y2: "{y2}",
                stroke: tokens::var::ACCENT,
                "stroke-width": "0.7",
                "vector-effect": "non-scaling-stroke",
                "marker-end": "url(#{head_id})",
            }
        }
    }
}

/// A freehand path overlay: a polyline over a sequence of normalized points.
///
/// Like [`ArrowMarker`], a full-stage percent-space SVG. A single `<polyline>`
/// traces the captured stroke; fewer than two points draws nothing.
#[component]
pub fn PathMarker(
    /// The stroke points, in draw order, each `0..1`.
    points: Vec<PinPoint>,
    /// The 1-based number for the stroke.
    number: usize,
) -> Element {
    if points.len() < 2 {
        return rsx! {};
    }
    let pts = points
        .iter()
        .map(|p| format!("{:.4},{:.4}", p.x * 100.0, p.y * 100.0))
        .collect::<Vec<_>>()
        .join(" ");
    let wrap = "position:absolute;inset:0;width:100%;height:100%;\
                pointer-events:none;overflow:visible;";
    rsx! {
        svg {
            class: "dr-annotate-path",
            "data-n": "{number}",
            style: "{wrap}",
            view_box: "0 0 100 100",
            preserve_aspect_ratio: "none",
            polyline {
                points: "{pts}",
                fill: "none",
                stroke: tokens::var::ACCENT,
                "stroke-width": "0.7",
                "stroke-linejoin": "round",
                "stroke-linecap": "round",
                "vector-effect": "non-scaling-stroke",
            }
        }
    }
}

/// A translucent highlight overlay over a normalized `0..1` rectangle.
///
/// Like [`BoxMarker`] but a filled sweep (no hard border) — the `highlight`
/// tool's softer mark. The numbered tag rides the top-left corner.
#[component]
pub fn HighlightMarker(
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
         background:{accent}33;border:1px solid {accent}66;border-radius:3px;box-sizing:border-box;\
         pointer-events:none;",
        left = x * 100.0,
        top = y * 100.0,
        width = w * 100.0,
        height = h * 100.0,
        // Hex (not the var) so the `33`/`66` alpha suffix forms a valid 8-digit color.
        accent = tokens::ACCENT,
    );
    let tag = format!(
        "position:absolute;top:-11px;left:-2px;background:{accent};color:{on};\
         font-family:{mono};font-size:10px;font-weight:700;border-radius:4px;padding:0 5px;",
        accent = tokens::var::ACCENT,
        on = tokens::var::ON_ACCENT,
        mono = tokens::FONT_MONO,
    );
    rsx! {
        div { class: "dr-annotate-highlight", "data-n": "{number}", style: "{style}",
            span { class: "dr-annotate-highlight-tag", style: "{tag}", "{number}" }
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

/// What the comment panel hands up on submit: the typed comment plus, when the
/// `suggest` tool authored a replacement, the suggestion body. The host folds the
/// suggestion onto the annotation's `suggestion` slot (a diff on the span).
///
/// A plain `Vec`-free struct that derives `PartialEq` so it round-trips through an
/// `EventHandler` without forcing the panel off `#[component]`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommentDraft {
    /// The free-form comment text (the *why*).
    pub comment: String,
    /// The proposed replacement text, when the `suggest` tool was used. Empty
    /// when the panel is in plain-comment mode or the field was left blank.
    pub suggestion: String,
}

impl CommentDraft {
    /// Whether this draft carries a non-empty suggestion replacement.
    pub fn has_suggestion(&self) -> bool {
        !self.suggestion.trim().is_empty()
    }
}

/// The comment side panel: the thread of comments tied to the placed marks, plus
/// a draft input. The input is *controlled* by a panel-local draft signal, so the
/// typed text is captured and handed up on submit.
///
/// When `suggest` is set (the text surface's `suggest` tool is active), a second
/// textarea is revealed for a replacement diff. Both fields ride up together as a
/// [`CommentDraft`]; the host stores the replacement on the annotation's
/// `suggestion` slot. Fires `on_submit` with the trimmed draft. The draft clears
/// after a submit.
#[component]
pub fn CommentPanel(
    /// The thread, in mark order.
    comments: Vec<ThreadComment>,
    /// Placeholder for the draft input.
    #[props(default = "comment on the last pin…".to_string())]
    placeholder: String,
    /// Whether the `suggest` tool is active — reveals the replacement-diff input.
    #[props(default)]
    suggest: bool,
    /// Fired when the submit button is pressed, carrying the typed draft.
    #[props(default)]
    on_submit: Option<EventHandler<CommentDraft>>,
) -> Element {
    let mut draft = use_signal(String::new);
    let mut replacement = use_signal(String::new);
    let wrap = "display:flex;flex-direction:column;gap:8px;min-width:0;";
    let h2 = format!(
        "margin:0;font-size:13px;font-weight:700;color:{text};\
         text-transform:uppercase;letter-spacing:0.04em;",
        text = tokens::var::TEXT,
    );
    let input = format!(
        "width:100%;box-sizing:border-box;padding:9px 12px;border-radius:6px;\
         border:1px solid {border};background:{base};color:{text};\
         font-family:{sans};font-size:13px;",
        border = tokens::var::BORDER,
        base = tokens::var::SURFACE_BASE,
        text = tokens::var::TEXT,
        sans = tokens::FONT_SANS,
    );
    let submit = format!(
        "background:{accent};color:{on};border:1px solid {accent};\
         font-family:{sans};font-size:13px;font-weight:600;\
         padding:7px 14px;border-radius:6px;cursor:pointer;",
        accent = tokens::var::ACCENT,
        on = tokens::var::ON_ACCENT,
        sans = tokens::FONT_SANS,
    );
    // The replacement-diff box is monospaced — it carries source the agent applies.
    let replace_input = format!(
        "width:100%;box-sizing:border-box;padding:9px 12px;border-radius:6px;\
         border:1px solid {border};background:{base};color:{text};\
         font-family:{mono};font-size:12.5px;min-height:64px;resize:vertical;",
        border = tokens::var::BORDER,
        base = tokens::var::SURFACE_BASE,
        text = tokens::var::TEXT,
        mono = tokens::FONT_MONO,
    );
    let replace_label = format!(
        "font-family:{mono};font-size:10.5px;text-transform:uppercase;\
         letter-spacing:0.04em;color:{faint};",
        mono = tokens::FONT_MONO,
        faint = tokens::var::TEXT_FAINT,
    );
    rsx! {
        div { class: "dr-comment-panel", style: "{wrap}",
            div { class: "dr-comment-h2", style: "{h2}", "Comments" }
            for c in comments.iter() {
                {
                    let cmt = format!(
                        "font-size:12.5px;color:{muted};padding:7px 0;\
                         border-bottom:1px solid {border};display:flex;gap:8px;",
                        muted = tokens::var::TEXT_MUTED,
                        border = tokens::var::BORDER,
                    );
                    let num = format!("color:{accent};font-weight:700;", accent = tokens::var::ACCENT);
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
            // The replacement-diff authoring box — revealed only for the `suggest`
            // tool, captured into the draft's suggestion slot on submit.
            if suggest {
                div { class: "dr-suggest-label", style: "{replace_label}", "Suggested replacement" }
                textarea {
                    class: "dr-suggest-input",
                    style: "{replace_input}",
                    placeholder: "type the replacement text for the selected span…",
                    value: "{replacement}",
                    oninput: move |evt| replacement.set(evt.value()),
                }
            }
            div { style: "display:flex;gap:8px;margin-top:4px;",
                button {
                    class: "dr-comment-submit",
                    style: "{submit}",
                    onclick: move |_| {
                        if let Some(h) = &on_submit {
                            h.call(CommentDraft {
                                comment: draft.read().trim().to_string(),
                                suggestion: if suggest {
                                    replacement.read().trim().to_string()
                                } else {
                                    String::new()
                                },
                            });
                        }
                        draft.set(String::new());
                        replacement.set(String::new());
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
    fn every_visual_tool_maps_to_a_drawable_shape() {
        use crate::selection::{NormBox, PinPoint, VisualMark};
        // Each non-cursor visual tool resolves to an `ImageShape` slug, and the
        // slug matches the `VisualMark` the tool produces — the host's routing
        // contract.
        for tool in AnnotateTool::for_surface(SurfaceKind::Visual) {
            match tool {
                AnnotateTool::Cursor => assert!(tool.image_shape().is_none()),
                _ => assert!(tool.image_shape().is_some(), "{:?} draws a shape", tool),
            }
        }
        assert_eq!(AnnotateTool::Pin.image_shape(), Some("pin"));
        assert_eq!(AnnotateTool::Box.image_shape(), Some("rect"));
        assert_eq!(AnnotateTool::Arrow.image_shape(), Some("arrow"));
        assert_eq!(AnnotateTool::Pen.image_shape(), Some("path"));
        assert_eq!(AnnotateTool::Highlight.image_shape(), Some("highlight"));

        // The tool's shape slug equals the mark it builds — pin/rect/arrow/path.
        assert_eq!(
            AnnotateTool::Pin.image_shape(),
            Some(VisualMark::Pin { point: PinPoint::new(0.5, 0.5, "") }.shape_slug()),
        );
        assert_eq!(
            AnnotateTool::Arrow.image_shape(),
            Some(
                VisualMark::Arrow {
                    from: PinPoint::new(0.1, 0.1, ""),
                    to: PinPoint::new(0.2, 0.2, ""),
                }
                .shape_slug()
            ),
        );
        assert_eq!(
            AnnotateTool::Box.image_shape(),
            Some(VisualMark::Rect { rect: NormBox::new(0.1, 0.1, 0.2, 0.2, "") }.shape_slug()),
        );
    }

    #[test]
    fn text_tools_carry_no_pixel_shape() {
        for tool in [AnnotateTool::Select, AnnotateTool::Strike, AnnotateTool::Suggest] {
            assert!(tool.image_shape().is_none(), "{:?} is a span tool", tool);
        }
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

    #[test]
    fn comment_draft_detects_a_suggestion() {
        let plain = CommentDraft {
            comment: "tighten this".into(),
            suggestion: String::new(),
        };
        assert!(!plain.has_suggestion());

        let with_fix = CommentDraft {
            comment: "use the declined path".into(),
            suggestion: "fn charge(card: Card) -> Result<(), Error>".into(),
        };
        assert!(with_fix.has_suggestion());

        // Whitespace-only replacements don't count as a suggestion.
        let blank = CommentDraft {
            comment: "nit".into(),
            suggestion: "   \n".into(),
        };
        assert!(!blank.has_suggestion());
    }
}

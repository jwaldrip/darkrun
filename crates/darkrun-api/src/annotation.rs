//! The annotation envelope — the human↔agent feedback channel wire contract.
//!
//! Every annotation, regardless of artifact type, is the same record: a
//! universal [`Annotation`] envelope whose only typed slot is the
//! [`Anchor`] — a durable, version-pinned locator into the artifact. The
//! desktop review surface marks pixels or a text span; the engine stores this
//! structured shape pinned to a `version_sha`; the agent receives a source
//! location plus the quote/crop plus the [`Ask`] intent so it can
//! deterministically re-find the work. Pixels in, `file:line` out.
//!
//! The [`AnnotationStatus`] carries the lifecycle (records are retained, never
//! hard-deleted) and `ask.severity` drives the checkpoint: an open `should` or
//! `must` flips Request-changes to primary; a `nit` never blocks.
//!
//! These are `serde` + `schemars` types only — the storage, re-anchor, and
//! severity-aggregation logic live in `darkrun-core`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::AuthorType;

/// The kind of work item an annotation hangs on.
///
/// A `station` annotation with no artifact anchor is the **global station
/// note** — a single station-level comment that ships with all per-artifact
/// annotations on Request-changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    /// A single unit of work.
    Unit,
    /// A produced output artifact.
    Output,
    /// The station as a whole (the global station note carries no artifact).
    Station,
}

/// The work item an annotation targets — a unit, an output, or the station.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkItem {
    /// Whether this targets a unit, an output, or the station itself.
    pub kind: WorkItemKind,
    /// The work item's id (unit slug / output id). Empty for a bare station
    /// note that targets the station as a whole.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The station this work item belongs to.
    pub station: String,
}

/// The artifact type an annotation's anchor is typed against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    /// Markdown, code, spec, plain text.
    Text,
    /// PNG/JPEG, screenshots, renders.
    Image,
    /// HTML / live visual — the strong case (pixels *and* source).
    Html,
    /// A PDF document.
    Pdf,
    /// An SVG (svg is its own source).
    Svg,
    /// A video.
    Video,
}

/// The artifact an annotation was made against, pinned to its exact version.
///
/// `version_sha` is load-bearing: an annotation is pinned to the exact bytes
/// of the artifact it was made against. When the agent produces a new version,
/// every annotation re-anchors against it and is flagged if it may have
/// shifted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactInfo {
    /// The artifact id (e.g. `dashboard.png`).
    pub id: String,
    /// Repo-root-relative path to the artifact.
    pub path: String,
    /// The artifact type the anchor is typed against.
    #[serde(rename = "type")]
    pub artifact_type: ArtifactType,
    /// The SHA-256 of the artifact bytes when the annotation was made.
    pub version_sha: String,
}

/// The human expression — how the mark was made (the *how*).
///
/// Expression is for the human; the [`Ask`] + [`Anchor`] are for the agent. A
/// freehand scrawl and a precise box both resolve to the same actionable
/// record — the richness is in how freely the human can point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Expression {
    /// The tool used: `pin`, `box`, `arrow`, `path`, `highlight`,
    /// `select`, `strike`, `suggestion`, … — kept free-form so the toolbar
    /// can adapt per artifact without a wire-contract change.
    pub tool: String,
    /// Optional stroke/fill color the human picked (e.g. `#5fd7ff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// The kind of structured ask attached to an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// Asks for an edit.
    Change,
    /// Asks a question.
    Question,
    /// A minor nit.
    Nit,
    /// Praise — no action required.
    Praise,
}

/// How strongly an ask steers the checkpoint.
///
/// `must` and `should` flip Request-changes to primary; `nit` never blocks.
/// These map onto the existing `FeedbackSeverity` taxonomy at checkpoint
/// decision time: `must` → `Blocker`, `should` → `High`, `nit` → `Low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskSeverity {
    /// A blocker — you can't cleanly approve over it.
    Must,
    /// High — fix before delivery.
    Should,
    /// A nit — never blocks.
    Nit,
}

impl AskSeverity {
    /// Whether an open ask at this severity blocks the checkpoint's clean
    /// Approve. `must` and `should` block; `nit` never does.
    pub fn blocks_checkpoint(self) -> bool {
        matches!(self, AskSeverity::Must | AskSeverity::Should)
    }
}

/// The structured intent attached to an annotation (the *why / what-to-do*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Ask {
    /// What kind of ask this is.
    pub kind: AskKind,
    /// How strongly it steers the checkpoint.
    pub severity: AskSeverity,
}

/// Lifecycle status of an annotation.
///
/// Records are retained, never hard-deleted — the status carries the
/// lifecycle. `shifted` is reached by the re-anchor pass when an annotation's
/// target is gone in a new artifact version; it surfaces for re-placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationStatus {
    /// Open and unaddressed.
    Open,
    /// A fix has landed; shown as a resolved thread.
    Addressed,
    /// Dismissed without a code change.
    Dismissed,
    /// The re-anchor pass could not re-find the target; needs re-placement.
    Shifted,
}

/// A line/column range inside a text artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextRange {
    /// 1-based start line.
    pub start_line: u32,
    /// 0-based start column.
    pub start_col: u32,
    /// 1-based end line (inclusive).
    pub end_line: u32,
    /// 0-based end column (exclusive).
    pub end_col: u32,
}

/// A normalized point in `0..1` space, resolution-independent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormPoint {
    /// X coordinate, `0..1` of render width.
    pub x: f64,
    /// Y coordinate, `0..1` of render height.
    pub y: f64,
}

/// A normalized rectangle in `0..1` space, resolution-independent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NormRect {
    /// Left edge, `0..1`.
    pub x: f64,
    /// Top edge, `0..1`.
    pub y: f64,
    /// Width, `0..1`.
    pub w: f64,
    /// Height, `0..1`.
    pub h: f64,
}

/// The visual shape a pixel-space annotation was drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageShape {
    /// A single point.
    Pin,
    /// A rectangle region.
    Rect,
    /// An arrow from one point to another.
    Arrow,
    /// A freehand path.
    Path,
    /// A highlight sweep.
    Highlight,
}

/// A pixel-space mark: the normalized geometry plus the render dimensions it
/// was drawn over. Shared by image anchors and the HTML pixel anchor.
///
/// Exactly one geometry field is populated, matching `shape`: `pin` → `point`,
/// `rect`/`highlight` → `rect`, `arrow` → `arrow_from`/`arrow_to`, `path` →
/// `path`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PixelMark {
    /// The shape that was drawn.
    pub shape: ImageShape,
    /// The point, for a `pin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<NormPoint>,
    /// The rectangle, for `rect`/`highlight`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<NormRect>,
    /// The arrow tail, for an `arrow`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow_from: Option<NormPoint>,
    /// The arrow head, for an `arrow`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow_to: Option<NormPoint>,
    /// The freehand points, for a `path`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<NormPoint>,
    /// Render width in pixels the mark was drawn over.
    pub render_w: u32,
    /// Render height in pixels the mark was drawn over.
    pub render_h: u32,
}

/// The DOM resolution of an HTML annotation — the source half of the strong
/// case. The factory injects `data-darkrun-src="file:line"` on rendered
/// elements; we read the element under the mark and resolve it here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DomAnchor {
    /// A CSS selector that locates the element.
    pub selector: String,
    /// The resolved source location, `file:line`, from the build-time source
    /// map. `None` when the element carried no `data-darkrun-src`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    /// The element's `outerHTML`, a human-readable snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_html: Option<String>,
}

/// The typed, durable locator — the only part of the envelope that varies by
/// artifact type. Tagged on `anchor_type` so the union round-trips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "anchor_type", rename_all = "snake_case")]
pub enum Anchor {
    /// A text span, stored three ways (range + quote + prefix/suffix) so it
    /// survives the file changing under it.
    Text {
        /// The line/column span the annotation was made against.
        range: TextRange,
        /// The exact text the span covered — the re-anchor search key.
        quote: String,
        /// ~40 chars of context before the span, to disambiguate duplicates.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        prefix: String,
        /// ~40 chars of context after the span, to disambiguate duplicates.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        suffix: String,
    },
    /// A normalized pixel-space mark over an image.
    Image {
        /// The mark geometry + render dimensions.
        #[serde(flatten)]
        mark: PixelMark,
    },
    /// Both pixels and source: the pixel mark plus the resolved DOM anchor.
    Html {
        /// The pixel-space mark.
        pixel: PixelMark,
        /// The DOM/source resolution.
        dom: DomAnchor,
    },
    /// A normalized rectangle on a specific PDF page.
    Pdf {
        /// 1-based page number.
        page: u32,
        /// The region on that page.
        rect: NormRect,
    },
    /// An SVG element by id or xpath, with its bounding box. SVG is its own
    /// source, so this resolves directly.
    Svg {
        /// The element id, when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        element_id: Option<String>,
        /// An xpath into the document, when no stable id exists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        xpath: Option<String>,
        /// The element's bounding box, normalized.
        bbox: NormRect,
    },
    /// A time span in a video, with an optional region on the frame.
    Video {
        /// Start time, seconds.
        t_start: f64,
        /// End time, seconds.
        t_end: f64,
        /// An optional region on the frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rect: Option<NormRect>,
    },
}

impl Anchor {
    /// The artifact type this anchor variant is valid for — used to reject a
    /// mismatched anchor/artifact pair at submit time.
    pub fn artifact_type(&self) -> ArtifactType {
        match self {
            Anchor::Text { .. } => ArtifactType::Text,
            Anchor::Image { .. } => ArtifactType::Image,
            Anchor::Html { .. } => ArtifactType::Html,
            Anchor::Pdf { .. } => ArtifactType::Pdf,
            Anchor::Svg { .. } => ArtifactType::Svg,
            Anchor::Video { .. } => ArtifactType::Video,
        }
    }
}

/// An optional inline replacement, stored as a unified-diff suggestion on a
/// text span — the agent gets a concrete edit to apply or argue with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Suggestion {
    /// The unified-diff (or before/after) replacement text.
    pub diff: String,
}

/// The universal annotation envelope.
///
/// Every annotation is this record; only [`anchor`](Annotation::anchor) is
/// typed per artifact. The global station note is an [`Annotation`] with
/// `work_item.kind == Station` and no `artifact`/`anchor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Annotation {
    /// Stable `anno_…` id.
    pub id: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Who made the mark — a human or an agent (agents can annotate too).
    pub author: AuthorType,
    /// The unit / output / station this hangs on.
    pub work_item: WorkItem,
    /// The version-pinned artifact, or `None` for a bare station note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactInfo>,
    /// The typed, durable locator, or `None` for a bare station note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// How the human marked it. `None` for a bare station note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
    /// The free-form comment (the *why*).
    pub comment: String,
    /// The structured ask — drives the checkpoint.
    pub ask: Ask,
    /// An optional inline-replacement suggestion (text artifacts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Suggestion>,
    /// Lifecycle status.
    pub status: AnnotationStatus,
}

impl Annotation {
    /// Whether this is the **global station note**: a station-scoped record
    /// with no artifact anchor, which ships with the per-artifact annotations
    /// on Request-changes.
    pub fn is_station_note(&self) -> bool {
        self.work_item.kind == WorkItemKind::Station && self.artifact.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_annotation() -> Annotation {
        Annotation {
            id: "anno_01J".into(),
            created_at: "2026-05-31T18:40:00Z".into(),
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "payment".into(),
                station: "build".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "payment.rs".into(),
                path: ".darkrun/checkout/build/outputs/payment.rs".into(),
                artifact_type: ArtifactType::Text,
                version_sha: "9f3c".into(),
            }),
            anchor: Some(Anchor::Text {
                range: TextRange {
                    start_line: 42,
                    start_col: 3,
                    end_line: 44,
                    end_col: 18,
                },
                quote: "fn charge(card: Card)".into(),
                prefix: "// the unhappy path\n".into(),
                suffix: "\n        .map_err(Error::Gateway)?;".into(),
            }),
            expression: Some(Expression {
                tool: "select".into(),
                color: None,
            }),
            comment: "handle the declined-card path here".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Should,
            },
            suggestion: None,
            status: AnnotationStatus::Open,
        }
    }

    #[test]
    fn envelope_roundtrips_with_text_anchor() {
        let a = text_annotation();
        let json = serde_json::to_value(&a).expect("serialize");
        assert_eq!(json["author"], "human");
        assert_eq!(json["work_item"]["kind"], "output");
        assert_eq!(json["artifact"]["type"], "text");
        assert_eq!(json["anchor"]["anchor_type"], "text");
        assert_eq!(json["anchor"]["range"]["start_line"], 42);
        assert_eq!(json["ask"]["severity"], "should");
        assert_eq!(json["status"], "open");

        let back: Annotation = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, a);
    }

    #[test]
    fn image_anchor_roundtrips_with_normalized_rect() {
        let mark = PixelMark {
            shape: ImageShape::Rect,
            point: None,
            rect: Some(NormRect {
                x: 0.55,
                y: 0.46,
                w: 0.18,
                h: 0.07,
            }),
            arrow_from: None,
            arrow_to: None,
            path: vec![],
            render_w: 1440,
            render_h: 900,
        };
        let anchor = Anchor::Image { mark };
        let json = serde_json::to_value(&anchor).expect("serialize");
        assert_eq!(json["anchor_type"], "image");
        // The mark is flattened, so shape/rect/render_w sit at the top level.
        assert_eq!(json["shape"], "rect");
        assert_eq!(json["rect"]["w"], 0.18);
        assert_eq!(json["render_w"], 1440);

        let back: Anchor = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.artifact_type(), ArtifactType::Image);
    }

    #[test]
    fn html_anchor_carries_pixel_and_dom() {
        let anchor = Anchor::Html {
            pixel: PixelMark {
                shape: ImageShape::Rect,
                point: None,
                rect: Some(NormRect {
                    x: 0.1,
                    y: 0.1,
                    w: 0.2,
                    h: 0.2,
                }),
                arrow_from: None,
                arrow_to: None,
                path: vec![],
                render_w: 1440,
                render_h: 900,
            },
            dom: DomAnchor {
                selector: "main > section.summary > .total-row".into(),
                src: Some("web/cart/Summary.tsx:118".into()),
                outer_html: Some("<div class=\"total-row\"></div>".into()),
            },
        };
        let json = serde_json::to_value(&anchor).expect("serialize");
        assert_eq!(json["anchor_type"], "html");
        assert_eq!(json["dom"]["src"], "web/cart/Summary.tsx:118");
        let back: Anchor = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.artifact_type(), ArtifactType::Html);
    }

    #[test]
    fn every_anchor_reports_its_artifact_type() {
        let cases: Vec<(Anchor, ArtifactType)> = vec![
            (
                Anchor::Pdf {
                    page: 3,
                    rect: NormRect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.5,
                        h: 0.5,
                    },
                },
                ArtifactType::Pdf,
            ),
            (
                Anchor::Svg {
                    element_id: Some("node-7".into()),
                    xpath: None,
                    bbox: NormRect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.1,
                        h: 0.1,
                    },
                },
                ArtifactType::Svg,
            ),
            (
                Anchor::Video {
                    t_start: 1.0,
                    t_end: 2.5,
                    rect: None,
                },
                ArtifactType::Video,
            ),
        ];
        for (anchor, ty) in cases {
            assert_eq!(anchor.artifact_type(), ty);
            let json = serde_json::to_value(&anchor).expect("serialize");
            let back: Anchor = serde_json::from_value(json).expect("deserialize");
            assert_eq!(back.artifact_type(), ty);
        }
    }

    #[test]
    fn station_note_has_no_artifact() {
        let note = Annotation {
            id: "anno_station".into(),
            created_at: "2026-05-31T18:40:00Z".into(),
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Station,
                id: String::new(),
                station: "build".into(),
            },
            artifact: None,
            anchor: None,
            expression: None,
            comment: "here's the gist".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Should,
            },
            suggestion: None,
            status: AnnotationStatus::Open,
        };
        assert!(note.is_station_note());
        let json = serde_json::to_value(&note).expect("serialize");
        // The empty work-item id and absent artifact are omitted on the wire.
        assert!(json["work_item"].get("id").is_none());
        assert!(json.get("artifact").is_none());
        assert!(json.get("anchor").is_none());
        let back: Annotation = serde_json::from_value(json).expect("deserialize");
        assert!(back.is_station_note());
    }

    #[test]
    fn severity_blocks_checkpoint_correctly() {
        assert!(AskSeverity::Must.blocks_checkpoint());
        assert!(AskSeverity::Should.blocks_checkpoint());
        assert!(!AskSeverity::Nit.blocks_checkpoint());
    }

    #[test]
    fn unknown_anchor_type_is_rejected() {
        let json = serde_json::json!({ "anchor_type": "hologram" });
        let parsed: std::result::Result<Anchor, _> = serde_json::from_value(json);
        assert!(parsed.is_err(), "unknown anchor_type must not parse");
    }
}

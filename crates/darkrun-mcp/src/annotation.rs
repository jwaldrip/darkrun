//! The annotation MCP tool surface — submit, list (the feedback-inbox data),
//! and the agent re-reference payload.
//!
//! The [`Annotation`] envelope (in `darkrun-api`) and its on-disk storage,
//! text re-anchor, and severity aggregation (in `darkrun-core`) are the
//! substrate; this module is the *verbs* the desktop review surface and the
//! agent loop call:
//!
//! - **submit** — record one annotation (a per-artifact mark OR the global
//!   station note) into `.darkrun/<run>/annotations/<id>.json`. Validates that
//!   the anchor's typed shape matches the artifact type, mints the `anno_…` id
//!   and timestamp, and — for an image/html mark with a rectangle — crops the
//!   marked region out of the version-pinned artifact to a PNG sitting beside
//!   the JSON, so the agent later re-reads exactly what the human boxed.
//! - **list** — the feedback-inbox read: every annotation on a work item (or a
//!   station), plus the open-severity tally and the checkpoint button steering
//!   (`should`/`must` flip Request-changes to primary; `nit` never blocks).
//! - **agent re-reference** — for the current work item, resolve each OPEN
//!   annotation to an actionable bundle: text → `file:line` + quote + comment
//!   (+ suggestion diff); image → a cropped region PNG + coords + comment;
//!   html → `dom.src` (`file:line`) + `outer_html` + comment. Where source
//!   resolution needs infra not present here (the web renderer injects
//!   `data-darkrun-src`), we pass the stored anchor through and flag it.
//!
//! Pixels in, `file:line` out — this is where the envelope earns its keep.

use std::path::{Path, PathBuf};

use chrono::Utc;
use darkrun_api::annotation::{
    Anchor, Annotation, AnnotationStatus, ArtifactInfo, Ask, AskKind, AskSeverity, Expression,
    NormRect, PixelMark, Suggestion, WorkItem, WorkItemKind,
};
use darkrun_api::common::AuthorType;
use darkrun_core::annotation::{checkpoint_button_state, count_open_by_severity, CheckpointButton};
use darkrun_core::StateStore;

use crate::error::{McpError, Result};

/// One annotation to submit. The anchor/artifact are absent for the global
/// station note; present (and shape-validated) for a per-artifact mark.
#[derive(Clone)]
pub struct SubmitArgs {
    /// Who is marking — a human reviewer or an agent.
    pub author: AuthorType,
    /// The unit / output / station this hangs on.
    pub work_item: WorkItem,
    /// The version-pinned artifact, or `None` for a bare station note.
    pub artifact: Option<ArtifactInfo>,
    /// The typed locator, or `None` for a bare station note.
    pub anchor: Option<Anchor>,
    /// How the human marked it (tool + color), or `None`.
    pub expression: Option<Expression>,
    /// The free-form comment (the *why*).
    pub comment: String,
    /// The structured ask — drives the checkpoint.
    pub ask: Ask,
    /// An optional inline-replacement suggestion (text artifacts).
    pub suggestion: Option<Suggestion>,
}

/// The record a `submit` produced, plus whether a region crop was written.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubmitResult {
    /// The persisted annotation.
    pub annotation: Annotation,
    /// The repo-relative path of the region crop written beside the JSON, when
    /// the mark was an image/html rect (else `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_path: Option<String>,
}

/// Mint the `anno_<ts>_<n>` id for a new annotation. The timestamp prefix keeps
/// the flat `annotations/` dir creation-sortable; the suffix disambiguates
/// same-millisecond submits.
fn mint_id(existing: &[Annotation], created_at: &str) -> String {
    // Compact the RFC3339 stamp to digits so the id stays filename-clean.
    let ts: String = created_at.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut n = 0u32;
    loop {
        let id = format!("anno_{ts}_{n:03}");
        if !existing.iter().any(|a| a.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Validate that an annotation's anchor matches its artifact, and that the
/// envelope is internally consistent (a station note carries neither; a
/// per-artifact mark carries both, with the anchor's typed shape matching the
/// artifact type).
fn validate(args: &SubmitArgs) -> Result<()> {
    if args.comment.trim().is_empty() {
        return Err(McpError::InvalidInput(
            "annotation comment must not be empty".into(),
        ));
    }
    match args.work_item.kind {
        // The global station note: no artifact, no anchor — it ships with the
        // per-artifact annotations on Request-changes.
        WorkItemKind::Station if args.artifact.is_none() => {
            if args.anchor.is_some() {
                return Err(McpError::InvalidInput(
                    "a station note carries no anchor".into(),
                ));
            }
            Ok(())
        }
        _ => {
            let Some(artifact) = args.artifact.as_ref() else {
                return Err(McpError::InvalidInput(
                    "a per-artifact annotation requires an artifact".into(),
                ));
            };
            let Some(anchor) = args.anchor.as_ref() else {
                return Err(McpError::InvalidInput(
                    "a per-artifact annotation requires an anchor".into(),
                ));
            };
            if anchor.artifact_type() != artifact.artifact_type {
                return Err(McpError::InvalidInput(format!(
                    "anchor type {:?} does not match artifact type {:?}",
                    anchor.artifact_type(),
                    artifact.artifact_type
                )));
            }
            Ok(())
        }
    }
}

/// Record one annotation, minting its id/timestamp and (for an image/html rect
/// mark) cropping the marked region out of the version-pinned artifact.
///
/// `repo_root` is the repository root the artifact `path` is relative to — the
/// crop reads the real file from there and writes the PNG beside the JSON.
pub fn submit(store: &StateStore, repo_root: &Path, run: &str, args: SubmitArgs) -> Result<SubmitResult> {
    validate(&args)?;

    let created_at = Utc::now().to_rfc3339();
    let existing = store.list_annotations(run)?;
    let id = mint_id(&existing, &created_at);

    let annotation = Annotation {
        id: id.clone(),
        created_at,
        author: args.author,
        work_item: args.work_item,
        artifact: args.artifact,
        anchor: args.anchor,
        expression: args.expression,
        comment: args.comment,
        ask: args.ask,
        suggestion: args.suggestion,
        status: AnnotationStatus::Open,
    };

    store.write_annotation(run, &annotation)?;

    // Crop the marked region for image/html rect marks, so the agent later
    // re-reads exactly the pixels the human boxed. A best-effort step: a missing
    // or undecodable artifact leaves the crop absent rather than failing the
    // submit (the coords still round-trip).
    let crop_path = match crop_for(&annotation) {
        Some(rect) => write_crop(store, repo_root, run, &annotation, rect).ok().flatten(),
        None => None,
    };

    Ok(SubmitResult {
        annotation,
        crop_path,
    })
}

/// The normalized rect to crop for an annotation, when its anchor is an image
/// or html mark drawn as a rectangle. Pins, arrows, and paths have no single
/// region to crop, so they yield `None`.
fn crop_for(annotation: &Annotation) -> Option<NormRect> {
    let mark = match annotation.anchor.as_ref()? {
        Anchor::Image { mark } => mark,
        Anchor::Html { pixel, .. } => pixel,
        _ => return None,
    };
    rect_of(mark)
}

/// The rectangle of a pixel mark — the `rect` for a rect/highlight, or the
/// bounding box of an arrow's two endpoints. `None` for pin/path (no region).
fn rect_of(mark: &PixelMark) -> Option<NormRect> {
    if let Some(rect) = mark.rect {
        return Some(rect);
    }
    if let (Some(from), Some(to)) = (mark.arrow_from, mark.arrow_to) {
        let x = from.x.min(to.x);
        let y = from.y.min(to.y);
        return Some(NormRect {
            x,
            y,
            w: (from.x - to.x).abs(),
            h: (from.y - to.y).abs(),
        });
    }
    None
}

/// The crop PNG path for an annotation: `annotations/<id>__crop.png`.
pub fn crop_file_path(store: &StateStore, run: &str, id: &str) -> PathBuf {
    store.annotations_dir(run).join(format!("{id}__crop.png"))
}

/// Crop the marked normalized rect out of the version-pinned artifact and write
/// it beside the annotation JSON. Returns the repo-relative crop path on
/// success, `None` when the artifact is absent/undecodable (best-effort), and an
/// error only on a write fault.
fn write_crop(
    store: &StateStore,
    repo_root: &Path,
    run: &str,
    annotation: &Annotation,
    rect: NormRect,
) -> Result<Option<String>> {
    let Some(artifact) = annotation.artifact.as_ref() else {
        return Ok(None);
    };
    let src = repo_root.join(&artifact.path);
    let Ok(img) = image::open(&src) else {
        // Best-effort: a missing/undecodable artifact just skips the crop.
        return Ok(None);
    };
    let cropped = crop_image_region(&img, rect);
    let out = crop_file_path(store, run, &annotation.id);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| McpError::InvalidInput(format!("crop dir: {e}")))?;
    }
    cropped
        .save_with_format(&out, image::ImageFormat::Png)
        .map_err(|e| McpError::InvalidInput(format!("crop write: {e}")))?;
    // Return the path relative to the repo root for the wire payload.
    let rel = out
        .strip_prefix(repo_root)
        .map(|p| p.to_path_buf())
        .unwrap_or(out);
    Ok(Some(rel.to_string_lossy().to_string()))
}

/// Crop the normalized rect (`0..1` over the image's own dimensions) out of a
/// decoded image. The rect is clamped to the image bounds, and a zero-area rect
/// degrades to a single pixel so the crop is always valid.
pub fn crop_image_region(img: &image::DynamicImage, rect: NormRect) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let clamp01 = |v: f64| v.clamp(0.0, 1.0);
    let px = (clamp01(rect.x) * w as f64).round() as u32;
    let py = (clamp01(rect.y) * h as f64).round() as u32;
    let pw = (clamp01(rect.w) * w as f64).round() as u32;
    let ph = (clamp01(rect.h) * h as f64).round() as u32;
    // Keep the crop inside the image and at least 1×1.
    let cw = pw.max(1).min(w.saturating_sub(px).max(1));
    let ch = ph.max(1).min(h.saturating_sub(py).max(1));
    img.crop_imm(px.min(w.saturating_sub(1)), py.min(h.saturating_sub(1)), cw, ch)
}

// ─── List (feedback-inbox data) ──────────────────────────────────────────────

/// The feedback-inbox read for one work item (or a station): the annotations
/// plus the open-severity tally and the checkpoint button steering they imply.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnnotationListing {
    /// The annotations on the work item (every status; filter client-side).
    pub annotations: Vec<Annotation>,
    /// Open `must` asks (blockers).
    pub must: usize,
    /// Open `should` asks (high).
    pub should: usize,
    /// Open `nit` asks (never block).
    pub nit: usize,
    /// The legible bar label, e.g. `2 blocker · 1 high · 3 nit`. Absent when
    /// there are no open asks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_label: Option<String>,
    /// Whether any open `should`/`must` blocks a clean Approve.
    pub blocks_clean_approve: bool,
    /// Which checkpoint button is primary: `approve` or `request_changes`.
    pub checkpoint_button_primary: &'static str,
}

/// The button token the gate/decision flow reads.
fn button_token(state: CheckpointButton) -> &'static str {
    match state {
        CheckpointButton::ApproveIsPrimary => "approve",
        CheckpointButton::RequestChangesIsPrimary => "request_changes",
    }
}

/// List the annotations for a work item (or, for a `station`-kind query, the
/// station-scoped records including the global note), decorated with the open
/// severity counts and the checkpoint button steering.
///
/// `open_only` filters to [`AnnotationStatus::Open`] records; the severity
/// counts always reflect open asks regardless, so the checkpoint steering is
/// stable whether or not the caller asked for the full history.
pub fn list(
    store: &StateStore,
    run: &str,
    work_item: &WorkItem,
    open_only: bool,
) -> Result<AnnotationListing> {
    let all = store.list_annotations_for_work_item(run, work_item)?;
    let counts = count_open_by_severity(&all);
    let button = checkpoint_button_state(&all);
    let annotations = if open_only {
        all.iter()
            .filter(|a| a.status == AnnotationStatus::Open)
            .cloned()
            .collect()
    } else {
        all
    };
    Ok(AnnotationListing {
        annotations,
        must: counts.must,
        should: counts.should,
        nit: counts.nit,
        bar_label: counts.bar_label(),
        blocks_clean_approve: counts.blocks_clean_approve(),
        checkpoint_button_primary: button_token(button),
    })
}

// ─── Agent re-reference payload ──────────────────────────────────────────────

/// How an open annotation's source was resolved for the agent, per artifact
/// type. Each variant carries exactly what the agent needs to act.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedSource {
    /// Text: the source is the file itself — `path` + the line range + the
    /// quote. The agent reads `path:line` directly.
    Text {
        /// Repo-relative file path.
        path: String,
        /// 1-based start line.
        start_line: u32,
        /// 1-based end line (inclusive).
        end_line: u32,
        /// The exact quoted span the mark covered.
        quote: String,
    },
    /// Image: a cropped region PNG (when one was written) + the normalized
    /// coords, so the agent re-reads the boxed pixels against the provenance
    /// artifact.
    Image {
        /// Repo-relative artifact path the crop came from.
        artifact_path: String,
        /// Repo-relative crop PNG path, when a rect/arrow region was cropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        crop_path: Option<String>,
        /// The normalized rect cropped, when one applied.
        #[serde(skip_serializing_if = "Option::is_none")]
        rect: Option<NormRect>,
    },
    /// HTML: the strong case — `dom.src` (`file:line`) + `outer_html`. When the
    /// renderer injected no `data-darkrun-src`, `src` is absent and `resolved`
    /// is false: the agent falls back to the selector + crop/provenance.
    Html {
        /// The resolved `file:line`, when the renderer injected a source map.
        #[serde(skip_serializing_if = "Option::is_none")]
        src: Option<String>,
        /// The element selector (always present).
        selector: String,
        /// The element's `outerHTML` snapshot, when captured.
        #[serde(skip_serializing_if = "Option::is_none")]
        outer_html: Option<String>,
        /// The crop PNG path, when the pixel rect was cropped.
        #[serde(skip_serializing_if = "Option::is_none")]
        crop_path: Option<String>,
        /// Whether `src` resolved to a real `file:line` (vs. degraded to
        /// selector + pixels because no source map was present).
        resolved: bool,
    },
    /// Any other artifact type (pdf/svg/video) or the bare station note: the
    /// raw anchor is passed through for the surface to interpret, with a note.
    Passthrough {
        /// A short note on why this wasn't resolved to a concrete source here.
        note: String,
    },
}

/// One open annotation resolved into an actionable agent bundle.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentReReferenceItem {
    /// The annotation id (stable handle to resolve/dismiss later).
    pub id: String,
    /// The resolved source location, per artifact type.
    pub source: ResolvedSource,
    /// The human comment (the *why*).
    pub comment: String,
    /// What kind of ask this is (`change`/`question`/`nit`/`praise`).
    pub ask_kind: AskKind,
    /// How strongly it steers the checkpoint (`must`/`should`/`nit`).
    pub severity: AskSeverity,
    /// The optional inline-replacement diff to apply or argue with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// The full agent re-reference payload for a work item: every OPEN annotation
/// resolved to an actionable bundle, plus the open-severity steering. This is
/// what the agent receives when it revisits the work item.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentReReferencePayload {
    /// The resolved per-annotation bundles.
    pub items: Vec<AgentReReferenceItem>,
    /// Open `must` asks (blockers).
    pub must: usize,
    /// Open `should` asks (high).
    pub should: usize,
    /// Open `nit` asks (never block).
    pub nit: usize,
    /// The legible bar label, when there are open asks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bar_label: Option<String>,
}

/// Resolve one annotation's source for the agent, given the repo root (to test
/// whether a crop exists) and the run.
fn resolve_source(
    store: &StateStore,
    run: &str,
    repo_root: &Path,
    annotation: &Annotation,
) -> ResolvedSource {
    // The bare station note has no anchor — pass it through with its comment.
    let Some(anchor) = annotation.anchor.as_ref() else {
        return ResolvedSource::Passthrough {
            note: "station note — no per-artifact anchor".into(),
        };
    };
    let artifact_path = annotation
        .artifact
        .as_ref()
        .map(|a| a.path.clone())
        .unwrap_or_default();

    // The crop, if one was written at submit time, sits beside the JSON.
    let crop = {
        let p = crop_file_path(store, run, &annotation.id);
        if p.exists() {
            p.strip_prefix(repo_root)
                .map(|r| r.to_path_buf())
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
                .into()
        } else {
            None
        }
    };

    match anchor {
        Anchor::Text { range, quote, .. } => ResolvedSource::Text {
            path: artifact_path,
            start_line: range.start_line,
            end_line: range.end_line,
            quote: quote.clone(),
        },
        Anchor::Image { mark } => ResolvedSource::Image {
            artifact_path,
            crop_path: crop,
            rect: rect_of(mark),
        },
        Anchor::Html { dom, .. } => {
            let resolved = dom.src.is_some();
            ResolvedSource::Html {
                src: dom.src.clone(),
                selector: dom.selector.clone(),
                outer_html: dom.outer_html.clone(),
                crop_path: crop,
                resolved,
            }
        }
        Anchor::Pdf { page, .. } => ResolvedSource::Passthrough {
            note: format!("pdf page {page}; resolve via the document viewer"),
        },
        Anchor::Svg { .. } => ResolvedSource::Passthrough {
            note: "svg is its own source — resolve the element by id/xpath".into(),
        },
        Anchor::Video { t_start, t_end, .. } => ResolvedSource::Passthrough {
            note: format!("video span {t_start}s–{t_end}s; resolve via the frame at t_start"),
        },
    }
}

/// Build the agent re-reference payload for a work item: resolve every OPEN
/// annotation to an actionable bundle and tally the open-severity steering.
pub fn agent_re_reference(
    store: &StateStore,
    repo_root: &Path,
    run: &str,
    work_item: &WorkItem,
) -> Result<AgentReReferencePayload> {
    let all = store.list_annotations_for_work_item(run, work_item)?;
    let counts = count_open_by_severity(&all);
    let items = all
        .iter()
        .filter(|a| a.status == AnnotationStatus::Open)
        .map(|a| AgentReReferenceItem {
            id: a.id.clone(),
            source: resolve_source(store, run, repo_root, a),
            comment: a.comment.clone(),
            ask_kind: a.ask.kind,
            severity: a.ask.severity,
            suggestion: a.suggestion.as_ref().map(|s| s.diff.clone()),
        })
        .collect();
    Ok(AgentReReferencePayload {
        items,
        must: counts.must,
        should: counts.should,
        nit: counts.nit,
        bar_label: counts.bar_label(),
    })
}

// ─── Checkpoint / request-changes integration ────────────────────────────────

/// The open annotations a station ships on Request-changes, ordered so the
/// global station note leads. This is the bundle that kicks the rework loop:
/// the station's per-artifact annotations *plus* the global station note (an
/// annotation with `work_item.kind = station` and no artifact).
///
/// Per the model, a `station`-kind query already returns only the station-level
/// records, so we union it with each work item's per-artifact annotations is
/// unnecessary here — the caller scopes to the station and we surface the note
/// first, then everything else.
pub fn station_rework_bundle(annotations: &[Annotation]) -> Vec<Annotation> {
    let mut ordered: Vec<Annotation> = Vec::with_capacity(annotations.len());
    // The global station note(s) lead — they frame the per-artifact asks.
    for a in annotations {
        if a.is_station_note() && a.status == AnnotationStatus::Open {
            ordered.push(a.clone());
        }
    }
    for a in annotations {
        if !a.is_station_note() && a.status == AnnotationStatus::Open {
            ordered.push(a.clone());
        }
    }
    ordered
}

/// Render the open annotations on a station into a single feedback-document
/// body — the request-changes hand-off. The global station note leads, then
/// each per-artifact ask with its severity, location, and comment. This is the
/// text the rework loop reads back as a `feedback/*.md` body.
pub fn render_rework_feedback(station: &str, annotations: &[Annotation]) -> String {
    let bundle = station_rework_bundle(annotations);
    let mut out = String::new();
    out.push_str(&format!("# Request changes — {station}\n\n"));
    for a in &bundle {
        let sev = match a.ask.severity {
            AskSeverity::Must => "blocker",
            AskSeverity::Should => "high",
            AskSeverity::Nit => "nit",
        };
        if a.is_station_note() {
            out.push_str(&format!("- **station note** ({sev}): {}\n", a.comment.trim()));
        } else {
            let loc = a
                .anchor
                .as_ref()
                .map(describe_anchor)
                .unwrap_or_else(|| "—".into());
            let artifact = a
                .artifact
                .as_ref()
                .map(|art| art.path.as_str())
                .unwrap_or("—");
            out.push_str(&format!(
                "- **{sev}** `{artifact}` ({loc}): {}\n",
                a.comment.trim()
            ));
        }
    }
    out
}

/// A short, human-legible location label for an anchor, for the rework body.
fn describe_anchor(anchor: &Anchor) -> String {
    match anchor {
        Anchor::Text { range, .. } => {
            if range.start_line == range.end_line {
                format!("line {}", range.start_line)
            } else {
                format!("lines {}–{}", range.start_line, range.end_line)
            }
        }
        Anchor::Image { .. } => "image region".into(),
        Anchor::Html { dom, .. } => dom
            .src
            .clone()
            .unwrap_or_else(|| dom.selector.clone()),
        Anchor::Pdf { page, .. } => format!("pdf page {page}"),
        Anchor::Svg { element_id, xpath, .. } => element_id
            .clone()
            .or_else(|| xpath.clone())
            .unwrap_or_else(|| "svg element".into()),
        Anchor::Video { t_start, .. } => format!("video @ {t_start}s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_api::annotation::{ArtifactType, ImageShape, NormPoint, TextRange};
    use image::{Rgba, RgbaImage};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore, PathBuf) {
        let dir = tempdir().expect("tmp");
        let root = dir.path().to_path_buf();
        let store = StateStore::new(&root);
        (dir, store, root)
    }

    fn text_args(severity: AskSeverity) -> SubmitArgs {
        SubmitArgs {
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "payment".into(),
                station: "build".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "payment.rs".into(),
                path: "src/payment.rs".into(),
                artifact_type: ArtifactType::Text,
                version_sha: "9f3c".into(),
            }),
            anchor: Some(Anchor::Text {
                range: TextRange {
                    start_line: 42,
                    start_col: 0,
                    end_line: 44,
                    end_col: 1,
                },
                quote: "fn charge()".into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            expression: Some(Expression {
                tool: "select".into(),
                color: None,
            }),
            comment: "handle the declined-card path".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity,
            },
            suggestion: None,
        }
    }

    fn write_png(path: &Path, w: u32, h: u32) {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = Rgba([10, 20, 30, 255]);
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        img.save_with_format(path, image::ImageFormat::Png).unwrap();
    }

    fn image_args(root: &Path) -> SubmitArgs {
        // A real PNG on disk so the crop can decode it.
        write_png(&root.join("shots/dashboard.png"), 1440, 900);
        SubmitArgs {
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "dashboard".into(),
                station: "shape".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "dashboard.png".into(),
                path: "shots/dashboard.png".into(),
                artifact_type: ArtifactType::Image,
                version_sha: "abcd".into(),
            }),
            anchor: Some(Anchor::Image {
                mark: PixelMark {
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
                },
            }),
            expression: Some(Expression {
                tool: "box".into(),
                color: Some("#5fd7ff".into()),
            }),
            comment: "this total is misaligned".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Should,
            },
            suggestion: None,
        }
    }

    fn station_note_args() -> SubmitArgs {
        SubmitArgs {
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Station,
                id: String::new(),
                station: "build".into(),
            },
            artifact: None,
            anchor: None,
            expression: None,
            comment: "overall: tighten the error handling pass".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Should,
            },
            suggestion: None,
        }
    }

    #[test]
    fn submit_text_validates_and_stores() {
        let (_d, store, root) = store();
        let res = submit(&store, &root, "run", text_args(AskSeverity::Should)).unwrap();
        assert!(res.annotation.id.starts_with("anno_"));
        assert_eq!(res.annotation.status, AnnotationStatus::Open);
        assert!(res.crop_path.is_none(), "text marks don't crop");
        // Persisted and re-readable.
        let back = store
            .read_annotation("run", &res.annotation.id)
            .unwrap()
            .unwrap();
        assert_eq!(back.comment, "handle the declined-card path");
    }

    #[test]
    fn submit_rejects_mismatched_anchor_artifact() {
        let (_d, store, root) = store();
        let mut args = text_args(AskSeverity::Must);
        // Claim the artifact is an image while the anchor is text.
        args.artifact.as_mut().unwrap().artifact_type = ArtifactType::Image;
        let err = submit(&store, &root, "run", args).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn submit_rejects_empty_comment() {
        let (_d, store, root) = store();
        let mut args = text_args(AskSeverity::Nit);
        args.comment = "   ".into();
        let err = submit(&store, &root, "run", args).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn submit_image_crops_to_disk() {
        let (_d, store, root) = store();
        let res = submit(&store, &root, "run", image_args(&root)).unwrap();
        let crop = res.crop_path.expect("image rect must crop");
        // The crop PNG actually exists on disk.
        assert!(root.join(&crop).exists());
        // And it's the expected fraction of the 1440×900 artifact.
        let img = image::open(root.join(&crop)).unwrap();
        assert_eq!(img.width(), (0.18 * 1440.0_f64).round() as u32);
        assert_eq!(img.height(), (0.07 * 900.0_f64).round() as u32);
    }

    #[test]
    fn submit_station_note_has_no_anchor() {
        let (_d, store, root) = store();
        let res = submit(&store, &root, "run", station_note_args()).unwrap();
        assert!(res.annotation.is_station_note());
        assert!(res.crop_path.is_none());
    }

    #[test]
    fn submit_station_note_rejects_an_anchor() {
        let (_d, store, root) = store();
        let mut args = station_note_args();
        args.anchor = Some(Anchor::Text {
            range: TextRange {
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 1,
            },
            quote: "x".into(),
            prefix: String::new(),
            suffix: String::new(),
        });
        let err = submit(&store, &root, "run", args).unwrap_err();
        assert!(matches!(err, McpError::InvalidInput(_)));
    }

    #[test]
    fn crop_image_region_clamps_and_floors_to_one_pixel() {
        let img = image::DynamicImage::ImageRgba8(RgbaImage::new(100, 100));
        // A zero-area rect still yields a 1×1 crop.
        let c = crop_image_region(&img, NormRect { x: 0.5, y: 0.5, w: 0.0, h: 0.0 });
        assert_eq!((c.width(), c.height()), (1, 1));
        // An over-1.0 rect clamps inside the bounds.
        let c = crop_image_region(&img, NormRect { x: 0.9, y: 0.9, w: 0.5, h: 0.5 });
        assert!(c.width() <= 10 && c.height() <= 10);
    }

    #[test]
    fn list_filters_by_work_item_and_tallies_severity() {
        let (_d, store, root) = store();
        // Two on `payment`, one on a different output.
        submit(&store, &root, "run", text_args(AskSeverity::Must)).unwrap();
        submit(&store, &root, "run", text_args(AskSeverity::Nit)).unwrap();
        let mut other = text_args(AskSeverity::Must);
        other.work_item.id = "checkout".into();
        submit(&store, &root, "run", other).unwrap();

        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "payment".into(),
            station: "build".into(),
        };
        let listing = list(&store, "run", &query, false).unwrap();
        assert_eq!(listing.annotations.len(), 2);
        assert_eq!(listing.must, 1);
        assert_eq!(listing.nit, 1);
        assert_eq!(listing.bar_label.as_deref(), Some("1 blocker · 1 nit"));
        assert!(listing.blocks_clean_approve);
        assert_eq!(listing.checkpoint_button_primary, "request_changes");
    }

    #[test]
    fn list_nit_only_keeps_approve_primary() {
        let (_d, store, root) = store();
        submit(&store, &root, "run", text_args(AskSeverity::Nit)).unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "payment".into(),
            station: "build".into(),
        };
        let listing = list(&store, "run", &query, true).unwrap();
        assert!(!listing.blocks_clean_approve);
        assert_eq!(listing.checkpoint_button_primary, "approve");
    }

    #[test]
    fn agent_payload_resolves_text_to_file_line() {
        let (_d, store, root) = store();
        let mut args = text_args(AskSeverity::Should);
        args.suggestion = Some(Suggestion {
            diff: "- old\n+ new".into(),
        });
        submit(&store, &root, "run", args).unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "payment".into(),
            station: "build".into(),
        };
        let payload = agent_re_reference(&store, &root, "run", &query).unwrap();
        assert_eq!(payload.items.len(), 1);
        let item = &payload.items[0];
        assert_eq!(item.suggestion.as_deref(), Some("- old\n+ new"));
        match &item.source {
            ResolvedSource::Text {
                path, start_line, ..
            } => {
                assert_eq!(path, "src/payment.rs");
                assert_eq!(*start_line, 42);
            }
            other => panic!("expected text source, got {other:?}"),
        }
    }

    #[test]
    fn agent_payload_embeds_image_crop() {
        let (_d, store, root) = store();
        submit(&store, &root, "run", image_args(&root)).unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "dashboard".into(),
            station: "shape".into(),
        };
        let payload = agent_re_reference(&store, &root, "run", &query).unwrap();
        match &payload.items[0].source {
            ResolvedSource::Image {
                crop_path, rect, ..
            } => {
                let crop = crop_path.as_ref().expect("crop present");
                assert!(root.join(crop).exists());
                assert!(rect.is_some());
            }
            other => panic!("expected image source, got {other:?}"),
        }
    }

    #[test]
    fn agent_payload_passes_html_dom_src_or_flags_unresolved() {
        let (_d, store, root) = store();
        // HTML with a source map → resolved.
        let mut args = SubmitArgs {
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "cart".into(),
                station: "shape".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "cart.html".into(),
                path: "out/cart.html".into(),
                artifact_type: ArtifactType::Html,
                version_sha: "aa".into(),
            }),
            anchor: Some(Anchor::Html {
                pixel: PixelMark {
                    shape: ImageShape::Pin,
                    point: Some(NormPoint { x: 0.2, y: 0.3 }),
                    rect: None,
                    arrow_from: None,
                    arrow_to: None,
                    path: vec![],
                    render_w: 1440,
                    render_h: 900,
                },
                dom: darkrun_api::annotation::DomAnchor {
                    selector: ".total-row".into(),
                    src: Some("web/Summary.tsx:118".into()),
                    outer_html: Some("<div class=\"total-row\"></div>".into()),
                },
            }),
            expression: None,
            comment: "the total is wrong".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity: AskSeverity::Must,
            },
            suggestion: None,
        };
        submit(&store, &root, "run", args.clone()).unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "cart".into(),
            station: "shape".into(),
        };
        let payload = agent_re_reference(&store, &root, "run", &query).unwrap();
        match &payload.items[0].source {
            ResolvedSource::Html { src, resolved, .. } => {
                assert!(resolved);
                assert_eq!(src.as_deref(), Some("web/Summary.tsx:118"));
            }
            other => panic!("expected html source, got {other:?}"),
        }

        // Now an HTML mark with no source map → unresolved, degrades to selector.
        if let Some(Anchor::Html { dom, .. }) = args.anchor.as_mut() {
            dom.src = None;
        }
        args.work_item.id = "cart2".into();
        args.artifact.as_mut().unwrap().path = "out/cart2.html".into();
        submit(&store, &root, "run", args).unwrap();
        let query2 = WorkItem {
            kind: WorkItemKind::Output,
            id: "cart2".into(),
            station: "shape".into(),
        };
        let payload2 = agent_re_reference(&store, &root, "run", &query2).unwrap();
        match &payload2.items[0].source {
            ResolvedSource::Html {
                resolved, selector, ..
            } => {
                assert!(!resolved);
                assert_eq!(selector, ".total-row");
            }
            other => panic!("expected html source, got {other:?}"),
        }
    }

    #[test]
    fn station_rework_bundle_leads_with_the_station_note() {
        let (_d, store, root) = store();
        // A per-artifact annotation on a unit, plus the global station note.
        submit(&store, &root, "run", text_args(AskSeverity::Should)).unwrap();
        submit(&store, &root, "run", station_note_args()).unwrap();

        // The station-scoped query returns only the station note...
        let station_q = WorkItem {
            kind: WorkItemKind::Station,
            id: String::new(),
            station: "build".into(),
        };
        let station_listing = list(&store, "run", &station_q, false).unwrap();
        assert_eq!(station_listing.annotations.len(), 1);
        assert!(station_listing.annotations[0].is_station_note());

        // ...and the rework bundle (over a mixed set) puts the note first.
        let all = store.list_annotations("run").unwrap();
        let bundle = station_rework_bundle(&all);
        assert!(bundle[0].is_station_note());
        assert_eq!(bundle.len(), 2);

        let body = render_rework_feedback("build", &all);
        assert!(body.contains("Request changes — build"));
        assert!(body.contains("station note"));
        assert!(body.contains("src/payment.rs"));
    }
}

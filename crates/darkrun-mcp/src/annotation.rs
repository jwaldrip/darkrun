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
use darkrun_core::annotation::{
    checkpoint_button_state, count_open_by_severity, flag_scene_changed, pixel_region,
    reanchor_annotation, region_out_of_bounds, scene_changed, CheckpointButton,
};
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

    let _ = crate::commit::commit_state(store, "darkrun: annotation submit");
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

// ─── New-version re-anchor (text + image/pdf re-crop in one pass) ─────────────

/// What re-anchoring one annotation against a new artifact version did. The
/// engine logs/surfaces these; a `SceneChanged` means the region no longer
/// frames the same content, so the annotation was flagged for re-placement
/// rather than silently mis-cropped.
#[derive(Debug, Clone, PartialEq)]
pub enum VersionReAnchor {
    /// A text annotation re-anchored (exact or shifted-but-found / not-found —
    /// the core text pass owns that distinction and the status).
    Text,
    /// An image/pdf region re-cropped cleanly against the new bytes; the crop
    /// file and `version_sha` were rewritten.
    ReCropped,
    /// The pinned region no longer lands on the new version (out of bounds or a
    /// materially different image shape): the annotation was flagged `shifted`
    /// with a "scene changed" note instead of being mis-cropped.
    SceneChanged,
    /// Nothing to do (a pin/path mark with no region, a bare station note, or a
    /// non-pixel/non-text anchor) — only `version_sha` was re-pinned.
    Repinned,
}

/// The decoded dimensions a stored pixel mark was drawn over, when the anchor
/// carries them. Image marks record `render_w`/`render_h`; a pdf rect has no
/// recorded render size, so it can't be compared for a scene change.
fn marked_render_dims(annotation: &Annotation) -> Option<(u32, u32)> {
    match annotation.anchor.as_ref()? {
        Anchor::Image { mark } => Some((mark.render_w, mark.render_h)),
        _ => None,
    }
}

/// Re-anchor every annotation on one artifact path against its new version
/// bytes, refreshing text spans AND image/pdf region crops in a single pass.
///
/// This is the version-update hook the engine runs when a locked artifact's new
/// version is accepted: text annotations re-anchor by the core line-hash / quote
/// search; image and pdf annotations RE-CROP their stored normalized rect out of
/// the new bytes and rewrite `<id>__crop.png`, re-pinning `version_sha`. If a
/// region now falls outside the artifact or the image's shape changed materially
/// (a "scene changed"), the annotation is flagged `shifted` with a note instead
/// of being silently mis-cropped.
///
/// `repo_root` roots the artifact path; `new_bytes` is the new version's content
/// (already on disk at `repo_root/<path>` — passed in so callers that already
/// read it don't re-read). Only OPEN/SHIFTED records are touched; resolved or
/// dismissed annotations are left alone. Best-effort per annotation: an
/// undecodable image skips its re-crop rather than failing the whole pass.
pub fn reanchor_artifact_version(
    store: &StateStore,
    repo_root: &Path,
    run: &str,
    artifact_path: &str,
    new_bytes: &[u8],
) -> Result<Vec<(String, VersionReAnchor)>> {
    let mut outcomes = Vec::new();
    // Decode the new image once (if it is one) so every pixel annotation on this
    // artifact re-crops and scene-checks against the same decoded bytes.
    let new_img = image::load_from_memory(new_bytes).ok();

    for mut annotation in store.list_annotations(run)? {
        // Only annotations pinned to THIS artifact, and only live ones.
        let on_this = annotation
            .artifact
            .as_ref()
            .map(|a| a.path == artifact_path)
            .unwrap_or(false);
        if !on_this {
            continue;
        }
        if !matches!(
            annotation.status,
            AnnotationStatus::Open | AnnotationStatus::Shifted
        ) {
            continue;
        }

        let outcome = reanchor_one(store, repo_root, run, &mut annotation, new_bytes, new_img.as_ref());
        store.write_annotation(run, &annotation)?;
        outcomes.push((annotation.id.clone(), outcome));
    }
    Ok(outcomes)
}

/// Re-anchor a single annotation in place against the new version, returning
/// what happened. Text delegates to the core pass; image/pdf re-crop here.
fn reanchor_one(
    store: &StateStore,
    repo_root: &Path,
    run: &str,
    annotation: &mut Annotation,
    new_bytes: &[u8],
    new_img: Option<&image::DynamicImage>,
) -> VersionReAnchor {
    // Text annotations re-anchor (and re-pin version_sha) via the core pass.
    if let Some(Anchor::Text { .. }) = annotation.anchor.as_ref() {
        reanchor_annotation(annotation, new_bytes);
        return VersionReAnchor::Text;
    }

    // Everything else: re-pin the version_sha up front (mirrors the text pass),
    // then handle a pixel region if there is one.
    if let Some(artifact) = annotation.artifact.as_mut() {
        artifact.version_sha = darkrun_core::hash_bytes(new_bytes);
    }

    let Some(rect) = pixel_region(annotation) else {
        // A pin/path/arrow-less mark, or a non-pixel anchor: nothing to re-crop.
        return VersionReAnchor::Repinned;
    };

    // The region must still land on the artifact.
    if region_out_of_bounds(&rect) {
        flag_scene_changed(
            annotation,
            "annotated region now falls outside the artifact",
        );
        return VersionReAnchor::SceneChanged;
    }

    // For images, decode and re-crop. A pdf has no codec here, so it only
    // re-pins (the document viewer owns pdf rendering).
    let Some(img) = new_img else {
        return VersionReAnchor::Repinned;
    };

    // A materially different image shape means the normalized rect can't be
    // trusted to frame the same content.
    if let Some((old_w, old_h)) = marked_render_dims(annotation) {
        if scene_changed(old_w, old_h, img.width(), img.height(), 0.05) {
            flag_scene_changed(
                annotation,
                "image dimensions changed materially since the mark was made",
            );
            return VersionReAnchor::SceneChanged;
        }
    }

    // Clean re-crop: rewrite the crop file from the new bytes.
    let cropped = crop_image_region(img, rect);
    let out = crop_file_path(store, run, &annotation.id);
    let wrote = (|| -> Result<()> {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| McpError::InvalidInput(format!("crop dir: {e}")))?;
        }
        cropped
            .save_with_format(&out, image::ImageFormat::Png)
            .map_err(|e| McpError::InvalidInput(format!("crop write: {e}")))?;
        Ok(())
    })();
    let _ = wrote; // best-effort: a write fault leaves the stale crop, not a hard fail
    let _ = repo_root; // path is resolved via the store-relative crop file
    VersionReAnchor::ReCropped
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

/// Parse and normalize an opt-in `data-darkrun-src` value into a real
/// `file:line` source. The web layer copies this string verbatim off the marked
/// element's attribute, so it can be anything: blank, a bare path with no line,
/// or a real `file:line`. We accept only `"<non-empty path>:<1-based line>"`
/// (line ≥ 1), trimming surrounding whitespace; everything else returns `None`
/// so the agent degrades to the selector + provenance rather than chasing a
/// bogus location.
///
/// Paths may themselves contain colons (rare, but Windows drive letters and URLs
/// do), so we split on the *last* colon and require the tail to be a line number.
fn parse_source_map(raw: &str) -> Option<String> {
    let s = raw.trim();
    let (file, line) = s.rsplit_once(':')?;
    let file = file.trim();
    let line: u32 = line.trim().parse().ok()?;
    if file.is_empty() || line == 0 {
        return None;
    }
    Some(format!("{file}:{line}"))
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
            // The opt-in source map is just a string the project injected into
            // `data-darkrun-src`; trust nothing about its shape. Only a real
            // `file:line` resolves — an empty/blank/garbage value degrades to
            // the selector + provenance, same as if no map were present.
            let src = dom.src.as_deref().and_then(parse_source_map);
            let resolved = src.is_some();
            ResolvedSource::Html {
                src,
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

/// Resolve one OPEN annotation into its actionable agent bundle. Shared by the
/// per-work-item payload and the station-scoped re-reference.
fn re_reference_item(
    store: &StateStore,
    run: &str,
    repo_root: &Path,
    a: &Annotation,
) -> AgentReReferenceItem {
    AgentReReferenceItem {
        id: a.id.clone(),
        source: resolve_source(store, run, repo_root, a),
        comment: a.comment.clone(),
        ask_kind: a.ask.kind,
        severity: a.ask.severity,
        suggestion: a.suggestion.as_ref().map(|s| s.diff.clone()),
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
        .map(|a| re_reference_item(store, run, repo_root, a))
        .collect();
    Ok(AgentReReferencePayload {
        items,
        must: counts.must,
        should: counts.should,
        nit: counts.nit,
        bar_label: counts.bar_label(),
    })
}

/// Build the agent re-reference payload for a whole **station**: every OPEN
/// annotation hanging on any of the station's work items (the global station
/// note included), each resolved to an actionable bundle, ordered station-note
/// first. This is what the manager auto-surfaces on the next action when a unit
/// re-enters as rework after Request-changes — the agent gets the resolved
/// `file:line` + crop + comment + suggestion without having to ask for it.
///
/// `cap` bounds the resolved items so a noisy station can't blow the prompt: the
/// severity tally always reflects *every* open ask, but only the first `cap`
/// items (station note(s) first, then the highest-severity asks) are resolved.
pub fn station_re_reference(
    store: &StateStore,
    repo_root: &Path,
    run: &str,
    station: &str,
    cap: usize,
) -> Result<AgentReReferencePayload> {
    let all = store.list_annotations(run)?;
    // Scope to this station and order: station note(s) lead, then per-artifact
    // asks by descending severity (must -> should -> nit) so the cap keeps the
    // ones that steer the checkpoint hardest.
    let mut open: Vec<&Annotation> = all
        .iter()
        .filter(|a| a.work_item.station == station && a.status == AnnotationStatus::Open)
        .collect();
    let counts = count_open_by_severity(&all);
    open.sort_by_key(|a| {
        let note_rank = if a.is_station_note() { 0 } else { 1 };
        let sev_rank = match a.ask.severity {
            AskSeverity::Must => 0,
            AskSeverity::Should => 1,
            AskSeverity::Nit => 2,
        };
        (note_rank, sev_rank)
    });
    let items = open
        .iter()
        .take(cap)
        .map(|a| re_reference_item(store, run, repo_root, a))
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
            .as_deref()
            .and_then(parse_source_map)
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
        write_png_color(path, w, h, [10, 20, 30, 255]);
    }

    /// Write a uniformly-colored PNG, so a re-crop's pixels prove which version
    /// they came from.
    fn write_png_color(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = Rgba(rgba);
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
    fn reanchor_repins_a_pdf_region_with_no_decodable_image() {
        let (_d, store, root) = store();
        // A pdf-anchored annotation with an in-bounds region.
        let mut a = submit(&store, &root, "run", image_args(&root)).unwrap().annotation;
        if let Some(art) = a.artifact.as_mut() {
            art.path = "doc.pdf".into();
            art.artifact_type = ArtifactType::Pdf;
        }
        a.anchor = Some(Anchor::Pdf { page: 1, rect: NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 } });
        store.write_annotation("run", &a).unwrap();

        // New bytes that don't decode as an image → the pdf region only re-pins.
        let outcomes = reanchor_artifact_version(&store, &root, "run", "doc.pdf", b"%PDF-1.4 not-an-image").unwrap();
        assert_eq!(outcomes, vec![(a.id.clone(), VersionReAnchor::Repinned)]);
    }

    #[test]
    fn annotation_helpers_cover_id_collision_dims_and_crop_skips() {
        let (_d, store, root) = store();
        let base = submit(&store, &root, "run", image_args(&root)).unwrap().annotation;

        // mint_id: a colliding `_000` id forces the disambiguation loop to bump.
        let ts = "2026-01-01T00:00:00Z";
        let mut clash = base.clone();
        clash.id = "anno_20260101000000_000".into();
        assert_eq!(mint_id(&[clash], ts), "anno_20260101000000_001");

        // marked_render_dims: an image mark records its render size; a pdf anchor
        // carries none (so it can't be scene-compared).
        assert!(marked_render_dims(&base).is_some(), "image mark has render dims");
        let mut pdf = base.clone();
        pdf.anchor = Some(Anchor::Pdf { page: 1, rect: NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 } });
        assert!(marked_render_dims(&pdf).is_none(), "a pdf anchor has no render dims");

        // write_crop: a bare note (no artifact) and an undecodable artifact both
        // skip the crop without erroring.
        let rect = NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        let mut note = base.clone();
        note.artifact = None;
        assert!(write_crop(&store, &root, "run", &note, rect).unwrap().is_none());
        let mut nonimage = base.clone();
        if let Some(a) = nonimage.artifact.as_mut() { a.path = "notes.txt".into(); }
        std::fs::write(root.join("notes.txt"), b"not an image").unwrap();
        assert!(write_crop(&store, &root, "run", &nonimage, rect).unwrap().is_none());
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
    fn reanchor_new_version_recrops_image_and_repins_version() {
        let (_d, store, root) = store();
        // Submit against the original (10,20,30) dashboard so a crop lands.
        let res = submit(&store, &root, "run", image_args(&root)).unwrap();
        let id = res.annotation.id.clone();
        let crop = crop_file_path(&store, "run", &id);
        // The original crop carries the OLD fill color.
        let before = image::open(&crop).unwrap().to_rgba8();
        assert_eq!(before.get_pixel(0, 0).0, [10, 20, 30, 255]);
        let old_sha = res.annotation.artifact.as_ref().unwrap().version_sha.clone();

        // A new version: same dimensions, a distinct fill color.
        let path = root.join("shots/dashboard.png");
        write_png_color(&path, 1440, 900, [200, 40, 60, 255]);
        let new_bytes = std::fs::read(&path).unwrap();

        let outcomes =
            reanchor_artifact_version(&store, &root, "run", "shots/dashboard.png", &new_bytes)
                .unwrap();
        assert_eq!(outcomes, vec![(id.clone(), VersionReAnchor::ReCropped)]);

        // The crop was re-cut from the NEW bytes...
        let after = image::open(&crop).unwrap().to_rgba8();
        assert_eq!(after.get_pixel(0, 0).0, [200, 40, 60, 255]);
        // ...at the same fraction of the (unchanged) 1440×900 artifact...
        assert_eq!(after.width(), (0.18 * 1440.0_f64).round() as u32);
        assert_eq!(after.height(), (0.07 * 900.0_f64).round() as u32);

        // ...and version_sha was re-pinned to the new content.
        let stored = store.read_annotation("run", &id).unwrap().unwrap();
        let new_sha = stored.artifact.as_ref().unwrap().version_sha.clone();
        assert_ne!(new_sha, old_sha);
        assert_eq!(new_sha, darkrun_core::hash_bytes(&new_bytes));
        // Still open — a clean re-crop doesn't disturb the lifecycle.
        assert_eq!(stored.status, AnnotationStatus::Open);
    }

    #[test]
    fn reanchor_new_version_flags_scene_change_on_aspect_shift() {
        let (_d, store, root) = store();
        let res = submit(&store, &root, "run", image_args(&root)).unwrap();
        let id = res.annotation.id.clone();

        // A new version with a materially different aspect ratio (square vs 16:10):
        // the normalized rect can't be trusted to frame the same content.
        let path = root.join("shots/dashboard.png");
        write_png_color(&path, 900, 900, [200, 40, 60, 255]);
        let new_bytes = std::fs::read(&path).unwrap();

        let outcomes =
            reanchor_artifact_version(&store, &root, "run", "shots/dashboard.png", &new_bytes)
                .unwrap();
        assert_eq!(outcomes, vec![(id.clone(), VersionReAnchor::SceneChanged)]);

        let stored = store.read_annotation("run", &id).unwrap().unwrap();
        // Flagged for re-placement, with a "scene changed" note prefixed.
        assert_eq!(stored.status, AnnotationStatus::Shifted);
        assert!(stored.comment.starts_with("[scene changed]"));
        // version_sha still re-pins so the record points at the new bytes.
        assert_eq!(
            stored.artifact.as_ref().unwrap().version_sha,
            darkrun_core::hash_bytes(&new_bytes)
        );
    }

    #[test]
    fn reanchor_skips_off_path_and_settled_marks_and_repins_a_regionless_one() {
        let (_d, store, root) = store();

        // Three image annotations on the same path + one text on a different path.
        let oob = submit(&store, &root, "run", image_args(&root)).unwrap().annotation.id;
        let settled = submit(&store, &root, "run", image_args(&root)).unwrap().annotation.id;
        let pin = submit(&store, &root, "run", image_args(&root)).unwrap().annotation.id;
        let off_path = submit(&store, &root, "run", text_args(AskSeverity::Should)).unwrap().annotation.id;

        // `oob`: push its rect out of bounds → the region no longer lands.
        let mut a = store.read_annotation("run", &oob).unwrap().unwrap();
        if let Some(Anchor::Image { mark }) = a.anchor.as_mut() {
            mark.rect = Some(NormRect { x: 0.9, y: 0.9, w: 0.5, h: 0.5 });
        }
        store.write_annotation("run", &a).unwrap();

        // `settled`: mark it addressed → the pass leaves it alone.
        let mut a = store.read_annotation("run", &settled).unwrap().unwrap();
        a.status = AnnotationStatus::Addressed;
        store.write_annotation("run", &a).unwrap();

        // `pin`: a regionless mark (no rect, no arrow) → nothing to re-crop.
        let mut a = store.read_annotation("run", &pin).unwrap().unwrap();
        if let Some(Anchor::Image { mark }) = a.anchor.as_mut() {
            mark.rect = None;
            mark.arrow_from = None;
            mark.arrow_to = None;
            mark.point = None;
        }
        store.write_annotation("run", &a).unwrap();

        // New bytes for the image path (same dims, distinct fill).
        let path = root.join("shots/dashboard.png");
        write_png_color(&path, 1440, 900, [1, 2, 3, 255]);
        let new_bytes = std::fs::read(&path).unwrap();
        let outcomes =
            reanchor_artifact_version(&store, &root, "run", "shots/dashboard.png", &new_bytes).unwrap();

        let got = |id: &str| outcomes.iter().find(|(i, _)| i == id).map(|(_, o)| o.clone());
        // The off-path text mark and the settled mark are skipped entirely.
        assert!(got(&off_path).is_none(), "a different-artifact mark is not touched");
        assert!(got(&settled).is_none(), "an addressed mark is not touched");
        // The out-of-bounds region is flagged; the regionless mark just re-pins.
        assert_eq!(got(&oob), Some(VersionReAnchor::SceneChanged));
        assert_eq!(got(&pin), Some(VersionReAnchor::Repinned));
    }

    #[test]
    fn reanchor_new_version_refreshes_text_in_the_same_pass() {
        let (_d, store, root) = store();
        // A text annotation pinned to src/payment.rs at line 42.
        let res = submit(&store, &root, "run", text_args(AskSeverity::Should)).unwrap();
        let id = res.annotation.id.clone();

        // New version: the quoted span moved down two lines.
        let new_bytes = b"// header\n// header2\nfn charge()\n";
        let outcomes =
            reanchor_artifact_version(&store, &root, "run", "src/payment.rs", new_bytes).unwrap();
        assert_eq!(outcomes, vec![(id.clone(), VersionReAnchor::Text)]);

        let stored = store.read_annotation("run", &id).unwrap().unwrap();
        // version_sha re-pinned and the span re-based onto the new line.
        assert_eq!(
            stored.artifact.as_ref().unwrap().version_sha,
            darkrun_core::hash_bytes(new_bytes)
        );
        if let Some(Anchor::Text { range, .. }) = &stored.anchor {
            assert_eq!(range.start_line, 3);
        } else {
            panic!("anchor must still be text");
        }
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
    fn parse_source_map_accepts_only_real_file_line() {
        // The strong case: a real `file:line` normalizes (and trims).
        assert_eq!(
            parse_source_map("web/Summary.tsx:118").as_deref(),
            Some("web/Summary.tsx:118")
        );
        assert_eq!(
            parse_source_map("  web/Summary.tsx : 118  ").as_deref(),
            Some("web/Summary.tsx:118")
        );
        // A path that itself contains colons splits on the LAST one.
        assert_eq!(
            parse_source_map("C:/app/Summary.tsx:42").as_deref(),
            Some("C:/app/Summary.tsx:42")
        );
        // Garbage the opt-in injector might emit all degrade to None.
        assert_eq!(parse_source_map(""), None);
        assert_eq!(parse_source_map("   "), None);
        assert_eq!(parse_source_map("web/Summary.tsx"), None); // no line
        assert_eq!(parse_source_map("web/Summary.tsx:"), None); // empty line
        assert_eq!(parse_source_map("web/Summary.tsx:abc"), None); // non-numeric
        assert_eq!(parse_source_map("web/Summary.tsx:0"), None); // 1-based only
        assert_eq!(parse_source_map(":118"), None); // empty path
    }

    /// A blank `data-darkrun-src=""` (the opt-in injector firing on an element it
    /// couldn't map) must degrade to the selector, not flag a bogus resolve.
    #[test]
    fn agent_payload_treats_blank_source_map_as_unresolved() {
        let (_d, store, root) = store();
        let args = SubmitArgs {
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
                    // The injector fired but had nothing to map → empty string.
                    src: Some("   ".into()),
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
        submit(&store, &root, "run", args).unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "cart".into(),
            station: "shape".into(),
        };
        let payload = agent_re_reference(&store, &root, "run", &query).unwrap();
        match &payload.items[0].source {
            ResolvedSource::Html {
                src,
                resolved,
                selector,
                ..
            } => {
                assert!(!resolved, "a blank source map must not resolve");
                assert!(src.is_none());
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

    #[test]
    fn describe_anchor_covers_every_variant() {
        use darkrun_api::annotation::{Anchor, DomAnchor, NormRect, PixelMark};
        let rect = NormRect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
        let mark = PixelMark {
            shape: ImageShape::Rect,
            point: None,
            rect: Some(rect.clone()),
            arrow_from: None,
            arrow_to: None,
            path: vec![],
            render_w: 800,
            render_h: 600,
        };
        let trange = |s: u32, e: u32| TextRange { start_line: s, start_col: 0, end_line: e, end_col: 0 };
        let text = |s, e| Anchor::Text { range: trange(s, e), quote: "q".into(), prefix: String::new(), suffix: String::new() };

        assert_eq!(describe_anchor(&text(5, 5)), "line 5");
        assert_eq!(describe_anchor(&text(5, 8)), "lines 5–8");
        assert_eq!(describe_anchor(&Anchor::Image { mark: mark.clone() }), "image region");
        let html = Anchor::Html {
            pixel: mark.clone(),
            dom: DomAnchor { selector: ".btn".into(), src: None, outer_html: None },
        };
        assert_eq!(describe_anchor(&html), ".btn");
        assert_eq!(describe_anchor(&Anchor::Pdf { page: 3, rect: rect.clone() }), "pdf page 3");
        assert_eq!(
            describe_anchor(&Anchor::Svg { element_id: Some("#icon".into()), xpath: None, bbox: rect.clone() }),
            "#icon"
        );
        assert_eq!(
            describe_anchor(&Anchor::Svg { element_id: None, xpath: Some("/svg/g".into()), bbox: rect.clone() }),
            "/svg/g"
        );
        assert_eq!(
            describe_anchor(&Anchor::Svg { element_id: None, xpath: None, bbox: rect.clone() }),
            "svg element"
        );
        assert!(describe_anchor(&Anchor::Video { t_start: 12.0, t_end: 15.0, rect: None }).contains("video @ 12"));
    }

    #[test]
    fn render_rework_emits_a_request_changes_header() {
        // With no open annotations the body is just the header — exercises the
        // bundle + render path's frame. (Per-item rendering is covered by the
        // full submit→list→render flow tests above.)
        let body = render_rework_feedback("build", &[]);
        assert_eq!(body, "# Request changes — build\n\n");
        assert!(station_rework_bundle(&[]).is_empty());
    }

    #[test]
    fn submit_rejects_a_per_artifact_annotation_missing_its_artifact_or_anchor() {
        let (_d, store, root) = store();
        // A non-station work item with no artifact.
        let mut no_artifact = text_args(AskSeverity::Should);
        no_artifact.artifact = None;
        assert!(matches!(
            submit(&store, &root, "run", no_artifact).unwrap_err(),
            McpError::InvalidInput(_)
        ));
        // An artifact present but no anchor.
        let mut no_anchor = text_args(AskSeverity::Should);
        no_anchor.anchor = None;
        assert!(matches!(
            submit(&store, &root, "run", no_anchor).unwrap_err(),
            McpError::InvalidInput(_)
        ));
        // A station note carrying a stray anchor is rejected too.
        let mut noted = station_note_args();
        noted.anchor = Some(Anchor::Text {
            range: TextRange { start_line: 1, start_col: 0, end_line: 1, end_col: 1 },
            quote: "x".into(), prefix: String::new(), suffix: String::new(),
        });
        assert!(matches!(
            submit(&store, &root, "run", noted).unwrap_err(),
            McpError::InvalidInput(_)
        ));
    }

    #[test]
    fn rework_render_and_station_re_reference_cover_every_severity_and_the_note() {
        let (_d, store, root) = store();
        // A station note + per-artifact asks at each severity, all on "build".
        submit(&store, &root, "run", station_note_args()).unwrap();
        for sev in [AskSeverity::Must, AskSeverity::Should, AskSeverity::Nit] {
            let mut a = text_args(sev);
            // Distinct artifact ids so they don't dedup onto one record.
            a.artifact.as_mut().unwrap().id = format!("payment-{sev:?}");
            a.work_item.id = format!("payment-{sev:?}");
            submit(&store, &root, "run", a).unwrap();
        }
        let anns = store.list_annotations("run").unwrap();
        let body = render_rework_feedback("build", &anns);
        // The header + the station note + each severity label render.
        assert!(body.contains("# Request changes — build"));
        assert!(body.contains("**station note**"));
        assert!(body.contains("**blocker**") && body.contains("**high**") && body.contains("**nit**"));

        // station_re_reference resolves + severity-sorts the open asks.
        let payload = station_re_reference(&store, &root, "run", "build", 10).unwrap();
        assert!(payload.must >= 1 && payload.should >= 1 && payload.nit >= 1);
        assert!(!payload.items.is_empty());
        // The cap bounds the resolved item count.
        let capped = station_re_reference(&store, &root, "run", "build", 1).unwrap();
        assert_eq!(capped.items.len(), 1, "the cap bounds resolved items");
    }

    #[test]
    fn rect_of_bounds_an_arrow_rect_and_pin() {
        use darkrun_api::{ImageShape, NormPoint, NormRect, PixelMark};
        // An arrow's bounding box spans its two endpoints.
        let arrow = PixelMark {
            shape: ImageShape::Arrow, point: None, rect: None,
            arrow_from: Some(NormPoint { x: 0.6, y: 0.2 }),
            arrow_to: Some(NormPoint { x: 0.2, y: 0.5 }),
            path: vec![], render_w: 100, render_h: 100,
        };
        let r = rect_of(&arrow).expect("an arrow has a bounding box");
        assert!((r.x - 0.2).abs() < 1e-9 && (r.y - 0.2).abs() < 1e-9);
        assert!((r.w - 0.4).abs() < 1e-9 && (r.h - 0.3).abs() < 1e-9);
        // A rect mark returns its rect; a pin returns None.
        let rect = PixelMark {
            shape: ImageShape::Rect, point: None,
            rect: Some(NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 }),
            arrow_from: None, arrow_to: None, path: vec![], render_w: 1, render_h: 1,
        };
        assert!(rect_of(&rect).is_some());
        let pin = PixelMark {
            shape: ImageShape::Pin, point: Some(NormPoint { x: 0.5, y: 0.5 }),
            rect: None, arrow_from: None, arrow_to: None, path: vec![], render_w: 1, render_h: 1,
        };
        assert!(rect_of(&pin).is_none());
    }

    #[test]
    fn resolve_source_passes_through_pdf_svg_and_video_anchors() {
        use darkrun_api::NormRect;
        let (_d, store, root) = store();
        // Seed a real annotation, then re-anchor it onto each passthrough type.
        submit(&store, &root, "run", text_args(AskSeverity::Should)).unwrap();
        let base = store.list_annotations("run").unwrap().pop().expect("one annotation");
        let rect = || NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        let anchors = [
            Anchor::Pdf { page: 3, rect: rect() },
            Anchor::Svg { element_id: Some("e1".into()), xpath: None, bbox: rect() },
            Anchor::Video { t_start: 1.0, t_end: 2.5, rect: Some(rect()) },
        ];
        for anchor in anchors {
            let mut a = base.clone();
            a.anchor = Some(anchor);
            assert!(matches!(
                resolve_source(&store, "run", &root, &a),
                ResolvedSource::Passthrough { .. }
            ));
        }
    }
}

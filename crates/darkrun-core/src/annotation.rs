//! Annotation storage, the text re-anchor pass, and severity aggregation.
//!
//! The [`Annotation`] envelope (and its typed [`Anchor`]) is the wire shape in
//! `darkrun-api`; this module owns its on-disk life and the two algorithms the
//! envelope exists to serve:
//!
//! - **Storage** — one JSON file per annotation under
//!   `.darkrun/<run>/annotations/<id>.json`, indexed in memory by `work_item`.
//!   Records are retained on exit (`status` carries the lifecycle); nothing is
//!   hard-deleted.
//! - **Text re-anchor** — when the agent emits a new artifact version, a text
//!   annotation re-anchors against it: exact if the lines still hash-match,
//!   else a quote-search disambiguated by prefix/suffix, else flagged
//!   `shifted` for re-placement.
//! - **Severity aggregation** — the open-ask counts that steer the checkpoint.
//!   `should`/`must` block a clean Approve; `nit` never does.

use std::fs;
use std::path::PathBuf;

use darkrun_api::annotation::{
    Anchor, Annotation, AnnotationStatus, AskSeverity, TextRange, WorkItem, WorkItemKind,
};

use crate::error::{CoreError, Result};
use crate::state::{io, StateStore};
use crate::witness::hash_bytes;

impl StateStore {
    /// The `annotations/` directory for a run.
    pub fn annotations_dir(&self, slug: &str) -> PathBuf {
        self.run_dir(slug).join("annotations")
    }

    /// The JSON path for one annotation.
    pub fn annotation_path(&self, slug: &str, id: &str) -> PathBuf {
        self.annotations_dir(slug).join(format!("{id}.json"))
    }

    /// Write (or overwrite) an annotation's JSON record. The store does not
    /// mint ids or timestamps — the caller stamps those before storing.
    pub fn write_annotation(&self, run: &str, annotation: &Annotation) -> Result<()> {
        let dir = self.annotations_dir(run);
        io(&dir, fs::create_dir_all(&dir))?;
        let path = self.annotation_path(run, &annotation.id);
        let json = serde_json::to_string_pretty(annotation)?;
        io(&path, fs::write(&path, json))
    }

    /// Read one annotation by id, or `None` when absent.
    pub fn read_annotation(&self, run: &str, id: &str) -> Result<Option<Annotation>> {
        let path = self.annotation_path(run, id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = io(&path, fs::read_to_string(&path))?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    /// List every annotation for a run, sorted by id (which is creation-ordered
    /// when ids are time-sortable). Retained records of every status are
    /// returned — filtering is the caller's job.
    pub fn list_annotations(&self, run: &str) -> Result<Vec<Annotation>> {
        let dir = self.annotations_dir(run);
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        let mut paths: Vec<PathBuf> = Vec::new();
        for entry in io(&dir, fs::read_dir(&dir))? {
            let entry = io(&dir, entry)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();
        for path in paths {
            let raw = io(&path, fs::read_to_string(&path))?;
            out.push(serde_json::from_str(&raw)?);
        }
        Ok(out)
    }

    /// List the annotations indexed onto one work item.
    ///
    /// A `station`-kind query (with an empty `id`) returns the station-level
    /// records — including the global station note — for that station. A
    /// `unit`/`output` query matches kind + id + station exactly.
    pub fn list_annotations_for_work_item(
        &self,
        run: &str,
        work_item: &WorkItem,
    ) -> Result<Vec<Annotation>> {
        let all = self.list_annotations(run)?;
        Ok(all
            .into_iter()
            .filter(|a| work_item_matches(&a.work_item, work_item))
            .collect())
    }

    /// List the still-open annotations on one work item — the inputs to the
    /// checkpoint's severity steering.
    pub fn list_open_annotations_for_work_item(
        &self,
        run: &str,
        work_item: &WorkItem,
    ) -> Result<Vec<Annotation>> {
        Ok(self
            .list_annotations_for_work_item(run, work_item)?
            .into_iter()
            .filter(|a| a.status == AnnotationStatus::Open)
            .collect())
    }

    /// Transition an annotation's [`AnnotationStatus`], retaining the record.
    /// Errors if the annotation does not exist (nothing is created here).
    pub fn update_annotation_status(
        &self,
        run: &str,
        id: &str,
        status: AnnotationStatus,
    ) -> Result<Annotation> {
        let mut annotation = self
            .read_annotation(run, id)?
            .ok_or_else(|| CoreError::AnnotationNotFound(id.to_string()))?;
        annotation.status = status;
        self.write_annotation(run, &annotation)?;
        Ok(annotation)
    }
}

/// Whether a stored annotation's work item matches a query.
///
/// `kind` and `station` must always match. A `station`-kind query ignores the
/// `id` (it scopes the whole station); a `unit`/`output` query also matches the
/// `id`.
fn work_item_matches(stored: &WorkItem, query: &WorkItem) -> bool {
    if stored.kind != query.kind || stored.station != query.station {
        return false;
    }
    match query.kind {
        WorkItemKind::Station => true,
        WorkItemKind::Unit | WorkItemKind::Output => stored.id == query.id,
    }
}

// ─── Text re-anchor ──────────────────────────────────────────────────────────

/// The outcome of re-anchoring a text annotation against a new artifact
/// version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReAnchor {
    /// The span's lines still hash-match at their recorded position — the
    /// agent jumps straight there, no shift.
    Exact(TextRange),
    /// The lines moved, but the `quote` was re-found (disambiguated by
    /// prefix/suffix); the span was silently re-based to this new range.
    Shifted(TextRange),
    /// The `quote` could not be re-found — the annotation is flagged
    /// [`AnnotationStatus::Shifted`] and surfaces for re-placement.
    NotFound,
}

/// Re-anchor a text annotation's span against the current artifact text.
///
/// The algorithm follows the spec: try the recorded line range first — if the
/// quote's lines still hash-match where they were, it's exact. Otherwise search
/// the text for the quote, using `prefix`/`suffix` to disambiguate duplicates;
/// a unique (or prefix/suffix-resolved) hit re-bases the span. No hit → not
/// found.
///
/// `range`/`quote`/`prefix`/`suffix` are the stored [`Anchor::Text`] fields;
/// `current` is the new artifact's full text.
pub fn reanchor_text(
    range: &TextRange,
    quote: &str,
    prefix: &str,
    suffix: &str,
    current: &str,
) -> ReAnchor {
    let lines: Vec<&str> = current.lines().collect();

    // 1) Exact: do the recorded lines still hash-match the quote's lines?
    if lines_match_at(&lines, range.start_line, quote) {
        return ReAnchor::Exact(*range);
    }

    // 2) Quote-search. Locate every byte offset where the quote occurs, then
    //    keep the candidates whose surrounding context matches prefix/suffix.
    let candidates = find_quote_offsets(current, quote);
    if candidates.is_empty() {
        return ReAnchor::NotFound;
    }

    let resolved: Vec<usize> = if candidates.len() == 1 {
        candidates
    } else {
        let narrowed: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|&off| context_matches(current, off, quote.len(), prefix, suffix))
            .collect();
        // If prefix/suffix disambiguates to exactly one, take it; otherwise the
        // span is ambiguous and we can't safely re-base it.
        if narrowed.len() == 1 {
            narrowed
        } else {
            return ReAnchor::NotFound;
        }
    };

    let off = resolved[0];
    let new_range = range_from_offset(current, off, quote);
    // A re-find that lands back on the original lines is still "exact" in
    // spirit, but reaching here means the line-hash check already failed, so
    // report it as a (possibly zero-distance) shift — re-based, flagged.
    ReAnchor::Shifted(new_range)
}

/// Whether the lines starting at `start_line` (1-based) hash-match the quote's
/// lines exactly.
fn lines_match_at(lines: &[&str], start_line: u32, quote: &str) -> bool {
    if start_line == 0 {
        return false;
    }
    let start = (start_line - 1) as usize;
    let quote_lines: Vec<&str> = quote.lines().collect();
    if quote_lines.is_empty() || start + quote_lines.len() > lines.len() {
        return false;
    }
    quote_lines
        .iter()
        .enumerate()
        .all(|(i, ql)| hash_bytes(lines[start + i].as_bytes()) == hash_bytes(ql.as_bytes()))
}

/// Every byte offset in `text` where `quote` begins.
fn find_quote_offsets(text: &str, quote: &str) -> Vec<usize> {
    if quote.is_empty() {
        return Vec::new();
    }
    let mut offsets = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find(quote) {
        let abs = from + rel;
        offsets.push(abs);
        from = abs + 1; // allow overlapping matches
    }
    offsets
}

/// Whether the text immediately before/after a quote occurrence ends-with the
/// recorded prefix and starts-with the recorded suffix. An empty prefix/suffix
/// is treated as "no constraint".
fn context_matches(
    text: &str,
    offset: usize,
    quote_len: usize,
    prefix: &str,
    suffix: &str,
) -> bool {
    let before = &text[..offset];
    let after_start = offset + quote_len;
    let after = if after_start <= text.len() {
        &text[after_start..]
    } else {
        ""
    };
    let prefix_ok = prefix.is_empty() || before.ends_with(prefix);
    let suffix_ok = suffix.is_empty() || after.starts_with(suffix);
    prefix_ok && suffix_ok
}

/// Compute the [`TextRange`] (1-based lines, 0-based cols) for a quote found at
/// byte `offset` in `text`.
fn range_from_offset(text: &str, offset: usize, quote: &str) -> TextRange {
    let (start_line, start_col) = line_col_at(text, offset);
    let end_off = offset + quote.len();
    let (end_line, end_col) = line_col_at(text, end_off);
    TextRange {
        start_line,
        start_col,
        end_line,
        end_col,
    }
}

/// The 1-based line / 0-based column of a byte offset in `text`.
fn line_col_at(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let mut line: u32 = 1;
    let mut col: u32 = 0;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Re-anchor a stored annotation against new artifact bytes and apply the
/// result to its status/anchor in place.
///
/// Text annotations re-anchor by the algorithm above: an exact or shifted-but-
/// found result re-bases the range (and leaves the status untouched on exact,
/// flips it to `Shifted` only when not found). Non-text annotations carry
/// version-relative coords, so this only re-pins the `version_sha` for them —
/// the pixel re-crop is the caller's concern (it owns the image codec); see
/// [`pixel_region`] and [`flag_scene_changed`] for the pieces a pixel re-anchor
/// uses. Returns the [`ReAnchor`] outcome for text annotations, `None` for
/// non-text.
pub fn reanchor_annotation(annotation: &mut Annotation, new_bytes: &[u8]) -> Option<ReAnchor> {
    let new_sha = hash_bytes(new_bytes);
    if let Some(artifact) = annotation.artifact.as_mut() {
        artifact.version_sha = new_sha;
    }

    let Some(Anchor::Text {
        range,
        quote,
        prefix,
        suffix,
    }) = annotation.anchor.as_ref()
    else {
        return None;
    };

    let current = String::from_utf8_lossy(new_bytes);
    let outcome = reanchor_text(range, quote, prefix, suffix, &current);

    match &outcome {
        ReAnchor::Exact(_) => { /* lines held; nothing to re-base */ }
        ReAnchor::Shifted(new_range) => {
            if let Some(Anchor::Text { range, .. }) = annotation.anchor.as_mut() {
                *range = *new_range;
            }
        }
        ReAnchor::NotFound => {
            annotation.status = AnnotationStatus::Shifted;
        }
    }
    Some(outcome)
}

// ─── Pixel (image / pdf) re-anchor ────────────────────────────────────────────

/// The normalized region an image/pdf annotation pinned — the rect to re-crop
/// out of a new artifact version. Text/html/svg/video anchors (and pin/path/
/// arrow-less marks) have no single pixel region to re-crop, so they yield
/// `None`.
///
/// A pdf anchor carries its `rect` directly; an image anchor carries a
/// [`PixelMark`] whose `rect` (or arrow bounding box) is the region — the same
/// rule the crop path uses at submit time.
pub fn pixel_region(annotation: &Annotation) -> Option<darkrun_api::annotation::NormRect> {
    use darkrun_api::annotation::Anchor as A;
    match annotation.anchor.as_ref()? {
        A::Image { mark } => mark_rect(mark),
        A::Pdf { rect, .. } => Some(*rect),
        _ => None,
    }
}

/// The rectangle of a pixel mark — the explicit `rect`, or the bounding box of
/// an arrow's two endpoints. `None` for pin/path (no single region).
fn mark_rect(mark: &darkrun_api::annotation::PixelMark) -> Option<darkrun_api::annotation::NormRect> {
    use darkrun_api::annotation::NormRect;
    if let Some(rect) = mark.rect {
        return Some(rect);
    }
    if let (Some(from), Some(to)) = (mark.arrow_from, mark.arrow_to) {
        return Some(NormRect {
            x: from.x.min(to.x),
            y: from.y.min(to.y),
            w: (from.x - to.x).abs(),
            h: (from.y - to.y).abs(),
        });
    }
    None
}

/// Whether a pinned normalized region falls (substantially) outside the unit
/// square — i.e. the rect no longer lands on the artifact, so re-cropping it
/// would yield a clamped, mis-aligned region. A small epsilon tolerates the
/// rounding a normalize/denormalize round-trip introduces.
pub fn region_out_of_bounds(rect: &darkrun_api::annotation::NormRect) -> bool {
    const EPS: f64 = 1e-3;
    rect.x < -EPS
        || rect.y < -EPS
        || rect.x + rect.w > 1.0 + EPS
        || rect.y + rect.h > 1.0 + EPS
        || rect.w <= 0.0
        || rect.h <= 0.0
}

/// Whether a new artifact version changed the image's aspect ratio materially
/// enough that a normalized region can no longer be trusted to frame the same
/// content — a "scene changed". Pure size (both dims scaled the same) keeps the
/// normalized rect valid; a ratio shift past `tol` (default ~5%) does not.
///
/// `(old_w, old_h)` is the render size the mark was drawn over (from the stored
/// [`PixelMark`], or the pdf page's prior crop); `(new_w, new_h)` is the new
/// version's decoded size. A zero in either pair is treated as a scene change
/// (we can't reason about it).
pub fn scene_changed(old_w: u32, old_h: u32, new_w: u32, new_h: u32, tol: f64) -> bool {
    if old_w == 0 || old_h == 0 || new_w == 0 || new_h == 0 {
        return true;
    }
    let old_ar = old_w as f64 / old_h as f64;
    let new_ar = new_w as f64 / new_h as f64;
    (old_ar - new_ar).abs() / old_ar > tol
}

/// Flag a pixel annotation whose region no longer frames the same content under
/// a new version: shift its status to [`AnnotationStatus::Shifted`] and prefix
/// its comment with a one-line "scene changed" note (idempotent — re-runs don't
/// stack notes). Returns whether anything changed.
pub fn flag_scene_changed(annotation: &mut Annotation, why: &str) -> bool {
    const MARKER: &str = "[scene changed]";
    let mut changed = false;
    if annotation.status != AnnotationStatus::Shifted {
        annotation.status = AnnotationStatus::Shifted;
        changed = true;
    }
    if !annotation.comment.starts_with(MARKER) {
        annotation.comment = format!("{MARKER} {why} — {}", annotation.comment);
        changed = true;
    }
    changed
}

// ─── Severity aggregation ────────────────────────────────────────────────────

/// Open-ask counts by severity for a work item — the legible steering the
/// checkpoint bar shows (`2 blocker · 1 high · 3 nit`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenSeverityCounts {
    /// Open `must` asks (blockers).
    pub must: usize,
    /// Open `should` asks (high).
    pub should: usize,
    /// Open `nit` asks (never block).
    pub nit: usize,
}

impl OpenSeverityCounts {
    /// Whether any open ask blocks a clean Approve — `must` or `should`.
    pub fn blocks_clean_approve(self) -> bool {
        self.must > 0 || self.should > 0
    }

    /// The bar label, in factory vocabulary — e.g. `2 blocker · 1 high · 3 nit`.
    /// `None` when there are no open asks at all (the bar shows nothing).
    pub fn bar_label(self) -> Option<String> {
        let mut parts = Vec::new();
        if self.must > 0 {
            parts.push(format!("{} blocker", self.must));
        }
        if self.should > 0 {
            parts.push(format!("{} high", self.should));
        }
        if self.nit > 0 {
            parts.push(format!("{} nit", self.nit));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }
}

/// Which checkpoint button is primary, steered by the open-ask severities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointButton {
    /// Clean (no open asks, or only nits) — Approve is primary.
    ApproveIsPrimary,
    /// Any open `should`/`must` — Approve darkens, Request-changes is primary.
    RequestChangesIsPrimary,
}

/// Tally the open asks across a set of annotations by severity. Only
/// [`AnnotationStatus::Open`] records count.
pub fn count_open_by_severity(annotations: &[Annotation]) -> OpenSeverityCounts {
    let mut counts = OpenSeverityCounts::default();
    for a in annotations {
        if a.status != AnnotationStatus::Open {
            continue;
        }
        match a.ask.severity {
            AskSeverity::Must => counts.must += 1,
            AskSeverity::Should => counts.should += 1,
            AskSeverity::Nit => counts.nit += 1,
        }
    }
    counts
}

/// The checkpoint button steering for a set of annotations: Request-changes
/// goes primary the moment any open `should`/`must` is present.
pub fn checkpoint_button_state(annotations: &[Annotation]) -> CheckpointButton {
    if count_open_by_severity(annotations).blocks_clean_approve() {
        CheckpointButton::RequestChangesIsPrimary
    } else {
        CheckpointButton::ApproveIsPrimary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use darkrun_api::annotation::{
        Anchor, Annotation, ArtifactInfo, ArtifactType, Ask, AskKind, AskSeverity, Expression,
        WorkItem, WorkItemKind,
    };
    use darkrun_api::common::AuthorType;

    fn text_anno(id: &str, severity: AskSeverity, status: AnnotationStatus) -> Annotation {
        Annotation {
            id: id.into(),
            created_at: "2026-05-31T00:00:00Z".into(),
            author: AuthorType::Human,
            work_item: WorkItem {
                kind: WorkItemKind::Output,
                id: "payment".into(),
                station: "build".into(),
            },
            artifact: Some(ArtifactInfo {
                id: "payment.rs".into(),
                path: "outputs/payment.rs".into(),
                artifact_type: ArtifactType::Text,
                version_sha: "old".into(),
            }),
            anchor: Some(Anchor::Text {
                range: TextRange {
                    start_line: 2,
                    start_col: 0,
                    end_line: 2,
                    end_col: 5,
                },
                quote: "world".into(),
                prefix: String::new(),
                suffix: String::new(),
            }),
            expression: Some(Expression {
                tool: "select".into(),
                color: None,
            }),
            comment: "fix this".into(),
            ask: Ask {
                kind: AskKind::Change,
                severity,
            },
            suggestion: None,
            status,
        }
    }

    #[test]
    fn store_list_and_update_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());

        let a = text_anno("anno_a", AskSeverity::Should, AnnotationStatus::Open);
        let b = text_anno("anno_b", AskSeverity::Nit, AnnotationStatus::Open);
        store.write_annotation("run", &a).unwrap();
        store.write_annotation("run", &b).unwrap();

        let all = store.list_annotations("run").unwrap();
        assert_eq!(all.len(), 2);
        // Sorted by id.
        assert_eq!(all[0].id, "anno_a");

        let one = store.read_annotation("run", "anno_b").unwrap().unwrap();
        assert_eq!(one.id, "anno_b");
        assert!(store.read_annotation("run", "ghost").unwrap().is_none());

        // Update status — record is retained, not deleted.
        let updated = store
            .update_annotation_status("run", "anno_a", AnnotationStatus::Addressed)
            .unwrap();
        assert_eq!(updated.status, AnnotationStatus::Addressed);
        let reread = store.read_annotation("run", "anno_a").unwrap().unwrap();
        assert_eq!(reread.status, AnnotationStatus::Addressed);
        // Still two on disk.
        assert_eq!(store.list_annotations("run").unwrap().len(), 2);
    }

    #[test]
    fn update_missing_annotation_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());
        let err = store
            .update_annotation_status("run", "ghost", AnnotationStatus::Dismissed)
            .unwrap_err();
        assert!(matches!(err, CoreError::AnnotationNotFound(_)));
    }

    #[test]
    fn list_by_work_item_filters_kind_id_station() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());

        let mut on_payment = text_anno("anno_a", AskSeverity::Must, AnnotationStatus::Open);
        on_payment.work_item.id = "payment".into();
        let mut on_other = text_anno("anno_b", AskSeverity::Must, AnnotationStatus::Open);
        on_other.work_item.id = "checkout".into();
        let mut on_other_station = text_anno("anno_c", AskSeverity::Must, AnnotationStatus::Open);
        on_other_station.work_item.station = "design".into();

        store.write_annotation("run", &on_payment).unwrap();
        store.write_annotation("run", &on_other).unwrap();
        store.write_annotation("run", &on_other_station).unwrap();

        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "payment".into(),
            station: "build".into(),
        };
        let hits = store
            .list_annotations_for_work_item("run", &query)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "anno_a");
    }

    #[test]
    fn station_query_returns_station_scoped_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());

        let note = Annotation {
            work_item: WorkItem {
                kind: WorkItemKind::Station,
                id: String::new(),
                station: "build".into(),
            },
            artifact: None,
            anchor: None,
            expression: None,
            ..text_anno("anno_note", AskSeverity::Should, AnnotationStatus::Open)
        };
        store.write_annotation("run", &note).unwrap();
        // A per-output annotation on the same station must NOT match a station
        // query.
        let per_output = text_anno("anno_out", AskSeverity::Should, AnnotationStatus::Open);
        store.write_annotation("run", &per_output).unwrap();

        let query = WorkItem {
            kind: WorkItemKind::Station,
            id: String::new(),
            station: "build".into(),
        };
        let hits = store
            .list_annotations_for_work_item("run", &query)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "anno_note");
    }

    #[test]
    fn open_filter_excludes_resolved() {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path());
        store
            .write_annotation(
                "run",
                &text_anno("anno_a", AskSeverity::Must, AnnotationStatus::Open),
            )
            .unwrap();
        store
            .write_annotation(
                "run",
                &text_anno("anno_b", AskSeverity::Must, AnnotationStatus::Addressed),
            )
            .unwrap();
        let query = WorkItem {
            kind: WorkItemKind::Output,
            id: "payment".into(),
            station: "build".into(),
        };
        let open = store
            .list_open_annotations_for_work_item("run", &query)
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "anno_a");
    }

    // ─── Text re-anchor ──────────────────────────────────────────────────────

    #[test]
    fn reanchor_exact_when_lines_hash_match() {
        let text = "alpha\nworld\nbeta\n";
        let range = TextRange {
            start_line: 2,
            start_col: 0,
            end_line: 2,
            end_col: 5,
        };
        let outcome = reanchor_text(&range, "world", "", "", text);
        assert_eq!(outcome, ReAnchor::Exact(range));
    }

    #[test]
    fn reanchor_shifted_when_quote_moves() {
        // The quote moved down two lines; line-hash at line 2 no longer holds,
        // so quote-search re-finds it and re-bases the range.
        let text = "new\nlines\nprepended\nworld\nbeta\n";
        let range = TextRange {
            start_line: 2,
            start_col: 0,
            end_line: 2,
            end_col: 5,
        };
        let outcome = reanchor_text(&range, "world", "", "", text);
        match outcome {
            ReAnchor::Shifted(r) => {
                assert_eq!(r.start_line, 4);
                assert_eq!(r.start_col, 0);
                assert_eq!(r.end_col, 5);
            }
            other => panic!("expected shifted, got {other:?}"),
        }
    }

    #[test]
    fn reanchor_not_found_when_quote_gone() {
        let text = "alpha\nbeta\ngamma\n";
        let range = TextRange {
            start_line: 2,
            start_col: 0,
            end_line: 2,
            end_col: 5,
        };
        let outcome = reanchor_text(&range, "world", "", "", text);
        assert_eq!(outcome, ReAnchor::NotFound);
    }

    #[test]
    fn reanchor_prefix_suffix_disambiguates_duplicates() {
        // Two "total" spans; only the one preceded by "grand " is the target.
        let text = "sub total\ngrand total\n";
        let range = TextRange {
            start_line: 1,
            start_col: 6,
            end_line: 1,
            end_col: 11,
        };
        let outcome = reanchor_text(&range, "total", "grand ", "\n", text);
        match outcome {
            ReAnchor::Shifted(r) => {
                assert_eq!(r.start_line, 2);
            }
            other => panic!("expected shifted to the grand-total line, got {other:?}"),
        }
    }

    #[test]
    fn reanchor_ambiguous_duplicates_without_context_is_not_found() {
        // Identical spans, no prefix/suffix to disambiguate — can't safely
        // re-base.
        let text = "total\ntotal\n";
        let range = TextRange {
            start_line: 5,
            start_col: 0,
            end_line: 5,
            end_col: 5,
        };
        let outcome = reanchor_text(&range, "total", "", "", text);
        assert_eq!(outcome, ReAnchor::NotFound);
    }

    #[test]
    fn reanchor_annotation_flips_status_to_shifted_when_gone() {
        let mut anno = text_anno("anno_a", AskSeverity::Should, AnnotationStatus::Open);
        let new_bytes = b"completely different content\n";
        let outcome = reanchor_annotation(&mut anno, new_bytes);
        assert_eq!(outcome, Some(ReAnchor::NotFound));
        assert_eq!(anno.status, AnnotationStatus::Shifted);
        // version_sha was re-pinned.
        assert_eq!(
            anno.artifact.as_ref().unwrap().version_sha,
            hash_bytes(new_bytes)
        );
    }

    #[test]
    fn reanchor_annotation_rebases_range_on_shift() {
        let mut anno = text_anno("anno_a", AskSeverity::Should, AnnotationStatus::Open);
        // Quote "world" now sits on line 3.
        let new_bytes = b"a\nb\nworld\n";
        let outcome = reanchor_annotation(&mut anno, new_bytes);
        assert!(matches!(outcome, Some(ReAnchor::Shifted(_))));
        // Status stays open (re-found, just re-based).
        assert_eq!(anno.status, AnnotationStatus::Open);
        if let Some(Anchor::Text { range, .. }) = &anno.anchor {
            assert_eq!(range.start_line, 3);
        } else {
            panic!("anchor must still be text");
        }
    }

    // ─── Severity aggregation ────────────────────────────────────────────────

    #[test]
    fn count_open_by_severity_tallies_only_open() {
        let annos = vec![
            text_anno("a", AskSeverity::Must, AnnotationStatus::Open),
            text_anno("b", AskSeverity::Must, AnnotationStatus::Open),
            text_anno("c", AskSeverity::Should, AnnotationStatus::Open),
            text_anno("d", AskSeverity::Nit, AnnotationStatus::Open),
            text_anno("e", AskSeverity::Nit, AnnotationStatus::Open),
            text_anno("f", AskSeverity::Nit, AnnotationStatus::Open),
            // Resolved ones don't count.
            text_anno("g", AskSeverity::Must, AnnotationStatus::Addressed),
            text_anno("h", AskSeverity::Should, AnnotationStatus::Dismissed),
        ];
        let counts = count_open_by_severity(&annos);
        assert_eq!(counts.must, 2);
        assert_eq!(counts.should, 1);
        assert_eq!(counts.nit, 3);
        assert_eq!(counts.bar_label().as_deref(), Some("2 blocker · 1 high · 3 nit"));
        assert!(counts.blocks_clean_approve());
    }

    #[test]
    fn nit_only_does_not_block_approve() {
        let annos = vec![
            text_anno("a", AskSeverity::Nit, AnnotationStatus::Open),
            text_anno("b", AskSeverity::Nit, AnnotationStatus::Open),
        ];
        let counts = count_open_by_severity(&annos);
        assert!(!counts.blocks_clean_approve());
        assert_eq!(counts.bar_label().as_deref(), Some("2 nit"));
        assert_eq!(
            checkpoint_button_state(&annos),
            CheckpointButton::ApproveIsPrimary
        );
    }

    #[test]
    fn should_or_must_flips_primary_to_request_changes() {
        let with_should = vec![text_anno("a", AskSeverity::Should, AnnotationStatus::Open)];
        assert_eq!(
            checkpoint_button_state(&with_should),
            CheckpointButton::RequestChangesIsPrimary
        );
        let with_must = vec![text_anno("a", AskSeverity::Must, AnnotationStatus::Open)];
        assert_eq!(
            checkpoint_button_state(&with_must),
            CheckpointButton::RequestChangesIsPrimary
        );
    }

    #[test]
    fn no_open_asks_has_no_bar_label() {
        let annos: Vec<Annotation> = vec![text_anno(
            "a",
            AskSeverity::Must,
            AnnotationStatus::Addressed,
        )];
        let counts = count_open_by_severity(&annos);
        assert_eq!(counts.bar_label(), None);
        assert_eq!(
            checkpoint_button_state(&annos),
            CheckpointButton::ApproveIsPrimary
        );
    }

    #[test]
    fn geometry_helpers_cover_region_scene_and_mark_rect() {
        use darkrun_api::annotation::{Anchor, ImageShape, NormPoint, NormRect, PixelMark};

        // scene_changed: a zero dim is always a change; equal aspect ratio is not;
        // a ratio shift past tolerance is.
        assert!(scene_changed(0, 10, 10, 10, 0.05));
        assert!(!scene_changed(100, 50, 200, 100, 0.05)); // 2:1 scaled — same ratio
        assert!(scene_changed(100, 50, 100, 100, 0.05)); // 2:1 -> 1:1

        // region_out_of_bounds.
        let inb = NormRect { x: 0.1, y: 0.1, w: 0.2, h: 0.2 };
        assert!(!region_out_of_bounds(&inb));
        assert!(region_out_of_bounds(&NormRect { x: -0.5, y: 0.0, w: 0.2, h: 0.2 }));
        assert!(region_out_of_bounds(&NormRect { x: 0.0, y: 0.0, w: 0.0, h: 0.2 }));

        // mark_rect: explicit rect wins, else the arrow's bounding box, else None.
        let mk = |rect, from, to| PixelMark {
            shape: ImageShape::Rect,
            point: None,
            rect,
            arrow_from: from,
            arrow_to: to,
            path: vec![],
            render_w: 0,
            render_h: 0,
        };
        assert_eq!(mark_rect(&mk(Some(inb), None, None)), Some(inb));
        let bbox = mark_rect(&mk(None, Some(NormPoint { x: 0.2, y: 0.3 }), Some(NormPoint { x: 0.5, y: 0.1 }))).unwrap();
        assert!((bbox.x - 0.2).abs() < 1e-9 && (bbox.w - 0.3).abs() < 1e-9);
        assert_eq!(mark_rect(&mk(None, None, None)), None);

        // pixel_region: Pdf returns its rect, Image returns the mark's region,
        // a text anchor has none.
        let mut pdf = text_anno("a", AskSeverity::Must, AnnotationStatus::Open);
        pdf.anchor = Some(Anchor::Pdf { page: 1, rect: inb });
        assert_eq!(pixel_region(&pdf), Some(inb));
        let mut img = text_anno("b", AskSeverity::Must, AnnotationStatus::Open);
        img.anchor = Some(Anchor::Image { mark: mk(Some(inb), None, None) });
        assert_eq!(pixel_region(&img), Some(inb));
        assert_eq!(pixel_region(&text_anno("c", AskSeverity::Must, AnnotationStatus::Open)), None);
    }
}

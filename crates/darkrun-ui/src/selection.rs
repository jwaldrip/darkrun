//! Pure selection + annotation math behind the visual-question, design-direction,
//! and picker views. No Dioxus, no rendering — just the state transitions and the
//! pin-placement geometry the components drive, kept here so they are trivially
//! testable on native and the components stay thin.
//!
//! Three concerns live here:
//! - [`SelectionModel`] — toggling option ids under a single- or multi-select
//!   policy, with a stable, order-preserving selected set.
//! - [`place_pin`] / [`PinPoint`] — converting a click at pixel coordinates over a
//!   preview image into the normalized `0..1` pin coordinate the wire carries
//!   (and back, for rendering).
//! - small helpers ([`is_selected`], [`selected_in_order`]) used by both.

/// The selection policy a question enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Exactly one option may be chosen; picking a new one replaces the old.
    Single,
    /// Any number of options may be chosen; picking toggles membership.
    Multi,
}

impl SelectMode {
    /// Derive the mode from the wire's `multi_select` flag.
    pub fn from_multi(multi: bool) -> Self {
        if multi {
            SelectMode::Multi
        } else {
            SelectMode::Single
        }
    }

    /// Whether this mode permits more than one concurrent selection.
    pub fn is_multi(self) -> bool {
        matches!(self, SelectMode::Multi)
    }
}

/// An order-preserving set of selected option ids under a [`SelectMode`].
///
/// The selected ids are kept in the order they were first chosen (not the order
/// of the option list), so the submitted answer reflects the operator's intent.
/// Re-selecting an already-selected id in [`SelectMode::Single`] is a no-op;
/// in [`SelectMode::Multi`] it deselects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionModel {
    mode: SelectMode,
    selected: Vec<String>,
}

impl SelectionModel {
    /// An empty selection under `mode`.
    pub fn new(mode: SelectMode) -> Self {
        Self { mode, selected: Vec::new() }
    }

    /// Seed a selection from an existing answer (e.g. a re-opened, already
    /// answered session). Ids are de-duplicated; under [`SelectMode::Single`]
    /// only the first survives.
    pub fn from_selected(mode: SelectMode, ids: impl IntoIterator<Item = String>) -> Self {
        let mut model = Self::new(mode);
        for id in ids {
            // Seeding mirrors a fresh pick: single-select keeps only the last,
            // multi-select accumulates uniquely.
            match mode {
                SelectMode::Single => model.selected = vec![id],
                SelectMode::Multi => {
                    if !model.selected.iter().any(|s| s == &id) {
                        model.selected.push(id);
                    }
                }
            }
        }
        model
    }

    /// The active selection mode.
    pub fn mode(&self) -> SelectMode {
        self.mode
    }

    /// The selected ids, in selection order.
    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    /// Whether `id` is currently selected.
    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.iter().any(|s| s == id)
    }

    /// How many options are currently selected.
    pub fn count(&self) -> usize {
        self.selected.len()
    }

    /// Whether any option is selected — the gate on enabling a submit button.
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// Toggle `id` under the active policy, returning the new selection state of
    /// that id (`true` = now selected).
    ///
    /// - Single-select: choosing a new id replaces the current one; choosing the
    ///   already-selected id clears it (so a single-select can be emptied).
    /// - Multi-select: toggles membership, preserving the order of the survivors.
    pub fn toggle(&mut self, id: &str) -> bool {
        let already = self.is_selected(id);
        match self.mode {
            SelectMode::Single => {
                if already {
                    self.selected.clear();
                    false
                } else {
                    self.selected = vec![id.to_string()];
                    true
                }
            }
            SelectMode::Multi => {
                if already {
                    self.selected.retain(|s| s != id);
                    false
                } else {
                    self.selected.push(id.to_string());
                    true
                }
            }
        }
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.selected.clear();
    }
}

/// A normalized pin coordinate in `0..1` over a preview image, paired with its
/// note — exactly the shape the wire carries (`DirectionPin`).
#[derive(Debug, Clone, PartialEq)]
pub struct PinPoint {
    /// X in `0..1`, relative to the preview width.
    pub x: f64,
    /// Y in `0..1`, relative to the preview height.
    pub y: f64,
    /// The note attached to this pin.
    pub note: String,
}

impl PinPoint {
    /// Construct a pin from already-normalized coordinates, clamping into `0..1`.
    pub fn new(x: f64, y: f64, note: impl Into<String>) -> Self {
        Self { x: clamp01(x), y: clamp01(y), note: note.into() }
    }

    /// The pin's left offset as a CSS percentage string (e.g. `"42.5%"`), for
    /// absolute positioning over the preview.
    pub fn left_pct(&self) -> String {
        format!("{:.4}%", self.x * 100.0)
    }

    /// The pin's top offset as a CSS percentage string.
    pub fn top_pct(&self) -> String {
        format!("{:.4}%", self.y * 100.0)
    }
}

/// Convert a click at pixel offset `(px, py)` inside a preview of size
/// `(width, height)` into a normalized [`PinPoint`].
///
/// Coordinates are clamped into the box and divided by the dimensions; a
/// zero-or-negative dimension degrades to `0.0` on that axis rather than
/// producing a NaN/Inf. The note is attached verbatim.
pub fn place_pin(px: f64, py: f64, width: f64, height: f64, note: impl Into<String>) -> PinPoint {
    let x = normalize(px, width);
    let y = normalize(py, height);
    PinPoint::new(x, y, note)
}

/// Normalize a pixel offset along an axis of length `dim` into `0..1`. A
/// non-positive `dim` yields `0.0` (no division by zero / Inf).
fn normalize(offset: f64, dim: f64) -> f64 {
    if dim <= 0.0 || !dim.is_finite() {
        0.0
    } else {
        clamp01(offset / dim)
    }
}

/// Clamp a float into the inclusive `0.0..=1.0` range, mapping NaN to `0.0`.
fn clamp01(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- SelectMode --------------------------------------------------------

    #[test]
    fn mode_from_multi_flag() {
        assert_eq!(SelectMode::from_multi(true), SelectMode::Multi);
        assert_eq!(SelectMode::from_multi(false), SelectMode::Single);
        assert!(SelectMode::Multi.is_multi());
        assert!(!SelectMode::Single.is_multi());
    }

    // --- single-select -----------------------------------------------------

    #[test]
    fn single_select_starts_empty() {
        let m = SelectionModel::new(SelectMode::Single);
        assert!(m.is_empty());
        assert_eq!(m.count(), 0);
        assert!(m.selected().is_empty());
    }

    #[test]
    fn single_select_picks_one() {
        let mut m = SelectionModel::new(SelectMode::Single);
        assert!(m.toggle("a"));
        assert!(m.is_selected("a"));
        assert_eq!(m.selected(), ["a".to_string()]);
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn single_select_replaces_prior_choice() {
        let mut m = SelectionModel::new(SelectMode::Single);
        m.toggle("a");
        assert!(m.toggle("b"));
        assert!(!m.is_selected("a"));
        assert!(m.is_selected("b"));
        assert_eq!(m.selected(), ["b".to_string()]);
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn single_select_toggling_same_clears() {
        let mut m = SelectionModel::new(SelectMode::Single);
        m.toggle("a");
        assert!(!m.toggle("a"));
        assert!(m.is_empty());
        assert!(!m.is_selected("a"));
    }

    // --- multi-select ------------------------------------------------------

    #[test]
    fn multi_select_accumulates_in_order() {
        let mut m = SelectionModel::new(SelectMode::Multi);
        m.toggle("a");
        m.toggle("b");
        m.toggle("c");
        assert_eq!(m.selected(), ["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(m.count(), 3);
    }

    #[test]
    fn multi_select_toggle_off_preserves_remaining_order() {
        let mut m = SelectionModel::new(SelectMode::Multi);
        m.toggle("a");
        m.toggle("b");
        m.toggle("c");
        assert!(!m.toggle("b"));
        assert_eq!(m.selected(), ["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn multi_select_reselect_after_removal_appends_at_end() {
        let mut m = SelectionModel::new(SelectMode::Multi);
        m.toggle("a");
        m.toggle("b");
        m.toggle("a"); // remove a
        m.toggle("a"); // re-add a -> now after b
        assert_eq!(m.selected(), ["b".to_string(), "a".to_string()]);
    }

    #[test]
    fn clear_empties_any_mode() {
        for mode in [SelectMode::Single, SelectMode::Multi] {
            let mut m = SelectionModel::new(mode);
            m.toggle("a");
            m.toggle("b");
            m.clear();
            assert!(m.is_empty());
        }
    }

    // --- seeding -----------------------------------------------------------

    #[test]
    fn from_selected_single_keeps_last() {
        let m = SelectionModel::from_selected(
            SelectMode::Single,
            ["a".to_string(), "b".to_string()],
        );
        assert_eq!(m.selected(), ["b".to_string()]);
    }

    #[test]
    fn from_selected_multi_dedupes_preserving_first_order() {
        let m = SelectionModel::from_selected(
            SelectMode::Multi,
            ["a".to_string(), "b".to_string(), "a".to_string(), "c".to_string()],
        );
        assert_eq!(
            m.selected(),
            ["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn from_selected_empty_is_empty() {
        let m = SelectionModel::from_selected(SelectMode::Multi, Vec::<String>::new());
        assert!(m.is_empty());
    }

    // --- pin placement -----------------------------------------------------

    #[test]
    fn place_pin_normalizes_center() {
        let p = place_pin(50.0, 25.0, 100.0, 50.0, "middle");
        assert!((p.x - 0.5).abs() < 1e-9);
        assert!((p.y - 0.5).abs() < 1e-9);
        assert_eq!(p.note, "middle");
    }

    #[test]
    fn place_pin_clamps_out_of_bounds() {
        let over = place_pin(150.0, -20.0, 100.0, 50.0, "x");
        assert_eq!(over.x, 1.0);
        assert_eq!(over.y, 0.0);
    }

    #[test]
    fn place_pin_handles_zero_dimension() {
        let p = place_pin(10.0, 10.0, 0.0, 0.0, "edge");
        assert_eq!(p.x, 0.0);
        assert_eq!(p.y, 0.0);
        assert!(p.x.is_finite() && p.y.is_finite());
    }

    #[test]
    fn place_pin_handles_nonfinite_dimension() {
        let p = place_pin(10.0, 10.0, f64::NAN, f64::INFINITY, "weird");
        assert_eq!(p.x, 0.0);
        // infinite dim -> offset/inf would be 0, but we guard non-finite as 0 too
        assert_eq!(p.y, 0.0);
    }

    #[test]
    fn pin_point_clamps_on_construction() {
        let p = PinPoint::new(2.0, -1.0, "n");
        assert_eq!(p.x, 1.0);
        assert_eq!(p.y, 0.0);
    }

    #[test]
    fn pin_point_nan_maps_to_zero() {
        let p = PinPoint::new(f64::NAN, f64::NAN, "n");
        assert_eq!(p.x, 0.0);
        assert_eq!(p.y, 0.0);
    }

    #[test]
    fn pin_point_percentages_render() {
        let p = PinPoint::new(0.425, 0.1, "n");
        assert_eq!(p.left_pct(), "42.5000%");
        assert_eq!(p.top_pct(), "10.0000%");
    }

    #[test]
    fn round_trip_pixel_to_pct_is_consistent() {
        // A pin placed at 1/4 width should render at 25%.
        let p = place_pin(40.0, 0.0, 160.0, 90.0, "q");
        assert_eq!(p.left_pct(), "25.0000%");
    }
}

//! Pure data assembly behind the factory browser pages.
//!
//! The `/factories/:factory` and `/factories/:factory/stations/:station` pages
//! read the embedded [`darkrun_content`] corpus and feed it into the
//! `darkrun-ui` factory-browser components ([`StationFlow`], [`PhaseMachine`],
//! [`ExpandableRole`], [`RunWalkthrough`], [`RightSizeStrip`]). That mapping —
//! domain → view-model — is gathered here so it is renderer-free and unit
//! testable: no Dioxus, just structs and functions.
//!
//! Two `CheckpointKind`s exist in the workspace: the content/core one
//! ([`darkrun_core::domain::CheckpointKind`]) and the UI one
//! ([`darkrun_ui::prelude::CheckpointKind`]). [`ui_checkpoint`] is the single
//! boundary that maps between them.

use darkrun_api::session::RunPhase;
use darkrun_core::domain::CheckpointKind as CoreCheckpoint;
use darkrun_ui::prelude::{
    CheckpointKind as UiCheckpoint, FlowStation, Phase as UiPhase, RightSizeTier, RoleKind,
};

use darkrun_content::{Factory, Role, Station};

/// Map the API session phase onto the UI component's [`UiPhase`].
///
/// The two enums mirror each other (no `Tests`; `Reflect` is the 5th phase,
/// before `Checkpoint`). This is the single boundary that bridges them.
pub fn ui_phase(phase: RunPhase) -> UiPhase {
    match phase {
        RunPhase::Spec => UiPhase::Spec,
        RunPhase::Review => UiPhase::Review,
        RunPhase::Manufacture => UiPhase::Manufacture,
        RunPhase::Audit => UiPhase::Audit,
        RunPhase::Reflect => UiPhase::Reflect,
        RunPhase::Checkpoint => UiPhase::Checkpoint,
    }
}

/// Map the content-layer checkpoint kind onto the UI component's enum.
pub fn ui_checkpoint(kind: CoreCheckpoint) -> UiCheckpoint {
    match kind {
        CoreCheckpoint::Auto => UiCheckpoint::Auto,
        CoreCheckpoint::Ask => UiCheckpoint::Ask,
        CoreCheckpoint::External => UiCheckpoint::External,
        CoreCheckpoint::Await => UiCheckpoint::Await,
    }
}

/// Map a content-layer role kind onto the UI component's [`RoleKind`].
pub fn ui_role_kind(kind: darkrun_content::RoleKind) -> RoleKind {
    match kind {
        darkrun_content::RoleKind::Explorer => RoleKind::Explorer,
        darkrun_content::RoleKind::Worker => RoleKind::Worker,
        darkrun_content::RoleKind::Reviewer => RoleKind::Reviewer,
        // A run-level Reflection looks back at the finished Run; closest UI kind
        // is the (non-gating) reviewer.
        darkrun_content::RoleKind::Reflection => RoleKind::Reviewer,
    }
}

/// Title-case a slug for display (`pressure_tester` → `Pressure Tester`).
pub fn humanize(slug: &str) -> String {
    slug.split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Pull the one-line "risk class eliminated" note out of a station body.
///
/// Stations document the risk they kill under a `## Risk class eliminated`
/// heading; the first emphasized phrase (`*Wrong-thing risk.*`) or the first
/// non-empty prose line after it is the chip text. Falls back to `None` so the
/// view can simply omit the chip.
pub fn risk_from_body(body: &str) -> Option<String> {
    let mut lines = body.lines();
    // Find the risk heading.
    let mut found = false;
    for line in lines.by_ref() {
        let l = line.trim();
        if l.starts_with('#') && l.to_lowercase().contains("risk class") {
            found = true;
            break;
        }
    }
    if !found {
        return None;
    }
    // First non-empty prose line under the heading.
    for line in lines {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        return Some(clean_risk_phrase(l));
    }
    None
}

/// Strip markdown emphasis and trailing punctuation from a risk phrase, keeping
/// just the short label (e.g. `*Wrong-thing risk.*` → `wrong-thing risk`).
fn clean_risk_phrase(raw: &str) -> String {
    // Prefer the leading emphasized span if present.
    let candidate = if let Some(rest) = raw.strip_prefix('*') {
        match rest.find('*') {
            Some(end) => &rest[..end],
            None => rest,
        }
    } else {
        // Otherwise take up to the first sentence break.
        raw.split(['.', '—', ':']).next().unwrap_or(raw)
    };
    candidate
        .trim()
        .trim_end_matches(['.', ',', ';', ':'])
        .trim_matches('*')
        .trim()
        .to_lowercase()
}

/// A station rendered as a [`FlowStation`] for the pipeline + walkthrough views.
///
/// The gate is no longer per-station — it's the run's global mode (team/solo/
/// dark) — so the static catalog passes a placeholder kind and renders the strip
/// with `show_checkpoints: false`. The placeholder is never displayed.
pub fn flow_station(station: &Station) -> FlowStation {
    // The slug stays the fixed position name (routing/on_select keys off it);
    // only the DISPLAYED label uses the domain-facing `station.label()`.
    let mut fs = FlowStation::new(station.name().to_string(), UiCheckpoint::Auto)
        .with_label(humanize(station.label()));
    if let Some(risk) = risk_from_body(&station.body) {
        fs = fs.with_risk(risk);
    }
    fs
}

/// Build the [`FlowStation`] list for a whole factory, in pipeline order.
pub fn flow_stations(factory: &Factory) -> Vec<FlowStation> {
    factory.stations.iter().map(flow_station).collect()
}

/// A single role's view-model: everything an `ExpandableRole` card needs, plus
/// the pre-rendered markdown HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleView {
    /// Role slug, humanized for display.
    pub name: String,
    /// The UI role-kind badge.
    pub kind: RoleKind,
    /// The agent_type string shown as a chip.
    pub agent_type: String,
    /// Optional model override.
    pub model: Option<String>,
    /// One-line summary (first prose sentence of the body).
    pub summary: Option<String>,
    /// Pre-rendered markdown HTML of the full instruction body.
    pub body_html: String,
}

/// Lift the first meaningful prose line out of a markdown body as a summary.
pub fn summary_from_body(body: &str) -> Option<String> {
    for line in body.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("---") {
            continue;
        }
        // Strip basic markdown emphasis markers for a clean one-liner.
        let cleaned: String = l.replace(['*', '`', '_'], "");
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        // Cap to the first sentence so the collapsed summary stays one line.
        let first = cleaned.split_inclusive(['.', '!', '?']).next().unwrap_or(cleaned);
        return Some(first.trim().to_string());
    }
    None
}

/// Assemble a [`RoleView`] from a content [`Role`], rendering its body with the
/// supplied markdown renderer (the site's `content::render_markdown`).
pub fn role_view(role: &Role, render: impl Fn(&str) -> String) -> RoleView {
    RoleView {
        name: humanize(role.name()),
        kind: ui_role_kind(role.kind()),
        agent_type: role_kind_label(role.kind()).to_string(),
        model: role.frontmatter.model.clone(),
        summary: summary_from_body(&role.body),
        body_html: render(&role.body),
    }
}

/// The lowercase label for a content role kind (used as the agent_type chip).
pub fn role_kind_label(kind: darkrun_content::RoleKind) -> &'static str {
    match kind {
        darkrun_content::RoleKind::Explorer => "explorer",
        darkrun_content::RoleKind::Worker => "worker",
        darkrun_content::RoleKind::Reviewer => "reviewer",
        darkrun_content::RoleKind::Reflection => "reflection",
    }
}

/// Right-sizing tiers for a factory: how small runs collapse stations.
///
/// The corpus documents that a one-line fix can drop straight to `build → prove`
/// and that mid-size work skips the heaviest framing. These tiers are derived
/// from the pipeline so the [`RightSizeStrip`] reflects the real station set:
/// `tiny` keeps the build/prove core, `small` adds specify + harden, `full`
/// keeps everything.
pub fn right_size_tiers(factory: &Factory) -> Vec<RightSizeTier> {
    let all: Vec<String> = factory.stations.iter().map(|s| s.name().to_string()).collect();
    let keep = |wanted: &[&str]| -> Vec<String> {
        all.iter().filter(|s| wanted.contains(&s.as_str())).cloned().collect()
    };
    vec![
        RightSizeTier::new("tiny", keep(&["build", "prove"])),
        RightSizeTier::new("small", keep(&["specify", "build", "prove", "harden"])),
        RightSizeTier::new("full", all),
    ]
}

/// The slugs that make up the factory pipeline, in order.
pub fn pipeline_slugs(factory: &Factory) -> Vec<String> {
    factory.stations.iter().map(|s| s.name().to_string()).collect()
}

/// Locate a station's index within the pipeline (for prev/next nav).
pub fn station_index(factory: &Factory, station_slug: &str) -> Option<usize> {
    factory.stations.iter().position(|s| s.name() == station_slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn software() -> Factory {
        darkrun_content::load_validated("software").expect("software factory")
    }

    #[test]
    fn humanize_title_cases_slugs() {
        assert_eq!(humanize("pressure_tester"), "Pressure Tester");
        assert_eq!(humanize("frame"), "Frame");
        assert_eq!(humanize("red-teamer"), "Red Teamer");
        assert_eq!(humanize(""), "");
    }

    #[test]
    fn ui_phase_maps_every_run_phase() {
        assert_eq!(ui_phase(RunPhase::Spec), UiPhase::Spec);
        assert_eq!(ui_phase(RunPhase::Review), UiPhase::Review);
        assert_eq!(ui_phase(RunPhase::Manufacture), UiPhase::Manufacture);
        assert_eq!(ui_phase(RunPhase::Audit), UiPhase::Audit);
        assert_eq!(ui_phase(RunPhase::Reflect), UiPhase::Reflect);
        assert_eq!(ui_phase(RunPhase::Checkpoint), UiPhase::Checkpoint);
    }

    #[test]
    fn ui_phase_preserves_canonical_order() {
        // The API and UI phase orders must stay in lockstep (no Tests; Reflect
        // is the 5th, before Checkpoint).
        let api = [
            RunPhase::Spec,
            RunPhase::Review,
            RunPhase::Manufacture,
            RunPhase::Audit,
            RunPhase::Reflect,
            RunPhase::Checkpoint,
        ];
        let mapped: Vec<UiPhase> = api.iter().map(|p| ui_phase(*p)).collect();
        assert_eq!(mapped, UiPhase::ALL.to_vec());
    }

    #[test]
    fn ui_checkpoint_maps_every_kind() {
        assert_eq!(ui_checkpoint(CoreCheckpoint::Auto), UiCheckpoint::Auto);
        assert_eq!(ui_checkpoint(CoreCheckpoint::Ask), UiCheckpoint::Ask);
        assert_eq!(ui_checkpoint(CoreCheckpoint::External), UiCheckpoint::External);
        assert_eq!(ui_checkpoint(CoreCheckpoint::Await), UiCheckpoint::Await);
    }

    #[test]
    fn risk_extracted_from_frame_body() {
        let f = software();
        let frame = f.station("frame").unwrap();
        let risk = risk_from_body(&frame.body).expect("frame documents a risk");
        assert!(risk.contains("wrong"), "got {risk:?}");
        // No markdown emphasis leaks through.
        assert!(!risk.contains('*'));
    }

    #[test]
    fn every_station_yields_a_flow_station() {
        let f = software();
        let flows = flow_stations(&f);
        assert_eq!(flows.len(), f.stations.len());
        for (fs, st) in flows.iter().zip(&f.stations) {
            assert_eq!(fs.slug, st.name());
            assert!(!fs.label.is_empty());
        }
    }

    #[test]
    fn flow_stations_carry_risk_for_known_stations() {
        let f = software();
        let flows = flow_stations(&f);
        // Every software station documents a risk class.
        assert!(flows.iter().all(|fs| fs.risk.is_some()), "missing risk on a station");
    }

    #[test]
    fn role_view_renders_real_markdown_body() {
        let f = software();
        let frame = f.station("frame").unwrap();
        let framer = &frame.workers[0];
        let view = role_view(framer, crate::content::render_markdown);
        assert_eq!(view.kind, RoleKind::Worker);
        assert_eq!(view.agent_type, "worker");
        assert!(view.body_html.contains('<'), "body should be rendered HTML");
        assert!(view.summary.is_some());
    }

    #[test]
    fn summary_skips_headings_and_frontmatter() {
        let body = "# Title\n\nThe framer drafts the problem. More text.";
        let s = summary_from_body(body).unwrap();
        assert!(s.starts_with("The framer"));
        assert!(s.ends_with('.'));
        assert!(!s.contains('\n'));
    }

    #[test]
    fn right_size_tiers_collapse_toward_build_prove() {
        let f = software();
        let tiers = right_size_tiers(&f);
        assert_eq!(tiers.len(), 3);
        let tiny = &tiers[0];
        assert_eq!(tiny.label, "tiny");
        assert!(tiny.kept.contains(&"build".to_string()));
        assert!(tiny.kept.contains(&"prove".to_string()));
        assert!(!tiny.kept.contains(&"frame".to_string()));
        // Full keeps everything.
        assert_eq!(tiers[2].kept.len(), f.stations.len());
    }

    #[test]
    fn station_index_finds_and_misses() {
        let f = software();
        assert_eq!(station_index(&f, "frame"), Some(0));
        assert_eq!(station_index(&f, "harden"), Some(5));
        assert_eq!(station_index(&f, "nope"), None);
    }

    #[test]
    fn pipeline_slugs_are_in_order() {
        let f = software();
        assert_eq!(
            pipeline_slugs(&f),
            vec!["frame", "specify", "shape", "build", "prove", "harden"]
        );
    }

    #[test]
    fn reflection_role_kind_maps_and_labels() {
        use darkrun_content::RoleKind;
        // Reflection folds into the Reviewer UI kind + its own label.
        let _ = ui_role_kind(RoleKind::Reflection);
        assert_eq!(role_kind_label(RoleKind::Reflection), "reflection");
    }

    #[test]
    fn risk_and_summary_extraction_edge_cases() {
        // No risk heading → None.
        assert!(risk_from_body("just prose, no heading\n").is_none());
        // Heading present but nothing after it → None.
        assert!(risk_from_body("## Risk class eliminated\n").is_none());
        // An emphasized phrase with no closing `*` keeps the remainder.
        let r1 = risk_from_body("## Risk class eliminated\n\n*unclosed phrase\n").unwrap();
        assert_eq!(r1, "unclosed phrase");
        // A plain phrase is cut at the first sentence break.
        let r2 = risk_from_body("## Risk class eliminated\n\nWrong-thing risk. More detail.\n").unwrap();
        assert_eq!(r2, "wrong-thing risk");

        // summary_from_body skips all-emphasis lines and returns None when there's
        // no prose at all.
        assert_eq!(summary_from_body("***\n\nReal first line. trailing\n").as_deref(), Some("Real first line."));
        assert!(summary_from_body("# Heading\n\n---\n").is_none());
    }
}

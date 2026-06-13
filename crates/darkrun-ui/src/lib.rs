//! darkrun-ui — darkrun's shared design system.
//!
//! One crate, two consumers: the Dioxus desktop app and the website. It carries
//! the **dark-only** design tokens (near-black surfaces, a single cool-cyan
//! accent, the six phase hues) and a set of Dioxus components built on top of
//! them. The crate is renderer-agnostic: it depends on Dioxus' macro/html/hooks
//! surface only, so it compiles for both native and `wasm32-unknown-unknown`.
//!
//! ## Layout
//!
//! - [`tokens`] — color/type/spacing constants plus the [`tokens::THEME_CSS`]
//!   custom-property block. Source of truth for both Rust styling and CSS.
//! - [`kinds`] — small `Copy` enums ([`kinds::Phase`], [`kinds::Tone`],
//!   [`kinds::Step`]) shared across components. No `darkrun-core` dependency.
//! - [`components`] — the primitives ([`Wordmark`](components::wordmark::Wordmark),
//!   [`Card`](components::primitives::Card), [`Badge`](components::primitives::Badge),
//!   [`Button`](components::primitives::Button)) plus the factory-browser set:
//!   [`StationFlow`](components::station_flow::StationFlow) (interactive SVG
//!   station pipeline), [`PhaseMachine`](components::phase_machine::PhaseMachine)
//!   (the within-station six-phase ring),
//!   [`ExpandableRole`](components::role::ExpandableRole)/[`ArtifactCard`](components::role::ArtifactCard)
//!   (markdown drill-down cards), [`RunWalkthrough`](components::walkthrough::RunWalkthrough)
//!   (the run stepper), and the small chips
//!   [`CheckpointBadge`](components::chips::CheckpointBadge),
//!   [`RiskChip`](components::chips::RiskChip),
//!   [`RightSizeStrip`](components::chips::RightSizeStrip).
//! - [`flow`] — the pure layout + semantics behind those visualizations: the
//!   station-pipeline placement, the phase-ring geometry, the phase-machine beat
//!   mapping, and the walkthrough step sequencing. Testable without a renderer.
//! - [`graph`] — the SVG unit-DAG visualization with a pluggable
//!   [`graph::layout::GraphLayout`] (default layered/Sugiyama-ish placement).
//!
//! ## Usage
//!
//! ```ignore
//! use darkrun_ui::prelude::*;
//!
//! fn app() -> Element {
//!     rsx! {
//!         style { "{darkrun_ui::tokens::THEME_CSS}" }
//!         Wordmark { variant: WordmarkVariant::Filled, size: 32.0 }
//!         FactoryCard {
//!             title: "Ship the importer".to_string(),
//!             factory: "software-factory".to_string(),
//!             station: Some("build".to_string()),
//!             phase: Some(Phase::Manufacture),
//!         }
//!     }
//! }
//! ```

pub mod components;
pub mod flow;
pub mod graph;
pub mod kinds;
pub mod markdown;
pub mod selection;
pub mod theme;
pub mod tokens;
pub mod view;

/// The recommended glob import for consumers: every public component, the shared
/// kinds, and the graph types, plus Dioxus' own prelude.
pub mod prelude {
    pub use dioxus::prelude::*;

    pub use crate::components::chips::{
        CheckpointBadge, RightSizeStrip, RightSizeTier, RiskChip,
    };
    pub use crate::components::annotate::{
        AnnotateTool, AnnotateToolbar, ArrowMarker, BoxMarker, CommentDraft,
        CommentPanel, HighlightMarker, PathMarker, PinMarker, SurfaceKind,
        ThreadComment,
    };
    pub use crate::components::feedback::{
        counts_by_severity, feedback_inbox, feedback_row, FeedbackAction,
        FeedbackEntry, Severity,
    };
    pub use crate::components::factory::{
        CheckpointBar, CheckpointKind, FactoryCard, UnitRow,
    };
    pub use crate::components::output_review::OutputReview;
    pub use crate::components::phase_machine::PhaseMachine;
    pub use crate::components::proof_panel::{
        AuditRow, BenchStat, ProofMetricKind, ProofPanel, ProofView, VitalMetric,
    };
    pub use crate::components::pipeline::{strip_for, PhaseDot, StationPipeline};
    pub use crate::components::primitives::{Badge, Button, ButtonVariant, Card};
    pub use crate::components::role::{ArtifactCard, ExpandableRole, RoleKind};
    pub use crate::components::run_list::{run_status_tone, RunCard, RunCardData, RunList};
    pub use crate::components::session_views::{
        ArchetypeCard, DirectionView, OptionCard, PickerItem, PickerView, QuestionView,
    };
    pub use crate::components::station_flow::StationFlow;
    pub use crate::components::station_strip::{
        strip_from, StationItem, StationStatus, StationStrip,
    };
    pub use crate::components::tab_bar::{TabBar, TabItem};
    pub use crate::components::view_artifacts::{ArtifactEntry, ViewArtifacts};
    pub use crate::components::walkthrough::RunWalkthrough;
    pub use crate::components::wordmark::{Wordmark, WordmarkVariant};
    pub use crate::flow::{
        layout_flow, phase_beat, phase_beats, phase_label, phase_ring_points,
        station_hue, walkthrough_steps, checkpoint_hue, Beat, FlowConnector,
        FlowLayout, FlowOptions, FlowStation, PassBeat, PlacedStation, WalkStep,
        TICKS_PER_STATION,
    };
    pub use crate::graph::layout::{
        GraphEdge, GraphLayout, GraphNode, LayeredLayout, LayoutOptions,
        LayoutResult, PlacedEdge, PlacedNode,
    };
    pub use crate::graph::view::{UnitGraph, UnitGraphNode};
    pub use crate::kinds::{Phase, Step, Tone};
    pub use crate::selection::{
        place_box, place_pin, NormBox, PinPoint, SelectMode, SelectionModel,
        VisualMark,
    };
    pub use crate::theme::{apply_script, ThemeChoice};
    pub use crate::tokens;
    pub use crate::view::{
        classify_vital, format_latency_ms, format_samples, format_throughput, format_vital,
        vital_label, ArtifactKind, VitalVerdict,
    };
}

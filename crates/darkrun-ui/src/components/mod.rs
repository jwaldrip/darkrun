//! The darkrun component library.
//!
//! Components are inline-styled against the dark theme tokens so they render the
//! same on native (WebView) and wasm (browser) with no external stylesheet
//! beyond the optional [`crate::tokens::THEME_CSS`] custom-property block.

pub mod annotate;
pub mod chips;
pub mod factory;
pub mod feedback;
pub mod output_review;
pub mod phase_machine;
pub mod pipeline;
pub mod primitives;
pub mod proof_panel;
pub mod role;
pub mod run_list;
pub mod session_views;
pub mod station_flow;
pub mod station_strip;
pub mod tab_bar;
pub mod view_artifacts;
pub mod walkthrough;
pub mod wordmark;

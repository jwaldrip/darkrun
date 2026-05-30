//! The darkrun component library.
//!
//! Components are inline-styled against the dark theme tokens so they render the
//! same on native (WebView) and wasm (browser) with no external stylesheet
//! beyond the optional [`crate::tokens::THEME_CSS`] custom-property block.

pub mod chips;
pub mod factory;
pub mod phase_machine;
pub mod pipeline;
pub mod primitives;
pub mod role;
pub mod station_flow;
pub mod walkthrough;
pub mod wordmark;

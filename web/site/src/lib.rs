//! darkrun-site — the darkrun.ai website as a Dioxus single-page app.
//!
//! Dark by default, light via `prefers-color-scheme`, with a System/Light/Dark
//! override in the header ([`theme_toggle`]); built on the shared [`darkrun_ui`]
//! design system. The route
//! table in [`route::Route`] drives both the in-browser router and the static
//! SEO generators in [`seo`]. Real content comes from three sources:
//!
//! - **`/factories`** renders the embedded factory corpus from `darkrun-content`.
//! - **`/docs`** and the concept pages render embedded markdown via
//!   `pulldown-cmark`.
//! - **`/browse`** and **`/review`** explain that review is a *local* surface:
//!   it runs in the darkrun desktop app over loopback and never takes over the
//!   browser (remote / web review is a later thing). They reference the real
//!   `darkrun-api` contract rather than connecting to an engine.
//!
//! The crate compiles to `wasm32-unknown-unknown` (the shipped target) and the
//! native host (for the static-site generator).

pub mod auth;
pub mod content;
pub mod factory_view;
pub mod history;
pub mod layout;
pub mod pages;
pub mod remote;
pub mod route;
pub mod search;
pub mod seo;
pub mod theme_toggle;
pub mod ui;

use darkrun_ui::prelude::*;

use crate::route::Route;

/// The website root component: mounts the router over the [`Route`] table. The
/// [`layout::Shell`] layout provides the chrome around every page.
#[component]
pub fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}

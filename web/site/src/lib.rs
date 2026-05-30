//! darkrun-site — the darkrun.ai website as a Dioxus single-page app.
//!
//! Dark-theme only, built on the shared [`darkrun_ui`] design system. The route
//! table in [`route::Route`] drives both the in-browser router and the static
//! SEO generators in [`seo`]. Real content comes from three sources:
//!
//! - **`/factories`** renders the embedded factory corpus from `darkrun-content`.
//! - **`/docs`** and the concept pages render embedded markdown via
//!   `pulldown-cmark`.
//! - **`/browse`** and **`/review`** are scaffolds wired to the real
//!   `darkrun-api` wire types, pending a live engine connection.
//!
//! The crate compiles to `wasm32-unknown-unknown` (the shipped target) and the
//! native host (for the static-site generator).

pub mod content;
pub mod factory_view;
pub mod layout;
pub mod pages;
pub mod route;
pub mod seo;
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

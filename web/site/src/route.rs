//! The site's route table.
//!
//! A single `Routable` enum is the source of truth for both the in-browser
//! router and the static-site generator (which walks [`Route::ALL`] to know what
//! to pre-render and list in the sitemap).

use darkrun_ui::prelude::*;

use crate::layout::Shell;
use crate::pages;

/// Every navigable route on darkrun.ai.
///
/// The `Shell` layout wraps every page with the header, nav, and footer. Nested
/// routes (`/docs/:slug`, `/factories/:slug`, …) render a detail page; the bare
/// section route renders its index.
#[derive(Routable, Clone, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
        #[route("/")]
        Landing {},

        #[route("/factories")]
        Factories {},
        #[route("/factories/:slug")]
        FactoryDetail { slug: String },
        #[route("/factories/:factory/stations/:station")]
        StationDetail { factory: String, station: String },

        #[route("/docs")]
        Docs {},
        #[route("/docs/:slug")]
        DocPage { slug: String },

        #[route("/methodology")]
        Methodology {},
        #[route("/methodology/:phase")]
        PhaseDetail { phase: String },
        #[route("/glossary")]
        Glossary {},
        #[route("/lifecycles")]
        Lifecycles {},

        #[route("/blog")]
        Blog {},
        #[route("/blog/:slug")]
        Post { slug: String },

        #[route("/changelog")]
        Changelog {},
        #[route("/paper")]
        Paper {},
        #[route("/templates")]
        Templates {},

        #[route("/browse")]
        Browse {},
        #[route("/review")]
        Review {},
        #[route("/preview")]
        Preview {},

        #[route("/privacy")]
        Privacy {},
        #[route("/terms")]
        Terms {},

        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
}

// Re-export the page components under the names the route attributes expect.
pub use pages::blog::{Blog, Post};
pub use pages::browse::Browse;
pub use pages::changelog::Changelog;
pub use pages::concepts::{Glossary, Lifecycles, Methodology, PhaseDetail};
pub use pages::docs::{DocPage, Docs};
pub use pages::factories::{FactoryDetail, Factories, StationDetail};
pub use pages::landing::Landing;
pub use pages::legal::{Privacy, Terms};
pub use pages::misc::{NotFound, Paper, Templates};
pub use pages::preview::Preview;
pub use pages::review::Review;

impl Route {
    /// Every concrete, generator-renderable URL path on the site, in nav order.
    ///
    /// Dynamic routes are expanded from the embedded corpora so the static-site
    /// generator and the sitemap cover real pages, not just the templates.
    pub fn all_paths() -> Vec<String> {
        let mut paths = vec![
            "/".to_string(),
            "/factories".to_string(),
            "/docs".to_string(),
            "/methodology".to_string(),
            "/glossary".to_string(),
            "/lifecycles".to_string(),
            "/blog".to_string(),
            "/changelog".to_string(),
            "/paper".to_string(),
            "/templates".to_string(),
            "/browse".to_string(),
            "/review".to_string(),
            "/preview".to_string(),
            "/privacy".to_string(),
            "/terms".to_string(),
        ];
        for slug in darkrun_content::list_factories() {
            paths.push(format!("/factories/{slug}"));
            // Each station drills down to its own deep page.
            if let Ok(factory) = darkrun_content::load_validated(&slug) {
                for station in &factory.stations {
                    paths.push(format!("/factories/{slug}/stations/{}", station.name()));
                }
            }
        }
        // One explainer page per phase under /methodology.
        for phase in crate::pages::concepts::PHASE_SLUGS {
            paths.push(format!("/methodology/{phase}"));
        }
        for doc in crate::content::DOCS {
            paths.push(format!("/docs/{}", doc.slug));
        }
        for post in crate::content::POSTS {
            paths.push(format!("/blog/{}", post.slug));
        }
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_paths_covers_the_static_sections() {
        let paths = Route::all_paths();
        for expected in [
            "/", "/factories", "/docs", "/methodology", "/glossary", "/lifecycles", "/blog",
            "/changelog", "/paper", "/templates", "/browse", "/review", "/preview", "/privacy",
            "/terms",
        ] {
            assert!(paths.iter().any(|p| p == expected), "missing route {expected}");
        }
    }

    #[test]
    fn all_paths_expands_dynamic_routes_from_the_corpora() {
        let paths = Route::all_paths();
        // At least one factory, one doc, and one post are expanded into URLs.
        assert!(paths.iter().any(|p| p.starts_with("/factories/")));
        assert!(paths.iter().any(|p| p.starts_with("/docs/")));
        assert!(paths.iter().any(|p| p.starts_with("/blog/")));
    }

    #[test]
    fn all_paths_expands_station_and_phase_routes() {
        let paths = Route::all_paths();
        // Every station of every factory has a deep page.
        for slug in darkrun_content::list_factories() {
            let factory = darkrun_content::load_validated(&slug).expect("load");
            for station in &factory.stations {
                let want = format!("/factories/{slug}/stations/{}", station.name());
                assert!(paths.iter().any(|p| p == &want), "missing {want}");
            }
        }
        // One explainer page per phase.
        for phase in crate::pages::concepts::PHASE_SLUGS {
            let want = format!("/methodology/{phase}");
            assert!(paths.iter().any(|p| p == &want), "missing {want}");
        }
    }

    #[test]
    fn all_paths_are_unique_and_rooted() {
        let mut paths = Route::all_paths();
        assert!(paths.iter().all(|p| p.starts_with('/')), "every path is absolute");
        let len = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(len, paths.len(), "route paths must be unique");
    }
}

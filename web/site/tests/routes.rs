//! Integration tests for `Route::all_paths` — the source of truth for the
//! sitemap and the static-site generator's route manifest.

use darkrun_site::route::Route;

#[test]
fn includes_the_landing_root() {
    assert!(Route::all_paths().iter().any(|p| p == "/"));
}

#[test]
fn covers_every_static_section() {
    let paths = Route::all_paths();
    for expected in [
        "/factories",
        "/docs",
        "/methodology",
        "/glossary",
        "/lifecycles",
        "/blog",
        "/changelog",
        "/paper",
        "/templates",
        "/browse",
        "/review",
        "/privacy",
        "/terms",
    ] {
        assert!(paths.iter().any(|p| p == expected), "missing {expected}");
    }
}

#[test]
fn every_path_is_absolute() {
    assert!(Route::all_paths().iter().all(|p| p.starts_with('/')));
}

#[test]
fn no_path_has_a_trailing_slash_except_root() {
    for p in Route::all_paths() {
        if p == "/" {
            continue;
        }
        assert!(!p.ends_with('/'), "trailing slash on {p}");
    }
}

#[test]
fn paths_are_unique() {
    let mut paths = Route::all_paths();
    let len = paths.len();
    paths.sort();
    paths.dedup();
    assert_eq!(len, paths.len(), "duplicate route path");
}

#[test]
fn expands_one_factory_url_per_embedded_factory() {
    let paths = Route::all_paths();
    for slug in darkrun_content::list_factories() {
        assert!(paths.iter().any(|p| p == &format!("/factories/{slug}")), "missing /factories/{slug}");
    }
}

#[test]
fn expands_one_doc_url_per_doc() {
    let paths = Route::all_paths();
    for doc in darkrun_site::content::DOCS {
        assert!(paths.iter().any(|p| p == &format!("/docs/{}", doc.slug)));
    }
}

#[test]
fn expands_one_post_url_per_post() {
    let paths = Route::all_paths();
    for post in darkrun_site::content::POSTS {
        assert!(paths.iter().any(|p| p == &format!("/blog/{}", post.slug)));
    }
}

#[test]
fn dynamic_count_matches_corpora_totals() {
    let paths = Route::all_paths();
    // A station path contains `/stations/`; a bare factory path does not.
    let factories = paths
        .iter()
        .filter(|p| p.starts_with("/factories/") && !p.contains("/stations/"))
        .count();
    let docs = paths.iter().filter(|p| p.starts_with("/docs/")).count();
    let posts = paths.iter().filter(|p| p.starts_with("/blog/")).count();
    assert_eq!(factories, darkrun_content::list_factories().len());
    assert_eq!(docs, darkrun_site::content::DOCS.len());
    assert_eq!(posts, darkrun_site::content::POSTS.len());
}

#[test]
fn station_count_matches_every_factory_station() {
    let paths = Route::all_paths();
    let stations = paths.iter().filter(|p| p.contains("/stations/")).count();
    let expected: usize = darkrun_content::list_factories()
        .iter()
        .map(|slug| darkrun_content::load_validated(slug).map(|f| f.stations.len()).unwrap_or(0))
        .sum();
    assert_eq!(stations, expected);
}

#[test]
fn phase_count_is_six() {
    let paths = Route::all_paths();
    let phases = paths.iter().filter(|p| p.starts_with("/methodology/")).count();
    assert_eq!(phases, 6);
}

#[test]
fn total_is_static_sections_plus_dynamic() {
    let paths = Route::all_paths();
    let static_count = 20; // "/" + 5 guides + 14 sections (includes the /preview fixture)
    let stations: usize = darkrun_content::list_factories()
        .iter()
        .map(|slug| darkrun_content::load_validated(slug).map(|f| f.stations.len()).unwrap_or(0))
        .sum();
    let phases = 6;
    let dynamic = darkrun_content::list_factories().len()
        + stations
        + phases
        + darkrun_site::content::DOCS.len()
        + darkrun_site::content::POSTS.len();
    assert_eq!(paths.len(), static_count + dynamic);
}

#[test]
fn static_sections_come_before_dynamic_expansions() {
    // The first dynamic path is a factory/doc/post; the static block precedes it.
    let paths = Route::all_paths();
    let first_dynamic = paths
        .iter()
        .position(|p| p.starts_with("/factories/") || p.starts_with("/docs/") || p.starts_with("/blog/"))
        .expect("at least one dynamic route");
    for p in &paths[..first_dynamic] {
        assert!(
            !(p.starts_with("/factories/") || p.starts_with("/docs/") || p.starts_with("/blog/")),
            "dynamic path {p} before the static block"
        );
    }
}

#[test]
fn is_deterministic_across_calls() {
    assert_eq!(Route::all_paths(), Route::all_paths());
}

#[test]
fn no_path_contains_a_slug_placeholder() {
    // Dynamic routes must be expanded, never left as `:slug` templates.
    for p in Route::all_paths() {
        assert!(!p.contains(':'), "unexpanded template {p}");
        assert!(!p.contains("{"), "unexpanded template {p}");
    }
}

#[test]
fn no_path_has_double_slashes() {
    for p in Route::all_paths() {
        assert!(!p.contains("//"), "double slash in {p}");
    }
}

#[test]
fn nav_order_places_factories_before_docs_before_blog() {
    let paths = Route::all_paths();
    let pos = |needle: &str| paths.iter().position(|p| p == needle).unwrap();
    assert!(pos("/factories") < pos("/docs"));
    assert!(pos("/docs") < pos("/blog"));
}

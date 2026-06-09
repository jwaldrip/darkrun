//! Integration tests for the embedded content corpus in `darkrun_site::content`:
//! docs, concept pages, and blog posts — their metadata, slugs, ordering, and
//! lookup behavior.

use darkrun_site::content::{find, CONCEPTS, DOCS, POSTS};

/// All three corpora, for table-driven checks.
fn corpora() -> [(&'static str, &'static [darkrun_site::content::Doc]); 3] {
    [("docs", DOCS), ("concepts", CONCEPTS), ("posts", POSTS)]
}

#[test]
fn every_corpus_is_non_empty() {
    for (name, corpus) in corpora() {
        assert!(!corpus.is_empty(), "{name} is empty");
    }
}

#[test]
fn docs_carry_the_expected_slugs_in_order() {
    let slugs: Vec<&str> = DOCS.iter().map(|d| d.slug).collect();
    assert_eq!(
        slugs,
        vec![
            "getting-started",
            "stations",
            "review",
            "tools-and-commands",
            "other-harnesses"
        ]
    );
}

#[test]
fn concepts_carry_the_expected_slugs_in_order() {
    let slugs: Vec<&str> = CONCEPTS.iter().map(|d| d.slug).collect();
    assert_eq!(slugs, vec!["methodology", "glossary", "lifecycles"]);
}

#[test]
fn posts_carry_the_expected_slugs_newest_first() {
    let slugs: Vec<&str> = POSTS.iter().map(|d| d.slug).collect();
    assert_eq!(
        slugs,
        vec![
            "darkrun-is-a-harness",
            "pure-rust-c-free",
            "team-solo-dark",
            "the-dark-factory",
            "checkpoints-not-babysitting",
        ]
    );
}

#[test]
fn every_doc_has_non_empty_metadata() {
    for (name, corpus) in corpora() {
        for d in corpus {
            assert!(!d.slug.is_empty(), "{name} has an empty slug");
            assert!(!d.title.is_empty(), "{name}/{} has empty title", d.slug);
            assert!(!d.summary.is_empty(), "{name}/{} has empty summary", d.slug);
            assert!(!d.markdown.is_empty(), "{name}/{} has empty markdown", d.slug);
        }
    }
}

#[test]
fn slugs_are_url_safe() {
    for (_, corpus) in corpora() {
        for d in corpus {
            assert!(
                d.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "non-url-safe slug: {}",
                d.slug
            );
            assert!(!d.slug.starts_with('-') && !d.slug.ends_with('-'), "slug edge dash: {}", d.slug);
        }
    }
}

#[test]
fn slugs_are_unique_within_each_corpus() {
    for (name, corpus) in corpora() {
        let mut slugs: Vec<&str> = corpus.iter().map(|d| d.slug).collect();
        let len = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(len, slugs.len(), "duplicate slug in {name}");
    }
}

#[test]
fn every_markdown_body_starts_with_a_level_one_heading() {
    for (name, corpus) in corpora() {
        for d in corpus {
            assert!(
                d.markdown.trim_start().starts_with("# "),
                "{name}/{} missing leading h1",
                d.slug
            );
        }
    }
}

#[test]
fn find_resolves_known_doc_slugs() {
    assert_eq!(find(DOCS, "getting-started").map(|d| d.slug), Some("getting-started"));
    assert_eq!(find(DOCS, "stations").map(|d| d.slug), Some("stations"));
    assert_eq!(find(DOCS, "review").map(|d| d.slug), Some("review"));
}

#[test]
fn find_resolves_known_concept_slugs() {
    assert!(find(CONCEPTS, "methodology").is_some());
    assert!(find(CONCEPTS, "glossary").is_some());
    assert!(find(CONCEPTS, "lifecycles").is_some());
}

#[test]
fn find_resolves_known_post_slugs() {
    assert!(find(POSTS, "the-dark-factory").is_some());
    assert!(find(POSTS, "checkpoints-not-babysitting").is_some());
}

#[test]
fn find_returns_none_for_unknown_slug() {
    assert!(find(DOCS, "does-not-exist").is_none());
    assert!(find(CONCEPTS, "does-not-exist").is_none());
    assert!(find(POSTS, "does-not-exist").is_none());
}

#[test]
fn find_is_corpus_scoped() {
    // A post slug must not resolve inside the docs corpus and vice versa.
    assert!(find(DOCS, "the-dark-factory").is_none());
    assert!(find(POSTS, "getting-started").is_none());
    assert!(find(CONCEPTS, "getting-started").is_none());
}

#[test]
fn find_is_case_sensitive() {
    assert!(find(DOCS, "Getting-Started").is_none());
    assert!(find(DOCS, "GETTING-STARTED").is_none());
}

#[test]
fn find_rejects_empty_slug() {
    assert!(find(DOCS, "").is_none());
}

#[test]
fn find_returns_a_reference_into_the_corpus() {
    let d = find(DOCS, "getting-started").unwrap();
    // The returned reference is the same one held in the slice.
    assert_eq!(d as *const _, &DOCS[0] as *const _);
}

#[test]
fn returned_doc_matches_its_listed_title() {
    let d = find(DOCS, "getting-started").unwrap();
    assert_eq!(d.title, "Getting started");
}

#[test]
fn no_slug_collides_across_docs_and_concepts() {
    // docs and concepts share the /docs-style detail surface conceptually; their
    // slugs happen to be distinct, which keeps routing unambiguous.
    for d in DOCS {
        assert!(find(CONCEPTS, d.slug).is_none(), "slug {} appears in both", d.slug);
    }
}

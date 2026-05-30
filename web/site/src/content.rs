//! Site content: the embedded markdown corpus and its rendering.
//!
//! The website ships its own prose — docs, concept pages, and blog posts — as
//! markdown embedded at compile time with `include_str!`. This keeps the wasm
//! bundle self-contained (no fetch at runtime) and lets the static-site
//! generator render the same bytes to HTML for SEO.
//!
//! Markdown is rendered with `pulldown-cmark`; the first level-1 heading is
//! lifted out as the document title.

use pulldown_cmark::{html, Options, Parser};

/// A single embedded markdown document: a stable slug, its raw source, and a
/// human title derived from the first `# ` heading.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Doc {
    /// URL slug (the last path segment).
    pub slug: &'static str,
    /// Short human title for indexes and nav.
    pub title: &'static str,
    /// One-line summary for cards and meta descriptions.
    pub summary: &'static str,
    /// Raw markdown source.
    pub markdown: &'static str,
}

impl Doc {
    /// Render this document's markdown body to an HTML string.
    pub fn to_html(&self) -> String {
        render_markdown(self.markdown)
    }
}

/// Render a markdown string to HTML using a common option set
/// (tables, strikethrough, footnotes).
pub fn render_markdown(src: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    let parser = Parser::new_ext(src, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// The ordered docs sidebar.
pub const DOCS: &[Doc] = &[
    Doc {
        slug: "getting-started",
        title: "Getting started",
        summary: "Install darkrun and open your first run.",
        markdown: include_str!("../content/docs/getting-started.md"),
    },
    Doc {
        slug: "stations",
        title: "Stations and phases",
        summary: "The six-phase machine every station runs.",
        markdown: include_str!("../content/docs/stations.md"),
    },
    Doc {
        slug: "review",
        title: "Review and feedback",
        summary: "Drive a run from its checkpoints.",
        markdown: include_str!("../content/docs/review.md"),
    },
    Doc {
        slug: "other-harnesses",
        title: "Other harnesses",
        summary: "Run darkrun in Cursor, Gemini, Codex, and more.",
        markdown: include_str!("../content/docs/other-harnesses.md"),
    },
];

/// The concept pages (methodology, glossary, lifecycles).
pub const CONCEPTS: &[Doc] = &[
    Doc {
        slug: "methodology",
        title: "The methodology",
        summary: "Why the line is ordered by the cost of late discovery.",
        markdown: include_str!("../content/concepts/methodology.md"),
    },
    Doc {
        slug: "glossary",
        title: "Glossary",
        summary: "darkrun's vocabulary, in one place.",
        markdown: include_str!("../content/concepts/glossary.md"),
    },
    Doc {
        slug: "lifecycles",
        title: "Lifecycles",
        summary: "The path work travels through a factory.",
        markdown: include_str!("../content/concepts/lifecycles.md"),
    },
];

/// Blog posts, newest first.
pub const POSTS: &[Doc] = &[
    Doc {
        slug: "checkpoints-not-babysitting",
        title: "Checkpoints, not babysitting",
        summary: "Spend a human's attention where it changes the outcome.",
        markdown: include_str!("../content/blog/checkpoints-not-babysitting.md"),
    },
    Doc {
        slug: "the-assembly-line",
        title: "The assembly line for software",
        summary: "Order the line by the cost of late discovery.",
        markdown: include_str!("../content/blog/the-assembly-line.md"),
    },
];

/// Look up a doc by slug within a corpus.
pub fn find<'a>(corpus: &'a [Doc], slug: &str) -> Option<&'a Doc> {
    corpus.iter().find(|d| d.slug == slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_corpus_has_entries() {
        assert!(!DOCS.is_empty());
        assert!(!CONCEPTS.is_empty());
        assert!(!POSTS.is_empty());
    }

    #[test]
    fn markdown_renders_to_html() {
        let html = render_markdown("# Title\n\nsome **bold** text");
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn slugs_are_unique_per_corpus() {
        for corpus in [DOCS, CONCEPTS, POSTS] {
            let mut slugs: Vec<&str> = corpus.iter().map(|d| d.slug).collect();
            slugs.sort_unstable();
            let len = slugs.len();
            slugs.dedup();
            assert_eq!(len, slugs.len(), "duplicate slug in corpus");
        }
    }

    #[test]
    fn lookup_finds_known_and_misses_unknown() {
        assert!(find(DOCS, "getting-started").is_some());
        assert!(find(DOCS, "nope").is_none());
    }
}

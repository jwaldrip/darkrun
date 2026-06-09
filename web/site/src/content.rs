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
    /// Publication date (`YYYY-MM-DD`) for blog posts; empty for non-post docs.
    pub date: &'static str,
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
    let src = preprocess_directives(src);
    let parser = Parser::new_ext(&src, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Transform fenced block-directives into raw-HTML `<div>` wrappers that
/// pulldown-cmark passes through while still rendering the inner content as
/// markdown.
///
/// Block syntax (CommonMark "generic directive" style):
///
/// ```text
/// :::callout warn
/// **Heads up.** Inner markdown still renders.
/// :::
/// ```
///
/// - A line matching `^:::<type>(\s+<rest>)?$` opens a directive, mapping to
///   `<div class="dr-md-<type>">`. A trailing bare word becomes a variant
///   class (`:::callout warn` → `dr-md-callout dr-md-callout-warn`).
/// - `key="value"` attrs are parsed; for `keypoints`, `title="..."` emits an
///   inner `<div class="dr-md-keypoints-title">Title</div>`.
/// - A line that is exactly `:::` closes the most recent open directive.
///   Nesting is supported via a stack; any still-open divs close at EOF.
/// - A blank line is emitted after each opening `<div>` and before each
///   `</div>` so pulldown-cmark treats the inner block as markdown (HTML
///   block type 6 terminates at a blank line).
/// - `:::` lines inside fenced code blocks (``` or `~~~`) pass through
///   verbatim.
/// - Unknown directive types still wrap (forward-compatible).
pub fn preprocess_directives(src: &str) -> String {
    let mut out = String::new();
    let mut stack: usize = 0;
    // The code-fence marker (backticks/tildes) currently open, if any.
    let mut fence: Option<String> = None;

    for line in src.lines() {
        let trimmed = line.trim_start();

        // Track code-fence state so `:::` inside code is never a directive.
        if let Some(open) = &fence {
            // Inside a fence: a line whose trimmed content is the same fence
            // char repeated at least as many times closes it.
            if is_closing_fence(trimmed, open) {
                fence = None;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(marker) = opening_fence(trimmed) {
            fence = Some(marker);
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Closing directive: a line that is exactly ":::".
        if line.trim() == ":::" {
            if stack > 0 {
                stack -= 1;
                // Blank line before </div> so the inner block is markdown.
                out.push('\n');
                out.push_str("</div>\n");
                continue;
            }
            // Stray ::: with nothing open — pass through unchanged.
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // Opening directive: ^:::<type>(\s+<rest>)?$
        if let Some((kind, rest)) = parse_directive_open(line) {
            let mut class = format!("dr-md-{kind}");
            let mut title: Option<String> = None;

            for token in DirectiveArgs::new(rest) {
                match token {
                    Arg::Bare(word) => {
                        class.push_str(&format!(" dr-md-{kind}-{word}"));
                    }
                    Arg::Pair(key, value) => {
                        if kind == "keypoints" && key == "title" {
                            title = Some(value);
                        }
                    }
                }
            }

            out.push_str(&format!("<div class=\"{class}\">\n"));
            // Blank line after the opening div so the inner block is markdown.
            out.push('\n');
            if let Some(title) = title {
                out.push_str(&format!(
                    "<div class=\"dr-md-keypoints-title\">{}</div>\n\n",
                    escape_html(&title)
                ));
            }
            stack += 1;
            continue;
        }

        // Non-directive line: pass through unchanged.
        out.push_str(line);
        out.push('\n');
    }

    // Close any still-open directives at EOF.
    while stack > 0 {
        stack -= 1;
        out.push('\n');
        out.push_str("</div>\n");
    }

    out
}

/// A parsed directive argument: a bare variant word or a `key="value"` pair.
enum Arg {
    Bare(String),
    Pair(String, String),
}

/// Iterator over the trailing args of a directive opening line, splitting on
/// whitespace but keeping `key="value"` (quoted values may contain spaces).
struct DirectiveArgs<'a> {
    rest: &'a str,
}

impl<'a> DirectiveArgs<'a> {
    fn new(rest: &'a str) -> Self {
        Self { rest }
    }
}

impl Iterator for DirectiveArgs<'_> {
    type Item = Arg;

    fn next(&mut self) -> Option<Arg> {
        let s = self.rest.trim_start();
        if s.is_empty() {
            self.rest = "";
            return None;
        }

        // Find the key boundary (up to `=` or whitespace).
        let eq = s.find('=');
        let ws = s.find(char::is_whitespace);
        let is_pair = match (eq, ws) {
            (Some(e), Some(w)) => e < w,
            (Some(_), None) => true,
            _ => false,
        };

        if is_pair {
            let eq = eq.unwrap();
            let key = s[..eq].trim().to_string();
            let after = &s[eq + 1..];
            if let Some(rest) = after.strip_prefix('"') {
                // Quoted value: read until the next unescaped quote.
                if let Some(end) = rest.find('"') {
                    let value = rest[..end].to_string();
                    self.rest = &rest[end + 1..];
                    return Some(Arg::Pair(key, value));
                }
                // Unterminated quote: take the remainder.
                self.rest = "";
                return Some(Arg::Pair(key, rest.to_string()));
            }
            // Bare value (no quotes): read to whitespace.
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            let value = after[..end].to_string();
            self.rest = &after[end..];
            return Some(Arg::Pair(key, value));
        }

        // Bare word: read to whitespace.
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        let word = s[..end].to_string();
        self.rest = &s[end..];
        Some(Arg::Bare(word))
    }
}

/// Parse an opening-directive line `^:::<type>(\s+<rest>)?$`, returning the
/// directive type and trailing argument text. The type must match
/// `[a-z][a-z0-9-]*`.
fn parse_directive_open(line: &str) -> Option<(String, &str)> {
    let after = line.strip_prefix(":::")?;
    // A bare "::: " (closing) is handled separately; require a type char here.
    let mut chars = after.char_indices();
    let (_, first) = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    let mut end = first.len_utf8();
    for (i, c) in chars {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let kind = &after[..end];
    let rest = &after[end..];
    // What follows the type must be end-of-line or whitespace, else not a
    // directive (e.g. "::::foo" or ":::ab.c").
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some((kind.to_string(), rest))
}

/// If `line` opens a code fence, return its fence marker (the run of `` ` ``
/// or `~`). Per CommonMark a fence is at least three of the same char.
fn opening_fence(line: &str) -> Option<String> {
    for ch in ['`', '~'] {
        let count = line.chars().take_while(|&c| c == ch).count();
        if count >= 3 {
            return Some(ch.to_string().repeat(count));
        }
    }
    None
}

/// Whether `line` closes a fence opened with `open`: same char, at least as
/// many of them, and no trailing non-whitespace (info strings are not allowed
/// on closing fences).
fn is_closing_fence(line: &str, open: &str) -> bool {
    let ch = open.chars().next().unwrap_or('`');
    let count = line.chars().take_while(|&c| c == ch).count();
    count >= open.len() && line[count..].trim().is_empty()
}

/// Minimal HTML-escape for directive attribute text injected into markup.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The ordered docs sidebar.
pub const DOCS: &[Doc] = &[
    Doc {
        slug: "getting-started",
        title: "Getting started",
        summary: "Install darkrun and open your first run.",
        date: "",
        markdown: include_str!("../content/docs/getting-started.md"),
    },
    Doc {
        slug: "stations",
        title: "Stations and phases",
        summary: "The six-phase machine every station runs.",
        date: "",
        markdown: include_str!("../content/docs/stations.md"),
    },
    Doc {
        slug: "review",
        title: "Review and feedback",
        summary: "Drive a run from its checkpoints.",
        date: "",
        markdown: include_str!("../content/docs/review.md"),
    },
    Doc {
        slug: "tools-and-commands",
        title: "Tools and commands",
        summary: "The slash commands you type and the MCP tools the manager calls.",
        date: "",
        markdown: include_str!("../content/docs/tools-and-commands.md"),
    },
    Doc {
        slug: "other-harnesses",
        title: "Other harnesses",
        summary: "Run darkrun in Cursor, Gemini, Codex, and more.",
        date: "",
        markdown: include_str!("../content/docs/other-harnesses.md"),
    },
];

/// The concept pages (methodology, glossary, lifecycles).
pub const CONCEPTS: &[Doc] = &[
    Doc {
        slug: "methodology",
        title: "The methodology",
        summary: "Why the line is ordered by the cost of late discovery.",
        date: "",
        markdown: include_str!("../content/concepts/methodology.md"),
    },
    Doc {
        slug: "glossary",
        title: "Glossary",
        summary: "darkrun's vocabulary, in one place.",
        date: "",
        markdown: include_str!("../content/concepts/glossary.md"),
    },
    Doc {
        slug: "lifecycles",
        title: "Lifecycles",
        summary: "The path work travels through a factory.",
        date: "",
        markdown: include_str!("../content/concepts/lifecycles.md"),
    },
];

/// The guide pages: onboarding and the prose-forward explainers
/// (start-here, how-it-works, big-picture, workflows, about).
pub const GUIDES: &[Doc] = &[
    Doc {
        slug: "start-here",
        title: "Start here",
        summary: "Install darkrun and run your first line, end to end.",
        date: "",
        markdown: include_str!("../content/guides/start-here.md"),
    },
    Doc {
        slug: "how-it-works",
        title: "How it works",
        summary: "The engine model: Factory > Station > Unit > Pass, the run loop, and the gates.",
        date: "",
        markdown: include_str!("../content/guides/how-it-works.md"),
    },
    Doc {
        slug: "big-picture",
        title: "The big picture",
        summary: "The dark factory, autonomous agents gated by humans, and where it's heading.",
        date: "",
        markdown: include_str!("../content/guides/big-picture.md"),
    },
    Doc {
        slug: "workflows",
        title: "Workflows",
        summary: "A practical catalog of the common darkrun workflows and commands.",
        date: "",
        markdown: include_str!("../content/guides/workflows.md"),
    },
    Doc {
        slug: "about",
        title: "About",
        summary: "What darkrun is, the philosophy, and the FSL-1.1-ALv2 license.",
        date: "",
        markdown: include_str!("../content/guides/about.md"),
    },
];

/// Blog posts, newest first.
pub const POSTS: &[Doc] = &[
    Doc {
        slug: "darkrun-is-a-harness",
        title: "darkrun is a harness",
        summary: "Map darkrun onto Anthropic's harness design, one part at a time.",
        date: "2026-06-08",
        markdown: include_str!("../content/blog/darkrun-is-a-harness.md"),
    },
    Doc {
        slug: "pure-rust-c-free",
        title: "Pure Rust, no C",
        summary: "End-to-end Rust, git through gix, one reproducible binary.",
        date: "2026-06-06",
        markdown: include_str!("../content/blog/pure-rust-c-free.md"),
    },
    Doc {
        slug: "team-solo-dark",
        title: "Team, solo, dark",
        summary: "One global dial sets where you sit relative to the run.",
        date: "2026-06-04",
        markdown: include_str!("../content/blog/team-solo-dark.md"),
    },
    Doc {
        slug: "the-dark-factory",
        title: "The dark factory",
        summary: "Lights-out manufacturing as the model for a run.",
        date: "2026-06-02",
        markdown: include_str!("../content/blog/the-dark-factory.md"),
    },
    Doc {
        slug: "checkpoints-not-babysitting",
        title: "Checkpoints, not babysitting",
        summary: "Spend a human's attention where it changes the outcome.",
        date: "2026-06-01",
        markdown: include_str!("../content/blog/checkpoints-not-babysitting.md"),
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
        assert!(!GUIDES.is_empty());
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
        for corpus in [DOCS, CONCEPTS, GUIDES, POSTS] {
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

    #[test]
    fn callout_variant_wraps_and_renders_inner_markdown() {
        let html = render_markdown(":::callout warn\n**hi**\n:::");
        // Variant class is applied alongside the base class.
        assert!(html.contains("dr-md-callout"));
        assert!(html.contains("dr-md-callout-warn"));
        // The inner markdown is rendered (not passed through as literal text).
        assert!(html.contains("<strong>hi</strong>"));
    }

    #[test]
    fn keypoints_title_attr_emits_eyebrow() {
        let html = render_markdown(":::keypoints title=\"The modes\"\n- a\n- b\n:::");
        assert!(html.contains("dr-md-keypoints"));
        assert!(html.contains("dr-md-keypoints-title"));
        assert!(html.contains("The modes"));
        // List items still render as markdown.
        assert!(html.contains("<li>a</li>"));
    }

    #[test]
    fn colons_inside_code_fence_are_not_directives() {
        let html = render_markdown("```\n:::callout warn\n:::\n```");
        // No directive div is emitted; the literal text survives in a code block.
        assert!(!html.contains("dr-md-callout"));
        assert!(html.contains(":::callout warn"));
    }

    #[test]
    fn nested_directives_close_in_order() {
        let src = ":::columns\n:::callout\ninner\n:::\nouter\n:::";
        let html = render_markdown(src);
        assert!(html.contains("dr-md-columns"));
        assert!(html.contains("dr-md-callout"));
        // Two opening divs and two matching closes.
        assert_eq!(html.matches("<div class=\"dr-md-").count(), 2);
        assert_eq!(html.matches("</div>").count(), 2);
        assert!(html.contains("inner"));
        assert!(html.contains("outer"));
    }

    #[test]
    fn unopened_directive_closes_at_eof() {
        let processed = preprocess_directives(":::callout\nbody");
        // The still-open div is closed at EOF.
        assert!(processed.contains("<div class=\"dr-md-callout\">"));
        assert!(processed.contains("</div>"));
    }

    #[test]
    fn unknown_directive_still_wraps() {
        let html = render_markdown(":::whatever\nbody\n:::");
        assert!(html.contains("dr-md-whatever"));
    }
}

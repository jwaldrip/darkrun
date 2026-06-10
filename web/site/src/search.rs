//! Site-wide content search — pure Rust over the compile-time corpus.
//!
//! The docs/concepts/guides/blog corpus is embedded at build time
//! ([`crate::content`]), so search needs no JS library and no fetched index:
//! a case-insensitive scan scores each document (title hits outweigh summary
//! hits outweigh body hits) and returns the top matches with a snippet around
//! the first body occurrence. Runs on every keystroke in the wasm — the corpus
//! is dozens of documents, not thousands.

use darkrun_ui::prelude::*;

use crate::content::{Doc, CONCEPTS, DOCS, GUIDES, POSTS};
use crate::ui::theme;

/// One search hit: where it lives, what to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The section label shown on the hit (`Docs`, `Concepts`, `Guides`, `Blog`).
    pub section: &'static str,
    /// The route path the hit links to.
    pub path: String,
    /// The document title.
    pub title: &'static str,
    /// A snippet around the first body match (the summary when the match was
    /// title-only).
    pub snippet: String,
}

/// The searched sections, with their route prefixes.
fn sections() -> [(&'static str, &'static str, &'static [Doc]); 4] {
    [
        ("Docs", "/docs", DOCS),
        ("Concepts", "/concepts", CONCEPTS),
        ("Guides", "/guides", GUIDES),
        ("Blog", "/blog", POSTS),
    ]
}

/// Search the whole corpus for `query`. Empty/whitespace queries return
/// nothing; results are scored (title 8 / summary 3 / per-body-hit 1, capped)
/// and the top 10 returned.
pub fn search(query: &str) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    if q.len() < 2 {
        return Vec::new();
    }
    let mut scored: Vec<(u32, Hit)> = Vec::new();
    for (section, prefix, corpus) in sections() {
        for doc in corpus {
            let title = doc.title.to_lowercase();
            let summary = doc.summary.to_lowercase();
            let body = doc.markdown.to_lowercase();
            let mut score = 0u32;
            if title.contains(&q) {
                score += 8;
            }
            if summary.contains(&q) {
                score += 3;
            }
            let body_hits = body.matches(&q).count().min(5) as u32;
            score += body_hits;
            if score == 0 {
                continue;
            }
            let snippet = body
                .find(&q)
                .map(|at| snippet_around(doc.markdown, at, q.len()))
                .unwrap_or_else(|| doc.summary.to_string());
            scored.push((
                score,
                Hit {
                    section,
                    path: format!("{prefix}/{}", doc.slug),
                    title: doc.title,
                    snippet,
                },
            ));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(b.1.title)));
    scored.into_iter().take(10).map(|(_, h)| h).collect()
}

/// A ~120-char window around the match at byte offset `at` (computed against
/// the lowercased text, applied to the original — same byte layout for ASCII;
/// for non-ASCII boundaries we walk to the nearest char boundary).
fn snippet_around(original: &str, at: usize, qlen: usize) -> String {
    let start = at.saturating_sub(48);
    let end = (at + qlen + 72).min(original.len());
    let start = floor_char_boundary(original, start);
    let end = floor_char_boundary(original, end);
    let mut s = original[start..end].replace(['\n', '#', '`', '*'], " ");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < original.len() { "…" } else { "" };
    format!("{prefix}{s}{suffix}")
}

/// The largest char boundary `<= i` (stable substitute for `str::floor_char_boundary`).
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}


/// Inject (or replace) the page's `application/ld+json` block in `<head>` —
/// per-route structured data for crawlers. Idempotent by element id; runs
/// whenever `json` changes (route navigation re-renders the page component).
pub fn use_json_ld(json: String) {
    use_effect(use_reactive!(|json| {
        let payload = serde_json::to_string(&json).unwrap_or_default();
        let _ = document::eval(&format!(
            "(function(){{var el=document.getElementById('ld-page');\
             if(!el){{el=document.createElement('script');el.id='ld-page';\
             el.type='application/ld+json';document.head.appendChild(el);}}\
             el.textContent={payload};}})();"
        ));
    }));
}

/// The docs-sidebar search box: an input filtering the whole content corpus
/// on every keystroke, results linking into their sections.
#[component]
pub fn SearchBox() -> Element {
    let mut query = use_signal(String::new);
    let nav = use_navigator();
    let hits = search(&query.read());
    let input_style = format!(
        "width:100%;box-sizing:border-box;background:transparent;color:{text};\
         border:1px solid {border};border-radius:6px;padding:6px 9px;\
         font-family:{sans};font-size:13px;outline:none;",
        text = theme::TEXT,
        border = theme::BORDER,
        sans = tokens::FONT_SANS,
    );
    let results_style = format!(
        "display:flex;flex-direction:column;gap:2px;margin:6px 0 10px;\
         border:1px solid {border};border-radius:8px;background:{surface};\
         padding:6px;max-height:340px;overflow:auto;",
        border = theme::BORDER,
        surface = theme::SURFACE_RAISED,
    );
    rsx! {
        div { style: "display:flex;flex-direction:column;margin-bottom:10px;",
            input {
                style: "{input_style}",
                r#type: "search",
                placeholder: "Search docs\u{2026}",
                aria_label: "Search the documentation",
                value: "{query}",
                oninput: move |e| query.set(e.value()),
            }
            if !hits.is_empty() {
                div { style: "{results_style}", role: "listbox",
                    for hit in hits {
                        {
                            let path = hit.path.clone();
                            let row = format!(
                                "display:flex;flex-direction:column;gap:1px;padding:6px 8px;\
                                 border-radius:6px;cursor:pointer;",
                            );
                            let kicker = format!(
                                "font-family:{mono};font-size:10px;text-transform:uppercase;\
                                 letter-spacing:0.08em;color:{faint};",
                                mono = tokens::FONT_MONO,
                                faint = theme::TEXT_FAINT,
                            );
                            let title_st = format!(
                                "font-family:{sans};font-size:13px;font-weight:600;color:{accent};",
                                sans = tokens::FONT_SANS,
                                accent = theme::ACCENT,
                            );
                            let snip = format!(
                                "font-family:{sans};font-size:12px;color:{muted};",
                                sans = tokens::FONT_SANS,
                                muted = theme::TEXT_MUTED,
                            );
                            rsx! {
                                div {
                                    style: "{row}",
                                    role: "option",
                                    onclick: move |_| {
                                        nav.push(path.as_str());
                                        query.set(String::new());
                                    },
                                    span { style: "{kicker}", "{hit.section}" }
                                    span { style: "{title_st}", "{hit.title}" }
                                    span { style: "{snip}", "{hit.snippet}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_tiny_queries_return_nothing() {
        assert!(search("").is_empty());
        assert!(search(" ").is_empty());
        assert!(search("a").is_empty());
    }

    #[test]
    fn title_matches_rank_above_body_matches() {
        let hits = search("station");
        assert!(!hits.is_empty(), "the corpus talks about stations");
        // The docs page titled about stations should lead.
        assert!(
            hits[0].title.to_lowercase().contains("station"),
            "title match leads: {:?}",
            hits.iter().map(|h| h.title).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hits_link_into_their_sections() {
        let hits = search("run");
        assert!(!hits.is_empty());
        for h in &hits {
            assert!(
                h.path.starts_with("/docs/")
                    || h.path.starts_with("/concepts/")
                    || h.path.starts_with("/guides/")
                    || h.path.starts_with("/blog/"),
                "{}",
                h.path
            );
            assert!(!h.snippet.is_empty());
        }
    }

    #[test]
    fn search_is_case_insensitive() {
        assert_eq!(
            search("STATION").first().map(|h| h.path.clone()),
            search("station").first().map(|h| h.path.clone())
        );
    }

    #[test]
    fn results_are_capped() {
        assert!(search("the").len() <= 10);
    }
}

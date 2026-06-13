//! A minimal, dependency-free markdown renderer for the short agent-authored
//! prose the interactive session views carry — question/direction prompts,
//! context preambles, and option descriptions.
//!
//! It covers exactly what those prompts use in practice: paragraphs (blank-line
//! separated), unordered lists (`- ` / `* `), inline `**bold**`, and inline
//! `` `code` ``. Everything else passes through as text. The output is a small
//! HTML string rendered via `dangerous_inner_html`; all source text is
//! HTML-escaped FIRST, then the inline markers are applied to the escaped text,
//! so there is no injection surface even though the input is agent-authored.
//!
//! This is deliberately not a full CommonMark engine (no headings, links,
//! nested lists, tables) — those don't appear in mid-run prompts, and keeping it
//! tiny avoids pulling a markdown crate into the wasm site build.

/// HTML-escape `&`, `<`, `>`, `"` so source text can never inject markup.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Apply inline markers to ALREADY-ESCAPED text: `**bold**` → `<strong>`,
/// `` `code` `` → `<code>`. Unmatched markers are left as literal text. Runs in
/// a single pass; `code` spans are taken verbatim (no bold inside code).
///
/// UTF-8 safe: the marker bytes (`*`, `` ` ``) are ASCII so `find` returns
/// char-boundary offsets, and untouched text is advanced one whole `char` at a
/// time — never byte-by-byte, which would shred multi-byte glyphs like `—`.
fn inline(escaped: &str) -> String {
    let mut out = String::with_capacity(escaped.len());
    let mut rest = escaped;
    while !rest.is_empty() {
        // Inline code: `...`
        if let Some(after) = rest.strip_prefix('`') {
            if let Some(end) = after.find('`') {
                out.push_str("<code class=\"dr-md-code\">");
                out.push_str(&after[..end]);
                out.push_str("</code>");
                rest = &after[end + 1..];
                continue;
            }
        }
        // Bold: **...**
        if let Some(after) = rest.strip_prefix("**") {
            if let Some(end) = after.find("**") {
                out.push_str("<strong>");
                out.push_str(&after[..end]);
                out.push_str("</strong>");
                rest = &after[end + 2..];
                continue;
            }
        }
        // Pass one whole char through (advance by its full UTF-8 width).
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// Whether a line is an unordered-list item (`- ` or `* `), returning its body.
fn list_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))
}

/// Render the supported markdown subset of `src` to an HTML string. Empty input
/// yields an empty string. Blocks are paragraphs and `<ul>` lists; consecutive
/// list lines group into one list, blank lines break paragraphs.
pub fn to_html(src: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut list: Vec<String> = Vec::new();

    let flush_para = |out: &mut String, para: &mut Vec<String>| {
        if !para.is_empty() {
            out.push_str("<p class=\"dr-md-p\">");
            out.push_str(&inline(&para.join(" ")));
            out.push_str("</p>");
            para.clear();
        }
    };
    let flush_list = |out: &mut String, list: &mut Vec<String>| {
        if !list.is_empty() {
            out.push_str("<ul class=\"dr-md-ul\">");
            for item in list.iter() {
                out.push_str("<li>");
                out.push_str(&inline(item));
                out.push_str("</li>");
            }
            out.push_str("</ul>");
            list.clear();
        }
    };

    for raw in src.lines() {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            flush_list(&mut out, &mut list);
            flush_para(&mut out, &mut para);
        } else if let Some(item) = list_item(line) {
            // A list starts: close any open paragraph first.
            flush_para(&mut out, &mut para);
            list.push(escape(item.trim()));
        } else {
            // A normal line: close any open list first.
            flush_list(&mut out, &mut list);
            para.push(escape(line.trim()));
        }
    }
    flush_list(&mut out, &mut list);
    flush_para(&mut out, &mut para);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(escape("a<b>&\"c"), "a&lt;b&gt;&amp;&quot;c");
    }

    #[test]
    fn renders_bold_and_code() {
        assert_eq!(
            to_html("call **run_tick** via `cargo test`"),
            "<p class=\"dr-md-p\">call <strong>run_tick</strong> via \
             <code class=\"dr-md-code\">cargo test</code></p>"
        );
    }

    #[test]
    fn renders_a_bulleted_list_distinct_from_paragraphs() {
        let html = to_html("Pick one:\n\n- **A** fast\n- B slow\n\ndone");
        assert!(html.contains("<p class=\"dr-md-p\">Pick one:</p>"), "{html}");
        assert!(
            html.contains("<ul class=\"dr-md-ul\"><li><strong>A</strong> fast</li><li>B slow</li></ul>"),
            "{html}"
        );
        assert!(html.ends_with("<p class=\"dr-md-p\">done</p>"), "{html}");
    }

    #[test]
    fn a_dash_run_on_splits_into_list_items() {
        // The real-world failure: bullets on their own lines must each become an
        // <li>, not one run-on paragraph with literal dashes.
        let html = to_html("- one\n- two\n- three");
        assert_eq!(html.matches("<li>").count(), 3, "{html}");
        assert!(!html.contains("- one"), "no literal dashes survive: {html}");
    }

    #[test]
    fn unmatched_markers_stay_literal_and_safe() {
        let html = to_html("2 * 3 and a lone ` tick");
        assert!(html.contains("2 * 3"), "{html}");
        assert!(!html.contains("<strong>"), "{html}");
        assert!(!html.contains("<code"), "{html}");
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(to_html(""), "");
        assert_eq!(to_html("   \n  \n"), "");
    }

    #[test]
    fn multibyte_glyphs_survive_intact() {
        // Regression: an em-dash (3 UTF-8 bytes) next to bold/code must not be
        // shredded into mojibake by a byte-wise passthrough.
        let html = to_html("**A** — calls `run_tick` — fast … done");
        assert!(html.contains("</strong> — calls"), "{html}");
        assert!(html.contains("</code> — fast … done"), "{html}");
        assert!(!html.contains('\u{00e2}'), "no Latin-1 mojibake: {html}");
    }
}

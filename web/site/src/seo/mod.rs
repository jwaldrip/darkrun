//! SEO artifacts: sitemap.xml, robots.txt, and the RSS / Atom / JSON feeds.
//!
//! These are pure string builders over the embedded corpora and the route
//! table, so they are unit-testable and compile to wasm. The native static-site
//! generator ([`crate::bin`]-side) writes their output to disk.

use crate::content::POSTS;
use crate::route::Route;

/// Canonical site origin (no trailing slash).
pub const SITE_URL: &str = "https://darkrun.ai";

/// The site's human name.
pub const SITE_NAME: &str = "darkrun";

/// One-line site description for feed metadata.
pub const SITE_DESCRIPTION: &str = "An agentic assembly line for your business.";

/// Build `sitemap.xml` covering every concrete route on the site.
pub fn sitemap() -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in Route::all_paths() {
        out.push_str("  <url><loc>");
        out.push_str(SITE_URL);
        out.push_str(&xml_escape(&path));
        out.push_str("</loc></url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

/// Build `robots.txt`: allow everything (including the major AI crawlers) and
/// point at the sitemap and feeds.
pub fn robots() -> String {
    format!(
        "# darkrun robots.txt\n\
         User-agent: *\n\
         Allow: /\n\n\
         # AI crawlers welcome\n\
         User-agent: GPTBot\nAllow: /\n\
         User-agent: ClaudeBot\nAllow: /\n\
         User-agent: anthropic-ai\nAllow: /\n\
         User-agent: Google-Extended\nAllow: /\n\
         User-agent: PerplexityBot\nAllow: /\n\
         User-agent: CCBot\nAllow: /\n\n\
         # Feeds: {site}/feed.xml (RSS) \u{00b7} {site}/atom.xml (Atom) \u{00b7} {site}/feed.json (JSON)\n\
         Sitemap: {site}/sitemap.xml\n",
        site = SITE_URL,
    )
}

/// Build an RSS 2.0 feed of the blog posts.
pub fn feed_rss() -> String {
    let mut items = String::new();
    for post in POSTS {
        let link = format!("{SITE_URL}/blog/{}", post.slug);
        items.push_str(&format!(
            "    <item>\n      <title>{title}</title>\n      <link>{link}</link>\n      <guid>{link}</guid>\n      <description>{summary}</description>\n    </item>\n",
            title = xml_escape(post.title),
            link = xml_escape(&link),
            summary = xml_escape(post.summary),
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<rss version=\"2.0\">\n  <channel>\n    <title>{name}</title>\n    <link>{site}</link>\n    <description>{desc}</description>\n{items}  </channel>\n</rss>\n",
        name = xml_escape(SITE_NAME),
        site = SITE_URL,
        desc = xml_escape(SITE_DESCRIPTION),
    )
}

/// Build an Atom 1.0 feed of the blog posts.
pub fn feed_atom() -> String {
    let mut entries = String::new();
    for post in POSTS {
        let link = format!("{SITE_URL}/blog/{}", post.slug);
        entries.push_str(&format!(
            "  <entry>\n    <title>{title}</title>\n    <id>{link}</id>\n    <link href=\"{link}\"/>\n    <summary>{summary}</summary>\n  </entry>\n",
            title = xml_escape(post.title),
            link = xml_escape(&link),
            summary = xml_escape(post.summary),
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n  <title>{name}</title>\n  <id>{site}/</id>\n  <link href=\"{site}/\"/>\n{entries}</feed>\n",
        name = xml_escape(SITE_NAME),
        site = SITE_URL,
    )
}

/// Build a JSON Feed 1.1 document of the blog posts.
pub fn feed_json() -> String {
    let items: Vec<String> = POSTS
        .iter()
        .map(|post| {
            let link = format!("{SITE_URL}/blog/{}", post.slug);
            format!(
                "{{\"id\":{id},\"url\":{url},\"title\":{title},\"summary\":{summary}}}",
                id = json_string(&link),
                url = json_string(&link),
                title = json_string(post.title),
                summary = json_string(post.summary),
            )
        })
        .collect();
    format!(
        "{{\"version\":\"https://jsonfeed.org/version/1.1\",\
         \"title\":{name},\"home_page_url\":{home},\"feed_url\":{feed},\
         \"description\":{desc},\"items\":[{items}]}}",
        name = json_string(SITE_NAME),
        home = json_string(SITE_URL),
        feed = json_string(&format!("{SITE_URL}/feed.json")),
        desc = json_string(SITE_DESCRIPTION),
        items = items.join(","),
    )
}

/// Minimal XML text escaping.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// JSON string literal escaping (quotes, backslashes, control chars).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sitemap_lists_landing_and_factories_index() {
        let xml = sitemap();
        assert!(xml.contains("<loc>https://darkrun.ai/</loc>"));
        assert!(xml.contains("<loc>https://darkrun.ai/factories</loc>"));
        assert!(xml.trim_end().ends_with("</urlset>"));
    }

    #[test]
    fn sitemap_includes_dynamic_factory_routes() {
        let xml = sitemap();
        // The embedded corpus ships at least the `software` factory.
        assert!(xml.contains("/factories/software"));
    }

    #[test]
    fn robots_allows_all_and_points_at_sitemap() {
        let txt = robots();
        assert!(txt.contains("User-agent: *"));
        assert!(txt.contains("Allow: /"));
        assert!(txt.contains("Sitemap: https://darkrun.ai/sitemap.xml"));
        assert!(txt.contains("ClaudeBot"));
    }

    #[test]
    fn feeds_render_every_post() {
        let rss = feed_rss();
        let atom = feed_atom();
        let json = feed_json();
        for post in POSTS {
            assert!(rss.contains(post.title), "rss missing {}", post.title);
            assert!(atom.contains(post.title), "atom missing {}", post.title);
            assert!(json.contains(post.title), "json missing {}", post.title);
        }
        assert!(rss.starts_with("<?xml"));
        assert!(atom.contains("<feed"));
        assert!(json.contains("jsonfeed.org"));
    }

    #[test]
    fn xml_escaping_is_applied() {
        assert_eq!(xml_escape("a & b < c"), "a &amp; b &lt; c");
    }

    #[test]
    fn json_string_escapes_quotes() {
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn xml_escape_handles_all_five_entities() {
        assert_eq!(
            xml_escape("&<>\"'"),
            "&amp;&lt;&gt;&quot;&apos;"
        );
    }

    #[test]
    fn xml_escape_ampersand_first_avoids_double_escaping() {
        // `<` becomes `&lt;`; the `&` it introduces must not be re-escaped.
        assert_eq!(xml_escape("<"), "&lt;");
        assert_eq!(xml_escape(">"), "&gt;");
        assert_eq!(xml_escape("\""), "&quot;");
        assert_eq!(xml_escape("'"), "&apos;");
    }

    #[test]
    fn xml_escape_passes_through_plain_text() {
        assert_eq!(xml_escape("plain text 123"), "plain text 123");
        assert_eq!(xml_escape(""), "");
    }

    #[test]
    fn xml_escape_preserves_unicode() {
        assert_eq!(xml_escape("café · darkrun"), "café · darkrun");
    }

    #[test]
    fn xml_escape_repeated_specials() {
        assert_eq!(xml_escape("a&&b"), "a&amp;&amp;b");
        assert_eq!(xml_escape("<<>>"), "&lt;&lt;&gt;&gt;");
    }

    #[test]
    fn json_string_wraps_in_quotes() {
        assert_eq!(json_string("hi"), "\"hi\"");
        assert_eq!(json_string(""), "\"\"");
    }

    #[test]
    fn json_string_escapes_backslash() {
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn json_string_escapes_whitespace_controls() {
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
        assert_eq!(json_string("a\rb"), "\"a\\rb\"");
        assert_eq!(json_string("a\tb"), "\"a\\tb\"");
    }

    #[test]
    fn json_string_escapes_low_control_chars_as_unicode() {
        // A NUL and a vertical tab fall to the \u00xx branch.
        assert_eq!(json_string("\u{0}"), "\"\\u0000\"");
        assert_eq!(json_string("\u{b}"), "\"\\u000b\"");
        assert_eq!(json_string("\u{1f}"), "\"\\u001f\"");
    }

    #[test]
    fn json_string_passes_through_unicode_above_control_range() {
        // 0x20 and above are emitted verbatim (no escaping of normal unicode).
        assert_eq!(json_string("é·🚀"), "\"é·🚀\"");
        assert_eq!(json_string(" "), "\" \"");
    }

    #[test]
    fn json_string_does_not_escape_forward_slash() {
        // Forward slashes are legal unescaped in JSON; our builder leaves them.
        assert_eq!(json_string("a/b"), "\"a/b\"");
    }

    #[test]
    fn json_string_combined() {
        assert_eq!(json_string("\"\\\n"), "\"\\\"\\\\\\n\"");
    }
    // ── JSON-LD ─────────────────────────────────────────────────────────────

    #[test]
    fn site_json_ld_is_valid_and_carries_org_website_and_search() {
        let v: serde_json::Value = serde_json::from_str(&json_ld_site()).expect("valid JSON");
        let graph = v["@graph"].as_array().expect("graph");
        assert!(graph.iter().any(|n| n["@type"] == "Organization"));
        let site = graph.iter().find(|n| n["@type"] == "WebSite").expect("WebSite");
        assert_eq!(site["potentialAction"]["@type"], "SearchAction");
    }

    #[test]
    fn index_html_embeds_the_same_site_json_ld() {
        // The static block in index.html and the builder must agree — compared
        // as parsed JSON so key order can't drift them apart.
        let html = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/index.html"));
        let start = html.find(r#"<script type="application/ld+json" id="ld-site">"#)
            .expect("index.html carries the site JSON-LD block");
        let rest = &html[start..];
        let open = rest.find('>').unwrap() + 1;
        let close = rest.find("</script>").unwrap();
        let embedded: serde_json::Value =
            serde_json::from_str(&rest[open..close]).expect("embedded block parses");
        let built: serde_json::Value = serde_json::from_str(&json_ld_site()).unwrap();
        assert_eq!(embedded, built, "index.html JSON-LD drifted from seo::json_ld_site()");
    }

    #[test]
    fn article_json_ld_distinguishes_posts_from_docs() {
        let post = crate::content::POSTS.first().expect("a post exists");
        let v: serde_json::Value =
            serde_json::from_str(&json_ld_article(post, "/blog/x")).unwrap();
        assert_eq!(v["@type"], "BlogPosting");
        assert!(v["datePublished"].is_string());

        let doc = crate::content::DOCS.first().expect("a doc exists");
        let v: serde_json::Value =
            serde_json::from_str(&json_ld_article(doc, "/docs/x")).unwrap();
        assert_eq!(v["@type"], "TechArticle");
        assert!(v.get("datePublished").is_none());
        assert_eq!(v["url"], format!("{SITE_URL}/docs/x"));
    }

}

// ── JSON-LD structured data (schema.org) ────────────────────────────────────

/// The site-level JSON-LD: an `Organization` + a `WebSite` carrying a
/// `SearchAction` (the docs search). Embedded statically in `index.html` and
/// kept in sync by a unit test, so the crawler-facing block and the builder
/// can't drift apart.
pub fn json_ld_site() -> String {
    serde_json::json!({
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "Organization",
                "@id": format!("{SITE_URL}/#org"),
                "name": SITE_NAME,
                "url": SITE_URL,
                "logo": format!("{SITE_URL}/assets/favicon.png"),
            },
            {
                "@type": "WebSite",
                "@id": format!("{SITE_URL}/#website"),
                "name": SITE_NAME,
                "description": SITE_DESCRIPTION,
                "url": SITE_URL,
                "publisher": { "@id": format!("{SITE_URL}/#org") },
                "potentialAction": {
                    "@type": "SearchAction",
                    "target": format!("{SITE_URL}/docs?q={{search_term_string}}"),
                    "query-input": "required name=search_term_string",
                },
            },
        ],
    })
    .to_string()
}

/// Per-document JSON-LD: a `BlogPosting` for dated posts, a `TechArticle` for
/// docs/concepts/guides. Injected into `<head>` when the page mounts.
pub fn json_ld_article(doc: &crate::content::Doc, path: &str) -> String {
    let kind = if doc.date.is_empty() { "TechArticle" } else { "BlogPosting" };
    let mut obj = serde_json::json!({
        "@context": "https://schema.org",
        "@type": kind,
        "headline": doc.title,
        "description": doc.summary,
        "url": format!("{SITE_URL}{path}"),
        "author": { "@id": format!("{SITE_URL}/#org") },
        "publisher": { "@id": format!("{SITE_URL}/#org") },
    });
    if !doc.date.is_empty() {
        obj["datePublished"] = serde_json::Value::String(doc.date.to_string());
    }
    obj.to_string()
}

//! YAML-frontmatter + markdown-body parsing.
//!
//! Documents follow a simple convention: an optional
//! leading `---` fence containing YAML, then the markdown body. This module
//! splits/joins that envelope and (de)serializes the frontmatter into any
//! serde type.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{CoreError, Result};

/// A document split into its raw YAML frontmatter and markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Raw YAML between the fences (empty when there is no frontmatter).
    pub frontmatter: String,
    /// Everything after the closing fence.
    pub body: String,
}

/// Split a raw document into frontmatter + body.
///
/// A document that opens with a `---`
/// line has its YAML extracted up to the next `---` line; anything else is
/// treated as a body-only document with empty frontmatter.
pub fn split(raw: &str) -> Document {
    // Normalize so a leading BOM or CRLF doesn't defeat the fence match.
    let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let normalized = trimmed.replace("\r\n", "\n");

    let Some(rest) = normalized.strip_prefix("---\n") else {
        return Document {
            frontmatter: String::new(),
            body: normalized,
        };
    };

    // Find the closing fence: a line that is exactly `---`.
    if let Some((fm, body)) = split_at_closing_fence(rest) {
        Document {
            frontmatter: fm,
            body,
        }
    } else {
        // Unterminated fence — treat the whole thing as body.
        Document {
            frontmatter: String::new(),
            body: normalized,
        }
    }
}

fn split_at_closing_fence(rest: &str) -> Option<(String, String)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if content == "---" {
            let fm = rest[..offset].to_string();
            let body_start = offset + line.len();
            let body = rest.get(body_start..).unwrap_or("").to_string();
            return Some((fm, body));
        }
        offset += line.len();
    }
    // No standalone closing `---` line. (`split_inclusive` always yields a
    // trailing `---`-without-newline as its own segment, so the loop above
    // already handles an EOF fence — no separate case is needed here.)
    None
}

/// Parse a document into a typed frontmatter `T` plus its markdown body.
///
/// Returns [`CoreError::MissingFrontmatter`] when the document has no
/// frontmatter fence — callers that allow body-only docs should use
/// [`split`] directly.
pub fn parse<T: DeserializeOwned>(raw: &str) -> Result<(T, String)> {
    let doc = split(raw);
    if doc.frontmatter.is_empty() && !raw.trim_start().starts_with("---") {
        return Err(CoreError::MissingFrontmatter);
    }
    let frontmatter: T = serde_yaml::from_str(&doc.frontmatter)?;
    Ok((frontmatter, doc.body))
}

/// Serialize a typed frontmatter `T` and a markdown body into a document.
pub fn serialize<T: Serialize>(frontmatter: &T, body: &str) -> Result<String> {
    let yaml = serde_yaml::to_string(frontmatter)?;
    // serde_yaml emits a trailing newline; normalize then re-fence.
    let yaml = yaml.trim_end_matches('\n');
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(yaml);
    out.push_str("\n---\n");
    if !body.is_empty() {
        // Ensure exactly one blank line is not forced; preserve body as-is
        // but guarantee separation from the fence.
        if !body.starts_with('\n') {
            out.push('\n');
        }
        out.push_str(body);
    }
    Ok(out)
}

/// Extract the first level-1 markdown heading (`# Title`) from a body.
pub fn first_heading(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

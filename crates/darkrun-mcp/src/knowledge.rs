//! Project-level knowledge — the explorer-maintained shared memory.
//!
//! Discovery explorers surface durable project facts (constraints, prior art,
//! traps, conventions). Unlike a run's per-station discovery, these are
//! **project-scoped**: they persist in `.darkrun/knowledge/<topic>.md` across
//! runs, so a later run's Spec reads them as priors instead of re-discovering
//! the same ground. Keyed by `topic` slug — re-recording a topic updates the
//! prior in place (the predecessor's `scope: project` knowledge that decompose
//! reads and updates when it diverges).

use chrono::Utc;
use darkrun_core::StateStore;
use serde::Serialize;

use crate::error::Result;

/// A piece of durable project knowledge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Knowledge {
    /// The topic slug (the doc id).
    pub topic: String,
    /// When the topic was first recorded.
    pub created_at: String,
    /// When the topic was last updated.
    pub updated_at: String,
    /// The knowledge prose.
    pub body: String,
}

/// Sanitize a topic into a single safe path component.
fn safe_topic(topic: &str) -> String {
    let s: String = topic
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if s.is_empty() { "untitled".to_string() } else { s }
}

/// Record project knowledge for `topic`, updating it in place when the topic
/// already exists (preserving its original `created_at`).
pub fn record(store: &StateStore, topic: &str, body: &str) -> Result<Knowledge> {
    let topic = safe_topic(topic);
    let now = Utc::now().to_rfc3339();
    // Preserve the original created_at when updating an existing topic.
    let created_at = store
        .read_knowledge_entry(&topic)?
        .map(|doc| parse(topic.clone(), &doc).created_at)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| now.clone());
    let doc = format!(
        "---\ntopic: {topic}\ncreated_at: {created_at}\nupdated_at: {now}\n---\n{}\n",
        body.trim()
    );
    store.write_knowledge_raw(&topic, &doc)?;
    let _ = crate::commit::commit_state(store, &format!("darkrun: knowledge {topic}"));
    Ok(Knowledge {
        topic,
        created_at,
        updated_at: now,
        body: body.trim().to_string(),
    })
}

/// List every project knowledge entry, by topic.
pub fn list(store: &StateStore) -> Result<Vec<Knowledge>> {
    let raw = store.read_knowledge_raw()?;
    Ok(raw.into_iter().map(|(id, doc)| parse(id, &doc)).collect())
}

/// Parse a knowledge document. Tolerant of a missing frontmatter fence.
fn parse(topic: String, doc: &str) -> Knowledge {
    let doc = doc.trim_start_matches('\u{feff}');
    let field = |fm: &str, key: &str| -> String {
        fm.lines()
            .find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().trim_matches('"').to_string()))
            .unwrap_or_default()
    };
    if let Some(rest) = doc.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').trim().to_string();
            return Knowledge {
                topic,
                created_at: field(fm, "created_at:"),
                updated_at: field(fm, "updated_at:"),
                body,
            };
        }
    }
    Knowledge {
        topic,
        created_at: String::new(),
        updated_at: String::new(),
        body: doc.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempdir().unwrap();
        let store = StateStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn record_is_project_scoped_and_lists() {
        let (_d, store) = store();
        record(&store, "auth-conventions", "use the shared CredentialStore").unwrap();
        record(&store, "build-traps", "the wasm target needs --no-default-features").unwrap();
        let all = list(&store).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|k| k.topic == "auth-conventions" && k.body.contains("CredentialStore")));
        // Stored at the PROJECT root, not under any run.
        assert!(store.knowledge_dir().join("auth-conventions.md").exists());
    }

    #[test]
    fn re_recording_a_topic_updates_in_place_keeping_created_at() {
        let (_d, store) = store();
        let first = record(&store, "x", "v1").unwrap();
        let second = record(&store, "x", "v2 revised").unwrap();
        let all = list(&store).unwrap();
        assert_eq!(all.len(), 1, "same topic overwrites");
        assert_eq!(all[0].body, "v2 revised");
        assert_eq!(second.created_at, first.created_at, "created_at preserved across updates");
    }

    #[test]
    fn topic_is_sanitized() {
        let (_d, store) = store();
        let k = record(&store, "a/b c!", "x").unwrap();
        assert_eq!(k.topic, "a-b-c-");
        assert!(store.knowledge_dir().join("a-b-c-.md").exists());
    }
}

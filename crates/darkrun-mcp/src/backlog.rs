//! Project backlog — ideas not yet ready to become a Run (the `darkrun-backlog`
//! skill). Stored repo-level under `.darkrun/backlog/<id>.md`, independent of any
//! Run: a frontmatter fence (`created_at`) over the idea body.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;

use crate::error::Result;

/// One backlog idea.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BacklogItem {
    /// The item id (`item-NN`).
    pub id: String,
    /// RFC3339 capture time.
    pub created_at: String,
    /// The idea.
    pub description: String,
}

fn backlog_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".darkrun").join("backlog")
}

/// Every backlog item, oldest id first.
pub fn list(repo_root: &Path) -> Result<Vec<BacklogItem>> {
    let dir = backlog_dir(repo_root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut items: Vec<BacklogItem> = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(darkrun_core::CoreError::from)? {
        let path = entry.map_err(darkrun_core::CoreError::from)?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                let raw = std::fs::read_to_string(&path).map_err(darkrun_core::CoreError::from)?;
                items.push(parse(id.to_string(), &raw));
            }
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(items)
}

/// Add a backlog item from a description.
pub fn add(repo_root: &Path, description: &str) -> Result<BacklogItem> {
    let dir = backlog_dir(repo_root);
    std::fs::create_dir_all(&dir).map_err(darkrun_core::CoreError::from)?;
    let n = list(repo_root)?.len() + 1;
    let id = format!("item-{n:02}");
    let created_at = Utc::now().to_rfc3339();
    let doc = format!("---\ncreated_at: {created_at}\n---\n{}\n", description.trim());
    std::fs::write(dir.join(format!("{id}.md")), doc).map_err(darkrun_core::CoreError::from)?;
    Ok(BacklogItem {
        id,
        created_at,
        description: description.trim().to_string(),
    })
}

/// Promote an item out of the backlog: return it and remove its file (the caller
/// hands it to `darkrun-start`). Returns `None` if the id isn't found.
pub fn promote(repo_root: &Path, id: &str) -> Result<Option<BacklogItem>> {
    let path = backlog_dir(repo_root).join(format!("{id}.md"));
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).map_err(darkrun_core::CoreError::from)?;
    let item = parse(id.to_string(), &raw);
    std::fs::remove_file(&path).map_err(darkrun_core::CoreError::from)?;
    Ok(Some(item))
}

fn parse(id: String, doc: &str) -> BacklogItem {
    let doc = doc.trim_start_matches('\u{feff}');
    if let Some(rest) = doc.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let body = rest[end + 4..].trim_start_matches('\n').trim().to_string();
            let created_at = fm
                .lines()
                .find_map(|l| l.trim().strip_prefix("created_at:").map(|v| v.trim().to_string()))
                .unwrap_or_default();
            return BacklogItem {
                id,
                created_at,
                description: body,
            };
        }
    }
    BacklogItem {
        id,
        created_at: String::new(),
        description: doc.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_promote_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(list(root).unwrap().is_empty());

        let a = add(root, "  ship dark mode  ").unwrap();
        assert_eq!(a.id, "item-01");
        assert_eq!(a.description, "ship dark mode");
        let b = add(root, "rate limiting").unwrap();
        assert_eq!(b.id, "item-02");
        assert_eq!(list(root).unwrap().len(), 2);

        let promoted = promote(root, "item-01").unwrap().expect("found");
        assert_eq!(promoted.description, "ship dark mode");
        // Removed after promotion.
        assert_eq!(list(root).unwrap().len(), 1);
        assert!(promote(root, "item-01").unwrap().is_none());
    }
}

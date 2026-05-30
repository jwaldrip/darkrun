//! The override cascade: project filesystem override beats embedded default.
//!
//! A template key (`rel`, e.g. `phases/spec`) resolves in precedence order:
//!
//! 1. **Project override** — `<repo_root>/.darkrun/prompts/<rel>.md` on the
//!    filesystem. Lets a user override *any* prompt by dropping a file, no fork.
//!    Reads are cached by modification time so edits are picked up live without
//!    re-reading on every render.
//! 2. **Embedded default** — the `content/prompts/<rel>.md` corpus baked into the
//!    binary at compile time via [`rust_embed`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use rust_embed::RustEmbed;

use crate::error::{PromptError, Result};

/// The embedded `content/prompts/` corpus (workspace-root relative).
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../content/prompts"]
struct Corpus;

/// A cached project-override read: the file's source plus the mtime it was read
/// at, so a later read can refresh only when the file actually changed.
#[derive(Clone)]
struct CachedOverride {
    mtime: SystemTime,
    source: String,
}

/// Resolves template sources with the project-override cascade.
///
/// One `Cascade` is bound to a `repo_root`; clone it freely (the override cache
/// is shared behind a mutex). It is the single source of truth used by both the
/// public [`resolve`](crate::resolve) path and the minijinja loader, so
/// `{% include %}` honors overrides too.
pub struct Cascade {
    /// `<repo_root>/.darkrun/prompts`, where project overrides live.
    override_root: PathBuf,
    /// mtime-keyed cache of override file sources, keyed by `rel`.
    cache: Mutex<HashMap<String, CachedOverride>>,
}

impl Cascade {
    /// Build a cascade rooted at `repo_root`. Overrides are looked up under
    /// `<repo_root>/.darkrun/prompts/`.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            override_root: repo_root.as_ref().join(".darkrun").join("prompts"),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The filesystem path a `rel` override would live at.
    fn override_path(&self, rel: &str) -> PathBuf {
        self.override_root.join(format!("{rel}.md"))
    }

    /// Read the embedded default source for `rel`, if one exists.
    fn embedded(rel: &str) -> Option<String> {
        let path = format!("{rel}.md");
        let file = Corpus::get(&path)?;
        String::from_utf8(file.data.into_owned()).ok()
    }

    /// Read the project override for `rel`, honoring the mtime cache.
    ///
    /// Returns `Ok(None)` when no override file exists, `Ok(Some(src))` when one
    /// does (fresh or cached), or an error when an existing file can't be read.
    fn read_override(&self, rel: &str) -> Result<Option<String>> {
        let path = self.override_path(rel);
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            // No override file: not an error, the cascade falls through.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Drop any stale cache entry — the override was removed.
                self.cache.lock().expect("cascade cache poisoned").remove(rel);
                return Ok(None);
            }
            Err(source) => {
                return Err(PromptError::OverrideRead {
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        // Cache hit only when the mtime matches what we last read.
        {
            let cache = self.cache.lock().expect("cascade cache poisoned");
            if let Some(hit) = cache.get(rel) {
                if hit.mtime == mtime {
                    return Ok(Some(hit.source.clone()));
                }
            }
        }

        // Miss or stale: read fresh and refresh the cache.
        let source = std::fs::read_to_string(&path).map_err(|source| PromptError::OverrideRead {
            path: path.display().to_string(),
            source,
        })?;
        self.cache.lock().expect("cascade cache poisoned").insert(
            rel.to_string(),
            CachedOverride {
                mtime,
                source: source.clone(),
            },
        );
        Ok(Some(source))
    }

    /// Resolve the source for `rel`: project override first, embedded default
    /// second. Errors with [`PromptError::UnknownTemplate`] when neither exists.
    pub fn resolve(&self, rel: &str) -> Result<String> {
        if let Some(over) = self.read_override(rel)? {
            return Ok(over);
        }
        Self::embedded(rel).ok_or_else(|| PromptError::UnknownTemplate(rel.to_string()))
    }

    /// Loader-shaped resolve: `Ok(None)` for "no such template" (so minijinja
    /// reports its own not-found), `Err` only for a genuine read failure.
    pub fn resolve_for_loader(&self, rel: &str) -> Result<Option<String>> {
        if let Some(over) = self.read_override(rel)? {
            return Ok(Some(over));
        }
        Ok(Self::embedded(rel))
    }

    /// Every template key available from the embedded corpus (no `.md` suffix).
    /// Useful for tests and tooling that want to enumerate the corpus.
    pub fn embedded_keys() -> Vec<String> {
        let mut keys: Vec<String> = Corpus::iter()
            .filter_map(|p| p.as_ref().strip_suffix(".md").map(str::to_string))
            .collect();
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_corpus_has_every_phase() {
        let keys = Cascade::embedded_keys();
        for phase in [
            "phases/spec",
            "phases/review",
            "phases/manufacture",
            "phases/audit",
            "phases/reflect",
            "phases/checkpoint",
        ] {
            assert!(keys.contains(&phase.to_string()), "missing {phase}");
        }
    }

    #[test]
    fn embedded_resolves_when_no_override() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cascade::new(dir.path());
        let src = c.resolve("phases/spec").unwrap();
        assert!(src.contains("Spec"));
    }

    #[test]
    fn unknown_template_errors() {
        let dir = tempfile::tempdir().unwrap();
        let c = Cascade::new(dir.path());
        match c.resolve("phases/does-not-exist") {
            Err(PromptError::UnknownTemplate(k)) => assert_eq!(k, "phases/does-not-exist"),
            other => panic!("expected UnknownTemplate, got {other:?}"),
        }
    }
}

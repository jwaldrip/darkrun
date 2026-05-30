//! The override cascade: filesystem overrides beat the embedded default.
//!
//! A template key (`rel`, e.g. `phases/spec`) resolves in precedence order
//! (highest → lowest):
//!
//! 1. **Project override** — `<repo_root>/.darkrun/prompts/<rel>.md` on the
//!    filesystem. Lets a user override *any* prompt by dropping a file, no fork.
//!    Reads are cached by modification time so edits are picked up live without
//!    re-reading on every render.
//! 2. **Installed-plugin override** — `$CLAUDE_PLUGIN_ROOT/prompts/<rel>.md` on
//!    disk, when `CLAUDE_PLUGIN_ROOT` is set. Lets someone edit the *installed*
//!    plugin's prompts in place and have the change take effect without
//!    rebuilding the binary. Also mtime-cached.
//! 3. **Embedded default** — the `plugin/prompts/<rel>.md` corpus baked into the
//!    binary at compile time via [`rust_embed`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use rust_embed::RustEmbed;

use crate::error::{PromptError, Result};

/// The embedded `plugin/prompts/` corpus (relative to this crate).
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../plugin/prompts"]
struct Corpus;

/// A cached project-override read: the file's source plus the mtime it was read
/// at, so a later read can refresh only when the file actually changed.
#[derive(Clone)]
struct CachedOverride {
    mtime: SystemTime,
    source: String,
}

/// One filesystem override layer: a root directory plus its own mtime cache.
struct OverrideLayer {
    /// Directory under which `<rel>.md` override files live.
    root: PathBuf,
    /// mtime-keyed cache of override file sources, keyed by `rel`.
    cache: Mutex<HashMap<String, CachedOverride>>,
}

impl OverrideLayer {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The filesystem path a `rel` override would live at in this layer.
    fn path_for(&self, rel: &str) -> PathBuf {
        self.root.join(format!("{rel}.md"))
    }

    /// Read this layer's override for `rel`, honoring the mtime cache.
    ///
    /// Returns `Ok(None)` when no file exists, `Ok(Some(src))` when one does
    /// (fresh or cached), or an error when an existing file can't be read.
    fn read(&self, rel: &str) -> Result<Option<String>> {
        let path = self.path_for(rel);
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
}

/// Resolves template sources with the filesystem-override cascade.
///
/// One `Cascade` is bound to a `repo_root`; clone it freely (the override caches
/// are shared behind mutexes). It is the single source of truth used by both the
/// public [`resolve`](crate::resolve) path and the minijinja loader, so
/// `{% include %}` honors overrides too.
///
/// Filesystem layers are consulted highest-precedence first: the project
/// override (`<repo_root>/.darkrun/prompts`) beats the installed-plugin override
/// (`$CLAUDE_PLUGIN_ROOT/prompts`), which beats the embedded corpus.
pub struct Cascade {
    /// Ordered override layers, highest precedence first.
    layers: Vec<OverrideLayer>,
}

impl Cascade {
    /// Build a cascade rooted at `repo_root`. Project overrides are looked up
    /// under `<repo_root>/.darkrun/prompts/`; if `CLAUDE_PLUGIN_ROOT` is set, the
    /// installed plugin's `$CLAUDE_PLUGIN_ROOT/prompts/` is consulted next.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        let plugin_root = std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from);
        Self::with_roots(repo_root, plugin_root)
    }

    /// Build a cascade from explicit roots. The project override layer is always
    /// present at `<repo_root>/.darkrun/prompts`; the installed-plugin layer at
    /// `<plugin_root>/prompts` is added only when `plugin_root` is `Some`. The
    /// env-driven [`new`](Self::new) is the normal entry point; this exists so the
    /// plugin root can be supplied without mutating the process environment.
    pub fn with_roots(repo_root: impl AsRef<Path>, plugin_root: Option<PathBuf>) -> Self {
        let mut layers = vec![OverrideLayer::new(
            repo_root.as_ref().join(".darkrun").join("prompts"),
        )];
        if let Some(plugin_root) = plugin_root {
            layers.push(OverrideLayer::new(plugin_root.join("prompts")));
        }
        Self { layers }
    }

    /// Read the embedded default source for `rel`, if one exists.
    fn embedded(rel: &str) -> Option<String> {
        let path = format!("{rel}.md");
        let file = Corpus::get(&path)?;
        String::from_utf8(file.data.into_owned()).ok()
    }

    /// First filesystem override that has `rel`, walking layers highest-first.
    fn read_override(&self, rel: &str) -> Result<Option<String>> {
        for layer in &self.layers {
            if let Some(src) = layer.read(rel)? {
                return Ok(Some(src));
            }
        }
        Ok(None)
    }

    /// Resolve the source for `rel`: filesystem overrides first (project, then
    /// installed plugin), embedded default last. Errors with
    /// [`PromptError::UnknownTemplate`] when no layer has it.
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

    /// The installed-plugin layer (`$CLAUDE_PLUGIN_ROOT/prompts`) overrides the
    /// embedded default, and is itself overridden by `.darkrun/prompts` — so
    /// editing an installed plugin's prompts works without rebuilding, while a
    /// project override still wins.
    ///
    /// Uses [`Cascade::with_roots`] rather than mutating `CLAUDE_PLUGIN_ROOT`, so
    /// the process-global env stays untouched and parallel tests don't race.
    #[test]
    fn plugin_root_layer_overrides_embedded_but_not_project() {
        let repo = tempfile::tempdir().unwrap();
        let plugin = tempfile::tempdir().unwrap();

        // Installed-plugin override on disk.
        let plugin_prompt = plugin.path().join("prompts").join("phases").join("spec.md");
        std::fs::create_dir_all(plugin_prompt.parent().unwrap()).unwrap();
        std::fs::write(&plugin_prompt, "FROM PLUGIN ROOT").unwrap();

        // With no project override, the plugin-root disk layer beats embedded.
        let c = Cascade::with_roots(repo.path(), Some(plugin.path().to_path_buf()));
        assert_eq!(c.resolve("phases/spec").unwrap(), "FROM PLUGIN ROOT");

        // A project override beats the plugin-root layer.
        let proj_prompt = repo
            .path()
            .join(".darkrun")
            .join("prompts")
            .join("phases")
            .join("spec.md");
        std::fs::create_dir_all(proj_prompt.parent().unwrap()).unwrap();
        std::fs::write(&proj_prompt, "FROM PROJECT").unwrap();
        let c = Cascade::with_roots(repo.path(), Some(plugin.path().to_path_buf()));
        assert_eq!(c.resolve("phases/spec").unwrap(), "FROM PROJECT");
    }
}

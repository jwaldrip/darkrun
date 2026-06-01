//! Home discovery registry — `~/.darkrun/<slug>/engine-<pid>.json`.
//!
//! When a `darkrun mcp` engine boots it binds an EPHEMERAL loopback port (so
//! many engines coexist without colliding on a fixed port) and then advertises
//! itself by writing a small JSON [`EngineDescriptor`] under the user's home:
//!
//! ```text
//! ~/.darkrun/<slug>/engine-<pid>.json
//! ```
//!
//! The `<slug>` is derived from the engine's repo root so all engines for one
//! repo share a directory; the `<pid>` keeps concurrent engines for the SAME
//! repo from clobbering each other's descriptor. A discoverer (the desktop app)
//! scans this tree to find LIVE engines and the port each is listening on — no
//! fixed port, no environment handshake.
//!
//! Descriptors are RETAINED on exit, never deleted: a clean shutdown flags the
//! file stale (renames it to `engine-<pid>.json.stale`) and [`list_live_engines`]
//! additionally treats any descriptor whose pid is no longer running as stale.
//! Keeping the on-disk record (rather than deleting it) leaves a discoverable
//! trail for debugging and tolerates engines that die without running their
//! shutdown hook.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// One engine's discovery descriptor: the LIVE record a discoverer reads to find
/// a running `darkrun mcp` engine and the loopback port it serves on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineDescriptor {
    /// OS process id of the engine. Used to check liveness (signal 0) and to
    /// name the descriptor file so concurrent engines for one repo don't clash.
    pub pid: u32,
    /// The loopback address the engine's HTTP/WS server is listening on, with
    /// the ACTUAL (post-bind) port — never `0`.
    pub addr: SocketAddr,
    /// Absolute repo root the engine was launched against.
    pub repo_root: PathBuf,
    /// Slug derived from `repo_root`; matches the parent directory name.
    pub slug: String,
    /// Harness key the engine adapted to (e.g. the agent flavor), for display.
    pub harness: String,
    /// RFC3339 timestamp the descriptor was written at boot.
    pub started_at: String,
}

/// The registry rooted at `~/.darkrun`, owning the descriptor lifecycle for ONE
/// engine: derive the slug, write the boot descriptor, and (on shutdown) flag it
/// stale.
#[derive(Debug, Clone)]
pub struct EngineRegistry {
    /// Root of the discovery tree (`~/.darkrun`).
    root: PathBuf,
    /// Absolute repo root this engine serves; recorded in the descriptor.
    repo_root: PathBuf,
    /// Slug for this engine's repo (the `<slug>` directory name).
    slug: String,
    /// This engine's pid.
    pid: u32,
}

impl EngineRegistry {
    /// Build a registry for `repo_root` rooted at the default `~/.darkrun`,
    /// deriving the slug and capturing the current pid.
    ///
    /// Fails only when the home directory can't be resolved.
    pub fn new(repo_root: impl AsRef<Path>) -> io::Result<Self> {
        let root = default_root().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "could not resolve home directory for the darkrun discovery registry",
            )
        })?;
        Ok(Self::with_root(root, repo_root))
    }

    /// Like [`new`](Self::new) but with an explicit registry `root`. Used by
    /// tests to point the tree at a temp dir.
    pub fn with_root(root: impl Into<PathBuf>, repo_root: impl AsRef<Path>) -> Self {
        let repo_root = repo_root.as_ref().to_path_buf();
        Self {
            root: root.into(),
            slug: slug_for(&repo_root),
            repo_root,
            pid: std::process::id(),
        }
    }

    /// The slug directory for this engine's repo (`<root>/<slug>`).
    pub fn slug_dir(&self) -> PathBuf {
        self.root.join(&self.slug)
    }

    /// The live descriptor path for this engine (`<slug_dir>/engine-<pid>.json`).
    pub fn descriptor_path(&self) -> PathBuf {
        self.slug_dir().join(format!("engine-{}.json", self.pid))
    }

    /// Write the boot descriptor advertising `addr` (the ACTUAL bound port) and
    /// `harness`, creating the slug directory if needed.
    ///
    /// Returns the descriptor written. Best-effort: callers treat a failure as
    /// non-fatal (the engine still serves; it just isn't auto-discoverable).
    pub fn announce(&self, addr: SocketAddr, harness: &str) -> io::Result<EngineDescriptor> {
        fs::create_dir_all(self.slug_dir())?;
        let descriptor = EngineDescriptor {
            pid: self.pid,
            addr,
            repo_root: self.repo_root.clone(),
            slug: self.slug.clone(),
            harness: harness.to_string(),
            started_at: now_rfc3339(),
        };
        let json = serde_json::to_vec_pretty(&descriptor)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(self.descriptor_path(), json)?;
        Ok(descriptor)
    }

    /// Flag this engine's descriptor stale on exit by renaming it to
    /// `engine-<pid>.json.stale`. RETAINS the record (never deletes it).
    ///
    /// Idempotent and best-effort: a missing descriptor (already flagged, or
    /// never written) is a no-op.
    pub fn mark_stale(&self) -> io::Result<()> {
        let live = self.descriptor_path();
        if !live.exists() {
            return Ok(());
        }
        let stale = stale_path(&live);
        fs::rename(&live, &stale)
    }
}

/// Resolve the default discovery root `~/.darkrun`.
///
/// Uses the `dirs` crate's home-directory resolution, falling back to the same
/// `$HOME` / `$USERPROFILE` env vars the rest of darkrun relies on.
pub fn default_root() -> Option<PathBuf> {
    dirs::home_dir()
        .or_else(home_dir_env)
        .map(|home| home.join(".darkrun"))
}

/// Env-var fallback mirroring the resolution used elsewhere in darkrun.
fn home_dir_env() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Derive the `<slug>` directory name for `repo_root`.
///
/// The slug is the sanitized basename of the path; to keep slugs unique across
/// different repos that share a basename (e.g. two `app` checkouts), a short
/// hash of the FULL path is appended. The result is a filesystem-safe,
/// collision-resistant directory name.
pub fn slug_for(repo_root: &Path) -> String {
    let base = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "root".to_string());
    let sanitized = sanitize(&base);
    let hash = short_hash(repo_root);
    format!("{sanitized}-{hash}")
}

/// Replace any character that isn't `[A-Za-z0-9._-]` with `-`, collapsing the
/// result so it's safe as a single path component.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    // Avoid leading dots so the slug dir isn't accidentally hidden, and trim
    // dashes for a tidy name.
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Short hex digest of the full repo path, for slug uniqueness.
fn short_hash(repo_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    // First 4 bytes as hex (8 chars) is plenty to disambiguate basenames.
    let mut s = String::with_capacity(8);
    for byte in &digest[..4] {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// The `.stale` sibling path for a live descriptor.
fn stale_path(live: &Path) -> PathBuf {
    let mut name = live
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".stale");
    live.with_file_name(name)
}

/// Read every LIVE engine descriptor under the default `~/.darkrun` tree.
///
/// A descriptor is live when its file is the active `engine-<pid>.json` (not a
/// `.stale` sibling) AND its pid is still running. Stale-but-running is ignored
/// (already flagged); live-but-dead is ignored (engine vanished without a clean
/// shutdown). Returns an empty list when the tree doesn't exist.
pub fn list_live_engines() -> io::Result<Vec<EngineDescriptor>> {
    match default_root() {
        Some(root) => list_live_engines_in(&root),
        None => Ok(Vec::new()),
    }
}

/// Like [`list_live_engines`] but scans an explicit `root`. Used by tests.
pub fn list_live_engines_in(root: &Path) -> io::Result<Vec<EngineDescriptor>> {
    let mut live = Vec::new();
    let slug_dirs = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(live),
        Err(e) => return Err(e),
    };
    for slug_entry in slug_dirs.flatten() {
        let slug_path = slug_entry.path();
        if !slug_path.is_dir() {
            continue;
        }
        let descriptors = match fs::read_dir(&slug_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in descriptors.flatten() {
            let path = entry.path();
            if !is_live_descriptor_name(&path) {
                continue;
            }
            let Ok(bytes) = fs::read(&path) else { continue };
            let Ok(descriptor) = serde_json::from_slice::<EngineDescriptor>(&bytes) else {
                continue;
            };
            if process_alive(descriptor.pid) {
                live.push(descriptor);
            }
        }
    }
    Ok(live)
}

/// True when `path` is an active `engine-*.json` descriptor (not a `.stale`
/// sibling and not some other file).
fn is_live_descriptor_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    name.starts_with("engine-") && name.ends_with(".json")
}

/// Check whether process `pid` is currently running.
///
/// On Unix this sends signal 0 (the no-op liveness probe): `Ok` or
/// `EPERM` mean the process exists; `ESRCH` means it's gone. On other platforms
/// we can't cheaply probe, so we conservatively report `true` (the caller falls
/// back to treating descriptors as live until a clean shutdown flags them).
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    matches!(
        kill(Pid::from_raw(pid as i32), None),
        Ok(()) | Err(Errno::EPERM)
    )
}

/// See the Unix implementation. On non-Unix targets we can't cheaply probe, so
/// conservatively report `true`.
#[cfg(not(unix))]
pub fn process_alive(_pid: u32) -> bool {
    true
}

/// RFC3339 timestamp for `started_at`.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    fn sample_addr() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4317))
    }

    #[test]
    fn test_slug_derivation_simple() {
        // The basename leads the slug; a hash suffix follows.
        let slug = slug_for(Path::new("/Users/dev/darkrun"));
        assert!(slug.starts_with("darkrun-"), "slug was {slug}");
        // Hash suffix is 8 hex chars.
        let suffix = slug.strip_prefix("darkrun-").unwrap();
        assert_eq!(suffix.len(), 8);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_slug_derivation_sanitizes() {
        // Spaces and special chars become dashes; same basename + different path
        // yields a different hash suffix (collision-resistant).
        let a = slug_for(Path::new("/home/alice/My App!"));
        let b = slug_for(Path::new("/home/bob/My App!"));
        assert!(a.starts_with("My-App-"), "slug was {a}");
        assert!(b.starts_with("My-App-"), "slug was {b}");
        assert_ne!(a, b, "different paths must not collide");
        // No illegal path characters survive.
        assert!(!a.contains(' '));
        assert!(!a.contains('!'));
        assert!(!a.contains('/'));
    }

    #[test]
    fn test_descriptor_roundtrip() {
        let descriptor = EngineDescriptor {
            pid: 4242,
            addr: sample_addr(),
            repo_root: PathBuf::from("/Users/dev/darkrun"),
            slug: "darkrun-deadbeef".to_string(),
            harness: "claude".to_string(),
            started_at: "2026-05-31T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_vec(&descriptor).unwrap();
        let back: EngineDescriptor = serde_json::from_slice(&json).unwrap();
        assert_eq!(descriptor, back);
    }

    #[test]
    fn test_stale_detection() {
        // The current process is alive; an almost-certainly-dead pid is not.
        assert!(process_alive(std::process::id()));
        // A very large pid is exceedingly unlikely to be a live process.
        assert!(!process_alive(0x7fff_fffe));
    }

    #[test]
    fn test_announce_and_list_live() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = EngineRegistry::with_root(tmp.path(), "/Users/dev/some-repo");
        let descriptor = registry.announce(sample_addr(), "claude").unwrap();

        assert!(registry.descriptor_path().exists());
        assert_eq!(descriptor.addr, sample_addr());

        // The live reader returns the descriptor while this process is alive.
        let live = list_live_engines_in(tmp.path()).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].pid, std::process::id());
        assert_eq!(live[0].addr, sample_addr());
    }

    #[test]
    fn test_mark_stale_retains_record() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = EngineRegistry::with_root(tmp.path(), "/Users/dev/another-repo");
        registry.announce(sample_addr(), "claude").unwrap();
        let live_path = registry.descriptor_path();
        assert!(live_path.exists());

        registry.mark_stale().unwrap();

        // Live descriptor is gone, but the record is RETAINED as `.stale`.
        assert!(!live_path.exists());
        let stale = stale_path(&live_path);
        assert!(stale.exists(), "stale record must be retained, not deleted");

        // A stale descriptor is no longer returned by the live reader.
        let live = list_live_engines_in(tmp.path()).unwrap();
        assert!(live.is_empty());

        // mark_stale is idempotent.
        registry.mark_stale().unwrap();
    }
}

//! The on-disk credential store: `~/.darkrun/credentials`, keyed by provider.
//!
//! The file is a JSON object mapping provider key → [`Credential`]. On unix the
//! file is created/maintained at mode `0600` so tokens are not world-readable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Result, VcsError};
use crate::provider::{Credential, Provider};

/// File mode applied to the credentials file on unix.
#[cfg(unix)]
const CRED_MODE: u32 = 0o600;

/// Persists OAuth credentials keyed by [`Provider`].
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

/// The serialized on-disk shape: provider key → credential.
type CredMap = BTreeMap<String, Credential>;

impl CredentialStore {
    /// Open the store at an explicit path (used by tests and custom homes).
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Open the default store at `~/.darkrun/credentials`.
    ///
    /// The home directory is resolved from `$HOME` (unix) / `$USERPROFILE`
    /// (windows). No directories are created until a save occurs.
    pub fn default_path() -> Result<Self> {
        let home = home_dir()
            .ok_or_else(|| VcsError::CredentialsPath("no home directory found".to_string()))?;
        Ok(Self::at(home.join(".darkrun").join("credentials")))
    }

    /// The path this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the full credential map, returning an empty map if the file is
    /// absent. Propagates I/O and parse errors otherwise.
    fn load_map(&self) -> Result<CredMap> {
        match std::fs::read(&self.path) {
            Ok(bytes) if bytes.is_empty() => Ok(CredMap::new()),
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CredMap::new()),
            Err(e) => Err(VcsError::from(e)),
        }
    }

    /// Atomically write the credential map to disk at `0600` (unix).
    fn write_map(&self, map: &CredMap) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(map)?;

        // Write to a temp sibling then rename for atomicity, applying the
        // restrictive mode before any token bytes land on disk.
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.path)?;
        enforce_mode(&self.path)?;
        Ok(())
    }

    /// Fetch the credential for `provider`, if present.
    pub fn get(&self, provider: Provider) -> Result<Option<Credential>> {
        Ok(self.load_map()?.remove(provider.key()))
    }

    /// Insert or replace the credential for its provider.
    pub fn save(&self, credential: &Credential) -> Result<()> {
        let mut map = self.load_map()?;
        map.insert(credential.provider.key().to_string(), credential.clone());
        self.write_map(&map)
    }

    /// Remove the credential for `provider`. Returns `true` if one was removed.
    pub fn remove(&self, provider: Provider) -> Result<bool> {
        let mut map = self.load_map()?;
        let removed = map.remove(provider.key()).is_some();
        if removed {
            self.write_map(&map)?;
        }
        Ok(removed)
    }

    /// List every provider that currently has a stored credential.
    pub fn list(&self) -> Result<Vec<Provider>> {
        let map = self.load_map()?;
        Ok(map
            .keys()
            .filter_map(|k| Provider::from_key(k))
            .collect())
    }
}

/// Resolve the user's home directory without pulling in an extra dependency.
fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Write `bytes` to `path`, creating it `0600` on unix.
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(CRED_MODE)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// Write `bytes` to `path` (non-unix: mode not enforced).
#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Re-assert the restrictive mode after a rename (rename can preserve the temp
/// file's mode, but a pre-existing destination's mode would otherwise survive).
#[cfg(unix)]
fn enforce_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(CRED_MODE);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

/// No-op on non-unix.
#[cfg(not(unix))]
fn enforce_mode(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use crate::provider::Provider;

    #[test]
    fn missing_credential_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::at(dir.path().join("does-not-exist.json"));
        assert!(store.get(Provider::GitHub).unwrap().is_none());
        assert!(store.path().ends_with("does-not-exist.json"));
    }

    #[test]
    fn empty_credential_file_loads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds.json");
        // A present-but-empty (0-byte) file is treated as an empty map, not a
        // parse error — the empty-bytes fast path in load_map.
        std::fs::write(&path, b"").unwrap();
        let store = CredentialStore::at(&path);
        assert!(store.get(Provider::GitHub).unwrap().is_none());
    }
}

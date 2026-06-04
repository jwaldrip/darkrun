//! Content hashing — the baseline the drift sweep compares input premises
//! against. A premise witness is a `path -> sha256` entry on the consuming unit
//! (`UnitFrontmatter.input_witnesses`); these helpers compute the digests.

use std::path::Path;

use sha2::{Digest, Sha256};

/// The SHA-256 hex digest of a file's content, or `None` if the file is missing
/// or unreadable (a missing locked artifact is itself drift).
pub fn hash_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hash_bytes(&bytes))
}

/// The SHA-256 hex digest of a byte slice.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_distinct() {
        let a = hash_bytes(b"hello");
        assert_eq!(a, hash_bytes(b"hello"));
        assert_ne!(a, hash_bytes(b"world"));
        // SHA-256 hex is 64 chars.
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hash_file_none_when_missing() {
        assert!(hash_file(Path::new("/nonexistent/darkrun/witness")).is_none());
    }

    #[test]
    fn hash_file_reads_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"content").unwrap();
        assert_eq!(hash_file(&p), Some(hash_bytes(b"content")));
    }
}

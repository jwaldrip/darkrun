//! Error type for darkrun-git.

use std::path::PathBuf;
use std::process::ExitStatus;

/// Errors produced by git worktree operations.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// The path is not a git repository (or could not be opened as one).
    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),

    /// A worktree with the requested name already exists.
    #[error("worktree already exists: {0}")]
    WorktreeExists(String),

    /// The requested worktree was not found.
    #[error("worktree not found: {0}")]
    WorktreeNotFound(String),

    /// A `git` subprocess exited non-zero (shell-out fallback path).
    #[error("git {args:?} failed ({status}): {stderr}")]
    Command {
        /// The argument vector passed to `git`.
        args: Vec<String>,
        /// The process exit status.
        status: ExitStatus,
        /// Captured stderr, trimmed.
        stderr: String,
    },

    /// An I/O operation failed against a path.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path the operation targeted.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A bare I/O error with no associated path.
    #[error("io error: {0}")]
    BareIo(#[from] std::io::Error),

    /// A network `git` subprocess exceeded its wall-clock ceiling and was killed
    /// (an unresponsive remote) — surfaced so a tick fails fast instead of hanging.
    #[error("git {args:?} timed out after {secs}s (killed)")]
    Timeout {
        /// The argument vector passed to `git`.
        args: Vec<String>,
        /// The ceiling that elapsed, in seconds.
        secs: u64,
    },

    /// An error surfaced from the pure-Rust gitoxide backend.
    #[error("gix error: {0}")]
    Gix(String),
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, GitError>;

//! Error type for the darkrun-core state engine.

use std::path::PathBuf;

/// Errors produced by the darkrun-core state engine.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// An I/O operation failed against a state-directory path.
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

    /// YAML frontmatter failed to (de)serialize.
    #[error("frontmatter yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// JSON state failed to (de)serialize.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A document was missing its frontmatter delimiters.
    #[error("missing frontmatter: a document must open with a `---` fence")]
    MissingFrontmatter,

    /// A requested run was not found on disk.
    #[error("run not found: {0}")]
    RunNotFound(String),

    /// A requested unit was not found on disk.
    #[error("unit not found: {0}")]
    UnitNotFound(String),

    /// A requested annotation was not found on disk.
    #[error("annotation not found: {0}")]
    AnnotationNotFound(String),

    /// The unit dependency graph contains a cycle.
    #[error("circular dependency detected among units: {0}")]
    CyclicDependency(String),

    /// A lock could not be acquired before the timeout elapsed.
    #[error("lock acquire timed out for {name} after {timeout_ms}ms")]
    LockTimeout {
        /// The lock name.
        name: String,
        /// The configured timeout in milliseconds.
        timeout_ms: u64,
    },
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, CoreError>;

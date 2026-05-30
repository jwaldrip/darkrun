//! The darkrun-mcp error type.

use darkrun_core::CoreError;

/// Errors raised by the manager and MCP tool handlers.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// A `darkrun-core` state/filesystem error.
    #[error(transparent)]
    Core(#[from] CoreError),

    /// The run named a factory this build does not know.
    #[error("unknown factory: {0}")]
    UnknownFactory(String),

    /// The state referenced a station not present in the factory plan.
    #[error("unknown station: {0}")]
    UnknownStation(String),

    /// A checkpoint decision arrived but no station is active.
    #[error("run '{0}' has no active station")]
    NoActiveStation(String),

    /// A tool received invalid input.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// A feedback item was referenced by an id that does not exist.
    #[error("feedback not found: {0}")]
    FeedbackNotFound(String),

    /// A mutation targeted a settled (terminal) feedback item, which is
    /// immutable.
    #[error("feedback '{0}' is settled and cannot be modified")]
    FeedbackSettled(String),

    /// A unit was referenced by a slug that does not exist.
    #[error("unit not found: {0}")]
    UnitNotFound(String),

    /// Rendering an engine-driven prompt template failed (unknown template,
    /// override read error, or a template syntax/render fault).
    #[error("prompt render failed: {0}")]
    Prompt(String),
}

/// Crate result alias.
pub type Result<T> = std::result::Result<T, McpError>;

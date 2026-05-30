//! Errors raised by the prompt engine.

use thiserror::Error;

/// The result type for the prompt engine.
pub type Result<T> = std::result::Result<T, PromptError>;

/// Failures resolving or rendering a prompt template.
#[derive(Debug, Error)]
pub enum PromptError {
    /// No template exists for the requested key — neither a project override
    /// at `.darkrun/prompts/<rel>.md` nor an embedded default.
    #[error("unknown prompt template: `{0}` (no project override and no embedded default)")]
    UnknownTemplate(String),

    /// A project-override file existed but could not be read.
    #[error("failed to read prompt override `{path}`: {source}")]
    OverrideRead {
        /// The override path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The serde-serializable context could not be turned into a value the
    /// template engine can consume.
    #[error("failed to serialize render context: {0}")]
    Context(#[source] serde_json::Error),

    /// The template engine failed to render the resolved template (syntax error,
    /// missing include, runtime error in the template).
    #[error("failed to render prompt `{rel}`: {source}")]
    Render {
        /// The template key being rendered.
        rel: String,
        /// The underlying minijinja error.
        #[source]
        source: minijinja::Error,
    },
}

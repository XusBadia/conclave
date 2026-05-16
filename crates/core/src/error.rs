//! Error and result types shared across Conclave crates.

use std::path::PathBuf;

/// Convenience alias used throughout the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Top-level error type for the Conclave workspace.
///
/// Domain crates expose their own variants via [`Error::Provider`],
/// [`Error::Rag`] and [`Error::Deident`] so that callers can pattern-match on
/// a single, stable error type without losing context.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// I/O failure with the offending path attached when known.
    #[error("I/O error at {path:?}: {source}")]
    Io {
        /// Filesystem path involved in the failure, if any.
        path: Option<PathBuf>,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// TOML deserialization failure (typically when loading config).
    #[error("failed to parse TOML: {0}")]
    TomlDe(#[from] toml::de::Error),

    /// TOML serialization failure (typically when persisting config).
    #[error("failed to serialize TOML: {0}")]
    TomlSer(#[from] toml::ser::Error),

    /// Configuration is structurally valid but semantically invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// OS-standard application directories could not be resolved.
    #[error("could not resolve OS-standard application directories")]
    MissingAppDirs,

    /// Requested workspace could not be located by id or name.
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(String),

    /// Workspace cannot be created because the slug is already in use.
    #[error("workspace already exists: {0}")]
    WorkspaceExists(String),

    /// Error originating in the providers layer (LLM I/O, auth, etc.).
    #[error("provider error: {0}")]
    Provider(String),

    /// Error originating in the RAG pipeline (ingestion, search, embeddings).
    #[error("rag error: {0}")]
    Rag(String),

    /// Error originating in the de-identification pipeline.
    #[error("deident error: {0}")]
    Deident(String),

    /// A required feature is not yet implemented (Phase 0 placeholder).
    #[error("not yet implemented: {0}")]
    Unimplemented(&'static str),
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::Io { path: None, source }
    }
}

impl Error {
    /// Attach the path involved in an I/O failure.
    pub fn io_at(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: Some(path.into()),
            source,
        }
    }

    /// Build an [`Error::InvalidConfig`] from any displayable value.
    pub fn invalid_config(msg: impl Into<String>) -> Self {
        Self::InvalidConfig(msg.into())
    }
}

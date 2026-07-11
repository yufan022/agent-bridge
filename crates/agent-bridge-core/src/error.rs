//! Error types for agent-bridge-core.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by adapters, converters, and the sync engine.
#[derive(Debug, Error)]
pub enum Error {
    #[error("home directory could not be resolved")]
    HomeNotFound,

    #[error("unsupported tool id: {0}")]
    UnknownTool(String),

    #[error("source and target tools must differ (got {0})")]
    SameSourceAndTarget(String),

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to parse TOML at {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to edit TOML at {path}: {source}")]
    TomlEdit {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },

    #[error("invalid MCP config at {path}: {message}")]
    InvalidMcp { path: PathBuf, message: String },

    #[error("skill conflict for '{name}' at {path}: {message}")]
    SkillConflict {
        name: String,
        path: PathBuf,
        message: String,
    },

    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn json(path: impl Into<PathBuf>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.into(),
            source,
        }
    }

    pub fn toml(path: impl Into<PathBuf>, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.into(),
            source,
        }
    }

    pub fn invalid_mcp(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::InvalidMcp {
            path: path.into(),
            message: message.into(),
        }
    }
}

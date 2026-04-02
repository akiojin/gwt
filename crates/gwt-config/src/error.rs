//! Error types for gwt-config.

use thiserror::Error;

/// Result alias for gwt-config operations.
pub type Result<T> = std::result::Result<T, ConfigError>;

/// Errors that can occur during configuration operations.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to parse a configuration file.
    #[error("config parse error: {reason}")]
    ParseError { reason: String },

    /// Failed to write a configuration file.
    #[error("config write error: {reason}")]
    WriteError { reason: String },

    /// The global config path could not be determined.
    #[error("could not determine global config path")]
    NoConfigPath,

    /// An I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Validation failed.
    #[error("validation error: {reason}")]
    ValidationError { reason: String },
}

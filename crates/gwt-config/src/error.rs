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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_display() {
        let err = ConfigError::ParseError {
            reason: "bad toml".into(),
        };
        assert_eq!(err.to_string(), "config parse error: bad toml");
    }

    #[test]
    fn write_error_display() {
        let err = ConfigError::WriteError {
            reason: "disk full".into(),
        };
        assert_eq!(err.to_string(), "config write error: disk full");
    }

    #[test]
    fn no_config_path_display() {
        let err = ConfigError::NoConfigPath;
        assert_eq!(err.to_string(), "could not determine global config path");
    }

    #[test]
    fn io_error_from_std() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let err: ConfigError = io_err.into();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn validation_error_display() {
        let err = ConfigError::ValidationError {
            reason: "missing field".into(),
        };
        assert_eq!(err.to_string(), "validation error: missing field");
    }
}

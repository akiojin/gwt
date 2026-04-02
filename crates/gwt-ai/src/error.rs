//! AI-specific error types.

use thiserror::Error;

/// Errors that can occur during AI operations.
#[derive(Debug, Error)]
pub enum AIError {
    /// Configuration is missing or invalid (e.g. empty endpoint, missing API key).
    #[error("config error: {0}")]
    ConfigError(String),

    /// The API returned a 429 rate-limit response.
    #[error("rate limited (retry after {retry_after_secs:?}s)")]
    RateLimited {
        /// Optional hint from the server about how long to wait.
        retry_after_secs: Option<u64>,
    },

    /// The API returned a 5xx server error.
    #[error("server error: {0}")]
    ServerError(String),

    /// Failed to parse the API response into the expected shape.
    #[error("parse error: {0}")]
    ParseError(String),

    /// A network-level failure (DNS, TLS, connection refused, etc.).
    #[error("network error: {0}")]
    NetworkError(String),

    /// The request timed out.
    #[error("timeout: {0}")]
    Timeout(String),

    /// The AI returned an incomplete summary (truncated or empty).
    #[error("incomplete summary: {0}")]
    IncompleteSummary(String),
}

//! Logger initialization

use crate::error::{ErrorCategory, GwtError, Result};
use std::path::PathBuf;
use tracing::error;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

/// Logger configuration
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Log directory
    pub log_dir: PathBuf,
    /// Workspace name (for subdirectory)
    pub workspace: String,
    /// Enable debug output
    pub debug: bool,
    /// Log retention days
    pub retention_days: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: dirs_default_log_dir(),
            workspace: "default".to_string(),
            debug: false,
            retention_days: 7,
        }
    }
}

fn dirs_default_log_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "gwt")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".gwt/logs"))
}

/// Initialize the logger with JSON Lines output (Pino compatible)
pub fn init_logger(config: &LogConfig) -> Result<()> {
    let log_dir = config.log_dir.join(&config.workspace);
    std::fs::create_dir_all(&log_dir)?;

    // Create rolling file appender (daily rotation)
    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, "gwt.jsonl");

    // Create JSON layer for file output
    let file_layer = fmt::layer()
        .json()
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(file_appender)
        .with_ansi(false);

    // Create console layer for debug output
    let console_layer = if config.debug {
        Some(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(false)
                .with_file(true)
                .with_line_number(true),
        )
    } else {
        None
    };

    // Set up filter (RUST_LOG takes precedence when present)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if config.debug {
            EnvFilter::new("gwt=debug,info")
        } else {
            EnvFilter::new("gwt=info,warn")
        }
    });

    // Initialize subscriber
    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()
        .ok(); // Ignore if already initialized

    Ok(())
}

/// Log a GwtError with full context (code, category, message, details)
pub fn log_gwt_error(err: &GwtError, details: Option<&str>) {
    let code = err.code();
    let category = err.category();
    let message = err.to_string();

    match category {
        ErrorCategory::Git => {
            error!(
                code = %code,
                category = "git",
                error_message = %message,
                details = details.unwrap_or(""),
                "Git operation error"
            );
        }
        ErrorCategory::Worktree => {
            error!(
                code = %code,
                category = "worktree",
                error_message = %message,
                details = details.unwrap_or(""),
                "Worktree operation error"
            );
        }
        ErrorCategory::Config => {
            error!(
                code = %code,
                category = "config",
                error_message = %message,
                details = details.unwrap_or(""),
                "Configuration error"
            );
        }
        ErrorCategory::Agent => {
            error!(
                code = %code,
                category = "agent",
                error_message = %message,
                details = details.unwrap_or(""),
                "Agent error"
            );
        }
        ErrorCategory::WebApi => {
            error!(
                code = %code,
                category = "webapi",
                error_message = %message,
                details = details.unwrap_or(""),
                "Web API error"
            );
        }
        ErrorCategory::Docker => {
            error!(
                code = %code,
                category = "docker",
                error_message = %message,
                details = details.unwrap_or(""),
                "Docker operation error"
            );
        }
        ErrorCategory::Internal => {
            error!(
                code = %code,
                category = "internal",
                error_message = %message,
                details = details.unwrap_or(""),
                "Internal error"
            );
        }
    }
}

/// Log an error message with code (for use when GwtError is not available)
pub fn log_error_message(code: &str, category: &str, message: &str, details: Option<&str>) {
    error!(
        code = %code,
        category = %category,
        error_message = %message,
        details = details.unwrap_or(""),
        "Error occurred"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_logger() {
        let temp = TempDir::new().unwrap();
        let config = LogConfig {
            log_dir: temp.path().to_path_buf(),
            workspace: "test".to_string(),
            debug: false,
            retention_days: 7,
        };

        // Should not panic
        init_logger(&config).unwrap();

        // Log directory should be created
        assert!(temp.path().join("test").exists());
    }
}

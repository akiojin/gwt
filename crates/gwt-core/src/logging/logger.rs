//! Logger initialization

use crate::error::{ErrorCategory, GwtError, Result};
use std::path::PathBuf;
use tracing::error;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_chrome::ChromeLayerBuilder;
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
    /// Enable performance profiling (Chrome Trace Event Format output)
    pub profiling: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            log_dir: dirs_default_log_dir(),
            workspace: "default".to_string(),
            debug: false,
            retention_days: 7,
            profiling: false,
        }
    }
}

fn dirs_default_log_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "gwt")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".gwt/logs"))
}

/// Guard that flushes the Chrome Trace profiling file on drop.
///
/// Must be held alive for the entire application lifetime when profiling is enabled.
/// When dropped, the trace file is flushed and finalized.
pub struct ProfilingGuard {
    _guard: Option<tracing_chrome::FlushGuard>,
}

/// Initialize the logger with JSON Lines output (Pino compatible)
///
/// Returns a [`ProfilingGuard`] that must be kept alive for the application lifetime.
/// When profiling is enabled, dropping the guard flushes the Chrome Trace output file.
pub fn init_logger(config: &LogConfig) -> Result<ProfilingGuard> {
    let log_dir = config.log_dir.join(&config.workspace);
    std::fs::create_dir_all(&log_dir)?;

    // Create rolling file appender (daily rotation)
    let file_appender = RollingFileAppender::new(Rotation::DAILY, &log_dir, "gwt.jsonl");

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

    // Create Chrome Trace profiling layer (when enabled)
    let (chrome_layer, chrome_guard) = if config.profiling {
        let trace_path = log_dir.join("profile.json");
        let (layer, guard) = ChromeLayerBuilder::new()
            .file(trace_path)
            .include_args(true)
            .build();
        (Some(layer), Some(guard))
    } else {
        (None, None)
    };

    // Set up filter (RUST_LOG takes precedence when present)
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if config.debug || config.profiling {
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
        .with(chrome_layer)
        .try_init()
        .ok(); // Ignore if already initialized

    Ok(ProfilingGuard {
        _guard: chrome_guard,
    })
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
        ErrorCategory::Terminal => {
            error!(
                code = %code,
                category = "terminal",
                error_message = %message,
                details = details.unwrap_or(""),
                "Terminal operation error"
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

/// Initialize a lightweight tracing subscriber for tests.
///
/// Uses `try_init()` so it silently succeeds even when another test in the
/// same process has already initialized the global subscriber.
#[cfg(test)]
pub fn init_test_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("gwt=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_test_writer())
        .try_init()
        .ok();
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
            profiling: false,
        };

        // Should not panic
        init_logger(&config).unwrap();

        // Log directory should be created
        assert!(temp.path().join("test").exists());
    }

    #[test]
    fn test_init_logger_with_profiling() {
        let temp = TempDir::new().unwrap();
        let config = LogConfig {
            log_dir: temp.path().to_path_buf(),
            workspace: "profiling_test".to_string(),
            debug: false,
            retention_days: 7,
            profiling: true,
        };

        let guard = init_logger(&config).unwrap();

        // Log directory should be created
        let log_dir = temp.path().join("profiling_test");
        assert!(log_dir.exists());

        // profile.json should be created when profiling is enabled
        let profile_path = log_dir.join("profile.json");
        assert!(
            profile_path.exists(),
            "profile.json should be created when profiling=true"
        );

        // Drop guard to flush the trace file
        drop(guard);

        // After flush, profile.json should have content
        let content = std::fs::read_to_string(&profile_path).unwrap();
        assert!(
            !content.is_empty(),
            "profile.json should have content after flush"
        );
    }
}

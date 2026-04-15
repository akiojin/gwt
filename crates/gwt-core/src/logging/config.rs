//! Logging configuration: level resolution from env / config / default.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Ordered severity level compatible with the old `gwt_notification::Severity`.
///
/// Variants are intentionally ordered `Debug < Info < Warn < Error` so that
/// comparison operators work for severity filters (`>= LogLevel::Warn` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn ordinal(self) -> u8 {
        match self {
            Self::Debug => 0,
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
        }
    }

    /// Return the directive string for `tracing_subscriber::EnvFilter::new`.
    ///
    /// Maps our four-level severity onto tracing's five-level model. `Debug`
    /// becomes `debug`, everything else becomes the corresponding tracing
    /// level name.
    pub fn to_env_directive(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Convert from a `tracing::Level` emitted by the fmt/forwarder layers.
    pub fn from_tracing(level: tracing::Level) -> Self {
        match level {
            tracing::Level::ERROR => Self::Error,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::INFO => Self::Info,
            tracing::Level::DEBUG | tracing::Level::TRACE => Self::Debug,
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordinal().cmp(&other.ordinal())
    }
}

impl FromStr for LogLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "debug" | "trace" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            other => Err(format!("unknown log level: {other}")),
        }
    }
}

/// Runtime logging configuration passed to `init`.
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Destination directory for the rolling log files (`gwt.log`, `gwt.log.YYYY-MM-DD`).
    pub log_dir: std::path::PathBuf,
    /// Initial level used when neither `RUST_LOG` nor a config-file override is present.
    pub default_level: LogLevel,
    /// Optional level read from `config.toml`. Ignored when `RUST_LOG` is set.
    pub config_file_level: Option<LogLevel>,
    /// Housekeeping retention window in days. Files older than this are
    /// deleted on startup. Pass `0` to disable.
    pub retention_days: u32,
}

impl LoggingConfig {
    /// Build a config with the standard default level (`INFO`) and 7-day retention.
    pub fn new(log_dir: std::path::PathBuf) -> Self {
        Self {
            log_dir,
            default_level: LogLevel::Info,
            config_file_level: None,
            retention_days: 7,
        }
    }

    /// Override the config-file level (used by Settings UI and
    /// `config.toml` round-trip).
    pub fn with_config_file_level(mut self, level: Option<LogLevel>) -> Self {
        self.config_file_level = level;
        self
    }

    /// Return the effective initial `EnvFilter` directive.
    ///
    /// Precedence (highest first):
    /// 1. `RUST_LOG` environment variable (raw string, passed through)
    /// 2. `config_file_level` from `config.toml`
    /// 3. `default_level`
    pub fn initial_filter_directive(&self) -> String {
        if let Ok(raw) = std::env::var("RUST_LOG") {
            if !raw.trim().is_empty() {
                return raw;
            }
        }
        self.config_file_level
            .unwrap_or(self.default_level)
            .to_env_directive()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize all tests that mutate RUST_LOG so they do not stomp on
    // each other. `cargo test` runs tests in parallel by default.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn ordering_debug_less_than_error() {
        assert!(LogLevel::Debug < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn display_variants() {
        assert_eq!(LogLevel::Debug.to_string(), "DEBUG");
        assert_eq!(LogLevel::Error.to_string(), "ERROR");
    }

    #[test]
    fn from_str_parses_common_spellings() {
        assert_eq!("info".parse::<LogLevel>().unwrap(), LogLevel::Info);
        assert_eq!("WARN".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("warning".parse::<LogLevel>().unwrap(), LogLevel::Warn);
        assert_eq!("trace".parse::<LogLevel>().unwrap(), LogLevel::Debug);
        assert!("fatal".parse::<LogLevel>().is_err());
    }

    #[test]
    fn from_tracing_maps_levels_correctly() {
        assert_eq!(
            LogLevel::from_tracing(tracing::Level::ERROR),
            LogLevel::Error
        );
        assert_eq!(LogLevel::from_tracing(tracing::Level::WARN), LogLevel::Warn);
        assert_eq!(LogLevel::from_tracing(tracing::Level::INFO), LogLevel::Info);
        assert_eq!(
            LogLevel::from_tracing(tracing::Level::DEBUG),
            LogLevel::Debug
        );
        assert_eq!(
            LogLevel::from_tracing(tracing::Level::TRACE),
            LogLevel::Debug
        );
    }

    #[test]
    fn initial_filter_directive_prefers_rust_log_env() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var("RUST_LOG").ok();
        std::env::set_var("RUST_LOG", "gwt=trace");
        let cfg = LoggingConfig::new("/tmp".into()).with_config_file_level(Some(LogLevel::Warn));
        assert_eq!(cfg.initial_filter_directive(), "gwt=trace");
        match original {
            Some(value) => std::env::set_var("RUST_LOG", value),
            None => std::env::remove_var("RUST_LOG"),
        }
    }

    #[test]
    fn initial_filter_directive_falls_back_to_config_file_level() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var("RUST_LOG").ok();
        std::env::remove_var("RUST_LOG");
        let cfg = LoggingConfig::new("/tmp".into()).with_config_file_level(Some(LogLevel::Debug));
        assert_eq!(cfg.initial_filter_directive(), "debug");
        if let Some(value) = original {
            std::env::set_var("RUST_LOG", value);
        }
    }

    #[test]
    fn initial_filter_directive_uses_default_when_neither_set() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var("RUST_LOG").ok();
        std::env::remove_var("RUST_LOG");
        let cfg = LoggingConfig::new("/tmp".into());
        assert_eq!(cfg.initial_filter_directive(), "info");
        if let Some(value) = original {
            std::env::set_var("RUST_LOG", value);
        }
    }
}

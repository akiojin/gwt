use serde::{Deserialize, Serialize};
use std::fmt;

/// Severity level for notifications.
///
/// Routing rules (SPEC-6):
/// - Debug  -> log only
/// - Info   -> status bar (5s)
/// - Warn   -> status bar (persist)
/// - Error  -> modal dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    fn ordinal(self) -> u8 {
        match self {
            Self::Debug => 0,
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

impl PartialOrd for Severity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Severity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordinal().cmp(&other.ordinal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_debug_less_than_info() {
        assert!(Severity::Debug < Severity::Info);
    }

    #[test]
    fn ordering_info_less_than_warn() {
        assert!(Severity::Info < Severity::Warn);
    }

    #[test]
    fn ordering_warn_less_than_error() {
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn ordering_full_chain() {
        let mut levels = vec![
            Severity::Error,
            Severity::Debug,
            Severity::Warn,
            Severity::Info,
        ];
        levels.sort();
        assert_eq!(
            levels,
            vec![
                Severity::Debug,
                Severity::Info,
                Severity::Warn,
                Severity::Error
            ]
        );
    }

    #[test]
    fn display_variants() {
        assert_eq!(Severity::Debug.to_string(), "DEBUG");
        assert_eq!(Severity::Info.to_string(), "INFO");
        assert_eq!(Severity::Warn.to_string(), "WARN");
        assert_eq!(Severity::Error.to_string(), "ERROR");
    }

    #[test]
    fn equality() {
        assert_eq!(Severity::Info, Severity::Info);
        assert_ne!(Severity::Debug, Severity::Error);
    }
}

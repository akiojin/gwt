//! Summary generation trigger for terminal scrollback output.
//!
//! Extracts AI-powered session summaries from terminal pane scrollback,
//! with rate-limiting and ANSI stripping.  Delegates file I/O and ANSI
//! stripping to [`crate::terminal::scrollback`] to avoid duplication.

use crate::error::GwtError;
use crate::terminal::scrollback::{self, ScrollbackFile};

/// Summary of an agent's terminal session.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub pane_id: String,
    pub summary: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub scrollback_lines: usize,
}

/// Configuration for summary generation.
#[derive(Debug, Clone)]
pub struct SummaryConfig {
    /// Maximum bytes of scrollback to analyze.
    pub max_scrollback_bytes: usize,
    /// Minimum interval between summaries for the same pane.
    pub min_interval_secs: u64,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            max_scrollback_bytes: 32_768, // 32KB
            min_interval_secs: 60,
        }
    }
}

/// Extract the last `max_bytes` of scrollback from a pane's log file.
///
/// Reads from `~/.gwt/terminals/{pane_id}.log` and returns the tail as a
/// UTF-8 string with ANSI escape sequences stripped.
pub fn read_scrollback_tail(pane_id: &str, max_bytes: usize) -> Result<String, GwtError> {
    let sb = ScrollbackFile::new(pane_id)?;
    let raw = sb.read_tail_bytes(max_bytes)?;
    Ok(scrollback::strip_ansi(&raw))
}

/// Strip ANSI escape sequences from terminal output.
///
/// Thin wrapper around [`scrollback::strip_ansi`] that accepts `&str`.
pub fn strip_ansi(input: &str) -> String {
    scrollback::strip_ansi(input.as_bytes())
}

/// Generate a summary prompt for the AI from scrollback text.
pub fn build_summary_prompt(scrollback: &str) -> String {
    format!(
        "Summarize what this terminal session is doing in 2-3 sentences. \
         Focus on the current task, progress, and any errors.\n\n\
         Terminal output:\n{scrollback}"
    )
}

/// Check if enough time has passed since the last summary.
pub fn should_regenerate(
    last_generated: Option<chrono::DateTime<chrono::Utc>>,
    config: &SummaryConfig,
) -> bool {
    match last_generated {
        None => true,
        Some(t) => {
            chrono::Utc::now().signed_duration_since(t).num_seconds() as u64
                >= config.min_interval_secs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_ansi_removes_colors() {
        let input = "\x1b[31mred text\x1b[0m normal";
        assert_eq!(strip_ansi(input), "red text normal");
    }

    #[test]
    fn test_strip_ansi_preserves_text() {
        let input = "hello world\nline two";
        assert_eq!(strip_ansi(input), "hello world\nline two");
    }

    #[test]
    fn test_strip_ansi_empty_string() {
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn test_build_summary_prompt_contains_scrollback() {
        let scrollback = "cargo test -- ok";
        let prompt = build_summary_prompt(scrollback);
        assert!(prompt.contains(scrollback));
        assert!(prompt.contains("Summarize"));
    }

    #[test]
    fn test_should_regenerate_none_returns_true() {
        let config = SummaryConfig::default();
        assert!(should_regenerate(None, &config));
    }

    #[test]
    fn test_should_regenerate_recent_returns_false() {
        let config = SummaryConfig::default();
        let now = chrono::Utc::now();
        assert!(!should_regenerate(Some(now), &config));
    }

    #[test]
    fn test_should_regenerate_old_returns_true() {
        let config = SummaryConfig {
            min_interval_secs: 10,
            ..Default::default()
        };
        let old = chrono::Utc::now() - chrono::Duration::seconds(20);
        assert!(should_regenerate(Some(old), &config));
    }

    #[test]
    fn test_summary_config_default() {
        let config = SummaryConfig::default();
        assert_eq!(config.max_scrollback_bytes, 32_768);
        assert_eq!(config.min_interval_secs, 60);
    }
}

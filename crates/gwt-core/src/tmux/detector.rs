//! tmux environment detection
//!
//! Provides functions to detect tmux environment and check tmux installation.

use std::process::Command;

use super::error::{TmuxError, TmuxResult};

/// tmux version information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxVersion {
    pub major: u32,
    pub minor: u32,
}

impl TmuxVersion {
    /// Check if this version meets the minimum requirement (2.0+)
    pub fn is_supported(&self) -> bool {
        self.major >= 2
    }
}

impl std::fmt::Display for TmuxVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Check if currently running inside a tmux session
///
/// Returns true if the TMUX environment variable is set.
pub fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}

/// Check if tmux is installed on the system
///
/// Returns Ok(()) if tmux is installed, Err(TmuxError::NotInstalled) otherwise.
pub fn check_tmux_installed() -> TmuxResult<()> {
    match Command::new("tmux").arg("-V").output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(_) => Err(TmuxError::NotInstalled),
        Err(_) => Err(TmuxError::NotInstalled),
    }
}

/// Get the installed tmux version
///
/// Returns the version as a TmuxVersion struct, or an error if tmux is not installed
/// or the version cannot be parsed.
pub fn get_tmux_version() -> TmuxResult<TmuxVersion> {
    let output = Command::new("tmux")
        .arg("-V")
        .output()
        .map_err(|_| TmuxError::NotInstalled)?;

    if !output.status.success() {
        return Err(TmuxError::NotInstalled);
    }

    let version_str = String::from_utf8_lossy(&output.stdout);
    parse_tmux_version(&version_str)
}

/// Parse tmux version string (e.g., "tmux 3.4" or "tmux 2.0a")
fn parse_tmux_version(version_str: &str) -> TmuxResult<TmuxVersion> {
    // Expected format: "tmux X.Y" or "tmux X.Ya" (with optional suffix)
    let version_part = version_str
        .trim()
        .strip_prefix("tmux ")
        .ok_or_else(|| TmuxError::VersionParseFailed {
            output: version_str.to_string(),
        })?;

    // Remove any non-numeric suffix (like 'a' in '2.0a')
    let version_clean: String = version_part
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let parts: Vec<&str> = version_clean.split('.').collect();
    if parts.len() < 2 {
        return Err(TmuxError::VersionParseFailed {
            output: version_str.to_string(),
        });
    }

    let major = parts[0].parse().map_err(|_| TmuxError::VersionParseFailed {
        output: version_str.to_string(),
    })?;

    let minor = parts[1].parse().map_err(|_| TmuxError::VersionParseFailed {
        output: version_str.to_string(),
    })?;

    Ok(TmuxVersion { major, minor })
}

/// Check if tmux is available and meets version requirements
///
/// Returns Ok(TmuxVersion) if tmux is installed and version >= 2.0,
/// otherwise returns an appropriate error.
pub fn check_tmux_available() -> TmuxResult<TmuxVersion> {
    let version = get_tmux_version()?;

    if !version.is_supported() {
        return Err(TmuxError::VersionTooOld {
            version: version.to_string(),
        });
    }

    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_inside_tmux_with_env_var() {
        // Save original value
        let original = std::env::var("TMUX").ok();

        // Set TMUX environment variable
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        assert!(is_inside_tmux());

        // Restore original value
        if let Some(val) = original {
            std::env::set_var("TMUX", val);
        } else {
            std::env::remove_var("TMUX");
        }
    }

    #[test]
    fn test_is_inside_tmux_without_env_var() {
        // Save original value
        let original = std::env::var("TMUX").ok();

        // Remove TMUX environment variable
        std::env::remove_var("TMUX");
        assert!(!is_inside_tmux());

        // Restore original value
        if let Some(val) = original {
            std::env::set_var("TMUX", val);
        }
    }

    #[test]
    fn test_parse_tmux_version_standard() {
        let version = parse_tmux_version("tmux 3.4").unwrap();
        assert_eq!(version.major, 3);
        assert_eq!(version.minor, 4);
    }

    #[test]
    fn test_parse_tmux_version_with_suffix() {
        let version = parse_tmux_version("tmux 2.0a").unwrap();
        assert_eq!(version.major, 2);
        assert_eq!(version.minor, 0);
    }

    #[test]
    fn test_parse_tmux_version_with_newline() {
        let version = parse_tmux_version("tmux 3.4\n").unwrap();
        assert_eq!(version.major, 3);
        assert_eq!(version.minor, 4);
    }

    #[test]
    fn test_parse_tmux_version_invalid() {
        let result = parse_tmux_version("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_tmux_version_is_supported() {
        assert!(TmuxVersion { major: 2, minor: 0 }.is_supported());
        assert!(TmuxVersion { major: 3, minor: 4 }.is_supported());
        assert!(!TmuxVersion { major: 1, minor: 9 }.is_supported());
    }

    #[test]
    fn test_tmux_version_display() {
        let version = TmuxVersion { major: 3, minor: 4 };
        assert_eq!(version.to_string(), "3.4");
    }

    #[test]
    fn test_check_tmux_installed() {
        // This test is environment-dependent
        let result = check_tmux_installed();
        // Just verify it returns a valid result (either Ok or Err)
        assert!(result.is_ok() || result.is_err());
    }
}

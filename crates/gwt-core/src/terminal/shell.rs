//! Windows shell detection and WSL path conversion
//!
//! Provides [`WindowsShell`] enum for identifying available Windows shells
//! (PowerShell, Command Prompt, WSL) and utilities for WSL path translation.

use serde::{Deserialize, Serialize};

use crate::terminal::error::TerminalError;

/// Supported Windows shell types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowsShell {
    PowerShell,
    Cmd,
    Wsl,
}

impl WindowsShell {
    /// Machine-readable identifier (matches serde serialization).
    pub fn id(&self) -> &str {
        match self {
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
            Self::Wsl => "wsl",
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &str {
        match self {
            Self::PowerShell => "PowerShell",
            Self::Cmd => "Command Prompt",
            Self::Wsl => "WSL",
        }
    }

    /// Check whether this shell executable is present on the system.
    pub fn is_available(&self) -> bool {
        match self {
            Self::PowerShell => which::which("pwsh").is_ok() || which::which("powershell").is_ok(),
            Self::Cmd => which::which("cmd").is_ok(),
            Self::Wsl => {
                which::which("wsl").is_ok()
                    && crate::process::command("wsl")
                        .args(["--list", "--quiet"])
                        .output()
                        .map(|o| !o.stdout.is_empty())
                        .unwrap_or(false)
            }
        }
    }

    /// Detect the shell version (PowerShell only).
    ///
    /// Returns `None` for Cmd and WSL.
    pub fn detect_version(&self) -> Option<String> {
        match self {
            Self::PowerShell => {
                // Try pwsh (PowerShell 7+) first
                if let Ok(output) = crate::process::command("pwsh").arg("--version").output() {
                    if output.status.success() {
                        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !version.is_empty() {
                            return Some(version);
                        }
                    }
                }
                // Fall back to Windows PowerShell 5.x
                if let Ok(output) = crate::process::command("powershell")
                    .args(["-Command", "$PSVersionTable.PSVersion.ToString()"])
                    .output()
                {
                    if output.status.success() {
                        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !version.is_empty() {
                            return Some(version);
                        }
                    }
                }
                None
            }
            Self::Cmd | Self::Wsl => None,
        }
    }

    /// All known variants (for iteration).
    pub const ALL: [WindowsShell; 3] = [Self::PowerShell, Self::Cmd, Self::Wsl];
}

/// Convert a Windows path to a WSL `/mnt/` path.
///
/// # Rules
/// - UNC paths (`\\server\share`) are rejected with an error.
/// - Paths already starting with `/mnt/` are returned unchanged.
/// - Drive letter paths (`C:\...`) are converted to `/mnt/c/...`.
pub fn windows_to_wsl_path(path: &str) -> Result<String, TerminalError> {
    // Reject UNC paths
    if path.starts_with("\\\\") {
        return Err(TerminalError::WslPathConversion {
            details: format!("UNC paths are not supported: {path}"),
        });
    }

    // Already WSL format
    if path.starts_with("/mnt/") {
        return Ok(path.to_string());
    }

    // Need at least "C:\" or "C:/"
    let bytes = path.as_bytes();
    if bytes.len() < 3 {
        return Err(TerminalError::WslPathConversion {
            details: format!("Path too short to be a valid Windows path: {path}"),
        });
    }

    let drive = bytes[0];
    if !drive.is_ascii_alphabetic() || bytes[1] != b':' || (bytes[2] != b'\\' && bytes[2] != b'/') {
        return Err(TerminalError::WslPathConversion {
            details: format!("Invalid Windows path format: {path}"),
        });
    }

    let drive_lower = (drive as char).to_ascii_lowercase();
    let rest = &path[3..].replace('\\', "/");
    if rest.is_empty() {
        Ok(format!("/mnt/{drive_lower}"))
    } else {
        Ok(format!("/mnt/{drive_lower}/{rest}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- T003: WindowsShell id() / display_name() tests ---

    #[test]
    fn test_powershell_id() {
        assert_eq!(WindowsShell::PowerShell.id(), "powershell");
    }

    #[test]
    fn test_cmd_id() {
        assert_eq!(WindowsShell::Cmd.id(), "cmd");
    }

    #[test]
    fn test_wsl_id() {
        assert_eq!(WindowsShell::Wsl.id(), "wsl");
    }

    #[test]
    fn test_powershell_display_name() {
        assert_eq!(WindowsShell::PowerShell.display_name(), "PowerShell");
    }

    #[test]
    fn test_cmd_display_name() {
        assert_eq!(WindowsShell::Cmd.display_name(), "Command Prompt");
    }

    #[test]
    fn test_wsl_display_name() {
        assert_eq!(WindowsShell::Wsl.display_name(), "WSL");
    }

    // --- T003: Serde round-trip tests ---

    #[test]
    fn test_serde_powershell_roundtrip() {
        let json = serde_json::to_string(&WindowsShell::PowerShell).unwrap();
        assert_eq!(json, "\"powershell\"");
        let back: WindowsShell = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WindowsShell::PowerShell);
    }

    #[test]
    fn test_serde_cmd_roundtrip() {
        let json = serde_json::to_string(&WindowsShell::Cmd).unwrap();
        assert_eq!(json, "\"cmd\"");
        let back: WindowsShell = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WindowsShell::Cmd);
    }

    #[test]
    fn test_serde_wsl_roundtrip() {
        let json = serde_json::to_string(&WindowsShell::Wsl).unwrap();
        assert_eq!(json, "\"wsl\"");
        let back: WindowsShell = serde_json::from_str(&json).unwrap();
        assert_eq!(back, WindowsShell::Wsl);
    }

    #[test]
    fn test_serde_deserialize_unknown_variant() {
        let result = serde_json::from_str::<WindowsShell>("\"bash\"");
        assert!(result.is_err());
    }

    // --- T003: ALL constant ---

    #[test]
    fn test_all_variants() {
        assert_eq!(WindowsShell::ALL.len(), 3);
        assert!(WindowsShell::ALL.contains(&WindowsShell::PowerShell));
        assert!(WindowsShell::ALL.contains(&WindowsShell::Cmd));
        assert!(WindowsShell::ALL.contains(&WindowsShell::Wsl));
    }

    // --- T006: windows_to_wsl_path() tests ---

    #[test]
    fn test_wsl_path_c_drive() {
        let result = windows_to_wsl_path("C:\\Users\\foo").unwrap();
        assert_eq!(result, "/mnt/c/Users/foo");
    }

    #[test]
    fn test_wsl_path_d_drive() {
        let result = windows_to_wsl_path("D:\\projects\\repo").unwrap();
        assert_eq!(result, "/mnt/d/projects/repo");
    }

    #[test]
    fn test_wsl_path_lowercase_drive() {
        let result = windows_to_wsl_path("c:\\foo").unwrap();
        assert_eq!(result, "/mnt/c/foo");
    }

    #[test]
    fn test_wsl_path_unc_rejected() {
        let result = windows_to_wsl_path("\\\\server\\share");
        assert!(result.is_err());
    }

    #[test]
    fn test_wsl_path_already_mnt() {
        let result = windows_to_wsl_path("/mnt/c/Users/foo").unwrap();
        assert_eq!(result, "/mnt/c/Users/foo");
    }

    #[test]
    fn test_wsl_path_root_drive() {
        let result = windows_to_wsl_path("C:\\").unwrap();
        assert_eq!(result, "/mnt/c");
    }

    #[test]
    fn test_wsl_path_forward_slashes() {
        let result = windows_to_wsl_path("C:/Users/foo").unwrap();
        assert_eq!(result, "/mnt/c/Users/foo");
    }
}

//! SPEC #2920 FR-005 — Autostart manager skeleton.
//!
//! Wraps the `auto-launch` crate so the tray-resident process can be
//! enrolled with the OS-native login-item mechanism per user. The actual
//! crate integration lands in Phase 7; this module currently captures the
//! public surface so Phase 1 can wire the WebSocket dispatch contract
//! ahead of the runtime work.
//!
//! Per-OS mechanism (handled internally by `auto-launch`):
//! - macOS: Login Items (NSWorkspace) with a LaunchAgent plist fallback.
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! - Linux: `~/.config/autostart/gwt.desktop` (XDG autostart).

use std::path::PathBuf;

/// Snapshot of the autostart entry state for the current user.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub install_path: Option<PathBuf>,
    pub mechanism: AutostartMechanism,
}

/// Per-OS implementation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum AutostartMechanism {
    LoginItems,
    LaunchAgent,
    Registry,
    XdgAutostart,
    Unsupported,
}

impl AutostartMechanism {
    /// Resolve the canonical mechanism for the host OS at compile time.
    pub const fn for_host_os() -> Self {
        if cfg!(target_os = "macos") {
            // auto-launch defaults to Login Items on macOS and falls back
            // to a LaunchAgent plist when the API is unavailable. The
            // status surface reports the resolved mechanism at runtime;
            // this constant only carries the preferred starting point.
            Self::LoginItems
        } else if cfg!(target_os = "windows") {
            Self::Registry
        } else if cfg!(target_os = "linux") {
            Self::XdgAutostart
        } else {
            Self::Unsupported
        }
    }
}

/// Errors surfaced from autostart install / uninstall / status.
#[derive(Debug, thiserror::Error)]
pub enum AutostartError {
    #[error("autostart manager is not yet implemented (SPEC #2920 Phase 7)")]
    NotYetImplemented,
}

/// Stateless wrapper around the `auto-launch` crate. Phase 7 will replace
/// the placeholder implementations.
pub struct AutostartManager;

impl AutostartManager {
    pub fn install() -> Result<(), AutostartError> {
        Err(AutostartError::NotYetImplemented)
    }

    pub fn uninstall() -> Result<(), AutostartError> {
        Err(AutostartError::NotYetImplemented)
    }

    pub fn status() -> Result<AutostartStatus, AutostartError> {
        Err(AutostartError::NotYetImplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mechanism_for_host_os_resolves_to_a_known_variant() {
        let mechanism = AutostartMechanism::for_host_os();
        assert!(matches!(
            mechanism,
            AutostartMechanism::LoginItems
                | AutostartMechanism::LaunchAgent
                | AutostartMechanism::Registry
                | AutostartMechanism::XdgAutostart
                | AutostartMechanism::Unsupported
        ));
    }

    #[test]
    fn placeholder_operations_report_not_yet_implemented() {
        assert!(matches!(
            AutostartManager::install(),
            Err(AutostartError::NotYetImplemented)
        ));
        assert!(matches!(
            AutostartManager::uninstall(),
            Err(AutostartError::NotYetImplemented)
        ));
        assert!(matches!(
            AutostartManager::status(),
            Err(AutostartError::NotYetImplemented)
        ));
    }
}

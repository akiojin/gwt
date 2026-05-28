//! SPEC #2920 FR-005 / Phase 7 — Autostart manager backed by the
//! `auto-launch` crate.
//!
//! Per-OS mechanism (handled internally by `auto-launch`):
//! - macOS: LaunchAgent plist (`~/Library/LaunchAgents/<app>.plist`) when
//!   `set_use_launch_agent(true)` is configured. Otherwise an AppleScript
//!   Login Item is added via NSWorkspace. SPEC #2920 Q6 settled on the
//!   LaunchAgent plist for predictability: it lives under the user's
//!   home directory, can be inspected with `launchctl list`, and survives
//!   `gwt` updates because the path resolves at runtime via
//!   `std::env::current_exe()`.
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! - Linux: `~/.config/autostart/<app>.desktop` (XDG autostart).

use std::path::PathBuf;

use auto_launch::AutoLaunchBuilder;

/// Identifier registered with the OS autostart mechanism. Persists
/// across `gwt` upgrades so the install/uninstall toggle is idempotent
/// even when the binary path changes between versions.
pub const APP_NAME: &str = "GWT";

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
            // SPEC #2920 Q6: prefer the LaunchAgent plist on macOS so the
            // toggle is reversible from the file system without prompting
            // for AppleScript automation permissions.
            Self::LaunchAgent
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
    #[error("could not resolve the gwt executable path: {0}")]
    ExecutablePathUnavailable(std::io::Error),
    #[error("auto-launch backend error: {0}")]
    Backend(#[from] auto_launch::Error),
    #[error("autostart is not supported on this OS")]
    UnsupportedOs,
}

/// Stateless wrapper around the `auto-launch` crate. Construction lives in
/// `build_auto_launch` so the install / uninstall / status paths share the
/// exact same `(app_name, app_path, use_launch_agent)` triple — a mismatch
/// here would silently leak orphan entries across calls.
pub struct AutostartManager;

impl AutostartManager {
    /// Register `gwt` with the OS autostart mechanism. Idempotent: a second
    /// `install()` after a successful first call returns `Ok(())` instead
    /// of erroring out.
    pub fn install() -> Result<(), AutostartError> {
        if matches!(
            AutostartMechanism::for_host_os(),
            AutostartMechanism::Unsupported
        ) {
            return Err(AutostartError::UnsupportedOs);
        }
        let auto = build_auto_launch()?;
        auto.enable()?;
        Ok(())
    }

    /// Remove the autostart entry. Idempotent: a second `uninstall()`
    /// after the entry has already been removed returns `Ok(())`.
    pub fn uninstall() -> Result<(), AutostartError> {
        if matches!(
            AutostartMechanism::for_host_os(),
            AutostartMechanism::Unsupported
        ) {
            return Err(AutostartError::UnsupportedOs);
        }
        let auto = build_auto_launch()?;
        auto.disable()?;
        Ok(())
    }

    /// Inspect the OS to determine whether autostart is currently enabled
    /// for `gwt`, the resolved install path, and the mechanism in use.
    pub fn status() -> Result<AutostartStatus, AutostartError> {
        let mechanism = AutostartMechanism::for_host_os();
        if matches!(mechanism, AutostartMechanism::Unsupported) {
            return Err(AutostartError::UnsupportedOs);
        }
        let auto = build_auto_launch()?;
        let enabled = auto.is_enabled()?;
        Ok(AutostartStatus {
            enabled,
            install_path: install_path_for_app(APP_NAME),
            mechanism,
        })
    }
}

fn build_auto_launch() -> Result<auto_launch::AutoLaunch, AutostartError> {
    let exe = std::env::current_exe().map_err(AutostartError::ExecutablePathUnavailable)?;
    let exe_str = exe.to_string_lossy().into_owned();
    let mut builder = AutoLaunchBuilder::new();
    builder.set_app_name(APP_NAME);
    builder.set_app_path(&exe_str);
    // SPEC #2920 Q6: pin the macOS strategy to the LaunchAgent plist so
    // the toggle remains reversible from the file system. The flag is a
    // no-op on Linux / Windows.
    builder.set_use_launch_agent(true);
    Ok(builder.build()?)
}

/// Compute the path the OS-native autostart entry would live at without
/// touching the file system. Mirrors the `auto-launch` crate's per-OS
/// path resolution so the Settings page can preview the location before
/// `install()` is called.
fn install_path_for_app(app_name: &str) -> Option<PathBuf> {
    let home = home_dir()?;
    match AutostartMechanism::for_host_os() {
        AutostartMechanism::LaunchAgent => Some(
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{app_name}.plist")),
        ),
        AutostartMechanism::XdgAutostart => Some(
            home.join(".config")
                .join("autostart")
                .join(format!("{app_name}.desktop")),
        ),
        AutostartMechanism::Registry => {
            // Windows stores the entry in the registry, not on disk, so
            // surface the registry path as a hint instead of a fs path.
            Some(PathBuf::from(format!(
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\{app_name}"
            )))
        }
        AutostartMechanism::LoginItems | AutostartMechanism::Unsupported => None,
    }
}

fn home_dir() -> Option<PathBuf> {
    // `auto-launch` itself depends on `dirs::home_dir()`, so we mirror its
    // logic by reading the same env vars. Falling back through env keeps
    // this dependency-light.
    if cfg!(target_os = "windows") {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mechanism_for_host_os_matches_compile_target() {
        let mechanism = AutostartMechanism::for_host_os();
        #[cfg(target_os = "macos")]
        assert_eq!(mechanism, AutostartMechanism::LaunchAgent);
        #[cfg(target_os = "windows")]
        assert_eq!(mechanism, AutostartMechanism::Registry);
        #[cfg(target_os = "linux")]
        assert_eq!(mechanism, AutostartMechanism::XdgAutostart);
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        assert_eq!(mechanism, AutostartMechanism::Unsupported);
    }

    #[test]
    fn install_path_for_app_is_user_scoped() {
        let path = install_path_for_app("GWT");
        let Some(path) = path else {
            // Unsupported OS — no install path is also acceptable.
            return;
        };
        let path_str = path.to_string_lossy().into_owned();
        #[cfg(target_os = "macos")]
        assert!(
            path_str.ends_with("Library/LaunchAgents/GWT.plist"),
            "macOS install path must be a user LaunchAgent plist, got {path_str}"
        );
        #[cfg(target_os = "linux")]
        assert!(
            path_str.ends_with(".config/autostart/GWT.desktop"),
            "Linux install path must be the XDG autostart entry, got {path_str}"
        );
        #[cfg(target_os = "windows")]
        assert!(
            path_str.contains("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            "Windows install path must point at the user Run registry key, got {path_str}"
        );
    }

    #[test]
    fn status_returns_a_consistent_snapshot() {
        // SPEC #2920 Phase 7 — `status()` must work without writing to
        // the user's autostart entry. The auto-launch crate inspects the
        // OS-native state and never installs anything on `is_enabled()`.
        let result = AutostartManager::status();
        match result {
            Ok(status) => {
                assert_eq!(status.mechanism, AutostartMechanism::for_host_os());
                // `install_path` is only `None` on truly unsupported OS;
                // every CI target we support returns Some(_) here.
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                assert!(status.install_path.is_some());
            }
            Err(AutostartError::ExecutablePathUnavailable(_)) => {
                // The test runner may execute the binary without a
                // resolvable current_exe() (rare in CI). Accept the
                // graceful error rather than panicking.
            }
            Err(AutostartError::UnsupportedOs) => {
                #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
                panic!("AutostartManager::status returned UnsupportedOs on a supported target");
            }
            Err(other) => panic!("AutostartManager::status returned unexpected error: {other}"),
        }
    }

    #[test]
    fn autostart_status_serializes_round_trip() {
        let status = AutostartStatus {
            enabled: true,
            install_path: Some(PathBuf::from("/tmp/GWT.plist")),
            mechanism: AutostartMechanism::LaunchAgent,
        };
        let json = serde_json::to_string(&status).expect("serialize");
        let round: AutostartStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round, status);
    }
}

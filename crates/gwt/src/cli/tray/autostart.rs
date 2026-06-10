//! SPEC #2920 FR-005 / Phase 7 + Phase 12 — Autostart manager.
//!
//! Per-OS mechanism:
//! - macOS 13+ (Ventura, Darwin 22+): the modern `SMAppService` API
//!   (`SMAppService.mainAppService`). Registering through SMAppService lets
//!   macOS attribute the entry to the app bundle, so System Settings >
//!   "Login Items & Extensions" shows the app name ("GWT") instead of the
//!   Developer ID team name. A raw LaunchAgent plist cannot be associated
//!   with a bundle, so Background Task Management falls back to the signing
//!   team name (e.g. a personal Developer ID shows the developer's own
//!   name) — that is the bug Phase 12 fixes.
//! - macOS 12 and earlier: the `auto-launch` LaunchAgent plist
//!   (`~/Library/LaunchAgents/<app>.plist`). These releases predate the
//!   Background Task Management UI, so the display-name problem does not
//!   exist there and the LaunchAgent path stays.
//! - Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` via
//!   `auto-launch`.
//! - Linux: `~/.config/autostart/<app>.desktop` (XDG autostart) via
//!   `auto-launch`.

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
    /// macOS 13+ modern Service Management registration (`SMAppService`).
    AppService,
    Registry,
    XdgAutostart,
    Unsupported,
}

impl AutostartMechanism {
    /// Resolve the mechanism for the host OS. On macOS the choice depends on
    /// the runtime OS version (SMAppService exists only on macOS 13+), so this
    /// is no longer a `const fn`.
    pub fn for_host_os() -> Self {
        #[cfg(target_os = "macos")]
        {
            mechanism_for_macos(current_darwin_major())
        }
        #[cfg(target_os = "windows")]
        {
            Self::Registry
        }
        #[cfg(target_os = "linux")]
        {
            Self::XdgAutostart
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Unsupported
        }
    }
}

/// Map a Darwin kernel major version to the macOS autostart mechanism.
///
/// SMAppService first shipped in macOS 13 Ventura, which is Darwin kernel 22.
/// The Darwin↔macOS *product* number map is no longer linear after the macOS
/// 26 renumber, so gate strictly on the Darwin major (the SMAppService floor),
/// never on a computed macOS product number.
#[cfg(any(target_os = "macos", test))]
const fn mechanism_for_macos(darwin_major: u32) -> AutostartMechanism {
    if macos_supports_app_service(darwin_major) {
        AutostartMechanism::AppService
    } else {
        AutostartMechanism::LaunchAgent
    }
}

/// `true` when the Darwin kernel is 22 (macOS 13 Ventura) or newer, i.e. when
/// `SMAppService` is available.
#[cfg(any(target_os = "macos", test))]
const fn macos_supports_app_service(darwin_major: u32) -> bool {
    darwin_major >= 22
}

/// Errors surfaced from autostart install / uninstall / status.
#[derive(Debug, thiserror::Error)]
pub enum AutostartError {
    #[error("could not resolve the gwt executable path: {0}")]
    ExecutablePathUnavailable(std::io::Error),
    #[error("auto-launch backend error: {0}")]
    Backend(#[from] auto_launch::Error),
    #[cfg(target_os = "macos")]
    #[error("SMAppService error: {0}")]
    AppService(String),
    #[error("autostart is not supported on this OS")]
    UnsupportedOs,
}

/// Stateless wrapper around the per-OS autostart mechanism. macOS 13+ routes
/// through `SMAppService`; every other target shares the exact same
/// `(app_name, app_path, use_launch_agent)` triple via `build_auto_launch` so a
/// mismatch here cannot silently leak orphan entries across calls.
pub struct AutostartManager;

impl AutostartManager {
    /// Register `gwt` with the OS autostart mechanism. Idempotent: a second
    /// `install()` after a successful first call returns `Ok(())` instead
    /// of erroring out.
    pub fn install() -> Result<(), AutostartError> {
        let mechanism = AutostartMechanism::for_host_os();
        if matches!(mechanism, AutostartMechanism::Unsupported) {
            return Err(AutostartError::UnsupportedOs);
        }
        #[cfg(target_os = "macos")]
        if matches!(mechanism, AutostartMechanism::AppService) {
            return macos_app_service::install();
        }
        let auto = build_auto_launch()?;
        auto.enable()?;
        Ok(())
    }

    /// Remove the autostart entry. Idempotent: a second `uninstall()`
    /// after the entry has already been removed returns `Ok(())`.
    pub fn uninstall() -> Result<(), AutostartError> {
        let mechanism = AutostartMechanism::for_host_os();
        if matches!(mechanism, AutostartMechanism::Unsupported) {
            return Err(AutostartError::UnsupportedOs);
        }
        #[cfg(target_os = "macos")]
        if matches!(mechanism, AutostartMechanism::AppService) {
            return macos_app_service::uninstall();
        }
        let auto = build_auto_launch()?;
        auto.disable()?;
        Ok(())
    }

    /// Inspect the OS to determine whether autostart is currently enabled
    /// for `gwt`, the resolved install path, and the mechanism in use.
    ///
    /// Read-only: this never writes to the user's autostart state. The legacy
    /// LaunchAgent migration cleanup happens only on the explicit `install` /
    /// `uninstall` mutations.
    pub fn status() -> Result<AutostartStatus, AutostartError> {
        let mechanism = AutostartMechanism::for_host_os();
        if matches!(mechanism, AutostartMechanism::Unsupported) {
            return Err(AutostartError::UnsupportedOs);
        }
        #[cfg(target_os = "macos")]
        if matches!(mechanism, AutostartMechanism::AppService) {
            return macos_app_service::status();
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
    // macOS 12 and earlier: pin the LaunchAgent plist so the toggle remains
    // reversible from the file system. The flag is a no-op on Linux / Windows.
    builder.set_use_launch_agent(true);
    Ok(builder.build()?)
}

/// Compute the path the OS-native autostart entry would live at without
/// touching the file system. Mirrors the per-OS path resolution so the
/// Settings page can preview the location before `install()` is called.
/// macOS 13+ uses SMAppService, which has no on-disk entry, so it returns
/// `None`.
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
        AutostartMechanism::AppService
        | AutostartMechanism::LoginItems
        | AutostartMechanism::Unsupported => None,
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

/// Parse the Darwin kernel major version from a `kern.osrelease` string such
/// as `"22.6.0"`.
#[cfg(any(target_os = "macos", test))]
fn parse_darwin_major(osrelease: &str) -> Option<u32> {
    osrelease.split('.').next()?.trim().parse::<u32>().ok()
}

// SMAppServiceStatus values from `<ServiceManagement/SMAppService.h>`:
// NotRegistered = 0, Enabled = 1, RequiresApproval = 2, NotFound = 3.
#[cfg(any(target_os = "macos", test))]
const SM_STATUS_ENABLED: isize = 1;
#[cfg(any(target_os = "macos", test))]
const SM_STATUS_REQUIRES_APPROVAL: isize = 2;

/// `true` when the SMAppService status means the login item is actively
/// enabled.
#[cfg(any(target_os = "macos", test))]
fn sm_status_is_enabled(raw: isize) -> bool {
    raw == SM_STATUS_ENABLED
}

/// `true` when the SMAppService status means an entry exists (enabled or
/// awaiting the user's approval) and therefore can be unregistered.
#[cfg(any(target_os = "macos", test))]
fn sm_status_is_registered(raw: isize) -> bool {
    raw == SM_STATUS_ENABLED || raw == SM_STATUS_REQUIRES_APPROVAL
}

/// Path of the legacy `auto-launch` LaunchAgent plist for `APP_NAME`.
#[cfg(any(target_os = "macos", test))]
fn legacy_launch_agent_path(home: &std::path::Path) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{APP_NAME}.plist"))
}

/// Remove `path` if it exists; treat an already-absent file as success so the
/// migration cleanup is idempotent.
#[cfg(any(target_os = "macos", test))]
fn remove_file_if_present(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Read the Darwin kernel major version via `sysctl kern.osrelease`. Returns 0
/// on failure, which makes [`mechanism_for_macos`] fall back to the LaunchAgent
/// path rather than risk calling an unavailable SMAppService.
#[cfg(target_os = "macos")]
fn current_darwin_major() -> u32 {
    let name = c"kern.osrelease";
    let mut size: libc::size_t = 0;
    // First call: query the required buffer size.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return 0;
    }
    let mut buf = vec![0u8; size];
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return 0;
    }
    // `size` now counts the trailing NUL written by sysctl.
    let end = size.saturating_sub(1);
    let text = String::from_utf8_lossy(&buf[..end.min(buf.len())]);
    parse_darwin_major(&text).unwrap_or(0)
}

/// Remove the legacy LaunchAgent plist left by older gwt versions so the
/// SMAppService entry is the only one Background Task Management lists. Older
/// builds dropped a raw plist that BTM attributes to the signing team name;
/// dropping it prevents a stale "<developer name>" entry from lingering under
/// "Allow in the Background".
#[cfg(target_os = "macos")]
fn remove_legacy_launch_agent() {
    if let Some(home) = home_dir() {
        // Best effort: an absent file is the desired end state.
        let _ = remove_file_if_present(&legacy_launch_agent_path(&home));
    }
}

#[cfg(target_os = "macos")]
mod macos_app_service {
    use super::{
        home_dir, legacy_launch_agent_path, remove_legacy_launch_agent, sm_status_is_enabled,
        sm_status_is_registered, AutostartError, AutostartMechanism, AutostartStatus,
    };
    use objc2_service_management::SMAppService;

    /// Register the main app as a login item and clear any legacy plist.
    /// Idempotent: skips `register` when already enabled.
    pub(super) fn install() -> Result<(), AutostartError> {
        let service = unsafe { SMAppService::mainAppService() };
        let raw = unsafe { service.status() }.0;
        if !sm_status_is_enabled(raw) {
            unsafe { service.registerAndReturnError() }
                .map_err(|err| AutostartError::AppService(format!("{err:?}")))?;
        }
        remove_legacy_launch_agent();
        Ok(())
    }

    /// Unregister the main-app login item and clear any legacy plist.
    /// Idempotent: skips `unregister` when nothing is registered.
    pub(super) fn uninstall() -> Result<(), AutostartError> {
        let service = unsafe { SMAppService::mainAppService() };
        let raw = unsafe { service.status() }.0;
        if sm_status_is_registered(raw) {
            unsafe { service.unregisterAndReturnError() }
                .map_err(|err| AutostartError::AppService(format!("{err:?}")))?;
        }
        remove_legacy_launch_agent();
        Ok(())
    }

    /// Read-only status. Reports enabled when SMAppService is registered or a
    /// legacy LaunchAgent plist still exists (so an upgraded user who enabled
    /// autostart via the old mechanism still sees the toggle ON until they
    /// re-toggle, which migrates them onto SMAppService).
    pub(super) fn status() -> Result<AutostartStatus, AutostartError> {
        let service = unsafe { SMAppService::mainAppService() };
        let raw = unsafe { service.status() }.0;
        let legacy_present = home_dir()
            .map(|home| legacy_launch_agent_path(&home).exists())
            .unwrap_or(false);
        Ok(AutostartStatus {
            enabled: sm_status_is_enabled(raw) || legacy_present,
            install_path: None,
            mechanism: AutostartMechanism::AppService,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mechanism_for_host_os_matches_compile_target() {
        let mechanism = AutostartMechanism::for_host_os();
        #[cfg(target_os = "macos")]
        assert!(
            matches!(
                mechanism,
                AutostartMechanism::AppService | AutostartMechanism::LaunchAgent
            ),
            "macOS resolves to SMAppService (13+) or the LaunchAgent fallback, got {mechanism:?}"
        );
        #[cfg(target_os = "windows")]
        assert_eq!(mechanism, AutostartMechanism::Registry);
        #[cfg(target_os = "linux")]
        assert_eq!(mechanism, AutostartMechanism::XdgAutostart);
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        assert_eq!(mechanism, AutostartMechanism::Unsupported);
    }

    #[test]
    fn macos_mechanism_switches_on_darwin_floor() {
        // Darwin 22 == macOS 13 Ventura, the SMAppService floor.
        assert_eq!(mechanism_for_macos(21), AutostartMechanism::LaunchAgent); // macOS 12
        assert_eq!(mechanism_for_macos(22), AutostartMechanism::AppService); // macOS 13
        assert_eq!(mechanism_for_macos(25), AutostartMechanism::AppService); // macOS 26
        assert!(!macos_supports_app_service(21));
        assert!(macos_supports_app_service(22));
        assert!(macos_supports_app_service(25));
    }

    #[test]
    fn parse_darwin_major_reads_first_component() {
        assert_eq!(parse_darwin_major("22.6.0"), Some(22));
        assert_eq!(parse_darwin_major("25.5.0"), Some(25));
        assert_eq!(parse_darwin_major("13"), Some(13));
        assert_eq!(parse_darwin_major(""), None);
        assert_eq!(parse_darwin_major("notanumber"), None);
    }

    #[test]
    fn sm_status_mapping_matches_apple_enum() {
        // NotRegistered = 0, Enabled = 1, RequiresApproval = 2, NotFound = 3.
        assert!(!sm_status_is_enabled(0));
        assert!(sm_status_is_enabled(1));
        assert!(!sm_status_is_enabled(2));
        assert!(!sm_status_is_enabled(3));

        assert!(!sm_status_is_registered(0));
        assert!(sm_status_is_registered(1));
        assert!(sm_status_is_registered(2));
        assert!(!sm_status_is_registered(3));
    }

    #[test]
    fn remove_file_if_present_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("GWT.plist");
        std::fs::write(&path, b"<plist/>").expect("write");
        assert!(path.exists());

        remove_file_if_present(&path).expect("first remove");
        assert!(!path.exists());
        // A second call on an absent file is a no-op success.
        remove_file_if_present(&path).expect("second remove");
    }

    #[test]
    fn legacy_launch_agent_path_is_user_scoped() {
        let home = std::path::Path::new("/Users/example");
        let path = legacy_launch_agent_path(home);
        let suffix = std::path::Path::new("Library")
            .join("LaunchAgents")
            .join("GWT.plist");
        assert!(path.ends_with(&suffix), "got {path:?}");
    }

    #[test]
    fn install_path_for_app_is_user_scoped() {
        let path = install_path_for_app("GWT");
        #[cfg(target_os = "macos")]
        {
            // macOS 13+ (SMAppService) has no on-disk entry; macOS 12 uses a
            // LaunchAgent plist.
            match AutostartMechanism::for_host_os() {
                AutostartMechanism::AppService => {
                    assert!(path.is_none(), "SMAppService has no fs path, got {path:?}")
                }
                AutostartMechanism::LaunchAgent => {
                    let path = path.expect("LaunchAgent path");
                    assert!(
                        path.to_string_lossy()
                            .ends_with("Library/LaunchAgents/GWT.plist"),
                        "got {path:?}"
                    );
                }
                other => panic!("unexpected macOS mechanism {other:?}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            let path = path.expect("xdg autostart path");
            assert!(
                path.to_string_lossy()
                    .ends_with(".config/autostart/GWT.desktop"),
                "got {path:?}"
            );
        }
        #[cfg(target_os = "windows")]
        {
            let path = path.expect("registry hint");
            assert!(
                path.to_string_lossy()
                    .contains("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
                "got {path:?}"
            );
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

    #[test]
    fn app_service_mechanism_serializes_to_pascal_case() {
        let json = serde_json::to_string(&AutostartMechanism::AppService).expect("serialize");
        assert_eq!(json, "\"AppService\"");
        let round: AutostartMechanism = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round, AutostartMechanism::AppService);
    }

    // macOS 13+ routes `status()` through the SMAppService FFI, which we do not
    // exercise from unit tests, so this lives on the non-macOS targets where
    // the auto-launch backend reads OS-native state without writing anything.
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn status_returns_a_consistent_snapshot() {
        match AutostartManager::status() {
            Ok(status) => {
                assert_eq!(status.mechanism, AutostartMechanism::for_host_os());
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                assert!(status.install_path.is_some());
            }
            Err(AutostartError::ExecutablePathUnavailable(_)) => {
                // The test runner may execute without a resolvable
                // current_exe(); accept the graceful error.
            }
            Err(AutostartError::UnsupportedOs) => {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                panic!("AutostartManager::status returned UnsupportedOs on a supported target");
            }
            Err(other) => panic!("AutostartManager::status returned unexpected error: {other}"),
        }
    }
}

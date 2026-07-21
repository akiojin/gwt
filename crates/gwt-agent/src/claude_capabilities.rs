//! Detection of Claude Code capabilities relevant to launch options.
//!
//! `ultracode` is a Claude Code *session setting* (not an effort level): it
//! sends `xhigh` to the model and additionally enables dynamic workflow
//! orchestration. It is available only on models that support `xhigh`
//! (Opus 4.7 / 4.8, Fable 5) and requires Claude Code >= 2.1.154 with workflows
//! enabled. These helpers determine, before launch, whether the installed
//! Claude Code can actually use ultracode so the Launch Wizard only offers it
//! when usable.
//!
//! Activation rides the existing `--settings` channel as `{"ultracode":true}`;
//! it is intentionally NOT exported via `CLAUDE_CODE_EFFORT_LEVEL` because
//! `ultracode` is not a valid effort-level value.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

/// Session-stable live Claude Code capability state.
///
/// Pure helpers such as [`supports_ultracode`] remain uncached. This snapshot
/// only covers environment/file/subprocess-backed detection that would
/// otherwise run on GUI hot paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeCapabilitySnapshot {
    pub workflows_enabled: bool,
    pub ultracode_supported: bool,
}

/// Extract a `MAJOR.MINOR.PATCH` semver from a raw `claude --version` string.
///
/// `claude --version` prints e.g. `"2.1.156 (Claude Code)"`, so we take the
/// first whitespace-delimited token (stripping an optional `v` prefix) before
/// parsing. Returns `None` when no leading semver can be parsed.
pub fn parse_claude_semver(raw: &str) -> Option<semver::Version> {
    let token = raw.split_whitespace().next()?;
    let token = token.strip_prefix('v').unwrap_or(token);
    semver::Version::parse(token).ok()
}

/// Minimum Claude Code version that supports ultracode + dynamic workflows.
fn ultracode_min_version() -> semver::Version {
    semver::Version::new(2, 1, 154)
}

/// Decide whether Claude Code dynamic workflows are enabled, given the raw
/// `CLAUDE_CODE_DISABLE_WORKFLOWS` env value and the contents of the user's
/// global `~/.claude/settings.json` (if any).
///
/// Workflows are enabled by default; only an explicit disable turns them off:
/// a truthy env value, or `"disableWorkflows": true` in the settings JSON. A
/// missing/empty env, missing key, or malformed JSON all leave workflows
/// enabled (the documented default).
pub fn workflows_enabled_from(env_disable: Option<&str>, user_settings_json: Option<&str>) -> bool {
    if env_truthy(env_disable) {
        return false;
    }
    if let Some(json) = user_settings_json {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
            if value.get("disableWorkflows").and_then(|v| v.as_bool()) == Some(true) {
                return false;
            }
        }
    }
    true
}

/// Whether the installed Claude Code (by raw `--version` string) plus the
/// current workflow state supports ultracode. Requires version >= 2.1.154 and
/// workflows enabled. An unparseable version yields `false` (conservative).
pub fn supports_ultracode(version_raw: &str, workflows_enabled: bool) -> bool {
    workflows_enabled
        && parse_claude_semver(version_raw)
            .is_some_and(|version| version >= ultracode_min_version())
}

/// Path to the user's global Claude Code settings (`~/.claude/settings.json`).
///
/// Resolves the home directory from `HOME` / `USERPROFILE` (matching
/// `gwt_core::paths`); returns `None` when neither is set.
pub fn claude_user_settings_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".claude").join("settings.json"))
}

/// Detect whether Claude Code dynamic workflows are enabled in the current
/// environment (env var + user global settings). Defaults to `true` when
/// nothing can be read.
pub fn claude_workflows_enabled() -> bool {
    claude_capability_snapshot().workflows_enabled
}

fn detect_claude_workflows_enabled() -> bool {
    let env_disable = std::env::var("CLAUDE_CODE_DISABLE_WORKFLOWS").ok();
    let settings = claude_user_settings_path().and_then(|path| std::fs::read_to_string(path).ok());
    workflows_enabled_from(env_disable.as_deref(), settings.as_deref())
}

/// Raw `claude --version` output (e.g. `"2.1.156 (Claude Code)"`), or `None`
/// when the `claude` binary is missing or the call fails. Run sparingly (it
/// spawns a subprocess): the Launch Wizard captures this once at open time,
/// not on every render.
pub fn detect_claude_version_raw() -> Option<String> {
    let request = gwt_core::process::ProcessPlanRequest::new("claude").arg("--version");
    let output = match gwt_core::process::resolved_command(request) {
        Ok(mut command) => command.output().ok()?,
        Err(error) => {
            tracing::warn!(
                command = "claude",
                error = %error,
                "Claude capability probe could not resolve a safe executable"
            );
            return None;
        }
    };
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!raw.is_empty()).then_some(raw)
}

/// Whether the locally installed Claude Code can use ultracode right now:
/// dynamic workflows enabled AND installed version >= 2.1.154. Composes the
/// (tested) pure predicates with live environment detection. Returns `false`
/// when the version cannot be determined (conservative).
///
/// The result is cached for the running process, so repeated Launch Wizard
/// opens and other live callers stay free of repeated subprocess/file I/O.
pub fn claude_ultracode_supported() -> bool {
    claude_capability_snapshot().ultracode_supported
}

/// Detect and cache live Claude Code capability state for this process.
pub fn claude_capability_snapshot() -> ClaudeCapabilitySnapshot {
    claude_capability_snapshot_with_detector(detect_claude_capability_snapshot)
}

fn claude_capability_snapshot_with_detector(
    detect: impl FnOnce() -> ClaudeCapabilitySnapshot,
) -> ClaudeCapabilitySnapshot {
    let mut cache = claude_capability_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(snapshot) = *cache {
        return snapshot;
    }
    let snapshot = detect();
    *cache = Some(snapshot);
    snapshot
}

fn detect_claude_capability_snapshot() -> ClaudeCapabilitySnapshot {
    // Cheap checks first: skip the `claude --version` subprocess when
    // workflows are already disabled.
    detect_claude_capability_snapshot_from(
        detect_claude_workflows_enabled(),
        detect_claude_version_raw,
    )
}

fn detect_claude_capability_snapshot_from(
    workflows_enabled: bool,
    detect_version_raw: impl FnOnce() -> Option<String>,
) -> ClaudeCapabilitySnapshot {
    let ultracode_supported =
        workflows_enabled && detect_version_raw().is_some_and(|raw| supports_ultracode(&raw, true));
    ClaudeCapabilitySnapshot {
        workflows_enabled,
        ultracode_supported,
    }
}

fn claude_capability_cache() -> &'static Mutex<Option<ClaudeCapabilitySnapshot>> {
    static CACHE: OnceLock<Mutex<Option<ClaudeCapabilitySnapshot>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn env_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        sync::{Mutex, OnceLock},
    };

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn lock_env_for_test() -> std::sync::MutexGuard<'static, ()> {
        env_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(test)]
    fn reset_claude_capability_snapshot_for_tests() {
        *claude_capability_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    struct CapabilitySnapshotReset;

    impl CapabilitySnapshotReset {
        fn new() -> Self {
            reset_claude_capability_snapshot_for_tests();
            Self
        }
    }

    impl Drop for CapabilitySnapshotReset {
        fn drop(&mut self) {
            reset_claude_capability_snapshot_for_tests();
        }
    }

    #[test]
    fn parses_decorated_version() {
        assert_eq!(
            parse_claude_semver("2.1.156 (Claude Code)"),
            Some(semver::Version::new(2, 1, 156))
        );
    }

    #[test]
    fn parses_bare_and_v_prefixed_versions() {
        assert_eq!(
            parse_claude_semver("2.1.154"),
            Some(semver::Version::new(2, 1, 154))
        );
        assert_eq!(
            parse_claude_semver("  v2.2.0  "),
            Some(semver::Version::new(2, 2, 0))
        );
    }

    #[test]
    fn rejects_garbage_version() {
        assert_eq!(parse_claude_semver("unknown"), None);
        assert_eq!(parse_claude_semver(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn live_version_probe_resolves_real_bun_global_placeholder_fixture() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        assert_eq!(
            detect_claude_version_raw().as_deref(),
            Some("2.1.210 (Claude Code)")
        );
    }

    #[cfg(windows)]
    #[test]
    fn live_version_probe_rejects_real_bun_global_placeholder_fixture_without_safe_target() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture =
            gwt_core::test_support::WindowsBunClaudeFixture::create(temp.path(), "2.1.210")
                .expect("create real Windows Bun fixture");
        fixture
            .remove_safe_targets()
            .expect("remove safe redirect targets");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", &fixture.bun_bin);
        let _path_ext = gwt_core::test_support::ScopedEnvVar::set("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        let _profile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", &fixture.profile);

        assert_eq!(detect_claude_version_raw(), None);
    }

    #[test]
    fn workflows_enabled_by_default() {
        assert!(workflows_enabled_from(None, None));
        assert!(workflows_enabled_from(None, Some("{}")));
    }

    #[test]
    fn workflows_disabled_by_env() {
        assert!(!workflows_enabled_from(Some("1"), None));
        assert!(!workflows_enabled_from(Some("true"), Some("{}")));
    }

    #[test]
    fn env_falsey_does_not_disable() {
        assert!(workflows_enabled_from(Some("0"), None));
        assert!(workflows_enabled_from(Some(""), None));
    }

    #[test]
    fn workflows_disabled_by_settings_key() {
        assert!(!workflows_enabled_from(
            None,
            Some(r#"{"disableWorkflows":true}"#)
        ));
    }

    #[test]
    fn settings_false_keeps_enabled() {
        assert!(workflows_enabled_from(
            None,
            Some(r#"{"disableWorkflows":false}"#)
        ));
    }

    #[test]
    fn malformed_settings_default_enabled() {
        assert!(workflows_enabled_from(None, Some("{not json")));
    }

    #[test]
    fn env_overrides_enabled_settings() {
        // env disable wins even if settings would keep workflows on
        assert!(!workflows_enabled_from(
            Some("1"),
            Some(r#"{"disableWorkflows":false}"#)
        ));
    }

    #[test]
    fn supports_ultracode_boundaries() {
        assert!(supports_ultracode("2.1.156 (Claude Code)", true));
        assert!(supports_ultracode("2.1.154", true));
        assert!(!supports_ultracode("2.1.153", true)); // below minimum
        assert!(!supports_ultracode("2.1.156", false)); // workflows disabled
        assert!(!supports_ultracode("garbage", true)); // unparseable version
    }

    #[test]
    fn live_capability_checks_reuse_session_snapshot_without_reprobe() {
        let _guard = lock_env_for_test();
        let _snapshot_reset = CapabilitySnapshotReset::new();
        let calls = AtomicUsize::new(0);
        let supported = ClaudeCapabilitySnapshot {
            workflows_enabled: true,
            ultracode_supported: true,
        };

        assert_eq!(
            claude_capability_snapshot_with_detector(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                supported
            }),
            supported
        );
        assert_eq!(
            claude_capability_snapshot_with_detector(|| panic!("snapshot should be cached")),
            supported
        );
        assert!(claude_workflows_enabled());
        assert!(claude_ultracode_supported());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn disabled_workflows_are_cached_and_skip_version_probe() {
        let _guard = lock_env_for_test();
        let _snapshot_reset = CapabilitySnapshotReset::new();
        let version_calls = AtomicUsize::new(0);
        let disabled = detect_claude_capability_snapshot_from(false, || {
            version_calls.fetch_add(1, Ordering::SeqCst);
            Some("2.1.156 (Claude Code)".to_string())
        });
        assert_eq!(
            disabled,
            ClaudeCapabilitySnapshot {
                workflows_enabled: false,
                ultracode_supported: false,
            }
        );
        assert_eq!(
            version_calls.load(Ordering::SeqCst),
            0,
            "disabled workflows should skip the version detector"
        );

        let detector_calls = AtomicUsize::new(0);
        assert_eq!(
            claude_capability_snapshot_with_detector(|| {
                detector_calls.fetch_add(1, Ordering::SeqCst);
                disabled
            }),
            disabled
        );
        assert_eq!(
            claude_capability_snapshot_with_detector(|| panic!(
                "disabled snapshot should be cached"
            )),
            disabled
        );
        assert!(!claude_workflows_enabled());
        assert!(!claude_ultracode_supported());
        assert_eq!(detector_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_version_probe_is_cached_as_unsupported() {
        let _guard = lock_env_for_test();
        let _snapshot_reset = CapabilitySnapshotReset::new();
        let version_calls = AtomicUsize::new(0);
        let unsupported = detect_claude_capability_snapshot_from(true, || {
            version_calls.fetch_add(1, Ordering::SeqCst);
            None
        });
        assert_eq!(
            unsupported,
            ClaudeCapabilitySnapshot {
                workflows_enabled: true,
                ultracode_supported: false,
            }
        );
        assert_eq!(version_calls.load(Ordering::SeqCst), 1);

        let detector_calls = AtomicUsize::new(0);
        assert_eq!(
            claude_capability_snapshot_with_detector(|| {
                detector_calls.fetch_add(1, Ordering::SeqCst);
                unsupported
            }),
            unsupported
        );
        assert!(!claude_ultracode_supported());
        assert!(!claude_ultracode_supported());
        assert!(claude_workflows_enabled());
        assert_eq!(
            claude_capability_snapshot_with_detector(|| panic!("failed probe should be cached")),
            unsupported
        );
        assert_eq!(detector_calls.load(Ordering::SeqCst), 1);
    }
}

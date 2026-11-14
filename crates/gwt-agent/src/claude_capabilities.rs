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
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Mutex,
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClaudeCapabilitySnapshot {
    workflows_enabled: bool,
    ultracode_supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaudeCapabilityCacheKey {
    env_disable: Option<OsString>,
    settings_path: Option<PathBuf>,
    claude_executable: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaudeCapabilityCacheEntry {
    key: ClaudeCapabilityCacheKey,
    snapshot: ClaudeCapabilitySnapshot,
}

static CLAUDE_CAPABILITY_CACHE: Mutex<Option<ClaudeCapabilityCacheEntry>> = Mutex::new(None);

fn claude_capability_cache_key() -> ClaudeCapabilityCacheKey {
    ClaudeCapabilityCacheKey {
        env_disable: std::env::var_os("CLAUDE_CODE_DISABLE_WORKFLOWS"),
        settings_path: claude_user_settings_path(),
        claude_executable: resolve_claude_executable(),
    }
}

fn claude_capability_snapshot() -> ClaudeCapabilitySnapshot {
    let key = claude_capability_cache_key();
    claude_capability_snapshot_for_key(key)
}

fn claude_capability_snapshot_for_key(key: ClaudeCapabilityCacheKey) -> ClaudeCapabilitySnapshot {
    let mut cache = CLAUDE_CAPABILITY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = cache.as_ref().filter(|entry| entry.key == key) {
        return entry.snapshot;
    }
    let snapshot = detect_claude_capability_snapshot(&key);
    *cache = Some(ClaudeCapabilityCacheEntry { key, snapshot });
    snapshot
}

fn resolve_claude_executable() -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match std::env::var_os("PATH") {
        Some(path) => which::which_in("claude", Some(path), cwd).ok(),
        None => which::which("claude").ok(),
    }
}

#[cfg(test)]
fn reset_claude_capability_cache_for_tests() {
    *CLAUDE_CAPABILITY_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

fn detect_claude_capability_snapshot(key: &ClaudeCapabilityCacheKey) -> ClaudeCapabilitySnapshot {
    let env_disable = key.env_disable.as_ref().and_then(|value| value.to_str());
    let settings = key
        .settings_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    let workflows_enabled = workflows_enabled_from(env_disable, settings.as_deref());
    let ultracode_supported = workflows_enabled
        && key
            .claude_executable
            .as_ref()
            .and_then(detect_claude_version_raw_at)
            .is_some_and(|raw| supports_ultracode(&raw, true));
    ClaudeCapabilitySnapshot {
        workflows_enabled,
        ultracode_supported,
    }
}

/// Detect whether Claude Code dynamic workflows are enabled in the current
/// environment (env var + user global settings). Defaults to `true` when
/// nothing can be read. The live result is cached for the GUI process because
/// it is session-stable and is read from Launch Wizard hot paths.
pub fn claude_workflows_enabled() -> bool {
    claude_capability_snapshot().workflows_enabled
}

/// Raw `claude --version` output (e.g. `"2.1.156 (Claude Code)"`), or `None`
/// when the `claude` binary is missing or the call fails. Run sparingly (it
/// spawns a subprocess): the Launch Wizard captures this once at open time,
/// not on every render.
pub fn detect_claude_version_raw() -> Option<String> {
    detect_claude_version_raw_at(OsStr::new("claude"))
}

fn detect_claude_version_raw_at(program: impl AsRef<OsStr>) -> Option<String> {
    let output = gwt_core::process::hidden_command(program)
        .arg("--version")
        .output()
        .ok()?;
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
/// The GUI process computes this once and stores the result, so Launch Wizard
/// open/render paths stay free of repeated subprocess/file I/O.
pub fn claude_ultracode_supported() -> bool {
    claude_capability_snapshot().ultracode_supported
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

    #[cfg(unix)]
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    #[cfg(unix)]
    fn write_fake_claude(bin_dir: &std::path::Path, script: String) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(bin_dir).expect("bin dir");
        let fake_claude = bin_dir.join("claude");
        std::fs::write(&fake_claude, script).expect("fake claude");
        let mut perms = std::fs::metadata(&fake_claude)
            .expect("fake claude metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_claude, perms).expect("chmod fake claude");
        fake_claude
    }

    #[cfg(unix)]
    fn test_cache_key(
        temp: &tempfile::TempDir,
        claude_executable: PathBuf,
        env_disable: Option<&str>,
    ) -> ClaudeCapabilityCacheKey {
        ClaudeCapabilityCacheKey {
            env_disable: env_disable.map(OsString::from),
            settings_path: Some(temp.path().join(".claude").join("settings.json")),
            claude_executable: Some(claude_executable),
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

    #[cfg(unix)]
    #[test]
    fn live_ultracode_detection_reuses_session_probe() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_claude_capability_cache_for_tests();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let probe_count = temp.path().join("probe-count");
        let fake_claude = write_fake_claude(
            &bin_dir,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nprintf '2.1.156 (Claude Code)\\n'\n",
                probe_count.display()
            ),
        );
        let key = test_cache_key(&temp, fake_claude, None);

        assert!(claude_capability_snapshot_for_key(key.clone()).ultracode_supported);
        assert!(claude_capability_snapshot_for_key(key).ultracode_supported);

        let count = std::fs::read_to_string(&probe_count).expect("probe count");
        assert_eq!(
            count, "x",
            "live capability detection should spawn claude --version once per GUI process"
        );
        reset_claude_capability_cache_for_tests();
    }

    #[cfg(unix)]
    #[test]
    fn live_capability_cache_skips_version_probe_when_workflows_disabled() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_claude_capability_cache_for_tests();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let probe_count = temp.path().join("probe-count");
        let fake_claude = write_fake_claude(
            &bin_dir,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nprintf '2.1.156 (Claude Code)\\n'\n",
                probe_count.display()
            ),
        );
        let key = test_cache_key(&temp, fake_claude, Some("1"));

        let snapshot = claude_capability_snapshot_for_key(key.clone());
        assert!(!snapshot.workflows_enabled);
        assert!(!snapshot.ultracode_supported);
        assert!(!claude_capability_snapshot_for_key(key).ultracode_supported);
        assert!(
            !probe_count.exists(),
            "disabled workflows should avoid claude --version entirely"
        );
        reset_claude_capability_cache_for_tests();
    }

    #[cfg(unix)]
    #[test]
    fn live_capability_cache_reuses_failed_version_probe() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        reset_claude_capability_cache_for_tests();
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        let probe_count = temp.path().join("probe-count");
        let fake_claude = write_fake_claude(
            &bin_dir,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nprintf 'boom\\n' >&2\nexit 42\n",
                probe_count.display()
            ),
        );
        let key = test_cache_key(&temp, fake_claude, None);

        assert!(!claude_capability_snapshot_for_key(key.clone()).ultracode_supported);
        assert!(!claude_capability_snapshot_for_key(key).ultracode_supported);
        let count = std::fs::read_to_string(&probe_count).expect("probe count");
        assert_eq!(
            count, "x",
            "failed version probe should also be cached for the GUI process"
        );
        reset_claude_capability_cache_for_tests();
    }
}

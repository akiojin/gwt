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

use std::path::PathBuf;

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
    let env_disable = std::env::var("CLAUDE_CODE_DISABLE_WORKFLOWS").ok();
    let settings = claude_user_settings_path().and_then(|path| std::fs::read_to_string(path).ok());
    workflows_enabled_from(env_disable.as_deref(), settings.as_deref())
}

/// Raw `claude --version` output (e.g. `"2.1.156 (Claude Code)"`), or `None`
/// when the `claude` binary is missing or the call fails. Run sparingly (it
/// spawns a subprocess): the Launch Wizard captures this once at open time,
/// not on every render.
pub fn detect_claude_version_raw() -> Option<String> {
    let output = gwt_core::process::hidden_command("claude")
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
/// The Launch Wizard computes this once at open time and stores the result, so
/// the per-render reasoning-option logic stays free of subprocess/file I/O.
pub fn claude_ultracode_supported() -> bool {
    // Cheap checks first: skip the `claude --version` subprocess when
    // workflows are already disabled.
    if !claude_workflows_enabled() {
        return false;
    }
    detect_claude_version_raw().is_some_and(|raw| supports_ultracode(&raw, true))
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
}

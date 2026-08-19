//! Managed hook health read model.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_agent::PendingDiscussionResume;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLOW_HANDLER_THRESHOLD_MS: f64 = 1000.0;
const SELF_HEALED_MARKER: &str = ".gwt/managed-hook-self-healed";
const MANAGED_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHookHealthStatus {
    Ready,
    NeedsAttention,
    SelfHealed,
    Degraded,
    Inactive,
    WaitingForFirstHookEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingHookGoal {
    pub proposal_label: String,
    pub proposal_title: String,
    pub condition: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookProfileEvidence {
    pub event: String,
    pub handler: String,
    pub status: String,
    pub duration_ms: f64,
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManagedHookHealth {
    pub status: ManagedHookHealthStatus,
    pub last_event: Option<String>,
    pub last_event_at: Option<String>,
    pub pending_discussion: Option<PendingDiscussionResume>,
    pub pending_goal: Option<PendingHookGoal>,
    pub slow_handlers: Vec<HookProfileEvidence>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHookHealthInput {
    pub worktree_root: PathBuf,
    pub runtime_state_path: Option<PathBuf>,
    pub profile_path: Option<PathBuf>,
    pub expected_hook_bin: Option<String>,
}

impl ManagedHookHealthInput {
    pub fn new(worktree_root: impl AsRef<Path>) -> Self {
        Self {
            worktree_root: worktree_root.as_ref().to_path_buf(),
            runtime_state_path: std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
                .map(PathBuf::from),
            profile_path: None,
            expected_hook_bin: std::env::var("GWT_HOOK_BIN")
                .ok()
                .filter(|value| !value.is_empty()),
        }
    }

    pub fn with_runtime_state_path(mut self, path: impl AsRef<Path>) -> Self {
        self.runtime_state_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_profile_path(mut self, path: impl AsRef<Path>) -> Self {
        self.profile_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_expected_hook_bin(mut self, bin: impl Into<String>) -> Self {
        self.expected_hook_bin = Some(bin.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedHookRepairOutcome {
    pub repaired: bool,
}

#[derive(Debug, Deserialize)]
struct RuntimeStateReadModel {
    pub status: String,
    pub updated_at: String,
    #[allow(dead_code)]
    pub last_activity_at: String,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

pub fn read_managed_hook_health(input: &ManagedHookHealthInput) -> ManagedHookHealth {
    let mut health = ManagedHookHealth {
        status: ManagedHookHealthStatus::Ready,
        last_event: None,
        last_event_at: None,
        pending_discussion: None,
        pending_goal: crate::discussion_resume::load_pending_goal_from_worktree_files(
            &input.worktree_root,
        )
        .ok()
        .flatten()
        .map(|goal| PendingHookGoal {
            proposal_label: goal.proposal_label,
            proposal_title: goal.proposal_title,
            condition: goal.condition,
        }),
        slow_handlers: Vec::new(),
        issues: Vec::new(),
    };

    audit_managed_hook_configs(input, &mut health);
    audit_hook_profile(input, &mut health);

    let Some(runtime_state_path) = input.runtime_state_path.as_ref() else {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::Inactive;
        }
        apply_self_healed_marker(input, &mut health);
        return health;
    };

    if !runtime_state_path.exists() {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
        }
        apply_self_healed_marker(input, &mut health);
        return health;
    }

    match read_runtime_state(runtime_state_path) {
        Ok(runtime_state) => {
            if let Some(source_event) = runtime_state.source_event {
                health.last_event = Some(source_event);
                health.last_event_at = Some(runtime_state.updated_at);
            } else if health.status == ManagedHookHealthStatus::Ready {
                health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
            }
            health.pending_discussion = runtime_state.pending_discussion;
            if runtime_state.status == "Stopped" && health.status == ManagedHookHealthStatus::Ready
            {
                health.status = ManagedHookHealthStatus::Inactive;
            }
        }
        Err(error) => {
            health.status = ManagedHookHealthStatus::Degraded;
            health
                .issues
                .push(format!("runtime state could not be read: {}", error));
        }
    }

    apply_self_healed_marker(input, &mut health);
    health
}

fn apply_self_healed_marker(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    if input.worktree_root.join(SELF_HEALED_MARKER).is_file()
        && matches!(
            health.status,
            ManagedHookHealthStatus::Ready
                | ManagedHookHealthStatus::Inactive
                | ManagedHookHealthStatus::WaitingForFirstHookEvent
        )
    {
        health.status = ManagedHookHealthStatus::SelfHealed;
    }
}

pub fn record_managed_hook_self_healed(worktree_root: &Path) -> io::Result<()> {
    let marker = worktree_root.join(SELF_HEALED_MARKER);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(marker, chrono::Utc::now().to_rfc3339())
}

fn audit_hook_profile(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    let Some(profile_path) = input.profile_path.as_ref() else {
        return;
    };
    if !profile_path.exists() {
        return;
    }

    let Ok(raw) = fs::read_to_string(profile_path) else {
        needs_attention(
            health,
            format!("hook profile could not be read: {}", profile_path.display()),
        );
        return;
    };

    for (index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            needs_attention(
                health,
                format!(
                    "hook profile line {} is not valid JSON: {}",
                    index + 1,
                    profile_path.display()
                ),
            );
            continue;
        };
        let Some(duration_ms) = record.get("duration_ms").and_then(Value::as_f64) else {
            continue;
        };
        if duration_ms < SLOW_HANDLER_THRESHOLD_MS {
            continue;
        }
        let event = record
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let handler = record
            .get("handler")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let status = record
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let occurred_at = record
            .get("occurred_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        health.slow_handlers.push(HookProfileEvidence {
            event: event.clone(),
            handler: handler.clone(),
            status,
            duration_ms,
            occurred_at,
        });
        needs_attention(
            health,
            format!("slow managed hook handler: {event}/{handler} took {duration_ms:.1}ms"),
        );
    }
}

fn audit_managed_hook_configs(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    let worktree = &input.worktree_root;
    let claude_dir = worktree.join(".claude");
    let claude_settings = worktree.join(".claude/settings.local.json");
    // #3474: audit every `.codex/hooks.json` the self-heal writer owns, not
    // just the worktree-local one. For a linked worktree the writer targets the
    // repo-root (workspace-home) copy that newer Codex reads, so auditing only
    // the worktree-local copy reported a file nothing would ever rewrite.
    let codex_hooks_paths = crate::managed_assets::managed_codex_hook_paths(worktree);
    let provider_hooks = [
        (
            worktree.join(".gwt/opencode"),
            worktree.join(".gwt/opencode/plugins/gwt-hooks.js"),
        ),
        (
            worktree.join(".gwt/openclaw"),
            worktree.join(".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts"),
        ),
        (
            worktree.join(".gwt/hermes"),
            worktree.join(".gwt/hermes/agent-hooks/gwt-hook.sh"),
        ),
    ];

    // Whether this worktree has a gwt surface at all stays a worktree-local
    // question: the workspace-home copy is shared by every worktree, so it must
    // never make an unmaterialized one report hook health.
    let codex_dir = worktree.join(".codex");
    let has_surface = claude_dir.exists()
        || claude_settings.exists()
        || codex_dir.exists()
        || worktree.join(".codex/hooks.json").exists()
        || provider_hooks
            .iter()
            .any(|(root, artifact)| root.exists() || artifact.exists());
    if !has_surface {
        health.status = ManagedHookHealthStatus::Inactive;
        return;
    }

    if claude_dir.exists() && !claude_settings.exists() {
        needs_attention(
            health,
            "managed hook config missing: .claude/settings.local.json",
        );
    }
    for hooks in &codex_hooks_paths {
        if codex_root_of(hooks).exists() && !hooks.exists() {
            needs_attention(
                health,
                format!("managed hook config missing: {}", hooks.display()),
            );
        }
    }

    if claude_settings.exists() {
        audit_hook_json_config(&claude_settings, input.expected_hook_bin.as_deref(), health);
    }
    for hooks in &codex_hooks_paths {
        if hooks.exists() {
            audit_hook_json_config(hooks, input.expected_hook_bin.as_deref(), health);
        }
    }

    for (root, artifact) in provider_hooks {
        if artifact.exists() {
            audit_provider_hook_config(&artifact, input.expected_hook_bin.as_deref(), health);
        } else if root.exists() {
            needs_attention(
                health,
                format!("managed hook config missing: {}", artifact.display()),
            );
        }
    }
}

fn audit_hook_json_config(
    path: &Path,
    expected_hook_bin: Option<&str>,
    health: &mut ManagedHookHealth,
) {
    let Ok(raw) = fs::read_to_string(path) else {
        degraded(
            health,
            format!("managed hook config could not be read: {}", path.display()),
        );
        return;
    };
    let Ok(root) = serde_json::from_str::<Value>(&raw) else {
        degraded(
            health,
            format!("managed hook config is not valid JSON: {}", path.display()),
        );
        return;
    };

    for event in MANAGED_EVENTS {
        let commands = hook_commands_for_event(&root, event);
        if !commands
            .iter()
            .any(|command| is_managed_event_command(command, event))
        {
            needs_attention(
                health,
                format!(
                    "managed hook event missing: {} in {}",
                    event,
                    path.display()
                ),
            );
        }
        for command in &commands {
            if !is_managed_event_command(command, event) {
                continue;
            }
            if !command.contains("GWT_BIN_PATH") {
                needs_attention(
                    health,
                    format!("managed hook runtime resolver missing: {}", path.display()),
                );
            }
            if !has_runtime_guard(command) {
                needs_attention(
                    health,
                    format!("managed hook runtime guard missing: {}", path.display()),
                );
            }
        }
        if let Some(expected) = expected_hook_bin {
            for command in commands {
                if !is_managed_event_command(&command, event) {
                    continue;
                }
                let Some(actual) = hook_command_binary_fallback(&command) else {
                    continue;
                };
                audit_hook_binary(path, &actual, Some(expected), health);
            }
        } else {
            for command in commands {
                if !is_managed_event_command(&command, event) {
                    continue;
                }
                if let Some(actual) = hook_command_binary_fallback(&command) {
                    audit_hook_binary(path, &actual, None, health);
                }
            }
        }
    }
}

fn audit_provider_hook_config(
    path: &Path,
    expected_hook_bin: Option<&str>,
    health: &mut ManagedHookHealth,
) {
    let Ok(raw) = fs::read_to_string(path) else {
        degraded(
            health,
            format!("managed hook config could not be read: {}", path.display()),
        );
        return;
    };
    if !raw.contains("GWT_BIN_PATH") {
        needs_attention(
            health,
            format!("managed hook runtime resolver missing: {}", path.display()),
        );
    }
    let Some(actual) = hook_command_binary_fallback(&raw) else {
        needs_attention(
            health,
            format!("managed hook runtime resolver missing: {}", path.display()),
        );
        return;
    };
    audit_hook_binary(path, &actual, expected_hook_bin, health);
}

fn audit_hook_binary(
    path: &Path,
    actual: &str,
    expected_hook_bin: Option<&str>,
    health: &mut ManagedHookHealth,
) {
    let actual_path = Path::new(actual);
    let explicitly_pinned = expected_hook_bin.is_some_and(|expected| expected == actual);
    if crate::managed_assets::is_worktree_local_build_binary(actual_path) && !explicitly_pinned {
        degraded(
            health,
            format!(
                "managed hook worktree-local binary: {} uses {}",
                path.display(),
                actual
            ),
        );
        return;
    }
    if let Some(expected) = expected_hook_bin {
        if actual != expected {
            degraded(
                health,
                format!(
                    "managed hook binary skew: {} uses {}, expected {}",
                    path.display(),
                    actual,
                    expected
                ),
            );
        }
    }
    if looks_absolute(actual) {
        if !actual_path.is_file() {
            degraded(
                health,
                format!(
                    "managed hook binary missing: {} uses {}",
                    path.display(),
                    actual
                ),
            );
        } else if which::which(actual).is_err() {
            degraded(
                health,
                format!(
                    "managed hook binary not executable: {} uses {}",
                    path.display(),
                    actual
                ),
            );
        }
    } else if !bare_hook_binary_is_resolvable(actual) {
        degraded(
            health,
            format!(
                "managed hook binary missing: {} uses {}",
                path.display(),
                actual
            ),
        );
    }
}

/// Whether a bare-name hook fallback such as `gwtd` resolves to a real binary.
///
/// #3474 root cause 4: `which` searches the *calling process's* PATH. A gwt GUI
/// launched from Finder or the Dock inherits launchd's PATH, which lacks
/// `/Applications/GWT.app/Contents/MacOS`, so the same fallback that resolves
/// in a terminal — and always resolves for a gwt-launched agent, which gets
/// `GWT_BIN_PATH` injected and its directory prepended to PATH — was reported
/// as missing and turned every Work card red. Fall back to gwt's own
/// PATH-independent resolver, and only accept a hit that actually names the
/// binary the hook asks for.
fn bare_hook_binary_is_resolvable(actual: &str) -> bool {
    if which::which(actual).is_ok() {
        return true;
    }
    crate::cli::gwtd_resolver::resolve_gwtd_path()
        .is_some_and(|resolved| binary_names_match(&resolved, actual))
}

fn binary_names_match(resolved: &Path, actual: &str) -> bool {
    let resolved = resolved.file_name().and_then(|name| name.to_str());
    resolved.is_some_and(|resolved| {
        strip_exe_suffix(resolved).eq_ignore_ascii_case(strip_exe_suffix(actual))
    })
}

fn strip_exe_suffix(value: &str) -> &str {
    value
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(value, |(stem, _)| stem)
}

fn codex_root_of(hooks_path: &Path) -> PathBuf {
    hooks_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

/// Whether a generated managed event command degrades to a no-op when its
/// binary cannot be resolved, instead of hard-failing the agent's hook.
///
/// #3474: the template committed before `b8fa26c04` / `2c660f11e` invoked
/// `"$gwt_bin"` unconditionally, so a Codex started outside gwt (no
/// `GWT_BIN_PATH`, no `gwtd` on PATH) failed every hook with
/// `command not found`. The current POSIX template guards the call with
/// `command -v`, and the PowerShell template wraps it in `try`/`catch`. A
/// missing guard is its own issue class so the startup self-heal loop breaker —
/// which skips a worktree whose issues are *only* `managed hook binary
/// missing:` — can never strand a legacy config (root cause 3).
fn has_runtime_guard(command: &str) -> bool {
    command.contains("command -v ") || command.contains("catch {")
}

fn hook_commands_for_event(root: &Value, event: &str) -> Vec<String> {
    let Some(groups) = root
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flat_map(|hooks| hooks.iter())
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn is_managed_event_command(command: &str, event: &str) -> bool {
    command.contains(&format!("hook event {event}"))
}

fn hook_command_binary_fallback(command: &str) -> Option<String> {
    if let Some(value) = posix_shell_assignment_value(command, "gwt_fallback") {
        return Some(value);
    }
    if let Some(rest) = command
        .split_once("gwt_bin=\"${GWT_BIN_PATH:-}\"")
        .map(|(_, rest)| rest)
    {
        if let Some(value) = posix_shell_assignment_value(rest, "gwt_bin") {
            return Some(value);
        }
    }
    if let Some(rest) = command.split_once("${GWT_BIN_PATH:-").map(|(_, rest)| rest) {
        let value = rest.split_once('}')?.0;
        return nonempty_unquoted(value);
    }
    if let Some(rest) = command
        .split_once("process.env.GWT_BIN_PATH ||")
        .map(|(_, rest)| rest.trim_start())
    {
        let value = rest.split_once(';').map_or(rest, |(value, _)| value).trim();
        if let Ok(value) = serde_json::from_str::<String>(value) {
            return (!value.is_empty()).then_some(value);
        }
        return nonempty_unquoted(value);
    }
    if let Some(rest) = command.split_once("else {").map(|(_, rest)| rest) {
        let value = rest.split_once('}')?.0;
        return nonempty_powershell_quoted(value);
    }
    let (prefix, _) = command.split_once(" hook ")?;
    nonempty_unquoted(prefix)
}

fn posix_shell_assignment_value(command: &str, variable: &str) -> Option<String> {
    let assignment = format!("{variable}=");
    let rest = command.split_once(&assignment)?.1.trim_start();
    posix_shell_word(rest)
}

fn posix_shell_word(value: &str) -> Option<String> {
    if value.starts_with('\'') {
        let mut cursor = 1;
        loop {
            let closing = cursor + value.get(cursor..)?.find('\'')?;
            if value
                .get(closing + 1..)
                .is_some_and(|rest| rest.starts_with("\\''"))
            {
                cursor = closing + 4;
                continue;
            }
            let token = value.get(..=closing)?;
            let decoded = token
                .strip_prefix('\'')?
                .strip_suffix('\'')?
                .replace(r"'\''", "'");
            return (!decoded.is_empty()).then_some(decoded);
        }
    }

    let end = value
        .find(|character: char| character == ';' || character.is_whitespace())
        .unwrap_or(value.len());
    nonempty_unquoted(value.get(..end)?)
}

fn nonempty_powershell_quoted(value: &str) -> Option<String> {
    nonempty_unquoted(value).map(|value| value.replace("''", "'"))
}

fn nonempty_unquoted(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .replace("\\\"", "\"")
        .replace("\\$", "$")
        .replace("\\`", "`")
        .replace("\\\\", "\\");
    (!value.is_empty()).then_some(value)
}

fn looks_absolute(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn needs_attention(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    if health.status == ManagedHookHealthStatus::Ready
        || health.status == ManagedHookHealthStatus::Inactive
        || health.status == ManagedHookHealthStatus::WaitingForFirstHookEvent
    {
        health.status = ManagedHookHealthStatus::NeedsAttention;
    }
    push_unique_issue(health, issue.into());
}

fn degraded(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    health.status = ManagedHookHealthStatus::Degraded;
    push_unique_issue(health, issue.into());
}

fn push_unique_issue(health: &mut ManagedHookHealth, issue: String) {
    if !health.issues.contains(&issue) {
        health.issues.push(issue);
    }
}

fn read_runtime_state(path: &Path) -> Result<RuntimeStateReadModel, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

pub fn repair_managed_hook_configs(worktree_root: &Path) -> io::Result<ManagedHookRepairOutcome> {
    let claude_surface = worktree_root.join(".claude").exists()
        || worktree_root.join(".claude/settings.local.json").exists();
    let codex_surface =
        worktree_root.join(".codex").exists() || worktree_root.join(".codex/hooks.json").exists();
    let provider_surface = worktree_root.join(".gwt/opencode").exists()
        || worktree_root.join(".gwt/openclaw").exists()
        || worktree_root.join(".gwt/hermes").exists();
    let mut repaired = false;

    if claude_surface || codex_surface || provider_surface {
        crate::managed_assets::regenerate_existing_managed_hook_configs(worktree_root)?;
        record_managed_hook_self_healed(worktree_root)?;
        repaired = true;
    }

    Ok(ManagedHookRepairOutcome { repaired })
}

#[cfg(test)]
mod tests {
    use super::hook_command_binary_fallback;

    #[test]
    fn posix_two_stage_runtime_fallback_decodes_embedded_apostrophes() {
        let command = r#"gwt_bin="${GWT_BIN_PATH:-}"; if [ -z "$gwt_bin" ]; then gwt_bin='/opt/GWT O'\''Brien}/gwtd'; fi; if command -v "$gwt_bin" >/dev/null 2>&1; then "$gwt_bin" hook event Stop; else true; fi"#;

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("/opt/GWT O'Brien}/gwtd")
        );
    }

    #[test]
    fn posix_named_runtime_fallback_decodes_embedded_apostrophes() {
        let command = r#"gwt_fallback='/opt/GWT O'\''Brien}/gwtd'; gwt_bin="${GWT_BIN_PATH:-$gwt_fallback}"; "$gwt_bin" hook event Stop"#;

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("/opt/GWT O'Brien}/gwtd")
        );
    }

    #[test]
    fn powershell_runtime_fallback_decodes_embedded_apostrophes() {
        let command = "powershell -NoProfile -Command \"& { $gwtBin = if ($env:GWT_BIN_PATH) { $env:GWT_BIN_PATH } else { 'C:\\Users\\O''Brien\\GWT\\gwtd.exe' }; & $gwtBin hook event Stop }\"";

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("C:\\Users\\O'Brien\\GWT\\gwtd.exe")
        );
    }
}

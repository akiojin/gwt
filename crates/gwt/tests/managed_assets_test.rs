use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

use gwt::refresh_managed_gwt_assets_for_worktree;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn refresh_managed_gwt_assets_materializes_skills_commands_hooks_and_excludes() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("refresh managed assets");

    assert!(dir
        .path()
        .join(".claude/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(dir
        .path()
        .join(".claude/commands/gwt-build-spec.md")
        .exists());
    assert!(dir
        .path()
        .join(".codex/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(dir.path().join(".claude/settings.local.json").exists());
    assert!(dir.path().join(".codex/hooks.json").exists());
    let claude_settings = std::fs::read_to_string(dir.path().join(".claude/settings.local.json"))
        .expect("read claude");
    let codex_hooks =
        std::fs::read_to_string(dir.path().join(".codex/hooks.json")).expect("read codex");
    let cli_bin_text = cli_bin.display().to_string();
    // Diagnostic-rich asserts: if the test ever flakes on CI again,
    // the failure message includes the resolved cli_bin path, the
    // observed GWT_HOOK_BIN env value, and a redacted view of the
    // generated commands so we can see WHICH command shape mismatched
    // instead of just `assertion failed`.
    let observed_env = std::env::var("GWT_HOOK_BIN").unwrap_or_else(|_| "<unset>".to_string());
    let claude_commands = json_commands(&claude_settings);
    assert!(
        claude_commands
            .iter()
            .any(|command| command.contains(&cli_bin_text)),
        "claude settings missing cli_bin path\n  cli_bin_text: {cli_bin_text}\n  GWT_HOOK_BIN env: {observed_env}\n  generated commands ({} entries):\n{}",
        claude_commands.len(),
        claude_commands
            .iter()
            .map(|c| format!("    - {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let codex_commands = json_commands(&codex_hooks);
    assert!(
        codex_commands
            .iter()
            .any(|command| command.contains(&cli_bin_text)),
        "codex hooks missing cli_bin path\n  cli_bin_text: {cli_bin_text}\n  GWT_HOOK_BIN env: {observed_env}\n  generated commands ({} entries):\n{}",
        codex_commands.len(),
        codex_commands
            .iter()
            .map(|c| format!("    - {c}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    let exclude_path = dir.path().join(".git/info/exclude");
    let exclude = std::fs::read_to_string(&exclude_path).expect("read exclude");
    assert!(exclude.contains(".claude/skills/gwt-*"));
    assert!(exclude.contains(".claude/commands/gwt-*"));
    assert!(exclude.contains(".codex/skills/gwt-*"));
}

#[test]
fn refresh_managed_gwt_assets_reports_the_failed_step() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("not-a-worktree");
    std::fs::write(&file_path, "plain file").expect("write file");

    let error = refresh_managed_gwt_assets_for_worktree(&file_path)
        .expect_err("refresh should fail for a file path");

    assert!(error
        .to_string()
        .contains("failed to distribute gwt managed assets"));
}

#[test]
fn refresh_managed_gwt_assets_keeps_command_assets_on_gwtd_cli_surface() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("refresh managed assets");

    let manage_pr = std::fs::read_to_string(dir.path().join(".claude/commands/gwt-manage-pr.md"))
        .expect("read gwt-manage-pr");
    assert!(
        manage_pr.contains("GWT_BIN_PATH"),
        "PR command asset should tell managed sessions to use GWT_BIN_PATH, got: {manage_pr}"
    );
    assert!(
        manage_pr.contains("resolve_gwt_bin()"),
        "PR command asset should define the gwtd resolver, got: {manage_pr}"
    );
    assert!(
        manage_pr.contains("command -v gwtd"),
        "PR command asset should fall back to PATH gwtd, got: {manage_pr}"
    );
    assert!(
        manage_pr.contains("target/debug/gwtd"),
        "PR command asset should fall back to repo-local gwtd, got: {manage_pr}"
    );
    assert!(
        manage_pr.contains("gwtd not found"),
        "PR command asset should fail with an actionable gwtd error, got: {manage_pr}"
    );
    assert!(
        !manage_pr.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwtd}\""),
        "PR command asset must not fall back directly to a bare gwtd lookup, got: {manage_pr}"
    );
    assert!(
        !manage_pr.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwt}\""),
        "PR command asset must not default to the GUI front door, got: {manage_pr}"
    );

    let release = std::fs::read_to_string(dir.path().join(".claude/commands/release.md"))
        .expect("read release command");
    assert!(
        release.contains("GWT_BIN_PATH"),
        "release command asset should shell out through GWT_BIN_PATH, got: {release}"
    );
    assert!(
        release.contains("resolve_gwt_bin()"),
        "release command asset should define the gwtd resolver, got: {release}"
    );
    assert!(
        release.contains("command -v gwtd"),
        "release command asset should fall back to PATH gwtd, got: {release}"
    );
    assert!(
        release.contains("target/debug/gwtd"),
        "release command asset should fall back to repo-local gwtd, got: {release}"
    );
    assert!(
        release.contains("gwtd not found"),
        "release command asset should fail with an actionable gwtd error, got: {release}"
    );
    assert!(
        !release.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwtd}\""),
        "release command asset must not fall back directly to a bare gwtd lookup, got: {release}"
    );
    assert!(
        !release.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwt}\""),
        "release command asset must not default to the GUI front door, got: {release}"
    );
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn json_commands(raw: &str) -> Vec<String> {
    fn collect(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(command) = map.get("command").and_then(Value::as_str) {
                    out.push(command.to_string());
                }
                for value in map.values() {
                    collect(value, out);
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect(value, out);
                }
            }
            _ => {}
        }
    }

    let value: Value = serde_json::from_str(raw).expect("valid json");
    let mut out = Vec::new();
    collect(&value, &mut out);
    out
}

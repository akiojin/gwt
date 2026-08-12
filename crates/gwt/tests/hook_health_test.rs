//! SPEC #1935 Phase 22 — managed hook health read model tests.

use std::{fs, path::Path};

use gwt::cli::hook::{
    health::{
        read_managed_hook_health, repair_managed_hook_configs, ManagedHookHealthInput,
        ManagedHookHealthStatus,
    },
    runtime_state::{self, RuntimeState},
};
use gwt_agent::PendingDiscussionResume;
use serde_json::json;

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

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
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

struct StableHookBinGuard {
    _tempdir: tempfile::TempDir,
    _env: ScopedEnvVar,
    path: std::path::PathBuf,
}

impl StableHookBinGuard {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn stable_hook_bin_guard() -> StableHookBinGuard {
    let tempdir = tempfile::tempdir().expect("stable hook bin root");
    let stable = tempdir
        .path()
        .join(if cfg!(windows) { "gwtd.exe" } else { "gwtd" });
    fs::write(&stable, "stable").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&stable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&stable, permissions).unwrap();
    }
    let env = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    StableHookBinGuard {
        _tempdir: tempdir,
        _env: env,
        path: stable,
    }
}

#[test]
fn managed_hook_health_is_ready_when_assets_and_runtime_state_are_current() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");
    runtime_state::write_for_event(&runtime_path, "PreToolUse").expect("runtime state");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_runtime_state_path(&runtime_path),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::Ready);
    assert_eq!(health.last_event.as_deref(), Some("PreToolUse"));
    assert!(health.last_event_at.is_some());
    assert!(health.pending_discussion.is_none());
    assert!(health.pending_goal.is_none());
    assert!(health.slow_handlers.is_empty());
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

#[test]
fn managed_hook_health_defaults_to_the_session_runtime_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");
    runtime_state::write_for_event(&runtime_path, "PreToolUse").expect("runtime state");
    let _runtime_path = ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::Ready);
    assert_eq!(health.last_event.as_deref(), Some("PreToolUse"));
    assert!(health.last_event_at.is_some());
}

#[test]
fn managed_hook_health_waits_for_first_event_when_session_start_is_delayed() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_runtime_state_path(&runtime_path),
    );

    assert_eq!(
        health.status,
        ManagedHookHealthStatus::WaitingForFirstHookEvent
    );
    assert!(health.last_event.is_none());
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

#[test]
fn managed_hook_health_tolerates_legacy_runtime_state_with_null_source_event() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");
    fs::write(
        &runtime_path,
        serde_json::to_vec_pretty(&json!({
            "status": "Running",
            "updated_at": "2026-06-24T06:27:01Z",
            "last_activity_at": "2026-06-24T06:27:01Z",
            "source_event": null,
            "pending_discussion": null
        }))
        .expect("serialize legacy runtime state"),
    )
    .expect("write legacy runtime state");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_runtime_state_path(&runtime_path),
    );

    assert_eq!(
        health.status,
        ManagedHookHealthStatus::WaitingForFirstHookEvent
    );
    assert!(health.last_event.is_none());
    assert!(health.last_event_at.is_none());
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

#[test]
fn managed_hook_health_projects_pending_discussion_and_goal() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");
    let now = "2026-06-17T00:00:00Z".to_string();
    let runtime_state = RuntimeState {
        status: "Running".to_string(),
        updated_at: now.clone(),
        last_activity_at: now,
        source_event: "UserPromptSubmit".to_string(),
        pending_discussion: Some(PendingDiscussionResume {
            proposal_label: "Proposal A".to_string(),
            proposal_title: "Hook health".to_string(),
            next_question: Some("Which surface should show this?".to_string()),
        }),
    };
    fs::write(
        &runtime_path,
        serde_json::to_vec_pretty(&runtime_state).expect("serialize runtime state"),
    )
    .expect("write runtime state");
    let discussion_path = worktree.path().join(".gwt/work/discussions.md");
    fs::create_dir_all(discussion_path.parent().expect("discussion parent")).unwrap();
    fs::write(
        discussion_path,
        "# Discussions\n\n\
## 2026-06-17 — Hook health\n\n\
Status: active\n\n\
### Proposal A - Hook health [chosen]\n\
- Goal State: pending\n\
- Goal Condition: implement backend hook health first\n",
    )
    .expect("write discussion");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_runtime_state_path(&runtime_path),
    );

    let pending_discussion = health
        .pending_discussion
        .as_ref()
        .expect("pending discussion");
    assert_eq!(pending_discussion.proposal_label, "Proposal A");
    assert_eq!(
        pending_discussion.next_question.as_deref(),
        Some("Which surface should show this?")
    );
    let pending_goal = health.pending_goal.as_ref().expect("pending goal");
    assert_eq!(pending_goal.proposal_label, "Proposal A");
    assert_eq!(
        pending_goal.condition,
        "implement backend hook health first"
    );
}

#[test]
fn managed_hook_health_detects_missing_managed_configs_and_repair_recreates_them() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    fs::create_dir_all(worktree.path().join(".codex")).expect("codex dir");

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains(".codex/hooks.json")),
        "{:?}",
        health.issues
    );

    let outcome = repair_managed_hook_configs(worktree.path()).expect("repair");

    assert!(outcome.repaired);
    assert!(worktree.path().join(".codex/hooks.json").exists());
}

#[test]
fn managed_hook_health_detects_missing_provider_artifacts_and_repair_recreates_them() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    let providers = [
        (".gwt/opencode", ".gwt/opencode/plugins/gwt-hooks.js"),
        (
            ".gwt/openclaw",
            ".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts",
        ),
        (".gwt/hermes", ".gwt/hermes/agent-hooks/gwt-hook.sh"),
    ];
    for (root, _) in providers {
        fs::create_dir_all(worktree.path().join(root)).expect("provider root");
    }

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_runtime_state_path(worktree.path().join("missing-runtime-state.json")),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    for (_, artifact) in providers {
        assert!(
            health
                .issues
                .iter()
                .any(|issue| issue.contains("managed hook config missing")
                    && issue.contains(artifact)),
            "missing {artifact} health evidence: {:?}",
            health.issues
        );
    }

    let outcome = repair_managed_hook_configs(worktree.path()).expect("repair");

    assert!(outcome.repaired);
    for (_, artifact) in providers {
        assert!(
            worktree.path().join(artifact).is_file(),
            "repair did not recreate {artifact}"
        );
    }
}

#[test]
fn managed_hook_health_keeps_config_issues_after_stop_event() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    fs::create_dir_all(worktree.path().join(".codex")).expect("codex dir");
    let runtime_path = worktree.path().join("runtime-state.json");
    let now = "2026-06-17T00:00:00Z".to_string();
    let runtime_state = RuntimeState {
        status: "Stopped".to_string(),
        updated_at: now.clone(),
        last_activity_at: now,
        source_event: "Stop".to_string(),
        pending_discussion: None,
    };
    fs::write(
        &runtime_path,
        serde_json::to_vec_pretty(&runtime_state).expect("serialize runtime state"),
    )
    .expect("write runtime state");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_runtime_state_path(&runtime_path),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    assert_eq!(health.last_event.as_deref(), Some("Stop"));
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains(".codex/hooks.json")),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_repair_preserves_user_hooks_and_top_level_settings() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let hooks_path = worktree.path().join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().expect("hooks parent")).unwrap();
    fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&json!({
            "customSetting": true,
            "hooks": {
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "echo user hook"
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let outcome = repair_managed_hook_configs(worktree.path()).expect("repair");

    assert!(outcome.repaired);
    let repaired: serde_json::Value =
        serde_json::from_slice(&fs::read(&hooks_path).expect("read hooks")).unwrap();
    assert_eq!(repaired["customSetting"], true);
    let rendered = serde_json::to_string(&repaired).unwrap();
    assert!(rendered.contains("echo user hook"), "{rendered}");
    assert!(rendered.contains("hook event Stop"), "{rendered}");
}

#[test]
fn managed_hook_health_reports_binary_skew() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let hooks_path = worktree.path().join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().expect("hooks parent")).unwrap();
    fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "/tmp/stale-gwtd hook event Stop"
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path()).with_expected_hook_bin("/tmp/current-gwtd"),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains("binary skew") && issue.contains("/tmp/stale-gwtd")),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_understands_runtime_indirect_fallbacks() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");

    for artifact in [".claude/settings.local.json", ".codex/hooks.json"] {
        let rendered = fs::read_to_string(worktree.path().join(artifact)).unwrap();
        assert!(rendered.contains("GWT_BIN_PATH"), "{artifact}: {rendered}");
        assert!(
            rendered.contains(&hook_bin.path().display().to_string()),
            "{artifact}: {rendered}"
        );
        assert!(
            !rendered.contains(&worktree.path().display().to_string()),
            "{artifact} persisted the tested worktree: {rendered}"
        );
    }

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_runtime_state_path(worktree.path().join("missing-runtime-state.json"))
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );

    assert_ne!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

#[cfg(unix)]
#[test]
fn managed_hook_health_reports_non_executable_absolute_runtime_fallback() {
    use std::os::unix::fs::PermissionsExt;

    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let fallback_root = tempfile::tempdir().expect("fallback root");
    let fallback = fallback_root.path().join("gwtd");
    fs::write(&fallback, "not executable").unwrap();
    let mut permissions = fs::metadata(&fallback).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&fallback, permissions).unwrap();
    let hooks_path = worktree.path().join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().expect("hooks parent")).unwrap();
    let hooks = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Stop",
    ]
    .into_iter()
    .map(|event| {
        (
            event.to_string(),
            json!([{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "gwt_bin=\"${{GWT_BIN_PATH:-{}}}\"; \"$gwt_bin\" hook event {event}",
                        fallback.display()
                    )
                }]
            }]),
        )
    })
    .collect::<serde_json::Map<_, _>>();
    fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&json!({ "hooks": hooks })).unwrap(),
    )
    .unwrap();

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_runtime_state_path(worktree.path().join("missing-runtime-state.json"))
            .with_expected_hook_bin(fallback.display().to_string()),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(
        health.issues.iter().any(|issue| {
            issue.contains("binary not executable")
                && issue.contains(&fallback.display().to_string())
        }),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_reports_missing_bare_runtime_fallback() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    {
        let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", "missing-gwtd-for-health-test");
        gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
        gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    }
    let _path = ScopedEnvVar::set("PATH", "");

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(
        health.issues.iter().any(|issue| {
            issue.contains("binary missing") && issue.contains("missing-gwtd-for-health-test")
        }),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_reports_missing_explicit_pin_without_rewriting_it() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let missing = worktree.path().join("missing/gwtd");
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &missing);
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains("binary missing")
                && issue.contains(&missing.display().to_string())),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_reports_worktree_local_fallback_once_per_artifact() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let local = worktree.path().join("target/debug/gwtd");
    fs::create_dir_all(local.parent().expect("local parent")).unwrap();
    fs::write(&local, "local").unwrap();
    {
        let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &local);
        gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
        gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    }

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    let local_issues = health
        .issues
        .iter()
        .filter(|issue| issue.contains("worktree-local binary"))
        .count();
    assert_eq!(
        local_issues, 2,
        "one issue per JSON artifact: {:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_audits_all_provider_bridge_artifacts() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let local = worktree.path().join("target/debug/gwtd");
    fs::create_dir_all(local.parent().expect("local parent")).unwrap();
    fs::write(&local, "local").unwrap();
    {
        let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &local);
        gwt_skills::generate_opencode_hooks(worktree.path()).expect("OpenCode hooks");
        gwt_skills::generate_openclaw_hooks(worktree.path()).expect("OpenClaw hooks");
        gwt_skills::generate_hermes_hooks(worktree.path()).expect("Hermes hooks");
    }

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::Degraded);
    for artifact in ["gwt-hooks.js", "plugin.ts", "gwt-hook.sh"] {
        assert!(
            health.issues.iter().any(|issue| issue.contains(artifact)),
            "missing {artifact} health evidence: {:?}",
            health.issues
        );
    }
}

#[test]
fn managed_hook_health_understands_all_provider_runtime_indirect_fallbacks() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_opencode_hooks(worktree.path()).expect("OpenCode hooks");
    gwt_skills::generate_openclaw_hooks(worktree.path()).expect("OpenClaw hooks");
    gwt_skills::generate_hermes_hooks(worktree.path()).expect("Hermes hooks");

    for artifact in [
        ".gwt/opencode/plugins/gwt-hooks.js",
        ".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts",
        ".gwt/hermes/agent-hooks/gwt-hook.sh",
    ] {
        let rendered = fs::read_to_string(worktree.path().join(artifact)).unwrap();
        assert!(rendered.contains("GWT_BIN_PATH"), "{artifact}: {rendered}");
        assert!(
            rendered.contains(&hook_bin.path().display().to_string()),
            "{artifact}: {rendered}"
        );
        assert!(
            !rendered.contains(&worktree.path().display().to_string()),
            "{artifact} persisted the tested worktree: {rendered}"
        );
    }

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_runtime_state_path(worktree.path().join("missing-runtime-state.json"))
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );

    assert_ne!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(health.issues.is_empty(), "{:?}", health.issues);
}

#[test]
fn managed_hook_health_reports_incomplete_codex_managed_entries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _hook_bin = ScopedEnvVar::remove("GWT_HOOK_BIN");
    let worktree = tempfile::tempdir().expect("worktree");
    let bin_dir = worktree.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let gwtd = bin_dir.join(if cfg!(windows) { "gwtd.exe" } else { "gwtd" });
    fs::write(&gwtd, "gwtd").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&gwtd).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gwtd, permissions).unwrap();
    }
    let _path = ScopedEnvVar::set("PATH", &bin_dir);
    let hooks_path = worktree.path().join(".codex/hooks.json");
    fs::create_dir_all(hooks_path.parent().expect("hooks parent")).unwrap();
    fs::write(
        &hooks_path,
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "gwtd hook event Stop"
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains("PreToolUse") && issue.contains("missing")),
        "{:?}",
        health.issues
    );
}

#[test]
fn managed_hook_health_projects_slow_profile_records_without_hook_stdout_noise() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard();
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    let runtime_path = worktree.path().join("runtime-state.json");
    runtime_state::write_for_event(&runtime_path, "PreToolUse").expect("runtime state");
    let profile_path = worktree.path().join(".gwt/hook-profile.jsonl");
    fs::create_dir_all(profile_path.parent().expect("profile parent")).unwrap();
    fs::write(
        &profile_path,
        [
            serde_json::to_string(&json!({
                "event": "PreToolUse",
                "handler": "runtime-state",
                "status": "ok",
                "duration_ms": 12.5,
                "occurred_at": "2026-06-17T00:00:00.000Z"
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "event": "PreToolUse",
                "handler": "workflow-policy",
                "status": "ok",
                "duration_ms": 1250.25,
                "occurred_at": "2026-06-17T00:00:01.000Z"
            }))
            .unwrap(),
        ]
        .join("\n"),
    )
    .unwrap();

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_runtime_state_path(&runtime_path)
            .with_profile_path(&profile_path),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    assert_eq!(health.slow_handlers.len(), 1);
    let slow = &health.slow_handlers[0];
    assert_eq!(slow.event, "PreToolUse");
    assert_eq!(slow.handler, "workflow-policy");
    assert_eq!(slow.status, "ok");
    assert_eq!(
        slow.occurred_at.as_deref(),
        Some("2026-06-17T00:00:01.000Z")
    );
    assert!(slow.duration_ms >= 1250.0, "{slow:?}");
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains("slow managed hook handler")),
        "{:?}",
        health.issues
    );
}

/// The shape `.codex/hooks.json` carried before the guarded template landed
/// (`b8fa26c04` / `2c660f11e`): a single-stage `${GWT_BIN_PATH:-<bin>}`
/// expansion that invokes the binary with no `command -v` guard, so an
/// unresolvable fallback hard-fails the hook instead of degrading to a no-op.
fn write_legacy_codex_hooks(path: &Path, fallback_bin: &str) {
    fs::create_dir_all(path.parent().expect("hooks parent")).expect("create hooks parent");
    let hooks = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "Stop",
    ]
    .into_iter()
    .map(|event| {
        (
            event.to_string(),
            json!([{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": format!(
                        "gwt_bin=\"${{GWT_BIN_PATH:-{fallback_bin}}}\"; \"$gwt_bin\" hook event {event}"
                    )
                }]
            }]),
        )
    })
    .collect::<serde_json::Map<_, _>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({ "hooks": hooks })).expect("serialize legacy hooks"),
    )
    .expect("write legacy hooks");
}

fn run_git(dir: &Path, args: &[&str]) {
    let output = gwt_core::process::hidden_command("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(dir: &Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "test@example.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
    run_git(dir, &["commit", "-q", "--allow-empty", "-m", "initial"]);
}

/// #3474 acceptance: the health auditor and the self-heal writer must cover the
/// SAME `.codex/hooks.json` path set. The writer used
/// `CodexHookDiscoveryMode::WorkspaceHome`, which resolves to the repo-root copy
/// for a linked worktree, while the auditor only ever read the worktree-local
/// copy — so a stale worktree-local file could never converge and every Work
/// card stayed red forever.
#[test]
fn codex_hook_audit_and_repair_cover_worktree_local_and_workspace_home_copies() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_git_repo(&repo);
    let worktree = root.path().join("wt-linked");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/linked",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let hook_bin = stable_hook_bin_guard();

    let paths = gwt::managed_assets::managed_codex_hook_paths(&worktree);
    assert_eq!(
        paths.len(),
        2,
        "a linked worktree owns two codex hook copies: {paths:?}"
    );
    assert_eq!(
        paths[0],
        worktree.join(".codex/hooks.json"),
        "the worktree-local copy must be managed: {paths:?}"
    );
    assert!(
        !paths[1].starts_with(&worktree) && paths[1].ends_with(".codex/hooks.json"),
        "the workspace-home copy lives outside the worktree: {paths:?}"
    );
    for path in &paths {
        write_legacy_codex_hooks(path, "gwtd");
    }
    assert!(
        repo.join(".codex/hooks.json").is_file(),
        "the workspace-home copy must resolve to the repo root"
    );

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(&worktree)
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );
    for path in &paths {
        assert!(
            health
                .issues
                .iter()
                .any(|issue| issue.contains(&path.display().to_string())),
            "auditor did not inspect {}: {:?}",
            path.display(),
            health.issues
        );
    }

    let outcome = repair_managed_hook_configs(&worktree).expect("repair");
    assert!(outcome.repaired);

    for path in &paths {
        let rendered = fs::read_to_string(path).expect("read repaired hooks");
        assert!(
            rendered.contains("command -v"),
            "{} kept the unguarded legacy template: {rendered}",
            path.display()
        );
        assert!(
            rendered.contains(&hook_bin.path().display().to_string()),
            "{} did not converge on the expected binary: {rendered}",
            path.display()
        );
    }

    let healed = read_managed_hook_health(
        &ManagedHookHealthInput::new(&worktree)
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );
    assert!(healed.issues.is_empty(), "{:?}", healed.issues);
}

/// #3474 root cause 3: a legacy hook config whose fallback happens to resolve
/// still hard-fails outside gwt, because the legacy command has no
/// `command -v` guard. The missing guard is its own issue class so the startup
/// self-heal loop breaker (which skips when EVERY issue is
/// `managed hook binary missing:`) can no longer strand a legacy file.
#[test]
fn managed_hook_health_flags_event_commands_without_a_runtime_guard() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let hook_bin = stable_hook_bin_guard();
    write_legacy_codex_hooks(
        &worktree.path().join(".codex/hooks.json"),
        &hook_bin.path().display().to_string(),
    );

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );

    assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
    assert!(
        health
            .issues
            .iter()
            .any(|issue| issue.contains("managed hook runtime guard missing")
                && issue.contains(".codex/hooks.json")),
        "{:?}",
        health.issues
    );
    assert!(
        !health
            .issues
            .iter()
            .all(|issue| issue.starts_with("managed hook binary missing:")),
        "the startup self-heal loop breaker must not swallow a legacy config: {:?}",
        health.issues
    );

    repair_managed_hook_configs(worktree.path()).expect("repair");

    let healed = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_expected_hook_bin(hook_bin.path().display().to_string()),
    );
    assert!(
        !healed
            .issues
            .iter()
            .any(|issue| issue.contains("runtime guard missing")),
        "a regenerated config must not report the guard issue again: {:?}",
        healed.issues
    );
}

/// #3474 root cause 4: `which` resolves against the *current process's* PATH.
/// A GUI process launched from Finder/Dock inherits launchd's PATH, which lacks
/// the installed app directory, so a bare `gwtd` fallback that resolves fine in
/// the agent's own environment was reported as `managed hook binary missing`
/// and every Work card went red.
#[test]
fn managed_hook_health_resolves_bare_fallback_without_the_process_path() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let installed = stable_hook_bin_guard();
    {
        let _bare = ScopedEnvVar::set("GWT_HOOK_BIN", "gwtd");
        gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");
    }
    let _bin_path = ScopedEnvVar::set(gwt_agent::session::GWT_BIN_PATH_ENV, installed.path());
    let _path = ScopedEnvVar::set("PATH", "");

    let health = read_managed_hook_health(&ManagedHookHealthInput::new(worktree.path()));

    assert!(
        !health
            .issues
            .iter()
            .any(|issue| issue.contains("binary missing")),
        "a bare fallback gwt itself can resolve is not missing: {:?}",
        health.issues
    );
}

/// #3474 root cause 1: `.codex/hooks.json` is version-controlled in THIS repo
/// (a repository decision gwt deliberately leaves to the project — see the
/// README "Hook file ownership" contract), so the committed copy is the one a
/// fresh clone and any gwt-external Codex run uses. It must stay on the guarded
/// template and must never pin a machine-local build output.
#[test]
fn committed_codex_hooks_stay_guarded_and_portable() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf();
    let hooks_path = repo_root.join(".codex/hooks.json");
    let rendered = fs::read_to_string(&hooks_path).expect("read committed codex hooks");
    let root: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");

    let mut managed = 0usize;
    for (event, groups) in root["hooks"].as_object().expect("hooks object") {
        for command in groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|group| group.get("hooks").and_then(|hooks| hooks.as_array()))
            .flatten()
            .filter_map(|hook| hook.get("command").and_then(|command| command.as_str()))
        {
            if !command.contains(&format!("hook event {event}")) {
                continue;
            }
            managed += 1;
            assert!(
                command.contains("command -v"),
                "{event} still uses the unguarded legacy template: {command}"
            );
        }
    }
    assert_eq!(managed, 5, "every managed event must be committed");
    assert!(
        !rendered.contains("/target/debug/") && !rendered.contains("\\\\target\\\\debug\\\\"),
        "a machine-local build output must never be committed: {rendered}"
    );
}

//! SPEC #1935 Phase 22 — managed hook health read model tests.

use std::fs;

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

fn env_test_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn stable_hook_bin_guard(worktree: &std::path::Path) -> ScopedEnvVar {
    let stable = worktree.join("stable/gwtd");
    fs::create_dir_all(stable.parent().expect("stable parent")).unwrap();
    fs::write(&stable, "stable").unwrap();
    ScopedEnvVar::set("GWT_HOOK_BIN", stable)
}

#[test]
fn managed_hook_health_is_ready_when_assets_and_runtime_state_are_current() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let worktree = tempfile::tempdir().expect("worktree");
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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
    let stable = worktree.path().join("stable/gwtd");
    fs::create_dir_all(stable.parent().expect("stable parent")).unwrap();
    fs::write(&stable, "stable").unwrap();
    let _hook_bin = ScopedEnvVar::set("GWT_HOOK_BIN", &stable);
    gwt_skills::generate_settings_local(worktree.path()).expect("claude hooks");
    gwt_skills::generate_codex_hooks(worktree.path()).expect("codex hooks");

    let health = read_managed_hook_health(
        &ManagedHookHealthInput::new(worktree.path())
            .with_expected_hook_bin(stable.display().to_string()),
    );

    assert_ne!(health.status, ManagedHookHealthStatus::Degraded);
    assert!(health.issues.is_empty(), "{:?}", health.issues);
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
fn managed_hook_health_reports_incomplete_codex_managed_entries() {
    let _env_lock = env_test_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _hook_bin = stable_hook_bin_guard(worktree.path());
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

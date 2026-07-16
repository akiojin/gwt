//! SPEC-3014 FR-004: integration tests for the agent launch pipeline
//! (detect → prepare → environment composition) up to — but not including —
//! the actual process spawn.
//!
//! These tests build launch environments through the crate's public API with
//! tempdir fixtures and assert on the composed env vars, PATH composition,
//! and the GWT_BIN_PATH parent-dir PATH prepend contract
//! (SPEC-2077 FR-020 / FR-021). No agent process is spawned.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gwt_agent::{
    install_launch_gwt_bin_env_with_lookup, prepare_agent_launch, AgentColor, AgentDetector,
    AgentId, DockerLifecycleIntent, HookForwardEnv, LaunchConfig, LaunchEnvironment,
    LaunchRuntimeTarget, SessionMode, GWT_BIN_PATH_ENV, GWT_HOOK_FORWARD_TOKEN_ENV,
    GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV, GWT_SESSION_RUNTIME_PATH_ENV,
};

/// A minimal on-disk git repo fixture: enough for repo-hash detection
/// (`.git/config` with an origin remote) without invoking the git binary.
fn git_repo_fixture(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join(".git")).expect("create .git");
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
    std::fs::write(
        repo.join(".git/config"),
        "[remote \"origin\"]\n    url = https://github.com/akiojin/gwt.git\n",
    )
    .expect("write config");
    repo
}

fn path_fixture(entries: &[&Path]) -> String {
    std::env::join_paths(entries)
        .expect("join fixture PATH")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn launch_environment_composes_project_root_hashes_and_terminal_defaults() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = git_repo_fixture(temp.path());

    let base_env = vec![
        ("PATH".to_string(), path_fixture(&[Path::new("/usr/bin")])),
        ("TERM".to_string(), "dumb".to_string()),
        ("NO_COLOR".to_string(), "1".to_string()),
        ("KEEP".to_string(), "base-value".to_string()),
    ];
    let launch_env = LaunchEnvironment::from_base_env(base_env).with_project_root(&repo);

    let mut env_vars = HashMap::from([("EXPLICIT".to_string(), "1".to_string())]);
    let mut remove_env = Vec::new();
    launch_env.apply_to_parts(&mut env_vars, &mut remove_env);

    // Launch-derived overrides.
    let normalized = gwt_core::paths::normalize_windows_child_process_path(&repo);
    assert_eq!(
        env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
        Some(normalized.display().to_string().as_str())
    );
    let expected_repo_hash =
        gwt_core::repo_hash::compute_repo_hash("https://github.com/akiojin/gwt.git");
    assert_eq!(
        env_vars.get("GWT_REPO_HASH").map(String::as_str),
        Some(expected_repo_hash.as_str())
    );
    let expected_worktree_hash = gwt_core::worktree_hash::compute_worktree_hash(&normalized)
        .expect("worktree hash for fixture repo");
    assert_eq!(
        env_vars.get("GWT_WORKTREE_HASH").map(String::as_str),
        Some(expected_worktree_hash.as_str())
    );

    // Terminal defaults + color suppressor removal.
    assert_eq!(
        env_vars.get("TERM").map(String::as_str),
        Some("xterm-256color")
    );
    assert_eq!(
        env_vars.get("COLORTERM").map(String::as_str),
        Some("truecolor")
    );
    assert!(!env_vars.contains_key("NO_COLOR"));
    assert!(remove_env.contains(&"NO_COLOR".to_string()));

    // Base and explicit env both survive the merge.
    assert_eq!(env_vars.get("KEEP").map(String::as_str), Some("base-value"));
    assert_eq!(env_vars.get("EXPLICIT").map(String::as_str), Some("1"));
}

#[test]
fn gwt_bin_path_injection_prepends_parent_dir_to_path_and_dedups() {
    // SPEC-2077 FR-020 / FR-021: the GWT_BIN_PATH parent directory must be
    // prepended to PATH so agent subshells resolve gwtd/gwt directly, and a
    // repeated injection must not duplicate the entry.
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bundle").join("MacOS");
    std::fs::create_dir_all(&bin_dir).expect("create bin dir");
    let gwtd = bin_dir.join("gwtd");
    std::fs::write(&gwtd, b"#!/bin/sh\n").expect("write gwtd fixture");
    let current_exe = bin_dir.join("gwt");

    let mut env_vars = HashMap::from([(
        "PATH".to_string(),
        path_fixture(&[Path::new("/usr/bin"), Path::new("/bin")]),
    )]);

    install_launch_gwt_bin_env_with_lookup(
        &mut env_vars,
        LaunchRuntimeTarget::Host,
        &current_exe,
        |_command| Some(gwtd.clone()),
    )
    .expect("install gwt bin env");

    assert_eq!(
        env_vars.get(GWT_BIN_PATH_ENV).map(String::as_str),
        Some(gwtd.to_string_lossy().as_ref()),
        "GWT_BIN_PATH must point at the resolved gwtd"
    );
    let path = env_vars.get("PATH").expect("PATH after injection").clone();
    let entries: Vec<PathBuf> = std::env::split_paths(&path).collect();
    assert_eq!(
        entries.first(),
        Some(&bin_dir),
        "GWT_BIN_PATH parent dir must be the first PATH entry; got {path}"
    );
    assert!(entries.contains(&PathBuf::from("/usr/bin")));
    assert!(entries.contains(&PathBuf::from("/bin")));

    // Second injection (e.g. re-prepare on relaunch) must be idempotent.
    install_launch_gwt_bin_env_with_lookup(
        &mut env_vars,
        LaunchRuntimeTarget::Host,
        &current_exe,
        |_command| Some(gwtd.clone()),
    )
    .expect("re-install gwt bin env");

    let entries_after: Vec<PathBuf> =
        std::env::split_paths(env_vars.get("PATH").expect("PATH")).collect();
    assert_eq!(
        entries_after, entries,
        "re-injection must not duplicate PATH entries"
    );
    assert_eq!(
        entries_after
            .iter()
            .filter(|entry| **entry == bin_dir)
            .count(),
        1,
        "exactly one PATH entry for the gwtd parent dir"
    );
}

#[test]
fn docker_runtime_pins_gwt_bin_path_to_container_gwtd() {
    // Docker launches must always use the container-side gwtd path and
    // prepend its directory to a POSIX PATH, regardless of host state.
    let mut env_vars = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);

    install_launch_gwt_bin_env_with_lookup(
        &mut env_vars,
        LaunchRuntimeTarget::Docker,
        Path::new("/host/path/is/ignored/in/docker"),
        |_command| None,
    )
    .expect("install docker gwt bin env");

    assert_eq!(
        env_vars.get(GWT_BIN_PATH_ENV).map(String::as_str),
        Some("/usr/local/bin/gwtd")
    );
    let entries: Vec<&str> = env_vars.get("PATH").expect("PATH").split(':').collect();
    assert_eq!(
        entries,
        vec!["/usr/local/bin", "/usr/bin", "/bin"],
        "container gwtd dir must be prepended to the POSIX PATH"
    );
}

#[test]
fn detect_by_command_returns_none_for_missing_agent() {
    assert!(
        AgentDetector::detect_by_command("gwt-launch-pipeline-missing-agent-3014").is_none(),
        "a command that is not on PATH must not be detected"
    );
}

#[test]
fn prepare_agent_launch_composes_session_env_without_spawning() {
    // Full prepare pipeline: worktree resolution (pre-resolved working_dir),
    // environment composition, GWT_BIN_PATH injection, session persistence.
    // The agent process itself is never spawned.
    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create worktree");
    let sessions_dir = temp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

    let config = LaunchConfig {
        agent_id: AgentId::Custom("integration-fake-agent".to_string()),
        command: "integration-fake-agent".to_string(),
        args: vec!["--flag".to_string()],
        env_vars: HashMap::from([("EXPLICIT_LAUNCH".to_string(), "yes".to_string())]),
        remove_env: Vec::new(),
        working_dir: Some(worktree.clone()),
        branch: None,
        base_branch: None,
        is_ephemeral: false,
        ephemeral_base_ref: None,
        display_name: "Integration Fake Agent".to_string(),
        color: AgentColor::Green,
        model: None,
        tool_version: None,
        reasoning_level: None,
        session_mode: SessionMode::Normal,
        resume_session_id: None,
        initial_prompt: None,
        recovery_continuation: None,
        recovery_retry_session_id: None,
        recovery_retry_created_at: None,
        skip_permissions: false,
        fast_mode: false,
        codex_fast_mode: false,
        runtime_target: LaunchRuntimeTarget::Host,
        docker_service: None,
        docker_lifecycle_intent: DockerLifecycleIntent::Connect,
        linked_issue_number: None,
        windows_shell: None,
        suppress_execution_control: false,
    };

    let mut refreshed_paths = Vec::new();
    let prepared = prepare_agent_launch(
        &worktree,
        &sessions_dir,
        config,
        Some(HookForwardEnv {
            url: "http://127.0.0.1:7777/hooks".to_string(),
            token: "hook-token".to_string(),
        }),
        |path: &Path| {
            refreshed_paths.push(path.to_path_buf());
            Ok(())
        },
    )
    .expect("prepare agent launch");

    // Worktree assets were refreshed for the resolved worktree, not spawned.
    let normalized_worktree = gwt_core::paths::normalize_windows_child_process_path(&worktree);
    assert_eq!(refreshed_paths, vec![normalized_worktree.clone()]);
    assert_eq!(prepared.worktree_path, normalized_worktree);

    let env = &prepared.process_launch.env;

    // Session wiring.
    assert_eq!(
        env.get(GWT_SESSION_ID_ENV).map(String::as_str),
        Some(prepared.session.id.as_str())
    );
    assert_eq!(
        env.get(GWT_SESSION_RUNTIME_PATH_ENV).map(String::as_str),
        Some(prepared.runtime_path.display().to_string().as_str())
    );
    assert!(
        prepared.runtime_path.exists(),
        "runtime state must be persisted before spawn"
    );

    // Hook forwarding env.
    assert_eq!(
        env.get(GWT_HOOK_FORWARD_URL_ENV).map(String::as_str),
        Some("http://127.0.0.1:7777/hooks")
    );
    assert_eq!(
        env.get(GWT_HOOK_FORWARD_TOKEN_ENV).map(String::as_str),
        Some("hook-token")
    );

    // Launch-derived overrides and explicit env survive composition.
    assert_eq!(
        env.get("GWT_PROJECT_ROOT").map(String::as_str),
        Some(normalized_worktree.display().to_string().as_str())
    );
    assert_eq!(env.get("EXPLICIT_LAUNCH").map(String::as_str), Some("yes"));
    assert!(env.contains_key("COLORTERM"));

    // SPEC-2077 FR-020 / FR-021 at pipeline level: GWT_BIN_PATH is injected
    // and its parent directory participates in the composed PATH.
    let gwt_bin = env
        .get(GWT_BIN_PATH_ENV)
        .expect("GWT_BIN_PATH must be injected");
    let parent = Path::new(gwt_bin)
        .parent()
        .expect("GWT_BIN_PATH must have a parent dir");
    if !parent.as_os_str().is_empty() {
        let path = env.get("PATH").expect("PATH must be composed");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert!(
            entries.iter().any(|entry| entry == parent),
            "PATH must contain the GWT_BIN_PATH parent dir {parent:?}; got {path}"
        );
    }

    // The launch command is untouched (no spawn, no runner fallback).
    assert_eq!(prepared.process_launch.command, "integration-fake-agent");
    assert_eq!(prepared.process_launch.args, vec!["--flag".to_string()]);
    assert_eq!(
        prepared.process_launch.cwd.as_deref(),
        Some(normalized_worktree.as_path())
    );
}

use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

#[cfg(unix)]
use std::{path::PathBuf, process::Output};

use gwt::{
    refresh_existing_managed_gwt_assets_for_worktree, refresh_managed_gwt_assets_for_agent,
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode,
    refresh_managed_gwt_assets_for_worktree,
};
use gwt_agent::AgentId;
use gwt_core::process::hidden_command;
use gwt_skills::CodexHookDiscoveryMode;
use serde_json::Value;
use tempfile::tempdir;

/// SPEC #3245 FR-004 / AC-1: the coordination guidance no longer branches by
/// session kind. Every materialization gets the single guidance including the
/// `workspace.update` Work-state instruction; the curation framing that told
/// intake sessions they "produce no Work" is gone (#3379 contradiction).
#[test]
fn coordination_guidance_is_identical_for_all_session_kinds() {
    fn materialize_and_read(is_ephemeral: bool) -> String {
        let dir = tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        let _env_guard = env_lock();
        let cli_bin = dir.path().join("bin/gwtd");
        std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
        std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
        let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

        refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
            dir.path(),
            &AgentId::ClaudeCode,
            CodexHookDiscoveryMode::WorkspaceHome,
            is_ephemeral,
        )
        .expect("materialize managed assets");

        std::fs::read_to_string(dir.path().join(".claude/skills/gwt-coordination/SKILL.md"))
            .expect("read coordination SKILL.md")
    }

    let intake = materialize_and_read(true);
    let execution = materialize_and_read(false);
    assert_eq!(
        intake, execution,
        "guidance must be identical for every session kind (single guidance, FR-004)"
    );
    assert!(
        intake.contains(r#""operation":"workspace.update""#),
        "the single guidance must keep the workspace.update Work-state instruction"
    );
    assert!(
        !intake.contains("intake sessions produce no Work"),
        "the curation framing must be gone from the single guidance"
    );
}

/// SPEC #3245 FR-003 / AC-1: every session kind receives the full skill set.
/// The reduced (curation) skill set is removed — implementation skills stay
/// available in intake-kind materializations too.
#[test]
fn intake_materialize_keeps_full_skill_set() {
    fn materialize(is_ephemeral: bool) -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        let _env_guard = env_lock();
        let cli_bin = dir.path().join("bin/gwtd");
        std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
        std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
        let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);
        refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
            dir.path(),
            &AgentId::ClaudeCode,
            CodexHookDiscoveryMode::WorkspaceHome,
            is_ephemeral,
        )
        .expect("materialize managed assets");
        dir
    }

    let intake = materialize(true);
    assert!(
        intake
            .path()
            .join(".claude/skills/gwt-build-spec/SKILL.md")
            .exists(),
        "intake must keep the implementation skill gwt-build-spec (full set)"
    );
    assert!(
        intake
            .path()
            .join(".claude/skills/gwt-register-issue/SKILL.md")
            .exists(),
        "intake must keep registration skills"
    );
    assert!(
        intake
            .path()
            .join(".claude/skills/gwt-register-spec/SKILL.md")
            .exists(),
        "intake must keep the register-spec alias too (FR-005)"
    );

    let execution = materialize(false);
    assert!(
        execution
            .path()
            .join(".claude/skills/gwt-build-spec/SKILL.md")
            .exists(),
        "execution must keep the full skill set"
    );
}

/// SPEC #3245 FR-003: an intake lane file no longer changes the distributed
/// skill set — envless re-materialization (e.g. the GUI front door) writes
/// the full set exactly like every other worktree.
#[test]
fn envless_rematerialize_keeps_full_skill_set_for_intake_lane_file() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

    refresh_managed_gwt_assets_for_agent(dir.path(), &AgentId::ClaudeCode)
        .expect("materialize managed assets");

    assert!(
        dir.path()
            .join(".claude/skills/gwt-build-spec/SKILL.md")
            .exists(),
        "an intake lane file must not reduce the distributed skill set"
    );
    assert!(
        dir.path()
            .join(".claude/skills/gwt-register-issue/SKILL.md")
            .exists(),
        "registration skills stay distributed everywhere"
    );
}

/// #3374: an ephemeral intake worktree must surface the embedded (binary)
/// skill bundle even where the project tracks gwt skills — the gwt repo
/// itself tracks `.claude/skills/**`, so a stale-base worktree would
/// otherwise pin months-old guidance that managed-asset distribution
/// refuses to heal. Execution worktrees keep the tracked copies (SPEC #1942).
#[test]
fn intake_materialize_overrides_stale_tracked_gwt_skills() {
    fn materialize_with_stale_tracked(is_ephemeral: bool) -> tempfile::TempDir {
        let dir = tempdir().expect("tempdir");
        run_git(dir.path(), &["init", "-q"]);
        // A stale tracked copy of a curation skill (survives the reduced set).
        let skill = dir
            .path()
            .join(".claude/skills/gwt-register-issue/SKILL.md");
        std::fs::create_dir_all(skill.parent().expect("skill dir")).expect("create skill dir");
        std::fs::write(&skill, "stale tracked skill").expect("write stale skill");
        run_git(
            dir.path(),
            &["add", ".claude/skills/gwt-register-issue/SKILL.md"],
        );

        let _env_guard = env_lock();
        let cli_bin = dir.path().join("bin/gwtd");
        std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
        std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
        let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);
        refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
            dir.path(),
            &AgentId::ClaudeCode,
            CodexHookDiscoveryMode::WorkspaceHome,
            is_ephemeral,
        )
        .expect("materialize managed assets");
        dir
    }

    let intake = materialize_with_stale_tracked(true);
    let refreshed = std::fs::read_to_string(
        intake
            .path()
            .join(".claude/skills/gwt-register-issue/SKILL.md"),
    )
    .expect("read refreshed skill");
    assert_ne!(
        refreshed, "stale tracked skill",
        "intake must refresh a stale tracked gwt skill from the embedded bundle"
    );

    let execution = materialize_with_stale_tracked(false);
    let preserved = std::fs::read_to_string(
        execution
            .path()
            .join(".claude/skills/gwt-register-issue/SKILL.md"),
    )
    .expect("read preserved skill");
    assert_eq!(
        preserved, "stale tracked skill",
        "execution must keep the tracked copy (SPEC #1942 preserve-tracked)"
    );
}

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
    // SPEC-1935 US-* (Coordination Guidance Generator): the generated
    // coordination skill must appear under both Claude and Codex skill
    // roots after a full materialize.
    let claude_coordination = dir.path().join(".claude/skills/gwt-coordination/SKILL.md");
    let codex_coordination = dir.path().join(".codex/skills/gwt-coordination/SKILL.md");
    assert!(
        claude_coordination.exists(),
        "Claude gwt-coordination SKILL.md not generated at {claude_coordination:?}"
    );
    assert!(
        codex_coordination.exists(),
        "Codex gwt-coordination SKILL.md not generated at {codex_coordination:?}"
    );
    let coordination_body =
        std::fs::read_to_string(&claude_coordination).expect("read coordination skill");
    assert!(
        coordination_body.contains("\"operation\":\"workspace.update\""),
        "coordination skill must contain canonical workspace.update JSON operation"
    );
    assert!(
        coordination_body.contains("regardless of project AGENTS.md / CLAUDE.md content"),
        "coordination skill description must declare project-AGENTS.md-independence"
    );
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
    assert!(
        exclude.contains("\n.gwt/*\n"),
        ".gwt/drop-files/ must stay covered by the broad project-local .gwt/* exclude"
    );
    assert!(
        exclude.contains("\n!.gwt/work/\n"),
        "the tracked .gwt/work/ directory must be carved out of the broad exclude"
    );
    assert!(exclude.contains(".claude/skills/gwt-*"));
    assert!(exclude.contains(".claude/commands/gwt-*"));
    assert!(exclude.contains(".codex/skills/gwt-*"));
}

#[cfg(unix)]
#[test]
fn browser_check_hook_audit_accepts_all_provider_surfaces_and_preserves_user_hooks() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_lock();
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let hermes_home = tempdir().expect("hermes home tempdir");
    let _hermes_home_guard = ScopedEnvVar::set("HERMES_HOME", hermes_home.path());

    let stable_hook_bin = dir.path().join("installed/gwtd'${stable}");
    std::fs::create_dir_all(stable_hook_bin.parent().expect("stable bin parent"))
        .expect("create stable bin parent");
    std::fs::write(&stable_hook_bin, "#!/bin/sh\nexit 0\n").expect("write stable hook bin");
    std::fs::set_permissions(&stable_hook_bin, std::fs::Permissions::from_mode(0o755))
        .expect("make stable hook bin executable");
    let _hook_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &stable_hook_bin);

    let claude_settings = dir.path().join(".claude/settings.local.json");
    std::fs::create_dir_all(claude_settings.parent().expect("Claude settings parent"))
        .expect("create Claude settings parent");
    std::fs::write(
        &claude_settings,
        serde_json::to_vec_pretty(&serde_json::json!({
            "customSetting": true,
            "hooks": {
                "Stop": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "echo keep-user-hook"
                    }]
                }, {
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "/old/worktree/target/debug/gwtd hook event Stop"
                    }]
                }]
            }
        }))
        .expect("serialize Claude settings"),
    )
    .expect("seed Claude settings");

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("refresh managed assets");

    let hook_artifacts = [
        ".claude/settings.local.json",
        ".codex/hooks.json",
        ".gwt/opencode/plugins/gwt-hooks.js",
        ".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts",
        ".gwt/hermes/agent-hooks/gwt-hook.sh",
    ];
    for artifact in hook_artifacts {
        assert!(
            dir.path().join(artifact).is_file(),
            "missing managed hook surface: {artifact}"
        );
    }

    let before = hook_artifacts
        .iter()
        .map(|artifact| std::fs::read(dir.path().join(artifact)).expect("read hook artifact"))
        .collect::<Vec<_>>();
    let output = run_browser_check_hook_audit(dir.path(), &stable_hook_bin, None);
    assert!(
        output.status.success(),
        "browser-check audit failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let after = hook_artifacts
        .iter()
        .map(|artifact| std::fs::read(dir.path().join(artifact)).expect("read hook artifact"))
        .collect::<Vec<_>>();
    assert_eq!(before, after, "hook.health audit must be read-only");

    let rendered_claude =
        std::fs::read_to_string(&claude_settings).expect("read refreshed Claude settings");
    assert!(rendered_claude.contains("echo keep-user-hook"));
    assert!(!rendered_claude.contains("/old/worktree/target/debug/gwtd"));
}

#[cfg(unix)]
#[test]
fn browser_check_hook_audit_blocks_exact_fallback_mismatch() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = env_lock();
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let hermes_home = tempdir().expect("hermes home tempdir");
    let _hermes_home_guard = ScopedEnvVar::set("HERMES_HOME", hermes_home.path());
    let _hook_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", "gwtd");

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("refresh managed assets");

    let opencode_hook = dir.path().join(".gwt/opencode/plugins/gwt-hooks.js");
    let rendered = std::fs::read_to_string(&opencode_hook).expect("read OpenCode hook");
    let mismatched = rendered.replacen(
        "process.env.GWT_BIN_PATH || \"gwtd\"",
        "process.env.GWT_BIN_PATH || \"/wrong/stable/gwtd\"",
        1,
    );
    assert_ne!(
        rendered, mismatched,
        "OpenCode fallback fixture must change"
    );
    std::fs::write(&opencode_hook, mismatched).expect("write mismatched OpenCode hook");

    let path_dir = tempdir().expect("PATH tempdir");
    let path_gwtd = path_dir.path().join("gwtd");
    std::fs::write(&path_gwtd, "#!/bin/sh\nexit 0\n").expect("write PATH gwtd");
    std::fs::set_permissions(&path_gwtd, std::fs::Permissions::from_mode(0o755))
        .expect("make PATH gwtd executable");
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let audit_path = std::env::join_paths(
        std::iter::once(path_dir.path().to_path_buf()).chain(std::env::split_paths(&current_path)),
    )
    .expect("compose audit PATH");

    let output = run_browser_check_hook_audit(dir.path(), Path::new("gwtd"), Some(&audit_path));
    assert!(
        !output.status.success(),
        "browser-check audit unexpectedly accepted mismatched fallback"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("managed hook surfaces did not converge"),
        "{stderr}"
    );
    assert!(
        stderr.contains("managed hook binary skew")
            && stderr.contains("/wrong/stable/gwtd")
            && stderr.contains("expected gwtd"),
        "audit must report the exact fallback mismatch even though the file contains other gwtd tokens:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn browser_check_hook_audit_allows_missing_logical_fallback() {
    let _env_guard = env_lock();
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let hermes_home = tempdir().expect("hermes home tempdir");
    let _hermes_home_guard = ScopedEnvVar::set("HERMES_HOME", hermes_home.path());
    let _hook_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", "gwtd");

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("refresh managed assets");

    let tools = tempdir().expect("isolated tool PATH");
    for name in ["bash", "env", "jq", "rg"] {
        let source = which::which(name).unwrap_or_else(|error| panic!("resolve {name}: {error}"));
        std::os::unix::fs::symlink(&source, tools.path().join(name))
            .unwrap_or_else(|error| panic!("link {name} from {}: {error}", source.display()));
    }
    assert!(
        which::which_in("gwtd", Some(tools.path().as_os_str()), dir.path()).is_err(),
        "isolated audit PATH must not contain gwtd"
    );

    let output = run_browser_check_hook_audit(
        dir.path(),
        Path::new("gwtd"),
        Some(tools.path().as_os_str()),
    );
    assert!(
        output.status.success(),
        "a missing logical fallback is fail-open\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn refresh_managed_assets_for_codex_only_materializes_codex_assets() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

    refresh_managed_gwt_assets_for_agent(dir.path(), &AgentId::Codex)
        .expect("refresh Codex assets");

    assert!(dir
        .path()
        .join(".codex/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(dir.path().join(".codex/hooks.json").exists());
    assert!(!dir
        .path()
        .join(".claude/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(!dir.path().join(".claude/settings.local.json").exists());
    assert!(!dir.path().join(".gwt/hermes/config.yaml").exists());
    // SPEC-1935 US-*: when only Codex is the target, the coordination
    // skill must be written under .codex/skills only and NOT under
    // .claude/skills.
    assert!(
        dir.path()
            .join(".codex/skills/gwt-coordination/SKILL.md")
            .exists(),
        "Codex coordination skill must materialize for Codex-only target"
    );
    assert!(
        !dir.path()
            .join(".claude/skills/gwt-coordination/SKILL.md")
            .exists(),
        "Claude coordination skill must NOT appear when only Codex is targeted"
    );

    let exclude =
        std::fs::read_to_string(dir.path().join(".git/info/exclude")).expect("read exclude");
    assert!(exclude.contains(".codex/skills/gwt-*"));
    assert!(!exclude.contains(".claude/skills/gwt-*"));
    assert!(!exclude.contains(".gwt/hermes/"));
}

#[test]
fn refresh_managed_assets_for_hermes_materializes_hermes_home_skills_only() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);
    // Pin HERMES_HOME to an isolated empty dir so the credential bridge never
    // reads the developer's real ~/.hermes during this test.
    let hermes_home = tempdir().expect("hermes home tempdir");
    let _hermes_home_guard = ScopedEnvVar::set("HERMES_HOME", hermes_home.path());

    refresh_managed_gwt_assets_for_agent(dir.path(), &AgentId::Hermes)
        .expect("refresh Hermes assets");

    assert!(dir.path().join(".gwt/hermes/config.yaml").exists());
    assert!(dir
        .path()
        .join(".gwt/hermes/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(!dir
        .path()
        .join(".claude/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(!dir
        .path()
        .join(".codex/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(!dir.path().join(".codex/hooks.json").exists());

    let exclude =
        std::fs::read_to_string(dir.path().join(".git/info/exclude")).expect("read exclude");
    // .gwt/hermes/ is subsumed by the broad project-local .gwt/* exclude
    // emitted for any managed target, while .gwt/work/ stays carved out.
    assert!(exclude.contains("\n.gwt/*\n"));
    assert!(exclude.contains("\n!.gwt/work/\n"));
    assert!(!exclude.contains(".gwt/hermes/"));
    assert!(!exclude.contains(".claude/skills/gwt-*"));
    assert!(!exclude.contains(".codex/skills/gwt-*"));
}

#[test]
fn refresh_existing_managed_assets_refreshes_only_present_provider_surfaces() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);
    std::fs::create_dir_all(dir.path().join(".codex/skills")).expect("create codex marker");

    refresh_existing_managed_gwt_assets_for_worktree(dir.path())
        .expect("refresh existing managed assets");

    assert!(dir
        .path()
        .join(".codex/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(dir.path().join(".codex/hooks.json").exists());
    assert!(!dir
        .path()
        .join(".claude/skills/gwt-build-spec/SKILL.md")
        .exists());
    assert!(!dir.path().join(".gwt/hermes/config.yaml").exists());

    let exclude =
        std::fs::read_to_string(dir.path().join(".git/info/exclude")).expect("read exclude");
    assert!(exclude.contains(".codex/skills/gwt-*"));
    assert!(!exclude.contains(".claude/skills/gwt-*"));
    assert!(!exclude.contains(".gwt/hermes/"));
}

#[test]
fn refresh_managed_gwt_assets_reports_the_failed_step() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("not-a-worktree");
    std::fs::write(&file_path, "plain file").expect("write file");

    let error = refresh_managed_gwt_assets_for_worktree(&file_path)
        .expect_err("refresh should fail for a file path");

    // A non-directory worktree (here a file, in practice a worktree whose
    // branch/worktree creation failed) is now caught up front with a clear,
    // attributed error instead of a misleading downstream skill/distribute error.
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error
        .to_string()
        .contains("worktree is not a ready directory"));
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

/// Materialize managed assets into a canonical PM worktree under an isolated
/// gwt home, returning the worktree so the caller can inspect the result of a
/// full distribution — the state a PM agent actually starts in.
fn materialize_into_pm_worktree(
    home: &Path,
    agent_id: &AgentId,
    seed: impl FnOnce(&Path),
) -> std::path::PathBuf {
    let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home);
    let repo = home.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    run_git(&repo, &["init", "-q"]);

    let worktree = gwt::pm_registry::pm_worktree_path_for_repo_path(&repo);
    std::fs::create_dir_all(&worktree).expect("create pm worktree");
    run_git(&worktree, &["init", "-q"]);

    seed(&worktree);

    let _env_guard = env_lock();
    let cli_bin = home.join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        &worktree,
        agent_id,
        CodexHookDiscoveryMode::WorkspaceHome,
        false,
    )
    .expect("materialize managed assets");
    worktree
}

/// SPEC-3431 FR-001 / T-052 regression: the `$gwt-pm` bootstrap prompt only
/// resolves if the gwt-pm skill survives the launch's own asset refresh.
///
/// The original implementation wrote the skill just before spawning, and the
/// launch thread's `prune_managed_asset_roots_for_targets` then deleted it
/// (it removes every `gwt-*` skill dir absent from the embedded bundle, and
/// gwt-pm is generated rather than bundled). The PM booted as a plain agent
/// with a dangling prompt. Asserting the post-distribution state is the whole
/// point — the old synchronous-existence assertion passed while this was live.
#[test]
fn pm_worktree_keeps_gwt_pm_guidance_after_asset_distribution() {
    let home = tempdir().expect("tempdir");
    let worktree = materialize_into_pm_worktree(home.path(), &AgentId::ClaudeCode, |worktree| {
        gwt_skills::pm_guidance::generate_pm_guidance(worktree).expect("pre-write guidance");
    });

    let skill = std::fs::read_to_string(worktree.join(".claude/skills/gwt-pm/SKILL.md"))
        .expect("gwt-pm skill must survive distribution");
    assert_eq!(skill, gwt_skills::pm_guidance::render_skill_md());
}

/// The resume path never pre-writes the skill, and a binary upgrade must not
/// leave a PM running an obsolete contract. Both cases are the same
/// requirement: distribution owns the file's content, unconditionally.
#[test]
fn pm_worktree_gwt_pm_guidance_is_regenerated_when_absent_or_tampered() {
    let home = tempdir().expect("tempdir");
    let worktree = materialize_into_pm_worktree(home.path(), &AgentId::ClaudeCode, |_| {});
    let path = worktree.join(".claude/skills/gwt-pm/SKILL.md");
    assert_eq!(
        std::fs::read_to_string(&path).expect("guidance generated without a pre-write"),
        gwt_skills::pm_guidance::render_skill_md()
    );

    std::fs::write(&path, "stale contract").expect("tamper");
    let _home_guard = gwt_core::test_support::ScopedGwtHome::set(home.path());
    let _env_guard = env_lock();
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", home.path().join("bin/gwtd"));
    refresh_managed_gwt_assets_for_agent_with_codex_hook_discovery_mode(
        &worktree,
        &AgentId::ClaudeCode,
        CodexHookDiscoveryMode::WorkspaceHome,
        false,
    )
    .expect("re-materialize");
    assert_eq!(
        std::fs::read_to_string(&path).expect("guidance restored"),
        gwt_skills::pm_guidance::render_skill_md(),
        "a tampered or stale contract must be rewritten from the canonical source"
    );
}

/// Per-target isolation holds for gwt-pm exactly as it does for
/// gwt-coordination: a Codex PM gets the Codex mirror only.
#[test]
fn pm_worktree_codex_only_target_writes_only_the_codex_mirror() {
    let home = tempdir().expect("tempdir");
    let worktree = materialize_into_pm_worktree(home.path(), &AgentId::Codex, |_| {});
    assert!(worktree.join(".codex/skills/gwt-pm/SKILL.md").exists());
    assert!(
        !worktree.join(".claude/skills/gwt-pm/SKILL.md").exists(),
        "a Codex-only target must not write the Claude mirror"
    );
}

/// The PM contract must never reach an implementation agent. Its description
/// shares gwt-coordination's "use proactively at the start of every
/// conversation" stem, so an agent that picked it up would adopt "you never
/// implement production code yourself" and silently refuse to implement.
#[test]
fn non_pm_worktree_never_receives_gwt_pm_guidance() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwtd");
    std::fs::create_dir_all(cli_bin.parent().expect("bin parent")).expect("create bin dir");
    std::fs::write(&cli_bin, "#!/bin/sh\n").expect("write cli bin");
    let _cli_bin_guard = ScopedEnvVar::set("GWT_HOOK_BIN", &cli_bin);

    refresh_managed_gwt_assets_for_worktree(dir.path()).expect("materialize managed assets");

    assert!(!dir.path().join(".claude/skills/gwt-pm").exists());
    assert!(!dir.path().join(".codex/skills/gwt-pm").exists());
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = hidden_command("git")
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

#[cfg(unix)]
fn browser_check_shell_block(name: &str) -> String {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let skill_path = workspace_root.join(".claude/skills/browser-check/SKILL.md");
    let skill = std::fs::read_to_string(&skill_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", skill_path.display()));
    let begin = format!("# browser-check-{name}-begin");
    let end = format!("# browser-check-{name}-end");
    let body = skill
        .split_once(&begin)
        .unwrap_or_else(|| panic!("missing executable browser-check marker {begin}"))
        .1
        .split_once(&end)
        .unwrap_or_else(|| panic!("missing executable browser-check marker {end}"))
        .0;
    body.lines()
        .map(|line| line.strip_prefix("     ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(unix)]
fn run_browser_check_hook_audit(
    worktree: &Path,
    expected_hook_bin: &Path,
    path: Option<&std::ffi::OsStr>,
) -> Output {
    let script = format!(
        "{}\n{}",
        browser_check_shell_block("hook-authority"),
        browser_check_shell_block("hook-audit")
    );
    let mut command = hidden_command("bash");
    command
        .args(["-c", &script])
        .current_dir(worktree)
        .env("REPO_ROOT", worktree)
        .env("CHECK_HOME", worktree)
        .env("CHECKOUT_GWTD", env!("CARGO_BIN_EXE_gwtd"))
        .env("GWT_HOOK_BIN", expected_hook_bin)
        .env("GWT_BIN_PATH", "/ambient/stale/target/debug/gwtd")
        .env_remove(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .env_remove("GWT_HOOK_FORWARD_TOKEN")
        .env_remove("GWT_HOOK_FORWARD_URL")
        .env_remove("GWT_PROJECT_ROOT")
        .env_remove("GWT_SESSION_ID");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command.output().expect("run browser-check hook audit")
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

use gwt_core::test_support::ScopedEnvVar;

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

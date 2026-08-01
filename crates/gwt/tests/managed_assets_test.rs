use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

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

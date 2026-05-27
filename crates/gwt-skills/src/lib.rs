//! gwt-skills: Embedded skill bundling, distribution, and hooks management for gwt.

pub mod assets;
pub mod codex_home;
pub mod codex_hook_trust;
pub mod coordination_guidance;
pub mod distribute;
pub mod git_exclude;
pub mod hooks;
pub mod provider_hooks;
pub mod registry;
pub mod settings_local;
pub mod validate;

pub use codex_home::{
    codex_env_key, codex_home_for_worktree, codex_provider_id,
    materialize as materialize_codex_home, render_config_toml as render_codex_config_toml,
    CodexHomeConfig, DEFAULT_WIRE_API, RESERVED_PROVIDER_IDS,
};
pub use codex_hook_trust::{
    collect_codex_managed_hook_trust_entries, collect_codex_managed_hook_trust_entries_for_mode,
    register_codex_managed_hook_trust, register_codex_managed_hook_trust_for_mode,
    CodexHookTrustEntry, CodexHookTrustReport,
};
pub use coordination_guidance::{
    generate_coordination_guidance, generate_coordination_guidance_for_claude,
    generate_coordination_guidance_for_codex,
};
pub use distribute::{
    distribute_to_worktree, distribute_to_worktree_for_targets, prune_stale_gwt_assets,
    prune_stale_gwt_assets_for_targets, DistributeReport, ManagedAssetTarget,
};
pub use git_exclude::{update_git_exclude, update_git_exclude_for_targets};
pub use hooks::{
    backup_hooks, detect_corruption, is_gwt_managed, merge_hooks, merge_hooks_safe,
    restore_from_backup, Hook, HooksConfig, HooksError,
};
pub use provider_hooks::{generate_hermes_hooks, generate_openclaw_hooks, generate_opencode_hooks};
pub use registry::{EmbeddedSkill, RegistryError, SkillRegistry};
pub use settings_local::{
    generate_codex_hooks, generate_codex_hooks_for_mode, generate_settings_local,
    CodexHookDiscoveryMode,
};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fs2::FileExt;

    use super::*;

    // ── SkillRegistry tests ──

    #[test]
    fn register_and_list_skills() {
        let mut reg = SkillRegistry::new();
        assert!(reg.list().is_empty());

        reg.register(make_skill("alpha"));
        reg.register(make_skill("beta"));
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn register_replaces_duplicate_name() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha"));
        reg.register(EmbeddedSkill {
            name: "alpha".to_string(),
            description: "updated".to_string(),
            script_path: PathBuf::from("new.sh"),
            enabled: false,
        });
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].description, "updated");
    }

    #[test]
    fn unregister_removes_skill() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha"));
        assert!(reg.unregister("alpha"));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn unregister_nonexistent_returns_false() {
        let mut reg = SkillRegistry::new();
        assert!(!reg.unregister("ghost"));
    }

    #[test]
    fn set_enabled_updates_matching_skill_and_reports_change() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha"));
        reg.register(make_skill("beta"));

        let result = reg.set_enabled("beta", false);

        assert_eq!(
            result,
            crate::registry::SkillUpdateResult {
                found: true,
                changed: true,
            }
        );
        assert!(reg
            .list()
            .iter()
            .any(|skill| skill.name == "alpha" && skill.enabled));
        assert!(reg
            .list()
            .iter()
            .any(|skill| skill.name == "beta" && !skill.enabled));
    }

    #[test]
    fn set_enabled_reports_found_without_change_when_state_matches() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha"));

        let result = reg.set_enabled("alpha", true);

        assert_eq!(
            result,
            crate::registry::SkillUpdateResult {
                found: true,
                changed: false,
            }
        );
        assert!(reg.list()[0].enabled);
    }

    #[test]
    fn set_enabled_reports_missing_skill() {
        let mut reg = SkillRegistry::new();
        reg.register(make_skill("alpha"));

        let result = reg.set_enabled("ghost", false);

        assert_eq!(
            result,
            crate::registry::SkillUpdateResult {
                found: false,
                changed: false,
            }
        );
        assert!(reg.list().iter().all(|skill| skill.enabled));
    }

    #[test]
    fn load_from_dir_reads_skill_json_files() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.json"),
            serde_json::to_string(&make_skill("loaded")).unwrap(),
        )
        .unwrap();

        let mut reg = SkillRegistry::new();
        let count = reg.load_from_dir(dir.path()).unwrap();
        assert_eq!(count, 1);
        assert_eq!(reg.list()[0].name, "loaded");
    }

    #[test]
    fn load_from_dir_skips_dirs_without_skill_json() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("empty-skill");
        std::fs::create_dir(&sub).unwrap();

        let mut reg = SkillRegistry::new();
        let count = reg.load_from_dir(dir.path()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn load_from_dir_error_on_missing_dir() {
        let mut reg = SkillRegistry::new();
        let result = reg.load_from_dir(&PathBuf::from("/nonexistent/path"));
        assert!(result.is_err());
    }

    // ── Hooks tests ──

    #[test]
    fn is_gwt_managed_with_marker() {
        let hook = Hook {
            event: "pre-commit".into(),
            command: "echo hi".into(),
            comment_marker: Some("# gwt-managed: lint".into()),
        };
        assert!(is_gwt_managed(&hook));
    }

    #[test]
    fn is_gwt_managed_without_marker() {
        let hook = Hook {
            event: "pre-commit".into(),
            command: "echo hi".into(),
            comment_marker: None,
        };
        assert!(!is_gwt_managed(&hook));
    }

    #[test]
    fn is_gwt_managed_with_user_marker() {
        let hook = Hook {
            event: "pre-commit".into(),
            command: "echo hi".into(),
            comment_marker: Some("# user-custom".into()),
        };
        assert!(!is_gwt_managed(&hook));
    }

    #[test]
    fn merge_hooks_combines_both() {
        let managed = vec![make_hook("pre-commit", "lint", true)];
        let user = vec![make_hook("post-merge", "notify", false)];
        let merged = merge_hooks(&managed, &user);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_hooks_deduplicates_same_event_and_command() {
        let managed = vec![make_hook("pre-commit", "lint", true)];
        let user = vec![make_hook("pre-commit", "lint", false)];
        let merged = merge_hooks(&managed, &user);
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn merge_hooks_keeps_different_commands_same_event() {
        let managed = vec![make_hook("pre-commit", "lint", true)];
        let user = vec![make_hook("pre-commit", "test", false)];
        let merged = merge_hooks(&managed, &user);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn hooks_config_default_is_empty() {
        let cfg = HooksConfig::default();
        assert!(cfg.managed_hooks.is_empty());
        assert!(cfg.user_hooks.is_empty());
    }

    // ── Hooks backup/restore/corruption/safe-merge tests ──

    #[test]
    fn backup_hooks_creates_timestamped_and_latest_backups() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let cfg = HooksConfig {
            managed_hooks: vec![make_hook("pre-commit", "lint", true)],
            user_hooks: vec![],
        };
        std::fs::write(&hooks_file, serde_json::to_string(&cfg).unwrap()).unwrap();

        let bak = backup_hooks(&hooks_file).unwrap();
        assert!(bak.exists());
        assert_ne!(bak, hooks_file.with_extension("json.bak"));

        let timestamped_name = bak.file_name().unwrap().to_string_lossy();
        assert!(timestamped_name.starts_with("hooks.json."));
        assert!(timestamped_name.ends_with(".bak"));

        let latest = hooks_file.with_extension("json.bak");
        assert!(latest.exists());
        let latest_content: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&latest).unwrap()).unwrap();
        assert_eq!(latest_content, cfg);

        let timestamped_content: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&bak).unwrap()).unwrap();
        assert_eq!(timestamped_content, cfg);
    }

    #[test]
    fn backup_hooks_errors_on_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent.json");
        assert!(backup_hooks(&missing).is_err());
    }

    #[test]
    fn restore_from_backup_restores_content() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let original = r#"{"managed_hooks":[],"user_hooks":[]}"#;
        std::fs::write(&hooks_file, original).unwrap();
        backup_hooks(&hooks_file).unwrap();

        // Corrupt the original
        std::fs::write(&hooks_file, "CORRUPTED").unwrap();

        restore_from_backup(&hooks_file).unwrap();
        let restored = std::fs::read_to_string(&hooks_file).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn restore_from_backup_uses_timestamped_backup_when_latest_missing() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let original = HooksConfig {
            managed_hooks: vec![make_hook("pre-commit", "lint", true)],
            user_hooks: vec![make_hook("post-merge", "notify", false)],
        };
        std::fs::write(&hooks_file, serde_json::to_string(&original).unwrap()).unwrap();

        let timestamped = backup_hooks(&hooks_file).unwrap();
        std::fs::remove_file(hooks_file.with_extension("json.bak")).unwrap();
        std::fs::write(&hooks_file, "BROKEN").unwrap();

        restore_from_backup(&hooks_file).unwrap();
        let restored: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&hooks_file).unwrap()).unwrap();
        assert_eq!(restored, original);
        assert_eq!(timestamped.parent(), Some(dir.path()));
    }

    #[test]
    fn restore_from_backup_errors_when_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        std::fs::write(&hooks_file, "{}").unwrap();
        let result = restore_from_backup(&hooks_file);
        assert!(result.is_err());
    }

    #[test]
    fn detect_corruption_valid_json() {
        let valid = r#"{"managed_hooks":[],"user_hooks":[]}"#;
        assert!(!detect_corruption(valid));
    }

    #[test]
    fn detect_corruption_invalid_json() {
        assert!(detect_corruption("not json at all"));
        assert!(detect_corruption("{invalid}"));
        assert!(detect_corruption(""));
    }

    #[test]
    fn merge_hooks_safe_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let managed = vec![make_hook("pre-commit", "lint", true)];

        merge_hooks_safe(&hooks_file, &managed).unwrap();

        assert!(hooks_file.exists());
        let cfg: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&hooks_file).unwrap()).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        assert_eq!(cfg.managed_hooks[0].command, "lint");
    }

    #[test]
    fn merge_hooks_safe_preserves_user_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let initial = HooksConfig {
            managed_hooks: vec![make_hook("pre-commit", "old-lint", true)],
            user_hooks: vec![make_hook("post-merge", "notify", false)],
        };
        std::fs::write(&hooks_file, serde_json::to_string(&initial).unwrap()).unwrap();

        let new_managed = vec![make_hook("pre-commit", "new-lint", true)];
        merge_hooks_safe(&hooks_file, &new_managed).unwrap();

        let cfg: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&hooks_file).unwrap()).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        assert_eq!(cfg.managed_hooks[0].command, "new-lint");
        assert_eq!(cfg.user_hooks.len(), 1);
        assert_eq!(cfg.user_hooks[0].command, "notify");
    }

    #[test]
    fn merge_hooks_safe_recovers_from_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");

        // Write valid, then backup
        let valid = HooksConfig {
            managed_hooks: vec![],
            user_hooks: vec![make_hook("post-merge", "user-hook", false)],
        };
        std::fs::write(&hooks_file, serde_json::to_string(&valid).unwrap()).unwrap();
        backup_hooks(&hooks_file).unwrap();

        // Corrupt the main file
        std::fs::write(&hooks_file, "CORRUPT!!!").unwrap();

        let managed = vec![make_hook("pre-commit", "lint", true)];
        merge_hooks_safe(&hooks_file, &managed).unwrap();

        let cfg: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&hooks_file).unwrap()).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        // User hooks restored from backup
        assert_eq!(cfg.user_hooks.len(), 1);
        assert_eq!(cfg.user_hooks[0].command, "user-hook");
    }

    #[test]
    fn merge_hooks_safe_creates_backup() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let initial = HooksConfig::default();
        std::fs::write(&hooks_file, serde_json::to_string(&initial).unwrap()).unwrap();

        merge_hooks_safe(&hooks_file, &[]).unwrap();

        let bak = hooks_file.with_extension("json.bak");
        assert!(bak.exists());
    }

    #[test]
    fn merge_hooks_safe_recovers_empty_file_from_timestamped_backup() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let initial = HooksConfig {
            managed_hooks: vec![make_hook("pre-commit", "old-lint", true)],
            user_hooks: vec![make_hook("post-merge", "notify", false)],
        };
        std::fs::write(&hooks_file, serde_json::to_string(&initial).unwrap()).unwrap();
        backup_hooks(&hooks_file).unwrap();
        std::fs::remove_file(hooks_file.with_extension("json.bak")).unwrap();
        std::fs::write(&hooks_file, "").unwrap();

        let managed = vec![make_hook("pre-commit", "new-lint", true)];
        merge_hooks_safe(&hooks_file, &managed).unwrap();

        let cfg: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&hooks_file).unwrap()).unwrap();
        assert_eq!(cfg.managed_hooks.len(), 1);
        assert_eq!(cfg.managed_hooks[0].command, "new-lint");
        assert_eq!(cfg.user_hooks, initial.user_hooks);
    }

    #[test]
    fn merge_hooks_safe_rejects_locked_file() {
        let dir = tempfile::tempdir().unwrap();
        let hooks_file = dir.path().join("hooks.json");
        let initial = HooksConfig::default();
        std::fs::write(&hooks_file, serde_json::to_string(&initial).unwrap()).unwrap();

        let lock_path = hooks_file.with_extension("json.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .unwrap();
        lock_file.try_lock_exclusive().unwrap();

        let err = merge_hooks_safe(&hooks_file, &[]).unwrap_err();
        assert!(matches!(err, HooksError::LockUnavailable(_)));
        drop(lock_file);
    }

    #[cfg(unix)]
    #[test]
    fn merge_hooks_safe_preserves_symlink_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().join("shared");
        let workspace_dir = dir.path().join("workspace");
        std::fs::create_dir(&shared_dir).unwrap();
        std::fs::create_dir(&workspace_dir).unwrap();

        let target = shared_dir.join("hooks.json");
        let link = workspace_dir.join("hooks.json");
        let initial = HooksConfig {
            managed_hooks: vec![make_hook("pre-commit", "old-lint", true)],
            user_hooks: vec![make_hook("post-merge", "notify", false)],
        };
        std::fs::write(&target, serde_json::to_string(&initial).unwrap()).unwrap();
        symlink(&target, &link).unwrap();

        let managed = vec![make_hook("pre-commit", "new-lint", true)];
        merge_hooks_safe(&link, &managed).unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let cfg: HooksConfig =
            serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(cfg.managed_hooks[0].command, "new-lint");
        assert_eq!(cfg.user_hooks, initial.user_hooks);
    }

    // ── Bundled assets tests ──

    #[test]
    fn claude_skills_contains_expected_directories() {
        use crate::assets::CLAUDE_SKILLS;
        let dirs: Vec<&str> = CLAUDE_SKILLS
            .dirs()
            .map(|d| {
                d.path()
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("")
            })
            .collect();
        assert!(
            dirs.contains(&"gwt-fix-issue"),
            "missing gwt-fix-issue skill dir"
        );
        assert!(
            dirs.contains(&"gwt-register-issue"),
            "missing gwt-register-issue skill dir"
        );
        assert!(
            dirs.contains(&"gwt-discussion"),
            "missing gwt-discussion skill dir"
        );
        assert!(
            dirs.contains(&"gwt-plan-spec"),
            "missing gwt-plan-spec skill dir"
        );
        assert!(
            dirs.contains(&"gwt-build-spec"),
            "missing gwt-build-spec skill dir"
        );
        assert!(
            dirs.contains(&"gwt-arch-review"),
            "missing gwt-arch-review skill dir"
        );
        assert!(dirs.contains(&"gwt-search"), "missing gwt-search skill dir");
        assert!(
            dirs.contains(&"gwt-memory-search"),
            "missing gwt-memory-search skill dir"
        );
        assert!(dirs.contains(&"gwt-agent"), "missing gwt-agent skill dir");
        assert!(
            dirs.contains(&"gwt-manage-pr"),
            "missing gwt-manage-pr skill dir"
        );
        for retired in [
            "gwt-issue",
            "gwt-pr",
            "gwt-spec-design",
            "gwt-spec-plan",
            "gwt-spec-build",
        ] {
            assert!(
                !dirs.contains(&retired),
                "unexpected retired skill dir {retired}"
            );
        }
    }

    #[test]
    fn claude_commands_contains_expected_files() {
        use crate::assets::CLAUDE_COMMANDS;
        let files: Vec<&str> = CLAUDE_COMMANDS
            .files()
            .map(|f| {
                f.path()
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("")
            })
            .collect();
        assert!(
            files.contains(&"gwt-fix-issue.md"),
            "missing gwt-fix-issue.md command"
        );
        assert!(
            files.contains(&"gwt-register-issue.md"),
            "missing gwt-register-issue.md command"
        );
        assert!(
            files.contains(&"gwt-discussion.md"),
            "missing gwt-discussion.md command"
        );
        assert!(
            files.contains(&"gwt-plan-spec.md"),
            "missing gwt-plan-spec.md command"
        );
        assert!(
            files.contains(&"gwt-build-spec.md"),
            "missing gwt-build-spec.md command"
        );
        assert!(
            files.contains(&"gwt-arch-review.md"),
            "missing gwt-arch-review.md command"
        );
        assert!(
            files.contains(&"gwt-search.md"),
            "missing gwt-search.md command"
        );
        assert!(
            files.contains(&"gwt-memory-search.md"),
            "missing gwt-memory-search.md command"
        );
        assert!(
            files.contains(&"gwt-agent.md"),
            "missing gwt-agent.md command"
        );
        assert!(
            files.contains(&"gwt-manage-pr.md"),
            "missing gwt-manage-pr.md command"
        );
        for retired in [
            "gwt-issue.md",
            "gwt-pr.md",
            "gwt-spec-design.md",
            "gwt-spec-plan.md",
            "gwt-spec-build.md",
        ] {
            assert!(
                !files.contains(&retired),
                "unexpected retired command {retired}"
            );
        }
    }

    #[test]
    fn repo_does_not_keep_claude_hook_scripts() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_no_gwt_hook_scripts(&workspace_root, ".claude");
    }

    #[test]
    fn repo_does_not_track_split_codex_agent_skill_symlinks() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".codex/skills/gwt-agent-discover",
            ".codex/skills/gwt-agent-read",
            ".codex/skills/gwt-agent-send",
            ".codex/skills/gwt-agent-lifecycle",
        ] {
            assert!(
                std::fs::symlink_metadata(workspace_root.join(relative)).is_err(),
                "unexpected stale asset {relative}"
            );
        }
    }

    // SPEC-12 migration: the legacy `spec_9_uses_unified_gwt_agent_contract`
    // test referenced `specs/SPEC-9/spec.md` via `include_str!`. That file was
    // deleted when SPEC-9 migrated to GitHub Issue #1927 (then split into
    // #1932 BUILD / #1935 DESIGN-SKILLS / #1936 DOCKER / #1939 SEARCH). The
    // "unified gwt-agent contract" invariant it asserted is now enforced by
    // the actual skill filesystem layout, not by a static SPEC markdown file,
    // and is covered by `repo_does_not_keep_unmanaged_local_search_assets`
    // below plus the runtime distribution sweeps in `distribute.rs`.

    #[test]
    fn repo_does_not_keep_unmanaged_local_search_assets() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/commands/gwt-file-search.md",
            ".claude/commands/gwt-issue-search.md",
            ".claude/skills/gwt-file-search",
            ".codex/skills/gwt-file-search",
        ] {
            assert!(
                std::fs::symlink_metadata(workspace_root.join(relative)).is_err(),
                "unexpected unmanaged local asset {relative}"
            );
        }
    }

    #[test]
    fn repo_does_not_keep_codex_hook_scripts() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_no_gwt_hook_scripts(&workspace_root, ".codex");
    }

    #[test]
    fn search_commands_route_issue_queries_through_unified_gwt_search() {
        for command in [
            include_str!("../../../.claude/commands/gwt-project-index.md"),
            include_str!("../../../.claude/commands/gwt-project-search.md"),
            include_str!("../../../.claude/commands/gwt-spec-search.md"),
        ] {
            assert!(
                command.contains("/gwt:gwt-search --issues"),
                "expected unified issue-search routing"
            );
            assert!(
                !command.contains("/gwt:gwt-issue-search"),
                "unexpected legacy gwt-issue-search command reference"
            );
        }
    }

    #[test]
    fn local_github_issue_workflows_use_canonical_gwt_surfaces() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-register-issue/SKILL.md",
            ".codex/skills/gwt-register-issue/SKILL.md",
        ] {
            let issue_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                issue_skill.contains("gwtd issue create --title ... -f ..."),
                "expected canonical gwtd issue create guidance in {relative}"
            );
            assert!(
                issue_skill
                    .contains("Direct `gh issue ...` commands are not part of the normal path."),
                "expected skill to forbid direct gh issue usage in the normal path: {relative}"
            );
            assert!(
                issue_skill.contains("gwt-discussion"),
                "expected visible discussion handoff guidance in {relative}"
            );
            assert!(
                issue_skill.contains("Spec Status")
                    && issue_skill.contains("ALIGNED")
                    && issue_skill.contains("IMPLEMENTATION-GAP")
                    && issue_skill.contains("SPEC-GAP")
                    && issue_skill.contains("SPEC-AMBIGUOUS"),
                "expected registration decision status guidance in {relative}"
            );
            assert!(
                issue_skill.contains("## Related SPECs"),
                "expected related-spec section guidance in {relative}"
            );
            assert!(
                issue_skill.contains("duplicate search")
                    && issue_skill.contains("before creating anything"),
                "expected duplicate-search-first guidance in {relative}"
            );
            assert!(
                issue_skill.contains("current user's language"),
                "expected language contract in {relative}"
            );
            assert!(
                !issue_skill.contains("Load `.claude/skills/gwt-issue/SKILL.md`"),
                "unexpected retired gwt-issue dependency in {relative}"
            );
        }

        let issue_command =
            std::fs::read_to_string(workspace_root.join(".claude/commands/gwt-register-issue.md"))
                .unwrap_or_else(|err| panic!("failed to read gwt-register-issue command: {err}"));
        assert!(
            issue_command.contains("Search for duplicates before creating anything.")
                && issue_command.contains("plain Issue")
                && issue_command.contains("SPEC"),
            "expected gwt-register-issue command to describe duplicate-search-first routing"
        );

        for relative in [
            ".claude/skills/gwt-fix-issue/SKILL.md",
            ".codex/skills/gwt-fix-issue/SKILL.md",
        ] {
            let issue_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                issue_skill.contains("gwtd issue view"),
                "expected canonical gwtd issue view guidance in {relative}"
            );
            assert!(
                issue_skill.contains("gwtd issue comments"),
                "expected canonical gwtd issue comments guidance in {relative}"
            );
            assert!(
                issue_skill.contains("gwtd issue comment"),
                "expected canonical gwtd issue comment guidance in {relative}"
            );
            assert!(
                issue_skill.contains("inspect_issue.py"),
                "expected inspect_issue helper guidance in {relative}"
            );
            assert!(
                issue_skill.contains("gwt-build-spec") && issue_skill.contains("gwt-discussion"),
                "expected visible build/discussion handoff guidance in {relative}"
            );
            assert!(
                issue_skill.contains("current user's language"),
                "expected language contract in {relative}"
            );
            assert!(
                !issue_skill.contains("Load `.claude/skills/gwt-issue/SKILL.md`"),
                "unexpected retired gwt-issue dependency in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/SKILL.md",
            ".codex/skills/gwt-discussion/SKILL.md",
        ] {
            let discussion_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                discussion_skill.contains("Check open SPEC Issues: `gwtd issue spec list`."),
                "expected discussion skill to use gwtd issue spec list in {relative}"
            );
            assert!(
                discussion_skill.contains("After each answer:")
                    && discussion_skill
                        .contains("Ask the next highest-impact question if any remain"),
                "expected discussion skill to continue with the next highest-impact question in {relative}"
            );
            assert!(
                discussion_skill.contains("Codex | `request_user_input`")
                    && discussion_skill.contains("Do not end the discussion after a single answer"),
                "expected discussion skill to require Codex question UI and forbid one-answer exits in {relative}"
            );
            assert!(
                discussion_skill.contains("### Action Delta"),
                "expected discussion skill to emit Action Delta in {relative}"
            );
            assert!(
                discussion_skill.contains("### Discussion TODO")
                    && discussion_skill.contains(".gwt/discussion.md"),
                "expected discussion skill to define Discussion TODO scratch state in {relative}"
            );
            assert!(
                discussion_skill.contains("## Discussion Depth Gate")
                    && discussion_skill.contains("Coverage Checks")
                    && discussion_skill.contains("Exit Blockers"),
                "expected discussion skill to define a depth gate with coverage and exit blocker tracking in {relative}"
            );
            assert!(
                discussion_skill.contains("## Evidence Gate")
                    && discussion_skill.contains("Implementation Proof")
                    && discussion_skill.contains("SPEC/Issue Proof")
                    && discussion_skill.contains("Gap Check Proof")
                    && discussion_skill.contains("Official Docs Proof")
                    && discussion_skill.contains("External Research Proof")
                    && discussion_skill.contains("Evidence Gate: complete"),
                "expected discussion skill to require evidence proof fields in {relative}"
            );
            assert!(
                discussion_skill.contains("## Depth Interview Loop")
                    && discussion_skill.contains("Question Ledger")
                    && discussion_skill.contains("Depth Gate")
                    && discussion_skill.contains("After every answer")
                    && discussion_skill.contains("Do not treat a small number of questions as a completion signal"),
                "expected discussion skill to require depth-gated follow-up questioning in {relative}"
            );
            assert!(
                discussion_skill.contains("official documentation")
                    && discussion_skill.contains("X search")
                    && discussion_skill.contains("X API Search Posts"),
                "expected discussion skill to require official docs and X research policy in {relative}"
            );
            assert!(
                discussion_skill.contains("scope boundary")
                    && discussion_skill.contains("ownership / integration")
                    && discussion_skill.contains("failure / edge case")
                    && discussion_skill.contains("migration / compatibility")
                    && discussion_skill.contains("verification / success signal"),
                "expected discussion skill to enumerate the discussion coverage categories in {relative}"
            );
            assert!(
                discussion_skill.contains("Start the discussion in Plan Mode")
                    && discussion_skill.contains("leave Plan Mode"),
                "expected discussion skill to define Plan Mode entry and exit expectations in {relative}"
            );
            assert!(
                discussion_skill.contains(".claude/settings.local.json")
                    && discussion_skill.contains(".codex/hooks.json")
                    && discussion_skill.contains("SessionStart")
                    && discussion_skill.contains("PreToolUse")
                    && discussion_skill.contains("workflow-policy")
                    && discussion_skill.contains("UserPromptSubmit")
                    && discussion_skill.contains("Stop"),
                "expected discussion skill to describe managed hook resume timing in {relative}"
            );
            assert!(
                discussion_skill.contains("Resume discussion")
                    && discussion_skill.contains("Park proposal")
                    && discussion_skill.contains("Dismiss for now"),
                "expected discussion skill to define the resume prompt choices in {relative}"
            );
            assert!(
                discussion_skill.contains("objective review")
                    && discussion_skill.contains("subagent"),
                "expected discussion skill to describe optional subagent-based objective review in {relative}"
            );
            assert!(
                !discussion_skill.contains("gh issue list --label gwt-spec --state open"),
                "unexpected direct gh issue list guidance in {relative}"
            );
            assert!(
                discussion_skill.contains("`gwt-plan-spec`")
                    && discussion_skill.contains("`gwt-register-issue`")
                    && discussion_skill.contains("`gwt-build-spec`"),
                "expected discussion skill to hand off through visible task entrypoints in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-issue-search/SKILL.md",
            ".codex/skills/gwt-issue-search/SKILL.md",
        ] {
            let issue_search_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                issue_search_skill.contains("before manual `gwtd issue view`"),
                "expected issue search skill to steer users away from direct issue reads in {relative}"
            );
            assert!(
                !issue_search_skill.contains("before manual `gh issue list`"),
                "unexpected direct gh issue list guidance in {relative}"
            );
        }

        assert!(
            workspace_root
                .join(".claude/commands/gwt-discussion.md")
                .exists(),
            "expected tracked gwt-discussion command to remain present"
        );
        {
            let relative = ".claude/commands/gwt-discussion.md";
            let discussion_command = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                discussion_command.contains("selection UI")
                    || discussion_command.contains("request_user_input"),
                "expected discussion command to mention selection UI guidance in {relative}"
            );
            assert!(
                discussion_command.contains("next highest-impact question")
                    || discussion_command.contains("high-impact unknowns"),
                "expected discussion command to require continued questioning in {relative}"
            );
            assert!(
                discussion_command.contains("Action Bundle")
                    || discussion_command.contains("Discussion TODO"),
                "expected discussion command to mention discussion artifacts in {relative}"
            );
            assert!(
                discussion_command.contains("Plan Mode")
                    && discussion_command.contains("leave Plan Mode")
                    && discussion_command.contains("Coverage Checks")
                    && discussion_command.contains("Exit Blockers")
                    && discussion_command.contains("Depth Gate")
                    && discussion_command.contains("Question Ledger"),
                "expected discussion command to describe the Plan Mode and depth-gate contract in {relative}"
            );
            assert!(
                discussion_command.contains(".gwt/discussion.md")
                    && discussion_command.contains("Resume discussion")
                    && discussion_command.contains("Park proposal")
                    && discussion_command.contains("Dismiss for now"),
                "expected discussion command to describe the resume prompt contract in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/references/clarification.md",
            ".codex/skills/gwt-discussion/references/clarification.md",
        ] {
            let clarification = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                clarification.contains("Do not stop because a fixed question count was reached.")
                    && clarification.contains("Continue asking follow-up clarification questions"),
                "expected clarification guidance to continue beyond a fixed number of questions in {relative}"
            );
            assert!(
                clarification.contains("Planning-ready requires covering the applicable categories")
                    && clarification.contains("Depth Gate")
                    && clarification.contains("Question Ledger")
                    && !clarification.contains("Ask at most 5 questions"),
                "expected clarification guidance to require checklist coverage instead of a five-question cap in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/references/deepening.md",
            ".codex/skills/gwt-discussion/references/deepening.md",
        ] {
            let deepening = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                deepening.contains("Escalate from the normal discussion flow into deepening")
                    && deepening.contains("top 3 highest-impact points")
                    && deepening.contains("first batch, not an exit condition"),
                "expected deepening guidance to allow automatic escalation and pre-prioritization in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/references/intake.md",
            ".codex/skills/gwt-discussion/references/intake.md",
        ] {
            let intake = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                intake.contains("Do not stop after the first slice success condition alone.")
                    && intake.contains("integration target")
                    && intake.contains("explicit non-goals")
                    && intake.contains("verification signal")
                    && intake.contains("Depth Gate")
                    && intake.contains("Question Ledger"),
                "expected intake guidance to continue beyond the first-slice success signal in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-manage-pr/SKILL.md",
            ".codex/skills/gwt-manage-pr/SKILL.md",
        ] {
            let pr_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                pr_skill.contains("gwtd pr current"),
                "expected canonical gwtd pr current guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwtd pr create"),
                "expected canonical gwtd pr create guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwtd pr edit"),
                "expected canonical gwtd pr edit guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwtd pr review-threads"),
                "expected canonical gwtd pr review-threads guidance in {relative}"
            );
            assert!(
                pr_skill.contains("no current pull request"),
                "expected canonical no-PR sentinel guidance in {relative}"
            );
            assert!(
                pr_skill.contains("mergeable: CONFLICTING")
                    || pr_skill.contains("`mergeable: CONFLICTING`"),
                "expected conflict-first mergeable guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwtd actions logs"),
                "expected canonical gwtd actions log guidance in {relative}"
            );
            assert!(
                pr_skill.contains("GWT_BIN_PATH") && pr_skill.contains("target/debug/gwtd"),
                "expected managed PR skill to resolve gwtd without relying on PATH in {relative}"
            );
            assert!(
                pr_skill.contains("current user's language"),
                "expected language contract in {relative}"
            );
            assert!(
                !pr_skill.contains("Fallback: `gh pr create"),
                "unexpected direct gh pr create guidance in {relative}"
            );
            assert!(
                !pr_skill.contains("Fallback: `gh pr comment`."),
                "unexpected direct gh pr comment guidance in {relative}"
            );
            assert!(
                !pr_skill.contains(
                    "gh api \"repos/$repo_slug/pulls?state=all&head=$owner:$head&per_page=100\""
                ),
                "unexpected raw gh pull lookup guidance in {relative}"
            );
            assert!(
                !pr_skill.contains("legacy prompt or internal handoff refers to gwt-pr"),
                "unexpected retired alias guidance in {relative}"
            );
        }

        {
            let relative = ".claude/skills/gwt-manage-pr/references/check-flow.md";
            let check_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                check_flow.contains("`gwtd pr current`"),
                "expected canonical gwtd pr current guidance in {relative}"
            );
            assert!(
                check_flow.contains("no current pull request"),
                "expected canonical no-PR sentinel guidance in {relative}"
            );
            assert!(
                check_flow.contains("mergeable: CONFLICTING")
                    || check_flow.contains("`mergeable: CONFLICTING`"),
                "expected conflict-first mergeable guidance in {relative}"
            );
            assert!(
                !check_flow.contains("[ -z \"$pr_summary\" ]"),
                "unexpected empty-string no-PR detection in {relative}"
            );
            assert!(
                !check_flow.contains(
                    "gh api repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>&per_page=100"
                ),
                "unexpected direct gh pull list guidance in {relative}"
            );
        }

        {
            let relative = ".claude/skills/gwt-manage-pr/references/create-flow.md";
            let create_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                create_flow.contains("Create: `gwtd pr create"),
                "expected canonical gwtd pr create guidance in {relative}"
            );
            assert!(
                create_flow.contains("Update: `gwtd pr edit"),
                "expected canonical gwtd pr edit guidance in {relative}"
            );
            assert!(
                create_flow.contains("no current pull request"),
                "expected canonical no-PR sentinel guidance in {relative}"
            );
            assert!(
                create_flow.contains("`CONFLICTING` / `DIRTY` / `BEHIND`")
                    || create_flow.contains("CONFLICTING` / `DIRTY` / `BEHIND"),
                "expected conflict-first routing guidance in {relative}"
            );
            assert!(
                !create_flow.contains("[ -z \"$pr_summary\" ]"),
                "unexpected empty-string no-PR detection in {relative}"
            );
            assert!(
                !create_flow.contains("Create: `gh pr create"),
                "unexpected direct gh pr create guidance in {relative}"
            );
            assert!(
                !create_flow.contains("Update: `gh pr edit"),
                "unexpected direct gh pr edit guidance in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-manage-pr/references/fix-flow.md",
            ".codex/skills/gwt-manage-pr/references/fix-flow.md",
        ] {
            let fix_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                fix_flow.contains("`gwtd pr review-threads reply-and-resolve"),
                "expected canonical gwt review-thread guidance in {relative}"
            );
            assert!(
                fix_flow.contains("`gwtd pr comment"),
                "expected canonical gwtd pr comment guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("--required-only"),
                "unexpected nonexistent gwtd pr checks --required-only guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("No iteration limit"),
                "unexpected unbounded PR fix loop guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("poll at 30-second intervals until ALL checks complete"),
                "unexpected CI polling-until-complete guidance in {relative}"
            );
            assert!(
                fix_flow.contains("CI pending/queued --> check at most 3 times")
                    && fix_flow.contains("gwtd board post --kind blocked")
                    && fix_flow.contains("stop instead of sleeping indefinitely"),
                "expected bounded CI wait handoff guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("Fallback: `gh pr comment`."),
                "unexpected direct gh pr comment guidance in {relative}"
            );
        }

        let release_command = include_str!("../../../.claude/commands/release.md");
        assert!(
            release_command.contains("GWT_BIN_PATH"),
            "expected release command to route shell snippets through GWT_BIN_PATH"
        );
        assert!(
            release_command.contains("resolve_gwt_bin()")
                && release_command.contains("command -v gwtd")
                && release_command.contains("target/debug/gwtd")
                && release_command.contains("gwtd not found"),
            "expected release command to resolve gwtd from GWT_BIN_PATH, PATH, or repo-local debug binary"
        );
        assert!(
            !release_command.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwtd}\""),
            "unexpected bare gwtd fallback in release command"
        );
        assert!(
            release_command.contains("\"$GWT_BIN\" issue comment"),
            "expected release command to use the canonical gwtd issue comment via GWT_BIN"
        );
        assert!(
            release_command.contains("\"$GWT_BIN\" pr current"),
            "expected release command to use the canonical gwtd pr current via GWT_BIN"
        );
        assert!(
            release_command.contains("\"$GWT_BIN\" pr create"),
            "expected release command to use the canonical gwtd pr create via GWT_BIN"
        );
        assert!(
            release_command.contains("\"$GWT_BIN\" pr edit"),
            "expected release command to use the canonical gwtd pr edit via GWT_BIN"
        );
        assert!(
            !release_command.contains("gh issue comment"),
            "unexpected direct gh issue comment guidance"
        );

        let pr_command = include_str!("../../../.claude/commands/gwt-manage-pr.md");
        assert!(
            pr_command.contains("GWT_BIN_PATH"),
            "expected gwt-manage-pr command wrapper to point to canonical GWT_BIN_PATH auth check"
        );
        assert!(
            pr_command.contains("resolve_gwt_bin()")
                && pr_command.contains("command -v gwtd")
                && pr_command.contains("target/debug/gwtd")
                && pr_command.contains("gwtd not found"),
            "expected gwt-manage-pr command wrapper to resolve gwtd from GWT_BIN_PATH, PATH, or repo-local debug binary"
        );
        assert!(
            !pr_command.contains("GWT_BIN=\"${GWT_BIN_PATH:-gwtd}\""),
            "unexpected bare gwtd fallback in gwt-manage-pr command"
        );
        assert!(
            pr_command.contains("conflicting") || pr_command.contains("behind"),
            "expected gwt-manage-pr command wrapper to mention conflict/behind fix routing"
        );

        for relative in [
            ".claude/skills/gwt-fix-issue/scripts/inspect_issue.py",
            ".codex/skills/gwt-fix-issue/scripts/inspect_issue.py",
        ] {
            let inspect_issue_script = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                !inspect_issue_script.contains("Fetch issue metadata via gh issue view."),
                "unexpected direct gh issue view docstring in {relative}"
            );
        }
    }

    #[test]
    fn public_task_entrypoints_are_documented() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));
        for needle in [
            "gwt-register-issue",
            "gwt-fix-issue",
            "gwt-discussion",
            "gwt-plan-spec",
            "gwt-build-spec",
            "gwt-arch-review",
            "gwt-search",
            "gwt-memory-search",
            "gwt-agent",
            "gwt-manage-pr",
        ] {
            assert!(
                agents.contains(needle),
                "expected AGENTS.md to document {needle}"
            );
        }
        for retired in [
            "Compatibility Aliases",
            "gwt-issue",
            "gwt-spec-design",
            "gwt-spec-plan",
            "gwt-spec-build",
            "gwt-pr",
        ] {
            assert!(
                !agents.contains(retired),
                "unexpected retired public documentation entry {retired}"
            );
        }
    }

    #[test]
    fn public_workflow_chain_uses_current_entrypoints() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));
        assert!(
            agents.contains("gwt-register-issue / gwt-fix-issue"),
            "expected AGENTS workflow to start from the current issue entrypoints"
        );
        assert!(
            agents.contains("gwt-discussion → gwt-plan-spec → gwt-build-spec → gwt-manage-pr"),
            "expected AGENTS workflow to document the current planning/build chain"
        );
        assert!(
            agents.contains("gwt-arch-review"),
            "expected AGENTS workflow to include gwt-arch-review feedback"
        );
        for retired in [
            "gwt-design",
            "gwt-plan ",
            "gwt-build ",
            "gwt-review",
            "design → plan → build → review",
        ] {
            assert!(
                !agents.contains(retired),
                "unexpected retired workflow entry {retired}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/SKILL.md",
            ".codex/skills/gwt-discussion/SKILL.md",
            ".claude/skills/gwt-arch-review/SKILL.md",
            ".codex/skills/gwt-arch-review/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("gwt-plan-spec") && content.contains("gwt-build-spec"),
                "expected current plan/build chain guidance in {relative}"
            );
            assert!(
                content.contains("gwt-discussion"),
                "expected current discussion entrypoint guidance in {relative}"
            );
        }
    }

    #[test]
    fn unified_support_entrypoints_document_current_mode_contracts() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-manage-pr/SKILL.md",
            ".codex/skills/gwt-manage-pr/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("Single skill for the full PR lifecycle")
                    && content.contains("Auto-detect"),
                "expected unified PR lifecycle contract in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-search/SKILL.md",
            ".codex/skills/gwt-search/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("Mandatory preflight")
                    && content.contains("--specs")
                    && content.contains("--issues")
                    && content.contains("--files")
                    && content.contains("--memory"),
                "expected unified search contract in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-agent/SKILL.md",
            ".codex/skills/gwt-agent/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("Auto-detect the operation mode from arguments")
                    && content.contains("gwtd board post")
                    && content.contains("GWT_BIN_PATH")
                    && content.contains("gwtd pane list")
                    && content.contains("gwtd pane read")
                    && content.contains("gwtd pane close")
                    && content.contains("--target")
                    && content.contains("handoff")
                    && content.contains("request"),
                "expected agent Board coordination and gwtd pane contract in {relative}"
            );
            assert!(
                !content.contains("pane send")
                    && !content.contains("pane broadcast")
                    && !content.contains("`pane list`")
                    && !content.contains("`pane read")
                    && !content.contains("`pane close")
                    && !content.contains("<pane-id> <message>")
                    && !content.contains("broadcast <message>"),
                "unexpected bare pane or direct communication contract in {relative}"
            );
        }

        let command = std::fs::read_to_string(workspace_root.join(".claude/commands/gwt-agent.md"))
            .unwrap_or_else(|err| panic!("failed to read gwt-agent command: {err}"));
        assert!(
            command.contains("Board")
                && command.contains("gwtd board post")
                && command.contains("gwtd pane")
                && !command.contains("[message]")
                && !command.contains("sending"),
            "expected gwt-agent command to route pane operations through gwtd and communication through Board"
        );

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));
        assert!(
            agents.contains("Use Board for agent-to-agent communication")
                && !agents.contains("pane ID + message"),
            "expected AGENTS.md to document the Board-first gwt-agent contract"
        );
    }

    #[test]
    fn gwt_arch_review_uses_scope_based_contract() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let command =
            std::fs::read_to_string(workspace_root.join(".claude/commands/gwt-arch-review.md"))
                .unwrap_or_else(|err| panic!("failed to read gwt-arch-review command: {err}"));
        assert!(
            command.contains("/gwt:gwt-arch-review --scope repo")
                && command.contains("/gwt:gwt-arch-review --scope changed --base"),
            "expected gwt-arch-review command to document the scope-based CLI"
        );
        assert!(
            command.contains("If omitted, prompt for the scope first."),
            "expected gwt-arch-review command to mention prompt-on-omit behavior"
        );
        assert!(
            !command.contains("/gwt:gwt-arch-review [path]"),
            "unexpected legacy path-based gwt-arch-review usage"
        );

        for relative in [
            ".claude/skills/gwt-arch-review/SKILL.md",
            ".codex/skills/gwt-arch-review/SKILL.md",
        ] {
            let skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                skill.contains("`--scope repo`")
                    && skill.contains("`--scope changed --base <ref>`"),
                "expected scope-based CLI guidance in {relative}"
            );
            assert!(
                skill.contains("Changed files since a base ref"),
                "expected changed-files scope wording in {relative}"
            );
            assert!(
                skill.contains("If the caller omits scope arguments")
                    && skill.contains("Non-interactive runs must pass `--base`"),
                "expected prompt and non-interactive base rules in {relative}"
            );
            assert!(
                !skill.contains("Crate/package subset")
                    && !skill.contains("specific crates, packages, or modules"),
                "unexpected repository-specific subset guidance in {relative}"
            );
        }
    }

    #[test]
    fn gwt_spec_skills_require_user_language_outputs() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-discussion/SKILL.md",
            ".codex/skills/gwt-discussion/SKILL.md",
            ".claude/skills/gwt-discussion/references/registration.md",
            ".claude/skills/gwt-plan-spec/SKILL.md",
            ".codex/skills/gwt-plan-spec/SKILL.md",
            ".claude/skills/gwt-build-spec/SKILL.md",
            ".codex/skills/gwt-build-spec/SKILL.md",
            ".claude/skills/gwt-build-spec/references/completion-gate.md",
            ".claude/skills/gwt-plan-spec/references/quality-gate.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("current user's language"),
                "expected gwt-spec language contract in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-discussion/SKILL.md",
            ".codex/skills/gwt-discussion/SKILL.md",
            ".claude/skills/gwt-discussion/references/registration.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                !content.contains("short English imperative"),
                "unexpected English-only title rule in {relative}"
            );
            assert!(
                !content.contains("concise English imperative description"),
                "unexpected English-only title template in {relative}"
            );
            assert!(
                !content.contains("summary in English"),
                "unexpected English-only description guidance in {relative}"
            );
        }
    }

    // ── Integration: full distribution pipeline ──

    #[test]
    fn full_distribution_pipeline_creates_all_targets() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();

        init_git_repo(wt);

        // Run full pipeline
        let report = distribute_to_worktree(wt).unwrap();
        assert!(report.files_written > 0);

        update_git_exclude(wt).unwrap();
        let exclude = std::fs::read_to_string(git_resolved_exclude_path(wt)).unwrap();
        assert!(exclude.contains("gwt-managed-begin"));

        generate_settings_local(wt).unwrap();
        generate_codex_hooks(wt).unwrap();
        assert!(wt.join(".claude/settings.local.json").exists());
        assert!(wt.join(".codex/hooks.json").exists());

        // Verify all distribution targets exist
        assert!(wt.join(".claude/skills/gwt-manage-pr/SKILL.md").exists());
        assert!(wt.join(".claude/skills/gwt-fix-issue/SKILL.md").exists());
        assert!(wt.join(".codex/skills/gwt-manage-pr/SKILL.md").exists());
        assert!(wt.join(".codex/skills/gwt-fix-issue/SKILL.md").exists());
        assert!(!wt.join(".agents/skills/gwt-manage-pr/SKILL.md").exists());
        assert!(wt.join(".claude/commands/gwt-manage-pr.md").exists());
        assert!(wt.join(".claude/commands/gwt-fix-issue.md").exists());
        for retired in [
            ".claude/skills/gwt-pr/SKILL.md",
            ".codex/skills/gwt-pr/SKILL.md",
            ".claude/commands/gwt-pr.md",
            ".claude/skills/gwt-spec-design/SKILL.md",
            ".claude/skills/gwt-spec-plan/SKILL.md",
            ".claude/skills/gwt-spec-build/SKILL.md",
        ] {
            assert!(
                !wt.join(retired).exists(),
                "unexpected retired distributed asset {retired}"
            );
        }
        assert_no_gwt_hook_scripts(wt, ".claude");
        assert_no_gwt_hook_scripts(wt, ".codex");
    }

    // ── helpers ──

    fn make_skill(name: &str) -> EmbeddedSkill {
        EmbeddedSkill {
            name: name.to_string(),
            description: format!("{name} skill"),
            script_path: PathBuf::from(format!("{name}.sh")),
            enabled: true,
        }
    }

    fn make_hook(event: &str, command: &str, managed: bool) -> Hook {
        Hook {
            event: event.to_string(),
            command: command.to_string(),
            comment_marker: if managed {
                Some(format!("# gwt-managed: {command}"))
            } else {
                None
            },
        }
    }

    fn init_git_repo(path: &std::path::Path) {
        let output = std::process::Command::new("git")
            .arg("init")
            .arg(path)
            .output()
            .unwrap();
        assert!(output.status.success(), "git init failed: {:?}", output);
    }

    fn git_resolved_exclude_path(worktree: &std::path::Path) -> PathBuf {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--git-path", "info/exclude"])
            .current_dir(worktree)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git rev-parse --git-path info/exclude failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let resolved = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        if resolved.is_absolute() {
            resolved
        } else {
            worktree.join(resolved)
        }
    }

    fn assert_no_gwt_hook_scripts(root: &std::path::Path, namespace: &str) {
        let scripts_dir = root.join(namespace).join("hooks/scripts");
        if !scripts_dir.exists() {
            return;
        }

        for entry in std::fs::read_dir(&scripts_dir)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", scripts_dir.display()))
        {
            let entry = entry.unwrap_or_else(|err| {
                panic!(
                    "failed to read hook entry under {}: {err}",
                    scripts_dir.display()
                )
            });
            let name = entry.file_name();
            let name = name.to_string_lossy();
            assert!(
                !name.starts_with("gwt-"),
                "unexpected retired hook script under {}: {}",
                scripts_dir.display(),
                name
            );
        }
    }

    #[test]
    fn agents_documents_project_local_scope_and_start_work_branch_policy() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));

        for needle in [
            "この AGENTS.md は gwt リポジトリ専用",
            "任意プロジェクト向けの汎用 Agent 指示ではない",
            "Start Work / Launch materialization",
            "git checkout -b",
            "git worktree add",
        ] {
            assert!(
                agents.contains(needle),
                "expected AGENTS.md to document workspace policy phrase: {needle}"
            );
        }

        assert!(
            !agents.contains("新規 SPEC を作成した場合、現在のブランチでは実装に入らず"),
            "AGENTS.md must not require agents to create a separate worktree after new SPEC creation"
        );
    }

    #[test]
    fn agents_does_not_duplicate_generated_guidance_body() {
        // SPEC-1935 US-* (Coordination Guidance Generator):
        // Board / Workspace operational rules (kind taxonomy, audience
        // selection, body template, tool-unit post prohibition) live
        // ONLY in `crates/gwt-skills/src/coordination_guidance.rs` and
        // the `board_reminder` hook. Earlier, AGENTS.md duplicated the
        // same body which silently elevated agent compliance inside the
        // gwt repo while leaving other projects without the guidance.
        // The cleanup mandates removal of the duplicated wording so
        // every gwt-managed worktree relies on the generated SKILL.md
        // and runtime reminder.
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));

        for retired in [
            "### Board 運用 (SPEC-1974)",
            "Board は Coordination ドメインの shared chat",
            "推論可視化軸",
            "ツール単位の報告（\"running gcc\"等）",
        ] {
            assert!(
                !agents.contains(retired),
                "AGENTS.md must not duplicate Board operational content (now lives only in generated guidance): {retired}"
            );
        }

        // Positive guard: AGENTS.md must point at the canonical
        // generated-guidance surface so future maintainers find the
        // single source of truth instead of re-adding the duplicate.
        for needle in [
            ".claude/skills/gwt-coordination/SKILL.md",
            ".codex/skills/gwt-coordination/SKILL.md",
            "crates/gwt-skills/src/coordination_guidance.rs",
            "generated guidance",
        ] {
            assert!(
                agents.contains(needle),
                "AGENTS.md must reference the canonical coordination guidance surface: {needle}"
            );
        }
    }

    // SPEC-1935 Phase 17 (FR-122..FR-128, SC-056..SC-061):
    // `gwt-verify` is the canonical verification-time skill that owns
    // surface→test-matrix selection. These tests assert it is bundled
    // alongside the other gwt skills and documented in AGENTS.md.
    #[test]
    fn claude_skills_contains_gwt_verify_dir() {
        use crate::assets::CLAUDE_SKILLS;
        let dirs: Vec<&str> = CLAUDE_SKILLS
            .dirs()
            .map(|d| {
                d.path()
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("")
            })
            .collect();
        assert!(
            dirs.contains(&"gwt-verify"),
            "missing gwt-verify skill dir; SPEC-1935 Phase 17 FR-122 requires the canonical skill"
        );
    }

    #[test]
    fn claude_commands_contains_gwt_verify_md() {
        use crate::assets::CLAUDE_COMMANDS;
        let files: Vec<&str> = CLAUDE_COMMANDS
            .files()
            .map(|f| {
                f.path()
                    .file_name()
                    .unwrap_or_default()
                    .to_str()
                    .unwrap_or("")
            })
            .collect();
        assert!(
            files.contains(&"gwt-verify.md"),
            "missing gwt-verify.md command wrapper; SPEC-1935 Phase 17 FR-122 requires the slash-command entrypoint"
        );
    }

    #[test]
    fn gwt_verify_skill_md_documents_surface_matrix_contract() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                crate::validate::validate_frontmatter(&content).is_ok(),
                "{relative} must have valid YAML frontmatter"
            );
            assert!(
                content.contains("name: gwt-verify"),
                "{relative} frontmatter must declare name: gwt-verify"
            );
            // FR-123 / FR-124: Playwright limited to WebView/browser UI.
            assert!(
                content.contains("Playwright") && content.contains("WebView"),
                "{relative} must document Playwright as the WebView/browser UI execution path"
            );
            // FR-124: Rust/CLI surfaces must not invoke Playwright.
            assert!(
                content.contains("not invoke Playwright")
                    || content.contains("does not invoke Playwright"),
                "{relative} must state that non-browser surfaces do not invoke Playwright"
            );
            // FR-125: Headed/Headless contract.
            assert!(
                content.contains("headless") && content.contains("--headed"),
                "{relative} must document the default-headless + opt-in --headed contract"
            );
            // FR-126: Tooling bootstrap with failed: tooling-missing.
            assert!(
                content.contains("tooling-missing"),
                "{relative} must document the failed: tooling-missing fail mode"
            );
            // FR-128: Evidence bundle.
            assert!(
                content.contains("Verification Report") || content.contains("evidence bundle"),
                "{relative} must describe the evidence bundle output"
            );
            // Modes.
            assert!(
                content.contains("--mode quick")
                    && content.contains("--mode full")
                    && content.contains("--mode pre-pr"),
                "{relative} must document the quick / full / pre-pr modes"
            );
            // FR-123: Diff baseline.
            assert!(
                content.contains("merge-base") && content.contains("origin/develop"),
                "{relative} must specify the merge-base HEAD..origin/develop baseline"
            );
        }
    }

    #[test]
    fn gwt_verify_references_are_bundled() {
        use crate::assets::CLAUDE_SKILLS;

        let verify_dir = CLAUDE_SKILLS
            .get_dir(".claude/skills/gwt-verify")
            .or_else(|| CLAUDE_SKILLS.get_dir("gwt-verify"))
            .expect("gwt-verify directory must be bundled in CLAUDE_SKILLS");

        let references = verify_dir
            .get_dir(
                verify_dir
                    .path()
                    .join("references")
                    .to_str()
                    .expect("references path is utf8"),
            )
            .expect("gwt-verify must ship a references/ subdir");

        let reference_files: Vec<&str> = references
            .files()
            .filter_map(|f| f.path().file_name().and_then(|n| n.to_str()))
            .collect();

        for required in [
            "test-matrix.md",
            "playwright-runbook.md",
            "tooling-bootstrap.md",
        ] {
            assert!(
                reference_files.contains(&required),
                "gwt-verify references/ must include {required}"
            );
        }
    }

    #[test]
    fn build_spec_delegates_verification_to_gwt_verify() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-build-spec/SKILL.md",
            ".codex/skills/gwt-build-spec/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            // FR-127: Phase 3 must delegate to `gwt-verify --mode full`.
            assert!(
                content.contains("gwt-verify --mode full"),
                "{relative} Phase 3 must delegate verification to gwt-verify --mode full"
            );
            // Pre-amendment cargo-only verification recipe must be retired.
            // We still allow mentioning cargo elsewhere, but the four-command
            // verbatim recipe in Phase 3 must be gone.
            let phase_3_marker = "Phase 3";
            if let Some(phase_3_start) = content.find(phase_3_marker) {
                let phase_3_window = &content[phase_3_start..];
                // Find next "## Phase" heading or fall back to end.
                let phase_3_end = phase_3_window
                    .find("\n## Phase 4")
                    .or_else(|| phase_3_window.find("\n## "))
                    .unwrap_or(phase_3_window.len());
                let phase_3_body = &phase_3_window[..phase_3_end];
                assert!(
                    !phase_3_body.contains("cargo test -p gwt-core -p gwt")
                        || phase_3_body.contains("gwt-verify"),
                    "{relative} Phase 3 must not contain the legacy verbatim cargo verification list without gwt-verify delegation context"
                );
            }
        }
    }

    #[test]
    fn manage_pr_requires_gwt_verify_pre_pr() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/skills/gwt-manage-pr/SKILL.md",
            ".codex/skills/gwt-manage-pr/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            // FR-127: manage-pr must require `gwt-verify --mode pre-pr`.
            assert!(
                content.contains("gwt-verify --mode pre-pr"),
                "{relative} must require gwt-verify --mode pre-pr before PR create/update"
            );
        }
    }

    #[test]
    fn agents_documents_gwt_verify_skill() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));
        assert!(
            agents.contains("gwt-verify"),
            "AGENTS.md must document the gwt-verify skill (SPEC-1935 Phase 17 FR-122)"
        );
    }

    // SPEC-1935 Amendment (FR-129..FR-133): gwt-verify is a project-agnostic
    // verification contract. The skill must document Test Inventory emission,
    // a User Verification Handoff phase with a 4-step 導線 structure
    // (build → launch → navigate → observe), and explicit Check Items.
    // Callers (gwt-build-spec, gwt-manage-pr) must gate completion / push on
    // the User Verification Result.

    #[test]
    fn gwt_verify_documents_test_inventory_section() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("Test Inventory"),
                "{relative} must document the Test Inventory section (SPEC-1935 FR-130)"
            );
            assert!(
                content.contains("inventory unavailable"),
                "{relative} must specify the `inventory unavailable: <reason>` fallback for unparseable runner output"
            );
        }
    }

    #[test]
    fn gwt_verify_documents_user_verification_phase() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("User Verification"),
                "{relative} must document the User Verification Handoff phase (SPEC-1935 FR-131)"
            );
            assert!(
                content.contains("User Verification Result"),
                "{relative} must record User Verification Result in the evidence bundle"
            );
            assert!(
                content.contains("confirmed")
                    && content.contains("rejected")
                    && content.contains("pending"),
                "{relative} must enumerate User Verification Result states (pending/confirmed/rejected)"
            );
        }
    }

    #[test]
    fn gwt_verify_documents_4_step_navigation_path() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("導線") || content.contains("How to access"),
                "{relative} must include a 導線 / How-to-access section for user verification (SPEC-1935 FR-132)"
            );
            for step in ["build", "launch", "navigate", "observe"] {
                assert!(
                    content.contains(step),
                    "{relative} must document the 4-step 導線 structure (missing step: {step})"
                );
            }
        }
    }

    #[test]
    fn gwt_verify_documents_check_items_section() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("Check Items"),
                "{relative} must document the Check Items section presented to the user"
            );
        }
    }

    #[test]
    fn gwt_verify_is_project_agnostic() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("project-agnostic")
                    || content.contains("Generic Verification Contract"),
                "{relative} must declare itself as project-agnostic / generic verification contract (SPEC-1935 FR-129)"
            );
            // Runner manifests the skill should hint at for autodetection.
            for manifest in [
                "Cargo.toml",
                "package.json",
                "pyproject.toml",
                "go.mod",
                "ProjectSettings",
            ] {
                assert!(
                    content.contains(manifest),
                    "{relative} must mention runner-manifest signal `{manifest}` for autodetection"
                );
            }
            // Project-local AGENTS.md must take precedence over the generic matrix.
            assert!(
                content.contains("AGENTS.md") && content.contains("project"),
                "{relative} must require honoring the project's own AGENTS.md when present"
            );
        }
    }

    #[test]
    fn gwt_verify_new_references_are_bundled() {
        use crate::assets::CLAUDE_SKILLS;

        let verify_dir = CLAUDE_SKILLS
            .get_dir(".claude/skills/gwt-verify")
            .or_else(|| CLAUDE_SKILLS.get_dir("gwt-verify"))
            .expect("gwt-verify directory must be bundled in CLAUDE_SKILLS");

        let references = verify_dir
            .get_dir(
                verify_dir
                    .path()
                    .join("references")
                    .to_str()
                    .expect("references path is utf8"),
            )
            .expect("gwt-verify must ship a references/ subdir");

        let reference_files: Vec<&str> = references
            .files()
            .filter_map(|f| f.path().file_name().and_then(|n| n.to_str()))
            .collect();

        for required in [
            "surface-taxonomy.md",
            "runner-detection.md",
            "user-verification-guide.md",
        ] {
            assert!(
                reference_files.contains(&required),
                "gwt-verify references/ must include {required} (SPEC-1935 Amendment FR-129..FR-132)"
            );
        }
    }

    #[test]
    fn build_spec_phase_3_requires_user_verification_result() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-build-spec/SKILL.md",
            ".codex/skills/gwt-build-spec/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("User Verification Result"),
                "{relative} must gate Phase 3 completion on User Verification Result (SPEC-1935 FR-133)"
            );
        }
    }

    #[test]
    fn manage_pr_pre_pr_requires_user_verification_result() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-manage-pr/SKILL.md",
            ".codex/skills/gwt-manage-pr/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                content.contains("User Verification Result"),
                "{relative} must gate push / PR create / update on User Verification Result (SPEC-1935 FR-133)"
            );
        }
    }

    #[test]
    fn ready_pr_gate_is_documented_in_agents_and_pr_skills() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        let agents = std::fs::read_to_string(workspace_root.join("AGENTS.md"))
            .unwrap_or_else(|err| panic!("failed to read AGENTS.md: {err}"));
        for required in [
            "Ready PR Gate",
            "Draft PR",
            "単独で配信可能",
            "Ready PR 禁止",
        ] {
            assert!(
                agents.contains(required),
                "AGENTS.md must document Ready PR Gate phrase: {required}"
            );
        }

        for relative in [
            ".claude/skills/gwt-manage-pr/SKILL.md",
            ".codex/skills/gwt-manage-pr/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            for required in [
                "Ready PR Gate",
                "Draft PR",
                "releaseable slice",
                "Ready for review",
                "known blockers",
            ] {
                assert!(
                    content.contains(required),
                    "{relative} must document Ready/Draft PR gate phrase: {required}"
                );
            }
        }
    }

    #[test]
    fn build_spec_does_not_treat_draft_pr_as_completion() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-build-spec/SKILL.md",
            ".codex/skills/gwt-build-spec/SKILL.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            for required in [
                "Draft PR",
                "does not satisfy",
                "Ready PR Gate",
                "releaseable slice",
            ] {
                assert!(
                    content.contains(required),
                    "{relative} must make Draft PR handoffs distinct from completion: {required}"
                );
            }
        }
    }

    #[test]
    fn pr_body_template_surfaces_ready_and_draft_state() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for relative in [
            ".claude/skills/gwt-manage-pr/references/pr-body-template.md",
            ".codex/skills/gwt-manage-pr/references/pr-body-template.md",
        ] {
            let content = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            for required in [
                "PR Readiness",
                "Ready for review",
                "Draft",
                "Known blockers",
                "Remaining acceptance",
            ] {
                assert!(
                    content.contains(required),
                    "{relative} must expose PR readiness fields: {required}"
                );
            }
        }
    }
}

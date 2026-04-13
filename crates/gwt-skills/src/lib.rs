//! gwt-skills: Embedded skill bundling, distribution, and hooks management for gwt.

pub mod assets;
pub mod distribute;
pub mod git_exclude;
pub mod hooks;
pub mod registry;
pub mod settings_local;
pub mod validate;

pub use distribute::{distribute_to_worktree, prune_stale_gwt_assets, DistributeReport};
pub use git_exclude::update_git_exclude;
pub use hooks::{
    backup_hooks, detect_corruption, is_gwt_managed, merge_hooks, merge_hooks_safe,
    restore_from_backup, Hook, HooksConfig, HooksError,
};
pub use registry::{EmbeddedSkill, RegistryError, SkillRegistry};
pub use settings_local::{generate_codex_hooks, generate_settings_local};

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use std::path::PathBuf;

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
        assert!(dirs.contains(&"gwt-pr"), "missing gwt-pr skill dir");
        assert!(
            dirs.contains(&"gwt-spec-brainstorm"),
            "missing gwt-spec-brainstorm skill dir"
        );
        assert!(
            dirs.contains(&"gwt-spec-design"),
            "missing gwt-spec-design skill dir"
        );
        assert!(
            dirs.contains(&"gwt-spec-build"),
            "missing gwt-spec-build skill dir"
        );
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
        assert!(files.contains(&"gwt-pr.md"), "missing gwt-pr.md command");
        assert!(
            files.contains(&"gwt-spec-brainstorm.md"),
            "missing gwt-spec-brainstorm.md command"
        );
        assert!(
            files.contains(&"gwt-spec-design.md"),
            "missing gwt-spec-design.md command"
        );
    }

    #[test]
    fn repo_does_not_keep_claude_hook_scripts() {
        let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");

        for relative in [
            ".claude/hooks/scripts/gwt-forward-hook.mjs",
            ".claude/hooks/scripts/gwt-block-file-ops.mjs",
            ".claude/hooks/scripts/gwt-block-cd-command.mjs",
            ".claude/hooks/scripts/gwt-block-git-branch-ops.mjs",
            ".claude/hooks/scripts/gwt-block-git-dir-override.mjs",
        ] {
            assert!(
                std::fs::symlink_metadata(workspace_root.join(relative)).is_err(),
                "unexpected retired claude hook script {relative}"
            );
        }
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

        for relative in [
            ".codex/hooks/scripts/gwt-forward-hook.mjs",
            ".codex/hooks/scripts/gwt-block-file-ops.mjs",
            ".codex/hooks/scripts/gwt-block-cd-command.mjs",
            ".codex/hooks/scripts/gwt-block-git-branch-ops.mjs",
            ".codex/hooks/scripts/gwt-block-git-dir-override.mjs",
        ] {
            assert!(
                std::fs::symlink_metadata(workspace_root.join(relative)).is_err(),
                "unexpected retired codex hook script {relative}"
            );
        }
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
            ".claude/skills/gwt-issue/SKILL.md",
            ".codex/skills/gwt-issue/SKILL.md",
        ] {
            let issue_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                issue_skill.contains("gwt issue create --title ... -f ..."),
                "expected canonical gwt issue create guidance in {relative}"
            );
            assert!(
                issue_skill.contains("gwt issue comment"),
                "expected canonical gwt issue comment guidance in {relative}"
            );
            assert!(
                issue_skill
                    .contains("Direct `gh issue ...` commands are not part of the normal path."),
                "expected skill to forbid direct gh issue usage in the normal path: {relative}"
            );
            assert!(
                !issue_skill.contains("Plain Issue: create directly with `gh issue create`."),
                "unexpected direct gh issue create guidance in {relative}"
            );
            assert!(
                !issue_skill.contains("Before posting with `gh issue comment`"),
                "unexpected direct gh issue comment guidance in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-spec-brainstorm/SKILL.md",
            ".codex/skills/gwt-spec-brainstorm/SKILL.md",
        ] {
            let brainstorm_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                brainstorm_skill.contains("Check open SPEC Issues: `gwt issue spec list`."),
                "expected brainstorm skill to use gwt issue spec list in {relative}"
            );
            assert!(
                brainstorm_skill.contains("After each answer:")
                    && brainstorm_skill.contains("Ask the next highest-impact question if any remain"),
                "expected brainstorm skill to continue with the next highest-impact question in {relative}"
            );
            assert!(
                brainstorm_skill.contains("Codex | `request_user_input`")
                    && brainstorm_skill.contains("Do not end the brainstorm after a single answer"),
                "expected brainstorm skill to require Codex question UI and forbid one-answer exits in {relative}"
            );
            assert!(
                brainstorm_skill.contains("### SPEC Delta"),
                "expected brainstorm skill to emit SPEC Delta in {relative}"
            );
            assert!(
                !brainstorm_skill.contains("gh issue list --label gwt-spec --state open"),
                "unexpected direct gh issue list guidance in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-issue-search/SKILL.md",
            ".codex/skills/gwt-issue-search/SKILL.md",
        ] {
            let issue_search_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                issue_search_skill.contains("before manual `gwt issue view`"),
                "expected issue search skill to steer users away from direct issue reads in {relative}"
            );
            assert!(
                !issue_search_skill.contains("before manual `gh issue list`"),
                "unexpected direct gh issue list guidance in {relative}"
            );
        }

        assert!(
            workspace_root
                .join(".claude/commands/gwt-spec-brainstorm.md")
                .exists(),
            "expected tracked gwt-spec-brainstorm command to remain present"
        );
        {
            let relative = ".claude/commands/gwt-spec-brainstorm.md";
            let brainstorm_command = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                brainstorm_command.contains("selection UI")
                    || brainstorm_command.contains("request_user_input"),
                "expected brainstorm command to mention selection UI guidance in {relative}"
            );
            assert!(
                brainstorm_command.contains("next highest-impact question")
                    || brainstorm_command.contains("high-impact unknowns"),
                "expected brainstorm command to require continued questioning in {relative}"
            );
        }

        for relative in [
            ".claude/skills/gwt-pr/SKILL.md",
            ".codex/skills/gwt-pr/SKILL.md",
        ] {
            let pr_skill = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                pr_skill.contains("gwt pr current"),
                "expected canonical gwt pr current guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwt pr create"),
                "expected canonical gwt pr create guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwt pr edit"),
                "expected canonical gwt pr edit guidance in {relative}"
            );
            assert!(
                pr_skill.contains("gwt pr review-threads"),
                "expected canonical gwt pr review-threads guidance in {relative}"
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
                pr_skill.contains("gwt actions logs"),
                "expected canonical gwt actions log guidance in {relative}"
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
        }

        {
            let relative = ".claude/skills/gwt-pr/references/check-flow.md";
            let check_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                check_flow.contains("`gwt pr current`"),
                "expected canonical gwt pr current guidance in {relative}"
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
            let relative = ".claude/skills/gwt-pr/references/create-flow.md";
            let create_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                create_flow.contains("Create: `gwt pr create"),
                "expected canonical gwt pr create guidance in {relative}"
            );
            assert!(
                create_flow.contains("Update: `gwt pr edit"),
                "expected canonical gwt pr edit guidance in {relative}"
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

        {
            let relative = ".claude/skills/gwt-pr/references/fix-flow.md";
            let fix_flow = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                fix_flow.contains("`gwt pr review-threads reply-and-resolve"),
                "expected canonical gwt review-thread guidance in {relative}"
            );
            assert!(
                fix_flow.contains("`gwt pr comment"),
                "expected canonical gwt pr comment guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("--required-only"),
                "unexpected nonexistent gwt pr checks --required-only guidance in {relative}"
            );
            assert!(
                !fix_flow.contains("Fallback: `gh pr comment`."),
                "unexpected direct gh pr comment guidance in {relative}"
            );
        }

        let release_command = include_str!("../../../.claude/commands/release.md");
        assert!(
            release_command.contains("gwt issue comment"),
            "expected release command to use gwt issue comment"
        );
        assert!(
            release_command.contains("gwt pr current"),
            "expected release command to use gwt pr current"
        );
        assert!(
            release_command.contains("gwt pr create"),
            "expected release command to use gwt pr create"
        );
        assert!(
            release_command.contains("gwt pr edit"),
            "expected release command to use gwt pr edit"
        );
        assert!(
            !release_command.contains("gh issue comment"),
            "unexpected direct gh issue comment guidance"
        );

        let pr_command = include_str!("../../../.claude/commands/gwt-pr.md");
        assert!(
            pr_command.contains("`gwt pr current` should succeed"),
            "expected gwt-pr command wrapper to point to canonical gwt auth check"
        );
        assert!(
            pr_command.contains("conflicting") || pr_command.contains("behind"),
            "expected gwt-pr command wrapper to mention conflict/behind fix routing"
        );

        for relative in [
            ".claude/skills/gwt-issue/scripts/inspect_issue.py",
            ".codex/skills/gwt-issue/scripts/inspect_issue.py",
        ] {
            let inspect_issue_script = std::fs::read_to_string(workspace_root.join(relative))
                .unwrap_or_else(|err| panic!("failed to read {relative}: {err}"));
            assert!(
                !inspect_issue_script.contains("Fetch issue metadata via gh issue view."),
                "unexpected direct gh issue view docstring in {relative}"
            );
        }
    }

    // ── Integration: full distribution pipeline ──

    #[test]
    fn full_distribution_pipeline_creates_all_targets() {
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();

        // Create .git/info for git_exclude
        std::fs::create_dir_all(wt.join(".git/info")).unwrap();

        // Run full pipeline
        let report = distribute_to_worktree(wt).unwrap();
        assert!(report.files_written > 0);

        update_git_exclude(wt).unwrap();
        let exclude = std::fs::read_to_string(wt.join(".git/info/exclude")).unwrap();
        assert!(exclude.contains("gwt-managed-begin"));

        generate_settings_local(wt).unwrap();
        generate_codex_hooks(wt).unwrap();
        assert!(wt.join(".claude/settings.local.json").exists());
        assert!(wt.join(".codex/hooks.json").exists());

        // Verify all distribution targets exist
        assert!(wt.join(".claude/skills/gwt-pr/SKILL.md").exists());
        assert!(wt.join(".codex/skills/gwt-pr/SKILL.md").exists());
        assert!(!wt.join(".agents/skills/gwt-pr/SKILL.md").exists());
        assert!(wt.join(".claude/commands/gwt-pr.md").exists());
        assert!(
            !wt.join(".claude/hooks/scripts/gwt-forward-hook.mjs")
                .exists(),
            "unexpected retired claude hook script"
        );
        assert!(
            !wt.join(".codex/hooks/scripts/gwt-forward-hook.mjs")
                .exists(),
            "unexpected retired codex hook script"
        );
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
}

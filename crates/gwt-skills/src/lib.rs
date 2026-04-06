//! gwt-skills: Embedded skill bundling, distribution, and hooks management for gwt.

pub mod assets;
pub mod distribute;
pub mod git_exclude;
pub mod hooks;
pub mod registry;
pub mod settings_local;
pub mod validate;

pub use distribute::{distribute_to_worktree, DistributeReport};
pub use git_exclude::update_git_exclude;
pub use hooks::{
    backup_hooks, detect_corruption, is_gwt_managed, merge_hooks, merge_hooks_safe,
    restore_from_backup, Hook, HooksConfig, HooksError,
};
pub use registry::{EmbeddedSkill, RegistryError, SkillRegistry};
pub use settings_local::generate_settings_local;

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
        assert!(dirs.contains(&"gwt-spec-design"), "missing gwt-spec-design skill dir");
        assert!(dirs.contains(&"gwt-spec-build"), "missing gwt-spec-build skill dir");
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
            files.contains(&"gwt-spec-design.md"),
            "missing gwt-spec-design.md command"
        );
    }

    #[test]
    fn claude_hooks_contains_expected_scripts() {
        use crate::assets::CLAUDE_HOOKS;
        let files: Vec<&str> = CLAUDE_HOOKS
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
            files.contains(&"gwt-forward-hook.mjs"),
            "missing gwt-forward-hook.mjs"
        );
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

        let hooks = vec![make_hook("PreToolUse", "gwt-hook", true)];
        generate_settings_local(wt, &hooks).unwrap();
        assert!(wt.join(".claude/settings.local.json").exists());

        // Verify all distribution targets exist
        assert!(wt.join(".claude/skills/gwt-pr/SKILL.md").exists());
        assert!(wt.join(".codex/skills/gwt-pr/SKILL.md").exists());
        assert!(!wt.join(".agents/skills/gwt-pr/SKILL.md").exists());
        assert!(wt.join(".claude/commands/gwt-pr.md").exists());
        assert!(wt
            .join(".claude/hooks/scripts/gwt-forward-hook.mjs")
            .exists());
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

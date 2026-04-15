use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

use tempfile::tempdir;

use gwt::refresh_managed_gwt_assets_for_worktree;

#[test]
fn refresh_managed_gwt_assets_materializes_skills_commands_hooks_and_excludes() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    let _env_guard = env_lock();
    let cli_bin = dir.path().join("bin/gwt");
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
    assert!(claude_settings.contains(&cli_bin_text));
    assert!(codex_hooks.contains(&cli_bin_text));

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
        .unwrap_or_else(|p| p.into_inner())
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

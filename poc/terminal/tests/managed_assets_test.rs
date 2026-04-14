use std::path::Path;

use tempfile::tempdir;

use poc_terminal::refresh_managed_gwt_assets_for_worktree;

#[test]
fn refresh_managed_gwt_assets_materializes_skills_commands_hooks_and_excludes() {
    let dir = tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);

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

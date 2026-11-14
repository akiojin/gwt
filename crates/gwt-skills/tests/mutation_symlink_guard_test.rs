//! Issue #4487: managed writers must never mutate an external symlink target.

#![cfg(any(unix, windows))]

use std::{fs, io, path::Path};

use gwt_skills::{distribute_to_worktree, generate_settings_local, prune_stale_gwt_assets};

fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let output = gwt_core::process::hidden_command("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.components().collect::<std::path::PathBuf>())
            .arg(target.components().collect::<std::path::PathBuf>())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

fn assert_rejected(error: io::Error, route: &str, path: &Path) {
    assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{error}");
    let message = error.to_string();
    assert!(message.contains(route), "{message}");
    assert!(
        message
            .replace('\\', "/")
            .contains(&path.display().to_string().replace('\\', "/")),
        "{message}"
    );
}

#[test]
fn distribution_rejects_external_managed_skill_before_write_or_prune() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("worktree");
    let outside = root.path().join("outside");
    fs::create_dir_all(worktree.join(".claude/skills")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("SKILL.md"), "sentinel").unwrap();
    fs::write(outside.join("stale.md"), "keep").unwrap();
    let link = worktree.join(".claude/skills/gwt-agent");
    link_dir(&outside, &link);

    assert_rejected(
        distribute_to_worktree(&worktree).unwrap_err(),
        "delete",
        &link,
    );
    assert_rejected(
        prune_stale_gwt_assets(&worktree).unwrap_err(),
        "delete",
        &link,
    );
    assert_eq!(
        fs::read_to_string(outside.join("SKILL.md")).unwrap(),
        "sentinel"
    );
    assert_eq!(
        fs::read_to_string(outside.join("stale.md")).unwrap(),
        "keep"
    );
}

#[test]
fn settings_rejects_external_ancestor_without_publishing_or_staging() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("worktree");
    let outside = root.path().join("outside");
    fs::create_dir(&worktree).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("settings.local.json"), "{}").unwrap();
    let link = worktree.join(".claude");
    link_dir(&outside, &link);

    assert_rejected(
        generate_settings_local(&worktree).unwrap_err(),
        "write",
        &link,
    );
    assert_eq!(
        fs::read_to_string(outside.join("settings.local.json")).unwrap(),
        "{}"
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
}

#[test]
fn distribution_ignores_unrelated_project_and_hermes_credential_links() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("worktree");
    let outside = root.path().join("outside");
    fs::create_dir_all(worktree.join(".claude/skills")).unwrap();
    fs::create_dir_all(worktree.join(".gwt/hermes")).unwrap();
    fs::create_dir_all(worktree.join(".claude/hooks")).unwrap();
    fs::create_dir_all(worktree.join(".codex/hooks")).unwrap();
    fs::create_dir(&outside).unwrap();
    let empty = root.path().join("outside-empty");
    fs::create_dir(&empty).unwrap();
    fs::write(outside.join("sentinel"), "keep").unwrap();
    let project = worktree.join(".claude/skills/project-helper");
    let credential = worktree.join(".gwt/hermes/auth.json");
    let scripts = worktree.join(".claude/hooks/scripts");
    let empty_scripts = worktree.join(".codex/hooks/scripts");
    link_dir(&outside, &project);
    link_dir(&outside, &credential);
    link_dir(&outside, &scripts);
    link_dir(&empty, &empty_scripts);

    distribute_to_worktree(&worktree).unwrap();
    assert!(fs::read_link(project).is_ok());
    assert!(fs::read_link(credential).is_ok());
    assert!(fs::read_link(&scripts).is_ok());
    assert!(fs::read_link(empty_scripts).is_ok());
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
    let retired = outside.join("gwt-retired.sh");
    fs::write(&retired, "keep retired").unwrap();
    assert_rejected(
        prune_stale_gwt_assets(&worktree).unwrap_err(),
        "delete",
        &scripts,
    );
    assert_eq!(fs::read_to_string(retired).unwrap(), "keep retired");
}

#[test]
fn prune_shared_project_root_ignores_user_entries_but_rejects_managed_deletions() {
    use gwt_skills::{prune_stale_gwt_assets_for_targets, ManagedAssetTarget};
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("worktree");
    let outside = root.path().join("outside");
    fs::create_dir_all(worktree.join(".claude")).unwrap();
    fs::create_dir_all(outside.join("project-helper")).unwrap();
    let sentinel = outside.join("project-helper/SKILL.md");
    fs::write(&sentinel, "user content").unwrap();
    let link = worktree.join(".claude/skills");
    link_dir(&outside, &link);

    assert_eq!(
        prune_stale_gwt_assets_for_targets(&worktree, &[ManagedAssetTarget::ClaudeCode]).unwrap(),
        0
    );
    fs::create_dir(outside.join("gwt-retired")).unwrap();
    let retired = outside.join("gwt-retired/SKILL.md");
    fs::write(&retired, "keep retired").unwrap();
    assert_rejected(
        prune_stale_gwt_assets_for_targets(&worktree, &[ManagedAssetTarget::ClaudeCode])
            .unwrap_err(),
        "delete",
        &link,
    );
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "user content");
    assert_eq!(fs::read_to_string(retired).unwrap(), "keep retired");
    assert!(fs::read_link(link).is_ok());
}

#[test]
fn mutation_helpers_reject_linked_parents_with_their_route_and_path() {
    use gwt_skills::asset_io;
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let source = root.path().join("source");
    fs::write(&source, "new").unwrap();
    fs::write(outside.join("sentinel"), "keep").unwrap();
    let link = root.path().join("link");
    link_dir(&outside, &link);
    let target = link.join("sentinel");

    assert_rejected(
        asset_io::write(&target, "new").unwrap_err().into(),
        "write",
        &link,
    );
    assert_rejected(
        asset_io::create_dir_all(link.join("new"))
            .unwrap_err()
            .into(),
        "write",
        &link,
    );
    assert_rejected(
        asset_io::backup(&source, &target).unwrap_err().into(),
        "backup",
        &link,
    );
    assert_rejected(
        asset_io::rename(&source, &target).unwrap_err().into(),
        "write",
        &link,
    );
    assert_rejected(
        asset_io::remove_file(&target).unwrap_err().into(),
        "delete",
        &link,
    );
    assert_rejected(
        asset_io::remove_dir(&link).unwrap_err().into(),
        "delete",
        &link,
    );
    assert_rejected(
        asset_io::remove_dir_all(&link).unwrap_err().into(),
        "delete",
        &link,
    );
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_to_string(source).unwrap(), "new");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
}

#[test]
fn hooks_backup_restore_and_merge_reject_external_parent() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("hooks.json"), "{}").unwrap();
    fs::write(outside.join("hooks.json.bak"), "backup").unwrap();
    let link = root.path().join("link");
    link_dir(&outside, &link);
    let target = link.join("hooks.json");

    for (route, error) in [
        ("backup", gwt_skills::backup_hooks(&target).unwrap_err()),
        (
            "write",
            gwt_skills::restore_from_backup(&target).unwrap_err(),
        ),
        (
            "write",
            gwt_skills::merge_hooks_safe(&target, &[]).unwrap_err(),
        ),
    ] {
        let message = error.to_string();
        assert!(message.contains(route), "{message}");
        assert!(message.contains(&link.display().to_string()), "{message}");
    }
    assert_eq!(
        fs::read_to_string(outside.join("hooks.json")).unwrap(),
        "{}"
    );
    assert_eq!(
        fs::read_to_string(outside.join("hooks.json.bak")).unwrap(),
        "backup"
    );
    assert_eq!(fs::read_dir(outside).unwrap().count(), 2);
}

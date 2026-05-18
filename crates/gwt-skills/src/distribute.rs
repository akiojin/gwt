//! Distribute bundled skill assets to a target worktree.

use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::process::hidden_command;
use include_dir::Dir;

use crate::assets::{CLAUDE_COMMANDS, CLAUDE_SKILLS};

const TRACKED_ROOTS: &[&str] = &[
    ".claude/skills",
    ".claude/commands",
    ".codex/skills",
    ".gwt/hermes/skills",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ManagedAssetTarget {
    ClaudeCode,
    Codex,
    OpenCode,
    OpenClaw,
    Hermes,
}

impl ManagedAssetTarget {
    pub const ALL: [Self; 5] = [
        Self::ClaudeCode,
        Self::Codex,
        Self::OpenCode,
        Self::OpenClaw,
        Self::Hermes,
    ];
}

/// Result of a distribution operation.
#[derive(Debug, Default)]
pub struct DistributeReport {
    /// Number of files written.
    pub files_written: usize,
    /// Number of directories created.
    pub dirs_created: usize,
    /// Number of stale managed-namespace paths removed.
    pub paths_removed: usize,
}

#[derive(Clone, Copy)]
enum RootEntryKind {
    Directories,
    Files,
}

/// Write bundled skill and command files to the target worktree.
///
/// Full distribution targets:
/// - `.claude/skills/gwt-*/`
/// - `.claude/commands/gwt-*.md`
/// - `.codex/skills/gwt-*/`  (same skill content)
/// - `.gwt/hermes/skills/gwt-*/`  (gwt skill content only)
///
/// Provider hook execution is driven by generated config owned by the
/// caller (`.claude/settings.local.json`, `.codex/hooks.json`, and
/// `.gwt/<provider>/...` provider homes).
/// Retired `hooks/scripts/gwt-*` files are pruned as stale managed
/// assets instead of being redistributed.
pub fn distribute_to_worktree(worktree: &Path) -> io::Result<DistributeReport> {
    distribute_to_worktree_for_targets(worktree, &ManagedAssetTarget::ALL)
}

pub fn distribute_to_worktree_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
) -> io::Result<DistributeReport> {
    let mut report = DistributeReport::default();
    let tracked_paths = tracked_gwt_asset_paths(worktree);
    let targets = normalize_targets(targets);

    prune_managed_asset_roots_for_targets(worktree, &tracked_paths, &targets, &mut report)?;

    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        write_gwt_skill_dir_assets(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".claude/skills"),
            &tracked_paths,
            &mut report,
        )?;
        write_dir_assets(
            &CLAUDE_COMMANDS,
            worktree,
            &worktree.join(".claude/commands"),
            &tracked_paths,
            &mut report,
        )?;
    }

    if targets.contains(&ManagedAssetTarget::Codex) {
        write_gwt_skill_dir_assets(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".codex/skills"),
            &tracked_paths,
            &mut report,
        )?;
    }

    if targets.contains(&ManagedAssetTarget::Hermes) {
        write_gwt_skill_dir_assets(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".gwt/hermes/skills"),
            &tracked_paths,
            &mut report,
        )?;
    }

    Ok(report)
}

/// Remove stale gwt-managed asset paths from the target worktree without
/// materializing the current bundle.
pub fn prune_stale_gwt_assets(worktree: &Path) -> io::Result<usize> {
    prune_stale_gwt_assets_for_targets(worktree, &ManagedAssetTarget::ALL)
}

pub fn prune_stale_gwt_assets_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
) -> io::Result<usize> {
    let mut report = DistributeReport::default();
    let tracked_paths = tracked_gwt_asset_paths(worktree);
    let targets = normalize_targets(targets);
    prune_managed_asset_roots_for_targets(worktree, &tracked_paths, &targets, &mut report)?;
    Ok(report.paths_removed)
}

fn prune_managed_asset_roots_for_targets(
    worktree: &Path,
    tracked_paths: &HashSet<PathBuf>,
    targets: &[ManagedAssetTarget],
    report: &mut DistributeReport,
) -> io::Result<()> {
    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        prune_dir_against_source(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".claude/skills"),
            Some(RootEntryKind::Directories),
            true,
            tracked_paths,
            report,
        )?;
        prune_dir_against_source(
            &CLAUDE_COMMANDS,
            worktree,
            &worktree.join(".claude/commands"),
            Some(RootEntryKind::Files),
            false,
            tracked_paths,
            report,
        )?;
        prune_retired_hook_scripts(&worktree.join(".claude/hooks/scripts"), report)?;
    }

    if targets.contains(&ManagedAssetTarget::Codex) {
        prune_dir_against_source(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".codex/skills"),
            Some(RootEntryKind::Directories),
            true,
            tracked_paths,
            report,
        )?;
        prune_retired_hook_scripts(&worktree.join(".codex/hooks/scripts"), report)?;
    }

    if targets.contains(&ManagedAssetTarget::Hermes) {
        prune_dir_against_source(
            &CLAUDE_SKILLS,
            worktree,
            &worktree.join(".gwt/hermes/skills"),
            Some(RootEntryKind::Directories),
            true,
            tracked_paths,
            report,
        )?;
    }

    Ok(())
}

fn normalize_targets(targets: &[ManagedAssetTarget]) -> Vec<ManagedAssetTarget> {
    let mut normalized = Vec::new();
    for target in targets {
        if !normalized.contains(target) {
            normalized.push(*target);
        }
    }
    normalized
}

fn prune_retired_hook_scripts(dest: &Path, report: &mut DistributeReport) -> io::Result<()> {
    if dest.exists() {
        for entry in fs::read_dir(dest)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("gwt-") {
                continue;
            }
            remove_path(&entry.path())?;
            report.paths_removed += 1;
        }

        if fs::read_dir(dest)?.next().is_none() {
            fs::remove_dir(dest)?;
            report.paths_removed += 1;
        }
    }

    let Some(parent) = dest.parent() else {
        return Ok(());
    };
    if parent.exists() && fs::read_dir(parent)?.next().is_none() {
        fs::remove_dir(parent)?;
        report.paths_removed += 1;
    }

    Ok(())
}

fn write_dir_assets(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    tracked_paths: &HashSet<PathBuf>,
    report: &mut DistributeReport,
) -> io::Result<()> {
    write_dir_assets_inner(source, worktree, dest, tracked_paths, false, report)
}

fn write_gwt_skill_dir_assets(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    tracked_paths: &HashSet<PathBuf>,
    report: &mut DistributeReport,
) -> io::Result<()> {
    write_dir_assets_inner(source, worktree, dest, tracked_paths, true, report)
}

fn write_dir_assets_inner(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    tracked_paths: &HashSet<PathBuf>,
    root_gwt_only: bool,
    report: &mut DistributeReport,
) -> io::Result<()> {
    for file in source.files() {
        let target = dest.join(file.path().file_name().unwrap_or_default());
        // Preserve existing tracked files so we do not overwrite user-checked-in
        // assets with an older bundle snapshot, but still recreate missing
        // tracked files so worktrees can self-heal after an accidental delete.
        if target.exists() && should_skip_tracked_path(worktree, &target, tracked_paths) {
            continue;
        }
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
                report.dirs_created += 1;
            }
        }
        fs::write(&target, file.contents())?;
        report.files_written += 1;
    }

    for subdir in source.dirs() {
        let subdir_name = subdir.path().file_name().unwrap_or_default();
        if root_gwt_only && !subdir_name.to_string_lossy().starts_with("gwt-") {
            continue;
        }
        write_dir_assets(
            subdir,
            worktree,
            &dest.join(subdir_name),
            tracked_paths,
            report,
        )?;
    }

    Ok(())
}

fn prune_dir_against_source(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    root_kind: Option<RootEntryKind>,
    root_gwt_only: bool,
    tracked_paths: &HashSet<PathBuf>,
    report: &mut DistributeReport,
) -> io::Result<()> {
    if !dest.exists() {
        return Ok(());
    }

    let desired_file_names: HashSet<String> = source
        .files()
        .filter_map(|file| file.path().file_name().and_then(|name| name.to_str()))
        .map(str::to_string)
        .collect();
    let desired_dir_names: HashSet<String> = source
        .dirs()
        .filter_map(|dir| dir.path().file_name().and_then(|name| name.to_str()))
        .filter(|name| !root_gwt_only || name.starts_with("gwt-"))
        .map(str::to_string)
        .collect();

    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if root_kind.is_some() && !name.starts_with("gwt-") {
            continue;
        }

        let keep = match root_kind {
            Some(RootEntryKind::Directories) => desired_dir_names.contains(name.as_ref()),
            Some(RootEntryKind::Files) => desired_file_names.contains(name.as_ref()),
            None => {
                desired_file_names.contains(name.as_ref())
                    || desired_dir_names.contains(name.as_ref())
            }
        };

        if !keep {
            // Protect git-tracked files from being pruned even if
            // they are not in the current binary's bundle. This
            // prevents the race condition where a newly added skill
            // or command file is committed to git but has not yet
            // been included in a build's `include_dir!` snapshot.
            if should_skip_tracked_path(worktree, &entry.path(), tracked_paths) {
                continue;
            }
            remove_path(&entry.path())?;
            report.paths_removed += 1;
        }
    }

    for subdir in source.dirs() {
        let subdir_name = subdir.path().file_name().unwrap_or_default();
        if root_gwt_only && !subdir_name.to_string_lossy().starts_with("gwt-") {
            continue;
        }
        prune_dir_against_source(
            subdir,
            worktree,
            &dest.join(subdir_name),
            None,
            false,
            tracked_paths,
            report,
        )?;
    }

    Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn should_skip_tracked_path(
    worktree: &Path,
    target: &Path,
    tracked_paths: &HashSet<PathBuf>,
) -> bool {
    target
        .strip_prefix(worktree)
        .ok()
        .map(|relative| {
            tracked_paths
                .iter()
                .any(|tracked| relative == tracked || relative.starts_with(tracked))
        })
        .unwrap_or(false)
}

fn tracked_gwt_asset_paths(worktree: &Path) -> HashSet<PathBuf> {
    match hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .args(TRACKED_ROOTS)
        .output()
    {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect(),
        Ok(_) if is_git_worktree(worktree) => TRACKED_ROOTS.iter().map(PathBuf::from).collect(),
        Ok(_) => HashSet::new(),
        Err(_) if worktree.join(".git").exists() => {
            TRACKED_ROOTS.iter().map(PathBuf::from).collect()
        }
        Err(_) => HashSet::new(),
    }
}

fn is_git_worktree(worktree: &Path) -> bool {
    match hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
    {
        Ok(output) => {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        }
        Err(_) => worktree.join(".git").exists(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribute_creates_claude_skills() {
        let dir = tempfile::tempdir().unwrap();
        let report = distribute_to_worktree(dir.path()).unwrap();
        assert!(report.files_written > 0);
        let skill_md = dir.path().join(".claude/skills/gwt-manage-pr/SKILL.md");
        assert!(skill_md.exists(), "expected {}", skill_md.display());
    }

    #[test]
    fn distribute_creates_codex_skills() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let skill_md = dir.path().join(".codex/skills/gwt-manage-pr/SKILL.md");
        assert!(skill_md.exists(), "expected {}", skill_md.display());
    }

    #[test]
    fn distribute_for_targets_only_materializes_requested_agent_assets() {
        let dir = tempfile::tempdir().unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::Codex]).unwrap();

        assert!(dir
            .path()
            .join(".codex/skills/gwt-manage-pr/SKILL.md")
            .exists());
        assert!(
            !dir.path()
                .join(".claude/skills/gwt-manage-pr/SKILL.md")
                .exists(),
            "Codex launch must not materialize Claude Code skills"
        );
        assert!(
            !dir.path()
                .join(".gwt/hermes/skills/gwt-manage-pr/SKILL.md")
                .exists(),
            "Codex launch must not materialize Hermes skills"
        );
    }

    #[test]
    fn distribute_for_targets_materializes_hermes_skills_under_hermes_home() {
        let dir = tempfile::tempdir().unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::Hermes]).unwrap();

        let skill_md = dir
            .path()
            .join(".gwt/hermes/skills/gwt-build-spec/SKILL.md");
        assert!(skill_md.exists(), "expected {}", skill_md.display());
        assert!(
            !dir.path()
                .join(".claude/skills/gwt-build-spec/SKILL.md")
                .exists(),
            "Hermes launch must not materialize Claude Code skills"
        );
        assert!(
            !dir.path()
                .join(".codex/skills/gwt-build-spec/SKILL.md")
                .exists(),
            "Hermes launch must not materialize Codex skills"
        );
    }

    #[test]
    fn distribute_for_targets_preserves_non_gwt_hermes_skill_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let user_asset = dir
            .path()
            .join(".gwt/hermes/skills/tui-design/user-note.md");
        fs::create_dir_all(user_asset.parent().unwrap()).unwrap();
        fs::write(&user_asset, "user owned").unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::Hermes]).unwrap();

        assert!(
            user_asset.exists(),
            "Hermes distribution must not prune non-gwt skill content"
        );
    }

    #[test]
    fn distribute_for_targets_skips_non_gwt_claude_skill_for_claude_code() {
        let dir = tempfile::tempdir().unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::ClaudeCode]).unwrap();

        assert!(
            !dir.path().join(".claude/skills/tui-design").exists(),
            "tui-design must not be materialized for ClaudeCode target"
        );
        assert!(
            dir.path()
                .join(".claude/skills/gwt-manage-pr/SKILL.md")
                .exists(),
            "gwt-* skill must still be materialized for ClaudeCode target"
        );
    }

    #[test]
    fn distribute_for_targets_skips_non_gwt_codex_skill_for_codex() {
        let dir = tempfile::tempdir().unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::Codex]).unwrap();

        assert!(
            !dir.path().join(".codex/skills/tui-design").exists(),
            "tui-design must not be materialized for Codex target"
        );
        assert!(
            dir.path()
                .join(".codex/skills/gwt-manage-pr/SKILL.md")
                .exists(),
            "gwt-* skill must still be materialized for Codex target"
        );
    }

    #[test]
    fn distribute_for_targets_preserves_user_created_non_gwt_claude_skill() {
        let dir = tempfile::tempdir().unwrap();
        let user_asset = dir.path().join(".claude/skills/my-local-skill/SKILL.md");
        fs::create_dir_all(user_asset.parent().unwrap()).unwrap();
        fs::write(&user_asset, "user owned").unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::ClaudeCode]).unwrap();

        assert!(
            user_asset.exists(),
            "ClaudeCode distribution must not prune user-created non-gwt skill content"
        );
    }

    #[test]
    fn distribute_for_targets_preserves_user_created_non_gwt_codex_skill() {
        let dir = tempfile::tempdir().unwrap();
        let user_asset = dir.path().join(".codex/skills/my-local-skill/SKILL.md");
        fs::create_dir_all(user_asset.parent().unwrap()).unwrap();
        fs::write(&user_asset, "user owned").unwrap();

        distribute_to_worktree_for_targets(dir.path(), &[ManagedAssetTarget::Codex]).unwrap();

        assert!(
            user_asset.exists(),
            "Codex distribution must not prune user-created non-gwt skill content"
        );
    }

    #[test]
    fn distribute_materializes_gwt_verify_skill_and_command() {
        // SPEC-1935 Phase 17 (FR-122): gwt-verify must materialize to both
        // Claude Code and Codex targets, alongside its references and the
        // slash-command wrapper.
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();

        for relative in [
            ".claude/skills/gwt-verify/SKILL.md",
            ".claude/skills/gwt-verify/references/test-matrix.md",
            ".claude/skills/gwt-verify/references/playwright-runbook.md",
            ".claude/skills/gwt-verify/references/tooling-bootstrap.md",
            ".codex/skills/gwt-verify/SKILL.md",
            ".codex/skills/gwt-verify/references/test-matrix.md",
            ".codex/skills/gwt-verify/references/playwright-runbook.md",
            ".codex/skills/gwt-verify/references/tooling-bootstrap.md",
            ".claude/commands/gwt-verify.md",
        ] {
            let path = dir.path().join(relative);
            assert!(
                path.exists(),
                "gwt-verify materialization must produce {}",
                relative
            );
        }
    }

    #[test]
    fn distribute_creates_canonical_project_search_skills() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();

        let claude_skill = dir
            .path()
            .join(".claude/skills/gwt-project-search/SKILL.md");
        let codex_skill = dir.path().join(".codex/skills/gwt-project-search/SKILL.md");

        assert!(claude_skill.exists(), "expected {}", claude_skill.display());
        assert!(codex_skill.exists(), "expected {}", codex_skill.display());
    }

    #[test]
    fn distribute_does_not_create_retired_codex_hook_scripts() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        assert!(
            !dir.path().join(".codex/hooks").exists(),
            "unexpected stale .codex/hooks directory"
        );
    }

    #[test]
    fn distribute_creates_claude_commands() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let cmd = dir.path().join(".claude/commands/gwt-manage-pr.md");
        assert!(cmd.exists(), "expected {}", cmd.display());
    }

    #[test]
    fn distribute_creates_canonical_project_search_command() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();

        let command = dir.path().join(".claude/commands/gwt-project-search.md");
        assert!(command.exists(), "expected {}", command.display());

        let content = fs::read_to_string(&command).unwrap();
        assert!(content.contains(".claude/skills/gwt-project-search/SKILL.md"));
        assert!(!content.contains("gwt-file-search"));
    }

    #[test]
    fn distribute_does_not_create_file_search_assets() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();

        let command = dir.path().join(".claude/commands/gwt-file-search.md");
        let claude_skill = dir.path().join(".claude/skills/gwt-file-search/SKILL.md");
        let codex_skill = dir.path().join(".codex/skills/gwt-file-search/SKILL.md");

        assert!(!command.exists(), "unexpected {}", command.display());
        assert!(
            !claude_skill.exists(),
            "unexpected {}",
            claude_skill.display()
        );
        assert!(
            !codex_skill.exists(),
            "unexpected {}",
            codex_skill.display()
        );
    }

    #[test]
    fn distribute_removes_untracked_stale_gwt_assets() {
        let dir = tempfile::tempdir().unwrap();

        let stale_skill = dir.path().join(".codex/skills/gwt-agent-read");
        let stale_command = dir.path().join(".claude/commands/gwt-issue-search.md");
        let stale_hook = dir
            .path()
            .join(".claude/hooks/scripts/gwt-legacy-hook.retired");

        fs::create_dir_all(stale_skill.join("nested")).unwrap();
        fs::create_dir_all(stale_command.parent().unwrap()).unwrap();
        fs::create_dir_all(stale_hook.parent().unwrap()).unwrap();
        fs::write(stale_skill.join("nested/SKILL.md"), "legacy").unwrap();
        fs::write(&stale_command, "legacy command").unwrap();
        fs::write(&stale_hook, "legacy hook").unwrap();

        distribute_to_worktree(dir.path()).unwrap();

        assert!(
            !stale_skill.exists(),
            "unexpected {}",
            stale_skill.display()
        );
        assert!(
            !stale_command.exists(),
            "unexpected {}",
            stale_command.display()
        );
        assert!(!stale_hook.exists(), "unexpected {}", stale_hook.display());
    }

    // SPEC #1942 fix: tracked gwt-managed assets are now preserved by
    // prune_dir_against_source so that newly committed skills/commands
    // are not deleted before they appear in the next binary build's
    // include_dir! snapshot.
    #[test]
    fn distribute_preserves_tracked_stale_gwt_assets() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_command = dir.path().join(".claude/commands/gwt-issue-search.md");
        let tracked_hook = dir
            .path()
            .join(".codex/hooks/scripts/gwt-legacy-hook.retired");

        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_hook.parent().unwrap()).unwrap();
        fs::write(&tracked_command, "tracked command").unwrap();
        fs::write(&tracked_hook, "tracked hook").unwrap();

        track_path(dir.path(), ".claude/commands/gwt-issue-search.md");
        track_path(dir.path(), ".codex/hooks/scripts/gwt-legacy-hook.retired");

        distribute_to_worktree(dir.path()).unwrap();

        assert!(
            tracked_command.exists(),
            "tracked gwt command must be preserved: {}",
            tracked_command.display()
        );
        assert!(
            !tracked_hook.exists(),
            "retired codex gwt hook must be removed even when tracked: {}",
            tracked_hook.display()
        );
    }

    // Nested paths inside managed skill dirs: untracked stale files
    // are pruned, but tracked files are preserved.
    #[test]
    fn distribute_prunes_untracked_stale_nested_paths_but_preserves_tracked() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_skill = dir.path().join(".claude/skills/gwt-manage-pr/SKILL.md");
        let untracked_nested = dir
            .path()
            .join(".claude/skills/gwt-manage-pr/references/legacy.md");
        let untracked_codex_nested = dir.path().join(".codex/skills/gwt-manage-pr/legacy.txt");

        fs::create_dir_all(tracked_skill.parent().unwrap()).unwrap();
        fs::create_dir_all(untracked_nested.parent().unwrap()).unwrap();
        fs::create_dir_all(untracked_codex_nested.parent().unwrap()).unwrap();
        fs::write(&tracked_skill, "tracked skill").unwrap();
        fs::write(&untracked_nested, "legacy nested file").unwrap();
        fs::write(&untracked_codex_nested, "legacy codex file").unwrap();

        // Only track the SKILL.md, NOT the nested legacy files.
        track_path(dir.path(), ".claude/skills/gwt-manage-pr/SKILL.md");

        distribute_to_worktree(dir.path()).unwrap();

        assert_eq!(fs::read_to_string(&tracked_skill).unwrap(), "tracked skill");
        assert!(
            !untracked_nested.exists(),
            "untracked stale nested file should be pruned: {}",
            untracked_nested.display()
        );
        assert!(
            !untracked_codex_nested.exists(),
            "untracked stale codex nested file should be pruned: {}",
            untracked_codex_nested.display()
        );
    }

    #[test]
    fn prune_stale_gwt_assets_removes_extras_without_materializing_bundle() {
        let dir = tempfile::tempdir().unwrap();

        let stale_command = dir.path().join(".claude/commands/gwt-issue-search.md");
        let stale_skill = dir.path().join(".codex/skills/gwt-agent-read/SKILL.md");
        let stale_hook = dir
            .path()
            .join(".claude/hooks/scripts/gwt-legacy-hook.retired");

        fs::create_dir_all(stale_command.parent().unwrap()).unwrap();
        fs::create_dir_all(stale_skill.parent().unwrap()).unwrap();
        fs::create_dir_all(stale_hook.parent().unwrap()).unwrap();
        fs::write(&stale_command, "legacy command").unwrap();
        fs::write(&stale_skill, "legacy skill").unwrap();
        fs::write(&stale_hook, "legacy hook").unwrap();

        let removed = prune_stale_gwt_assets(dir.path()).unwrap();

        assert_eq!(removed, 5);
        assert!(
            !stale_command.exists(),
            "unexpected {}",
            stale_command.display()
        );
        assert!(
            !stale_skill.exists(),
            "unexpected {}",
            stale_skill.display()
        );
        assert!(!stale_hook.exists(), "unexpected {}", stale_hook.display());
        assert!(
            !dir.path().join(".claude/hooks").exists(),
            "unexpected stale .claude/hooks directory"
        );
        assert!(
            !dir.path()
                .join(".claude/skills/gwt-manage-pr/SKILL.md")
                .exists(),
            "prune-only sweep must not materialize bundle assets"
        );
    }

    #[test]
    fn prune_stale_gwt_assets_removes_empty_retired_hook_dirs() {
        let dir = tempfile::tempdir().unwrap();

        fs::create_dir_all(dir.path().join(".claude/hooks")).unwrap();
        fs::create_dir_all(dir.path().join(".codex/hooks")).unwrap();

        let removed = prune_stale_gwt_assets(dir.path()).unwrap();

        assert_eq!(removed, 2);
        assert!(
            !dir.path().join(".claude/hooks").exists(),
            "unexpected stale .claude/hooks directory"
        );
        assert!(
            !dir.path().join(".codex/hooks").exists(),
            "unexpected stale .codex/hooks directory"
        );
    }

    #[test]
    fn distribute_does_not_create_retired_claude_hook_scripts() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        assert!(
            !dir.path().join(".claude/hooks").exists(),
            "unexpected stale .claude/hooks directory"
        );
    }

    #[test]
    fn distribute_overwrites_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let skill_md = dir.path().join(".claude/skills/gwt-manage-pr/SKILL.md");
        fs::create_dir_all(skill_md.parent().unwrap()).unwrap();
        fs::write(&skill_md, "old content").unwrap();

        distribute_to_worktree(dir.path()).unwrap();

        let content = fs::read_to_string(&skill_md).unwrap();
        assert_ne!(content, "old content");
        assert!(content.contains("gwt-manage-pr"));
    }

    #[test]
    fn distribute_preserves_tracked_managed_assets() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_skill = dir.path().join(".claude/skills/gwt-manage-pr/SKILL.md");
        let tracked_command = dir.path().join(".claude/commands/gwt-manage-pr.md");
        let tracked_codex_skill = dir.path().join(".codex/skills/gwt-manage-pr/SKILL.md");

        fs::create_dir_all(tracked_skill.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_codex_skill.parent().unwrap()).unwrap();
        fs::write(&tracked_skill, "tracked skill").unwrap();
        fs::write(&tracked_command, "tracked command").unwrap();
        fs::write(&tracked_codex_skill, "tracked codex skill").unwrap();

        track_path(dir.path(), ".claude/skills/gwt-manage-pr/SKILL.md");
        track_path(dir.path(), ".claude/commands/gwt-manage-pr.md");
        track_path(dir.path(), ".codex/skills/gwt-manage-pr/SKILL.md");

        distribute_to_worktree(dir.path()).unwrap();

        assert_eq!(fs::read_to_string(&tracked_skill).unwrap(), "tracked skill");
        assert_eq!(
            fs::read_to_string(&tracked_command).unwrap(),
            "tracked command"
        );
        assert_eq!(
            fs::read_to_string(&tracked_codex_skill).unwrap(),
            "tracked codex skill"
        );
    }

    #[test]
    fn distribute_preserves_tracked_bundled_discussion_command() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_command = dir.path().join(".claude/commands/gwt-discussion.md");
        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::write(&tracked_command, "tracked discussion command").unwrap();

        track_path(dir.path(), ".claude/commands/gwt-discussion.md");

        distribute_to_worktree(dir.path()).unwrap();

        assert_eq!(
            fs::read_to_string(&tracked_command).unwrap(),
            "tracked discussion command"
        );
    }

    #[test]
    fn distribute_restores_missing_tracked_bundled_discussion_command() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_command = dir.path().join(".claude/commands/gwt-discussion.md");
        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::write(&tracked_command, "tracked discussion command").unwrap();

        track_path(dir.path(), ".claude/commands/gwt-discussion.md");
        fs::remove_file(&tracked_command).unwrap();

        distribute_to_worktree(dir.path()).unwrap();

        let content = fs::read_to_string(&tracked_command).unwrap();
        assert!(
            content.contains("gwt Discussion Command"),
            "missing tracked bundled command should be restored from the current bundle"
        );
    }

    #[test]
    fn root_level_protection_skips_nested_assets() {
        let worktree = Path::new("/tmp/repo");
        let protected_roots = HashSet::from([
            PathBuf::from(".claude/skills"),
            PathBuf::from(".codex/skills"),
        ]);

        assert!(should_skip_tracked_path(
            worktree,
            &worktree.join(".claude/skills/gwt-manage-pr/SKILL.md"),
            &protected_roots,
        ));
        assert!(should_skip_tracked_path(
            worktree,
            &worktree.join(".codex/skills/gwt-agent/SKILL.md"),
            &protected_roots,
        ));
    }

    fn init_git_repo(worktree: &Path) {
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg(worktree)
            .status()
            .unwrap()
            .success());
    }

    fn track_path(worktree: &Path, relative_path: &str) {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(worktree)
            .arg("add")
            .arg(relative_path)
            .status()
            .unwrap()
            .success());
    }
}

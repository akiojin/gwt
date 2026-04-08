//! Distribute bundled skill assets to a target worktree.

use crate::assets::{CLAUDE_COMMANDS, CLAUDE_HOOKS, CLAUDE_SKILLS, CODEX_HOOKS};
use include_dir::Dir;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const TRACKED_ROOTS: &[&str] = &[
    ".claude/skills",
    ".claude/commands",
    ".claude/hooks/scripts",
    ".codex/skills",
    ".codex/hooks/scripts",
];

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

/// Write all bundled skill, command, and hook files to the target worktree.
///
/// Distribution targets:
/// - `.claude/skills/gwt-*/`
/// - `.claude/commands/gwt-*.md`
/// - `.claude/hooks/scripts/gwt-*.mjs`
/// - `.codex/skills/gwt-*/`  (same skill content)
/// - `.codex/hooks/scripts/gwt-*.mjs`
pub fn distribute_to_worktree(worktree: &Path) -> io::Result<DistributeReport> {
    let mut report = DistributeReport::default();
    let tracked_paths = tracked_gwt_asset_paths(worktree);

    // Claude Code targets
    sync_dir_assets(
        &CLAUDE_SKILLS,
        worktree,
        &worktree.join(".claude/skills"),
        &tracked_paths,
        &mut report,
        RootEntryKind::Directories,
    )?;
    sync_dir_assets(
        &CLAUDE_COMMANDS,
        worktree,
        &worktree.join(".claude/commands"),
        &tracked_paths,
        &mut report,
        RootEntryKind::Files,
    )?;
    sync_dir_assets(
        &CLAUDE_HOOKS,
        worktree,
        &worktree.join(".claude/hooks/scripts"),
        &tracked_paths,
        &mut report,
        RootEntryKind::Files,
    )?;

    // Codex targets
    sync_dir_assets(
        &CLAUDE_SKILLS,
        worktree,
        &worktree.join(".codex/skills"),
        &tracked_paths,
        &mut report,
        RootEntryKind::Directories,
    )?;
    sync_dir_assets(
        &CODEX_HOOKS,
        worktree,
        &worktree.join(".codex/hooks/scripts"),
        &tracked_paths,
        &mut report,
        RootEntryKind::Files,
    )?;

    Ok(report)
}

fn sync_dir_assets(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    tracked_paths: &HashSet<PathBuf>,
    report: &mut DistributeReport,
    root_kind: RootEntryKind,
) -> io::Result<()> {
    let desired_entries = desired_gwt_root_entries(source, root_kind);
    remove_stale_gwt_root_entries(dest, &desired_entries, report)?;
    write_dir_assets(source, worktree, dest, tracked_paths, report)
}

fn write_dir_assets(
    source: &Dir<'_>,
    worktree: &Path,
    dest: &Path,
    tracked_paths: &HashSet<PathBuf>,
    report: &mut DistributeReport,
) -> io::Result<()> {
    for file in source.files() {
        let target = dest.join(file.path().file_name().unwrap_or_default());
        if should_skip_tracked_path(worktree, &target, tracked_paths) {
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

fn desired_gwt_root_entries(source: &Dir<'_>, root_kind: RootEntryKind) -> HashSet<String> {
    match root_kind {
        RootEntryKind::Directories => source
            .dirs()
            .filter_map(|dir| dir.path().file_name().and_then(|name| name.to_str()))
            .filter(|name| name.starts_with("gwt-"))
            .map(str::to_string)
            .collect(),
        RootEntryKind::Files => source
            .files()
            .filter_map(|file| file.path().file_name().and_then(|name| name.to_str()))
            .filter(|name| name.starts_with("gwt-"))
            .map(str::to_string)
            .collect(),
    }
}

fn remove_stale_gwt_root_entries(
    dest: &Path,
    desired_entries: &HashSet<String>,
    report: &mut DistributeReport,
) -> io::Result<()> {
    if !dest.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("gwt-") || desired_entries.contains(name.as_ref()) {
            continue;
        }

        remove_path(&entry.path())?;
        report.paths_removed += 1;
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
    match Command::new("git")
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
    match Command::new("git")
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
        let skill_md = dir.path().join(".claude/skills/gwt-pr/SKILL.md");
        assert!(skill_md.exists(), "expected {}", skill_md.display());
    }

    #[test]
    fn distribute_creates_codex_skills() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let skill_md = dir.path().join(".codex/skills/gwt-pr/SKILL.md");
        assert!(skill_md.exists(), "expected {}", skill_md.display());
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
    fn distribute_creates_codex_hooks() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let hook = dir.path().join(".codex/hooks/scripts/gwt-forward-hook.mjs");
        assert!(hook.exists(), "expected {}", hook.display());
    }

    #[test]
    fn distribute_creates_claude_commands() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let cmd = dir.path().join(".claude/commands/gwt-pr.md");
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
        let stale_hook = dir.path().join(".claude/hooks/scripts/gwt-legacy-hook.mjs");

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

    #[test]
    fn distribute_removes_tracked_stale_gwt_assets() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_command = dir.path().join(".claude/commands/gwt-issue-search.md");
        let tracked_hook = dir.path().join(".codex/hooks/scripts/gwt-legacy-hook.mjs");

        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_hook.parent().unwrap()).unwrap();
        fs::write(&tracked_command, "tracked command").unwrap();
        fs::write(&tracked_hook, "tracked hook").unwrap();

        track_path(dir.path(), ".claude/commands/gwt-issue-search.md");
        track_path(dir.path(), ".codex/hooks/scripts/gwt-legacy-hook.mjs");

        distribute_to_worktree(dir.path()).unwrap();

        assert!(
            !tracked_command.exists(),
            "unexpected {}",
            tracked_command.display()
        );
        assert!(
            !tracked_hook.exists(),
            "unexpected {}",
            tracked_hook.display()
        );
    }

    #[test]
    fn distribute_creates_claude_hooks() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let hook = dir
            .path()
            .join(".claude/hooks/scripts/gwt-forward-hook.mjs");
        assert!(hook.exists(), "expected {}", hook.display());
    }

    #[test]
    fn distribute_overwrites_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let skill_md = dir.path().join(".claude/skills/gwt-pr/SKILL.md");
        fs::create_dir_all(skill_md.parent().unwrap()).unwrap();
        fs::write(&skill_md, "old content").unwrap();

        distribute_to_worktree(dir.path()).unwrap();

        let content = fs::read_to_string(&skill_md).unwrap();
        assert_ne!(content, "old content");
        assert!(content.contains("gwt-pr"));
    }

    #[test]
    fn distribute_preserves_tracked_managed_assets() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let tracked_skill = dir.path().join(".claude/skills/gwt-pr/SKILL.md");
        let tracked_command = dir.path().join(".claude/commands/gwt-pr.md");
        let tracked_hook = dir
            .path()
            .join(".claude/hooks/scripts/gwt-forward-hook.mjs");
        let tracked_codex_skill = dir.path().join(".codex/skills/gwt-pr/SKILL.md");

        fs::create_dir_all(tracked_skill.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_command.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_hook.parent().unwrap()).unwrap();
        fs::create_dir_all(tracked_codex_skill.parent().unwrap()).unwrap();
        fs::write(&tracked_skill, "tracked skill").unwrap();
        fs::write(&tracked_command, "tracked command").unwrap();
        fs::write(&tracked_hook, "tracked hook").unwrap();
        fs::write(&tracked_codex_skill, "tracked codex skill").unwrap();

        track_path(dir.path(), ".claude/skills/gwt-pr/SKILL.md");
        track_path(dir.path(), ".claude/commands/gwt-pr.md");
        track_path(dir.path(), ".claude/hooks/scripts/gwt-forward-hook.mjs");
        track_path(dir.path(), ".codex/skills/gwt-pr/SKILL.md");

        distribute_to_worktree(dir.path()).unwrap();

        assert_eq!(fs::read_to_string(&tracked_skill).unwrap(), "tracked skill");
        assert_eq!(
            fs::read_to_string(&tracked_command).unwrap(),
            "tracked command"
        );
        assert_eq!(fs::read_to_string(&tracked_hook).unwrap(), "tracked hook");
        assert_eq!(
            fs::read_to_string(&tracked_codex_skill).unwrap(),
            "tracked codex skill"
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
            &worktree.join(".claude/skills/gwt-pr/SKILL.md"),
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

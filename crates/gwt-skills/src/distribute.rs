//! Distribute bundled skill assets to a target worktree.

use crate::assets::{CLAUDE_COMMANDS, CLAUDE_HOOKS, CLAUDE_SKILLS};
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
];

/// Result of a distribution operation.
#[derive(Debug, Default)]
pub struct DistributeReport {
    /// Number of files written.
    pub files_written: usize,
    /// Number of directories created.
    pub dirs_created: usize,
}

/// Write all bundled skill, command, and hook files to the target worktree.
///
/// Distribution targets:
/// - `.claude/skills/gwt-*/`
/// - `.claude/commands/gwt-*.md`
/// - `.claude/hooks/scripts/gwt-*.mjs`
/// - `.codex/skills/gwt-*/`  (same skill content)
pub fn distribute_to_worktree(worktree: &Path) -> io::Result<DistributeReport> {
    let mut report = DistributeReport::default();
    let tracked_paths = tracked_gwt_asset_paths(worktree);

    // Claude Code targets
    write_dir_assets(
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
    write_dir_assets(
        &CLAUDE_HOOKS,
        worktree,
        &worktree.join(".claude/hooks/scripts"),
        &tracked_paths,
        &mut report,
    )?;

    // Codex targets (skills only)
    write_dir_assets(
        &CLAUDE_SKILLS,
        worktree,
        &worktree.join(".codex/skills"),
        &tracked_paths,
        &mut report,
    )?;

    Ok(report)
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
    fn distribute_creates_claude_commands() {
        let dir = tempfile::tempdir().unwrap();
        distribute_to_worktree(dir.path()).unwrap();
        let cmd = dir.path().join(".claude/commands/gwt-pr.md");
        assert!(cmd.exists(), "expected {}", cmd.display());
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

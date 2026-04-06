//! Distribute bundled skill assets to a target worktree.

use crate::assets::{CLAUDE_COMMANDS, CLAUDE_HOOKS, CLAUDE_SKILLS, CODEX_HOOKS};
use include_dir::Dir;
use std::fs;
use std::io;
use std::path::Path;

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
/// - `.codex/hooks/scripts/gwt-*.mjs`
pub fn distribute_to_worktree(worktree: &Path) -> io::Result<DistributeReport> {
    let mut report = DistributeReport::default();

    // Claude Code targets
    write_dir_assets(
        &CLAUDE_SKILLS,
        &worktree.join(".claude/skills"),
        &mut report,
    )?;
    write_dir_assets(
        &CLAUDE_COMMANDS,
        &worktree.join(".claude/commands"),
        &mut report,
    )?;
    write_dir_assets(
        &CLAUDE_HOOKS,
        &worktree.join(".claude/hooks/scripts"),
        &mut report,
    )?;

    // Codex targets (skills only)
    write_dir_assets(&CLAUDE_SKILLS, &worktree.join(".codex/skills"), &mut report)?;
    write_dir_assets(
        &CODEX_HOOKS,
        &worktree.join(".codex/hooks/scripts"),
        &mut report,
    )?;

    Ok(report)
}

fn write_dir_assets(
    source: &Dir<'_>,
    dest: &Path,
    report: &mut DistributeReport,
) -> io::Result<()> {
    for file in source.files() {
        let target = dest.join(file.path().file_name().unwrap_or_default());
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
        write_dir_assets(subdir, &dest.join(subdir_name), report)?;
    }

    Ok(())
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
}

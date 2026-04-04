//! Distribute bundled skill assets to a target worktree.

use crate::assets::{CLAUDE_COMMANDS, CLAUDE_HOOKS, CLAUDE_SKILLS};
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
/// - `.agents/skills/gwt-*/` (same skill content)
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
    write_dir_assets(
        &CLAUDE_SKILLS,
        &worktree.join(".codex/skills"),
        &mut report,
    )?;

    // Agent Skills standard (skills only)
    write_dir_assets(
        &CLAUDE_SKILLS,
        &worktree.join(".agents/skills"),
        &mut report,
    )?;

    Ok(report)
}

fn write_dir_assets(source: &Dir<'_>, dest: &Path, report: &mut DistributeReport) -> io::Result<()> {
    for file in source.files() {
        let target = dest.join(file.path());
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
                report.dirs_created += 1;
            }
        }
        fs::write(&target, file.contents())?;
        report.files_written += 1;
    }

    for dir in source.dirs() {
        write_dir_assets(dir, &dest.join(dir.path().file_name().unwrap_or_default()), report)?;
    }

    Ok(())
}

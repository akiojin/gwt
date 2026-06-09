//! Manage `.git/info/exclude` entries for gwt-distributed assets.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_core::process::hidden_command;

use crate::distribute::ManagedAssetTarget;

const BEGIN_MARKER: &str = "# gwt-managed-begin";
const END_MARKER: &str = "# gwt-managed-end";
const LEGACY_BEGIN_MARKER: &str = "# BEGIN gwt managed local assets";
const LEGACY_END_MARKER: &str = "# END gwt managed local assets";

/// Update `.git/info/exclude` to include gwt-managed asset exclusions.
///
/// Preserves existing user entries. gwt-managed entries are delimited by
/// `# gwt-managed-begin` / `# gwt-managed-end` markers and replaced on
/// each call.
pub fn update_git_exclude(worktree: &Path) -> io::Result<()> {
    write_git_exclude(worktree, replace_managed_block)
}

pub fn update_git_exclude_for_targets(
    worktree: &Path,
    targets: &[ManagedAssetTarget],
) -> io::Result<()> {
    write_git_exclude(worktree, |existing| {
        replace_managed_block_for_targets(existing, targets)
    })
}

fn write_git_exclude(
    worktree: &Path,
    replace: impl FnOnce(&str) -> io::Result<String>,
) -> io::Result<()> {
    let exclude_path = resolve_git_exclude_path(worktree)?;

    // Create parent directory if needed
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = if exclude_path.exists() {
        fs::read_to_string(&exclude_path)?
    } else {
        String::new()
    };

    let updated = replace(&existing)?;
    fs::write(&exclude_path, updated)?;

    Ok(())
}

fn resolve_git_exclude_path(worktree: &Path) -> io::Result<PathBuf> {
    let output = hidden_command("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "failed to resolve git exclude path: {}",
            stderr.trim()
        )));
    }

    let resolved = String::from_utf8_lossy(&output.stdout);
    let resolved = resolved.trim();
    if resolved.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "git rev-parse --git-path info/exclude returned an empty path",
        ));
    }

    let path = PathBuf::from(resolved);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(worktree.join(path))
    }
}

fn replace_managed_block(content: &str) -> io::Result<String> {
    let mut patterns = exclude_patterns_for_targets(&ManagedAssetTarget::ALL);
    push_unique(&mut patterns, "docker-compose.override.yml");
    replace_managed_block_with_patterns(content, &patterns)
}

fn replace_managed_block_for_targets(
    content: &str,
    targets: &[ManagedAssetTarget],
) -> io::Result<String> {
    let patterns = exclude_patterns_for_targets(targets);
    replace_managed_block_with_patterns(content, &patterns)
}

fn replace_managed_block_with_patterns(
    content: &str,
    patterns: &[&'static str],
) -> io::Result<String> {
    let mut result = String::new();
    let mut in_managed_block = false;
    let mut in_legacy_managed_block = false;

    for line in content.lines() {
        if line.trim() == BEGIN_MARKER {
            if in_managed_block {
                return Err(malformed_marker_error(
                    "nested begin marker in gwt-managed exclude block",
                ));
            }
            in_managed_block = true;
            continue;
        }
        if line.trim() == LEGACY_BEGIN_MARKER {
            if in_legacy_managed_block {
                return Err(malformed_marker_error(
                    "nested begin marker in legacy gwt-managed exclude block",
                ));
            }
            in_legacy_managed_block = true;
            continue;
        }
        if line.trim() == END_MARKER {
            if !in_managed_block {
                return Err(malformed_marker_error(
                    "end marker without matching begin marker in gwt-managed exclude block",
                ));
            }
            in_managed_block = false;
            continue;
        }
        if line.trim() == LEGACY_END_MARKER {
            if !in_legacy_managed_block {
                return Err(malformed_marker_error(
                    "legacy end marker without matching begin marker in gwt-managed exclude block",
                ));
            }
            in_legacy_managed_block = false;
            continue;
        }
        if !in_managed_block && !in_legacy_managed_block {
            result.push_str(line);
            result.push('\n');
        }
    }

    if in_managed_block {
        return Err(malformed_marker_error(
            "unterminated gwt-managed exclude block",
        ));
    }
    if in_legacy_managed_block {
        return Err(malformed_marker_error(
            "unterminated legacy gwt-managed exclude block",
        ));
    }

    // Remove trailing blank lines before appending managed block
    let trimmed = result.trim_end();
    let mut final_content = if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    };

    if !patterns.is_empty() {
        // Append managed block
        final_content.push('\n');
        final_content.push_str(BEGIN_MARKER);
        final_content.push('\n');
        for pattern in patterns {
            final_content.push_str(pattern);
            final_content.push('\n');
        }
        final_content.push_str(END_MARKER);
        final_content.push('\n');
    }

    Ok(final_content)
}

fn exclude_patterns_for_targets(targets: &[ManagedAssetTarget]) -> Vec<&'static str> {
    let mut patterns = Vec::new();
    if targets.is_empty() {
        return patterns;
    }
    // project-local `.gwt/` holds gwt-managed files (project.toml,
    // discussion.md, agent homes). Exclude its contents broadly, but carve out
    // `.gwt/work/` so the tracked Work persistent core (`.gwt/work/events.jsonl`,
    // `.gwt/work/memory.md`, `.gwt/work/discussions.md`) is committed with the
    // repo (SPEC-2359 Phase W-12). `.gwt/*` excludes the children without
    // excluding the `.gwt/` directory itself, so the later `!.gwt/work/`
    // negation can re-include the Work directory (re-inclusion is impossible
    // only when a parent directory is excluded).
    push_unique(&mut patterns, ".gwt/*");
    push_unique(&mut patterns, "!.gwt/work/");
    if targets.contains(&ManagedAssetTarget::ClaudeCode) {
        push_unique(&mut patterns, ".claude/skills/gwt-*");
        push_unique(&mut patterns, ".claude/commands/gwt-*");
        push_unique(&mut patterns, ".claude/settings.local.json");
    }
    if targets.contains(&ManagedAssetTarget::Codex) {
        push_unique(&mut patterns, ".codex/skills/gwt-*");
    }
    // OpenCode / OpenClaw / Hermes need no additional pattern: their agent
    // homes (`.gwt/opencode/`, `.gwt/openclaw/`, `.gwt/hermes/`) are
    // subsumed by the broad `.gwt/` pattern above.
    patterns
}

fn push_unique(patterns: &mut Vec<&'static str>, pattern: &'static str) {
    if !patterns.contains(&pattern) {
        patterns.push(pattern);
    }
}

fn malformed_marker_error(detail: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("malformed gwt-managed exclude markers: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_managed_block_to_empty_file() {
        let result = replace_managed_block("").unwrap();
        assert!(result.contains(BEGIN_MARKER));
        assert!(result.contains(END_MARKER));
        assert!(result.contains(".claude/skills/gwt-*"));
        assert!(result.contains(".codex/skills/gwt-*"));
        assert!(result.contains("\n.gwt/*\n"));
        assert!(result.contains("\n!.gwt/work/\n"));
        assert!(result.contains("docker-compose.override.yml"));
        assert!(!result.contains(".gwt/discussion.md"));
        assert!(!result.contains(".gwt/opencode/"));
        assert!(!result.contains(".gwt/openclaw/"));
        assert!(!result.contains(".gwt/hermes/"));
        assert!(!result.contains(".codex/hooks.json"));
        assert!(!result.contains(".codex/hooks/scripts/gwt-*"));
        assert!(!result.contains(".agents/skills/gwt-*"));
    }

    #[test]
    fn adds_only_requested_provider_patterns() {
        let result = replace_managed_block_for_targets("", &[ManagedAssetTarget::Hermes]).unwrap();

        assert!(result.contains(BEGIN_MARKER));
        assert!(result.contains("\n.gwt/*\n"));
        assert!(result.contains("\n!.gwt/work/\n"));
        assert!(!result.contains(".claude/skills/gwt-*"));
        assert!(!result.contains(".codex/skills/gwt-*"));
        assert!(!result.contains(".gwt/discussion.md"));
        assert!(!result.contains(".gwt/hermes/"));
        assert!(!result.contains(".gwt/opencode/"));
        assert!(!result.contains(".gwt/openclaw/"));
    }

    #[test]
    fn excludes_gwt_contents_but_carves_out_tracked_work_dir() {
        let result = replace_managed_block("").unwrap();
        assert!(
            result.contains("\n.gwt/*\n"),
            "managed block should exclude `.gwt/` contents via `.gwt/*`: {result}"
        );
        assert!(
            result.contains("\n!.gwt/work/\n"),
            "managed block should carve out the tracked `.gwt/work/` dir: {result}"
        );
        // The negation must follow the broad exclusion so the Work dir is
        // re-included (gitignore evaluates patterns top-to-bottom).
        let exclude_idx = result.find("\n.gwt/*\n").unwrap();
        let carveout_idx = result.find("\n!.gwt/work/\n").unwrap();
        assert!(
            exclude_idx < carveout_idx,
            "`.gwt/*` must precede `!.gwt/work/` so the carve-out re-includes: {result}"
        );
    }

    #[test]
    fn does_not_emit_individual_gwt_subpaths_when_broad_pattern_included() {
        let result = replace_managed_block("").unwrap();
        assert!(
            !result.contains(".gwt/discussion.md"),
            "individual .gwt/discussion.md must be subsumed by broad .gwt/ pattern: {result}"
        );
        assert!(
            !result.contains(".gwt/hermes/"),
            "individual .gwt/hermes/ must be subsumed by broad .gwt/ pattern: {result}"
        );
        assert!(
            !result.contains(".gwt/opencode/"),
            "individual .gwt/opencode/ must be subsumed by broad .gwt/ pattern: {result}"
        );
        assert!(
            !result.contains(".gwt/openclaw/"),
            "individual .gwt/openclaw/ must be subsumed by broad .gwt/ pattern: {result}"
        );
    }

    #[test]
    fn broad_gwt_dir_pattern_present_for_single_provider_target() {
        let result = replace_managed_block_for_targets("", &[ManagedAssetTarget::Codex]).unwrap();
        assert!(
            result.contains("\n.gwt/*\n") && result.contains("\n!.gwt/work/\n"),
            "single-target managed block should exclude `.gwt/` contents but carve out `.gwt/work/`: {result}"
        );
        assert!(
            !result.contains(".gwt/discussion.md"),
            "single-target managed block should not emit .gwt/discussion.md separately: {result}"
        );
    }

    #[test]
    fn replaces_legacy_broad_managed_block_with_scoped_patterns() {
        let existing = "\
user-entry
# BEGIN gwt managed local assets
/.gwt/
/.codex/skills/gwt-*/
/.claude/skills/gwt-*/
# END gwt managed local assets
";

        let result =
            replace_managed_block_for_targets(existing, &[ManagedAssetTarget::Codex]).unwrap();

        assert!(result.contains("user-entry"));
        assert!(result.contains(".codex/skills/gwt-*"));
        assert!(!result.contains("/.gwt/"));
        assert!(!result.contains(".claude/skills/gwt-*"));
        assert!(!result.contains("# BEGIN gwt managed local assets"));
    }

    #[test]
    fn preserves_user_entries() {
        let existing = "my-custom-pattern\nanother-pattern\n";
        let result = replace_managed_block(existing).unwrap();
        assert!(result.contains("my-custom-pattern"));
        assert!(result.contains("another-pattern"));
        assert!(result.contains(BEGIN_MARKER));
    }

    #[test]
    fn replaces_existing_managed_block() {
        let existing = format!(
            "user-entry\n{}\nold-gwt-pattern\n{}\nuser-entry-2\n",
            BEGIN_MARKER, END_MARKER
        );
        let result = replace_managed_block(&existing).unwrap();
        assert!(!result.contains("old-gwt-pattern"));
        assert!(result.contains("user-entry"));
        assert!(result.contains("user-entry-2"));
        assert!(result.contains(".claude/skills/gwt-*"));
    }

    #[test]
    fn update_git_exclude_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        init_git_repo(worktree);

        update_git_exclude(worktree).unwrap();

        let content = fs::read_to_string(git_resolved_exclude_path(worktree)).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains(".claude/skills/gwt-*"));
        assert!(content.contains("docker-compose.override.yml"));
    }

    #[test]
    fn update_git_exclude_updates_git_resolved_path_for_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        git_commit_allow_empty(&repo, "initial commit");

        let worktree = dir.path().join("wt-linked");
        git_add_worktree(&repo, &worktree, "feature/linked");
        assert!(
            worktree.join(".git").is_file(),
            "linked worktree should have a .git file"
        );

        update_git_exclude(&worktree).unwrap();

        let exclude_path = git_resolved_exclude_path(&worktree);
        let content = fs::read_to_string(&exclude_path).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains("docker-compose.override.yml"));
        assert!(!content.contains(".codex/hooks.json"));
        assert!(
            !worktree.join(".git/info/exclude").exists(),
            "linked worktree should not create a nested path under the .git file"
        );
    }

    #[test]
    fn update_git_exclude_returns_error_without_modifying_file_when_markers_are_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        init_git_repo(worktree);

        let exclude_path = git_resolved_exclude_path(worktree);
        fs::create_dir_all(exclude_path.parent().unwrap()).unwrap();
        let malformed = format!("user-entry\n{BEGIN_MARKER}\nstale-pattern\n");
        fs::write(&exclude_path, &malformed).unwrap();

        let error = update_git_exclude(worktree).expect_err("malformed managed block should fail");
        assert_eq!(
            fs::read_to_string(&exclude_path).unwrap(),
            malformed,
            "malformed managed block should be left untouched"
        );
        assert!(
            error.to_string().contains("malformed"),
            "error should explain the malformed marker state: {error}"
        );
    }

    fn init_git_repo(path: &Path) {
        let init = std::process::Command::new("git")
            .arg("init")
            .arg(path)
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed: {:?}", init);

        let email = std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(email.status.success(), "git config user.email failed");

        let name = std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(name.status.success(), "git config user.name failed");
    }

    fn git_commit_allow_empty(path: &Path, message: &str) {
        let output = std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_add_worktree(repo: &Path, worktree: &Path, branch: &str) {
        let output = std::process::Command::new("git")
            .args(["worktree", "add", "-b", branch, worktree.to_str().unwrap()])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_resolved_exclude_path(worktree: &Path) -> std::path::PathBuf {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--git-path", "info/exclude"])
            .current_dir(worktree)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git rev-parse --git-path info/exclude failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let resolved = String::from_utf8(output.stdout).unwrap();
        let path = std::path::PathBuf::from(resolved.trim());
        if path.is_absolute() {
            path
        } else {
            worktree.join(path)
        }
    }
}

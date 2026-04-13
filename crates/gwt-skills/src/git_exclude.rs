//! Manage `.git/info/exclude` entries for gwt-distributed assets.

use std::fs;
use std::io;
use std::path::Path;

const BEGIN_MARKER: &str = "# gwt-managed-begin";
const END_MARKER: &str = "# gwt-managed-end";

/// Patterns to exclude gwt-managed assets from git tracking.
const GWT_EXCLUDE_PATTERNS: &[&str] = &[
    ".claude/skills/gwt-*",
    ".claude/commands/gwt-*",
    ".claude/settings.local.json",
    ".codex/skills/gwt-*",
    ".codex/hooks.json",
];

/// Update `.git/info/exclude` to include gwt-managed asset exclusions.
///
/// Preserves existing user entries. gwt-managed entries are delimited by
/// `# gwt-managed-begin` / `# gwt-managed-end` markers and replaced on
/// each call.
pub fn update_git_exclude(worktree: &Path) -> io::Result<()> {
    let exclude_path = worktree.join(".git/info/exclude");

    // Create parent directory if needed
    if let Some(parent) = exclude_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = if exclude_path.exists() {
        fs::read_to_string(&exclude_path)?
    } else {
        String::new()
    };

    let updated = replace_managed_block(&existing);
    fs::write(&exclude_path, updated)?;

    Ok(())
}

fn replace_managed_block(content: &str) -> String {
    let mut result = String::new();
    let mut in_managed_block = false;

    for line in content.lines() {
        if line.trim() == BEGIN_MARKER {
            in_managed_block = true;
            continue;
        }
        if line.trim() == END_MARKER {
            in_managed_block = false;
            continue;
        }
        if !in_managed_block {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing blank lines before appending managed block
    let trimmed = result.trim_end();
    let mut final_content = if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    };

    // Append managed block
    final_content.push('\n');
    final_content.push_str(BEGIN_MARKER);
    final_content.push('\n');
    for pattern in GWT_EXCLUDE_PATTERNS {
        final_content.push_str(pattern);
        final_content.push('\n');
    }
    final_content.push_str(END_MARKER);
    final_content.push('\n');

    final_content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_managed_block_to_empty_file() {
        let result = replace_managed_block("");
        assert!(result.contains(BEGIN_MARKER));
        assert!(result.contains(END_MARKER));
        assert!(result.contains(".claude/skills/gwt-*"));
        assert!(result.contains(".codex/skills/gwt-*"));
        assert!(result.contains(".codex/hooks.json"));
        assert!(!result.contains(".codex/hooks/scripts/gwt-*"));
        assert!(!result.contains(".agents/skills/gwt-*"));
    }

    #[test]
    fn preserves_user_entries() {
        let existing = "my-custom-pattern\nanother-pattern\n";
        let result = replace_managed_block(existing);
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
        let result = replace_managed_block(&existing);
        assert!(!result.contains("old-gwt-pattern"));
        assert!(result.contains("user-entry"));
        assert!(result.contains("user-entry-2"));
        assert!(result.contains(".claude/skills/gwt-*"));
    }

    #[test]
    fn update_git_exclude_creates_file_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path();
        // Create .git/info directory structure
        fs::create_dir_all(worktree.join(".git/info")).unwrap();

        update_git_exclude(worktree).unwrap();

        let content = fs::read_to_string(worktree.join(".git/info/exclude")).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains(".claude/skills/gwt-*"));
    }
}

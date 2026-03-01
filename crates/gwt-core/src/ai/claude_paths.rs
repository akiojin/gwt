//! Claude Code project path helpers.
//!
//! Claude Code stores sessions under:
//! `~/.claude/projects/{encoded-worktree-path}/{session-id}.jsonl`
//!
//! The encoding logic must match ClaudeEncoder output to ensure session discovery
//! and conversion remain consistent.

use std::path::Path;

/// Encode a worktree path for Claude Code project directory name.
///
/// Keep in sync with the encoder used for session conversion.
pub(crate) fn encode_claude_project_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();
    path_str
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn encode_claude_project_path_replaces_separators() {
        let path = PathBuf::from("/home/user/projects/my-app");
        let encoded = encode_claude_project_path(&path);
        assert!(!encoded.contains('/'));
        assert!(!encoded.is_empty());
    }

    #[test]
    fn encode_preserves_alphanumeric() {
        let path = PathBuf::from("/abc123");
        let encoded = encode_claude_project_path(&path);
        assert!(encoded.contains("abc123"));
    }

    #[test]
    fn encode_preserves_dots_and_underscores() {
        let path = PathBuf::from("my_project.v2");
        let encoded = encode_claude_project_path(&path);
        assert!(encoded.contains("my_project.v2"));
    }

    #[test]
    fn encode_replaces_spaces() {
        let path = PathBuf::from("/home/user/my project");
        let encoded = encode_claude_project_path(&path);
        assert!(!encoded.contains(' '));
    }

    #[test]
    fn encode_replaces_special_chars() {
        let path = PathBuf::from("/path/with@special#chars");
        let encoded = encode_claude_project_path(&path);
        assert!(!encoded.contains('@'));
        assert!(!encoded.contains('#'));
    }

    #[test]
    fn encode_same_path_gives_same_result() {
        let path = PathBuf::from("/repo/worktrees/feature-x");
        let a = encode_claude_project_path(&path);
        let b = encode_claude_project_path(&path);
        assert_eq!(a, b);
    }
}

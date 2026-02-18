//! Worktree references for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::types::TaskId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRef {
    pub branch_name: String,
    pub path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub task_ids: Vec<TaskId>,
}

impl WorktreeRef {
    pub fn new(branch_name: impl Into<String>, path: PathBuf, task_ids: Vec<TaskId>) -> Self {
        Self {
            branch_name: branch_name.into(),
            path,
            created_at: Utc::now(),
            task_ids,
        }
    }
}

/// Sanitize a string into a valid git branch name component.
///
/// Rules:
/// - Lowercase
/// - Spaces/underscores to hyphens
/// - Remove non-alphanumeric/non-hyphen characters
/// - Collapse multiple hyphens
/// - Trim leading/trailing hyphens
/// - Truncate to 64 characters
pub fn sanitize_branch_name(name: &str) -> String {
    let sanitized: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();

    // Collapse multiple hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Trim leading/trailing hyphens
    let trimmed = result.trim_matches('-');

    // Truncate to 64 characters
    if trimmed.len() > 64 {
        trimmed[..64].trim_end_matches('-').to_string()
    } else {
        trimmed.to_string()
    }
}

/// Create a branch name for an agent worktree with `agent/` prefix.
///
/// If a branch with the same name already exists, appends a numeric suffix.
pub fn create_agent_branch_name(task_name: &str, existing_branches: &[String]) -> String {
    let sanitized = sanitize_branch_name(task_name);
    let base = format!("agent/{}", sanitized);

    if !existing_branches.contains(&base) {
        return base;
    }

    // Try numbered suffixes
    for i in 2..=99 {
        let candidate = format!("{}-{}", base, i);
        if !existing_branches.contains(&candidate) {
            return candidate;
        }
    }

    // Fallback with UUID fragment
    format!("{}-{}", base, &uuid::Uuid::new_v4().to_string()[..8])
}

/// Create a worktree path under `.worktrees/` in the repository root.
pub fn worktree_path(repo_root: &std::path::Path, branch_name: &str) -> PathBuf {
    let dir_name = branch_name.replace('/', "-");
    repo_root.join(".worktrees").join(dir_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worktree_ref_new() {
        let task_id = TaskId("task-1".to_string());
        let wt = WorktreeRef::new("agent/test", PathBuf::from("/tmp/wt"), vec![task_id]);
        assert_eq!(wt.branch_name, "agent/test");
        assert_eq!(wt.task_ids.len(), 1);
    }

    #[test]
    fn test_sanitize_branch_name_basic() {
        assert_eq!(
            sanitize_branch_name("Add Login Feature"),
            "add-login-feature"
        );
    }

    #[test]
    fn test_sanitize_branch_name_special_chars() {
        assert_eq!(sanitize_branch_name("fix: bug #123!"), "fix-bug-123");
    }

    #[test]
    fn test_sanitize_branch_name_unicode() {
        assert_eq!(sanitize_branch_name("日本語テスト"), "");
    }

    #[test]
    fn test_sanitize_branch_name_long() {
        let long_name = "a".repeat(100);
        let result = sanitize_branch_name(&long_name);
        assert!(result.len() <= 64);
    }

    #[test]
    fn test_sanitize_branch_name_collapse_hyphens() {
        assert_eq!(sanitize_branch_name("a--b---c"), "a-b-c");
    }

    #[test]
    fn test_create_agent_branch_name_no_conflict() {
        let existing: Vec<String> = vec![];
        let result = create_agent_branch_name("add feature", &existing);
        assert_eq!(result, "agent/add-feature");
    }

    #[test]
    fn test_create_agent_branch_name_with_conflict() {
        let existing = vec!["agent/add-feature".to_string()];
        let result = create_agent_branch_name("add feature", &existing);
        assert_eq!(result, "agent/add-feature-2");
    }

    #[test]
    fn test_worktree_path() {
        let path = worktree_path(std::path::Path::new("/repo"), "agent/my-task");
        assert_eq!(path, PathBuf::from("/repo/.worktrees/agent-my-task"));
    }
}

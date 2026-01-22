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
}

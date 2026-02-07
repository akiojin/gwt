//! Task definitions for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::sub_agent::SubAgent;
use super::types::TaskId;
use super::worktree::WorktreeRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeStrategy {
    New,
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRef {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestStatus {
    NotRun,
    Running,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestVerification {
    pub status: TestStatus,
    pub command: String,
    pub output: Option<String>,
    pub attempt: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: bool,
    pub summary: String,
    pub pull_request: Option<PullRequestRef>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub name: String,
    pub description: String,
    pub status: TaskStatus,
    pub dependencies: Vec<TaskId>,
    pub worktree_strategy: WorktreeStrategy,
    pub assigned_worktree: Option<WorktreeRef>,
    pub sub_agent: Option<SubAgent>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<TaskResult>,
    /// Test verification state for this task
    #[serde(default)]
    pub test_status: Option<TestVerification>,
    /// Number of retry attempts
    #[serde(default)]
    pub retry_count: u8,
    /// Associated pull request (after PR creation)
    #[serde(default)]
    pub pull_request: Option<PullRequestRef>,
}

impl Task {
    pub fn new(id: TaskId, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            status: TaskStatus::Pending,
            dependencies: Vec::new(),
            worktree_strategy: WorktreeStrategy::New,
            assigned_worktree: None,
            sub_agent: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            result: None,
            test_status: None,
            retry_count: 0,
            pull_request: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_new_defaults() {
        let task = Task::new(TaskId("task-1".to_string()), "test", "desc");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.worktree_strategy, WorktreeStrategy::New);
        assert!(task.dependencies.is_empty());
    }
}

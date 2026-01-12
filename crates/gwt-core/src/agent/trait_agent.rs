//! Agent Trait Definition

use crate::error::Result;
use async_trait::async_trait;
use std::path::Path;

/// Agent information
#[derive(Debug, Clone)]
pub struct AgentInfo {
    /// Agent name
    pub name: String,
    /// Agent version
    pub version: String,
    /// Agent path
    pub path: Option<std::path::PathBuf>,
    /// Is authenticated
    pub authenticated: bool,
}

/// Agent capabilities
#[derive(Debug, Clone, Default)]
pub struct AgentCapabilities {
    /// Can execute code
    pub can_execute: bool,
    /// Can read files
    pub can_read_files: bool,
    /// Can write files
    pub can_write_files: bool,
    /// Can use bash
    pub can_use_bash: bool,
    /// Can use MCP tools
    pub can_use_mcp: bool,
    /// Supports streaming
    pub supports_streaming: bool,
    /// Supports multi-turn
    pub supports_multi_turn: bool,
}

/// Task execution result
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// Success flag
    pub success: bool,
    /// Output text
    pub output: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Files modified
    pub files_modified: Vec<String>,
    /// Execution time in milliseconds
    pub duration_ms: u64,
}

impl TaskResult {
    /// Create a successful result
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
            files_modified: Vec::new(),
            duration_ms: 0,
        }
    }

    /// Create a failed result
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
            files_modified: Vec::new(),
            duration_ms: 0,
        }
    }

    /// Add modified files
    pub fn with_files(mut self, files: Vec<String>) -> Self {
        self.files_modified = files;
        self
    }

    /// Set duration
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// Agent trait for all coding agents
#[async_trait]
pub trait AgentTrait: Send + Sync {
    /// Get agent information
    fn info(&self) -> AgentInfo;

    /// Get agent capabilities
    fn capabilities(&self) -> AgentCapabilities;

    /// Check if the agent is available
    fn is_available(&self) -> bool;

    /// Run a task with a prompt
    async fn run_task(&self, prompt: &str) -> Result<TaskResult>;

    /// Run a task in a specific directory (path as &Path)
    async fn run_in_directory_path(
        &self,
        directory: &Path,
        prompt: &str,
    ) -> Result<TaskResult>;

    /// Cancel a running task
    async fn cancel(&self) -> Result<()>;

    /// Get the agent's conversation history
    fn get_history(&self) -> Vec<(String, String)>;

    /// Clear the agent's conversation history
    fn clear_history(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_result_success() {
        let result = TaskResult::success("Done")
            .with_files(vec!["file.rs".to_string()])
            .with_duration(100);

        assert!(result.success);
        assert_eq!(result.output, "Done");
        assert!(result.error.is_none());
        assert_eq!(result.files_modified.len(), 1);
        assert_eq!(result.duration_ms, 100);
    }

    #[test]
    fn test_task_result_failure() {
        let result = TaskResult::failure("Error occurred");

        assert!(!result.success);
        assert_eq!(result.error, Some("Error occurred".to_string()));
    }
}

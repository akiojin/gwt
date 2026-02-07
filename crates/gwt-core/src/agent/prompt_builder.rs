//! Adaptive prompt builder for sub-agents
//!
//! Generates task-specific prompts for sub-agents, including:
//! - Task description and instructions
//! - CLAUDE.md project rules
//! - Completion signal instructions
//! - Adaptive context (directory tree, related tasks, etc.)

use super::scanner::RepositoryScanResult;
use super::task::Task;

/// Builder for sub-agent prompts
pub struct PromptBuilder {
    /// CLAUDE.md content
    claude_md: Option<String>,
    /// Directory tree
    directory_tree: String,
    /// Whether to include extended context
    include_extended_context: bool,
}

impl PromptBuilder {
    pub fn new(scan_result: &RepositoryScanResult) -> Self {
        Self {
            claude_md: scan_result.claude_md.clone(),
            directory_tree: scan_result.directory_tree.clone(),
            include_extended_context: false,
        }
    }

    /// Enable extended context for complex tasks
    pub fn with_extended_context(mut self, enabled: bool) -> Self {
        self.include_extended_context = enabled;
        self
    }

    /// Build a prompt for a sub-agent task
    pub fn build_sub_agent_prompt(
        &self,
        task: &Task,
        related_tasks: &[&Task],
    ) -> String {
        let mut sections = Vec::new();

        // Task instruction
        sections.push(format!(
            "# Task: {}\n\n{}\n",
            task.name, task.description
        ));

        // CLAUDE.md rules
        if let Some(claude_md) = &self.claude_md {
            sections.push(format!(
                "# Project Rules (CLAUDE.md)\n\n{}\n",
                claude_md
            ));
        }

        // Completion instructions
        sections.push(self.build_completion_instructions());

        // Adaptive context
        if self.should_include_extended(task) {
            sections.push(format!(
                "# Directory Structure\n\n```\n{}\n```\n",
                self.directory_tree
            ));

            if !related_tasks.is_empty() {
                let related = related_tasks
                    .iter()
                    .map(|t| format!("- {}: {}", t.name, t.description))
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(format!(
                    "# Related Tasks\n\n{}\n",
                    related
                ));
            }
        }

        sections.join("\n")
    }

    fn build_completion_instructions(&self) -> String {
        "# Completion Instructions\n\n\
         When you have completed the task:\n\
         1. Commit your changes with a Conventional Commits message\n\
         2. Exit the session (type `q` or use the exit command)\n\
         3. The marker `GWT_TASK_DONE` will be used to detect completion\n"
            .to_string()
    }

    fn should_include_extended(&self, task: &Task) -> bool {
        if self.include_extended_context {
            return true;
        }
        // Heuristic: include extended context for tasks with long descriptions
        task.description.len() > 200 || !task.dependencies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::scanner::{BuildSystem, RepositoryScanResult};
    use crate::agent::task::Task;
    use crate::agent::types::TaskId;

    fn make_scan_result() -> RepositoryScanResult {
        RepositoryScanResult {
            claude_md: Some("# Rules\n- Be concise".to_string()),
            directory_tree: "src\ntests".to_string(),
            build_system: BuildSystem::Cargo,
            test_command: "cargo test".to_string(),
            existing_specs: vec![],
            source_overview: vec![],
        }
    }

    #[test]
    fn test_build_prompt_basic() {
        let scan = make_scan_result();
        let builder = PromptBuilder::new(&scan);
        let task = Task::new(
            TaskId::new(),
            "Add feature X",
            "Implement feature X in module Y",
        );
        let prompt = builder.build_sub_agent_prompt(&task, &[]);
        assert!(prompt.contains("Add feature X"));
        assert!(prompt.contains("Implement feature X"));
        assert!(prompt.contains("GWT_TASK_DONE"));
        assert!(prompt.contains("# Rules"));
    }

    #[test]
    fn test_build_prompt_with_extended() {
        let scan = make_scan_result();
        let builder = PromptBuilder::new(&scan).with_extended_context(true);
        let task = Task::new(TaskId::new(), "Test", "Short desc");
        let prompt = builder.build_sub_agent_prompt(&task, &[]);
        assert!(prompt.contains("Directory Structure"));
    }

    #[test]
    fn test_build_prompt_without_claude_md() {
        let mut scan = make_scan_result();
        scan.claude_md = None;
        let builder = PromptBuilder::new(&scan);
        let task = Task::new(TaskId::new(), "Test", "Short desc");
        let prompt = builder.build_sub_agent_prompt(&task, &[]);
        assert!(!prompt.contains("Project Rules"));
    }

    #[test]
    fn test_build_prompt_with_related_tasks() {
        let scan = make_scan_result();
        let builder = PromptBuilder::new(&scan).with_extended_context(true);
        let task = Task::new(TaskId::new(), "Main task", "Do the main thing");
        let related = Task::new(TaskId::new(), "Related", "Related work");
        let prompt = builder.build_sub_agent_prompt(&task, &[&related]);
        assert!(prompt.contains("Related Tasks"));
        assert!(prompt.contains("Related work"));
    }
}

//! Coding Agent Integration Module
//!
//! Provides a unified interface for interacting with various AI coding agents
//! (Claude Code, Codex CLI, Gemini CLI).

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod trait_agent;

use crate::error::{GwtError, Result};
use std::path::Path;

pub use trait_agent::{AgentCapabilities, AgentInfo, AgentTrait, TaskResult};

/// Agent type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    Claude,
    Codex,
    Gemini,
}

impl AgentType {
    /// Get display name
    pub fn name(&self) -> &'static str {
        match self {
            AgentType::Claude => "Claude Code",
            AgentType::Codex => "Codex CLI",
            AgentType::Gemini => "Gemini CLI",
        }
    }

    /// Get command name
    pub fn command(&self) -> &'static str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Codex => "codex",
            AgentType::Gemini => "gemini",
        }
    }
}

/// Agent manager for coordinating multiple agents
pub struct AgentManager {
    /// Working directory
    working_dir: std::path::PathBuf,
    /// Preferred agent
    preferred: Option<AgentType>,
}

impl AgentManager {
    /// Create a new agent manager
    pub fn new(working_dir: impl AsRef<Path>) -> Self {
        Self {
            working_dir: working_dir.as_ref().to_path_buf(),
            preferred: None,
        }
    }

    /// Set preferred agent
    pub fn with_preferred(mut self, agent: AgentType) -> Self {
        self.preferred = Some(agent);
        self
    }

    /// Detect available agents
    pub fn detect_agents(&self) -> Vec<AgentInfo> {
        let mut agents = Vec::new();

        // Check Claude Code
        if let Some(info) = claude::ClaudeAgent::detect() {
            agents.push(info);
        }

        // Check Codex CLI
        if let Some(info) = codex::CodexAgent::detect() {
            agents.push(info);
        }

        // Check Gemini CLI
        if let Some(info) = gemini::GeminiAgent::detect() {
            agents.push(info);
        }

        agents
    }

    /// Get the best available agent
    pub fn get_best_agent(&self) -> Option<Box<dyn AgentTrait>> {
        // If preferred is set, try that first
        if let Some(preferred) = &self.preferred {
            match preferred {
                AgentType::Claude => {
                    if claude::ClaudeAgent::detect().is_some() {
                        return Some(Box::new(claude::ClaudeAgent::new(&self.working_dir)));
                    }
                }
                AgentType::Codex => {
                    if codex::CodexAgent::detect().is_some() {
                        return Some(Box::new(codex::CodexAgent::new(&self.working_dir)));
                    }
                }
                AgentType::Gemini => {
                    if gemini::GeminiAgent::detect().is_some() {
                        return Some(Box::new(gemini::GeminiAgent::new(&self.working_dir)));
                    }
                }
            }
        }

        // Fall back to first available
        if claude::ClaudeAgent::detect().is_some() {
            return Some(Box::new(claude::ClaudeAgent::new(&self.working_dir)));
        }
        if codex::CodexAgent::detect().is_some() {
            return Some(Box::new(codex::CodexAgent::new(&self.working_dir)));
        }
        if gemini::GeminiAgent::detect().is_some() {
            return Some(Box::new(gemini::GeminiAgent::new(&self.working_dir)));
        }

        None
    }

    /// Get a specific agent by type
    pub fn get_agent(&self, agent_type: AgentType) -> Option<Box<dyn AgentTrait>> {
        match agent_type {
            AgentType::Claude => {
                if claude::ClaudeAgent::detect().is_some() {
                    Some(Box::new(claude::ClaudeAgent::new(&self.working_dir)))
                } else {
                    None
                }
            }
            AgentType::Codex => {
                if codex::CodexAgent::detect().is_some() {
                    Some(Box::new(codex::CodexAgent::new(&self.working_dir)))
                } else {
                    None
                }
            }
            AgentType::Gemini => {
                if gemini::GeminiAgent::detect().is_some() {
                    Some(Box::new(gemini::GeminiAgent::new(&self.working_dir)))
                } else {
                    None
                }
            }
        }
    }

    /// Run a task with the best available agent
    pub async fn run_task(&self, prompt: &str) -> Result<TaskResult> {
        let agent = self.get_best_agent().ok_or_else(|| {
            GwtError::Internal("No coding agent available".to_string())
        })?;

        agent.run_task(prompt).await
    }

    /// Run a task in a specific worktree
    pub async fn run_in_worktree(
        &self,
        worktree_path: impl AsRef<Path>,
        prompt: &str,
    ) -> Result<TaskResult> {
        let agent = self.get_best_agent().ok_or_else(|| {
            GwtError::Internal("No coding agent available".to_string())
        })?;

        agent.run_in_directory_path(worktree_path.as_ref(), prompt).await
    }
}

/// Check if a command exists in PATH
pub fn command_exists(command: &str) -> bool {
    std::process::Command::new("which")
        .arg(command)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get command version
pub fn get_command_version(command: &str, version_flag: &str) -> Option<String> {
    std::process::Command::new(command)
        .arg(version_flag)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    

    #[test]
    fn test_agent_type_name() {
        assert_eq!(AgentType::Claude.name(), "Claude Code");
        assert_eq!(AgentType::Codex.name(), "Codex CLI");
        assert_eq!(AgentType::Gemini.name(), "Gemini CLI");
    }

    #[test]
    fn test_agent_type_command() {
        assert_eq!(AgentType::Claude.command(), "claude");
        assert_eq!(AgentType::Codex.command(), "codex");
        assert_eq!(AgentType::Gemini.command(), "gemini");
    }

    #[test]
    fn test_agent_manager_creation() {
        let manager = AgentManager::new("/tmp/test");
        assert!(manager.preferred.is_none());

        let manager = manager.with_preferred(AgentType::Claude);
        assert_eq!(manager.preferred, Some(AgentType::Claude));
    }
}

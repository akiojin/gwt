//! Gemini CLI Integration

use super::trait_agent::{AgentCapabilities, AgentInfo, AgentTrait, TaskResult};
use super::{command_exists, get_command_version};
use crate::error::{GwtError, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::process::Command;

/// Gemini CLI agent
pub struct GeminiAgent {
    /// Working directory
    working_dir: PathBuf,
    /// Conversation history
    history: Arc<Mutex<Vec<(String, String)>>>,
    /// Child process handle for cancellation
    child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl GeminiAgent {
    /// Create a new Gemini agent
    pub fn new(working_dir: impl AsRef<Path>) -> Self {
        Self {
            working_dir: working_dir.as_ref().to_path_buf(),
            history: Arc::new(Mutex::new(Vec::new())),
            child: Arc::new(Mutex::new(None)),
        }
    }

    /// Detect if Gemini CLI is available
    pub fn detect() -> Option<AgentInfo> {
        if !command_exists("gemini") {
            return None;
        }

        let version = get_command_version("gemini", "--version")
            .unwrap_or_else(|| "unknown".to_string());

        // Check if authenticated (Gemini uses API key from env or config)
        let authenticated = std::env::var("GOOGLE_API_KEY").is_ok()
            || std::env::var("GEMINI_API_KEY").is_ok();

        Some(AgentInfo {
            name: "Gemini CLI".to_string(),
            version,
            path: which::which("gemini").ok(),
            authenticated,
        })
    }

    /// Build command with common arguments
    fn build_command(&self, prompt: &str, directory: &Path) -> Command {
        let mut cmd = Command::new("gemini");
        cmd.arg("--non-interactive")
            .arg(prompt)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

#[async_trait]
impl AgentTrait for GeminiAgent {
    fn info(&self) -> AgentInfo {
        Self::detect().unwrap_or(AgentInfo {
            name: "Gemini CLI".to_string(),
            version: "unknown".to_string(),
            path: None,
            authenticated: false,
        })
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            can_execute: true,
            can_read_files: true,
            can_write_files: true,
            can_use_bash: true,
            can_use_mcp: false, // Gemini CLI doesn't support MCP yet
            supports_streaming: true,
            supports_multi_turn: true,
        }
    }

    fn is_available(&self) -> bool {
        Self::detect().is_some()
    }

    async fn run_task(&self, prompt: &str) -> Result<TaskResult> {
        self.run_in_directory_path(&self.working_dir.clone(), prompt).await
    }

    async fn run_in_directory_path(
        &self,
        directory: &Path,
        prompt: &str,
    ) -> Result<TaskResult> {
        let start = Instant::now();

        let mut cmd = self.build_command(prompt, directory);

        let child = cmd.spawn().map_err(|e| {
            GwtError::Internal(format!("Failed to spawn Gemini CLI: {}", e))
        })?;

        let output = child.wait_with_output().await.map_err(|e| {
            GwtError::Internal(format!("Failed to run Gemini CLI: {}", e))
        })?;

        let duration = start.elapsed().as_millis() as u64;

        if output.status.success() {
            let output_text = String::from_utf8_lossy(&output.stdout).to_string();

            // Store in history
            {
                let mut history = self.history.lock().unwrap();
                history.push((prompt.to_string(), output_text.clone()));
            }

            Ok(TaskResult::success(output_text).with_duration(duration))
        } else {
            let error = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(TaskResult::failure(error).with_duration(duration))
        }
    }

    async fn cancel(&self) -> Result<()> {
        let child_opt = {
            let mut guard = self.child.lock().unwrap();
            guard.take()
        };
        if let Some(mut child) = child_opt {
            child.kill().await.map_err(|e| {
                GwtError::Internal(format!("Failed to kill process: {}", e))
            })?;
        }
        Ok(())
    }

    fn get_history(&self) -> Vec<(String, String)> {
        self.history.lock().unwrap().clone()
    }

    fn clear_history(&mut self) {
        self.history.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemini_agent_creation() {
        let agent = GeminiAgent::new("/tmp/test");
        assert_eq!(agent.working_dir, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_gemini_capabilities() {
        let agent = GeminiAgent::new("/tmp/test");
        let caps = agent.capabilities();
        assert!(caps.can_execute);
        assert!(caps.can_read_files);
        assert!(!caps.can_use_mcp); // Gemini doesn't support MCP yet
    }
}

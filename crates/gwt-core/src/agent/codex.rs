//! Codex CLI Integration

use super::trait_agent::{AgentCapabilities, AgentInfo, AgentTrait, TaskResult};
use super::{command_exists, get_command_version};
use crate::error::{GwtError, Result};
use async_trait::async_trait;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::process::Command;

const DEFAULT_CODEX_MODEL: &str = "gpt-5.1-codex";
const DEFAULT_CODEX_REASONING: &str = "high";
const CODEX_SKILLS_FLAG_DEPRECATED_FROM: &str = "0.80.0";
const CODEX_SKIP_FLAG_DEPRECATED_FROM: &str = "0.80.0";
const CODEX_SKIP_FLAG_LEGACY: &str = "--yolo";
const CODEX_SKIP_FLAG_DANGEROUS: &str = "--dangerously-bypass-approvals-and-sandbox";
const MODEL_FLAG_PREFIX: &str = "--model=";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

/// Codex CLI agent
pub struct CodexAgent {
    /// Working directory
    working_dir: PathBuf,
    /// Conversation history
    history: Arc<Mutex<Vec<(String, String)>>>,
    /// Child process handle for cancellation
    child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl CodexAgent {
    /// Create a new Codex agent
    pub fn new(working_dir: impl AsRef<Path>) -> Self {
        Self {
            working_dir: working_dir.as_ref().to_path_buf(),
            history: Arc::new(Mutex::new(Vec::new())),
            child: Arc::new(Mutex::new(None)),
        }
    }

    /// Detect if Codex CLI is available
    pub fn detect() -> Option<AgentInfo> {
        if !command_exists("codex") {
            return None;
        }

        let version =
            get_command_version("codex", "--version").unwrap_or_else(|| "unknown".to_string());

        // Check if authenticated (Codex uses API key from env)
        let authenticated = std::env::var("OPENAI_API_KEY").is_ok();

        Some(AgentInfo {
            name: "Codex CLI".to_string(),
            version,
            path: which::which("codex").ok(),
            authenticated,
        })
    }

    /// Build command with common arguments
    fn build_command(&self, prompt: &str, directory: &Path) -> Command {
        let mut cmd = Command::new("codex");
        let version = get_command_version("codex", "--version");
        cmd.arg("--quiet")
            .args(codex_default_args(None, None, version.as_deref(), false))
            .arg(prompt)
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
}

fn normalize_version(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    for token in raw.split_whitespace() {
        let trimmed = token.trim_start_matches('v');
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
            let cleaned = trimmed.trim_end_matches(|ch: char| {
                !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '-')
            });
            if cleaned.is_empty() {
                continue;
            }
            return Some(cleaned.to_string());
        }
    }
    None
}

fn parse_version(value: Option<&str>) -> Option<ParsedVersion> {
    let normalized = normalize_version(value)?;
    let mut parts = normalized.splitn(2, '-');
    let core = parts.next().unwrap_or_default();
    let prerelease = parts.next().map(|s| s.to_string());
    let core_parts: Vec<&str> = core.split('.').collect();
    if core_parts.len() < 2 || core_parts.len() > 3 {
        return None;
    }
    let major = core_parts.first()?.parse().ok()?;
    let minor = core_parts.get(1)?.parse().ok()?;
    let patch = if core_parts.len() == 3 {
        core_parts.get(2)?.parse().ok()?
    } else {
        0
    };
    Some(ParsedVersion {
        major,
        minor,
        patch,
        prerelease,
    })
}

fn compare_versions(a: &ParsedVersion, b: &ParsedVersion) -> Ordering {
    if a.major != b.major {
        return a.major.cmp(&b.major);
    }
    if a.minor != b.minor {
        return a.minor.cmp(&b.minor);
    }
    if a.patch != b.patch {
        return a.patch.cmp(&b.patch);
    }
    match (&a.prerelease, &b.prerelease) {
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(a_pre), Some(b_pre)) => a_pre.cmp(b_pre),
        (None, None) => Ordering::Equal,
    }
}

fn should_enable_codex_skills_flag(version: Option<&str>) -> bool {
    let parsed = parse_version(version);
    let threshold = parse_version(Some(CODEX_SKILLS_FLAG_DEPRECATED_FROM));
    match (parsed, threshold) {
        (Some(parsed), Some(threshold)) => compare_versions(&parsed, &threshold) == Ordering::Less,
        _ => false,
    }
}

pub fn codex_skip_permissions_flag(version: Option<&str>) -> &'static str {
    let parsed = parse_version(version);
    let threshold = parse_version(Some(CODEX_SKIP_FLAG_DEPRECATED_FROM));
    match (parsed, threshold) {
        (Some(parsed), Some(threshold)) => {
            if compare_versions(&parsed, &threshold) == Ordering::Less {
                CODEX_SKIP_FLAG_LEGACY
            } else {
                CODEX_SKIP_FLAG_DANGEROUS
            }
        }
        _ => CODEX_SKIP_FLAG_DANGEROUS,
    }
}

fn with_codex_skills_flag(mut args: Vec<String>, enable: bool) -> Vec<String> {
    if !enable {
        return args;
    }
    let already_enabled = args.iter().enumerate().any(|(index, arg)| {
        arg == "--enable" && args.get(index + 1).is_some_and(|v| v == "skills")
    });
    if already_enabled {
        return args;
    }
    let insert_index = args
        .iter()
        .position(|arg| arg.starts_with(MODEL_FLAG_PREFIX))
        .unwrap_or(args.len());
    args.splice(
        insert_index..insert_index,
        ["--enable".to_string(), "skills".to_string()],
    );
    args
}

pub fn codex_default_args(
    model_override: Option<&str>,
    reasoning_override: Option<&str>,
    skills_flag_version: Option<&str>,
    bypass_sandbox: bool,
) -> Vec<String> {
    let model = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_MODEL);
    let reasoning = reasoning_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_REASONING);

    let mut args = vec![
        "--enable".to_string(),
        "web_search_request".to_string(),
        format!("--model={}", model),
    ];

    if !bypass_sandbox {
        args.push("--sandbox".to_string());
        args.push("workspace-write".to_string());
    }

    args.extend([
        "-c".to_string(),
        format!("model_reasoning_effort={}", reasoning),
        "-c".to_string(),
        "model_reasoning_summaries=detailed".to_string(),
    ]);

    if !bypass_sandbox {
        args.extend([
            "-c".to_string(),
            "sandbox_workspace_write.network_access=true".to_string(),
        ]);
    }

    args.extend([
        "-c".to_string(),
        "shell_environment_policy.inherit=all".to_string(),
        "-c".to_string(),
        "shell_environment_policy.ignore_default_excludes=true".to_string(),
        "-c".to_string(),
        "shell_environment_policy.experimental_use_profile=true".to_string(),
    ]);

    let enable_skills = should_enable_codex_skills_flag(skills_flag_version);
    args = with_codex_skills_flag(args, enable_skills);
    args
}

#[async_trait]
impl AgentTrait for CodexAgent {
    fn info(&self) -> AgentInfo {
        Self::detect().unwrap_or(AgentInfo {
            name: "Codex CLI".to_string(),
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
            can_use_mcp: false, // Codex doesn't support MCP
            supports_streaming: true,
            supports_multi_turn: true,
        }
    }

    fn is_available(&self) -> bool {
        Self::detect().is_some()
    }

    async fn run_task(&self, prompt: &str) -> Result<TaskResult> {
        self.run_in_directory_path(&self.working_dir.clone(), prompt)
            .await
    }

    async fn run_in_directory_path(&self, directory: &Path, prompt: &str) -> Result<TaskResult> {
        let start = Instant::now();

        let mut cmd = self.build_command(prompt, directory);

        let child = cmd
            .spawn()
            .map_err(|e| GwtError::Internal(format!("Failed to spawn Codex CLI: {}", e)))?;

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| GwtError::Internal(format!("Failed to run Codex CLI: {}", e)))?;

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
            child
                .kill()
                .await
                .map_err(|e| GwtError::Internal(format!("Failed to kill process: {}", e)))?;
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
    fn test_codex_agent_creation() {
        let agent = CodexAgent::new("/tmp/test");
        assert_eq!(agent.working_dir, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_codex_capabilities() {
        let agent = CodexAgent::new("/tmp/test");
        let caps = agent.capabilities();
        assert!(caps.can_execute);
        assert!(caps.can_read_files);
        assert!(!caps.can_use_mcp); // Codex doesn't support MCP
    }

    #[test]
    fn test_codex_default_args_defaults() {
        let args = codex_default_args(None, None, None, false);
        assert!(args.contains(&"--enable".to_string()));
        assert!(args.contains(&"web_search_request".to_string()));
        assert!(args.contains(&"--model=gpt-5.1-codex".to_string()));
        assert!(args.contains(&"model_reasoning_effort=high".to_string()));
    }

    #[test]
    fn test_codex_default_args_overrides() {
        let args = codex_default_args(Some("gpt-5.2"), Some("xhigh"), None, false);
        assert!(args.contains(&"--model=gpt-5.2".to_string()));
        assert!(args.contains(&"model_reasoning_effort=xhigh".to_string()));
    }

    #[test]
    fn test_codex_skills_flag_version_gate() {
        let args_old = codex_default_args(None, None, Some("0.79.0"), false);
        let skills_present_old = args_old.iter().enumerate().any(|(idx, arg)| {
            arg == "--enable" && args_old.get(idx + 1).is_some_and(|v| v == "skills")
        });
        assert!(skills_present_old);

        let args_new = codex_default_args(None, None, Some("0.80.0"), false);
        let skills_present_new = args_new.iter().enumerate().any(|(idx, arg)| {
            arg == "--enable" && args_new.get(idx + 1).is_some_and(|v| v == "skills")
        });
        assert!(!skills_present_new);
    }

    #[test]
    fn test_codex_default_args_bypass_sandbox() {
        let args = codex_default_args(None, None, None, true);
        assert!(!args.contains(&"--sandbox".to_string()));
        assert!(!args.contains(&"sandbox_workspace_write.network_access=true".to_string()));
    }

    #[test]
    fn test_codex_skip_permissions_flag_version_gate() {
        assert_eq!(codex_skip_permissions_flag(Some("0.79.9")), "--yolo");
        assert_eq!(
            codex_skip_permissions_flag(Some("0.80.0")),
            "--dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(
            codex_skip_permissions_flag(Some("codex-cli 0.80.1")),
            "--dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(codex_skip_permissions_flag(Some("v0.79.0")), "--yolo");
        assert_eq!(
            codex_skip_permissions_flag(None),
            "--dangerously-bypass-approvals-and-sandbox"
        );
        assert_eq!(
            codex_skip_permissions_flag(Some("unknown")),
            "--dangerously-bypass-approvals-and-sandbox"
        );
    }
}

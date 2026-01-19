//! TUI application

mod app;
mod components;
mod event;
mod screens;

pub use app::{run_with_context, AgentLaunchConfig, TuiEntryContext};
pub use screens::{CodingAgent, ExecutionMode};

pub(crate) fn normalize_agent_label(agent_name: &str) -> String {
    let lower = agent_name.trim().to_lowercase();
    match lower.as_str() {
        "codex-cli" | "codex" => "Codex".to_string(),
        "gemini-cli" | "gemini" => "Gemini".to_string(),
        "claude-code" | "claude" => "Claude".to_string(),
        "opencode" | "open-code" => "OpenCode".to_string(),
        "aider" => "Aider".to_string(),
        "cursor" => "Cursor".to_string(),
        "cline" => "Cline".to_string(),
        "copilot" => "Copilot".to_string(),
        "gpt" => "Gpt".to_string(),
        _ => {
            let mut chars = lower.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }
    }
}

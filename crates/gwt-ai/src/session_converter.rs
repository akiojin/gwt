//! Session format conversion between AI agent formats.
//!
//! Each agent (Claude, Codex, Gemini, OpenCode) stores sessions in its own
//! format. This module provides a trait-based encoder system and a top-level
//! `convert_session` function to translate a generic session history from
//! one format to another.

use serde::{Deserialize, Serialize};

use crate::error::AIError;

/// Supported session formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFormat {
    Claude,
    Codex,
    Gemini,
    OpenCode,
}

impl SessionFormat {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
        }
    }
}

/// A role in the conversation history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A single message in a session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: Role,
    pub content: String,
}

/// Trait for encoding a session history into a specific agent format.
pub trait SessionEncoder {
    /// Human-readable name of the target format (e.g. "Claude", "Codex").
    fn name(&self) -> &str;

    /// Encode the given history into the target format.
    fn encode(&self, history: &[SessionMessage]) -> Result<String, AIError>;
}

// ── Concrete encoders ──────────────────────────────────────────────────

/// Encoder for Claude session format (JSONL of messages).
pub struct ClaudeEncoder;

impl SessionEncoder for ClaudeEncoder {
    fn name(&self) -> &str {
        "Claude"
    }

    fn encode(&self, history: &[SessionMessage]) -> Result<String, AIError> {
        if history.is_empty() {
            return Err(AIError::ParseError("Empty session history".into()));
        }
        let lines: Vec<String> = history
            .iter()
            .map(|m| {
                serde_json::to_string(m)
                    .map_err(|e| AIError::ParseError(format!("JSON encode error: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(lines.join("\n"))
    }
}

/// Encoder for Codex CLI session format.
pub struct CodexEncoder;

impl SessionEncoder for CodexEncoder {
    fn name(&self) -> &str {
        "Codex"
    }

    fn encode(&self, history: &[SessionMessage]) -> Result<String, AIError> {
        if history.is_empty() {
            return Err(AIError::ParseError("Empty session history".into()));
        }
        let messages: Vec<serde_json::Value> = history
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": [{"type": "input_text", "text": m.content}]
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "messages": messages }))
            .map_err(|e| AIError::ParseError(format!("JSON encode error: {e}")))
    }
}

/// Encoder for Gemini CLI session format.
pub struct GeminiEncoder;

impl SessionEncoder for GeminiEncoder {
    fn name(&self) -> &str {
        "Gemini"
    }

    fn encode(&self, history: &[SessionMessage]) -> Result<String, AIError> {
        if history.is_empty() {
            return Err(AIError::ParseError("Empty session history".into()));
        }
        let contents: Vec<serde_json::Value> = history
            .iter()
            .map(|m| {
                let gemini_role = match m.role {
                    Role::System | Role::User => "user",
                    Role::Assistant => "model",
                };
                serde_json::json!({
                    "role": gemini_role,
                    "parts": [{"text": m.content}]
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "contents": contents }))
            .map_err(|e| AIError::ParseError(format!("JSON encode error: {e}")))
    }
}

/// Encoder for OpenCode session format.
pub struct OpenCodeEncoder;

impl SessionEncoder for OpenCodeEncoder {
    fn name(&self) -> &str {
        "OpenCode"
    }

    fn encode(&self, history: &[SessionMessage]) -> Result<String, AIError> {
        if history.is_empty() {
            return Err(AIError::ParseError("Empty session history".into()));
        }
        let messages: Vec<serde_json::Value> = history
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })
            })
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "messages": messages }))
            .map_err(|e| AIError::ParseError(format!("JSON encode error: {e}")))
    }
}

/// Get an encoder by target format name (case-insensitive).
pub fn get_encoder(name: &str) -> Result<Box<dyn SessionEncoder>, AIError> {
    match SessionFormat::parse(name) {
        Some(SessionFormat::Claude) => Ok(Box::new(ClaudeEncoder)),
        Some(SessionFormat::Codex) => Ok(Box::new(CodexEncoder)),
        Some(SessionFormat::Gemini) => Ok(Box::new(GeminiEncoder)),
        Some(SessionFormat::OpenCode) => Ok(Box::new(OpenCodeEncoder)),
        None => Err(AIError::ConfigError(format!(
            "Unknown target format: {name}"
        ))),
    }
}

/// Return the supported format names in stable order.
pub fn supported_formats() -> &'static [&'static str] {
    &["claude", "codex", "gemini", "opencode"]
}

/// Convert a session history from one format to another.
///
/// The `from` and `to` parameters are format names (e.g. `"claude"`, `"codex"`).
pub fn convert_session(
    from: &str,
    to: &str,
    history: &[SessionMessage],
) -> Result<String, AIError> {
    let source = SessionFormat::parse(from)
        .ok_or_else(|| AIError::ConfigError(format!("Unknown source format: {from}")))?;
    let target = SessionFormat::parse(to)
        .ok_or_else(|| AIError::ConfigError(format!("Unknown target format: {to}")))?;

    if source == target {
        return Err(AIError::ConfigError(
            "Source and target formats are the same".into(),
        ));
    }
    if history.is_empty() {
        return Err(AIError::ParseError("Empty session history".into()));
    }

    let encoder = get_encoder(target.name())?;
    encoder.encode(history)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_history() -> Vec<SessionMessage> {
        vec![
            SessionMessage {
                role: Role::System,
                content: "You are helpful.".into(),
            },
            SessionMessage {
                role: Role::User,
                content: "Hello".into(),
            },
            SessionMessage {
                role: Role::Assistant,
                content: "Hi there!".into(),
            },
        ]
    }

    // ── ClaudeEncoder ──────────────────────────────────────────────────

    #[test]
    fn claude_encoder_produces_jsonl() {
        let enc = ClaudeEncoder;
        let result = enc.encode(&sample_history()).unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        // Each line should be valid JSON
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn claude_encoder_rejects_empty() {
        assert!(ClaudeEncoder.encode(&[]).is_err());
    }

    // ── CodexEncoder ───────────────────────────────────────────────────

    #[test]
    fn codex_encoder_produces_json() {
        let enc = CodexEncoder;
        let result = enc.encode(&sample_history()).unwrap();
        let val: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(val["messages"].is_array());
        assert_eq!(val["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn codex_encoder_rejects_empty() {
        assert!(CodexEncoder.encode(&[]).is_err());
    }

    // ── GeminiEncoder ──────────────────────────────────────────────────

    #[test]
    fn gemini_encoder_maps_roles() {
        let enc = GeminiEncoder;
        let result = enc.encode(&sample_history()).unwrap();
        let val: serde_json::Value = serde_json::from_str(&result).unwrap();
        let contents = val["contents"].as_array().unwrap();
        // system -> user, user -> user, assistant -> model
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[1]["role"], "user");
        assert_eq!(contents[2]["role"], "model");
    }

    #[test]
    fn gemini_encoder_rejects_empty() {
        assert!(GeminiEncoder.encode(&[]).is_err());
    }

    // ── OpenCodeEncoder ────────────────────────────────────────────────

    #[test]
    fn opencode_encoder_produces_json() {
        let enc = OpenCodeEncoder;
        let result = enc.encode(&sample_history()).unwrap();
        let val: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(val["messages"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn opencode_encoder_rejects_empty() {
        assert!(OpenCodeEncoder.encode(&[]).is_err());
    }

    // ── get_encoder ────────────────────────────────────────────────────

    #[test]
    fn get_encoder_case_insensitive() {
        assert_eq!(get_encoder("Claude").unwrap().name(), "Claude");
        assert_eq!(get_encoder("CODEX").unwrap().name(), "Codex");
        assert_eq!(get_encoder("gemini").unwrap().name(), "Gemini");
        assert_eq!(get_encoder("OpenCode").unwrap().name(), "OpenCode");
    }

    #[test]
    fn get_encoder_unknown_fails() {
        assert!(get_encoder("unknown").is_err());
    }

    // ── convert_session ────────────────────────────────────────────────

    #[test]
    fn convert_session_works() {
        let history = sample_history();
        let result = convert_session("claude", "codex", &history).unwrap();
        let val: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(val["messages"].is_array());
    }

    #[test]
    fn convert_session_rejects_same_format() {
        let history = sample_history();
        let err = convert_session("claude", "claude", &history).unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    #[test]
    fn convert_session_rejects_empty_history() {
        let err = convert_session("claude", "codex", &[]).unwrap_err();
        assert!(matches!(err, AIError::ParseError(_)));
    }

    #[test]
    fn convert_session_rejects_unknown_target() {
        let history = sample_history();
        let err = convert_session("claude", "unknown", &history).unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    #[test]
    fn convert_session_rejects_unknown_source() {
        let history = sample_history();
        let err = convert_session("unknown", "codex", &history).unwrap_err();
        assert!(matches!(err, AIError::ConfigError(_)));
    }

    #[test]
    fn supported_formats_are_stable_and_non_empty() {
        let formats = supported_formats();
        assert_eq!(formats, &["claude", "codex", "gemini", "opencode"]);
    }
}

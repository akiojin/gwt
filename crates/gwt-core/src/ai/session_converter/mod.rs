//! Session converter infrastructure for converting sessions between agents.
//!
//! This module provides the ability to convert parsed sessions from one agent format
//! to another, enabling session continuity when switching between different AI agents.

use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};

use super::session_parser::{AgentType, ParsedSession};

pub mod claude_encoder;
pub mod codex_encoder;
pub mod gemini_encoder;
pub mod metadata;
pub mod opencode_encoder;

pub use claude_encoder::ClaudeEncoder;
pub use codex_encoder::CodexEncoder;
pub use gemini_encoder::GeminiEncoder;
pub use metadata::{ConversionMetadataStore, MetadataStoreError};
pub use opencode_encoder::OpenCodeEncoder;

/// Error type for session conversion operations.
#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("Source session not found: {0}")]
    SessionNotFound(String),
    #[error("Conversion not supported from {from:?} to {to:?}")]
    UnsupportedConversion { from: AgentType, to: AgentType },
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("No messages to convert")]
    EmptySession,
    #[error("Failed to determine output path: {0}")]
    OutputPathError(String),
    #[error("Metadata error: {0}")]
    MetadataError(#[from] MetadataStoreError),
}

/// Information about data lost during conversion.
#[derive(Debug, Clone, Default)]
pub struct LossInfo {
    /// Number of messages that could not be converted.
    pub dropped_messages: usize,
    /// Number of tool execution results that could not be converted.
    pub dropped_tool_results: usize,
    /// Metadata fields that could not be preserved.
    pub lost_metadata_fields: Vec<String>,
    /// Human-readable summary of information loss.
    pub summary: String,
}

impl LossInfo {
    /// Returns true if any data was lost during conversion.
    pub fn has_loss(&self) -> bool {
        self.dropped_messages > 0
            || self.dropped_tool_results > 0
            || !self.lost_metadata_fields.is_empty()
    }

    /// Creates a LossInfo with no data loss.
    pub fn none() -> Self {
        Self {
            summary: "No data loss".to_string(),
            ..Default::default()
        }
    }

    /// Builds a summary string from the current loss info.
    pub fn build_summary(&mut self) {
        let mut parts = Vec::new();

        if self.dropped_messages > 0 {
            parts.push(format!("{} messages dropped", self.dropped_messages));
        }
        if self.dropped_tool_results > 0 {
            parts.push(format!(
                "{} tool results dropped",
                self.dropped_tool_results
            ));
        }
        if !self.lost_metadata_fields.is_empty() {
            parts.push(format!(
                "metadata fields lost: {}",
                self.lost_metadata_fields.join(", ")
            ));
        }

        if parts.is_empty() {
            self.summary = "No data loss".to_string();
        } else {
            self.summary = parts.join("; ");
        }
    }
}

/// Metadata recorded for a conversion operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversionMetadata {
    /// The agent type the session was converted from.
    pub converted_from_agent: String,
    /// The original session ID before conversion.
    pub converted_from_session_id: String,
    /// When the conversion was performed.
    pub converted_at: DateTime<Utc>,
    /// Number of messages dropped during conversion.
    pub dropped_messages: usize,
    /// Number of tool results dropped during conversion.
    pub dropped_tool_results: usize,
    /// Metadata fields that were lost.
    pub lost_metadata_fields: Vec<String>,
    /// Human-readable summary of the conversion.
    pub loss_summary: String,
    /// Number of messages in the original session.
    pub original_message_count: usize,
    /// Number of tool executions in the original session.
    pub original_tool_count: usize,
}

impl ConversionMetadata {
    /// Creates a new ConversionMetadata from a parsed session and loss info.
    pub fn new(
        source_session: &ParsedSession,
        loss_info: &LossInfo,
        converted_at: DateTime<Utc>,
    ) -> Self {
        Self {
            converted_from_agent: source_session.agent_type.display_name().to_string(),
            converted_from_session_id: source_session.session_id.clone(),
            converted_at,
            dropped_messages: loss_info.dropped_messages,
            dropped_tool_results: loss_info.dropped_tool_results,
            lost_metadata_fields: loss_info.lost_metadata_fields.clone(),
            loss_summary: loss_info.summary.clone(),
            original_message_count: source_session.messages.len(),
            original_tool_count: source_session.tool_executions.len(),
        }
    }
}

/// Result of a session conversion operation.
#[derive(Debug, Clone)]
pub struct ConversionResult {
    /// Path to the converted session file.
    pub output_path: PathBuf,
    /// New session ID for the converted session.
    pub new_session_id: String,
    /// The target agent type.
    pub target_agent: AgentType,
    /// Information about data lost during conversion.
    pub loss_info: LossInfo,
    /// Conversion metadata for tracing.
    pub metadata: ConversionMetadata,
}

/// Trait for session encoders that convert ParsedSession to agent-specific formats.
pub trait SessionEncoder: Send + Sync {
    /// Returns the target agent type for this encoder.
    fn target_agent(&self) -> AgentType;

    /// Encodes a parsed session into the target agent's format.
    ///
    /// # Arguments
    /// * `session` - The parsed session to convert
    /// * `worktree_path` - The path to the current worktree (used for project-specific paths)
    ///
    /// # Returns
    /// A ConversionResult containing the output path, new session ID, and loss information.
    fn encode(
        &self,
        session: &ParsedSession,
        worktree_path: &Path,
    ) -> Result<ConversionResult, ConversionError>;

    /// Returns the expected output path for a converted session.
    fn output_path(&self, worktree_path: &Path, session_id: &str) -> PathBuf;

    /// Generates a new session ID for the converted session.
    fn generate_session_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// Gets an encoder for the target agent type.
pub fn get_encoder(target_agent: AgentType) -> Box<dyn SessionEncoder> {
    match target_agent {
        AgentType::ClaudeCode => Box::new(ClaudeEncoder::new()),
        AgentType::CodexCli => Box::new(CodexEncoder::new()),
        AgentType::GeminiCli => Box::new(GeminiEncoder::new()),
        AgentType::OpenCode => Box::new(OpenCodeEncoder::new()),
    }
}

/// Converts a session from one agent format to another.
///
/// # Arguments
/// * `session` - The parsed session to convert
/// * `target_agent` - The agent type to convert to
/// * `worktree_path` - The path to the current worktree
///
/// # Returns
/// A ConversionResult containing the output path, new session ID, and loss information.
pub fn convert_session(
    session: &ParsedSession,
    target_agent: AgentType,
    worktree_path: &Path,
) -> Result<ConversionResult, ConversionError> {
    if session.messages.is_empty() {
        return Err(ConversionError::EmptySession);
    }

    let encoder = get_encoder(target_agent);
    let result = encoder.encode(session, worktree_path)?;

    // Store conversion metadata
    let metadata_store = ConversionMetadataStore::new()?;
    metadata_store.save(&result.new_session_id, &result.metadata)?;

    Ok(result)
}

/// Checks if conversion is available from one agent to another.
pub fn is_conversion_available(from: AgentType, to: AgentType) -> bool {
    // All conversions are available (though with potential data loss)
    from != to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loss_info_has_loss() {
        let mut info = LossInfo::default();
        assert!(!info.has_loss());

        info.dropped_messages = 1;
        assert!(info.has_loss());

        let info2 = LossInfo::none();
        assert!(!info2.has_loss());
    }

    #[test]
    fn test_loss_info_build_summary() {
        let mut info = LossInfo {
            dropped_messages: 2,
            dropped_tool_results: 1,
            lost_metadata_fields: vec!["custom_field".to_string()],
            summary: String::new(),
        };
        info.build_summary();
        assert!(info.summary.contains("2 messages dropped"));
        assert!(info.summary.contains("1 tool results dropped"));
        assert!(info.summary.contains("custom_field"));
    }

    #[test]
    fn test_is_conversion_available() {
        assert!(is_conversion_available(
            AgentType::ClaudeCode,
            AgentType::CodexCli
        ));
        assert!(is_conversion_available(
            AgentType::CodexCli,
            AgentType::GeminiCli
        ));
        assert!(!is_conversion_available(
            AgentType::ClaudeCode,
            AgentType::ClaudeCode
        ));
    }

    #[test]
    fn test_get_encoder() {
        let encoder = get_encoder(AgentType::ClaudeCode);
        assert_eq!(encoder.target_agent(), AgentType::ClaudeCode);

        let encoder = get_encoder(AgentType::CodexCli);
        assert_eq!(encoder.target_agent(), AgentType::CodexCli);
    }
}

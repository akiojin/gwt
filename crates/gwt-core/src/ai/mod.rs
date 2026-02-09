//! AI module for OpenAI-compatible APIs

pub mod agent_history;
pub mod branch_suggest;
pub(crate) mod claude_paths;
pub mod client;
pub mod session_converter;
pub mod session_parser;
pub mod summary;

pub use branch_suggest::{
    parse_branch_suggestions, suggest_branch_names, BRANCH_SUGGEST_SYSTEM_PROMPT,
};
pub use client::{format_error_for_display, AIClient, AIError, ChatMessage, ModelInfo};
pub use session_converter::{
    convert_session, get_encoder, is_conversion_available, ClaudeEncoder, CodexEncoder,
    ConversionError, ConversionMetadata, ConversionMetadataStore, ConversionResult, GeminiEncoder,
    LossInfo, MetadataStoreError, OpenCodeEncoder, SessionEncoder,
};
pub use session_parser::{
    AgentType, ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, MessageRole,
    OpenCodeSessionParser, ParsedSession, SessionListEntry, SessionMessage, SessionParseError,
    SessionParser, ToolExecution,
};
pub use summary::{
    build_session_prompt, parse_summary_lines, summarize_scrollback, summarize_session,
    SessionMetrics, SessionSummary, SessionSummaryCache, SESSION_SYSTEM_PROMPT_BASE,
};

pub use agent_history::{AgentHistoryEntry, AgentHistoryError, AgentHistoryStore};

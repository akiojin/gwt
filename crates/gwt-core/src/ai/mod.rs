//! AI module for OpenAI-compatible APIs

pub mod client;
pub mod session_parser;
pub mod summary;

pub use client::{AIClient, AIError, ChatMessage};
pub use session_parser::{
    AgentType, ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, MessageRole,
    OpenCodeSessionParser, ParsedSession, SessionMessage, SessionParseError, SessionParser,
    ToolExecution,
};
pub use summary::{
    build_session_prompt, parse_summary_lines, summarize_session, SessionMetrics, SessionSummary,
    SessionSummaryCache, SESSION_SYSTEM_PROMPT,
};

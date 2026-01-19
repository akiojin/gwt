//! AI module for OpenAI-compatible APIs

pub mod client;
pub mod summary;

pub use client::{AIClient, AIError, ChatMessage};
pub use summary::{
    build_user_prompt, parse_summary_lines, summarize_commits, AISummaryCache, SummaryRequest,
    SYSTEM_PROMPT,
};

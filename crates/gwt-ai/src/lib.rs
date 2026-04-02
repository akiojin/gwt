//! AI client and utilities for Git Worktree Manager.
//!
//! This crate provides:
//! - [`client::AIClient`] — OpenAI Responses API client with retry logic
//! - [`branch_suggest`] — AI-powered branch name suggestions
//! - [`issue_classify`] — AI-powered issue classification
//! - [`session_converter`] — Session format conversion between agents
//! - [`error::AIError`] — Unified error type

pub mod branch_suggest;
pub mod client;
pub mod error;
pub mod issue_classify;
pub mod session_converter;

pub use branch_suggest::{parse_suggestions, suggest_branch_name};
pub use client::{AIClient, ChatMessage};
pub use error::AIError;
pub use issue_classify::{classify_issue, parse_classify_response};
pub use session_converter::{
    convert_session, get_encoder, ClaudeEncoder, CodexEncoder, GeminiEncoder, OpenCodeEncoder,
    Role, SessionEncoder, SessionMessage,
};

//! Lead state models for Project Team agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::conversation::MessageRole;

/// Lead status in the orchestration lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadStatus {
    Idle,
    Thinking,
    WaitingApproval,
    Orchestrating,
    Error,
}

/// Kind of message in lead conversation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Message,
    Thought,
    Action,
    Observation,
    Error,
    Progress,
}

/// A message in the lead conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadMessage {
    pub role: MessageRole,
    pub kind: MessageKind,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl LeadMessage {
    pub fn new(role: MessageRole, kind: MessageKind, content: impl Into<String>) -> Self {
        Self {
            role,
            kind,
            content: content.into(),
            timestamp: Utc::now(),
        }
    }
}

/// State of the Lead agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadState {
    pub conversation: Vec<LeadMessage>,
    pub status: LeadStatus,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
    pub active_issue_numbers: Vec<u64>,
    pub last_poll_at: Option<DateTime<Utc>>,
}

impl Default for LeadState {
    fn default() -> Self {
        Self {
            conversation: Vec::new(),
            status: LeadStatus::Idle,
            llm_call_count: 0,
            estimated_tokens: 0,
            active_issue_numbers: Vec::new(),
            last_poll_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- LeadStatus serde tests ---

    #[test]
    fn test_lead_status_serialize_idle() {
        let json = serde_json::to_string(&LeadStatus::Idle).unwrap();
        assert_eq!(json, r#""idle""#);
    }

    #[test]
    fn test_lead_status_serialize_thinking() {
        let json = serde_json::to_string(&LeadStatus::Thinking).unwrap();
        assert_eq!(json, r#""thinking""#);
    }

    #[test]
    fn test_lead_status_serialize_waiting_approval() {
        let json = serde_json::to_string(&LeadStatus::WaitingApproval).unwrap();
        assert_eq!(json, r#""waiting_approval""#);
    }

    #[test]
    fn test_lead_status_serialize_orchestrating() {
        let json = serde_json::to_string(&LeadStatus::Orchestrating).unwrap();
        assert_eq!(json, r#""orchestrating""#);
    }

    #[test]
    fn test_lead_status_serialize_error() {
        let json = serde_json::to_string(&LeadStatus::Error).unwrap();
        assert_eq!(json, r#""error""#);
    }

    #[test]
    fn test_lead_status_deserialize_roundtrip() {
        for status in [
            LeadStatus::Idle,
            LeadStatus::Thinking,
            LeadStatus::WaitingApproval,
            LeadStatus::Orchestrating,
            LeadStatus::Error,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: LeadStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    // --- MessageKind serde tests ---

    #[test]
    fn test_message_kind_serialize_all() {
        assert_eq!(
            serde_json::to_string(&MessageKind::Message).unwrap(),
            r#""message""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Thought).unwrap(),
            r#""thought""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Action).unwrap(),
            r#""action""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Observation).unwrap(),
            r#""observation""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Error).unwrap(),
            r#""error""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Progress).unwrap(),
            r#""progress""#
        );
    }

    #[test]
    fn test_message_kind_deserialize_roundtrip() {
        for kind in [
            MessageKind::Message,
            MessageKind::Thought,
            MessageKind::Action,
            MessageKind::Observation,
            MessageKind::Error,
            MessageKind::Progress,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let deserialized: MessageKind = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, kind);
        }
    }

    // --- LeadMessage tests ---

    #[test]
    fn test_lead_message_new() {
        let msg = LeadMessage::new(MessageRole::User, MessageKind::Message, "hello");
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.kind, MessageKind::Message);
    }

    #[test]
    fn test_lead_message_serde_roundtrip() {
        let msg = LeadMessage::new(MessageRole::Assistant, MessageKind::Thought, "thinking...");
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: LeadMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content, "thinking...");
        assert_eq!(deserialized.kind, MessageKind::Thought);
        assert_eq!(deserialized.timestamp, msg.timestamp);
    }

    // --- LeadState tests ---

    #[test]
    fn test_lead_state_default() {
        let state = LeadState::default();
        assert_eq!(state.status, LeadStatus::Idle);
        assert!(state.conversation.is_empty());
        assert_eq!(state.llm_call_count, 0);
        assert_eq!(state.estimated_tokens, 0);
        assert!(state.active_issue_numbers.is_empty());
        assert!(state.last_poll_at.is_none());
    }

    #[test]
    fn test_lead_state_serde_roundtrip() {
        let mut state = LeadState {
            status: LeadStatus::Orchestrating,
            llm_call_count: 42,
            estimated_tokens: 150000,
            active_issue_numbers: vec![10, 11],
            ..Default::default()
        };
        state.conversation.push(LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "start project",
        ));

        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: LeadState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.status, LeadStatus::Orchestrating);
        assert_eq!(deserialized.llm_call_count, 42);
        assert_eq!(deserialized.estimated_tokens, 150000);
        assert_eq!(deserialized.active_issue_numbers, vec![10, 11]);
        assert_eq!(deserialized.conversation.len(), 1);
        assert_eq!(deserialized.conversation[0].content, "start project");
    }
}

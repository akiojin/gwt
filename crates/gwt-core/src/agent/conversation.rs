//! Conversation history for agent mode

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl Message {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Conversation {
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub fn push(&mut self, role: MessageRole, content: impl Into<String>) {
        self.messages.push(Message::new(role, content));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_push() {
        let mut convo = Conversation::new();
        convo.push(MessageRole::User, "hello");
        assert_eq!(convo.messages.len(), 1);
        assert_eq!(convo.messages[0].content, "hello");
    }
}

//! Master agent implementation for agent mode

use crate::ai::{AIClient, AIError, ChatMessage};
use crate::config::ResolvedAISettings;

use super::conversation::{Conversation, MessageRole};

const DEFAULT_SYSTEM_PROMPT: &str = "You are the master agent. Analyze tasks and propose a plan.";

pub struct MasterAgent {
    client: AIClient,
    conversation: Conversation,
    system_prompt: String,
}

impl MasterAgent {
    pub fn new(settings: ResolvedAISettings) -> Result<Self, AIError> {
        let client = AIClient::new(settings)?;
        Ok(Self {
            client,
            conversation: Conversation::new(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        })
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn conversation_mut(&mut self) -> &mut Conversation {
        &mut self.conversation
    }

    pub fn set_system_prompt(&mut self, prompt: impl Into<String>) {
        self.system_prompt = prompt.into();
    }

    pub fn send_message(&mut self, user_message: &str) -> Result<String, AIError> {
        self.conversation
            .push(MessageRole::User, user_message.to_string());

        let mut messages: Vec<ChatMessage> = Vec::new();
        if !self.system_prompt.trim().is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: self.system_prompt.clone(),
            });
        }

        for message in &self.conversation.messages {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
            };
            messages.push(ChatMessage {
                role: role.to_string(),
                content: message.content.clone(),
            });
        }

        let response = self.client.create_response(messages)?;
        self.conversation
            .push(MessageRole::Assistant, response.clone());
        Ok(response)
    }
}

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

    /// Run the full Spec Kit workflow: clarify -> specify -> plan -> tasks
    ///
    /// Returns the generated spec content, plan content, and tasks content.
    pub fn run_speckit_workflow(
        &mut self,
        user_request: &str,
        repository_context: &str,
        claude_md: &str,
        existing_specs: &str,
        directory_tree: &str,
    ) -> Result<(String, String, String), AIError> {
        // Step 1: Generate specification
        let spec_content = crate::speckit::specify::run_specify(
            &self.client,
            user_request,
            repository_context,
            claude_md,
            existing_specs,
        )?;

        // Step 2: Generate plan
        let plan_content = crate::speckit::plan::run_plan(
            &self.client,
            &spec_content,
            repository_context,
            claude_md,
            directory_tree,
        )?;

        // Step 3: Generate tasks
        let tasks_content = crate::speckit::tasks::run_tasks(
            &self.client,
            &spec_content,
            &plan_content,
            repository_context,
        )?;

        Ok((spec_content, plan_content, tasks_content))
    }

    /// Parse a task plan from LLM response into structured task data.
    ///
    /// Expects JSON array format: `[{"name": "...", "description": "..."}, ...]`
    /// Retries up to 2 times on parse failure.
    pub fn parse_task_plan(&mut self, response: &str) -> Result<Vec<ParsedTask>, AIError> {
        // Try to find JSON array in the response
        if let Some(tasks) = try_parse_task_json(response) {
            return Ok(tasks);
        }

        // Retry: ask LLM to format as JSON
        let retry_prompt = format!(
            "The following response needs to be converted to a JSON array of tasks.\n\
             Each task should have \"name\" and \"description\" fields.\n\
             Output ONLY the JSON array, no other text.\n\n\
             Response:\n{}",
            response
        );
        let retry_response = self.send_message(&retry_prompt)?;
        if let Some(tasks) = try_parse_task_json(&retry_response) {
            return Ok(tasks);
        }

        // Second retry
        let retry2_response = self.send_message(
            "Please output ONLY a valid JSON array like: [{\"name\":\"task1\",\"description\":\"desc1\"}]"
        )?;
        if let Some(tasks) = try_parse_task_json(&retry2_response) {
            return Ok(tasks);
        }

        // Fallback: single task
        Ok(vec![ParsedTask {
            name: "Complete request".to_string(),
            description: response.to_string(),
        }])
    }
}

/// A parsed task from LLM output
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParsedTask {
    pub name: String,
    pub description: String,
}

/// Try to parse a JSON array of tasks from text
fn try_parse_task_json(text: &str) -> Option<Vec<ParsedTask>> {
    // Find JSON array in the text
    let trimmed = text.trim();

    // Try direct parse
    if let Ok(tasks) = serde_json::from_str::<Vec<ParsedTask>>(trimmed) {
        if !tasks.is_empty() {
            return Some(tasks);
        }
    }

    // Try to extract JSON array from markdown code block
    for block in trimmed.split("```") {
        let block = block.trim().trim_start_matches("json").trim();
        if block.starts_with('[') {
            if let Ok(tasks) = serde_json::from_str::<Vec<ParsedTask>>(block) {
                if !tasks.is_empty() {
                    return Some(tasks);
                }
            }
        }
    }

    None
}

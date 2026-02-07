//! Master agent implementation for agent mode

use crate::ai::{AIClient, AIError, ChatMessage};
use crate::config::ResolvedAISettings;
use tracing;

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

    /// Returns the cumulative estimated token count across all LLM calls
    pub fn estimated_tokens(&self) -> u64 {
        self.client.cumulative_tokens()
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

        let response = match self.client.create_response(messages) {
            Ok(resp) => {
                tracing::info!(
                    category = "agent.master.llm",
                    prompt_len = user_message.len(),
                    response_len = resp.len(),
                    "LLM call completed"
                );
                resp
            }
            Err(err) => {
                tracing::warn!(
                    category = "agent.master.llm",
                    prompt_len = user_message.len(),
                    error = %err,
                    "LLM call failed"
                );
                return Err(err);
            }
        };
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

    /// Estimate token count from messages.
    ///
    /// Uses a simple heuristic: English ~4 chars per token, Japanese ~1 char per token.
    pub fn estimate_token_count(&self) -> usize {
        estimate_token_count_for_messages(&self.conversation.messages)
    }

    /// Check if context compression is needed.
    ///
    /// Returns true if estimated tokens exceed 80% of max_context.
    /// When max_context is unknown, assumes 16k tokens.
    pub fn should_compress(&self, max_context: Option<usize>) -> bool {
        let max = max_context.unwrap_or(16_000);
        let estimated = self.estimate_token_count();
        estimated > max * 80 / 100
    }

    /// Compress conversation context by summarizing old messages.
    ///
    /// Keeps the most recent `keep_recent` messages in original form,
    /// summarizes the rest via LLM, and inserts the summary as a System message.
    pub fn compress_context(&mut self, keep_recent: usize) -> Result<(), AIError> {
        let total = self.conversation.messages.len();
        if total <= keep_recent {
            return Ok(());
        }

        let old_messages: Vec<String> = self.conversation.messages[..total - keep_recent]
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::User => "User",
                    MessageRole::Assistant => "Assistant",
                    MessageRole::System => "System",
                };
                format!("[{}] {}", role, m.content)
            })
            .collect();

        let summary_prompt = format!(
            "Summarize the following conversation history concisely, preserving key decisions, \
             completed tasks, and important context. Output only the summary.\n\n{}",
            old_messages.join("\n\n")
        );

        // Use a temporary conversation to avoid polluting history
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are a conversation summarizer. Produce a concise summary."
                    .to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: summary_prompt,
            },
        ];

        let summary = self.client.create_response(messages)?;

        // Rebuild conversation: summary + recent messages
        let recent: Vec<_> = self.conversation.messages[total - keep_recent..].to_vec();
        self.conversation.messages.clear();
        self.conversation.push(
            MessageRole::System,
            format!("[Context Summary] {}", summary),
        );
        self.conversation.messages.extend(recent);

        tracing::info!(
            category = "agent.master.compress",
            old_count = total,
            new_count = self.conversation.messages.len(),
            "Context compressed"
        );

        Ok(())
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

/// Estimate token count for a list of messages.
///
/// Heuristic: count ASCII chars / 4 + non-ASCII chars / 1.
fn estimate_token_count_for_messages(messages: &[super::conversation::Message]) -> usize {
    messages
        .iter()
        .map(|m| estimate_tokens_for_text(&m.content))
        .sum()
}

/// Estimate tokens for a text string.
fn estimate_tokens_for_text(text: &str) -> usize {
    let ascii_chars = text.chars().filter(|c| c.is_ascii()).count();
    let non_ascii_chars = text.chars().filter(|c| !c.is_ascii()).count();
    // English: ~4 chars per token, Japanese: ~1 char per token
    ascii_chars / 4 + non_ascii_chars
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_settings() -> ResolvedAISettings {
        ResolvedAISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "test-key".to_string(),
            model: "gpt-4o-mini".to_string(),
        }
    }

    #[test]
    fn test_estimated_tokens_initial_zero() {
        let agent = MasterAgent::new(make_settings()).unwrap();
        assert_eq!(agent.estimated_tokens(), 0);
    }

    #[test]
    fn test_try_parse_task_json_direct() {
        let json = r#"[{"name":"task1","description":"desc1"}]"#;
        let tasks = try_parse_task_json(json).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "task1");
    }

    #[test]
    fn test_try_parse_task_json_code_block() {
        let text = "```json\n[{\"name\":\"task1\",\"description\":\"desc1\"}]\n```";
        let tasks = try_parse_task_json(text).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_try_parse_task_json_invalid() {
        assert!(try_parse_task_json("not json").is_none());
    }

    #[test]
    fn test_try_parse_task_json_empty_array() {
        assert!(try_parse_task_json("[]").is_none());
    }

    #[test]
    fn test_set_system_prompt() {
        let mut agent = MasterAgent::new(make_settings()).unwrap();
        agent.set_system_prompt("Custom prompt");
        assert_eq!(agent.system_prompt, "Custom prompt");
    }

    #[test]
    fn test_estimate_tokens_for_text_english() {
        // "hello world" = 11 ASCII chars -> 11/4 = 2
        assert_eq!(estimate_tokens_for_text("hello world"), 2);
    }

    #[test]
    fn test_estimate_tokens_for_text_japanese() {
        // 3 Japanese chars -> 3 non-ASCII tokens
        assert_eq!(estimate_tokens_for_text("日本語"), 3);
    }

    #[test]
    fn test_estimate_tokens_for_text_mixed() {
        // "hello 世界" = 6 ASCII chars (incl space) / 4 = 1 + 2 non-ASCII = 3
        assert_eq!(estimate_tokens_for_text("hello 世界"), 3);
    }

    #[test]
    fn test_estimate_token_count_empty() {
        let agent = MasterAgent::new(make_settings()).unwrap();
        assert_eq!(agent.estimate_token_count(), 0);
    }

    #[test]
    fn test_should_compress_below_threshold() {
        let agent = MasterAgent::new(make_settings()).unwrap();
        assert!(!agent.should_compress(Some(16_000)));
    }

    #[test]
    fn test_should_compress_default_max() {
        let agent = MasterAgent::new(make_settings()).unwrap();
        // Empty conversation, should not compress
        assert!(!agent.should_compress(None));
    }

    #[test]
    fn test_compress_context_no_op_when_few_messages() {
        let mut agent = MasterAgent::new(make_settings()).unwrap();
        agent.conversation.push(MessageRole::User, "hello");
        agent.conversation.push(MessageRole::Assistant, "hi");
        // keep_recent=20 > 2 messages, so no-op
        assert!(agent.compress_context(20).is_ok());
        assert_eq!(agent.conversation.messages.len(), 2);
    }
}

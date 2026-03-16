#![allow(dead_code)]
//! Assistant Mode engine — LLM conversation loop with tool calling.

use std::path::PathBuf;

use gwt_core::ai::{
    AIClient, ChatCompletionsToolCallFunction, ChatCompletionsToolCallRef,
    ChatCompletionsToolMessage,
};
use serde::Serialize;
use tracing::{info, warn};

use crate::assistant_monitor::MonitorEvent;
use crate::assistant_tools;
use crate::state::AppState;

const MAX_TOOL_LOOP_ITERATIONS: usize = 10;
const ASSISTANT_MAX_TOKENS: u32 = 4096;
const ASSISTANT_TEMPERATURE: f32 = 0.3;

const SYSTEM_PROMPT: &str = r#"あなたは gwt (Git Worktree Manager) のアシスタントです。
プロアクティブな参謀として、ユーザーの開発作業を支援します。

## 行動指針
- 日本語で回答する
- 利用可能なツールを積極的に活用してリポジトリの状態を把握する
- gwt-spec (GitHub Issue) を重視し、仕様に基づいた提案を行う
- ユーザーが指示した作業について、自律的にツールを使って情報を収集し、的確な提案・実行を行う
- 不明点がある場合は推測せず、ユーザーに確認する

## ツール利用
- ファイル読み取り、grep、ディレクトリ一覧、git操作などのツールが利用可能
- コマンド実行ツールで任意のシェルコマンドを実行可能（30秒タイムアウト）
- エージェントペインへの入力送信・スクロールバック取得が可能
- gwt-spec Issue の取得・更新が可能
"#;

#[derive(Debug, Clone, Serialize)]
pub struct AssistantResponse {
    pub text: String,
    pub actions_taken: Vec<String>,
}

pub struct AssistantEngine {
    conversation: Vec<ChatCompletionsToolMessage>,
    system_prompt: String,
    project_path: PathBuf,
    window_label: String,
    pub llm_call_count: u64,
    pub estimated_tokens: u64,
}

impl AssistantEngine {
    pub fn new(project_path: PathBuf, window_label: String) -> Self {
        let conversation = vec![ChatCompletionsToolMessage {
            role: "system".to_string(),
            content: Some(SYSTEM_PROMPT.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];

        Self {
            conversation,
            system_prompt: SYSTEM_PROMPT.to_string(),
            project_path,
            window_label,
            llm_call_count: 0,
            estimated_tokens: 0,
        }
    }

    /// Get a copy of the conversation for serialization.
    pub fn conversation(&self) -> &[ChatCompletionsToolMessage] {
        &self.conversation
    }

    /// Handle a user message: add to conversation, run LLM loop, return response.
    pub fn handle_user_message(
        &mut self,
        input: &str,
        state: &AppState,
    ) -> Result<AssistantResponse, String> {
        self.conversation.push(ChatCompletionsToolMessage {
            role: "user".to_string(),
            content: Some(input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        self.run_llm_loop(state)
    }

    /// Handle a batch of monitor events: summarize changes, optionally call LLM.
    pub fn handle_monitor_batch(
        &mut self,
        events: Vec<MonitorEvent>,
        state: &AppState,
    ) -> Result<Option<AssistantResponse>, String> {
        if events.is_empty() {
            return Ok(None);
        }

        // Build a summary of monitor events
        let mut summaries = Vec::new();
        for event in &events {
            match event {
                MonitorEvent::SnapshotChanged(snapshot) => {
                    let pane_count = snapshot.panes.len();
                    let branch = &snapshot.git.branch;
                    let uncommitted = snapshot.git.uncommitted_count;
                    summaries.push(format!(
                        "[Monitor] {} panes active, branch={}, uncommitted={}",
                        pane_count, branch, uncommitted
                    ));
                }
            }
        }

        let summary = summaries.join("\n");
        self.conversation.push(ChatCompletionsToolMessage {
            role: "user".to_string(),
            content: Some(format!(
                "[System Monitor Update]\n{}\n\nAnalyze the current state and report any issues or suggestions.",
                summary
            )),
            tool_calls: None,
            tool_call_id: None,
        });

        let response = self.run_llm_loop(state)?;
        Ok(Some(response))
    }

    /// Run the LLM tool-use loop: call LLM, execute tool calls, repeat until
    /// the LLM returns a text response (no tool calls) or max iterations reached.
    fn run_llm_loop(&mut self, state: &AppState) -> Result<AssistantResponse, String> {
        let tools = assistant_tools::assistant_tool_definitions();
        let mut actions_taken = Vec::new();

        for iteration in 0..MAX_TOOL_LOOP_ITERATIONS {
            let ai_client = build_ai_client(state)?;
            let messages = self.conversation.clone();
            let tools_clone = tools.clone();

            // Run the blocking AI call on a blocking thread
            let response = std::thread::spawn(move || {
                ai_client.create_chat_completion_with_tools(
                    messages,
                    tools_clone,
                    ASSISTANT_MAX_TOKENS,
                    ASSISTANT_TEMPERATURE,
                )
            })
            .join()
            .map_err(|_| "LLM call panicked".to_string())?
            .map_err(|e| format!("LLM call failed: {}", e))?;

            self.llm_call_count += 1;
            if let Some(tokens) = response.usage_tokens {
                self.estimated_tokens += tokens;
            }

            if response.tool_calls.is_empty() {
                // No tool calls — this is the final text response
                self.conversation.push(ChatCompletionsToolMessage {
                    role: "assistant".to_string(),
                    content: Some(response.text.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });

                return Ok(AssistantResponse {
                    text: response.text,
                    actions_taken,
                });
            }

            // Build tool_calls references for the assistant message
            let tool_call_refs: Vec<ChatCompletionsToolCallRef> = response
                .tool_calls
                .iter()
                .map(|tc| ChatCompletionsToolCallRef {
                    id: tc.call_id.clone().unwrap_or_default(),
                    call_type: "function".to_string(),
                    function: ChatCompletionsToolCallFunction {
                        name: tc.name.clone(),
                        arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                    },
                })
                .collect();

            // Add the assistant message with tool_calls
            self.conversation.push(ChatCompletionsToolMessage {
                role: "assistant".to_string(),
                content: if response.text.is_empty() {
                    None
                } else {
                    Some(response.text.clone())
                },
                tool_calls: Some(tool_call_refs),
                tool_call_id: None,
            });

            // Execute each tool call and add tool results
            let project_path = self.project_path.to_string_lossy().to_string();
            for tc in &response.tool_calls {
                let tool_result = assistant_tools::execute_assistant_tool(
                    tc,
                    state,
                    &self.window_label,
                    &project_path,
                );

                let result_text = match &tool_result {
                    Ok(text) => {
                        actions_taken.push(format!("{}(ok)", tc.name));
                        text.clone()
                    }
                    Err(err) => {
                        actions_taken.push(format!("{}(error)", tc.name));
                        format!("Error: {}", err)
                    }
                };

                let call_id = tc.call_id.clone().unwrap_or_default();
                self.conversation.push(ChatCompletionsToolMessage {
                    role: "tool".to_string(),
                    content: Some(truncate_tool_result(&result_text)),
                    tool_calls: None,
                    tool_call_id: Some(call_id),
                });
            }

            info!(
                iteration = iteration + 1,
                tool_count = response.tool_calls.len(),
                "Assistant tool loop iteration"
            );
        }

        warn!("Assistant tool loop reached max iterations");
        Ok(AssistantResponse {
            text: "Maximum tool call iterations reached. Please try again with a more specific request.".to_string(),
            actions_taken,
        })
    }
}

/// Truncate tool results to avoid exceeding context limits.
fn truncate_tool_result(text: &str) -> String {
    const MAX_TOOL_RESULT_CHARS: usize = 32_000;
    if text.len() <= MAX_TOOL_RESULT_CHARS {
        text.to_string()
    } else {
        let truncated = &text[..MAX_TOOL_RESULT_CHARS];
        format!("{}...\n[truncated, {} total chars]", truncated, text.len())
    }
}

/// Build an AIClient from the current profile settings.
fn build_ai_client(_state: &AppState) -> Result<AIClient, String> {
    let profiles = gwt_core::config::ProfilesConfig::load()
        .map_err(|e| format!("Failed to load profiles: {}", e))?;

    let ai = profiles.resolve_active_ai_settings();
    let settings = ai
        .resolved
        .ok_or_else(|| "AI is not configured. Please configure AI settings first.".to_string())?;

    AIClient::new(settings).map_err(|e| format!("Failed to create AI client: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_tool_result_short() {
        let text = "hello world";
        assert_eq!(truncate_tool_result(text), "hello world");
    }

    #[test]
    fn test_truncate_tool_result_long() {
        let text = "a".repeat(40_000);
        let result = truncate_tool_result(&text);
        assert!(result.len() < 40_000);
        assert!(result.contains("[truncated"));
    }

    #[test]
    fn test_assistant_engine_new() {
        let engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        assert_eq!(engine.llm_call_count, 0);
        assert_eq!(engine.estimated_tokens, 0);
        // Conversation should have system message
        assert_eq!(engine.conversation.len(), 1);
        assert_eq!(engine.conversation[0].role, "system");
    }
}

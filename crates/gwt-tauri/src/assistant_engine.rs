#![allow(dead_code)]
//! Assistant Mode engine — LLM conversation loop with tool calling.

use std::path::{Path, PathBuf};

use gwt_core::ai::{
    ChatCompletionsToolCallFunction, ChatCompletionsToolCallRef, ChatCompletionsToolMessage,
};
use serde::Serialize;
use tracing::{info, warn};

use crate::assistant_monitor::MonitorEvent;
use crate::assistant_tools::{self, AssistantToolMode};
use crate::state::AppState;

const MAX_TOOL_LOOP_ITERATIONS: usize = 10;
const ASSISTANT_MAX_TOKENS: u32 = 4096;
const ASSISTANT_TEMPERATURE: f32 = 0.3;
const MAX_CONVERSATION_MESSAGES: usize = 50;
const STARTUP_REPORT_PROMPT: &str = r#"これは Assistant の自律起動です。
この応答では、開いている project 全体を read-only で調査し、最初の起動レポートを返してください。

必須:
- `git_status` と `git_log` を確認する
- `list_panes` で現在の pane を確認する
- `list_issues` と `list_pull_requests` で GitHub の open 状態を確認する
- pane が存在する場合のみ、必要な pane に対して `capture_scrollback_tail` を使って状況を補足してよい

禁止:
- `run_command` を使わない
- `send_keys_to_pane` を使わない
- `upsert_spec_issue` を使わない
- ファイル、Issue、PR、pane に対する変更を行わない

出力形式:
## 現在の状況
- ...

## 検出事項
- ...

## 次アクション候補
- ...

## 確認事項
- 本当に必要な場合のみ 1 件
"#;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantStartupStatus {
    Idle,
    Analyzing,
    Ready,
    Failed,
}

impl AssistantStartupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Analyzing => "analyzing",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone)]
pub struct AssistantEngine {
    conversation: Vec<ChatCompletionsToolMessage>,
    project_path: PathBuf,
    window_label: String,
    startup_status: AssistantStartupStatus,
    startup_summary_ready: bool,
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
            project_path,
            window_label,
            startup_status: AssistantStartupStatus::Idle,
            startup_summary_ready: false,
            llm_call_count: 0,
            estimated_tokens: 0,
        }
    }

    /// Get a copy of the conversation for serialization.
    pub fn conversation(&self) -> &[ChatCompletionsToolMessage] {
        &self.conversation
    }

    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    pub fn startup_status(&self) -> AssistantStartupStatus {
        self.startup_status
    }

    pub fn startup_summary_ready(&self) -> bool {
        self.startup_summary_ready
    }

    pub fn handle_startup(&mut self, state: &AppState) -> Result<(), String> {
        self.handle_startup_with_cancel(state, || false).map(|_| ())
    }

    pub fn handle_startup_with_cancel<F>(
        &mut self,
        state: &AppState,
        should_cancel: F,
    ) -> Result<bool, String>
    where
        F: Fn() -> bool,
    {
        if self.startup_summary_ready {
            return Ok(true);
        }

        let base_len = self.conversation.len();
        self.startup_status = AssistantStartupStatus::Analyzing;
        self.startup_summary_ready = false;
        self.conversation.push(ChatCompletionsToolMessage {
            role: "system".to_string(),
            content: Some(STARTUP_REPORT_PROMPT.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        match self.run_llm_loop(state, AssistantToolMode::ReadOnly, &should_cancel) {
            Ok(Some(_)) => {
                let summary = self
                    .conversation
                    .last()
                    .and_then(|message| message.content.clone())
                    .unwrap_or_default();
                self.finish_startup_transcript(base_len, &summary);
                self.startup_status = AssistantStartupStatus::Ready;
                self.startup_summary_ready = true;
                Ok(true)
            }
            Ok(None) => {
                self.conversation.truncate(base_len);
                self.startup_status = AssistantStartupStatus::Idle;
                self.startup_summary_ready = false;
                Ok(false)
            }
            Err(err) => {
                self.finish_startup_transcript(base_len, &format!("自律起動に失敗しました: {err}"));
                self.startup_status = AssistantStartupStatus::Failed;
                self.startup_summary_ready = false;
                Ok(true)
            }
        }
    }

    pub fn push_visible_assistant_message(&mut self, content: impl Into<String>) {
        self.conversation.push(ChatCompletionsToolMessage {
            role: "assistant".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    #[cfg(test)]
    pub fn push_hidden_system_message_for_test(&mut self, content: impl Into<String>) {
        self.conversation.push(ChatCompletionsToolMessage {
            role: "system".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    pub fn apply_cached_startup_summary(&mut self, summary: impl Into<String>) {
        let summary = summary.into();
        let base_len = self.conversation.len();
        self.finish_startup_transcript(base_len, &summary);
        self.startup_status = AssistantStartupStatus::Ready;
        self.startup_summary_ready = true;
    }

    pub fn apply_startup_failure_message(&mut self, message: impl Into<String>) {
        let message = message.into();
        let base_len = self.conversation.len();
        self.finish_startup_transcript(base_len, &message);
        self.startup_status = AssistantStartupStatus::Failed;
        self.startup_summary_ready = false;
    }

    fn finish_startup_transcript(&mut self, base_len: usize, summary: &str) {
        self.conversation.truncate(base_len);
        self.conversation.push(ChatCompletionsToolMessage {
            role: "assistant".to_string(),
            content: Some(summary.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    /// Handle a user message: add to conversation, run LLM loop, return response.
    pub fn handle_user_message(
        &mut self,
        input: &str,
        state: &AppState,
    ) -> Result<AssistantResponse, String> {
        self.handle_user_message_with_cancel(input, state, || false)?
            .ok_or_else(|| "Assistant run cancelled".to_string())
    }

    pub fn handle_user_message_with_cancel<F>(
        &mut self,
        input: &str,
        state: &AppState,
        should_cancel: F,
    ) -> Result<Option<AssistantResponse>, String>
    where
        F: Fn() -> bool,
    {
        if should_cancel() {
            return Ok(None);
        }

        self.conversation.push(ChatCompletionsToolMessage {
            role: "user".to_string(),
            content: Some(input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        self.run_llm_loop(state, AssistantToolMode::FullAccess, &should_cancel)
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

        let response = self
            .run_llm_loop(state, AssistantToolMode::FullAccess, &|| false)?
            .ok_or_else(|| "Assistant monitor run cancelled".to_string())?;
        Ok(Some(response))
    }

    /// Prune conversation to stay within the sliding window limit.
    /// Keeps the system prompt (index 0) and the most recent messages.
    /// Ensures the cut point does not orphan tool result messages.
    fn prune_conversation(&mut self) {
        if self.conversation.len() <= MAX_CONVERSATION_MESSAGES {
            return;
        }

        // Always keep message[0] (system prompt).
        // Keep the last (MAX_CONVERSATION_MESSAGES - 1) messages from the tail.
        let keep_tail = MAX_CONVERSATION_MESSAGES - 1;
        let mut cut = self.conversation.len() - keep_tail;

        // Ensure cut point doesn't orphan a "tool" role message (tool result without
        // the preceding assistant tool_calls message). Walk forward from the cut point
        // to find the first message that is NOT a "tool" role.
        while cut < self.conversation.len() && self.conversation[cut].role == "tool" {
            cut += 1;
        }

        if cut >= self.conversation.len() {
            // Edge case: all remaining messages are tool results — keep everything
            return;
        }

        // Build pruned conversation: system prompt + messages from cut onward
        let mut pruned = Vec::with_capacity(1 + self.conversation.len() - cut);
        pruned.push(self.conversation[0].clone());
        pruned.extend_from_slice(&self.conversation[cut..]);
        self.conversation = pruned;
    }

    /// Run the LLM tool-use loop: call LLM, execute tool calls, repeat until
    /// the LLM returns a text response (no tool calls) or max iterations reached.
    fn run_llm_loop(
        &mut self,
        state: &AppState,
        tool_mode: AssistantToolMode,
        should_cancel: &impl Fn() -> bool,
    ) -> Result<Option<AssistantResponse>, String> {
        if should_cancel() {
            return Ok(None);
        }

        let tools = assistant_tools::assistant_tool_definitions(tool_mode);
        let mut actions_taken = Vec::new();

        // Load AI settings once before the loop to avoid repeated config reads.
        let ai_settings = resolve_ai_settings()?;

        // Prune conversation before sending to LLM to avoid context window overflow.
        self.prune_conversation();

        for iteration in 0..MAX_TOOL_LOOP_ITERATIONS {
            if should_cancel() {
                return Ok(None);
            }

            let messages = self.conversation.clone();
            let tools_clone = tools.clone();
            let settings = ai_settings.clone();

            // Run the blocking AI call on a separate thread.
            // AIClient is created per-thread because it cannot be sent across threads
            // (contains non-Send AtomicU64), but the settings are loaded only once.
            let response = std::thread::spawn(move || -> Result<_, String> {
                let client = gwt_core::ai::AIClient::new(settings)
                    .map_err(|e| format!("Failed to create AI client: {}", e))?;
                client
                    .create_chat_completion_with_tools(
                        messages,
                        tools_clone,
                        ASSISTANT_MAX_TOKENS,
                        ASSISTANT_TEMPERATURE,
                    )
                    .map_err(|e| format!("LLM call failed: {}", e))
            })
            .join()
            .map_err(|_| "LLM call panicked".to_string())??;

            self.llm_call_count += 1;
            if let Some(tokens) = response.usage_tokens {
                self.estimated_tokens += tokens;
            }

            if should_cancel() {
                return Ok(None);
            }

            if response.tool_calls.is_empty() {
                // No tool calls — this is the final text response
                if should_cancel() {
                    return Ok(None);
                }
                self.conversation.push(ChatCompletionsToolMessage {
                    role: "assistant".to_string(),
                    content: Some(response.text.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });

                return Ok(Some(AssistantResponse {
                    text: response.text,
                    actions_taken,
                }));
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
                if should_cancel() {
                    return Ok(None);
                }

                let tool_result = assistant_tools::execute_assistant_tool(
                    tc,
                    state,
                    &self.window_label,
                    &project_path,
                    tool_mode,
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
        Ok(Some(AssistantResponse {
            text: "Maximum tool call iterations reached. Please try again with a more specific request.".to_string(),
            actions_taken,
        }))
    }
}

/// Truncate tool results to avoid exceeding context limits.
/// Uses char boundary to avoid panicking on multi-byte UTF-8 characters.
fn truncate_tool_result(text: &str) -> String {
    const MAX_TOOL_RESULT_CHARS: usize = 32_000;
    if text.len() <= MAX_TOOL_RESULT_CHARS {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
        format!("{}...\n[truncated, {} total chars]", truncated, text.len())
    }
}

/// Resolve AI settings from the current profile (loaded once, reused across loop iterations).
fn resolve_ai_settings() -> Result<gwt_core::config::ResolvedAISettings, String> {
    let profiles = gwt_core::config::ProfilesConfig::load()
        .map_err(|e| format!("Failed to load profiles: {}", e))?;

    let ai = profiles.resolve_active_ai_settings();
    ai.resolved
        .ok_or_else(|| "AI is not configured. Please configure AI settings first.".to_string())
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
    fn test_truncate_tool_result_multibyte() {
        // Japanese text: each char is 3 bytes in UTF-8
        let text = "あ".repeat(40_000);
        let result = truncate_tool_result(&text);
        assert!(result.contains("[truncated"));
        // Should not panic
    }

    #[test]
    fn test_assistant_engine_new() {
        let engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        assert_eq!(engine.llm_call_count, 0);
        assert_eq!(engine.estimated_tokens, 0);
        assert_eq!(engine.startup_status(), AssistantStartupStatus::Idle);
        assert!(!engine.startup_summary_ready());
        assert_eq!(engine.project_path(), Path::new("/repo"));
        // Conversation should have system message
        assert_eq!(engine.conversation.len(), 1);
        assert_eq!(engine.conversation[0].role, "system");
    }

    #[test]
    fn test_finish_startup_transcript_removes_startup_prompt_and_tool_messages() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        let base_len = engine.conversation.len();

        engine.conversation.push(ChatCompletionsToolMessage {
            role: "system".to_string(),
            content: Some(STARTUP_REPORT_PROMPT.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });
        engine.conversation.push(ChatCompletionsToolMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![]),
            tool_call_id: None,
        });
        engine.conversation.push(ChatCompletionsToolMessage {
            role: "tool".to_string(),
            content: Some("tool-result".to_string()),
            tool_calls: None,
            tool_call_id: Some("call-1".to_string()),
        });

        engine.finish_startup_transcript(base_len, "startup summary");

        assert_eq!(engine.conversation.len(), base_len + 1);
        assert_eq!(engine.conversation[0].role, "system");
        assert_eq!(
            engine
                .conversation
                .last()
                .and_then(|message| message.content.as_deref()),
            Some("startup summary")
        );
        assert!(!engine
            .conversation
            .iter()
            .any(|message| message.content.as_deref() == Some(STARTUP_REPORT_PROMPT)));
        assert!(!engine
            .conversation
            .iter()
            .any(|message| message.role == "tool"));
    }

    #[test]
    fn test_apply_cached_startup_summary_marks_engine_ready() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());

        engine.apply_cached_startup_summary("cached");

        assert_eq!(engine.startup_status(), AssistantStartupStatus::Ready);
        assert!(engine.startup_summary_ready());
        assert_eq!(
            engine
                .conversation
                .last()
                .and_then(|message| message.content.as_deref()),
            Some("cached")
        );
    }

    fn make_msg(role: &str, content: &str) -> ChatCompletionsToolMessage {
        ChatCompletionsToolMessage {
            role: role.to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn test_prune_conversation_under_limit() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        // Add messages up to exactly MAX_CONVERSATION_MESSAGES (system + 49 user)
        for i in 0..(MAX_CONVERSATION_MESSAGES - 1) {
            engine
                .conversation
                .push(make_msg("user", &format!("msg {}", i)));
        }
        assert_eq!(engine.conversation.len(), MAX_CONVERSATION_MESSAGES);
        engine.prune_conversation();
        // Should not change
        assert_eq!(engine.conversation.len(), MAX_CONVERSATION_MESSAGES);
    }

    #[test]
    fn test_prune_conversation_over_limit() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        // Add 60 user messages (total = 61 with system)
        for i in 0..60 {
            engine
                .conversation
                .push(make_msg("user", &format!("msg {}", i)));
        }
        assert_eq!(engine.conversation.len(), 61);
        engine.prune_conversation();
        // Should be MAX_CONVERSATION_MESSAGES
        assert_eq!(engine.conversation.len(), MAX_CONVERSATION_MESSAGES);
        // First message is always system
        assert_eq!(engine.conversation[0].role, "system");
        // Last message should be the most recent
        assert_eq!(
            engine.conversation.last().unwrap().content.as_deref(),
            Some("msg 59")
        );
    }

    #[test]
    fn test_prune_conversation_tool_boundary() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        // Build a conversation that would cut in the middle of tool results:
        // [system, user*10, assistant(with tool_calls), tool, tool, user*40]
        // Total: 1 + 10 + 1 + 2 + 40 = 54
        for i in 0..10 {
            engine
                .conversation
                .push(make_msg("user", &format!("early {}", i)));
        }
        // Assistant message with tool_calls
        engine.conversation.push(ChatCompletionsToolMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![]),
            tool_call_id: None,
        });
        // Tool results
        engine.conversation.push(ChatCompletionsToolMessage {
            role: "tool".to_string(),
            content: Some("result1".to_string()),
            tool_calls: None,
            tool_call_id: Some("call1".to_string()),
        });
        engine.conversation.push(ChatCompletionsToolMessage {
            role: "tool".to_string(),
            content: Some("result2".to_string()),
            tool_calls: None,
            tool_call_id: Some("call2".to_string()),
        });
        for i in 0..40 {
            engine
                .conversation
                .push(make_msg("user", &format!("late {}", i)));
        }
        assert_eq!(engine.conversation.len(), 54);
        engine.prune_conversation();
        // First should be system
        assert_eq!(engine.conversation[0].role, "system");
        // No orphaned tool messages at the start of the kept window
        assert_ne!(engine.conversation[1].role, "tool");
        // Should have been pruned
        assert!(engine.conversation.len() <= MAX_CONVERSATION_MESSAGES);
    }

    #[test]
    fn handle_user_message_with_cancel_returns_none_before_mutating_conversation() {
        let mut engine = AssistantEngine::new(PathBuf::from("/repo"), "main".to_string());
        let state = crate::state::AppState::new();

        let result = engine
            .handle_user_message_with_cancel("hello", &state, || true)
            .expect("cancelled run should not error");

        assert!(result.is_none());
        assert_eq!(engine.conversation.len(), 1);
        assert_eq!(engine.conversation[0].role, "system");
    }
}

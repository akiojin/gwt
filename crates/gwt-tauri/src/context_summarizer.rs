// Context summarization and compression for Lead/Coordinator conversations.
// Developer agents handle their own context (built into Claude/Codex/Gemini).
#![allow(dead_code)]

use gwt_core::agent::conversation::MessageRole;
use gwt_core::agent::lead::{LeadMessage, MessageKind};

/// Context window usage threshold (80%) that triggers summarization
const CONTEXT_THRESHOLD_RATIO: f64 = 0.80;

/// Approximate max tokens for typical models
const DEFAULT_MAX_TOKENS: u64 = 200_000;

/// Number of recent messages to preserve during compaction
const PRESERVE_LAST_N: usize = 5;

/// Check whether context usage has reached the threshold for summarization.
///
/// Returns true if `estimated_tokens / max_tokens >= CONTEXT_THRESHOLD_RATIO`.
pub fn should_summarize(estimated_tokens: u64, max_tokens: u64) -> bool {
    if max_tokens == 0 {
        return false;
    }
    let ratio = estimated_tokens as f64 / max_tokens as f64;
    ratio >= CONTEXT_THRESHOLD_RATIO
}

/// Estimate token count for a text string using a simple heuristic.
///
/// Uses chars / 4 as a rough approximation. This avoids calling the LLM
/// just for token counting.
pub fn estimate_message_tokens(text: &str) -> u64 {
    (text.chars().count() as u64) / 4
}

/// Build a summarization prompt from the conversation history.
///
/// Includes only completed task summaries and key decisions.
/// The resulting prompt is intended to be sent to an LLM for summarization.
pub fn build_summary_prompt(messages: &[LeadMessage]) -> String {
    let mut prompt = String::from(
        "Summarize this conversation history for context continuity. \
         Focus on: decisions made, tasks completed, current status, pending items.\n\n",
    );

    for msg in messages {
        let role_label = match msg.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
            MessageRole::System => "System",
        };
        let kind_label = match msg.kind {
            MessageKind::Message => "message",
            MessageKind::Thought => "thought",
            MessageKind::Action => "action",
            MessageKind::Observation => "observation",
            MessageKind::Error => "error",
            MessageKind::Progress => "progress",
        };
        prompt.push_str(&format!(
            "[{}/{}]: {}\n",
            role_label, kind_label, msg.content
        ));
    }

    prompt
}

/// Compact a conversation by replacing old messages with a summary.
///
/// Removes old messages (keeping the last `PRESERVE_LAST_N` entries) and
/// inserts a System message at the beginning containing the summary.
/// This reduces overall message count while preserving recent context.
pub fn compact_conversation(messages: &mut Vec<LeadMessage>, summary: &str) {
    if messages.len() <= PRESERVE_LAST_N {
        // Nothing to compact; just prepend the summary
        messages.insert(
            0,
            LeadMessage::new(
                MessageRole::System,
                MessageKind::Message,
                format!("[Context Summary]: {}", summary),
            ),
        );
        return;
    }

    // Keep only the last PRESERVE_LAST_N messages
    let preserved: Vec<LeadMessage> = messages
        .iter()
        .rev()
        .take(PRESERVE_LAST_N)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    messages.clear();

    // Insert summary as the first message
    messages.push(LeadMessage::new(
        MessageRole::System,
        MessageKind::Message,
        format!("[Context Summary]: {}", summary),
    ));

    // Re-add preserved messages
    messages.extend(preserved);
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- should_summarize tests ---

    #[test]
    fn should_summarize_returns_false_at_79_percent() {
        // 79% usage → below threshold
        assert!(!should_summarize(158_000, 200_000));
    }

    #[test]
    fn should_summarize_returns_true_at_80_percent() {
        // Exactly 80% → at threshold
        assert!(should_summarize(160_000, 200_000));
    }

    #[test]
    fn should_summarize_returns_true_at_100_percent() {
        assert!(should_summarize(200_000, 200_000));
    }

    #[test]
    fn should_summarize_returns_false_when_max_tokens_zero() {
        // Avoid division by zero
        assert!(!should_summarize(100, 0));
    }

    #[test]
    fn should_summarize_returns_false_at_zero_usage() {
        assert!(!should_summarize(0, 200_000));
    }

    #[test]
    fn should_summarize_returns_true_above_100_percent() {
        // Over budget
        assert!(should_summarize(250_000, 200_000));
    }

    // --- estimate_message_tokens tests ---

    #[test]
    fn estimate_message_tokens_gives_reasonable_estimate() {
        // 100 chars → ~25 tokens
        let text = "a".repeat(100);
        let tokens = estimate_message_tokens(&text);
        assert_eq!(tokens, 25);
    }

    #[test]
    fn estimate_message_tokens_empty_string() {
        assert_eq!(estimate_message_tokens(""), 0);
    }

    #[test]
    fn estimate_message_tokens_short_text() {
        // "hello" = 5 chars → 1 token (5/4 = 1)
        assert_eq!(estimate_message_tokens("hello"), 1);
    }

    #[test]
    fn estimate_message_tokens_handles_multibyte() {
        // Japanese chars: each is 1 char, so 4 chars → 1 token
        let text = "あいうえ";
        assert_eq!(estimate_message_tokens(text), 1);
    }

    // --- build_summary_prompt tests ---

    #[test]
    fn build_summary_prompt_includes_instruction_text() {
        let messages = vec![LeadMessage::new(
            MessageRole::User,
            MessageKind::Message,
            "implement login",
        )];
        let prompt = build_summary_prompt(&messages);
        assert!(prompt.contains("Summarize this conversation history"));
        assert!(prompt.contains("decisions made"));
        assert!(prompt.contains("tasks completed"));
        assert!(prompt.contains("current status"));
        assert!(prompt.contains("pending items"));
    }

    #[test]
    fn build_summary_prompt_includes_message_content() {
        let messages = vec![
            LeadMessage::new(MessageRole::User, MessageKind::Message, "implement login"),
            LeadMessage::new(
                MessageRole::Assistant,
                MessageKind::Thought,
                "analyzing requirements",
            ),
        ];
        let prompt = build_summary_prompt(&messages);
        assert!(prompt.contains("implement login"));
        assert!(prompt.contains("analyzing requirements"));
    }

    #[test]
    fn build_summary_prompt_includes_role_and_kind_labels() {
        let messages = vec![
            LeadMessage::new(MessageRole::User, MessageKind::Message, "hello"),
            LeadMessage::new(MessageRole::Assistant, MessageKind::Action, "tool call"),
            LeadMessage::new(MessageRole::System, MessageKind::Observation, "result"),
        ];
        let prompt = build_summary_prompt(&messages);
        assert!(prompt.contains("[User/message]"));
        assert!(prompt.contains("[Assistant/action]"));
        assert!(prompt.contains("[System/observation]"));
    }

    #[test]
    fn build_summary_prompt_empty_messages() {
        let prompt = build_summary_prompt(&[]);
        assert!(prompt.contains("Summarize this conversation history"));
        // No message lines appended
        assert!(!prompt.contains("[User/"));
        assert!(!prompt.contains("[Assistant/"));
    }

    // --- compact_conversation tests ---

    #[test]
    fn compact_conversation_preserves_last_5_messages() {
        let mut messages: Vec<LeadMessage> = (0..10)
            .map(|i| {
                LeadMessage::new(
                    MessageRole::User,
                    MessageKind::Message,
                    format!("message {}", i),
                )
            })
            .collect();

        compact_conversation(&mut messages, "Summary of early conversation");

        // 1 summary + 5 preserved = 6
        assert_eq!(messages.len(), 6);

        // Last 5 original messages (indices 5-9) should be preserved
        assert_eq!(messages[1].content, "message 5");
        assert_eq!(messages[2].content, "message 6");
        assert_eq!(messages[3].content, "message 7");
        assert_eq!(messages[4].content, "message 8");
        assert_eq!(messages[5].content, "message 9");
    }

    #[test]
    fn compact_conversation_inserts_summary_message_at_start() {
        let mut messages: Vec<LeadMessage> = (0..10)
            .map(|i| {
                LeadMessage::new(
                    MessageRole::User,
                    MessageKind::Message,
                    format!("message {}", i),
                )
            })
            .collect();

        compact_conversation(&mut messages, "All tasks completed, auth feature done");

        assert_eq!(messages[0].role, MessageRole::System);
        assert!(messages[0]
            .content
            .contains("All tasks completed, auth feature done"));
        assert!(messages[0].content.contains("[Context Summary]"));
    }

    #[test]
    fn compact_conversation_reduces_total_message_count() {
        let mut messages: Vec<LeadMessage> = (0..20)
            .map(|i| {
                LeadMessage::new(
                    MessageRole::User,
                    MessageKind::Message,
                    format!("message {}", i),
                )
            })
            .collect();

        let original_count = messages.len();
        compact_conversation(&mut messages, "summary");

        // 1 summary + 5 preserved = 6, which is less than 20
        assert!(messages.len() < original_count);
        assert_eq!(messages.len(), 6);
    }

    #[test]
    fn compact_conversation_with_fewer_than_preserve_count() {
        let mut messages = vec![
            LeadMessage::new(MessageRole::User, MessageKind::Message, "hello"),
            LeadMessage::new(MessageRole::Assistant, MessageKind::Message, "hi"),
        ];

        compact_conversation(&mut messages, "brief summary");

        // All original messages preserved + summary prepended = 3
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, MessageRole::System);
        assert!(messages[0].content.contains("brief summary"));
        assert_eq!(messages[1].content, "hello");
        assert_eq!(messages[2].content, "hi");
    }

    #[test]
    fn compact_conversation_with_exactly_preserve_count() {
        let mut messages: Vec<LeadMessage> = (0..5)
            .map(|i| {
                LeadMessage::new(
                    MessageRole::User,
                    MessageKind::Message,
                    format!("msg {}", i),
                )
            })
            .collect();

        compact_conversation(&mut messages, "summary for 5 messages");

        // 5 messages + 1 summary = 6 (all preserved since len <= PRESERVE_LAST_N)
        assert_eq!(messages.len(), 6);
        assert_eq!(messages[0].role, MessageRole::System);
    }

    #[test]
    fn compact_conversation_summary_has_system_role() {
        let mut messages: Vec<LeadMessage> = (0..8)
            .map(|i| {
                LeadMessage::new(
                    MessageRole::User,
                    MessageKind::Message,
                    format!("item {}", i),
                )
            })
            .collect();

        compact_conversation(&mut messages, "test summary");

        assert_eq!(messages[0].role, MessageRole::System);
        assert_eq!(messages[0].kind, MessageKind::Message);
    }

    // --- Constants tests ---

    #[test]
    fn context_threshold_ratio_is_80_percent() {
        assert!((CONTEXT_THRESHOLD_RATIO - 0.80).abs() < f64::EPSILON);
    }

    #[test]
    fn default_max_tokens_is_200k() {
        assert_eq!(DEFAULT_MAX_TOKENS, 200_000);
    }

    #[test]
    fn preserve_last_n_is_5() {
        assert_eq!(PRESERVE_LAST_N, 5);
    }
}

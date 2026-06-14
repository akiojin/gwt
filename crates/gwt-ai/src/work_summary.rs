//! AI polish for Workspace rail summaries (SPEC-3075 FR-006).
//!
//! Given each Workspace's structured meta — owner plus raw signals (recent
//! non-merge commit subjects, PR title) — the AI produces a concise, human
//! "what work was running" purpose. This is the optional top-priority source of
//! the rail summary: it cleans up merge/release commit noise and conventional
//! commit prefixes that the non-AI fallback (PR title / commit subject) cannot.
//!
//! No session transcript is sent — only the structured meta. Callers gate this
//! behind `AISettings.summary_enabled && AISettings.is_enabled()` and fall back
//! to the non-AI chain when AI is disabled or errors.

use std::collections::HashMap;

use serde::Deserialize;

use crate::{
    client::{AIClient, ChatMessage},
    error::AIError,
};

/// Structured meta for one Workspace, fed to the AI summary generator. Holds no
/// session transcript — only the branch, its owner, and raw textual signals.
#[derive(Debug, Clone)]
pub struct WorkSummaryInput {
    pub branch: String,
    pub owner: Option<String>,
    /// Recent non-merge commit subjects, a PR title, etc. — the raw material the
    /// AI condenses into a single human purpose line.
    pub signals: Vec<String>,
}

const SYSTEM_PROMPT: &str = "\
You write concise one-line summaries of what work was happening on each git \
branch (a developer's \"Workspace\"). For every branch you are given its owner \
(a SPEC/Issue id, optional) and raw signals (recent commit subjects). Produce a \
short, human-readable purpose describing what the work is about.\n\n\
Rules:\n\
- Write in the same language as the signals (Japanese stays Japanese).\n\
- Strip conventional-commit prefixes (feat:, fix(scope):, chore:, etc.).\n\
- Ignore merge and release noise (\"Merge pull request ...\", \"Merge branch ...\", \
\"chore(release): vX.Y.Z\"); infer the real work from the other signals instead.\n\
- One line, no trailing period, at most 80 characters.\n\
- If the signals carry no real purpose, omit that branch from the output.\n\n\
Respond with JSON only in this exact shape:\n\
{\"summaries\": [{\"branch\": \"<branch>\", \"summary\": \"<one line>\"}]}";

const MAX_SUMMARY_CHARS: usize = 80;

#[derive(Debug, Deserialize)]
struct SummaryItem {
    branch: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct SummariesResponse {
    summaries: Vec<SummaryItem>,
}

/// Parse the AI JSON response into a `branch -> summary` map. Empty branches /
/// summaries are skipped; over-long summaries are truncated on a char boundary.
pub fn parse_work_summaries(response: &str) -> Result<HashMap<String, String>, AIError> {
    let parsed: SummariesResponse = serde_json::from_str(response.trim())
        .map_err(|error| AIError::ParseError(format!("work summaries JSON: {error}")))?;
    let mut map = HashMap::new();
    for item in parsed.summaries {
        let branch = item.branch.trim();
        let summary = item.summary.trim();
        if branch.is_empty() || summary.is_empty() {
            continue;
        }
        map.insert(
            branch.to_string(),
            truncate_chars(summary, MAX_SUMMARY_CHARS),
        );
    }
    Ok(map)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>()
}

/// Build the user payload (a compact JSON array of the structured meta).
fn build_user_payload(inputs: &[WorkSummaryInput]) -> String {
    let items: Vec<serde_json::Value> = inputs
        .iter()
        .map(|input| {
            serde_json::json!({
                "branch": input.branch,
                "owner": input.owner,
                "signals": input.signals,
            })
        })
        .collect();
    serde_json::json!({ "branches": items }).to_string()
}

/// Ask the AI client to summarize each Workspace's purpose in one batched call.
/// Returns a `branch -> summary` map; branches the AI could not summarize are
/// simply absent. Returns an empty map (no AI call) when `inputs` is empty.
pub fn summarize_work_purposes(
    client: &AIClient,
    inputs: &[WorkSummaryInput],
) -> Result<HashMap<String, String>, AIError> {
    if inputs.is_empty() {
        return Ok(HashMap::new());
    }
    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.into(),
        },
        ChatMessage {
            role: "user".into(),
            content: build_user_payload(inputs),
        },
    ];
    let response = client.create_response(messages)?;
    parse_work_summaries(&response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_work_summaries_maps_branch_to_summary_and_skips_empty() {
        let json = r#"{"summaries": [
            {"branch": "work/20260601-0908", "summary": "tray の Copy URL のちらつきを修正"},
            {"branch": "work/x", "summary": ""},
            {"branch": "", "summary": "ignored"},
            {"branch": "develop", "summary": "リリース準備"}
        ]}"#;
        let map = parse_work_summaries(json).unwrap();
        assert_eq!(
            map.get("work/20260601-0908").map(String::as_str),
            Some("tray の Copy URL のちらつきを修正"),
        );
        assert_eq!(map.get("develop").map(String::as_str), Some("リリース準備"));
        // Empty summary / empty branch are skipped.
        assert!(!map.contains_key("work/x"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_work_summaries_truncates_overlong_summary_on_char_boundary() {
        let long = "あ".repeat(120);
        let json = format!(r#"{{"summaries":[{{"branch":"b","summary":"{long}"}}]}}"#);
        let map = parse_work_summaries(&json).unwrap();
        let summary = map.get("b").unwrap();
        assert_eq!(summary.chars().count(), MAX_SUMMARY_CHARS);
    }

    #[test]
    fn parse_work_summaries_rejects_malformed_json() {
        assert!(parse_work_summaries("not json").is_err());
    }

    #[test]
    fn build_user_payload_includes_branch_owner_signals() {
        let inputs = vec![WorkSummaryInput {
            branch: "work/x".into(),
            owner: Some("SPEC-3075".into()),
            signals: vec!["feat(workspace): purpose-first rail".into()],
        }];
        let payload = build_user_payload(&inputs);
        assert!(payload.contains("work/x"));
        assert!(payload.contains("SPEC-3075"));
        assert!(payload.contains("purpose-first rail"));
    }
}

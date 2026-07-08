//! Stop-check for false Execution completion claims.
//!
//! This is a narrow guard for the observed failure mode where an Execution
//! agent reports a pushed branch as complete even though no PR handoff exists.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{context::HookContext, envelope::stop_hook_active_from, HookEvent, HookOutput};

const TRANSCRIPT_TAIL_LIMIT: u64 = 128 * 1024;

pub fn handle_with_input(worktree: &Path, input: &str) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }

    let lane = HookContext::for_worktree(worktree).lane;
    if lane.policy_flags.reduced_skill_set {
        return HookOutput::Silent;
    }

    let Some(event) = HookEvent::read_from_str(input).ok().flatten() else {
        return HookOutput::Silent;
    };
    let Some(transcript_path) = event.transcript_path.as_deref() else {
        return HookOutput::Silent;
    };
    let path = transcript_path_for_worktree(worktree, transcript_path);
    let Ok(tail) = read_transcript_tail(&path) else {
        return HookOutput::Silent;
    };
    let Some(latest_assistant_text) = latest_assistant_text(&tail) else {
        return HookOutput::Silent;
    };

    if is_push_only_completion_claim(&latest_assistant_text) {
        return HookOutput::stop_block(push_only_completion_reason());
    }

    HookOutput::Silent
}

fn transcript_path_for_worktree(worktree: &Path, transcript_path: &str) -> PathBuf {
    let path = PathBuf::from(transcript_path);
    if path.is_absolute() {
        path
    } else {
        worktree.join(path)
    }
}

fn read_transcript_tail(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL_LIMIT);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn latest_assistant_text(transcript_tail: &str) -> Option<String> {
    let mut latest = None;
    for line in transcript_tail.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !contains_assistant_role(&value) {
            continue;
        }
        let mut strings = Vec::new();
        collect_text_strings(&value, &mut strings);
        let text = strings.join("\n");
        if !text.trim().is_empty() {
            latest = Some(text);
        }
    }
    latest
}

fn contains_assistant_role(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.get("role").and_then(Value::as_str) == Some("assistant")
                || map.get("type").and_then(Value::as_str) == Some("assistant")
                || map.values().any(contains_assistant_role)
        }
        Value::Array(items) => items.iter().any(contains_assistant_role),
        _ => false,
    }
}

fn collect_text_strings(value: &Value, strings: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "text" || key == "content" {
                    if let Some(text) = child.as_str() {
                        strings.push(text.to_string());
                        continue;
                    }
                }
                collect_text_strings(child, strings);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_text_strings(item, strings);
            }
        }
        _ => {}
    }
}

fn is_push_only_completion_claim(text: &str) -> bool {
    let lower = text.to_lowercase();
    let has_done_claim = text.contains("完了しました")
        || lower.contains("completed.")
        || lower.contains("work is complete")
        || lower.contains("done.");
    let has_push_only_marker = text.contains("push 済み")
        || text.contains("push済み")
        || lower.contains("push-only")
        || lower.contains("pushed to")
        || lower.contains("git push");
    let has_owner_marker =
        text.contains("Issue #") || text.contains("SPEC-") || lower.contains("issue #");
    has_done_claim && has_push_only_marker && has_owner_marker && !has_pr_delivery_evidence(&lower)
}

fn has_pr_delivery_evidence(lower: &str) -> bool {
    lower.contains("/pull/")
        || lower.contains("pull request #")
        || lower.contains("pr #")
        || lower.contains("pull request:")
        || lower.contains("pr:")
}

fn push_only_completion_reason() -> &'static str {
    "Execution completion blocked: push-only is not completion. The latest assistant message claims the Issue/SPEC is complete after a branch push, but no PR URL or PR number is present. Continue by running gwt-manage-pr to create or update the PR, or report a blocked PR handoff with the exact reason. Do not say the work is complete until the PR handoff exists and the relevant verification gate is satisfied."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_push_only_japanese_issue_completion_without_pr() {
        assert!(is_push_only_completion_claim(
            "完了しました。Issue #3233 は 3d827ffe2 として work/issue-3233 に push 済みです。"
        ));
    }

    #[test]
    fn allows_completion_with_pr_evidence() {
        assert!(!is_push_only_completion_claim(
            "完了しました。Issue #3233 は work/issue-3233 に push 済みです。PR #42 を作成しました。"
        ));
        assert!(!is_push_only_completion_claim(
            "Completed. Issue #3233 pushed to work/issue-3233. Pull request #42: https://github.com/akiojin/gwt/pull/42"
        ));
    }

    #[test]
    fn reads_latest_assistant_message_only() {
        let transcript =
            r#"{"role":"user","content":"完了しました。Issue #3233 は push 済みです。"}"#
                .to_string()
                + "\n"
                + r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"調査中です。"}]}}"#;
        assert_eq!(
            latest_assistant_text(&transcript).as_deref(),
            Some("調査中です。")
        );
    }
}

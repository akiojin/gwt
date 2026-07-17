//! `gwtd hook block-bash-policy` — consolidated PreToolUse Bash policy hook.
//!
//! Evaluates the existing Bash safety rules in a fixed order and returns the
//! first blocking decision, if any.

use std::{io::Read, path::Path};

use super::{
    block_cd_command, block_file_ops, block_git_branch_ops, block_git_dir_override, HookError,
    HookEvent, HookOutput,
};

pub fn evaluate_bash_command(command: &str, worktree_root: &Path) -> Option<HookOutput> {
    block_git_branch_ops::evaluate_bash_command(command)
        .or_else(|| block_cd_command::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_file_ops::evaluate_bash_command(command, worktree_root))
        .or_else(|| block_git_dir_override::evaluate_bash_command(command))
        .or_else(|| evaluate_long_pr_ci_polling_sleep(command))
        .or_else(|| evaluate_github_workflow_cli(command))
}

pub fn evaluate(event: &HookEvent, worktree_root: &Path) -> Result<HookOutput, HookError> {
    if event.tool_name.as_deref() != Some("Bash") {
        return Ok(HookOutput::Silent);
    }
    let Some(command) = event.command() else {
        return Ok(HookOutput::Silent);
    };
    Ok(evaluate_bash_command(command, worktree_root).unwrap_or(HookOutput::Silent))
}

pub(crate) fn evaluate_intake_checkpoint_authority(
    input: &str,
) -> Result<Option<HookOutput>, HookError> {
    let value = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let is_subagent = value
        .get("isSidechain")
        .or_else(|| value.get("is_sidechain"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        || ["agent_id", "agentId"].iter().any(|name| {
            value
                .get(*name)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        });
    let is_bash = value
        .get("tool_name")
        .or_else(|| value.get("toolName"))
        .and_then(serde_json::Value::as_str)
        == Some("Bash");
    let Some(command) = value
        .get("tool_input")
        .or_else(|| value.get("toolInput"))
        .and_then(|tool_input| tool_input.get("command"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    if !is_bash {
        return Ok(None);
    }
    let operation = match super::intake_checkpoint_authority::IntakeCheckpointOperation::from_command(command) {
        Ok(Some(operation)) => operation,
        Ok(None) => return Ok(None),
        Err(_) => {
            return Ok(Some(HookOutput::pre_tool_use_permission(
                "Intake checkpoint command is ambiguous",
                "Invoke exactly one of discussion.update, intake.checkpoint.current, or intake.checkpoint.update per Bash tool call.",
            )))
        }
    };
    if is_subagent {
        return Ok(Some(HookOutput::pre_tool_use_permission(
            "Intake discussion durability is root-session owned",
            "A Claude subagent cannot read or replace the root Intake checkpoint or publish a structured discussion milestone. Return the result to the root agent and let the root invoke discussion.update or intake.checkpoint.current/update.",
        )));
    }
    let is_official_pre_tool_use = value
        .get("hook_event_name")
        .or_else(|| value.get("hookEventName"))
        .and_then(serde_json::Value::as_str)
        == Some("PreToolUse");
    let hook_provider_session_id = value
        .get("session_id")
        .or_else(|| value.get("sessionId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(hook_provider_session_id) =
        hook_provider_session_id.filter(|_| is_official_pre_tool_use)
    else {
        // A direct CLI call or an older/partial JSON shape is not a minting
        // oracle. The checkpoint command itself still fails closed because
        // no one-shot permit is injected.
        return Ok(None);
    };
    let token = match super::intake_checkpoint_authority::issue_for_current_managed_claude(
        operation,
        hook_provider_session_id,
    ) {
        Ok(Some(token)) => token,
        Ok(None) => return Ok(None),
        Err(_) => {
            return Ok(Some(HookOutput::pre_tool_use_permission(
                "Root Intake checkpoint authorization is unavailable",
                "The managed Claude root session could not create a one-shot checkpoint permit. Retry from the root session after its Session ledger is available.",
            )))
        }
    };
    let Some(mut updated_input) = value
        .get("tool_input")
        .or_else(|| value.get("toolInput"))
        .cloned()
    else {
        return Ok(Some(HookOutput::pre_tool_use_permission(
            "Root Intake checkpoint authorization is unavailable",
            "The Bash hook input did not contain an editable tool_input object.",
        )));
    };
    let Some(object) = updated_input.as_object_mut() else {
        return Ok(Some(HookOutput::pre_tool_use_permission(
            "Root Intake checkpoint authorization is unavailable",
            "The Bash hook input did not contain an editable tool_input object.",
        )));
    };
    object.insert(
        "command".to_string(),
        serde_json::Value::String(format!(
            "export {}={token}; {command}",
            super::intake_checkpoint_authority::INTAKE_CHECKPOINT_PERMIT_ENV
        )),
    );
    Ok(Some(HookOutput::pre_tool_use_updated_input(updated_input)))
}

pub fn handle() -> Result<HookOutput, HookError> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    handle_with_input(&input)
}

pub fn handle_with_input(input: &str) -> Result<HookOutput, HookError> {
    if let Some(output) = evaluate_intake_checkpoint_authority(input)? {
        return Ok(output);
    }
    let Some(event) = HookEvent::read_from_str(input)? else {
        return Ok(HookOutput::Silent);
    };
    let root = crate::cli::hook::worktree::detect_worktree_root();
    evaluate(&event, &root)
}

fn evaluate_github_workflow_cli(command: &str) -> Option<HookOutput> {
    for segment in super::segments::split_command_segments(command) {
        let tokens = command_tokens(&segment);
        let Some(command_name) = tokens.first().copied() else {
            continue;
        };
        if command_name != "gh" {
            continue;
        }

        if let Some(subcommand) = tokens.get(1).copied() {
            match subcommand {
                "auth" | "repo" | "release" => continue,
                "issue" if is_blocked_issue_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "pr" if is_blocked_pr_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "run" if is_blocked_run_subcommand(tokens.get(2).copied()) => {
                    return Some(github_workflow_block_decision(command));
                }
                "api" if is_workflow_api_command(&segment, &tokens) => {
                    return Some(github_workflow_block_decision(command));
                }
                _ => {}
            }
        }
    }
    None
}

fn evaluate_long_pr_ci_polling_sleep(command: &str) -> Option<HookOutput> {
    let segments = super::segments::split_command_segments(command);
    let has_pr_ci_polling = segments
        .iter()
        .any(|segment| is_pr_ci_polling_segment(segment));
    if !has_pr_ci_polling {
        return None;
    }

    for segment in &segments {
        let tokens = command_tokens(segment);
        if is_long_sleep_segment(&tokens) {
            return Some(long_pr_ci_polling_sleep_block_decision(command));
        }
    }
    None
}

fn is_long_sleep_segment(tokens: &[&str]) -> bool {
    let Some(command_name) = tokens.first().copied() else {
        return false;
    };
    if normalize_command_name(command_name) != "sleep" {
        return false;
    }

    parse_sleep_args_seconds(&tokens[1..]).is_some_and(|seconds| seconds >= 120.0)
}

fn parse_sleep_args_seconds(args: &[&str]) -> Option<f64> {
    let mut total = 0.0;
    let mut parsed_any = false;
    for arg in args {
        let seconds = parse_sleep_duration_seconds(arg)?;
        total += seconds;
        parsed_any = true;
    }
    parsed_any.then_some(total)
}

fn parse_sleep_duration_seconds(duration: &str) -> Option<f64> {
    let duration = duration.trim_matches(|ch| ch == '\'' || ch == '"');
    let (numeric, multiplier) = match duration.chars().last() {
        Some('s') => (&duration[..duration.len() - 1], 1.0),
        Some('m') => (&duration[..duration.len() - 1], 60.0),
        Some('h') => (&duration[..duration.len() - 1], 60.0 * 60.0),
        Some('d') => (&duration[..duration.len() - 1], 24.0 * 60.0 * 60.0),
        _ => (duration, 1.0),
    };
    let value: f64 = numeric.parse().ok()?;
    if value.is_sign_negative() {
        return None;
    }
    Some(value * multiplier)
}

fn is_pr_ci_polling_segment(segment: &str) -> bool {
    let tokens = command_tokens(segment);
    let Some(command_name) = tokens.first().copied().map(normalize_command_name) else {
        return false;
    };

    match command_name.as_str() {
        "gwtd" => matches!(tokens.get(1).copied(), Some("pr" | "actions")),
        "gh" => tokens
            .get(1)
            .copied()
            .is_some_and(|subcommand| matches!(subcommand, "pr" | "run" | "api")),
        _ => false,
    }
}

fn normalize_command_name(token: &str) -> String {
    let token = token.trim_matches(|ch| ch == '\'' || ch == '"');
    Path::new(token)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(token)
        .to_string()
}

fn long_pr_ci_polling_sleep_block_decision(command: &str) -> HookOutput {
    HookOutput::pre_tool_use_permission(
        "Long PR/CI polling sleeps are not allowed",
        format!(
            "Do not keep Claude Code idle while waiting for PR or CI state changes.\n\n\
Run JSON operation `pr.checks` with `params.number:<pr>` once. If checks are still pending or queued, post the wait state with \
JSON operation `board.post` and `params.kind:\"blocked\"` plus `params.body`, then hand off instead of sleeping indefinitely.\n\n\
Blocked command: {command}"
        ),
    )
}

fn is_blocked_issue_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view" | "create" | "comment"))
}

fn is_blocked_pr_subcommand(subcommand: Option<&str>) -> bool {
    // "draft" does not exist as a gh subcommand (the real reverse of
    // `gh pr ready` is `gh pr ready --undo`, already covered by "ready");
    // it is intercepted anyway so an agent guessing it by symmetry with the
    // `pr.draft` JSON operation gets redirected instead of a gh usage error.
    matches!(
        subcommand,
        Some(
            "view"
                | "create"
                | "edit"
                | "ready"
                | "draft"
                | "comment"
                | "checks"
                | "reviews"
                | "review-threads"
        )
    )
}

fn is_blocked_run_subcommand(subcommand: Option<&str>) -> bool {
    matches!(subcommand, Some("view"))
}

fn github_workflow_block_decision(command: &str) -> HookOutput {
    HookOutput::pre_tool_use_permission(
        "\u{1F6AB} Direct GitHub workflow CLI commands are not allowed",
        format!(
            "Use the gwt workflow surfaces instead of direct `gh issue`, `gh pr`, `gh run`, or workflow-focused `gh api` commands.\n\n\
Recommended alternatives:\n\
- read: JSON operations `issue.view`, `issue.comments`, `issue.linked_prs`\n\
- write: JSON operations `issue.create`, `issue.comment`\n\
- PR workflow: JSON operations `pr.current`, `pr.view`, `pr.create`, `pr.edit`, `pr.ready`, `pr.draft`, `pr.comment`, `pr.checks`\n\
- PR reviews: JSON operations `pr.reviews`, `pr.review_threads`, `pr.review_threads.reply_and_resolve`\n\
- Actions logs: JSON operations `actions.logs`, `actions.job_logs`\n\
- discovery: `gwt-search`, `~/.gwt/cache/issues/<repo-hash>/`\n\n\
Blocked command: {command}"
        ),
    )
}

fn command_tokens(segment: &str) -> Vec<&str> {
    let raw: Vec<&str> = segment.split_whitespace().collect();
    let mut start = 0;

    while raw
        .get(start)
        .is_some_and(|token| matches!(*token, "do" | "then"))
    {
        start += 1;
    }

    if raw.get(start) == Some(&"env") {
        start += 1;
    }

    while start < raw.len() && is_env_assignment(raw[start]) {
        start += 1;
    }

    raw[start..].to_vec()
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_workflow_api_command(segment: &str, tokens: &[&str]) -> bool {
    let Some(target) = gh_api_target(tokens) else {
        return false;
    };

    if target == "graphql" {
        let lowered = segment.to_ascii_lowercase();
        return lowered.contains("issue(")
            || lowered.contains("issues(")
            || lowered.contains("updateissue")
            || lowered.contains("closeissue")
            || lowered.contains("reopenissue")
            || lowered.contains("pullrequest(")
            || lowered.contains("pullrequests(")
            || lowered.contains("reviews(")
            || lowered.contains("reviewthreads")
            || lowered.contains("workflowrun")
            || lowered.contains("workflowruns")
            || lowered.contains("checkrun")
            || lowered.contains("checkruns")
            || lowered.contains("checksuite")
            || lowered.contains("checksuites");
    }

    let lowered = target.to_ascii_lowercase();
    lowered.contains("/issues")
        || lowered.contains("/pulls")
        || lowered.contains("/actions/runs")
        || lowered.contains("/actions/jobs")
        || lowered.contains("/check-runs")
        || lowered.contains("/check-suites")
}

fn gh_api_target<'a>(tokens: &'a [&'a str]) -> Option<&'a str> {
    let mut i = 2;
    while i < tokens.len() {
        let token = tokens[i];
        if !token.starts_with('-') {
            return Some(token);
        }

        let consumes_value = matches!(
            token,
            "-H" | "--header"
                | "-f"
                | "--field"
                | "-F"
                | "--raw-field"
                | "-X"
                | "--method"
                | "--input"
                | "--jq"
                | "-q"
                | "--hostname"
                | "--cache"
        );
        i += if consumes_value { 2 } else { 1 };
    }
    None
}

#[cfg(test)]
mod tests {
    use gwt_agent::{AgentId, Session, GWT_RECOVERY_ID_ENV, GWT_SESSION_ID_ENV};

    use crate::cli::test_support::ScopedEnvVar;

    use super::*;

    #[test]
    fn blocks_long_sleep_before_gwtd_pr_polling() {
        let decision = evaluate_bash_command(
            "sleep 280 && /Applications/GWT.app/Contents/MacOS/gwtd pr view 123",
            Path::new("/worktree"),
        )
        .expect("expected long PR polling sleep to be blocked");

        assert_eq!(
            decision.summary(),
            "Long PR/CI polling sleeps are not allowed"
        );
        assert!(decision.detail().contains("JSON operation `pr.checks`"));
    }

    #[test]
    fn blocks_long_sleep_before_gh_run_polling() {
        let decision = evaluate_bash_command(
            "sleep 280 && gh run view 123456 --log",
            Path::new("/worktree"),
        )
        .expect("expected long GitHub Actions polling sleep to be blocked");

        assert_eq!(
            decision.summary(),
            "Long PR/CI polling sleeps are not allowed"
        );
    }

    #[test]
    fn allows_short_sleep_before_gwtd_pr_check() {
        let decision =
            evaluate_bash_command("sleep 30 && gwtd pr checks 123", Path::new("/worktree"));

        assert!(decision.is_none());
    }

    #[test]
    fn claude_subagent_cannot_invoke_root_intake_checkpoint_operations() {
        let input = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"$GWT_BIN < checkpoint.json # intake.checkpoint.update"},
              "agent_id":"child-1",
              "agent_type":"general-purpose"
            }"#;

        let decision = evaluate_intake_checkpoint_authority(input)
            .unwrap()
            .unwrap();
        assert_eq!(
            decision.summary(),
            "Intake discussion durability is root-session owned"
        );
    }

    #[test]
    fn claude_subagent_cannot_publish_discussion_update_checkpoint() {
        let input = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"$GWT_BIN < update.json # discussion.update"},
              "agent_id":"child-discussion"
            }"#;

        let decision = evaluate_intake_checkpoint_authority(input)
            .unwrap()
            .unwrap();
        assert_eq!(
            decision.summary(),
            "Intake discussion durability is root-session owned"
        );
    }

    #[test]
    fn unmanaged_root_command_needs_no_managed_checkpoint_permit() {
        let input = r#"{
              "tool_name":"Bash",
              "tool_input":{"command":"$GWT_BIN < checkpoint.json # intake.checkpoint.current"}
            }"#;

        assert!(evaluate_intake_checkpoint_authority(input)
            .unwrap()
            .is_none());
    }

    #[test]
    fn managed_claude_root_receives_one_shot_checkpoint_permit_in_updated_input() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = Session::new(temp.path(), "intake/root", AgentId::ClaudeCode);
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.agent_session_id = Some("provider-root".to_string());
        session.recovery_id = Some("recovery-root".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).expect("save Session");
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _recovery = ScopedEnvVar::set(GWT_RECOVERY_ID_ENV, "recovery-root");
        let input = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"gwtd <<'JSON'\n{\"operation\":\"intake.checkpoint.current\"}\nJSON","timeout":30}
            }"#;

        let output = evaluate_intake_checkpoint_authority(input)
            .expect("authority evaluation")
            .expect("updated input");
        let HookOutput::PreToolUseUpdatedInput { tool_input } = output else {
            panic!("managed Claude root must receive updated input")
        };
        let command = tool_input["command"].as_str().expect("updated command");
        assert!(
            command.starts_with("export GWT_INTAKE_CHECKPOINT_PERMIT="),
            "{command}"
        );
        assert!(command.ends_with("intake.checkpoint.current\"}\nJSON"));
        assert_eq!(tool_input["timeout"], 30);
        let token = command
            .strip_prefix("export GWT_INTAKE_CHECKPOINT_PERMIT=")
            .and_then(|tail| tail.split_once(';'))
            .map(|(token, _)| token)
            .expect("opaque permit");
        assert_eq!(token.len(), 64);
        let stored = std::fs::read_dir(sessions_dir.join(".intake-checkpoint-authority"))
            .expect("permit inventory")
            .map(|entry| {
                std::fs::read_to_string(entry.expect("entry").path()).expect("permit body")
            })
            .collect::<String>();
        assert!(
            !stored.contains(token),
            "permit plaintext must not be stored"
        );
    }

    #[test]
    fn managed_claude_root_partial_or_mismatched_hook_cannot_mint_permit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = Session::new(temp.path(), "intake/root", AgentId::ClaudeCode);
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.agent_session_id = Some("provider-root".to_string());
        session.recovery_id = Some("recovery-root".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).expect("save Session");
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _recovery = ScopedEnvVar::set(GWT_RECOVERY_ID_ENV, "recovery-root");

        let missing_event = r#"{
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"gwtd <<'JSON'\n{\"operation\":\"intake.checkpoint.current\"}\nJSON"}
            }"#;
        assert!(evaluate_intake_checkpoint_authority(missing_event)
            .expect("partial hook evaluation")
            .is_none());

        let mismatched_session = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"forged-provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"gwtd <<'JSON'\n{\"operation\":\"intake.checkpoint.current\"}\nJSON"}
            }"#;
        let decision = evaluate_intake_checkpoint_authority(mismatched_session)
            .expect("mismatched hook evaluation")
            .expect("mismatched managed hook must fail closed");
        assert_eq!(
            decision.summary(),
            "Root Intake checkpoint authorization is unavailable"
        );
        assert!(!sessions_dir.join(".intake-checkpoint-authority").exists());
    }

    #[test]
    fn managed_claude_root_requires_exact_recovery_env_except_deterministic_legacy_id() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session = Session::new(temp.path(), "intake/root", AgentId::ClaudeCode);
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.agent_session_id = Some("provider-root".to_string());
        session.recovery_id = Some("current-recovery".to_string());
        let session_id = session.id.clone();
        session.save(&sessions_dir).expect("save Session");
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _recovery = ScopedEnvVar::unset(GWT_RECOVERY_ID_ENV);
        let input = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"gwtd <<'JSON'\n{\"operation\":\"intake.checkpoint.current\"}\nJSON"}
            }"#;

        let rejected = evaluate_intake_checkpoint_authority(input)
            .expect("current recovery evaluation")
            .expect("missing current recovery env must fail closed");
        assert_eq!(
            rejected.summary(),
            "Root Intake checkpoint authorization is unavailable"
        );
        assert!(!sessions_dir.join(".intake-checkpoint-authority").exists());

        session.recovery_id = Some(format!("legacy-{session_id}"));
        session.save(&sessions_dir).expect("save legacy Session");
        let output = evaluate_intake_checkpoint_authority(input)
            .expect("legacy recovery evaluation")
            .expect("deterministic legacy recovery may omit the pre-upgrade env");
        assert!(matches!(output, HookOutput::PreToolUseUpdatedInput { .. }));
    }

    #[test]
    fn forged_runtime_path_cannot_select_a_checkpoint_authority_store() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", temp.path().join("canonical-home"));
        let _userprofile = ScopedEnvVar::set("USERPROFILE", temp.path().join("canonical-home"));
        let forged_sessions = temp.path().join("forged-sessions");
        let mut forged = Session::new(temp.path(), "intake/root", AgentId::ClaudeCode);
        forged.session_kind = Some(gwt_skills::SessionKind::Intake);
        forged.is_ephemeral = true;
        forged.agent_session_id = Some("provider-root".to_string());
        forged.recovery_id = Some("recovery-root".to_string());
        let session_id = forged.id.clone();
        forged.save(&forged_sessions).expect("save forged Session");
        let forged_runtime = gwt_agent::runtime_state_path(&forged_sessions, &session_id);
        let _session = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session_id);
        let _recovery = ScopedEnvVar::set(GWT_RECOVERY_ID_ENV, "recovery-root");
        let _runtime = ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &forged_runtime);
        let input = r#"{
              "hook_event_name":"PreToolUse",
              "session_id":"provider-root",
              "tool_name":"Bash",
              "tool_input":{"command":"gwtd <<'JSON'\n{\"operation\":\"intake.checkpoint.current\"}\nJSON"}
            }"#;

        let decision = evaluate_intake_checkpoint_authority(input)
            .expect("forged runtime evaluation")
            .expect("forged runtime must fail closed");
        assert_eq!(
            decision.summary(),
            "Root Intake checkpoint authorization is unavailable"
        );
        assert!(!forged_sessions
            .join(".intake-checkpoint-authority")
            .exists());
    }

    #[test]
    fn indirect_subagent_command_gets_no_permit_for_cli_replay() {
        let input = r#"{
              "tool_name":"Bash",
              "tool_input":{"command":"$GWT_BIN < checkpoint-payload.json"},
              "agent_id":"child-indirect"
            }"#;

        assert!(evaluate_intake_checkpoint_authority(input)
            .expect("authority evaluation")
            .is_none());
    }

    #[test]
    fn claude_main_thread_agent_type_without_agent_id_is_not_rejected() {
        let input = r#"{
              "tool_name":"Bash",
              "tool_input":{"command":"$GWT_BIN < checkpoint.json # intake.checkpoint.current"},
              "agent_type":"security-reviewer"
            }"#;

        assert!(evaluate_intake_checkpoint_authority(input)
            .unwrap()
            .is_none());
    }
}

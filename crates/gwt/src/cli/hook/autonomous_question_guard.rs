//! Issue #3478 (SPEC #3200 FR-025): PreToolUse interception that converts an
//! autonomous agent's confirmation question into a structured NeedsHuman
//! handoff *before* the provider's question UI can wait.
//!
//! The conversion is unconditional inside a monitor-launched autonomous
//! session and is a complete no-op everywhere else, so human-driven launches
//! keep their existing behavior. Reversible, in-scope decisions are handled by
//! the decision policy delivered as launch context
//! ([`autonomous_decision_policy`](crate::autonomous_handoff::autonomous_decision_policy));
//! reaching a question tool at all means the execution ends here and the Issue
//! Monitor slot is released for the next ready Issue.

use std::path::{Path, PathBuf};

use crate::autonomous_handoff::{
    autonomous_execution_context_from_env, extract_question, is_question_tool,
    AutonomousExecutionContext, AutonomousQuestionHandoff,
};

use super::{HookError, HookEvent, HookOutput};

/// Denial headline. Stable so the provider-facing contract is greppable.
pub const QUESTION_HANDOFF_SUMMARY: &str =
    "Autonomous execution cannot wait for a human — question converted to a NeedsHuman handoff";

/// Everything the guard needs that it must not discover on its own, so the
/// decision stays deterministic and testable.
pub struct QuestionGuardInputs<'a> {
    pub context: AutonomousExecutionContext,
    /// Issue Monitor control-plane prefs for the owning project.
    pub prefs_path: &'a Path,
    /// Agent/provider identifier of the asking session.
    pub provider: &'a str,
    pub now: &'a str,
    pub handoff_id: &'a str,
}

/// Convert a question tool call into a durable handoff and deny the call.
///
/// Returns [`HookOutput::Silent`] for every non-question tool. A control-plane
/// write failure still denies the call: allowing the question through would
/// hand the slot back to an indefinite wait, which is exactly the failure this
/// guard exists to remove.
pub fn evaluate_and_record(event: &HookEvent, inputs: &QuestionGuardInputs<'_>) -> HookOutput {
    let Some(tool_name) = event.tool_name.as_deref() else {
        return HookOutput::Silent;
    };
    if !is_question_tool(tool_name) {
        return HookOutput::Silent;
    }

    let handoff = AutonomousQuestionHandoff::new(
        inputs.handoff_id.to_string(),
        &inputs.context,
        inputs.provider,
        tool_name,
        extract_question(event.tool_input.as_ref()),
        inputs.now,
    );

    let recorded =
        match crate::issue_monitor::record_autonomous_question_handoff(inputs.prefs_path, &handoff)
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %inputs.prefs_path.display(),
                    "failed to persist autonomous question handoff"
                );
                false
            }
        };

    HookOutput::pre_tool_use_permission(QUESTION_HANDOFF_SUMMARY, deny_detail(&handoff, recorded))
}

fn deny_detail(handoff: &AutonomousQuestionHandoff, recorded: bool) -> String {
    let mut detail = format!(
        "This session was launched unattended by the gwt Issue Monitor for Issue #{issue}, so no human is \
watching it. Question tools would block the session and hold the Issue Monitor's active slot until the \
stuck timeout expires, so the question was converted into a NeedsHuman handoff instead.\n\n\
Handoff id: {handoff_id}\n\
Reason code: {reason}\n\
Question: {question}\n\n",
        issue = handoff.issue_number,
        handoff_id = handoff.handoff_id,
        reason = handoff.reason_code.as_str(),
        question = handoff.question,
    );
    if !recorded {
        detail.push_str(
            "WARNING: the handoff could not be written to the Issue Monitor control plane. The question is \
still refused; report this failure in your closing summary so the owner Issue is not silently lost.\n\n",
        );
    }
    detail.push_str(
        "What to do now:\n\
- If this decision is actually reversible and inside the owner Issue / SPEC scope, choose the smallest \
fail-closed default, record the assumption and its reason, and continue working. Do not call a question \
tool again.\n\
- Otherwise stop working on this Issue. It is parked for a human and the Issue Monitor slot has been \
released for the next ready Issue. Summarize what you completed, what remains, and the exact question, \
then end your turn.\n\n\
User verification, PR/merge gates, branch protection and permission boundaries are unchanged and must \
not be bypassed.",
    );
    detail
}

/// Production entry point: resolve the autonomous execution context and the
/// owning project's control plane from the environment, then evaluate.
///
/// Every resolution failure returns [`HookOutput::Silent`]: without a complete,
/// machine-readable autonomous context there is no owner to hand off to, and a
/// human-driven session must keep its question UI.
pub fn handle_with_input(input: &str) -> Result<HookOutput, HookError> {
    let Some(event) = HookEvent::read_from_str(input)? else {
        return Ok(HookOutput::Silent);
    };
    let Some(context) = autonomous_execution_context_from_env(|name| std::env::var(name).ok())
    else {
        return Ok(HookOutput::Silent);
    };
    let Some(tool_name) = event.tool_name.as_deref() else {
        return Ok(HookOutput::Silent);
    };
    if !is_question_tool(tool_name) {
        return Ok(HookOutput::Silent);
    }

    let worktree_root = super::worktree::detect_worktree_root();
    let prefs_path = prefs_path_for_session(&context, &worktree_root);
    Ok(evaluate_and_record(
        &event,
        &QuestionGuardInputs {
            provider: &provider_for_session(&context),
            context: context.clone(),
            prefs_path: &prefs_path,
            now: &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            handoff_id: &new_handoff_id(&context),
        },
    ))
}

/// Resolve the owning project's control plane. The linked worktree resolves to
/// the same project scope as the main checkout, so the agent's cwd is enough.
fn prefs_path_for_session(context: &AutonomousExecutionContext, worktree_root: &Path) -> PathBuf {
    let root = load_session(context)
        .map(|session| session.worktree_path)
        .filter(|path| path.exists())
        .unwrap_or_else(|| worktree_root.to_path_buf());
    crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&root)
}

fn provider_for_session(context: &AutonomousExecutionContext) -> String {
    load_session(context)
        .map(|session| session.agent_id.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn load_session(context: &AutonomousExecutionContext) -> Option<gwt_agent::Session> {
    let path = gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", context.session_id));
    gwt_agent::Session::load_and_migrate(&path).ok()
}

fn new_handoff_id(context: &AutonomousExecutionContext) -> String {
    format!(
        "handoff:{issue}:{session}:{unique}",
        issue = context.issue_number,
        session = context.session_id,
        unique = uuid::Uuid::new_v4(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inputs<'a>(prefs_path: &'a Path) -> QuestionGuardInputs<'a> {
        QuestionGuardInputs {
            context: AutonomousExecutionContext {
                issue_number: 7,
                session_id: "s".to_string(),
            },
            prefs_path,
            provider: "claude-code",
            now: "2026-08-06T05:00:00Z",
            handoff_id: "h",
        }
    }

    #[test]
    fn an_event_without_a_tool_name_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let output = evaluate_and_record(
            &HookEvent {
                tool_name: None,
                tool_input: Some(json!({})),
                transcript_path: None,
                cwd: None,
            },
            &inputs(&dir.path().join("prefs.json")),
        );
        assert_eq!(output, HookOutput::Silent);
    }

    #[test]
    fn an_unwritable_control_plane_still_denies_the_question() {
        // A directory in place of the prefs file makes every write fail.
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("prefs.json");
        std::fs::create_dir_all(&blocked).unwrap();

        let output = evaluate_and_record(
            &HookEvent {
                tool_name: Some("AskUserQuestion".to_string()),
                tool_input: Some(json!({"question": "Proceed?"})),
                transcript_path: None,
                cwd: None,
            },
            &inputs(&blocked),
        );

        let HookOutput::PreToolUsePermission { detail, .. } = output else {
            panic!("expected a denial even when the control plane is unwritable");
        };
        assert!(detail.contains("WARNING"));
    }

    #[test]
    fn handoff_ids_are_unique_per_interception() {
        let context = AutonomousExecutionContext {
            issue_number: 7,
            session_id: "s".to_string(),
        };
        assert_ne!(new_handoff_id(&context), new_handoff_id(&context));
    }
}

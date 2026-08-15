//! Issue #3478 (SPEC #3200 FR-025): provider-neutral conversion of an
//! autonomous agent's confirmation question into a structured NeedsHuman
//! handoff.
//!
//! An unattended agent that opens a question UI blocks on human input and can
//! leave its own work stranded until the `stuck_timeout_secs` (default 1800s)
//! reclaim fires. The contract here is
//! deliberately fail-closed and deterministic: a monitor-launched autonomous
//! session may never *wait* on a question. Every question tool call is denied
//! **before** the provider's UI starts waiting and is converted into a durable
//! handoff record carrying owner, session, question, options, rationale and a
//! machine-readable reason code.
//!
//! The complementary half is the decision policy delivered as launch context
//! (see [`autonomous_decision_policy`]): reversible, in-scope choices must be
//! resolved by the agent itself with a minimal, fail-closed default instead of
//! being asked at all. Reaching a question tool therefore already means the
//! agent judged the decision to need a human — or ignored the policy, in which
//! case escalating is the safe outcome.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Marks a launch as a monitor-driven autonomous execution (SPEC #3200).
/// Absent for every human-driven launch, which keeps their behavior unchanged.
pub const GWT_AUTONOMOUS_EXECUTION_ENV: &str = "GWT_AUTONOMOUS_EXECUTION";
/// Owner Issue number of the autonomous execution, paired with
/// [`GWT_AUTONOMOUS_EXECUTION_ENV`].
pub const GWT_AUTONOMOUS_ISSUE_ENV: &str = "GWT_AUTONOMOUS_ISSUE";

/// Why an autonomous agent could not resolve a decision on its own.
///
/// Informational for the human queue only: the handoff itself is
/// unconditional, so a misclassification can never suppress one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousHandoffReason {
    /// Irreversible or destructive effect (delete, force-push, drop, release).
    IrreversibleAction,
    /// Secrets, credentials, permissions, or another security boundary.
    SecurityCredential,
    /// Observable side effect outside the repository (publish, notify, deploy).
    ExternalSideEffect,
    /// The owner spec/acceptance criteria contradict the observed behavior.
    SpecConflict,
    /// Only a human can perform the verification (visual/manual check).
    HumanVerification,
    /// No specific boundary matched; a human still decides.
    Unclassified,
}

impl AutonomousHandoffReason {
    /// Stable machine-readable code, also used in operator-facing English text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IrreversibleAction => "irreversible_action",
            Self::SecurityCredential => "security_credential",
            Self::ExternalSideEffect => "external_side_effect",
            Self::SpecConflict => "spec_conflict",
            Self::HumanVerification => "human_verification",
            Self::Unclassified => "unclassified",
        }
    }
}

/// Lifecycle of one structured handoff. The Issue Monitor driver owns every
/// transition out of [`Pending`](AutonomousHandoffState::Pending); the hook
/// only ever appends a `Pending` record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousHandoffState {
    /// Written by the intercepting hook; not yet applied by the driver.
    #[default]
    Pending,
    /// Applied: the owner Issue is `NeedsHuman` and its active slot is free.
    AwaitingHuman,
    /// A human (GUI or PM) registered an answer; awaiting delivery.
    Answered,
    /// The answer was delivered back to the owning session.
    Resumed,
}

/// One selectable answer the agent offered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousHandoffOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Provider-neutral question payload lifted out of a question tool call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedQuestion {
    pub question: String,
    pub options: Vec<AutonomousHandoffOption>,
}

/// Machine-readable identity of a monitor-launched autonomous execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousExecutionContext {
    pub issue_number: u64,
    pub session_id: String,
}

/// One durable question handoff. Persisted in the Issue Monitor control plane
/// so it survives a daemon restart and stays addressable after the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousQuestionHandoff {
    pub handoff_id: String,
    pub issue_number: u64,
    pub session_id: String,
    /// Agent/provider identifier of the asking session (`claude-code`, `codex`, …).
    pub provider: String,
    pub tool_name: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AutonomousHandoffOption>,
    /// Operator-facing English explanation of why this reached the queue.
    pub rationale: String,
    pub reason_code: AutonomousHandoffReason,
    pub created_at: String,
    #[serde(default)]
    pub state: AutonomousHandoffState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    /// When the answer was handed to the resumed session. `None` on a
    /// `Resumed` handoff means the resume prompt is still owed to the launch
    /// path; set exactly once so the answer is never replayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<String>,
}

impl AutonomousQuestionHandoff {
    pub fn new(
        handoff_id: String,
        context: &AutonomousExecutionContext,
        provider: &str,
        tool_name: &str,
        question: ExtractedQuestion,
        created_at: &str,
    ) -> Self {
        let reason_code = classify_handoff_reason(&question.question, &question.options);
        let rationale = format!(
            "Autonomous execution for Issue #{issue} reached a question that needs human judgment ({reason}). \
The question was converted into this handoff before the provider's question UI could wait, so the active slot was released immediately.",
            issue = context.issue_number,
            reason = reason_code.as_str(),
        );
        Self {
            handoff_id,
            issue_number: context.issue_number,
            session_id: context.session_id.clone(),
            provider: provider.to_string(),
            tool_name: tool_name.to_string(),
            question: question.question,
            options: question.options,
            rationale,
            reason_code,
            created_at: created_at.to_string(),
            state: AutonomousHandoffState::Pending,
            answer: None,
            answered_at: None,
            delivered_at: None,
        }
    }

    /// Whether this handoff still blocks its owner Issue.
    pub fn is_open(&self) -> bool {
        matches!(
            self.state,
            AutonomousHandoffState::Pending | AutonomousHandoffState::AwaitingHuman
        )
    }
}

/// Normalized names of the confirmation-question tools across providers.
///
/// Matching is exact after normalization (lowercase, non-alphanumerics
/// removed) so an unrelated tool can never be swallowed by a substring match.
const QUESTION_TOOL_NAMES: &[&str] = &[
    // Claude Code
    "askuserquestion",
    "askuserquestiontool",
    // Codex
    "requestuserinput",
    "requestuserconfirmation",
    // Other providers routed through the canonical hook vocabulary
    "askuser",
    "askfollowupquestion",
    "userquestion",
];

fn normalize_tool_name(tool_name: &str) -> String {
    tool_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Whether `tool_name` is a confirmation-question tool that would put the
/// session into a human-input wait.
pub fn is_question_tool(tool_name: &str) -> bool {
    let normalized = normalize_tool_name(tool_name);
    QUESTION_TOOL_NAMES.contains(&normalized.as_str())
}

/// Read the autonomous execution context from the launch-injected environment.
///
/// Fail-closed: the marker alone is not enough — without an owner Issue number
/// and a session id there is nothing to hand off to, so no interception
/// happens and the launch behaves like any human-driven session.
pub fn autonomous_execution_context_from_env<F>(read: F) -> Option<AutonomousExecutionContext>
where
    F: Fn(&str) -> Option<String>,
{
    let marker = read(GWT_AUTONOMOUS_EXECUTION_ENV)?;
    if !is_truthy(&marker) {
        return None;
    }
    let issue_number = read(GWT_AUTONOMOUS_ISSUE_ENV)?.trim().parse::<u64>().ok()?;
    let session_id = read(gwt_agent::GWT_SESSION_ID_ENV)?.trim().to_string();
    if session_id.is_empty() {
        return None;
    }
    Some(AutonomousExecutionContext {
        issue_number,
        session_id,
    })
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Placeholder used when a question tool call carries no readable text. The
/// handoff must still be created — losing the text may never degrade into
/// silently waiting for a human.
const UNREADABLE_QUESTION: &str =
    "The autonomous agent opened a question with an unreadable payload.";

/// Lift the question text and options out of a provider's tool input.
///
/// Supports the Claude Code `AskUserQuestion` shape (`questions[]` with
/// `options[{label, description}]`) and the flat Codex `request_user_input`
/// shape (`question`/`prompt` with plain-string options).
pub fn extract_question(tool_input: Option<&Value>) -> ExtractedQuestion {
    let Some(input) = tool_input else {
        return unreadable_question();
    };

    if let Some(questions) = input.get("questions").and_then(Value::as_array) {
        let texts = questions
            .iter()
            .filter_map(question_text)
            .collect::<Vec<_>>();
        let options = questions.iter().flat_map(question_options).collect();
        if !texts.is_empty() {
            return ExtractedQuestion {
                question: texts.join("\n"),
                options,
            };
        }
    }

    let Some(question) = question_text(input) else {
        return unreadable_question();
    };
    ExtractedQuestion {
        question,
        options: question_options(input),
    }
}

fn unreadable_question() -> ExtractedQuestion {
    ExtractedQuestion {
        question: UNREADABLE_QUESTION.to_string(),
        options: Vec::new(),
    }
}

fn question_text(value: &Value) -> Option<String> {
    for key in ["question", "prompt", "text", "message", "header"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn question_options(value: &Value) -> Vec<AutonomousHandoffOption> {
    let Some(options) = value.get("options").and_then(Value::as_array) else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|option| match option {
            Value::String(label) if !label.trim().is_empty() => Some(AutonomousHandoffOption {
                label: label.trim().to_string(),
                description: String::new(),
            }),
            Value::Object(_) => {
                let label = option
                    .get("label")
                    .or_else(|| option.get("name"))
                    .or_else(|| option.get("value"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|label| !label.is_empty())?;
                let description = option
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default();
                Some(AutonomousHandoffOption {
                    label: label.to_string(),
                    description: description.to_string(),
                })
            }
            _ => None,
        })
        .collect()
}

/// Keyword markers per reason code, checked in declaration order so the most
/// consequential boundary wins when a question mentions several.
const REASON_MARKERS: &[(AutonomousHandoffReason, &[&str])] = &[
    (
        AutonomousHandoffReason::SecurityCredential,
        &[
            "credential",
            "secret",
            "token",
            "password",
            "api key",
            "private key",
            "permission",
            "auth",
        ],
    ),
    (
        AutonomousHandoffReason::IrreversibleAction,
        &[
            "delete",
            "drop",
            "destroy",
            "force-push",
            "force push",
            "overwrite",
            "irreversible",
            "wipe",
            "reset --hard",
            "purge",
        ],
    ),
    (
        AutonomousHandoffReason::ExternalSideEffect,
        &[
            "publish",
            "deploy",
            "release",
            "notify",
            "send email",
            "post to",
            "external service",
            "production",
        ],
    ),
    (
        AutonomousHandoffReason::SpecConflict,
        &[
            "contradict",
            "conflict",
            "inconsistent",
            "spec says",
            "acceptance criteria",
            "ambiguous requirement",
        ],
    ),
    (
        AutonomousHandoffReason::HumanVerification,
        &[
            "visually",
            "visual verification",
            "manually verify",
            "manual verification",
            "please confirm in the gui",
            "screenshot",
            "look at the screen",
        ],
    ),
];

/// Label the handoff for the human queue. Never gates whether the handoff
/// happens — an unmatched question is [`AutonomousHandoffReason::Unclassified`]
/// and still escalates.
pub fn classify_handoff_reason(
    question: &str,
    options: &[AutonomousHandoffOption],
) -> AutonomousHandoffReason {
    let mut haystack = question.to_ascii_lowercase();
    for option in options {
        haystack.push('\n');
        haystack.push_str(&option.label.to_ascii_lowercase());
        haystack.push('\n');
        haystack.push_str(&option.description.to_ascii_lowercase());
    }
    for (reason, markers) in REASON_MARKERS {
        if markers.iter().any(|marker| haystack.contains(marker)) {
            return *reason;
        }
    }
    AutonomousHandoffReason::Unclassified
}

/// Decision policy delivered to a monitor-launched autonomous session
/// (AC-1/AC-2). Injected as hook additional context at every intent boundary so
/// it survives context compaction.
pub fn autonomous_decision_policy(context: &AutonomousExecutionContext) -> String {
    format!(
        "# Autonomous execution policy (Issue #{issue})\n\n\
This session was launched unattended by the gwt Issue Monitor. No human is watching it.\n\n\
**Decide and continue** when the choice is reversible and inside the owner Issue / SPEC scope: \
pick the smallest, fail-closed default, record the assumption and the reason in your work notes \
and in the PR body, and keep going. Do not open a question UI for it.\n\n\
**Hand off** only when a human must decide: irreversible or destructive effects, \
security/credential boundaries, side effects outside this repository, a contradiction between \
the spec and the observed behavior, or a verification only a human can perform.\n\n\
Question tools are blocked in this session. A question tool call is converted into a structured \
NeedsHuman handoff before it can wait, the owner Issue is parked for a human, and the Issue \
Monitor slot is released for the next ready Issue — so asking ends this execution instead of \
pausing it.\n\n\
User verification, PR/merge gates, branch protection and permission boundaries are unchanged and \
must not be bypassed.",
        issue = context.issue_number,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_name_matching_ignores_case_and_separators_but_not_substrings() {
        assert!(is_question_tool("ASK_USER_QUESTION"));
        assert!(is_question_tool("  AskUserQuestion  "));
        // A tool whose name merely contains a question tool name is not one.
        assert!(!is_question_tool("AskUserQuestionHistoryReader"));
    }

    #[test]
    fn multiple_questions_are_joined_and_their_options_merged() {
        let input = json!({
            "questions": [
                {"question": "First?", "options": [{"label": "A"}]},
                {"question": "Second?", "options": [{"label": "B"}]}
            ]
        });
        let extracted = extract_question(Some(&input));
        assert_eq!(extracted.question, "First?\nSecond?");
        assert_eq!(extracted.options.len(), 2);
    }

    #[test]
    fn empty_questions_array_falls_back_to_the_flat_shape() {
        let input = json!({"questions": [], "prompt": "Flat prompt?"});
        assert_eq!(extract_question(Some(&input)).question, "Flat prompt?");
    }

    #[test]
    fn options_are_classified_too() {
        assert_eq!(
            classify_handoff_reason(
                "Pick one",
                &[AutonomousHandoffOption {
                    label: "Delete the branch".to_string(),
                    description: String::new(),
                }]
            ),
            AutonomousHandoffReason::IrreversibleAction
        );
    }

    #[test]
    fn non_truthy_marker_disables_interception() {
        assert!(autonomous_execution_context_from_env(|name| match name {
            "GWT_AUTONOMOUS_EXECUTION" => Some("0".to_string()),
            "GWT_AUTONOMOUS_ISSUE" => Some("1".to_string()),
            "GWT_SESSION_ID" => Some("s".to_string()),
            _ => None,
        })
        .is_none());
    }

    #[test]
    fn decision_policy_names_the_owner_issue_and_the_handoff_consequence() {
        let policy = autonomous_decision_policy(&AutonomousExecutionContext {
            issue_number: 3478,
            session_id: "s".to_string(),
        });
        assert!(policy.contains("Issue #3478"));
        assert!(policy.contains("Question tools are blocked"));
    }
}

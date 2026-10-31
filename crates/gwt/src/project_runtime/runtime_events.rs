//! Runtime failure and screen policy shared by the physical project owner and read-only GUI adapters.
use crate as gwt;

pub const AGENT_ERROR_TAIL_MAX_CHARS: usize = 240;
const EXACT_RESUME_FAILURE_SIGNATURE: &str = "No conversation found with session ID";
const PROVIDER_ERROR_PREFIX: &str = "Error: ";
const RESUME_WRITER_CONFLICT_OUTER_PREFIX: &str = "Failed to resume session from ";
const RESUME_WRITER_CONFLICT_PREFIX: &str = "thread/resume failed during TUI bootstrap:";
const RESUME_WRITER_CONFLICT_SUFFIX: &str = "already has an active writer";
const RESUME_WRITER_CONFLICT_CODE: &str = "(code -32600)";

fn marker_is_inside_double_quotes(line: &str, marker_offset: usize) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for byte in line[..marker_offset].bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => quoted = !quoted,
            _ => {}
        }
    }
    quoted
}

fn resume_writer_conflict_outer_offset(line: &str) -> Option<usize> {
    let anchored_provider_output = |offset: usize| {
        let output = &line[offset..];
        if output.starts_with(RESUME_WRITER_CONFLICT_OUTER_PREFIX) {
            Some(offset)
        } else {
            output
                .strip_prefix(PROVIDER_ERROR_PREFIX)
                .filter(|detail| detail.starts_with(RESUME_WRITER_CONFLICT_OUTER_PREFIX))
                .map(|_| offset + PROVIDER_ERROR_PREFIX.len())
        }
    };

    let trimmed = line.trim_start();
    let leading_whitespace = line.len() - trimmed.len();
    anchored_provider_output(leading_whitespace).or_else(|| {
        let last_output = " — last output: ";
        let output_offset = line.find(last_output)? + last_output.len();
        anchored_provider_output(output_offset)
    })
}

/// Classify a typed failure out of an agent's exit detail.
///
/// The two exit-detail causes are checked in this order because they answer
/// different questions. A provider quota block is a property of the account
/// and applies to any session mode, so it is tested first; the late-resume
/// writer race can only happen while resuming. Screen-only onboarding prompts
/// are classified separately from this exit-detail path.
pub fn classify_issue_monitor_failure(
    detail: &str,
    session_mode: gwt_agent::SessionMode,
) -> Option<gwt::IssueMonitorFailure> {
    if let Some(failure) = classify_provider_usage_limit(detail) {
        return Some(failure);
    }
    classify_resume_writer_conflict(detail, session_mode)
}

/// Issue #3616: the provider stated it is out of quota.
///
/// Detection lives in `gwt_core::usage` because the message formats and their
/// reset clauses are provider domain knowledge, not runtime-event knowledge.
/// `Local::now()` is the reference zone on purpose: both providers print a
/// local wall clock with no offset, so the machine running the agent is the
/// only anchor available.
pub fn classify_provider_usage_limit(detail: &str) -> Option<gwt::IssueMonitorFailure> {
    let notice = gwt_core::usage::detect_provider_limit_notice(detail, &chrono::Local::now())?;
    Some(provider_usage_limit_failure(&notice, None, None))
}

/// `pane_agent_id` is the agent the pane is actually running, and it wins over
/// the notice's own wording: Claude's weekly-limit text names no provider at
/// all, and mis-attributing the block would tell the PM the wrong account is
/// out.
pub fn provider_usage_limit_failure(
    notice: &gwt_core::usage::ProviderLimitNotice,
    pane_agent_id: Option<&str>,
    evidence: Option<gwt::IssueMonitorProviderQuotaHoldEvidence>,
) -> gwt::IssueMonitorFailure {
    gwt::IssueMonitorFailure::ProviderUsageLimit {
        provider: provider_label(notice, pane_agent_id),
        resets_at: notice
            .resets_at
            .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        evidence: evidence.map(Box::new),
    }
}

/// The agent id to report as the exhausted account, falling back to the
/// notice's hint and finally to `unknown` — never to a guess.
pub fn provider_label(
    notice: &gwt_core::usage::ProviderLimitNotice,
    pane_agent_id: Option<&str>,
) -> String {
    if let Some(agent_id) = pane_agent_id
        .map(str::trim)
        .filter(|agent_id| !agent_id.is_empty())
    {
        return agent_id.to_string();
    }
    match notice.provider {
        Some(gwt_core::usage::UsageProvider::Codex) => "codex".to_string(),
        Some(gwt_core::usage::UsageProvider::ClaudeCode) => "claude".to_string(),
        None => "unknown".to_string(),
    }
}

/// Classify the provider's exact late-resume writer race without promoting
/// unrelated launch failures that happen to mention a writer.
fn classify_resume_writer_conflict(
    detail: &str,
    session_mode: gwt_agent::SessionMode,
) -> Option<gwt::IssueMonitorFailure> {
    if session_mode != gwt_agent::SessionMode::Resume {
        return None;
    }
    detail.lines().find_map(|line| {
        let outer_offset = resume_writer_conflict_outer_offset(line)?;
        let prefix_offset =
            outer_offset + line[outer_offset..].find(RESUME_WRITER_CONFLICT_PREFIX)?;
        if marker_is_inside_double_quotes(line, prefix_offset) {
            return None;
        }
        let after_prefix = prefix_offset + RESUME_WRITER_CONFLICT_PREFIX.len();
        let writer_offset =
            after_prefix + line[after_prefix..].find(RESUME_WRITER_CONFLICT_SUFFIX)?;
        if marker_is_inside_double_quotes(line, writer_offset) {
            return None;
        }
        let after_writer = writer_offset + RESUME_WRITER_CONFLICT_SUFFIX.len();
        let code_offset = after_writer + line[after_writer..].find(RESUME_WRITER_CONFLICT_CODE)?;
        if marker_is_inside_double_quotes(line, code_offset) {
            return None;
        }
        Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
            holder_window_id: None,
        })
    })
}

/// Compose the persistent window detail for an errored agent process from the
/// plain exit detail and the final screen tail. Pure so the classification is
/// unit-testable without a PTY.
pub fn compose_agent_error_detail(base: Option<String>, tail: Option<&str>) -> Option<String> {
    let tail = tail.map(str::trim).filter(|tail| !tail.is_empty());
    let Some(tail) = tail else {
        return base;
    };
    // Issue #3490: several Codex agents initializing the shared `~/.codex`
    // state directory at once lose the SQLite race, and the provider's answer
    // is a nested stack trace that tells the operator nothing actionable.
    // Checked before truncation because the lock refusal is the tail of a long
    // path-heavy message and would be cut off. The replacement carries the
    // transient-retry hint, so the Issue Monitor requeues without spending an
    // attempt on host contention.
    if gwt_agent::is_codex_shared_state_lock_failure(tail) {
        return Some(gwt_agent::codex_shared_state_lock_detail());
    }
    let sandbox_error =
        "Could not read Windows sandbox configuration and requirements from the app server.";
    if tail.contains(sandbox_error) {
        return Some(format!(
            "Agent startup failed: {sandbox_error}; {}",
            gwt_agent::TRANSIENT_LAUNCH_RETRY_HINT
        ));
    }
    // Issue #4445: an npm shim whose target executable was renamed away by the
    // provider's auto-update reports a localized shell message and a bare exit
    // status. The path it could not find is the whole diagnosis, and it is at
    // the head of the tail, so it is named before truncation can drop it.
    if let Some(named) = gwt_agent::missing_launcher_binary_detail(tail) {
        return Some(named);
    }
    let tail: String = if tail.chars().count() > AGENT_ERROR_TAIL_MAX_CHARS {
        let mut truncated: String = tail.chars().take(AGENT_ERROR_TAIL_MAX_CHARS).collect();
        truncated.push('…');
        truncated
    } else {
        tail.to_string()
    };
    if tail.contains(EXACT_RESUME_FAILURE_SIGNATURE) {
        return Some(
            "Exact session restore failed. The agent no longer has this conversation; \
             use Continue work to start a new conversation with handoff context."
                .to_string(),
        );
    }
    match base {
        Some(base) if !base.is_empty() => Some(format!("{base} — last output: {tail}")),
        _ => Some(format!("Agent exited — last output: {tail}")),
    }
}

pub fn screen_tail_text(contents: &str, lines: usize, separator: &str) -> Option<String> {
    let screen_lines: Vec<&str> = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let tail = screen_lines[screen_lines.len().saturating_sub(lines)..].join(separator);
    (!tail.is_empty()).then_some(tail)
}

pub fn issue_monitor_launch_failed_payload(
    issue_number: u64,
    message: &str,
    delivery_id: Option<&str>,
    materializer_id: Option<&str>,
    session_mode: gwt_agent::SessionMode,
) -> serde_json::Value {
    issue_monitor_launch_failed_payload_with_retryability(
        issue_number,
        message,
        delivery_id,
        materializer_id,
        session_mode,
        true,
    )
}

pub fn issue_monitor_launch_failed_payload_with_retryability(
    issue_number: u64,
    message: &str,
    delivery_id: Option<&str>,
    materializer_id: Option<&str>,
    session_mode: gwt_agent::SessionMode,
    retryable: bool,
) -> serde_json::Value {
    let mut launch_failed = serde_json::json!({
        "issue_number": issue_number,
        "message": message,
    });
    if let Some(delivery_id) = delivery_id {
        launch_failed["delivery_id"] = serde_json::json!(delivery_id);
    }
    if let Some(materializer_id) = materializer_id {
        launch_failed["materializer_id"] = serde_json::json!(materializer_id);
    }
    if !retryable {
        launch_failed["retryable"] = serde_json::json!(false);
    }
    if let Some(failure) = retryable
        .then(|| classify_issue_monitor_failure(message, session_mode))
        .flatten()
    {
        launch_failed["failure"] =
            serde_json::to_value(failure).expect("Issue Monitor failure serializes");
    }
    serde_json::json!({ "launch_failed": launch_failed })
}

pub fn issue_monitor_agent_failed_payload(
    window_id: &str,
    message: &str,
    issue_number_hint: Option<u64>,
    session_mode: gwt_agent::SessionMode,
) -> serde_json::Value {
    let failure = classify_issue_monitor_failure(message, session_mode);
    issue_monitor_agent_failed_payload_with_failure(
        window_id,
        message,
        issue_number_hint,
        failure.as_ref(),
    )
}

pub fn issue_monitor_agent_failed_from_launch_payload(
    window_id: &str,
    message: &str,
    source: &gwt::runtime_daemon_events::IssueMonitorAgentFailureSource,
    failure: Option<&gwt::IssueMonitorFailure>,
) -> serde_json::Value {
    let mut legacy =
        issue_monitor_agent_failed_payload_with_failure(window_id, message, None, failure);
    let mut agent_failed = legacy["agent_failed"].take();
    agent_failed["source"] =
        serde_json::to_value(source).expect("captured agent failure source serializes");
    serde_json::json!({ "agent_failed_from_launch": agent_failed })
}

pub fn issue_monitor_agent_failed_payload_with_failure(
    window_id: &str,
    message: &str,
    issue_number_hint: Option<u64>,
    failure: Option<&gwt::IssueMonitorFailure>,
) -> serde_json::Value {
    let mut agent_failed = serde_json::json!({
        "window_id": window_id,
        "message": message,
    });
    if let Some(issue_number) = issue_number_hint {
        agent_failed["issue_number"] = serde_json::json!(issue_number);
    }
    if let Some(failure) = failure {
        agent_failed["failure"] =
            serde_json::to_value(failure).expect("Issue Monitor failure serializes");
    }
    serde_json::json!({ "agent_failed": agent_failed })
}

/// One capacity retry belongs to the Session, including an exact pane replacement.
pub(crate) struct ProviderCapacityResume {
    pub(crate) window_id: String,
    pub(crate) incarnation: u64,
    pub(crate) binding: gwt_agent::SessionExecutionBinding,
    pub(crate) due: Option<std::time::Instant>,
    pub(crate) input_revision: Option<u64>,
    pub(crate) attempted: bool,
    pub(crate) permit: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// Per-incarnation runtime policy owned beside the physical PTY. GUI caches
/// never carry the leases, grants or process handles that apply its effects.
#[derive(Clone, Default)]
pub struct RuntimePolicyState {
    pub approval: Option<ApprovalPromptState>,
    pub approval_settle_at: Option<std::time::Instant>,
    pub quota_candidate_since: Option<chrono::DateTime<chrono::Utc>>,
    pub quota_hold: Option<crate::IssueMonitorFailure>,
    pub released_quota_notice: Option<gwt_core::usage::ProviderLimitNotice>,
    pub api_error: Option<gwt_core::usage::ProviderApiError>,
    pub recoverable_error: bool,
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Clone, Copy, Default)]
pub struct ApprovalPromptState {
    pub active_fingerprint: Option<u64>,
    pub resolving_fingerprint: Option<u64>,
    pub resolution_started: bool,
}

pub struct ExitPolicyInput<'a> {
    pub window_id: &'a str,
    pub preset: crate::WindowPreset,
    pub agent_id: Option<&'a str>,
    pub has_agent_session: bool,
    pub raw_status: crate::WindowProcessStatus,
    pub hook_state: Option<crate::WindowProcessStatus>,
    pub exit_confirmed: bool,
    pub exact_incarnation: bool,
    pub detail: Option<String>,
    pub final_screen: Option<&'a str>,
    pub accounts: &'a [gwt_core::usage::ProviderUsage],
    pub now: chrono::DateTime<chrono::Utc>,
}

pub struct ExitPolicyOutcome {
    pub status: crate::WindowProcessStatus,
    pub detail: Option<String>,
    pub auto_close: bool,
    pub keep_recovery: bool,
    pub quota_hold: Option<crate::IssueMonitorFailure>,
    pub clear_hook: bool,
}

pub fn approval_provider(agent_id: Option<&str>) -> crate::window_state::ApprovalPromptProvider {
    use crate::window_state::ApprovalPromptProvider;
    match agent_id.and_then(gwt_agent::resolve_agent_id) {
        Some(gwt_agent::AgentId::Codex) => ApprovalPromptProvider::Codex,
        Some(gwt_agent::AgentId::ClaudeCode) => ApprovalPromptProvider::ClaudeCode,
        _ => ApprovalPromptProvider::Unsupported,
    }
}

pub fn quota_provider(agent_id: Option<&str>) -> Option<gwt_core::usage::UsageProvider> {
    use crate::window_state::ApprovalPromptProvider;
    use gwt_core::usage::UsageProvider;
    match approval_provider(agent_id) {
        ApprovalPromptProvider::Codex => Some(UsageProvider::Codex),
        ApprovalPromptProvider::ClaudeCode => Some(UsageProvider::ClaudeCode),
        ApprovalPromptProvider::Unsupported => None,
    }
}

pub fn composed_policy_status(
    state: &RuntimePolicyState,
    raw_status: crate::WindowProcessStatus,
    preset: crate::WindowPreset,
    hook_state: Option<crate::WindowProcessStatus>,
    has_agent_session: bool,
) -> crate::WindowProcessStatus {
    let composed = crate::window_state::compose_window_state_with_approval_wait(
        raw_status,
        preset,
        hook_state,
        has_agent_session,
        state.approval.is_some(),
    );
    let composed =
        crate::window_state::apply_provider_quota_block(composed, state.quota_hold.is_some());
    crate::window_state::apply_provider_api_error_block(composed, state.api_error.is_some())
}

/// The existing exact-exit decision, separated from physical retirement so a
/// retained diagnostic pane and a clean auto-close use one classification.
pub fn classify_exit(
    state: &mut RuntimePolicyState,
    input: ExitPolicyInput<'_>,
) -> ExitPolicyOutcome {
    use crate::WindowState::{Error, Stopped};
    let is_agent = crate::window_state::uses_agent_hook_state(input.preset);
    let approval_active = state.approval.is_some()
        || (input.raw_status == Error
            && input.final_screen.is_some_and(|screen| {
                crate::window_state::approval_prompt_fingerprint(
                    approval_provider(input.agent_id),
                    screen,
                )
                .is_some()
            }));
    let mut detail = if approval_active && input.raw_status == Error {
        Some("Agent approval prompt ended unexpectedly".to_string())
    } else {
        input.detail
    };
    let quota_match =
        if input.exit_confirmed && is_agent && matches!(input.raw_status, Error | Stopped) {
            let haystack = format!(
                "{}\n{}",
                detail.as_deref().unwrap_or_default(),
                input.final_screen.unwrap_or_default()
            );
            gwt_core::usage::detect_provider_limit_notice(
                &haystack,
                &input.now.with_timezone(&chrono::Local),
            )
            .filter(|notice| notice.provider == quota_provider(input.agent_id))
            .filter(|notice| state.released_quota_notice.as_ref() != Some(notice))
            .map(|notice| (notice, haystack))
        } else {
            None
        };
    state.quota_hold = quota_match.as_ref().map(|(notice, screen)| {
        let evidence = crate::IssueMonitorProviderQuotaHoldEvidence::screen_notice(
            &input
                .now
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            input.window_id,
            screen,
        )
        .with_poller(input.agent_id, input.accounts);
        provider_usage_limit_failure(notice, input.agent_id, Some(evidence))
    });
    let keep_recovery = !input.exit_confirmed
        || quota_match.is_some()
        || (input.raw_status == Error
            && input.has_agent_session
            && is_agent
            && (input
                .hook_state
                .is_some_and(crate::window_state::is_live_agent_hook_state)
                || state.recoverable_error));
    if let Some((notice, _)) = &quota_match {
        detail = Some(gwt_core::usage::describe_provider_limit_notice(
            notice,
            Some(provider_label(notice, input.agent_id).as_str()),
        ));
    } else if input.raw_status == Error && !approval_active && is_agent {
        let tail = input
            .final_screen
            .and_then(|screen| screen_tail_text(screen, 3, " "));
        detail = compose_agent_error_detail(detail, tail.as_deref());
    }
    let clear_hook = input.raw_status == Error;
    let status = composed_policy_status(
        state,
        input.raw_status,
        input.preset,
        input.hook_state.filter(|_| !clear_hook),
        input.has_agent_session,
    );
    if matches!(input.raw_status, Error | Stopped) {
        state.approval = None;
        state.approval_settle_at = None;
    }
    if keep_recovery && matches!(input.raw_status, Error | Stopped) {
        state.recoverable_error = true;
    } else if input.raw_status != Error {
        state.recoverable_error = false;
    }
    ExitPolicyOutcome {
        status,
        detail,
        auto_close: input.exit_confirmed
            && input.exact_incarnation
            && input.has_agent_session
            && status == Stopped,
        keep_recovery,
        quota_hold: state.quota_hold.clone(),
        clear_hook,
    }
}

pub struct ScreenPolicyInput<'a> {
    pub window_id: &'a str,
    pub preset: crate::WindowPreset,
    pub agent_id: Option<&'a str>,
    pub screen: Option<&'a str>,
    pub accounts: &'a [gwt_core::usage::ProviderUsage],
    pub now: chrono::DateTime<chrono::Utc>,
    pub monotonic_now: std::time::Instant,
}

#[derive(Default)]
pub struct ScreenPolicyOutcome {
    pub detail: Option<String>,
    pub clear_detail: bool,
    pub failure: Option<crate::IssueMonitorFailure>,
    pub permission_prompt: Option<u64>,
    pub approval_reentered: bool,
}

pub fn observe_screen_policy(
    state: &mut RuntimePolicyState,
    input: ScreenPolicyInput<'_>,
) -> ScreenPolicyOutcome {
    let mut outcome = ScreenPolicyOutcome::default();
    if !crate::window_state::uses_agent_hook_state(input.preset) {
        return outcome;
    }
    let provider = approval_provider(input.agent_id);
    let prompt = input
        .screen
        .and_then(|screen| crate::window_state::approval_prompt_fingerprint(provider, screen));
    outcome.permission_prompt = prompt;
    match (state.approval, prompt) {
        (None, Some(fingerprint)) => {
            state.approval = Some(ApprovalPromptState {
                active_fingerprint: Some(fingerprint),
                ..Default::default()
            })
        }
        (Some(latch), None)
            if latch.resolution_started
                && (latch.active_fingerprint.is_none()
                    || latch.resolving_fingerprint == latch.active_fingerprint) =>
        {
            if state.approval_settle_at.is_none() {
                state.approval_settle_at =
                    Some(input.monotonic_now + std::time::Duration::from_millis(100));
            }
        }
        (Some(mut latch), Some(fingerprint))
            if latch.active_fingerprint.is_none()
                || latch.active_fingerprint == Some(fingerprint) =>
        {
            latch.active_fingerprint = Some(fingerprint);
            if latch.resolution_started {
                latch.resolving_fingerprint = None;
                latch.resolution_started = false;
                state.approval_settle_at = None;
            }
            state.approval = Some(latch);
        }
        (Some(_), Some(fingerprint)) => {
            outcome.approval_reentered = true;
            state.approval = Some(ApprovalPromptState {
                active_fingerprint: Some(fingerprint),
                ..Default::default()
            });
            state.approval_settle_at = None;
        }
        _ => {}
    }
    if input.screen.is_some_and(|screen| {
        crate::window_state::directory_trust_prompt_fingerprint(provider, screen).is_some()
    }) {
        outcome.failure = Some(crate::IssueMonitorFailure::CodexDirectoryTrustPrompt);
    }
    if state.quota_hold.is_none() {
        let notice = input
            .screen
            .and_then(|screen| {
                gwt_core::usage::limit_notice::detect_provider_limit_notice_with_clock(
                    screen,
                    || input.now.with_timezone(&chrono::Local),
                )
            })
            .filter(|notice| notice.provider == quota_provider(input.agent_id));
        match notice {
            None => {
                state.quota_candidate_since = None;
                state.released_quota_notice = None;
            }
            Some(notice) if state.released_quota_notice.as_ref() == Some(&notice) => {}
            Some(notice) => {
                state.released_quota_notice = None;
                let first_seen = *state.quota_candidate_since.get_or_insert(input.now);
                let corroborated = input.agent_id.is_some_and(|agent_id| {
                    crate::issue_monitor::provider_limit_reached_for_agent(agent_id, input.accounts)
                });
                if corroborated || input.now.signed_duration_since(first_seen).num_seconds() >= 120
                {
                    state.quota_candidate_since = None;
                    let evidence = crate::IssueMonitorProviderQuotaHoldEvidence::screen_notice(
                        &input
                            .now
                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                        input.window_id,
                        input.screen.unwrap_or_default(),
                    )
                    .with_poller(input.agent_id, input.accounts);
                    let failure =
                        provider_usage_limit_failure(&notice, input.agent_id, Some(evidence));
                    outcome.detail = Some(gwt_core::usage::describe_provider_limit_notice(
                        &notice,
                        Some(provider_label(&notice, input.agent_id).as_str()),
                    ));
                    outcome.failure = Some(failure.clone());
                    state.quota_hold = Some(failure);
                }
            }
        }
    }
    let auth_agent = input
        .screen
        .and_then(gwt_core::usage::limit_notice::detect_provider_auth_notice)
        .filter(|provider| Some(*provider) == quota_provider(input.agent_id))
        .map(|provider| match provider {
            gwt_core::usage::UsageProvider::Codex => "codex",
            gwt_core::usage::UsageProvider::ClaudeCode => "claude",
        });
    if let Some(agent_id) = auth_agent {
        state.quota_hold = None;
        state.quota_candidate_since = None;
        outcome.failure = Some(crate::IssueMonitorFailure::ProviderAuthenticationRequired {
            provider: agent_id.into(),
        });
    } else if state.quota_hold.is_some() {
        state.api_error = None;
        return outcome;
    }
    let detected = auth_agent
        .map(|agent_id| gwt_core::usage::ProviderApiError {
            http_status: None,
            summary: crate::issue_monitor::provider_reauthentication_required_message(agent_id),
            transient: false,
        })
        .or_else(|| {
            input
                .screen
                .and_then(gwt_core::usage::detect_provider_api_error)
        });
    match detected {
        Some(error) if state.api_error.as_ref() != Some(&error) => {
            outcome.detail = Some(if auth_agent.is_some() {
                error.summary.clone()
            } else {
                gwt_core::usage::describe_provider_api_error(&error, input.agent_id)
            });
            state.api_error = Some(error);
        }
        None if state.api_error.take().is_some() => outcome.clear_detail = true,
        _ => {}
    }
    outcome
}

pub fn release_policy_quota(
    state: &mut RuntimePolicyState,
    accounts: &[gwt_core::usage::ProviderUsage],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    let Some(crate::IssueMonitorFailure::ProviderUsageLimit {
        provider,
        evidence: Some(evidence),
        ..
    }) = state.quota_hold.as_ref()
    else {
        return false;
    };
    let contradicted = accounts
        .iter()
        .find(|account| {
            Some(account.provider) == crate::issue_monitor::usage_provider_for_agent(provider)
        })
        .is_some_and(|account| evidence.contradicted_by(account, now));
    if !contradicted {
        return false;
    }
    state.released_quota_notice = evidence.screen_text.as_deref().and_then(|screen| {
        gwt_core::usage::detect_provider_limit_notice(screen, &now.with_timezone(&chrono::Local))
    });
    state.quota_hold = None;
    state.quota_candidate_since = None;
    true
}

#[cfg(test)]
mod tests {
    use crate as gwt;

    #[test]
    fn exit_policy_does_not_relatch_released_quota_but_keeps_a_fresh_refusal() {
        use super::{classify_exit, ExitPolicyInput, RuntimePolicyState};
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-22T09:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let screen="■ You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage\n  to purchase more credits or try again at Aug 22nd, 2026 12:46 PM.";
        let notice = gwt_core::usage::detect_provider_limit_notice(
            screen,
            &now.with_timezone(&chrono::Local),
        )
        .expect("existing provider quota fixture");
        let input = || ExitPolicyInput {
            window_id: "owner::agent",
            preset: crate::WindowPreset::Agent,
            agent_id: Some("codex"),
            has_agent_session: true,
            raw_status: crate::WindowProcessStatus::Stopped,
            hook_state: Some(crate::WindowProcessStatus::Idle),
            exit_confirmed: true,
            exact_incarnation: true,
            detail: Some("Process exited".into()),
            final_screen: Some(screen),
            accounts: &[],
            now,
        };
        let mut released = RuntimePolicyState {
            released_quota_notice: Some(notice),
            ..Default::default()
        };
        let clean = classify_exit(&mut released, input());
        assert_eq!(clean.status, crate::WindowProcessStatus::Stopped);
        assert!(
            clean.auto_close,
            "an account-switched old refusal cannot retain an exited agent"
        );
        assert!(!clean.keep_recovery);
        assert!(clean.quota_hold.is_none());
        let mut fresh = RuntimePolicyState::default();
        let held = classify_exit(&mut fresh, input());
        assert_eq!(held.status, crate::WindowProcessStatus::Waiting);
        assert!(!held.auto_close);
        assert!(held.keep_recovery);
        assert!(matches!(
            held.quota_hold,
            Some(crate::IssueMonitorFailure::ProviderUsageLimit { .. })
        ));
    }

    #[test]
    fn a_missing_launcher_binary_is_named_in_the_pane_detail() {
        let missing = r"C:\Users\dev\AppData\Local\npm-cache\_npx\2842f47953679de1\node_modules\.bin\..\@anthropic-ai\claude-code\bin\claude.exe";

        // The shim prints the path first, so a raw tail loses it to truncation
        // exactly when the screen carries anything else.
        let scrollback = "x".repeat(super::AGENT_ERROR_TAIL_MAX_CHARS);
        let detail = super::compose_agent_error_detail(
            Some("Process exited with status 1".to_string()),
            Some(&format!(
                "'\"{missing}\"' \u{306f}\u{3001}\u{5185}\u{90e8}\u{30b3}\u{30de}\u{30f3}\u{30c9}\u{307e}\u{305f}\u{306f}\u{5916}\u{90e8}\u{30b3}\u{30de}\u{30f3}\u{30c9}\u{3068}\u{3057}\u{3066}\u{8a8d}\u{8b58}\u{3055}\u{308c}\u{3066}\u{3044}\u{307e}\u{305b}\u{3093}\u{3002}\n{scrollback}"
            )),
        )
        .expect("an errored agent pane always carries a detail");

        assert!(
            detail.contains(missing),
            "the pane must name the executable that was not found: {detail}"
        );
        assert!(
            !detail.starts_with("Process exited with status 1"),
            "a bare exit status is what the operator could not diagnose: {detail}"
        );
    }

    #[test]
    fn codex_shared_state_lock_replaces_the_raw_stack_in_the_pane_detail() {
        let observed_stack = "/Users/akiojin/.codex/state_5.sqlite: failed to initialize state \
             runtime at /Users/akiojin/.codex: failed to open log DB at \
             /Users/akiojin/.codex/logs_2.sqlite: error returned from database: (code: 5) \
             database is locked";

        let detail = super::compose_agent_error_detail(
            Some("Agent exited with status 1".to_string()),
            Some(observed_stack),
        )
        .expect("an errored agent pane always carries a detail");

        assert!(
            !detail.contains("state_5.sqlite"),
            "the raw Codex stack must not reach the pane: {detail}"
        );
        assert!(
            detail.contains("~/.codex"),
            "the pane must name the shared state directory as the cause: {detail}"
        );
        assert!(
            detail.contains("max_active"),
            "the pane must name what the operator can do about it: {detail}"
        );
        assert!(
            gwt_agent::is_transient_launch_failure(&detail),
            "the Issue Monitor must requeue this without spending an attempt: {detail}"
        );
    }

    #[test]
    fn issue_5415_codex_sandbox_startup_error_retries_without_spending_an_attempt() {
        let error =
            "Could not read Windows sandbox configuration and requirements from the app server.";
        let detail = super::compose_agent_error_detail(None, Some(error)).unwrap();
        assert!(detail.contains(error), "keep the provider's diagnostic");
        assert!(gwt_agent::is_transient_launch_failure(&detail));
    }

    #[test]
    fn late_provider_active_writer_error_is_classified_as_resume_writer_conflict() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::classify_issue_monitor_failure(detail, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "a late provider race is typed immediately even when the external holder is unknown"
        );
        for prefixed_detail in [
            "Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            "Agent exited — last output: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
        ] {
            assert_eq!(
                super::classify_issue_monitor_failure(
                    prefixed_detail,
                    gwt_agent::SessionMode::Resume,
                ),
                Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                    holder_window_id: None,
                }),
                "the provider's observed Error: prefix must preserve typed recovery"
            );
        }
        assert_eq!(
            super::classify_issue_monitor_failure(
                "thread/resume failed during TUI bootstrap: provider temporarily unavailable",
                gwt_agent::SessionMode::Resume,
            ),
            None,
            "generic provider failures must stay on the existing failure path"
        );

        for (case, unrelated_detail) in [
            (
                "reversed markers",
                "thread/resume failed during TUI bootstrap: (code -32600) thread 019 already has an active writer",
            ),
            (
                "markers split across lines",
                "thread/resume failed during TUI bootstrap:\nthread 019 already has an active writer (code -32600)",
            ),
            (
                "different provider code",
                "thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32601)",
            ),
            (
                "quoted compound diagnostic",
                "provider wrapper repeated \"thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)\"",
            ),
            (
                "unanchored Error prefix",
                "provider wrapper: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            ),
            (
                "nested last-output prefix",
                "Agent exited — last output: Warning: Error: Failed to resume session from ~/.codex/sessions/rollout.jsonl: thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)",
            ),
        ] {
            assert_eq!(
                super::classify_issue_monitor_failure(
                    unrelated_detail,
                    gwt_agent::SessionMode::Resume,
                ),
                None,
                "{case} must not be promoted to a typed writer conflict"
            );
        }
    }

    #[test]
    fn agent_failed_payload_carries_classified_resume_writer_conflict_to_daemon() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::issue_monitor_agent_failed_payload(
                "tab-1::agent-42",
                detail,
                Some(42),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "agent_failed": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-42",
                    "message": detail,
                    "failure": {
                        "kind": "resume_writer_conflict",
                    },
                }
            }),
            "the payload helper used by issue_monitor_agent_failed_events must bridge the classifier into the daemon envelope"
        );
        assert_eq!(
            super::issue_monitor_agent_failed_payload(
                "tab-1::agent-42",
                "provider temporarily unavailable",
                Some(42),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "agent_failed": {
                    "issue_number": 42,
                    "window_id": "tab-1::agent-42",
                    "message": "provider temporarily unavailable",
                }
            }),
            "generic failures must keep the legacy message-only envelope"
        );
    }

    #[test]
    fn launch_failed_payload_carries_classified_resume_writer_conflict_and_source_identity() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";

        assert_eq!(
            super::issue_monitor_launch_failed_payload(
                42,
                detail,
                Some("launch:effect-42"),
                Some("gui-42"),
                gwt_agent::SessionMode::Resume,
            ),
            serde_json::json!({
                "launch_failed": {
                    "issue_number": 42,
                    "message": detail,
                    "delivery_id": "launch:effect-42",
                    "materializer_id": "gui-42",
                    "failure": {
                        "kind": "resume_writer_conflict",
                    },
                }
            })
        );
    }

    #[test]
    fn resume_writer_conflict_classification_and_payload_require_resume_session_origin() {
        let detail = "Failed to resume session from ~/.codex/sessions/rollout.jsonl: \
            thread/resume failed during TUI bootstrap: thread 019 already has an active writer \
            (code -32600)";
        let classify: fn(&str, gwt_agent::SessionMode) -> Option<gwt::IssueMonitorFailure> =
            super::classify_issue_monitor_failure;
        let agent_payload: fn(
            &str,
            &str,
            Option<u64>,
            gwt_agent::SessionMode,
        ) -> serde_json::Value = super::issue_monitor_agent_failed_payload;
        let launch_payload: fn(
            u64,
            &str,
            Option<&str>,
            Option<&str>,
            gwt_agent::SessionMode,
        ) -> serde_json::Value = super::issue_monitor_launch_failed_payload;

        assert_eq!(
            classify(detail, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "the exact provider diagnostic is typed only for an actual Resume launch"
        );
        let composed = format!("Process exited with status 1 — last output: {detail}");
        assert_eq!(
            classify(&composed, gwt_agent::SessionMode::Resume),
            Some(gwt::IssueMonitorFailure::ResumeWriterConflict {
                holder_window_id: None,
            }),
            "the runtime-composed error detail retains the anchored provider diagnostic",
        );
        for (case, echoed_detail) in [
            ("raw normal output", detail),
            (
                "single-quoted fresh output echo",
                "launcher echoed 'thread/resume failed during TUI bootstrap: thread 019 already has an active writer (code -32600)'",
            ),
        ] {
            assert_eq!(
                classify(echoed_detail, gwt_agent::SessionMode::Normal),
                None,
                "{case} is generic because a Normal/fresh launch cannot encounter a real Resume writer race"
            );
        }

        let resume_agent = agent_payload(
            "tab-1::agent-42",
            detail,
            Some(42),
            gwt_agent::SessionMode::Resume,
        );
        assert_eq!(
            resume_agent
                .pointer("/agent_failed/failure/kind")
                .and_then(serde_json::Value::as_str),
            Some("resume_writer_conflict")
        );
        let fresh_agent = agent_payload(
            "tab-1::agent-42",
            detail,
            Some(42),
            gwt_agent::SessionMode::Normal,
        );
        assert!(
            fresh_agent.pointer("/agent_failed/failure").is_none(),
            "a fresh AgentFailed payload keeps the marker as a generic message"
        );

        let resume_launch = launch_payload(
            42,
            detail,
            Some("launch:effect-42"),
            Some("gui-42"),
            gwt_agent::SessionMode::Resume,
        );
        assert_eq!(
            resume_launch
                .pointer("/launch_failed/failure/kind")
                .and_then(serde_json::Value::as_str),
            Some("resume_writer_conflict")
        );
        let fresh_launch = launch_payload(
            42,
            detail,
            Some("launch:effect-42"),
            Some("gui-42"),
            gwt_agent::SessionMode::Normal,
        );
        assert!(
            fresh_launch.pointer("/launch_failed/failure").is_none(),
            "a FreshRequired/Normal LaunchFailed payload cannot synthesize a Resume conflict"
        );
    }
}

use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use gwt_agent::{
    Session, GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
    GWT_SESSION_RUNTIME_PATH_ENV,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::cli::hook::{
    coordination_event, forward, resolve_hook_agent_session_id, runtime_state, HookAgentSessionId,
    HookError, RawHookEvent,
};

const HOOK_LIVE_TIMEOUT_MS: u64 = 100;
const HOOK_LIVE_OVERALL_DEADLINE_MS: u64 = 1_000;
const HOOK_LIVE_RETRY_DELAY_MS: u64 = 25;
const AGENT_BRIDGE_ERROR_BODY_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy)]
struct HookLiveRetryPolicy {
    per_attempt_timeout: Duration,
    overall_deadline: Duration,
    retry_delay: Duration,
}

impl HookLiveRetryPolicy {
    fn production() -> Self {
        Self {
            per_attempt_timeout: Duration::from_millis(HOOK_LIVE_TIMEOUT_MS),
            overall_deadline: Duration::from_millis(HOOK_LIVE_OVERALL_DEADLINE_MS),
            retry_delay: Duration::from_millis(HOOK_LIVE_RETRY_DELAY_MS),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HookLiveAttemptFailure {
    Timeout,
    Transport,
    Http(reqwest::StatusCode),
}

impl HookLiveAttemptFailure {
    fn is_retryable(self) -> bool {
        match self {
            Self::Timeout | Self::Transport => true,
            Self::Http(status) => {
                status == reqwest::StatusCode::REQUEST_TIMEOUT
                    || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
            }
        }
    }

    fn diagnostic(self) -> String {
        match self {
            Self::Timeout => "hook live event transport timed out".to_string(),
            Self::Transport => "hook live event transport failed".to_string(),
            Self::Http(status) => {
                format!(
                    "hook live endpoint returned http_status={}",
                    status.as_u16()
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentBridgeFailureReason {
    TransportFailure,
    AuthorityMismatch,
    ReceiptMismatch,
    WorkspaceEnsureRequired,
    OperationRejected,
}

impl AgentBridgeFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::TransportFailure => "transport_failure",
            Self::AuthorityMismatch => "authority_mismatch",
            Self::ReceiptMismatch => "receipt_mismatch",
            Self::WorkspaceEnsureRequired => "workspace_ensure_required",
            Self::OperationRejected => "operation_rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentBridgeFailure {
    reason: AgentBridgeFailureReason,
    http_status: Option<reqwest::StatusCode>,
    error_code: Option<crate::AgentWorkspaceUpdateErrorCode>,
    bridge_code: Option<String>,
    bridge_reason: Option<String>,
    exact_workspace_ensure_required: bool,
    message: &'static str,
}

impl AgentBridgeFailure {
    fn new(reason: AgentBridgeFailureReason, message: &'static str) -> Self {
        Self {
            reason,
            http_status: None,
            error_code: None,
            bridge_code: None,
            bridge_reason: None,
            exact_workspace_ensure_required: false,
            message,
        }
    }

    fn rejected(
        reason: AgentBridgeFailureReason,
        status: reqwest::StatusCode,
        response: Option<&WorkspaceBridgeDiagnosticResponse>,
        exact_workspace_ensure_required: bool,
        message: &'static str,
    ) -> Self {
        let bridge_code = response.and_then(|response| safe_bridge_token(&response.code));
        let bridge_reason = response.and_then(|response| safe_bridge_token(&response.reason));
        Self {
            reason,
            http_status: Some(status),
            error_code: bridge_code
                .as_deref()
                .and_then(parse_workspace_update_error_code),
            bridge_code,
            bridge_reason,
            exact_workspace_ensure_required,
            message,
        }
    }

    pub(crate) fn is_exact_workspace_ensure_required(&self) -> bool {
        self.exact_workspace_ensure_required
            && self.reason == AgentBridgeFailureReason::WorkspaceEnsureRequired
            && self.http_status == Some(reqwest::StatusCode::CONFLICT)
            && self.error_code
                == Some(crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired)
            && self.bridge_reason.as_deref() == Some("workspace_ensure_required")
    }

    pub(crate) fn is_missing_route_rejection(&self) -> bool {
        self.reason == AgentBridgeFailureReason::OperationRejected
            && matches!(
                self.http_status,
                Some(reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED)
            )
            && self.error_code != Some(crate::AgentWorkspaceUpdateErrorCode::Internal)
    }
}

impl std::fmt::Display for AgentBridgeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "[{}] {}", self.reason.as_str(), self.message)?;
        if self.http_status.is_some() || self.bridge_code.is_some() || self.bridge_reason.is_some()
        {
            formatter.write_str(" (")?;
            let mut separator = "";
            if let Some(status) = self.http_status {
                write!(formatter, "http_status={}", status.as_u16())?;
                separator = ", ";
            }
            if let Some(code) = self.bridge_code.as_deref() {
                write!(formatter, "{separator}code={code}")?;
                separator = ", ";
            }
            if let Some(reason) = self.bridge_reason.as_deref() {
                write!(formatter, "{separator}bridge_reason={reason}")?;
            }
            formatter.write_str(")")?;
        }
        Ok(())
    }
}

fn read_bounded_agent_bridge_error_body(
    response: reqwest::blocking::Response,
    message: &'static str,
) -> Result<Vec<u8>, AgentBridgeFailure> {
    if response
        .content_length()
        .is_some_and(|length| length > AGENT_BRIDGE_ERROR_BODY_MAX_BYTES)
    {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::TransportFailure,
            message,
        ));
    }
    let mut body = Vec::new();
    response
        .take(AGENT_BRIDGE_ERROR_BODY_MAX_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| {
            AgentBridgeFailure::new(AgentBridgeFailureReason::TransportFailure, message)
        })?;
    if body.len() as u64 > AGENT_BRIDGE_ERROR_BODY_MAX_BYTES {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::TransportFailure,
            message,
        ));
    }
    Ok(body)
}

fn safe_bridge_token(value: &Option<String>) -> Option<String> {
    value.as_deref().and_then(|value| {
        (!value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
        .then(|| value.to_string())
    })
}

fn parse_workspace_update_error_code(code: &str) -> Option<crate::AgentWorkspaceUpdateErrorCode> {
    match code {
        "invalid_request" => Some(crate::AgentWorkspaceUpdateErrorCode::InvalidRequest),
        "relaunch_required" => Some(crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired),
        "execution_binding_mismatch" => {
            Some(crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch)
        }
        "workspace_ensure_required" => {
            Some(crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired)
        }
        "provenance_mismatch" => Some(crate::AgentWorkspaceUpdateErrorCode::ProvenanceMismatch),
        "identity_conflict" => Some(crate::AgentWorkspaceUpdateErrorCode::IdentityConflict),
        "transaction_conflict" => Some(crate::AgentWorkspaceUpdateErrorCode::TransactionConflict),
        "internal" => Some(crate::AgentWorkspaceUpdateErrorCode::Internal),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
struct AgentBridgeErrorResponse {
    code: crate::AgentWorkspaceUpdateErrorCode,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceBridgeDiagnosticResponse {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceBridgeErrorResponse {
    code: crate::AgentWorkspaceUpdateErrorCode,
    reason: String,
    #[serde(default, rename = "message")]
    _message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeHookEventKind {
    RuntimeState,
    CoordinationEvent,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHookEvent {
    pub kind: RuntimeHookEventKind,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub gwt_session_id: Option<String>,
    /// Internal one-time Continue work readiness challenge. AppRuntime strips
    /// this field before broadcasting a runtime event to browser clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_readiness_nonce: Option<String>,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub project_root: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    pub occurred_at: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HookForwardTarget {
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for HookForwardTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HookForwardTarget")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl HookForwardTarget {
    pub fn from_env() -> Option<Self> {
        let url = std::env::var(GWT_HOOK_FORWARD_URL_ENV).ok()?;
        let token = std::env::var(GWT_HOOK_FORWARD_TOKEN_ENV).ok()?;
        let url = url.trim().to_string();
        let token = token.trim().to_string();
        if url.is_empty() || token.is_empty() {
            return None;
        }
        Some(Self { url, token })
    }

    pub fn from_env_strict() -> Result<Option<Self>, String> {
        let url = std::env::var(GWT_HOOK_FORWARD_URL_ENV);
        let token = std::env::var(GWT_HOOK_FORWARD_TOKEN_ENV);
        match (url, token) {
            (Err(std::env::VarError::NotPresent), Err(std::env::VarError::NotPresent)) => Ok(None),
            (Ok(url), Ok(token)) => {
                let target = Self {
                    url: url.trim().to_string(),
                    token: token.trim().to_string(),
                };
                if target.url.is_empty() || target.token.is_empty() {
                    return Err(
                        "agent bridge endpoint and token must both be non-empty; relaunch the Session"
                            .to_string(),
                    );
                }
                target.validate()?;
                Ok(Some(target))
            }
            _ => Err(
                "agent bridge endpoint and token must be provided together; relaunch the Session"
                    .to_string(),
            ),
        }
    }

    fn validate(&self) -> Result<(), String> {
        let url = Url::parse(&self.url).map_err(|err| format!("invalid hook live URL: {err}"))?;
        match url.scheme() {
            "http" | "https" => {}
            other => {
                return Err(format!("unsupported hook live URL scheme: {other}"));
            }
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err("hook live URL must not contain user credentials".to_string());
        }

        let Some(host) = url.host_str() else {
            return Err("hook live URL is missing a host".to_string());
        };
        if !is_allowed_hook_forward_host(host) {
            return Err(format!(
                "hook live URL must stay on loopback or a reserved container host bridge, got: {host}"
            ));
        }
        if url.port().is_none() {
            return Err("hook live URL must include an explicit port".to_string());
        }
        if url.path() != "/internal/hook-live" || url.query().is_some() || url.fragment().is_some()
        {
            return Err(
                "hook live URL must use the exact /internal/hook-live path without query or fragment"
                    .to_string(),
            );
        }

        Ok(())
    }

    pub fn workspace_update_url(&self) -> Result<Url, String> {
        self.validate()?;
        let mut url =
            Url::parse(&self.url).map_err(|error| format!("invalid agent bridge URL: {error}"))?;
        url.set_path("/internal/workspace-update");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    pub fn work_terminalization_url(&self) -> Result<Url, String> {
        self.validate()?;
        let mut url =
            Url::parse(&self.url).map_err(|error| format!("invalid agent bridge URL: {error}"))?;
        url.set_path("/internal/work-terminalization");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    pub fn blocked_build_abort_terminalization_url(&self) -> Result<Url, String> {
        self.validate()?;
        let mut url =
            Url::parse(&self.url).map_err(|error| format!("invalid agent bridge URL: {error}"))?;
        url.set_path("/internal/build-abort-terminalization");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }

    pub fn execution_continuation_url(&self) -> Result<Url, String> {
        self.validate()?;
        let mut url =
            Url::parse(&self.url).map_err(|error| format!("invalid agent bridge URL: {error}"))?;
        url.set_path("/internal/execution-continuation");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }
}

pub fn send_execution_continuation_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentExecutionContinuationRequest,
) -> Result<crate::AgentExecutionContinuationReceipt, String> {
    let url = target.execution_continuation_url().map_err(|_| {
        AgentBridgeFailure::new(
            AgentBridgeFailureReason::TransportFailure,
            "Host continuation bridge target is invalid",
        )
        .to_string()
    })?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "failed to build the Host continuation bridge client",
            )
            .to_string()
        })?;
    let response = client
        .post(url)
        .bearer_auth(&target.token)
        .json(request)
        .send()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "Host continuation bridge is unavailable; no local fallback was attempted",
            )
            .to_string()
        })?;
    if !response.status().is_success() {
        let reason = response
            .json::<AgentBridgeErrorResponse>()
            .map(|error| {
                if error.code == crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch
                    || error.reason.as_deref() == Some("authority_mismatch")
                {
                    AgentBridgeFailureReason::AuthorityMismatch
                } else {
                    AgentBridgeFailureReason::OperationRejected
                }
            })
            .unwrap_or(AgentBridgeFailureReason::OperationRejected);
        return Err(AgentBridgeFailure::new(
            reason,
            "Host continuation bridge rejected the operation; no local fallback was attempted",
        )
        .to_string());
    }
    let receipt = response
        .json::<crate::AgentExecutionContinuationReceipt>()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::ReceiptMismatch,
                "Host continuation bridge returned an invalid success response",
            )
            .to_string()
        })?;
    if receipt.schema_version != crate::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION
        || receipt.operation_id != request.operation_id
        || receipt.generation_id != receipt.execution_binding.generation_id
        || receipt.capability_generation == 0
        || !receipt.validated
    {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::ReceiptMismatch,
            "Host continuation bridge returned mismatched authority evidence",
        )
        .to_string());
    }
    Ok(receipt)
}

pub(crate) fn send_workspace_update_via_agent_bridge_detailed(
    target: &HookForwardTarget,
    request: &crate::AgentWorkspaceUpdateRequest,
) -> Result<crate::AgentWorkspaceUpdateReceipt, AgentBridgeFailure> {
    let url = target.workspace_update_url().map_err(|_| {
        AgentBridgeFailure::new(
            AgentBridgeFailureReason::TransportFailure,
            "Host workspace bridge target is invalid",
        )
    })?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "failed to build the Host workspace bridge client",
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(&target.token)
        .json(request)
        .send()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "Host workspace bridge is unavailable; the update was not retried locally and its outcome may be unknown",
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = read_bounded_agent_bridge_error_body(
            response,
            "Host workspace bridge rejection body could not be read safely; no local fallback was attempted",
        )?;
        let diagnostic = serde_json::from_slice::<WorkspaceBridgeDiagnosticResponse>(&body).ok();
        let strict = serde_json::from_slice::<WorkspaceBridgeErrorResponse>(&body).ok();
        let exact_workspace_ensure_required = strict.as_ref().is_some_and(|error| {
            status == reqwest::StatusCode::CONFLICT
                && error.code == crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
                && error.reason == "workspace_ensure_required"
        });
        let diagnostic_code = diagnostic
            .as_ref()
            .and_then(|error| safe_bridge_token(&error.code))
            .as_deref()
            .and_then(parse_workspace_update_error_code);
        let diagnostic_reason = diagnostic
            .as_ref()
            .and_then(|error| safe_bridge_token(&error.reason));
        let reason = if exact_workspace_ensure_required {
            AgentBridgeFailureReason::WorkspaceEnsureRequired
        } else if diagnostic_code
            == Some(crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch)
            || diagnostic_reason.as_deref() == Some("authority_mismatch")
        {
            AgentBridgeFailureReason::AuthorityMismatch
        } else {
            AgentBridgeFailureReason::OperationRejected
        };
        return Err(AgentBridgeFailure::rejected(
            reason,
            status,
            diagnostic.as_ref(),
            exact_workspace_ensure_required,
            "Host workspace bridge rejected the update; no local fallback was attempted",
        ));
    }
    let receipt = response
        .json::<crate::AgentWorkspaceUpdateReceipt>()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::ReceiptMismatch,
                "Host workspace bridge returned an invalid success response; no local fallback was attempted",
            )
        })?;
    if receipt.schema_version != crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION
        || receipt.work_id.trim().is_empty()
        || receipt.journal_entry_id.trim().is_empty()
    {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::ReceiptMismatch,
            "Host workspace bridge returned invalid receipt evidence; no local fallback was attempted",
        ));
    }
    Ok(receipt)
}

pub fn send_workspace_update_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentWorkspaceUpdateRequest,
) -> Result<crate::AgentWorkspaceUpdateReceipt, String> {
    send_workspace_update_via_agent_bridge_detailed(target, request)
        .map_err(|error| error.to_string())
}

pub fn send_work_terminalization_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentWorkTerminalizationRequest,
) -> Result<crate::AgentWorkTerminalizationReceipt, String> {
    let url = target.work_terminalization_url()?;
    send_terminalization_via_agent_bridge(target, url, request).map_err(|error| error.to_string())
}

pub(crate) fn send_blocked_build_abort_terminalization_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentBuildAbortTerminalizationRequest,
) -> Result<crate::AgentWorkTerminalizationReceipt, AgentBridgeFailure> {
    let url = target
        .blocked_build_abort_terminalization_url()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "Host build abort terminalization bridge target is invalid",
            )
        })?;
    send_terminalization_via_agent_bridge(target, url, request)
}

fn send_terminalization_via_agent_bridge(
    target: &HookForwardTarget,
    url: Url,
    request: &impl Serialize,
) -> Result<crate::AgentWorkTerminalizationReceipt, AgentBridgeFailure> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "failed to build the Host Work terminalization bridge client",
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(&target.token)
        .json(request)
        .send()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "Host Work terminalization bridge is unavailable; the close was not retried locally and its outcome may be unknown",
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = read_bounded_agent_bridge_error_body(
            response,
            "Host Work terminalization bridge rejection body could not be read safely; no local fallback was attempted",
        )?;
        let diagnostic = serde_json::from_slice::<WorkspaceBridgeDiagnosticResponse>(&body).ok();
        let diagnostic_code = diagnostic
            .as_ref()
            .and_then(|error| safe_bridge_token(&error.code))
            .as_deref()
            .and_then(parse_workspace_update_error_code);
        let diagnostic_reason = diagnostic
            .as_ref()
            .and_then(|error| safe_bridge_token(&error.reason));
        let reason = if diagnostic_code
            == Some(crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch)
            || diagnostic_reason.as_deref() == Some("authority_mismatch")
        {
            AgentBridgeFailureReason::AuthorityMismatch
        } else {
            AgentBridgeFailureReason::OperationRejected
        };
        return Err(AgentBridgeFailure::rejected(
            reason,
            status,
            diagnostic.as_ref(),
            false,
            "Host Work terminalization bridge rejected the close; no local fallback was attempted",
        ));
    }
    let receipt = response
        .json::<crate::AgentWorkTerminalizationReceipt>()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::ReceiptMismatch,
                "Host Work terminalization bridge returned an invalid success response; no local fallback was attempted",
            )
        })?;
    if receipt.schema_version != crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::ReceiptMismatch,
            "Host Work terminalization bridge returned an unsupported response schema; no local fallback was attempted",
        ));
    }
    Ok(receipt)
}

pub fn handle_runtime_state(event: &str, input: &str) -> Result<(), HookError> {
    if std::env::var_os(GWT_SESSION_RUNTIME_PATH_ENV).is_none() {
        return Ok(());
    }
    runtime_state::handle_with_input(event, input)?;
    emit_live_event_fail_open(RuntimeHookEvent::from_hook(
        RuntimeHookEventKind::RuntimeState,
        Some(event),
        runtime_state::status_for_event(event).map(str::to_string),
        None,
        current_session_from_env(),
        parse_hook_event_best_effort(input),
    ));
    Ok(())
}

pub fn handle_blocked_stop_runtime_state(input: &str) -> Result<(), HookError> {
    if std::env::var_os(GWT_SESSION_RUNTIME_PATH_ENV).is_none() {
        return Ok(());
    }
    runtime_state::record_blocked_stop_from_env()?;
    emit_live_event_fail_open(RuntimeHookEvent::from_hook(
        RuntimeHookEventKind::RuntimeState,
        Some("Stop"),
        Some("Running".to_string()),
        Some("blocked-stop".to_string()),
        current_session_from_env(),
        parse_hook_event_best_effort(input),
    ));
    Ok(())
}

pub fn handle_coordination_event(event: &str, input: &str) -> Result<(), HookError> {
    coordination_event::handle(event)?;
    emit_live_event_fail_open(RuntimeHookEvent::from_hook(
        RuntimeHookEventKind::CoordinationEvent,
        Some(event),
        None,
        Some(format!("coordination:{event}")),
        current_session_from_env(),
        parse_hook_event_best_effort(input),
    ));
    Ok(())
}

pub fn handle_forward(input: &str) -> Result<(), HookError> {
    forward::handle_with_input(input)?;
    emit_live_event_fail_open(RuntimeHookEvent::from_hook(
        RuntimeHookEventKind::Forward,
        None,
        None,
        None,
        current_session_from_env(),
        parse_hook_event_best_effort(input),
    ));
    Ok(())
}

impl RuntimeHookEvent {
    fn from_hook(
        kind: RuntimeHookEventKind,
        source_event: Option<&str>,
        status: Option<String>,
        message: Option<String>,
        session: Option<Session>,
        hook_event: Option<RawHookEvent>,
    ) -> Self {
        let project_root = session
            .as_ref()
            .map(|session| session.worktree_path.display().to_string())
            .or_else(|| {
                hook_event
                    .as_ref()
                    .and_then(|event| event.cwd().map(str::to_string))
            });
        let branch = session.as_ref().map(|session| session.branch.clone());
        let agent_session_id =
            live_event_agent_session_id(&kind, source_event, session.as_ref(), hook_event.as_ref());

        Self {
            kind,
            source_event: source_event.map(str::to_string),
            gwt_session_id: std::env::var(GWT_SESSION_ID_ENV).ok(),
            continuation_readiness_nonce: std::env::var(
                gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV,
            )
            .ok(),
            agent_session_id,
            project_root,
            branch,
            status,
            tool_name: hook_event
                .as_ref()
                .and_then(|event| event.tool_name().map(str::to_string)),
            message,
            occurred_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        }
    }
}

fn live_event_agent_session_id(
    kind: &RuntimeHookEventKind,
    source_event: Option<&str>,
    session: Option<&Session>,
    hook_event: Option<&RawHookEvent>,
) -> Option<String> {
    match resolve_hook_agent_session_id(session, hook_event) {
        HookAgentSessionId::Provided(agent_session_id) => {
            return Some(agent_session_id.into_string());
        }
        HookAgentSessionId::MissingRequiredForCodex => {
            // Codex omits a usable session_id on tool-use events; fall back to
            // the persisted resume id (captured at SessionStart). Only warn when
            // there is genuinely nothing to fall back to, so the common case
            // does not spam stderr on every tool call.
            if session
                .and_then(gwt_agent::Session::exact_resume_session_id)
                .is_none()
            {
                let gwt_session_id =
                    std::env::var(GWT_SESSION_ID_ENV).unwrap_or_else(|_| "-".to_string());
                let source_event = source_event.unwrap_or("-");
                let tool_name = hook_event.and_then(RawHookEvent::tool_name).unwrap_or("-");
                eprintln!(
                    "gwtd hook live event: missing Codex hook session_id kind={kind:?} source_event={source_event} gwt_session_id={gwt_session_id} persisted_agent_session_id=- tool_name={tool_name}"
                );
            }
        }
        HookAgentSessionId::MissingOptional => {}
    }

    session
        .and_then(gwt_agent::Session::exact_resume_session_id)
        .map(str::to_string)
}

fn emit_live_event_fail_open(event: RuntimeHookEvent) {
    if let Err(error) = emit_live_event(&event) {
        eprintln!("gwtd hook live event: {error}");
    }
}

fn emit_live_event(event: &RuntimeHookEvent) -> Result<(), String> {
    let Some(target) = HookForwardTarget::from_env() else {
        return Ok(());
    };
    emit_live_event_with_policy(event, &target, HookLiveRetryPolicy::production())
}

fn emit_live_event_with_policy(
    event: &RuntimeHookEvent,
    target: &HookForwardTarget,
    policy: HookLiveRetryPolicy,
) -> Result<(), String> {
    target.validate()?;

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| format!("build hook live client failed: {err}"))?;
    let readiness_delivery = event.source_event.as_deref() == Some("SessionStart")
        && event.continuation_readiness_nonce.is_some();
    let overall_timeout = if readiness_delivery {
        policy.overall_deadline
    } else {
        policy.per_attempt_timeout
    };
    let started = Instant::now();
    let deadline = started.checked_add(overall_timeout).unwrap_or(started);
    let mut attempts = 0usize;

    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(format!(
                "hook live readiness delivery exhausted its bounded deadline after {attempts} attempts"
            ));
        };
        if remaining.is_zero() {
            return Err(format!(
                "hook live readiness delivery exhausted its bounded deadline after {attempts} attempts"
            ));
        }
        attempts += 1;
        let attempt_timeout = policy.per_attempt_timeout.min(remaining);
        let failure = match client
            .post(&target.url)
            .bearer_auth(&target.token)
            .json(event)
            .timeout(attempt_timeout)
            .send()
        {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => HookLiveAttemptFailure::Http(response.status()),
            Err(error) if error.is_timeout() => HookLiveAttemptFailure::Timeout,
            Err(_) => HookLiveAttemptFailure::Transport,
        };

        if !readiness_delivery || !failure.is_retryable() {
            return Err(failure.diagnostic());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_default();
        if remaining <= policy.retry_delay {
            return Err(format!(
                "hook live readiness delivery exhausted its bounded deadline after {attempts} attempts: {}",
                failure.diagnostic()
            ));
        }
        std::thread::sleep(policy.retry_delay);
    }
}

fn parse_hook_event_best_effort(input: &str) -> Option<RawHookEvent> {
    RawHookEvent::read_from_str(input).ok().flatten()
}

fn current_session_from_env() -> Option<Session> {
    let session_id = std::env::var_os(GWT_SESSION_ID_ENV)?;
    let sessions_dir =
        session_dir_from_runtime_path_env().unwrap_or_else(gwt_core::paths::gwt_sessions_dir);
    let path = sessions_dir.join(format!("{}.toml", session_id.to_string_lossy()));
    if !path.exists() {
        return None;
    }
    match Session::load_and_migrate(&path) {
        Ok(session) => Some(session),
        Err(error) => {
            eprintln!(
                "gwtd hook live event: failed to load session metadata {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn session_dir_from_runtime_path_env() -> Option<PathBuf> {
    let runtime_path = PathBuf::from(std::env::var_os(GWT_SESSION_RUNTIME_PATH_ENV)?);
    gwt_agent::sessions_dir_from_runtime_path(&runtime_path)
}

fn is_allowed_hook_forward_host(host: &str) -> bool {
    let normalized = host
        .strip_prefix('[')
        .and_then(|candidate| candidate.strip_suffix(']'))
        .unwrap_or(host);
    normalized.eq_ignore_ascii_case("host.docker.internal")
        || normalized.eq_ignore_ascii_case("host.containers.internal")
        || is_loopback_host(normalized)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::{Duration, Instant},
    };

    use super::*;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };

    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    use gwt_core::test_support::ScopedEnvVar;
    use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

    #[derive(Clone)]
    struct HookLiveTestState {
        attempts: Arc<AtomicUsize>,
        responses: Arc<Vec<(Duration, StatusCode)>>,
    }

    struct HookLiveTestServer {
        runtime: Runtime,
        shutdown_tx: Option<oneshot::Sender<()>>,
        attempts: Arc<AtomicUsize>,
        forward_url: String,
    }

    impl HookLiveTestServer {
        fn start(responses: Vec<(Duration, StatusCode)>) -> Self {
            assert!(!responses.is_empty(), "at least one response is required");
            let runtime = Runtime::new().expect("hook live test runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("hook live test listener");
            let address = listener.local_addr().expect("hook live test address");
            let attempts = Arc::new(AtomicUsize::new(0));
            let state = HookLiveTestState {
                attempts: Arc::clone(&attempts),
                responses: Arc::new(responses),
            };
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let app = Router::new()
                .route(
                    "/internal/hook-live",
                    post(
                        |State(state): State<HookLiveTestState>,
                         Json(_body): Json<RuntimeHookEvent>| async move {
                            let attempt = state.attempts.fetch_add(1, Ordering::SeqCst);
                            let &(delay, status) = state
                                .responses
                                .get(attempt)
                                .unwrap_or_else(|| state.responses.last().expect("response"));
                            tokio::time::sleep(delay).await;
                            status
                        },
                    ),
                )
                .with_state(state);
            runtime.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("hook live test server");
            });
            Self {
                runtime,
                shutdown_tx: Some(shutdown_tx),
                attempts,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn target(&self, token: &str) -> HookForwardTarget {
            HookForwardTarget {
                url: self.forward_url.clone(),
                token: token.to_string(),
            }
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    impl Drop for HookLiveTestServer {
        fn drop(&mut self) {
            if let Some(shutdown_tx) = self.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
            self.runtime
                .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
        }
    }

    fn hook_live_test_event(source_event: &str, readiness_nonce: Option<&str>) -> RuntimeHookEvent {
        RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some(source_event.to_string()),
            gwt_session_id: Some("gwt-session-test".to_string()),
            continuation_readiness_nonce: readiness_nonce.map(str::to_string),
            agent_session_id: Some("agent-session-test".to_string()),
            project_root: Some("/tmp/project".to_string()),
            branch: Some("work/issue-3480".to_string()),
            status: Some("Running".to_string()),
            tool_name: None,
            message: None,
            occurred_at: "2026-08-15T00:00:00Z".to_string(),
        }
    }

    fn short_hook_live_retry_policy() -> HookLiveRetryPolicy {
        HookLiveRetryPolicy {
            per_attempt_timeout: Duration::from_millis(40),
            overall_deadline: Duration::from_millis(250),
            retry_delay: Duration::from_millis(5),
        }
    }

    fn hook_live_status_retry_policy() -> HookLiveRetryPolicy {
        HookLiveRetryPolicy {
            per_attempt_timeout: Duration::from_secs(2),
            overall_deadline: Duration::from_secs(5),
            retry_delay: Duration::from_millis(5),
        }
    }

    #[test]
    fn hook_live_failure_retryability_matches_transport_and_http_contract() {
        assert!(HookLiveAttemptFailure::Timeout.is_retryable());
        assert!(HookLiveAttemptFailure::Transport.is_retryable());
        assert!(HookLiveAttemptFailure::Http(StatusCode::REQUEST_TIMEOUT).is_retryable());
        assert!(HookLiveAttemptFailure::Http(StatusCode::TOO_MANY_REQUESTS).is_retryable());
        assert!(HookLiveAttemptFailure::Http(StatusCode::INTERNAL_SERVER_ERROR).is_retryable());
        assert!(!HookLiveAttemptFailure::Http(StatusCode::BAD_REQUEST).is_retryable());
        assert!(!HookLiveAttemptFailure::Http(StatusCode::UNAUTHORIZED).is_retryable());
    }

    #[test]
    fn readiness_hook_retries_after_first_attempt_timeout_and_succeeds_within_deadline() {
        let server = HookLiveTestServer::start(vec![
            (Duration::from_millis(80), StatusCode::NO_CONTENT),
            (Duration::ZERO, StatusCode::NO_CONTENT),
        ]);
        let event = hook_live_test_event("SessionStart", Some("private-readiness-nonce"));

        emit_live_event_with_policy(
            &event,
            &server.target("private-forward-token"),
            short_hook_live_retry_policy(),
        )
        .expect("readiness hook should recover within its bounded deadline");

        assert_eq!(server.attempts(), 2);
    }

    #[test]
    fn readiness_hook_stops_retrying_at_overall_deadline_without_exposing_secrets() {
        let server =
            HookLiveTestServer::start(vec![(Duration::from_millis(100), StatusCode::NO_CONTENT)]);
        let event = hook_live_test_event("SessionStart", Some("private-readiness-nonce"));
        let policy = HookLiveRetryPolicy {
            per_attempt_timeout: Duration::from_millis(30),
            overall_deadline: Duration::from_millis(120),
            retry_delay: Duration::from_millis(10),
        };
        let started = Instant::now();

        let error =
            emit_live_event_with_policy(&event, &server.target("private-forward-token"), policy)
                .expect_err("all delayed readiness attempts must fail");

        assert!(
            started.elapsed() >= Duration::from_millis(80),
            "bounded retry must continue near its deadline"
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        assert_eq!(
            server.attempts(),
            3,
            "30ms attempts plus 10ms delays should consume three attempts before the 120ms deadline"
        );
        assert!(!error.contains("private-readiness-nonce"), "{error}");
        assert!(!error.contains("private-forward-token"), "{error}");
    }

    #[test]
    fn ordinary_hook_does_not_retry_after_attempt_timeout() {
        let server = HookLiveTestServer::start(vec![
            (Duration::from_millis(80), StatusCode::NO_CONTENT),
            (Duration::ZERO, StatusCode::NO_CONTENT),
        ]);
        let event = hook_live_test_event("PreToolUse", None);

        emit_live_event_with_policy(
            &event,
            &server.target("private-forward-token"),
            short_hook_live_retry_policy(),
        )
        .expect_err("ordinary hook remains a single fail-open transport attempt");

        assert_eq!(server.attempts(), 1);
    }

    #[test]
    fn readiness_hook_does_not_retry_permanent_client_error() {
        let server = HookLiveTestServer::start(vec![
            (Duration::ZERO, StatusCode::BAD_REQUEST),
            (Duration::ZERO, StatusCode::NO_CONTENT),
        ]);
        let event = hook_live_test_event("SessionStart", Some("private-readiness-nonce"));

        emit_live_event_with_policy(
            &event,
            &server.target("private-forward-token"),
            hook_live_status_retry_policy(),
        )
        .expect_err("permanent client error must fail immediately");

        assert_eq!(server.attempts(), 1);
    }

    #[test]
    fn readiness_hook_retries_transient_http_statuses() {
        for status in [
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let server = HookLiveTestServer::start(vec![
                (Duration::ZERO, status),
                (Duration::ZERO, StatusCode::NO_CONTENT),
            ]);
            let event = hook_live_test_event("SessionStart", Some("private-readiness-nonce"));

            emit_live_event_with_policy(
                &event,
                &server.target("private-forward-token"),
                hook_live_status_retry_policy(),
            )
            .unwrap_or_else(|error| panic!("{status} should be retried: {error}"));

            assert_eq!(server.attempts(), 2, "status={status}");
        }
    }

    struct BindingProbeServer {
        runtime: Runtime,
        shutdown_tx: Option<oneshot::Sender<()>>,
        rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        redirect_rx: mpsc::Receiver<HeaderMap>,
        forward_url: String,
    }

    #[derive(Clone)]
    struct BindingProbeState {
        tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        status: StatusCode,
        body: String,
        redirect_location: Option<String>,
        redirect_tx: mpsc::Sender<HeaderMap>,
    }

    impl BindingProbeServer {
        fn start(status: StatusCode, body: serde_json::Value) -> Self {
            Self::start_inner(status, body, None)
        }

        fn start_redirect() -> Self {
            Self::start_inner(
                StatusCode::TEMPORARY_REDIRECT,
                serde_json::Value::Null,
                Some("/redirected-continuation".to_string()),
            )
        }

        fn start_inner(
            status: StatusCode,
            body: serde_json::Value,
            redirect_location: Option<String>,
        ) -> Self {
            let runtime = Runtime::new().expect("binding probe runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("binding probe listener");
            let address = listener.local_addr().expect("binding probe address");
            let (tx, rx) = mpsc::channel();
            let (redirect_tx, redirect_rx) = mpsc::channel();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let app = Router::new()
                .route(
                    "/internal/execution-binding-probe",
                    post(
                        |headers: HeaderMap,
                         State(state): State<BindingProbeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture binding probe request");
                            (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response()
                        },
                    ),
                )
                .route(
                    "/internal/workspace-update",
                    post(
                        |headers: HeaderMap,
                         State(state): State<BindingProbeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture workspace update request");
                            (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response()
                        },
                    ),
                )
                .route(
                    "/internal/execution-continuation",
                    post(
                        |headers: HeaderMap,
                         State(state): State<BindingProbeState>,
                         Json(body): Json<serde_json::Value>| async move {
                            state
                                .tx
                                .send((headers, body))
                                .expect("capture execution continuation request");
                            let mut response = (
                                state.status,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                state.body,
                            )
                                .into_response();
                            if let Some(location) = state.redirect_location.as_deref() {
                                response.headers_mut().insert(
                                    axum::http::header::LOCATION,
                                    axum::http::HeaderValue::from_str(location)
                                        .expect("valid redirect location"),
                                );
                            }
                            response
                        },
                    ),
                )
                .route(
                    "/redirected-continuation",
                    post(
                        |headers: HeaderMap, State(state): State<BindingProbeState>| async move {
                            state
                                .redirect_tx
                                .send(headers)
                                .expect("capture redirected execution continuation request");
                            StatusCode::OK
                        },
                    ),
                )
                .with_state(BindingProbeState {
                    tx,
                    status,
                    body: body.to_string(),
                    redirect_location,
                    redirect_tx,
                });
            runtime.spawn(async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .expect("binding probe server");
            });
            Self {
                runtime,
                shutdown_tx: Some(shutdown_tx),
                rx,
                redirect_rx,
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) -> (HeaderMap, serde_json::Value) {
            self.rx
                .recv_timeout(Duration::from_secs(2))
                .expect("binding probe request")
        }

        fn assert_no_redirect(&self) {
            assert!(
                matches!(self.redirect_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "execution continuation client must not follow redirects"
            );
        }
    }

    impl Drop for BindingProbeServer {
        fn drop(&mut self) {
            if let Some(shutdown_tx) = self.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
            self.runtime
                .block_on(async { tokio::time::sleep(Duration::from_millis(10)).await });
        }
    }

    #[test]
    fn runtime_hook_event_captures_continue_work_readiness_only_for_internal_delivery() {
        let _env_lock = env_test_lock();
        let _nonce = ScopedEnvVar::set(
            gwt_agent::GWT_CONTINUE_WORK_READY_NONCE_ENV,
            "continue-ready-private",
        );

        let event = RuntimeHookEvent::from_hook(
            RuntimeHookEventKind::RuntimeState,
            Some("SessionStart"),
            Some("Running".to_string()),
            None,
            None,
            None,
        );

        assert_eq!(
            event.continuation_readiness_nonce.as_deref(),
            Some("continue-ready-private")
        );
    }

    #[test]
    fn loopback_target_rejects_remote_hosts() {
        let target = HookForwardTarget {
            url: "http://example.com/hook-live".to_string(),
            token: "secret".to_string(),
        };

        let err = target.validate().expect_err("remote host should fail");
        assert!(err.contains("loopback"));
    }

    #[test]
    fn loopback_target_accepts_localhost() {
        let target = HookForwardTarget {
            url: "http://127.0.0.1:8787/internal/hook-live".to_string(),
            token: "secret".to_string(),
        };

        target.validate().expect("loopback target");
    }

    #[test]
    fn hook_forward_target_debug_redacts_bearer() {
        let target = HookForwardTarget {
            url: "http://127.0.0.1:8787/internal/hook-live".to_string(),
            token: "agent-capability-secret-sentinel".to_string(),
        };

        let debug = format!("{target:?}");
        assert!(!debug.contains("agent-capability-secret-sentinel"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn execution_continuation_never_follows_redirects_or_forwards_its_bearer() {
        let server = BindingProbeServer::start_redirect();
        let target = HookForwardTarget {
            url: server.forward_url.clone(),
            token: "continuation-redirect-secret".to_string(),
        };
        let request = crate::AgentExecutionContinuationRequest {
            schema_version: crate::AGENT_EXECUTION_CONTINUATION_SCHEMA_VERSION,
            operation_id: "continuation-redirect".to_string(),
        };

        let error = send_execution_continuation_via_agent_bridge(&target, &request)
            .expect_err("redirected continuation bridge response must fail closed");

        assert!(!error.contains("continuation-redirect-secret"), "{error}");
        let (headers, body) = server.receive();
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer continuation-redirect-secret")
        );
        assert_eq!(body["operation_id"], "continuation-redirect");
        server.assert_no_redirect();
    }

    #[test]
    fn strict_agent_bridge_env_rejects_partial_pair_without_fallback() {
        let _env_lock = env_test_lock();
        let _url = ScopedEnvVar::set(
            GWT_HOOK_FORWARD_URL_ENV,
            "http://127.0.0.1:8787/internal/hook-live",
        );
        let _token = ScopedEnvVar::unset(GWT_HOOK_FORWARD_TOKEN_ENV);

        let error = HookForwardTarget::from_env_strict()
            .expect_err("partial agent bridge environment must fail closed");
        assert!(error.contains("provided together"), "{error}");
    }

    #[test]
    fn mutation_urls_accept_only_reserved_bridge_hosts_and_exact_hook_path() {
        for host in [
            "127.0.0.1",
            "localhost",
            "host.docker.internal",
            "host.containers.internal",
        ] {
            let target = HookForwardTarget {
                url: format!("http://{host}:45123/internal/hook-live"),
                token: "secret".to_string(),
            };
            assert_eq!(
                target
                    .workspace_update_url()
                    .unwrap_or_else(|error| panic!("{host}: {error}"))
                    .as_str(),
                format!("http://{host}:45123/internal/workspace-update")
            );
            assert_eq!(
                target
                    .work_terminalization_url()
                    .unwrap_or_else(|error| panic!("{host}: {error}"))
                    .as_str(),
                format!("http://{host}:45123/internal/work-terminalization")
            );
            assert_eq!(
                target
                    .blocked_build_abort_terminalization_url()
                    .unwrap_or_else(|error| panic!("{host}: {error}"))
                    .as_str(),
                format!("http://{host}:45123/internal/build-abort-terminalization")
            );
            assert_eq!(
                target
                    .execution_continuation_url()
                    .unwrap_or_else(|error| panic!("{host}: {error}"))
                    .as_str(),
                format!("http://{host}:45123/internal/execution-continuation")
            );
        }

        for url in [
            "http://example.com:45123/internal/hook-live",
            "http://127.0.0.1/internal/hook-live",
            "http://127.0.0.1:45123/healthz",
            "http://127.0.0.1:45123/internal/hook-live?token=forbidden",
        ] {
            let error = HookForwardTarget {
                url: url.to_string(),
                token: "secret".to_string(),
            }
            .workspace_update_url()
            .expect_err("non-canonical bridge target must fail closed");
            assert!(!error.contains("secret"));
            let error = HookForwardTarget {
                url: url.to_string(),
                token: "secret".to_string(),
            }
            .work_terminalization_url()
            .expect_err("non-canonical terminal bridge target must fail closed");
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn operation_local_bridge_failures_have_stable_reason_codes() {
        let request = crate::AgentWorkspaceUpdateRequest {
            schema_version: crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION,
            claimed_session_id: "session-reason-codes".to_string(),
            observation: crate::AgentRuntimeObservation {
                cwd: "/workspace/repo".to_string(),
                git_toplevel: "/workspace/repo".to_string(),
                repo_hash: "repo-hash".to_string(),
                branch: "work/reason-codes".to_string(),
            },
            intent: crate::AgentWorkspaceUpdateIntent::default(),
        };

        let unavailable = HookForwardTarget {
            url: "http://127.0.0.1:1/internal/hook-live".to_string(),
            token: "transport-secret".to_string(),
        };
        let transport = send_workspace_update_via_agent_bridge(&unavailable, &request)
            .expect_err("unreachable Host must be typed");
        assert!(transport.contains("transport_failure"), "{transport}");

        let oversized_server = BindingProbeServer::start(
            StatusCode::NOT_FOUND,
            serde_json::Value::String("x".repeat(64 * 1024 + 1)),
        );
        let oversized_target = HookForwardTarget {
            url: oversized_server.forward_url.clone(),
            token: "oversized-secret".to_string(),
        };
        let oversized =
            send_workspace_update_via_agent_bridge_detailed(&oversized_target, &request)
                .expect_err("oversized Host diagnostic must fail closed as transport");
        assert_eq!(
            oversized.reason,
            AgentBridgeFailureReason::TransportFailure,
            "oversized rejection bodies are not authoritative diagnostics: {oversized}"
        );
        oversized_server.receive();

        let authority_server = BindingProbeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "execution_binding_mismatch",
                "reason": "authority_mismatch",
                "message": "current authority does not match"
            }),
        );
        let authority_target = HookForwardTarget {
            url: authority_server.forward_url.clone(),
            token: "authority-secret".to_string(),
        };
        let authority = send_workspace_update_via_agent_bridge(&authority_target, &request)
            .expect_err("authority mismatch must be typed");
        assert!(authority.contains("authority_mismatch"), "{authority}");
        authority_server.receive();

        let ensure_server = BindingProbeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "workspace_ensure_required",
                "message": "old Host uses the legacy WorkItems scope"
            }),
        );
        let ensure_target = HookForwardTarget {
            url: ensure_server.forward_url.clone(),
            token: "ensure-secret".to_string(),
        };
        let ensure = send_workspace_update_via_agent_bridge_detailed(&ensure_target, &request)
            .expect_err("exact ensure-required rejection must stay typed");
        assert_eq!(ensure.http_status, Some(StatusCode::CONFLICT));
        assert_eq!(
            ensure.error_code,
            Some(crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired)
        );
        assert_eq!(
            ensure.bridge_reason.as_deref(),
            Some("workspace_ensure_required")
        );
        assert!(
            ensure.is_exact_workspace_ensure_required(),
            "exact 409/code/reason must retain the bounded compatibility signal: {ensure}"
        );
        let ensure_diagnostic = ensure.to_string();
        assert!(
            ensure_diagnostic.contains("http_status=409"),
            "{ensure_diagnostic}"
        );
        assert!(
            ensure_diagnostic.contains("code=workspace_ensure_required"),
            "{ensure_diagnostic}"
        );
        assert!(
            ensure_diagnostic.contains("bridge_reason=workspace_ensure_required"),
            "{ensure_diagnostic}"
        );
        ensure_server.receive();

        let lookalike_server = BindingProbeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "workspace_ensure_required",
                "reason": "different_reason",
                "message": "must remain a diagnostic, never compatibility authority"
            }),
        );
        let lookalike_target = HookForwardTarget {
            url: lookalike_server.forward_url.clone(),
            token: "lookalike-secret".to_string(),
        };
        let lookalike =
            send_workspace_update_via_agent_bridge_detailed(&lookalike_target, &request)
                .expect_err("a non-exact typed rejection must fail closed");
        assert!(!lookalike.is_exact_workspace_ensure_required());
        let lookalike_diagnostic = lookalike.to_string();
        assert!(
            lookalike_diagnostic.contains("http_status=409"),
            "{lookalike_diagnostic}"
        );
        assert!(
            lookalike_diagnostic.contains("code=workspace_ensure_required"),
            "{lookalike_diagnostic}"
        );
        assert!(
            lookalike_diagnostic.contains("bridge_reason=different_reason"),
            "{lookalike_diagnostic}"
        );
        lookalike_server.receive();

        let future_server = BindingProbeServer::start(
            StatusCode::CONFLICT,
            serde_json::json!({
                "code": "future_workspace_state",
                "reason": "future_host_reason",
                "message": "rolling-version diagnostic",
                "future_field": true
            }),
        );
        let future_target = HookForwardTarget {
            url: future_server.forward_url.clone(),
            token: "future-secret".to_string(),
        };
        let future = send_workspace_update_via_agent_bridge_detailed(&future_target, &request)
            .expect_err("an unknown rolling-version rejection must stay diagnostic");
        assert!(!future.is_exact_workspace_ensure_required());
        let future_diagnostic = future.to_string();
        assert!(
            future_diagnostic.contains("http_status=409"),
            "{future_diagnostic}"
        );
        assert!(
            future_diagnostic.contains("code=future_workspace_state"),
            "{future_diagnostic}"
        );
        assert!(
            future_diagnostic.contains("bridge_reason=future_host_reason"),
            "{future_diagnostic}"
        );
        future_server.receive();

        let receipt_server = BindingProbeServer::start(
            StatusCode::OK,
            serde_json::json!({
                "schema_version": crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION + 1,
                "work_id": "work-receipt",
                "journal_entry_id": "journal-receipt"
            }),
        );
        let receipt_target = HookForwardTarget {
            url: receipt_server.forward_url.clone(),
            token: "receipt-secret".to_string(),
        };
        let receipt = send_workspace_update_via_agent_bridge(&receipt_target, &request)
            .expect_err("mismatched receipt must be typed");
        assert!(receipt.contains("receipt_mismatch"), "{receipt}");
        receipt_server.receive();
    }

    #[test]
    fn forward_hook_ignores_corrupt_session_metadata() {
        let _env_lock = env_test_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("sessions dir");
        std::fs::write(sessions_dir.join("session-1.toml"), "odex\"")
            .expect("corrupt session file");
        let runtime_path = sessions_dir
            .join("runtime")
            .join("42")
            .join("session-1.json");
        let _session_id = ScopedEnvVar::set(GWT_SESSION_ID_ENV, "session-1");
        let _runtime_path = ScopedEnvVar::set(GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
        let _forward_url = ScopedEnvVar::unset(GWT_HOOK_FORWARD_URL_ENV);
        let _forward_token = ScopedEnvVar::unset(GWT_HOOK_FORWARD_TOKEN_ENV);

        handle_forward("{}").expect("forward hook remains fail-open");
    }
}

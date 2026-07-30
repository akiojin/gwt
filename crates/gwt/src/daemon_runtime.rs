use std::{path::PathBuf, time::Duration};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentBridgeFailureReason {
    TransportFailure,
    AuthorityMismatch,
    ReceiptMismatch,
    OperationRejected,
}

impl AgentBridgeFailureReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::TransportFailure => "transport_failure",
            Self::AuthorityMismatch => "authority_mismatch",
            Self::ReceiptMismatch => "receipt_mismatch",
            Self::OperationRejected => "operation_rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentBridgeFailure {
    reason: AgentBridgeFailureReason,
    message: &'static str,
}

impl AgentBridgeFailure {
    fn new(reason: AgentBridgeFailureReason, message: &'static str) -> Self {
        Self { reason, message }
    }
}

impl std::fmt::Display for AgentBridgeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "[{}] {}", self.reason.as_str(), self.message)
    }
}

#[derive(Debug, Deserialize)]
struct AgentBridgeErrorResponse {
    code: crate::AgentWorkspaceUpdateErrorCode,
    #[serde(default)]
    reason: Option<String>,
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

#[derive(Debug)]
pub(crate) enum AgentBridgeRequestError {
    NotSent(String),
    RejectedBeforeHandler(String),
    Rejected(crate::AgentWorkspaceUpdateError),
    Unknown(String),
}

impl AgentBridgeRequestError {
    pub(crate) fn rejection_code(&self) -> Option<crate::AgentWorkspaceUpdateErrorCode> {
        match self {
            Self::Rejected(error) => Some(error.code),
            Self::NotSent(_) | Self::RejectedBeforeHandler(_) | Self::Unknown(_) => None,
        }
    }

    pub(crate) fn proves_zero_mutation(&self) -> bool {
        matches!(self, Self::NotSent(_) | Self::RejectedBeforeHandler(_))
            || matches!(
                self.rejection_code(),
                Some(
                    crate::AgentWorkspaceUpdateErrorCode::RelaunchRequired
                        | crate::AgentWorkspaceUpdateErrorCode::ExecutionBindingMismatch
                        | crate::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
                )
            )
    }
}

impl std::fmt::Display for AgentBridgeRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSent(message)
            | Self::RejectedBeforeHandler(message)
            | Self::Unknown(message) => formatter.write_str(message),
            Self::Rejected(error) => write!(
                formatter,
                "Host bridge rejected the request ({:?}); no local fallback was attempted",
                error.code
            ),
        }
    }
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

    pub fn work_materialization_probe_url(&self) -> Result<Url, String> {
        self.validate()?;
        let mut url =
            Url::parse(&self.url).map_err(|error| format!("invalid agent bridge URL: {error}"))?;
        url.set_path("/internal/work-materialization-probe");
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

pub fn send_workspace_update_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentWorkspaceUpdateRequest,
) -> Result<crate::AgentWorkspaceUpdateReceipt, String> {
    let url = target.workspace_update_url().map_err(|_| {
        AgentBridgeFailure::new(
            AgentBridgeFailureReason::TransportFailure,
            "Host workspace bridge target is invalid",
        )
        .to_string()
    })?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::TransportFailure,
                "failed to build the Host workspace bridge client",
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
                "Host workspace bridge is unavailable; the update was not retried locally and its outcome may be unknown",
            )
            .to_string()
        })?;
    let status = response.status();
    if !status.is_success() {
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
            "Host workspace bridge rejected the update; no local fallback was attempted",
        )
        .to_string());
    }
    let receipt = response
        .json::<crate::AgentWorkspaceUpdateReceipt>()
        .map_err(|_| {
            AgentBridgeFailure::new(
                AgentBridgeFailureReason::ReceiptMismatch,
                "Host workspace bridge returned an invalid success response; no local fallback was attempted",
            )
            .to_string()
        })?;
    if receipt.schema_version != crate::AGENT_WORKSPACE_UPDATE_SCHEMA_VERSION {
        return Err(AgentBridgeFailure::new(
            AgentBridgeFailureReason::ReceiptMismatch,
            "Host workspace bridge returned an unsupported response schema; no local fallback was attempted",
        )
        .to_string());
    }
    Ok(receipt)
}

pub(crate) fn send_work_terminalization_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentWorkTerminalizationRequest,
) -> Result<crate::AgentWorkTerminalizationReceipt, AgentBridgeRequestError> {
    let url = target
        .work_terminalization_url()
        .map_err(AgentBridgeRequestError::NotSent)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            AgentBridgeRequestError::NotSent(
                "failed to build the Host Work terminalization bridge client".to_string(),
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(&target.token)
        .json(request)
        .send()
        .map_err(|_| {
            AgentBridgeRequestError::Unknown(
                "Host Work terminalization bridge is unavailable; the close was not retried locally and its outcome may be unknown"
                    .to_string(),
            )
        })?;
    let status = response.status();
    let body = response.bytes().map_err(|_| {
        AgentBridgeRequestError::Unknown(
            "Host Work terminalization bridge response could not be read; its mutation outcome is unknown"
                .to_string(),
        )
    })?;
    if !status.is_success() {
        if let Ok(error) = serde_json::from_slice::<crate::AgentWorkspaceUpdateError>(&body) {
            return Err(AgentBridgeRequestError::Rejected(error));
        }
        if legacy_terminalization_schema_rejection(status, &body) {
            return Err(AgentBridgeRequestError::RejectedBeforeHandler(
                "legacy Host rejected Work terminalization before handler dispatch; no Work mutation was attempted"
                    .to_string(),
            ));
        }
        return Err(AgentBridgeRequestError::Unknown(format!(
                "Host Work terminalization bridge rejected the close with HTTP {status}; its mutation outcome is unknown"
            )));
    }
    let receipt =
        serde_json::from_slice::<crate::AgentWorkTerminalizationReceipt>(&body).map_err(|_| {
            AgentBridgeRequestError::Unknown(
                "Host Work terminalization bridge returned an invalid success response; no local fallback was attempted"
                    .to_string(),
            )
        })?;
    if receipt.schema_version != crate::AGENT_WORK_TERMINALIZATION_SCHEMA_VERSION
        || receipt.owner_number != request.owner_number
    {
        return Err(AgentBridgeRequestError::Unknown(
            "Host Work terminalization bridge returned an unsupported response schema; no local fallback was attempted"
                .to_string(),
        ));
    }
    Ok(receipt)
}

fn legacy_terminalization_schema_rejection(status: reqwest::StatusCode, body: &[u8]) -> bool {
    if status != reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        return false;
    }
    let Ok(body) = std::str::from_utf8(body) else {
        return false;
    };
    body.starts_with("Failed to deserialize the JSON body into the target type:")
        && body.contains("owner_number: unknown field `owner_number`")
        && body.contains(
            "expected one of `schema_version`, `claimed_session_id`, `observation`, `terminal_kind`",
        )
}

pub(crate) fn send_work_materialization_probe_via_agent_bridge(
    target: &HookForwardTarget,
    request: &crate::AgentWorkMaterializationProbeRequest,
) -> Result<crate::AgentWorkMaterializationProbeReceipt, AgentBridgeRequestError> {
    let url = target
        .work_materialization_probe_url()
        .map_err(AgentBridgeRequestError::NotSent)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| {
            AgentBridgeRequestError::NotSent(
                "failed to build the Host Work materialization probe client".to_string(),
            )
        })?;
    let response = client
        .post(url)
        .bearer_auth(&target.token)
        .json(request)
        .send()
        .map_err(|_| {
            AgentBridgeRequestError::Unknown(
                "Host Work materialization probe is unavailable; no build lifecycle state was created"
                    .to_string(),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        return match response.json::<crate::AgentWorkspaceUpdateError>() {
            Ok(error) => Err(AgentBridgeRequestError::Rejected(error)),
            Err(_) => Err(AgentBridgeRequestError::Unknown(format!(
                "Host Work materialization probe failed with HTTP {status}; no build lifecycle state was created"
            ))),
        };
    }
    let receipt = response
        .json::<crate::AgentWorkMaterializationProbeReceipt>()
        .map_err(|_| {
            AgentBridgeRequestError::Unknown(
                "Host Work materialization probe returned an invalid success response; no build lifecycle state was created"
                    .to_string(),
            )
        })?;
    if receipt.schema_version != crate::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION
        || receipt.owner_number != request.owner_number
        || receipt.work_id.trim().is_empty()
    {
        return Err(AgentBridgeRequestError::Unknown(
            "Host Work materialization probe returned an unsupported or incomplete receipt; no build lifecycle state was created"
                .to_string(),
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
    target.validate()?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(HOOK_LIVE_TIMEOUT_MS))
        .build()
        .map_err(|err| format!("build hook live client failed: {err}"))?;
    let response = client
        .post(&target.url)
        .bearer_auth(&target.token)
        .json(event)
        .send()
        .map_err(|err| format!("send hook live event failed: {err}"))?;

    if !response.status().is_success() {
        return Err(format!("hook live endpoint returned {}", response.status()));
    }

    Ok(())
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
    use std::{sync::mpsc, time::Duration};

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

    struct BindingProbeServer {
        runtime: Runtime,
        shutdown_tx: Option<oneshot::Sender<()>>,
        rx: mpsc::Receiver<(HeaderMap, serde_json::Value)>,
        forward_url: String,
    }

    #[derive(Clone)]
    struct BindingProbeState {
        tx: mpsc::Sender<(HeaderMap, serde_json::Value)>,
        status: StatusCode,
        body: String,
    }

    impl BindingProbeServer {
        fn start(status: StatusCode, body: serde_json::Value) -> Self {
            let runtime = Runtime::new().expect("binding probe runtime");
            let listener = runtime
                .block_on(TcpListener::bind(("127.0.0.1", 0)))
                .expect("binding probe listener");
            let address = listener.local_addr().expect("binding probe address");
            let (tx, rx) = mpsc::channel();
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
                .with_state(BindingProbeState {
                    tx,
                    status,
                    body: body.to_string(),
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
                forward_url: format!("http://127.0.0.1:{}/internal/hook-live", address.port()),
            }
        }

        fn receive(&self) -> (HeaderMap, serde_json::Value) {
            self.rx
                .recv_timeout(Duration::from_secs(2))
                .expect("binding probe request")
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
                    .work_materialization_probe_url()
                    .unwrap_or_else(|error| panic!("{host}: {error}"))
                    .as_str(),
                format!("http://{host}:45123/internal/work-materialization-probe")
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
            let error = HookForwardTarget {
                url: url.to_string(),
                token: "secret".to_string(),
            }
            .work_materialization_probe_url()
            .expect_err("non-canonical materialization probe target must fail closed");
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

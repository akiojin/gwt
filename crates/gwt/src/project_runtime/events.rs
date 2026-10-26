//! Private project-owner wire data. Never contains capability or GUI authority.
use serde::{Deserialize, Serialize};

/// Immutable readback of an exact Genesis compensation. This receipt only
/// invalidates caches; it never grants cleanup or input authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerCompensatedLaunchFailure {
    pub error: String,
    pub removed_genesis_session: gwt_agent::SessionExecutionIdentity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OwnerRequest {
    pub project_key: String,
    pub command: OwnerCommand,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OwnerCommand {
    NotifyBoardDecision {
        entry_id: String,
        session_id: String,
    },
    CloseWork {
        work_id: String,
        close_kind: String,
    },
    UpdateLayout {
        layout: crate::persistence::PersistedWindowRegistry,
    },
    CloseStoppedSession {
        session_id: String,
        expected: Box<gwt_agent::Session>,
    },
    ProviderUsageSnapshot {
        accounts: Vec<gwt_core::usage::ProviderUsage>,
    },
    DeliverAutonomousAnswer {
        id: String,
        incarnation: u64,
        issue_number: u64,
        delivery_id: Option<String>,
    },
    AcknowledgeMonitorLaunch {
        id: String,
        incarnation: u64,
        issue_number: u64,
        delivery_id: Option<String>,
    },
    AdoptMonitorLaunch {
        id: String,
        incarnation: u64,
        issue_number: u64,
        delivery_id: Option<String>,
    },
    Health,
    ListPanes,
    ContinueWork {
        client_id: String,
        operation_id: String,
        work_id: String,
    },
    PendingCancel {
        id: String,
        operation_id: String,
    },
    PromoteStandalone,
    LaunchShell {
        id: String,
        command: String,
        args: Vec<String>,
        cols: u16,
        rows: u16,
    },
    LaunchConfiguredShell {
        id: String,
        operation_id: String,
        config: Box<crate::launch_wizard::ShellLaunchConfig>,
        cols: u16,
        rows: u16,
    },
    RestoreSavedSession {
        id: String,
        operation_id: String,
        source_session_id: String,
        expected_source: Box<gwt_agent::Session>,
        origin: gwt_agent::SessionLaunchOrigin,
        cols: u16,
        rows: u16,
    },
    LaunchAgent {
        id: String,
        operation_id: String,
        tab_id: String,
        config: Box<gwt_agent::LaunchConfig>,
        #[serde(default)]
        metadata: Box<super::launch_payload::OwnerLaunchMetadata>,
        cols: u16,
        rows: u16,
    },
    Input {
        id: String,
        incarnation: u64,
        data: Vec<u8>,
    },
    SendInput {
        id: String,
        incarnation: u64,
        text: String,
    },
    PmWakeInput {
        id: String,
        incarnation: u64,
        host_instance_id: String,
        delivery: super::pm_delivery::PmWakeInput,
    },
    CancelPmWake {
        id: String,
        incarnation: u64,
        host_instance_id: String,
        operation_id: String,
    },
    Resize {
        id: String,
        incarnation: u64,
        cols: u16,
        rows: u16,
    },
    Snapshot {
        id: String,
        incarnation: u64,
    },
    Close {
        id: String,
        incarnation: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OwnerEvent {
    LaunchProgress {
        operation_id: String,
        id: String,
        incarnation: u64,
        message: String,
    },
    PreparationOutput {
        operation_id: String,
        id: String,
        incarnation: u64,
        data: Vec<u8>,
    },
    AgentStarted {
        id: String,
        incarnation: u64,
        session_id: String,
        awaiting_readiness: bool,
    },
    LaunchFailed {
        id: String,
        incarnation: u64,
        detail: String,
    },
    FocusExisting {
        id: String,
        incarnation: u64,
    },
    ContinueWorkOutcome {
        client_id: String,
        operation_id: String,
        work_id: String,
        outcome: crate::ContinueWorkOutcomeKind,
        message: Option<String>,
        error_code: Option<String>,
        retryable: bool,
    },
    Hook {
        event: crate::RuntimeHookEvent,
    },
    Notice {
        level: String,
        message: String,
        issue_number: Option<u64>,
    },
    Output {
        id: String,
        incarnation: u64,
        data: Vec<u8>,
        seq: u64,
        screen: String,
        screen_ansi: Vec<u8>,
        screen_seq: u64,
    },
    Status {
        id: String,
        incarnation: u64,
        status: crate::WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
        raw_status: crate::WindowProcessStatus,
        exit_code: Option<u32>,
    },
    WindowRemoved {
        id: String,
        incarnation: u64,
    },
    Closed {
        id: String,
        incarnation: u64,
        error: Option<String>,
    },
    PmLifecycle {
        id: String,
        incarnation: u64,
        outcome: super::pm_delivery::PmLifecycleOutcome,
    },
    Invalidated,
}

/// A failed manual launch retained as an exact restartable diagnostic row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerLaunchFailure {
    pub id: String,
    pub incarnation: u64,
    pub host_pid: u32,
    pub host_instance_id: String,
    pub session_id: String,
    pub session_fingerprint: String,
    pub detail: String,
    pub error_code: String,
    pub retryable: bool,
}

/// Exact read-model anchor; it grants no mutation or process authority.
pub fn launch_failure_session_fingerprint(session: &gwt_agent::Session) -> Result<String, String> {
    use sha2::Digest as _;
    let bytes = serde_json::to_vec(session).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", sha2::Sha256::digest(bytes)))
}

pub type OwnerReply = Result<serde_json::Value, String>;

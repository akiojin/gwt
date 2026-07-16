//! Host-only bridge between the Codex remote TUI and a per-launch app-server.
//!
//! One process-wide, loopback-only WebSocket listener routes bearer-authenticated
//! connections to isolated app-server processes. JSON is inspected for recovery
//! barriers and then forwarded byte-for-byte; it is never re-serialized.

use std::{
    collections::HashMap,
    fmt,
    net::TcpListener as StdTcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    thread,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use gwt_core::bounded_file::BoundedRegularFile;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

const MIN_BEARER_TOKEN_BYTES: usize = 32;
const BRIDGE_ROUTE_PATH: &str = "/codex";
const MAX_BRIDGE_ROUTES: usize = 128;
const MAX_JSON_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CAPTURE_TEXT_BYTES: usize = 64 * 1024;
const MAX_ERROR_MESSAGE_BYTES: usize = 1024;
const MAX_RECOVERY_ATTACHMENT_BYTES: usize = 24 * 1024 * 1024;
pub(super) const MAX_RECOVERY_ATTACHMENT_COUNT: usize = 128;
pub(super) const MAX_RECOVERY_ATTACHMENT_AGGREGATE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_RECOVERY_ATTACHMENT_CONTROL_BYTES: usize = 36 * 1024 * 1024;
pub const CODEX_REMOTE_AUTH_TOKEN_ENV: &str = "GWT_CODEX_REMOTE_AUTH_TOKEN";

pub fn parse_codex_cli_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|token| {
            token
                .trim()
                .trim_start_matches('v')
                .trim_matches(|character| matches!(character, ',' | ';' | '(' | ')'))
        })
        .find_map(|token| {
            semver::Version::parse(token)
                .ok()
                .map(|version| version.to_string())
        })
}

#[path = "codex_sidecar.rs"]
mod sidecar;
pub use sidecar::{
    prepare_container_recovery_attachments, run_container_codex_sidecar,
    start_container_codex_bridge, CodexContainerBridgeConfig, CodexContainerBridgeLease,
    CodexLaunchBridgeLease, ContainerRecoveryAttachmentBundle,
};

/// Hash-only routing identity for a bridge bearer token.
///
/// The plaintext capability is never retained, formatted, or exposed as a URL
/// component. The type can be used directly as a `HashMap` key by the future
/// network transport.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CodexRouteIdentity([u8; 32]);

impl CodexRouteIdentity {
    pub fn from_bearer_token(token: &str) -> Result<Self, RouteIdentityError> {
        if token.len() < MIN_BEARER_TOKEN_BYTES {
            return Err(RouteIdentityError::TokenTooShort);
        }
        Ok(Self(Self::digest(token)))
    }

    pub fn matches_bearer_token(&self, candidate: &str) -> bool {
        let candidate = Self::digest(candidate);
        self.0
            .iter()
            .zip(candidate.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }

    fn digest(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }
}

impl fmt::Debug for CodexRouteIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CodexRouteIdentity([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RouteIdentityError {
    #[error("Codex bridge bearer token is too short")]
    TokenTooShort,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RpcId {
    String(String),
    Number(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    WaitingInitialize,
    WaitingInitializeResponse,
    WaitingInitialized,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadOperation {
    Start,
    Resume,
    Fork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootThreadBinding {
    pub thread_id: String,
    pub session_id: String,
    pub cli_version: String,
    pub forked_from_id: Option<String>,
    pub operation: ThreadOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildThreadBinding {
    pub thread_id: String,
    pub session_id: String,
    pub cli_version: String,
    pub parent_thread_id: String,
    pub operation: ThreadOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadBinding {
    Root(RootThreadBinding),
    Child(ChildThreadBinding),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInputKind {
    Start,
    Steer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentCandidateKind {
    ImageUrl,
    LocalImage,
}

/// Ephemeral provider attachment reference awaiting content-addressed import.
///
/// `source` is never written into a semantic checkpoint by the bridge. The
/// RecoveryStore attachment importer consumes it while the source is still
/// reachable and persists only a project-owned content-addressed reference.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentCandidate {
    pub kind: AttachmentCandidateKind,
    pub source: String,
    pub detail: Option<String>,
}

impl fmt::Debug for AttachmentCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AttachmentCandidate")
            .field("kind", &self.kind)
            .field("source", &"[REDACTED]")
            .field("detail", &self.detail)
            .finish()
    }
}

/// Container-local bytes transferred over the private sidecar control pipe.
/// Debug formatting deliberately omits the content.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferredAttachment {
    pub file_name: String,
    pub base64_data: String,
}

impl fmt::Debug for TransferredAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferredAttachment")
            .field("file_name", &self.file_name)
            .field("base64_data", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInputCapture {
    pub kind: UserInputKind,
    pub thread_id: String,
    pub client_user_message_id: Option<String>,
    pub text_segments: Vec<String>,
    pub attachment_candidates: Vec<AttachmentCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleDiscussionRole {
    Assistant,
    User,
}

impl VisibleDiscussionRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibleDiscussionKind {
    AssistantMessage,
    Plan,
    StructuredQuestion,
    StructuredAnswer,
}

impl VisibleDiscussionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::AssistantMessage => "assistant_message",
            Self::Plan => "plan",
            Self::StructuredQuestion => "structured_question",
            Self::StructuredAnswer => "structured_answer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleDiscussionItemCapture {
    pub role: VisibleDiscussionRole,
    pub kind: VisibleDiscussionKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleDiscussionCapture {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub items: Vec<VisibleDiscussionItemCapture>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexBridgeFailureKind {
    DefinitiveThreadNotFound,
    Authentication,
    InvalidRequest,
    Transport,
    Durability,
    Protocol,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexBridgeFailure {
    pub operation: Option<ThreadOperation>,
    pub kind: CodexBridgeFailureKind,
    /// Bounded, secret-free diagnostic suitable for lifecycle state and UI.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolEvent {
    Passthrough,
    InitializeRequested,
    InitializeAcknowledged,
    Ready,
    ThreadRequested {
        operation: ThreadOperation,
        requested_thread_id: Option<String>,
    },
    ThreadBinding(ThreadBinding),
    ThreadOperationFailed {
        operation: ThreadOperation,
        failure: CodexBridgeFailure,
    },
    UserInput(UserInputCapture),
    VisibleDiscussion(VisibleDiscussionCapture),
}

/// Inspection result that intentionally borrows the original wire text.
/// Serializing the parsed JSON again would reorder fields or normalize
/// whitespace and would no longer be a transparent proxy.
pub struct InspectedMessage<'a> {
    wire_text: &'a str,
    event: ProtocolEvent,
}

impl<'a> InspectedMessage<'a> {
    pub fn wire_text(&self) -> &'a str {
        self.wire_text
    }

    pub fn event(&self) -> &ProtocolEvent {
        &self.event
    }

    pub fn into_event(self) -> ProtocolEvent {
        self.event
    }
}

impl fmt::Debug for InspectedMessage<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InspectedMessage")
            .field("wire_text", &"[REDACTED]")
            .field("event", &self.event)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    #[error("invalid Codex JSON-RPC: {0}")]
    InvalidJson(String),
    #[error("Codex JSON-RPC message must be an object")]
    InvalidEnvelope,
    #[error("missing required Codex JSON-RPC field: {0}")]
    MissingField(&'static str),
    #[error("invalid Codex JSON-RPC field: {0}")]
    InvalidField(&'static str),
    #[error("Codex JSON-RPC request id is already pending")]
    DuplicateRequestId,
    #[error("Codex protocol message arrived before initialization completed")]
    NotReady,
    #[error("unexpected Codex initialize/initialized order")]
    InvalidHandshakeOrder,
    #[error("Codex resume thread mismatch: expected {expected}, got {actual}")]
    ResumeThreadMismatch { expected: String, actual: String },
    #[error("Codex exact resume expected thread/resume, got {actual:?}")]
    ExpectedResumeOperation { actual: ThreadOperation },
    #[error("Codex resume returned child thread {thread_id} of {parent_thread_id}")]
    ResumeReturnedChild {
        thread_id: String,
        parent_thread_id: String,
    },
    #[error("Codex user input targeted {actual}, but the bound root is {expected}")]
    UserInputThreadMismatch { expected: String, actual: String },
    #[error("Codex user input exceeds the recovery attachment count limit ({limit})")]
    TooManyAttachments { limit: usize },
    #[error("Codex remote TUI/app-server version mismatch: client {client}, server {server}")]
    CliVersionMismatch { client: String, server: String },
}

#[derive(Debug, Clone)]
struct PendingThreadRequest {
    operation: ThreadOperation,
    requested_thread_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingStructuredQuestion {
    question_id: String,
    secret: bool,
}

#[derive(Debug, Clone)]
struct PendingStructuredInput {
    thread_id: String,
    turn_id: String,
    item_id: String,
    questions: Vec<PendingStructuredQuestion>,
}

#[derive(Debug, Clone)]
struct ParsedThread {
    thread_id: String,
    session_id: String,
    cli_version: String,
    parent_thread_id: Option<String>,
    forked_from_id: Option<String>,
}

/// Stateful protocol observer for one remote TUI ↔ app-server connection.
pub struct CodexProtocolTracker {
    handshake_state: HandshakeState,
    initialize_request_id: Option<RpcId>,
    pending_thread_requests: HashMap<RpcId, PendingThreadRequest>,
    pending_structured_inputs: HashMap<RpcId, PendingStructuredInput>,
    expected_resume_id: Option<String>,
    root_thread_id: Option<String>,
    client_cli_version: Option<String>,
}

impl CodexProtocolTracker {
    pub fn new(expected_resume_id: Option<String>) -> Self {
        Self {
            handshake_state: HandshakeState::WaitingInitialize,
            initialize_request_id: None,
            pending_thread_requests: HashMap::new(),
            pending_structured_inputs: HashMap::new(),
            expected_resume_id,
            root_thread_id: None,
            client_cli_version: None,
        }
    }

    pub fn handshake_state(&self) -> HandshakeState {
        self.handshake_state
    }

    pub fn is_ready(&self) -> bool {
        self.handshake_state == HandshakeState::Ready
    }

    pub fn root_thread_id(&self) -> Option<&str> {
        self.root_thread_id.as_deref()
    }

    pub fn inspect_client<'a>(
        &mut self,
        wire_text: &'a str,
    ) -> Result<InspectedMessage<'a>, ProtocolError> {
        let value = parse_message(wire_text)?;
        let object = message_object(&value)?;
        let method = object.get("method").and_then(Value::as_str);

        let event = match method {
            Some("initialize") => self.observe_initialize_request(object)?,
            Some("initialized") => self.observe_initialized()?,
            Some("thread/start") => self.observe_thread_request(object, ThreadOperation::Start)?,
            Some("thread/resume") => {
                self.observe_thread_request(object, ThreadOperation::Resume)?
            }
            Some("thread/fork") => self.observe_thread_request(object, ThreadOperation::Fork)?,
            Some("turn/start") => self.observe_user_input(object, UserInputKind::Start)?,
            Some("turn/steer") => self.observe_user_input(object, UserInputKind::Steer)?,
            Some(_) => ProtocolEvent::Passthrough,
            None => {
                let Some(id) = optional_rpc_id(object)? else {
                    return Ok(InspectedMessage {
                        wire_text,
                        event: ProtocolEvent::Passthrough,
                    });
                };
                if let Some(pending) = self.pending_structured_inputs.remove(&id) {
                    self.observe_structured_answer(object, pending)?
                } else {
                    ProtocolEvent::Passthrough
                }
            }
        };

        Ok(InspectedMessage { wire_text, event })
    }

    pub fn inspect_server<'a>(
        &mut self,
        wire_text: &'a str,
    ) -> Result<InspectedMessage<'a>, ProtocolError> {
        let value = parse_message(wire_text)?;
        let object = message_object(&value)?;
        // JSON-RPC is bidirectional. A server request may legally reuse a
        // numeric/string id that is pending in the client→server direction;
        // only response envelopes (no `method`) may consume client pending
        // state.
        if let Some(method) = object.get("method").and_then(Value::as_str) {
            let event = match method {
                "item/completed" => self.observe_completed_item(object)?,
                "item/tool/requestUserInput" => self.observe_structured_question(object)?,
                _ => ProtocolEvent::Passthrough,
            };
            return Ok(InspectedMessage { wire_text, event });
        }
        let Some(id) = optional_rpc_id(object)? else {
            return Ok(InspectedMessage {
                wire_text,
                event: ProtocolEvent::Passthrough,
            });
        };

        let event = if self.initialize_request_id.as_ref() == Some(&id) {
            self.observe_initialize_response(object)?
        } else if let Some(pending) = self.pending_thread_requests.remove(&id) {
            self.observe_thread_response(object, pending)?
        } else {
            ProtocolEvent::Passthrough
        };

        Ok(InspectedMessage { wire_text, event })
    }

    fn observe_initialize_request(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<ProtocolEvent, ProtocolError> {
        if self.handshake_state != HandshakeState::WaitingInitialize {
            return Err(ProtocolError::InvalidHandshakeOrder);
        }
        let id = required_rpc_id(object)?;
        let params = required_object(object, "params")?;
        let client_info = required_object(params, "clientInfo")?;
        self.client_cli_version = Some(required_string(client_info, "version")?.to_string());
        self.initialize_request_id = Some(id);
        self.handshake_state = HandshakeState::WaitingInitializeResponse;
        Ok(ProtocolEvent::InitializeRequested)
    }

    fn observe_initialize_response(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<ProtocolEvent, ProtocolError> {
        if self.handshake_state != HandshakeState::WaitingInitializeResponse {
            return Err(ProtocolError::InvalidHandshakeOrder);
        }
        if object.contains_key("error") {
            self.initialize_request_id = None;
            self.handshake_state = HandshakeState::WaitingInitialize;
            return Ok(ProtocolEvent::Passthrough);
        }
        if !object.get("result").is_some_and(Value::is_object) {
            return Err(ProtocolError::MissingField("result"));
        }
        self.initialize_request_id = None;
        self.handshake_state = HandshakeState::WaitingInitialized;
        Ok(ProtocolEvent::InitializeAcknowledged)
    }

    fn observe_initialized(&mut self) -> Result<ProtocolEvent, ProtocolError> {
        if self.handshake_state != HandshakeState::WaitingInitialized {
            return Err(ProtocolError::InvalidHandshakeOrder);
        }
        self.handshake_state = HandshakeState::Ready;
        Ok(ProtocolEvent::Ready)
    }

    fn observe_thread_request(
        &mut self,
        object: &Map<String, Value>,
        operation: ThreadOperation,
    ) -> Result<ProtocolEvent, ProtocolError> {
        self.require_ready()?;
        if self.expected_resume_id.is_some()
            && self.root_thread_id.is_none()
            && operation != ThreadOperation::Resume
        {
            return Err(ProtocolError::ExpectedResumeOperation { actual: operation });
        }
        let id = required_rpc_id(object)?;
        let params = required_object(object, "params")?;
        let requested_thread_id = match operation {
            ThreadOperation::Start => None,
            ThreadOperation::Resume | ThreadOperation::Fork => {
                Some(required_string(params, "threadId")?.to_string())
            }
        };

        if operation == ThreadOperation::Resume {
            if let (Some(expected), Some(actual)) = (
                self.expected_resume_id.as_deref(),
                requested_thread_id.as_deref(),
            ) {
                if expected != actual {
                    return Err(ProtocolError::ResumeThreadMismatch {
                        expected: expected.to_string(),
                        actual: actual.to_string(),
                    });
                }
            }
        }

        if self.pending_thread_requests.contains_key(&id) {
            return Err(ProtocolError::DuplicateRequestId);
        }
        self.pending_thread_requests.insert(
            id,
            PendingThreadRequest {
                operation,
                requested_thread_id: requested_thread_id.clone(),
            },
        );

        Ok(ProtocolEvent::ThreadRequested {
            operation,
            requested_thread_id,
        })
    }

    fn observe_thread_response(
        &mut self,
        object: &Map<String, Value>,
        pending: PendingThreadRequest,
    ) -> Result<ProtocolEvent, ProtocolError> {
        if let Some(error) = object.get("error") {
            let error = error
                .as_object()
                .ok_or(ProtocolError::InvalidField("error"))?;
            let failure = classify_thread_failure(&pending, error)?;
            return Ok(ProtocolEvent::ThreadOperationFailed {
                operation: pending.operation,
                failure,
            });
        }
        let result = required_object(object, "result")?;
        let thread = required_object(result, "thread")?;
        let parsed = parse_thread(thread)?;
        if let Some(client_version) = self.client_cli_version.as_deref() {
            if client_version != parsed.cli_version {
                return Err(ProtocolError::CliVersionMismatch {
                    client: client_version.to_string(),
                    server: parsed.cli_version,
                });
            }
        }

        if pending.operation == ThreadOperation::Resume {
            if let Some(expected) = self.expected_resume_id.as_deref() {
                if expected != parsed.thread_id {
                    return Err(ProtocolError::ResumeThreadMismatch {
                        expected: expected.to_string(),
                        actual: parsed.thread_id,
                    });
                }
            }
            if let Some(parent_thread_id) = parsed.parent_thread_id.as_ref() {
                return Err(ProtocolError::ResumeReturnedChild {
                    thread_id: parsed.thread_id,
                    parent_thread_id: parent_thread_id.clone(),
                });
            }
        }

        let binding = if let Some(parent_thread_id) = parsed.parent_thread_id {
            ThreadBinding::Child(ChildThreadBinding {
                thread_id: parsed.thread_id,
                session_id: parsed.session_id,
                cli_version: parsed.cli_version,
                parent_thread_id,
                operation: pending.operation,
            })
        } else {
            self.root_thread_id = Some(parsed.thread_id.clone());
            ThreadBinding::Root(RootThreadBinding {
                thread_id: parsed.thread_id,
                session_id: parsed.session_id,
                cli_version: parsed.cli_version,
                forked_from_id: parsed.forked_from_id,
                operation: pending.operation,
            })
        };
        Ok(ProtocolEvent::ThreadBinding(binding))
    }

    fn observe_user_input(
        &self,
        object: &Map<String, Value>,
        kind: UserInputKind,
    ) -> Result<ProtocolEvent, ProtocolError> {
        self.require_ready()?;
        let params = required_object(object, "params")?;
        let thread_id = required_string(params, "threadId")?.to_string();
        if let Some(expected) = self.root_thread_id.as_deref() {
            if expected != thread_id {
                return Err(ProtocolError::UserInputThreadMismatch {
                    expected: expected.to_string(),
                    actual: thread_id,
                });
            }
        }
        let client_user_message_id = optional_string(params, "clientUserMessageId")?;
        let input = params
            .get("input")
            .ok_or(ProtocolError::MissingField("input"))?
            .as_array()
            .ok_or(ProtocolError::InvalidField("input"))?;
        let mut text_segments = Vec::new();
        let mut attachment_candidates = Vec::new();
        for item in input {
            let item = item
                .as_object()
                .ok_or(ProtocolError::InvalidField("input[]"))?;
            match item.get("type").and_then(Value::as_str) {
                Some("text") => text_segments.push(required_string(item, "text")?.to_string()),
                Some("image") => {
                    if attachment_candidates.len() >= MAX_RECOVERY_ATTACHMENT_COUNT {
                        return Err(ProtocolError::TooManyAttachments {
                            limit: MAX_RECOVERY_ATTACHMENT_COUNT,
                        });
                    }
                    attachment_candidates.push(AttachmentCandidate {
                        kind: AttachmentCandidateKind::ImageUrl,
                        source: required_string(item, "url")?.to_string(),
                        detail: optional_string(item, "detail")?,
                    });
                }
                Some("localImage") => {
                    if attachment_candidates.len() >= MAX_RECOVERY_ATTACHMENT_COUNT {
                        return Err(ProtocolError::TooManyAttachments {
                            limit: MAX_RECOVERY_ATTACHMENT_COUNT,
                        });
                    }
                    attachment_candidates.push(AttachmentCandidate {
                        kind: AttachmentCandidateKind::LocalImage,
                        source: required_string(item, "path")?.to_string(),
                        detail: optional_string(item, "detail")?,
                    });
                }
                _ => {}
            }
        }
        Ok(ProtocolEvent::UserInput(UserInputCapture {
            kind,
            thread_id,
            client_user_message_id,
            text_segments,
            attachment_candidates,
        }))
    }

    fn observe_completed_item(
        &self,
        object: &Map<String, Value>,
    ) -> Result<ProtocolEvent, ProtocolError> {
        self.require_ready()?;
        let params = required_object(object, "params")?;
        let thread_id = required_string(params, "threadId")?;
        if self.root_thread_id.as_deref() != Some(thread_id) {
            return Ok(ProtocolEvent::Passthrough);
        }
        let turn_id = required_string(params, "turnId")?;
        let item = required_object(params, "item")?;
        let item_id = required_string(item, "id")?;
        let kind = match required_string(item, "type")? {
            "agentMessage" => VisibleDiscussionKind::AssistantMessage,
            "plan" => VisibleDiscussionKind::Plan,
            _ => return Ok(ProtocolEvent::Passthrough),
        };
        let text = bounded_visible_text(required_string(item, "text")?);
        if text.trim().is_empty() {
            return Ok(ProtocolEvent::Passthrough);
        }
        Ok(ProtocolEvent::VisibleDiscussion(VisibleDiscussionCapture {
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            item_id: item_id.to_string(),
            items: vec![VisibleDiscussionItemCapture {
                role: VisibleDiscussionRole::Assistant,
                kind,
                text,
            }],
        }))
    }

    fn observe_structured_question(
        &mut self,
        object: &Map<String, Value>,
    ) -> Result<ProtocolEvent, ProtocolError> {
        self.require_ready()?;
        let params = required_object(object, "params")?;
        let thread_id = required_string(params, "threadId")?;
        if self.root_thread_id.as_deref() != Some(thread_id) {
            return Ok(ProtocolEvent::Passthrough);
        }
        let request_id = required_rpc_id(object)?;
        if self.pending_structured_inputs.contains_key(&request_id) {
            return Err(ProtocolError::DuplicateRequestId);
        }
        let turn_id = required_string(params, "turnId")?.to_string();
        let item_id = required_string(params, "itemId")?.to_string();
        let questions = params
            .get("questions")
            .ok_or(ProtocolError::MissingField("questions"))?
            .as_array()
            .ok_or(ProtocolError::InvalidField("questions"))?;
        let mut pending_questions = Vec::with_capacity(questions.len());
        let mut visible_items = Vec::with_capacity(questions.len());
        for question in questions {
            let question = question
                .as_object()
                .ok_or(ProtocolError::InvalidField("questions[]"))?;
            let question_id = required_string(question, "id")?.to_string();
            let secret = optional_bool(question, "isSecret")?.unwrap_or(false);
            pending_questions.push(PendingStructuredQuestion {
                question_id,
                secret,
            });
            let header = required_string(question, "header")?;
            let prompt = required_string(question, "question")?;
            let text = if header.trim().is_empty() {
                bounded_visible_text(prompt)
            } else {
                bounded_visible_text(&format!("{}: {}", header.trim(), prompt.trim()))
            };
            if !text.trim().is_empty() {
                visible_items.push(VisibleDiscussionItemCapture {
                    role: VisibleDiscussionRole::Assistant,
                    kind: VisibleDiscussionKind::StructuredQuestion,
                    text,
                });
            }
        }
        self.pending_structured_inputs.insert(
            request_id,
            PendingStructuredInput {
                thread_id: thread_id.to_string(),
                turn_id: turn_id.clone(),
                item_id: item_id.clone(),
                questions: pending_questions,
            },
        );
        if visible_items.is_empty() {
            return Ok(ProtocolEvent::Passthrough);
        }
        Ok(ProtocolEvent::VisibleDiscussion(VisibleDiscussionCapture {
            thread_id: thread_id.to_string(),
            turn_id,
            item_id,
            items: visible_items,
        }))
    }

    fn observe_structured_answer(
        &self,
        object: &Map<String, Value>,
        pending: PendingStructuredInput,
    ) -> Result<ProtocolEvent, ProtocolError> {
        if object.contains_key("error") {
            return Ok(ProtocolEvent::Passthrough);
        }
        let result = required_object(object, "result")?;
        let answers = required_object(result, "answers")?;
        let mut visible_items = Vec::new();
        for question in &pending.questions {
            if question.secret {
                continue;
            }
            let Some(answer) = answers.get(&question.question_id) else {
                continue;
            };
            let answer = answer
                .as_object()
                .ok_or(ProtocolError::InvalidField("answers.*"))?;
            let values = answer
                .get("answers")
                .ok_or(ProtocolError::MissingField("answers.*.answers"))?
                .as_array()
                .ok_or(ProtocolError::InvalidField("answers.*.answers"))?;
            let values = values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or(ProtocolError::InvalidField("answers.*.answers[]"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let text = bounded_visible_text(&values.join("\n"));
            if !text.trim().is_empty() {
                visible_items.push(VisibleDiscussionItemCapture {
                    role: VisibleDiscussionRole::User,
                    kind: VisibleDiscussionKind::StructuredAnswer,
                    text,
                });
            }
        }
        if visible_items.is_empty() {
            return Ok(ProtocolEvent::Passthrough);
        }
        Ok(ProtocolEvent::VisibleDiscussion(VisibleDiscussionCapture {
            thread_id: pending.thread_id,
            turn_id: pending.turn_id,
            item_id: pending.item_id,
            items: visible_items,
        }))
    }

    fn require_ready(&self) -> Result<(), ProtocolError> {
        if self.is_ready() {
            Ok(())
        } else {
            Err(ProtocolError::NotReady)
        }
    }
}

/// Persistence seam used by the bridge's two forwarding barriers.
///
/// Implementations must return only after the relevant state is durable. A
/// failure closes the bridge connection and the original JSON is not sent
/// across the boundary.
pub trait CodexDurabilitySink: Send + Sync {
    fn persist_root_binding(
        &self,
        binding: &RootThreadBinding,
        wire_text: &str,
    ) -> Result<(), String>;

    fn persist_user_input(&self, input: &UserInputCapture, wire_text: &str) -> Result<(), String>;

    fn persist_visible_discussion(
        &self,
        capture: &VisibleDiscussionCapture,
        wire_text: &str,
    ) -> Result<(), String>;

    fn persist_transferred_user_input(
        &self,
        input: &UserInputCapture,
        attachments: &[TransferredAttachment],
        wire_text: &str,
    ) -> Result<(), String>;
}

/// Exact process specification for the app-server paired with one remote TUI.
///
/// This intentionally has no derived `Debug`: environment variables may hold
/// provider credentials or hook capabilities and must not be formatted.
#[derive(Clone, Serialize, Deserialize)]
pub struct CodexAppServerLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remove_env: Vec<String>,
    pub cwd: Option<PathBuf>,
}

pub struct CodexBridgeRouteConfig {
    pub app_server: CodexAppServerLaunch,
    pub expected_resume_id: Option<String>,
    pub durability: Arc<dyn CodexDurabilitySink>,
    pub on_ready: Arc<dyn Fn() + Send + Sync>,
    pub on_failure: Arc<dyn Fn(CodexBridgeFailure) + Send + Sync>,
}

#[derive(Debug, Error)]
pub enum CodexBridgeRegistrationError {
    #[error("failed to start the Codex loopback bridge: {0}")]
    Listener(String),
    #[error("the Codex loopback bridge has reached its route limit")]
    RouteCapacity,
}

struct CodexBridgeRoute {
    identity: CodexRouteIdentity,
    app_server: CodexAppServerLaunch,
    expected_resume_id: Option<String>,
    durability: Arc<dyn CodexDurabilitySink>,
    on_ready: Arc<dyn Fn() + Send + Sync>,
    on_failure: Arc<dyn Fn(CodexBridgeFailure) + Send + Sync>,
    claimed: AtomicBool,
    cancelled: AtomicBool,
    cancel_notify: tokio::sync::Notify,
    root_forwarded: AtomicBool,
    failure_reported: AtomicBool,
}

impl CodexBridgeRoute {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel_notify.notify_waiters();
    }

    async fn cancelled(&self) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let notified = self.cancel_notify.notified();
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }

    fn mark_root_forwarded(&self) {
        if !self.root_forwarded.swap(true, Ordering::AcqRel) {
            (self.on_ready)();
        }
    }

    fn report_failure(&self, failure: CodexBridgeFailure) {
        if !self.failure_reported.swap(true, Ordering::AcqRel) {
            (self.on_failure)(failure);
        }
    }
}

#[derive(Default)]
struct CodexRouteRegistry {
    routes: Mutex<HashMap<CodexRouteIdentity, Arc<CodexBridgeRoute>>>,
}

impl CodexRouteRegistry {
    fn insert(
        &self,
        identity: CodexRouteIdentity,
        route: Arc<CodexBridgeRoute>,
    ) -> Result<(), CodexBridgeRegistrationError> {
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if routes.len() >= MAX_BRIDGE_ROUTES {
            return Err(CodexBridgeRegistrationError::RouteCapacity);
        }
        routes.insert(identity, route);
        Ok(())
    }

    fn claim(&self, bearer_token: &str) -> Option<Arc<CodexBridgeRoute>> {
        let identity = CodexRouteIdentity::from_bearer_token(bearer_token).ok()?;
        let route = self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&identity)
            .cloned()?;
        if !route.identity.matches_bearer_token(bearer_token) {
            return None;
        }
        route
            .claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| route)
    }

    fn remove(&self, identity: &CodexRouteIdentity, expected: &Arc<CodexBridgeRoute>) {
        let mut routes = self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if routes
            .get(identity)
            .is_some_and(|registered| Arc::ptr_eq(registered, expected))
        {
            routes.remove(identity);
        }
    }
}

struct BridgeLeaseInner {
    endpoint: String,
    bearer_token: String,
    identity: CodexRouteIdentity,
    registry: Weak<CodexRouteRegistry>,
    route: Arc<CodexBridgeRoute>,
}

impl Drop for BridgeLeaseInner {
    fn drop(&mut self) {
        self.route.cancel();
        if let Some(registry) = self.registry.upgrade() {
            registry.remove(&self.identity, &self.route);
        }
    }
}

/// Capability lease retained for exactly as long as the associated PTY.
///
/// Clones share one secret-bearing inner value. Formatting is always redacted,
/// and dropping the last clone unregisters the route and stops its app-server.
#[derive(Clone)]
pub struct CodexBridgeRouteLease {
    inner: Arc<BridgeLeaseInner>,
}

impl CodexBridgeRouteLease {
    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }

    #[cfg(test)]
    fn auth_token(&self) -> &str {
        &self.inner.bearer_token
    }

    pub fn install_auth_token(&self, env: &mut HashMap<String, String>) {
        env.insert(
            CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
            self.inner.bearer_token.clone(),
        );
    }

    pub fn root_forwarded(&self) -> bool {
        self.inner.route.root_forwarded.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CodexBridgeRouteLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexBridgeRouteLease")
            .field("endpoint", &self.inner.endpoint)
            .field("bearer_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
struct BridgeHttpState {
    registry: Arc<CodexRouteRegistry>,
}

struct CodexBridgeServer {
    endpoint: String,
    registry: Arc<CodexRouteRegistry>,
}

static CODEX_BRIDGE_SERVER: OnceLock<Result<CodexBridgeServer, String>> = OnceLock::new();

fn bridge_server() -> Result<&'static CodexBridgeServer, CodexBridgeRegistrationError> {
    match CODEX_BRIDGE_SERVER.get_or_init(start_bridge_server) {
        Ok(server) => Ok(server),
        Err(error) => Err(CodexBridgeRegistrationError::Listener(error.clone())),
    }
}

fn start_bridge_server() -> Result<CodexBridgeServer, String> {
    let listener = StdTcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    if !address.ip().is_loopback() {
        return Err("Codex bridge did not bind a loopback address".to_string());
    }

    let registry = Arc::new(CodexRouteRegistry::default());
    let server_registry = registry.clone();
    thread::Builder::new()
        .name("gwt-codex-bridge".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => return,
            };
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(_) => return,
                };
                let router = Router::new()
                    .route(BRIDGE_ROUTE_PATH, get(handle_codex_websocket))
                    .with_state(BridgeHttpState {
                        registry: server_registry,
                    });
                let _ = axum::serve(listener, router).await;
            });
        })
        .map_err(|error| error.to_string())?;

    Ok(CodexBridgeServer {
        endpoint: format!("ws://127.0.0.1:{}{BRIDGE_ROUTE_PATH}", address.port()),
        registry,
    })
}

pub fn codex_bridge_endpoint() -> Result<String, CodexBridgeRegistrationError> {
    Ok(bridge_server()?.endpoint.clone())
}

pub fn register_codex_bridge_route(
    mut config: CodexBridgeRouteConfig,
) -> Result<CodexBridgeRouteLease, CodexBridgeRegistrationError> {
    let server = bridge_server()?;
    // A nested gwt launch may inherit an outer route capability from its
    // parent process. The app-server never needs the remote-TUI bearer, so
    // scrub both an explicit value and the inherited process environment at
    // the final spawn-owning boundary.
    config.app_server.env.remove(CODEX_REMOTE_AUTH_TOKEN_ENV);
    if !config
        .app_server
        .remove_env
        .iter()
        .any(|key| key == CODEX_REMOTE_AUTH_TOKEN_ENV)
    {
        config
            .app_server
            .remove_env
            .push(CODEX_REMOTE_AUTH_TOKEN_ENV.to_string());
    }
    // Two UUIDv4 values retain at least 244 random bits while remaining easy
    // to place in an environment variable without quoting or encoding.
    let bearer_token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let identity = CodexRouteIdentity::from_bearer_token(&bearer_token)
        .expect("generated Codex bridge bearer tokens exceed the minimum length");
    let route = Arc::new(CodexBridgeRoute {
        identity: identity.clone(),
        app_server: config.app_server,
        expected_resume_id: config.expected_resume_id,
        durability: config.durability,
        on_ready: config.on_ready,
        on_failure: config.on_failure,
        claimed: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        cancel_notify: tokio::sync::Notify::new(),
        root_forwarded: AtomicBool::new(false),
        failure_reported: AtomicBool::new(false),
    });
    server.registry.insert(identity.clone(), route.clone())?;
    Ok(CodexBridgeRouteLease {
        inner: Arc::new(BridgeLeaseInner {
            endpoint: server.endpoint.clone(),
            bearer_token,
            identity,
            registry: Arc::downgrade(&server.registry),
            route,
        }),
    })
}

async fn handle_codex_websocket(
    State(state): State<BridgeHttpState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(route) = state.registry.claim(token) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    websocket
        .max_message_size(MAX_JSON_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let proxy_route = route.clone();
            if let Err(failure) = proxy_codex_connection(socket, proxy_route).await {
                route.report_failure(failure.into_bridge_failure());
            }
        })
        .into_response()
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    (!token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_whitespace())).then_some(token)
}

#[derive(Debug, Clone, Copy)]
enum ProxyFailure {
    Spawn,
    Input,
    Output,
    Protocol,
    Durability,
    WebSocket,
    Oversized,
}

impl ProxyFailure {
    fn into_bridge_failure(self) -> CodexBridgeFailure {
        let (kind, reason) = match self {
            Self::Durability => (
                CodexBridgeFailureKind::Durability,
                "Codex bridge could not persist recovery state",
            ),
            Self::Protocol | Self::Oversized => (
                CodexBridgeFailureKind::Protocol,
                "Codex bridge rejected an invalid protocol message",
            ),
            Self::Spawn | Self::Input | Self::Output | Self::WebSocket => (
                CodexBridgeFailureKind::Transport,
                "Codex bridge transport failed",
            ),
        };
        CodexBridgeFailure {
            operation: None,
            kind,
            reason: reason.to_string(),
        }
    }
}

fn configure_app_server_command(
    command: &mut tokio::process::Command,
    launch: &CodexAppServerLaunch,
) {
    command
        .args(&launch.args)
        .envs(&launch.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in &launch.remove_env {
        command.env_remove(key);
    }
    if let Some(cwd) = launch.cwd.as_ref() {
        command.current_dir(cwd);
    }
}

async fn proxy_codex_connection(
    socket: WebSocket,
    route: Arc<CodexBridgeRoute>,
) -> Result<(), ProxyFailure> {
    let mut command = tokio::process::Command::new(&route.app_server.command);
    configure_app_server_command(&mut command, &route.app_server);
    gwt_core::process::configure_hidden_command(command.as_std_mut());

    let mut child = command.spawn().map_err(|_| ProxyFailure::Spawn)?;
    let mut app_server_input = child.stdin.take().ok_or(ProxyFailure::Input)?;
    let app_server_output = child.stdout.take().ok_or(ProxyFailure::Output)?;
    let mut app_server_output = BufReader::new(app_server_output);
    let (mut client_output, mut client_input) = socket.split();
    let mut tracker = CodexProtocolTracker::new(route.expected_resume_id.clone());
    let mut line = Vec::new();

    let proxy_result = loop {
        tokio::select! {
            _ = route.cancelled() => break Ok(()),
            client_message = client_input.next() => {
                let Some(client_message) = client_message else {
                    break Ok(());
                };
                let client_message = client_message.map_err(|_| ProxyFailure::WebSocket)?;
                match client_message {
                    Message::Text(raw) => {
                        let raw = raw.as_str();
                        let event = tracker
                            .inspect_client(raw)
                            .map_err(|_| ProxyFailure::Protocol)?
                            .into_event();
                        persist_event_async(route.durability.clone(), event, raw.to_string()).await?;
                        app_server_input
                            .write_all(raw.as_bytes())
                            .await
                            .map_err(|_| ProxyFailure::Input)?;
                        app_server_input
                            .write_all(b"\n")
                            .await
                            .map_err(|_| ProxyFailure::Input)?;
                        app_server_input.flush().await.map_err(|_| ProxyFailure::Input)?;
                    }
                    Message::Ping(payload) => {
                        client_output
                            .send(Message::Pong(payload))
                            .await
                            .map_err(|_| ProxyFailure::WebSocket)?;
                    }
                    Message::Close(_) => break Ok(()),
                    Message::Pong(_) => {}
                    Message::Binary(_) => break Err(ProxyFailure::Protocol),
                }
            }
            server_line = read_bounded_json_line(&mut app_server_output, &mut line) => {
                let Some(raw) = server_line? else {
                    break Ok(());
                };
                let event = tracker
                    .inspect_server(&raw)
                    .map_err(|_| ProxyFailure::Protocol)?
                    .into_event();
                let bridge_failure = match &event {
                    ProtocolEvent::ThreadOperationFailed { failure, .. } => Some(failure.clone()),
                    _ => None,
                };
                let is_root_binding = matches!(
                    event,
                    ProtocolEvent::ThreadBinding(ThreadBinding::Root(_))
                );
                persist_event_async(route.durability.clone(), event, raw.clone()).await?;
                client_output
                    .send(Message::Text(raw.into()))
                    .await
                    .map_err(|_| ProxyFailure::WebSocket)?;
                if is_root_binding && tracker.is_ready() {
                    // The source session may be retired only after the verified
                    // root response is durable and visible to the remote TUI.
                    route.mark_root_forwarded();
                }
                if let Some(failure) = bridge_failure {
                    // Release the structured app-server rejection to the TUI
                    // before asking AppRuntime to replace or retain the launch.
                    route.report_failure(failure);
                }
            }
        }
    };

    let _ = child.kill().await;
    let _ = child.wait().await;
    proxy_result
}

async fn persist_event_async(
    sink: Arc<dyn CodexDurabilitySink>,
    event: ProtocolEvent,
    wire_text: String,
) -> Result<(), ProxyFailure> {
    tokio::task::spawn_blocking(move || persist_protocol_event(sink.as_ref(), &event, &wire_text))
        .await
        .map_err(|_| ProxyFailure::Durability)?
        .map_err(|_| ProxyFailure::Durability)
}

fn persist_protocol_event(
    sink: &dyn CodexDurabilitySink,
    event: &ProtocolEvent,
    wire_text: &str,
) -> Result<(), String> {
    match event {
        ProtocolEvent::ThreadBinding(ThreadBinding::Root(binding)) => {
            sink.persist_root_binding(binding, wire_text)
        }
        ProtocolEvent::UserInput(input) => sink.persist_user_input(input, wire_text),
        ProtocolEvent::VisibleDiscussion(capture) => {
            sink.persist_visible_discussion(capture, wire_text)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
fn inspect_client_before_forward<'a>(
    tracker: &mut CodexProtocolTracker,
    sink: &dyn CodexDurabilitySink,
    wire_text: &'a str,
) -> Result<&'a str, String> {
    let inspected = tracker
        .inspect_client(wire_text)
        .map_err(|error| error.to_string())?;
    persist_protocol_event(sink, inspected.event(), wire_text)?;
    Ok(inspected.wire_text())
}

async fn read_bounded_json_line<R>(
    reader: &mut R,
    destination: &mut Vec<u8>,
) -> Result<Option<String>, ProxyFailure>
where
    R: AsyncBufRead + Unpin,
{
    destination.clear();
    loop {
        let (consumed, reached_line_end) = {
            let available = reader.fill_buf().await.map_err(|_| ProxyFailure::Output)?;
            if available.is_empty() {
                if destination.is_empty() {
                    return Ok(None);
                }
                (0, true)
            } else if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
                if destination.len() + index > MAX_JSON_MESSAGE_BYTES {
                    return Err(ProxyFailure::Oversized);
                }
                destination.extend_from_slice(&available[..index]);
                (index + 1, true)
            } else {
                if destination.len() + available.len() > MAX_JSON_MESSAGE_BYTES {
                    return Err(ProxyFailure::Oversized);
                }
                destination.extend_from_slice(available);
                (available.len(), false)
            }
        };
        reader.consume(consumed);
        if reached_line_end {
            if destination.last() == Some(&b'\r') {
                destination.pop();
            }
            let raw = String::from_utf8(destination.clone()).map_err(|_| ProxyFailure::Protocol)?;
            return Ok(Some(raw));
        }
    }
}

/// Filesystem-backed implementation used by production Host launches.
pub struct RecoveryCodexDurability {
    sessions_dir: PathBuf,
    session_id: String,
    recovery_id: String,
    project_dir: PathBuf,
    source_cwd: PathBuf,
}

impl RecoveryCodexDurability {
    pub fn new(
        sessions_dir: PathBuf,
        session_id: String,
        recovery_id: String,
        project_dir: PathBuf,
        source_cwd: PathBuf,
    ) -> Self {
        Self {
            sessions_dir,
            session_id,
            recovery_id,
            project_dir,
            source_cwd,
        }
    }

    fn collect_attachments(
        &self,
        input: &UserInputCapture,
        transferred: &[TransferredAttachment],
    ) -> Result<Vec<gwt_core::recovery::RecoveryAttachmentPayload>, String> {
        let attachment_count = input
            .attachment_candidates
            .len()
            .checked_add(transferred.len())
            .ok_or_else(attachment_count_error)?;
        let mut budget = RecoveryAttachmentAggregateBudget::default();
        budget.reserve_count(attachment_count)?;
        let mut prepared = Vec::with_capacity(attachment_count);

        for candidate in &input.attachment_candidates {
            match candidate.kind {
                AttachmentCandidateKind::LocalImage => {
                    let source = PathBuf::from(&candidate.source);
                    let source = if source.is_absolute() {
                        source
                    } else {
                        self.source_cwd.join(source)
                    };
                    let file_name = source
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| "Codex attachment file name is not UTF-8".to_string())?
                        .to_string();
                    prepared.push(PreparedRecoveryAttachment::Local(
                        open_prepared_local_attachment(&source, file_name, &mut budget)?,
                    ));
                }
                AttachmentCandidateKind::ImageUrl => {
                    let (file_name, encoded) = parse_image_data_url(&candidate.source)?;
                    reserve_base64_attachment(&mut budget, &file_name, encoded)?;
                    prepared.push(PreparedRecoveryAttachment::Base64 { file_name, encoded });
                }
            }
        }

        for attachment in transferred {
            reserve_base64_attachment(&mut budget, &attachment.file_name, &attachment.base64_data)?;
            prepared.push(PreparedRecoveryAttachment::Base64 {
                file_name: attachment.file_name.clone(),
                encoded: &attachment.base64_data,
            });
        }

        prepared
            .into_iter()
            .map(|attachment| match attachment {
                PreparedRecoveryAttachment::Local(local) => {
                    let (file_name, bytes) = read_prepared_local_attachment(local, &mut budget)?;
                    Ok(gwt_core::recovery::RecoveryAttachmentPayload { file_name, bytes })
                }
                PreparedRecoveryAttachment::Base64 { file_name, encoded } => {
                    let bytes = decode_base64_attachment(&mut budget, &file_name, encoded)?;
                    Ok(gwt_core::recovery::RecoveryAttachmentPayload { file_name, bytes })
                }
            })
            .collect()
    }

    fn persist_input_with_attachments(
        &self,
        input: &UserInputCapture,
        attachments: Vec<gwt_core::recovery::RecoveryAttachmentPayload>,
    ) -> Result<(), String> {
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&self.project_dir);
        let (operation_id, turn_id) = user_input_semantic_identity(input, &attachments);
        let update = gwt_core::recovery::RootTurnUpdate {
            root_id: input.thread_id.clone(),
            turn_id,
            input_text: Some(input.text_segments.join("\n")),
            visible_items: Vec::new(),
            attachment_refs: Vec::new(),
        };
        let result = if attachments.is_empty() {
            store.record_root_turn(&self.recovery_id, update, operation_id)
        } else {
            store.record_root_turn_with_attachments(
                &self.recovery_id,
                update,
                attachments,
                operation_id,
            )
        };
        result.map(|_| ()).map_err(|error| error.to_string())
    }
}

impl CodexDurabilitySink for RecoveryCodexDurability {
    fn persist_root_binding(
        &self,
        binding: &RootThreadBinding,
        _wire_text: &str,
    ) -> Result<(), String> {
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&self.project_dir);
        store
            .bind_root_semantic(
                &self.recovery_id,
                &binding.thread_id,
                Some(binding.session_id.clone()),
                gwt_core::recovery::BindingQuality::Verified,
                "codex-root",
            )
            .map_err(|error| error.to_string())?;

        gwt_agent::update_session(&self.sessions_dir, &self.session_id, |session| {
            if !session
                .session_history
                .iter()
                .any(|entry| entry.agent_session_id == binding.thread_id)
            {
                session
                    .session_history
                    .push(gwt_agent::session::AgentSessionHistoryEntry {
                        agent_session_id: binding.thread_id.clone(),
                        started_at: chrono::Utc::now(),
                    });
            }
            session.agent_session_id = Some(binding.thread_id.clone());
            session.observe_provider_root_role(gwt_agent::session::ProviderRootRole::Root)?;
            session.provider_binding_quality =
                Some(gwt_agent::session::ProviderBindingQuality::Verified);
            session.advance_recovery_launch_stage(
                gwt_agent::session::RecoveryLaunchStage::ProviderBound,
            )
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn persist_user_input(&self, input: &UserInputCapture, _wire_text: &str) -> Result<(), String> {
        let attachments = self.collect_attachments(input, &[])?;
        self.persist_input_with_attachments(input, attachments)
    }

    fn persist_visible_discussion(
        &self,
        capture: &VisibleDiscussionCapture,
        _wire_text: &str,
    ) -> Result<(), String> {
        let store = gwt_core::recovery::RecoveryStore::for_project_dir(&self.project_dir);
        let user_text = capture
            .items
            .iter()
            .filter(|item| item.role == VisibleDiscussionRole::User)
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let operation_id = visible_discussion_semantic_operation_id(capture)?;
        store
            .record_root_turn(
                &self.recovery_id,
                gwt_core::recovery::RootTurnUpdate {
                    root_id: capture.thread_id.clone(),
                    turn_id: capture.turn_id.clone(),
                    input_text: (!user_text.is_empty()).then_some(user_text),
                    visible_items: capture
                        .items
                        .iter()
                        .map(|item| gwt_core::recovery::VisibleDiscussionItem {
                            role: item.role.as_str().to_string(),
                            kind: item.kind.as_str().to_string(),
                            text: item.text.clone(),
                            partial: false,
                        })
                        .collect(),
                    attachment_refs: Vec::new(),
                },
                operation_id,
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn persist_transferred_user_input(
        &self,
        input: &UserInputCapture,
        attachments: &[TransferredAttachment],
        _wire_text: &str,
    ) -> Result<(), String> {
        let payloads = self.collect_attachments(input, attachments)?;
        self.persist_input_with_attachments(input, payloads)
    }
}

fn classify_thread_failure(
    pending: &PendingThreadRequest,
    error: &Map<String, Value>,
) -> Result<CodexBridgeFailure, ProtocolError> {
    let code = error
        .get("code")
        .ok_or(ProtocolError::MissingField("error.code"))?
        .as_i64()
        .ok_or(ProtocolError::InvalidField("error.code"))?;
    let message = error
        .get("message")
        .ok_or(ProtocolError::MissingField("error.message"))?
        .as_str()
        .ok_or(ProtocolError::InvalidField("error.message"))?;
    let bounded_message = (message.len() <= MAX_ERROR_MESSAGE_BYTES).then_some(message);

    let definitive_not_found = pending.operation == ThreadOperation::Resume
        && code == -32600
        && pending
            .requested_thread_id
            .as_deref()
            .filter(|thread_id| Uuid::parse_str(thread_id).is_ok())
            .is_some_and(|thread_id| {
                bounded_message
                    == Some(format!("no rollout found for thread id {thread_id}").as_str())
            });
    let kind = if definitive_not_found {
        CodexBridgeFailureKind::DefinitiveThreadNotFound
    } else if bounded_message.is_some_and(message_indicates_authentication_failure) {
        CodexBridgeFailureKind::Authentication
    } else if (-32602..=-32600).contains(&code) {
        CodexBridgeFailureKind::InvalidRequest
    } else {
        CodexBridgeFailureKind::Unknown
    };
    let reason = match kind {
        CodexBridgeFailureKind::DefinitiveThreadNotFound => {
            "Codex exact-resume thread was not found"
        }
        CodexBridgeFailureKind::Authentication => "Codex app-server authentication failed",
        CodexBridgeFailureKind::InvalidRequest => "Codex app-server rejected the thread request",
        _ => "Codex app-server rejected the thread operation",
    };
    Ok(CodexBridgeFailure {
        operation: Some(pending.operation),
        kind,
        reason: reason.to_string(),
    })
}

fn message_indicates_authentication_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "unauthorized",
        "authentication failed",
        "invalid authentication",
        "invalid token",
        "permission denied",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn bounded_visible_text(value: &str) -> String {
    if value.len() <= MAX_CAPTURE_TEXT_BYTES {
        return value.to_string();
    }
    let mut end = MAX_CAPTURE_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_string()
}

fn attachment_count_error() -> String {
    format!(
        "Codex recovery attachment count exceeds the {MAX_RECOVERY_ATTACHMENT_COUNT} item safety bound"
    )
}

fn attachment_aggregate_error() -> String {
    "Codex recovery attachment aggregate exceeds the safety bound".to_string()
}

pub(super) struct RecoveryAttachmentAggregateBudget {
    count_limit: usize,
    raw_limit: usize,
    encoded_limit: usize,
    count: usize,
    projected_raw: usize,
    projected_encoded: usize,
    actual_raw: usize,
    actual_encoded: usize,
}

impl Default for RecoveryAttachmentAggregateBudget {
    fn default() -> Self {
        Self::with_limits(
            MAX_RECOVERY_ATTACHMENT_COUNT,
            MAX_RECOVERY_ATTACHMENT_AGGREGATE_BYTES,
            MAX_RECOVERY_ATTACHMENT_CONTROL_BYTES,
        )
    }
}

impl RecoveryAttachmentAggregateBudget {
    fn with_limits(count_limit: usize, raw_limit: usize, encoded_limit: usize) -> Self {
        Self {
            count_limit,
            raw_limit,
            encoded_limit,
            count: 0,
            projected_raw: 0,
            projected_encoded: 0,
            actual_raw: 0,
            actual_encoded: 0,
        }
    }

    pub(super) fn reserve_count(&mut self, count: usize) -> Result<(), String> {
        let total = self
            .count
            .checked_add(count)
            .ok_or_else(attachment_count_error)?;
        if total > self.count_limit {
            return Err(attachment_count_error());
        }
        self.count = total;
        Ok(())
    }

    pub(super) fn reserve_projected(
        &mut self,
        raw_bytes: usize,
        encoded_bytes: usize,
    ) -> Result<(), String> {
        let raw_total = self
            .projected_raw
            .checked_add(raw_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        let encoded_total = self
            .projected_encoded
            .checked_add(encoded_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        if raw_total > self.raw_limit || encoded_total > self.encoded_limit {
            return Err(attachment_aggregate_error());
        }
        self.projected_raw = raw_total;
        self.projected_encoded = encoded_total;
        Ok(())
    }

    pub(super) fn consume_actual(
        &mut self,
        raw_bytes: usize,
        encoded_bytes: usize,
    ) -> Result<(), String> {
        let raw_total = self
            .actual_raw
            .checked_add(raw_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        let encoded_total = self
            .actual_encoded
            .checked_add(encoded_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        if raw_total > self.raw_limit || encoded_total > self.encoded_limit {
            return Err(attachment_aggregate_error());
        }
        self.actual_raw = raw_total;
        self.actual_encoded = encoded_total;
        Ok(())
    }

    pub(super) fn ensure_actual_capacity(
        &self,
        raw_bytes: usize,
        encoded_bytes: usize,
    ) -> Result<(), String> {
        let raw_total = self
            .actual_raw
            .checked_add(raw_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        let encoded_total = self
            .actual_encoded
            .checked_add(encoded_bytes)
            .ok_or_else(attachment_aggregate_error)?;
        if raw_total > self.raw_limit || encoded_total > self.encoded_limit {
            return Err(attachment_aggregate_error());
        }
        Ok(())
    }

    pub(super) fn remaining_actual_raw(&self) -> usize {
        self.raw_limit.saturating_sub(self.actual_raw)
    }

    pub(super) fn remaining_actual_encoded(&self) -> usize {
        self.encoded_limit.saturating_sub(self.actual_encoded)
    }
}

pub(super) struct PreparedLocalAttachment {
    file_name: String,
    file: BoundedRegularFile,
}

enum PreparedRecoveryAttachment<'a> {
    Local(PreparedLocalAttachment),
    Base64 { file_name: String, encoded: &'a str },
}

pub(super) fn projected_base64_len(raw_bytes: usize) -> Result<usize, String> {
    raw_bytes
        .checked_add(2)
        .map(|bytes| bytes / 3)
        .and_then(|groups| groups.checked_mul(4))
        .ok_or_else(attachment_aggregate_error)
}

pub(super) fn max_raw_bytes_for_base64_capacity(encoded_capacity: usize) -> usize {
    (encoded_capacity / 4).saturating_mul(3)
}

pub(super) fn projected_decoded_base64_len(encoded: &str) -> Result<usize, String> {
    let groups = encoded
        .len()
        .checked_add(3)
        .map(|bytes| bytes / 4)
        .ok_or_else(attachment_aggregate_error)?;
    let upper_bound = groups
        .checked_mul(3)
        .ok_or_else(attachment_aggregate_error)?;
    let padding = encoded
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .take(2)
        .count();
    Ok(upper_bound.saturating_sub(padding))
}

pub(super) fn attachment_encoded_footprint(
    encoded_bytes: usize,
    file_name: &str,
) -> Result<usize, String> {
    encoded_bytes
        .checked_add(file_name.len())
        .ok_or_else(attachment_aggregate_error)
}

pub(super) fn open_prepared_local_attachment(
    path: &Path,
    file_name: String,
    budget: &mut RecoveryAttachmentAggregateBudget,
) -> Result<PreparedLocalAttachment, String> {
    let file = BoundedRegularFile::open(
        path,
        MAX_RECOVERY_ATTACHMENT_BYTES as u64,
        "Codex attachment source",
    )
    .map_err(|error| format!("read Codex attachment: {error}"))?;
    let raw_bytes = usize::try_from(file.byte_len()).map_err(|_| attachment_aggregate_error())?;
    let encoded_bytes = attachment_encoded_footprint(projected_base64_len(raw_bytes)?, &file_name)?;
    budget.reserve_projected(raw_bytes, encoded_bytes)?;
    Ok(PreparedLocalAttachment { file_name, file })
}

pub(super) fn read_prepared_local_attachment(
    attachment: PreparedLocalAttachment,
    budget: &mut RecoveryAttachmentAggregateBudget,
) -> Result<(String, Vec<u8>), String> {
    let encoded_capacity = budget
        .remaining_actual_encoded()
        .checked_sub(attachment.file_name.len())
        .ok_or_else(attachment_aggregate_error)?;
    let max_bytes = budget
        .remaining_actual_raw()
        .min(max_raw_bytes_for_base64_capacity(encoded_capacity))
        .min(MAX_RECOVERY_ATTACHMENT_BYTES);
    let bytes = attachment
        .file
        .read_all_with_limit(max_bytes as u64)
        .map_err(|error| format!("read Codex attachment: {error}"))?;
    let encoded_bytes =
        attachment_encoded_footprint(projected_base64_len(bytes.len())?, &attachment.file_name)?;
    budget.consume_actual(bytes.len(), encoded_bytes)?;
    Ok((attachment.file_name, bytes))
}

fn reserve_base64_attachment(
    budget: &mut RecoveryAttachmentAggregateBudget,
    file_name: &str,
    encoded: &str,
) -> Result<(), String> {
    let raw_bytes = projected_decoded_base64_len(encoded)?;
    let encoded_bytes = attachment_encoded_footprint(encoded.len(), file_name)?;
    budget.reserve_projected(raw_bytes, encoded_bytes)
}

fn decode_base64_attachment(
    budget: &mut RecoveryAttachmentAggregateBudget,
    file_name: &str,
    encoded: &str,
) -> Result<Vec<u8>, String> {
    let encoded_bytes = attachment_encoded_footprint(encoded.len(), file_name)?;
    budget.ensure_actual_capacity(0, encoded_bytes)?;
    let max_bytes = budget
        .remaining_actual_raw()
        .min(MAX_RECOVERY_ATTACHMENT_BYTES);
    let bytes = decode_bounded_base64_with_limit(encoded, max_bytes)?;
    budget.consume_actual(bytes.len(), encoded_bytes)?;
    Ok(bytes)
}

pub(super) fn decode_bounded_base64_with_limit(
    encoded: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let max_bytes = max_bytes.min(MAX_RECOVERY_ATTACHMENT_BYTES);
    if projected_decoded_base64_len(encoded)? > max_bytes {
        return Err("Codex attachment exceeds the recovery safety bound".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Codex attachment has invalid base64 content".to_string())?;
    if bytes.len() > max_bytes {
        return Err("Codex attachment exceeds the recovery safety bound".to_string());
    }
    Ok(bytes)
}

#[cfg(test)]
fn read_bounded_attachment(path: &Path) -> Result<Vec<u8>, String> {
    gwt_core::recovery::read_recovery_attachment_bytes_with_limit(
        path,
        MAX_RECOVERY_ATTACHMENT_BYTES as u64,
    )
    .map_err(|error| format!("read Codex attachment: {error}"))
}

pub(super) fn parse_image_data_url(source: &str) -> Result<(String, &str), String> {
    let source = source
        .strip_prefix("data:")
        .ok_or_else(|| "remote image URLs cannot be made crash-safe".to_string())?;
    let (metadata, encoded) = source
        .split_once(',')
        .ok_or_else(|| "Codex image data URL is malformed".to_string())?;
    let mime = metadata
        .strip_suffix(";base64")
        .ok_or_else(|| "Codex image data URL must use base64 encoding".to_string())?;
    let extension = match mime.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "image",
    };
    Ok((format!("codex-attachment.{extension}"), encoded))
}

fn semantic_operation_id(prefix: &str, components: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for component in components {
        hasher.update((component.len() as u64).to_le_bytes());
        hasher.update(component.as_bytes());
    }
    format!("{prefix}:{}", hex::encode(hasher.finalize()))
}

fn user_input_semantic_identity(
    input: &UserInputCapture,
    attachments: &[gwt_core::recovery::RecoveryAttachmentPayload],
) -> (String, String) {
    if let Some(client_id) = input
        .client_user_message_id
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= 128)
    {
        return (
            semantic_operation_id("codex-input", &[&input.thread_id, "client", client_id]),
            client_id.to_string(),
        );
    }

    // Some Codex clients omit clientUserMessageId, and JSON-RPC request ids
    // may be regenerated after reconnect. Use only the canonical provider
    // input semantics so a reserialized/restarted retry remains the same turn.
    let kind = match input.kind {
        UserInputKind::Start => "start",
        UserInputKind::Steer => "steer",
    };
    let mut components = vec![
        input.thread_id.clone(),
        kind.to_string(),
        format!("text-count:{}", input.text_segments.len()),
    ];
    for text in &input.text_segments {
        components.push("text".to_string());
        components.push(text.clone());
    }
    components.push(format!("attachment-count:{}", attachments.len()));
    for attachment in attachments {
        components.push("attachment".to_string());
        components.push(hex::encode(Sha256::digest(&attachment.bytes)));
    }
    let borrowed = components.iter().map(String::as_str).collect::<Vec<_>>();
    let operation_id = semantic_operation_id("codex-input", &borrowed);
    let turn_id = operation_id.split_once(':').map_or_else(
        || operation_id.clone(),
        |(_, digest)| format!("codex-semantic-{digest}"),
    );
    (operation_id, turn_id)
}

fn visible_discussion_semantic_operation_id(
    capture: &VisibleDiscussionCapture,
) -> Result<String, String> {
    let role = capture
        .items
        .first()
        .map(|item| item.role)
        .ok_or_else(|| "Codex visible discussion has no items".to_string())?;
    if capture.items.iter().any(|item| item.role != role) {
        return Err("Codex visible discussion mixes semantic roles".to_string());
    }
    Ok(semantic_operation_id(
        "codex-visible",
        &[
            &capture.thread_id,
            &capture.turn_id,
            &capture.item_id,
            role.as_str(),
        ],
    ))
}

fn parse_message(wire_text: &str) -> Result<Value, ProtocolError> {
    serde_json::from_str(wire_text).map_err(|error| ProtocolError::InvalidJson(error.to_string()))
}

fn message_object(value: &Value) -> Result<&Map<String, Value>, ProtocolError> {
    value.as_object().ok_or(ProtocolError::InvalidEnvelope)
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a Map<String, Value>, ProtocolError> {
    object
        .get(field)
        .ok_or(ProtocolError::MissingField(field))?
        .as_object()
        .ok_or(ProtocolError::InvalidField(field))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, ProtocolError> {
    object
        .get(field)
        .ok_or(ProtocolError::MissingField(field))?
        .as_str()
        .ok_or(ProtocolError::InvalidField(field))
}

fn optional_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ProtocolError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ProtocolError::InvalidField(field)),
    }
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<bool>, ProtocolError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(ProtocolError::InvalidField(field)),
    }
}

fn required_rpc_id(object: &Map<String, Value>) -> Result<RpcId, ProtocolError> {
    optional_rpc_id(object)?.ok_or(ProtocolError::MissingField("id"))
}

fn optional_rpc_id(object: &Map<String, Value>) -> Result<Option<RpcId>, ProtocolError> {
    match object.get("id") {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(RpcId::String(value.clone()))),
        Some(Value::Number(value)) => value
            .as_i64()
            .map(RpcId::Number)
            .map(Some)
            .ok_or(ProtocolError::InvalidField("id")),
        Some(_) => Err(ProtocolError::InvalidField("id")),
    }
}

fn parse_thread(object: &Map<String, Value>) -> Result<ParsedThread, ProtocolError> {
    Ok(ParsedThread {
        thread_id: required_string(object, "id")?.to_string(),
        session_id: required_string(object, "sessionId")?.to_string(),
        cli_version: required_string(object, "cliVersion")?.to_string(),
        parent_thread_id: optional_string(object, "parentThreadId")?,
        forked_from_id: optional_string(object, "forkedFromId")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_tracker(expected_resume_id: Option<&str>) -> CodexProtocolTracker {
        let mut tracker = CodexProtocolTracker::new(expected_resume_id.map(str::to_string));
        tracker
            .inspect_client(
                r#"{"id":1,"method":"initialize","params":{"clientInfo":{"name":"codex-tui","version":"0.144.5"}}}"#,
            )
            .expect("initialize request");
        tracker
            .inspect_server(
                r#"{"id":1,"result":{"codexHome":"/tmp/codex","platformFamily":"unix","platformOs":"linux","userAgent":"codex-cli/0.144.5"}}"#,
            )
            .expect("initialize response");
        let initialized = tracker
            .inspect_client(r#"{"method":"initialized","params":null}"#)
            .expect("initialized notification");
        assert_eq!(initialized.event(), &ProtocolEvent::Ready);
        assert!(tracker.is_ready());
        tracker
    }

    #[test]
    fn unknown_method_and_fields_preserve_wire_text_exactly() {
        let mut tracker = CodexProtocolTracker::new(None);
        let raw = "{ \"id\" : \"future-1\", \"method\":\"future/method\", \"params\":{\"x\":1}, \"newField\":true }";

        let inspected = tracker.inspect_client(raw).expect("valid JSON-RPC");

        assert_eq!(inspected.wire_text(), raw);
        assert_eq!(inspected.event(), &ProtocolEvent::Passthrough);
    }

    #[test]
    fn initialize_response_then_initialized_marks_bridge_ready() {
        let tracker = ready_tracker(None);
        assert_eq!(tracker.handshake_state(), HandshakeState::Ready);
    }

    #[test]
    fn thread_start_response_extracts_root_binding() {
        let mut tracker = ready_tracker(None);
        tracker
            .inspect_client(r#"{"id":"start-1","method":"thread/start","params":{"cwd":"/repo"}}"#)
            .expect("thread start request");

        let inspected = tracker
            .inspect_server(
                r#"{"id":"start-1","result":{"thread":{"id":"019-root","sessionId":"tree-1","cliVersion":"0.144.5","parentThreadId":null,"forkedFromId":null,"future":"kept"}},"extra":42}"#,
            )
            .expect("thread start response");

        assert_eq!(
            inspected.event(),
            &ProtocolEvent::ThreadBinding(ThreadBinding::Root(RootThreadBinding {
                thread_id: "019-root".to_string(),
                session_id: "tree-1".to_string(),
                cli_version: "0.144.5".to_string(),
                forked_from_id: None,
                operation: ThreadOperation::Start,
            }))
        );
        assert_eq!(tracker.root_thread_id(), Some("019-root"));
    }

    #[test]
    fn child_thread_response_is_never_promoted_to_root() {
        let mut tracker = ready_tracker(None);
        tracker
            .inspect_client(r#"{"id":2,"method":"thread/start","params":{}}"#)
            .expect("thread start request");

        let inspected = tracker
            .inspect_server(
                r#"{"id":2,"result":{"thread":{"id":"child-1","sessionId":"tree-1","cliVersion":"0.144.5","parentThreadId":"root-1","forkedFromId":null}}}"#,
            )
            .expect("child response remains observable");

        assert_eq!(
            inspected.event(),
            &ProtocolEvent::ThreadBinding(ThreadBinding::Child(ChildThreadBinding {
                thread_id: "child-1".to_string(),
                session_id: "tree-1".to_string(),
                cli_version: "0.144.5".to_string(),
                parent_thread_id: "root-1".to_string(),
                operation: ThreadOperation::Start,
            }))
        );
        assert_eq!(tracker.root_thread_id(), None);
    }

    #[test]
    fn server_request_id_cannot_consume_client_pending_request() {
        let mut tracker = ready_tracker(None);
        tracker
            .inspect_client(r#"{"id":7,"method":"thread/start","params":{}}"#)
            .expect("thread start request");

        let server_request = tracker
            .inspect_server(
                r#"{"id":7,"method":"item/commandExecution/requestApproval","params":{"threadId":"root-1"}}"#,
            )
            .expect("server request passes through");
        assert_eq!(server_request.event(), &ProtocolEvent::Passthrough);

        let response = tracker
            .inspect_server(
                r#"{"id":7,"result":{"thread":{"id":"root-1","sessionId":"tree-1","cliVersion":"0.144.5","parentThreadId":null}}}"#,
            )
            .expect("client response remains correlated");
        assert!(matches!(
            response.event(),
            ProtocolEvent::ThreadBinding(ThreadBinding::Root(_))
        ));
    }

    #[test]
    fn thread_fork_response_extracts_new_root_and_source() {
        let mut tracker = ready_tracker(None);
        tracker
            .inspect_client(
                r#"{"id":"fork-1","method":"thread/fork","params":{"threadId":"root-1"}}"#,
            )
            .expect("thread fork request");

        let response = tracker
            .inspect_server(
                r#"{"id":"fork-1","result":{"thread":{"id":"root-2","sessionId":"tree-2","cliVersion":"0.144.5","parentThreadId":null,"forkedFromId":"root-1"}}}"#,
            )
            .expect("thread fork response");

        assert_eq!(
            response.event(),
            &ProtocolEvent::ThreadBinding(ThreadBinding::Root(RootThreadBinding {
                thread_id: "root-2".to_string(),
                session_id: "tree-2".to_string(),
                cli_version: "0.144.5".to_string(),
                forked_from_id: Some("root-1".to_string()),
                operation: ThreadOperation::Fork,
            }))
        );
    }

    #[test]
    fn resume_request_with_unexpected_id_fails_before_forwarding() {
        let mut tracker = ready_tracker(Some("expected-root"));

        let error = tracker
            .inspect_client(
                r#"{"id":3,"method":"thread/resume","params":{"threadId":"wrong-root"}}"#,
            )
            .expect_err("resume mismatch must fail closed");

        assert_eq!(
            error,
            ProtocolError::ResumeThreadMismatch {
                expected: "expected-root".to_string(),
                actual: "wrong-root".to_string(),
            }
        );
    }

    #[test]
    fn exact_resume_rejects_start_instead_of_silently_creating_root() {
        let mut tracker = ready_tracker(Some("expected-root"));

        let error = tracker
            .inspect_client(r#"{"id":31,"method":"thread/start","params":{}}"#)
            .expect_err("exact resume must not become a new thread");

        assert_eq!(
            error,
            ProtocolError::ExpectedResumeOperation {
                actual: ThreadOperation::Start,
            }
        );
        assert_eq!(tracker.root_thread_id(), None);
    }

    #[test]
    fn matching_resume_response_extracts_root_binding() {
        let mut tracker = ready_tracker(Some("expected-root"));
        tracker
            .inspect_client(
                r#"{"id":32,"method":"thread/resume","params":{"threadId":"expected-root"}}"#,
            )
            .expect("matching resume request");

        let response = tracker
            .inspect_server(
                r#"{"id":32,"result":{"thread":{"id":"expected-root","sessionId":"tree-1","cliVersion":"0.144.5","parentThreadId":null}}}"#,
            )
            .expect("matching resume response");

        assert_eq!(
            response.event(),
            &ProtocolEvent::ThreadBinding(ThreadBinding::Root(RootThreadBinding {
                thread_id: "expected-root".to_string(),
                session_id: "tree-1".to_string(),
                cli_version: "0.144.5".to_string(),
                forked_from_id: None,
                operation: ThreadOperation::Resume,
            }))
        );
    }

    #[test]
    fn exact_measured_resume_rejection_is_the_only_definitive_not_found_signal() {
        let mut tracker = ready_tracker(Some("00000000-0000-0000-0000-000000000000"));
        tracker
            .inspect_client(
                r#"{"id":34,"method":"thread/resume","params":{"threadId":"00000000-0000-0000-0000-000000000000"}}"#,
            )
            .expect("resume request");

        let response = tracker
            .inspect_server(
                r#"{"error":{"code":-32600,"message":"no rollout found for thread id 00000000-0000-0000-0000-000000000000"},"id":34}"#,
            )
            .expect("structured rejection");

        assert_eq!(
            response.event(),
            &ProtocolEvent::ThreadOperationFailed {
                operation: ThreadOperation::Resume,
                failure: CodexBridgeFailure {
                    operation: Some(ThreadOperation::Resume),
                    kind: CodexBridgeFailureKind::DefinitiveThreadNotFound,
                    reason: "Codex exact-resume thread was not found".to_string(),
                },
            }
        );
        assert!(!format!("{:?}", response.event()).contains("00000000"));
    }

    #[test]
    fn generic_invalid_request_never_becomes_checkpoint_fallback() {
        let mut tracker = ready_tracker(Some("expected-root"));
        tracker
            .inspect_client(
                r#"{"id":35,"method":"thread/resume","params":{"threadId":"expected-root"}}"#,
            )
            .expect("resume request");

        let response = tracker
            .inspect_server(
                r#"{"id":35,"error":{"code":-32600,"message":"invalid params: secret-provider-token"}}"#,
            )
            .expect("structured rejection");

        let ProtocolEvent::ThreadOperationFailed { failure, .. } = response.event() else {
            panic!("expected failure event");
        };
        assert_eq!(failure.kind, CodexBridgeFailureKind::InvalidRequest);
        assert!(!failure.reason.contains("secret-provider-token"));
    }

    #[test]
    fn app_server_version_mismatch_fails_before_root_response_is_released() {
        let mut tracker = ready_tracker(None);
        tracker
            .inspect_client(r#"{"id":33,"method":"thread/start","params":{}}"#)
            .expect("thread start request");

        let error = tracker
            .inspect_server(
                r#"{"id":33,"result":{"thread":{"id":"root-1","sessionId":"tree-1","cliVersion":"0.145.0","parentThreadId":null}}}"#,
            )
            .expect_err("mixed Codex versions must fail closed");

        assert_eq!(
            error,
            ProtocolError::CliVersionMismatch {
                client: "0.144.5".to_string(),
                server: "0.145.0".to_string(),
            }
        );
        assert_eq!(tracker.root_thread_id(), None);
    }

    #[test]
    fn resume_response_with_unexpected_id_fails_before_forwarding() {
        let mut tracker = ready_tracker(Some("expected-root"));
        tracker
            .inspect_client(
                r#"{"id":4,"method":"thread/resume","params":{"threadId":"expected-root"}}"#,
            )
            .expect("matching resume request");

        let error = tracker
            .inspect_server(
                r#"{"id":4,"result":{"thread":{"id":"different-root","sessionId":"tree-1","cliVersion":"0.144.5","parentThreadId":null,"forkedFromId":null}}}"#,
            )
            .expect_err("response mismatch must fail closed");

        assert_eq!(
            error,
            ProtocolError::ResumeThreadMismatch {
                expected: "expected-root".to_string(),
                actual: "different-root".to_string(),
            }
        );
    }

    #[test]
    fn turn_start_extracts_only_user_text_segments() {
        let mut tracker = ready_tracker(None);
        let inspected = tracker
            .inspect_client(
                r#"{"id":5,"method":"turn/start","params":{"threadId":"root-1","clientUserMessageId":"msg-1","input":[{"type":"text","text":"first","future":1},{"type":"image","url":"data:image/png;base64,abc"},{"type":"text","text":"second"}]}}"#,
            )
            .expect("turn start");

        assert_eq!(
            inspected.event(),
            &ProtocolEvent::UserInput(UserInputCapture {
                kind: UserInputKind::Start,
                thread_id: "root-1".to_string(),
                client_user_message_id: Some("msg-1".to_string()),
                text_segments: vec!["first".to_string(), "second".to_string()],
                attachment_candidates: vec![AttachmentCandidate {
                    kind: AttachmentCandidateKind::ImageUrl,
                    source: "data:image/png;base64,abc".to_string(),
                    detail: None,
                }],
            })
        );
    }

    #[test]
    fn user_input_rejects_attachment_fanout_before_capture_allocation_grows_unbounded() {
        let mut tracker = ready_tracker(None);
        let input = (0..=MAX_RECOVERY_ATTACHMENT_COUNT)
            .map(|index| {
                serde_json::json!({
                    "type": "localImage",
                    "path": format!("missing-{index}.png")
                })
            })
            .collect::<Vec<_>>();
        let raw = serde_json::json!({
            "id": 55,
            "method": "turn/start",
            "params": {
                "threadId": "root-1",
                "input": input
            }
        })
        .to_string();

        let error = tracker
            .inspect_client(&raw)
            .expect_err("attachment candidate count must be bounded while parsing");
        assert_eq!(
            error,
            ProtocolError::TooManyAttachments {
                limit: MAX_RECOVERY_ATTACHMENT_COUNT
            }
        );
    }

    #[test]
    fn turn_steer_extracts_user_text() {
        let mut tracker = ready_tracker(None);
        let inspected = tracker
            .inspect_client(
                r#"{"id":6,"method":"turn/steer","params":{"threadId":"root-1","expectedTurnId":"turn-1","input":[{"type":"text","text":"change course"}]}}"#,
            )
            .expect("turn steer");

        assert_eq!(
            inspected.event(),
            &ProtocolEvent::UserInput(UserInputCapture {
                kind: UserInputKind::Steer,
                thread_id: "root-1".to_string(),
                client_user_message_id: None,
                text_segments: vec!["change course".to_string()],
                attachment_candidates: Vec::new(),
            })
        );
    }

    #[test]
    fn completed_root_visible_items_are_captured_but_deltas_reasoning_and_children_are_not() {
        let mut tracker = ready_tracker(None);
        tracker.root_thread_id = Some("root-1".to_string());

        let assistant = tracker
            .inspect_server(
                r#"{"method":"item/completed","params":{"threadId":"root-1","turnId":"turn-1","completedAtMs":1,"item":{"id":"item-1","type":"agentMessage","text":"Visible answer","phase":"final_answer"}}}"#,
            )
            .expect("completed assistant item");
        assert_eq!(
            assistant.event(),
            &ProtocolEvent::VisibleDiscussion(VisibleDiscussionCapture {
                thread_id: "root-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                items: vec![VisibleDiscussionItemCapture {
                    role: VisibleDiscussionRole::Assistant,
                    kind: VisibleDiscussionKind::AssistantMessage,
                    text: "Visible answer".to_string(),
                }],
            })
        );

        let plan = tracker
            .inspect_server(
                r#"{"method":"item/completed","params":{"threadId":"root-1","turnId":"turn-1","completedAtMs":2,"item":{"id":"item-2","type":"plan","text":"Visible plan"}}}"#,
            )
            .expect("completed plan item");
        assert!(matches!(plan.event(), ProtocolEvent::VisibleDiscussion(_)));

        for ignored in [
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"root-1","turnId":"turn-1","itemId":"item-1","delta":"stream"}}"#,
            r#"{"method":"item/completed","params":{"threadId":"root-1","turnId":"turn-1","completedAtMs":3,"item":{"id":"reason-1","type":"reasoning","summary":["hidden"]}}}"#,
            r#"{"method":"item/completed","params":{"threadId":"child-1","turnId":"turn-c","completedAtMs":4,"item":{"id":"item-c","type":"agentMessage","text":"child output"}}}"#,
        ] {
            assert_eq!(
                tracker
                    .inspect_server(ignored)
                    .expect("ignored item")
                    .event(),
                &ProtocolEvent::Passthrough
            );
        }
    }

    #[test]
    fn structured_questions_and_non_secret_answers_are_correlated_on_the_root_only() {
        let mut tracker = ready_tracker(None);
        tracker.root_thread_id = Some("root-1".to_string());
        let question = tracker
            .inspect_server(
                r#"{"id":"ask-1","method":"item/tool/requestUserInput","params":{"threadId":"root-1","turnId":"turn-2","itemId":"ask-item","questions":[{"id":"choice","header":"Mode","question":"Choose mode","options":[{"label":"Safe","description":"Use safeguards"}]},{"id":"password","header":"Secret","question":"Enter token","isSecret":true}]}}"#,
            )
            .expect("structured question");
        let ProtocolEvent::VisibleDiscussion(capture) = question.event() else {
            panic!("question must be captured");
        };
        assert_eq!(capture.items.len(), 2);
        assert_eq!(
            capture.items[0].kind,
            VisibleDiscussionKind::StructuredQuestion
        );
        assert_eq!(capture.items[0].text, "Mode: Choose mode");

        let answer = tracker
            .inspect_client(
                r#"{"id":"ask-1","result":{"answers":{"choice":{"answers":["Safe"]},"password":{"answers":["super-secret"]}}}}"#,
            )
            .expect("structured answer");
        let ProtocolEvent::VisibleDiscussion(capture) = answer.event() else {
            panic!("answer must be captured");
        };
        assert_eq!(capture.items.len(), 1, "secret answer is never captured");
        assert_eq!(capture.items[0].role, VisibleDiscussionRole::User);
        assert_eq!(
            capture.items[0].kind,
            VisibleDiscussionKind::StructuredAnswer
        );
        assert_eq!(capture.items[0].text, "Safe");
        assert!(!format!("{capture:?}").contains("super-secret"));

        let child = tracker
            .inspect_server(
                r#"{"id":"ask-child","method":"item/tool/requestUserInput","params":{"threadId":"child-1","turnId":"turn-c","itemId":"ask-c","questions":[{"id":"q","header":"Child","question":"Ignore me"}]}}"#,
            )
            .expect("child question passthrough");
        assert_eq!(child.event(), &ProtocolEvent::Passthrough);
        let child_answer = tracker
            .inspect_client(
                r#"{"id":"ask-child","result":{"answers":{"q":{"answers":["ignored"]}}}}"#,
            )
            .expect("uncorrelated child answer passthrough");
        assert_eq!(child_answer.event(), &ProtocolEvent::Passthrough);
    }

    #[test]
    fn route_identity_never_formats_plain_bearer_token() {
        let token = "0123456789abcdef0123456789abcdef-secret";
        let identity = CodexRouteIdentity::from_bearer_token(token).expect("strong token");

        assert!(identity.matches_bearer_token(token));
        assert!(!identity.matches_bearer_token("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-secret"));
        let debug = format!("{identity:?}");
        assert!(!debug.contains(token));
        assert_eq!(debug, "CodexRouteIdentity([REDACTED])");
    }

    #[test]
    fn route_identity_rejects_short_tokens() {
        assert_eq!(
            CodexRouteIdentity::from_bearer_token("too-short"),
            Err(RouteIdentityError::TokenTooShort)
        );
    }

    #[derive(Default)]
    struct RecordingDurabilitySink {
        persisted_inputs: std::sync::Mutex<Vec<String>>,
        fail_inputs: bool,
    }

    impl CodexDurabilitySink for RecordingDurabilitySink {
        fn persist_root_binding(
            &self,
            _binding: &RootThreadBinding,
            _wire_text: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        fn persist_user_input(
            &self,
            input: &UserInputCapture,
            _wire_text: &str,
        ) -> Result<(), String> {
            if self.fail_inputs {
                return Err("injected persistence failure".to_string());
            }
            self.persisted_inputs
                .lock()
                .expect("record inputs")
                .extend(input.text_segments.clone());
            Ok(())
        }

        fn persist_visible_discussion(
            &self,
            capture: &VisibleDiscussionCapture,
            _wire_text: &str,
        ) -> Result<(), String> {
            self.persisted_inputs
                .lock()
                .expect("record visible discussion")
                .extend(capture.items.iter().map(|item| item.text.clone()));
            Ok(())
        }

        fn persist_transferred_user_input(
            &self,
            input: &UserInputCapture,
            _attachments: &[TransferredAttachment],
            wire_text: &str,
        ) -> Result<(), String> {
            self.persist_user_input(input, wire_text)
        }
    }

    #[test]
    fn client_input_persistence_barrier_runs_before_raw_forward_is_released() {
        let mut tracker = ready_tracker(None);
        tracker.root_thread_id = Some("root-1".to_string());
        let sink = RecordingDurabilitySink::default();
        let raw = r#"{"id":5,"method":"turn/start","params":{"threadId":"root-1","input":[{"type":"text","text":"persist me"}]},"future":true}"#;

        let released = inspect_client_before_forward(&mut tracker, &sink, raw)
            .expect("durability barrier succeeds");

        assert_eq!(released, raw, "the exact wire JSON must be released");
        assert_eq!(
            sink.persisted_inputs
                .lock()
                .expect("record inputs")
                .as_slice(),
            ["persist me"]
        );
    }

    #[test]
    fn failed_client_input_persistence_never_releases_raw_forward() {
        let mut tracker = ready_tracker(None);
        tracker.root_thread_id = Some("root-1".to_string());
        let sink = RecordingDurabilitySink {
            fail_inputs: true,
            ..RecordingDurabilitySink::default()
        };
        let raw = r#"{"id":5,"method":"turn/start","params":{"threadId":"root-1","input":[{"type":"text","text":"do not forward"}]}}"#;

        let error = inspect_client_before_forward(&mut tracker, &sink, raw)
            .expect_err("persistence failure must hold the forwarding barrier");

        assert!(error.contains("persistence"));
    }

    #[test]
    fn host_attachment_reader_rejects_oversized_and_symlink_sources() {
        let temp = tempfile::tempdir().expect("temp");
        let oversized = temp.path().join("oversized.bin");
        std::fs::File::create(&oversized)
            .expect("create sparse attachment")
            .set_len(MAX_RECOVERY_ATTACHMENT_BYTES as u64 + 1)
            .expect("size sparse attachment");
        let oversized_error = match read_bounded_attachment(&oversized) {
            Err(error) => error,
            Ok(_) => panic!("oversized host attachment must be rejected before reading"),
        };
        assert!(oversized_error.contains("size limit"), "{oversized_error}");

        let target = temp.path().join("private.txt");
        let link = temp.path().join("image.txt");
        std::fs::write(&target, b"private bytes").expect("target");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        #[cfg(windows)]
        if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                return;
            }
            panic!("create attachment symlink: {error}");
        }
        assert!(read_bounded_attachment(&link).is_err());
    }

    #[test]
    fn host_attachment_collection_rejects_many_regular_candidates_before_opening_them() {
        let temp = tempfile::tempdir().expect("temp");
        let sink = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session".to_string(),
            "recovery".to_string(),
            temp.path().join("project"),
            temp.path().to_path_buf(),
        );
        let input = UserInputCapture {
            kind: UserInputKind::Start,
            thread_id: "root".to_string(),
            client_user_message_id: None,
            text_segments: Vec::new(),
            attachment_candidates: (0..=MAX_RECOVERY_ATTACHMENT_COUNT)
                .map(|index| AttachmentCandidate {
                    kind: AttachmentCandidateKind::LocalImage,
                    source: format!("missing-{index}.png"),
                    detail: None,
                })
                .collect(),
        };

        let error = sink
            .collect_attachments(&input, &[])
            .expect_err("count preflight must run before opening any candidate path");
        assert!(error.contains("count"), "{error}");
    }

    #[test]
    fn host_attachment_collection_rejects_projected_aggregate_before_file_reads() {
        let temp = tempfile::tempdir().expect("temp");
        let first = temp.path().join("first.png");
        let second = temp.path().join("second.png");
        for path in [&first, &second] {
            std::fs::File::create(path)
                .expect("create sparse attachment")
                .set_len(17 * 1024 * 1024)
                .expect("size sparse attachment");
        }
        let sink = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session".to_string(),
            "recovery".to_string(),
            temp.path().join("project"),
            temp.path().to_path_buf(),
        );
        let input = UserInputCapture {
            kind: UserInputKind::Start,
            thread_id: "root".to_string(),
            client_user_message_id: None,
            text_segments: Vec::new(),
            attachment_candidates: [first, second]
                .into_iter()
                .map(|path| AttachmentCandidate {
                    kind: AttachmentCandidateKind::LocalImage,
                    source: path.display().to_string(),
                    detail: None,
                })
                .collect(),
        };

        let error = sink
            .collect_attachments(&input, &[])
            .expect_err("aggregate metadata preflight must reject before sparse files are read");
        assert!(error.contains("aggregate"), "{error}");
    }

    #[test]
    fn prepared_attachment_growth_cannot_cross_the_actual_aggregate_read_budget() {
        use std::io::Write as _;

        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("growing.png");
        std::fs::write(&path, b"a").expect("initial byte");
        let mut budget = RecoveryAttachmentAggregateBudget::with_limits(1, 4, 100);
        budget.reserve_count(1).expect("count");
        let prepared =
            open_prepared_local_attachment(&path, "growing.png".to_string(), &mut budget)
                .expect("metadata preflight");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open growth writer")
            .write_all(b"bcde")
            .expect("grow after metadata preflight");

        let error = read_prepared_local_attachment(prepared, &mut budget)
            .expect_err("the same handle must stop at the late-bound aggregate cap");
        assert!(error.contains("4 byte size limit"), "{error}");
    }

    #[test]
    fn recovery_input_copies_host_and_sidecar_attachments_without_persisting_sources() {
        let temp = tempfile::tempdir().expect("temp");
        let project_dir = temp.path().join("project-state");
        let source_cwd = temp.path().join("worktree");
        std::fs::create_dir_all(source_cwd.join(".gwt/drop-files")).expect("drop dir");
        let source = source_cwd.join(".gwt/drop-files/screenshot.png");
        std::fs::write(&source, b"host-image-bytes").expect("source attachment");
        let store = gwt_core::recovery::RecoveryStore::new(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "recovery-attachment".to_string(),
                    session_id: "session-attachment".to_string(),
                    repo_id: "repo-attachment".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: source_cwd.clone(),
                    launch_base_ref: None,
                    launch_base_oid: "base".to_string(),
                    launch_head_oid: "head".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Discuss screenshot".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-attachment",
            )
            .expect("create recovery");
        store
            .bind_root(
                "recovery-attachment",
                gwt_core::recovery::ProviderRootBinding {
                    root_id: "root-attachment".to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Verified,
                    bound_at: chrono::Utc::now(),
                },
                "bind-attachment",
            )
            .expect("bind root");
        let sink = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session-attachment".to_string(),
            "recovery-attachment".to_string(),
            project_dir,
            source_cwd,
        );
        let input = UserInputCapture {
            kind: UserInputKind::Start,
            thread_id: "root-attachment".to_string(),
            client_user_message_id: Some("turn-host".to_string()),
            text_segments: vec!["See attachment".to_string()],
            attachment_candidates: vec![AttachmentCandidate {
                kind: AttachmentCandidateKind::LocalImage,
                source: source.display().to_string(),
                detail: None,
            }],
        };
        sink.persist_user_input(
            &input,
            r#"{"id":41,"method":"turn/start","params":{"threadId":"root-attachment"}}"#,
        )
        .expect("persist host attachment");

        let transferred_input = UserInputCapture {
            kind: UserInputKind::Start,
            thread_id: "root-attachment".to_string(),
            client_user_message_id: Some("turn-container".to_string()),
            text_segments: vec!["Container image".to_string()],
            attachment_candidates: Vec::new(),
        };
        sink.persist_transferred_user_input(
            &transferred_input,
            &[TransferredAttachment {
                file_name: "container.png".to_string(),
                base64_data: base64::engine::general_purpose::STANDARD
                    .encode(b"container-image-bytes"),
            }],
            r#"{"id":42,"method":"turn/start","params":{"threadId":"root-attachment"}}"#,
        )
        .expect("persist transferred attachment");

        let revision_before_retry = store
            .load("recovery-attachment")
            .expect("load before retry")
            .expect("record before retry")
            .checkpoint_revision;
        sink.persist_user_input(
            &input,
            r#"{ "id":404, "method":"turn/start", "unknownField":true, "params": {"threadId":"root-attachment"} }"#,
        )
        .expect("same Host operation is idempotent");
        sink.persist_transferred_user_input(
            &transferred_input,
            &[TransferredAttachment {
                file_name: "container.png".to_string(),
                base64_data: base64::engine::general_purpose::STANDARD
                    .encode(b"container-image-bytes"),
            }],
            r#"{ "id":405, "method":"turn/start", "future":1, "params": {"threadId":"root-attachment"} }"#,
        )
        .expect("same container operation is idempotent");

        let mut changed_host = input.clone();
        changed_host.text_segments = vec!["Changed under the same client message id".to_string()];
        let host_conflict = sink
            .persist_user_input(&changed_host, r#"{"wire":"host-payload-drift"}"#)
            .expect_err("same Host semantic key with changed payload must conflict");
        assert!(
            host_conflict.contains("different content"),
            "{host_conflict}"
        );
        let container_conflict = sink
            .persist_transferred_user_input(
                &transferred_input,
                &[TransferredAttachment {
                    file_name: "container.png".to_string(),
                    base64_data: base64::engine::general_purpose::STANDARD
                        .encode(b"changed-container-image-bytes"),
                }],
                r#"{"wire":"container-payload-drift"}"#,
            )
            .expect_err("same container semantic key with changed bytes must conflict");
        assert!(
            container_conflict.contains("different content"),
            "{container_conflict}"
        );

        let record = store
            .load("recovery-attachment")
            .expect("load")
            .expect("record");
        assert_eq!(record.checkpoint_revision, revision_before_retry);
        let attachments = &record
            .checkpoint
            .as_ref()
            .expect("checkpoint")
            .attachment_refs;
        assert_eq!(attachments.len(), 2);
        assert_eq!(
            store
                .read_attachment_bytes(
                    &attachments[0],
                    gwt_core::recovery::MAX_RECOVERY_ATTACHMENT_BYTES,
                )
                .expect("read host blob"),
            b"host-image-bytes"
        );
        assert_eq!(
            store
                .read_attachment_bytes(
                    &attachments[1],
                    gwt_core::recovery::MAX_RECOVERY_ATTACHMENT_BYTES,
                )
                .expect("read container blob"),
            b"container-image-bytes"
        );
        let persisted = serde_json::to_string(&record).expect("serialize record");
        assert!(!persisted.contains("drop-files"));
        assert!(!persisted.contains(source.to_string_lossy().as_ref()));
        assert!(!format!("{:?}", input.attachment_candidates[0]).contains("drop-files"));
    }

    #[test]
    fn input_without_client_id_uses_canonical_semantics_across_sidecar_restart() {
        let temp = tempfile::tempdir().expect("temp");
        let project_dir = temp.path().join("project-state");
        let worktree_path = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree_path).expect("worktree");
        let store = gwt_core::recovery::RecoveryStore::new(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "recovery-fingerprint".to_string(),
                    session_id: "session-fingerprint".to_string(),
                    repo_id: "repo-fingerprint".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree_path.clone(),
                    launch_base_ref: None,
                    launch_base_oid: "base".to_string(),
                    launch_head_oid: "head".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "docker".to_string(),
                    initial_prompt: "Fingerprint retry".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-fingerprint",
            )
            .expect("create recovery");
        store
            .bind_root_semantic(
                "recovery-fingerprint",
                "root-fingerprint",
                None,
                gwt_core::recovery::BindingQuality::Verified,
                "bind-fingerprint",
            )
            .expect("bind root");
        let input = UserInputCapture {
            kind: UserInputKind::Steer,
            thread_id: "root-fingerprint".to_string(),
            client_user_message_id: None,
            text_segments: vec!["Keep the attachment and continue".to_string()],
            attachment_candidates: Vec::new(),
        };
        let transferred = vec![TransferredAttachment {
            file_name: "diagram.png".to_string(),
            base64_data: base64::engine::general_purpose::STANDARD
                .encode(b"canonical-container-bytes"),
        }];
        let first = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session-fingerprint".to_string(),
            "recovery-fingerprint".to_string(),
            project_dir.clone(),
            worktree_path.clone(),
        );
        first
            .persist_transferred_user_input(
                &input,
                &transferred,
                r#"{"id":41,"method":"turn/steer","params":{"threadId":"root-fingerprint"}}"#,
            )
            .expect("first container input");
        let before_restart = store
            .load("recovery-fingerprint")
            .expect("load before restart")
            .expect("record before restart");

        let restarted = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session-fingerprint".to_string(),
            "recovery-fingerprint".to_string(),
            project_dir,
            worktree_path,
        );
        restarted
            .persist_transferred_user_input(
                &input,
                &transferred,
                r#"{ "id": 999, "method":"turn/steer", "unknownField":true, "params": {"threadId":"root-fingerprint"} }"#,
            )
            .expect("reserialized retry after sidecar restart");
        let after_restart = store
            .load("recovery-fingerprint")
            .expect("load after restart")
            .expect("record after restart");
        assert_eq!(after_restart.generation, before_restart.generation);
        assert_eq!(
            after_restart.checkpoint_revision,
            before_restart.checkpoint_revision
        );
        assert_eq!(
            after_restart
                .checkpoint
                .as_ref()
                .expect("checkpoint")
                .attachment_refs
                .len(),
            1
        );

        let renamed = vec![TransferredAttachment {
            file_name: "renamed-diagram.png".to_string(),
            base64_data: base64::engine::general_purpose::STANDARD
                .encode(b"canonical-container-bytes"),
        }];
        let conflict = restarted
            .persist_transferred_user_input(
                &input,
                &renamed,
                r#"{"id":1000,"method":"turn/steer","params":{"threadId":"root-fingerprint"}}"#,
            )
            .expect_err("same canonical key with changed attachment metadata must conflict");
        assert!(conflict.contains("different content"), "{conflict}");
    }

    #[test]
    fn visible_discussion_retry_does_not_duplicate_checkpoint_items() {
        let temp = tempfile::tempdir().expect("temp");
        let project_dir = temp.path().join("project-state");
        let worktree_path = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree_path).expect("worktree");
        let store = gwt_core::recovery::RecoveryStore::new(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "recovery-visible".to_string(),
                    session_id: "session-visible".to_string(),
                    repo_id: "repo-visible".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree_path.clone(),
                    launch_base_ref: None,
                    launch_base_oid: "base".to_string(),
                    launch_head_oid: "head".to_string(),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Discuss recovery".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-visible",
            )
            .expect("create recovery");
        store
            .bind_root(
                "recovery-visible",
                gwt_core::recovery::ProviderRootBinding {
                    root_id: "root-visible".to_string(),
                    session_tree_id: None,
                    quality: gwt_core::recovery::BindingQuality::Verified,
                    bound_at: chrono::Utc::now(),
                },
                "bind-visible",
            )
            .expect("bind root");
        let sink = RecoveryCodexDurability::new(
            temp.path().join("sessions"),
            "session-visible".to_string(),
            "recovery-visible".to_string(),
            project_dir,
            worktree_path,
        );
        let capture = VisibleDiscussionCapture {
            thread_id: "root-visible".to_string(),
            turn_id: "turn-visible".to_string(),
            item_id: "item-visible".to_string(),
            items: vec![VisibleDiscussionItemCapture {
                role: VisibleDiscussionRole::Assistant,
                kind: VisibleDiscussionKind::AssistantMessage,
                text: "A durable milestone".to_string(),
            }],
        };
        let revision_before = store
            .load("recovery-visible")
            .expect("load before")
            .expect("record before")
            .checkpoint_revision;

        sink.persist_visible_discussion(&capture, r#"{"wire":"visible"}"#)
            .expect("persist visible discussion");
        sink.persist_visible_discussion(&capture, r#"{"wire":"visible-reformatted"}"#)
            .expect("retry visible discussion");

        let mut changed = capture.clone();
        changed.items[0].text = "Changed milestone under the same item id".to_string();
        let conflict = sink
            .persist_visible_discussion(&changed, r#"{"wire":"visible-payload-drift"}"#)
            .expect_err("same visible semantic key with changed payload must conflict");
        assert!(conflict.contains("different content"), "{conflict}");

        let record = store
            .load("recovery-visible")
            .expect("load after")
            .expect("record after");
        assert_eq!(record.checkpoint_revision, revision_before + 1);
        let visible_items = &record.checkpoint.expect("checkpoint").visible_items;
        assert_eq!(visible_items.len(), 1);
        assert_eq!(visible_items[0].text, "A durable milestone");
    }

    #[test]
    fn semantic_root_retry_conflicts_on_drift_and_startup_repairs_session_metadata() {
        let temp = tempfile::tempdir().expect("temp");
        let project_dir = temp.path().join("project-state");
        let sessions_dir = temp.path().join("sessions");
        let worktree_path = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree_path).expect("worktree");

        let mut session =
            gwt_agent::Session::new(&worktree_path, "intake/recovery", gwt_agent::AgentId::Codex);
        session.id = "session-root".to_string();
        session.repo_hash = Some("repo-root".to_string());
        session.recovery_id = Some("recovery-root".to_string());
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.save(&sessions_dir).expect("save Session");

        let store = gwt_core::recovery::RecoveryStore::new(&project_dir);
        store
            .create(
                gwt_core::recovery::CreateRecovery {
                    recovery_id: "recovery-root".to_string(),
                    session_id: session.id.clone(),
                    repo_id: "repo-root".to_string(),
                    session_kind: gwt_core::recovery::RecoverySessionKind::Intake,
                    worktree_path: worktree_path.clone(),
                    launch_base_ref: None,
                    launch_base_oid: "base".to_string(),
                    launch_head_oid: "head".to_string(),
                    provider: session.agent_id.to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Recover root".to_string(),
                    created_at: chrono::Utc::now(),
                },
                "create-root",
            )
            .expect("create recovery");
        let sink = RecoveryCodexDurability::new(
            sessions_dir.clone(),
            session.id.clone(),
            "recovery-root".to_string(),
            project_dir,
            worktree_path,
        );
        let binding = RootThreadBinding {
            thread_id: "provider-root".to_string(),
            session_id: "provider-tree".to_string(),
            cli_version: "0.144.5".to_string(),
            forked_from_id: None,
            operation: ThreadOperation::Start,
        };

        sink.persist_root_binding(&binding, r#"{"id":1,"result":"root"}"#)
            .expect("persist root");
        let generation = store
            .load("recovery-root")
            .expect("load")
            .expect("record")
            .generation;
        sink.persist_root_binding(&binding, r#"{ "id":999, "result":"root" }"#)
            .expect("semantic root retry");
        assert_eq!(
            store
                .load("recovery-root")
                .expect("load retry")
                .expect("record retry")
                .generation,
            generation
        );
        let persisted = gwt_agent::Session::load(&sessions_dir.join("session-root.toml"))
            .expect("load persisted Session");
        assert_eq!(persisted.session_history.len(), 1);

        let mut changed = binding.clone();
        changed.session_id = "different-provider-tree".to_string();
        let conflict = sink
            .persist_root_binding(&changed, r#"{"id":1000,"result":"root"}"#)
            .expect_err("same root semantic key with changed binding must conflict");
        assert!(conflict.contains("different content"), "{conflict}");

        gwt_agent::update_session(&sessions_dir, "session-root", |current| {
            current.agent_session_id = None;
            current.session_history.clear();
            current.provider_root_role = None;
            current.provider_binding_quality = None;
            current.recovery_launch_stage = None;
            Ok(())
        })
        .expect("simulate crash before Session metadata commit");

        let report = gwt_agent::import_legacy_recovery_sessions(&sessions_dir, &store, "repo-root");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let repaired = gwt_agent::Session::load(&sessions_dir.join("session-root.toml"))
            .expect("load repaired Session");
        assert_eq!(repaired.agent_session_id.as_deref(), Some("provider-root"));
        assert_eq!(repaired.session_history.len(), 1);
        assert_eq!(
            repaired.session_history[0].agent_session_id,
            "provider-root"
        );
        assert_eq!(
            repaired.provider_binding_quality,
            Some(gwt_agent::session::ProviderBindingQuality::Verified)
        );
    }

    #[test]
    fn registered_route_uses_loopback_endpoint_and_redacts_capability() {
        let sink: std::sync::Arc<dyn CodexDurabilitySink> =
            std::sync::Arc::new(RecordingDurabilitySink::default());
        let mut app_server_env = HashMap::new();
        app_server_env.insert(
            CODEX_REMOTE_AUTH_TOKEN_ENV.to_string(),
            "outer-route-capability".to_string(),
        );
        app_server_env.insert("GWT_SAFE_TEST_VALUE".to_string(), "retained".to_string());
        let lease = register_codex_bridge_route(CodexBridgeRouteConfig {
            app_server: CodexAppServerLaunch {
                command: "codex".to_string(),
                args: vec![
                    "app-server".to_string(),
                    "--listen".to_string(),
                    "stdio://".to_string(),
                ],
                env: app_server_env,
                remove_env: Vec::new(),
                cwd: None,
            },
            expected_resume_id: None,
            durability: sink,
            on_ready: std::sync::Arc::new(|| {}),
            on_failure: std::sync::Arc::new(|_| {}),
        })
        .expect("register route");

        let endpoint = lease.endpoint();
        assert!(endpoint.starts_with("ws://127.0.0.1:"));
        assert!(!endpoint.contains(lease.auth_token()));
        assert!(!format!("{lease:?}").contains(lease.auth_token()));
        assert!(!lease
            .inner
            .route
            .app_server
            .env
            .contains_key(CODEX_REMOTE_AUTH_TOKEN_ENV));
        assert_eq!(
            lease
                .inner
                .route
                .app_server
                .env
                .get("GWT_SAFE_TEST_VALUE")
                .map(String::as_str),
            Some("retained")
        );
        assert!(lease
            .inner
            .route
            .app_server
            .remove_env
            .iter()
            .any(|key| key == CODEX_REMOTE_AUTH_TOKEN_ENV));
    }

    #[tokio::test]
    async fn app_server_spawn_scrubs_a_token_already_present_on_the_command() {
        #[cfg(windows)]
        let (program, args) = (
            "cmd",
            vec![
                "/D".to_string(),
                "/S".to_string(),
                "/C".to_string(),
                format!("if defined {CODEX_REMOTE_AUTH_TOKEN_ENV} (exit /b 9) else (exit /b 0)"),
            ],
        );
        #[cfg(unix)]
        let (program, args) = (
            "sh",
            vec![
                "-c".to_string(),
                format!("test -z \"${{{CODEX_REMOTE_AUTH_TOKEN_ENV}+x}}\""),
            ],
        );
        let launch = CodexAppServerLaunch {
            command: program.to_string(),
            args,
            env: HashMap::new(),
            remove_env: vec![CODEX_REMOTE_AUTH_TOKEN_ENV.to_string()],
            cwd: None,
        };
        let mut command = tokio::process::Command::new(&launch.command);
        // Seed the child command as though gwt itself inherited an outer
        // launch capability. The shared spawn configuration must remove it.
        command.env(CODEX_REMOTE_AUTH_TOKEN_ENV, "outer-route-capability");
        configure_app_server_command(&mut command, &launch);

        let status = command.status().await.expect("spawn token sentinel");

        assert!(
            status.success(),
            "app-server child observed the outer token"
        );
    }
}

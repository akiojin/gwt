//! Loopback private GUI control is separate from the capability agent listener.
use super::{
    events::{OwnerEvent, OwnerRequest},
    owner::{OwnerHandle, OwnerState},
};
use crate::{
    agent_capability::{constant_time_token_eq, AgentCapabilityIssuer, AgentCapabilityRegistry},
    project_transport::{
        agent_bridge_bind_ip, agent_router, AccessLogSink, ClientHub, TransportState,
    },
};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
};

pub(crate) struct BoundProjectTransport {
    private_listener: TcpListener,
    agent_listener: TcpListener,
    pub(crate) private_url: String,
    pub(crate) owner: OwnerHandle,
    state: PrivateState,
    agent_state: TransportState,
}
#[derive(Clone)]
struct PrivateState {
    token: String,
    owner: OwnerHandle,
}
impl BoundProjectTransport {
    pub(crate) fn bind(endpoint: &gwt_core::daemon::DaemonEndpoint) -> std::io::Result<Self> {
        let private_listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let agent_listener = TcpListener::bind(SocketAddr::new(agent_bridge_bind_ip(), 0))?;
        private_listener.set_nonblocking(true)?;
        agent_listener.set_nonblocking(true)?;
        let private_url = format!("http://{}", private_listener.local_addr()?);
        let agent_port = agent_listener.local_addr()?.port();
        let capabilities = AgentCapabilityRegistry::default();
        let clients = ClientHub::default();
        let issuer = AgentCapabilityIssuer::new(
            format!("http://127.0.0.1:{agent_port}/internal/hook-live"),
            format!("ws://127.0.0.1:{agent_port}/internal/pane-ws"),
            format!("ws://127.0.0.1:{agent_port}/internal/pane-ws"),
            capabilities.clone(),
        );
        crate::cli::host_contract::mark_host_process();
        let host_instance_id = uuid::Uuid::new_v4().to_string();
        let owner = OwnerState::start(
            endpoint.scope.project_root.clone(),
            endpoint.scope.repo_hash.clone(),
            clients.clone(),
            issuer,
            host_instance_id.clone(),
            endpoint.startup_intent,
        );
        let sink = owner.clone();
        let agent_state = TransportState::new(
            clients,
            capabilities,
            host_instance_id,
            Arc::new(move |event| sink.transport_event(event)),
        );
        Ok(Self {
            private_listener,
            agent_listener,
            private_url,
            state: PrivateState {
                token: endpoint.auth_token.clone(),
                owner: owner.clone(),
            },
            owner,
            agent_state,
        })
    }
    pub(crate) fn serve(self) -> std::io::Result<ProjectTransportTask> {
        let private = tokio::net::TcpListener::from_std(self.private_listener)?;
        let agent = tokio::net::TcpListener::from_std(self.agent_listener)?;
        let router = Router::new()
            .route("/private/project/command", post(command))
            .route("/private/project/events", get(events))
            .with_state(self.state);
        let private_task = tokio::spawn(async move {
            if let Err(error) = axum::serve(private, router).await {
                tracing::error!(%error, "private project transport failed");
            }
        });
        let router = agent_router(self.agent_state, AccessLogSink::default());
        let agent_task = tokio::spawn(async move {
            if let Err(error) = axum::serve(
                agent,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            {
                tracing::error!(%error, "project agent transport failed");
            }
        });
        Ok(ProjectTransportTask {
            private_task,
            agent_task,
            owner: self.owner,
        })
    }
}
pub(crate) struct ProjectTransportTask {
    private_task: tokio::task::JoinHandle<()>,
    agent_task: tokio::task::JoinHandle<()>,
    owner: OwnerHandle,
}
impl ProjectTransportTask {
    pub(crate) fn residency(
        &self,
    ) -> tokio::sync::watch::Receiver<super::presence::ResidencySnapshot> {
        self.owner.residency()
    }
    pub(crate) async fn shutdown(self) {
        // Keep authenticated exact cleanup retries reachable while draining.
        self.owner.shutdown().await;
        self.private_task.abort();
        self.agent_task.abort();
    }
}
fn authorized(headers: &HeaderMap, state: &PrivateState) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| constant_time_token_eq(token, &state.token))
}
async fn command(
    State(state): State<PrivateState>,
    headers: HeaderMap,
    Json(request): Json<OwnerRequest>,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.owner.request(request).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":error})),
        )
            .into_response(),
    }
}
struct GuiPresenceLease {
    owner: OwnerHandle,
    id: u64,
}
impl Drop for GuiPresenceLease {
    fn drop(&mut self) {
        self.owner.detach_gui(self.id);
    }
}

async fn events(
    State(state): State<PrivateState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if !authorized(&headers, &state) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Subscribe before publishing presence: no event may fall into an attach gap.
    let receiver = state.owner.subscribe();
    let presence = if headers
        .get("x-gwt-gui-presence")
        .is_some_and(|value| value == "1")
    {
        match state.owner.attach_gui().await {
            Ok(id) => Some(GuiPresenceLease {
                owner: state.owner.clone(),
                id,
            }),
            Err(error) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error":error})),
                )
                    .into_response()
            }
        }
    } else {
        None
    };
    websocket
        .on_upgrade(move |socket| event_socket(socket, receiver, presence))
        .into_response()
}
async fn event_socket(
    socket: WebSocket,
    mut receiver: tokio::sync::broadcast::Receiver<OwnerEvent>,
    _presence: Option<GuiPresenceLease>,
) {
    use futures_util::{SinkExt, StreamExt};
    let (mut writer, mut reader) = socket.split();
    loop {
        tokio::select! {
            message = reader.next() => match message {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                _ => {},
            },
            event = receiver.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => OwnerEvent::Invalidated,
                    Err(_) => return,
                };
                let send = writer.send(Message::Text(serde_json::to_string(&event).expect("private event serializes").into()));
                futures_util::pin_mut!(send);
                loop {
                    tokio::select! {
                        result = &mut send => { if result.is_err() { return; } break; },
                        message = reader.next() => match message {
                            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                            _ => {},
                        },
                    }
                }
            }
        }
    }
}

use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    time::Instant,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        HeaderMap, StatusCode,
    },
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use gwt::{FrontendEvent, HookForwardTarget, RuntimeHookEvent};
use gwt_terminal::PtyHandle;
use tokio::{
    net::TcpListener,
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use uuid::Uuid;

use crate::{embedded_web, AppEventProxy, DispatchTarget, OutboundEvent, UserEvent};

type PtyWriterRegistry = Arc<RwLock<HashMap<String, Arc<PtyHandle>>>>;
const CLIENT_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Default)]
pub(super) struct ClientHub {
    clients: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
}

impl ClientHub {
    pub(super) fn register(&self, client_id: String) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(CLIENT_QUEUE_CAPACITY);
        self.clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(client_id, tx);
        rx
    }

    pub(super) fn unregister(&self, client_id: &str) {
        self.clients
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(client_id);
    }

    pub(super) fn dispatch(&self, events: Vec<OutboundEvent>) {
        let mut clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());
        let mut stale_clients = Vec::new();
        for outbound in events {
            let payload = serde_json::to_string(&outbound.event).expect("backend event json");
            match outbound.target {
                DispatchTarget::Broadcast => {
                    for (client_id, sender) in clients.iter() {
                        if sender.try_send(payload.clone()).is_err() {
                            stale_clients.push(client_id.clone());
                        }
                    }
                }
                DispatchTarget::Client(client_id) => {
                    if let Some(sender) = clients.get(&client_id) {
                        if sender.try_send(payload).is_err() {
                            stale_clients.push(client_id);
                        }
                    }
                }
            }
        }
        for client_id in stale_clients {
            clients.remove(&client_id);
        }
    }
}

#[derive(Clone)]
struct ServerState {
    proxy: AppEventProxy,
    clients: ClientHub,
    hook_forward_token: String,
    pty_writers: PtyWriterRegistry,
}

pub(super) struct EmbeddedServer {
    url: String,
    hook_forward_token: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl EmbeddedServer {
    pub(super) fn start(
        runtime: &Runtime,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(("127.0.0.1", 0)))?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let hook_forward_token = Uuid::new_v4().to_string();

        let app = Router::new()
            .route("/", get(embedded_web::index_handler))
            .route("/app.js", get(embedded_web::app_js_handler))
            .route(
                "/branch-cleanup-modal.js",
                get(embedded_web::branch_cleanup_modal_js_handler),
            )
            .route(
                "/assets/xterm/xterm.mjs",
                get(embedded_web::xterm_js_handler),
            )
            .route(
                "/assets/xterm/addon-fit.mjs",
                get(embedded_web::xterm_fit_js_handler),
            )
            .route(
                "/assets/xterm/xterm.css",
                get(embedded_web::xterm_css_handler),
            )
            .route("/healthz", get(health_handler))
            .route("/internal/hook-live", post(hook_live_handler))
            .route("/ws", get(websocket_handler))
            .with_state(ServerState {
                proxy,
                clients,
                hook_forward_token: hook_forward_token.clone(),
                pty_writers,
            });

        runtime.spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded server error: {error}");
            }
        });

        Ok(Self {
            url: format!("http://127.0.0.1:{}/", addr.port()),
            hook_forward_token,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub(super) fn url(&self) -> &str {
        &self.url
    }

    pub(super) fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    pub(super) fn hook_forward_target(&self) -> HookForwardTarget {
        HookForwardTarget {
            url: format!("{}internal/hook-live", self.url),
            token: self.hook_forward_token.clone(),
        }
    }
}

pub(super) async fn health_handler() -> &'static str {
    "ok"
}

async fn websocket_handler(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    if !websocket_origin_authorized(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| client_session(socket, state))
}

async fn hook_live_handler(
    headers: HeaderMap,
    State(state): State<ServerState>,
    Json(event): Json<RuntimeHookEvent>,
) -> StatusCode {
    if !hook_forward_authorized(&headers, &state.hook_forward_token) {
        return StatusCode::UNAUTHORIZED;
    }

    state.proxy.send(UserEvent::RuntimeHook(event));
    StatusCode::NO_CONTENT
}

async fn client_session(socket: WebSocket, state: ServerState) {
    let client_id = Uuid::new_v4().to_string();
    let mut outbound = state.clients.register(client_id.clone());
    let (mut sender, mut receiver) = socket.split();

    let input_seq = Arc::new(AtomicU64::new(0));

    loop {
        tokio::select! {
            maybe_payload = outbound.recv() => {
                let Some(payload) = maybe_payload else {
                    break;
                };
                if sender.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            maybe_message = receiver.next() => {
                match maybe_message {
                    Some(Ok(Message::Text(text))) => {
                        let text_len = text.len();
                        match serde_json::from_str::<FrontendEvent>(text.as_ref()) {
                            Ok(event) => {
                                handle_frontend_message(
                                    &state,
                                    &client_id,
                                    &input_seq,
                                    text_len,
                                    event,
                                );
                            }
                            Err(error) => {
                                eprintln!("invalid frontend message: {error}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        eprintln!("websocket error: {error}");
                        break;
                    }
                }
            }
        }
    }

    state.clients.unregister(&client_id);
}

fn handle_frontend_message(
    state: &ServerState,
    client_id: &str,
    input_seq: &AtomicU64,
    text_len: usize,
    event: FrontendEvent,
) {
    let (id, data) = match event {
        FrontendEvent::TerminalInput { id, data } => (id, data),
        other => {
            state.proxy.send(UserEvent::Frontend {
                client_id: client_id.to_string(),
                event: other,
            });
            return;
        }
    };

    let seq = input_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let data_len = data.len();
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_recv",
        client_id = %client_id,
        seq,
        window_id = %id,
        data_len,
        text_len,
        "terminal_input received over WebSocket"
    );

    let pty_handle = match state.pty_writers.read() {
        Ok(guard) => guard.get(&id).cloned(),
        Err(error) => {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "fast_path_lock_poisoned",
                client_id = %client_id,
                seq,
                window_id = %id,
                error = %error,
                "pty_writers read lock poisoned; falling back to event loop"
            );
            None
        }
    };

    if let Some(pty) = pty_handle {
        let write_started = Instant::now();
        match pty.write_input(data.as_bytes()) {
            Ok(()) => {
                tracing::debug!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    data_len,
                    write_us = write_started.elapsed().as_micros() as u64,
                    "terminal_input written to PTY via WS fast-path"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write_err",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    data_len,
                    error = %error,
                    "fast-path PTY write failed; forwarding to event loop for error handling"
                );
            }
        }
    } else {
        tracing::debug!(
            target: "gwt_input_trace",
            stage = "fast_path_miss",
            client_id = %client_id,
            seq,
            window_id = %id,
            data_len,
            "pty_writers registry miss; falling back to event loop"
        );
    }

    state.proxy.send(UserEvent::Frontend {
        client_id: client_id.to_string(),
        event: FrontendEvent::TerminalInput {
            id: id.clone(),
            data,
        },
    });
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_dispatch",
        client_id = %client_id,
        seq,
        window_id = %id,
        data_len,
        ok = true,
        "terminal_input forwarded to event loop proxy (fallback)"
    );
}

pub(super) fn hook_forward_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected_token)
}

pub(super) fn websocket_origin_authorized(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    let Some(host) = headers.get(HOST) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(host) = host.to_str() else {
        return false;
    };

    let origin = origin.trim_end_matches('/');
    origin == format!("http://{host}") || origin == format!("https://{host}")
}

#[cfg(test)]
pub(super) fn broadcast_runtime_hook_event(clients: &ClientHub, event: RuntimeHookEvent) {
    clients.dispatch(vec![OutboundEvent::broadcast(
        gwt::BackendEvent::RuntimeHookEvent { event },
    )]);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    };

    use axum::http::{
        header::{HOST, ORIGIN},
        HeaderMap,
    };
    use gwt::{BackendEvent, FrontendEvent, RuntimeHookEvent, RuntimeHookEventKind};
    use reqwest::StatusCode as HttpStatusCode;
    use tokio::runtime::Runtime;

    use crate::{AppEventProxy, OutboundEvent, UserEvent};

    use super::{
        handle_frontend_message, websocket_origin_authorized, ClientHub, EmbeddedServer,
        ServerState, CLIENT_QUEUE_CAPACITY,
    };

    fn sample_server_state() -> (ServerState, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, events) = AppEventProxy::stub();
        (
            ServerState {
                proxy,
                clients: ClientHub::default(),
                hook_forward_token: "token".to_string(),
                pty_writers: Arc::new(RwLock::new(HashMap::new())),
            },
            events,
        )
    }

    #[test]
    fn handle_frontend_message_forwards_non_terminal_events_to_proxy() {
        let (state, events) = sample_server_state();

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            32,
            FrontendEvent::FrontendReady,
        );

        let recorded = events.lock().unwrap_or_else(|p| p.into_inner());
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, event: FrontendEvent::FrontendReady }]
                if client_id == "client-1"
        ));
    }

    #[test]
    fn handle_frontend_message_falls_back_to_proxy_when_pty_writer_is_missing() {
        let (state, events) = sample_server_state();

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            48,
            FrontendEvent::TerminalInput {
                id: "tab-1::shell-1".to_string(),
                data: "ls\n".to_string(),
            },
        );

        let recorded = events.lock().unwrap_or_else(|p| p.into_inner());
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, event: FrontendEvent::TerminalInput { id, data } }]
                if client_id == "client-1"
                    && id == "tab-1::shell-1"
                    && data == "ls\n"
        ));
    }

    #[test]
    fn client_hub_drops_lagging_client_when_bounded_queue_is_full() {
        let hub = ClientHub::default();
        let _receiver = hub.register("slow-client".to_string());

        for index in 0..=CLIENT_QUEUE_CAPACITY {
            hub.dispatch(vec![OutboundEvent::broadcast(
                BackendEvent::ProjectOpenError {
                    message: format!("message-{index}"),
                },
            )]);
        }

        let clients = hub.clients.lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            !clients.contains_key("slow-client"),
            "lagging websocket client should be unregistered once its queue is full"
        );
    }

    #[test]
    fn websocket_origin_authorized_requires_same_host_when_origin_is_present() {
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "127.0.0.1:3000".parse().expect("host header"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "http://127.0.0.1:3000".parse().expect("origin"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "https://127.0.0.1:3000".parse().expect("origin"));
        assert!(websocket_origin_authorized(&headers));

        headers.insert(ORIGIN, "http://evil.example:3000".parse().expect("origin"));
        assert!(!websocket_origin_authorized(&headers));
    }

    #[test]
    fn embedded_server_exposes_health_and_authenticated_hook_live_routes() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(&runtime, proxy, clients.clone(), pty_writers)
            .expect("embedded server");
        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();

        assert_eq!(hook.url, format!("{}internal/hook-live", server.url()));

        let health = client
            .get(format!("{}healthz", server.url()))
            .send()
            .expect("health request");
        assert_eq!(health.status(), HttpStatusCode::OK);
        assert_eq!(health.text().expect("health body"), "ok");

        let app_js = client
            .get(format!("{}app.js", server.url()))
            .send()
            .expect("app.js request");
        assert_eq!(app_js.status(), HttpStatusCode::OK);
        let content_type = app_js
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .expect("app.js content type");
        assert_eq!(content_type, "text/javascript; charset=utf-8");
        assert!(
            app_js
                .text()
                .expect("app.js body")
                .contains("function websocketUrl()"),
            "expected embedded server to serve the shared frontend bundle script",
        );

        let xterm_js = client
            .get(format!("{}assets/xterm/xterm.mjs", server.url()))
            .send()
            .expect("xterm module request");
        assert_eq!(xterm_js.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            xterm_js
                .text()
                .expect("xterm module body")
                .contains("Terminal"),
            "expected embedded server to serve pinned xterm module asset",
        );

        let xterm_fit_js = client
            .get(format!("{}assets/xterm/addon-fit.mjs", server.url()))
            .send()
            .expect("xterm fit module request");
        assert_eq!(xterm_fit_js.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_fit_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            xterm_fit_js
                .text()
                .expect("xterm fit module body")
                .contains("FitAddon"),
            "expected embedded server to serve pinned xterm fit addon asset",
        );

        let xterm_css = client
            .get(format!("{}assets/xterm/xterm.css", server.url()))
            .send()
            .expect("xterm css request");
        assert_eq!(xterm_css.status(), HttpStatusCode::OK);
        assert_eq!(
            xterm_css
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/css; charset=utf-8")
        );
        assert!(
            xterm_css.text().expect("xterm css body").contains(".xterm"),
            "expected embedded server to serve pinned xterm stylesheet asset",
        );

        let event = RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("PreToolUse".to_string()),
            gwt_session_id: Some("session-1".to_string()),
            agent_session_id: Some("agent-1".to_string()),
            project_root: Some("E:/gwt/test-repo".to_string()),
            branch: Some("feature/runtime".to_string()),
            status: Some("Running".to_string()),
            tool_name: Some("Bash".to_string()),
            message: None,
            occurred_at: "2026-04-21T00:00:00Z".to_string(),
        };

        let unauthorized = client
            .post(&hook.url)
            .json(&event)
            .send()
            .expect("unauthorized hook request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let wrong_token = client
            .post(&hook.url)
            .bearer_auth("wrong-token")
            .json(&event)
            .send()
            .expect("wrong token hook request");
        assert_eq!(wrong_token.status(), HttpStatusCode::UNAUTHORIZED);

        let accepted = client
            .post(&hook.url)
            .bearer_auth(&hook.token)
            .json(&event)
            .send()
            .expect("authorized hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let recorded = events.lock().unwrap_or_else(|p| p.into_inner());
        assert!(recorded.iter().any(|user_event| {
            matches!(
                user_event,
                UserEvent::RuntimeHook(recorded_event)
                    if recorded_event.kind == RuntimeHookEventKind::RuntimeState
                        && recorded_event.source_event.as_deref() == Some("PreToolUse")
                        && recorded_event.agent_session_id.as_deref() == Some("agent-1")
            )
        }));

        server.shutdown();
    }
}

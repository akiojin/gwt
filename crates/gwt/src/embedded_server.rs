use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
    time::Instant,
};

use axum::{
    extract::{
        connect_info::ConnectInfo,
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, Request, State,
    },
    http::{
        header::{AUTHORIZATION, HOST, ORIGIN, USER_AGENT},
        HeaderMap, StatusCode,
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use gwt::{FrontendEvent, HookForwardTarget, RuntimeHookEvent};
use gwt_terminal::PtyHandle;
use serde::{Deserialize, Serialize};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    runtime::Runtime,
    sync::{mpsc, oneshot},
};
use uuid::Uuid;

use crate::{
    embedded_web, AppEventProxy, AttachmentUploadStore, DispatchTarget, OutboundEvent,
    UploadedAttachment, UserEvent,
};

type PtyWriterRegistry = Arc<RwLock<HashMap<String, Arc<PtyHandle>>>>;
const CLIENT_QUEUE_CAPACITY: usize = 64;
/// Upper bound on the in-memory access log ring buffer. The canonical sink
/// for production is `tracing::info!(target: "gwt_access", ...)` which writes
/// to `~/.gwt/logs/<date>/`; this in-memory ring exists only so tests (and an
/// eventual operator-visible Live tab) can sample the most recent entries
/// without parsing log files. Older entries are evicted FIFO once the ring
/// reaches the cap. SPEC-1942 US-14 follow-up review: previous unbounded Vec
/// would grow without limit in long-running browser-server sessions.
const ACCESS_LOG_RING_CAPACITY: usize = 1024;

/// One captured HTTP / WebSocket access event. Emitted both as
/// `tracing::info!(target: "gwt_access", ...)` (or `debug!` for `/healthz`)
/// and into an in-memory [`AccessLogSink`] for test inspection.
///
/// SPEC-1942 FR-098: visibility for LAN-bound browser-server mode — operators need to see
/// where access comes from when running with `--bind` on a LAN-reachable
/// address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessLogRecord {
    pub method: String,
    pub path: String,
    pub status: u16,
    pub peer: Option<String>,
    pub user_agent: Option<String>,
    pub elapsed_ms: u64,
}

/// In-memory ring of access log entries. Cloning yields a handle to the same
/// underlying buffer (Arc-wrapped) so the embedded server, middleware and
/// tests observe the same recordings. The ring is capped at
/// [`ACCESS_LOG_RING_CAPACITY`] entries; older records are evicted FIFO so
/// memory stays bounded under long-running browser-server sessions.
#[derive(Clone, Default)]
pub struct AccessLogSink {
    inner: Arc<Mutex<std::collections::VecDeque<AccessLogRecord>>>,
}

impl AccessLogSink {
    pub(crate) fn record(&self, rec: AccessLogRecord) {
        if let Ok(mut guard) = self.inner.lock() {
            if guard.len() == ACCESS_LOG_RING_CAPACITY {
                guard.pop_front();
            }
            guard.push_back(rec);
        }
    }

    /// Returns a snapshot copy of every recorded entry so callers do not have
    /// to hold the underlying mutex.
    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<AccessLogRecord> {
        self.inner
            .lock()
            .map(|guard| guard.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub struct ClientHub {
    clients: Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>,
}

impl ClientHub {
    pub(super) fn register(&self, client_id: String) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(CLIENT_QUEUE_CAPACITY);
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(client_id, tx);
        rx
    }

    pub(super) fn unregister(&self, client_id: &str) {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(client_id);
    }

    pub(super) fn dispatch(&self, events: Vec<OutboundEvent>) {
        // Snapshot sender clones under a short-lived lock so that serialization
        // and per-client try_send work happen outside the registry mutex. This
        // keeps register/unregister responsive even when the broadcast batch is
        // large or one client is slow to drain its queue.
        let snapshot: Vec<(String, mpsc::Sender<String>)> = {
            let clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            clients
                .iter()
                .map(|(id, sender)| (id.clone(), sender.clone()))
                .collect()
        };

        let mut stale_clients: Vec<String> = Vec::new();
        for outbound in events {
            let payload = serde_json::to_string(&outbound.event).expect("backend event json");
            match outbound.target {
                DispatchTarget::Broadcast => {
                    for (client_id, sender) in &snapshot {
                        if sender.try_send(payload.clone()).is_err() {
                            stale_clients.push(client_id.clone());
                        }
                    }
                }
                DispatchTarget::Client(client_id) => {
                    if let Some((_, sender)) = snapshot.iter().find(|(id, _)| id == &client_id) {
                        if sender.try_send(payload).is_err() {
                            stale_clients.push(client_id);
                        }
                    }
                }
            }
        }

        if !stale_clients.is_empty() {
            let mut clients = self
                .clients
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for client_id in stale_clients {
                clients.remove(&client_id);
            }
        }
    }
}

#[derive(Clone)]
struct ServerState {
    proxy: AppEventProxy,
    clients: ClientHub,
    hook_forward_token: String,
    attachment_upload_token: String,
    attachment_uploads: AttachmentUploadStore,
    pty_writers: PtyWriterRegistry,
    // Held only so the in-process sink stays alive for the lifetime of the
    // server. Read directly through [`EmbeddedServer::access_log`] in tests.
    #[allow(dead_code)]
    access_log: AccessLogSink,
}

pub struct EmbeddedServer {
    url: String,
    hook_forward_token: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    // Same rationale as `ServerState::access_log`: tests read it via the
    // `access_log()` accessor; production code (main bootstrap) does not yet
    // surface the sink to the UI.
    #[allow(dead_code)]
    access_log: AccessLogSink,
}

impl EmbeddedServer {
    /// Loopback (`127.0.0.1`) on an ephemeral port — the original GUI default.
    /// Kept as a thin shim so non-headless callers do not have to know about
    /// the bind/port surface introduced for SPEC-1942 US-14.
    #[cfg(test)]
    pub(super) fn start(
        runtime: &Runtime,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        Self::start_with_bind(
            runtime,
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
        )
    }

    /// SPEC-1942 FR-095 / FR-098: bind the embedded server to a caller-chosen
    /// IP / port and install the access-log middleware. Used by the current
    /// browser-server route for both loopback defaults and operator-chosen
    /// `--bind` / `--port`.
    pub(super) fn start_with_bind(
        runtime: &Runtime,
        bind: IpAddr,
        port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind((bind, port)))?;
        let addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let hook_forward_token = Uuid::new_v4().to_string();
        let attachment_upload_token = Uuid::new_v4().to_string();
        let access_log = AccessLogSink::default();

        let app = route_root_js_modules(
            Router::new()
                .route("/", get(embedded_web::index_handler))
                .route("/app.js", get(embedded_web::app_js_handler)),
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
        // SPEC-2009 Phase 2b — highlight.js vendored module + dark theme
        // for the File Tree text viewer syntax highlighting overlay.
        .route(
            "/assets/highlight/highlight.min.js",
            get(embedded_web::highlight_js_handler),
        )
        .route(
            "/assets/highlight/github-dark.min.css",
            get(embedded_web::highlight_css_handler),
        )
        // SPEC-2356 Operator Design System — styles + fonts.
        .route(
            "/styles/tokens.css",
            get(embedded_web::styles_tokens_css_handler),
        )
        .route(
            "/styles/typography.css",
            get(embedded_web::styles_typography_css_handler),
        )
        .route(
            "/styles/components.css",
            get(embedded_web::styles_components_css_handler),
        )
        .route("/styles/app.css", get(embedded_web::styles_app_css_handler))
        .route(
            "/assets/fonts/MonaSans.woff2",
            get(embedded_web::font_mona_sans_handler),
        )
        .route(
            "/assets/fonts/HubotSans-Regular.woff2",
            get(embedded_web::font_hubot_regular_handler),
        )
        .route(
            "/assets/fonts/HubotSans-Bold.woff2",
            get(embedded_web::font_hubot_bold_handler),
        )
        .route(
            "/assets/fonts/HubotSansCondensed-Bold.woff2",
            get(embedded_web::font_hubot_condensed_bold_handler),
        )
        .route(
            "/assets/fonts/JetBrainsMono.woff2",
            get(embedded_web::font_jetbrains_mono_handler),
        )
        .route("/healthz", get(health_handler))
        // SPEC-2963 Phase 5: OAuth redirect target for remote Board provider
        // sign-in. Completes the flow against the process-global session store.
        .route("/oauth/callback", get(oauth_callback_handler))
        .route(
            "/internal/attachment-upload-token",
            get(attachment_upload_token_handler),
        )
        .route(
            "/internal/attachments/upload",
            post(attachment_upload_handler),
        )
        .route("/internal/hook-live", post(hook_live_handler))
        .route("/ws", get(websocket_handler))
        .with_state(ServerState {
            proxy,
            clients,
            hook_forward_token: hook_forward_token.clone(),
            attachment_upload_token,
            attachment_uploads,
            pty_writers,
            access_log: access_log.clone(),
        })
        .layer(middleware::from_fn_with_state(
            access_log.clone(),
            access_log_middleware,
        ));

        runtime.spawn(async move {
            let server = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded server error: {error}");
            }
        });

        Ok(Self {
            url: format!("http://{}:{}/", display_host(addr.ip()), addr.port()),
            hook_forward_token,
            shutdown_tx: Some(shutdown_tx),
            access_log,
        })
    }

    /// Returns the in-memory sink that captures every access log record.
    /// Used by tests and (eventually) by an operator-visible Live tab.
    #[cfg(test)]
    pub(super) fn access_log(&self) -> &AccessLogSink {
        &self.access_log
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

fn route_root_js_modules(mut router: Router<ServerState>) -> Router<ServerState> {
    for asset in embedded_web::root_js_module_assets() {
        let asset = *asset;
        router = router.route(
            asset.path,
            get(move || async move { embedded_web::root_js_module_response(asset) }),
        );
    }
    router
}

pub async fn health_handler() -> &'static str {
    "ok"
}

/// Query parameters on the OAuth redirect (SPEC-2963 Phase 5).
#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn oauth_result_page(title: &str, message: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;padding:2.5rem;max-width:34rem;margin:auto\">\
         <h2>{title}</h2><p>{message}</p>\
         <p style=\"color:#666\">You can close this tab and return to gwt.</p></body></html>"
    ))
}

/// OAuth redirect handler: completes the remote Board provider sign-in against
/// the process-global session store. On success it broadcasts a refreshed
/// [`gwt::BackendEvent::BoardAuthStatus`] to every connected client so the
/// settings UI flips to "Signed in" without a manual Refresh (SPEC-2963
/// FR-012). The token exchange itself is self-contained (global session +
/// token store); only the broadcast needs the shared [`ServerState`].
async fn oauth_callback_handler(
    State(state): State<ServerState>,
    Query(params): Query<OAuthCallbackQuery>,
) -> Html<String> {
    if let Some(error) = params.error.as_deref().filter(|value| !value.is_empty()) {
        return oauth_result_page("Sign-in failed", error);
    }
    let (Some(code), Some(oauth_state)) = (params.code, params.state) else {
        return oauth_result_page("Sign-in failed", "Missing authorization code or state.");
    };
    // The token exchange is blocking (reqwest); run it off the async worker.
    let outcome = tokio::task::spawn_blocking(move || {
        let poster = gwt::board_remote::http::ReqwestHttpClient::new();
        gwt::board_remote::oauth_session::complete_callback(
            gwt::board_remote::signin::sessions(),
            &code,
            &oauth_state,
            &poster,
            &gwt::board_remote::token_store::default_dir(),
            chrono::Utc::now(),
        )
    })
    .await;
    match outcome {
        Ok(Ok(provider_key)) => {
            // Push the refreshed auth/config view to all connected gwt clients
            // so the Settings panel reflects the new sign-in immediately.
            state.clients.dispatch(vec![OutboundEvent::broadcast(
                gwt::system_settings::board_auth_status_event(Some(format!(
                    "Signed in to {provider_key}."
                ))),
            )]);
            oauth_result_page(
                "Signed in",
                &format!("Connected the {provider_key} Board provider."),
            )
        }
        Ok(Err(reason)) => oauth_result_page("Sign-in failed", &reason),
        Err(_) => oauth_result_page("Sign-in failed", "Internal error completing sign-in."),
    }
}

#[derive(Debug, Serialize)]
struct AttachmentUploadTokenResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct AttachmentUploadQuery {
    filename: Option<String>,
    mime_type: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Serialize)]
struct AttachmentUploadResponse {
    upload_id: String,
    filename: String,
    mime_type: Option<String>,
    size: u64,
}

async fn attachment_upload_token_handler(State(state): State<ServerState>) -> impl IntoResponse {
    Json(AttachmentUploadTokenResponse {
        token: state.attachment_upload_token,
    })
}

async fn attachment_upload_handler(
    headers: HeaderMap,
    Query(query): Query<AttachmentUploadQuery>,
    State(state): State<ServerState>,
    request: Request,
) -> Response {
    if !websocket_origin_authorized(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let authorized = headers
        .get("x-gwt-upload-token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|token| token == state.attachment_upload_token);
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let filename = query
        .filename
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("file")
        .to_string();
    let mime_type = query
        .mime_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let (upload_id, path) = state.attachment_uploads.allocate_path();

    if let Some(parent) = path.parent() {
        if let Err(error) = tokio::fs::create_dir_all(parent).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create upload directory: {error}"),
            )
                .into_response();
        }
    }

    let mut file = match tokio::fs::File::create(&path).await {
        Ok(file) => file,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create upload file: {error}"),
            )
                .into_response();
        }
    };
    let mut total_size = 0_u64;
    let mut stream = request.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tokio::fs::remove_file(&path).await;
                return (
                    StatusCode::BAD_REQUEST,
                    format!("failed to read upload: {error}"),
                )
                    .into_response();
            }
        };
        total_size += chunk.len() as u64;
        if let Err(error) = file.write_all(&chunk).await {
            let _ = tokio::fs::remove_file(&path).await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to write upload: {error}"),
            )
                .into_response();
        }
    }
    if let Err(error) = file.flush().await {
        let _ = tokio::fs::remove_file(&path).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to flush upload: {error}"),
        )
            .into_response();
    }
    if let Some(declared) = query.size {
        if declared != total_size {
            let _ = tokio::fs::remove_file(&path).await;
            return (
                StatusCode::BAD_REQUEST,
                format!("upload size mismatch: declared {declared}, received {total_size}"),
            )
                .into_response();
        }
    }

    if let Err(error) = state.attachment_uploads.insert(
        upload_id.clone(),
        UploadedAttachment {
            path,
            filename: filename.clone(),
            mime_type: mime_type.clone(),
            size: total_size,
        },
    ) {
        return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
    }

    Json(AttachmentUploadResponse {
        upload_id,
        filename,
        mime_type,
        size: total_size,
    })
    .into_response()
}

/// Format an [`IpAddr`] for embedding in a URL: IPv6 addresses are wrapped in
/// `[...]` per RFC 3986, IPv4 / hostnames are emitted verbatim.
fn display_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

/// SPEC-1942 FR-098: access log middleware. Captures every HTTP request (and
/// the start of every WebSocket upgrade — the upgrade returns a `101 Switching
/// Protocols` response which is exactly what we record) into both
/// `tracing::info!(target: "gwt_access", ...)` and an in-memory sink for tests.
///
/// `/healthz` is demoted to `tracing::debug!` so periodic health probes do not
/// dominate the stderr stream when the operator wants to spot real LAN access.
async fn access_log_middleware(
    State(sink): State<AccessLogSink>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let user_agent = request
        .headers()
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    let record = AccessLogRecord {
        method,
        path,
        status,
        peer: Some(peer.to_string()),
        user_agent,
        elapsed_ms,
    };

    if record.path == "/healthz" {
        tracing::debug!(
            target: "gwt_access",
            method = %record.method,
            path = %record.path,
            status,
            peer = %peer,
            user_agent = ?record.user_agent,
            elapsed_ms,
            "healthz probe"
        );
    } else {
        tracing::info!(
            target: "gwt_access",
            method = %record.method,
            path = %record.path,
            status,
            peer = %peer,
            user_agent = ?record.user_agent,
            elapsed_ms,
            "embedded server access"
        );
    }
    sink.record(record);

    response
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

pub fn hook_forward_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected_token)
}

pub fn websocket_origin_authorized(headers: &HeaderMap) -> bool {
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
pub fn broadcast_runtime_hook_event(clients: &ClientHub, event: RuntimeHookEvent) {
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
    use tokio::{runtime::Runtime, sync::mpsc};

    use crate::{AppEventProxy, AttachmentUploadStore, OutboundEvent, UserEvent};

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
                attachment_upload_token: "upload-token".to_string(),
                attachment_uploads: AttachmentUploadStore::in_system_temp(),
                pty_writers: Arc::new(RwLock::new(HashMap::new())),
                access_log: super::AccessLogSink::default(),
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

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

        let clients = hub
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !clients.contains_key("slow-client"),
            "lagging websocket client should be unregistered once its queue is full"
        );
    }

    #[test]
    fn client_hub_dispatch_delivers_to_fast_clients_and_drops_only_full_one() {
        let hub = ClientHub::default();
        let mut slow_rx = hub.register("slow".to_string());
        let mut fast_receivers: Vec<(String, mpsc::Receiver<String>)> = (0..5)
            .map(|i| {
                let id = format!("fast-{i}");
                let rx = hub.register(id.clone());
                (id, rx)
            })
            .collect();

        for index in 0..CLIENT_QUEUE_CAPACITY {
            hub.dispatch(vec![OutboundEvent::broadcast(
                BackendEvent::ProjectOpenError {
                    message: format!("fill-{index}"),
                },
            )]);
        }

        for (_, rx) in &mut fast_receivers {
            for _ in 0..CLIENT_QUEUE_CAPACITY {
                rx.try_recv()
                    .expect("fast client receives every fill message");
            }
        }
        for _ in 0..CLIENT_QUEUE_CAPACITY {
            slow_rx
                .try_recv()
                .expect("slow client buffers every fill message before going full");
        }

        for _ in 0..CLIENT_QUEUE_CAPACITY {
            hub.dispatch(vec![OutboundEvent::broadcast(
                BackendEvent::ProjectOpenError {
                    message: "saturate".to_string(),
                },
            )]);
        }

        for (_, rx) in &mut fast_receivers {
            for _ in 0..CLIENT_QUEUE_CAPACITY {
                rx.try_recv()
                    .expect("fast client keeps draining while slow client backs up");
            }
        }

        hub.dispatch(vec![OutboundEvent::broadcast(
            BackendEvent::ProjectOpenError {
                message: "after-saturate".to_string(),
            },
        )]);

        let clients = hub
            .clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !clients.contains_key("slow"),
            "saturated slow client should be evicted"
        );
        for i in 0..5 {
            assert!(
                clients.contains_key(&format!("fast-{i}")),
                "fast client {i} should remain registered"
            );
        }
        drop(clients);

        for (_, rx) in &mut fast_receivers {
            let payload = rx
                .try_recv()
                .expect("fast client still receives after slow eviction");
            assert!(payload.contains("after-saturate"));
        }
    }

    #[test]
    fn client_hub_dispatch_releases_lock_before_serializing_and_sending() {
        let hub = ClientHub::default();
        let _receivers: Vec<_> = (0..200)
            .map(|i| hub.register(format!("client-{i}")))
            .collect();

        let events: Vec<OutboundEvent> = (0..1000)
            .map(|i| {
                OutboundEvent::broadcast(BackendEvent::ProjectOpenError {
                    message: format!("event-{i}"),
                })
            })
            .collect();

        let dispatch_hub = hub.clone();
        let started_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_flag_for_thread = started_flag.clone();
        let dispatch_handle = std::thread::spawn(move || {
            started_flag_for_thread.store(true, std::sync::atomic::Ordering::Release);
            dispatch_hub.dispatch(events);
        });

        while !started_flag.load(std::sync::atomic::Ordering::Acquire) {
            std::thread::yield_now();
        }
        std::thread::sleep(std::time::Duration::from_micros(200));

        let register_start = std::time::Instant::now();
        let _intruder_rx = hub.register("intruder".to_string());
        let register_elapsed = register_start.elapsed();

        dispatch_handle.join().expect("dispatch thread joins");

        assert!(
            register_elapsed < std::time::Duration::from_millis(20),
            "register must not wait for dispatch's serialize+send loop; waited {register_elapsed:?}"
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
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
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

        let theme_toggle_js = client
            .get(format!("{}theme-toggle.js", server.url()))
            .send()
            .expect("theme toggle module request");
        assert_eq!(theme_toggle_js.status(), HttpStatusCode::OK);
        assert_eq!(
            theme_toggle_js
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
        assert!(
            theme_toggle_js
                .text()
                .expect("theme toggle module body")
                .contains("wireThemeToggle"),
            "expected embedded server to serve the segmented theme toggle module",
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

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[test]
    fn embedded_server_streams_attachment_uploads_into_upload_store() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let upload_store = AttachmentUploadStore::in_system_temp();
        let mut server =
            EmbeddedServer::start(&runtime, proxy, clients, pty_writers, upload_store.clone())
                .expect("embedded server");
        let client = reqwest::blocking::Client::new();
        let token_response: serde_json::Value = client
            .get(format!("{}internal/attachment-upload-token", server.url()))
            .send()
            .expect("token request")
            .json()
            .expect("token json");
        let token = token_response
            .get("token")
            .and_then(|value| value.as_str())
            .expect("token field")
            .to_string();

        let upload_response: serde_json::Value = client
            .post(format!(
                "{}internal/attachments/upload?filename=Large%20File.bin&mime_type=application%2Foctet-stream&size=12",
                server.url()
            ))
            .header("x-gwt-upload-token", token)
            .body("upload-bytes")
            .send()
            .expect("upload request")
            .json()
            .expect("upload json");
        let upload_id = upload_response
            .get("upload_id")
            .and_then(|value| value.as_str())
            .expect("upload id");

        let uploaded = upload_store
            .take(upload_id)
            .expect("take upload")
            .expect("uploaded file registered");
        assert_eq!(uploaded.filename, "Large File.bin");
        assert_eq!(
            uploaded.mime_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(uploaded.size, 12);
        assert_eq!(
            std::fs::read(uploaded.path).expect("read uploaded temp"),
            b"upload-bytes"
        );

        server.shutdown();
    }

    // ---------------------------------------------------------------
    // SPEC-1942 US-14: bind / port surface + access log middleware
    // ---------------------------------------------------------------

    #[test]
    fn embedded_server_start_with_bind_accepts_loopback_and_emits_loopback_url() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("loopback bind succeeds");

        assert!(
            server.url().starts_with("http://127.0.0.1:"),
            "loopback bind must surface 127.0.0.1 url, got {}",
            server.url(),
        );
        server.shutdown();
    }

    #[test]
    fn embedded_server_start_with_bind_accepts_unspecified_v4_and_surfaces_actual_ip() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            0,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("0.0.0.0 bind succeeds");

        assert!(
            server.url().starts_with("http://0.0.0.0:"),
            "0.0.0.0 bind must surface 0.0.0.0 url, got {}",
            server.url(),
        );
        server.shutdown();
    }

    /// SPEC #2920 Phase 4 partial — end-to-end coverage that mirrors how
    /// `main.rs` wires the GUI route after the `--bind`/`--port` restore:
    /// argv tokens → `parse_tray_argv` → `TrayArgs` → `start_with_bind` →
    /// served URL. The full main bootstrap blocks on the per-worktree
    /// project-index runtime, so we cannot exercise it inline, but this
    /// composes the pieces that actually deliver VPN-reachable bind.
    #[test]
    fn parsed_tray_argv_drives_embedded_server_bind_end_to_end() {
        let argv: Vec<String> = [
            "gwt",
            "--bind",
            "0.0.0.0",
            "--port",
            "0",
            "--no-tray",
            "--no-open",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let tray_args =
            gwt::cli::tray::parse_tray_argv(&argv).expect("argv with --bind / --port parses");
        assert_eq!(
            tray_args.bind,
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        );
        assert_eq!(tray_args.port, 0);

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            tray_args.bind,
            tray_args.port,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("start_with_bind succeeds for parsed TrayArgs");

        let url = server.url().to_string();
        assert!(
            url.starts_with("http://0.0.0.0:"),
            "parsed `--bind 0.0.0.0` must surface a 0.0.0.0 URL, got {url}",
        );
        server.shutdown();
    }

    #[test]
    fn access_log_layer_records_http_request_with_method_path_status_and_peer() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let url = server.url().to_string();
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(format!("{url}app.js"))
            .header(reqwest::header::USER_AGENT, "build-spec-test/1.0")
            .send()
            .expect("app.js request");
        assert_eq!(response.status(), HttpStatusCode::OK);

        let records = server.access_log().snapshot();
        let app_js = records
            .iter()
            .find(|r| r.path == "/app.js")
            .expect("/app.js entry must be recorded by access log middleware");
        assert_eq!(app_js.method, "GET");
        assert_eq!(app_js.status, 200);
        assert_eq!(
            app_js.user_agent.as_deref(),
            Some("build-spec-test/1.0"),
            "user agent must be carried into the record"
        );
        let peer = app_js.peer.as_deref().expect("peer addr captured");
        assert!(
            peer.starts_with("127.0.0.1:"),
            "peer must be the loopback client, got {peer}"
        );

        server.shutdown();
    }

    #[test]
    fn access_log_layer_still_records_healthz_and_distinguishes_path() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients,
            pty_writers,
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");

        let url = server.url().to_string();
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(format!("{url}healthz"))
            .send()
            .expect("healthz request");
        assert_eq!(response.status(), HttpStatusCode::OK);

        // The sink still captures /healthz so an in-process operator panel
        // can render it, but the tracing layer demotes it to debug — this
        // distinction is asserted at the path level: /healthz is recorded
        // but lives separately from real LAN access records.
        let records = server.access_log().snapshot();
        let healthz = records
            .iter()
            .find(|r| r.path == "/healthz")
            .expect("healthz still appears in the in-memory sink");
        assert_eq!(healthz.method, "GET");
        assert_eq!(healthz.status, 200);

        server.shutdown();
    }
}

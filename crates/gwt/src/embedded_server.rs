pub(crate) use gwt::agent_capability::AgentDurableAuthority;
#[cfg(test)]
pub(crate) use gwt::agent_capability::AgentExecutionAuthorityKind;
pub(crate) use gwt::agent_capability::{
    AgentCapabilityGrant, AgentCapabilityIssuer, AgentCapabilityRegistry, AgentFrontendRequest,
    AgentPmSendResponder, AgentSelfCloseCapabilityTicket, AgentSelfCloseDirectAcceptance,
    AgentSelfCloseResponder, AgentSessionPrincipal,
};
#[cfg(test)]
use gwt::project_transport::send_agent_self_close_acceptance_for_test as send_agent_self_close_acceptance;
use gwt::project_transport::{
    access_log_middleware, agent_bridge_bind_ip, agent_router, client_session, AccessLogPolicy,
    AccessLogSink, TransportEvent, TransportState,
};
#[cfg(test)]
pub(crate) use gwt::project_transport::{
    hook_forward_authorized, prepare_outbound_event, ClientQueue, DrainStep,
};
pub(crate) use gwt::project_transport::{
    ClientHub, ClientHubHealthStats, AGENT_AUTHORITY_UNAVAILABLE_CLOSE, AGENT_STALE_BINDING_CLOSE,
};
#[cfg(test)]
use gwt::HookForwardTarget;
#[cfg(test)]
use std::{collections::HashMap, sync::RwLock};
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU16,
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
    time::{Duration, Instant},
};

use crate::app_runtime::ClientScope;
#[cfg(test)]
use crate::DispatchTarget;
use axum::{
    extract::{ws::WebSocketUpgrade, Query, Request, State},
    http::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        HeaderMap, StatusCode,
    },
    middleware,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use gwt::FrontendEvent;
#[cfg(test)]
use gwt::RuntimeHookEvent;
use gwt_core::repo_hash::ProjectKey;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, net::TcpListener, runtime::Runtime, sync::oneshot};
use uuid::Uuid;

use crate::{
    embedded_web, AppEventProxy, AttachmentUploadStore, OutboundEvent, UploadedAttachment,
    UserEvent,
};

use crate::PtyWriterRegistry;

#[derive(Clone)]
struct ServerState {
    proxy: AppEventProxy,
    clients: ClientHub,
    agent_capabilities: AgentCapabilityRegistry,
    host_instance_id: String,
    attachment_upload_token: String,
    attachment_uploads: AttachmentUploadStore,
    pty_writers: PtyWriterRegistry,
    /// Issue #4538 AC-4: bearer token for `POST /internal/projects/open`,
    /// published only through the `0600` tray lock file. `None` refuses every
    /// control request.
    control_token: Option<Arc<str>>,
    project_open_timeout: Duration,
    // Held only so the in-process sink stays alive for the lifetime of the
    // server. Read directly through [`EmbeddedServer::access_log`] in tests.
    #[allow(dead_code)]
    access_log: AccessLogSink,
}

impl ServerState {
    fn transport(&self) -> TransportState {
        let state = self.clone();
        TransportState::new(
            self.clients.clone(),
            self.agent_capabilities.clone(),
            self.host_instance_id.clone(),
            Arc::new(move |event| match event {
                TransportEvent::FreshExecutionReadyResend {
                    grant,
                    request,
                    reply,
                } => state.proxy.send(UserEvent::FreshExecutionReadyResend {
                    grant,
                    request,
                    reply,
                }),
                TransportEvent::RuntimeHook(event) => {
                    state.proxy.send(UserEvent::RuntimeHook(event))
                }
                TransportEvent::WorkspaceProjectionChanged { project_root } => state
                    .proxy
                    .send(UserEvent::WorkspaceProjectionChanged { project_root }),
                TransportEvent::AgentFrontend {
                    client_id,
                    grant,
                    request,
                } => state.proxy.send(UserEvent::AgentFrontend {
                    client_id,
                    grant,
                    request,
                }),
                TransportEvent::ClientPaneSnapshotRepair {
                    client_id,
                    pane_ids,
                } => state.proxy.send(UserEvent::ClientPaneSnapshotRepair {
                    client_id,
                    pane_ids,
                }),
                TransportEvent::BrowserFrontend {
                    client_id,
                    input_seq,
                    event,
                    received_at,
                } => handle_frontend_message(&state, &client_id, &input_seq, event, received_at),
            }),
        )
    }
}

/// Upper bound on one `gwt open <path>` round trip through the runtime.
const PROJECT_OPEN_CONTROL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct EmbeddedServer {
    url: String,
    bound_addr: SocketAddr,
    agent_capability_issuer: AgentCapabilityIssuer,
    shutdown_tx: Option<oneshot::Sender<()>>,
    agent_shutdown_tx: Option<oneshot::Sender<()>>,
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
            // 0 disables the dedicated fixed-port OAuth listener so parallel
            // tests never contend on a shared loopback port.
            0,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
        )
    }

    /// Test server that accepts `gwt open` control requests with `token`.
    #[cfg(test)]
    pub(super) fn start_with_control_token(
        runtime: &Runtime,
        proxy: AppEventProxy,
        control_token: &str,
        project_open_timeout: Duration,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(SocketAddr::new(
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
        )))?;
        Self::start_serving(
            runtime,
            listener.into_std()?,
            0,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
            Some(control_token.to_string()),
            project_open_timeout,
        )
    }

    /// SPEC-1942 FR-095 / FR-098: bind the embedded server to a caller-chosen
    /// IP / port and install the access-log middleware. Used by the current
    /// browser-server route for both loopback defaults and operator-chosen
    /// `--bind` / `--port`.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_with_bind(
        runtime: &Runtime,
        bind: IpAddr,
        port: u16,
        oauth_redirect_port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
    ) -> std::io::Result<Self> {
        let listener = runtime.block_on(TcpListener::bind(SocketAddr::new(bind, port)))?;
        let listener = listener.into_std()?;
        Self::start_with_listener(
            runtime,
            listener,
            oauth_redirect_port,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
            None,
        )
    }

    /// Start serving from a listener that was bound and committed by the
    /// stable-port startup transaction.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_with_listener(
        runtime: &Runtime,
        listener: std::net::TcpListener,
        oauth_redirect_port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
        control_token: Option<String>,
    ) -> std::io::Result<Self> {
        Self::start_serving(
            runtime,
            listener,
            oauth_redirect_port,
            proxy,
            clients,
            pty_writers,
            attachment_uploads,
            control_token,
            PROJECT_OPEN_CONTROL_TIMEOUT,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_serving(
        runtime: &Runtime,
        listener: std::net::TcpListener,
        oauth_redirect_port: u16,
        proxy: AppEventProxy,
        clients: ClientHub,
        pty_writers: PtyWriterRegistry,
        attachment_uploads: AttachmentUploadStore,
        control_token: Option<String>,
        project_open_timeout: Duration,
    ) -> std::io::Result<Self> {
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        if addr.port() == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "embedded server listener reported bound port 0",
            ));
        }
        let listener = {
            let _runtime_guard = runtime.enter();
            TcpListener::from_std(listener)?
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let agent_listener = runtime.block_on(TcpListener::bind(SocketAddr::new(
            agent_bridge_bind_ip(),
            0,
        )))?;
        let agent_addr = agent_listener.local_addr()?;
        let (agent_shutdown_tx, agent_shutdown_rx) = oneshot::channel();
        let agent_capabilities = AgentCapabilityRegistry::default();
        let agent_capability_issuer = AgentCapabilityIssuer::new(
            format!("http://127.0.0.1:{}/internal/hook-live", agent_addr.port()),
            format!(
                "ws://{}:{}/ws",
                display_host(local_browser_client_ip(addr.ip())),
                addr.port()
            ),
            format!("ws://127.0.0.1:{}/internal/pane-ws", agent_addr.port()),
            agent_capabilities.clone(),
        );
        let attachment_upload_token = Uuid::new_v4().to_string();
        let host_instance_id = Uuid::new_v4().to_string();
        // SPEC #3248 FR-242: from here on this process *is* a Host, so the
        // generation preflight must not ask some other Host to vouch for it.
        // A `gwt` started from inside an agent pane inherits that pane's
        // bridge environment, and without this marker it would preflight
        // itself against the pane's parent Host and refuse its own launches.
        gwt::cli::host_contract::mark_host_process();
        let access_log = AccessLogSink::default();
        let server_state = ServerState {
            proxy,
            clients,
            agent_capabilities,
            host_instance_id,
            attachment_upload_token,
            attachment_uploads,
            pty_writers,
            control_token: control_token.map(Arc::from),
            project_open_timeout,
            access_log: access_log.clone(),
        };

        // Agent-originated HTTP traffic is isolated from the browser surface.
        // This router is deliberately capability-only; future agent routes can
        // be added here and reuse the same authenticated principal boundary.
        let agent_app = agent_router(server_state.transport(), access_log.clone());

        // SPEC-3016: every embedded frontend asset route (entrypoints, root
        // JS modules, vendor JS/CSS, stylesheets, fonts) is registered from
        // the embedded_web manifest tables.
        let app = route_root_js_modules(route_static_assets(Router::new()))
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
            .route("/ws", get(websocket_handler))
            // Issue #4538 AC-1: per-project URLs share the entrypoint.
            .route(
                "/p/{repo_hash}",
                get(
                    |axum::extract::Path(repo_hash): axum::extract::Path<String>| async move {
                        embedded_web::project_route_response(&repo_hash)
                    },
                ),
            )
            .route(
                "/p/{repo_hash}/{*rest}",
                get(|| async { embedded_web::project_not_found_response() }),
            )
            .route(
                gwt::project_open_control::PROJECT_OPEN_CONTROL_PATH,
                post(project_open_control_handler),
            )
            .with_state(server_state)
            .layer(middleware::from_fn_with_state(
                AccessLogPolicy::browser(access_log.clone()),
                access_log_middleware,
            ));

        // SPEC-2963 FR-005: dedicated fixed-port loopback OAuth callback
        // listener. The OAuth redirect_uri must be a stable, pre-registered URL
        // (`http://127.0.0.1:<oauth_redirect_port>/oauth/callback`), but the
        // main server uses an ephemeral / operator-chosen port. Bind the fixed
        // loopback port and serve the same router so `/oauth/callback` is
        // reachable there. Skipped when disabled (`0`, e.g. tests) or when the
        // main server already listens on that port (no double-bind).
        let oauth_listener = if oauth_redirect_port != 0 && oauth_redirect_port != addr.port() {
            match runtime.block_on(TcpListener::bind((
                IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                oauth_redirect_port,
            ))) {
                Ok(listener) => Some(listener),
                Err(error) => {
                    eprintln!(
                        "gwt: OAuth callback port {oauth_redirect_port} is unavailable \
                         ({error}); remote Board sign-in may fail until it is freed or \
                         changed in Settings."
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(oauth_listener) = oauth_listener {
            let oauth_app = app.clone();
            runtime.spawn(async move {
                if let Err(error) = axum::serve(
                    oauth_listener,
                    oauth_app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                {
                    eprintln!("embedded OAuth callback server error: {error}");
                }
            });
        }

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

        runtime.spawn(async move {
            let server = axum::serve(
                agent_listener,
                agent_app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = agent_shutdown_rx.await;
            });
            if let Err(error) = server.await {
                eprintln!("embedded agent bridge error: {error}");
            }
        });

        Ok(Self {
            url: format!("http://{}:{}/", display_host(addr.ip()), addr.port()),
            bound_addr: addr,
            agent_capability_issuer,
            shutdown_tx: Some(shutdown_tx),
            agent_shutdown_tx: Some(agent_shutdown_tx),
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

    pub(super) fn bound_port(&self) -> NonZeroU16 {
        NonZeroU16::new(self.bound_addr.port())
            .expect("EmbeddedServer validates its bound port before construction")
    }

    pub(super) fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.agent_shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    pub(crate) fn agent_capability_issuer(&self) -> AgentCapabilityIssuer {
        self.agent_capability_issuer.clone()
    }

    #[cfg(test)]
    pub(super) fn hook_forward_target(&self) -> HookForwardTarget {
        let project_root = std::env::current_dir().expect("embedded-server test project root");
        self.agent_capability_issuer
            .issue(&project_root, "session-1")
            .expect("canonical embedded-server test session")
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

/// Registers one GET route per [`embedded_web::StaticAsset`] manifest entry
/// (SPEC-3016: the manifest is the routing source of truth).
fn route_static_assets(mut router: Router<ServerState>) -> Router<ServerState> {
    for asset in embedded_web::static_assets() {
        router = router.route(
            asset.route,
            get(move || async move { embedded_web::static_asset_response(asset) }),
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

fn local_browser_client_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ip => ip,
    }
}

/// Issue #4538 AC-4 / AC-5: `gwt open <path>` asks the runtime to open a
/// Project and answers with its ProjectKey once the async open has committed.
/// Nothing is dispatched to the runtime before authorization and validation.
async fn project_open_control_handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    use gwt::project_open_control::{
        authorize_control_request, parse_control_request, project_url_path,
        ProjectOpenControlRejection, ProjectOpenControlResponse,
        PROJECT_OPEN_CONTROL_MAX_BODY_BYTES,
    };
    let header_text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let result = async {
        authorize_control_request(header_text(AUTHORIZATION), state.control_token.as_deref())?;
        let mut body = Vec::new();
        let mut stream = request.into_body().into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                ProjectOpenControlRejection::BadRequest(format!("failed to read body: {error}"))
            })?;
            if body.len() + chunk.len() > PROJECT_OPEN_CONTROL_MAX_BODY_BYTES {
                return Err(ProjectOpenControlRejection::PayloadTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        let path = parse_control_request(header_text(axum::http::header::CONTENT_TYPE), &body)?;
        open_project_through_runtime(&state.proxy, path, state.project_open_timeout).await
    }
    .await;
    match result {
        Ok(project_key) => Json(ProjectOpenControlResponse {
            url_path: project_url_path(project_key.as_str()),
            project_key: project_key.to_string(),
        })
        .into_response(),
        Err(rejection) => (
            StatusCode::from_u16(rejection.status()).unwrap_or(StatusCode::BAD_REQUEST),
            Json(gwt::project_open_control::ProjectOpenControlErrorBody {
                error: rejection.message(),
            }),
        )
            .into_response(),
    }
}

async fn open_project_through_runtime(
    proxy: &AppEventProxy,
    path: PathBuf,
    timeout: Duration,
) -> Result<ProjectKey, gwt::project_open_control::ProjectOpenControlRejection> {
    use crate::app_runtime::{ProjectOpenControlFailure, ProjectOpenReply};
    use gwt::project_open_control::ProjectOpenControlRejection as Rejection;
    let (reply, outcome) = ProjectOpenReply::channel();
    proxy.send(UserEvent::ControlProjectOpen { path, reply });
    match tokio::time::timeout(timeout, outcome).await {
        Err(_) => Err(Rejection::Timeout),
        Ok(Err(_)) => Err(Rejection::Unavailable(
            "gwt is not accepting project requests".to_string(),
        )),
        Ok(Ok(Ok(project_key))) => Ok(project_key),
        Ok(Ok(Err(ProjectOpenControlFailure::Rejected(message)))) => {
            Err(Rejection::Unprocessable(message))
        }
        Ok(Ok(Err(ProjectOpenControlFailure::Unavailable(message)))) => {
            Err(Rejection::Unavailable(message))
        }
    }
}

#[derive(Default, Deserialize)]
struct WebsocketQuery {
    repo_hash: Option<String>,
}

impl WebsocketQuery {
    fn scope(self) -> Result<ClientScope, StatusCode> {
        match self.repo_hash {
            Some(hash) => ProjectKey::parse(&hash)
                .map(ClientScope::Project)
                .map_err(|_| StatusCode::BAD_REQUEST),
            None => Ok(ClientScope::Hub),
        }
    }
}

async fn websocket_handler(
    Query(query): Query<WebsocketQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<ServerState>,
) -> impl IntoResponse {
    if !websocket_origin_authorized(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let scope = match query.scope() {
        Ok(scope) => scope,
        Err(status) => return status.into_response(),
    };
    ws.on_upgrade(move |socket| client_session(socket, state.transport(), scope))
}

fn handle_frontend_message(
    state: &ServerState,
    client_id: &str,
    input_seq: &AtomicU64,
    event: FrontendEvent,
    received_at: Instant,
) {
    let (id, data) = match event {
        FrontendEvent::TerminalInput { id, data } => (id, data),
        FrontendEvent::StartupFirstFrame { navigation_ms } => {
            gwt::perf::startup::first_frame(navigation_ms);
            // Keep the shell/event-loop readiness acknowledgement separate
            // from the paint observation when restore is still draining.
            state.proxy.send(UserEvent::Frontend {
                client_id: client_id.to_string(),
                client_scope: state.clients.scope(client_id),
                event: FrontendEvent::StartupFirstFrame { navigation_ms },
                received_at,
            });
            return;
        }
        FrontendEvent::StartupTerminalReady { id } => {
            // Input uses this WebSocket fast path too: an unrelated event-loop
            // backlog must not inflate the time at which keys can reach the PTY.
            gwt::perf::startup::terminal_ready(&id);
            return;
        }
        other => {
            state.proxy.send(UserEvent::Frontend {
                client_id: client_id.to_string(),
                client_scope: state.clients.scope(client_id),
                event: other,
                received_at,
            });
            return;
        }
    };

    let Some(ClientScope::Project(project_key)) = state.clients.scope(client_id) else {
        return;
    };

    // Issue #4145 AC-1: the prompt-send route is the submit reaching the PTY,
    // covering both the WebSocket fast path and the event-loop fallback below.
    // Only a submit is timed — Issue #3611 is the reminder that per-keystroke
    // work on this path is exactly what must not be added.
    let _perf_route = (data.contains('\n') || data.contains('\r'))
        .then(|| gwt::perf::RouteTimer::start(gwt::perf::PerfRoute::PromptSend));

    let seq = input_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_recv",
        client_id = %client_id,
        seq,
        window_id = %id,
        "terminal_input received over WebSocket"
    );

    let pty_handle = match state.pty_writers.read() {
        Ok(guard) => guard.get(&id).map(|pty| (pty.clone(), guard.len())),
        Err(_error) => {
            tracing::warn!(
                target: "gwt_input_trace",
                stage = "fast_path_lock_poisoned",
                client_id = %client_id,
                seq,
                window_id = %id,
                "pty_writers read lock poisoned; falling back to event loop"
            );
            None
        }
    };

    let approval_resolution = gwt::window_state::is_approval_resolution_input(&data);
    let mut resolution_marked = false;
    if let Some((entry, pty_writer_count)) = pty_handle {
        if entry.project_key != project_key {
            return;
        }
        let pty = &entry.handle;
        if approval_resolution {
            // `EventLoopProxy::send_event` completes the tao channel enqueue
            // synchronously. Enqueue the causal marker before the PTY write so
            // provider output cannot overtake it on the event-loop receiver.
            state
                .proxy
                .send(UserEvent::RuntimeApprovalResolutionStarted { id: id.clone() });
            resolution_marked = true;
        }
        let had_unsent = pty.has_unsent_user_input();
        let write_started = Instant::now();
        match pty.write_input(data.as_bytes()) {
            Ok(()) => {
                let completed_at = Instant::now();
                log_terminal_input_completion(
                    client_id,
                    seq,
                    &id,
                    completed_at.duration_since(write_started).as_micros() as u64,
                    completed_at.duration_since(received_at).as_millis() as u64,
                    pty_writer_count,
                );
                if had_unsent && !pty.has_unsent_user_input() {
                    state
                        .proxy
                        .send(UserEvent::FlushPendingPmWake { id: id.clone() });
                }
                return;
            }
            Err(_error) => {
                if resolution_marked {
                    state
                        .proxy
                        .send(UserEvent::RuntimeApprovalResolutionCancelled { id: id.clone() });
                }
                tracing::warn!(
                    target: "gwt_input_trace",
                    stage = "fast_path_write_err",
                    client_id = %client_id,
                    seq,
                    window_id = %id,
                    "fast-path PTY write failed; dropping input to preserve the generation fence"
                );
                return;
            }
        }
    } else {
        tracing::debug!(
            target: "gwt_input_trace",
            stage = "fast_path_miss",
            client_id = %client_id,
            seq,
            window_id = %id,
            "pty_writers registry miss; falling back to event loop"
        );
    }

    forward_terminal_input_to_event_loop(state, client_id, id.clone(), data, received_at);
    tracing::debug!(
        target: "gwt_input_trace",
        stage = "ws_dispatch",
        client_id = %client_id,
        seq,
        window_id = %id,
        ok = true,
        "terminal_input forwarded to event loop proxy (fallback)"
    );
}

fn forward_terminal_input_to_event_loop(
    state: &ServerState,
    client_id: &str,
    id: String,
    data: String,
    received_at: Instant,
) {
    state.proxy.send(UserEvent::Frontend {
        client_id: client_id.to_string(),
        client_scope: state.clients.scope(client_id),
        event: FrontendEvent::TerminalInput { id, data },
        received_at,
    });
}

fn log_terminal_input_completion(
    client_id: &str,
    seq: u64,
    window_id: &str,
    write_us: u64,
    elapsed_ms: u64,
    pty_writer_count: usize,
) {
    if elapsed_ms >= crate::GUI_EVENT_LOOP_SLOW_DISPATCH_MS {
        tracing::warn!(
            target: "gwt_input_trace",
            stage = "fast_path_write",
            client_id,
            seq,
            window_id,
            write_us,
            elapsed_ms,
            pty_writer_count,
            "terminal_input receive-to-PTY latency exceeded budget"
        );
    } else {
        tracing::debug!(
            target: "gwt_input_trace",
            stage = "fast_path_write",
            client_id,
            seq,
            window_id,
            write_us,
            elapsed_ms,
            pty_writer_count,
            "terminal_input written to PTY via WS fast-path"
        );
    }
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
    clients.dispatch(vec![OutboundEvent {
        target: DispatchTarget::All,
        event: gwt::BackendEvent::RuntimeHookEvent { event },
        knowledge_wire_metadata: None,
        terminal_stream_seq: None,
        error_origin: None,
    }]);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        net::IpAddr,
        pin::Pin,
        sync::{atomic::AtomicU64, Arc, Mutex, RwLock},
        task::{Context, Poll},
        time::{Duration, Instant},
    };

    use axum::extract::ws::Message as AxumMessage;
    use axum::http::{
        header::{AUTHORIZATION, HOST, ORIGIN},
        HeaderMap, StatusCode,
    };
    use futures_util::{Sink, SinkExt, StreamExt};
    use gwt::{BackendEvent, FrontendEvent, RuntimeHookEvent, RuntimeHookEventKind};
    use gwt_core::test_support::ScopedEnvVar;
    use reqwest::StatusCode as HttpStatusCode;
    use tokio::runtime::Runtime;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{
            client::IntoClientRequest, Error as WebSocketError, Message as WebSocketMessage,
        },
    };

    use crate::{AppEventProxy, AttachmentUploadStore, OutboundEvent, UserEvent};

    use super::{
        agent_bridge_bind_ip, handle_frontend_message, send_agent_self_close_acceptance,
        websocket_origin_authorized, AgentCapabilityIssuer, AgentCapabilityRegistry,
        AgentFrontendRequest, AgentSelfCloseDirectAcceptance, ClientHub, EmbeddedServer,
        HookForwardTarget, ServerState,
    };

    struct FailingMessageSink;

    impl Sink<AxumMessage> for FailingMessageSink {
        type Error = &'static str;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err("socket closed"))
        }

        fn start_send(self: Pin<&mut Self>, _item: AxumMessage) -> Result<(), Self::Error> {
            Err("socket closed")
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    struct PendingMessageSink;

    impl Sink<AxumMessage> for PendingMessageSink {
        type Error = &'static str;

        fn poll_ready(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, _item: AxumMessage) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }
    }

    use crate::app_runtime::ClientScope;
    use gwt_core::repo_hash::ProjectKey;

    fn project_a() -> ProjectKey {
        ProjectKey::parse("0123456789abcdef").unwrap()
    }

    fn sample_server_state() -> (ServerState, Arc<Mutex<Vec<UserEvent>>>) {
        let (proxy, events) = AppEventProxy::stub();
        (
            ServerState {
                proxy,
                clients: ClientHub::default(),
                agent_capabilities: AgentCapabilityRegistry::default(),
                host_instance_id: "test-host-instance".to_string(),
                attachment_upload_token: "upload-token".to_string(),
                attachment_uploads: AttachmentUploadStore::in_system_temp(),
                pty_writers: Arc::new(RwLock::new(HashMap::new())),
                control_token: None,
                project_open_timeout: Duration::from_secs(1),
                access_log: super::AccessLogSink::default(),
            },
            events,
        )
    }

    fn sample_runtime_hook_event() -> RuntimeHookEvent {
        RuntimeHookEvent {
            kind: RuntimeHookEventKind::RuntimeState,
            source_event: Some("PreToolUse".to_string()),
            gwt_session_id: Some("session-1".to_string()),
            continuation_readiness_nonce: None,
            agent_session_id: Some("agent-1".to_string()),
            project_root: Some("E:/gwt/test-repo".to_string()),
            branch: Some("feature/runtime".to_string()),
            status: Some("Running".to_string()),
            tool_name: Some("Bash".to_string()),
            message: None,
            occurred_at: "2026-04-21T00:00:00Z".to_string(),
        }
    }

    fn direct_acceptance_for_test(
        proxy: AppEventProxy,
        ticket_id: &str,
    ) -> AgentSelfCloseDirectAcceptance {
        AgentSelfCloseDirectAcceptance::new(
            "e544de42-fd9f-49a7-9ba2-b8b16ca1572a".to_string(),
            "tab-owned::agent-1".to_string(),
            super::AgentSelfCloseCapabilityTicket::for_test(ticket_id.to_string()),
            Arc::new(move |ticket| proxy.send(UserEvent::CommitAgentSelfClose { ticket })),
        )
    }

    fn recorded_self_close_commit_ids(events: &Arc<Mutex<Vec<UserEvent>>>) -> Vec<String> {
        events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(|event| match event {
                UserEvent::CommitAgentSelfClose { ticket } => Some(ticket.id().to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn accepted_self_close_send_error_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut sink = FailingMessageSink;

        runtime.block_on(send_agent_self_close_acceptance(
            &mut sink,
            direct_acceptance_for_test(proxy, "send-error-ticket"),
            Duration::from_secs(1),
        ));

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["send-error-ticket"]
        );
    }

    #[test]
    fn accepted_self_close_send_timeout_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut sink = PendingMessageSink;

        runtime.block_on(send_agent_self_close_acceptance(
            &mut sink,
            direct_acceptance_for_test(proxy, "send-timeout-ticket"),
            Duration::from_millis(10),
        ));

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["send-timeout-ticket"]
        );
    }

    #[test]
    fn accepted_self_close_task_cancellation_still_finalizes_exactly_once() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();

        runtime.block_on(async {
            let acceptance = direct_acceptance_for_test(proxy, "cancelled-task-ticket");
            let task = tokio::spawn(async move {
                let mut sink = PendingMessageSink;
                send_agent_self_close_acceptance(&mut sink, acceptance, Duration::from_secs(60))
                    .await;
            });
            tokio::task::yield_now().await;
            task.abort();
            assert!(task
                .await
                .expect_err("task must be cancelled")
                .is_cancelled());
        });

        assert_eq!(
            recorded_self_close_commit_ids(&events),
            vec!["cancelled-task-ticket"]
        );
    }

    #[test]
    fn dropping_pm_origin_cancels_its_pending_mutation() {
        let (responder, _result, cancellation) = super::AgentPmSendResponder::channel();
        assert!(responder.mutation_is_current());

        drop(cancellation);

        assert!(
            !responder.mutation_is_current(),
            "an aborted origin task must revoke a pending PTY mutation"
        );
    }

    #[test]
    fn pm_origin_timeout_distinguishes_zero_mutation_from_committed_input() {
        let (pending, _pending_result, pending_cancellation) =
            super::AgentPmSendResponder::channel();
        assert!(
            !pending_cancellation.cancel(),
            "cancellation that wins the CAS proves zero mutation"
        );
        assert!(!pending.try_commit_mutation());

        let (committed, _committed_result, committed_cancellation) =
            super::AgentPmSendResponder::channel();
        assert!(committed.try_commit_mutation());
        assert!(
            committed_cancellation.cancel(),
            "timeout after the commit CAS must be reported as ambiguous"
        );
    }

    #[test]
    fn correlated_agent_self_close_acceptance_uses_only_the_origin_socket() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let finalizer_proxy = proxy.clone();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let request_id = "e544de42-fd9f-49a7-9ba2-b8b16ca1572a";
        let window_id = "tab-owned::agent-1";

        let ticket = runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");

            let pane_queue = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let queue = clients.first_agent_queue_for_test();
                    if let Some(queue) = queue {
                        break queue;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("pane client registration");
            assert!(!pane_queue.enqueue_workspace_for_test(Arc::from(
                serde_json::json!({
                    "kind": "workspace_state",
                    "workspace": {
                        "active_tab_id": "tab-owned",
                        "recent_projects": [],
                        "tabs": [{
                            "id": "tab-owned",
                            "project_root": project.path(),
                            "workspace": { "windows": [{ "id": window_id }] }
                        }]
                    }
                })
                .to_string()
            )));
            let workspace = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("scoped workspace response")
                .expect("workspace frame")
                .expect("valid workspace frame");
            assert!(matches!(workspace, WebSocketMessage::Text(_)));

            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "close_window",
                        "id": window_id,
                        "request_id": request_id,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send correlated close");

            let (grant, responder) = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let dispatched = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded
                            .iter()
                            .position(|event| matches!(event, UserEvent::AgentFrontend { .. }));
                        position.map(|position| recorded.remove(position))
                    };
                    if let Some(UserEvent::AgentFrontend {
                        grant,
                        request:
                            AgentFrontendRequest::CloseWindow {
                                id,
                                request_id: Some(correlation),
                                responder: Some(responder),
                            },
                        ..
                    }) = dispatched
                    {
                        assert_eq!(id, window_id);
                        assert_eq!(correlation, request_id);
                        break (grant, responder);
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("agent close dispatch");
            let ticket = issuer
                .begin_self_close_if_current(&grant)
                .expect("accept current self-close generation");
            responder
                .send(AgentSelfCloseDirectAcceptance::new(
                    request_id.to_string(),
                    window_id.to_string(),
                    ticket,
                    Arc::new({
                        let proxy = finalizer_proxy.clone();
                        move |ticket| proxy.send(UserEvent::CommitAgentSelfClose { ticket })
                    }),
                ))
                .expect("origin response task is waiting");

            let response = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("direct close acceptance")
                .expect("acceptance frame")
                .expect("valid acceptance frame");
            let WebSocketMessage::Text(response) = response else {
                panic!("acceptance must be text");
            };
            let response: serde_json::Value =
                serde_json::from_str(response.as_ref()).expect("acceptance JSON");
            assert_eq!(response["kind"], "pane_close_accepted");
            assert_eq!(response["request_id"], request_id);
            assert_eq!(response["window_id"], window_id);
            assert_eq!(
                pane_queue.len_for_test(),
                0,
                "the direct acceptance must not pass through ClientHub"
            );

            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let ticket = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded.iter().position(|event| {
                            matches!(event, UserEvent::CommitAgentSelfClose { .. })
                        });
                        position.map(|position| match recorded.remove(position) {
                            UserEvent::CommitAgentSelfClose { ticket } => ticket,
                            _ => unreachable!("matched self-close finalizer"),
                        })
                    };
                    if let Some(ticket) = ticket {
                        break ticket;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("accepted response attempt must schedule finalization")
        });
        assert!(issuer.finish_self_close(&ticket));
        server.shutdown();
    }

    #[test]
    fn authenticated_pm_send_routes_to_runtime_and_returns_only_on_origin_socket() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "pm-session")
            .expect("PM capability");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let operation_id = "72fc3cd4-ad49-43e3-bf3d-d791357643b0";
        let window_id = "tab-owned::agent-1";

        runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");
            let pane_queue = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let queue = clients.first_agent_queue_for_test();
                    if let Some(queue) = queue {
                        break queue;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("pane client registration");
            assert!(!pane_queue.enqueue_workspace_for_test(Arc::from(
                serde_json::json!({
                    "kind": "workspace_state",
                    "workspace": {
                        "active_tab_id": "tab-owned",
                        "recent_projects": [],
                        "tabs": [{
                            "id": "tab-owned",
                            "project_root": project.path(),
                            "workspace": { "windows": [{
                                "id": window_id,
                                "preset": "agent",
                                "status": "idle",
                                "session_id": "target-session"
                            }] }
                        }]
                    }
                })
                .to_string()
            )));
            let _workspace = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("scoped workspace response")
                .expect("workspace frame")
                .expect("valid workspace frame");

            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pm_pane_send_input",
                        "operation_id": operation_id,
                        "window_id": window_id,
                        "text": "report status\r",
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send PM request");

            let responder = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let dispatched = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded
                            .iter()
                            .position(|event| matches!(event, UserEvent::AgentFrontend { .. }));
                        position.map(|position| recorded.remove(position))
                    };
                    if let Some(UserEvent::AgentFrontend {
                        grant,
                        request:
                            AgentFrontendRequest::PmSendInput {
                                operation_id: routed_operation,
                                window_id: routed_window,
                                responder: Some(responder),
                                ..
                            },
                        ..
                    }) = dispatched
                    {
                        assert_eq!(grant.principal().session_id(), "pm-session");
                        assert_eq!(routed_operation, operation_id);
                        assert_eq!(routed_window, window_id);
                        break responder;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("PM runtime dispatch");
            responder
                .send(BackendEvent::PmMessageSendResult {
                    operation_id: operation_id.to_string(),
                    status: "delivered".to_string(),
                    window_id: Some(window_id.to_string()),
                    reason: None,
                })
                .expect("origin response task is waiting");

            let response = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("direct PM terminal result")
                .expect("terminal frame")
                .expect("valid terminal frame");
            let WebSocketMessage::Text(response) = response else {
                panic!("PM result must be text");
            };
            let response: serde_json::Value =
                serde_json::from_str(response.as_ref()).expect("PM result JSON");
            assert_eq!(response["kind"], "pm_message_send_result");
            assert_eq!(response["operation_id"], operation_id);
            assert_eq!(response["status"], "delivered");
            assert_eq!(response["window_id"], window_id);
            assert_eq!(
                pane_queue.len_for_test(),
                0,
                "the correlated PM result must not enter ClientHub"
            );
        });
        server.shutdown();
    }

    #[test]
    fn authenticated_monitor_scan_routes_scope_guard_and_result_only_to_origin_socket() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let decoy = clients.register_pane_for_test("decoy-client".to_string());
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "pm-session")
            .expect("PM capability");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let expected_project_scope = gwt_core::paths::project_scope_hash(project.path())
            .as_str()
            .to_string();

        runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "agent_issue_monitor_scan_now",
                        "expected_project_scope": expected_project_scope,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send Monitor scan request");

            let client_id = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let dispatched = {
                        let mut recorded = events
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let position = recorded
                            .iter()
                            .position(|event| matches!(event, UserEvent::AgentFrontend { .. }));
                        position.map(|position| recorded.remove(position))
                    };
                    if let Some(UserEvent::AgentFrontend {
                        client_id,
                        grant,
                        request:
                            AgentFrontendRequest::IssueMonitorScanNow {
                                expected_project_scope: routed_scope,
                            },
                    }) = dispatched
                    {
                        assert_eq!(grant.principal().session_id(), "pm-session");
                        assert_eq!(routed_scope, expected_project_scope);
                        break client_id;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("Monitor scan runtime dispatch");

            clients.dispatch(vec![OutboundEvent::reply(
                client_id,
                BackendEvent::IssueMonitorScanRequestResult {
                    accepted: true,
                    reason: None,
                },
            )]);
            let response = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("origin Monitor result")
                .expect("Monitor result frame")
                .expect("valid Monitor result frame");
            let WebSocketMessage::Text(response) = response else {
                panic!("Monitor result must be text");
            };
            let response: serde_json::Value =
                serde_json::from_str(response.as_ref()).expect("Monitor result JSON");
            assert_eq!(response["kind"], "issue_monitor_scan_request_result");
            assert_eq!(response["accepted"], true);
            assert!(
                decoy.try_recv().is_none(),
                "the Monitor result must not reach another pane client"
            );
        });
        server.shutdown();
    }

    #[test]
    fn correlated_agent_self_close_is_rejected_before_enqueue_after_rotation() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original target");
        let pane_url = issuer.agent_pane_websocket_url().to_string();

        runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", original.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, _) = connect_async(request).await.expect("agent pane WebSocket");
            let pane_queue = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let queue = clients.first_agent_queue_for_test();
                    if let Some(queue) = queue {
                        break queue;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("pane client registration");
            assert!(!pane_queue.enqueue_workspace_for_test(Arc::from(
                serde_json::json!({
                    "kind": "workspace_state",
                    "workspace": {
                        "active_tab_id": "tab-owned",
                        "recent_projects": [],
                        "tabs": [{
                            "id": "tab-owned",
                            "project_root": project.path(),
                            "workspace": {
                                "windows": [{ "id": "tab-owned::agent-1" }]
                            }
                        }]
                    }
                })
                .to_string()
            )));
            let _workspace = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("scoped workspace response")
                .expect("workspace frame")
                .expect("valid workspace frame");

            issuer
                .issue(project.path(), "session-1")
                .expect("rotate capability");
            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "close_window",
                        "id": "tab-owned::agent-1",
                        "request_id": "52185ac8-3d18-470f-bfc3-73fa5eac2ff5",
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send close after rotation");
            let _ = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .expect("rotated correlated socket must close");
        });

        assert!(
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "a rotated correlated close must not enqueue AgentFrontend"
        );
        server.shutdown();
    }

    #[test]
    fn agent_bridge_bind_policy_widens_only_for_native_linux_container_access() {
        let expected = if cfg!(target_os = "linux") {
            IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        };

        assert_eq!(agent_bridge_bind_ip(), expected);
    }

    #[test]
    fn agent_pane_websocket_route_requires_its_capability_and_keeps_browser_ws_open() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign_token = AgentCapabilityRegistry::default()
            .issue(project.path(), "session-1")
            .expect("foreign-registry capability");
        let agent_pane_url = issuer.agent_pane_websocket_url().to_string();
        let browser_pane_url = issuer.pane_websocket_url().to_string();

        runtime.block_on(async {
            for (case, token) in [("missing", None), ("foreign", Some(foreign_token.as_str()))] {
                let mut request = agent_pane_url
                    .as_str()
                    .into_client_request()
                    .expect("agent pane WebSocket request");
                if let Some(token) = token {
                    request.headers_mut().insert(
                        AUTHORIZATION,
                        format!("Bearer {token}")
                            .parse()
                            .expect("bearer header value"),
                    );
                }

                match connect_async(request).await {
                    Err(WebSocketError::Http(response)) => assert_eq!(
                        response.status().as_u16(),
                        StatusCode::UNAUTHORIZED.as_u16(),
                        "{case} capability must be rejected during the handshake"
                    ),
                    Err(error) => panic!("{case} handshake returned the wrong error: {error}"),
                    Ok((socket, _)) => {
                        drop(socket);
                        panic!("{case} capability unexpectedly upgraded")
                    }
                }
            }

            let mut authorized_request = agent_pane_url
                .as_str()
                .into_client_request()
                .expect("authorized agent pane WebSocket request");
            authorized_request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("authorized bearer header value"),
            );
            let (mut authorized_socket, response) = connect_async(authorized_request)
                .await
                .expect("authorized agent pane WebSocket upgrade");
            assert_eq!(
                response.status().as_u16(),
                StatusCode::SWITCHING_PROTOCOLS.as_u16()
            );
            authorized_socket
                .close(None)
                .await
                .expect("close authorized agent pane WebSocket");

            let (mut browser_socket, response) = connect_async(browser_pane_url.as_str())
                .await
                .expect("browser WebSocket remains token-free");
            assert_eq!(
                response.status().as_u16(),
                StatusCode::SWITCHING_PROTOCOLS.as_u16()
            );
            browser_socket
                .close(None)
                .await
                .expect("close browser WebSocket");
        });

        let records = server.access_log().snapshot();
        assert!(records.iter().any(|record| {
            record.path == "/internal/pane-ws" && record.status == StatusCode::UNAUTHORIZED.as_u16()
        }));
        assert!(records.iter().any(|record| {
            record.path == "/internal/pane-ws"
                && record.status == StatusCode::SWITCHING_PROTOCOLS.as_u16()
        }));
        assert!(records.iter().any(|record| {
            record.path == "/ws" && record.status == StatusCode::SWITCHING_PROTOCOLS.as_u16()
        }));

        server.shutdown();
    }

    /// Issue #3667 AC-1/AC-2/AC-3/AC-4: a settled session (in-memory Active
    /// binding whose durable record no longer matches) keeps pane observation
    /// on the agent WebSocket while producing mutation stays refused on the
    /// very same connection.
    #[test]
    fn settled_session_pane_socket_allows_observation_and_refuses_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The durable authority check runs on spawn_blocking threads where the
        // thread-local ScopedGwtHome does not apply, so isolate HOME itself.
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        // No durable session file exists for this id, so every durable check
        // resolves Stale — the same authority a session holds right after its
        // Execution Control Record settles.
        let session_id = "session-issue-3667-settled";
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session_id.to_string(),
            repo_hash: "repo-3667".to_string(),
            owner_kind: "issue".to_string(),
            owner_number: 3667,
            identity: gwt_agent::ExecutionBindingIdentity {
                generation_id: "generation-3667".to_string(),
                binding_id: "binding-3667".to_string(),
                ledger_head_hash: "head-3667".to_string(),
            },
            capability_generation: 1,
        };
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue_bound(project.path(), session_id, binding)
            .expect("settled capability");
        let pane_url = issuer.agent_pane_websocket_url().to_string();

        runtime.block_on(async {
            let mut request = pane_url
                .as_str()
                .into_client_request()
                .expect("agent pane WebSocket request");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {}", target.token)
                    .parse()
                    .expect("bearer header value"),
            );
            let (mut socket, response) = connect_async(request)
                .await
                .expect("settled session must keep the pane observation transport");
            assert_eq!(
                response.status().as_u16(),
                StatusCode::SWITCHING_PROTOCOLS.as_u16()
            );

            socket
                .send(WebSocketMessage::Text(
                    r#"{"kind":"list_windows"}"#.to_string().into(),
                ))
                .await
                .expect("send list_windows on settled socket");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .iter()
                        .any(|event| {
                            matches!(
                                event,
                                UserEvent::AgentFrontend {
                                    request: AgentFrontendRequest::ListWindows,
                                    ..
                                }
                            )
                        })
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("settled observation request must reach runtime dispatch");

            socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": session_id,
                        "text": "must-not-dispatch"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send pane input on settled socket");
            let close = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match socket.next().await {
                        Some(Ok(WebSocketMessage::Close(frame))) => break frame,
                        Some(Ok(_)) => continue,
                        other => {
                            panic!("settled mutation must end in a close frame, got {other:?}")
                        }
                    }
                }
            })
            .await
            .expect("settled mutation must be fenced")
            .expect("explicit close frame");
            assert_eq!(u16::from(close.code), 1008);
            assert_eq!(close.reason, "execution binding is no longer current");
            assert!(
                !events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .any(|event| {
                        matches!(
                            event,
                            UserEvent::AgentFrontend {
                                request: AgentFrontendRequest::SendInput { .. },
                                ..
                            }
                        )
                    }),
                "settled mutation must not reach AgentFrontend dispatch"
            );
        });

        server.shutdown();
    }

    #[test]
    fn connected_agent_pane_socket_stops_dispatching_after_rotation_and_revoke() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original capability");
        let pane_url = issuer.agent_pane_websocket_url().to_string();
        let ready = r#"{"kind":"frontend_ready"}"#.to_string();

        runtime.block_on(async {
            let connect = |token: &str| {
                let mut request = pane_url
                    .as_str()
                    .into_client_request()
                    .expect("agent pane WebSocket request");
                request.headers_mut().insert(
                    AUTHORIZATION,
                    format!("Bearer {token}")
                        .parse()
                        .expect("bearer header value"),
                );
                request
            };

            let (mut original_socket, _) = connect_async(connect(&original.token))
                .await
                .expect("original agent pane WebSocket");
            original_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready on original socket");
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if !events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("original ready dispatch");
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();

            let current = issuer
                .issue(project.path(), "session-1")
                .expect("rotated capability");
            original_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready after rotation");
            let _ = tokio::time::timeout(Duration::from_secs(1), original_socket.next())
                .await
                .expect("rotated socket must be closed by the server");
            assert!(
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_empty(),
                "a rotated socket must not enqueue an AgentFrontend event"
            );

            let (mut current_socket, _) = connect_async(connect(&current.token))
                .await
                .expect("current agent pane WebSocket");
            current_socket
                .send(WebSocketMessage::Text(ready.clone().into()))
                .await
                .expect("send ready on current socket");
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if !events
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_empty()
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("current ready dispatch");
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();

            assert!(issuer.revoke_token(&current.token));
            current_socket
                .send(WebSocketMessage::Text(ready.into()))
                .await
                .expect("send ready after revoke");
            let _ = tokio::time::timeout(Duration::from_secs(1), current_socket.next())
                .await
                .expect("revoked socket must be closed by the server");
            assert!(
                events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_empty(),
                "a revoked socket must not enqueue an AgentFrontend event"
            );
        });

        server.shutdown();
    }

    #[test]
    fn accepted_self_close_makes_grant_non_current_until_ticket_finishes() {
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = super::AgentCapabilityIssuer::for_test(
            "http://127.0.0.1:1/internal/hook-live",
            "ws://127.0.0.1:1/ws",
            "ws://127.0.0.1:2/internal/pane-ws",
        );
        let original = issuer
            .issue(project.path(), "session-1")
            .expect("original capability");
        let grant = issuer
            .grant_for_test(&original.token)
            .expect("current grant");

        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("begin self-close");
        assert!(!issuer.grant_is_current(&grant));
        assert!(!issuer.authenticates_token(&original.token));
        assert!(
            issuer.issue(project.path(), "session-1").is_err(),
            "the same principal cannot reissue while its close ticket is pending"
        );

        assert!(issuer.rollback_self_close(&ticket));
        assert!(issuer.grant_is_current(&grant));
        assert!(issuer.authenticates_token(&original.token));

        let ticket = issuer
            .begin_self_close_if_current(&grant)
            .expect("begin accepted self-close");
        assert!(issuer.revoke_token(&original.token));
        assert!(
            !issuer.rollback_self_close(&ticket),
            "an independently revoked closing grant must never become active again"
        );
        assert!(!issuer.grant_is_current(&grant));

        let replacement = issuer
            .issue(project.path(), "session-1")
            .expect("reissue after revoked ticket clears");
        let replacement_grant = issuer
            .grant_for_test(&replacement.token)
            .expect("replacement grant");
        let ticket = issuer
            .begin_self_close_if_current(&replacement_grant)
            .expect("begin replacement self-close");
        assert!(issuer.finish_self_close(&ticket));
        assert!(
            !issuer.finish_self_close(&ticket),
            "ticket replay must be a no-op"
        );
        assert!(issuer.issue(project.path(), "session-1").is_ok());
    }

    #[test]
    fn agent_bridge_uses_capability_only_listener_and_rejects_stale_or_foreign_tokens() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let foreign_project = tempfile::tempdir().expect("foreign project tempdir");
        let issuer = server.agent_capability_issuer();
        let pane_websocket_url = issuer.pane_websocket_url().to_string();
        let stale = issuer
            .issue(project.path(), "session-1")
            .expect("stale target");
        let current = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign = issuer
            .issue(foreign_project.path(), "session-2")
            .expect("foreign target");
        let client = reqwest::blocking::Client::new();

        assert_ne!(
            reqwest::Url::parse(server.url())
                .expect("browser URL")
                .port_or_known_default(),
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .port_or_known_default(),
        );
        assert_eq!(
            reqwest::Url::parse(&pane_websocket_url)
                .expect("pane WebSocket URL")
                .port_or_known_default(),
            reqwest::Url::parse(server.url())
                .expect("browser URL")
                .port_or_known_default(),
        );
        assert_ne!(
            reqwest::Url::parse(&pane_websocket_url)
                .expect("pane WebSocket URL")
                .port_or_known_default(),
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .port_or_known_default(),
        );
        assert_eq!(
            reqwest::Url::parse(&current.url)
                .expect("agent URL")
                .host_str(),
            Some("127.0.0.1")
        );

        let agent_health = client
            .get(
                reqwest::Url::parse(&current.url)
                    .expect("agent URL")
                    .join("/healthz")
                    .expect("agent health URL"),
            )
            .send()
            .expect("agent health request");
        assert_eq!(agent_health.status(), HttpStatusCode::NOT_FOUND);

        let browser_hook = client
            .post(format!("{}internal/hook-live", server.url()))
            .json(&sample_runtime_hook_event())
            .send()
            .expect("browser hook request");
        assert_eq!(browser_hook.status(), HttpStatusCode::NOT_FOUND);

        let stale_response = client
            .post(&stale.url)
            .bearer_auth(&stale.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("stale hook request");
        assert_eq!(stale_response.status(), HttpStatusCode::UNAUTHORIZED);

        let foreign_response = client
            .post(&foreign.url)
            .bearer_auth(&foreign.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("foreign hook request");
        assert_eq!(foreign_response.status(), HttpStatusCode::UNAUTHORIZED);

        let accepted = client
            .post(&current.url)
            .bearer_auth(&current.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("current hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let [UserEvent::RuntimeHook(recorded_event)] = recorded.as_slice() else {
            panic!("only the current matching capability should dispatch: {recorded:?}");
        };
        let canonical_project = dunce::canonicalize(project.path())
            .expect("canonical project")
            .to_string_lossy()
            .into_owned();
        assert_eq!(recorded_event.gwt_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            recorded_event.project_root.as_deref(),
            Some(canonical_project.as_str())
        );

        drop(recorded);
        server.shutdown();
    }

    // SPEC #3248 FR-242 / Issue #4546 AC-1: the preflight is authenticated,
    // reachable only on the agent router, and — unlike every other agent
    // route — answerable to a principal that holds no execution authority
    // yet. That last property is the whole point: the contract has to be
    // provable *before* a generation exists, or the launch mints one first
    // and discovers the mismatch afterwards.
    #[test]
    fn host_contract_route_authenticates_without_demanding_execution_authority() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let issuer = server.agent_capability_issuer();
        // An Inspection principal: authenticated, but carrying no execution
        // binding — exactly the state a launch is in before genesis.
        let inspection = issuer
            .issue(project.path(), "session-preflight")
            .expect("inspection capability");
        let mut host_contract_url = reqwest::Url::parse(&inspection.url).expect("agent hook URL");
        host_contract_url.set_path("/internal/host-contract");
        let request = serde_json::json!({
            "schema_version": gwt::AGENT_HOST_CONTRACT_SCHEMA_VERSION,
            "operation_id": "host-contract:genesis",
            "nonce": "nonce-route",
        });
        let client = reqwest::blocking::Client::new();

        // The browser surface must not expose it at all.
        let browser_response = client
            .post(format!("{}internal/host-contract", server.url()))
            .json(&request)
            .send()
            .expect("browser host-contract request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        // An unauthenticated caller learns nothing.
        let anonymous = client
            .post(host_contract_url.clone())
            .json(&request)
            .send()
            .expect("anonymous host-contract request");
        assert_eq!(anonymous.status(), HttpStatusCode::UNAUTHORIZED);

        let response = client
            .post(host_contract_url.clone())
            .bearer_auth(&inspection.token)
            .json(&request)
            .send()
            .expect("authenticated host-contract request");
        assert_eq!(response.status(), HttpStatusCode::OK);
        let receipt = response
            .json::<gwt::AgentHostContractReceipt>()
            .expect("host contract receipt");
        assert_eq!(receipt.operation_id, "host-contract:genesis");
        assert_eq!(receipt.nonce, "nonce-route");
        assert_eq!(receipt.session_id, "session-preflight");
        assert_eq!(
            receipt.execution_generation_contract_version,
            gwt::EXECUTION_GENERATION_CONTRACT_VERSION
        );
        assert!(!receipt.host_instance_id.trim().is_empty());
        assert_eq!(receipt.host_version, env!("CARGO_PKG_VERSION"));
        // No generation exists yet, so the capability generation is zero —
        // and that must not read as a defective receipt.
        assert_eq!(receipt.capability_generation, 0);

        // A malformed question is refused without inventing an answer.
        let malformed = client
            .post(host_contract_url)
            .bearer_auth(&inspection.token)
            .json(&serde_json::json!({
                "schema_version": gwt::AGENT_HOST_CONTRACT_SCHEMA_VERSION,
                "operation_id": "",
                "nonce": "nonce-route",
            }))
            .send()
            .expect("malformed host-contract request");
        assert_eq!(malformed.status(), HttpStatusCode::BAD_REQUEST);

        server.shutdown();
    }

    #[test]
    fn workspace_update_route_authenticates_before_host_mutation_service() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let foreign_project = tempfile::tempdir().expect("foreign project tempdir");
        let issuer = server.agent_capability_issuer();
        let stale = issuer
            .issue(project.path(), "session-1")
            .expect("stale target");
        let current = issuer
            .issue(project.path(), "session-1")
            .expect("current target");
        let foreign = AgentCapabilityIssuer::new(
            current.url.clone(),
            issuer.pane_websocket_url().to_string(),
            issuer.agent_pane_websocket_url().to_string(),
            AgentCapabilityRegistry::default(),
        )
        .issue(foreign_project.path(), "session-1")
        .expect("foreign-registry target");
        let mut workspace_update_url = reqwest::Url::parse(&current.url).expect("agent hook URL");
        workspace_update_url.set_path("/internal/workspace-update");
        let request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "different-session",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "intent": {}
        });
        let client = reqwest::blocking::Client::new();

        let browser_response = client
            .post(format!("{}internal/workspace-update", server.url()))
            .json(&request)
            .send()
            .expect("browser workspace-update request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        for (case, token) in [
            ("missing", None),
            ("stale", Some(stale.token.as_str())),
            ("foreign", Some(foreign.token.as_str())),
        ] {
            let mut request_builder = client.post(workspace_update_url.clone()).json(&request);
            if let Some(token) = token {
                request_builder = request_builder.bearer_auth(token);
            }
            let response = request_builder
                .send()
                .unwrap_or_else(|error| panic!("{case} workspace-update request: {error}"));
            assert_eq!(
                response.status(),
                HttpStatusCode::UNAUTHORIZED,
                "{case} bearer must be rejected before Host mutation"
            );
            let body = response.text().expect("unauthorized response body");
            assert!(!body.contains(&stale.token));
            assert!(!body.contains(&foreign.token));
        }

        let current_response = client
            .post(workspace_update_url)
            .bearer_auth(&current.token)
            .json(&request)
            .send()
            .expect("current workspace-update request");
        assert_eq!(current_response.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = current_response
            .json()
            .expect("Host mutation service error body");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Execution binding")));
        assert!(!error.to_string().contains(&current.token));
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_route_rejects_inspection_principal_without_mutation() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let target = server
            .agent_capability_issuer()
            .issue(project.path(), "session-inspection")
            .expect("inspection target");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/execution-binding-probe");
        let request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-inspection",
            "nonce": "nonce-inspection"
        });
        let client = reqwest::blocking::Client::new();

        let response = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .expect("inspection binding probe");

        assert_eq!(response.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = response.json().expect("binding probe error");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert_eq!(error["reason"], "authority_mismatch");
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_route_rejects_prepared_authority_until_activation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repository");
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/prepared-probe.git",
            ],
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(&args)
                .current_dir(&repo)
                .output()
                .expect("run fixture git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let repo = dunce::canonicalize(repo).expect("canonical repository");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        let completed_at = chrono::Utc::now();
        gwt::cli::execution_state::save(
            &repo,
            &gwt::cli::execution_state::ExecutionControlRecord {
                owner_kind: owner.kind,
                owner_number: owner.number,
                primary_session_id: "session-predecessor".to_string(),
                entrypoint: "gwt-execute".to_string(),
                bundled_required_owners: Vec::new(),
                status: gwt::cli::execution_state::ExecutionControlStatus::Completed,
                blocked_reason: None,
                missing_verification: None,
                launched_at: completed_at,
                settled_at: Some(completed_at),
                completion_evidence: None,
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
                permission_decision: None,
            },
        )
        .expect("save completed predecessor");
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Unknown,
        )
        .expect("import completed predecessor");
        let continuation_session_id = "session-prepared-probe";
        let request = gwt::cli::execution_state::SuccessorRequest {
            operation_id: "operation-prepared-probe".to_string(),
            principal_id: "host-prepared-probe".to_string(),
            work_id: Some("work-prepared-probe".to_string()),
            source: "continue-work".to_string(),
            session_binding_id: "binding-prepared-probe".to_string(),
            initial_session_id: continuation_session_id.to_string(),
            entrypoint: "resume".to_string(),
            requested_at: chrono::Utc::now(),
        };
        gwt::cli::execution_state::prepare_successor(&repo, owner, &request)
            .expect("prepare successor");
        let planned_identity =
            gwt::cli::execution_state::prepared_successor_execution_binding(&repo, owner, &request)
                .expect("derive Prepared binding");
        let mut session =
            gwt_agent::Session::new(&repo, "work/prepared-probe", gwt_agent::AgentId::Codex);
        session.id = continuation_session_id.to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(owner.number);
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: planned_identity.clone(),
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Prepared Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist Prepared Session");

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let target = server
            .agent_capability_issuer()
            .issue_prepared(&repo, continuation_session_id, binding.clone())
            .expect("Prepared Host capability");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/execution-binding-probe");
        let response = reqwest::blocking::Client::new()
            .post(url)
            .bearer_auth(&target.token)
            .json(&serde_json::json!({
                "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
                "operation_id": "operation-prepared-probe",
                "nonce": "nonce-prepared-probe"
            }))
            .send()
            .expect("Prepared binding probe");

        assert_eq!(
            response.status(),
            HttpStatusCode::CONFLICT,
            "the agent-facing mutation probe must require Active authority",
        );
        assert!(
            gwt::cli::execution_state::prepared_execution_binding_matches(
                &repo,
                owner,
                continuation_session_id,
                &binding.identity,
            )
            .expect("Prepared authority remains pending")
        );
        assert_eq!(
            gwt::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("read generation ledger")
                .expect("generation ledger")
                .current_effective_status(),
            Some(gwt::cli::execution_state::ExecutionControlStatus::Completed),
            "an HTTP probe must not activate the successor"
        );
        assert!(
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "the probe is side-effect-free at the runtime dispatch boundary"
        );
        server.shutdown();
    }

    #[test]
    fn execution_adoption_synchronizes_host_binding_before_terminal_work_update() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
            vec!["checkout", "-b", "work/adoption"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/adoption.git",
            ],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            assert!(gwt_core::process::run_git_logged(&args, Some(&repo))
                .unwrap()
                .status
                .success());
        }
        let repo = dunce::canonicalize(repo).unwrap();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 4278,
        };
        let mut session =
            gwt_agent::Session::new(&repo, "work/adoption", gwt_agent::AgentId::Codex);
        session.id = "adoption-successor".into();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(owner.number);
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            "adoption-predecessor",
            "gwt-execute",
            false,
        )
        .unwrap();
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let before = gwt_agent::SessionExecutionBinding {
            schema_version: 1,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().unwrap(),
            owner_kind: "issue".into(),
            owner_number: owner.number,
            identity: gwt::cli::execution_state::current_execution_binding(&repo, owner)
                .unwrap()
                .unwrap(),
            capability_generation: 1,
        };
        session.set_execution_binding(Some(before.clone())).unwrap();
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        let runtime = Runtime::new().unwrap();
        let (proxy, _) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .unwrap();
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue_bound(&repo, &session.id, before.clone())
            .unwrap();
        let _bridge_url = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_URL_ENV, &target.url);
        let _bridge_token = ScopedEnvVar::set(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV, &target.token);
        let mut env = gwt::cli::TestEnv::new(repo.clone());
        let code = gwt::cli::run(
            &mut env,
            gwt::cli::CliCommand::Execution(gwt::cli::execution_state::ExecutionCommand::Adopt {
                reason: "recover the previous execution".into(),
            }),
        )
        .unwrap();
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&env.stdout));
        let client = reqwest::blocking::Client::new();
        let mut url = reqwest::Url::parse(&target.url).unwrap();
        url.set_path("/internal/execution-adoption");
        let request = serde_json::json!({"schema_version":1, "claimed_session_id":session.id, "reason":"recover the previous execution"});
        let adopted = gwt_agent::Session::load(
            &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
        )
        .unwrap();
        let after = adopted.execution_binding.unwrap();
        assert_ne!(after.identity, before.identity);
        assert_eq!(
            after.capability_generation,
            before.capability_generation + 1
        );
        assert_eq!(
            issuer.active_execution_binding_for_token(&target.token),
            Some(after.clone())
        );
        let replay = client
            .post(url.clone())
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .unwrap();
        assert_eq!(
            replay.status(),
            HttpStatusCode::OK,
            "{}",
            replay.text().unwrap()
        );
        assert_eq!(
            issuer.active_execution_binding_for_token(&target.token),
            Some(after.clone())
        );

        let result = gwt::cli::run(
            &mut env,
            gwt::cli::CliCommand::Workspace(gwt::cli::WorkspaceCommand::Ensure {
                agent_session: session.id.clone(),
                title_summary: "Host adoption synchronization".into(),
                current_focus: None,
                spec: None,
                issue: Some(owner.number),
                topic: None,
                boundary: None,
            }),
        )
        .unwrap();
        assert_eq!(result, 0, "{}", String::from_utf8_lossy(&env.stdout));
        url.set_path("/internal/workspace-update");
        let response = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&serde_json::json!({
                "schema_version":1, "claimed_session_id":session.id,
                "observation":gwt::observe_agent_runtime(&repo).unwrap(),
                "intent":{"status_category":"done"}
            }))
            .send()
            .unwrap();
        assert_eq!(
            response.status(),
            HttpStatusCode::OK,
            "{}",
            response.text().unwrap()
        );
        server.shutdown();
    }

    /// Issue #4443 AC-10 / AC-2: no request may drive the adoption bridge to
    /// `http_status=500 code=internal`, the shape the PM recorded when #3697's
    /// 409 turned into a 500 in the same session. A caller-input refusal answers
    /// `400 invalid_request`, and a state refusal answers `409` carrying the
    /// operation the agent can actually run.
    #[test]
    fn execution_adoption_refuses_without_an_internal_server_error() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _profile = ScopedEnvVar::set("USERPROFILE", home.path());
        let _runtime_path = ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV);
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
            vec!["checkout", "-b", "work/legacy-adoption"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/legacy-adoption.git",
            ],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            assert!(gwt_core::process::run_git_logged(&args, Some(&repo))
                .unwrap()
                .status
                .success());
        }
        let repo = dunce::canonicalize(repo).unwrap();
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 4443,
        };
        let mut session =
            gwt_agent::Session::new(&repo, "work/legacy-adoption", gwt_agent::AgentId::Codex);
        session.id = "legacy-adoption-successor".into();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(owner.number);
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            "legacy-adoption-predecessor",
            "gwt-execute",
            false,
        )
        .unwrap();
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: 1,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().unwrap(),
            owner_kind: "issue".into(),
            owner_number: owner.number,
            identity: gwt::cli::execution_state::current_execution_binding(&repo, owner)
                .unwrap()
                .unwrap(),
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .unwrap();
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        // The dead host left the flat record behind without its owner
        // generation ledger: the state the relaunched Session inherits.
        let trusted = gwt::cli::trusted_store::trusted_dir_for_worktree(&repo)
            .expect("trusted store for the fixture worktree");
        let owners = trusted
            .parent()
            .expect("trusted store root")
            .join("execution-owners");
        assert!(owners.is_dir(), "fixture never wrote an owner ledger");
        std::fs::remove_dir_all(&owners).unwrap();

        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, &session.id);
        let runtime = Runtime::new().unwrap();
        let (proxy, _) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .unwrap();
        let issuer = server.agent_capability_issuer();
        let target = issuer
            .issue_bound(&repo, &session.id, binding.clone())
            .unwrap();
        let client = reqwest::blocking::Client::new();
        let mut url = reqwest::Url::parse(&target.url).unwrap();
        url.set_path("/internal/execution-adoption");
        let adopt = |reason: &str| {
            let response = client
                .post(url.clone())
                .bearer_auth(&target.token)
                .json(&serde_json::json!({
                    "schema_version": 1,
                    "claimed_session_id": session.id,
                    "reason": reason,
                }))
                .send()
                .unwrap();
            (response.status(), response.text().unwrap())
        };

        // A reserved-namespace reason is a caller input error. It used to reach
        // the CLI guard and come back as an opaque `500 code=internal`.
        let (status, body) = adopt("gwt:execution-recovery:v1:forged");
        assert_ne!(
            status,
            HttpStatusCode::INTERNAL_SERVER_ERROR,
            "a caller input error answered as an unhandled exception: {body}"
        );
        assert_eq!(status, HttpStatusCode::BAD_REQUEST, "{body}");

        // The inherited record refuses on state, and the refusal must name the
        // operation this agent can run to get out of it.
        let (status, body) = adopt("recover the dead host's record");
        assert_ne!(
            status,
            HttpStatusCode::INTERNAL_SERVER_ERROR,
            "a state refusal answered as an unhandled exception: {body}"
        );
        assert_eq!(status, HttpStatusCode::CONFLICT, "{body}");
        let refusal = serde_json::from_str::<serde_json::Value>(&body).unwrap();
        assert_eq!(
            refusal["recovery_operations"],
            serde_json::json!(["execution.repair"]),
            "the refusal names no recovery operation: {body}"
        );

        // AC-2: the route out must survive the agent-side bridge client, which
        // previously reported only `code=` and `bridge_reason=`.
        let bridge_target = gwt::HookForwardTarget {
            url: target.url.clone(),
            token: target.token.clone(),
        };
        let bridged = gwt::daemon_runtime::send_execution_adoption_via_agent_bridge(
            &bridge_target,
            &gwt::AgentExecutionAdoptionRequest {
                schema_version: 1,
                claimed_session_id: session.id.clone(),
                reason: "recover the dead host's record".into(),
            },
            &session,
        )
        .expect_err("the inherited record refuses adoption");
        assert!(
            bridged.contains("execution.repair"),
            "the agent-visible refusal dropped the Host's recovery route: {bridged}"
        );
        assert!(
            !binding.identity.generation_id.is_empty(),
            "fixture binding is structurally valid"
        );
        server.shutdown();
    }

    #[test]
    fn execution_binding_probe_fences_an_older_host_with_the_durable_capability_epoch() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = tempfile::tempdir().expect("fixture root");
        let repo = fixture.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repository fixture");
        for args in [
            vec!["init"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
            vec!["checkout", "-b", "work/execution-binding-probe"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/execution-binding-probe.git",
            ],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let output =
                gwt_core::process::run_git_logged(&args, Some(&repo)).expect("run fixture git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let repo = dunce::canonicalize(repo).expect("canonical repository fixture");
        let mut session = gwt_agent::Session::new(
            &repo,
            "work/execution-binding-probe",
            gwt_agent::AgentId::Codex,
        );
        session.id = "session-two-host".to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(2359);
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save durable Session");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 2359,
        };
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            &session.id,
            "gwt-execute",
            false,
        )
        .expect("materialize execution projection");
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize owner ledger");
        let identity = gwt::cli::execution_state::current_execution_binding(&repo, owner)
            .expect("read current binding")
            .expect("active generation binding");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session
                .repo_hash
                .clone()
                .expect("Session repository identity"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity,
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session to active generation");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("persist initial execution binding");
        let work_id = "work-materialization-probe-route";
        let now = chrono::Utc::now();
        let mut projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        projection.agents = vec![gwt_core::workspace_projection::WorkspaceAgentSummary {
            session_id: session.id.clone(),
            window_id: Some("project::agent-materialization-probe".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: Some(repo.clone()),
            branch: Some(session.branch.clone()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status:
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: Some(work_id.to_string()),
            updated_at: now,
        }];
        gwt_core::workspace_projection::save_workspace_projection(&repo, &projection)
            .expect("save materialization probe assignment");
        let mut work_items =
            gwt_core::workspace_projection::WorkItemsProjection::empty(chrono::Utc::now());
        let mut work_event = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id,
            now,
        );
        work_event.title = Some("Materialization probe route".to_string());
        work_event.owner = Some("Issue #2359".to_string());
        work_event.agent_id = Some("codex".to_string());
        work_event.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        work_event.agent_session_id = Some(session.id.clone());
        work_event.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(session.branch.clone()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        work_items.apply_event(work_event);
        let work_items_path = gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &work_items_path,
            &work_items,
        )
        .expect("save materialized Work");

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy_a, events_a) = AppEventProxy::stub();
        let mut server_a = EmbeddedServer::start(
            &runtime,
            proxy_a,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("first Host");
        let target_a = server_a
            .agent_capability_issuer()
            .issue_bound(&repo, &session.id, binding)
            .expect("first Host binding");
        let pane_url_a = server_a
            .agent_capability_issuer()
            .agent_pane_websocket_url()
            .to_string();
        let mut pane_request_a = pane_url_a
            .as_str()
            .into_client_request()
            .expect("old Host pane request");
        pane_request_a.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_a.token)
                .parse()
                .expect("old Host bearer"),
        );
        let (mut old_host_socket, _) = runtime
            .block_on(connect_async(pane_request_a))
            .expect("old Host socket is current before rotation");

        let (proxy_b, events_b) = AppEventProxy::stub();
        let mut server_b = EmbeddedServer::start(
            &runtime,
            proxy_b,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("second Host");
        let rotated = gwt_agent::rotate_session_execution_capability(
            &gwt_core::paths::gwt_sessions_dir(),
            &session.id,
        )
        .expect("rotate durable Host epoch");
        let target_b = server_b
            .agent_capability_issuer()
            .issue_bound(&repo, &session.id, rotated.clone())
            .expect("second Host binding");

        runtime.block_on(async {
            old_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "must-not-dispatch"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input through old Host socket");
            let close = tokio::time::timeout(Duration::from_secs(2), old_host_socket.next())
                .await
                .expect("old Host socket must be fenced")
                .expect("old Host close frame")
                .expect("valid old Host close frame");
            let WebSocketMessage::Close(Some(close)) = close else {
                panic!("old Host socket must receive an explicit policy close");
            };
            assert_eq!(u16::from(close.code), 1008);
            assert_eq!(close.reason, "execution binding is no longer current");
            assert!(!close.reason.contains(&target_a.token));
            assert!(!close.reason.contains(&rotated.identity.binding_id));
            assert!(!close.reason.contains(repo.to_string_lossy().as_ref()));
        });
        assert!(
            events_a
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "old Host input must be rejected before AgentFrontend dispatch"
        );

        let mut stale_handshake = pane_url_a
            .as_str()
            .into_client_request()
            .expect("stale Host pane request");
        stale_handshake.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_a.token)
                .parse()
                .expect("stale Host bearer"),
        );
        // Issue #3667: a durably stale capability keeps the observation
        // transport, so the handshake upgrades; producing mutation on that
        // socket is still fenced by the per-request durable check.
        let (mut stale_socket, _) = runtime
            .block_on(connect_async(stale_handshake))
            .expect("durably stale Host capability keeps the observation transport");
        runtime.block_on(async {
            stale_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "must-not-dispatch-after-reconnect"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input through the stale reconnect socket");
            let close = tokio::time::timeout(Duration::from_secs(2), stale_socket.next())
                .await
                .expect("stale reconnect socket must be fenced")
                .expect("stale reconnect close frame")
                .expect("valid stale reconnect close frame");
            let WebSocketMessage::Close(Some(close)) = close else {
                panic!("stale reconnect socket must receive an explicit policy close");
            };
            assert_eq!(u16::from(close.code), 1008);
            assert_eq!(close.reason, "execution binding is no longer current");
        });
        assert!(
            events_a
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "stale reconnect input must be rejected before AgentFrontend dispatch"
        );

        let pane_url_b = server_b
            .agent_capability_issuer()
            .agent_pane_websocket_url()
            .to_string();
        let mut pane_request_b = pane_url_b
            .as_str()
            .into_client_request()
            .expect("current Host pane request");
        pane_request_b.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_b.token)
                .parse()
                .expect("current Host bearer"),
        );
        runtime.block_on(async {
            let (mut current_host_socket, _) = connect_async(pane_request_b)
                .await
                .expect("current Host socket upgrades");
            current_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "current-dispatch"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input through current Host socket");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if events_b
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .iter()
                        .any(|event| matches!(event, UserEvent::AgentFrontend { .. }))
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("current Host input reaches runtime dispatch queue");
            current_host_socket
                .close(None)
                .await
                .expect("close current Host socket");
        });

        let request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-two-host",
            "nonce": "nonce-two-host"
        });
        let client = reqwest::blocking::Client::new();
        let probe = |target: &HookForwardTarget| {
            let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
            url.set_path("/internal/execution-binding-probe");
            client
                .post(url)
                .bearer_auth(&target.token)
                .json(&request)
                .send()
                .expect("binding probe request")
        };

        let stale = probe(&target_a);
        assert_eq!(stale.status(), HttpStatusCode::CONFLICT);
        let diagnostic: serde_json::Value = stale.json().expect("stale binding diagnostic");
        assert_eq!(
            diagnostic["diagnostic_reason"],
            "session_binding_identity_mismatch"
        );
        assert_eq!(
            diagnostic["mismatched_fields"],
            serde_json::json!(["capability_generation"])
        );
        let session_path = gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
        let before_adopt = std::fs::read(&session_path).unwrap();
        let mut adoption_url = reqwest::Url::parse(&target_a.url).unwrap();
        adoption_url.set_path("/internal/execution-adoption");
        let rejected_adopt = client.post(adoption_url).bearer_auth(&target_a.token)
            .json(&serde_json::json!({"schema_version":1,"claimed_session_id":session.id,"reason":"stale Host must not recover itself"}))
            .send().unwrap();
        assert_eq!(rejected_adopt.status(), HttpStatusCode::CONFLICT);
        assert_eq!(std::fs::read(&session_path).unwrap(), before_adopt);
        let current = probe(&target_b);
        assert_eq!(current.status(), HttpStatusCode::OK);
        let receipt: gwt::AgentExecutionBindingProbeReceipt =
            current.json().expect("current Host receipt");
        assert_eq!(receipt.execution_binding, rotated.identity);
        assert_eq!(receipt.capability_generation, rotated.capability_generation);
        assert!(!receipt.host_instance_id.trim().is_empty());

        let materialization_request = gwt::AgentWorkMaterializationProbeRequest {
            schema_version: gwt::AGENT_WORK_MATERIALIZATION_PROBE_SCHEMA_VERSION,
            claimed_session_id: session.id.clone(),
            owner_number: 2359,
            observation: gwt::observe_agent_runtime(&repo).expect("runtime observation"),
        };
        let materialization_probe = |target: &HookForwardTarget| {
            let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
            url.set_path("/internal/work-materialization-probe");
            client
                .post(url)
                .bearer_auth(&target.token)
                .json(&materialization_request)
                .send()
                .expect("Work materialization probe request")
        };
        let projection_path = gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&repo);
        let projection_before =
            std::fs::read(&projection_path).expect("projection before route probes");
        let work_items_before =
            std::fs::read(&work_items_path).expect("WorkItems before route probes");
        let stale_materialization = materialization_probe(&target_a);
        assert_eq!(stale_materialization.status(), HttpStatusCode::CONFLICT);
        let current_materialization = materialization_probe(&target_b);
        assert_eq!(current_materialization.status(), HttpStatusCode::OK);
        let materialization_receipt: gwt::AgentWorkMaterializationProbeReceipt =
            current_materialization
                .json()
                .expect("current Work materialization receipt");
        assert_eq!(materialization_receipt.owner_number, 2359);
        assert_eq!(materialization_receipt.work_id, work_id);
        assert_eq!(
            std::fs::read(&projection_path).expect("projection after route probes"),
            projection_before,
            "materialization route must not mutate Workspace projection"
        );
        assert_eq!(
            std::fs::read(&work_items_path).expect("WorkItems after route probes"),
            work_items_before,
            "materialization route must not mutate WorkItems"
        );
        let missing_work_items =
            gwt_core::workspace_projection::WorkItemsProjection::empty(chrono::Utc::now());
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &work_items_path,
            &missing_work_items,
        )
        .expect("remove assigned Work from route fixture");
        let missing_before =
            std::fs::read(&work_items_path).expect("missing WorkItems before rejection");
        let missing_materialization = materialization_probe(&target_b);
        assert_eq!(missing_materialization.status(), HttpStatusCode::CONFLICT);
        let missing_error: serde_json::Value = missing_materialization
            .json()
            .expect("missing Work rejection wire response");
        let missing_code: gwt::AgentWorkspaceUpdateErrorCode =
            serde_json::from_value(missing_error["code"].clone())
                .expect("typed missing Work error code");
        assert_eq!(
            missing_code,
            gwt::AgentWorkspaceUpdateErrorCode::WorkspaceEnsureRequired
        );
        assert_eq!(missing_error["reason"], "workspace_ensure_required");
        assert!(missing_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("workspace.ensure")));
        assert_eq!(
            missing_error["recovery_operations"],
            serde_json::json!(["workspace.ensure"])
        );
        assert_eq!(
            std::fs::read(&work_items_path).expect("WorkItems after missing rejection"),
            missing_before,
            "missing Work rejection must not mutate WorkItems"
        );

        let dispatched_before_corruption = events_b
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let mut corrupt_session_request = pane_url_b
            .as_str()
            .into_client_request()
            .expect("corrupt Session pane request");
        corrupt_session_request.headers_mut().insert(
            AUTHORIZATION,
            format!("Bearer {}", target_b.token)
                .parse()
                .expect("current Host bearer"),
        );
        runtime.block_on(async {
            let (mut current_host_socket, _) = connect_async(corrupt_session_request)
                .await
                .expect("current Host socket upgrades before Session corruption");
            std::fs::write(
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
                "{",
            )
            .expect("corrupt durable Session fixture");
            current_host_socket
                .send(WebSocketMessage::Text(
                    serde_json::json!({
                        "kind": "pane_send_input",
                        "session_id": &session.id,
                        "text": "must-not-dispatch-when-authority-is-unavailable"
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect("send input after Session corruption");
            let close = tokio::time::timeout(Duration::from_secs(2), current_host_socket.next())
                .await
                .expect("current Host socket must fail closed")
                .expect("current Host close frame")
                .expect("valid current Host close frame");
            let WebSocketMessage::Close(Some(close)) = close else {
                panic!("unknown durable authority must receive an explicit internal-error close");
            };
            assert_eq!(u16::from(close.code), 1011);
            assert_eq!(close.reason, "execution authority is unavailable");
            assert!(!close.reason.contains(&target_b.token));
            assert!(!close.reason.contains(&session.id));
            assert!(!close.reason.contains(repo.to_string_lossy().as_ref()));
        });
        assert_eq!(
            events_b
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            dispatched_before_corruption,
            "corrupt durable authority must be rejected before AgentFrontend dispatch"
        );

        server_a.shutdown();
        server_b.shutdown();
    }

    #[test]
    fn work_terminalization_route_authenticates_before_host_mutation_service() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let target = server
            .agent_capability_issuer()
            .issue(project.path(), "session-1")
            .expect("terminalization target");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/work-terminalization");
        let request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "different-session",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "terminal_kind": "done"
        });
        let client = reqwest::blocking::Client::new();

        let browser_response = client
            .post(format!("{}internal/work-terminalization", server.url()))
            .json(&request)
            .send()
            .expect("browser terminalization request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        let unauthorized = client
            .post(url.clone())
            .json(&request)
            .send()
            .expect("unauthorized terminalization request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let authenticated = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .expect("authenticated terminalization request");
        assert_eq!(authenticated.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = authenticated
            .json()
            .expect("terminalization service error body");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(error["message"]
            .as_str()
            .is_some_and(|message| message.contains("Execution binding")));
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
    }

    #[test]
    fn blocked_build_abort_route_is_dedicated_and_authenticates_before_mutation() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let project = tempfile::tempdir().expect("project tempdir");
        let _gwt_home = gwt_core::test_support::ScopedGwtHome::set(project.path());
        let target = server
            .agent_capability_issuer()
            .issue(project.path(), "session-1")
            .expect("build abort target");
        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/build-abort-terminalization");
        let request = serde_json::json!({
            "schema_version": gwt::AGENT_BUILD_ABORT_TERMINALIZATION_SCHEMA_VERSION,
            "claimed_session_id": "session-1",
            "owner_number": 3580,
            "reason": "canonical verification cannot proceed",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            }
        });
        let client = reqwest::blocking::Client::new();

        let browser_response = client
            .post(format!(
                "{}internal/build-abort-terminalization",
                server.url()
            ))
            .json(&request)
            .send()
            .expect("browser build abort request");
        assert_eq!(browser_response.status(), HttpStatusCode::NOT_FOUND);

        let unauthorized = client
            .post(url.clone())
            .json(&request)
            .send()
            .expect("unauthorized build abort request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let authenticated = client
            .post(url)
            .bearer_auth(&target.token)
            .json(&request)
            .send()
            .expect("authenticated build abort request");
        assert_eq!(authenticated.status(), HttpStatusCode::CONFLICT);
        let error: serde_json::Value = authenticated.json().expect("build abort error body");
        assert_eq!(error["code"], "execution_binding_mismatch");
        assert!(events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());

        server.shutdown();
    }

    #[test]
    fn blocked_build_abort_route_discards_exact_bound_work_under_terminal_execution() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repository");
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test User"],
            vec!["checkout", "-b", "work/blocked-build-abort"],
            vec![
                "remote",
                "add",
                "origin",
                "https://example.invalid/acme/blocked-build-abort.git",
            ],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            let output =
                gwt_core::process::run_git_logged(&args, Some(&repo)).expect("run fixture git");
            assert!(output.status.success(), "git {args:?} failed");
        }
        let repo = dunce::canonicalize(repo).expect("canonical repository");
        let owner = gwt::cli::execution_state::ExecutionOwnerKey {
            kind: gwt::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 3580,
        };
        let mut session =
            gwt_agent::Session::new(&repo, "work/blocked-build-abort", gwt_agent::AgentId::Codex);
        session.id = "session-blocked-build-abort-http".to_string();
        session.project_state_root = Some(repo.clone());
        session.linked_issue_number = Some(owner.number);
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save unbound Session");
        gwt::cli::execution_state::materialize_at_launch(
            &repo,
            owner.kind,
            owner.number,
            &session.id,
            "gwt-execute",
            false,
        )
        .expect("materialize execution control");
        gwt::cli::execution_state::ensure_generation_ledger(
            &repo,
            owner,
            gwt::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize generation ledger");
        let binding = gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().expect("repository hash"),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: gwt::cli::execution_state::current_execution_binding(&repo, owner)
                .expect("read active binding")
                .expect("active binding"),
            capability_generation: 1,
        };
        session
            .set_execution_binding(Some(binding.clone()))
            .expect("bind Session");
        session
            .save(&gwt_core::paths::gwt_sessions_dir())
            .expect("save bound Session");

        let work_id = "work-blocked-build-abort-http";
        let now = chrono::Utc::now();
        let mut current =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&repo);
        current
            .agents
            .push(gwt_core::workspace_projection::WorkspaceAgentSummary {
                session_id: session.id.clone(),
                window_id: Some("project::blocked-build-abort-http".to_string()),
                agent_id: session.agent_id.command().to_string(),
                display_name: session.agent_id.display_name().to_string(),
                status_category: gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                current_focus: None,
                title_summary: None,
                worktree_path: Some(repo.clone()),
                branch: Some(session.branch.clone()),
                last_board_entry_id: None,
                last_board_entry_kind: None,
                coordination_scope: None,
                affiliation_status:
                    gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Assigned,
                workspace_id: Some(work_id.to_string()),
                updated_at: now,
            });
        gwt_core::workspace_projection::save_workspace_projection(&repo, &current)
            .expect("save current projection");
        gwt_core::workspace_projection::update_workspace_projection_with_journal_for_work_event_root(
            &repo,
            &repo,
            gwt_core::workspace_projection::WorkspaceProjectionUpdate {
                title: Some("Blocked build abort".to_string()),
                status_category: Some(
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Active,
                ),
                status_text: None,
                owner: Some(format!("Issue #{}", owner.number)),
                next_action: None,
                summary: Some("active build".to_string()),
                progress_summary: None,
                agent_session_id: Some(session.id.clone()),
                agent_current_focus: None,
                agent_title_summary: None,
            },
            gwt_core::workspace_projection::TrackedWorkEventPolicy::Persist,
        )
        .expect("seed Work event surfaces");
        let mut works = gwt_core::workspace_projection::WorkItemsProjection::empty(now);
        let mut start = gwt_core::workspace_projection::WorkEvent::new(
            gwt_core::workspace_projection::WorkEventKind::Start,
            work_id,
            now,
        );
        start.title = Some("Blocked build abort".to_string());
        start.owner = Some(format!("Issue #{}", owner.number));
        start.status_category =
            Some(gwt_core::workspace_projection::WorkspaceStatusCategory::Active);
        start.agent_session_id = Some(session.id.clone());
        start.agent_id = Some(session.agent_id.command().to_string());
        start.execution_container = Some(
            gwt_core::workspace_projection::WorkspaceExecutionContainerRef {
                branch: Some(session.branch.clone()),
                worktree_path: Some(repo.clone()),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            },
        );
        works.apply_event(start);
        gwt_core::workspace_projection::save_workspace_work_items_projection_to_path(
            &gwt_core::paths::gwt_workspace_work_items_path_for_repo_path(&repo),
            &works,
        )
        .expect("save WorkItems");
        gwt_core::skill_state::save(
            &repo,
            "build-spec",
            &gwt_core::skill_state::SkillState {
                start_evidence: None,
                active: true,
                owner_spec: Some(owner.number),
                started_at: now,
                phase: Some("verify".to_string()),
                session_id: session.id.clone(),
            },
        )
        .expect("save active build lifecycle");

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let target = server
            .agent_capability_issuer()
            .issue_bound(&repo, &session.id, binding)
            .expect("issue active Host capability");
        assert!(matches!(
            gwt::cli::execution_state::settle(
                &repo,
                &session.id,
                gwt::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "canonical verification is externally blocked".to_string(),
                    missing_verification: Some("full matrix".to_string()),
                },
            )
            .expect("settle execution"),
            gwt::cli::execution_state::SettleResult::Settled(_)
        ));

        let mut url = reqwest::Url::parse(&target.url).expect("agent hook URL");
        url.set_path("/internal/build-abort-terminalization");
        let response = reqwest::blocking::Client::new()
            .post(url)
            .bearer_auth(&target.token)
            .json(&serde_json::json!({
                "schema_version": gwt::AGENT_BUILD_ABORT_TERMINALIZATION_SCHEMA_VERSION,
                "claimed_session_id": session.id,
                "owner_number": owner.number,
                "reason": "canonical verification cannot proceed",
                "observation": gwt::observe_agent_runtime(&repo).expect("observe runtime")
            }))
            .send()
            .expect("send dedicated build abort");

        assert_eq!(response.status(), HttpStatusCode::OK);
        let receipt: gwt::AgentWorkTerminalizationReceipt =
            response.json().expect("build abort receipt");
        assert_eq!(
            receipt.outcome,
            gwt::AgentWorkTerminalizationOutcome::Emitted
        );
        let work = gwt_core::workspace_projection::load_workspace_work_items(&repo)
            .expect("load WorkItems")
            .expect("WorkItems")
            .work_items
            .into_iter()
            .find(|work| work.id == work_id)
            .expect("terminal Work");
        assert!(work.is_terminal());
        assert!(work.discarded);
        assert_eq!(
            events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
        server.shutdown();
    }

    #[test]
    fn handle_frontend_message_forwards_non_terminal_events_to_proxy() {
        let (state, events) = sample_server_state();
        let project = ProjectKey::parse("0123456789abcdef").unwrap();
        state
            .clients
            .register_scoped("client-1".into(), ClientScope::Project(project.clone()));
        let received_at = Instant::now() - Duration::from_millis(100);

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::UpdateTerminalGrid {
                id: "window-1".into(),
                cols: 120,
                rows: 24,
            },
            received_at,
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, client_scope: Some(ClientScope::Project(forwarded_project)),
                event: FrontendEvent::UpdateTerminalGrid { id, cols: 120, rows: 24 }, received_at: forwarded_at }]
                if client_id == "client-1" && forwarded_project == &project
                    && id == "window-1" && *forwarded_at == received_at
        ));
    }

    #[test]
    fn terminal_input_timing_warns_only_for_slow_successful_write() {
        let output = crate::tests::capture_timing_warnings(|| {
            super::log_terminal_input_completion("client-1", 7, "window-1", 1000, 30, 9);
            super::log_terminal_input_completion("client-1", 8, "window-1", 1000, 29, 9);
        });
        let logs: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("timing JSON"))
            .collect();
        assert_eq!(logs.len(), 1, "29ms must not warn; 30ms must warn");
        let fields = &logs[0]["fields"];
        assert_eq!(fields["stage"], "fast_path_write");
        assert_eq!(fields["elapsed_ms"], 30);
        assert_eq!(fields["write_us"], 1000);
        assert_eq!(fields["pty_writer_count"], 9);
        assert_eq!(fields["seq"], 7);
        assert!(fields.get("data").is_none());
    }

    #[test]
    fn websocket_handshake_binds_scope_until_disconnect_and_reconnect() {
        let runtime = Runtime::new().unwrap();
        let (proxy, events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            clients.clone(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .unwrap();
        let url = server
            .agent_capability_issuer()
            .pane_websocket_url()
            .to_string();
        runtime.block_on(async {
            let invalid = connect_async(format!("{url}?repo_hash=invalid")).await.unwrap_err();
            assert!(matches!(invalid, WebSocketError::Http(response) if response.status() == StatusCode::BAD_REQUEST));
            for expected in [ClientScope::Project(project_a()), ClientScope::Hub] {
                let scoped_url = match &expected { ClientScope::Project(key) => format!("{url}?repo_hash={key}"), ClientScope::Hub => url.clone() };
                let (mut socket, _) = connect_async(scoped_url).await.unwrap();
                socket.send(WebSocketMessage::Text(r#"{"kind":"frontend_ready","repo_hash":"fedcba9876543210"}"#.into())).await.unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        if !events.lock().unwrap().is_empty() { break; }
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                {
                    let registration = clients.scopes_for_test();
                    assert_eq!(registration.len(), 1);
                    assert_eq!(registration[0], expected);
                }
                events.lock().unwrap().clear();
                socket.close(None).await.unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    while clients.has_clients() { tokio::task::yield_now().await; }
                }).await.unwrap();
            }
        });
        server.shutdown();
    }

    // AC-5 inventory: host process/log streams, provider usage (including
    // its session rows), runtime health, and Board sign-in are global host
    // views. Uploads are host staging addressed by opaque upload_id and reply
    // only to their HTTP caller; attaching one to a pane follows scoped input.
    // Project-derived hook/PTY/projection payloads must go through runtime
    // routing, never a new direct transport broadcast.
    #[test]
    fn transport_direct_dispatch_inventory_stays_explicit() {
        let source = include_str!("embedded_server.rs").replace("\r\n", "\n");
        assert_transport_direct_dispatch_inventory(&source);
        assert_transport_direct_dispatch_inventory(&source.replace('\n', "\r\n"));
    }

    fn assert_transport_direct_dispatch_inventory(source: &str) {
        let server = source
            .split_once("pub fn broadcast_runtime_hook_event(")
            .expect("test-only hook helper marks the end of production server code")
            .0;
        assert_eq!(
            server.matches(".dispatch(").count(),
            1,
            "classify every new direct transport dispatch"
        );
        let oauth = server
            .split("async fn oauth_callback_handler(")
            .nth(1)
            .unwrap()
            .split("struct AttachmentUploadTokenResponse")
            .next()
            .unwrap();
        assert!(oauth.contains("OutboundEvent::broadcast("));
        assert!(oauth.contains("board_auth_status_event("));
        let upload = server
            .split("async fn attachment_upload_handler(")
            .nth(1)
            .unwrap()
            .split("async fn access_log_middleware(")
            .next()
            .unwrap();
        assert!(
            !upload.contains(".dispatch("),
            "upload staging replies only to its HTTP caller"
        );
        let hook = include_str!("project_transport.rs")
            .split("async fn hook_live_handler(")
            .nth(1)
            .unwrap()
            .split("async fn workspace_update_handler(")
            .next()
            .unwrap();
        assert!(hook.contains("TransportEvent::RuntimeHook(event)"));
        assert!(
            !hook.contains(".dispatch("),
            "hook events require project routing in the runtime"
        );
        let main = include_str!("main.rs")
            .split("fn main() ->")
            .nth(1)
            .unwrap();
        for (arm, policy) in [
            (
                "ActiveWorkProjectionPrepared(prepared)",
                "DispatchTarget::Project(prepared_dispatch.context.project_key)",
            ),
            (
                "LaunchProgress { window_id, message }",
                "project_key_for_window",
            ),
            (
                "LaunchTerminalOutput { window_id, data }",
                "project_key_for_window",
            ),
            ("ProjectIndexStatus {", "OutboundEvent::project("),
            ("MigrationProgress {", "handle_migration_progress"),
        ] {
            let body = main
                .split(&format!("Event::UserEvent(UserEvent::{arm}"))
                .nth(1)
                .unwrap()
                .split("Event::UserEvent(")
                .next()
                .unwrap();
            assert!(
                body.contains(policy),
                "direct event {arm} must use {policy}"
            );
            assert!(!body.contains("OutboundEvent::broadcast("));
        }
        // Only host monitor/update notifications and pre-project clone navigation
        // retain direct global construction in the event loop.
        let compact: String = main.chars().filter(|c| !c.is_whitespace()).collect();
        let direct_globals: Vec<_> = compact
            .split("OutboundEvent::broadcast(BackendEvent::")
            .skip(1)
            .map(|tail| tail.split('{').next().unwrap())
            .collect();
        assert_eq!(direct_globals, ["UpdateProgress", "UpdateReady",]);
        let health = include_str!("runtime_health_poller.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert_eq!(health.matches(".dispatch(").count(), 1);
        assert!(health.contains("BackendEvent::RuntimeHealth { snapshot }"));
        let usage = include_str!("usage_poller.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert_eq!(usage.matches(".dispatch(").count(), 1);
        assert!(usage.contains("OutboundEvent::broadcast("));
        assert!(usage.contains("BackendEvent::ProviderUsage {"));
        assert!(usage.contains("sessions: snapshot.sessions"));
        let runtime = include_str!("app_runtime/runtime_events.rs");
        for event in ["ProcessLine", "LogEntryAppended"] {
            assert!(
                runtime.contains(&format!("BackendEvent::{event}"))
                    && runtime.contains("project_events_for_open_surface"),
                "diagnostics must be delivered only to projects with an open consumer: {event}"
            );
        }
    }

    #[test]
    fn websocket_query_validates_project_scope() {
        assert_eq!(
            super::WebsocketQuery::default().scope().unwrap(),
            ClientScope::Hub
        );
        assert_eq!(
            super::WebsocketQuery {
                repo_hash: Some(project_a().to_string())
            }
            .scope()
            .unwrap(),
            ClientScope::Project(project_a())
        );
        for hash in ["", "../project", "0123456789ABCDEF", "0123456789abcde"] {
            assert_eq!(
                super::WebsocketQuery {
                    repo_hash: Some(hash.into())
                }
                .scope()
                .unwrap_err(),
                axum::http::StatusCode::BAD_REQUEST
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn terminal_input_rejects_other_project_without_writing_or_fallback() {
        let (state, events) = sample_server_state();
        let pane = gwt_terminal::Pane::new(
            "scoped-pane".into(),
            "sh".into(),
            vec!["-c".into(), "cat >/dev/null".into()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .unwrap();
        let handle = pane.shared_pty();
        state.pty_writers.write().unwrap().insert(
            "pane-a".into(),
            Arc::new(crate::PtyWriterEntry {
                project_key: project_a(),
                handle: handle.clone(),
                monitor_runtime: None,
            }),
        );
        state.clients.register_scoped(
            "b".into(),
            ClientScope::Project(ProjectKey::parse("fedcba9876543210").unwrap()),
        );
        state.clients.register("hub".into());
        for client_id in ["b", "hub", "unknown"] {
            handle_frontend_message(
                &state,
                client_id,
                &AtomicU64::new(0),
                FrontendEvent::TerminalInput {
                    id: "pane-a".into(),
                    data: "x".into(),
                },
                Instant::now(),
            );
        }
        assert!(!handle.has_unsent_user_input());
        assert!(events.lock().unwrap().is_empty());
        state
            .clients
            .register_scoped("a".into(), ClientScope::Project(project_a()));
        handle_frontend_message(
            &state,
            "a",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "pane-a".into(),
                data: "x".into(),
            },
            Instant::now(),
        );
        assert!(handle.has_unsent_user_input());
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn terminal_input_rejects_hub_and_unregistered_clients_before_fallback() {
        let (state, events) = sample_server_state();
        state.clients.register("hub".to_string());
        for client_id in ["hub", "unknown"] {
            handle_frontend_message(
                &state,
                client_id,
                &AtomicU64::new(0),
                FrontendEvent::TerminalInput {
                    id: "project-a-pane".into(),
                    data: "x".into(),
                },
                Instant::now(),
            );
        }
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn handle_frontend_message_falls_back_to_proxy_when_pty_writer_is_missing() {
        let (state, events) = sample_server_state();
        state
            .clients
            .register_scoped("client-1".into(), ClientScope::Project(project_a()));
        let received_at = Instant::now() - Duration::from_millis(50);

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "tab-1::shell-1".to_string(),
                data: "ls\n".to_string(),
            },
            received_at,
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::Frontend { client_id, event: FrontendEvent::TerminalInput { id, data }, received_at: forwarded_at, .. }]
                if client_id == "client-1"
                    && id == "tab-1::shell-1"
                    && data == "ls\n"
                    && *forwarded_at == received_at
        ));
    }

    #[cfg(unix)]
    #[test]
    fn invalidated_fast_path_generation_cancels_resolution_without_fallback_input() {
        let (state, events) = sample_server_state();
        state
            .clients
            .register_scoped("client-1".into(), ClientScope::Project(project_a()));
        let pane = gwt_terminal::Pane::new(
            "stale-pane".to_string(),
            "sh".to_string(),
            vec!["-c".to_string(), "cat >/dev/null".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("long-running stale pane");
        let stale_generation = pane.shared_pty();
        stale_generation.invalidate_input_generation();
        state.pty_writers.write().expect("writer registry").insert(
            "tab-1::agent-1".to_string(),
            Arc::new(crate::PtyWriterEntry {
                project_key: project_a(),
                handle: stale_generation,
                monitor_runtime: None,
            }),
        );

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "tab-1::agent-1".to_string(),
                data: "1\r".to_string(),
            },
            Instant::now(),
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [
                UserEvent::RuntimeApprovalResolutionStarted { id: started },
                UserEvent::RuntimeApprovalResolutionCancelled { id: cancelled },
            ] if started == "tab-1::agent-1" && cancelled == "tab-1::agent-1"
        ));
        assert!(recorded.iter().all(|event| !matches!(
            event,
            UserEvent::Frontend {
                event: FrontendEvent::TerminalInput { .. },
                ..
            }
        )));
        drop(recorded);
        drop(pane);
    }

    #[cfg(unix)]
    #[test]
    fn handle_frontend_message_fast_path_marks_submit_before_write_and_ignores_navigation() {
        let (state, events) = sample_server_state();
        state
            .clients
            .register_scoped("client-1".into(), ClientScope::Project(project_a()));
        let pane = gwt_terminal::Pane::new(
            "test-pane".to_string(),
            "sh".to_string(),
            vec!["-c".to_string(), "cat >/dev/null".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("long-running test pane");
        state.pty_writers.write().expect("writer registry").insert(
            "tab-1::agent-1".to_string(),
            Arc::new(crate::PtyWriterEntry {
                project_key: project_a(),
                handle: pane.shared_pty(),
                monitor_runtime: None,
            }),
        );

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "tab-1::agent-1".to_string(),
                data: "1\r".to_string(),
            },
            Instant::now(),
        );
        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(1),
            FrontendEvent::TerminalInput {
                id: "tab-1::agent-1".to_string(),
                data: "\u{1b}[A".to_string(),
            },
            Instant::now(),
        );
        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(2),
            FrontendEvent::TerminalInput {
                id: "tab-1::agent-1".to_string(),
                data: "x".to_string(),
            },
            Instant::now(),
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(matches!(
            recorded.as_slice(),
            [UserEvent::RuntimeApprovalResolutionStarted { id }]
                if id == "tab-1::agent-1"
        ));
        drop(recorded);
        drop(pane);
    }

    #[cfg(unix)]
    #[test]
    fn handle_frontend_message_flushes_held_pm_wake_after_composer_submit() {
        let (state, events) = sample_server_state();
        state
            .clients
            .register_scoped("client-1".into(), ClientScope::Project(project_a()));
        let pane = gwt_terminal::Pane::new(
            "test-pane".to_string(),
            "sh".to_string(),
            vec!["-c".to_string(), "cat >/dev/null".to_string()],
            80,
            24,
            HashMap::new(),
            None,
        )
        .expect("long-running test pane");
        state.pty_writers.write().expect("writer registry").insert(
            "tab-1::pm-window".to_string(),
            Arc::new(crate::PtyWriterEntry {
                project_key: project_a(),
                handle: pane.shared_pty(),
                monitor_runtime: None,
            }),
        );

        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(0),
            FrontendEvent::TerminalInput {
                id: "tab-1::pm-window".to_string(),
                data: "実行されてい".to_string(),
            },
            Instant::now(),
        );
        handle_frontend_message(
            &state,
            "client-1",
            &AtomicU64::new(1),
            FrontendEvent::TerminalInput {
                id: "tab-1::pm-window".to_string(),
                data: "ますか？\r".to_string(),
            },
            Instant::now(),
        );

        let recorded = events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            recorded.iter().any(|event| matches!(
                event,
                UserEvent::FlushPendingPmWake { id } if id == "tab-1::pm-window"
            )),
            "submitting unsent composer text must ask the event loop to flush a held PM wake: {recorded:?}"
        );
        drop(recorded);
        drop(pane);
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

        assert_ne!(hook.url, format!("{}internal/hook-live", server.url()));

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
                .contains("function websocketUrl(projectKey = activeProjectKey())"),
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

        let event = sample_runtime_hook_event();

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
    fn successful_hook_live_requests_do_not_fill_access_log_ring() {
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

        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();
        let accepted = client
            .post(&hook.url)
            .bearer_auth(&hook.token)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("authorized hook request");
        assert_eq!(accepted.status(), HttpStatusCode::NO_CONTENT);

        let records = server.access_log().snapshot();
        assert!(
            records
                .iter()
                .all(|record| record.path != "/internal/hook-live"),
            "successful internal hook-live traffic must not evict operator-relevant access records"
        );

        server.shutdown();
    }

    #[test]
    fn unsuccessful_hook_live_requests_remain_in_access_log_ring() {
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

        let hook = server.hook_forward_target();
        let client = reqwest::blocking::Client::new();
        let unauthorized = client
            .post(&hook.url)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("unauthorized hook request");
        assert_eq!(unauthorized.status(), HttpStatusCode::UNAUTHORIZED);

        let records = server.access_log().snapshot();
        let hook_record = records
            .iter()
            .find(|record| record.path == "/internal/hook-live")
            .expect("failed hook-live access should remain visible");
        assert_eq!(hook_record.method, "POST");
        assert_eq!(hook_record.status, 401);

        server.shutdown();
    }

    #[test]
    fn failed_agent_routes_never_record_client_metadata_that_can_repeat_capability_secrets() {
        const TOKEN_SENTINEL: &str = "agent-capability-secret-sentinel";

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("server");
        let hook = server.hook_forward_target();
        let mut workspace_update_url = reqwest::Url::parse(&hook.url).expect("agent hook URL");
        workspace_update_url.set_path("/internal/workspace-update");
        let mut work_terminalization_url = reqwest::Url::parse(&hook.url).expect("agent hook URL");
        work_terminalization_url.set_path("/internal/work-terminalization");
        let mut work_materialization_probe_url =
            reqwest::Url::parse(&hook.url).expect("agent hook URL");
        work_materialization_probe_url.set_path("/internal/work-materialization-probe");
        let mut build_abort_url = reqwest::Url::parse(&hook.url).expect("agent hook URL");
        build_abort_url.set_path("/internal/build-abort-terminalization");
        let mut execution_binding_probe_url =
            reqwest::Url::parse(&hook.url).expect("agent hook URL");
        execution_binding_probe_url.set_path("/internal/execution-binding-probe");
        let workspace_request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "intent": {}
        });
        let terminalization_request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            },
            "terminal_kind": "done"
        });
        let materialization_probe_request = serde_json::json!({
            "schema_version": 1,
            "claimed_session_id": "session-1",
            "owner_number": 2359,
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            }
        });
        let binding_probe_request = serde_json::json!({
            "schema_version": gwt::AGENT_EXECUTION_BINDING_PROBE_SCHEMA_VERSION,
            "operation_id": "operation-access-log",
            "nonce": "nonce-access-log"
        });
        let build_abort_request = serde_json::json!({
            "schema_version": gwt::AGENT_BUILD_ABORT_TERMINALIZATION_SCHEMA_VERSION,
            "claimed_session_id": "session-1",
            "owner_number": 3580,
            "reason": "blocked",
            "observation": {
                "cwd": "/workspace/repo",
                "git_toplevel": "/workspace/repo",
                "repo_hash": "observed-repo-hash",
                "branch": "work/observed"
            }
        });
        let client = reqwest::blocking::Client::new();

        let hook_response = client
            .post(&hook.url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&sample_runtime_hook_event())
            .send()
            .expect("unauthorized hook request");
        assert_eq!(hook_response.status(), HttpStatusCode::UNAUTHORIZED);

        let workspace_response = client
            .post(workspace_update_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&workspace_request)
            .send()
            .expect("unauthorized workspace-update request");
        assert_eq!(workspace_response.status(), HttpStatusCode::UNAUTHORIZED);

        let terminalization_response = client
            .post(work_terminalization_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&terminalization_request)
            .send()
            .expect("unauthorized Work terminalization request");
        assert_eq!(
            terminalization_response.status(),
            HttpStatusCode::UNAUTHORIZED
        );

        let materialization_probe_response = client
            .post(work_materialization_probe_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&materialization_probe_request)
            .send()
            .expect("unauthorized Work materialization probe request");
        assert_eq!(
            materialization_probe_response.status(),
            HttpStatusCode::UNAUTHORIZED
        );
        let build_abort_response = client
            .post(build_abort_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&build_abort_request)
            .send()
            .expect("unauthorized build abort request");
        assert_eq!(build_abort_response.status(), HttpStatusCode::UNAUTHORIZED);

        let binding_probe_response = client
            .post(execution_binding_probe_url)
            .header(reqwest::header::USER_AGENT, TOKEN_SENTINEL)
            .json(&binding_probe_request)
            .send()
            .expect("unauthorized execution binding probe request");
        assert_eq!(
            binding_probe_response.status(),
            HttpStatusCode::UNAUTHORIZED
        );

        let records = server.access_log().snapshot();
        for path in [
            "/internal/hook-live",
            "/internal/execution-binding-probe",
            "/internal/workspace-update",
            "/internal/work-materialization-probe",
            "/internal/work-terminalization",
            "/internal/build-abort-terminalization",
        ] {
            let record = records
                .iter()
                .find(|record| record.path == path)
                .unwrap_or_else(|| panic!("failed {path} access should remain visible"));
            assert_eq!(record.status, 401);
            assert_eq!(
                record.user_agent, None,
                "agent access records must not retain caller-controlled metadata"
            );
        }
        assert!(
            !format!("{records:?}").contains(TOKEN_SENTINEL),
            "agent access records must stay capability-secret-free"
        );

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

    #[test]
    fn embedded_server_preserves_unicode_attachment_upload_filename() {
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
                "{}internal/attachments/upload?filename=%E8%B3%87%E6%96%99%20%E6%97%A5%E6%9C%AC%E8%AA%9E.txt&mime_type=text%2Fplain&size=7",
                server.url()
            ))
            .header("x-gwt-upload-token", token)
            .body("nihongo")
            .send()
            .expect("unicode filename upload request")
            .json()
            .expect("unicode filename upload json");
        assert_eq!(
            upload_response
                .get("filename")
                .and_then(|value| value.as_str()),
            Some("資料 日本語.txt")
        );
        let upload_id = upload_response
            .get("upload_id")
            .and_then(|value| value.as_str())
            .expect("upload id");

        let uploaded = upload_store
            .take(upload_id)
            .expect("take upload")
            .expect("uploaded file registered");
        assert_eq!(uploaded.filename, "資料 日本語.txt");
        assert_eq!(uploaded.size, 7);
        assert_eq!(
            std::fs::read(uploaded.path).expect("read uploaded temp"),
            b"nihongo"
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
            0, // no dedicated OAuth listener in tests
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
        assert_ne!(server.bound_port().get(), 0);
        assert!(server.url().contains(&format!(":{}/", server.bound_port())));
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
            0, // no dedicated OAuth listener in tests
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
        assert!(
            server
                .agent_capability_issuer()
                .pane_websocket_url()
                .starts_with("ws://127.0.0.1:"),
            "pane clients must receive a connectable loopback URL for a wildcard browser bind"
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
        assert_eq!(tray_args.port, Some(0));

        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, _events) = AppEventProxy::stub();
        let clients = ClientHub::default();
        let pty_writers = Arc::new(RwLock::new(HashMap::new()));
        let mut server = EmbeddedServer::start_with_bind(
            &runtime,
            tray_args.bind,
            tray_args.port.unwrap_or(0),
            0, // no dedicated OAuth listener in tests
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

    // Issue #4538 AC-1: per-project URLs serve the shared entrypoint for a
    // canonical ProjectKey and a deterministic, path-free 404 otherwise.
    #[test]
    fn per_project_routes_serve_the_entrypoint_or_a_path_free_not_found() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let client = reqwest::blocking::Client::new();
        let get = |path: &str| {
            let response = client
                .get(format!("{}{path}", server.url()))
                .send()
                .expect("request");
            let status = response.status();
            (status, response.text().expect("body"))
        };

        let (hub_status, hub) = get("");
        let (project_status, project) = get("p/0123456789abcdef");
        assert_eq!(hub_status, HttpStatusCode::OK);
        assert_eq!(project_status, HttpStatusCode::OK);
        assert_eq!(project, hub, "Hub and Project share one route bootstrap");

        let home = std::env::var("HOME").unwrap_or_default();
        for path in [
            "p/ZZZ",
            "p/0123456789ABCDEF",
            "p/0123456789abcdef0",
            "p/..%2F..%2Fetc",
            "p/0123456789abcdef/extra",
        ] {
            let (status, body) = get(path);
            assert_eq!(status, HttpStatusCode::NOT_FOUND, "{path}");
            assert!(body.contains("Project not found"), "{path}: {body}");
            assert!(body.contains(r#"href="/""#), "not-found links to the Hub");
            if !home.is_empty() {
                assert!(!body.contains(&home), "no filesystem path in {path}");
            }
        }
        assert!(
            events.lock().expect("events").is_empty(),
            "routing never reaches the runtime"
        );
        server.shutdown();
    }

    fn post_project_open(
        server: &EmbeddedServer,
        authorization: Option<&str>,
        content_type: &str,
        body: Vec<u8>,
    ) -> (HttpStatusCode, serde_json::Value) {
        let mut request = reqwest::blocking::Client::new()
            .post(format!("{}internal/projects/open", server.url()))
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        if let Some(authorization) = authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        let response = request.send().expect("control request");
        let status = response.status();
        (status, response.json().unwrap_or(serde_json::Value::Null))
    }

    fn project_open_body(path: &std::path::Path) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "path": path.display().to_string() })).unwrap()
    }

    fn answer_control_open(
        events: Arc<Mutex<Vec<UserEvent>>>,
        answer: impl FnOnce(std::path::PathBuf, crate::app_runtime::ProjectOpenReply) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                let request = {
                    let mut events = events.lock().expect("events");
                    events
                        .iter()
                        .position(|event| matches!(event, UserEvent::ControlProjectOpen { .. }))
                        .map(|index| events.remove(index))
                };
                if let Some(UserEvent::ControlProjectOpen { path, reply }) = request {
                    answer(path, reply);
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            panic!("control open request never reached the runtime");
        })
    }

    // Issue #4538 AC-4 / AC-5: the control request is authorized and fully
    // validated before the runtime sees it; runtime outcomes map to fixed
    // statuses and nothing speculative happens on failure.
    #[test]
    fn project_open_control_rejects_before_dispatching_to_the_runtime() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start_with_control_token(
            &runtime,
            proxy,
            "secret-token",
            Duration::from_secs(10),
        )
        .expect("embedded server");
        let absolute = std::env::temp_dir().join("gwt-4538-project");
        let valid = project_open_body(&absolute);
        let cases: Vec<(Option<&str>, &str, Vec<u8>, HttpStatusCode)> = vec![
            (
                None,
                "application/json",
                valid.clone(),
                HttpStatusCode::UNAUTHORIZED,
            ),
            (
                Some("Bearer wrong"),
                "application/json",
                valid.clone(),
                HttpStatusCode::UNAUTHORIZED,
            ),
            (
                Some("Bearer secret-token"),
                "text/plain",
                valid.clone(),
                HttpStatusCode::BAD_REQUEST,
            ),
            (
                Some("Bearer secret-token"),
                "application/json",
                vec![b'{', 0xfe, b'}'],
                HttpStatusCode::BAD_REQUEST,
            ),
            (
                Some("Bearer secret-token"),
                "application/json",
                br#"{"path":"/x","unexpected":1}"#.to_vec(),
                HttpStatusCode::BAD_REQUEST,
            ),
            (
                Some("Bearer secret-token"),
                "application/json",
                vec![b' '; gwt::project_open_control::PROJECT_OPEN_CONTROL_MAX_BODY_BYTES + 1],
                HttpStatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                Some("Bearer secret-token"),
                "application/json",
                br#"{"path":"relative/path"}"#.to_vec(),
                HttpStatusCode::UNPROCESSABLE_ENTITY,
            ),
        ];
        for (authorization, content_type, body, expected) in cases {
            let (status, error) = post_project_open(&server, authorization, content_type, body);
            assert_eq!(status, expected, "{authorization:?} {content_type}");
            assert!(error["error"].is_string(), "{error}");
        }
        assert!(
            events.lock().expect("events").is_empty(),
            "no rejected request reaches the runtime"
        );
        server.shutdown();
    }

    #[test]
    fn project_open_control_answers_with_the_committed_project_key() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start_with_control_token(
            &runtime,
            proxy,
            "secret-token",
            Duration::from_secs(10),
        )
        .expect("embedded server");
        let target = std::env::temp_dir().join("gwt-4538-project");
        let expected_path = target.clone();
        let responder = answer_control_open(events.clone(), move |path, reply| {
            assert_eq!(path, expected_path, "the exact path is forwarded");
            reply.send(Ok(ProjectKey::parse("0123456789abcdef").unwrap()));
        });
        let (status, body) = post_project_open(
            &server,
            Some("Bearer secret-token"),
            "application/json",
            project_open_body(&target),
        );
        responder.join().expect("responder");
        assert_eq!(status, HttpStatusCode::OK);
        assert_eq!(body["project_key"], "0123456789abcdef");
        assert_eq!(body["url_path"], "/p/0123456789abcdef");

        let responder = answer_control_open(events.clone(), |_, reply| {
            reply.send(Err(
                crate::app_runtime::ProjectOpenControlFailure::Rejected(
                    "not a project".to_string(),
                ),
            ));
        });
        let (status, body) = post_project_open(
            &server,
            Some("Bearer secret-token"),
            "application/json",
            project_open_body(&target),
        );
        responder.join().expect("responder");
        assert_eq!(status, HttpStatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "not a project");

        // The runtime dropped the request without answering (event loop gone).
        let responder = answer_control_open(events, |_, reply| drop(reply));
        let (status, _) = post_project_open(
            &server,
            Some("Bearer secret-token"),
            "application/json",
            project_open_body(&target),
        );
        responder.join().expect("responder");
        assert_eq!(status, HttpStatusCode::SERVICE_UNAVAILABLE);
        server.shutdown();
    }

    #[test]
    fn project_open_control_times_out_when_the_runtime_never_answers() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start_with_control_token(
            &runtime,
            proxy,
            "secret-token",
            Duration::from_millis(300),
        )
        .expect("embedded server");
        let (status, body) = post_project_open(
            &server,
            Some("Bearer secret-token"),
            "application/json",
            project_open_body(&std::env::temp_dir().join("gwt-4538-project")),
        );
        assert_eq!(status, HttpStatusCode::GATEWAY_TIMEOUT);
        assert!(body["error"].as_str().unwrap().contains("timed out"));
        assert_eq!(events.lock().expect("events").len(), 1);
        server.shutdown();
    }

    #[test]
    fn project_open_control_refuses_everything_without_a_published_token() {
        let runtime = Runtime::new().expect("tokio runtime");
        let (proxy, events) = AppEventProxy::stub();
        let mut server = EmbeddedServer::start(
            &runtime,
            proxy,
            ClientHub::default(),
            Arc::new(RwLock::new(HashMap::new())),
            AttachmentUploadStore::in_system_temp(),
        )
        .expect("embedded server");
        let (status, _) = post_project_open(
            &server,
            Some("Bearer "),
            "application/json",
            project_open_body(&std::env::temp_dir()),
        );
        assert_eq!(status, HttpStatusCode::UNAUTHORIZED);
        assert!(events.lock().expect("events").is_empty());
        server.shutdown();
    }
}

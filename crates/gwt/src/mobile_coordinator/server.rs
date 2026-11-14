//! Explicit, default-deny coordinator startup (Issue #6476).
//!
//! The only route is the Host-only registration WebSocket; there is no browser
//! signaling, invite or UI route. The caller must keep the selected home, fixed private directories
//! and TLS ancestors trusted and non-replaceable, and prevent in-place edits
//! during loading and serving, as required by the existing storage adapters.

use super::{
    host_route::{self, HostState, SharedSessions, HOST_REGISTER_PATH},
    listener::{TlsListener, HANDSHAKE_TIMEOUT},
    registration::CoordinatorSessions,
    storage::{CoordinatorStore, StoreError},
    ConfigError, CoordinatorConfig, CredentialError, TlsError,
};
use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    serve::Listener,
};
use std::{
    future::Future,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::net::TcpListener;

/// Largest accepted request body: the mobile policy's maximum SDP size.
const MAX_REQUEST_BODY: usize = 64 * 1024;

/// Fixed diagnostics only. Raw storage and socket errors are discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error("coordinator registry is locked; stop the coordinator or administrator and explicitly retry startup")]
    RegistryBusy,
    #[error("unsafe coordinator registry storage; explicitly repair owned 0700 directories and owned single-link regular 0600 files, without symlinks or macOS extended ACLs, and retry startup")]
    UnsafeRegistryStorage,
    #[error("coordinator registry is unavailable; inspect and explicitly repair the existing private registry, then retry startup; startup never initializes a registry")]
    RegistryUnavailable,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("coordinator bind is unavailable; inspect the configured address, port availability and permissions, then explicitly retry startup")]
    BindUnavailable,
    #[error("coordinator serving failed; stop the instance, inspect its deployment and explicitly retry startup")]
    ServeUnavailable,
}

impl From<StoreError> for StartupError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Busy => Self::RegistryBusy,
            StoreError::UnsafeStorage => Self::UnsafeRegistryStorage,
            StoreError::UnsupportedPrivateStorage => {
                Self::Config(ConfigError::UnsupportedPrivateStorage)
            }
            StoreError::Credential(error) => Self::Credential(error),
            _ => Self::RegistryUnavailable,
        }
    }
}

pub struct CoordinatorServer {
    listener: TlsListener,
    sessions: SharedSessions,
    origin: Arc<str>,
}

/// Origin checks are additional constraints, never registration or device
/// authorization. Missing Origin is allowed for non-browser clients and direct
/// navigation; routes apply their own checks and unknown paths are 404. Parser-level HTTP
/// errors are handled before this application middleware.
async fn http_boundary(State(origin): State<Arc<str>>, request: Request, next: Next) -> Response {
    let mut origins = request.headers().get_all(header::ORIGIN).iter();
    let allowed = match origins.next() {
        None => true,
        Some(value) => value.as_bytes() == origin.as_bytes() && origins.next().is_none(),
    };
    let oversized = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_REQUEST_BODY as u64);
    let mut response = if !allowed {
        (
            StatusCode::FORBIDDEN,
            "coordinator origin rejected; retry from the configured HTTPS origin",
        )
            .into_response()
    } else if oversized {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "coordinator request body too large; retry with a smaller request",
        )
            .into_response()
    } else {
        next.run(request).await
    };
    for (name, value) in [
        (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::CACHE_CONTROL, "no-store"),
    ] {
        response.headers_mut().insert(name, HeaderValue::from_static(value));
    }
    response
}

/// Wrap the coordinator routes in the HTTP boundary shared by every route.
/// A body declared larger than `MAX_REQUEST_BODY` is refused before routing;
/// route extractors read at most `MAX_REQUEST_BODY` of an undeclared body.
/// `Router::layer` wraps only routes that already exist, so every route must be
/// added before the router is passed here.
fn with_http_boundary(router: axum::Router, origin: Arc<str>) -> axum::Router {
    router
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY))
        .layer(middleware::from_fn_with_state(origin, http_boundary))
}

impl CoordinatorServer {
    /// Validate fixed HOME config, TLS and the existing registry before binding.
    /// `wall` is trusted Unix time supplied by the caller, never a remote value.
    /// No fallback initialization or permission repair is performed. The store
    /// may create its existing fixed lock file. Successful binding starts the
    /// bounded TLS accept task; HTTP is handled only after `serve_until` starts.
    pub async fn bind_from_home(home: &Path, wall: u64) -> Result<Self, StartupError> {
        let config = CoordinatorConfig::load_from_home(home)?;
        let tls = config.tls_server_config()?;
        let directory = home
            .join(".gwt/mobile/coordinator")
            .canonicalize()
            .map_err(|_| StartupError::RegistryUnavailable)?;
        let store = CoordinatorStore::open(&directory, wall)?;
        let tcp = TcpListener::bind(config.bind())
            .await
            .map_err(|_| StartupError::BindUnavailable)?;
        let listener = TlsListener::new(tcp, tls, HANDSHAKE_TIMEOUT)
            .map_err(|_| StartupError::BindUnavailable)?;
        Ok(Self {
            listener,
            sessions: Arc::new(Mutex::new(CoordinatorSessions::new(
                store,
                Instant::now(),
                wall,
            ))),
            origin: Arc::from(config.origin()),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, StartupError> {
        self.listener
            .local_addr()
            .map_err(|_| StartupError::BindUnavailable)
    }

    /// Serve only the Host registration route behind Origin checks and security
    /// headers; every other path is 404. No local/agent router can be supplied.
    /// Each connection service also retains the registry guard: axum's HTTP
    /// tasks can outlive a cancelled serving future and must still exclude a
    /// second writer until those connections close.
    /// Graceful shutdown waits for accepted HTTP connections, then signals every
    /// Host registration socket and waits until each registration is logically
    /// closed before returning. A connection whose
    /// reads and writes stall ends at the listener's idle deadline, and the
    /// listener caps how many connections exist at once. Pending TLS handshakes
    /// retain their existing deadline.
    pub async fn serve_until(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), StartupError> {
        let Self {
            listener,
            sessions,
            origin,
        } = self;
        let (stop, stopping) = tokio::sync::watch::channel(false);
        let (alive, mut sockets) = tokio::sync::mpsc::channel::<()>(1);
        let state = HostState {
            sessions: Arc::clone(&sessions),
            stopping,
            alive,
        };
        let router = with_http_boundary(
            axum::Router::new()
                .route(HOST_REGISTER_PATH, axum::routing::get(host_route::register))
                .with_state(state),
            origin,
        );
        let shutdown = async move {
            shutdown.await;
            let _ = stop.send(true);
        };
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|_| StartupError::ServeUnavailable);
        if result.is_ok() {
            // Every sender is gone once the router and all Host sockets end.
            while sockets.recv().await.is_some() {}
        }
        drop(sessions);
        result
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::host_route::{SharedSessions, HOST_REGISTER_PATH};
    use super::super::{
        storage::{CoordinatorStore, StoreError},
        ConfigError, CredentialError, TlsError,
    };
    use super::*;
    use crate::{
        mobile_connection::{HostBinding, MobileAuthorization},
        mobile_host::{
            credential::{load_host_credential, LoadedHostCredential},
            registration_client::{run_registration, RegistrationClientError},
        },
    };
    use gwt_core::repo_hash::ProjectKey;
    use rustls::pki_types::ServerName;
    use serde_json::json;
    use std::{fs, net::TcpListener as StdListener, os::unix::fs::PermissionsExt, path::PathBuf};
    use std::{net::SocketAddr, sync::Arc};
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpStream,
        sync::oneshot,
    };
    use uuid::Uuid;

    const WALL: u64 = 1_000;

    async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
        // test-hygiene: allow-wall-clock-deadline hang guard only; assertions observe actual I/O and lock state
        tokio::time::timeout(std::time::Duration::from_secs(10), future)
            .await
            .unwrap()
    }

    struct Fixture {
        _home: tempfile::TempDir,
        home: PathBuf,
        directory: PathBuf,
        config: Vec<u8>,
        roots: rustls::RootCertStore,
        reservation: Option<StdListener>,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let path = home.path().canonicalize().unwrap();
            let directory = path.join(".gwt/mobile/coordinator");
            fs::create_dir_all(&directory).unwrap();
            for name in [".gwt", ".gwt/mobile", ".gwt/mobile/coordinator"] {
                fs::set_permissions(path.join(name), fs::Permissions::from_mode(0o700)).unwrap();
            }
            let issued =
                rcgen::generate_simple_self_signed(vec!["coordinator.example".into()]).unwrap();
            for (name, bytes) in [
                ("cert.pem", issued.cert.pem()),
                ("key.pem", issued.signing_key.serialize_pem()),
            ] {
                fs::write(directory.join(name), bytes).unwrap();
                fs::set_permissions(directory.join(name), fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            let reservation = StdListener::bind("127.0.0.1:0").unwrap();
            let config = serde_json::to_vec(&json!({
                "schema_version": 1, "origin": "https://coordinator.example",
                "bind": reservation.local_addr().unwrap().to_string(),
                "tls_certificate": directory.join("cert.pem"), "tls_private_key": directory.join("key.pem"),
                "limits": {"registrations":64,"peers_per_host":8,"device_messages_per_second":32,
                           "device_burst":64,"host_messages_per_second":256,"host_burst":512}
            })).unwrap();
            fs::write(directory.join("config.json"), &config).unwrap();
            fs::set_permissions(
                directory.join("config.json"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
            drop(CoordinatorStore::initialize(&directory, WALL).unwrap());
            let mut roots = rustls::RootCertStore::empty();
            roots.add(issued.cert.der().clone()).unwrap();
            Self {
                _home: home,
                home: path,
                directory,
                config,
                roots,
                reservation: Some(reservation),
            }
        }

        async fn start(&mut self) -> CoordinatorServer {
            let expected = self.reservation.take().unwrap().local_addr().unwrap();
            let server = CoordinatorServer::bind_from_home(&self.home, WALL)
                .await
                .unwrap();
            assert_eq!(server.local_addr().unwrap(), expected);
            server
        }

        /// Issue a Host credential at trusted `wall` and return a bearer token.
        fn issue_token(&self, wall: u64) -> String {
            let mut store = CoordinatorStore::open(&self.directory, wall).unwrap();
            let issued = store
                .issue(
                    "https://coordinator.example",
                    Uuid::new_v4(),
                    &[ProjectKey::parse(PROJECT).unwrap()],
                    wall + 3_600,
                    wall,
                )
                .unwrap();
            drop(store);
            let file: serde_json::Value =
                serde_json::from_slice(&fs::read(&issued.path).unwrap()).unwrap();
            let identity = issued.identity;
            token(json!({
                "schema_version": 1, "host_id": identity.host_id,
                "credential_id": identity.credential_id, "generation": identity.generation,
                "project": PROJECT, "host_epoch": Uuid::new_v4(), "secret": file["secret"],
            }))
        }

        async fn start_at(&mut self, wall: u64) -> CoordinatorServer {
            self.reservation.take();
            CoordinatorServer::bind_from_home(&self.home, wall)
                .await
                .unwrap()
        }

        fn client_config(&self) -> Arc<rustls::ClientConfig> {
            Arc::new(
                rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(self.roots.clone())
                .with_no_client_auth(),
            )
        }

        /// Issue a Host credential at trusted `wall` and load it as the PC does.
        fn issue_credential(&self, wall: u64) -> (LoadedHostCredential, HostBinding) {
            let mut store = CoordinatorStore::open(&self.directory, wall).unwrap();
            let project = ProjectKey::parse(PROJECT).unwrap();
            let issued = store
                .issue(
                    "https://coordinator.example",
                    Uuid::new_v4(),
                    std::slice::from_ref(&project),
                    wall + 3_600,
                    wall,
                )
                .unwrap();
            drop(store);
            let binding = HostBinding::new(
                &issued.identity.host_id.to_string(),
                "https://coordinator.example",
            )
            .unwrap();
            let credential = load_host_credential(&issued.path, &binding, &project, wall).unwrap();
            (credential, binding)
        }

        async fn connect(
            &self,
            addr: SocketAddr,
        ) -> BufReader<tokio_rustls::client::TlsStream<TcpStream>> {
            let tcp = TcpStream::connect(addr).await.unwrap();
            let tls = bounded(
                tokio_rustls::TlsConnector::from(self.client_config())
                    .connect(ServerName::try_from("coordinator.example").unwrap(), tcp),
            )
            .await
            .unwrap();
            BufReader::new(tls)
        }
    }

    #[tokio::test]
    async fn http_origin_boundary_and_security_headers_apply_to_denied_routes() {
        let mut fixture = Fixture::new();
        let server = fixture.start().await;
        let addr = server.local_addr().unwrap();
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        for (origin, status) in [
            ("Origin: https://private-secret.example\r\n", 403),
            ("Origin: null\r\n", 403),
            ("Origin: http://coordinator.example\r\n", 403),
            ("Origin: https://coordinator.example/path\r\n", 403),
            (
                "Origin: https://coordinator.example\r\nOrigin: https://coordinator.example\r\n",
                403,
            ),
            ("Origin: https://coordinator.example\r\n", 404),
            ("", 404),
        ] {
            let mut client = fixture.connect(addr).await;
            bounded(client.write_all(
                format!("GET /ws HTTP/1.1\r\nHost: coordinator.example\r\n{origin}Connection: close\r\n\r\n")
                    .as_bytes(),
            ))
            .await
            .unwrap();
            let mut response = String::new();
            bounded(client.read_to_string(&mut response)).await.unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
            let headers = response
                .split("\r\n\r\n")
                .next()
                .unwrap()
                .to_ascii_lowercase();
            for header in [
                "content-security-policy: default-src 'none'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
                "x-content-type-options: nosniff",
                "referrer-policy: no-referrer",
                "cache-control: no-store",
            ] {
                assert!(headers.lines().any(|line| line == header), "{response}");
            }
            assert!(!headers.contains("access-control-allow-"));
            assert!(!response.contains("private-secret"));
            if status == 403 {
                assert!(response.ends_with(
                    "coordinator origin rejected; retry from the configured HTTPS origin"
                ));
            }
        }
        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
    }

    fn assert_security_headers(response: &str) {
        let headers = response
            .split("\r\n\r\n")
            .next()
            .unwrap()
            .to_ascii_lowercase();
        for header in [
            "content-security-policy: default-src 'none'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
            "x-content-type-options: nosniff",
            "referrer-policy: no-referrer",
            "cache-control: no-store",
        ] {
            assert!(headers.lines().any(|line| line == header), "{response}");
        }
    }

    #[tokio::test]
    async fn declared_oversized_request_bodies_are_rejected_before_routing() {
        let mut fixture = Fixture::new();
        let server = fixture.start().await;
        let addr = server.local_addr().unwrap();
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        for (length, status) in [(MAX_REQUEST_BODY + 1, 413), (MAX_REQUEST_BODY, 404)] {
            let mut client = fixture.connect(addr).await;
            // The body is withheld: the boundary must answer from the header.
            bounded(client.write_all(
                format!(
                    "POST /ws HTTP/1.1\r\nHost: coordinator.example\r\nOrigin: https://coordinator.example\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            ))
            .await
            .unwrap();
            let mut response = String::new();
            bounded(client.read_to_string(&mut response)).await.unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
            assert_security_headers(&response);
            if status == 413 {
                assert!(response
                    .ends_with("coordinator request body too large; retry with a smaller request"));
            }
        }
        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn undeclared_request_bodies_are_limited_for_route_extractors() {
        let router = with_http_boundary(
            axum::Router::new().route(
                "/echo",
                axum::routing::post(
                    |body: axum::body::Bytes| async move { body.len().to_string() },
                ),
            ),
            Arc::from("https://coordinator.example"),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        for (length, status) in [(MAX_REQUEST_BODY, 200), (MAX_REQUEST_BODY + 1, 413)] {
            let mut client = TcpStream::connect(addr).await.unwrap();
            let mut request = format!(
                "POST /echo HTTP/1.1\r\nHost: coordinator.example\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{length:x}\r\n"
            )
            .into_bytes();
            request.resize(request.len() + length, b'a');
            request.extend_from_slice(b"\r\n0\r\n\r\n");
            bounded(client.write_all(&request)).await.unwrap();
            let mut response = String::new();
            bounded(client.read_to_string(&mut response)).await.unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{response}"
            );
            assert_security_headers(&response);
            if status == 200 {
                assert!(response.ends_with(&length.to_string()), "{response}");
            }
        }
        task.abort();
    }

    #[tokio::test]
    async fn fixed_home_startup_denies_routes_and_retains_lock_through_shutdown_or_cancel() {
        for cancel in [false, true] {
            let mut fixture = Fixture::new();
            let registry = fs::read(fixture.directory.join("registry.json")).unwrap();
            let server = fixture.start().await;
            let addr = server.local_addr().unwrap();
            let (shutdown, signal) = oneshot::channel();
            let task = tokio::spawn(server.serve_until(async move {
                let _ = signal.await;
            }));
            let mut client = fixture.connect(addr).await;
            for path in ["/", "/ws", "/mobile/"] {
                bounded(client.write_all(
                    format!("GET {path} HTTP/1.1\r\nHost: coordinator.example\r\n\r\n").as_bytes(),
                ))
                .await
                .unwrap();
                let mut line = String::new();
                bounded(client.read_line(&mut line)).await.unwrap();
                assert!(line.starts_with("HTTP/1.1 404"), "{line}");
                loop {
                    line.clear();
                    assert_ne!(bounded(client.read_line(&mut line)).await.unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
            }
            assert!(matches!(
                CoordinatorStore::open(&fixture.directory, WALL),
                Err(StoreError::Busy)
            ));
            if cancel {
                // A completed response proves the accepted HTTP task owns this connection.
                client
                    .write_all(b"GET /ws HTTP/1.1\r\nHost:")
                    .await
                    .unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(matches!(
                    CoordinatorStore::open(&fixture.directory, WALL),
                    Err(StoreError::Busy)
                ));
                drop(client);
                bounded(async {
                    loop {
                        match CoordinatorStore::open(&fixture.directory, WALL) {
                            Ok(store) => break drop(store),
                            Err(StoreError::Busy) => tokio::task::yield_now().await,
                            Err(error) => panic!("{error}"),
                        }
                    }
                })
                .await;
                let _ = shutdown.send(());
            } else {
                shutdown.send(()).unwrap();
                bounded(task).await.unwrap().unwrap();
                drop(client);
                drop(CoordinatorStore::open(&fixture.directory, WALL).unwrap());
            }
            assert_eq!(
                fs::read(fixture.directory.join("registry.json")).unwrap(),
                registry
            );
            assert_eq!(
                fs::read(fixture.directory.join("config.json")).unwrap(),
                fixture.config
            );
        }
    }

    #[tokio::test]
    async fn startup_failures_do_not_initialize_registry_or_keep_its_lock() {
        let mut fixture = Fixture::new();
        let config = fixture.directory.join("config.json");
        let registry = fixture.directory.join("registry.json");
        let registry_bytes = fs::read(&registry).unwrap();
        fs::write(&config, b"invalid-secret-sentinel").unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::Config(ConfigError::InvalidJson))
        ));
        fs::write(&config, &fixture.config).unwrap();
        let key = fixture.directory.join("key.pem");
        let key_bytes = fs::read(&key).unwrap();
        fs::write(&key, b"invalid-secret-sentinel").unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::Tls(TlsError::InvalidPrivateKey))
        ));
        fs::write(&key, &key_bytes).unwrap();
        fs::remove_file(&registry).unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::RegistryUnavailable)
        ));
        assert!(!registry.exists());
        fs::write(&registry, b"{}").unwrap();
        fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::Credential(CredentialError::InvalidRegistry))
        ));
        fs::write(&registry, &registry_bytes).unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL - 1).await,
            Err(StartupError::Credential(CredentialError::ClockRollback))
        ));
        let guard = CoordinatorStore::open(&fixture.directory, WALL).unwrap();
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::RegistryBusy)
        ));
        drop(guard);
        assert!(matches!(
            CoordinatorServer::bind_from_home(&fixture.home, WALL).await,
            Err(StartupError::BindUnavailable)
        ));
        drop(CoordinatorStore::open(&fixture.directory, WALL).unwrap());
        let server = fixture.start().await;
        drop(server);
        drop(CoordinatorStore::open(&fixture.directory, WALL).unwrap());
        for (name, bytes) in [
            ("config.json", &fixture.config),
            ("key.pem", &key_bytes),
            ("registry.json", &registry_bytes),
        ] {
            assert_eq!(&fs::read(fixture.directory.join(name)).unwrap(), bytes);
            assert_eq!(
                fs::metadata(fixture.directory.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    const PROJECT: &str = "0123456789abcdef";

    fn token(value: serde_json::Value) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
    }

    fn real_wall() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    type HostSocket =
        tokio_tungstenite::WebSocketStream<BufReader<tokio_rustls::client::TlsStream<TcpStream>>>;

    async fn open_host(
        fixture: &Fixture,
        addr: SocketAddr,
        headers: &[(&'static str, String)],
    ) -> Result<HostSocket, u16> {
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Error};
        let mut request = format!("wss://coordinator.example{HOST_REGISTER_PATH}")
            .into_client_request()
            .unwrap();
        for (name, value) in headers {
            request.headers_mut().append(*name, value.parse().unwrap());
        }
        let tls = fixture.connect(addr).await;
        match bounded(tokio_tungstenite::client_async(request, tls)).await {
            Ok((socket, _)) => Ok(socket),
            Err(Error::Http(response)) => Err(response.status().as_u16()),
            Err(error) => panic!("{error}"),
        }
    }

    fn live(sessions: &SharedSessions) -> usize {
        sessions.lock().unwrap().live_registrations()
    }

    async fn wait_until_closed(sessions: &SharedSessions) {
        bounded(async {
            while live(sessions) != 0 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;
    }

    async fn next_json(socket: &mut HostSocket) -> serde_json::Value {
        use futures_util::StreamExt as _;
        match bounded(socket.next()).await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                serde_json::from_str(&text).unwrap()
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn host_registration_rejects_browsers_and_bad_bearers_before_registering() {
        let mut fixture = Fixture::new();
        let wall = real_wall();
        let valid = fixture.issue_token(wall);
        let server = fixture.start_at(wall).await;
        let addr = server.local_addr().unwrap();
        let sessions = Arc::clone(&server.sessions);
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        let decode = |token: &str| -> serde_json::Value {
            use base64::Engine as _;
            serde_json::from_slice(
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(token)
                    .unwrap(),
            )
            .unwrap()
        };
        let mut wrong_secret = decode(&valid);
        wrong_secret["secret"] = json!("00".repeat(32));
        let wrong_secret = token(wrong_secret);
        let mut unknown = decode(&valid);
        unknown["extra"] = json!(true);
        let cases: Vec<(Vec<(&'static str, String)>, u16)> = vec![
            (
                vec![
                    ("origin", "https://coordinator.example".into()),
                    ("authorization", format!("Bearer {valid}")),
                ],
                403,
            ),
            (vec![], 401),
            (vec![("authorization", format!("Basic {valid}"))], 401),
            (vec![("authorization", "Bearer not-a-token".into())], 401),
            (
                vec![("authorization", format!("Bearer {wrong_secret}"))],
                401,
            ),
            (
                vec![("authorization", format!("Bearer {}", token(unknown)))],
                401,
            ),
            (
                vec![("authorization", format!("Bearer {}", "A".repeat(1_100)))],
                401,
            ),
            (
                vec![
                    ("authorization", format!("Bearer {valid}")),
                    ("authorization", format!("Bearer {valid}")),
                ],
                401,
            ),
        ];
        for (headers, status) in cases {
            assert_eq!(
                open_host(&fixture, addr, &headers).await.err(),
                Some(status)
            );
            assert_eq!(live(&sessions), 0);
        }
        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn host_registration_reports_its_generation_echoes_renewals_and_closes() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message;
        let mut fixture = Fixture::new();
        let wall = real_wall();
        let valid = fixture.issue_token(wall);
        let server = fixture.start_at(wall).await;
        let addr = server.local_addr().unwrap();
        let sessions = Arc::clone(&server.sessions);
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        let auth = [("authorization", format!("Bearer {valid}"))];

        let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
        let registered = next_json(&mut socket).await;
        assert_eq!(registered["type"], "registered");
        let generation: Uuid = serde_json::from_value(registered["generation"].clone()).unwrap();
        let ttl = registered["ttl_secs"].as_u64().unwrap();
        assert!((1..=30).contains(&ttl), "{registered}");
        assert_eq!(live(&sessions), 1);
        let nonce = Uuid::new_v4();
        bounded(socket.send(Message::text(
            json!({"type":"renew","generation":generation,"nonce":nonce}).to_string(),
        )))
        .await
        .unwrap();
        let renewed = next_json(&mut socket).await;
        assert_eq!(
            (&renewed["type"], &renewed["generation"], &renewed["nonce"]),
            (&json!("renewed"), &json!(generation), &json!(nonce))
        );
        assert!((1..=30).contains(&renewed["ttl_secs"].as_u64().unwrap()));
        bounded(socket.send(Message::text(
            json!({"type":"renew","generation":Uuid::new_v4(),"nonce":nonce}).to_string(),
        )))
        .await
        .unwrap();
        assert!(matches!(
            bounded(socket.next()).await,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
        wait_until_closed(&sessions).await;

        let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
        assert_eq!(next_json(&mut socket).await["type"], "registered");
        assert_eq!(live(&sessions), 1);
        bounded(socket.close(None)).await.unwrap();
        wait_until_closed(&sessions).await;

        let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
        assert_eq!(next_json(&mut socket).await["type"], "registered");
        bounded(socket.send(Message::binary(vec![0u8; 4])))
            .await
            .unwrap();
        wait_until_closed(&sessions).await;

        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn host_registration_closes_oversized_messages_and_reports_capacity_as_503() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message;
        let mut fixture = Fixture::new();
        let wall = real_wall();
        let valid = fixture.issue_token(wall);
        let server = fixture.start_at(wall).await;
        let addr = server.local_addr().unwrap();
        let sessions = Arc::clone(&server.sessions);
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        let auth = [("authorization", format!("Bearer {valid}"))];
        // A valid renew padded with JSON whitespace to exactly `len` bytes.
        let padded_renew = |generation: Uuid, nonce: Uuid, len: usize| {
            let mut text =
                json!({"type":"renew","generation":generation,"nonce":nonce}).to_string();
            text.push_str(&" ".repeat(len - text.len()));
            text
        };

        let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
        let registered = next_json(&mut socket).await;
        let generation: Uuid = serde_json::from_value(registered["generation"].clone()).unwrap();
        let nonce = Uuid::new_v4();
        bounded(socket.send(Message::text(padded_renew(generation, nonce, 64 * 1024))))
            .await
            .unwrap();
        let renewed = next_json(&mut socket).await;
        assert_eq!(
            (&renewed["type"], &renewed["nonce"]),
            (&json!("renewed"), &json!(nonce))
        );
        assert_eq!(live(&sessions), 1);
        let _ = bounded(socket.send(Message::text(padded_renew(
            generation,
            Uuid::new_v4(),
            64 * 1024 + 1,
        ))))
        .await;
        assert!(matches!(
            bounded(socket.next()).await,
            None | Some(Err(_)) | Some(Ok(Message::Close(_)))
        ));
        wait_until_closed(&sessions).await;

        let mut open = Vec::new();
        for _ in 0..8 {
            let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
            assert_eq!(next_json(&mut socket).await["type"], "registered");
            open.push(socket);
        }
        assert_eq!(open_host(&fixture, addr, &auth).await.err(), Some(503));
        assert_eq!(live(&sessions), 8);
        bounded(open.pop().unwrap().close(None)).await.unwrap();
        bounded(async {
            while live(&sessions) != 7 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;
        let mut socket = open_host(&fixture, addr, &auth).await.unwrap();
        assert_eq!(next_json(&mut socket).await["type"], "registered");
        assert_eq!(live(&sessions), 8);

        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn shutdown_closes_open_host_registrations_and_releases_the_registry() {
        use futures_util::StreamExt as _;
        let mut fixture = Fixture::new();
        let wall = real_wall();
        let valid = fixture.issue_token(wall);
        let server = fixture.start_at(wall).await;
        let addr = server.local_addr().unwrap();
        let sessions = Arc::clone(&server.sessions);
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        let mut socket = open_host(
            &fixture,
            addr,
            &[("authorization", format!("Bearer {valid}"))],
        )
        .await
        .unwrap();
        assert_eq!(next_json(&mut socket).await["type"], "registered");
        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
        assert_eq!(live(&sessions), 0);
        assert!(matches!(
            bounded(socket.next()).await,
            None | Some(Err(_)) | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
        ));
        drop(sessions);
        drop(CoordinatorStore::open(&fixture.directory, real_wall()).unwrap());
    }

    #[tokio::test]
    async fn host_client_registers_through_its_authority_and_disables_on_every_exit() {
        use std::sync::RwLock;
        use tokio::sync::watch;
        let mut fixture = Fixture::new();
        let wall = real_wall();
        let (credential, binding) = fixture.issue_credential(wall);
        let (unknown, unknown_binding) = Fixture::new().issue_credential(wall);
        let server = fixture.start_at(wall).await;
        let addr = server.local_addr().unwrap();
        let sessions = Arc::clone(&server.sessions);
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(server.serve_until(async move {
            let _ = signal.await;
        }));
        let devices = fixture.home.join("devices.json");
        let authority = |binding: &HostBinding| {
            Arc::new(RwLock::new(
                MobileAuthorization::load_bound(&devices, binding).unwrap(),
            ))
        };
        let credential = Arc::new(credential);
        let spawn = |authority: Arc<RwLock<MobileAuthorization>>,
                     credential: Arc<LoadedHostCredential>,
                     stopping: watch::Receiver<bool>| {
            let tls = fixture.client_config();
            tokio::spawn(async move {
                let tcp = TcpStream::connect(addr).await.unwrap();
                run_registration(authority, &credential, tls, tcp, stopping).await
            })
        };
        let enabled = |authority: &Arc<RwLock<MobileAuthorization>>| {
            let authority = Arc::clone(authority);
            bounded(async move {
                while !authority.read().unwrap().is_enabled() {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            })
        };

        // A credential this coordinator never issued gets no lease.
        let (_stop, stopping) = watch::channel(false);
        let foreign = authority(&unknown_binding);
        let result = bounded(spawn(Arc::clone(&foreign), Arc::new(unknown), stopping))
            .await
            .unwrap();
        assert!(
            matches!(result, Err(RegistrationClientError::Rejected(401))),
            "{result:?}"
        );
        assert!(!foreign.read().unwrap().is_enabled());
        assert_eq!(live(&sessions), 0);

        // A coordinator outside the caller's trust roots never sees the bearer.
        let (_stop, stopping) = watch::channel(false);
        let untrusted = authority(&binding);
        let result = bounded({
            let authority = Arc::clone(&untrusted);
            let credential = Arc::clone(&credential);
            let tls = Arc::new(
                rustls::ClientConfig::builder_with_provider(Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth(),
            );
            tokio::spawn(async move {
                let tcp = TcpStream::connect(addr).await.unwrap();
                run_registration(authority, &credential, tls, tcp, stopping).await
            })
        })
        .await
        .unwrap();
        assert!(
            matches!(result, Err(RegistrationClientError::Connect)),
            "{result:?}"
        );
        assert!(!untrusted.read().unwrap().is_enabled());
        assert_eq!(live(&sessions), 0);

        // Registration plus the first renewal round trip enables the Host;
        // an explicit PC stop disables it and logically closes the registration.
        let (stop, stopping) = watch::channel(false);
        let host = authority(&binding);
        let client = spawn(Arc::clone(&host), Arc::clone(&credential), stopping);
        enabled(&host).await;
        assert_eq!(live(&sessions), 1);
        stop.send(true).unwrap();
        assert!(matches!(bounded(client).await.unwrap(), Ok(())));
        assert!(!host.read().unwrap().is_enabled());
        wait_until_closed(&sessions).await;

        // Losing the coordinator also disables the Host with an error.
        let (_stop, stopping) = watch::channel(false);
        let host = authority(&binding);
        let client = spawn(Arc::clone(&host), credential, stopping);
        enabled(&host).await;
        shutdown.send(()).unwrap();
        bounded(task).await.unwrap().unwrap();
        let result = bounded(client).await.unwrap();
        assert!(
            matches!(result, Err(RegistrationClientError::Closed)),
            "{result:?}"
        );
        assert!(!host.read().unwrap().is_enabled());
    }
}

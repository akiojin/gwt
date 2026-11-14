//! Explicit, default-deny coordinator startup (Issue #6476).
//!
//! This library adapter has no CLI activation, registration, signaling or UI
//! routes. The caller must keep the selected home, fixed private directories
//! and TLS ancestors trusted and non-replaceable, and prevent in-place edits
//! during loading and serving, as required by the existing storage adapters.

use super::{
    listener::{TlsListener, HANDSHAKE_TIMEOUT},
    storage::{CoordinatorStore, StoreError},
    ConfigError, CoordinatorConfig, CredentialError, TlsError,
};
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    serve::Listener,
};
use std::{future::Future, net::SocketAddr, path::Path, sync::Arc};
use tokio::net::TcpListener;

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
    store: Arc<CoordinatorStore>,
    origin: Arc<str>,
}

/// Origin checks are additional constraints, never registration or device
/// authorization. Missing Origin is allowed for non-browser clients and direct
/// navigation; the fixed router still denies every route. Parser-level HTTP
/// errors are handled before this application middleware.
async fn http_boundary(State(origin): State<Arc<str>>, request: Request, next: Next) -> Response {
    let mut origins = request.headers().get_all(header::ORIGIN).iter();
    let allowed = match origins.next() {
        None => true,
        Some(value) => value.as_bytes() == origin.as_bytes() && origins.next().is_none(),
    };
    let mut response = if allowed {
        next.run(request).await
    } else {
        (
            StatusCode::FORBIDDEN,
            "coordinator origin rejected; retry from the configured HTTPS origin",
        )
            .into_response()
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
        let store = Arc::new(CoordinatorStore::open(&directory, wall)?);
        let tcp = TcpListener::bind(config.bind())
            .await
            .map_err(|_| StartupError::BindUnavailable)?;
        let listener = TlsListener::new(tcp, tls, HANDSHAKE_TIMEOUT)
            .map_err(|_| StartupError::BindUnavailable)?;
        Ok(Self {
            listener,
            store,
            origin: Arc::from(config.origin()),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, StartupError> {
        self.listener
            .local_addr()
            .map_err(|_| StartupError::BindUnavailable)
    }

    /// Serve only a fixed 404 router behind Origin checks and security headers.
    /// No local/agent router can be supplied.
    /// Each connection service also retains the registry guard: axum's HTTP
    /// tasks can outlive a cancelled serving future and must still exclude a
    /// second writer until those connections close.
    /// Graceful shutdown waits for accepted HTTP connections. A connection whose
    /// reads and writes stall ends at the listener's idle deadline, and the
    /// listener caps how many connections exist at once. Pending TLS handshakes
    /// retain their existing deadline.
    pub async fn serve_until(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), StartupError> {
        let Self {
            listener,
            store,
            origin,
        } = self;
        let router = axum::Router::new()
            .layer(axum::Extension(Arc::clone(&store)))
            .layer(middleware::from_fn_with_state(origin, http_boundary));
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|_| StartupError::ServeUnavailable);
        drop(store);
        result
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::{
        storage::{CoordinatorStore, StoreError},
        ConfigError, CredentialError, TlsError,
    };
    use super::*;
    use rustls::pki_types::ServerName;
    use serde_json::json;
    use std::{fs, net::TcpListener as StdListener, os::unix::fs::PermissionsExt, path::PathBuf};
    use std::{net::SocketAddr, sync::Arc};
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::TcpStream,
        sync::oneshot,
    };

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

        async fn connect(
            &self,
            addr: SocketAddr,
        ) -> BufReader<tokio_rustls::client::TlsStream<TcpStream>> {
            let client = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(self.roots.clone())
            .with_no_client_auth();
            let tcp = TcpStream::connect(addr).await.unwrap();
            let tls = bounded(
                tokio_rustls::TlsConnector::from(Arc::new(client))
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
}

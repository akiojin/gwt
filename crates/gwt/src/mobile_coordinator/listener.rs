//! TLS accept boundary for the future coordinator listener (Issue #6476).
//!
//! Wraps a bound TCP listener so that `axum::serve` only ever sees connections
//! whose TLS handshake completed within a deadline. Handshakes run
//! concurrently, so a stalled or hostile peer cannot block other clients. At
//! most `MAX_PENDING_HANDSHAKES` accepted connections are held until axum takes
//! them; at that cap TCP accept pauses. Failed or late handshakes are closed
//! without reaching the router. Handshakes already running when the listener
//! is dropped end at their deadline. This is not a route, Origin, registration
//! or device authority check.

use axum::serve::Listener;
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};

/// Default handshake deadline for coordinator callers.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PENDING_HANDSHAKES: usize = 64;

pub struct TlsListener {
    local_addr: SocketAddr,
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr, OwnedSemaphorePermit)>,
    acceptor_task: JoinHandle<()>,
}

impl TlsListener {
    /// Start accepting on `tcp`. Must be called inside a Tokio runtime.
    pub fn new(
        tcp: TcpListener,
        tls: rustls::ServerConfig,
        handshake_timeout: Duration,
    ) -> io::Result<Self> {
        Self::with_capacity(tcp, tls, handshake_timeout, MAX_PENDING_HANDSHAKES)
    }

    fn with_capacity(
        mut tcp: TcpListener,
        tls: rustls::ServerConfig,
        handshake_timeout: Duration,
        capacity: usize,
    ) -> io::Result<Self> {
        let local_addr = tcp.local_addr()?;
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        // A permit travels with the connection and is released when axum
        // receives it, so `capacity` bounds handshakes plus queued connections.
        let (sender, ready) = mpsc::channel(1);
        let pending = Arc::new(Semaphore::new(capacity));
        let acceptor_task = tokio::spawn(async move {
            loop {
                let Ok(permit) = pending.clone().acquire_owned().await else {
                    return;
                };
                // axum's TcpListener impl retries and backs off on accept errors.
                let (stream, peer) = Listener::accept(&mut tcp).await;
                let (acceptor, sender) = (acceptor.clone(), sender.clone());
                tokio::spawn(async move {
                    if let Ok(Ok(stream)) =
                        tokio::time::timeout(handshake_timeout, acceptor.accept(stream)).await
                    {
                        let _ = sender.send((stream, peer, permit)).await;
                    }
                });
            }
        });
        Ok(Self {
            local_addr,
            ready,
            acceptor_task,
        })
    }
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        self.acceptor_task.abort();
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some((stream, peer, _permit)) => (stream, peer),
            // The acceptor task never ends while `self` is alive.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, ServerName};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const BOUND: Duration = Duration::from_secs(10);
    const NEVER: Duration = Duration::from_secs(600);

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    struct Fixture {
        addr: SocketAddr,
        roots: rustls::RootCertStore,
        server: JoinHandle<io::Result<()>>,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn serve(handshake_timeout: Duration, capacity: usize) -> Fixture {
        let issued =
            rcgen::generate_simple_self_signed(vec!["coordinator.example".into()]).unwrap();
        let certificate = CertificateDer::from(issued.cert.der().to_vec());
        let key =
            PrivateKeyDer::from_pem_slice(issued.signing_key.serialize_pem().as_bytes()).unwrap();
        let tls = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate.clone()], key)
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener = TlsListener::with_capacity(tcp, tls, handshake_timeout, capacity).unwrap();
        let addr = listener.local_addr().unwrap();
        let router = axum::Router::new().route("/", axum::routing::get(|| async { "ok" }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        Fixture {
            addr,
            roots,
            server,
        }
    }

    async fn https_get(addr: SocketAddr, roots: rustls::RootCertStore) -> io::Result<String> {
        let client = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr).await?;
        let mut tls = tokio_rustls::TlsConnector::from(Arc::new(client))
            .connect(ServerName::try_from("coordinator.example").unwrap(), tcp)
            .await?;
        tls.write_all(b"GET / HTTP/1.1\r\nHost: coordinator.example\r\nConnection: close\r\n\r\n")
            .await?;
        let mut response = String::new();
        tls.read_to_string(&mut response).await?;
        Ok(response)
    }

    async fn assert_ok(fixture: &Fixture) {
        // test-hygiene: allow-wall-clock-deadline hang guard only; the loopback I/O result, not elapsed time, is asserted
        let response = tokio::time::timeout(BOUND, https_get(fixture.addr, fixture.roots.clone()))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.ends_with("ok"), "{response}");
    }

    #[tokio::test]
    async fn stalled_peer_does_not_block_other_handshakes_and_plaintext_never_reaches_router() {
        // The deadline exceeds the client bound: only concurrent handshakes pass.
        let fixture = serve(NEVER, MAX_PENDING_HANDSHAKES).await;
        let _stalled = TcpStream::connect(fixture.addr).await.unwrap();
        assert_ok(&fixture).await;
        let mut plain = TcpStream::connect(fixture.addr).await.unwrap();
        plain
            .write_all(b"GET / HTTP/1.1\r\nHost: coordinator.example\r\n\r\n")
            .await
            .unwrap();
        let mut bytes = Vec::new();
        // test-hygiene: allow-wall-clock-deadline hang guard only; the loopback I/O result, not elapsed time, is asserted
        let _ = tokio::time::timeout(BOUND, plain.read_to_end(&mut bytes))
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("200 OK"));
        assert_ok(&fixture).await;
    }

    #[tokio::test]
    async fn handshake_deadline_closes_stalled_peer() {
        let fixture = serve(Duration::from_millis(500), MAX_PENDING_HANDSHAKES).await;
        let mut stalled = TcpStream::connect(fixture.addr).await.unwrap();
        let mut buffer = [0; 1];
        // test-hygiene: allow-wall-clock-deadline hang guard only; the loopback I/O result, not elapsed time, is asserted
        let closed = tokio::time::timeout(BOUND, stalled.read(&mut buffer))
            .await
            .unwrap();
        assert!(matches!(closed, Ok(0) | Err(_)));
        assert_ok(&fixture).await;
    }

    #[tokio::test]
    async fn pending_cap_pauses_accept_until_a_slot_is_released() {
        let fixture = serve(NEVER, 2).await;
        let first = TcpStream::connect(fixture.addr).await.unwrap();
        let _second = TcpStream::connect(fixture.addr).await.unwrap();
        let mut waiting = tokio::spawn(https_get(fixture.addr, fixture.roots.clone()));
        assert!(
            // test-hygiene: allow-wall-clock-deadline the cap contract is that this handshake makes no progress while both slots are held; expiry is the asserted outcome
            tokio::time::timeout(Duration::from_secs(1), &mut waiting)
                .await
                .is_err(),
            "a third handshake must wait while two slots are held"
        );
        drop(first);
        // test-hygiene: allow-wall-clock-deadline hang guard only; the loopback I/O result, not elapsed time, is asserted
        let response = tokio::time::timeout(BOUND, waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(response.ends_with("ok"), "{response}");
    }
}

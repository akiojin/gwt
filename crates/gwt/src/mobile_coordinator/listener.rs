//! TLS accept boundary for the future coordinator listener (Issue #6476).
//!
//! Wraps a bound TCP listener so that `axum::serve` only ever sees connections
//! whose TLS handshake completed within a deadline. Handshakes run
//! concurrently, so a stalled or hostile peer cannot block other clients. At
//! most `MAX_CONNECTIONS` connections exist at once, counting handshakes,
//! queued connections and established connections until they close; at that
//! cap TCP accept pauses. One peer (an IPv4 address or an IPv6 /64) may hold at
//! most `MAX_CONNECTIONS_PER_PEER` of them; a connection over that share is
//! closed before its handshake, so one peer cannot occupy every slot. An
//! established connection whose reads and writes make no progress for the idle
//! deadline is closed. Failed or late handshakes are closed without reaching
//! the router. Handshakes already running when the listener is dropped end at
//! their deadline. This is not a route, Origin, registration or device
//! authority check.

use axum::serve::Listener;
use std::{
    collections::HashMap,
    future::Future,
    io,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{mpsc, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, Sleep},
};
use tokio_rustls::{server::TlsStream, TlsAcceptor};

/// Default handshake deadline for coordinator callers.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 64;
/// One peer's share of `MAX_CONNECTIONS`. Several devices behind one NAT share
/// it. A TCP proxy in front of the coordinator would make every client one peer.
const MAX_CONNECTIONS_PER_PEER: usize = 16;
/// Reads and writes that make no progress for this long close the connection.
/// hyper keeps a read pending while a handler runs, so a handler slower than
/// this deadline also loses its connection. A future long-lived or slow route
/// (for example a WebSocket) must keep traffic flowing within this deadline.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TlsListener {
    local_addr: SocketAddr,
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr, Slot)>,
    acceptor_task: JoinHandle<()>,
    idle_timeout: Duration,
}

/// An established connection. It holds its connection slot until dropped and
/// fails reads and writes that make no progress within the idle deadline.
pub struct Connection {
    stream: TlsStream<TcpStream>,
    _slot: Slot,
    idle_timeout: Duration,
    deadline: Pin<Box<Sleep>>,
}

type PeerCounts = Arc<Mutex<HashMap<IpAddr, usize>>>;

/// One total and one per-peer connection slot, both released on drop.
struct Slot {
    _total: OwnedSemaphorePermit,
    peers: PeerCounts,
    peer: IpAddr,
}

impl Slot {
    /// `None` when the peer already holds `per_peer` connections.
    fn claim(
        total: OwnedSemaphorePermit,
        peers: &PeerCounts,
        addr: SocketAddr,
        per_peer: usize,
    ) -> Option<Self> {
        let peer = peer_key(addr.ip());
        let mut counts = peers.lock().unwrap_or_else(PoisonError::into_inner);
        let held = counts.entry(peer).or_insert(0);
        if *held >= per_peer {
            return None;
        }
        *held += 1;
        Some(Self {
            _total: total,
            peers: peers.clone(),
            peer,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = self.peers.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = counts.get_mut(&self.peer) {
            *held -= 1;
            if *held == 0 {
                counts.remove(&self.peer);
            }
        }
    }
}

/// IPv4 address, or the /64 prefix of an IPv6 address (one IPv6 host usually
/// owns a whole /64). IPv4-mapped IPv6 addresses count as IPv4.
fn peer_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let [a, b, c, d, ..] = v6.segments();
            IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

impl Connection {
    fn progress<T>(
        &mut self,
        cx: &mut Context<'_>,
        poll: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        if poll.is_ready() {
            self.deadline
                .as_mut()
                .reset(Instant::now() + self.idle_timeout);
            return poll;
        }
        match self.deadline.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::ErrorKind::TimedOut.into())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.stream).poll_read(cx, buf);
        this.progress(cx, poll)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.stream).poll_write(cx, buf);
        this.progress(cx, poll)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.stream).poll_flush(cx);
        this.progress(cx, poll)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let poll = Pin::new(&mut this.stream).poll_shutdown(cx);
        this.progress(cx, poll)
    }
}

impl TlsListener {
    /// Start accepting on `tcp`. Must be called inside a Tokio runtime.
    pub fn new(
        tcp: TcpListener,
        tls: rustls::ServerConfig,
        handshake_timeout: Duration,
    ) -> io::Result<Self> {
        Self::with_capacity(tcp, tls, handshake_timeout, MAX_CONNECTIONS)
    }

    fn with_capacity(
        tcp: TcpListener,
        tls: rustls::ServerConfig,
        handshake_timeout: Duration,
        capacity: usize,
    ) -> io::Result<Self> {
        Self::with_limits(
            tcp,
            tls,
            handshake_timeout,
            capacity,
            MAX_CONNECTIONS_PER_PEER,
            IDLE_TIMEOUT,
        )
    }

    fn with_limits(
        mut tcp: TcpListener,
        tls: rustls::ServerConfig,
        handshake_timeout: Duration,
        capacity: usize,
        per_peer: usize,
        idle_timeout: Duration,
    ) -> io::Result<Self> {
        let local_addr = tcp.local_addr()?;
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        // A slot travels with the connection until it closes, so `capacity`
        // and `per_peer` bound handshakes, queued and established connections
        // together.
        let (sender, ready) = mpsc::channel(1);
        let pending = Arc::new(Semaphore::new(capacity));
        let peers = PeerCounts::default();
        let acceptor_task = tokio::spawn(async move {
            loop {
                let Ok(permit) = pending.clone().acquire_owned().await else {
                    return;
                };
                // axum's TcpListener impl retries and backs off on accept errors.
                let (stream, peer) = Listener::accept(&mut tcp).await;
                // Over its share, the peer is closed here and the total slot
                // returns at once rather than waiting like the total cap.
                let Some(slot) = Slot::claim(permit, &peers, peer, per_peer) else {
                    continue;
                };
                let (acceptor, sender) = (acceptor.clone(), sender.clone());
                tokio::spawn(async move {
                    if let Ok(Ok(stream)) =
                        tokio::time::timeout(handshake_timeout, acceptor.accept(stream)).await
                    {
                        let _ = sender.send((stream, peer, slot)).await;
                    }
                });
            }
        });
        Ok(Self {
            local_addr,
            ready,
            acceptor_task,
            idle_timeout,
        })
    }
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        self.acceptor_task.abort();
    }
}

impl Listener for TlsListener {
    type Io = Connection;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some((stream, peer, slot)) => (
                Connection {
                    stream,
                    _slot: slot,
                    idle_timeout: self.idle_timeout,
                    deadline: Box::pin(tokio::time::sleep(self.idle_timeout)),
                },
                peer,
            ),
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
        serve_with(handshake_timeout, capacity, capacity, NEVER).await
    }

    async fn serve_with(
        handshake_timeout: Duration,
        capacity: usize,
        per_peer: usize,
        idle_timeout: Duration,
    ) -> Fixture {
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
        let listener = TlsListener::with_limits(
            tcp,
            tls,
            handshake_timeout,
            capacity,
            per_peer,
            idle_timeout,
        )
        .unwrap();
        let addr = listener.local_addr().unwrap();
        let router = axum::Router::new().route("/", axum::routing::get(|| async { "ok" }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await });
        Fixture {
            addr,
            roots,
            server,
        }
    }

    async fn connect(
        addr: SocketAddr,
        roots: rustls::RootCertStore,
    ) -> io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
        let client = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr).await?;
        tokio_rustls::TlsConnector::from(Arc::new(client))
            .connect(ServerName::try_from("coordinator.example").unwrap(), tcp)
            .await
    }

    /// One keep-alive request; the connection stays open afterwards.
    async fn keep_alive(fixture: &Fixture) -> tokio_rustls::client::TlsStream<TcpStream> {
        // test-hygiene: allow-wall-clock-deadline hang guard only; the loopback I/O result, not elapsed time, is asserted
        tokio::time::timeout(BOUND, async {
            let mut tls = connect(fixture.addr, fixture.roots.clone()).await.unwrap();
            tls.write_all(b"GET / HTTP/1.1\r\nHost: coordinator.example\r\n\r\n")
                .await
                .unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0; 512];
            while !seen.ends_with(b"ok") {
                let read = tls.read(&mut buffer).await.unwrap();
                assert!(read > 0, "the keep-alive response must arrive");
                seen.extend_from_slice(&buffer[..read]);
            }
            tls
        })
        .await
        .unwrap()
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
        let fixture = serve(NEVER, MAX_CONNECTIONS).await;
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
        let fixture = serve(Duration::from_millis(500), MAX_CONNECTIONS).await;
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

    #[tokio::test]
    async fn established_connections_hold_their_slot_until_closed() {
        let fixture = serve(NEVER, 2).await;
        let first = keep_alive(&fixture).await;
        let _second = keep_alive(&fixture).await;
        let mut waiting = tokio::spawn(https_get(fixture.addr, fixture.roots.clone()));
        assert!(
            // test-hygiene: allow-wall-clock-deadline the cap contract is that this handshake makes no progress while both established connections are open; expiry is the asserted outcome
            tokio::time::timeout(Duration::from_secs(1), &mut waiting)
                .await
                .is_err(),
            "a third connection must wait while two established connections are open"
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

    #[tokio::test]
    async fn idle_connection_closes_at_the_idle_deadline() {
        let fixture = serve_with(
            NEVER,
            MAX_CONNECTIONS,
            MAX_CONNECTIONS,
            Duration::from_millis(500),
        )
        .await;
        let mut idle = keep_alive(&fixture).await;
        let mut buffer = [0; 1];
        // test-hygiene: allow-wall-clock-deadline hang guard only; the server closing the idle connection, not elapsed time, is asserted
        let closed = tokio::time::timeout(BOUND, idle.read(&mut buffer))
            .await
            .expect("an idle connection must be closed by the server");
        assert!(matches!(closed, Ok(0) | Err(_)));
        assert_ok(&fixture).await;
    }

    #[tokio::test]
    async fn one_peer_cannot_hold_more_than_its_share_of_slots() {
        let fixture = serve_with(NEVER, 8, 2, NEVER).await;
        let first = keep_alive(&fixture).await;
        let _second = keep_alive(&fixture).await;
        // test-hygiene: allow-wall-clock-deadline hang guard only; the refused handshake, not elapsed time, is asserted
        let refused = tokio::time::timeout(BOUND, connect(fixture.addr, fixture.roots.clone()))
            .await
            .expect("a peer over its share must be closed, not left waiting");
        assert!(
            refused.is_err(),
            "a peer over its share must not complete TLS"
        );
        drop(first);
        // The server frees the slot only after it reads the close, and an
        // over-share connection is refused rather than queued, so retry.
        // test-hygiene: allow-wall-clock-deadline hang guard only; the eventual 200 response, not elapsed time, is asserted
        let response = tokio::time::timeout(BOUND, async {
            loop {
                if let Ok(response) = https_get(fixture.addr, fixture.roots.clone()).await {
                    return response;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("a closed connection must return its peer slot");
        assert!(response.ends_with("ok"), "{response}");
    }

    #[test]
    fn peer_slots_are_counted_per_peer_and_returned_on_drop() {
        let total = Arc::new(Semaphore::new(8));
        let peers = PeerCounts::default();
        let claim = |addr: &str| {
            let permit = total.clone().try_acquire_owned().unwrap();
            Slot::claim(permit, &peers, addr.parse().unwrap(), 2)
        };
        let first = claim("203.0.113.7:1").unwrap();
        let _second = claim("203.0.113.7:2").unwrap();
        assert!(claim("203.0.113.7:3").is_none());
        let other = claim("198.51.100.1:1").unwrap();
        assert_eq!(
            total.available_permits(),
            5,
            "a refusal returns its total slot"
        );
        drop(first);
        assert!(claim("203.0.113.7:4").is_some());
        drop(other);
        assert!(!peers
            .lock()
            .unwrap()
            .contains_key(&peer_key("198.51.100.1".parse().unwrap())));
    }

    #[test]
    fn peer_key_unmaps_ipv4_and_groups_ipv6_by_slash_64() {
        let key = |text: &str| peer_key(text.parse().unwrap());
        assert_eq!(key("203.0.113.7"), key("203.0.113.7"));
        assert_ne!(key("203.0.113.7"), key("203.0.113.8"));
        assert_eq!(key("::ffff:203.0.113.7"), key("203.0.113.7"));
        assert_eq!(key("2001:db8:1:2::1"), key("2001:db8:1:2:ffff::9"));
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
    }
}

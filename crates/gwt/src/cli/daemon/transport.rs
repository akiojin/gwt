//! Platform IPC transport for the runtime daemon (Issue #3526).
//!
//! The daemon protocol (handshake + newline-delimited JSON frames) is
//! transport-agnostic. This module hides the one place where the platform
//! differs:
//!
//! - Unix: a Unix domain socket bound at the endpoint's `.sock` path.
//! - Windows: a named pipe `\\.\pipe\gwtd-<stem>-<hash>` derived from the
//!   endpoint file path, so distinct `GWT_HOME` / scope pairs never share a
//!   pipe name. Remote clients are rejected at the pipe level; the existing
//!   `auth_token` handshake (token persisted under the user-private
//!   `~/.gwt`) remains the authorization boundary.
//!
//! Both sides expose the same surface: [`IpcListener::bind`] +
//! [`IpcListener::accept`] for the server and [`IpcStream::connect`] for the
//! client. [`IpcStream`] implements `AsyncRead + AsyncWrite`, so callers
//! split it with `tokio::io::split` and never see the platform type.

use std::{
    io,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Read half type produced by [`IpcStream::into_split`].
pub(crate) type IpcReadHalf = tokio::io::ReadHalf<IpcStream>;
/// Write half type produced by [`IpcStream::into_split`].
pub(crate) type IpcWriteHalf = tokio::io::WriteHalf<IpcStream>;

/// Derive the daemon bind address from the persisted endpoint file path.
///
/// Unix keeps the historical `<endpoint>.sock` sibling. Windows maps the
/// path onto a named pipe name (see [`pipe_name_for`]).
pub(crate) fn bind_address_for_endpoint_path(endpoint_path: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        endpoint_path.with_extension("sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(pipe_name_for(&endpoint_path.to_string_lossy()))
    }
}

/// Prepare the filesystem for a bind: create the socket's parent directory
/// on Unix. Named pipes live in the pipe namespace, so Windows has nothing
/// to prepare.
pub(crate) fn prepare_bind_parent(bind: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Some(parent) = bind.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        let _ = bind;
        Ok(())
    }
}

/// Remove a stale socket file left behind by a crashed daemon. Named pipes
/// disappear with their owning process, so Windows is a no-op.
pub(crate) fn cleanup_stale_bind(bind: &Path) {
    #[cfg(unix)]
    {
        if bind.exists() {
            let _ = std::fs::remove_file(bind);
        }
    }
    #[cfg(windows)]
    {
        let _ = bind;
    }
}

/// Windows named-pipe name for an arbitrary bind string.
///
/// A value that already lives in the pipe namespace (`\\.\pipe\...`) is
/// returned unchanged, so persisted endpoint binds round-trip. Any other
/// value (a test's temp path, a legacy `.sock` path) is hashed into a
/// unique, length-bounded pipe name. Pipe names cannot contain path
/// separators, hence the hash rather than the raw path.
#[cfg(any(windows, test))]
pub(crate) fn pipe_name_for(bind: &str) -> String {
    const PIPE_PREFIX: &str = r"\\.\pipe\";
    if bind.len() >= PIPE_PREFIX.len()
        && bind[..PIPE_PREFIX.len()].eq_ignore_ascii_case(PIPE_PREFIX)
    {
        return bind.to_string();
    }
    // Split on both separators by hand so the derived name is identical on
    // every host (a `\` is not a separator for `std::path` on Unix).
    let stem = bind
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("daemon")
        .rsplit_once('.')
        .map_or("daemon", |(stem, _)| stem);
    let stem = stem
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(32)
        .collect::<String>();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bind.as_bytes());
        hex::encode(hasher.finalize())
    };
    format!("{PIPE_PREFIX}gwtd-{stem}-{}", &digest[..16])
}

/// Listening side of the daemon transport.
pub(crate) struct IpcListener {
    #[cfg(unix)]
    inner: tokio::net::UnixListener,
    #[cfg(windows)]
    pipe_name: String,
    #[cfg(windows)]
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl IpcListener {
    /// Bind the daemon transport at `bind`.
    pub(crate) fn bind(bind: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                inner: tokio::net::UnixListener::bind(bind)?,
            })
        }
        #[cfg(windows)]
        {
            let pipe_name = pipe_name_for(&bind.to_string_lossy());
            let first = windows_impl::create_server_instance(&pipe_name, true)?;
            Ok(Self {
                pipe_name,
                pending: Some(first),
            })
        }
    }

    /// Wait for the next client connection.
    pub(crate) async fn accept(&mut self) -> io::Result<IpcStream> {
        #[cfg(unix)]
        {
            let (stream, _addr) = self.inner.accept().await?;
            Ok(IpcStream::Unix(stream))
        }
        #[cfg(windows)]
        {
            let server = match self.pending.take() {
                Some(server) => server,
                None => windows_impl::create_server_instance(&self.pipe_name, false)?,
            };
            server.connect().await?;
            // Create the next instance before handing this one out so a
            // client that connects immediately after finds a listener.
            match windows_impl::create_server_instance(&self.pipe_name, false) {
                Ok(next) => self.pending = Some(next),
                Err(error) => {
                    tracing::warn!(
                        target: "gwtd::daemon",
                        error = %error,
                        "named pipe: failed to pre-create next server instance"
                    );
                }
            }
            Ok(IpcStream::PipeServer(server))
        }
    }
}

/// A connected daemon transport stream.
pub(crate) enum IpcStream {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    PipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
    #[cfg(windows)]
    PipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl IpcStream {
    /// Connect to a daemon listening at `bind` (a socket path on Unix, a
    /// pipe name or path-derived pipe name on Windows).
    pub(crate) async fn connect(bind: &str) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::Unix(tokio::net::UnixStream::connect(bind).await?))
        }
        #[cfg(windows)]
        {
            windows_impl::connect_client(&pipe_name_for(bind)).await
        }
    }

    /// Split into independently owned read / write halves.
    pub(crate) fn into_split(self) -> (IpcReadHalf, IpcWriteHalf) {
        tokio::io::split(self)
    }
}

impl AsyncRead for IpcStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            Self::PipeServer(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            Self::PipeClient(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for IpcStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, data),
            #[cfg(windows)]
            Self::PipeServer(stream) => Pin::new(stream).poll_write(cx, data),
            #[cfg(windows)]
            Self::PipeClient(stream) => Pin::new(stream).poll_write(cx, data),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            Self::PipeServer(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            Self::PipeClient(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            Self::PipeServer(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            Self::PipeClient(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::{io, time::Duration};

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::IpcStream;

    /// `ERROR_PIPE_BUSY`: every server instance is mid-connect; retry.
    const ERROR_PIPE_BUSY: i32 = 231;
    const BUSY_RETRY_DELAY: Duration = Duration::from_millis(20);
    const BUSY_RETRY_LIMIT: usize = 50;

    pub(super) fn create_server_instance(
        pipe_name: &str,
        first: bool,
    ) -> io::Result<NamedPipeServer> {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(pipe_name)
    }

    pub(super) async fn connect_client(pipe_name: &str) -> io::Result<IpcStream> {
        let mut attempts = 0;
        loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(IpcStream::PipeClient(client)),
                Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    attempts += 1;
                    if attempts >= BUSY_RETRY_LIMIT {
                        return Err(error);
                    }
                    tokio::time::sleep(BUSY_RETRY_DELAY).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// Test-only readiness probe: resolves once a daemon accepts connections at
/// `bind`. Unix checks the socket file; Windows opens (and immediately
/// drops) a pipe client because pipe names are not visible on the file
/// system.
#[cfg(test)]
pub(crate) async fn wait_until_bound(bind: &Path) {
    for _ in 0..100 {
        #[cfg(unix)]
        let ready = bind.exists();
        #[cfg(windows)]
        let ready = IpcStream::connect(&bind.to_string_lossy()).await.is_ok();
        if ready {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("daemon transport never became ready at {}", bind.display());
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    /// AC-1 / AC-2: the same listener + client surface must round-trip a
    /// newline-delimited frame on every supported host. On Windows this is
    /// the named-pipe path; on Unix it is the historical socket path.
    #[tokio::test]
    async fn listener_and_client_round_trip_a_line_on_this_platform() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let bind = bind_address_for_endpoint_path(&temp.path().join("feedfacecafebeef.json"));
        prepare_bind_parent(&bind).expect("prepare parent");
        cleanup_stale_bind(&bind);

        let mut listener = IpcListener::bind(&bind).expect("bind");
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.expect("accept");
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            assert_eq!(line, "ping\n");
            write_half.write_all(b"pong\n").await.expect("write reply");
            write_half.flush().await.expect("flush");
        });

        wait_until_bound(&bind).await;
        let client = IpcStream::connect(&bind.to_string_lossy())
            .await
            .expect("connect");
        let (read_half, mut write_half) = client.into_split();
        write_half
            .write_all(b"ping\n")
            .await
            .expect("write request");
        let mut reader = BufReader::new(read_half);
        let mut reply = String::new();
        reader.read_line(&mut reply).await.expect("read reply");
        assert_eq!(reply, "pong\n");
        server.await.expect("server task");
    }

    /// AC-1: a second daemon for the same scope must fail to bind instead
    /// of silently sharing the transport with the first one.
    #[tokio::test]
    async fn second_bind_on_the_same_address_is_rejected() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let bind = bind_address_for_endpoint_path(&temp.path().join("feedfacecafebeef.json"));
        prepare_bind_parent(&bind).expect("prepare parent");
        let _first = IpcListener::bind(&bind).expect("first bind");
        assert!(
            IpcListener::bind(&bind).is_err(),
            "second bind must be rejected while the first listener is alive"
        );
    }

    /// AC-5: connecting to a daemon that is not running must fail fast with
    /// an error rather than hang, so `daemon.status` can report `failed:`.
    #[tokio::test]
    async fn connect_to_an_unbound_address_fails() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let bind = bind_address_for_endpoint_path(&temp.path().join("does-not-exist.json"));
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            IpcStream::connect(&bind.to_string_lossy()),
        )
        .await
        .expect("connect must not hang");
        assert!(result.is_err(), "connect to an unbound address must fail");
    }

    #[test]
    fn bind_address_is_derived_from_the_endpoint_path() {
        let endpoint = Path::new("/home/user/.gwt/projects/repo/runtime/daemon/abcd.json");
        let bind = bind_address_for_endpoint_path(endpoint);
        #[cfg(unix)]
        assert_eq!(
            bind,
            Path::new("/home/user/.gwt/projects/repo/runtime/daemon/abcd.sock")
        );
        #[cfg(windows)]
        assert!(
            bind.to_string_lossy().starts_with(r"\\.\pipe\gwtd-abcd-"),
            "unexpected pipe name {}",
            bind.display()
        );
    }

    #[test]
    fn pipe_name_is_stable_unique_and_idempotent() {
        let a = pipe_name_for(r"C:\Users\me\.gwt\projects\r\runtime\daemon\abcd.json");
        let b = pipe_name_for(r"C:\Users\me\.gwt\projects\r\runtime\daemon\abcd.json");
        let c = pipe_name_for(r"C:\Users\other\.gwt\projects\r\runtime\daemon\abcd.json");
        assert_eq!(a, b, "same path must map to the same pipe name");
        assert_ne!(a, c, "different homes must not share a pipe name");
        assert!(a.starts_with(r"\\.\pipe\gwtd-abcd-"), "{a}");
        assert!(a.len() < 256, "pipe names are limited to 256 characters");
        assert_eq!(pipe_name_for(&a), a, "pipe names must round-trip unchanged");
        assert_eq!(
            pipe_name_for(r"\\.\PIPE\already-a-pipe"),
            r"\\.\PIPE\already-a-pipe"
        );
    }
}

//! Stable embedded-server port selection and listener preparation.
//!
//! The listener stays unserved until any required settings update succeeds.
//! This makes startup transactional: a failed atomic save drops the raw
//! listener synchronously, before a URL or server task can be published.

use gwt_config::{ConfigError, Settings};
use std::{
    io,
    net::{IpAddr, SocketAddr, TcpListener},
    num::NonZeroU16,
    path::Path,
};
use tokio::{net::TcpSocket, runtime::Handle};

const LISTEN_BACKLOG: u32 = 1024;

/// Inputs that affect stable-port selection independently of the bind IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StablePortRequest {
    explicit_port: Option<u16>,
    forced_secondary: bool,
}

impl StablePortRequest {
    pub fn new(explicit_port: Option<u16>, forced_secondary: bool) -> Self {
        Self {
            explicit_port,
            forced_secondary,
        }
    }
}

/// Describes how the prepared listener's port was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StablePortOutcome {
    /// An explicit CLI port or forced-secondary ephemeral port; never saved.
    Transient,
    /// The persisted port bound successfully and required no write.
    Reused,
    /// No port was persisted, so the selected ephemeral port was saved.
    Stored,
    /// The persisted port was occupied and was atomically replaced.
    Replaced { previous: NonZeroU16 },
}

/// A bound but not-yet-served listener owned by the startup transaction.
#[derive(Debug)]
pub struct PreparedEmbeddedListener {
    listener: TcpListener,
    local_addr: SocketAddr,
    outcome: StablePortOutcome,
}

impl PreparedEmbeddedListener {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn port(&self) -> NonZeroU16 {
        NonZeroU16::new(self.local_addr.port())
            .expect("a bound TCP listener must have a non-zero port")
    }

    pub fn outcome(&self) -> StablePortOutcome {
        self.outcome
    }

    pub fn into_listener(self) -> TcpListener {
        self.listener
    }
}

/// Fatal errors while preparing the embedded-server listener.
#[derive(Debug, thiserror::Error)]
pub enum StablePortError {
    #[error("failed to load embedded-server port settings: {0}")]
    Load(#[source] ConfigError),
    #[error("failed to bind embedded server to port {requested_port}: {source}")]
    Bind {
        requested_port: u16,
        #[source]
        source: io::Error,
    },
    #[error("embedded server reported invalid bound port 0")]
    InvalidBoundPort,
    #[error("failed to save embedded-server port {port}: {source}")]
    Save {
        port: NonZeroU16,
        #[source]
        source: ConfigError,
    },
}

impl StablePortError {
    fn is_addr_in_use(&self) -> bool {
        matches!(
            self,
            Self::Bind { source, .. } if source.kind() == io::ErrorKind::AddrInUse
        )
    }
}

/// Bind the startup listener and apply the persisted stable-port policy.
pub fn prepare_stable_listener(
    runtime: &Handle,
    config_path: &Path,
    bind_addr: IpAddr,
    request: StablePortRequest,
) -> Result<PreparedEmbeddedListener, StablePortError> {
    bind_stable_port_with_store(
        request,
        || {
            if config_path.exists() {
                Settings::load_from_path(config_path)
            } else {
                Ok(Settings::default())
            }
        },
        |settings| {
            let port =
                settings
                    .server
                    .embedded_port
                    .ok_or_else(|| ConfigError::ValidationError {
                        reason: "embedded server port is absent during persistence".to_string(),
                    })?;
            Settings::persist_embedded_port(config_path, port)
        },
        |port| bind_reusable_listener(runtime, SocketAddr::new(bind_addr, port)),
    )
}

fn bind_reusable_listener(runtime: &Handle, addr: SocketAddr) -> io::Result<TcpListener> {
    let _runtime_guard = runtime.enter();
    let socket = match addr {
        SocketAddr::V4(_) => TcpSocket::new_v4()?,
        SocketAddr::V6(_) => TcpSocket::new_v6()?,
    };
    #[cfg(unix)]
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(LISTEN_BACKLOG)?.into_std()
}

fn bind_stable_port_with_store<Load, Save, Bind>(
    request: StablePortRequest,
    load: Load,
    save: Save,
    mut bind: Bind,
) -> Result<PreparedEmbeddedListener, StablePortError>
where
    Load: FnOnce() -> gwt_config::Result<Settings>,
    Save: FnOnce(&Settings) -> gwt_config::Result<()>,
    Bind: FnMut(u16) -> io::Result<TcpListener>,
{
    if let Some(port) = request.explicit_port {
        return bind_once(&mut bind, port, StablePortOutcome::Transient);
    }
    if request.forced_secondary {
        return bind_once(&mut bind, 0, StablePortOutcome::Transient);
    }

    let mut settings = load().map_err(StablePortError::Load)?;
    if let Some(saved_port) = settings.server.embedded_port {
        match bind_once(&mut bind, saved_port.get(), StablePortOutcome::Reused) {
            Ok(prepared) => return Ok(prepared),
            Err(error) if error.is_addr_in_use() => {
                let prepared = bind_once(
                    &mut bind,
                    0,
                    StablePortOutcome::Replaced {
                        previous: saved_port,
                    },
                )?;
                persist_selected_port(&mut settings, prepared.port(), save)?;
                return Ok(prepared);
            }
            Err(error) => return Err(error),
        }
    }

    let prepared = bind_once(&mut bind, 0, StablePortOutcome::Stored)?;
    persist_selected_port(&mut settings, prepared.port(), save)?;
    Ok(prepared)
}

fn bind_once<Bind>(
    bind: &mut Bind,
    requested_port: u16,
    outcome: StablePortOutcome,
) -> Result<PreparedEmbeddedListener, StablePortError>
where
    Bind: FnMut(u16) -> io::Result<TcpListener>,
{
    let listener = bind(requested_port).map_err(|source| StablePortError::Bind {
        requested_port,
        source,
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|source| StablePortError::Bind {
            requested_port,
            source,
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|source| StablePortError::Bind {
            requested_port,
            source,
        })?;
    if local_addr.port() == 0 {
        return Err(StablePortError::InvalidBoundPort);
    }
    Ok(PreparedEmbeddedListener {
        listener,
        local_addr,
        outcome,
    })
}

fn persist_selected_port<Save>(
    settings: &mut Settings,
    port: NonZeroU16,
    save: Save,
) -> Result<(), StablePortError>
where
    Save: FnOnce(&Settings) -> gwt_config::Result<()>,
{
    settings.server.embedded_port = Some(port);
    save(settings).map_err(|source| StablePortError::Save { port, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_config::{ConfigError, Settings};
    use std::{
        cell::{Cell, RefCell},
        io,
        net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
        num::NonZeroU16,
    };
    use tokio::runtime::Runtime;

    fn request(explicit_port: Option<u16>, forced_secondary: bool) -> StablePortRequest {
        StablePortRequest::new(explicit_port, forced_secondary)
    }

    fn bind_loopback(port: u16) -> io::Result<TcpListener> {
        TcpListener::bind((Ipv4Addr::LOCALHOST, port))
    }

    fn listener_port(listener: &TcpListener) -> u16 {
        listener.local_addr().expect("listener address").port()
    }

    fn settings_with_port(port: u16) -> Settings {
        let mut settings = Settings::default();
        settings.server.embedded_port = NonZeroU16::new(port);
        settings
    }

    #[test]
    fn first_implicit_bind_saves_actual_port_and_restart_reuses_it() {
        let persisted = RefCell::new(Settings::default());
        let save_count = Cell::new(0);
        let requested_ports = RefCell::new(Vec::new());

        let first = bind_stable_port_with_store(
            request(None, false),
            || Ok(persisted.borrow().clone()),
            |settings| {
                save_count.set(save_count.get() + 1);
                *persisted.borrow_mut() = settings.clone();
                Ok(())
            },
            |port| {
                requested_ports.borrow_mut().push(port);
                bind_loopback(port)
            },
        )
        .expect("first implicit bind");
        let first_port = first.port();
        assert_ne!(first_port.get(), 0);
        assert_eq!(persisted.borrow().server.embedded_port, Some(first_port));
        assert_eq!(save_count.get(), 1);
        assert_eq!(first.outcome(), StablePortOutcome::Stored);
        drop(first);

        let second = bind_stable_port_with_store(
            request(None, false),
            || Ok(persisted.borrow().clone()),
            |_| panic!("successful saved-port reuse must not rewrite settings"),
            |port| {
                requested_ports.borrow_mut().push(port);
                bind_loopback(port)
            },
        )
        .expect("restart bind");

        assert_eq!(second.port(), first_port);
        assert_eq!(requested_ports.borrow().as_slice(), &[0, first_port.get()]);
        assert_eq!(second.outcome(), StablePortOutcome::Reused);
    }

    #[test]
    fn occupied_saved_port_falls_back_to_ephemeral_and_replaces_setting() {
        let occupied = bind_loopback(0).expect("occupy loopback port");
        let saved_port = NonZeroU16::new(listener_port(&occupied)).expect("non-zero port");
        let persisted = RefCell::new(settings_with_port(saved_port.get()));
        let requested_ports = RefCell::new(Vec::new());

        let outcome = bind_stable_port_with_store(
            request(None, false),
            || Ok(persisted.borrow().clone()),
            |settings| {
                *persisted.borrow_mut() = settings.clone();
                Ok(())
            },
            |port| {
                requested_ports.borrow_mut().push(port);
                bind_loopback(port)
            },
        )
        .expect("AddrInUse falls back");

        assert_ne!(outcome.port(), saved_port);
        assert_eq!(
            outcome.outcome(),
            StablePortOutcome::Replaced {
                previous: saved_port
            }
        );
        assert_eq!(requested_ports.borrow().as_slice(), &[saved_port.get(), 0]);
        assert_eq!(
            persisted.borrow().server.embedded_port,
            Some(outcome.port())
        );
    }

    #[test]
    fn explicit_zero_is_transient_and_skips_settings_io() {
        let outcome = bind_stable_port_with_store(
            request(Some(0), false),
            || panic!("explicit port must not load stable-port settings"),
            |_| panic!("explicit port must not save stable-port settings"),
            bind_loopback,
        )
        .expect("explicit ephemeral bind");

        assert_ne!(outcome.port().get(), 0);
        assert_eq!(outcome.outcome(), StablePortOutcome::Transient);
    }

    #[test]
    fn explicit_occupied_port_does_not_fallback_or_touch_settings() {
        let occupied = bind_loopback(0).expect("occupy loopback port");
        let port = listener_port(&occupied);
        let attempts = RefCell::new(Vec::new());

        let error = bind_stable_port_with_store(
            request(Some(port), false),
            || panic!("explicit port must not load stable-port settings"),
            |_| panic!("explicit port must not save stable-port settings"),
            |requested| {
                attempts.borrow_mut().push(requested);
                bind_loopback(requested)
            },
        )
        .expect_err("explicit occupied port must fail");

        match error {
            StablePortError::Bind {
                requested_port,
                source,
            } => {
                assert_eq!(requested_port, port);
                assert_eq!(source.kind(), io::ErrorKind::AddrInUse);
            }
            other => panic!("unexpected error: {other}"),
        }
        assert_eq!(attempts.borrow().as_slice(), &[port]);
    }

    #[test]
    fn forced_secondary_uses_ephemeral_port_without_settings_io() {
        let requested_ports = RefCell::new(Vec::new());

        let outcome = bind_stable_port_with_store(
            request(None, true),
            || panic!("forced secondary must ignore persisted port"),
            |_| panic!("forced secondary must not save a port"),
            |port| {
                requested_ports.borrow_mut().push(port);
                bind_loopback(port)
            },
        )
        .expect("forced secondary bind");

        assert_eq!(requested_ports.borrow().as_slice(), &[0]);
        assert_ne!(outcome.port().get(), 0);
        assert_eq!(outcome.outcome(), StablePortOutcome::Transient);
    }

    #[test]
    fn non_addr_in_use_bind_error_propagates_without_save() {
        let saved_port = NonZeroU16::new(49152).expect("fixture port");
        let settings = settings_with_port(saved_port.get());
        let save_count = Cell::new(0);

        let error = bind_stable_port_with_store(
            request(None, false),
            || Ok(settings.clone()),
            |_| {
                save_count.set(save_count.get() + 1);
                Ok(())
            },
            |requested| {
                assert_eq!(requested, saved_port.get());
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected permission denial",
                ))
            },
        )
        .expect_err("non-AddrInUse errors are fatal");

        match error {
            StablePortError::Bind { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied)
            }
            other => panic!("unexpected error: {other}"),
        }
        assert_eq!(save_count.get(), 0);
    }

    #[test]
    fn persistence_failure_releases_new_listener_before_returning_error() {
        let selected_port = Cell::new(None);

        let error = bind_stable_port_with_store(
            request(None, false),
            || Ok(Settings::default()),
            |settings| {
                selected_port.set(settings.server.embedded_port);
                Err(ConfigError::WriteError {
                    reason: "injected atomic writer failure".to_string(),
                })
            },
            bind_loopback,
        )
        .expect_err("persistence failure is fatal");

        assert!(matches!(error, StablePortError::Save { .. }));
        let selected_port = selected_port.get().expect("writer saw selected port");
        let rebound = bind_loopback(selected_port.get())
            .expect("failed transaction must release its listener before returning");
        assert_eq!(listener_port(&rebound), selected_port.get());
    }

    #[test]
    fn malformed_config_propagates_without_binding_or_overwrite() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let malformed = "[server\nembedded_port = 12345\n";
        std::fs::write(&config_path, malformed).expect("write malformed config");
        let runtime = Runtime::new().expect("runtime");
        let error = prepare_stable_listener(
            runtime.handle(),
            &config_path,
            Ipv4Addr::LOCALHOST.into(),
            request(None, false),
        )
        .expect_err("malformed config is fatal");

        assert!(matches!(error, StablePortError::Load(_)));
        assert_eq!(
            std::fs::read_to_string(&config_path).expect("read config"),
            malformed
        );
    }

    #[test]
    fn file_store_persists_port_without_changing_unrelated_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let settings = Settings {
            debug: true,
            default_base_branch: "integration".to_string(),
            ..Settings::default()
        };
        settings.save(&config_path).expect("seed config");
        let runtime = Runtime::new().expect("runtime");

        let outcome = prepare_stable_listener(
            runtime.handle(),
            &config_path,
            Ipv4Addr::LOCALHOST.into(),
            request(None, false),
        )
        .expect("implicit bind");
        let selected_port = outcome.port();

        let reloaded = Settings::load_from_path(&config_path).expect("reload settings");
        assert!(reloaded.debug);
        assert_eq!(reloaded.default_base_branch, "integration");
        assert_eq!(reloaded.server.embedded_port, Some(selected_port));
    }

    #[test]
    fn accepted_connection_does_not_force_saved_port_replacement_on_restart() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let runtime = Runtime::new().expect("runtime");

        let first = prepare_stable_listener(
            runtime.handle(),
            &config_path,
            Ipv4Addr::LOCALHOST.into(),
            request(None, false),
        )
        .expect("first listener");
        let first_port = first.port();
        let listener = first.into_listener();
        listener
            .set_nonblocking(false)
            .expect("blocking accept fixture");
        let client = TcpStream::connect((Ipv4Addr::LOCALHOST, first_port.get()))
            .expect("connect to first listener");
        let (accepted, _) = listener.accept().expect("accept fixture connection");
        accepted
            .shutdown(Shutdown::Both)
            .expect("actively close server connection");
        drop(accepted);
        drop(listener);
        drop(client);

        let restarted = prepare_stable_listener(
            runtime.handle(),
            &config_path,
            Ipv4Addr::LOCALHOST.into(),
            request(None, false),
        )
        .expect("restart must reuse the saved port after a real connection");

        assert_eq!(restarted.port(), first_port);
        assert_eq!(restarted.outcome(), StablePortOutcome::Reused);
    }
}

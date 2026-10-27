//! Private Rust GUI client; the daemon token never enters a browser projection.
use super::events::{OwnerCommand, OwnerReply, OwnerRequest};
#[derive(Clone)]
pub struct ProjectRuntimeClient {
    url: String,
    token: String,
    project_key: String,
    http: reqwest::Client,
    identity: Option<crate::daemon_subscriber::ProjectOwnerIdentity>,
}
impl ProjectRuntimeClient {
    pub fn new(endpoint: &gwt_core::daemon::DaemonEndpoint) -> Result<Self, String> {
        Ok(Self {
            url: endpoint
                .project_transport_url
                .clone()
                .ok_or("daemon has no project transport")?,
            token: endpoint.auth_token.clone(),
            project_key: endpoint.scope.repo_hash.clone(),
            http: reqwest::Client::new(),
            identity: None,
        })
    }

    /// Resolve and authenticate this project's current physical owner. The
    /// existing health validator pins PID, birth and instance to discovery.
    pub async fn from_current_project(
        project_root: &std::path::Path,
        gwt_home: &std::path::Path,
    ) -> Result<Self, String> {
        use gwt_core::daemon::{DaemonBootstrapAction, RuntimeScope, RuntimeTarget};
        let scope = RuntimeScope::from_project_root(project_root, RuntimeTarget::Host)
            .map_err(|error| error.to_string())?;
        let action = gwt_core::daemon::resolve_bootstrap_action_for_version(
            gwt_home,
            &scope,
            gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
            env!("CARGO_PKG_VERSION"),
            crate::process::is_process_alive,
        )
        .map_err(|error| error.to_string())?;
        let DaemonBootstrapAction::Reuse(endpoint) = action else {
            return Err("Project execution owner is not ready".into());
        };
        let mut client = Self::new(&endpoint)?;
        client.identity =
            Some(crate::daemon_subscriber::inspect_project_owner(&client, &endpoint).await?);
        Ok(client)
    }

    pub fn identity(&self) -> Option<&crate::daemon_subscriber::ProjectOwnerIdentity> {
        self.identity.as_ref()
    }

    /// Used from the GUI's existing background tasks. The established
    /// supervisor remains the only child/ensure registry.
    pub async fn connect_for_gui(
        project_root: std::path::PathBuf,
        supervisor: std::sync::Arc<crate::daemon_supervisor::DaemonSupervisor>,
    ) -> Result<Self, String> {
        let gwt_home = gwt_core::paths::gwt_home();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let error = match Self::from_current_project(&project_root, &gwt_home).await {
                Ok(client) => return Ok(client),
                Err(error) => error,
            };
            if tokio::time::Instant::now() >= deadline {
                return Err(error);
            }
            let ensure = supervisor.clone();
            let root = project_root.clone();
            tokio::task::spawn_blocking(move || ensure.ensure_running(&root))
                .await
                .map_err(|error| error.to_string())??;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    pub async fn events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        self.events_with_gui_presence(false).await
    }
    /// An authenticated Rust GUI connection owns exactly one residency lease.
    pub async fn gui_events(
        &self,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        self.events_with_gui_presence(true).await
    }
    async fn events_with_gui_presence(
        &self,
        gui_presence: bool,
    ) -> Result<impl futures_util::Stream<Item = Result<super::events::OwnerEvent, String>>, String>
    {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let url = format!(
            "{}/private/project/events",
            self.url.replacen("http://", "ws://", 1)
        );
        let mut request = url
            .into_client_request()
            .map_err(|error| error.to_string())?;
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", self.token)
                .parse()
                .map_err(|error: axum::http::header::InvalidHeaderValue| error.to_string())?,
        );
        if gui_presence {
            request.headers_mut().insert(
                "x-gwt-gui-presence",
                "1".parse().expect("static presence header"),
            );
        }
        #[cfg(windows)]
        let (stream, _) = {
            use tokio_tungstenite::tungstenite::error::{Error, UrlError};

            let host = request
                .uri()
                .host()
                .ok_or_else(|| Error::Url(UrlError::NoHostName).to_string())?;
            // The private owner publishes plain HTTP. Resolve the same host
            // and try every address in order, as connect_async does, but create
            // the socket with NO_HANDLE_INHERIT before any concurrent spawn.
            // Otherwise a later project's daemon can retain this GUI lease.
            let port = match request.uri().scheme_str() {
                Some("ws") => request.uri().port_u16().unwrap_or(80),
                _ => return Err(Error::Url(UrlError::UnsupportedUrlScheme).to_string()),
            };
            let addresses = tokio::net::lookup_host(format!("{host}:{port}"))
                .await
                .map_err(|error| error.to_string())?;
            let mut connected = None;
            let mut last_error = None;
            for address in addresses {
                let socket = if address.is_ipv4() {
                    tokio::net::TcpSocket::new_v4()
                } else {
                    tokio::net::TcpSocket::new_v6()
                };
                let result = match socket {
                    Ok(socket) => socket.connect(address).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(socket) => {
                        connected = Some(socket);
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            let socket = connected.ok_or_else(|| {
                last_error
                    .unwrap_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "could not resolve to any address",
                        )
                    })
                    .to_string()
            })?;
            tokio_tungstenite::client_async(request, socket)
                .await
                .map_err(|error| error.to_string())?
        };
        #[cfg(not(windows))]
        let (stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|error| error.to_string())?;
        Ok(stream.filter_map(|message| async move {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    Some(serde_json::from_str(&text).map_err(|error| error.to_string()))
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    Some(Err("project owner event connection closed".into()))
                }
                Err(error) => Some(Err(error.to_string())),
                _ => None,
            }
        }))
    }
    pub async fn command(&self, command: OwnerCommand) -> OwnerReply {
        let response = self
            .http
            .post(format!("{}/private/project/command", self.url))
            .bearer_auth(&self.token)
            .timeout(std::time::Duration::from_secs(120))
            .json(&OwnerRequest {
                project_key: self.project_key.clone(),
                command,
            })
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let reply: serde_json::Value = response.json().await.map_err(|error| error.to_string())?;
        if status.is_success() {
            Ok(reply)
        } else {
            Err(reply["error"]
                .as_str()
                .unwrap_or("project command refused")
                .into())
        }
    }
}

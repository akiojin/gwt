//! Settings / update / autostart / Board auth / server-URL handlers split
//! out of `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - Release Notes + update lifecycle replies
//!   ([`AppRuntime::release_notes_events`],
//!   [`AppRuntime::apply_pending_update_events`] and the
//!   start/cancel/later/restart handlers, with the Phase 14/19 error
//!   builders `update_apply_error_message` / `update_apply_error_failed`)
//! - System settings + autostart toggles
//!   ([`AppRuntime::system_settings_get_events`] /
//!   [`AppRuntime::system_settings_update_events`] /
//!   [`AppRuntime::autostart_status_events`] /
//!   [`AppRuntime::autostart_update_events`])
//! - Remote Board provider auth + config
//!   ([`AppRuntime::board_auth_status_events`], provider config / OAuth port
//!   updates, sign-in / sign-out)
//! - OS-opener gates ([`validate_server_url`], [`validate_update_log_path`],
//!   [`os_url_open_command`], `open_url_with_os_default`,
//!   `open_path_with_os_default`) used by
//!   [`AppRuntime::open_server_url_events`] /
//!   [`AppRuntime::open_update_log_events`]
//! - UI trace persistence ([`AppRuntime::save_ui_trace_events`]) and the
//!   custom-agent backend connection probe
//!   ([`AppRuntime::spawn_backend_connection_probe`])

use std::path::{Path, PathBuf};

use super::{
    save_ui_trace_to_log_dir, AppRuntime, BackendEvent, ClientId, OutboundEvent, UiTracePayload,
    UserEvent,
};

fn restart_impact_lease_ids(
    slots: &[gwt_core::index_coordinator::HeavySlotStatus],
    belongs_to_app: impl Fn(u32) -> bool,
) -> Vec<String> {
    slots
        .iter()
        .map(|slot| &slot.status)
        .filter(|status| {
            status.held
                && !status.expired
                && status
                    .owner
                    .as_ref()
                    .is_some_and(|owner| belongs_to_app(owner.pid))
        })
        .filter_map(|status| status.lease_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn autostart_status_event_from_result(
    result: Result<
        gwt::cli::tray::autostart::AutostartStatus,
        gwt::cli::tray::autostart::AutostartError,
    >,
) -> BackendEvent {
    match result {
        Ok(status) => BackendEvent::AutostartStatus {
            enabled: status.enabled,
            mechanism: format!("{:?}", status.mechanism),
            install_path: status
                .install_path
                .map(|path| path.to_string_lossy().into_owned()),
        },
        Err(error) => BackendEvent::AutostartError {
            message: error.to_string(),
        },
    }
}

/// `true` when `git status --porcelain` reports any entry. Failures are
/// treated as "not dirty" since the backend can fall through to the regular
/// validator pass.
/// Build a Phase 14 message-only [`BackendEvent::UpdateApplyError`].
/// New callers should prefer [`update_apply_error_failed`] which also fills
/// the structured Phase 19 fields.
fn update_apply_error_message(message: &str) -> BackendEvent {
    BackendEvent::UpdateApplyError {
        message: Some(message.to_string()),
        stage: None,
        reason: None,
        log_path: None,
    }
}

/// SPEC-2041 Phase 19 (FR-063): structured update failure event with stage
/// and reason. The legacy `message` field is populated with `reason` for
/// frontends that still read it.
fn update_apply_error_failed(stage: &str, reason: &str) -> BackendEvent {
    BackendEvent::UpdateApplyError {
        message: Some(reason.to_string()),
        stage: Some(stage.to_string()),
        reason: Some(reason.to_string()),
        log_path: None,
    }
}

/// SPEC-2041 Phase 19 (FR-065, CodeRabbit review on PR #2630): pure
/// validator for renderer-supplied update log paths. Returns the canonical
/// path when (1) the input is non-empty and contains no URL scheme,
/// (2) it canonicalizes successfully, (3) it is a file, and (4) it
/// resides within the canonicalized `logs_root`. Returns `None` otherwise so
/// callers can silently drop the request.
pub(super) fn validate_update_log_path(raw: &str, logs_root: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains("://") {
        return None;
    }
    let canonical_root = std::fs::canonicalize(logs_root).ok()?;
    let candidate = std::fs::canonicalize(trimmed).ok()?;
    if !candidate.starts_with(&canonical_root) || !candidate.is_file() {
        return None;
    }
    Some(candidate)
}

/// SPEC-2785 FR-E: exact same-origin match between the embedded server's
/// bound URL and a frontend-supplied URL. Used as the pre-spawn gate by
/// [`AppRuntime::open_server_url_events`] so a renderer compromise cannot
/// smuggle an arbitrary URL into the OS opener.
///
/// Comparison is byte-exact. Trailing-slash and case differences are NOT
/// normalized — the frontend derives its URL from `window.location.origin`
/// so the two strings are always produced by the same source and any drift
/// is a bug worth surfacing rather than papering over.
pub(super) fn validate_server_url(allowed: Option<&str>, requested: &str) -> bool {
    matches!(allowed, Some(value) if value == requested)
}

/// SPEC-2785 FR-C / FR-E: launch the platform default browser for a URL
/// argument (analogous to [`open_path_with_os_default`] but reserved for URLs
/// that have already cleared [`validate_server_url`]). The opener receives
/// the URL via argv directly, never through a shell, so URL contents cannot
/// trigger shell metacharacter expansion.
/// Build the `(program, args)` that opens `url` in the OS default browser.
///
/// Windows deliberately uses `rundll32 url.dll,FileProtocolHandler <url>`
/// instead of `cmd /C start "" <url>`. `cmd.exe` re-parses its command line with
/// shell rules, so a URL's `&` (the query-string separator) is treated as a
/// command separator: the browser receives only the text up to the first `&`
/// and every later parameter is dropped. For OAuth authorize URLs that silently
/// strips `redirect_uri`, `scope`, and `state`, producing Slack's
/// "redirect_uri did not match" / "No scopes requested" errors. `rundll32`
/// (like `open` / `xdg-open`) receives the URL as a single CreateProcess
/// argument and hands the full string to the default protocol handler, so `&`
/// and `%` survive verbatim.
pub(super) fn os_url_open_command(url: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        ("open", vec![url.to_string()])
    } else if cfg!(target_os = "windows") {
        (
            "rundll32.exe",
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
        )
    } else {
        ("xdg-open", vec![url.to_string()])
    }
}

fn open_url_with_os_default(url: &str) -> Result<(), std::io::Error> {
    let (program, args) = os_url_open_command(url);
    let child = gwt_core::process::hidden_command(program)
        .args(&args)
        .spawn()?;
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// SPEC-2041 Phase 19 (FR-065): launch the platform default opener
/// (`open` on macOS, `xdg-open` on Linux, `explorer` on Windows). Errors are
/// silently dropped so the modal does not surface noise; the path is logged
/// at the trace level.
fn open_path_with_os_default(path: &str) -> Result<(), std::io::Error> {
    // Reap the spawned opener on a detached thread so repeated invocations
    // do not accumulate zombie processes on Unix. `std::process::Child` has
    // no Drop-time wait, so without this the PID stays in the process table
    // until parent exit (CodeRabbit review on PR #2630).
    let child = if cfg!(target_os = "macos") {
        let mut cmd = gwt_core::process::hidden_command("open");
        cmd.arg(path);
        cmd.spawn()?
    } else if cfg!(target_os = "windows") {
        let mut cmd = gwt_core::process::hidden_command("cmd");
        cmd.args(["/C", "start", "", path]);
        cmd.spawn()?
    } else {
        let mut cmd = gwt_core::process::hidden_command("xdg-open");
        cmd.arg(path);
        cmd.spawn()?
    };
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

fn release_notes_version_present(
    entries: &[gwt_core::release_notes::ReleaseEntry],
    version: &str,
) -> bool {
    let version = gwt_core::release_notes::normalize_version(version);
    entries
        .iter()
        .any(|entry| gwt_core::release_notes::normalize_version(&entry.version) == version)
}

fn release_notes_payload_event(
    id: String,
    entries: Vec<gwt_core::release_notes::ReleaseEntry>,
    focus_version: Option<String>,
) -> BackendEvent {
    BackendEvent::ReleaseNotesPayload {
        id,
        entries,
        focus_version,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn release_notes_error_event(id: String, message: impl Into<String>) -> BackendEvent {
    BackendEvent::ReleaseNotesError {
        id,
        message: message.into(),
    }
}

fn merge_remote_release_entry(
    mut entry: gwt_core::release_notes::ReleaseEntry,
    bundled: Vec<gwt_core::release_notes::ReleaseEntry>,
) -> Vec<gwt_core::release_notes::ReleaseEntry> {
    let version = gwt_core::release_notes::normalize_version(&entry.version);
    entry.version = version.clone();
    let mut entries = Vec::with_capacity(bundled.len() + 1);
    entries.push(entry);
    entries.extend(bundled.into_iter().filter(|bundled_entry| {
        gwt_core::release_notes::normalize_version(&bundled_entry.version) != version
    }));
    entries
}

impl AppRuntime {
    /// SPEC #2780: serve the bundled `CHANGELOG.md` to the Release Notes
    /// window. The parse runs once per process (cached) so this handler is
    /// effectively a copy from a static slice.
    ///
    /// SPEC #2780 v2 Amendment (FR-013): `current_version` is included so the
    /// frontend can label the Update / Downgrade / Current action button.
    pub(super) fn release_notes_events(
        &self,
        client_id: ClientId,
        id: String,
        focus_version: Option<String>,
    ) -> Vec<OutboundEvent> {
        let entries = gwt_core::release_notes::bundled_releases().to_vec();
        let needs_remote_focus = focus_version
            .as_deref()
            .is_some_and(|version| !release_notes_version_present(&entries, version));

        if needs_remote_focus {
            let proxy = self.proxy.clone();
            let client_id_owned = client_id.clone();
            let id_owned = id.clone();
            let focus_version_owned = focus_version.clone();
            self.blocking_tasks.spawn(move || {
                let manager = gwt_core::update::UpdateManager::new();
                let event = match focus_version_owned.as_deref() {
                    Some(version) => match manager.fetch_release_notes_entry(version) {
                        Ok(Some(remote_entry)) => release_notes_payload_event(
                            id_owned,
                            merge_remote_release_entry(remote_entry, entries),
                            focus_version_owned,
                        ),
                        Ok(None) => release_notes_error_event(
                            id_owned,
                            format!("Release notes for v{} could not be loaded.", version),
                        ),
                        Err(message) => release_notes_error_event(id_owned, message),
                    },
                    None => {
                        release_notes_error_event(id_owned, "Release notes could not be loaded.")
                    }
                };
                proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                    client_id_owned,
                    event,
                )]));
            });
            return vec![];
        }

        let event = if entries.is_empty() {
            release_notes_error_event(id, "Release notes could not be loaded.")
        } else {
            release_notes_payload_event(id, entries, focus_version)
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// SPEC #2780 v2 Amendment (FR-014): user clicked Update / Downgrade on
    /// a specific release in the Release Notes window. Resolves the platform
    /// asset for the requested tag on a worker thread (network), then routes
    /// through the existing `ApplyUpdateStart` pipeline so the standard
    /// update modal renders downloading → ready → restart.
    ///
    /// The shared event-loop admission publishes the accepted state before
    /// downloading, so Later / Restart now use the selected release. Manual
    /// selection never enters the automatic discovery path.
    pub(super) fn apply_update_to_version_events(
        &self,
        client_id: &str,
        version: String,
    ) -> Vec<OutboundEvent> {
        let proxy = self.proxy.clone();
        let client_id_owned = client_id.to_string();
        self.blocking_tasks.spawn(move || {
            let manager = gwt_core::update::UpdateManager::new();
            let current_exe = std::env::current_exe().ok();
            match manager.resolve_state_for_version(&version, current_exe.as_deref()) {
                Ok(state) => {
                    proxy.send(UserEvent::ApplyUpdateStart {
                        state,
                        client_id: client_id_owned,
                    });
                }
                Err(message) => {
                    proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                        client_id_owned,
                        update_apply_error_failed("Resolve release", &message),
                    )]));
                }
            }
        });
        vec![]
    }

    pub(super) fn save_ui_trace_events(
        &self,
        context: Option<&super::ProjectContext>,
        client_id: ClientId,
        trace: UiTracePayload,
    ) -> Vec<OutboundEvent> {
        let log_dir = context
            .and_then(|context| self.project_log_scope_for_tab(&context.tab_id))
            .map(|scope| scope.log_dir())
            .unwrap_or(&self.log_dir);
        let event = match save_ui_trace_to_log_dir(log_dir, trace) {
            Ok(result) => BackendEvent::UiTraceSaved {
                path: result.path.display().to_string(),
                entries: result.entries,
            },
            Err(message) => BackendEvent::UiTraceError { message },
        };
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// SPEC-2963: reply with remote Board provider sign-in state plus the
    /// editable (non-secret) provider configuration for the settings UI.
    pub(super) fn board_auth_status_events(
        &self,
        client_id: ClientId,
        message: Option<String>,
    ) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::board_auth_status_event(message),
        )]
    }

    /// SPEC-2963: persist remote Board provider configuration captured in the
    /// settings UI, then reply with the refreshed auth/config view. Non-secret
    /// fields go to `config.toml`; the client secret goes to the secure store.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn board_provider_config_update_events(
        &self,
        client_id: ClientId,
        provider: &str,
        provider_client_id: Option<String>,
        default_channel: Option<String>,
        tenant_id: Option<String>,
        client_secret: Option<String>,
    ) -> Vec<OutboundEvent> {
        let Some(path) = gwt_config::Settings::global_config_path() else {
            return self.board_auth_status_events(
                client_id,
                Some("unable to resolve home directory (`~/.gwt/config.toml`)".to_string()),
            );
        };
        let message = match gwt::system_settings::write_board_provider_config(
            &path,
            provider,
            provider_client_id,
            default_channel,
            tenant_id,
            client_secret,
        ) {
            Ok(_) => Some(format!("Saved {provider} configuration.")),
            Err(error) => Some(format!("Failed to save configuration: {error}")),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963 FR-030: reply with a repo's per-project Board config (the raw
    /// `board.toml` override) plus its resolved effective routing, for the
    /// Settings Board tab's per-project section.
    pub(super) fn project_board_config_events(
        &self,
        client_id: ClientId,
        project_root: String,
    ) -> Vec<OutboundEvent> {
        let event = self.project_board_config_event(&project_root, None);
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// SPEC-2963 FR-025/FR-030: write per-project Board config to the repo's
    /// `.gwt/work/board.toml`, then reply with the refreshed config + routing.
    /// `Some("")` / `inherit` clears a field (inherit global), `Some(value)`
    /// sets it, `None` leaves it unchanged.
    pub(super) fn project_board_config_update_events(
        &self,
        client_id: ClientId,
        project_root: String,
        provider: Option<String>,
        channel: Option<String>,
        tenant: Option<String>,
    ) -> Vec<OutboundEvent> {
        let path = std::path::Path::new(&project_root);
        let provider_arg = provider.map(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "local" => Some(gwt_config::BoardProviderKind::Local),
            "slack" => Some(gwt_config::BoardProviderKind::Slack),
            "teams" => Some(gwt_config::BoardProviderKind::Teams),
            // "" / "inherit" / unknown → clear the override (inherit global).
            _ => None,
        });
        let message = match gwt::board_provider::update_project_board_config(
            path,
            provider_arg,
            channel.map(Some),
            tenant.map(Some),
        ) {
            Ok(_) => Some("Saved project Board configuration.".to_string()),
            Err(error) => Some(format!(
                "Failed to save project Board configuration: {error}"
            )),
        };
        let event = self.project_board_config_event(&project_root, message);
        vec![OutboundEvent::reply(client_id, event)]
    }

    /// Build a [`BackendEvent::ProjectBoardConfig`] from a repo's `board.toml`
    /// override and its resolved routing.
    fn project_board_config_event(
        &self,
        project_root: &str,
        message: Option<String>,
    ) -> BackendEvent {
        let path = std::path::Path::new(project_root);
        let work_dir = gwt_core::paths::gwt_repo_local_work_dir(path);
        let project = gwt_config::ProjectBoardConfig::load_from_work_dir(&work_dir);
        let routing = gwt::board_provider::routing_for(path);
        BackendEvent::ProjectBoardConfig {
            project_root: project_root.to_string(),
            provider: project.provider.map(|kind| kind.as_str().to_string()),
            channel: project.channel,
            tenant: project.tenant,
            resolved_provider: routing.provider,
            resolved_source: routing.provider_source.to_string(),
            resolved_channel: routing.channel,
            signed_in: routing.signed_in,
            message,
        }
    }

    /// SPEC-2963 FR-005: persist the fixed OAuth callback port, then reply with
    /// the refreshed auth/config view. The new port binds on the next launch.
    pub(super) fn board_oauth_port_update_events(
        &self,
        client_id: ClientId,
        port: u16,
    ) -> Vec<OutboundEvent> {
        let Some(path) = gwt_config::Settings::global_config_path() else {
            return self.board_auth_status_events(
                client_id,
                Some("unable to resolve home directory (`~/.gwt/config.toml`)".to_string()),
            );
        };
        let message = match gwt::system_settings::write_oauth_redirect_port(&path, port) {
            Ok(saved) => Some(format!(
                "Saved OAuth callback port {saved}. Restart gwt and register \
                 http://127.0.0.1:{saved}/oauth/callback in the provider app."
            )),
            Err(error) => Some(format!("Failed to save OAuth port: {error}")),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963: begin OAuth sign-in for a remote Board provider by opening the
    /// browser to the authorize URL (redirect back to the embedded server).
    pub(super) fn board_provider_sign_in_events(
        &self,
        client_id: ClientId,
        provider: &str,
    ) -> Vec<OutboundEvent> {
        let kind = match provider.trim().to_ascii_lowercase().as_str() {
            "slack" => gwt_config::BoardProviderKind::Slack,
            "teams" => gwt_config::BoardProviderKind::Teams,
            other => {
                return self.board_auth_status_events(
                    client_id,
                    Some(format!("Unknown provider '{other}'")),
                );
            }
        };
        // The OAuth redirect uses a fixed loopback callback port (from
        // settings.board.oauth_redirect_port), not the embedded server's
        // ephemeral URL, so sign-in works regardless of how the GUI server
        // bound. The dedicated callback listener is started at server boot.
        let settings = gwt_config::Settings::load().unwrap_or_default();
        let message = match gwt::board_remote::signin::begin_signin(kind, &settings) {
            Ok(authorize_url) => match open_url_with_os_default(&authorize_url) {
                Ok(()) => Some(format!(
                    "Opened the browser to sign in to {provider}. Complete it, then Refresh."
                )),
                Err(error) => Some(format!("Failed to open browser: {error}")),
            },
            Err(reason) => Some(reason),
        };
        self.board_auth_status_events(client_id, message)
    }

    /// SPEC-2963: clear stored credentials for a remote Board provider.
    pub(super) fn board_provider_sign_out_events(
        &self,
        client_id: ClientId,
        provider: &str,
    ) -> Vec<OutboundEvent> {
        let key = match provider.trim().to_ascii_lowercase().as_str() {
            "slack" => "slack",
            "teams" => "teams",
            other => {
                return self.board_auth_status_events(
                    client_id,
                    Some(format!("Unknown provider '{other}'")),
                );
            }
        };
        let message = match gwt::board_remote::signin::sign_out(key) {
            Ok(()) => Some(format!("Signed out of {provider}.")),
            Err(error) => Some(format!("Failed to sign out: {error}")),
        };
        self.board_auth_status_events(client_id, message)
    }

    pub(super) fn system_settings_get_events(&self, client_id: ClientId) -> Vec<OutboundEvent> {
        if let Err(error) = gwt::update_settings::migrate() {
            tracing::warn!(target: "gwt::update", %error, "automatic update migration failed; returning existing system settings");
        }
        let path = gwt_core::paths::gwt_config_path();
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::get_event(&path),
        )]
    }

    pub(super) fn system_settings_update_events(
        &self,
        client_id: ClientId,
        language: String,
        codex_trust_managed_hooks: Option<bool>,
        board_provider: Option<String>,
        agent_resource: Option<gwt::protocol::AgentResourceSettings>,
        works_preview_lines: Option<u32>,
    ) -> Vec<OutboundEvent> {
        let path = gwt_core::paths::gwt_config_path();
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::update_event(
                &path,
                language,
                codex_trust_managed_hooks,
                board_provider,
                agent_resource,
                works_preview_lines,
            ),
        )]
    }

    pub(super) fn autostart_status_events(&self, client_id: ClientId) -> Vec<OutboundEvent> {
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = autostart_status_event_from_result(
                gwt::cli::tray::autostart::AutostartManager::status(),
            );
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
        Vec::new()
    }

    pub(super) fn autostart_update_events(
        &self,
        client_id: ClientId,
        enabled: bool,
    ) -> Vec<OutboundEvent> {
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let result = if enabled {
                gwt::cli::tray::autostart::AutostartManager::install()
            } else {
                gwt::cli::tray::autostart::AutostartManager::uninstall()
            };
            let event = match result {
                Ok(()) => autostart_status_event_from_result(
                    gwt::cli::tray::autostart::AutostartManager::status(),
                ),
                Err(error) => BackendEvent::AutostartError {
                    message: error.to_string(),
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
        Vec::new()
    }

    pub(super) fn custom_agent_reply_with_cache_refresh(
        &mut self,
        client_id: ClientId,
        event: BackendEvent,
    ) -> Vec<OutboundEvent> {
        if matches!(
            &event,
            BackendEvent::CustomAgentSaved { .. } | BackendEvent::CustomAgentDeleted { .. }
        ) {
            self.launch_wizard_cache.refresh_agent_options();
            let contexts = self
                .project_states
                .values()
                .filter(|state| state.launch_wizard.is_some())
                .map(|state| state.context.clone())
                .collect::<Vec<_>>();
            let mut events = vec![OutboundEvent::reply(client_id, event)];
            for context in contexts {
                self.refresh_open_launch_wizard_from_cache(&context);
                events.push(self.launch_wizard_state_outbound(&context));
            }
            return events;
        }
        vec![OutboundEvent::reply(client_id, event)]
    }

    pub(crate) fn spawn_backend_connection_probe(
        &self,
        client_id: ClientId,
        base_url: String,
        api_key: String,
    ) {
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = gwt::custom_agents_dispatch::test_connection_event(&base_url, &api_key);
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    pub(crate) fn spawn_supported_agent_list(&self, client_id: ClientId) {
        // Only maintenance changes the epoch; concurrent clients each need a reply.
        let generation = self.agent_maintenance.catalog_generation;
        let mut cache = self.launch_wizard_cache.clone();
        let config_path = self.profile_config_path();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = match config_path {
                Ok(path) => {
                    cache.refresh_agent_options_for_profile(&path, &cwd);
                    super::agent_maintenance::supported_agent_catalog(&cache, &path, &cwd)
                }
                Err(message) => BackendEvent::SystemSettingsError { message },
            };
            proxy.send(UserEvent::SupportedAgentCatalogReady {
                client_id,
                generation,
                event,
            });
        });
    }

    pub(super) fn apply_pending_update_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdate {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            Some(gwt_core::update::UpdateState::Available { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message(
                    "No applicable update asset is available for this platform.",
                ),
            )],
            Some(gwt_core::update::UpdateState::UpToDate { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message("No pending update is available."),
            )],
            Some(gwt_core::update::UpdateState::Failed { message, .. }) => {
                vec![OutboundEvent::reply(
                    client_id,
                    update_apply_error_message(&format!("Update check failed: {message}")),
                )]
            }
            None => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_message("No pending update is available."),
            )],
        }
    }

    /// Issue #4801: discovery enters the same staging path as the Update
    /// button when an open project opted into unattended updates.
    pub(crate) fn start_automatic_update_download(&mut self) {
        let Some(gwt_core::update::UpdateState::Available {
            latest,
            asset_url: Some(_),
            ..
        }) = self.pending_update.as_ref()
        else {
            return;
        };
        if gwt_core::update::load_pending_update_manifest()
            .is_some_and(|manifest| manifest.version == *latest)
        {
            gwt_core::update::log_update_event(
                "auto_apply_skipped",
                &[("version", latest), ("reason", "already_staged")],
            );
            return;
        }
        if let Some(reason) = self.automatic_update_skip_reason() {
            gwt_core::update::log_update_event(
                "auto_apply_skipped",
                &[("version", latest), ("reason", reason)],
            );
        } else {
            self.apply_update_start_events(super::UPDATE_AUTO_APPLY_CLIENT_ID);
        }
    }

    pub(crate) fn automatic_updates_enabled(&self) -> bool {
        self.automatic_update_skip_reason().is_none()
    }

    fn automatic_update_skip_reason(&self) -> Option<&'static str> {
        let enabled = match gwt::update_settings::migrate() {
            Ok(enabled) => enabled,
            Err(error) => {
                tracing::warn!(target: "gwt::update", %error, "automatic update migration failed; treating as OFF");
                return Some("settings_unavailable");
            }
        };
        if !enabled {
            return Some("automatic_updates_disabled");
        }
        let mut unreadable = false;
        let autonomous = self.project_contexts().iter().any(|context| {
            match gwt::load_issue_monitor_prefs(&gwt::issue_monitor_prefs_path_for_repo_path(
                &context.project_root,
            )) {
                Ok(prefs) => prefs.autonomous_mode,
                Err(_) => {
                    unreadable = true;
                    false
                }
            }
        });
        if autonomous {
            None
        } else if unreadable {
            Some("project_settings_unavailable")
        } else {
            Some("no_autonomous_project")
        }
    }

    pub(super) fn update_settings_event(
        &self,
        project_root: Option<&std::path::Path>,
    ) -> BackendEvent {
        let status = match project_root {
            Some(root) => gwt::update_settings::status(root),
            None => {
                gwt::update_settings::configured().map(|configured| gwt::update_settings::Status {
                    auto_apply_updates: configured,
                    auto_apply_updates_effective: configured.unwrap_or(false),
                    update_drain: None,
                })
            }
        };
        match status {
            Ok(mut status) => {
                if let Some(drain) = status.update_drain.as_mut() {
                    if drain.blocking.is_empty() {
                        if let Some(cached) = project_root
                            .and_then(|root| self.project_state_for_root(root))
                            .and_then(|state| {
                                state
                                    .issue_monitor_status_cache
                                    .borrow()
                                    .as_ref()
                                    .and_then(|status| status.update_drain.clone())
                            })
                            .filter(|cached| {
                                cached.version == drain.version && cached.since == drain.since
                            })
                        {
                            drain.blocking = cached.blocking;
                        }
                    }
                    self.enrich_update_drain_blockers(drain);
                }
                BackendEvent::UpdateSettings {
                    project_root: project_root.map(|root| root.display().to_string()),
                    status,
                }
            }
            Err(error) => BackendEvent::SystemSettingsError {
                message: error.to_string(),
            },
        }
    }

    pub(super) fn set_global_auto_apply_updates_events(
        &mut self,
        client_id: &str,
        enabled: bool,
    ) -> Vec<OutboundEvent> {
        if let Err(error) = gwt::update_settings::set(Some(enabled)) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::SystemSettingsError {
                    message: error.to_string(),
                },
            )];
        }
        let mut events = Vec::new();
        for context in self.project_contexts() {
            if !enabled {
                if let Some((prefs, drain)) = self.project_auto_update_drain(&context) {
                    if drain.automatic.unwrap_or(prefs.autonomous_mode) {
                        events.extend(
                            self.release_update_auto_apply_for_project(&context, &drain.version),
                        );
                    }
                }
            }
            if let Ok(prefs) = gwt::load_issue_monitor_prefs(
                &gwt::issue_monitor_prefs_path_for_repo_path(&context.project_root),
            ) {
                let monitor =
                    gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
                events.extend(self.issue_monitor_snapshot_events_without_wake(
                    None,
                    Some(&context.project_root),
                    monitor,
                ));
            }
            events.push(OutboundEvent::project(
                context.project_key,
                self.update_settings_event(Some(&context.project_root)),
            ));
        }
        events.push(OutboundEvent::hub(self.update_settings_event(None)));
        let path = gwt_core::paths::gwt_config_path();
        events.push(OutboundEvent::broadcast(gwt::system_settings::get_event(
            &path,
        )));
        events
    }

    /// Shared staging request for the update CTA and automatic discovery.
    /// The worker emits `UpdateReady` or `UpdateApplyError` without exiting.
    pub(super) fn apply_update_start_events(&mut self, client_id: &str) -> Vec<OutboundEvent> {
        if self.update_download_in_flight.is_some() {
            return Vec::new();
        }
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdateStart {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            Some(gwt_core::update::UpdateState::Available { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed(
                    "Download asset",
                    "No applicable update asset is available for this platform.",
                ),
            )],
            Some(gwt_core::update::UpdateState::UpToDate { .. }) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Download asset", "No pending update is available."),
            )],
            Some(gwt_core::update::UpdateState::Failed { message, .. }) => {
                vec![OutboundEvent::reply(
                    client_id,
                    update_apply_error_failed(
                        "Update check",
                        &format!("Update check failed: {message}"),
                    ),
                )]
            }
            None => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Download asset", "No pending update is available."),
            )],
        }
    }

    /// SPEC-2041 Phase 19 (FR-055): user pressed `Cancel` mid-download.
    /// `prepare_update` runs synchronously on a worker thread, so a true
    /// mid-download abort is best-effort. We still defensively clear any
    /// `~/.gwt/pending-update/manifest.json` that the worker may have
    /// persisted between the user's click and the modal close — without this
    /// guard, a race would leave the bootstrap path applying an update the
    /// user explicitly cancelled (CodeRabbit P1 review on PR #2630).
    pub(super) fn cancel_update_download_events(&self, _client_id: &str) -> Vec<OutboundEvent> {
        let _ = gwt_core::update::clear_pending_update_manifest();
        vec![]
    }

    /// SPEC-2041 Phase 19 (FR-059..061, FR-064): user pressed `Later`.
    /// Verifies the manifest persisted by `ApplyUpdateStart`'s worker thread
    /// is still on disk via [`crate::update_front_door::commit_update_later_pending`],
    /// then emits [`BackendEvent::UpdateApplyPendingPersisted`] so the CTA
    /// morphs to ready state. If persistence somehow vanished (external
    /// cleanup, disk-full race), surface a structured error instead of
    /// silently lying about pending state.
    pub(super) fn apply_update_later_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        let version = match self.pending_update.as_ref() {
            Some(gwt_core::update::UpdateState::Available { latest, .. }) => latest.clone(),
            _ => return vec![],
        };
        match crate::update_front_door::commit_update_later_pending() {
            Ok(()) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::UpdateApplyPendingPersisted { version },
            )],
            Err(message) => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Persist pending", &message),
            )],
        }
    }

    /// SPEC-2041 Phase 19 (FR-058): user pressed `Restart now`. The event
    /// loop resolves the prepared payload (persisted manifest, or a download
    /// when the user re-clicked the CTA before it persisted) and commits it
    /// through the graceful `ApplyUpdateGraceful` route (Issue #4038), which
    /// quits via `QuitApp` instead of exiting from a worker thread.
    pub(super) fn apply_update_restart_now_events(&self, client_id: &str) -> Vec<OutboundEvent> {
        match self.pending_update.clone() {
            Some(
                state @ gwt_core::update::UpdateState::Available {
                    asset_url: Some(_), ..
                },
            ) => {
                self.proxy.send(UserEvent::ApplyUpdateRestartNow {
                    state,
                    client_id: client_id.to_string(),
                });
                vec![]
            }
            _ => vec![OutboundEvent::reply(
                client_id,
                update_apply_error_failed(
                    "Restart now",
                    "No prepared update available for restart.",
                ),
            )],
        }
    }

    /// Observe on a worker: checking ECRs/leases must not block the GUI loop.
    pub(super) fn get_update_restart_impact_events(
        &self,
        client_id: &str,
        request_id: &str,
    ) -> Vec<OutboundEvent> {
        let (panes, worktrees) = self.capture_update_quiescence_inputs();
        let mut issue_numbers = std::collections::BTreeSet::new();
        let mut unresolved_sessions = Vec::new();
        for pane in panes.iter().filter(|pane| {
            !pane.resident_pm
                && matches!(
                    pane.state,
                    gwt::persistence::WindowState::Running
                        | gwt::persistence::WindowState::Starting
                )
        }) {
            let issue = self
                .active_agent_sessions
                .get(&pane.window_id)
                .and_then(|active| self.launch_wizard_cache.session_by_id(&active.session_id))
                .and_then(|session| session.linked_issue_number)
                .or_else(|| {
                    let address = self.window_lookup.get(&pane.window_id)?;
                    self.tab(&address.tab_id)?
                        .workspace
                        .window(&address.raw_id)?
                        .linked_issue_number
                });
            if issue.is_none() {
                if let Some(active) = self.active_agent_sessions.get(&pane.window_id) {
                    unresolved_sessions.push(
                        self.sessions_dir
                            .join(format!("{}.toml", active.session_id)),
                    );
                }
            }
            issue_numbers.extend(issue);
        }
        let contexts = self.project_contexts();
        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let request_id = request_id.to_string();
        self.blocking_tasks.spawn(move || {
            let result = (|| -> Result<BackendEvent, String> {
                let mut blocking = Vec::new();
                let pool =
                    gwt_core::index_coordinator::IndexCoordinator::open_default_verification()
                        .and_then(|coordinator| coordinator.heavy_pool_status())
                        .map_err(|error| format!("Cannot inspect verification leases: {error}"))?;
                let lease_ids = restart_impact_lease_ids(&pool.slots, |pid| {
                    gwt::process::is_descendant_of(pid, std::process::id())
                });
                for path in unresolved_sessions {
                    if let Ok(session) = gwt_agent::Session::load(&path) {
                        issue_numbers.extend(session.linked_issue_number);
                    }
                }
                // Host activity still matters when no project is open.
                let mut monitors = Vec::new();
                for context in contexts {
                    let prefs = gwt::load_issue_monitor_prefs(
                        &gwt::issue_monitor_prefs_path_for_repo_path(&context.project_root),
                    )
                    .map_err(|error| error.to_string())?;
                    monitors.push(gwt::IssueMonitorState::with_prefs(
                        gwt::IssueMonitorConfig::default(),
                        prefs,
                    ));
                }
                if monitors.is_empty() {
                    monitors.push(gwt::IssueMonitorState::with_prefs(
                        gwt::IssueMonitorConfig::default(),
                        Default::default(),
                    ));
                }
                for monitor in monitors {
                    let mut snapshot = Self::read_update_quiescence_snapshot(
                        panes.clone(),
                        worktrees.clone(),
                        &monitor,
                    );
                    // Preview counts every affected pool slot; automatic drain
                    // observations retain their existing decision contract.
                    snapshot.held_verification_leases = lease_ids.clone();
                    issue_numbers.extend(snapshot.pending_acquire_claims.iter().copied());
                    for worktree in &snapshot.active_executions {
                        if let Ok(Some(record)) =
                            gwt::cli::execution_state::load(Path::new(worktree))
                        {
                            issue_numbers.insert(record.owner_number);
                        }
                    }
                    for blocker in gwt::update_drain::update_quiescence(&snapshot)
                        .err()
                        .unwrap_or_default()
                    {
                        if !blocking.contains(&blocker) {
                            blocking.push(blocker);
                        }
                    }
                }
                Ok(BackendEvent::UpdateRestartImpact {
                    request_id: request_id.clone(),
                    blocking,
                    issue_numbers: issue_numbers.into_iter().collect(),
                    error: None,
                })
            })();
            let event = result.unwrap_or_else(|reason| BackendEvent::UpdateRestartImpact {
                request_id,
                blocking: Vec::new(),
                issue_numbers: Vec::new(),
                error: Some(reason),
            });
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
        Vec::new()
    }

    pub(super) fn apply_update_when_idle_events(&mut self, client_id: &str) -> Vec<OutboundEvent> {
        let mut events = Vec::new();
        let result = (|| -> Result<(), String> {
            let manifest = gwt_core::update::load_pending_update_manifest()
                .ok_or_else(|| "No prepared update is available to schedule.".to_string())?;
            // A failed wait request must also keep startup apply deferred.
            gwt_core::update::defer_pending_update(&manifest.version)?;
            let contexts = self.project_contexts();
            if contexts.is_empty() {
                return Err("Open a project before scheduling the update.".into());
            }
            // Validate every project before starting any drain.
            let projects = contexts
                .into_iter()
                .map(|context| {
                    let prefs = gwt::load_issue_monitor_prefs(
                        &gwt::issue_monitor_prefs_path_for_repo_path(&context.project_root),
                    )
                    .map_err(|error| error.to_string())?;
                    Ok((context, prefs))
                })
                .collect::<Result<Vec<_>, String>>()?;
            for (context, prefs) in projects {
                let (project_events, saved) =
                    self.begin_update_drain_for_project(&context, &manifest.version, prefs, false);
                events.extend(project_events);
                if !saved {
                    return Err(format!(
                        "Could not start the update drain for {}; automatic apply remains deferred.",
                        context.project_root.display()
                    ));
                }
            }
            gwt_core::update::resume_pending_update(&manifest.version)?;
            Ok(())
        })();
        if let Err(reason) = result {
            events.push(OutboundEvent::reply(
                client_id,
                update_apply_error_failed("Apply after agents finish", &reason),
            ));
        }
        Self::deduplicate_update_host_events(events)
    }

    /// SPEC-2785 US-1 / FR-C / FR-E: user clicked the server URL cell in the
    /// status strip. The renderer-supplied `url` is treated as untrusted and
    /// is only forwarded to [`open_url_with_os_default`] when it matches the
    /// embedded server's bound URL captured by [`Self::set_server_url`].
    /// Mismatched origins (or an unset server URL) are dropped with a trace
    /// log so a compromised renderer cannot redirect the OS opener to an
    /// arbitrary URL. The handler returns no outbound events; the click is a
    /// side-effect only.
    pub(super) fn open_server_url_events(
        &self,
        _client_id: &str,
        url: String,
    ) -> Vec<OutboundEvent> {
        if validate_server_url(self.server_url.as_deref(), &url) {
            if let Err(error) = open_url_with_os_default(&url) {
                tracing::trace!(
                    target: "gwt::open_server_url",
                    %error,
                    "failed to spawn OS browser opener"
                );
            }
        } else {
            tracing::trace!(
                target: "gwt::open_server_url",
                requested = %url,
                allowed = ?self.server_url,
                "rejected open_server_url request: origin mismatch"
            );
        }
        Vec::new()
    }

    /// SPEC-2041 Phase 19 (FR-065): user pressed `Open log` on the failed
    /// modal. Backend opens the log file with the OS default application.
    /// The renderer-supplied `log_path` is treated as untrusted: the path
    /// must canonicalize to a child of the gwt logs directory, must exist as
    /// a file, and must not contain a URL scheme (CodeRabbit review on PR
    /// #2630). Validation failures are silently dropped — the modal already
    /// surfaces the in-memory `Reason` so a missing log file is not blocking.
    pub(super) fn open_update_log_events(
        &self,
        _client_id: &str,
        log_path: Option<String>,
    ) -> Vec<OutboundEvent> {
        if let Some(raw) = log_path {
            // Derive the allowed logs root from the canonical update log
            // resolver itself. AppRuntime is not allowed to call the legacy
            // `gwt_logs_dir()` directly (project-scoped resolver test in
            // main.rs), so we ride on `update_log_path()`'s parent.
            if let Some(logs_root) = gwt_core::update::update_log_path()
                .parent()
                .map(|p| p.to_path_buf())
            {
                if let Some(safe) = validate_update_log_path(&raw, &logs_root) {
                    let _ = open_path_with_os_default(&safe.to_string_lossy());
                }
            }
        }
        vec![]
    }
}

#[cfg(test)]
mod restart_impact_tests {
    use super::*;

    #[test]
    fn issue_5646_restart_preview_collects_all_local_verification_slots() {
        use gwt_core::index_coordinator::{HeavyLeaseStatus, HeavySlotStatus, OwnerIdentity};
        let slot = |pid, lease_id: &str| HeavySlotStatus {
            slot: None,
            status: HeavyLeaseStatus {
                held: true,
                lease_id: Some(lease_id.into()),
                owner: Some(OwnerIdentity {
                    pid,
                    start_id: "test".into(),
                }),
                ..Default::default()
            },
        };
        let mut slots = vec![
            slot(99, "other-app"),
            slot(1, "local-one"),
            slot(2, "local-two"),
            slot(2, "local-two"),
        ];
        let mut expired = slot(1, "expired");
        expired.status.expired = true;
        slots.push(expired);
        let mut released = slot(1, "released");
        released.status.held = false;
        slots.push(released);
        assert_eq!(
            restart_impact_lease_ids(&slots, |pid| pid <= 2),
            ["local-one", "local-two"]
        );
    }
}

#[cfg(test)]
mod release_notes_tests {
    use super::*;
    use gwt_core::release_notes::{ReleaseEntry, Section};

    fn release_entry(version: &str, item: &str) -> ReleaseEntry {
        ReleaseEntry {
            version: version.to_string(),
            date: "2026-06-20".to_string(),
            sections: vec![Section {
                heading: "Notes".to_string(),
                items: vec![item.to_string()],
            }],
        }
    }

    #[test]
    fn remote_release_entry_precedes_and_replaces_bundled_version() {
        let entries = merge_remote_release_entry(
            release_entry("v9.62.0", "Remote notes"),
            vec![
                release_entry("9.61.0", "Bundled current notes"),
                release_entry("9.60.0", "Bundled older notes"),
            ],
        );

        let versions: Vec<&str> = entries.iter().map(|entry| entry.version.as_str()).collect();
        assert_eq!(versions, vec!["9.62.0", "9.61.0", "9.60.0"]);
        assert_eq!(entries[0].sections[0].items, vec!["Remote notes"]);
    }

    #[test]
    fn remote_release_entry_deduplicates_same_bundled_version() {
        let entries = merge_remote_release_entry(
            release_entry("v9.62.0", "Remote notes"),
            vec![
                release_entry("9.62.0", "Stale bundled notes"),
                release_entry("9.61.0", "Bundled current notes"),
            ],
        );

        let versions: Vec<&str> = entries.iter().map(|entry| entry.version.as_str()).collect();
        assert_eq!(versions, vec!["9.62.0", "9.61.0"]);
        assert_eq!(entries[0].sections[0].items, vec!["Remote notes"]);
    }
}

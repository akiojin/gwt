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
    use std::process::Command;
    let (program, args) = os_url_open_command(url);
    let child = Command::new(program).args(&args).spawn()?;
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
    use std::process::Command;
    // Reap the spawned opener on a detached thread so repeated invocations
    // do not accumulate zombie processes on Unix. `std::process::Child` has
    // no Drop-time wait, so without this the PID stays in the process table
    // until parent exit (CodeRabbit review on PR #2630).
    let child = if cfg!(target_os = "macos") {
        let mut cmd = Command::new("open");
        cmd.arg(path);
        cmd.spawn()?
    } else if cfg!(target_os = "windows") {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", path]);
        cmd.spawn()?
    } else {
        let mut cmd = Command::new("xdg-open");
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
    /// Codex review on PR #2917: the resolved state is also published as
    /// `UserEvent::UpdateAvailable` so `AppRuntime.pending_update` reflects
    /// the chosen release. Without this step, `ApplyUpdateLater` /
    /// `ApplyUpdateRestartNow` (which both gate on `self.pending_update`)
    /// would either no-op or fire against an unrelated latest-update state
    /// when the user selected a downgrade while `pending_update` was
    /// `UpToDate`.
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
                    // Update `pending_update` first so Later / Restart now
                    // read the selected release. The frontend update-cta
                    // ignores the broadcast `UpdateState` here because its
                    // local status is already `applying` (the modal was
                    // opened by `beginUpdateDownloading` on click).
                    proxy.send(UserEvent::UpdateAvailable(state.clone()));
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
        client_id: ClientId,
        trace: UiTracePayload,
    ) -> Vec<OutboundEvent> {
        let event = match save_ui_trace_to_log_dir(&self.log_dir, trace) {
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
        let path = match gwt_config::Settings::global_config_path() {
            Some(p) => p,
            None => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::SystemSettingsError {
                        message: "unable to resolve home directory (`~/.gwt/config.toml`)"
                            .to_string(),
                    },
                )];
            }
        };
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
    ) -> Vec<OutboundEvent> {
        let path = match gwt_config::Settings::global_config_path() {
            Some(p) => p,
            None => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::SystemSettingsError {
                        message: "unable to resolve home directory (`~/.gwt/config.toml`)"
                            .to_string(),
                    },
                )];
            }
        };
        vec![OutboundEvent::reply(
            client_id,
            gwt::system_settings::update_event(
                &path,
                language,
                codex_trust_managed_hooks,
                board_provider,
            ),
        )]
    }

    pub(super) fn autostart_status_events(&self, client_id: ClientId) -> Vec<OutboundEvent> {
        vec![OutboundEvent::reply(
            client_id,
            autostart_status_event_from_result(
                gwt::cli::tray::autostart::AutostartManager::status(),
            ),
        )]
    }

    pub(super) fn autostart_update_events(
        &self,
        client_id: ClientId,
        enabled: bool,
    ) -> Vec<OutboundEvent> {
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
        vec![OutboundEvent::reply(client_id, event)]
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
            let had_open_wizard = self.launch_wizard.is_some();
            self.refresh_open_launch_wizard_from_cache();
            let mut events = vec![OutboundEvent::reply(client_id, event)];
            if had_open_wizard {
                events.push(self.launch_wizard_state_outbound());
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

    /// SPEC-2041 Phase 19 (FR-052): user clicked the update CTA and the modal
    /// is opening in the `downloading` state. Backend kicks off
    /// `prepare_update` on a worker thread and emits
    /// [`BackendEvent::UpdateReady`] (or [`BackendEvent::UpdateApplyError`])
    /// without exiting the parent process.
    pub(super) fn apply_update_start_events(&self, client_id: &str) -> Vec<OutboundEvent> {
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

    /// SPEC-2041 Phase 19 (FR-058): user pressed `Restart now`. Backend
    /// commits the prepared payload via the helper subprocess and exits the
    /// parent. Falls back to the legacy `apply_update_state_and_exit` path
    /// when no prepared payload exists yet (e.g. user manually re-clicked CTA
    /// before download completed).
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

//! Profile handler split out of `app_runtime/mod.rs` for SPEC-2077 Phase E
//! (arch-review handoff, 2026-05-01). Originally planned as the "Agent
//! handler" extraction, but since `mod.rs` does not host dedicated
//! `agent_*_events` functions (agent launch flows through the wizard /
//! runtime spawn paths owned by other phases), Phase E is redirected to
//! the Profile handler family which fits the same single-responsibility
//! template.
//!
//! Owns:
//! - [`AppRuntime::load_profile_events`] / [`AppRuntime::select_profile_events`]
//!   / [`AppRuntime::create_profile_events`] /
//!   [`AppRuntime::set_active_profile_events`] /
//!   [`AppRuntime::delete_profile_events`] — public profile handlers
//! - [`AppRuntime::save_profile_events`] — private save handler
//!   (consumed by the SaveProfile dispatch arm)
//! - profile-window helpers (`resolve_profile_window_context`,
//!   `profile_window_ids_for_tab`, `profile_config_path`,
//!   `active_profile_spawn_env`, `profile_snapshot_events`)
//! - [`ProfileSaveRequest`] payload re-exported via `mod.rs` for the
//!   dispatch arm
//!
//! Persistence flows through `gwt::profile_dispatch::*`, agent launch
//! integration uses `gwt_agent::LaunchEnvironment::from_active_profile`,
//! and broadcasts use `BackendEvent::ProfileSnapshot` /
//! `BackendEvent::ProfileError`.

use std::path::PathBuf;

use gwt::{profile_dispatch, ProfileEnvEntryView};

use super::{combined_window_id, AppRuntime, BackendEvent, OutboundEvent, WindowPreset};

pub(super) struct ProfileSaveRequest {
    pub(super) current_name: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) env_vars: Vec<ProfileEnvEntryView>,
    pub(super) disabled_env: Vec<String>,
}

impl AppRuntime {
    pub(crate) fn load_profile_events(&mut self, client_id: &str, id: &str) -> Vec<OutboundEvent> {
        if let Err(message) = self.resolve_profile_window_context(id) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message,
                },
            )];
        }

        let selected_profile = self.profile_selections.get(id).cloned();
        let config_path = match self.profile_config_path() {
            Ok(path) => path,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };
        match profile_dispatch::load_snapshot_at(&config_path, selected_profile.as_deref()) {
            Ok(snapshot) => {
                self.profile_selections
                    .insert(id.to_string(), snapshot.selected_profile.clone());
                vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileSnapshot {
                        id: id.to_string(),
                        snapshot,
                    },
                )]
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )],
        }
    }

    pub(crate) fn select_profile_events(
        &mut self,
        client_id: &str,
        id: &str,
        profile_name: &str,
    ) -> Vec<OutboundEvent> {
        if let Err(message) = self.resolve_profile_window_context(id) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message,
                },
            )];
        }

        self.profile_selections
            .insert(id.to_string(), profile_name.to_string());
        self.load_profile_events(client_id, id)
    }

    pub(crate) fn create_profile_events(
        &mut self,
        client_id: &str,
        id: &str,
        name: &str,
    ) -> Vec<OutboundEvent> {
        let tab_id = match self.resolve_profile_window_context(id) {
            Ok(tab_id) => tab_id,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let selected_profile = name.trim().to_string();
        let config_path = match self.profile_config_path() {
            Ok(path) => path,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };
        if let Err(error) = profile_dispatch::create_profile_at(&config_path, &selected_profile) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )];
        }

        self.profile_selections
            .insert(id.to_string(), selected_profile);
        self.profile_snapshot_events(&tab_id, id, client_id)
    }

    pub(crate) fn set_active_profile_events(
        &mut self,
        client_id: &str,
        id: &str,
        profile_name: &str,
    ) -> Vec<OutboundEvent> {
        let tab_id = match self.resolve_profile_window_context(id) {
            Ok(tab_id) => tab_id,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let config_path = match self.profile_config_path() {
            Ok(path) => path,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };
        if let Err(error) = profile_dispatch::switch_active_profile_at(&config_path, profile_name) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )];
        }

        self.profile_selections
            .insert(id.to_string(), profile_name.to_string());
        self.profile_snapshot_events(&tab_id, id, client_id)
    }

    pub(super) fn save_profile_events(
        &mut self,
        client_id: &str,
        id: &str,
        request: ProfileSaveRequest,
    ) -> Vec<OutboundEvent> {
        let tab_id = match self.resolve_profile_window_context(id) {
            Ok(tab_id) => tab_id,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let config_path = match self.profile_config_path() {
            Ok(path) => path,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };
        if let Err(error) = profile_dispatch::save_profile_at(
            &config_path,
            &request.current_name,
            &request.name,
            &request.description,
            &request.env_vars,
            &request.disabled_env,
        ) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )];
        }

        self.profile_selections
            .insert(id.to_string(), request.name.trim().to_string());
        self.profile_snapshot_events(&tab_id, id, client_id)
    }

    pub(crate) fn delete_profile_events(
        &mut self,
        client_id: &str,
        id: &str,
        profile_name: &str,
    ) -> Vec<OutboundEvent> {
        let tab_id = match self.resolve_profile_window_context(id) {
            Ok(tab_id) => tab_id,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };

        let config_path = match self.profile_config_path() {
            Ok(path) => path,
            Err(message) => {
                return vec![OutboundEvent::reply(
                    client_id,
                    BackendEvent::ProfileError {
                        id: id.to_string(),
                        message,
                    },
                )];
            }
        };
        if let Err(error) = profile_dispatch::delete_profile_at(&config_path, profile_name) {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProfileError {
                    id: id.to_string(),
                    message: error.to_string(),
                },
            )];
        }

        self.profile_snapshot_events(&tab_id, id, client_id)
    }

    pub(super) fn resolve_profile_window_context(
        &self,
        id: &str,
    ) -> std::result::Result<String, String> {
        let Some(address) = self.window_lookup.get(id) else {
            return Err("Window not found".to_string());
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return Err("Project tab not found".to_string());
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return Err("Window not found".to_string());
        };
        if window.preset != WindowPreset::Profile {
            return Err("Window is not a Profile surface".to_string());
        }

        Ok(address.tab_id.clone())
    }

    pub(super) fn profile_window_ids_for_tab(&self, tab_id: &str) -> Vec<String> {
        let Some(tab) = self.tab(tab_id) else {
            return Vec::new();
        };
        tab.workspace
            .persisted()
            .windows
            .iter()
            .filter(|window| window.preset == WindowPreset::Profile)
            .map(|window| combined_window_id(tab_id, &window.id))
            .collect()
    }

    pub(super) fn profile_config_path(&self) -> std::result::Result<PathBuf, String> {
        if let Some(path) = &self.profile_config_path {
            return Ok(path.clone());
        }
        profile_dispatch::config_path().map_err(|error| error.to_string())
    }

    pub(super) fn active_profile_spawn_env(
        &self,
    ) -> Result<gwt_agent::LaunchEnvironment, String> {
        let config_path = self.profile_config_path()?;
        gwt_agent::LaunchEnvironment::from_active_profile(
            &config_path,
            gwt_agent::LaunchRuntimeTarget::Host,
        )
    }

    pub(super) fn profile_snapshot_events(
        &mut self,
        tab_id: &str,
        selected_window_id: &str,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let window_ids = self.profile_window_ids_for_tab(tab_id);
        let mut events = Vec::new();

        for window_id in window_ids {
            let selected_profile = self.profile_selections.get(&window_id).cloned();
            let config_path = match self.profile_config_path() {
                Ok(path) => path,
                Err(message) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::ProfileError {
                            id: selected_window_id.to_string(),
                            message,
                        },
                    )];
                }
            };
            match profile_dispatch::load_snapshot_at(&config_path, selected_profile.as_deref()) {
                Ok(snapshot) => {
                    self.profile_selections
                        .insert(window_id.clone(), snapshot.selected_profile.clone());
                    events.push(OutboundEvent::broadcast(BackendEvent::ProfileSnapshot {
                        id: window_id,
                        snapshot,
                    }));
                }
                Err(error) => {
                    return vec![OutboundEvent::reply(
                        client_id,
                        BackendEvent::ProfileError {
                            id: selected_window_id.to_string(),
                            message: error.to_string(),
                        },
                    )];
                }
            }
        }

        events
    }
}

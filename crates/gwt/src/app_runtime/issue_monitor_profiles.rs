use super::*;
use gwt::runtime_daemon_events::IssueMonitorControlPublishError;

const PROFILE_SAVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Owned by the runtime, rather than a tab generation: reopening a project must
/// not let a new write overtake a durable write from its previous generation.
pub(crate) struct IssueMonitorProfileSaveLane {
    ticket: Arc<()>,
    deferred: Vec<UserEvent>,
}

#[derive(Debug, Clone)]
struct SavedProfiles {
    profiles: Vec<gwt::IssueMonitorLaunchProfile>,
    fallback_monitor: Option<gwt::IssueMonitorState>,
}

#[derive(Debug, Clone)]
pub(crate) struct IssueMonitorProfilesSaveCompletion {
    context: ProjectContext,
    client_id: ClientId,
    request_id: Option<u64>,
    ticket: Arc<()>,
    result: Result<SavedProfiles, IssueMonitorControlPublishError>,
}

impl AppRuntime {
    pub(super) fn defer_issue_monitor_control_during_profile_save(
        &mut self,
        context: &ProjectContext,
        client_id: &str,
        event: FrontendEvent,
    ) {
        let lane = self
            .issue_monitor_profile_saves
            .get_mut(&context.project_key)
            .expect("active profile save");
        // Replay through the normal generation and live-client authority checks.
        // Never invoke the frontend handler recursively from a worker completion.
        lane.deferred.push(UserEvent::ProjectCompletion {
            context: context.clone(),
            event: Box::new(UserEvent::Frontend {
                client_id: client_id.to_string(),
                client_scope: Some(ClientScope::Project(context.project_key.clone())),
                event,
                received_at: Instant::now(),
            }),
        });
    }

    pub(super) fn issue_monitor_profiles_set_events(
        &mut self,
        context: &ProjectContext,
        client_id: &str,
        patches: Vec<gwt::IssueMonitorLaunchProfilePatch>,
        usage_threshold_percent: Option<u8>,
        request_id: Option<u64>,
    ) -> Vec<OutboundEvent> {
        let ticket = Arc::new(());
        self.issue_monitor_profile_saves.insert(
            context.project_key.clone(),
            IssueMonitorProfileSaveLane {
                ticket: ticket.clone(),
                deferred: Vec::new(),
            },
        );
        let context = context.clone();
        let worker_context = context.clone();
        let client_id = client_id.to_string();
        let worker_client = client_id.clone();
        let worker_ticket = ticket.clone();
        // The completion must survive project closure so it can release the lane.
        let proxy = self.proxy.clone();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::save_issue_monitor_profiles(
                    &worker_context.project_root, &patches, usage_threshold_percent,
                )
            })).unwrap_or_else(|_| Err(IssueMonitorControlPublishError::OutcomeUnknown(
                "Issue Monitor profile save worker panicked; reload settings to inspect the durable state".into(),
            )));
            proxy.send(UserEvent::IssueMonitorProfilesSaved(Box::new(
                IssueMonitorProfilesSaveCompletion {
                    context: worker_context,
                    client_id: worker_client,
                    request_id,
                    ticket: worker_ticket,
                    result,
                },
            )));
        }) {
            return self.handle_issue_monitor_profiles_saved(IssueMonitorProfilesSaveCompletion {
                context, client_id, request_id, ticket,
                result: Err(IssueMonitorControlPublishError::Rejected(format!(
                    "could not start Issue Monitor profile save: {error}",
                ))),
            });
        }
        Vec::new()
    }

    fn save_issue_monitor_profiles(
        project_root: &Path,
        patches: &[gwt::IssueMonitorLaunchProfilePatch],
        usage_threshold_percent: Option<u8>,
    ) -> Result<SavedProfiles, IssueMonitorControlPublishError> {
        use gwt_core::operation_deadline::{now, ScopedOperationDeadline};
        let _deadline = ScopedOperationDeadline::enter(now() + PROFILE_SAVE_TIMEOUT);
        let remaining = || {
            gwt_core::operation_deadline::current()
                .and_then(|deadline| deadline.checked_duration_since(now()))
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| IssueMonitorControlPublishError::OutcomeUnknown(
                    "Issue Monitor profile save deadline expired; reload settings to inspect the durable state".into(),
                ))
        };
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let saved = gwt::load_issue_monitor_prefs(&prefs_path)
            .map_err(|error| IssueMonitorControlPublishError::Rejected(error.to_string()))?;
        let request =
            gwt::issue_monitor::IssueMonitorProfilesSetRequest::capture(&saved, patches.to_vec())
                .map_err(|error| IssueMonitorControlPublishError::Rejected(error.to_string()))?;
        let payload = gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({"profiles_patch": request}),
            std::process::id(),
        );
        let publication = gwt::daemon_publisher::publish_issue_monitor_control_with_timeout(
            project_root,
            payload,
            remaining()?,
        );
        let fallback_monitor = match publication {
            Ok(()) => None,
            Err(error) if error.allows_local_fallback() => {
                remaining()?;
                let (cached_issues, projection_error, now) =
                    Self::load_local_issue_monitor_fallback_projection(project_root);
                Some(
                    gwt::try_mutate_issue_monitor_prefs_without_authority_fence(
                        &prefs_path,
                        |prefs| {
                            // Compare and merge the raw disk read set before the
                            // observer normalizes the legacy saved-head mirror.
                            let (profiles, _) = request.resolve(prefs)?;
                            gwt::validate_monitor_profiles_set(&profiles, usage_threshold_percent)?;
                            let mut monitor = gwt::IssueMonitorState::with_prefs(
                                gwt::IssueMonitorConfig::default(),
                                prefs.clone(),
                            );
                            monitor.advance_effect_authority_epoch().ok_or_else(|| {
                                std::io::Error::other("authority epoch exhausted")
                            })?;
                            monitor.set_launch_profile_pool(profiles);
                            Self::apply_local_issue_monitor_fallback_projection(
                                &mut monitor,
                                &cached_issues,
                                projection_error.as_deref(),
                                &now,
                            );
                            *prefs = monitor.prefs();
                            Ok(monitor)
                        },
                    )
                    .map_err(|error| {
                        IssueMonitorControlPublishError::Rejected(format!(
                            "local fallback control commit failed: {error}",
                        ))
                    })?
                    .1,
                )
            }
            Err(error) => return Err(error),
        };
        // An ACK means durable commit plus canonical readback, never transport
        // admission alone. A readback failure must also settle the saving UI.
        remaining()?;
        let profiles = gwt::load_issue_monitor_prefs(&prefs_path)
            .map_err(|error| {
                IssueMonitorControlPublishError::OutcomeUnknown(format!(
                    "Issue Monitor profiles committed but readback failed: {error}",
                ))
            })?
            .launch_profile_pool();
        remaining()?;
        Ok(SavedProfiles {
            profiles,
            fallback_monitor,
        })
    }

    pub(crate) fn handle_issue_monitor_profiles_saved(
        &mut self,
        completion: IssueMonitorProfilesSaveCompletion,
    ) -> Vec<OutboundEvent> {
        let IssueMonitorProfilesSaveCompletion {
            context,
            client_id,
            request_id,
            ticket,
            result,
        } = completion;
        if !self
            .issue_monitor_profile_saves
            .get(&context.project_key)
            .is_some_and(|lane| Arc::ptr_eq(&lane.ticket, &ticket))
        {
            return Vec::new();
        }
        let lane = self
            .issue_monitor_profile_saves
            .remove(&context.project_key)
            .expect("matched profile save lane");
        self.proxy.prepend_events(lane.deferred);
        if !self.project_context_is_current(&context) {
            return Vec::new();
        }
        self.invalidate_issue_monitor_list(&context.project_root);
        match result {
            Ok(SavedProfiles {
                profiles,
                fallback_monitor,
            }) => {
                let mut events = fallback_monitor
                    .map(|monitor| {
                        self.issue_monitor_snapshot_events_for(
                            Some(&client_id),
                            Some(&context.project_root),
                            monitor,
                        )
                    })
                    .unwrap_or_default();
                let choices = self.issue_monitor_profile_choices(&context.project_root, &profiles);
                events.push(OutboundEvent::reply(
                    &client_id,
                    BackendEvent::IssueMonitorProfiles {
                        project_root: context.project_root.display().to_string(),
                        profiles,
                        choices,
                        request_id,
                    },
                ));
                events
            }
            Err(error) => {
                let message = error.to_string();
                let mut events = self.issue_monitor_control_error_events(
                    Some(&context.project_root),
                    Some(&client_id),
                    error,
                    "profiles-set",
                    None,
                );
                if let Some(request_id) = request_id {
                    events.push(OutboundEvent::reply(
                        &client_id,
                        BackendEvent::IssueMonitorProfilesWriteFailed {
                            project_root: context.project_root.display().to_string(),
                            request_id,
                            message,
                        },
                    ));
                }
                events
            }
        }
    }
}

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
        let resolve = |prefs: &gwt::IssueMonitorPrefs| {
            let pool = prefs.launch_profile_pool();
            let (profiles, _) = gwt::merge_issue_monitor_profiles_set(
                &pool,
                prefs.launch_profile.as_ref().or(pool.first()),
                patches,
            );
            gwt::validate_monitor_profiles_set(&profiles, usage_threshold_percent)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(profiles)
        };
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let profiles = gwt::load_issue_monitor_prefs(&prefs_path)
            .map_err(|error| error.to_string())
            .and_then(|prefs| resolve(&prefs))
            .map_err(IssueMonitorControlPublishError::Rejected)?;
        let payload = gwt::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({"profiles_set": {"profiles": profiles}}),
            std::process::id(),
        );
        let publication = gwt::daemon_publisher::publish_issue_monitor_control_with_timeout(
            project_root,
            payload,
            remaining()?,
        );
        let fallback_monitor = match publication {
            Ok(()) => None,
            Err(error) if error.allows_local_fallback() => Some(
                Self::commit_local_issue_monitor_authorizing_control_with_timeout(
                    project_root,
                    remaining()?,
                    |monitor| {
                        // Omitted fields resolve against locked current prefs.
                        let profiles = resolve(&monitor.prefs())?;
                        monitor
                            .advance_effect_authority_epoch()
                            .ok_or_else(|| "authority epoch exhausted".to_string())?;
                        monitor.set_launch_profile_pool(profiles);
                        Ok(())
                    },
                )?
                .0,
            ),
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

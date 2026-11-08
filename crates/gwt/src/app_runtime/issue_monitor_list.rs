use super::*;
use gwt::runtime_daemon_events::IssueMonitorControlPublishError;

#[derive(Debug, Clone)]
struct ListSnapshot {
    normalized_root: PathBuf,
    status: Box<gwt::IssueMonitorStatusView>,
    inbox: Option<Vec<gwt::IssueMonitorInboxItem>>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedIssueMonitorList {
    context: ProjectContext,
    client_id: ClientId,
    request: Arc<()>,
    revision: u64,
    status_revision: u64,
    inbox_revision: u64,
    result: Result<Option<ListSnapshot>, IssueMonitorControlPublishError>,
}

impl AppRuntime {
    pub(super) fn remember_issue_monitor_inbox(
        &self,
        project_root: &Path,
        items: &[gwt::IssueMonitorInboxItem],
    ) {
        if let Some(state) = self.project_state_for_root(project_root) {
            state
                .issue_monitor_inbox_revision
                .set(state.issue_monitor_inbox_revision.get().wrapping_add(1));
            state
                .issue_monitor_inbox_cache
                .replace(Some(items.to_vec()));
        }
    }

    pub(super) fn invalidate_issue_monitor_list(&self, project_root: &Path) {
        if let Some(state) = self.project_state_for_root(project_root) {
            state
                .issue_monitor_list_revision
                .set(state.issue_monitor_list_revision.get().wrapping_add(1));
        }
    }

    pub(super) fn list_issue_monitor_events_with_reader(
        &mut self,
        context: &ProjectContext,
        client_id: &str,
        read_status: impl FnOnce(&Path) -> Result<Option<serde_json::Value>, IssueMonitorControlPublishError>
            + Send
            + 'static,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state_mut(context) else {
            return Vec::new();
        };
        let request = Arc::new(());
        state
            .issue_monitor_list_requests
            .insert(client_id.to_string(), request.clone());
        let revision = state.issue_monitor_list_revision.get();
        let status_revision = state.issue_monitor_status_revision.get();
        let inbox_revision = state.issue_monitor_inbox_revision.get();
        let context = context.clone();
        let worker_context = context.clone();
        let worker_client = client_id.to_string();
        let proxy = self.proxy.for_project(context.clone());
        let cache = self.launch_wizard_cache.clone();
        let (panes, worktrees) = self.capture_update_quiescence_inputs();
        if let Err(error) = self.blocking_tasks.try_spawn(move || {
            let result = Self::prepare_issue_monitor_list(
                &worker_context.project_root,
                &cache,
                panes,
                worktrees,
                read_status,
            );
            proxy.send(UserEvent::IssueMonitorListPrepared(Box::new(
                PreparedIssueMonitorList {
                    context: worker_context,
                    client_id: worker_client,
                    request,
                    revision,
                    status_revision,
                    inbox_revision,
                    result,
                },
            )));
        }) {
            self.project_state_mut(&context)
                .unwrap()
                .issue_monitor_list_requests
                .remove(client_id);
            return self.issue_monitor_control_error_events(
                Some(&context.project_root),
                Some(client_id),
                IssueMonitorControlPublishError::TransportUnavailable(error),
                "list",
                None,
            );
        }
        Vec::new()
    }

    fn prepare_issue_monitor_list(
        project_root: &Path,
        cache: &launch::LaunchWizardMemoryCache,
        panes: Vec<gwt::update_drain::PaneObservation>,
        worktrees: Vec<PathBuf>,
        read_status: impl FnOnce(
            &Path,
        )
            -> Result<Option<serde_json::Value>, IssueMonitorControlPublishError>,
    ) -> Result<Option<ListSnapshot>, IssueMonitorControlPublishError> {
        let normalized_root = knowledge::normalized_snapshot_project_root(project_root);
        if let Some(value) = read_status(project_root)? {
            let status =
                serde_json::from_value::<gwt::IssueMonitorAgentStatus>(value).map_err(|error| {
                    IssueMonitorControlPublishError::OutcomeUnknown(format!(
                        "invalid daemon Issue Monitor status: {error}"
                    ))
                })?;
            // Older live daemons supply their projection through subscriber frames.
            return Ok(status.gui_status.map(|status| ListSnapshot {
                normalized_root,
                status: Box::new(status),
                inbox: None,
            }));
        }
        let prefs_path = gwt::issue_monitor_prefs_path_for_repo_path(project_root);
        let prefs = gwt::load_issue_monitor_prefs(&prefs_path)
            .unwrap_or_else(|_| gwt::IssueMonitorPrefs::recovery_default());
        let mut monitor =
            gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs);
        monitor.refresh_agent_capacity(project_root);
        let (issues, error, now, source_repository) =
            Self::load_local_issue_monitor_fallback_projection(project_root);
        monitor.set_source_repository(
            source_repository
                .as_ref()
                .map(|(owner, repo)| (owner.as_str(), repo.as_str())),
        );
        // A List is a read model, like the quick cache view. Scheduled scans own
        // durable admission, launch expiry and PM wake transitions.
        gwt::scan_issue_monitor_candidates(&mut monitor, &issues, &now);
        if monitor.config.enabled {
            monitor.set_gui_connected(true);
        }
        if let Some(error) = error {
            monitor.record_scan_error(&now, error);
        }
        let mut status = monitor.status_view();
        Self::apply_issue_monitor_launch_profile_status_from_cache(
            &mut status,
            Some(project_root),
            cache,
        );
        if let Some(drain) = status.update_drain.as_mut() {
            drain.blocking = gwt::update_drain::update_quiescence(
                &Self::read_update_quiescence_snapshot(panes, worktrees, &monitor),
            )
            .err()
            .unwrap_or_default();
        }
        Ok(Some(ListSnapshot {
            normalized_root,
            status: Box::new(status),
            inbox: Some(monitor.inbox_view_at(&now)),
        }))
    }

    pub(crate) fn issue_monitor_list_prepared_events(
        &mut self,
        prepared: PreparedIssueMonitorList,
    ) -> Vec<OutboundEvent> {
        let PreparedIssueMonitorList {
            context,
            client_id,
            request,
            revision,
            status_revision,
            inbox_revision,
            result,
        } = prepared;
        let Some(state) = self.project_state_mut(&context) else {
            return Vec::new();
        };
        if !state
            .issue_monitor_list_requests
            .get(&client_id)
            .is_some_and(|current| Arc::ptr_eq(current, &request))
        {
            return Vec::new();
        }
        state.issue_monitor_list_requests.remove(&client_id);
        if revision != state.issue_monitor_list_revision.get() {
            // A control changed the durable source while this read was running.
            return self.list_issue_monitor_events_with_reader(
                &context,
                &client_id,
                gwt::daemon_publisher::read_issue_monitor_status,
            );
        }
        match result {
            Ok(Some(ListSnapshot {
                normalized_root,
                mut status,
                mut inbox,
            })) => {
                // Newer published projections already own the display. Reuse
                // them rather than retrying forever under periodic pushes.
                let state = self.project_state(&context).unwrap();
                if status_revision != state.issue_monitor_status_revision.get() {
                    if let Some(current) = state.issue_monitor_status_cache.borrow().as_ref() {
                        status = Box::new(current.clone());
                    }
                }
                if inbox_revision != state.issue_monitor_inbox_revision.get() {
                    inbox = state.issue_monitor_inbox_cache.borrow().clone();
                }
                self.retain_missing_issue_monitor_capacity(&context.project_root, &mut status);
                self.apply_issue_monitor_tier_status(&mut status, Some(&context.project_root));
                if let Some(drain) = status.update_drain.as_mut() {
                    self.enrich_update_drain_blockers(drain);
                }
                self.knowledge_monitor_snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .replace_queue_normalized(&normalized_root, &status.terminal_queue);
                self.remember_issue_monitor_status(Some(&context.project_root), &status);
                let mut events = self.project_attention_events(
                    &context.project_root,
                    status.last_error.as_deref(),
                    status.last_scan_at.as_deref(),
                );
                events.push(OutboundEvent::reply(
                    &client_id,
                    BackendEvent::IssueMonitorStatus { status },
                ));
                if let Some(items) = inbox {
                    self.remember_issue_monitor_inbox(&context.project_root, &items);
                    self.knowledge_monitor_snapshot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .replace_normalized(&normalized_root, &items);
                    events.push(OutboundEvent::reply(
                        &client_id,
                        BackendEvent::IssueMonitorInbox { items },
                    ));
                }
                events
            }
            Ok(None) => Vec::new(),
            Err(error) => self.issue_monitor_control_error_events(
                Some(&context.project_root),
                Some(&client_id),
                error,
                "list",
                None,
            ),
        }
    }

    #[cfg(test)]
    pub(super) fn completed_issue_monitor_list_for_test(
        &mut self,
        client_id: &str,
        reader: impl FnOnce(&Path) -> Result<Option<serde_json::Value>, IssueMonitorControlPublishError>
            + Send
            + 'static,
    ) -> Vec<OutboundEvent> {
        // Let the fixture's persistence worker enter its scoped HOME before
        // a fast List-only test can remove that temporary directory.
        self.persist().unwrap();
        assert!(self
            .persist_dispatcher
            .wait_idle(std::time::Duration::from_secs(30)));
        let (proxy, events) = AppEventProxy::stub();
        let (spawner, tasks) = BlockingTaskSpawner::queued();
        let old_proxy = std::mem::replace(&mut self.proxy, proxy);
        let old_spawner = std::mem::replace(&mut self.blocking_tasks, spawner);
        assert!(self
            .list_issue_monitor_events_with_reader(&self.test_context(), client_id, reader)
            .is_empty());
        tasks.lock().unwrap().pop().unwrap()();
        let event = events.lock().unwrap().pop().unwrap();
        let Some(UserEvent::IssueMonitorListPrepared(prepared)) =
            self.accept_project_completion(event)
        else {
            panic!("List completion expected");
        };
        let result = self.issue_monitor_list_prepared_events(*prepared);
        self.proxy = old_proxy;
        self.blocking_tasks = old_spawner;
        result
    }
}

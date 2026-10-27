//! Workspace projection / resume / cleanup helpers split out of
//! `app_runtime/mod.rs` for SPEC-2077 US-11 / FR-044 (Phase H5,
//! arch-review handoff).
//!
//! Owns the free-function cluster that:
//! - projects [`ActiveAgentSession`]s into the persisted
//!   `WorkspaceProjection` ([`active_agent_summary_from_session`],
//!   [`merge_active_sessions_into_projection`],
//!   [`retain_live_workspace_agents`],
//!   [`workspace_projection_for_current_resume`])
//! - derives Active Work resume / cleanup state
//!   ([`gwt::project_runtime::installation::workspace_projection_owner_title`],
//!   [`workspace_cleanup_candidate_for_projection`])
//! - reexports shared project-runtime launch projection and Work event writers
//!   ([`save_workspace_launch_projection`])
//! - runs the Workspace cleanup flow keyed by
//!   `WORKSPACE_CLEANUP_EVENT_ID` ([`spawn_workspace_cleanup_async`],
//!   [`clear_workspace_cleanup_git_details_event`])
//!
//! GUI view and cleanup adapters remain here. Launch publication and successor
//! transitions use the single implementation in `gwt::project_runtime::workspace`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use gwt::BranchCleanupOperationStore;

use super::{
    active_work_cleanup_candidate_view_from_candidate,
    active_work_projection_from_saved_with_journal, cleanup_selected_branches_with_progress,
    list_branch_entries_with_active_sessions, resume_branch_refs_snapshot, work_session_index,
    workspace_journal_entry_view_from_entry, workspace_work_item_view_from_item,
    ActiveAgentSession, AppEventProxy, AppRuntime, BackendEvent, BranchCleanupOptions,
    BranchEntriesPhase, OutboundEvent, ResumeBranchIndex, UserEvent, WindowPreset,
    WORKSPACE_CLEANUP_EVENT_ID, WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
};

pub(super) fn merge_active_sessions_into_projection<'a>(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: impl IntoIterator<Item = &'a ActiveAgentSession>,
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    for session in sessions {
        let existing = projection
            .latest_agent_for_session(&session.session_id)
            .or_else(|| {
                projection
                    .agents
                    .iter()
                    .find(|agent| agent.window_id.as_deref() == Some(session.window_id.as_str()))
            });
        let mut summary = active_agent_summary_from_session(session, updated_at);
        if let Some(existing) = existing {
            summary.affiliation_status = existing.affiliation_status;
            summary.workspace_id = existing.workspace_id.clone();
            summary.title_summary = existing.title_summary.clone();
            summary.current_focus = existing.current_focus.clone();
            summary.last_board_entry_id = existing.last_board_entry_id.clone();
            summary.last_board_entry_kind = existing.last_board_entry_kind.clone();
            summary.coordination_scope = existing.coordination_scope.clone();
        } else {
            summary.affiliation_status =
                gwt_core::workspace_projection::WorkspaceAgentAffiliationStatus::Unassigned;
            summary.workspace_id = None;
        }
        projection.upsert_agent_summary(summary);
    }
}

pub(super) fn retain_live_workspace_agents(
    projection: &mut gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    updated_at: chrono::DateTime<chrono::Utc>,
) {
    // SPEC-2359 US-80 (FR-429): live agent sessions have no authority over
    // shell liveness, so keep every Shell Work here — otherwise the broadcast
    // rebuild would prune a just-registered Shell Work that has no agent
    // session. Shell Works are pruned only by their own PTY / window lifecycle.
    projection.retain_live_agents_keep_shells(
        sessions.iter().map(|session| session.session_id.as_str()),
        updated_at,
    );
}

pub(super) fn workspace_projection_for_current_resume(
    mut projection: gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
    tab_title: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> gwt_core::workspace_projection::WorkspaceProjection {
    merge_active_sessions_into_projection(&mut projection, sessions.iter().copied(), updated_at);
    retain_live_workspace_agents(&mut projection, sessions, updated_at);
    if !projection.has_current_agents() {
        projection.reset_idle_identity(tab_title, updated_at);
    }
    projection
}

pub(super) fn workspace_cleanup_candidate_for_projection(
    projection: &gwt_core::workspace_projection::WorkspaceProjection,
    sessions: &[&ActiveAgentSession],
) -> Option<gwt::ActiveWorkCleanupCandidateView> {
    let branch = projection.git_details.as_ref()?.branch.as_deref()?;
    let branch_has_live_agent = sessions.iter().any(|session| session.branch_name == branch);
    let candidate = projection.cleanup_candidate(branch_has_live_agent)?;
    Some(active_work_cleanup_candidate_view_from_candidate(candidate))
}

/// Issue #4433: everything the Workspace cleanup worker needs. Grouped into a
/// struct so the reconnect bookkeeping (`operation_id`, `operations`) does not
/// push the spawn helpers past clippy's argument budget.
pub(super) struct WorkspaceCleanupAsyncTask {
    pub(super) context: super::ProjectContext,
    pub(super) project_root: PathBuf,
    pub(super) active_session_branches: std::collections::HashSet<String>,
    pub(super) branch: String,
    pub(super) operation_id: String,
    pub(super) operations: Arc<BranchCleanupOperationStore>,
    pub(super) options: BranchCleanupOptions,
}

/// Issue #4433: same as [`WorkspaceCleanupAsyncTask`], for the Branches
/// window cleanup worker.
struct BranchCleanupAsyncTask {
    context: super::ProjectContext,
    window_id: String,
    project_root: PathBuf,
    active_session_branches: std::collections::HashSet<String>,
    branches: Vec<String>,
    operation_id: String,
    operations: Arc<BranchCleanupOperationStore>,
    options: BranchCleanupOptions,
}

pub(super) fn spawn_workspace_cleanup_async(proxy: AppEventProxy, task: WorkspaceCleanupAsyncTask) {
    let WorkspaceCleanupAsyncTask {
        context,
        project_root,
        active_session_branches,
        branch,
        operation_id,
        operations,
        options,
    } = task;
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_context = context.clone();
                    let progress_operation_id = operation_id.clone();
                    let progress_operations = operations.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        std::slice::from_ref(&branch),
                        options,
                        move |progress| {
                            // Issue #4433: broadcast, so a client that
                            // reconnected mid-cleanup still receives progress.
                            progress_operations.record_progress(
                                WORKSPACE_CLEANUP_EVENT_ID,
                                &progress_operation_id,
                                &progress,
                            );
                            progress_proxy.send(UserEvent::ProjectDispatch {
                                context: progress_context.clone(),
                                events: vec![OutboundEvent::project(
                                    progress_context.project_key.clone(),
                                    BackendEvent::BranchCleanupProgress {
                                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                                        operation_id: progress_operation_id.clone(),
                                        branch: progress.branch,
                                        execution_branch: progress.execution_branch,
                                        index: progress.index,
                                        total: progress.total,
                                        phase: progress.phase,
                                        message: progress.message,
                                    },
                                )],
                            });
                        },
                    );
                    operations.record_result(WORKSPACE_CLEANUP_EVENT_ID, &operation_id, &results);
                    let mut events = vec![OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                            operation_id: operation_id.clone(),
                            results: results.clone(),
                        },
                    )];
                    if results.iter().any(|result| {
                        result.branch == branch
                            && matches!(
                                result.status,
                                gwt::BranchCleanupResultStatus::Success
                                    | gwt::BranchCleanupResultStatus::Partial
                            )
                    }) {
                        // SPEC-2359 US-37 / FR-118: emit Done only after the
                        // matching workspace cleanup actually succeeded.
                        let _ =
                            gwt_core::workspace_projection::emit_workspace_done_event_for_branch(
                                &project_root,
                                &branch,
                                chrono::Utc::now(),
                            );
                        if let Some(event) =
                            clear_workspace_cleanup_git_details_event(&project_root)
                        {
                            events.push(event);
                        }
                    }
                    events
                }
                // Issue #4433: broadcast so a reconnected client is not left
                // waiting on a cleanup that already failed to start.
                Err(error) => vec![OutboundEvent::project(
                    context.project_key.clone(),
                    BackendEvent::BranchError {
                        id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::ProjectDispatch { context, events });
    });
}

fn clear_workspace_cleanup_git_details_event(project_root: &Path) -> Option<OutboundEvent> {
    let projection =
        match gwt_core::workspace_projection::mutate_existing_workspace_projection_for_cleanup(
            project_root,
            |projection| {
                projection.clear_git_details_to_idle(chrono::Utc::now());
                Ok(projection.clone())
            },
        ) {
            Ok(Some(projection)) => projection,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    project_root = %project_root.display(),
                    error = %error,
                    "workspace projection cleanup state update skipped"
                );
                return None;
            }
        };
    let journal_entries = gwt_core::workspace_projection::load_recent_workspace_journal_entries(
        project_root,
        WORKSPACE_OVERVIEW_JOURNAL_LIMIT,
    )
    .unwrap_or_default()
    .iter()
    .map(workspace_journal_entry_view_from_entry)
    .collect::<Vec<_>>();
    // Rare post-cleanup path with no runtime handle: a one-shot cache load
    // matches the previous eager loader's cost and semantics.
    let agent_sessions = crate::session_ledger_cache::SessionLedgerCache::new()
        .load(&gwt_core::paths::gwt_sessions_dir());
    let session_index = work_session_index(&agent_sessions);
    // Issue #3611: this runs on the branch-cleanup worker, so one bulk ref
    // snapshot is affordable — but a per-Session Git probe is not, and the
    // event loop must never inherit one through this view builder.
    let known_branch_refs = resume_branch_refs_snapshot(project_root);
    let resume_branches = ResumeBranchIndex::scanned(Some(&known_branch_refs));
    let workspaces =
        gwt_core::workspace_projection::load_or_synthesize_workspace_work_items(project_root)
            .unwrap_or_else(|_| gwt_core::workspace_projection::WorkItemsProjection {
                updated_at: projection.updated_at,
                work_items: Vec::new(),
            })
            .work_items
            .iter()
            .map(|item| workspace_work_item_view_from_item(item, &session_index, resume_branches))
            .collect::<Vec<_>>();
    Some(OutboundEvent::project(
        gwt_core::paths::resolve_project_scope(project_root).hash,
        BackendEvent::ActiveWorkProjection {
            projection: Box::new(active_work_projection_from_saved_with_journal(
                projection,
                journal_entries,
                workspaces,
                None,
            )),
        },
    ))
}

/// Issue #4433: rebuild the wire event a stored cleanup snapshot stands for.
fn branch_cleanup_snapshot_event(
    id: &str,
    operation_id: &str,
    snapshot: gwt::BranchCleanupOperationSnapshot,
) -> BackendEvent {
    match snapshot {
        gwt::BranchCleanupOperationSnapshot::Progress(progress) => {
            BackendEvent::BranchCleanupProgress {
                id: id.to_string(),
                operation_id: operation_id.to_string(),
                branch: progress.branch,
                execution_branch: progress.execution_branch,
                index: progress.index,
                total: progress.total,
                phase: progress.phase,
                message: progress.message,
            }
        }
        gwt::BranchCleanupOperationSnapshot::Result(results) => BackendEvent::BranchCleanupResult {
            id: id.to_string(),
            operation_id: operation_id.to_string(),
            results,
        },
    }
}

fn spawn_branch_cleanup_async(proxy: AppEventProxy, task: BranchCleanupAsyncTask) {
    let BranchCleanupAsyncTask {
        context,
        window_id,
        project_root,
        active_session_branches,
        branches,
        operation_id,
        operations,
        options,
    } = task;
    thread::spawn(move || {
        let events =
            match list_branch_entries_with_active_sessions(&project_root, &active_session_branches)
            {
                Ok(entries) => {
                    let progress_proxy = proxy.clone();
                    let progress_context = context.clone();
                    let progress_window_id = window_id.clone();
                    let progress_operation_id = operation_id.clone();
                    let progress_operations = operations.clone();
                    let results = cleanup_selected_branches_with_progress(
                        &project_root,
                        &entries,
                        &branches,
                        options,
                        move |progress| {
                            // Issue #4433: broadcast, so a client that
                            // reconnected mid-cleanup still receives progress.
                            progress_operations.record_progress(
                                &progress_window_id,
                                &progress_operation_id,
                                &progress,
                            );
                            progress_proxy.send(UserEvent::ProjectDispatch {
                                context: progress_context.clone(),
                                events: vec![OutboundEvent::project(
                                    progress_context.project_key.clone(),
                                    BackendEvent::BranchCleanupProgress {
                                        id: progress_window_id.clone(),
                                        operation_id: progress_operation_id.clone(),
                                        branch: progress.branch,
                                        execution_branch: progress.execution_branch,
                                        index: progress.index,
                                        total: progress.total,
                                        phase: progress.phase,
                                        message: progress.message,
                                    },
                                )],
                            });
                        },
                    );
                    operations.record_result(&window_id, &operation_id, &results);
                    let mut events = vec![OutboundEvent::project(
                        context.project_key.clone(),
                        BackendEvent::BranchCleanupResult {
                            id: window_id.clone(),
                            operation_id: operation_id.clone(),
                            results,
                        },
                    )];
                    match list_branch_entries_with_active_sessions(
                        &project_root,
                        &active_session_branches,
                    ) {
                        Ok(entries) => {
                            events.push(OutboundEvent::project(
                                context.project_key.clone(),
                                BackendEvent::BranchEntries {
                                    id: window_id.clone(),
                                    phase: BranchEntriesPhase::Hydrated,
                                    entries,
                                    // SPEC-2009 FR-067: fresh load id from the shared
                                    // sequence so the post-cleanup reload is never
                                    // dropped as stale by the frontend.
                                    load_id: gwt::next_branch_load_id(),
                                },
                            ))
                        }
                        Err(error) => events.push(OutboundEvent::project(
                            context.project_key.clone(),
                            BackendEvent::BranchError {
                                id: window_id.clone(),
                                message: error.to_string(),
                            },
                        )),
                    }
                    events
                }
                Err(error) => vec![OutboundEvent::project(
                    context.project_key.clone(),
                    BackendEvent::BranchError {
                        id: window_id,
                        message: error.to_string(),
                    },
                )],
            };
        proxy.send(UserEvent::ProjectDispatch { context, events });
    });
}

impl AppRuntime {
    /// A cached rail refresh cannot prove that an unreadable works.json has
    /// recovered: file permissions are not part of its cache signature.
    pub(crate) fn recheck_workspace_state_after_projection(&self, project_root: &Path) {
        let Some(context) = self.project_context_for_root(project_root) else {
            return;
        };
        let pending = self.project_state(&context).is_some_and(|state| {
            state
                .workspace_state_notice
                .as_ref()
                .is_some_and(|notice| notice.kind == gwt::WorkspaceStateNoticeKind::LoadError)
        });
        if pending {
            crate::spawn_workspace_projection_reload(
                &self.blocking_tasks,
                self.proxy.clone(),
                context,
            );
        }
    }

    pub(crate) fn handle_workspace_state_load_failed(
        &mut self,
        project_root: &Path,
        error: gwt_core::WorkspaceStateLoadError,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        tracing::warn!(path = %error.path.display(), error = %error.message, "workspace state load failed");
        let notice = gwt::WorkspaceStateNoticeView {
            path: error.path.display().to_string(),
            message: error.message,
            kind: gwt::WorkspaceStateNoticeKind::LoadError,
        };
        self.project_state_mut(&context)
            .expect("current project")
            .workspace_state_notice = Some(notice.clone());
        vec![OutboundEvent::project(
            context.project_key,
            BackendEvent::WorkspaceStateNotice {
                notice: Some(notice),
            },
        )]
    }

    pub(crate) fn handle_workspace_state_loaded(
        &mut self,
        project_root: &Path,
    ) -> Vec<OutboundEvent> {
        let Some(context) = self.project_context_for_root(project_root) else {
            return Vec::new();
        };
        let state = self.project_state_mut(&context).expect("current project");
        let recovered = state
            .workspace_state_notice
            .as_ref()
            .is_some_and(|notice| notice.kind == gwt::WorkspaceStateNoticeKind::LoadError);
        if !recovered {
            return Vec::new();
        }
        state.workspace_state_notice = None;
        self.spawn_work_events_ingest(project_root.to_path_buf(), true);
        let _ = self.active_work_projection_broadcast_for_tab(&context.tab_id);
        vec![OutboundEvent::project(
            context.project_key,
            BackendEvent::WorkspaceStateNotice { notice: None },
        )]
    }

    pub(crate) fn run_branch_cleanup_events(
        &self,
        client_id: &str,
        id: &str,
        branches: &[String],
        delete_remote: bool,
        force_filesystem_delete: bool,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: "Window not found".to_string(),
                },
            )];
        };

        if window.preset != WindowPreset::Branches && window.preset != WindowPreset::Work {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: id.to_string(),
                    message: format!("Window preset {:?} is not a Work surface", window.preset),
                },
            )];
        }

        let Some(context) = self.project_context(&address.tab_id) else {
            return Vec::new();
        };
        let Some(state) = self.project_state(&context) else {
            return Vec::new();
        };
        spawn_branch_cleanup_async(
            self.proxy.clone(),
            BranchCleanupAsyncTask {
                context: context.clone(),
                window_id: id.to_string(),
                project_root: tab.project_root.clone(),
                active_session_branches: self.active_session_branches_for_tab(&address.tab_id),
                branches: branches.to_vec(),
                operation_id: operation_id.to_string(),
                operations: state.branch_cleanup_operations.clone(),
                options: BranchCleanupOptions {
                    delete_remote,
                    force_filesystem_delete,
                },
            },
        );
        Vec::new()
    }

    pub(crate) fn run_workspace_cleanup_events(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
        branch: &str,
        delete_remote: bool,
        force_filesystem_delete: bool,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let tab_id = &context.tab_id;
        let Some(tab) = self.tab(tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::BranchError {
                    id: WORKSPACE_CLEANUP_EVENT_ID.to_string(),
                    message: "Project tab not found".to_string(),
                },
            )];
        };

        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        spawn_workspace_cleanup_async(
            self.proxy.clone(),
            WorkspaceCleanupAsyncTask {
                context: context.clone(),
                project_root: tab.project_root.clone(),
                active_session_branches: self.active_session_branches_for_tab(tab_id),
                branch: branch.to_string(),
                operation_id: operation_id.to_string(),
                operations: state.branch_cleanup_operations.clone(),
                options: BranchCleanupOptions {
                    delete_remote,
                    force_filesystem_delete,
                },
            },
        );
        Vec::new()
    }

    /// Issue #4433 AC-2: replay the current state of an in-flight cleanup to a
    /// client that reconnected (and therefore has a new `client_id`) while the
    /// cleanup was still running. An unknown operation replies with nothing —
    /// silence must never be rendered as a cleanup failure.
    pub(crate) fn sync_branch_cleanup_events(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
        id: &str,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        let Some(snapshot) = state.branch_cleanup_operations.snapshot(id, operation_id) else {
            return Vec::new();
        };
        vec![OutboundEvent::reply(
            client_id,
            branch_cleanup_snapshot_event(id, operation_id, snapshot),
        )]
    }

    /// Issue #4433 AC-2: replay every live cleanup to a client during its
    /// initial sync. A reloaded page lost the operation ids it was watching, so
    /// it cannot ask for them by name.
    pub(crate) fn live_branch_cleanup_replies(
        &self,
        context: &super::ProjectContext,
        client_id: &str,
    ) -> Vec<OutboundEvent> {
        let Some(state) = self.project_state(context) else {
            return Vec::new();
        };
        state
            .branch_cleanup_operations
            .live_operations()
            .into_iter()
            .map(|(id, operation_id, snapshot)| {
                OutboundEvent::reply(
                    client_id,
                    branch_cleanup_snapshot_event(&id, &operation_id, snapshot),
                )
            })
            .collect()
    }

    /// Issue #4433 AC-3: the client consumed the operation's result, so drop
    /// the snapshot instead of replaying it into the next cleanup.
    pub(crate) fn clear_branch_cleanup_status_events(
        &self,
        context: &super::ProjectContext,
        id: &str,
        operation_id: &str,
    ) -> Vec<OutboundEvent> {
        if let Some(state) = self.project_state(context) {
            state.branch_cleanup_operations.clear(id, operation_id);
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs2::FileExt;
    use gwt_core::test_support::{env_lock, ScopedEnvVar};

    #[test]
    fn cleanup_completion_does_not_recreate_projection_deleted_while_waiting_for_lock() {
        let _env_guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("create home");
        let _home = ScopedEnvVar::set("HOME", &home);
        let project_root = temp.path().join("repo");
        std::fs::create_dir_all(&project_root).expect("create project root");

        let projection =
            gwt_core::workspace_projection::WorkspaceProjection::default_for_project(&project_root);
        gwt_core::workspace_projection::save_workspace_projection(&project_root, &projection)
            .expect("save current projection");
        let legacy_path = gwt_core::paths::gwt_project_dir_for_repo_path(&project_root)
            .join("workspace/current.json");
        // Retired HOME residue predates the current writer, which rejects it.
        let legacy_bytes = serde_json::to_vec(&projection).expect("serialize legacy fixture");
        std::fs::create_dir_all(legacy_path.parent().unwrap()).expect("create legacy directory");
        std::fs::write(&legacy_path, &legacy_bytes).expect("seed remaining legacy projection");
        let current_path =
            gwt_core::paths::gwt_workspace_projection_path_for_repo_path(&project_root);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(current_path.with_file_name("works.lock"))
            .expect("open project lock");
        lock.lock_exclusive().expect("lock project state");

        let root_for_cleanup = project_root.clone();
        let handle = std::thread::spawn(move || {
            clear_workspace_cleanup_git_details_event(&root_for_cleanup)
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            !handle.is_finished(),
            "cleanup completion must wait for the project lock"
        );

        std::fs::remove_file(&current_path).expect("remove current projection while locked");
        FileExt::unlock(&lock).expect("unlock project state");
        let event = handle.join().expect("join cleanup completion");

        assert_eq!(
            (event.is_none(), current_path.exists()),
            (true, false),
            "a deleted current projection must not be broadcast or recreated by cleanup completion"
        );
        assert_eq!(
            std::fs::read(&legacy_path).expect("read preserved legacy projection"),
            legacy_bytes,
            "cleanup must preserve retired HOME bytes when canonical state disappears"
        );
        let error = gwt_core::workspace_projection::load_workspace_projection(&project_root)
            .expect_err("remaining retired HOME state must be refused");
        assert!(
            matches!(
                error,
                gwt_core::error::GwtError::WorkspaceStateLoad(ref detail)
                    if detail.kind == gwt_core::error::WorkspaceStateLoadErrorKind::LegacyLayout
                        && detail.path == legacy_path
            ),
            "a normal load must refuse retired state without recreating canonical state: {error}"
        );
    }
}

#[cfg(test)]
pub(super) use gwt::project_runtime::workspace::save_workspace_launch_projection;
pub(super) use gwt::project_runtime::workspace::{
    active_agent_summary_from_session, apply_workspace_launch_for_current_work,
    apply_workspace_launch_transition, invalid_successor_supersession_event, workspace_owner_label,
    SuccessorSupersessionAuthority, WorkspaceLaunchProjectionKind, WorkspaceLaunchTransition,
};

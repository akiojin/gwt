//! Cooperative switching beside the fixed project's physical runtime owner.
use super::agent_dispatch::PtyWriterEntry;
use crate as gwt;
use gwt::issue_monitor::{AgentSwitchAction, AgentSwitchIntent, AgentSwitchStage};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock},
};

pub(crate) struct AgentSwitchSourceExited {
    pub operation_id: String,
    pub proof: gwt::issue_monitor::IssueMonitorRuntimeStopRequest,
    pub pty: Arc<gwt_terminal::PtyHandle>,
}

pub(super) fn apply_action(
    root: &Path,
    sessions: &Path,
    action: AgentSwitchAction,
) -> Result<gwt::issue_monitor::AgentSwitchSnapshot, String> {
    recover_previous_host_intents(root).map_err(|error| error.to_string())?;
    gwt::issue_monitor::mutate_project_agent_switch(root, sessions, action)
}

pub(super) fn drive_pending(
    root: &Path,
    sessions: &Path,
    entries: &[Arc<PtyWriterEntry>],
    writers: &Arc<RwLock<HashMap<String, Arc<PtyWriterEntry>>>>,
    retire: impl Fn(AgentSwitchSourceExited),
) -> Result<gwt::issue_monitor::AgentSwitchSnapshot, String> {
    let path = gwt::issue_monitor_prefs_path_for_repo_path(root);
    if let Some(_guard) = SwitchTickGuard::claim(root) {
        recover_previous_host_intents(root).map_err(|error| error.to_string())?;
        let prefs = gwt::load_issue_monitor_prefs(&path).map_err(|error| error.to_string())?;
        let host = gwt::process::current_hostname();
        let host_pid = std::process::id();
        let host_started_at = gwt::process::host_process_start_time(host_pid);
        for intent in prefs.agent_switch_intents.values().filter(|intent| {
            intent.source.assignment.host_id == host
                && intent.runtime_stop.as_ref().is_some_and(|proof| {
                    proof.host_pid == host_pid && Some(proof.host_started_at) == host_started_at
                })
                && !matches!(
                    intent.stage,
                    AgentSwitchStage::Completed
                        | AgentSwitchStage::Cancelled
                        | AgentSwitchStage::Failed
                        | AgentSwitchStage::Unknown
                )
        }) {
            if let Err(error) = drive_switch(root, sessions, intent, entries, writers, &retire) {
                gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
                    if let Some(current) =
                        prefs.agent_switch_intents.get(&intent.request.operation_id)
                    {
                        if error.kind() == io::ErrorKind::WouldBlock
                            && matches!(
                                current.stage,
                                AgentSwitchStage::Requested | AgentSwitchStage::CheckpointVerified
                            )
                        {
                            prefs
                                .transition_agent_switch(
                                    &intent.request.operation_id,
                                    current.stage,
                                    Some(error.to_string()),
                                )
                                .map_err(switch_io)?;
                            return Ok(());
                        }
                        let stage = if matches!(
                            current.stage,
                            AgentSwitchStage::StoppingCurrent | AgentSwitchStage::LaunchingNext
                        ) {
                            AgentSwitchStage::Unknown
                        } else {
                            AgentSwitchStage::Failed
                        };
                        if !matches!(
                            current.stage,
                            AgentSwitchStage::Cancelled
                                | AgentSwitchStage::Completed
                                | AgentSwitchStage::Failed
                                | AgentSwitchStage::Unknown
                        ) {
                            prefs
                                .transition_agent_switch(
                                    &intent.request.operation_id,
                                    stage,
                                    Some(error.to_string()),
                                )
                                .map_err(switch_io)?;
                        }
                    }
                    Ok(())
                })
                .map_err(|error| error.to_string())?;
            }
        }
    }
    gwt::load_issue_monitor_prefs(&path)
        .map(|prefs| {
            prefs.agent_switch_snapshot(gwt_core::paths::project_scope_hash(root).as_str())
        })
        .map_err(|error| error.to_string())
}

fn switch_io(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

fn checkpoint_input_revision(
    stage: AgentSwitchStage,
    saved: Option<u64>,
    current: Option<u64>,
) -> io::Result<u64> {
    let current = current.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::WouldBlock,
            "Source input observation is unavailable",
        )
    })?;
    if stage == AgentSwitchStage::CheckpointVerified && saved != Some(current) {
        return Err(switch_io("New input invalidated the saved checkpoint. Current work is kept; request another switch."));
    }
    Ok(current)
}

fn switch_host_was_replaced(pid: u32, started_at: u64) -> bool {
    gwt::process::host_process_liveness(pid) == Some(false)
        || gwt::process::host_process_start_time(pid).is_some_and(|current| current != started_at)
}

fn recover_previous_host_intents(root: &Path) -> io::Result<()> {
    let path = gwt::issue_monitor_prefs_path_for_repo_path(root);
    let observed = gwt::load_issue_monitor_prefs(&path)?;
    let host = gwt::process::current_hostname();
    let abandoned: Vec<_> = observed
        .agent_switch_intents
        .values()
        .filter(|intent| {
            intent.source.assignment.host_id == host
                && !matches!(
                    intent.stage,
                    AgentSwitchStage::Completed
                        | AgentSwitchStage::Cancelled
                        | AgentSwitchStage::Failed
                        | AgentSwitchStage::Unknown
                )
                && intent.runtime_stop.as_ref().is_none_or(|proof| {
                    switch_host_was_replaced(proof.host_pid, proof.host_started_at)
                })
        })
        .cloned()
        .collect();
    if abandoned.is_empty() {
        return Ok(());
    }
    gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
        for old in &abandoned {
            if prefs.agent_switch_intents.get(&old.request.operation_id) != Some(old) {
                continue;
            }
            if matches!(
                old.stage,
                AgentSwitchStage::Requested
                    | AgentSwitchStage::Checkpointing
                    | AgentSwitchStage::CheckpointVerified
            ) {
                prefs
                    .cancel_agent_switch(&old.request.operation_id)
                    .map_err(switch_io)?;
            } else {
                prefs.transition_agent_switch(&old.request.operation_id, AgentSwitchStage::Unknown,
                    Some("Host restarted after stopping began. Open the saved work in Works; no stop is replayed.".into())).map_err(switch_io)?;
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Deduplicate work on the existing Host worker pool; this is not authority.
struct SwitchTickGuard(PathBuf);
static SWITCH_TICKS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
impl SwitchTickGuard {
    fn claim(root: &Path) -> Option<Self> {
        let mut active = SWITCH_TICKS.get_or_init(Mutex::default).lock().ok()?;
        active
            .insert(root.to_path_buf())
            .then(|| Self(root.to_path_buf()))
    }
}
impl Drop for SwitchTickGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = SWITCH_TICKS.get_or_init(Mutex::default).lock() {
            active.remove(&self.0);
        }
    }
}

fn checkpoint_prompt(intent: &AgentSwitchIntent) -> String {
    let operation = &intent.request.operation_id;
    let source = intent.source.assignment.issue_number.unwrap_or_default();
    let target = intent.request.target_issue_number;
    format!("GWT checkpoint request {operation}: save the current Issue #{source} before switching this member to Issue #{target}. Continue only the current owner in this process. Read issue.monitor.switch.read for this exact operation before acting. Save only while it is checkpointing; if absent, cancelled, failed or unknown, keep current and clear any declared wait without saving for this request. If already verified, do not save it again. Prepare a coherent checkpoint with your ordinary verification, commit and push workflow; never mark unrun checks PASS. First post a Board handoff for Issue #{source} containing the exact UUID token {operation} and the continuation; confirm its link in your canonical Work. Then record the final workspace.update with status=idle, summary containing the same UUID, nonempty progress_summary and next_action, so the saved Work remains Paused after Board reflection. Commit and push the source changes and immutable Work shards, leaving tracked/untracked state clean. Then call gwtd JSON operation issue.monitor.switch.ack with operation_id={operation}, head=<actual HEAD SHA>, work_id=<your canonical Work ID>, board_entry_id=<the handoff entry ID>. The Host independently verifies these records and the live upstream. Do not complete or block the unfinished execution, kill a process, or start Issue #{target} in this provider. Declare issue.monitor.wait with number={source} and resume_condition=switch cancelled or saved work resumed, then end the turn after ACK; keep the source unchanged. If a coherent checkpoint cannot be saved, report the reason and keep current.\r")
}

fn set_switch_reason(root: &Path, intent: &AgentSwitchIntent, reason: &str) -> io::Result<()> {
    gwt::try_mutate_issue_monitor_prefs(
        &gwt::issue_monitor_prefs_path_for_repo_path(root),
        |prefs| {
            if prefs.agent_switch_intents.get(&intent.request.operation_id) == Some(intent)
                && intent.reason.as_deref() != Some(reason)
            {
                prefs
                    .transition_agent_switch(
                        &intent.request.operation_id,
                        intent.stage,
                        Some(reason.into()),
                    )
                    .map_err(switch_io)?;
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn current_switch_runtime(
    root: &Path,
    intent: &AgentSwitchIntent,
    sessions: &Path,
) -> io::Result<bool> {
    let Some(expected) = intent.runtime_stop.as_ref() else {
        return Ok(false);
    };
    let observed = gwt::monitor_duplicate_runtime::capture_stop_request(
        root,
        expected.issue_number,
        &expected.window_id,
        Some(&expected.session_id),
        &expected.requested_at,
    )?;
    if observed.as_ref() != Some(expected) {
        return Ok(false);
    }
    let runtime = gwt_agent::SessionRuntimeState::load(&gwt_agent::runtime_state_path_for_pid(
        sessions,
        expected.host_pid,
        &expected.session_id,
    ))?;
    Ok(runtime.status == gwt_agent::AgentStatus::Idle
        && expected.matches_runtime(&expected.session_id, expected.host_pid, &runtime))
}

fn exact_writer_is_current(
    writers: &Arc<RwLock<HashMap<String, Arc<PtyWriterEntry>>>>,
    intent: &AgentSwitchIntent,
    entry: &Arc<PtyWriterEntry>,
) -> bool {
    writers.read().ok().is_some_and(|current| {
        current
            .get(&intent.source.window_id)
            .is_some_and(|current| Arc::ptr_eq(current, entry))
    })
}

fn checkpoint_saved(intent: &AgentSwitchIntent, sessions: &Path) -> io::Result<()> {
    let proof = intent
        .runtime_stop
        .as_ref()
        .ok_or_else(|| switch_io("Exact source runtime is unavailable"))?;
    let checkpoint = intent
        .checkpoint
        .as_ref()
        .ok_or_else(|| switch_io("Source checkpoint is absent"))?;
    let source = gwt_agent::Session::load(&sessions.join(format!("{}.toml", proof.session_id)))?;
    if gwt_agent::SessionExecutionIdentity::from_session(&source)
        .map_err(switch_io)?
        .as_ref()
        != Some(&proof.execution_identity)
    {
        return Err(switch_io(
            "Source execution identity changed after checkpoint",
        ));
    }
    let worktree = &proof.execution_identity.worktree_path;
    gwt::issue_monitor::verify_switch_git_checkpoint(worktree, &checkpoint.verified_head)?;
    gwt::issue_monitor::verify_switch_continuation(
        worktree,
        &source.id,
        &intent.request.operation_id,
        &checkpoint.work_id,
        &checkpoint.board_entry_id,
    )
}

fn prepare_switch_target(
    root: &Path,
    intent: &AgentSwitchIntent,
) -> io::Result<(
    gwt::IssueMonitorState,
    Vec<super::installation::IssueMonitorAdmissionFileStamp>,
)> {
    let pending = |error: String| io::Error::new(io::ErrorKind::WouldBlock, error);
    let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + std::time::Duration::from_secs(30),
    );
    let (owner, repo) = gwt::issue_monitor_worker::github_remote_owner_and_repo(root)
        .map_err(|error| pending(error.to_string()))?;
    let cache_root = gwt::issue_cache::issue_cache_root_for_repo_path(root)
        .unwrap_or_else(|| gwt::issue_cache::issue_cache_root_for_repo_slug(&owner, &repo));
    let number = intent.request.target_issue_number;
    let cached =
        gwt::issue_monitor_worker::load_cached_issue_monitor_candidate(&cache_root, number)
            .ok_or_else(|| {
                pending(
                    "Next issue metadata is unavailable. Current work is kept; refresh plans."
                        .into(),
                )
            })?;
    gwt::issue_monitor_worker::try_refresh_issue_monitor_candidate(root, &owner, &repo, &cached)
        .map_err(|error| {
            pending(format!(
                "Next issue could not be checked. Current work is kept: {error}"
            ))
        })?;
    let mut dependencies = Vec::new();
    let mut monitor =
        super::installation::prepare_issue_monitor_spawn_capacity(root, number, &mut dependencies)
            .map_err(pending)?;
    // Keep the candidate loaded with the admission stamps. A concurrent cache
    // refresh can supersede the earlier network response before hydration.
    let planned = monitor
        .inbox_item(number)
        .map(|item| item.issue.clone())
        .ok_or_else(|| {
            pending("Next issue could not be hydrated for admission. Refresh plans.".into())
        })?;
    let entry = gwt_github::Cache::new(cache_root)
        .load_entry(gwt_github::IssueNumber(number))
        .ok_or_else(|| {
            pending("Next issue metadata changed during inspection. Refresh plans.".into())
        })?;
    let claims = gwt_github::issue_auto_claim::extract_claim_comments(&entry.snapshot.comments);
    if let Some(claim) = gwt_github::issue_auto_claim::select_winning_claim(
        &claims,
        &chrono::Utc::now().to_rfc3339(),
    ) {
        monitor.record_blocked_by_claim(
            planned,
            claim.owner.clone(),
            claim.expires_at.clone(),
            Some(&claim.claim_id),
        );
    }
    let reasons = monitor.agent_switch_target_exclusions(intent);
    if !reasons.is_empty() {
        return Err(pending(format!(
            "Waiting for the next issue. Current work is kept: {}",
            reasons.join("; ")
        )));
    }
    if dependencies
        .iter()
        .any(|dependency| !dependency.is_current())
    {
        return Err(pending(
            "Next issue metadata changed during inspection. Current work is kept.".into(),
        ));
    }
    Ok((monitor, dependencies))
}

fn drive_switch(
    root: &Path,
    sessions: &Path,
    intent: &AgentSwitchIntent,
    entries: &[Arc<PtyWriterEntry>],
    writers: &Arc<RwLock<HashMap<String, Arc<PtyWriterEntry>>>>,
    retire: impl Fn(AgentSwitchSourceExited),
) -> io::Result<bool> {
    use AgentSwitchStage::*;
    let path = gwt::issue_monitor_prefs_path_for_repo_path(root);
    let host = gwt::process::current_hostname();
    let current = gwt::load_issue_monitor_prefs(&path)?;
    if current
        .agent_switch_intents
        .get(&intent.request.operation_id)
        != Some(intent)
    {
        return Ok(false);
    }
    if matches!(intent.stage, Requested | Checkpointing | CheckpointVerified)
        && (!current.agent_switch_source_unchanged(intent, &host)
            || !current.agent_switch_target_available(intent, &host))
    {
        gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
            if prefs.agent_switch_intents.get(&intent.request.operation_id) == Some(intent) {
                prefs.transition_agent_switch(&intent.request.operation_id, Failed,
                    Some("The current assignment or next plan changed. Current work is kept; refresh plans.".into())).map_err(switch_io)?;
            }
            Ok(())
        })?;
        return Ok(false);
    }
    if matches!(intent.stage, Requested | CheckpointVerified) {
        let Some(proof) = intent.runtime_stop.as_ref() else {
            return Ok(false);
        };
        let Some(entry) = entries.iter().find(|entry| {
            entry.monitor_runtime.as_ref().is_some_and(|registration| {
                registration.session_id == proof.session_id
                    && registration.incarnation == proof.runtime_incarnation
                    && registration.window_id == intent.source.window_id
                    && !registration.review_dispatch
            })
        }) else {
            set_switch_reason(
                root,
                intent,
                "Waiting for an exact local activity observation. Current work is kept.",
            )?;
            return Ok(false);
        };
        let revision = checkpoint_input_revision(
            intent.stage,
            intent.source_input_revision,
            entry.handle.ordinary_input_revision(),
        )?;
        if entry.handle.has_unsent_user_input()
            || !exact_writer_is_current(writers, intent, entry)
            || !current_switch_runtime(root, intent, sessions).unwrap_or(false)
        {
            set_switch_reason(
                root,
                intent,
                "Waiting for the current turn to finish. Current work is kept.",
            )?;
            return Ok(false);
        }
        if !gwt::issue_monitor::observe_agent_switch_checkpoint_cwd(
            &proof.execution_identity.worktree_path,
            proof.child_pid,
            proof.child_started_at,
        )
        .is_clear()
        {
            set_switch_reason(root, intent, "Waiting for background work and a complete activity observation. Current work is kept.")?;
            return Ok(false);
        }
        let (target_planning, target_dependencies) = prepare_switch_target(root, intent)?;
        let reservation = match Arc::clone(&entry.handle).reserve_input_transaction() {
            Ok(reservation) => reservation,
            Err(_) => {
                set_switch_reason(
                    root,
                    intent,
                    "Waiting for pending terminal input. Current work is kept.",
                )?;
                return Ok(false);
            }
        };
        if intent.stage == Requested {
            let delivered =
                gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                    sessions,
                    &proof.execution_identity,
                    || -> io::Result<()> {
                        if !current_switch_runtime(root, intent, sessions).unwrap_or(false)
                            || entry.handle.has_unsent_user_input()
                            || entry.handle.ordinary_input_revision() != Some(revision)
                        {
                            return Err(io::Error::new(io::ErrorKind::WouldBlock, "Waiting for the current turn and input to settle. Current work is kept."));
                        }
                        let (saved, _) = gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
                            if prefs.agent_switch_intents.get(&intent.request.operation_id)
                                != Some(intent)
                                || !prefs.agent_switch_source_unchanged(intent, &host)
                                || !prefs.agent_switch_target_available(intent, &host)
                            {
                                return Err(switch_io("Switch changed before checkpoint delivery"));
                            }
                            prefs.transition_agent_switch(&intent.request.operation_id, Checkpointing,
                            Some("Waiting for the source to acknowledge its saved checkpoint…".into())).map_err(switch_io)?;
                            prefs
                                .agent_switch_intents
                                .get_mut(&intent.request.operation_id)
                                .expect("checked switch")
                                .source_input_revision = Some(revision);
                            Ok(())
                        })?;
                        let expected = &saved.agent_switch_intents[&intent.request.operation_id];
                        let prompt = checkpoint_prompt(intent);
                        let mut body_committed = false;
                        // Once the full body is accepted, finish its one submit so
                        // cancellation cannot leave a hidden partial next prompt.
                        // The source must read the intent before saving, and a
                        // cancelled intent can never acknowledge or stop anything.
                        let _delivery = super::agent_dispatch::drive_verified_pane_submit(&prompt,
                        reservation.bracketed_paste_enabled(), 1, |bytes| {
                            let submitting = bytes == b"\r";
                            if submitting && body_committed {
                                // Membership and intent changes cannot strand
                                // this accepted body in the same composer. The
                                // exact owner/Session lease remains held, and a
                                // revoked/replaced writer still refuses input.
                                if !exact_writer_is_current(writers, intent, entry) {
                                    return Err("Source writer changed after checkpoint body delivery".into());
                                }
                                return reservation.write_input_authorized(bytes, |commit| commit(&mut || {}))
                                    .map_err(|error| error.to_string());
                            }
                            gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
                                if prefs.agent_switch_intents.get(&intent.request.operation_id) != Some(expected)
                                    || !prefs.agent_switch_source_unchanged(expected, &host)
                                    || !exact_writer_is_current(writers, intent, entry) {
                                    return Err(switch_io("Switch cancelled or source changed before checkpoint delivery"));
                                }
                                reservation.write_input_authorized(bytes, |commit| {
                                    reservation.with_unchanged_ordinary_input(revision, || commit(&mut || {}))
                                }).map_err(switch_io)?;
                                body_committed = true;
                                Ok(())
                            }).map(|_| ()).map_err(|error| error.to_string())
                        }, std::thread::sleep, || Ok(false)).map_err(switch_io)?;
                        Ok(())
                    },
                )?;
            delivered.ok_or_else(|| {
                switch_io("Source authority changed before checkpoint delivery")
            })??;
            return Ok(false);
        }
        // Hold protected input until the captured runtime has actually exited.
        let outcome =
            gwt::cli::execution_state::with_current_active_session_execution_identity_lease(
                sessions,
                &proof.execution_identity,
                || -> io::Result<()> {
                    checkpoint_saved(intent, sessions)?;
                    if target_dependencies
                        .iter()
                        .any(|dependency| !dependency.is_current())
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            "Next issue changed after inspection; current work is kept",
                        ));
                    }
                    if !exact_writer_is_current(writers, intent, entry)
                        || !current_switch_runtime(root, intent, sessions).unwrap_or(false)
                        || !gwt::issue_monitor::observe_agent_switch_checkpoint_cwd(
                            &proof.execution_identity.worktree_path,
                            proof.child_pid,
                            proof.child_started_at,
                        )
                        .is_clear()
                    {
                        return Err(io::Error::new(io::ErrorKind::WouldBlock, "Waiting for a complete idle activity observation; current work is kept"));
                    }
                    reservation.with_unchanged_ordinary_input(revision, || {
                    Ok(gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
                        if prefs.agent_switch_intents.get(&intent.request.operation_id) != Some(intent) {
                            return Err(switch_io("Switch changed before stop"));
                        }
                        if target_dependencies.iter().any(|dependency| !dependency.is_current()) {
                            return Err(io::Error::new(io::ErrorKind::WouldBlock,
                                "Next issue changed before stop; current work is kept"));
                        }
                        let mut target = target_planning.clone();
                        target.rebase_daemon_driver_prefs(prefs);
                        let reasons = target.agent_switch_target_exclusions(intent);
                        if !reasons.is_empty() {
                            return Err(io::Error::new(io::ErrorKind::WouldBlock, format!("Next issue is no longer ready; current work is kept: {}", reasons.join("; "))));
                        }
                        prefs.transition_agent_switch(&intent.request.operation_id, StoppingCurrent,
                            Some("Current work is saved. Waiting for the previous runtime to exit…".into())).map_err(switch_io)?;
                        let mut monitor = gwt::IssueMonitorState::with_prefs(gwt::IssueMonitorConfig::default(), prefs.clone());
                        if !monitor.terminal_queue_move(intent.request.target_issue_number, 0, &chrono::Utc::now().to_rfc3339()) {
                            return Err(switch_io("The target is no longer queued; current work is kept"));
                        }
                        let target = gwt::issue_monitor::IssueMonitorStopTarget { issue_number: proof.issue_number,
                            window_id: Some(intent.source.window_id.clone()), claim_id: Some(intent.source.claim_id.clone()),
                            delivery_id: intent.source.assignment.delivery_id.clone() };
                        if !matches!(monitor.stop_only_with_settlement(&target, "Saved checkpoint switch", &chrono::Utc::now().to_rfc3339(),
                            gwt::IssueMonitorExecutionSettlement::Active), gwt::IssueMonitorStopOutcome::Stopped { .. }) {
                            return Err(switch_io("Exact source could not be stopped"));
                        }
                        *prefs = monitor.prefs();
                        if !prefs.monitor_runtime_stop_requests.contains(proof) { prefs.monitor_runtime_stop_requests.push(proof.clone()); }
                        Ok(())
                    }).map(|_| entry.handle.revoke_input_generation()))
                }).map_err(switch_io)?
                },
            )?;
        outcome.ok_or_else(|| {
            switch_io("Source execution authority changed; no stop was committed")
        })??;
        let registration = entry
            .monitor_runtime
            .clone()
            .ok_or_else(|| switch_io("Captured runtime registration is absent"))?;
        gwt::monitor_duplicate_runtime::stop_requested_runtimes(
            root,
            &[gwt::monitor_duplicate_runtime::RegisteredMonitorRuntime {
                registration,
                handle: Arc::clone(&entry.handle),
            }],
        )?;
        let exited = gwt::load_issue_monitor_prefs(&path)?
            .agent_switch_intents
            .get(&intent.request.operation_id)
            .is_some_and(|saved| {
                saved.runtime_stop.as_ref() == Some(proof) && saved.runtime_exit_confirmed
            });
        if !exited || !entry.handle.tree_exit_confirmed() {
            return Err(switch_io("Current work is saved, but the previous runtime's exit is unconfirmed. Continue saved work to check its current state."));
        }
        drop(reservation);
        retire(AgentSwitchSourceExited {
            operation_id: intent.request.operation_id.clone(),
            proof: proof.clone(),
            pty: Arc::clone(&entry.handle),
        });
        return Ok(true);
    }
    if intent.stage == StoppingCurrent && intent.runtime_exit_confirmed {
        // Normal admission still chooses the provider and enforces all readiness/claim gates.
        let _ = gwt::agent_names::read_project_roster(root, sessions).map_err(switch_io)?;
        gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
            if prefs.agent_switch_intents.get(&intent.request.operation_id) != Some(intent) {
                return Ok(());
            }
            let mut monitor = gwt::IssueMonitorState::with_prefs(
                gwt::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            if !monitor.terminal_queue_move(
                intent.request.target_issue_number,
                0,
                &chrono::Utc::now().to_rfc3339(),
            ) {
                return Err(switch_io("Saved work is paused; the target is no longer queued. Open saved work in Works."));
            }
            *prefs = monitor.prefs();
            prefs
                .transition_agent_switch(
                    &intent.request.operation_id,
                    LaunchingNext,
                    Some("Launching the next planned issue through Issue Monitor…".into()),
                )
                .map_err(switch_io)?;
            Ok(())
        })?;
        return Ok(true);
    }
    if intent.stage == LaunchingNext {
        gwt::try_mutate_issue_monitor_prefs(&path, |prefs| {
            if prefs.agent_switch_intents.get(&intent.request.operation_id) == Some(intent) {
                let member = prefs.agent_roster.as_ref().and_then(|roster| {
                    roster
                        .members
                        .iter()
                        .find(|member| member.agent_member_id == intent.request.member_id)
                });
                let launched = member
                    .and_then(|member| member.assignment.as_ref())
                    .is_some_and(|assignment| {
                        assignment.issue_number == Some(intent.request.target_issue_number)
                            && assignment.session_id.is_some()
                            && assignment.session_id != intent.source.assignment.session_id
                    })
                    && prefs
                        .launch_confirmations
                        .get(&intent.request.target_issue_number)
                        .is_some_and(|confirmation| {
                            prefs.launch_bindings.get(&confirmation.window_id)
                                == Some(&intent.request.target_issue_number)
                                && prefs.launched_issues.iter().any(|launch| {
                                    launch.issue_number == intent.request.target_issue_number
                                        && launch.window_id == confirmation.window_id
                                })
                        });
                if launched {
                    prefs.transition_agent_switch(&intent.request.operation_id, Completed, Some("The same member is working on the next issue. Previous work is saved in Works.".into())).map_err(switch_io)?;
                } else if prefs
                    .failed_issues
                    .iter()
                    .any(|failure| failure.issue_number == intent.request.target_issue_number)
                {
                    prefs.transition_agent_switch(&intent.request.operation_id, Unknown, Some("The next issue could not start. Previous work is saved; resume it from Works.".into())).map_err(switch_io)?;
                }
            }
            Ok(())
        })?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exact_live_host_is_not_an_abandoned_switch_request() {
        let pid = std::process::id();
        let started = gwt::process::host_process_start_time(pid).expect("own process identity");
        assert!(!switch_host_was_replaced(pid, started));
        assert!(switch_host_was_replaced(pid, started.wrapping_add(1)));
    }

    #[test]
    fn checkpoint_rejects_a_user_submit_between_ack_and_stop() {
        assert!(
            checkpoint_input_revision(AgentSwitchStage::CheckpointVerified, Some(7), Some(8))
                .is_err()
        );
        assert!(
            checkpoint_input_revision(AgentSwitchStage::CheckpointVerified, None, Some(8)).is_err()
        );
        assert_eq!(
            checkpoint_input_revision(AgentSwitchStage::CheckpointVerified, Some(7), Some(7))
                .unwrap(),
            7
        );
        assert_eq!(
            checkpoint_input_revision(AgentSwitchStage::Requested, None, Some(7)).unwrap(),
            7
        );
    }
}

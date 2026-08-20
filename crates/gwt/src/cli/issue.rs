use std::{fs, io, path::PathBuf};

use gwt_github::{
    cache::write_atomic, client::ApiError, Cache, IssueClient, IssueNumber, IssueSnapshot,
    IssueState, SpecOpsError,
};

use crate::cli::{
    CliEnv, CliParseError, IssueCommand, IssueMonitorPriorityPosition, LinkedPrSummary,
};
use crate::issue_monitor::{
    transact_issue_monitor_prefs, IssueMonitorPmControlAction, IssueMonitorPmControlAdmission,
    IssueMonitorPmControlRefusal, IssueMonitorPmControlRequest, IssueMonitorPrefsMutation,
    IssueMonitorPrefsTransactionOutcome,
};

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

pub(super) fn parse(args: &[String]) -> Result<IssueCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("spec") => super::issue_spec::parse(it.collect::<Vec<_>>().as_slice()),
        Some("view") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "view"),
        Some("comments") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "comments"),
        Some("linked-prs") => {
            parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "linked-prs")
        }
        Some("create") => parse_issue_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("comment") => parse_issue_comment_args(it.collect::<Vec<_>>().as_slice()),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: IssueCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if matches!(
        cmd,
        IssueCommand::SpecReadAll { .. }
            | IssueCommand::SpecReadSection { .. }
            | IssueCommand::SpecEditSection { .. }
            | IssueCommand::SpecEditSectionBody { .. }
            | IssueCommand::SpecEditSectionJson { .. }
            | IssueCommand::SpecEditSectionJsonBody { .. }
            | IssueCommand::SpecList { .. }
            | IssueCommand::SpecCreate { .. }
            | IssueCommand::SpecCreateBody { .. }
            | IssueCommand::SpecCreateJson { .. }
            | IssueCommand::SpecCreateJsonBody { .. }
            | IssueCommand::SpecCreateHelp
            | IssueCommand::SpecPull { .. }
            | IssueCommand::SpecRepair { .. }
            | IssueCommand::SpecRename { .. }
    ) {
        return super::issue_spec::run(env, cmd, out);
    }

    let code = match cmd {
        IssueCommand::View { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue(out, &entry.snapshot);
            0
        }
        IssueCommand::Comments { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue_comments(out, &entry.snapshot);
            0
        }
        IssueCommand::LinkedPrs { number, refresh } => {
            let linked_prs = load_or_refresh_linked_prs(env, IssueNumber(number), refresh)?;
            render_linked_prs(out, &linked_prs);
            0
        }
        IssueCommand::Create {
            title,
            file,
            labels,
        } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.create",
                super::intake_outcome::IntakeOutcomeKind::IssueCreated,
                snapshot.number.0,
            );
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::CreateBody {
            title,
            body,
            labels,
        } => {
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.create",
                super::intake_outcome::IntakeOutcomeKind::IssueCreated,
                snapshot.number.0,
            );
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        IssueCommand::Comment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.comment",
                super::intake_outcome::IntakeOutcomeKind::IssueUpdated,
                number,
            );
            let _ = refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        IssueCommand::CommentBody { number, body } => {
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            super::intake_outcome::auto_record_issue_operation(
                env.repo_path(),
                "issue.comment",
                super::intake_outcome::IntakeOutcomeKind::IssueUpdated,
                number,
            );
            let _ = refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        IssueCommand::MonitorReviewVerdict {
            issue_number,
            reviewed_sha,
            verdict_raw,
        } => run_monitor_review_verdict(env, issue_number, &reviewed_sha, &verdict_raw, out),
        IssueCommand::MonitorStatus { project_root } => {
            run_monitor_status(env, project_root.as_deref(), out)?
        }
        IssueCommand::MonitorPriorityMove {
            project_root,
            number,
            position,
        } => run_monitor_priority_move(env, project_root.as_deref(), number, position, out)?,
        IssueCommand::MonitorPrioritySet {
            project_root,
            issue_numbers,
        } => run_monitor_priority_set(env, project_root.as_deref(), &issue_numbers, out)?,
        IssueCommand::MonitorLaunchNow {
            project_root,
            number,
        } => run_monitor_launch_now(env, project_root.as_deref(), number, out)?,
        IssueCommand::MonitorStop {
            project_root,
            number,
            operation_id,
            reason,
            launch_generation,
            claim_id,
            claim_owner,
            delivery_id,
            materializer_window_id,
            window_id,
        } => run_monitor_stop(
            env,
            project_root.as_deref(),
            number,
            operation_id.as_deref(),
            &reason,
            crate::IssueMonitorStopTarget {
                issue_number: number,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
            },
            out,
        )?,
        IssueCommand::MonitorFailover {
            project_root,
            number,
            operation_id,
            reason,
            launch_generation,
            claim_id,
            claim_owner,
            delivery_id,
            materializer_window_id,
            window_id,
        } => run_monitor_failover(
            env,
            project_root.as_deref(),
            number,
            operation_id.as_deref(),
            &reason,
            crate::IssueMonitorStopTarget {
                issue_number: number,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
            },
            out,
        )?,
        IssueCommand::MonitorRecover {
            project_root,
            number,
            operation_id,
            reason,
            launch_generation,
            claim_id,
            claim_owner,
            delivery_id,
            materializer_window_id,
            window_id,
        } => run_monitor_recover(
            env,
            project_root.as_deref(),
            number,
            &operation_id,
            &reason,
            crate::IssueMonitorStopTarget {
                issue_number: number,
                launch_generation: Some(launch_generation),
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
            },
            out,
        )?,
        IssueCommand::MonitorControlReconcile {
            project_root,
            operation_id,
            revoked_generation,
        } => run_monitor_control_reconcile(
            env,
            project_root.as_deref(),
            &operation_id,
            revoked_generation,
            out,
        )?,
        IssueCommand::MonitorRequeue {
            project_root,
            number,
            reason,
        } => run_monitor_requeue(env, project_root.as_deref(), number, &reason, out)?,
        IssueCommand::MonitorQuestions { project_root } => {
            run_monitor_questions(env, project_root.as_deref(), out)?
        }
        IssueCommand::MonitorQuestionAnswer {
            project_root,
            handoff_id,
            answer,
        } => run_monitor_question_answer(env, project_root.as_deref(), &handoff_id, &answer, out)?,
        IssueCommand::MonitorConfigSet {
            project_root,
            enabled,
            autonomous_mode,
            max_active,
        } => run_monitor_config_set(
            env,
            project_root.as_deref(),
            enabled,
            autonomous_mode,
            max_active,
            out,
        )?,
        _ => unreachable!("issue::run called with non-issue command"),
    };
    Ok(code)
}

fn issue_monitor_project_root<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
) -> Result<PathBuf, SpecOpsError> {
    let requested = project_root.unwrap_or_else(|| env.repo_path());
    let canonical = fs::canonicalize(requested).map_err(|error| {
        io_as_api_error(io::Error::new(
            error.kind(),
            format!(
                "Issue Monitor project_root {} is unavailable: {error}",
                requested.display()
            ),
        ))
    })?;
    if !canonical.is_dir() {
        return Err(io_as_api_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Issue Monitor project_root {} is not a directory",
                requested.display()
            ),
        )));
    }
    let resolved = gwt_core::paths::resolve_current_worktree_root(&canonical);
    // Issue #3606: this is the one place an `issue.monitor.*` operation turns a
    // caller-supplied `project_root` into a project store. Recording it here is
    // what lets the JSON envelope answer "which store did this land in", which
    // `ok: true` alone never did.
    gwt_core::paths::record_operation_project_store(&resolved);
    Ok(resolved)
}

/// Issue #3655 AC-4 / AC-9: fold Board escalations into `needs_human`.
///
/// The autonomous lifecycle only knows about the issues *it* parked, so an
/// agent that stopped because an operation refused it was invisible in the one
/// field a PM reads to find work needing a human. Merging here — after the
/// snapshot is obtained, not inside either branch — means the daemon
/// projection and the offline fallback cannot disagree, and it deliberately
/// reads a file rather than a pane, so it still answers while `pane.read` is
/// failing under GUI event-loop saturation (#3629).
fn merge_board_escalations_into_needs_human(
    project_root: &std::path::Path,
    status: &mut crate::IssueMonitorAgentStatus,
) {
    let escalated = match gwt_core::coordination::load_escalation_store(project_root) {
        Ok(store) => store.open_owner_issue_numbers(),
        Err(error) => {
            tracing::warn!(
                %error,
                "could not read the Board escalation index for issue.monitor.status"
            );
            return;
        }
    };
    for issue_number in escalated {
        if !status.needs_human.contains(&issue_number) {
            status.needs_human.push(issue_number);
        }
    }
    status.needs_human.sort_unstable();
}

fn daemon_issue_monitor_status_is_current(
    status: &crate::IssueMonitorAgentStatus,
    prefs: &crate::IssueMonitorPrefs,
) -> bool {
    status.control_state_revision >= prefs.control_state_revision
}

/// Join the exact owner-generation recovery preflight into the same status
/// response as the AppRuntime inventory. Runtime absence alone is not enough:
/// Recover is actionable only when the execution control plane can identify a
/// decisively unreachable current Active holder for that Issue.
fn apply_execution_recovery_readiness(
    project_root: &std::path::Path,
    status: &mut crate::IssueMonitorAgentStatus,
) {
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    for row in &mut status.inbox {
        if row.control_ready.recover.degraded_reason.as_deref() != Some("execution_unverified") {
            continue;
        }
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: row.issue_number,
        };
        row.control_ready.recover =
            match crate::cli::execution_state::unreachable_current_generation_holder(
                &sessions_dir,
                project_root,
                owner,
            ) {
                Ok(Some(_)) => crate::IssueMonitorControlActionReadiness::ready(),
                Ok(None) => crate::IssueMonitorControlActionReadiness::degraded(
                    "execution_target_unavailable",
                ),
                Err(error) => {
                    tracing::warn!(
                        issue = row.issue_number,
                        %error,
                        "could not prove Issue Monitor Recover execution target"
                    );
                    crate::IssueMonitorControlActionReadiness::degraded(
                        "execution_target_unavailable",
                    )
                }
            };
    }
}

fn run_monitor_status<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    #[cfg(unix)]
    let daemon_status = crate::daemon_publisher::read_issue_monitor_status(&project_root)
        .map_err(|error| io_as_api_error(io::Error::other(error.to_string())))?
        .map(|status| {
            serde_json::from_value::<crate::IssueMonitorAgentStatus>(status)
                .map_err(|error| io_as_api_error(io::Error::other(error)))
        })
        .transpose()?;
    #[cfg(not(unix))]
    let daemon_status: Option<crate::IssueMonitorAgentStatus> = None;

    let daemon_status =
        daemon_status.filter(|status| daemon_issue_monitor_status_is_current(status, &prefs));
    let mut status = if let Some(status) = daemon_status {
        status
    } else {
        // Issue #3633 AC-5: the only durable evidence of the real scan cadence.
        // Reaching this branch at all means no live daemon holds the projection.
        let persisted_last_scan_at = prefs.last_scan_at.clone();
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&project_root);
        let candidates =
            crate::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root)
                .map_err(|error| io_as_api_error(io::Error::other(error)))?;
        crate::scan_issue_monitor_candidates(&mut monitor, &candidates, &now);
        // Rebuilding the queue from the local Issue cache is a projection, not
        // a scan: preserve the persisted cadence evidence.
        monitor.restore_persisted_last_scan_at(persisted_last_scan_at);
        monitor.agent_status_at(&now)
    };

    let inventory = super::pane::request_issue_monitor_runtime_inventory(&project_root)
        .unwrap_or_else(|reason| crate::IssueMonitorRuntimeInventory::Unavailable {
            project_scope: gwt_core::paths::project_scope_hash(&project_root)
                .as_str()
                .to_string(),
            observed_at: now.clone(),
            reason,
        });
    status.apply_runtime_inventory(&inventory);
    apply_execution_recovery_readiness(&project_root, &mut status);
    merge_board_escalations_into_needs_human(&project_root, &mut status);
    out.push_str(
        &serde_json::to_string(&status)
            .map_err(|error| io_as_api_error(io::Error::other(error)))?,
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3478 (AC-9): the questions autonomous executions are parked on.
///
/// Reads the control plane directly rather than the daemon status projection:
/// this must answer "what is blocking the queue" even when no daemon is
/// running, which is exactly when a human is most likely to be looking.
fn run_monitor_questions<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let questions = prefs
        .autonomous_handoffs
        .iter()
        .filter(|handoff| handoff.is_open())
        .collect::<Vec<_>>();
    out.push_str(
        &serde_json::to_string(&serde_json::json!({ "questions": questions }))
            .map_err(|error| io_as_api_error(io::Error::other(error)))?,
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3478 (AC-5): register a human answer for one parked question.
///
/// Fail-closed on an unknown or already-answered handoff: reporting success for
/// an answer that reaches nobody would leave the Issue parked forever while the
/// human believes they unblocked it.
fn run_monitor_question_answer<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    handoff_id: &str,
    answer: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if answer.trim().is_empty() {
        return Err(io_as_api_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "answer must not be empty",
        )));
    }
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (_, issue_number) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut candidate = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        if !candidate.answer_autonomous_handoff(handoff_id, answer, &now) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no open Issue Monitor question with handoff_id {handoff_id}"),
            ));
        }
        let issue_number = candidate
            .autonomous_handoffs()
            .iter()
            .find(|handoff| handoff.handoff_id == handoff_id)
            .map(|handoff| handoff.issue_number);
        // Only the handoff queue moves here. The driver owns un-parking the
        // Issue and re-arming its launch, so an answer registered while the
        // daemon is down is applied on its next scan instead of being lost.
        prefs.autonomous_handoffs = candidate.prefs().autonomous_handoffs;
        Ok(issue_number)
    })
    .map_err(io_as_api_error)?;
    out.push_str(
        &serde_json::json!({
            "handoff_id": handoff_id,
            "issue_number": issue_number,
            "answered_at": now,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

fn run_monitor_priority_move<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    position: IssueMonitorPriorityPosition,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.priority_order.retain(|existing| *existing != number);
        let index = match position {
            IssueMonitorPriorityPosition::Head => 0,
            IssueMonitorPriorityPosition::Index(index) => index,
        };
        if index > prefs.priority_order.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "position {index} is outside priority_order length {}",
                    prefs.priority_order.len()
                ),
            ));
        }
        prefs.priority_order.insert(index, number);
        Ok(())
    })
    .map_err(io_as_api_error)?;
    out.push_str(&serde_json::json!({"priority_order": prefs.priority_order}).to_string());
    out.push('\n');
    Ok(0)
}

fn run_monitor_priority_set<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    issue_numbers: &[u64],
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.priority_order = issue_numbers.to_vec();
        Ok(())
    })
    .map_err(io_as_api_error)?;
    out.push_str(&serde_json::json!({"priority_order": prefs.priority_order}).to_string());
    out.push('\n');
    Ok(0)
}

/// SPEC-3431 FR-006: the PM's launch instruction. It does exactly two things —
/// move the issue to the head of `priority_order` (prefs is the SOT the scan
/// driver re-reads) and ask the current platform authority for one immediate scan. The launch
/// itself stays on the Monitor's claim/slot path, so this cannot produce a
/// duplicate agent. Priority persistence and scan delivery are reported
/// separately; no unacknowledged scheduler is presented as future delivery.
fn run_monitor_launch_now<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let (prefs, hold_cleared) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.priority_order.retain(|existing| *existing != number);
        prefs.priority_order.insert(0, number);
        // Issue #3616 AC-5: priority alone cannot beat `retry_ready`. A
        // provider reset can be days out, so leaving the hold in place would
        // accept this instruction and then ignore it for the whole window.
        // Dropping the hold here is what makes "switch provider and run it
        // now" an actual recovery instead of a no-op.
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let hold_cleared = monitor.clear_retry_hold(number);
        if hold_cleared {
            prefs.autonomous_records = monitor.prefs().autonomous_records;
        }
        Ok(hold_cleared)
    })
    .map_err(io_as_api_error)?;

    let delivery = issue_monitor_scan_delivery(request_immediate_monitor_scan(&project_root));

    out.push_str(
        &serde_json::json!({
            "number": number,
            "priority_order": prefs.priority_order,
            "priority_updated": true,
            "hold_cleared": hold_cleared,
            "scan_requested": delivery.scan_requested,
            "scan_delivery": delivery.scan_delivery,
            "scan_error": delivery.scan_error,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(if delivery.scan_requested { 0 } else { 1 })
}

#[cfg(test)]
type IssueMonitorControlLeaseTestHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
fn issue_monitor_control_lease_test_hook(
) -> &'static std::sync::Mutex<Option<IssueMonitorControlLeaseTestHook>> {
    static HOOK: std::sync::OnceLock<std::sync::Mutex<Option<IssueMonitorControlLeaseTestHook>>> =
        std::sync::OnceLock::new();
    HOOK.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
struct IssueMonitorControlLeaseTestHookGuard;

#[cfg(test)]
impl Drop for IssueMonitorControlLeaseTestHookGuard {
    fn drop(&mut self) {
        *issue_monitor_control_lease_test_hook()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
fn install_issue_monitor_control_lease_test_hook(
    hook: impl FnOnce() + Send + 'static,
) -> IssueMonitorControlLeaseTestHookGuard {
    let mut slot = issue_monitor_control_lease_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        slot.is_none(),
        "Issue Monitor control lease hook already installed"
    );
    *slot = Some(Box::new(hook));
    IssueMonitorControlLeaseTestHookGuard
}

#[cfg(test)]
fn run_issue_monitor_control_lease_test_hook() {
    let hook = issue_monitor_control_lease_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(not(test))]
fn run_issue_monitor_control_lease_test_hook() {}

#[cfg(test)]
fn issue_monitor_control_post_admission_test_hook(
) -> &'static std::sync::Mutex<Option<IssueMonitorControlLeaseTestHook>> {
    static HOOK: std::sync::OnceLock<std::sync::Mutex<Option<IssueMonitorControlLeaseTestHook>>> =
        std::sync::OnceLock::new();
    HOOK.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
struct IssueMonitorControlPostAdmissionTestHookGuard;

#[cfg(test)]
impl Drop for IssueMonitorControlPostAdmissionTestHookGuard {
    fn drop(&mut self) {
        *issue_monitor_control_post_admission_test_hook()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
fn install_issue_monitor_control_post_admission_test_hook(
    hook: impl FnOnce() + Send + 'static,
) -> IssueMonitorControlPostAdmissionTestHookGuard {
    let mut slot = issue_monitor_control_post_admission_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(slot.is_none(), "post-admission hook already installed");
    *slot = Some(Box::new(hook));
    IssueMonitorControlPostAdmissionTestHookGuard
}

#[cfg(test)]
fn run_issue_monitor_control_post_admission_test_hook() {
    let hook = issue_monitor_control_post_admission_test_hook()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(not(test))]
fn run_issue_monitor_control_post_admission_test_hook() {}

#[cfg(test)]
fn issue_monitor_control_runtime_inventory_overrides() -> &'static std::sync::Mutex<
    std::collections::BTreeMap<std::path::PathBuf, crate::IssueMonitorRuntimeInventory>,
> {
    static OVERRIDES: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::BTreeMap<std::path::PathBuf, crate::IssueMonitorRuntimeInventory>,
        >,
    > = std::sync::OnceLock::new();
    OVERRIDES.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

#[cfg(test)]
fn set_issue_monitor_control_runtime_inventory(
    project_root: &std::path::Path,
    inventory: crate::IssueMonitorRuntimeInventory,
) {
    let project_root =
        std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    issue_monitor_control_runtime_inventory_overrides()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(project_root, inventory);
}

#[cfg(test)]
fn test_issue_monitor_control_runtime_inventory(
    project_root: &std::path::Path,
) -> Option<crate::IssueMonitorRuntimeInventory> {
    let project_root =
        std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());
    issue_monitor_control_runtime_inventory_overrides()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&project_root)
        .cloned()
}

#[cfg(not(test))]
fn test_issue_monitor_control_runtime_inventory(
    _project_root: &std::path::Path,
) -> Option<crate::IssueMonitorRuntimeInventory> {
    None
}

/// SPEC-3431 FR-033 / T-087b: revoke one launch's authority and slot.
///
/// The stop is committed to prefs inside the same lock-protected mutation that
/// evaluated the identity, so a scan running concurrently cannot observe a
/// half-applied stop.
///
/// This operation does not tear the pane down, and deliberately so. Once the
/// launch is revoked and held, `pane.close` on the returned window is inert —
/// [`crate::IssueMonitorState::requeue_window_at`] finds no launch to requeue
/// and the issue is terminal — so the safe teardown the PM already has under
/// FR-066 composes with this stop instead of needing a second, redundant
/// daemon→GUI channel that could disagree with it.
fn run_monitor_stop<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    operation_id: Option<&str>,
    reason: &str,
    target: crate::IssueMonitorStopTarget,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_monitor_pm_control(
        env,
        project_root,
        MonitorPmControlCommand {
            number,
            operation_id,
            reason,
            target,
            action: IssueMonitorPmControlAction::Stop,
        },
        out,
    )
}

/// SPEC-3431 FR-029〜031 / T-081: revoke one launch and requeue its issue for
/// the currently saved launch profile.
///
/// Switching provider is therefore two steps the PM already owns: edit the
/// launch profile, then call this. The relaunch itself still goes through the
/// ordinary claim/slot path — this operation never spawns an agent directly,
/// so `max_active` and the claim gate keep meaning what they meant.
///
/// An immediate scan is requested afterwards so the requeue takes effect now
/// rather than at the next interval tick; that is the whole point of asking for
/// a failover instead of just waiting.
fn run_monitor_failover<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    operation_id: Option<&str>,
    reason: &str,
    target: crate::IssueMonitorStopTarget,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_monitor_pm_control(
        env,
        project_root,
        MonitorPmControlCommand {
            number,
            operation_id,
            reason,
            target,
            action: IssueMonitorPmControlAction::Failover,
        },
        out,
    )
}

struct MonitorPmControlCommand<'a> {
    number: u64,
    operation_id: Option<&'a str>,
    reason: &'a str,
    target: crate::IssueMonitorStopTarget,
    action: IssueMonitorPmControlAction,
}

#[derive(Debug)]
enum MonitorPmControlDecision {
    Admission(IssueMonitorPmControlAdmission),
    RuntimeNotTerminal(&'static str),
}

fn recover_runtime_inventory_refusal(
    inventory: &crate::IssueMonitorRuntimeInventory,
    target: &crate::IssueMonitorStopTarget,
) -> Option<&'static str> {
    let crate::IssueMonitorRuntimeInventory::Available { windows, .. } = inventory else {
        return Some("runtime_inventory_unavailable");
    };
    let Some(window_id) = target
        .window_id
        .as_deref()
        .or(target.materializer_window_id.as_deref())
    else {
        return Some("runtime_ambiguous");
    };
    let matches = windows
        .iter()
        .filter(|window| window.window_id == window_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => None,
        [window]
            if matches!(
                window.pane_state,
                crate::IssueMonitorPaneState::Stopped | crate::IssueMonitorPaneState::Error
            ) =>
        {
            None
        }
        [_] => Some("runtime_live"),
        _ => Some("runtime_ambiguous"),
    }
}

fn monitor_pm_control_refusal_label(
    refusal: &IssueMonitorPmControlRefusal,
) -> (&'static str, &'static str) {
    match refusal {
        IssueMonitorPmControlRefusal::TargetMismatch { mismatch } => {
            ("mismatch", issue_monitor_stop_mismatch_label(*mismatch))
        }
        IssueMonitorPmControlRefusal::OperationIdConflict => ("refusal", "operation_id_conflict"),
        IssueMonitorPmControlRefusal::Capacity => ("refusal", "receipt_capacity"),
        IssueMonitorPmControlRefusal::AuthorityExhausted => ("refusal", "authority_exhausted"),
        IssueMonitorPmControlRefusal::InvalidOperationId => ("refusal", "invalid_operation_id"),
        IssueMonitorPmControlRefusal::InvalidReason => ("refusal", "invalid_reason"),
        IssueMonitorPmControlRefusal::SourceProfileUnavailable => {
            ("refusal", "source_profile_unavailable")
        }
        IssueMonitorPmControlRefusal::ExecutionTargetUnavailable => {
            ("refusal", "execution_target_unavailable")
        }
        IssueMonitorPmControlRefusal::IssueAlreadyPending => ("refusal", "issue_control_pending"),
    }
}

fn write_monitor_pm_control_refusal(
    out: &mut String,
    operation_id: &str,
    number: u64,
    key: &str,
    value: &str,
) {
    let mut diagnostic = serde_json::Map::new();
    diagnostic.insert("status".to_string(), serde_json::json!("refused"));
    diagnostic.insert("operation_id".to_string(), serde_json::json!(operation_id));
    diagnostic.insert("number".to_string(), serde_json::json!(number));
    diagnostic.insert(key.to_string(), serde_json::json!(value));
    out.push_str(&serde_json::Value::Object(diagnostic).to_string());
    out.push('\n');
}

struct AuthorizedMonitorPmControlResult {
    decision: MonitorPmControlDecision,
    persisted: crate::IssueMonitorPrefs,
    outcome_unknown: Option<String>,
    execution_settlement: Option<serde_json::Value>,
    execution_failure: Option<String>,
    should_scan: bool,
}

#[allow(clippy::too_many_arguments)]
fn converge_monitor_recover_execution(
    project_root: &std::path::Path,
    prefs_path: &std::path::Path,
    sessions_dir: &std::path::Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    operation_id: &str,
    reason: &str,
    now: &str,
    target: &crate::IssueMonitorPmControlExecutionSettlement,
    proof: Option<(
        gwt_agent::SessionExecutionIdentity,
        gwt_agent::ManualLaunchRuntimeEvidence,
    )>,
) -> io::Result<(serde_json::Value, IssueMonitorPrefsTransactionOutcome<bool>)> {
    let settled = if let Some((identity, runtime)) = proof {
        if identity.execution_binding.identity.generation_id != target.generation_id
            || identity.session_id != target.session_id
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "execution proof no longer matches the admitted Recover target",
            ));
        }
        serde_json::to_value(
            crate::cli::execution_state::settle_exact_terminal_active_generation(
                project_root,
                owner,
                &target.settlement_operation_id,
                sessions_dir,
                &identity,
                runtime,
                reason,
            )?,
        )
        .unwrap_or(serde_json::Value::Null)
    } else {
        let session_path = sessions_dir.join(format!("{}.toml", target.session_id));
        let replay_identity = match gwt_agent::inspect_session_path(&session_path) {
            gwt_agent::SessionPathState::Present(session) => {
                gwt_agent::SessionExecutionIdentity::from_session(&session)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
                    .filter(|identity| {
                        identity.session_id == target.session_id
                            && identity.execution_binding.identity.generation_id
                                == target.generation_id
                    })
            }
            gwt_agent::SessionPathState::Missing => None,
            gwt_agent::SessionPathState::Error(error) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("cannot read Recover source Session: {error}"),
                ));
            }
        };
        if let Some(identity) = replay_identity {
            serde_json::to_value(
                crate::cli::execution_state::settle_exact_terminal_active_generation(
                    project_root,
                    owner,
                    &target.settlement_operation_id,
                    sessions_dir,
                    &identity,
                    gwt_agent::ManualLaunchRuntimeEvidence::Absent,
                    reason,
                )?,
            )
            .unwrap_or(serde_json::Value::Null)
        } else if crate::cli::execution_state::exact_terminal_settlement_receipt_committed(
            project_root,
            owner,
            &target.settlement_operation_id,
            &target.generation_id,
            &target.session_id,
            reason,
        )? {
            serde_json::json!({
                "outcome": "replayed_from_ledger",
                "operation_id": target.settlement_operation_id,
                "generation_id": target.generation_id,
                "session_id": target.session_id,
            })
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "exact execution settlement receipt is not committed",
            ));
        }
    };

    let transaction = transact_issue_monitor_prefs(prefs_path, operation_id, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let already_settled = monitor.pending_controls().iter().any(|pending| {
            pending.operation_id == operation_id
                && pending
                    .execution_settlement
                    .as_ref()
                    .is_some_and(|execution| {
                        execution.settlement_operation_id == target.settlement_operation_id
                            && execution.generation_id == target.generation_id
                            && execution.session_id == target.session_id
                            && execution.settled
                    })
        });
        if already_settled {
            return Ok(IssueMonitorPrefsMutation::NoWrite(false));
        }
        let settlement = monitor.settle_pm_control_execution(
            operation_id,
            &target.settlement_operation_id,
            &target.generation_id,
            &target.session_id,
            now,
        );
        let should_scan = match settlement {
            crate::IssueMonitorPmControlSettlement::RestartQueued { should_scan, .. } => {
                should_scan
            }
            crate::IssueMonitorPmControlSettlement::PrerequisiteSettled { .. } => false,
            crate::IssueMonitorPmControlSettlement::Unrelated
            | crate::IssueMonitorPmControlSettlement::AuthorityExhausted
            | crate::IssueMonitorPmControlSettlement::InventoryFenceEstablished { .. } => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Recover execution prerequisite no longer matches durable control state",
                ));
            }
        };
        *prefs = monitor.prefs();
        Ok(IssueMonitorPrefsMutation::Commit(should_scan))
    })?;
    Ok((settled, transaction))
}

fn run_monitor_recover<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    operation_id: &str,
    reason: &str,
    target: crate::IssueMonitorStopTarget,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    run_monitor_pm_control(
        env,
        project_root,
        MonitorPmControlCommand {
            number,
            operation_id: Some(operation_id),
            reason,
            target,
            action: IssueMonitorPmControlAction::Recover,
        },
        out,
    )
}

fn run_monitor_pm_control<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    command: MonitorPmControlCommand<'_>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let MonitorPmControlCommand {
        number,
        operation_id,
        reason,
        target,
        action,
    } = command;
    let project_root = issue_monitor_project_root(env, project_root)?;
    let operation_id = operation_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("legacy-pm-control-{}", uuid::Uuid::new_v4().simple()));
    let ambient_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let project_key = gwt_core::paths::project_scope_hash(&project_root)
        .as_str()
        .to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // Capture the AppRuntime clock before control admission. A later lost-close
    // reconciliation is ordered only against this process-incarnation fence,
    // never against the unrelated durable Monitor revision.
    let runtime_inventory = test_issue_monitor_control_runtime_inventory(&project_root)
        .unwrap_or_else(|| {
            super::pane::request_issue_monitor_runtime_inventory(&project_root).unwrap_or_else(
                |reason| crate::IssueMonitorRuntimeInventory::Unavailable {
                    project_scope: project_key.clone(),
                    observed_at: now.clone(),
                    reason,
                },
            )
        });
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
        number,
    };
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let mut recovery_proof = None;

    let authorized =
        crate::pm_registry::with_registered_pm_authority(
            &pm_prefs_path,
            &ambient_session,
            |principal| {
                let transaction =
                    transact_issue_monitor_prefs(&prefs_path, &operation_id, |prefs| {
                        run_issue_monitor_control_lease_test_hook();
                        let mut monitor = crate::IssueMonitorState::with_prefs(
                            crate::IssueMonitorConfig::default(),
                            prefs.clone(),
                        );
                        if action == IssueMonitorPmControlAction::Recover
                            && monitor.pm_control_receipt(&operation_id).is_none()
                        {
                            if let Some(reason) =
                                recover_runtime_inventory_refusal(&runtime_inventory, &target)
                            {
                                return Ok(IssueMonitorPrefsMutation::NoWrite(
                                    MonitorPmControlDecision::RuntimeNotTerminal(reason),
                                ));
                            }
                            recovery_proof =
                                crate::cli::execution_state::unreachable_current_generation_holder(
                                    &sessions_dir,
                                    &project_root,
                                    owner,
                                )?;
                            if recovery_proof.is_none() {
                                return Ok(IssueMonitorPrefsMutation::NoWrite(
                                    MonitorPmControlDecision::RuntimeNotTerminal(
                                        "execution_target_unavailable",
                                    ),
                                ));
                            }
                        }
                        let execution_target = recovery_proof.as_ref().map(|(identity, _)| {
                            crate::issue_monitor::IssueMonitorPmControlExecutionTarget {
                                generation_id: identity
                                    .execution_binding
                                    .identity
                                    .generation_id
                                    .clone(),
                                session_id: identity.session_id.clone(),
                                settlement_operation_id: format!(
                                    "{operation_id}:execution-settlement"
                                ),
                            }
                        });
                        let admission = monitor.admit_pm_control(IssueMonitorPmControlRequest {
                            project_key: &project_key,
                            principal,
                            operation_id: &operation_id,
                            action,
                            target: &target,
                            reason,
                            now: &now,
                            runtime_inventory: Some(&runtime_inventory),
                            execution_target: execution_target.as_ref(),
                        });
                        let mutation =
                            if matches!(admission, IssueMonitorPmControlAdmission::Admitted(_)) {
                                *prefs = monitor.prefs();
                                IssueMonitorPrefsMutation::Commit(
                                    MonitorPmControlDecision::Admission(admission),
                                )
                            } else {
                                IssueMonitorPrefsMutation::NoWrite(
                                    MonitorPmControlDecision::Admission(admission),
                                )
                            };
                        Ok(mutation)
                    })?;
                let (decision, outcome_unknown, persisted) = match transaction {
                    IssueMonitorPrefsTransactionOutcome::Committed { prefs, value }
                    | IssueMonitorPrefsTransactionOutcome::NoWrite { prefs, value } => {
                        (value, None, prefs)
                    }
                    IssueMonitorPrefsTransactionOutcome::OutcomeUnknown {
                        candidate,
                        value,
                        error,
                        ..
                    } => (value, Some(error), candidate),
                };
                let mut result = AuthorizedMonitorPmControlResult {
                    decision,
                    persisted,
                    outcome_unknown,
                    execution_settlement: None,
                    execution_failure: None,
                    should_scan: false,
                };
                run_issue_monitor_control_post_admission_test_hook();
                if result.outcome_unknown.is_some()
                    || action != IssueMonitorPmControlAction::Recover
                {
                    return Ok(result);
                }
                let receipt = match &result.decision {
                    MonitorPmControlDecision::Admission(
                        IssueMonitorPmControlAdmission::Admitted(receipt)
                        | IssueMonitorPmControlAdmission::Replay(receipt),
                    ) => receipt,
                    MonitorPmControlDecision::RuntimeNotTerminal(_)
                    | MonitorPmControlDecision::Admission(
                        IssueMonitorPmControlAdmission::Refused(_),
                    ) => return Ok(result),
                };
                let execution_target = result
                    .persisted
                    .pending_controls
                    .iter()
                    .find(|pending| pending.operation_id == operation_id)
                    .and_then(|pending| pending.execution_settlement.clone());
                let Some(target) = execution_target else {
                    if receipt.outcome != crate::IssueMonitorPmControlOutcome::RecoveryQueued {
                        result.execution_failure = Some(
                            "recover receipt has no durable execution prerequisite".to_string(),
                        );
                    }
                    return Ok(result);
                };
                if target.settled {
                    result.execution_settlement = Some(serde_json::json!({
                        "outcome": "already_settled",
                        "operation_id": target.settlement_operation_id,
                        "generation_id": target.generation_id,
                        "session_id": target.session_id,
                    }));
                    return Ok(result);
                }
                match converge_monitor_recover_execution(
                    &project_root,
                    &prefs_path,
                    &sessions_dir,
                    owner,
                    &operation_id,
                    reason,
                    &now,
                    &target,
                    recovery_proof.take(),
                ) {
                    Ok((settlement, transaction)) => {
                        result.execution_settlement = Some(settlement);
                        match transaction {
                            IssueMonitorPrefsTransactionOutcome::Committed { prefs, value }
                            | IssueMonitorPrefsTransactionOutcome::NoWrite { prefs, value } => {
                                result.persisted = prefs;
                                result.should_scan = value;
                            }
                            IssueMonitorPrefsTransactionOutcome::OutcomeUnknown {
                                candidate,
                                error,
                                ..
                            } => {
                                result.persisted = candidate;
                                result.outcome_unknown = Some(error);
                            }
                        }
                    }
                    Err(error) => result.execution_failure = Some(error.to_string()),
                }
                Ok(result)
            },
        )
        .map_err(io_as_api_error)?;

    let Some(authorized) = authorized else {
        write_monitor_pm_control_refusal(out, &operation_id, number, "refusal", "pm_authority");
        return Ok(1);
    };
    let diagnostic_number = authorized
        .persisted
        .pm_control_receipts
        .iter()
        .find(|receipt| receipt.operation_id == operation_id)
        .map_or(number, |receipt| receipt.issue_number);
    let admission = match authorized.decision {
        MonitorPmControlDecision::RuntimeNotTerminal(reason) => {
            write_monitor_pm_control_refusal(
                out,
                &operation_id,
                diagnostic_number,
                "refusal",
                reason,
            );
            return Ok(1);
        }
        MonitorPmControlDecision::Admission(admission) => admission,
    };
    let receipt = match admission {
        IssueMonitorPmControlAdmission::Refused(refusal) => {
            let (key, value) = monitor_pm_control_refusal_label(&refusal);
            write_monitor_pm_control_refusal(out, &operation_id, diagnostic_number, key, value);
            return Ok(1);
        }
        IssueMonitorPmControlAdmission::Admitted(receipt)
        | IssueMonitorPmControlAdmission::Replay(receipt) => receipt,
    };

    if let Some(error) = authorized.outcome_unknown {
        out.push_str(
            &serde_json::json!({
                "number": receipt.issue_number,
                "status": "outcome_unknown",
                "operation_id": operation_id,
                "receipt": receipt,
                "error": error,
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    }
    if let Some(error) = authorized.execution_failure {
        out.push_str(
            &serde_json::json!({
                "number": receipt.issue_number,
                "status": "execution_settlement_failed",
                "operation_id": operation_id,
                "receipt": receipt,
                "error": error,
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    }
    if authorized.should_scan {
        let _ = request_immediate_monitor_scan(&project_root);
    }
    let receipt = authorized
        .persisted
        .pm_control_receipts
        .iter()
        .find(|candidate| candidate.operation_id == operation_id)
        .cloned()
        .unwrap_or(receipt);
    out.push_str(
        &serde_json::json!({
            "number": receipt.issue_number,
            "status": receipt.outcome,
            "receipt": receipt,
            "execution_settlement": authorized.execution_settlement,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

fn run_monitor_control_reconcile<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    operation_id: &str,
    revoked_generation: u64,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let ambient_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let inventory = super::pane::request_issue_monitor_runtime_inventory(&project_root)
        .unwrap_or_else(|reason| crate::IssueMonitorRuntimeInventory::Unavailable {
            project_scope: gwt_core::paths::project_scope_hash(&project_root)
                .as_str()
                .to_string(),
            observed_at: now.clone(),
            reason,
        });
    let authorized =
        crate::pm_registry::with_registered_pm_authority(&pm_prefs_path, &ambient_session, |_| {
            transact_issue_monitor_prefs(&prefs_path, operation_id, |prefs| {
                run_issue_monitor_control_lease_test_hook();
                let mut monitor = crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(),
                    prefs.clone(),
                );
                let Some(pending) = monitor
                    .pending_controls()
                    .iter()
                    .find(|pending| pending.operation_id == operation_id)
                else {
                    return Ok(IssueMonitorPrefsMutation::NoWrite(Err(
                        "operation_not_pending",
                    )));
                };
                if pending.source_identity.launch_generation != revoked_generation {
                    return Ok(IssueMonitorPrefsMutation::NoWrite(Err(
                        "generation_mismatch",
                    )));
                }
                let settlement =
                    monitor.settle_pm_control_runtime_absence(operation_id, &inventory, &now);
                match settlement {
                    crate::IssueMonitorPmControlSettlement::Unrelated => Ok(
                        IssueMonitorPrefsMutation::NoWrite(Err("absence_not_proven")),
                    ),
                    crate::IssueMonitorPmControlSettlement::AuthorityExhausted => Ok(
                        IssueMonitorPrefsMutation::NoWrite(Err("authority_exhausted")),
                    ),
                    settlement @ (crate::IssueMonitorPmControlSettlement::InventoryFenceEstablished {
                        ..
                    }
                    | crate::IssueMonitorPmControlSettlement::PrerequisiteSettled { .. }
                    | crate::IssueMonitorPmControlSettlement::RestartQueued {
                        ..
                    }) => {
                        *prefs = monitor.prefs();
                        Ok(IssueMonitorPrefsMutation::Commit(Ok(settlement)))
                    }
                }
            })
        })
        .map_err(io_as_api_error)?;
    let Some(transaction) = authorized else {
        out.push_str(
            &serde_json::json!({
                "status": "refused",
                "operation_id": operation_id,
                "refusal": "pm_authority",
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    };
    let (decision, outcome_unknown) = match transaction {
        IssueMonitorPrefsTransactionOutcome::Committed { value, .. }
        | IssueMonitorPrefsTransactionOutcome::NoWrite { value, .. } => (value, None),
        IssueMonitorPrefsTransactionOutcome::OutcomeUnknown { value, error, .. } => {
            (value, Some(error))
        }
    };
    if let Some(error) = outcome_unknown {
        out.push_str(
            &serde_json::json!({
                "status": "outcome_unknown",
                "operation_id": operation_id,
                "error": error,
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    }
    let settlement = match decision {
        Err(refusal) => {
            out.push_str(
                &serde_json::json!({
                    "status": "refused",
                    "operation_id": operation_id,
                    "refusal": refusal,
                })
                .to_string(),
            );
            out.push('\n');
            return Ok(1);
        }
        Ok(settlement) => settlement,
    };
    let (status, release_prepared, should_scan) = match settlement {
        crate::IssueMonitorPmControlSettlement::InventoryFenceEstablished { .. } => {
            ("inventory_fence_established", false, false)
        }
        crate::IssueMonitorPmControlSettlement::PrerequisiteSettled { release_effect, .. } => {
            ("prerequisite_settled", release_effect.is_some(), false)
        }
        crate::IssueMonitorPmControlSettlement::RestartQueued { should_scan, .. } => {
            ("restart_queued", false, should_scan)
        }
        crate::IssueMonitorPmControlSettlement::Unrelated
        | crate::IssueMonitorPmControlSettlement::AuthorityExhausted => unreachable!(
            "unrelated and authority-exhausted reconciliation are classified as refusals"
        ),
    };
    let delivery = should_scan
        .then(|| issue_monitor_scan_delivery(request_immediate_monitor_scan(&project_root)));
    out.push_str(
        &serde_json::json!({
            "status": status,
            "operation_id": operation_id,
            "revoked_generation": revoked_generation,
            "release_prepared": release_prepared,
            "scan_requested": delivery.as_ref().is_some_and(|delivery| delivery.scan_requested),
            "scan_delivery": delivery.as_ref().map(|delivery| &delivery.scan_delivery),
            "scan_error": delivery.as_ref().and_then(|delivery| delivery.scan_error.as_deref()),
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3645 AC-1 / #3628 AC-1〜AC-3: release the failure hold on one issue.
///
/// Deliberately not a relaxation of [`run_monitor_failover`]'s gate. That gate
/// resolves an exact live launch, which is the correct contract for handing
/// running work to another provider and an impossible one for a row whose
/// launch is already gone — the state this operation exists for. Keeping them
/// separate means the failover can never be talked into killing a running agent
/// by an operator who meant "recover the dead row".
///
/// An immediate scan is requested afterwards so the recovered issue re-enters
/// the claim path now rather than at the next interval tick.
fn run_monitor_requeue<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (prefs, outcome) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let outcome = monitor.requeue_failed_issue(number, reason, &now);
        if matches!(outcome, crate::IssueMonitorRequeueOutcome::Requeued { .. }) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .map_err(io_as_api_error)?;

    let stale_window_id = match outcome {
        crate::IssueMonitorRequeueOutcome::Requeued { stale_window_id } => stale_window_id,
        // Fail closed and name the reason, so the caller can tell "I aimed at a
        // running agent" apart from "there was nothing to recover" instead of
        // retrying blindly.
        crate::IssueMonitorRequeueOutcome::LaunchLive => {
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "refused",
                    "refusal": "launch_live",
                    "detail": "a launch still owns this issue — use issue.monitor.stop or issue.monitor.failover, which verify the exact live launch identity",
                })
                .to_string(),
            );
            out.push('\n');
            return Ok(1);
        }
        crate::IssueMonitorRequeueOutcome::NotHeld => {
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "refused",
                    "refusal": "not_held",
                    "detail": "no failure is holding this issue out of the queue",
                })
                .to_string(),
            );
            out.push('\n');
            return Ok(1);
        }
    };

    let delivery = issue_monitor_scan_delivery(request_immediate_monitor_scan(&project_root));

    out.push_str(
        &serde_json::json!({
            "number": number,
            "status": "requeued",
            "reason": reason,
            "stale_window_id": stale_window_id,
            "released_at": now,
            "failure_release_version": prefs.failure_release_version,
            "scan_requested": delivery.scan_requested,
            "scan_delivery": delivery.scan_delivery,
            "scan_error": delivery.scan_error,
            "pane_teardown": if stale_window_id.is_some() {
                "close the returned window with pane.close — the release already unbound it from the issue, so the close cannot requeue it again"
            } else {
                "none"
            },
        })
        .to_string(),
    );
    out.push('\n');
    // The release itself is committed even when the follow-up scan authority is
    // unavailable; scan delivery is reported truthfully in the JSON fields.
    Ok(0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueMonitorScanDelivery {
    scan_requested: bool,
    scan_delivery: &'static str,
    scan_error: Option<String>,
}

fn issue_monitor_scan_delivery(result: Result<(), String>) -> IssueMonitorScanDelivery {
    match result {
        Ok(()) => IssueMonitorScanDelivery {
            scan_requested: true,
            scan_delivery: "immediate",
            scan_error: None,
        },
        Err(error) => IssueMonitorScanDelivery {
            scan_requested: false,
            scan_delivery: "unavailable",
            scan_error: Some(error),
        },
    }
}

#[cfg(unix)]
fn request_immediate_monitor_scan(project_root: &std::path::Path) -> Result<(), String> {
    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({ "scan_now": {} }),
        std::process::id(),
    );
    publish_monitor_config_set(project_root, payload).map_err(|error| match error {
        crate::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(_) => {
            "daemon_control_unavailable".to_string()
        }
        crate::runtime_daemon_events::IssueMonitorControlPublishError::OutcomeUnknown(_) => {
            "scan_delivery_unknown".to_string()
        }
        crate::runtime_daemon_events::IssueMonitorControlPublishError::Busy(_) => {
            "scan_request_busy".to_string()
        }
        crate::runtime_daemon_events::IssueMonitorControlPublishError::RecoveryBlocked => {
            "authority_recovery_blocked".to_string()
        }
        crate::runtime_daemon_events::IssueMonitorControlPublishError::Rejected(_) => {
            "scan_request_rejected".to_string()
        }
    })
}

#[cfg(not(unix))]
fn request_immediate_monitor_scan(project_root: &std::path::Path) -> Result<(), String> {
    super::pane::request_issue_monitor_scan_now(project_root)
}

/// SPEC-3431 FR-031: a stable, greppable name for each refusal.
fn issue_monitor_stop_mismatch_label(mismatch: crate::IssueMonitorStopMismatch) -> &'static str {
    match mismatch {
        crate::IssueMonitorStopMismatch::UnknownIssue => "unknown_issue",
        crate::IssueMonitorStopMismatch::NotRunning => "not_running",
        crate::IssueMonitorStopMismatch::LaunchGenerationMismatch => "generation_mismatch",
        crate::IssueMonitorStopMismatch::ClaimMismatch => "claim_mismatch",
        crate::IssueMonitorStopMismatch::ClaimOwnerMismatch => "claim_owner_mismatch",
        crate::IssueMonitorStopMismatch::DeliveryMismatch => "delivery_mismatch",
        crate::IssueMonitorStopMismatch::MaterializerWindowMismatch => {
            "materializer_window_mismatch"
        }
        crate::IssueMonitorStopMismatch::WindowMismatch => "window_mismatch",
    }
}

fn apply_monitor_config_set(
    prefs: &mut crate::IssueMonitorPrefs,
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    pm_privileged: bool,
) -> io::Result<()> {
    validate_monitor_config_set(enabled, autonomous_mode, max_active, pm_privileged)?;
    let mut candidate =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    if let Some(enabled) = enabled {
        candidate
            .set_enabled_with_effect_revocation(enabled)
            .ok_or_else(|| io::Error::other("Issue Monitor authority epoch overflow"))?;
    }
    if let Some(autonomous_mode) = autonomous_mode {
        candidate
            .set_autonomous_mode_with_effect_revocation(autonomous_mode)
            .ok_or_else(|| io::Error::other("Issue Monitor authority epoch overflow"))?;
    }
    if let Some(max_active) = max_active {
        candidate.set_max_active_agents(max_active);
    }
    *prefs = candidate.prefs();
    Ok(())
}

fn validate_monitor_config_set(
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    pm_privileged: bool,
) -> io::Result<()> {
    if enabled.is_none() && autonomous_mode.is_none() && max_active.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one Issue Monitor config field is required",
        ));
    }
    // SPEC-3431 FR-008/FR-009: same rule as the parse layer — the registered
    // PM may raise the switches; every other session must use the GUI.
    if !pm_privileged && (enabled == Some(true) || autonomous_mode == Some(true)) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "enabling Issue Monitor or autonomous mode requires an explicit GUI action              (only the project's registered PM agent may raise it from the CLI;              run `pm.status` to see the current PM)",
        ));
    }
    if max_active == Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "max_active must be greater than zero",
        ));
    }
    Ok(())
}

/// SPEC-3431 FR-008/FR-009: the asymmetric boundary from Issue #3357 stays in
/// force for every agent session — only the project's registered PM may raise
/// `enabled` / `autonomous_mode`. The privileged subject is resolved from the
/// ambient `GWT_SESSION_ID` (the caller cannot claim someone else's id through
/// params) matched against the durable PM registration. Raising the switch
/// changes nothing about merges: SPEC #3200's fail-closed merge gate still
/// decides every merge on its own.
fn caller_is_registered_pm(project_root: &std::path::Path) -> bool {
    let Ok(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV) else {
        return false;
    };
    crate::pm_registry::session_is_registered_pm(
        &crate::pm_registry::pm_prefs_path_for_repo_path(project_root),
        session_id.trim(),
    )
}

fn run_monitor_config_set<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let pm_privileged = caller_is_registered_pm(&project_root);
    validate_monitor_config_set(enabled, autonomous_mode, max_active, pm_privileged)
        .map_err(io_as_api_error)?;

    // SPEC-3431 FR-008: a PM raising a switch writes the prefs SOT directly
    // and asks for an immediate rescan. The daemon control lane refuses ON in
    // its own decoder (it cannot see who sent a frame), and prefs is the
    // source the scan driver re-reads every pass — so this is the honest path
    // rather than a second authority channel.
    if pm_privileged && (enabled == Some(true) || autonomous_mode == Some(true)) {
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
        crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            apply_monitor_config_set(prefs, enabled, autonomous_mode, max_active, true)
        })
        .map_err(io_as_api_error)?;
        let scan_now = crate::runtime_daemon_events::issue_monitor_payload(
            "control",
            serde_json::json!({ "scan_now": {} }),
            std::process::id(),
        );
        let _ = publish_monitor_config_set(&project_root, scan_now);
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
        out.push_str(
            &serde_json::json!({
                "enabled": prefs.enabled,
                "autonomous_mode": prefs.autonomous_mode,
                "max_active": prefs.max_active_agents.max(1),
                "applied_by": "pm",
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(0);
    }

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "config_set": {
                "enabled": enabled,
                "autonomous_mode": autonomous_mode,
                "max_active_agents": max_active,
            }
        }),
        std::process::id(),
    );
    let publication = publish_monitor_config_set(&project_root, payload);
    if let Err(error) = publication {
        if !error.allows_local_fallback() {
            return Err(io_as_api_error(io::Error::other(error.to_string())));
        }
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
        crate::try_mutate_issue_monitor_prefs_without_authority_fence(&prefs_path, |prefs| {
            apply_monitor_config_set(prefs, enabled, autonomous_mode, max_active, pm_privileged)
        })
        .map_err(io_as_api_error)?;
    }
    let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
        &project_root,
    ))
    .map_err(io_as_api_error)?;
    out.push_str(
        &serde_json::json!({
            "enabled": prefs.enabled,
            "autonomous_mode": prefs.autonomous_mode,
            "max_active": prefs.max_active_agents.max(1),
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

#[cfg(unix)]
fn publish_monitor_config_set(
    project_root: &std::path::Path,
    payload: serde_json::Value,
) -> Result<(), crate::runtime_daemon_events::IssueMonitorControlPublishError> {
    crate::daemon_publisher::publish_issue_monitor_control(project_root, payload)
}

#[cfg(not(unix))]
fn publish_monitor_config_set(
    _project_root: &std::path::Path,
    _payload: serde_json::Value,
) -> Result<(), crate::runtime_daemon_events::IssueMonitorControlPublishError> {
    Err(
        crate::runtime_daemon_events::IssueMonitorControlPublishError::TransportUnavailable(
            "Issue Monitor daemon control is unavailable on this platform".to_string(),
        ),
    )
}

/// SPEC #3200 Option A: publish an independent-review verdict to the Issue
/// Monitor daemon's control channel. The daemon re-judges the raw verdict
/// (SHA-bound) — this only transports it.
#[cfg(unix)]
fn run_monitor_review_verdict<E: CliEnv>(
    env: &mut E,
    issue_number: u64,
    reviewed_sha: &str,
    verdict_raw: &str,
    out: &mut String,
) -> i32 {
    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "review_verdict": {
                "issue_number": issue_number,
                "reviewed_sha": reviewed_sha,
                "verdict_raw": verdict_raw,
            }
        }),
        std::process::id(),
    );
    match crate::daemon_publisher::publish_event(
        env.repo_path(),
        crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
        payload,
    ) {
        Ok(()) => {
            out.push_str(&format!(
                "review verdict published for #{issue_number} at {reviewed_sha}\n"
            ));
            0
        }
        Err(error) => {
            out.push_str(&format!(
                "review verdict publish failed for #{issue_number}: {error}\n"
            ));
            1
        }
    }
}

#[cfg(not(unix))]
fn run_monitor_review_verdict<E: CliEnv>(
    _env: &mut E,
    issue_number: u64,
    _reviewed_sha: &str,
    _verdict_raw: &str,
    out: &mut String,
) -> i32 {
    out.push_str(&format!(
        "review verdict publish unavailable on this platform (#{issue_number})\n"
    ));
    1
}

fn parse_issue_read_args(args: &[&String], mode: &str) -> Result<IssueCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;
    let mut refresh = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--refresh" => refresh = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
    }
    Ok(match mode {
        "view" => IssueCommand::View { number, refresh },
        "comments" => IssueCommand::Comments { number, refresh },
        "linked-prs" => IssueCommand::LinkedPrs { number, refresh },
        _ => return Err(CliParseError::Usage),
    })
}

fn parse_issue_create_args(args: &[&String]) -> Result<IssueCommand, CliParseError> {
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    Ok(IssueCommand::Create {
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
    })
}

fn parse_issue_comment_args(args: &[&String]) -> Result<IssueCommand, CliParseError> {
    if args.len() != 3 {
        return Err(CliParseError::Usage);
    }
    let number = args[0]
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(args[0].clone()))?;
    match args[1].as_str() {
        "-f" | "--file" => Ok(IssueCommand::Comment {
            number,
            file: args[2].clone(),
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

pub(super) fn issue_state_label(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => "OPEN",
        IssueState::Closed => "CLOSED",
    }
}

pub(super) fn render_issue(out: &mut String, snapshot: &IssueSnapshot) {
    out.push_str(&format!(
        "#{} [{}] {}\n",
        snapshot.number.0,
        issue_state_label(snapshot.state),
        snapshot.title
    ));
    if !snapshot.labels.is_empty() {
        out.push_str(&format!("labels: {}\n", snapshot.labels.join(", ")));
    }
    out.push_str(&format!("updated_at: {}\n\n", snapshot.updated_at.0));
    if !snapshot.body.is_empty() {
        out.push_str(snapshot.body.trim_end_matches('\n'));
        out.push('\n');
    }
}

pub(super) fn render_issue_comments(out: &mut String, snapshot: &IssueSnapshot) {
    if snapshot.comments.is_empty() {
        out.push_str("no comments\n");
        return;
    }
    for comment in &snapshot.comments {
        out.push_str(&format!(
            "=== comment:{} ({}) ===\n{}\n",
            comment.id.0, comment.updated_at.0, comment.body
        ));
    }
}

pub(super) fn render_linked_prs(out: &mut String, linked_prs: &[LinkedPrSummary]) {
    if linked_prs.is_empty() {
        out.push_str("no linked pull requests\n");
        return;
    }
    for pr in linked_prs {
        out.push_str(&format!(
            "#{} [{}] {}\n{}\n",
            pr.number, pr.state, pr.title, pr.url
        ));
    }
}

pub(super) fn load_or_refresh_issue<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    if !refresh {
        if let Some(entry) = cache.load_entry(number) {
            return Ok(entry);
        }
    }
    refresh_issue_cache(env, number)
}

pub(super) fn refresh_issue_cache<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    refresh_issue_cache_with_index_rebuild(env, number, |repo_path| {
        if crate::index_worker::detect_repo_hash(repo_path).is_none() {
            return Ok(());
        }
        crate::index_worker::default_rebuild_runner(
            repo_path,
            crate::index_worker::IndexRebuildScope::Issues,
            None,
        )
    })
}

pub(super) fn refresh_issue_cache_with_index_rebuild<E, F>(
    env: &mut E,
    number: IssueNumber,
    mut rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    let cache_root = env.cache_root();
    let before = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    let snapshot = match env.client().fetch(number, None)? {
        gwt_github::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::FetchResult::NotModified => {
            return Cache::new(cache_root)
                .load_entry(number)
                .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)));
        }
    };
    let cache = Cache::new(cache_root.clone());
    cache.write_snapshot(&snapshot)?;
    let after = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    if crate::issue_cache::issue_cache_source_changed(&before, &after) {
        rebuild_issue_index(env.repo_path()).map_err(|err| {
            SpecOpsError::from(ApiError::Network(format!("rebuild issue index: {err}")))
        })?;
    }
    cache
        .load_entry(number)
        .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)))
}

pub(super) fn load_or_refresh_linked_prs<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<Vec<LinkedPrSummary>, SpecOpsError> {
    let cache_root = env.cache_root();
    if !refresh {
        if let Some(cached) = read_linked_prs_cache(&cache_root, number)? {
            return Ok(cached);
        }
    }
    let linked_prs = env.fetch_linked_prs(number).map_err(io_as_api_error)?;
    write_linked_prs_cache(&cache_root, number, &linked_prs)?;
    Ok(linked_prs)
}

pub(super) fn linked_prs_cache_path(cache_root: &std::path::Path, number: IssueNumber) -> PathBuf {
    cache_root
        .join(number.0.to_string())
        .join("linked_prs.json")
}

pub(super) fn read_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
) -> Result<Option<Vec<LinkedPrSummary>>, SpecOpsError> {
    let path = linked_prs_cache_path(cache_root, number);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = serde_json::from_str(&text)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            Ok(Some(parsed))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_as_api_error(err)),
    }
}

pub(super) fn write_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
    linked_prs: &[LinkedPrSummary],
) -> Result<(), SpecOpsError> {
    let bytes = serde_json::to_vec_pretty(linked_prs)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
    write_atomic(&linked_prs_cache_path(cache_root, number), &bytes).map_err(io_as_api_error)
}

pub(crate) fn fetch_linked_prs_via_gh(
    owner: &str,
    repo: &str,
    number: IssueNumber,
) -> io::Result<Vec<LinkedPrSummary>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT, CONNECTED_EVENT]) {
        nodes {
          __typename
          ... on CrossReferencedEvent {
            willCloseTarget
            source {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
                body
                mergedAt
              }
            }
          }
          ... on ConnectedEvent {
            subject {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
                body
                mergedAt
              }
            }
          }
        }
      }
    }
  }
}
"#;

    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("repo={repo}"),
            "-F",
            &format!("number={}", number.0),
        ],
        gwt_core::process_console::SpawnOptions::new("gh api graphql issue timeline"),
    )?;

    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api graphql failed: {}",
            output.stderr.trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(parse_linked_pr_nodes(&value, number.0))
}

/// Parse the issue-timeline GraphQL response into linked-PR summaries.
/// `will_close_target` comes from `CrossReferencedEvent.willCloseTarget`;
/// `ConnectedEvent` (a manually linked PR) closes the issue on merge, so it
/// counts as closing. Duplicate PR numbers OR-merge the closing flag so a PR
/// seen as both a plain reference and a closing link keeps `true`.
pub(crate) fn parse_linked_pr_nodes(
    value: &serde_json::Value,
    issue_number: u64,
) -> Vec<LinkedPrSummary> {
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("issue"))
        .and_then(|v| v.get("timelineItems"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut index: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    let mut out: Vec<LinkedPrSummary> = Vec::new();
    for node in nodes {
        let typename = node
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let (pr, mut will_close_target) = match typename {
            "CrossReferencedEvent" => (
                node.get("source"),
                node.get("willCloseTarget")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            ),
            "ConnectedEvent" => (node.get("subject"), true),
            _ => (None, false),
        };
        let Some(pr) = pr else { continue };
        // gwt merges fixes into develop (not the default branch), so GitHub
        // reports willCloseTarget=false for every real fix PR (measured on
        // #3222/#3213). The closing INTENT therefore also comes from closing
        // keywords in the PR body targeting THIS issue (`Closes #N` — the gwt
        // PR-body contract).
        if !will_close_target {
            if let Some(body) = pr.get("body").and_then(|v| v.as_str()) {
                will_close_target = body_closes_issue(body, issue_number);
            }
        }
        if pr.get("__typename").and_then(|v| v.as_str()) != Some("PullRequest") {
            continue;
        }
        let Some(pr_number) = pr.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let merged_at = pr
            .get("mergedAt")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if let Some(existing) = index.get(&pr_number) {
            out[*existing].will_close_target |= will_close_target;
            if out[*existing].merged_at.is_none() {
                out[*existing].merged_at = merged_at;
            }
            continue;
        }
        index.insert(pr_number, out.len());
        out.push(LinkedPrSummary {
            number: pr_number,
            will_close_target,
            merged_at,
            title: pr
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: pr
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: pr
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    out
}

/// GitHub closing keywords (close/closes/closed, fix/fixes/fixed,
/// resolve/resolves/resolved) followed by `#<issue_number>`, matched
/// case-insensitively anywhere in the PR body.
pub(crate) fn body_closes_issue(body: &str, issue_number: u64) -> bool {
    let needle = format!("#{issue_number}");
    let lower = body.to_lowercase();
    let bytes = lower.as_bytes();
    for keyword in [
        "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
    ] {
        let mut start = 0;
        while let Some(pos) = lower[start..].find(keyword) {
            let begin = start + pos;
            let after = begin + keyword.len();
            start = after;
            // #3228 review: the keyword must stand alone. Without a leading
            // word boundary, `prefix #42` / `hotfix #42` (fix) and
            // `disclosed #42` / `enclosed #42` (closed) would count as
            // closing intent. A trailing boundary is required too so `close`
            // does not fire inside `closedown #42`-style words (the exact
            // keywords `closes`/`closed` match via their own entries).
            let leading_ok = begin == 0 || !bytes[begin - 1].is_ascii_alphanumeric();
            let trailing_ok = !bytes
                .get(after)
                .copied()
                .is_some_and(|next| next.is_ascii_alphanumeric());
            if !leading_ok || !trailing_ok {
                continue;
            }
            let rest = lower[after..].trim_start_matches([':', ' ', '\t']);
            if rest.starts_with(&needle) {
                let tail = &rest[needle.len()..];
                let digit_follows = tail
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_digit());
                if !digit_follows {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{
        io::{BufRead, Write},
        os::unix::net::UnixListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use gwt_core::test_support::ScopedGwtHome;
    use gwt_github::client::{IssueSnapshot, IssueState, UpdatedAt};
    use tempfile::TempDir;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn parse_linked_pr_nodes_tracks_will_close_target() {
        // codex #3226/#3227 review: the completion probe must distinguish PRs
        // that CLOSE the issue from mere cross-references. Measured reality:
        // in gwt's develop-based flow GitHub reports willCloseTarget=false for
        // every PR (auto-close only applies to the default branch), so the
        // closing INTENT must also be derived from closing keywords in the PR
        // body (`Closes #N` — the gwt PR-body contract).
        let value = serde_json::json!({"data":{"repository":{"issue":{"timelineItems":{"nodes":[
            {"__typename":"CrossReferencedEvent","willCloseTarget":true,
             "source":{"__typename":"PullRequest","number":10,"title":"closes it","state":"MERGED","url":"u10","body":"","mergedAt":"2026-08-10T00:00:00Z"}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":11,"title":"refs only","state":"MERGED","url":"u11","body":"Related to #42 (no closing keyword)"}},
            {"__typename":"ConnectedEvent",
             "subject":{"__typename":"PullRequest","number":12,"title":"manually linked","state":"OPEN","url":"u12","body":""}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":13,"title":"develop-based fix","state":"MERGED","url":"u13",
                       "body":"## Closing Issues\n\nCloses #42"}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":14,"title":"closes another","state":"MERGED","url":"u14",
                       "body":"Fixes #43"}}
        ]}}}}});
        let prs = parse_linked_pr_nodes(&value, 42);
        let get = |n: u64| prs.iter().find(|pr| pr.number == n).expect("pr");
        assert!(get(10).will_close_target, "GraphQL willCloseTarget");
        assert_eq!(get(10).merged_at.as_deref(), Some("2026-08-10T00:00:00Z"));
        assert!(!get(11).will_close_target, "plain reference must NOT close");
        assert!(
            get(12).will_close_target,
            "manually connected PR closes on merge"
        );
        assert!(
            get(13).will_close_target,
            "body closing keyword for THIS issue counts (develop-based flow)"
        );
        assert!(
            !get(14).will_close_target,
            "closing keyword for a DIFFERENT issue does not count"
        );
    }

    #[test]
    fn body_closes_issue_requires_word_boundaries() {
        // codex/coderabbit #3228 review: `find(keyword)` matched substrings
        // inside longer words, so `prefix #42` (fix), `disclosed #42` /
        // `enclosed #42` (closed), `hotfix #42` (fix) all counted as closing.
        for negative in [
            "prefix #42",
            "hotfix #42",
            "disclosed #42",
            "enclosed #42",
            "unfixed #42",
        ] {
            assert!(
                !body_closes_issue(negative, 42),
                "substring keyword must not close: {negative}"
            );
        }
        for positive in [
            "Closes #42",
            "fixes #42",
            "Fixed: #42",
            "resolve #42",
            "- Fix #42 in the parser",
        ] {
            assert!(
                body_closes_issue(positive, 42),
                "standalone keyword closes: {positive}"
            );
        }
        // Trailing-digit boundary is preserved.
        assert!(!body_closes_issue("Closes #421", 42));
    }

    #[test]
    fn parse_linked_pr_nodes_or_merges_duplicate_pr_flags() {
        // The same PR seen first as a plain reference and later as a closing
        // link must keep will_close_target=true.
        let value = serde_json::json!({"data":{"repository":{"issue":{"timelineItems":{"nodes":[
            {"__typename":"CrossReferencedEvent","willCloseTarget":false,
             "source":{"__typename":"PullRequest","number":10,"title":"t","state":"MERGED","url":"u"}},
            {"__typename":"CrossReferencedEvent","willCloseTarget":true,
             "source":{"__typename":"PullRequest","number":10,"title":"t","state":"MERGED","url":"u"}}
        ]}}}}});
        let prs = parse_linked_pr_nodes(&value, 42);
        assert_eq!(prs.len(), 1);
        assert!(
            prs[0].will_close_target,
            "closing flag OR-merges across events"
        );
    }

    #[test]
    fn issue_family_parse_directly_handles_view() {
        let cmd = parse(&[s("view"), s("42")]).expect("parse issue family command");
        assert_eq!(
            cmd,
            IssueCommand::View {
                number: 42,
                refresh: false,
            }
        );
    }

    #[test]
    fn issue_spec_submodule_parse_directly_handles_list() {
        let args = [s("list"), s("--phase"), s("phase/implementation")];
        let refs = args.iter().collect::<Vec<_>>();
        let cmd = crate::cli::issue_spec::parse(&refs).expect("parse spec family command");
        assert_eq!(
            cmd,
            IssueCommand::SpecList {
                phase: Some("phase/implementation".to_string()),
                state: None,
            }
        );
    }

    #[test]
    fn issue_family_run_directly_renders_cached_issue() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let snapshot = IssueSnapshot {
            number: IssueNumber(42),
            title: "Issue family direct run".to_string(),
            body: "body".to_string(),
            labels: vec!["bug".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-12T00:00:00Z"),
            comments: vec![],
        };
        gwt_github::Cache::new(tmp.path().to_path_buf())
            .write_snapshot(&snapshot)
            .expect("write cache");

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::View {
                number: 42,
                refresh: false,
            },
            &mut out,
        )
        .expect("run issue family");

        assert_eq!(code, 0);
        assert!(out.contains("#42 [OPEN] Issue family direct run"));
    }

    #[test]
    fn immediate_scan_delivery_never_claims_an_unacknowledged_schedule() {
        let immediate = issue_monitor_scan_delivery(Ok(()));
        assert!(immediate.scan_requested);
        assert_eq!(immediate.scan_delivery, "immediate");
        assert_eq!(immediate.scan_error, None);

        let unavailable = issue_monitor_scan_delivery(Err("gui_command_unavailable".to_string()));
        assert!(!unavailable.scan_requested);
        assert_eq!(unavailable.scan_delivery, "unavailable");
        assert_eq!(
            unavailable.scan_error.as_deref(),
            Some("gui_command_unavailable")
        );
        assert_ne!(unavailable.scan_delivery, "next-scheduled-scan");
    }

    #[test]
    #[cfg(windows)]
    // The tungstenite handshake callback's `Err` variant is the library's own
    // `ErrorResponse` type, so its size is not ours to shrink.
    #[allow(clippy::result_large_err, reason = "tungstenite fixes this signature")]
    fn windows_launch_now_persists_priority_and_reports_authenticated_gui_ack() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use gwt_core::test_support::ScopedEnvVar;
        use tokio_tungstenite::tungstenite::Message;

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![7, 42],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");

        let expected_scope = gwt_core::paths::project_scope_hash(&repo)
            .as_str()
            .to_string();
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build Windows scan mock runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind Windows scan mock");
                address_tx
                    .send(listener.local_addr().expect("Windows scan mock address"))
                    .expect("publish Windows scan mock address");
                let (stream, _) =
                    tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                        .await
                        .expect("Windows launch_now must connect to the GUI scan authority")
                        .expect("accept scan client");
                let mut socket = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio_tungstenite::accept_hdr_async(
                        stream,
                        |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                         response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                            assert_eq!(request.uri().path(), "/internal/pane-ws");
                            assert_eq!(
                                request
                                    .headers()
                                    .get(
                                        tokio_tungstenite::tungstenite::http::header::AUTHORIZATION,
                                    )
                                    .and_then(|value| value.to_str().ok()),
                                Some("Bearer windows-scan-capability")
                            );
                            Ok(response)
                        },
                    ),
                )
                .await
                .expect("Windows scan client must complete its WebSocket handshake")
                .expect("accept authenticated scan WebSocket");
                let message =
                    tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                        .await
                        .expect("Windows scan client must send its request frame")
                        .expect("scan request frame")
                        .expect("valid scan request frame");
                let text = message.into_text().expect("text scan request");
                let request: serde_json::Value =
                    serde_json::from_str(text.as_ref()).expect("scan request JSON");
                assert_eq!(request["kind"], "agent_issue_monitor_scan_now");
                assert_eq!(request["expected_project_scope"], expected_scope);
                assert!(
                    request.get("project_root").is_none(),
                    "the Windows request must not claim project authority"
                );
                socket
                    .send(Message::Text(
                        serde_json::json!({
                            "kind": "issue_monitor_scan_request_result",
                            "accepted": true,
                            "reason": null,
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .expect("send immediate scan acknowledgement");
            });
        });
        let address = address_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("Windows scan mock ready");
        let pane_url = format!("ws://{address}/internal/pane-ws");
        let _pane_url = ScopedEnvVar::set(gwt_agent::GWT_PANE_WS_URL_ENV, &pane_url);
        let _token = ScopedEnvVar::set(
            gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV,
            "windows-scan-capability",
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorLaunchNow {
                project_root: None,
                number: 42,
            },
            &mut out,
        )
        .expect("Windows launch_now result");
        server.join().expect("Windows scan mock thread");
        let result: serde_json::Value = serde_json::from_str(out.trim()).expect("result JSON");

        assert_eq!(code, 0);
        assert_eq!(result["priority_updated"], true);
        assert_eq!(result["priority_order"], serde_json::json!([42, 7]));
        assert_eq!(result["scan_requested"], true);
        assert_eq!(result["scan_delivery"], "immediate");
        assert_eq!(result["scan_error"], serde_json::Value::Null);
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("persisted prefs")
                .priority_order,
            vec![42, 7]
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_launch_now_reports_gui_unavailable_without_claiming_future_delivery() {
        use gwt_core::test_support::ScopedEnvVar;

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let _pane_url = ScopedEnvVar::unset(gwt_agent::GWT_PANE_WS_URL_ENV);
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![7, 42],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorLaunchNow {
                project_root: None,
                number: 42,
            },
            &mut out,
        )
        .expect("Windows launch_now unavailable result");
        let result: serde_json::Value = serde_json::from_str(out.trim()).expect("result JSON");

        assert_eq!(code, 1);
        assert_eq!(result["priority_updated"], true);
        assert_eq!(result["priority_order"], serde_json::json!([42, 7]));
        assert_eq!(result["scan_requested"], false);
        assert_eq!(result["scan_delivery"], "unavailable");
        assert_eq!(result["scan_error"], "gui_command_unavailable");
        assert_ne!(result["scan_delivery"], "next-scheduled-scan");
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("persisted prefs")
                .priority_order,
            vec![42, 7]
        );
    }

    #[test]
    #[cfg(unix)]
    fn launch_now_persists_priority_but_fails_closed_without_scan_authority() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![7, 42],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorLaunchNow {
                project_root: None,
                number: 42,
            },
            &mut out,
        )
        .expect("launch_now result");
        let result: serde_json::Value = serde_json::from_str(out.trim()).expect("result JSON");

        assert_eq!(code, 1, "unaccepted immediate delivery is not success");
        assert_eq!(result["priority_updated"], true);
        assert_eq!(result["priority_order"], serde_json::json!([42, 7]));
        assert_eq!(result["scan_requested"], false);
        assert_eq!(result["scan_delivery"], "unavailable");
        assert_eq!(result["scan_error"], "daemon_control_unavailable");
        assert!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("persisted prefs")
                .priority_order
                .starts_with(&[42]),
            "the partial priority update remains explicit and durable"
        );
    }

    /// Issue #3616 AC-5: an explicit PM launch instruction overrides a provider
    /// quota hold.
    ///
    /// The observed reset was six days out. `launch_now` only reorders
    /// `priority_order`, so without clearing the hold the instruction is
    /// accepted, reported as applied, and then silently ignored by
    /// `retry_ready` for the whole window — which is precisely the recovery a
    /// PM reaches for after switching to a healthy provider.
    #[test]
    fn launch_now_clears_a_provider_quota_hold_so_the_instruction_is_not_inert() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![42],
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 42,
                    retry_not_before: Some("2026-08-22T03:46:00Z".to_string()),
                    retry_hold_reason: Some("Codex usage limit reached".to_string()),
                    ..crate::AutonomousIssueRecord::new(42)
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();

        let _ = run(
            &mut env,
            IssueCommand::MonitorLaunchNow {
                project_root: None,
                number: 42,
            },
            &mut out,
        )
        .expect("launch_now result");
        let result: serde_json::Value = serde_json::from_str(out.trim()).expect("result JSON");

        assert_eq!(result["hold_cleared"], true);
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("persisted prefs");
        let record = persisted
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 42)
            .expect("the record survives");
        assert_eq!(record.retry_not_before, None);
        assert_eq!(record.retry_hold_reason, None);
    }

    /// Issue #3655 AC-4: an open unblock request has to be visible in the one
    /// field the PM reads to find work that needs a human.
    #[test]
    fn issue_monitor_status_surfaces_an_open_board_escalation_as_needs_human() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let escalation = gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Claude Code",
            gwt_core::coordination::BoardEntryKind::Blocked,
            "事象: 拒否\n原因: immutable\n依頼: fresh launch\n再開条件: 新 pane",
            None,
            None,
            vec![],
            vec!["2338".to_string()],
        );
        gwt_core::coordination::post_entry(&repo, escalation).expect("post escalation");

        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        let status: serde_json::Value =
            serde_json::from_str(out.trim()).expect("status json: {out}");
        assert_eq!(
            status["needs_human"],
            serde_json::json!([2338]),
            "an agent blocked on #2338 must be findable without reading its pane: {out}"
        );
    }

    /// ...and disappear again the moment somebody resolves it.
    #[test]
    fn issue_monitor_status_drops_the_escalation_once_it_is_resolved() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let escalation = gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::Agent,
            "Claude Code",
            gwt_core::coordination::BoardEntryKind::Blocked,
            "事象: 拒否\n原因: immutable\n依頼: fresh launch\n再開条件: 新 pane",
            None,
            None,
            vec![],
            vec!["2338".to_string()],
        );
        let escalation_id = escalation.id.clone();
        gwt_core::coordination::post_entry(&repo, escalation).expect("post escalation");
        let mut resolution = gwt_core::coordination::BoardEntry::new(
            gwt_core::coordination::AuthorKind::User,
            "You",
            gwt_core::coordination::BoardEntryKind::Decision,
            "fresh launch を手配しました",
            None,
            None,
            vec![],
            vec!["2338".to_string()],
        );
        resolution.resolves_entry_ids = vec![escalation_id];
        gwt_core::coordination::post_entry(&repo, resolution).expect("post resolution");

        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        let status: serde_json::Value =
            serde_json::from_str(out.trim()).expect("status json: {out}");
        assert_eq!(status["needs_human"], serde_json::json!([]), "{out}");
    }

    #[test]
    fn issue_monitor_status_reports_ordered_queue_and_active_launches() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 3,
                priority_order: vec![2, 1],
                launching_issues: vec![crate::IssueMonitorLaunchingIssue {
                    issue_number: 9,
                    claimed_at: Some("2026-08-03T00:00:00Z".to_string()),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        let cache = gwt_github::Cache::new(cache_root);
        for number in [1, 2] {
            cache
                .write_snapshot(&IssueSnapshot {
                    number: IssueNumber(number),
                    title: format!("Issue {number}"),
                    body: String::new(),
                    labels: Vec::new(),
                    state: IssueState::Open,
                    updated_at: UpdatedAt::new("2026-08-03T00:00:00Z"),
                    comments: Vec::new(),
                })
                .expect("write cache");
        }
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        let prefs_before = std::fs::read(&prefs_path).expect("prefs bytes");

        let code = run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        assert_eq!(code, 0);
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            prefs_before,
            "status must stay read-only"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(out.trim()).expect("status json"),
            serde_json::json!({
                "queue": [2, 1],
                "active_launches": [9],
                "max_active": 3,
                "enabled": true,
                "autonomous_mode": false,
                "has_launch_profile": false,
                // SPEC-3431 FR-024: the offline fallback serializes the same
                // projection as the daemon branch, so a caller sees one shape
                // regardless of whether the daemon happens to be publishing.
                "needs_human": [],
                "control_state_revision": 0,
                "inbox": [
                    {
                        "issue_number": 2,
                        "state": "queued",
                        "github_state": "open",
                        "issue_updated_at": "2026-08-03T00:00:00Z",
                        "readiness": "not_applicable",
                        "recoverable_merged": false,
                        "wait_reason": null,
                        "control_ready": {
                            "stop": { "ready": false, "degraded_reason": "not_running" },
                            "failover": { "ready": false, "degraded_reason": "not_running" },
                            "recover": { "ready": false, "degraded_reason": "not_running" },
                        },
                    },
                    {
                        "issue_number": 1,
                        "state": "queued",
                        "github_state": "open",
                        "issue_updated_at": "2026-08-03T00:00:00Z",
                        "readiness": "not_applicable",
                        "recoverable_merged": false,
                        "wait_reason": null,
                        "control_ready": {
                            "stop": { "ready": false, "degraded_reason": "not_running" },
                            "failover": { "ready": false, "degraded_reason": "not_running" },
                            "recover": { "ready": false, "degraded_reason": "not_running" },
                        },
                    },
                ],
                // Issue #3633 AC-5: this branch rebuilds the queue from the
                // local Issue cache, which is a projection and not a scan. It
                // used to stamp the literal string `gwtd-status` into
                // `last_scan_at`, so a monitor no driver had ever scanned
                // reported a cadence — the healthy-looking snapshot that made
                // the stall unobservable. There is no scan to report, and the
                // stall says why.
                "scan_stall": "Issue Monitor has never completed a scan for this project; no driver is running",
            })
        );
    }

    /// Issue #3633 AC-5: the offline branch must report the durable cadence,
    /// not the age of its own cache rebuild.
    #[test]
    fn issue_monitor_status_reports_the_persisted_scan_time_without_a_daemon() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                last_scan_at: Some("2020-01-01T00:00:00Z".to_string()),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        assert_eq!(code, 0);
        let status: serde_json::Value = serde_json::from_str(out.trim()).expect("status json");
        assert_eq!(status["last_scan_at"], "2020-01-01T00:00:00Z");
        assert!(
            status["scan_stall"]
                .as_str()
                .is_some_and(|reason| reason.contains("2020-01-01T00:00:00Z")),
            "a scan that last ran in 2020 must read as stalled: {status}"
        );
    }

    #[test]
    fn issue_monitor_status_rejects_a_daemon_projection_older_than_disk_control_state() {
        let prefs = crate::IssueMonitorPrefs {
            control_state_revision: 7,
            ..crate::IssueMonitorPrefs::default()
        };
        let current = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        )
        .agent_status();
        assert!(daemon_issue_monitor_status_is_current(&current, &prefs));

        let mut stale = current;
        stale.control_state_revision = 6;
        assert!(
            !daemon_issue_monitor_status_is_current(&stale, &prefs),
            "a stale daemon snapshot must be rebuilt from the newer durable control plane"
        );
    }

    #[test]
    fn issue_monitor_status_exposes_recoverable_legacy_merged_evidence() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                merged_issues: vec![42],
                issue_completion_migration_version: 0,
                completion_records: Vec::new(),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save legacy prefs");
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        gwt_github::Cache::new(cache_root)
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(42),
                title: "Still open".to_string(),
                body: String::new(),
                labels: Vec::new(),
                state: IssueState::Open,
                updated_at: UpdatedAt::new("2026-08-15T00:00:00Z"),
                comments: Vec::new(),
            })
            .expect("write cache");
        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        assert_eq!(code, 0);
        let status: serde_json::Value = serde_json::from_str(out.trim()).expect("status json");
        assert_eq!(status["inbox"][0]["issue_number"], 42);
        assert_eq!(status["inbox"][0]["github_state"], "open");
        assert_eq!(status["inbox"][0]["state"], "merged");
        assert_eq!(status["inbox"][0]["recoverable_merged"], true);
        assert_eq!(status["inbox"][0]["completion_reason"], "legacy_unverified");
    }

    #[test]
    #[cfg(unix)]
    fn issue_monitor_status_prefers_live_daemon_queue_over_cached_candidates() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(&prefs_path, &crate::IssueMonitorPrefs::default())
            .expect("save prefs");
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        gwt_github::Cache::new(cache_root)
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(42),
                title: "Claimed elsewhere".to_string(),
                body: String::new(),
                labels: Vec::new(),
                state: IssueState::Open,
                updated_at: UpdatedAt::new("2026-08-03T00:00:00Z"),
                comments: Vec::new(),
            })
            .expect("write stale cache candidate");

        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            &repo,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .expect("runtime scope");
        let socket_path = tmp.path().join("live-status.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind live daemon");
        listener
            .set_nonblocking(true)
            .expect("nonblocking live daemon");
        let endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            "live-status-token".to_string(),
            "test-daemon".to_string(),
        );
        gwt_core::daemon::persist_endpoint(
            &scope.endpoint_path(&gwt_core::paths::gwt_home()),
            &endpoint,
        )
        .expect("persist live daemon endpoint");
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !server_stop.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("accept status client: {error}"),
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking status client stream");
                let mut reader =
                    std::io::BufReader::new(stream.try_clone().expect("clone status stream"));
                let mut writer = stream;
                let mut line = String::new();
                reader.read_line(&mut line).expect("read status handshake");
                let request: gwt_core::daemon::IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse status handshake");
                assert_eq!(request.scope, scope);
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&gwt_core::daemon::IpcHandshakeResponse {
                        protocol_version: gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
                        daemon_version: "test-daemon".to_string(),
                        accepted: true,
                        rejection_reason: None,
                    })
                    .expect("serialize status handshake")
                )
                .expect("write status handshake");
                line.clear();
                reader.read_line(&mut line).expect("read status request");
                assert!(matches!(
                    serde_json::from_str::<gwt_core::daemon::ClientFrame>(line.trim_end())
                        .expect("parse status request"),
                    gwt_core::daemon::ClientFrame::Status
                ));
                writeln!(
                    writer,
                    "{}",
                    serde_json::json!({
                        "type": "status",
                        "protocol_version": gwt_core::daemon::DAEMON_PROTOCOL_VERSION,
                        "daemon_version": "test-daemon",
                        "uptime_seconds": 1,
                        "broadcast_channels": 1,
                        "connections": 1,
                        "issue_monitor": {
                            "queue": [],
                            "active_launches": [],
                            "max_active": 1,
                            "enabled": false,
                            "autonomous_mode": false,
                            "has_launch_profile": false
                        }
                    })
                )
                .expect("write live status");
                return;
            }
        });

        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();
        let result = run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        );
        stop.store(true, Ordering::Release);
        server.join().expect("live daemon joins");
        result.expect("status");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(out.trim())
                .expect("status json")
                .get("queue"),
            Some(&serde_json::json!([]))
        );
    }

    #[test]
    fn issue_monitor_priority_operations_roundtrip_and_reject_out_of_range() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                priority_order: vec![1, 2, 3],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        assert_eq!(
            run(
                &mut env,
                IssueCommand::MonitorPriorityMove {
                    project_root: Some(repo.clone()),
                    number: 1,
                    position: crate::cli::IssueMonitorPriorityPosition::Index(2),
                },
                &mut out,
            )
            .expect("move backward"),
            0
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("load backward move")
                .priority_order,
            vec![2, 3, 1]
        );
        run(
            &mut env,
            IssueCommand::MonitorPrioritySet {
                project_root: Some(repo.clone()),
                issue_numbers: vec![1, 2, 3],
            },
            &mut out,
        )
        .expect("restore priorities");
        out.clear();

        assert_eq!(
            run(
                &mut env,
                IssueCommand::MonitorPriorityMove {
                    project_root: Some(repo.clone()),
                    number: 2,
                    position: crate::cli::IssueMonitorPriorityPosition::Head,
                },
                &mut out,
            )
            .expect("move"),
            0
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("load moved prefs")
                .priority_order,
            vec![2, 1, 3]
        );

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::MonitorPrioritySet {
                    project_root: Some(repo.clone()),
                    issue_numbers: vec![8, 5],
                },
                &mut out,
            )
            .expect("set"),
            0
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("load set prefs")
                .priority_order,
            vec![8, 5]
        );

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::MonitorPriorityMove {
                    project_root: Some(repo.clone()),
                    number: 13,
                    position: crate::cli::IssueMonitorPriorityPosition::Index(1),
                },
                &mut out,
            )
            .expect("insert missing priority"),
            0
        );
        assert_eq!(
            crate::load_issue_monitor_prefs(&prefs_path)
                .expect("load inserted prefs")
                .priority_order,
            vec![8, 13, 5]
        );

        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        out.clear();
        assert!(run(
            &mut env,
            IssueCommand::MonitorPriorityMove {
                project_root: Some(repo.clone()),
                number: 13,
                position: crate::cli::IssueMonitorPriorityPosition::Index(4),
            },
            &mut out,
        )
        .is_err());
        assert_eq!(std::fs::read(&prefs_path).expect("prefs bytes"), before);

        out.clear();
        assert_eq!(
            run(
                &mut env,
                IssueCommand::MonitorPrioritySet {
                    project_root: Some(repo),
                    issue_numbers: Vec::new(),
                },
                &mut out,
            )
            .expect("clear priorities"),
            0
        );
        assert!(crate::load_issue_monitor_prefs(&prefs_path)
            .expect("load cleared prefs")
            .priority_order
            .is_empty());

        out.clear();
        assert!(run(
            &mut env,
            IssueCommand::MonitorPrioritySet {
                project_root: Some(tmp.path().join("missing-project")),
                issue_numbers: vec![99],
            },
            &mut out,
        )
        .is_err());
    }

    #[test]
    fn issue_monitor_config_set_falls_back_safely_when_daemon_is_absent() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorConfigSet {
                project_root: Some(repo),
                enabled: Some(false),
                autonomous_mode: Some(false),
                max_active: Some(3),
            },
            &mut out,
        )
        .expect("config set");

        assert_eq!(code, 0);
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(!prefs.enabled);
        assert!(!prefs.autonomous_mode);
        assert_eq!(prefs.max_active_agents, 3);
        assert_eq!(prefs.effect_authority_epoch, 2);

        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        out.clear();
        assert!(run(
            &mut env,
            IssueCommand::MonitorConfigSet {
                project_root: None,
                enabled: Some(true),
                autonomous_mode: None,
                max_active: None,
            },
            &mut out,
        )
        .is_err());
        assert_eq!(std::fs::read(&prefs_path).expect("prefs bytes"), before);
    }

    /// SPEC-3431 FR-033 / T-087b: the operation the PM actually calls.
    ///
    /// Drives the whole path a `gwtd` invocation takes — load prefs, evaluate
    /// the identity, commit or refuse — because the unit matrix on
    /// `IssueMonitorState` cannot catch a handler that writes prefs on a
    /// refusal or reports success it did not achieve.
    #[test]
    fn monitor_stop_revokes_the_launch_and_refuses_a_stale_identity() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        t254_register_replacement_pm(&repo);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        // A stale window id must change nothing on disk.
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo.clone()),
                number: 42,
                operation_id: Some("legacy-stop-stale".to_string()),
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-9".to_string()),
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 1, "a refused stop is not a success");
        assert!(out.contains("\"status\":\"refused\""), "{out}");
        assert!(out.contains("\"mismatch\":\"window_mismatch\""), "{out}");
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            before,
            "a refused stop must be zero-mutation"
        );

        // The exact identity releases the slot and holds the issue durably.
        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo.clone()),
                number: 42,
                operation_id: Some("legacy-stop-exact".to_string()),
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"stopped\""), "{out}");
        assert!(out.contains("legacy-stop-exact"), "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.launched_issues.is_empty(),
            "the slot must be released on disk"
        );
        let reloaded =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        assert_eq!(
            reloaded.stop_only_reason(42).as_deref(),
            Some("provider rate limit"),
            "FR-031: the reason must survive the prefs roundtrip"
        );

        // Repeating it is idempotent rather than an error.
        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo),
                number: 42,
                operation_id: Some("legacy-stop-exact".to_string()),
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"stopped\""), "{out}");
    }

    /// SPEC-3431 FR-029〜031 / T-081: the failover the PM calls when a provider
    /// runs out of quota.
    #[test]
    fn monitor_failover_requeues_at_the_head_and_refuses_a_stale_identity() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![43, 42],
                launch_profile: Some(t254_source_launch_profile()),
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        t254_register_replacement_pm(&repo);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorFailover {
                project_root: Some(repo.clone()),
                number: 42,
                operation_id: Some("legacy-failover-stale".to_string()),
                reason: "codex rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-9".to_string()),
            },
            &mut out,
        )
        .expect("failover runs");
        assert_eq!(code, 1);
        assert!(out.contains("\"mismatch\":\"window_mismatch\""), "{out}");
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            before,
            "a refused failover must be zero-mutation"
        );

        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorFailover {
                project_root: Some(repo),
                number: 42,
                operation_id: Some("legacy-failover-exact".to_string()),
                reason: "codex rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("failover runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"failover_pending\""), "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.launched_issues.is_empty(),
            "the old launch must be revoked on disk"
        );
        assert!(prefs.pending_controls.iter().any(|pending| {
            pending.operation_id == "legacy-failover-exact" && !pending.teardown_settled
        }));
        assert!(
            prefs.failed_issues.is_empty(),
            "a failover is not a failure and must not leave a hold behind"
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum T254ControlAction {
        Stop,
        Failover,
        Recover,
    }

    impl T254ControlAction {
        fn as_str(self) -> &'static str {
            match self {
                Self::Stop => "stop",
                Self::Failover => "failover",
                Self::Recover => "recover",
            }
        }

        fn expected_initial_outcome(self) -> &'static str {
            match self {
                Self::Stop => "stopped",
                Self::Failover => "failover_pending",
                Self::Recover => "recovery_pending",
            }
        }
    }

    #[derive(Debug, Clone)]
    struct T254ControlTarget {
        issue_number: u64,
        reason: String,
        launch_generation: Option<u64>,
        claim_id: Option<String>,
        claim_owner: Option<String>,
        delivery_id: Option<String>,
        materializer_window_id: Option<String>,
        window_id: Option<String>,
    }

    fn t254_exact_control_target() -> T254ControlTarget {
        T254ControlTarget {
            issue_number: 42,
            reason: "operator observed a stalled launch".to_string(),
            launch_generation: Some(7),
            claim_id: Some("claim-42-generation-7".to_string()),
            claim_owner: Some("source-agent-session".to_string()),
            delivery_id: Some("delivery-42-generation-7".to_string()),
            materializer_window_id: Some("tab-pm::materializer-7".to_string()),
            window_id: Some("tab-work::agent-7".to_string()),
        }
    }

    #[derive(serde::Serialize)]
    struct T254CanonicalReceiptFingerprint<'a> {
        version: &'static str,
        project_key: &'a str,
        action: &'static str,
        actor_session: &'a str,
        pm_registration_generation: u64,
        reason: &'a str,
        issue_number: u64,
        launch_generation: Option<u64>,
        claim_id: &'a Option<String>,
        claim_owner: &'a Option<String>,
        delivery_id: &'a Option<String>,
        materializer_window_id: &'a Option<String>,
        window_id: &'a Option<String>,
        pinned_profile_digest: &'a str,
    }

    fn t254_receipt_fingerprint(
        repo: &std::path::Path,
        action: T254ControlAction,
        actor_session: &str,
        pm_registration_generation: u64,
        pinned_profile_digest: &str,
        target: &T254ControlTarget,
    ) -> String {
        let canonical_root = dunce::canonicalize(repo).expect("canonical fixture project root");
        let project_key = gwt_core::paths::project_scope_hash(&canonical_root)
            .as_str()
            .to_string();
        let reason = target.reason.trim().to_string();
        let canonical = T254CanonicalReceiptFingerprint {
            version: "pm-control-request-v1",
            project_key: &project_key,
            action: action.as_str(),
            actor_session,
            pm_registration_generation,
            reason: &reason,
            issue_number: target.issue_number,
            launch_generation: target.launch_generation,
            claim_id: &target.claim_id,
            claim_owner: &target.claim_owner,
            delivery_id: &target.delivery_id,
            materializer_window_id: &target.materializer_window_id,
            window_id: &target.window_id,
            pinned_profile_digest,
        };
        format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(
                serde_json::to_vec(&canonical).expect("serialize canonical control target")
            )
        )
    }

    fn t254_launch_profile_digest(profile: &crate::IssueMonitorLaunchProfile) -> String {
        format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(
                serde_json::to_vec(profile).expect("serialize canonical launch profile")
            )
        )
    }

    fn t254_source_launch_profile() -> crate::IssueMonitorLaunchProfile {
        crate::IssueMonitorLaunchProfile {
            agent_id: "codex".to_string(),
            model: Some("gpt-5.6-sol".to_string()),
            reasoning: Some("high".to_string()),
            version: None,
            session_mode: gwt_agent::SessionMode::default(),
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: gwt_agent::LaunchRuntimeTarget::default(),
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::default(),
            windows_shell: None,
        }
    }

    fn t254_changed_global_launch_profile() -> crate::IssueMonitorLaunchProfile {
        crate::IssueMonitorLaunchProfile {
            agent_id: "claude".to_string(),
            model: Some("claude-opus-4-1".to_string()),
            reasoning: None,
            ..t254_source_launch_profile()
        }
    }

    fn t254_control_command(
        action: T254ControlAction,
        project_root: &std::path::Path,
        operation_id: &str,
        target: &T254ControlTarget,
    ) -> IssueCommand {
        match action {
            T254ControlAction::Stop => IssueCommand::MonitorStop {
                project_root: Some(project_root.to_path_buf()),
                number: target.issue_number,
                operation_id: Some(operation_id.to_string()),
                reason: target.reason.clone(),
                launch_generation: target.launch_generation,
                claim_id: target.claim_id.clone(),
                claim_owner: target.claim_owner.clone(),
                delivery_id: target.delivery_id.clone(),
                materializer_window_id: target.materializer_window_id.clone(),
                window_id: target.window_id.clone(),
            },
            T254ControlAction::Failover => IssueCommand::MonitorFailover {
                project_root: Some(project_root.to_path_buf()),
                number: target.issue_number,
                operation_id: Some(operation_id.to_string()),
                reason: target.reason.clone(),
                launch_generation: target.launch_generation,
                claim_id: target.claim_id.clone(),
                claim_owner: target.claim_owner.clone(),
                delivery_id: target.delivery_id.clone(),
                materializer_window_id: target.materializer_window_id.clone(),
                window_id: target.window_id.clone(),
            },
            T254ControlAction::Recover => IssueCommand::MonitorRecover {
                project_root: Some(project_root.to_path_buf()),
                number: target.issue_number,
                operation_id: operation_id.to_string(),
                reason: target.reason.clone(),
                launch_generation: target.launch_generation.unwrap_or_default(),
                claim_id: target.claim_id.clone(),
                claim_owner: target.claim_owner.clone(),
                delivery_id: target.delivery_id.clone(),
                materializer_window_id: target.materializer_window_id.clone(),
                window_id: target.window_id.clone(),
            },
        }
    }

    fn t254_seed_control_fixture(repo: &std::path::Path) -> std::path::PathBuf {
        t254_seed_control_fixture_with_origin(repo, None)
    }

    fn t254_seed_control_fixture_with_origin(
        repo: &std::path::Path,
        origin: Option<&str>,
    ) -> std::path::PathBuf {
        std::fs::create_dir_all(repo).expect("create fixture repo");
        crate::cli::trusted_store::init_git_repo_with_origin(repo);
        if let Some(origin) = origin {
            let status = gwt_core::process::hidden_command("git")
                .args(["remote", "set-url", "origin", origin])
                .current_dir(repo)
                .status()
                .expect("set fixture origin");
            assert!(status.success(), "set fixture origin to {origin}");
        }

        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        crate::cli::execution_state::save(
            repo,
            &crate::cli::execution_state::ExecutionControlRecord {
                owner_kind: owner.kind,
                owner_number: owner.number,
                primary_session_id: "source-agent-session".to_string(),
                entrypoint: "gwt-execute".to_string(),
                bundled_required_owners: Vec::new(),
                status: crate::cli::execution_state::ExecutionControlStatus::Active,
                blocked_reason: None,
                missing_verification: None,
                launched_at: chrono::Utc::now(),
                settled_at: None,
                transfers: Vec::new(),
                recoveries: Vec::new(),
                content_hash: String::new(),
            },
        )
        .expect("seed canonical execution projection");
        crate::cli::execution_state::ensure_generation_ledger(
            repo,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .expect("materialize execution generation ledger");
        let execution_binding = crate::cli::execution_state::current_execution_binding(repo, owner)
            .expect("read current execution generation")
            .expect("current execution generation exists");
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut source_session =
            gwt_agent::Session::new(repo, "work/issue-42", gwt_agent::AgentId::Codex);
        source_session.id = "source-agent-session".to_string();
        source_session.project_state_root = Some(repo.to_path_buf());
        source_session.linked_issue_number = Some(42);
        source_session
            .set_execution_binding(Some(gwt_agent::SessionExecutionBinding {
                schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
                session_id: source_session.id.clone(),
                repo_hash: source_session
                    .repo_hash
                    .clone()
                    .expect("fixture Session has repo hash"),
                owner_kind: owner.kind.as_str().to_string(),
                owner_number: owner.number,
                identity: execution_binding,
                capability_generation: 1,
            }))
            .expect("bind source Session to current execution generation");
        source_session
            .save(&sessions_dir)
            .expect("persist source Session without a runtime sidecar");

        let launch_profile = t254_source_launch_profile();
        let profile_fingerprint = t254_launch_profile_digest(&launch_profile);
        assert_eq!(
            profile_fingerprint, "8eb6d1861daf19a979a38ea80ae79108f822c952e9742fddaf66db16005cb9c1",
            "the pinned source profile fixture must remain byte stable"
        );
        assert!(t254_is_canonical_sha256(&profile_fingerprint));
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(launch_profile.clone()),
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-work::agent-7".to_string(),
                }],
                launched_control_identities: vec![
                    crate::issue_monitor::IssueMonitorLaunchedControlIdentity {
                        issue_number: 42,
                        window_id: "tab-work::agent-7".to_string(),
                        claim_id: "claim-42-generation-7".to_string(),
                        claim_owner: "source-agent-session".to_string(),
                        delivery_id: "delivery-42-generation-7".to_string(),
                        materializer_window_id: Some("tab-pm::materializer-7".to_string()),
                        launch_generation: 7,
                        launch_profile_snapshot: Some(launch_profile.clone()),
                        launch_profile_fingerprint: Some(profile_fingerprint),
                    },
                ],
                launch_generations: std::collections::BTreeMap::from([(42, 7)]),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed modern control prefs");
        set_issue_monitor_control_runtime_inventory(
            repo,
            crate::IssueMonitorRuntimeInventory::Available {
                project_scope: gwt_core::paths::project_scope_hash(repo)
                    .as_str()
                    .to_string(),
                runtime_instance_id: "t254-runtime".to_string(),
                revision: 1,
                observed_at: "2026-08-20T10:00:02Z".to_string(),
                windows: vec![crate::IssueMonitorRuntimeWindow {
                    window_id: "tab-work::agent-7".to_string(),
                    pane_state: crate::IssueMonitorPaneState::Stopped,
                    wait_signal: None,
                }],
            },
        );
        prefs_path
    }

    fn t254_is_canonical_sha256(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn t254_register_replacement_pm(repo: &std::path::Path) {
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(repo);
        let registration = |session_id: &str| crate::pm_registry::PmRegistration {
            session_id: session_id.to_string(),
            agent_id: "codex".to_string(),
            worktree_path: repo.to_string_lossy().into_owned(),
            created_at: None,
            consecutive_crashes: 0,
            next_not_before: None,
        };
        crate::pm_registry::try_register_pm(&prefs_path, registration("replaced-pm"), |_| false)
            .expect("register old PM");
        crate::pm_registry::try_register_pm(&prefs_path, registration("current-pm"), |_| false)
            .expect("replace old PM");
    }

    fn t254_replace_current_pm(repo: &std::path::Path, session_id: &str) {
        let prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(repo);
        crate::pm_registry::try_register_pm(
            &prefs_path,
            crate::pm_registry::PmRegistration {
                session_id: session_id.to_string(),
                agent_id: "codex".to_string(),
                worktree_path: repo.to_string_lossy().into_owned(),
                created_at: None,
                consecutive_crashes: 0,
                next_not_before: None,
            },
            |_| false,
        )
        .expect("replace current PM registration");
    }

    fn t254_authority_bytes(repo: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let trusted_dir = crate::cli::trusted_store::trusted_dir_for_worktree(repo)
            .expect("fixture has trusted execution store");
        let trusted_root = trusted_dir.parent().expect("trusted root");
        let paths = [
            (
                "trusted_projection",
                trusted_dir.join("execution-control.json"),
            ),
            (
                "trusted_pointer",
                trusted_dir.join("execution-generation-pointer.json"),
            ),
            (
                "owner_ledger",
                trusted_root
                    .join("execution-owners")
                    .join("owner-42")
                    .join("generation-ledger.json"),
            ),
            (
                "projection_mirror",
                repo.join(crate::cli::execution_state::EXECUTION_CONTROL_STATE_RELATIVE),
            ),
            (
                "pointer_mirror",
                repo.join(crate::cli::execution_state::EXECUTION_GENERATION_POINTER_STATE_RELATIVE),
            ),
        ];
        paths
            .into_iter()
            .map(|(label, path)| {
                (
                    label.to_string(),
                    std::fs::read(&path).unwrap_or_else(|error| {
                        panic!("read {label} at {}: {error}", path.display())
                    }),
                )
            })
            .collect()
    }

    fn t254_optional_file_bytes(path: &std::path::Path) -> Option<Vec<u8>> {
        match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("read {}: {error}", path.display()),
        }
    }

    fn t254_pm_control_receipt_snapshot(
        prefs_path: &std::path::Path,
        operation_id: &str,
    ) -> (usize, Vec<u8>, serde_json::Value) {
        let prefs = crate::load_issue_monitor_prefs(prefs_path).expect("load PM control receipts");
        let state =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        let receipts = state.pm_control_receipts();
        assert!(
            receipts.len() <= crate::issue_monitor::PM_CONTROL_RECEIPT_LIMIT,
            "PM control receipt history must remain bounded"
        );
        let receipt = state
            .pm_control_receipt(operation_id)
            .unwrap_or_else(|| panic!("lookup retained receipt {operation_id} by operation ID"));
        (
            receipts.len(),
            serde_json::to_vec(receipts).expect("serialize the entire PM receipt collection"),
            serde_json::to_value(receipt).expect("serialize selected PM control receipt"),
        )
    }

    struct T254ReceiptExpectation<'a> {
        repo: &'a std::path::Path,
        action: T254ControlAction,
        operation_id: &'a str,
        target: &'a T254ControlTarget,
        actor_session: &'a str,
        pm_registration_generation: u64,
        pinned_profile_digest: &'a str,
    }

    fn t254_assert_full_receipt(
        response: &serde_json::Value,
        expected: T254ReceiptExpectation<'_>,
    ) -> serde_json::Value {
        let T254ReceiptExpectation {
            repo,
            action,
            operation_id,
            target,
            actor_session,
            pm_registration_generation,
            pinned_profile_digest,
        } = expected;
        let receipt = response
            .get("receipt")
            .and_then(serde_json::Value::as_object)
            .expect("success includes a non-null receipt object");
        assert_eq!(
            receipt.get("operation_id"),
            Some(&serde_json::json!(operation_id))
        );
        assert_eq!(
            receipt.get("action"),
            Some(&serde_json::json!(action.as_str()))
        );
        assert_eq!(
            receipt.get("actor_session"),
            Some(&serde_json::json!(actor_session))
        );
        assert_eq!(
            receipt.get("pm_registration_generation"),
            Some(&serde_json::json!(pm_registration_generation))
        );
        assert_eq!(receipt.get("issue"), Some(&serde_json::json!(42)));
        let expected_fingerprint = t254_receipt_fingerprint(
            repo,
            action,
            actor_session,
            pm_registration_generation,
            pinned_profile_digest,
            target,
        );
        assert_eq!(
            receipt.get("target_fingerprint"),
            Some(&serde_json::json!(expected_fingerprint))
        );
        assert!(t254_is_canonical_sha256(&expected_fingerprint));
        assert_eq!(
            receipt.get("reason"),
            Some(&serde_json::json!(target.reason))
        );
        assert_eq!(
            receipt.get("outcome"),
            Some(&serde_json::json!(action.expected_initial_outcome()))
        );
        let requested_at = receipt
            .get("requested_at")
            .and_then(serde_json::Value::as_str)
            .expect("receipt has a request timestamp");
        chrono::DateTime::parse_from_rfc3339(requested_at)
            .expect("requested_at is canonical RFC3339");
        let settled_at = receipt
            .get("settled_at")
            .expect("receipt explicitly represents its settlement timestamp");
        match action {
            T254ControlAction::Stop => assert!(
                settled_at.is_null(),
                "the stop hold is immediate, but cleanup settlement remains pending"
            ),
            T254ControlAction::Failover | T254ControlAction::Recover => assert!(
                settled_at.is_null(),
                "pending failover/recover receipts must have exactly null settled_at"
            ),
        }
        serde_json::Value::Object(receipt.clone())
    }

    fn t254_run_refused(
        env: &mut crate::cli::TestEnv,
        command: IssueCommand,
        operation_id: &str,
        diagnostic_key: &str,
        diagnostic_value: &str,
    ) -> serde_json::Value {
        let mut out = String::new();
        match run(env, command, &mut out) {
            Ok(code) => {
                assert_ne!(code, 0, "refusal must not report success: {out}");
            }
            Err(error) => panic!(
                "definitive refusal must use the stable JSON diagnostic contract: {error}; {out}"
            ),
        }
        let diagnostic: serde_json::Value =
            serde_json::from_str(out.trim()).expect("refusal is one JSON object");
        assert_eq!(
            diagnostic,
            serde_json::json!({
                "status": "refused",
                "operation_id": operation_id,
                "number": 42,
                (diagnostic_key): diagnostic_value,
            }),
            "refusals use one stable, secret-free JSON shape"
        );
        diagnostic
    }

    /// SPEC-3431 FR-128 / AS-PM-CONTROL-AUTH-001: authority is the ambient,
    /// current PM registration, not a caller-provided Session or a historical
    /// PM identity. The check precedes both Monitor and execution authority.
    #[test]
    fn t254_control_operations_require_the_current_registered_pm_before_any_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        t254_register_replacement_pm(&repo);
        let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        let target = t254_exact_control_target();

        for action in [
            T254ControlAction::Stop,
            T254ControlAction::Failover,
            T254ControlAction::Recover,
        ] {
            for (label, session_id) in [
                ("missing", None),
                ("ordinary", Some("ordinary-agent")),
                ("replaced", Some("replaced-pm")),
            ] {
                let before_prefs = std::fs::read(&prefs_path).expect("prefs bytes");
                let before_pm = std::fs::read(&pm_prefs_path).expect("PM prefs bytes");
                let before_authority = t254_authority_bytes(&repo);
                let _session = session_id.map_or_else(
                    || gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
                    |session_id| {
                        gwt_core::test_support::ScopedEnvVar::set(
                            gwt_agent::GWT_SESSION_ID_ENV,
                            session_id,
                        )
                    },
                );
                let operation_id = format!("t254-auth-{action:?}-{label}");
                let command = t254_control_command(action, &repo, &operation_id, &target);
                t254_run_refused(
                    &mut crate::cli::TestEnv::new(repo.clone()),
                    command,
                    &operation_id,
                    "refusal",
                    "pm_authority",
                );
                assert_eq!(
                    std::fs::read(&prefs_path).expect("prefs bytes"),
                    before_prefs,
                    "{action:?}/{label} must not mutate Issue Monitor prefs"
                );
                assert_eq!(
                    std::fs::read(&pm_prefs_path).expect("PM prefs bytes"),
                    before_pm,
                    "{action:?}/{label} must not rewrite pm.json"
                );
                assert_eq!(
                    t254_authority_bytes(&repo),
                    before_authority,
                    "{action:?}/{label} must not mutate execution authority"
                );
            }
        }

        let foreign = tmp.path().join("foreign-repo");
        let foreign_prefs_path = t254_seed_control_fixture_with_origin(
            &foreign,
            Some("https://example.com/t/foreign-control-project.git"),
        );
        let foreign_pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&foreign);
        let _current_pm =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        for action in [
            T254ControlAction::Stop,
            T254ControlAction::Failover,
            T254ControlAction::Recover,
        ] {
            let before_foreign_prefs = std::fs::read(&foreign_prefs_path).expect("foreign prefs");
            let before_foreign_pm = t254_optional_file_bytes(&foreign_pm_path);
            let before_foreign_authority = t254_authority_bytes(&foreign);
            let operation_id = format!("t254-auth-foreign-project-{}", action.as_str());
            t254_run_refused(
                &mut crate::cli::TestEnv::new(repo.clone()),
                t254_control_command(action, &foreign, &operation_id, &target),
                &operation_id,
                "refusal",
                "pm_authority",
            );
            assert_eq!(
                std::fs::read(&foreign_prefs_path).expect("foreign prefs"),
                before_foreign_prefs
            );
            assert_eq!(
                t254_optional_file_bytes(&foreign_pm_path),
                before_foreign_pm
            );
            assert_eq!(t254_authority_bytes(&foreign), before_foreign_authority);
        }
    }

    /// FR-128 lock order is PM prefs -> Issue Monitor prefs. Holding the PM
    /// generation guard through the Issue transaction prevents replacement
    /// from slipping between authorization and commit.
    #[test]
    fn t254_pm_generation_guard_serializes_replacement_after_issue_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        for action in [
            T254ControlAction::Stop,
            T254ControlAction::Failover,
            T254ControlAction::Recover,
        ] {
            let action_root = tmp.path().join(action.as_str());
            let _home = ScopedGwtHome::set(action_root.join("home"));
            let repo = action_root.join("repo");
            let issue_prefs_path = t254_seed_control_fixture(&repo);
            t254_register_replacement_pm(&repo);
            let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
            let pm_before = std::fs::read(&pm_prefs_path).expect("PM prefs before race");
            let issue_before = std::fs::read(&issue_prefs_path).expect("Issue prefs before race");
            let authority_before = t254_authority_bytes(&repo);
            let _session = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_ID_ENV,
                "current-pm",
            );

            let (lease_entered_tx, lease_entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let release_rx = std::sync::Arc::new(std::sync::Mutex::new(release_rx));
            // This test-only seam is invoked by the shared stop/failover/recover
            // handler core after it owns the real PM lease and Issue lock, but
            // before it mutates the candidate. It must not wrap the lock
            // primitive directly or the handler could regress independently.
            let _pre_admission_barrier = if !matches!(action, T254ControlAction::Recover) {
                let lease_entered_tx = lease_entered_tx.clone();
                let release_rx = std::sync::Arc::clone(&release_rx);
                Some(install_issue_monitor_control_lease_test_hook(move || {
                    lease_entered_tx
                        .send(())
                        .expect("announce handler PM lease + Issue lock");
                    release_rx
                        .lock()
                        .expect("release receiver lock")
                        .recv()
                        .expect("release real control handler");
                }))
            } else {
                None
            };
            let _post_admission_barrier = if matches!(action, T254ControlAction::Recover) {
                let release_rx = std::sync::Arc::clone(&release_rx);
                Some(install_issue_monitor_control_post_admission_test_hook(
                    move || {
                        lease_entered_tx
                            .send(())
                            .expect("announce Recover post-admission PM lease");
                        release_rx
                            .lock()
                            .expect("release receiver lock")
                            .recv()
                            .expect("release real Recover handler");
                    },
                ))
            } else {
                None
            };
            let control_repo = repo.clone();
            let control_home = action_root.join("home");
            let target = t254_exact_control_target();
            let operation_id = format!("t254-lock-order-{}", action.as_str());
            let control = std::thread::spawn(move || {
                let _home = ScopedGwtHome::set(control_home);
                let mut out = String::new();
                let code = run(
                    &mut crate::cli::TestEnv::new(control_repo.clone()),
                    t254_control_command(action, &control_repo, &operation_id, &target),
                    &mut out,
                )
                .expect("real PM control handler runs");
                (code, out, operation_id)
            });
            lease_entered_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("real handler reached the in-lease pre-mutation barrier");

            let replacement_pm_path = pm_prefs_path.clone();
            let replacement_repo = repo.clone();
            let replacement_home = action_root.join("home");
            let (replacement_started_tx, replacement_started_rx) = std::sync::mpsc::channel();
            let (replacement_done_tx, replacement_done_rx) = std::sync::mpsc::channel();
            let replacement = std::thread::spawn(move || {
                let _home = ScopedGwtHome::set(replacement_home);
                replacement_started_tx
                    .send(())
                    .expect("announce replacement attempt");
                let result = crate::pm_registry::try_register_pm(
                    &replacement_pm_path,
                    crate::pm_registry::PmRegistration {
                        session_id: "replacement-after-control".to_string(),
                        agent_id: "codex".to_string(),
                        worktree_path: replacement_repo.to_string_lossy().into_owned(),
                        created_at: None,
                        consecutive_crashes: 0,
                        next_not_before: None,
                    },
                    |_| false,
                );
                replacement_done_tx
                    .send(result)
                    .expect("publish replacement outcome");
            });
            replacement_started_rx.recv().expect("replacement started");
            assert!(
                replacement_done_rx
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "PM replacement must block while {action:?} owns its real nested lease"
            );
            assert_eq!(
                std::fs::read(&pm_prefs_path).expect("PM prefs during race"),
                pm_before,
                "blocked replacement must leave pm.json byte-identical"
            );
            let issue_during = std::fs::read(&issue_prefs_path).expect("Issue prefs during race");
            if matches!(action, T254ControlAction::Recover) {
                assert_ne!(
                    issue_during, issue_before,
                    "Recover barrier is deliberately after durable admission"
                );
                let pending = crate::load_issue_monitor_prefs(&issue_prefs_path)
                    .expect("load admitted Recover during race");
                assert!(pending.pending_controls.iter().any(|control| {
                    control.operation_id == "t254-lock-order-recover"
                        && control
                            .execution_settlement
                            .as_ref()
                            .is_some_and(|settlement| !settlement.settled)
                }));
            } else {
                assert_eq!(
                    issue_during, issue_before,
                    "the handler has not committed before its barrier releases"
                );
            }
            assert_eq!(t254_authority_bytes(&repo), authority_before);

            release_tx.send(()).expect("release control handler");
            let (code, out, operation_id) = control.join().expect("control thread joins");
            assert_eq!(code, 0, "{action:?} succeeds before replacement: {out}");
            let response: serde_json::Value =
                serde_json::from_str(out.trim()).expect("control response JSON");
            t254_assert_full_receipt(
                &response,
                T254ReceiptExpectation {
                    repo: &repo,
                    action,
                    operation_id: &operation_id,
                    target: &t254_exact_control_target(),
                    actor_session: "current-pm",
                    pm_registration_generation: 2,
                    pinned_profile_digest: &t254_launch_profile_digest(
                        &t254_source_launch_profile(),
                    ),
                },
            );
            replacement_done_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("replacement completes only after the control commit")
                .expect("replacement result");
            replacement.join().expect("replacement thread joins");
            let pm_after = crate::pm_registry::load_pm_prefs(&pm_prefs_path)
                .expect("read replacement PM prefs");
            assert_eq!(pm_after.registration_generation, 3);
            assert_eq!(
                pm_after
                    .registration
                    .as_ref()
                    .map(|registration| registration.session_id.as_str()),
                Some("replacement-after-control")
            );
        }
    }

    #[test]
    fn t254_execution_authority_fixture_is_a_strict_current_generation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        t254_seed_control_fixture(&repo);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        let ledger = crate::cli::execution_state::load_generation_ledger(&repo, owner)
            .expect("strict authority read")
            .expect("strict generation ledger exists");
        assert_eq!(ledger.owner, owner);
        assert!(ledger.current_generation().is_some());
        let raw = t254_authority_bytes(&repo);
        assert_eq!(raw.len(), 5);
        assert!(raw.values().all(|bytes| !bytes.is_empty()));
        assert_eq!(
            raw.get("trusted_projection"),
            raw.get("projection_mirror"),
            "trusted ECR and mirror must be the same lifecycle projection"
        );
        assert_eq!(
            raw.get("trusted_pointer"),
            raw.get("pointer_mirror"),
            "trusted pointer and mirror must identify the same current generation"
        );
    }

    #[test]
    fn t254_status_marks_recover_ready_only_after_exact_execution_preflight() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load monitor prefs");
        let mut monitor =
            crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: Vec::new(),
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
                readiness: crate::IssueMonitorReadiness::NotApplicable,
                updated_at: Some("2026-08-20T10:00:00Z".to_string()),
            }],
            "2026-08-20T10:00:01Z",
        );
        let mut status = monitor.agent_status();
        status.apply_runtime_inventory(&crate::IssueMonitorRuntimeInventory::Available {
            project_scope: gwt_core::paths::project_scope_hash(&repo)
                .as_str()
                .to_string(),
            runtime_instance_id: "runtime-status-3712".to_string(),
            revision: 1,
            observed_at: "2026-08-20T10:00:02Z".to_string(),
            windows: vec![crate::IssueMonitorRuntimeWindow {
                window_id: "tab-work::agent-7".to_string(),
                pane_state: crate::IssueMonitorPaneState::Stopped,
                wait_signal: None,
            }],
        });
        assert_eq!(
            status.inbox[0]
                .control_ready
                .recover
                .degraded_reason
                .as_deref(),
            Some("execution_unverified")
        );

        apply_execution_recovery_readiness(&repo, &mut status);
        assert_eq!(
            status.inbox[0].control_ready.recover,
            crate::IssueMonitorControlActionReadiness::ready(),
            "the one status response becomes actionable only after the same proof Recover consumes"
        );
    }

    /// SPEC-3431 FR-130 / AS-PM-CONTROL-RECOVER-001: first admission uses
    /// both authoritative proofs. A terminal ECR cannot override a live exact
    /// AppRuntime pane, and refusal is byte-identical across both authorities.
    #[test]
    fn t254_recover_refuses_live_runtime_before_first_admission_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        t254_register_replacement_pm(&repo);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        set_issue_monitor_control_runtime_inventory(
            &repo,
            crate::IssueMonitorRuntimeInventory::Available {
                project_scope: gwt_core::paths::project_scope_hash(&repo)
                    .as_str()
                    .to_string(),
                runtime_instance_id: "t254-live-runtime".to_string(),
                revision: 2,
                observed_at: "2026-08-20T10:00:03Z".to_string(),
                windows: vec![crate::IssueMonitorRuntimeWindow {
                    window_id: "tab-work::agent-7".to_string(),
                    pane_state: crate::IssueMonitorPaneState::Running,
                    wait_signal: None,
                }],
            },
        );
        let before_prefs = std::fs::read(&prefs_path).expect("prefs bytes");
        let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        let before_pm = std::fs::read(&pm_path).expect("PM prefs bytes");
        let before_authority = t254_authority_bytes(&repo);
        let target = t254_exact_control_target();
        let operation_id = "t254-recover-live-runtime";

        t254_run_refused(
            &mut crate::cli::TestEnv::new(repo.clone()),
            t254_control_command(T254ControlAction::Recover, &repo, operation_id, &target),
            operation_id,
            "refusal",
            "runtime_live",
        );

        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            before_prefs
        );
        assert_eq!(std::fs::read(&pm_path).expect("PM prefs bytes"), before_pm);
        assert_eq!(t254_authority_bytes(&repo), before_authority);
    }

    #[test]
    fn t254_recover_runtime_inventory_gate_accepts_only_exact_terminal_or_absent() {
        let target = crate::IssueMonitorStopTarget {
            issue_number: 42,
            launch_generation: Some(7),
            claim_id: Some("claim-42-generation-7".to_string()),
            claim_owner: Some("source-agent-session".to_string()),
            delivery_id: Some("delivery-42-generation-7".to_string()),
            materializer_window_id: Some("tab-pm::materializer-7".to_string()),
            window_id: Some("tab-work::agent-7".to_string()),
        };
        let available = |windows| crate::IssueMonitorRuntimeInventory::Available {
            project_scope: "project-3712".to_string(),
            runtime_instance_id: "runtime-3712".to_string(),
            revision: 1,
            observed_at: "2026-08-20T10:00:03Z".to_string(),
            windows,
        };
        let window = |pane_state| crate::IssueMonitorRuntimeWindow {
            window_id: "tab-work::agent-7".to_string(),
            pane_state,
            wait_signal: None,
        };

        assert_eq!(
            recover_runtime_inventory_refusal(&available(vec![]), &target),
            None
        );
        for pane_state in [
            crate::IssueMonitorPaneState::Stopped,
            crate::IssueMonitorPaneState::Error,
        ] {
            assert_eq!(
                recover_runtime_inventory_refusal(&available(vec![window(pane_state)]), &target),
                None
            );
        }
        for pane_state in [
            crate::IssueMonitorPaneState::Running,
            crate::IssueMonitorPaneState::Idle,
            crate::IssueMonitorPaneState::Waiting,
        ] {
            assert_eq!(
                recover_runtime_inventory_refusal(&available(vec![window(pane_state)]), &target),
                Some("runtime_live")
            );
        }
        assert_eq!(
            recover_runtime_inventory_refusal(
                &available(vec![
                    window(crate::IssueMonitorPaneState::Stopped),
                    window(crate::IssueMonitorPaneState::Error),
                ]),
                &target,
            ),
            Some("runtime_ambiguous")
        );
        assert_eq!(
            recover_runtime_inventory_refusal(
                &crate::IssueMonitorRuntimeInventory::Unavailable {
                    project_scope: "project-3712".to_string(),
                    observed_at: "2026-08-20T10:00:03Z".to_string(),
                    reason: "AppRuntime unavailable".to_string(),
                },
                &target,
            ),
            Some("runtime_inventory_unavailable")
        );
    }

    #[test]
    fn t254_registered_pm_can_failover_and_recover_with_a_persisted_full_receipt() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");

        for (action, action_name) in [
            (T254ControlAction::Failover, "failover"),
            (T254ControlAction::Recover, "recover"),
        ] {
            let action_root = tmp.path().join(action_name);
            let _home = ScopedGwtHome::set(action_root.join("home"));
            let repo = action_root.join("repo");
            let origin = format!("https://example.com/t/t254-{action_name}.git");
            let prefs_path = t254_seed_control_fixture_with_origin(&repo, Some(origin.as_str()));
            t254_register_replacement_pm(&repo);
            if matches!(action, T254ControlAction::Recover) {
                crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                    prefs.launch_profile = Some(t254_changed_global_launch_profile());
                    Ok(())
                })
                .expect("edit the global profile after the launch ACK");
                let after_edit = crate::load_issue_monitor_prefs(&prefs_path)
                    .expect("roundtrip changed global profile");
                assert_eq!(
                    after_edit.launch_profile,
                    Some(t254_changed_global_launch_profile())
                );
                assert_ne!(
                    after_edit.launch_profile,
                    Some(t254_source_launch_profile()),
                    "the recover test must distinguish old per-launch intent from the current global profile"
                );
            }
            let _session = gwt_core::test_support::ScopedEnvVar::set(
                gwt_agent::GWT_SESSION_ID_ENV,
                "current-pm",
            );
            let target = t254_exact_control_target();
            let operation_id = format!("t254-registered-{action_name}");
            let mut out = String::new();
            assert_eq!(
                run(
                    &mut crate::cli::TestEnv::new(repo.clone()),
                    t254_control_command(action, &repo, &operation_id, &target),
                    &mut out,
                )
                .expect("registered PM control"),
                0,
                "registered PM {action_name} must succeed: {out}"
            );
            let response: serde_json::Value =
                serde_json::from_str(out.trim()).expect("control success JSON");
            let response_receipt = t254_assert_full_receipt(
                &response,
                T254ReceiptExpectation {
                    repo: &repo,
                    action,
                    operation_id: &operation_id,
                    target: &target,
                    actor_session: "current-pm",
                    pm_registration_generation: 2,
                    pinned_profile_digest: &t254_launch_profile_digest(
                        &t254_source_launch_profile(),
                    ),
                },
            );
            let (receipt_count, _, persisted_receipt) =
                t254_pm_control_receipt_snapshot(&prefs_path, &operation_id);
            assert_eq!(receipt_count, 1);
            assert_eq!(persisted_receipt, response_receipt);

            if matches!(action, T254ControlAction::Recover) {
                let prefs = crate::load_issue_monitor_prefs(&prefs_path)
                    .expect("recover pending survives prefs roundtrip");
                assert_eq!(
                    prefs.launch_profile,
                    Some(t254_changed_global_launch_profile()),
                    "recover must not rewrite the concurrently edited global profile"
                );
                let persisted = serde_json::to_value(prefs).expect("serialize recovery pending");
                let pending = persisted["pending_controls"]
                    .as_array()
                    .and_then(|pending| {
                        pending
                            .iter()
                            .find(|entry| entry["operation_id"] == serde_json::json!(operation_id))
                    })
                    .expect("recover persists pending state by operation ID");
                assert_eq!(
                    pending["next_launch_profile"],
                    serde_json::to_value(t254_source_launch_profile())
                        .expect("serialize source launch profile"),
                    "recover pins the per-launch source snapshot, not the later global profile"
                );
                assert_eq!(
                    pending["execution_settlement"]["settled"],
                    serde_json::json!(true),
                    "Recover remains pending on teardown, but its exact ECR prerequisite is durable"
                );
                assert!(
                    response["execution_settlement"].is_object(),
                    "the successful response must expose the exact ECR settlement receipt"
                );
                assert_eq!(
                    crate::cli::execution_state::load(&repo)
                        .expect("load settled execution projection")
                        .expect("execution projection exists")
                        .status,
                    crate::cli::execution_state::ExecutionControlStatus::Blocked,
                    "Recover success cannot leave the owner generation Active"
                );
            }
        }
    }

    /// #3712 / FR-130: a Recover whose execution ledger commit becomes
    /// outcome-ambiguous is not successful and cannot queue a successor. The
    /// same operation repairs/replays that exact receipt before closing the
    /// Monitor prerequisite.
    #[test]
    fn t254_recover_execution_settlement_failure_is_nonzero_and_exact_replay_converges() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        t254_register_replacement_pm(&repo);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        let target = t254_exact_control_target();
        let operation_id = "t254-recover-partial-execution";

        crate::cli::execution_state::set_generation_write_failure_after_ledger();
        let mut first_out = String::new();
        assert_eq!(
            run(
                &mut crate::cli::TestEnv::new(repo.clone()),
                t254_control_command(T254ControlAction::Recover, &repo, operation_id, &target,),
                &mut first_out,
            )
            .expect("Recover reports the injected execution failure"),
            1,
            "an ambiguous ECR settlement cannot be reported as success: {first_out}"
        );
        assert!(
            first_out.contains("\"status\":\"execution_settlement_failed\""),
            "{first_out}"
        );
        let after_failure = crate::load_issue_monitor_prefs(&prefs_path)
            .expect("load pending Recover after execution failure");
        assert!(after_failure.queued_launch_overrides.is_empty());
        assert!(after_failure.pending_controls[0]
            .execution_settlement
            .as_ref()
            .is_some_and(|settlement| !settlement.settled));
        std::fs::remove_file(gwt_core::paths::gwt_sessions_dir().join("source-agent-session.toml"))
            .expect("remove ephemeral source Session before response-loss replay");

        let mut replay_out = String::new();
        assert_eq!(
            run(
                &mut crate::cli::TestEnv::new(repo.clone()),
                t254_control_command(T254ControlAction::Recover, &repo, operation_id, &target,),
                &mut replay_out,
            )
            .expect("exact Recover replay repairs publication"),
            0,
            "exact replay must converge the committed ECR receipt: {replay_out}"
        );
        let after_replay = crate::load_issue_monitor_prefs(&prefs_path)
            .expect("load converged Recover prerequisite");
        assert_eq!(
            after_replay
                .pm_control_receipts
                .iter()
                .filter(|receipt| receipt.operation_id == operation_id)
                .count(),
            1,
            "replay must not duplicate the PM receipt"
        );
        assert!(after_replay.pending_controls[0]
            .execution_settlement
            .as_ref()
            .is_some_and(|settlement| settlement.settled));
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        assert!(
            crate::cli::execution_state::load_generation_ledger(&repo, owner)
                .expect("strict generation publication repaired")
                .is_some()
        );
        assert_eq!(
            crate::cli::execution_state::load(&repo)
                .expect("load replayed execution projection")
                .expect("execution projection exists")
                .status,
            crate::cli::execution_state::ExecutionControlStatus::Blocked
        );
    }

    /// SPEC-3431 FR-128 / AS-PM-CONTROL-TARGET-001: all six durable identity
    /// fields are exact-match fences. A stale value is never a wildcard and
    /// no refusal may append a receipt or advance either authority store.
    #[test]
    fn t254_stop_and_failover_refuse_each_stale_full_target_component_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        t254_register_replacement_pm(&repo);
        let _session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        let exact = t254_exact_control_target();
        let mismatches = [
            (
                "generation_mismatch",
                T254ControlTarget {
                    launch_generation: Some(6),
                    ..exact.clone()
                },
            ),
            (
                "generation_mismatch",
                T254ControlTarget {
                    launch_generation: None,
                    ..exact.clone()
                },
            ),
            (
                "claim_mismatch",
                T254ControlTarget {
                    claim_id: Some("stale-claim".to_string()),
                    ..exact.clone()
                },
            ),
            (
                "claim_mismatch",
                T254ControlTarget {
                    claim_id: None,
                    ..exact.clone()
                },
            ),
            (
                "claim_owner_mismatch",
                T254ControlTarget {
                    claim_owner: Some("foreign/owner".to_string()),
                    ..exact.clone()
                },
            ),
            (
                "claim_owner_mismatch",
                T254ControlTarget {
                    claim_owner: None,
                    ..exact.clone()
                },
            ),
            (
                "delivery_mismatch",
                T254ControlTarget {
                    delivery_id: Some("stale-delivery".to_string()),
                    ..exact.clone()
                },
            ),
            (
                "delivery_mismatch",
                T254ControlTarget {
                    delivery_id: None,
                    ..exact.clone()
                },
            ),
            (
                "materializer_window_mismatch",
                T254ControlTarget {
                    materializer_window_id: Some("tab-pm::stale-materializer".to_string()),
                    ..exact.clone()
                },
            ),
            (
                "materializer_window_mismatch",
                T254ControlTarget {
                    materializer_window_id: None,
                    ..exact.clone()
                },
            ),
            (
                "window_mismatch",
                T254ControlTarget {
                    window_id: Some("tab-work::stale-agent".to_string()),
                    ..exact
                },
            ),
            (
                "window_mismatch",
                T254ControlTarget {
                    window_id: None,
                    ..t254_exact_control_target()
                },
            ),
        ];

        for action in [
            T254ControlAction::Stop,
            T254ControlAction::Failover,
            T254ControlAction::Recover,
        ] {
            for (case_index, (expected_mismatch, target)) in mismatches.iter().enumerate() {
                let before_prefs = std::fs::read(&prefs_path).expect("prefs bytes");
                let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
                let before_pm = std::fs::read(&pm_path).expect("PM prefs bytes");
                let before_authority = t254_authority_bytes(&repo);
                let operation_id =
                    format!("t254-target-{action:?}-{expected_mismatch}-{case_index}");
                let command = t254_control_command(action, &repo, &operation_id, target);
                t254_run_refused(
                    &mut crate::cli::TestEnv::new(repo.clone()),
                    command,
                    &operation_id,
                    "mismatch",
                    expected_mismatch,
                );
                assert_eq!(
                    std::fs::read(&prefs_path).expect("prefs bytes"),
                    before_prefs,
                    "{action:?}/{expected_mismatch} must be zero-mutation"
                );
                assert_eq!(
                    std::fs::read(&pm_path).expect("PM prefs bytes"),
                    before_pm,
                    "target refusal must not rewrite pm.json"
                );
                assert_eq!(
                    t254_authority_bytes(&repo),
                    before_authority,
                    "{action:?}/{expected_mismatch} must preserve execution authority"
                );
            }
        }
    }

    /// SPEC-3431 FR-129 / AS-PM-CONTROL-REPLAY-001: operation replay is
    /// receipt-based, not the historical Issue-number `already_stopped`
    /// shortcut. Exact replay returns the committed receipt; conflicting reuse
    /// is refused before any second transition.
    #[test]
    fn t254_operation_id_replay_returns_the_same_receipt_and_conflict_is_zero_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        let prefs_path = t254_seed_control_fixture(&repo);
        t254_register_replacement_pm(&repo);
        let current_session =
            gwt_core::test_support::ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "current-pm");
        let target = t254_exact_control_target();
        let operation_id = "t254-stop-replay-42-generation-7";
        let pinned_profile_digest = t254_launch_profile_digest(&t254_source_launch_profile());
        assert_eq!(
            t254_receipt_fingerprint(
                &repo,
                T254ControlAction::Stop,
                "current-pm",
                2,
                &pinned_profile_digest,
                &target,
            ),
            "e462c6d8de238fdb82d09f16523fec568979e9ffe13201bd5b24e7586c1d3989",
            "the complete versioned receipt tuple must have one fixed canonical digest"
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());
        #[cfg(unix)]
        let command_root = {
            let symlink = tmp.path().join("canonical-repo-link");
            std::os::unix::fs::symlink(&repo, &symlink).expect("create canonical project symlink");
            symlink
        };
        #[cfg(not(unix))]
        let command_root = repo.clone();

        let mut first_out = String::new();
        assert_eq!(
            run(
                &mut env,
                t254_control_command(
                    T254ControlAction::Stop,
                    &command_root,
                    operation_id,
                    &target,
                ),
                &mut first_out,
            )
            .expect("first control attempt"),
            0
        );
        let first: serde_json::Value =
            serde_json::from_str(first_out.trim()).expect("first receipt JSON");
        let first_receipt = t254_assert_full_receipt(
            &first,
            T254ReceiptExpectation {
                repo: &repo,
                action: T254ControlAction::Stop,
                operation_id,
                target: &target,
                actor_session: "current-pm",
                pm_registration_generation: 2,
                pinned_profile_digest: &pinned_profile_digest,
            },
        );
        let (first_receipt_count, first_receipt_bytes, persisted_receipt) =
            t254_pm_control_receipt_snapshot(&prefs_path, operation_id);
        assert_eq!(first_receipt_count, 1);
        assert_eq!(persisted_receipt, first_receipt);
        let after_first_prefs = std::fs::read(&prefs_path).expect("prefs after first stop");
        let pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        let after_first_pm = std::fs::read(&pm_path).expect("PM prefs after first stop");
        let after_first_authority = t254_authority_bytes(&repo);

        let mut replay_out = String::new();
        assert_eq!(
            run(
                &mut env,
                t254_control_command(T254ControlAction::Stop, &repo, operation_id, &target),
                &mut replay_out,
            )
            .expect("exact replay"),
            0
        );
        let replay: serde_json::Value =
            serde_json::from_str(replay_out.trim()).expect("replay receipt JSON");
        assert!(
            first.get("receipt").is_some(),
            "control result must expose its durable receipt: {first}"
        );
        assert_eq!(replay.get("receipt"), first.get("receipt"));
        let (replay_receipt_count, replay_receipt_bytes, replay_persisted_receipt) =
            t254_pm_control_receipt_snapshot(&prefs_path, operation_id);
        assert_eq!(replay_receipt_count, first_receipt_count);
        assert_eq!(replay_receipt_bytes, first_receipt_bytes);
        assert_eq!(replay_persisted_receipt, first_receipt);
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs after replay"),
            after_first_prefs,
            "exact replay must not advance authority or append a second receipt"
        );
        assert_eq!(
            std::fs::read(&pm_path).expect("PM prefs after replay"),
            after_first_pm,
            "exact replay must not rewrite pm.json"
        );
        assert_eq!(t254_authority_bytes(&repo), after_first_authority);

        let conflict_inputs = [
            (T254ControlAction::Failover, t254_exact_control_target()),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    issue_number: 43,
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    reason: "different reason".to_string(),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    launch_generation: Some(8),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    claim_id: Some("different-claim".to_string()),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    claim_owner: Some("different-owner".to_string()),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    delivery_id: Some("different-delivery".to_string()),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    materializer_window_id: Some("different-materializer".to_string()),
                    ..t254_exact_control_target()
                },
            ),
            (
                T254ControlAction::Stop,
                T254ControlTarget {
                    window_id: Some("different-window".to_string()),
                    ..t254_exact_control_target()
                },
            ),
        ];
        for (action, conflicting) in conflict_inputs {
            t254_run_refused(
                &mut env,
                t254_control_command(action, &repo, operation_id, &conflicting),
                operation_id,
                "refusal",
                "operation_id_conflict",
            );
            let (count, receipt_bytes, receipt) =
                t254_pm_control_receipt_snapshot(&prefs_path, operation_id);
            assert_eq!(count, first_receipt_count);
            assert_eq!(receipt_bytes, first_receipt_bytes);
            assert_eq!(receipt, first_receipt);
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs after conflict"),
                after_first_prefs,
                "same ID with different fingerprint input must be zero-mutation"
            );
            assert_eq!(
                std::fs::read(&pm_path).expect("PM prefs after conflict"),
                after_first_pm
            );
            assert_eq!(t254_authority_bytes(&repo), after_first_authority);
        }

        drop(current_session);
        t254_replace_current_pm(&repo, "successor-pm");
        let after_replacement_pm = std::fs::read(&pm_path).expect("replacement PM prefs");
        let _successor_session = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::GWT_SESSION_ID_ENV,
            "successor-pm",
        );
        t254_run_refused(
            &mut env,
            t254_control_command(T254ControlAction::Stop, &repo, operation_id, &target),
            operation_id,
            "refusal",
            "operation_id_conflict",
        );
        let (count, receipt_bytes, receipt) =
            t254_pm_control_receipt_snapshot(&prefs_path, operation_id);
        assert_eq!(count, first_receipt_count);
        assert_eq!(receipt_bytes, first_receipt_bytes);
        assert_eq!(receipt, first_receipt);
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs after actor/generation conflict"),
            after_first_prefs,
            "a different PM actor/registration generation cannot replay a retained operation"
        );
        assert_eq!(
            std::fs::read(&pm_path).expect("PM prefs after actor/generation conflict"),
            after_replacement_pm,
            "the refusal must not rewrite the new PM registration"
        );
        assert_eq!(t254_authority_bytes(&repo), after_first_authority);
    }

    /// Issue #3645 AC-1 / #3628 AC-2: the recovery an operator reaches for when
    /// the row has no launch left. Reproduces the 2026-08-17 shape exactly — a
    /// persisted `agent_failed` hold with no `launched_issues` entry — because
    /// that is the state every identity-checked operation refuses.
    #[test]
    fn monitor_requeue_releases_a_dead_hold_and_refuses_a_live_launch() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![43, 42],
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 43,
                    window_id: "tab-1::agent-live".to_string(),
                }],
                failed_issues: vec![crate::IssueMonitorFailedIssue {
                    issue_number: 42,
                    message: "an execution generation already exists for issue #42".to_string(),
                    window_id: Some("tab-1::agent-dead".to_string()),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        // A live launch is refused with zero mutation: this operation exists for
        // rows nothing owns, and stop/failover own the rest.
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorRequeue {
                project_root: Some(repo.clone()),
                number: 43,
                reason: "operator recovery".to_string(),
            },
            &mut out,
        )
        .expect("requeue runs");
        assert_eq!(code, 1);
        assert!(out.contains("\"status\":\"refused\""), "{out}");
        assert!(out.contains("launch_live"), "{out}");
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            before,
            "a refused recovery must be zero-mutation"
        );

        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorRequeue {
                project_root: Some(repo.clone()),
                number: 42,
                reason: "operator recovery".to_string(),
            },
            &mut out,
        )
        .expect("requeue runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"requeued\""), "{out}");
        assert!(out.contains("tab-1::agent-dead"), "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.failed_issues.is_empty(),
            "the persisted hold must be gone"
        );
        assert_eq!(
            prefs
                .released_failures
                .iter()
                .map(|release| release.issue_number)
                .collect::<Vec<_>>(),
            vec![42],
            "the release must be published so other processes converge on it"
        );
        assert_eq!(
            prefs.launched_issues.len(),
            1,
            "the unrelated live launch must survive"
        );

        // Recovering an issue nothing is holding reports that, rather than
        // claiming a state change that did not happen.
        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorRequeue {
                project_root: Some(repo),
                number: 42,
                reason: "operator recovery".to_string(),
            },
            &mut out,
        )
        .expect("requeue runs");
        assert_eq!(code, 1);
        assert!(out.contains("not_held"), "{out}");
    }

    // -------------------------------------------------------------------
    // SPEC-1942 SC-025 follow-up: issue-family helper tests relocated
    // from cli.rs.
    // -------------------------------------------------------------------

    use crate::cli::test_support::sample_issue_snapshot;
    use crate::cli::LinkedPrSummary;

    #[test]
    fn cache_backed_issue_and_linked_pr_helpers_reuse_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let snapshot = sample_issue_snapshot();
        env.client.seed(snapshot.clone());

        let loaded = load_or_refresh_issue(&mut env, snapshot.number, false).expect("load issue");
        assert_eq!(loaded.snapshot.number, snapshot.number);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        let cached = load_or_refresh_issue(&mut env, snapshot.number, false).expect("cached issue");
        assert_eq!(cached.snapshot.title, snapshot.title);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);

        env.seed_linked_prs(
            42,
            vec![LinkedPrSummary {
                number: 128,
                title: "Enforce coverage".to_string(),
                state: "OPEN".to_string(),
                url: "https://github.com/akiojin/gwt/pull/128".to_string(),
                will_close_target: true,
                merged_at: None,
            }],
        );
        let linked =
            load_or_refresh_linked_prs(&mut env, snapshot.number, false).expect("linked prs");
        assert_eq!(linked.len(), 1);
        assert_eq!(env.linked_pr_calls(), vec![42]);

        env.clear_linked_pr_calls();
        let cached_linked = load_or_refresh_linked_prs(&mut env, snapshot.number, false)
            .expect("cached linked prs");
        assert_eq!(cached_linked.len(), 1);
        assert!(env.linked_pr_calls().is_empty());

        let cache_path = linked_prs_cache_path(temp.path(), snapshot.number);
        std::fs::create_dir_all(cache_path.parent().expect("cache dir")).expect("create cache dir");
        std::fs::write(&cache_path, "{not-json").expect("write invalid json");
        assert!(read_linked_prs_cache(temp.path(), snapshot.number).is_err());
    }

    #[test]
    fn explicit_issue_refresh_rebuilds_issue_index_when_cache_source_changes() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut old = sample_issue_snapshot();
        old.state = IssueState::Open;
        gwt_github::Cache::new(env.cache_root())
            .write_snapshot(&old)
            .expect("write old cache");

        let mut updated = old.clone();
        updated.state = IssueState::Closed;
        env.client.seed(updated.clone());

        let mut rebuild_calls = Vec::new();
        let entry = refresh_issue_cache_with_index_rebuild(&mut env, updated.number, |repo_path| {
            rebuild_calls.push(repo_path.to_path_buf());
            Ok(())
        })
        .expect("refresh with rebuild");

        assert_eq!(entry.snapshot.state, IssueState::Closed);
        assert_eq!(rebuild_calls, vec![env.repo_path().to_path_buf()]);
    }
}

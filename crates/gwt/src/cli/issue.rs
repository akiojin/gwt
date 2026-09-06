use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use gwt_github::{
    cache::{write_atomic, CacheGeneration, ValidatedCacheEntry},
    client::ApiError,
    Cache, IssueClient, IssueNumber, IssueSnapshot, IssueState, SpecOpsError,
};

use crate::cli::{
    CliEnv, CliParseError, IssueCommand, IssueMonitorPriorityPosition, LinkedPrSummary,
};

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

/// Issue #3873: refuse to write an autonomous-candidate body the Issue Monitor
/// cannot read.
///
/// An Issue carrying the `auto-merge` label opts into autonomous execution, and
/// the Monitor only admits it when `classify_acceptance_criteria` finds a
/// machine-checkable block. Without this guard the write succeeds and the Issue
/// silently lands in `needs_human` on the next scan. The guard reuses the
/// Monitor's classifier verbatim so the two can never disagree (AC-3); an Issue
/// without the label keeps today's behaviour.
pub(crate) fn guard_autonomous_acceptance_block(
    labels: &[String],
    body: &str,
) -> Result<(), SpecOpsError> {
    let opted_in = labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(crate::issue_monitor::AUTO_MERGE_LABEL));
    if !opted_in {
        return Ok(());
    }
    let criteria = crate::issue_monitor_gate::classify_acceptance_criteria(body);
    let Some(missing) = criteria.rejection_reason() else {
        return Ok(());
    };
    // Issue #3930 AC-2: the refusal names the element that is actually missing.
    Err(SpecOpsError::Validation(format!(
        "the `{}` label opts this Issue into autonomous execution, but {missing}; fix the \
         body or drop the label",
        crate::issue_monitor::AUTO_MERGE_LABEL
    )))
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
            guard_autonomous_acceptance_block(&labels, &body)?;
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
        IssueCommand::Edit {
            number,
            title,
            body,
            labels,
        } => run_issue_edit(env, number, title, body, labels, out)?,
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
            reason,
            claim_id,
            delivery_id,
            window_id,
        } => run_monitor_stop(
            env,
            project_root.as_deref(),
            number,
            &reason,
            crate::IssueMonitorStopTarget {
                issue_number: number,
                claim_id,
                delivery_id,
                window_id,
            },
            out,
        )?,
        IssueCommand::MonitorFailover {
            project_root,
            number,
            reason,
            claim_id,
            delivery_id,
            window_id,
        } => run_monitor_failover(
            env,
            project_root.as_deref(),
            number,
            &reason,
            crate::IssueMonitorStopTarget {
                issue_number: number,
                claim_id,
                delivery_id,
                window_id,
            },
            out,
        )?,
        IssueCommand::MonitorRequeue {
            project_root,
            number,
            reason,
        } => run_monitor_requeue(env, project_root.as_deref(), number, &reason, out)?,
        IssueCommand::MonitorQuotaHoldList { project_root } => {
            run_monitor_quota_hold_list(env, project_root.as_deref(), out)?
        }
        IssueCommand::MonitorReconcile { project_root } => {
            run_monitor_reconcile(env, project_root.as_deref(), out)?
        }
        IssueCommand::MonitorQuotaHoldClear {
            project_root,
            provider,
            reason,
        } => run_monitor_quota_hold_clear(env, project_root.as_deref(), &provider, &reason, out)?,
        IssueCommand::MonitorWait {
            project_root,
            number,
            reason,
            resume_condition,
            clear,
        } => run_monitor_wait(
            env,
            project_root.as_deref(),
            number,
            reason.as_deref(),
            resume_condition.as_deref(),
            clear,
            out,
        )?,
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
            auto_close_merged_issues,
            launch_agent,
        } => run_monitor_config_set(
            env,
            project_root.as_deref(),
            enabled,
            autonomous_mode,
            max_active,
            auto_close_merged_issues,
            launch_agent.as_deref(),
            out,
        )?,
        IssueCommand::MonitorProfiles { project_root } => {
            run_monitor_profiles(env, project_root.as_deref(), out)?
        }
        IssueCommand::MonitorProfilesSet {
            project_root,
            profiles,
            usage_threshold_percent,
        } => run_monitor_profiles_set(
            env,
            project_root.as_deref(),
            profiles,
            usage_threshold_percent,
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
/// Issue #3602: first remove cache-proven closed Issues from every
/// current-action status collection, including an older daemon projection.
///
/// The autonomous lifecycle only knows about the issues *it* parked, so an
/// agent that stopped because an operation refused it was invisible in the one
/// field a PM reads to find work needing a human. Merging here — after the
/// snapshot is obtained, not inside either branch — means the daemon
/// projection and the offline fallback cannot disagree, and it deliberately
/// reads a file rather than a pane, so it still answers while `pane.read` is
/// failing under GUI event-loop saturation (#3629).
/// Issue #3928 AC-4: the GitHub budget the queue is running on, from the
/// machine-local ledger every gwt process on this host writes to. Attached at
/// the surface rather than in the daemon projection so it is current at read
/// time and answers with or without a live daemon.
fn attach_github_budget(status: &mut crate::IssueMonitorAgentStatus) {
    let now = chrono::Utc::now();
    let ledger = gwt_core::github_budget::BudgetLedger::global();
    status.github_budget = Some(gwt_core::github_budget::status_by_resource(
        &ledger.snapshot(now),
        &gwt_core::github_budget::ThrottlePolicy::default(),
        now,
    ));
}

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
            Vec::new()
        }
    };
    let cache =
        Cache::new(crate::issue_cache::issue_cache_root_for_repo_path_or_detached(project_root));
    let projected_issue_numbers = status
        .queue
        .iter()
        .chain(&status.active_launches)
        .chain(&status.needs_human)
        .copied()
        .chain(status.inbox.iter().map(|item| item.issue_number))
        .chain(escalated.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    let closed_issue_numbers = projected_issue_numbers
        .into_iter()
        .filter(|issue_number| {
            let Some(entry) = cache.load_entry(IssueNumber(*issue_number)) else {
                return false;
            };
            if entry.snapshot.state != IssueState::Closed {
                return false;
            }
            let cached_closed_at =
                chrono::DateTime::parse_from_rfc3339(&entry.snapshot.updated_at.0).ok();
            let newer_live_open = status.inbox.iter().any(|item| {
                if item.issue_number != *issue_number
                    || item.github_state != crate::IssueMonitorIssueState::Open
                {
                    return false;
                }
                let Some(cached_closed_at) = cached_closed_at else {
                    // A malformed cached revision cannot suppress a positive
                    // live Open row from the daemon projection.
                    return true;
                };
                match item
                    .issue_updated_at
                    .as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                {
                    Some(live_open_at) => live_open_at > cached_closed_at,
                    // A live Open row with a missing or malformed timestamp
                    // cannot be proven stale; fail open like a malformed
                    // cached revision so the daemon's positive Open signal
                    // is never erased by an older Closed cache entry.
                    None => true,
                }
            });
            !newer_live_open
        })
        .collect::<std::collections::BTreeSet<_>>();
    status
        .queue
        .retain(|issue_number| !closed_issue_numbers.contains(issue_number));
    status
        .active_launches
        .retain(|issue_number| !closed_issue_numbers.contains(issue_number));
    status
        .needs_human
        .retain(|issue_number| !closed_issue_numbers.contains(issue_number));
    status
        .inbox
        .retain(|item| !closed_issue_numbers.contains(&item.issue_number));
    if status.last_error.as_ref().is_some_and(|error| {
        closed_issue_numbers
            .iter()
            .any(|issue_number| error.starts_with(&format!("issue #{issue_number}:")))
    }) {
        status.last_error = None;
    }
    for issue_number in escalated {
        // Issue #3602: Board is immutable coordination history, while
        // `needs_human` is a current-action projection. Suppress only when the
        // canonical cache positively proves Closed; missing/corrupt cache data
        // deliberately fails open so an unverified escalation is never hidden.
        if closed_issue_numbers.contains(&issue_number) {
            continue;
        }
        if !status.needs_human.contains(&issue_number) {
            status.needs_human.push(issue_number);
        }
    }
    status.needs_human.sort_unstable();
}

fn run_monitor_status<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    #[cfg(unix)]
    if let Some(status) = crate::daemon_publisher::read_issue_monitor_status(&project_root)
        .map_err(|error| io_as_api_error(io::Error::other(error.to_string())))?
    {
        let mut status = serde_json::from_value::<crate::IssueMonitorAgentStatus>(status)
            .map_err(|error| io_as_api_error(io::Error::other(error)))?;
        merge_board_escalations_into_needs_human(&project_root, &mut status);
        attach_github_budget(&mut status);
        out.push_str(
            &serde_json::to_string(&status)
                .map_err(|error| io_as_api_error(io::Error::other(error)))?,
        );
        out.push('\n');
        return Ok(0);
    }
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    // Issue #3633 AC-5: the only durable evidence of the real scan cadence.
    // Reaching this branch at all means no live daemon holds the projection.
    let persisted_last_scan_at = prefs.last_scan_at.clone();
    let mut monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&project_root);
    let candidates = crate::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root)
        .map_err(|error| io_as_api_error(io::Error::other(error)))?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    crate::scan_issue_monitor_candidates(&mut monitor, &candidates, &now);
    // Rebuilding the queue from the local Issue cache is a projection, not a
    // scan: nothing was fetched and nothing was claimed. Stamping it as a scan
    // (this used to report the literal string `gwtd-status`) told every reader
    // the monitor had just run, which is exactly how a permanently stopped
    // monitor kept looking healthy.
    monitor.restore_persisted_last_scan_at(persisted_last_scan_at);
    // Serialize through the same projection as the daemon branch above. The
    // offline fallback used to hand-roll an equivalent JSON object, so every
    // field added to the snapshot had to be added twice or the two branches
    // would silently disagree about what a caller can rely on.
    let mut status = monitor.agent_status_at(&now);
    merge_board_escalations_into_needs_human(&project_root, &mut status);
    attach_github_budget(&mut status);
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
        let retry_hold_cleared = monitor.clear_retry_hold(number);
        let completion_hold_cleared = monitor.clear_completion_hold(number);
        let hold_cleared = retry_hold_cleared || completion_hold_cleared;
        if completion_hold_cleared {
            *prefs = monitor.prefs();
        } else if retry_hold_cleared {
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

/// Issue #3923 AC-1 / Issue #3961 AC-3: list every provider quota hold in
/// force with the evidence it was formed from.
///
/// Read from the durable prefs — the store every Issue Monitor process
/// persists to and the file the PM inspects — so the list can never disagree
/// with that file the way a live projection served by a daemon that predates
/// the hold fields did (it answered `[]` while the file held two providers).
/// Expired and released holds are not listed: they no longer gate anything.
fn run_monitor_quota_hold_list<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let monitor = crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    out.push_str(
        &serde_json::json!({
            "provider_quota_holds": monitor.agent_status_at(&now).provider_quota_holds,
            "source": "durable_prefs",
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3883 AC-6: recover from "running but untracked" without touching a
/// single running agent.
///
/// The recovery the incident needed was six live panes against three slots,
/// where the fix could not be "close something": all six were working. So this
/// only ever *adds* tracking back — it re-adopts the launches whose windows the
/// canvas still shows, and never revokes, prunes, or closes anything. That also
/// makes it safe without the daemon control lane: additive bindings and
/// launches are union-merged by every cross-process rebase, so a daemon that
/// owns the state absorbs this commit instead of racing it.
///
/// Judgement comes from the live canvas, exactly as `pane.list` reads it,
/// because the durable snapshot disagreeing with the running windows is the
/// condition being repaired.
fn run_monitor_reconcile<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let live_window_ids = crate::cli::pane::live_window_ids(&project_root)
        .map_err(|error| SpecOpsError::from(ApiError::Network(error)))?;
    let (prefs, readopted) = crate::mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let readopted = monitor.readopt_live_launch_bindings(&live_window_ids);
        if !readopted.is_empty() {
            *prefs = monitor.prefs();
        }
        readopted
    })
    .map_err(io_as_api_error)?;
    let monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    out.push_str(
        &serde_json::json!({
            "readopted": readopted,
            "active_launches": monitor.active_issue_numbers(),
            "max_active": prefs.max_active_agents,
            "live_windows": live_window_ids.len(),
            "source": "live_canvas",
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3923 AC-1 / Issue #3961 AC-4: release one provider's quota
/// hold on the operator's authority.
///
/// The release is applied to the authoritative state. When a daemon owns the
/// control lane, the control lands in its in-memory state inside the same
/// lock-protected commit that persists it — a release that only reached disk
/// was joined away again by the daemon's own rebase, or dropped outright by a
/// daemon that predates the fence. Only when no daemon transport exists (and
/// no authority fence is held) are the durable prefs the authority and
/// written directly. Either way the durable prefs are read back and must
/// carry this release's fence with the provider no longer held; otherwise the
/// operation refuses instead of reporting a release nobody adopted. Every
/// issue the hold was holding is readmitted, and one immediate scan is
/// requested so the queue moves without waiting for the next tick.
fn run_monitor_quota_hold_clear<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    provider: &str,
    reason: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    // Refuse before publishing so the daemon never has to reject a control it
    // cannot explain back to the caller.
    let Some(provider) = crate::issue_monitor::normalize_issue_monitor_provider(provider) else {
        return refuse_quota_hold_clear(
            out,
            provider,
            "unknown_provider",
            "provider must name the held agent (for example codex or claude)",
        );
    };
    // Millisecond precision: the fence orders holds by instant, and a hold
    // formed right after this clear must not share its second.
    let released_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let before = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let held_before = issues_held_by_provider(&before, &provider);

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "quota_hold_clear": {
                "provider": provider,
                "reason": reason,
                "released_at": released_at,
            }
        }),
        std::process::id(),
    );
    let delivery = match publish_monitor_config_set(&project_root, payload) {
        Ok(()) => "daemon",
        Err(error) if error.allows_local_fallback() => {
            // No daemon transport: the durable prefs are the authority — unless
            // a fence says a daemon still owns them, in which case the write
            // is refused rather than forked.
            let written = crate::try_mutate_issue_monitor_prefs_without_authority_fence(
                &prefs_path,
                |prefs| {
                    let mut monitor = crate::IssueMonitorState::with_prefs(
                        crate::IssueMonitorConfig::default(),
                        prefs.clone(),
                    );
                    if matches!(
                        monitor.clear_provider_quota_hold(&provider, reason, &released_at),
                        crate::IssueMonitorProviderQuotaHoldClearOutcome::Cleared { .. }
                    ) {
                        *prefs = monitor.prefs();
                    }
                    Ok(())
                },
            );
            if let Err(error) = written {
                return refuse_quota_hold_clear(
                    out,
                    &provider,
                    "durable_write_refused",
                    &format!(
                        "no daemon transport and the durable prefs refused the release: {error}"
                    ),
                );
            }
            "durable_prefs"
        }
        Err(error) => {
            return refuse_quota_hold_clear(
                out,
                &provider,
                quota_hold_clear_publish_refusal(&error),
                &format!(
                    "the live Issue Monitor daemon did not adopt the release ({error}); \
                     a daemon that predates this control rejects it — restart the GWT app and retry"
                ),
            );
        }
    };

    // Adoption proof: the durable prefs must carry this release's fence and
    // must no longer hold the provider. An acknowledgment alone is not it.
    let after = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let release = after
        .provider_quota_hold_releases
        .get(&provider)
        .filter(|release| release.released_at == released_at)
        .cloned();
    let remaining =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), after.clone())
            .agent_status_at(&released_at)
            .provider_quota_holds;
    let still_held = remaining.iter().any(|hold| hold.provider == provider);
    let Some(release) = release.filter(|_| !still_held) else {
        return refuse_quota_hold_clear(
            out,
            &provider,
            "not_adopted",
            &format!(
                "{delivery} accepted the release but the durable prefs do not carry it \
                 (fence recorded: {}, provider still held: {still_held}); the process owning \
                 the Issue Monitor state may predate this operation — restart the GWT app and retry",
                after.provider_quota_hold_releases.contains_key(&provider)
            ),
        );
    };
    let held_after = issues_held_by_provider(&after, &provider);
    let released_issues = held_before
        .difference(&held_after)
        .copied()
        .collect::<Vec<_>>();
    let scan = issue_monitor_scan_delivery(request_immediate_monitor_scan(&project_root));
    out.push_str(
        &serde_json::json!({
            "provider": provider,
            "status": if release.released_reset_at.is_some() { "cleared" } else { "not_held" },
            "reason": reason,
            "released_at": released_at,
            "released_reset_at": release.released_reset_at,
            "released_issues": released_issues,
            "provider_quota_holds": remaining,
            "delivery": delivery,
            "scan_requested": scan.scan_requested,
            "scan_delivery": scan.scan_delivery,
            "scan_error": scan.scan_error,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

/// Issue #3961 AC-4: a stable, greppable name for each way the release can
/// fail to reach the authoritative state.
fn quota_hold_clear_publish_refusal(
    error: &crate::runtime_daemon_events::IssueMonitorControlPublishError,
) -> &'static str {
    use crate::runtime_daemon_events::IssueMonitorControlPublishError as PublishError;
    match error {
        PublishError::Rejected(_) => "daemon_rejected",
        PublishError::OutcomeUnknown(_) => "daemon_outcome_unknown",
        PublishError::Busy(_) => "daemon_busy",
        PublishError::RecoveryBlocked => "authority_recovery_blocked",
        PublishError::TransportUnavailable(_) => "daemon_unavailable",
    }
}

fn refuse_quota_hold_clear(
    out: &mut String,
    provider: &str,
    refusal: &str,
    detail: &str,
) -> Result<i32, SpecOpsError> {
    out.push_str(
        &serde_json::json!({
            "provider": provider,
            "status": "refused",
            "refusal": refusal,
            "detail": detail,
        })
        .to_string(),
    );
    out.push('\n');
    Ok(1)
}

/// Issues whose retry hold mirrors `provider`'s quota hold.
fn issues_held_by_provider(
    prefs: &crate::IssueMonitorPrefs,
    provider: &str,
) -> std::collections::BTreeSet<u64> {
    prefs
        .autonomous_records
        .iter()
        .filter(|record| {
            record
                .retry_hold_provider
                .as_deref()
                .and_then(crate::issue_monitor::normalize_issue_monitor_provider)
                .is_some_and(|held| held == provider)
        })
        .map(|record| record.issue_number)
        .collect()
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
    reason: &str,
    target: crate::IssueMonitorStopTarget,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let (_, outcome) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let outcome = monitor.stop_only(&target, reason, &now);
        if !matches!(outcome, crate::IssueMonitorStopOutcome::Mismatch(_)) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .map_err(io_as_api_error)?;

    let (status, stopped_window_id) = match &outcome {
        crate::IssueMonitorStopOutcome::Stopped { window_id } => {
            ("stopped", Some(window_id.clone()))
        }
        crate::IssueMonitorStopOutcome::AlreadyStopped => ("already_stopped", None),
        crate::IssueMonitorStopOutcome::Mismatch(mismatch) => {
            // Fail closed: nothing was written, nothing is torn down, and the
            // caller is told which component disagreed so it can re-read the
            // snapshot rather than retry blindly.
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "refused",
                    "mismatch": issue_monitor_stop_mismatch_label(*mismatch),
                })
                .to_string(),
            );
            out.push('\n');
            return Ok(1);
        }
    };

    let stopped_window_id = stopped_window_id.filter(|window_id| !window_id.is_empty());
    out.push_str(
        &serde_json::json!({
            "number": number,
            "status": status,
            "reason": reason,
            "stopped_window_id": stopped_window_id,
            // Say what is left to do rather than implying the pane is gone.
            "pane_teardown": if stopped_window_id.is_some() {
                "close the returned window with pane.close — the launch is already revoked, so the close cannot requeue or relaunch it"
            } else {
                "none"
            },
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
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
    reason: &str,
    target: crate::IssueMonitorStopTarget,
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
        let outcome = monitor.failover_restart(&target, reason, &now);
        if matches!(
            outcome,
            crate::IssueMonitorFailoverOutcome::Restarting { .. }
        ) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .map_err(io_as_api_error)?;

    let stopped_window_id = match outcome {
        crate::IssueMonitorFailoverOutcome::Restarting { stopped_window_id } => stopped_window_id,
        crate::IssueMonitorFailoverOutcome::AuthorityExhausted => {
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "refused",
                    "reason": "effect_authority_epoch_exhausted",
                })
                .to_string(),
            );
            out.push('\n');
            return Ok(1);
        }
        crate::IssueMonitorFailoverOutcome::Mismatch(mismatch) => {
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "refused",
                    "mismatch": issue_monitor_stop_mismatch_label(mismatch),
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
            "status": "restarting",
            "reason": reason,
            "stopped_window_id": stopped_window_id,
            "priority_order": prefs.priority_order,
            "launch_profile": prefs.launch_profile.as_ref().map(|profile| &profile.agent_id),
            "scan_requested": delivery.scan_requested,
            "scan_delivery": delivery.scan_delivery,
            "scan_error": delivery.scan_error,
            "pane_teardown": if stopped_window_id.is_some() {
                "close the returned window with pane.close — it is no longer bound to the issue, so the close cannot requeue it"
            } else {
                "none"
            },
        })
        .to_string(),
    );
    out.push('\n');
    // The failover mutation itself is complete even when the follow-up scan
    // authority is unavailable. Keep the established command success
    // contract while reporting scan delivery truthfully in the JSON fields.
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
    let (prefs, (outcome, completion_hold_cleared)) =
        crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
            let mut monitor = crate::IssueMonitorState::with_prefs(
                crate::IssueMonitorConfig::default(),
                prefs.clone(),
            );
            let outcome = monitor.requeue_failed_issue(number, reason, &now);
            let completion_hold_cleared =
                matches!(outcome, crate::IssueMonitorRequeueOutcome::NotHeld)
                    && monitor.clear_completion_hold(number);
            if matches!(outcome, crate::IssueMonitorRequeueOutcome::Requeued { .. })
                || completion_hold_cleared
            {
                *prefs = monitor.prefs();
            }
            Ok((outcome, completion_hold_cleared))
        })
        .map_err(io_as_api_error)?;

    let (stale_window_id, attempts_before, attempts_after) = match outcome {
        crate::IssueMonitorRequeueOutcome::Requeued {
            stale_window_id,
            attempts_before,
            attempts_after,
        } => (stale_window_id, attempts_before, attempts_after),
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
            if completion_hold_cleared {
                let delivery =
                    issue_monitor_scan_delivery(request_immediate_monitor_scan(&project_root));
                out.push_str(
                    &serde_json::json!({
                        "number": number,
                        "status": "requeued",
                        "reason": reason,
                        "released_hold": "completion",
                        "released_at": now,
                        "scan_requested": delivery.scan_requested,
                        "scan_delivery": delivery.scan_delivery,
                        "scan_error": delivery.scan_error,
                        "pane_teardown": "none",
                    })
                    .to_string(),
                );
                out.push('\n');
                return Ok(0);
            }
            // Issue #3683 (AC-3): a `BlockedByClaim` hold lives only in the
            // driving process's inbox, never in the prefs this process reads,
            // so the failure gate above cannot see it. Ask the live daemon's
            // status projection whether the row is claim-blocked and publish
            // an operator release if so; the driver adopts it on its next
            // prefs rebase. Without a daemon there is no in-memory hold to
            // release and the `not_held` refusal stands.
            if monitor_projection_reports_blocked_by_claim(&project_root, number) {
                return run_monitor_release_claim_block(
                    &prefs_path,
                    &project_root,
                    number,
                    reason,
                    &now,
                    out,
                );
            }
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
            "attempts_before": attempts_before,
            "attempts_after": attempts_after,
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

/// Issue #3683 (AC-3): whether the live daemon's status projection reports
/// this issue as `blocked_by_claim`. A missing daemon or an unreadable
/// projection means no verifiable in-memory claim hold, so the caller keeps
/// the fail-closed `not_held` refusal.
fn monitor_projection_reports_blocked_by_claim(
    project_root: &std::path::Path,
    number: u64,
) -> bool {
    #[cfg(unix)]
    {
        let Ok(Some(status)) = crate::daemon_publisher::read_issue_monitor_status(project_root)
        else {
            return false;
        };
        let Ok(status) = serde_json::from_value::<crate::IssueMonitorAgentStatus>(status) else {
            return false;
        };
        agent_status_reports_blocked_by_claim(&status, number)
    }
    #[cfg(not(unix))]
    {
        let _ = (project_root, number);
        false
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn agent_status_reports_blocked_by_claim(
    status: &crate::IssueMonitorAgentStatus,
    number: u64,
) -> bool {
    status.inbox.iter().any(|row| {
        row.issue_number == number && row.state == crate::MonitorInboxState::BlockedByClaim
    })
}

/// Issue #3683 (AC-3): publish an operator release for a daemon-reported
/// `BlockedByClaim` hold and request an immediate scan, mirroring the
/// requeue-success contract. Safe even if the block was just re-recorded: the
/// next acquire re-validates against the live GitHub claims and re-records the
/// block while a foreign claim is genuinely active.
fn run_monitor_release_claim_block(
    prefs_path: &std::path::Path,
    project_root: &std::path::Path,
    number: u64,
    reason: &str,
    now: &str,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let (prefs, outcome) = crate::try_mutate_issue_monitor_prefs(prefs_path, |prefs| {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let outcome = monitor.release_claim_block(number, reason, now);
        if matches!(outcome, crate::IssueMonitorRequeueOutcome::Requeued { .. }) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .map_err(io_as_api_error)?;

    match outcome {
        crate::IssueMonitorRequeueOutcome::Requeued { .. } => {}
        // The projection race window is real: a launch can go live between the
        // daemon read and this mutation. Fail closed exactly like the failure
        // path.
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
    }

    let delivery = issue_monitor_scan_delivery(request_immediate_monitor_scan(project_root));

    out.push_str(
        &serde_json::json!({
            "number": number,
            "status": "requeued",
            "released_hold": "blocked_by_claim",
            "reason": reason,
            "released_at": now,
            "failure_release_version": prefs.failure_release_version,
            "scan_requested": delivery.scan_requested,
            "scan_delivery": delivery.scan_delivery,
            "scan_error": delivery.scan_error,
        })
        .to_string(),
    );
    out.push('\n');
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
        crate::IssueMonitorStopMismatch::ClaimMismatch => "claim_mismatch",
        crate::IssueMonitorStopMismatch::DeliveryMismatch => "delivery_mismatch",
        crate::IssueMonitorStopMismatch::WindowMismatch => "window_mismatch",
    }
}

fn apply_monitor_config_set(
    prefs: &mut crate::IssueMonitorPrefs,
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    auto_close_merged_issues: Option<bool>,
    launch_agent: Option<&str>,
) -> io::Result<()> {
    validate_monitor_config_set(
        enabled,
        autonomous_mode,
        max_active,
        auto_close_merged_issues,
        launch_agent,
    )?;
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
    if let Some(auto_close_merged_issues) = auto_close_merged_issues {
        candidate
            .set_auto_close_merged_issues_with_effect_revocation(Some(auto_close_merged_issues))
            .ok_or_else(|| io::Error::other("Issue Monitor authority epoch overflow"))?;
    }
    if let Some(launch_agent) = launch_agent {
        candidate
            .switch_launch_profile_agent(launch_agent)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    }
    *prefs = candidate.prefs();
    Ok(())
}

fn validate_monitor_config_set(
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    auto_close_merged_issues: Option<bool>,
    launch_agent: Option<&str>,
) -> io::Result<()> {
    if launch_agent.is_some_and(|agent| agent.trim().is_empty()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            crate::IssueMonitorLaunchProfileSwitchError::InvalidAgent.to_string(),
        ));
    }
    if enabled.is_none()
        && autonomous_mode.is_none()
        && max_active.is_none()
        && auto_close_merged_issues.is_none()
        && launch_agent.is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one Issue Monitor config field is required",
        ));
    }
    // Issue #3814: policy lives in the command handler so both JSON dispatch
    // and direct callers receive the same effect-free GUI-only ON refusal.
    if enabled == Some(true) || autonomous_mode == Some(true) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "enabling Issue Monitor or autonomous mode requires an explicit GUI action",
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

// One optional parameter per settable config field: the arity tracks the
// wire schema of `issue.monitor.config.set`, so bundling them into a struct
// would only move the same list one indirection away.
#[allow(clippy::too_many_arguments)]
fn run_monitor_config_set<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    enabled: Option<bool>,
    autonomous_mode: Option<bool>,
    max_active: Option<usize>,
    auto_close_merged_issues: Option<bool>,
    launch_agent: Option<&str>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    validate_monitor_config_set(
        enabled,
        autonomous_mode,
        max_active,
        auto_close_merged_issues,
        launch_agent,
    )
    .map_err(io_as_api_error)?;
    // Issue #3923 AC-5: a switch needs a saved profile to switch. Refuse
    // before publishing so the daemon never has to reject a control it
    // cannot explain back to the caller.
    if launch_agent.is_some() {
        let prefs = crate::load_issue_monitor_prefs(
            &crate::issue_monitor_prefs_path_for_repo_path(&project_root),
        )
        .map_err(io_as_api_error)?;
        if prefs.launch_profile.is_none() {
            return Err(io_as_api_error(io::Error::new(
                io::ErrorKind::InvalidInput,
                crate::IssueMonitorLaunchProfileSwitchError::NoSavedProfile.to_string(),
            )));
        }
    }

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "config_set": {
                "enabled": enabled,
                "autonomous_mode": autonomous_mode,
                "max_active_agents": max_active,
                "auto_close_merged_issues": auto_close_merged_issues,
                "launch_agent": launch_agent,
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
            apply_monitor_config_set(
                prefs,
                enabled,
                autonomous_mode,
                max_active,
                auto_close_merged_issues,
                launch_agent,
            )
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
            "auto_close_merged_issues": prefs.auto_close_merged_issues,
            "auto_close_merged_issues_effective": prefs
                .auto_close_merged_issues
                .unwrap_or(prefs.autonomous_mode),
            "launch_profile": prefs.launch_profile.as_ref().map(|profile| {
                crate::issue_monitor_launch_profile_summary(&profile.clone().into())
            }),
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
}

/// SPEC #3914 FR-011: the pool projection shared by `issue.monitor.profiles`
/// and the `profiles.set` reply: every candidate with its pool index, summary
/// and current hold, plus the provider holds and the usage threshold.
fn monitor_profiles_projection(prefs: &crate::IssueMonitorPrefs) -> serde_json::Value {
    let monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    let status = monitor.status_view();
    let launch_profiles = prefs
        .launch_profile_pool()
        .iter()
        .zip(status.launch_profile_candidates.iter())
        .map(|(profile, candidate)| {
            let mut value = serde_json::to_value(profile).unwrap_or_default();
            if let Some(object) = value.as_object_mut() {
                object.insert("index".to_string(), serde_json::json!(candidate.index));
                object.insert("summary".to_string(), serde_json::json!(candidate.summary));
                object.insert(
                    "prefer_for".to_string(),
                    serde_json::json!(candidate.prefer_for),
                );
                if let Some(held_until) = &candidate.held_until {
                    object.insert("held_until".to_string(), serde_json::json!(held_until));
                }
            }
            value
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "launch_profiles": launch_profiles,
        "launch_profile_summary": status.launch_profile_summary,
        "provider_quota_holds": status.provider_quota_holds,
        "usage_threshold_percent": status.usage_threshold_percent,
    })
}

fn run_monitor_profiles<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
        &project_root,
    ))
    .map_err(io_as_api_error)?;
    out.push_str(&monitor_profiles_projection(&prefs).to_string());
    out.push('\n');
    Ok(0)
}

/// SPEC #3914 FR-011: `prefer_for` entries are `type:` / `kind:` / `label:`
/// followed by a lowercase token (`^(type|kind|label):[a-z0-9_.-]+$`).
fn is_valid_prefer_for_tag(tag: &str) -> bool {
    let Some((prefix, value)) = tag.split_once(':') else {
        return false;
    };
    matches!(prefix, "type" | "kind" | "label")
        && !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        })
}

/// SPEC #3914 FR-011 / AC-8: the pool must be non-empty, unique per provider,
/// limited to known agents, carry well-formed routing tags, and the threshold
/// must be within 1..=100. Shared by the daemon control and the local
/// fallback so the two can never accept different pools.
pub(crate) fn validate_monitor_profiles_set(
    profiles: &[crate::IssueMonitorLaunchProfile],
    usage_threshold_percent: Option<u8>,
) -> io::Result<()> {
    if profiles.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "profiles must contain at least one launch candidate",
        ));
    }
    let mut providers = std::collections::BTreeSet::new();
    for profile in profiles {
        let provider = match gwt_agent::resolve_agent_id(&profile.agent_id) {
            Some(gwt_agent::AgentId::Custom(_)) | None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown agent_id {:?}", profile.agent_id),
                ));
            }
            Some(agent_id) => agent_id.command().to_ascii_lowercase(),
        };
        if !providers.insert(provider.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("duplicate launch candidate for provider {provider}"),
            ));
        }
        if let Some(tag) = profile
            .prefer_for
            .iter()
            .find(|tag| !is_valid_prefer_for_tag(tag))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("prefer_for entry {tag:?} must match ^(type|kind|label):[a-z0-9_.-]+$"),
            ));
        }
    }
    if usage_threshold_percent.is_some_and(|percent| !(1..=100).contains(&percent)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage_threshold_percent must be between 1 and 100",
        ));
    }
    Ok(())
}

fn apply_monitor_profiles_set(
    prefs: &mut crate::IssueMonitorPrefs,
    profiles: &[crate::IssueMonitorLaunchProfile],
    usage_threshold_percent: Option<u8>,
) -> io::Result<()> {
    validate_monitor_profiles_set(profiles, usage_threshold_percent)?;
    // Same revocation the GUI save performs: proposals prepared against the
    // previous pool must not commit under the new one.
    prefs
        .advance_effect_authority_epoch()
        .ok_or_else(|| io::Error::other("Issue Monitor authority epoch overflow"))?;
    prefs.set_launch_profile_pool(profiles.to_vec());
    if let Some(percent) = usage_threshold_percent {
        prefs.launch_usage_threshold_percent = percent;
    }
    Ok(())
}

fn run_monitor_profiles_set<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    profiles: Vec<crate::IssueMonitorLaunchProfile>,
    usage_threshold_percent: Option<u8>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    validate_monitor_profiles_set(&profiles, usage_threshold_percent).map_err(io_as_api_error)?;

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({
            "profiles_set": {
                "profiles": profiles,
                "usage_threshold_percent": usage_threshold_percent,
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
            apply_monitor_profiles_set(prefs, &profiles, usage_threshold_percent)
        })
        .map_err(io_as_api_error)?;
    }
    let prefs = crate::load_issue_monitor_prefs(&crate::issue_monitor_prefs_path_for_repo_path(
        &project_root,
    ))
    .map_err(io_as_api_error)?;
    out.push_str(&monitor_profiles_projection(&prefs).to_string());
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

/// Issue #3844: the Issue an agent's wait declaration is about. The launch
/// context (`GWT_AUTONOMOUS_ISSUE`) names the owner, so a monitor-launched
/// agent may omit `number`; an explicit `number` always wins.
fn resolve_monitor_wait_issue_number(
    explicit: Option<u64>,
    launch_context: Option<&str>,
) -> Option<u64> {
    explicit.or_else(|| launch_context?.trim().parse::<u64>().ok())
}

/// Issue #3844: whether `number` has a live launch in the Issue Monitor that
/// owns `project_root` — the daemon snapshot when one is running, the
/// persisted prefs otherwise.
fn monitor_launch_is_live(
    project_root: &std::path::Path,
    number: u64,
) -> Result<bool, SpecOpsError> {
    #[cfg(unix)]
    if let Some(status) = crate::daemon_publisher::read_issue_monitor_status(project_root)
        .map_err(|error| io_as_api_error(io::Error::other(error.to_string())))?
    {
        let status = serde_json::from_value::<crate::IssueMonitorAgentStatus>(status)
            .map_err(|error| io_as_api_error(io::Error::other(error)))?;
        return Ok(status.active_launches.contains(&number));
    }
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let monitor = crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs);
    Ok(monitor.launched_window_id(number).is_some())
}

#[cfg(unix)]
fn publish_monitor_wait_control(
    project_root: &std::path::Path,
    wait: serde_json::Value,
) -> Result<(), String> {
    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({ "wait": wait }),
        std::process::id(),
    );
    crate::daemon_publisher::publish_event(
        project_root,
        crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL,
        payload,
    )
}

#[cfg(not(unix))]
fn publish_monitor_wait_control(
    _project_root: &std::path::Path,
    _wait: serde_json::Value,
) -> Result<(), String> {
    Err("wait declaration publish unavailable on this platform".to_string())
}

/// Issue #3844 AC-1/AC-2: tell the Issue Monitor that the current launch is
/// waiting (`reason`, `resume_condition`) or has resumed (`clear`). The daemon
/// records the declaration on the issue's autonomous record, where stuck
/// detection honours it for at most [`crate::AUTONOMOUS_WAIT_MAX_SECS`] and the
/// PM reads it back from `issue.monitor.status`.
fn run_monitor_wait<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: Option<u64>,
    reason: Option<&str>,
    resume_condition: Option<&str>,
    clear: bool,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let launch_context = std::env::var(crate::autonomous_handoff::GWT_AUTONOMOUS_ISSUE_ENV).ok();
    let Some(number) = resolve_monitor_wait_issue_number(number, launch_context.as_deref()) else {
        out.push_str(
            &serde_json::json!({
                "status": "refused",
                "refusal": "issue_unknown",
                "detail": "pass params.number, or run inside a monitor-launched session where GWT_AUTONOMOUS_ISSUE names the owner Issue",
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    };
    let project_root = issue_monitor_project_root(env, project_root)?;
    if !monitor_launch_is_live(&project_root, number)? {
        // Fail closed: a wait only suspends stuck detection for a launch that
        // exists, and accepting one for a queued or parked row would tell the
        // caller it is protected when nothing is.
        out.push_str(
            &serde_json::json!({
                "number": number,
                "status": "refused",
                "refusal": "not_launched",
                "detail": "no live Issue Monitor launch owns this issue; only the running agent of a launched issue can declare a wait",
            })
            .to_string(),
        );
        out.push('\n');
        return Ok(1);
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let wait = if clear {
        serde_json::json!({ "issue_number": number, "clear": true, "at": now })
    } else {
        serde_json::json!({
            "issue_number": number,
            "reason": reason.unwrap_or_default(),
            "resume_condition": resume_condition.unwrap_or_default(),
            "at": now,
        })
    };
    match publish_monitor_wait_control(&project_root, wait) {
        Ok(()) => {
            let mut response = serde_json::json!({
                "number": number,
                "status": if clear { "cleared" } else { "waiting" },
                "at": now,
            });
            if !clear {
                response["reason"] = serde_json::Value::from(reason.unwrap_or_default());
                response["resume_condition"] =
                    serde_json::Value::from(resume_condition.unwrap_or_default());
                response["max_wait_secs"] =
                    serde_json::Value::from(crate::AUTONOMOUS_WAIT_MAX_SECS);
                response["detail"] = serde_json::Value::from(
                    "stuck detection is suspended for this launch until the wait is cleared or max_wait_secs elapses; clear it with params.clear:true when you resume",
                );
            }
            out.push_str(&response.to_string());
            out.push('\n');
            Ok(0)
        }
        Err(error) => {
            out.push_str(
                &serde_json::json!({
                    "number": number,
                    "status": "failed",
                    "detail": format!("wait declaration publish failed: {error}"),
                })
                .to_string(),
            );
            out.push('\n');
            Ok(1)
        }
    }
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

/// Issue #3865: update a plain Issue's title / body / labels in place.
///
/// Only the supplied fields are sent. A body update on a `gwt-spec` Issue is
/// refused before any write because that body is section-managed by
/// `issue.spec.edit`; title and labels are not section-managed and stay
/// editable. API failures pass through untouched so the caller can tell a
/// missing Issue, a permission failure, and a network failure apart.
fn run_issue_edit<E: CliEnv>(
    env: &mut E,
    number: u64,
    title: Option<String>,
    body: Option<String>,
    labels: Option<Vec<String>>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if title.is_none() && body.is_none() && labels.is_none() {
        return Ok(write_issue_edit_refusal(
            out,
            number,
            "nothing to update: pass at least one of params.title, params.body, params.labels",
        ));
    }
    let issue = IssueNumber(number);
    let current = load_or_refresh_issue(env, issue, true)?.snapshot;
    if body.is_some()
        && current
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("gwt-spec"))
    {
        return Ok(write_issue_edit_refusal(
            out,
            number,
            &format!(
                "issue #{number} carries the gwt-spec label and its body is section-managed;                  use issue.spec.edit per section instead of issue.edit (title and labels                  remain editable here)"
            ),
        ));
    }

    // Issue #3873 guard, applied to the post-edit state: replacing the body
    // or the labels must not leave an `auto-merge` Issue without the
    // machine-checkable acceptance block that `issue.create` requires.
    if body.is_some() || labels.is_some() {
        let effective_labels = labels.as_deref().unwrap_or(&current.labels);
        let effective_body = body.as_deref().unwrap_or(&current.body);
        guard_autonomous_acceptance_block(effective_labels, effective_body)?;
    }

    let fields = gwt_github::client::IssueFieldsPatch {
        title,
        body,
        labels,
    };
    let updated: Vec<&str> = [
        fields.title.as_ref().map(|_| "title"),
        fields.body.as_ref().map(|_| "body"),
        fields.labels.as_ref().map(|_| "labels"),
    ]
    .into_iter()
    .flatten()
    .collect();
    // One remote request: a failure leaves the Issue exactly as it was, so
    // the guard above cannot be defeated by a half-applied edit.
    env.client().patch_issue_fields(issue, &fields)?;
    super::intake_outcome::auto_record_issue_operation(
        env.repo_path(),
        "issue.edit",
        super::intake_outcome::IntakeOutcomeKind::IssueUpdated,
        number,
    );
    let _ = refresh_issue_cache(env, issue)?;
    out.push_str(&format!(
        "updated issue #{number} ({})\n",
        updated.join(", ")
    ));
    Ok(0)
}

fn write_issue_edit_refusal(out: &mut String, number: u64, reason: &str) -> i32 {
    out.push_str(
        &serde_json::json!({
            "number": number,
            "status": "refused",
            "reason": reason,
        })
        .to_string(),
    );
    out.push('\n');
    1
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
    load_or_refresh_issue_with_index_rebuild(env, number, refresh, |repo_path| {
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

fn cache_resource_is_fresh(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < crate::issue_cache::ISSUE_CACHE_TTL)
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
    rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    let generation = Cache::new(env.cache_root()).current_generation(number)?;
    refresh_issue_cache_with_index_rebuild_since(
        env,
        number,
        None,
        generation.as_ref(),
        None,
        false,
        rebuild_issue_index,
    )
}

fn load_or_refresh_issue_with_index_rebuild<E, F>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
    rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    if refresh {
        let generation = Cache::new(env.cache_root()).current_generation(number)?;
        return refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            None,
            generation.as_ref(),
            None,
            false,
            rebuild_issue_index,
        );
    }

    match Cache::new(env.cache_root())
        .load_validated_entry(number, crate::issue_cache::ISSUE_CACHE_TTL)?
    {
        ValidatedCacheEntry::Fresh(entry) => Ok(entry.entry),
        ValidatedCacheEntry::Stale(entry) => refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            Some(&entry.entry.snapshot.updated_at),
            entry.generation.as_ref(),
            Some(&entry.entry.snapshot),
            false,
            rebuild_issue_index,
        ),
        ValidatedCacheEntry::Unvalidated(entry) => refresh_issue_cache_with_index_rebuild_since(
            env,
            number,
            None,
            entry.generation.as_ref(),
            None,
            true,
            rebuild_issue_index,
        ),
        ValidatedCacheEntry::Missing { generation } => {
            refresh_issue_cache_with_index_rebuild_since(
                env,
                number,
                None,
                generation.as_ref(),
                None,
                true,
                rebuild_issue_index,
            )
        }
    }
}

fn refresh_issue_cache_with_index_rebuild_since<E, F>(
    env: &mut E,
    number: IssueNumber,
    since: Option<&gwt_github::UpdatedAt>,
    expected_generation: Option<&CacheGeneration>,
    not_modified_snapshot: Option<&IssueSnapshot>,
    force_rebuild: bool,
    mut rebuild_issue_index: F,
) -> Result<gwt_github::CacheEntry, SpecOpsError>
where
    E: CliEnv,
    F: FnMut(&std::path::Path) -> Result<(), String>,
{
    let cache_root = env.cache_root();
    let before = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    let snapshot = match env.client().fetch(number, since)? {
        gwt_github::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::FetchResult::NotModified => {
            let cache = Cache::new(cache_root);
            let expected = not_modified_snapshot.ok_or_else(|| {
                SpecOpsError::from(ApiError::Network(format!(
                    "issue #{} returned NotModified without a validated cache snapshot",
                    number.0
                )))
            })?;
            if !cache.renew_validation_receipt_if_generation(expected, expected_generation)? {
                return Err(SpecOpsError::from(ApiError::Network(format!(
                    "issue #{} cache changed during validation",
                    number.0
                ))));
            }
            return load_fresh_validated_entry(&cache, number);
        }
    };
    let cache = Cache::new(cache_root.clone());
    let Some(committed_generation) =
        cache.write_snapshot_if_generation(&snapshot, expected_generation)?
    else {
        return Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache changed while fetching remote snapshot",
            number.0
        ))));
    };
    let after = crate::issue_cache::issue_cache_source_fingerprint(&cache_root)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err)))?;
    if force_rebuild || crate::issue_cache::issue_cache_source_changed(&before, &after) {
        rebuild_issue_index(env.repo_path()).map_err(|err| {
            SpecOpsError::from(ApiError::Network(format!("rebuild issue index: {err}")))
        })?;
    }
    if !cache.renew_validation_receipt_if_generation(&snapshot, Some(&committed_generation))? {
        return Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache changed before validation receipt publication",
            number.0
        ))));
    }
    load_fresh_validated_entry(&cache, number)
}

fn load_fresh_validated_entry(
    cache: &Cache,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    match cache.load_validated_entry(number, crate::issue_cache::ISSUE_CACHE_TTL)? {
        ValidatedCacheEntry::Fresh(entry) => Ok(entry.entry),
        _ => Err(SpecOpsError::from(ApiError::Network(format!(
            "issue #{} cache validation receipt is unstable",
            number.0
        )))),
    }
}

pub(super) fn load_or_refresh_linked_prs<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<Vec<LinkedPrSummary>, SpecOpsError> {
    let cache_root = env.cache_root();
    if !refresh {
        if let Ok(Some(cached)) = read_linked_prs_cache(&cache_root, number) {
            if cache_resource_is_fresh(&linked_prs_cache_path(&cache_root, number)) {
                return Ok(cached);
            }
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
    use std::{
        fs::File,
        path::Path,
        time::{Duration, SystemTime},
    };

    #[cfg(unix)]
    use std::{
        io::{BufRead, Write},
        os::unix::net::UnixListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    use gwt_core::test_support::ScopedGwtHome;
    use gwt_github::client::{CommentId, CommentSnapshot, IssueSnapshot, IssueState, UpdatedAt};
    use tempfile::TempDir;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    /// Issue #3683 (AC-3): the claim-block probe reads the exact wire format
    /// the daemon status projection serves, so the state match must survive
    /// the snake_case serialization of `MonitorInboxState`.
    #[test]
    fn agent_status_probe_matches_only_the_blocked_by_claim_row() {
        let status: crate::IssueMonitorAgentStatus = serde_json::from_value(serde_json::json!({
            "queue": [7],
            "active_launches": [],
            "max_active": 1,
            "enabled": true,
            "autonomous_mode": false,
            "has_launch_profile": false,
            "inbox": [
                {"issue_number": 7, "state": "queued"},
                {"issue_number": 42, "state": "blocked_by_claim",
                 "blocked_by_owner": "AkioJinsenji:9720"},
            ],
        }))
        .expect("projection wire format deserializes");

        assert!(agent_status_reports_blocked_by_claim(&status, 42));
        assert!(!agent_status_reports_blocked_by_claim(&status, 7));
        assert!(!agent_status_reports_blocked_by_claim(&status, 99));
    }

    fn set_modified(path: &Path, modified: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .expect("open cache receipt")
            .set_modified(modified)
            .expect("set cache receipt mtime");
    }

    fn stale_time() -> SystemTime {
        SystemTime::now() - crate::issue_cache::ISSUE_CACHE_TTL - Duration::from_secs(1)
    }

    fn write_issue_validation_receipt(
        cache_root: &Path,
        snapshot: &IssueSnapshot,
        validated_at: &str,
    ) -> String {
        let cache = Cache::new(cache_root.to_path_buf());
        assert!(cache
            .renew_validation_receipt_if_current(snapshot)
            .expect("publish validation receipt"));
        let path = cache.validation_receipt_path(snapshot.number);
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read validation receipt"))
                .expect("parse validation receipt");
        let generation = receipt["generation"]
            .as_str()
            .expect("validation generation")
            .to_string();
        receipt["validated_at"] = serde_json::Value::String(validated_at.to_string());
        fs::write(
            path,
            serde_json::to_vec_pretty(&receipt).expect("serialize validation receipt"),
        )
        .expect("write validation receipt");
        generation
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
        write_issue_validation_receipt(tmp.path(), &snapshot, &chrono::Utc::now().to_rfc3339());

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

    fn seeded_edit_env(labels: &[&str]) -> (TempDir, crate::cli::TestEnv) {
        let tmp = TempDir::new().expect("tempdir");
        let env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.client.seed(IssueSnapshot {
            number: IssueNumber(7),
            title: "Original title".to_string(),
            body: "Original body".to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-09-01T00:00:00Z"),
            comments: vec![],
        });
        (tmp, env)
    }

    fn fetched(env: &crate::cli::TestEnv, number: u64) -> IssueSnapshot {
        match env
            .client
            .fetch(IssueNumber(number), None)
            .expect("fresh fetch")
        {
            gwt_github::client::FetchResult::Updated(snapshot) => snapshot,
            gwt_github::client::FetchResult::NotModified => panic!("fresh fetch should update"),
        }
    }

    fn edit_body(number: u64, body: &str) -> IssueCommand {
        IssueCommand::Edit {
            number,
            title: None,
            body: Some(body.to_string()),
            labels: None,
        }
    }

    /// Issue #3865 AC-2: title / body / labels are each optional and only the
    /// supplied fields change; the local cache reflects the write.
    #[test]
    fn issue_edit_updates_only_the_supplied_fields() {
        let (_tmp, mut env) = seeded_edit_env(&["bug"]);

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::Edit {
                number: 7,
                title: Some("Renamed title".to_string()),
                body: None,
                labels: None,
            },
            &mut out,
        )
        .expect("title-only edit");
        assert_eq!(code, 0, "{out}");
        let snapshot = fetched(&env, 7);
        assert_eq!(snapshot.title, "Renamed title");
        assert_eq!(snapshot.body, "Original body");
        assert_eq!(snapshot.labels, vec!["bug"]);

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::Edit {
                number: 7,
                title: None,
                body: Some("Corrected body".to_string()),
                labels: Some(vec!["bug".to_string(), "enhancement".to_string()]),
            },
            &mut out,
        )
        .expect("body+labels edit");
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("updated issue #7"), "{out}");
        let snapshot = fetched(&env, 7);
        assert_eq!(snapshot.title, "Renamed title");
        assert_eq!(snapshot.body, "Corrected body");
        assert_eq!(snapshot.labels, vec!["bug", "enhancement"]);
        // Both fields travel in one remote request; no single-field patch
        // is issued, so a failure cannot leave the Issue half-applied.
        let mutations: Vec<String> = env
            .client
            .call_log()
            .into_iter()
            .filter(|call| call.starts_with("patch_") || call.starts_with("set_labels"))
            .collect();
        assert_eq!(
            mutations,
            vec!["patch_issue_fields:#7".to_string(); 2],
            "one combined patch per edit"
        );

        let cached = Cache::new(env.cache_root())
            .load_entry(IssueNumber(7))
            .expect("cache entry after edit");
        assert_eq!(cached.snapshot.body, "Corrected body");
        assert_eq!(cached.snapshot.title, "Renamed title");
    }

    /// Issue #3865 AC-3: a `gwt-spec` body is section-managed, so the plain
    /// editor refuses it, writes nothing, and points at `issue.spec.edit`.
    #[test]
    fn issue_edit_refuses_body_updates_on_gwt_spec_issues() {
        let (_tmp, mut env) = seeded_edit_env(&["GWT-Spec", "phase/draft"]);
        let mut out = String::new();
        let code = run(&mut env, edit_body(7, "rewritten"), &mut out).expect("refusal is a result");
        assert_eq!(code, 1, "{out}");
        let refusal: serde_json::Value = serde_json::from_str(out.trim()).expect("refusal JSON");
        assert_eq!(refusal["status"], "refused");
        let reason = refusal["reason"].as_str().expect("reason");
        assert!(reason.contains("gwt-spec"), "{reason}");
        assert!(reason.contains("issue.spec.edit"), "{reason}");
        assert_eq!(fetched(&env, 7).body, "Original body");
        assert!(
            !env.client
                .call_log()
                .iter()
                .any(|call| call.starts_with("patch_body")),
            "refusal must not reach the API"
        );

        // Title and labels are not section-managed: they stay editable.
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::Edit {
                number: 7,
                title: Some("Renamed spec".to_string()),
                body: None,
                labels: None,
            },
            &mut out,
        )
        .expect("title edit on spec");
        assert_eq!(code, 0, "{out}");
        assert_eq!(fetched(&env, 7).title, "Renamed spec");
    }

    /// Issue #3865 AC-5: a missing Issue, a permission failure, and a network
    /// failure each surface their own cause, and none of them writes anything.
    #[test]
    fn issue_edit_reports_distinguishable_failures() {
        let (_tmp, mut env) = seeded_edit_env(&["bug"]);
        let mut out = String::new();

        let missing = run(&mut env, edit_body(999, "x"), &mut out).expect_err("missing issue");
        assert!(missing.to_string().contains("#999 not found"), "{missing}");

        env.client
            .fail_next_issue_patch(ApiError::PermissionDenied {
                message: "Resource not accessible by integration".to_string(),
            });
        let denied = run(&mut env, edit_body(7, "x"), &mut out).expect_err("permission");
        assert!(
            denied.to_string().starts_with("permission denied:"),
            "{denied}"
        );

        env.client
            .fail_next_issue_patch(ApiError::Network("connection reset".to_string()));
        let network = run(&mut env, edit_body(7, "x"), &mut out).expect_err("network");
        assert!(
            network.to_string().starts_with("network error:"),
            "{network}"
        );

        assert_eq!(fetched(&env, 7).body, "Original body");
    }

    /// Issue #3865 x #3873: `issue.edit` cannot bypass the autonomous
    /// acceptance-block guard — neither by replacing the body of an
    /// `auto-merge` Issue with one the Monitor cannot read, nor by adding the
    /// label to an Issue whose body has no block. A compliant body still goes
    /// through.
    #[test]
    fn issue_edit_keeps_the_autonomous_acceptance_block_guard() {
        let (_tmp, mut env) = seeded_edit_env(&["auto-merge"]);
        let mut out = String::new();

        let stripped = run(
            &mut env,
            edit_body(7, "## 成功基準\n- [ ] AC-1: wrong heading\n"),
            &mut out,
        )
        .expect_err("auto-merge body without a readable AC block must be refused");
        let message = stripped.to_string();
        assert!(
            message.contains("受け入れ基準") && message.contains("AC-"),
            "{message}"
        );

        let (_tmp2, mut plain) = seeded_edit_env(&["bug"]);
        let labelled = run(
            &mut plain,
            IssueCommand::Edit {
                number: 7,
                title: None,
                body: None,
                labels: Some(vec!["bug".to_string(), "auto-merge".to_string()]),
            },
            &mut out,
        )
        .expect_err("adding auto-merge to a body without an AC block must be refused");
        assert!(labelled.to_string().contains("受け入れ基準"), "{labelled}");
        for env in [&env, &plain] {
            assert!(
                !env.client
                    .call_log()
                    .iter()
                    .any(|call| call.starts_with("patch_") || call.starts_with("set_labels")),
                "refused edits must not reach the API"
            );
        }

        let code = run(
            &mut env,
            edit_body(7, "## 受け入れ基準\n- [ ] AC-1: cargo test is GREEN\n"),
            &mut out,
        )
        .expect("compliant body is accepted");
        assert_eq!(code, 0, "{out}");
        assert!(fetched(&env, 7).body.contains("AC-1"));
    }

    #[test]
    fn issue_edit_refuses_an_empty_update() {
        let (_tmp, mut env) = seeded_edit_env(&["bug"]);
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::Edit {
                number: 7,
                title: None,
                body: None,
                labels: None,
            },
            &mut out,
        )
        .expect("refusal is a result");
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("refused"), "{out}");
        assert!(out.contains("params.title"), "{out}");
        assert!(
            !env.client
                .call_log()
                .iter()
                .any(|call| call.starts_with("patch_") || call.starts_with("set_labels")),
            "nothing may reach the API"
        );
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
        let home = tmp.path().join("home");
        let _home = ScopedGwtHome::set(&home);
        let runtime_path = home.join(".gwt/sessions/runtime/123/session.json");
        std::fs::create_dir_all(runtime_path.parent().expect("runtime parent"))
            .expect("runtime directory");
        std::fs::write(&runtime_path, "{}").expect("runtime evidence");
        let _runtime = ScopedEnvVar::set(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV, &runtime_path);
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
                    retry_hold_provider: Some("codex".to_string()),
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
        assert_eq!(record.retry_hold_provider, None);
    }

    #[test]
    fn launch_now_clears_a_stale_completion_hold_and_requires_a_fresh_session() {
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
                issue_completion_migration_version:
                    crate::issue_monitor::ISSUE_COMPLETION_MIGRATION_VERSION,
                completion_records: vec![crate::issue_monitor::IssueCompletionRecord {
                    issue_number: 42,
                    generation: 1,
                    state: crate::issue_monitor::IssueCompletionState::Completed,
                    issue_updated_at: Some("2026-08-15T00:00:00Z".to_string()),
                    evidence: crate::issue_monitor::IssueCompletionEvidence::LinkedPr,
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
        assert!(persisted.merged_issues.is_empty());
        assert_eq!(
            persisted.completion_records[0].state,
            crate::issue_monitor::IssueCompletionState::Reopened
        );
        assert_eq!(
            persisted.queued_launch_session_strategies.get(&42),
            Some(&crate::IssueMonitorLaunchSessionStrategy::FreshRequired)
        );
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

    #[test]
    fn issue_monitor_status_excludes_only_cache_proven_closed_board_owners() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        for issue_number in [2338, 2339] {
            let escalation = gwt_core::coordination::BoardEntry::new(
                gwt_core::coordination::AuthorKind::Agent,
                "Claude Code",
                gwt_core::coordination::BoardEntryKind::Blocked,
                "事象: 拒否\n原因: immutable\n依頼: fresh launch\n再開条件: 新 pane",
                None,
                None,
                vec![],
                vec![issue_number.to_string()],
            );
            gwt_core::coordination::post_entry(&repo, escalation).expect("post escalation");
        }
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        gwt_github::Cache::new(cache_root)
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(2338),
                title: "Closed owner".to_string(),
                body: String::new(),
                labels: Vec::new(),
                state: IssueState::Closed,
                updated_at: UpdatedAt::new("2026-08-26T00:00:00Z"),
                comments: Vec::new(),
            })
            .expect("write closed cache entry");

        let mut published = crate::IssueMonitorAgentStatus {
            queue: vec![2338],
            active_launches: vec![2338],
            max_active: 1,
            enabled: true,
            autonomous_mode: true,
            has_launch_profile: true,
            quota_hold: None,
            launch_profile_summary: String::new(),
            launch_profile_candidates: Vec::new(),
            usage_threshold_percent: 80,
            provider_quota_holds: Vec::new(),
            needs_human: vec![2338],
            inbox: Vec::new(),
            last_error: Some("issue #2338: stale failure".to_string()),
            last_scan_at: Some("2026-08-26T00:00:00Z".to_string()),
            scan_stall: None,
            github_budget: None,
            generation_reclaim: None,
        };
        merge_board_escalations_into_needs_human(&repo, &mut published);
        assert!(published.queue.is_empty());
        assert!(published.active_launches.is_empty());
        assert_eq!(published.needs_human, vec![2339]);
        assert_eq!(published.last_error, None);

        let mut live_open = crate::IssueMonitorAgentStatus {
            queue: vec![2338],
            active_launches: Vec::new(),
            max_active: 1,
            enabled: true,
            autonomous_mode: true,
            has_launch_profile: true,
            quota_hold: None,
            launch_profile_summary: String::new(),
            launch_profile_candidates: Vec::new(),
            usage_threshold_percent: 80,
            provider_quota_holds: Vec::new(),
            needs_human: vec![2338],
            inbox: vec![crate::issue_monitor::IssueMonitorInboxSummary {
                issue_number: 2338,
                state: crate::MonitorInboxState::Queued,
                github_state: crate::IssueMonitorIssueState::Open,
                issue_updated_at: Some("2026-08-27T00:00:00Z".to_string()),
                readiness: crate::IssueMonitorReadiness::NotApplicable,
                recoverable_merged: false,
                completion_reason: None,
                blocked_by_owner: None,
                launched_window_id: None,
                error_message: None,
                last_activity_at: None,
                retry_not_before: None,
                retry_hold_reason: None,
                claim_id: None,
                delivery_id: None,
                waiting: None,
                steering: None,
            }],
            last_error: Some("issue #2338: live failure".to_string()),
            last_scan_at: Some("2026-08-27T00:00:00Z".to_string()),
            scan_stall: None,
            github_budget: None,
            generation_reclaim: None,
        };
        merge_board_escalations_into_needs_human(&repo, &mut live_open);
        assert_eq!(live_open.queue, vec![2338]);
        assert_eq!(live_open.needs_human, vec![2338, 2339]);
        assert_eq!(live_open.inbox.len(), 1);
        assert_eq!(
            live_open.last_error.as_deref(),
            Some("issue #2338: live failure")
        );

        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        let status: serde_json::Value = serde_json::from_str(out.trim()).expect("status json");
        assert_eq!(
            status["needs_human"],
            serde_json::json!([2339]),
            "cache-proven closed owners are not actionable; missing cache fails open: {out}"
        );
        assert_eq!(
            gwt_core::coordination::load_escalation_store(&repo)
                .expect("escalation store")
                .open_owner_issue_numbers(),
            vec![2338, 2339],
            "status filtering must not rewrite Board history"
        );
    }

    /// Issue #3602 regression: a live daemon Open row can carry a missing or
    /// malformed `issue_updated_at`. Timestamp absence proves nothing, so it
    /// must fail open exactly like a malformed cached revision instead of
    /// letting a stale Closed cache erase the issue from every projection.
    #[test]
    fn live_open_row_without_timestamp_fails_open() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        gwt_github::Cache::new(cache_root)
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(2338),
                title: "Closed owner".to_string(),
                body: String::new(),
                labels: Vec::new(),
                state: IssueState::Closed,
                updated_at: UpdatedAt::new("2026-08-26T00:00:00Z"),
                comments: Vec::new(),
            })
            .expect("write closed cache entry");

        for issue_updated_at in [None, Some("not-a-timestamp".to_string())] {
            let mut status = crate::IssueMonitorAgentStatus {
                queue: vec![2338],
                active_launches: Vec::new(),
                max_active: 1,
                enabled: true,
                autonomous_mode: true,
                has_launch_profile: true,
                quota_hold: None,
                launch_profile_summary: String::new(),
                launch_profile_candidates: Vec::new(),
                usage_threshold_percent: 80,
                provider_quota_holds: Vec::new(),
                needs_human: vec![2338],
                inbox: vec![crate::issue_monitor::IssueMonitorInboxSummary {
                    issue_number: 2338,
                    state: crate::MonitorInboxState::Queued,
                    github_state: crate::IssueMonitorIssueState::Open,
                    issue_updated_at: issue_updated_at.clone(),
                    readiness: crate::IssueMonitorReadiness::NotApplicable,
                    recoverable_merged: false,
                    completion_reason: None,
                    blocked_by_owner: None,
                    launched_window_id: None,
                    error_message: None,
                    last_activity_at: None,
                    retry_not_before: None,
                    retry_hold_reason: None,
                    claim_id: None,
                    delivery_id: None,
                    waiting: None,
                    steering: None,
                }],
                last_error: None,
                last_scan_at: None,
                scan_stall: None,
                github_budget: None,
                generation_reclaim: None,
            };
            merge_board_escalations_into_needs_human(&repo, &mut status);
            assert_eq!(
                status.queue,
                vec![2338],
                "live Open row with {issue_updated_at:?} must fail open"
            );
            assert_eq!(status.needs_human, vec![2338]);
            assert_eq!(status.inbox.len(), 1);
        }
    }

    #[test]
    fn closed_cache_reconciliation_does_not_depend_on_the_board_index() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&repo);
        gwt_github::Cache::new(cache_root)
            .write_snapshot(&IssueSnapshot {
                number: IssueNumber(2338),
                title: "Closed owner".to_string(),
                body: String::new(),
                labels: Vec::new(),
                state: IssueState::Closed,
                updated_at: UpdatedAt::new("2026-08-26T00:00:00Z"),
                comments: Vec::new(),
            })
            .expect("write closed cache entry");
        let escalation_path = gwt_core::coordination::coordination_escalations_path(&repo);
        std::fs::create_dir_all(&escalation_path).expect("make escalation index unreadable");
        let mut published = crate::IssueMonitorAgentStatus {
            queue: vec![2338],
            active_launches: Vec::new(),
            max_active: 1,
            enabled: true,
            autonomous_mode: true,
            has_launch_profile: true,
            quota_hold: None,
            launch_profile_summary: String::new(),
            launch_profile_candidates: Vec::new(),
            usage_threshold_percent: 80,
            provider_quota_holds: Vec::new(),
            needs_human: vec![2338],
            inbox: Vec::new(),
            last_error: Some("issue #2338: stale failure".to_string()),
            last_scan_at: None,
            scan_stall: None,
            github_budget: None,
            generation_reclaim: None,
        };

        merge_board_escalations_into_needs_human(&repo, &mut published);

        assert!(published.queue.is_empty());
        assert!(published.needs_human.is_empty());
        assert_eq!(published.last_error, None);
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
        let mut status =
            serde_json::from_str::<serde_json::Value>(out.trim()).expect("status json");
        // Issue #3928 AC-4: the GitHub budget block is read from the live
        // ledger at call time and has its own test; the queue projection is
        // compared without it.
        assert!(
            status
                .as_object_mut()
                .expect("status object")
                .remove("github_budget")
                .is_some(),
            "the offline fallback reports the GitHub budget too: {out}"
        );
        assert_eq!(
            status,
            serde_json::json!({
                "queue": [2, 1],
                "active_launches": [9],
                "max_active": 3,
                "enabled": true,
                "autonomous_mode": false,
                "has_launch_profile": false,
                "launch_profile_summary": "configure before auto start",
                "launch_profile_candidates": [],
                "usage_threshold_percent": 80,
                // SPEC-3431 FR-024: the offline fallback serializes the same
                // projection as the daemon branch, so a caller sees one shape
                // regardless of whether the daemon happens to be publishing.
                "needs_human": [],
                "inbox": [
                    {
                        "issue_number": 2,
                        "state": "queued",
                        "github_state": "open",
                        "issue_updated_at": "2026-08-03T00:00:00Z",
                        "readiness": "not_applicable",
                        "recoverable_merged": false,
                    },
                    {
                        "issue_number": 1,
                        "state": "queued",
                        "github_state": "open",
                        "issue_updated_at": "2026-08-03T00:00:00Z",
                        "readiness": "not_applicable",
                        "recoverable_merged": false,
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

    /// Issue #3928 AC-4: the PM must be able to see, from the one snapshot it
    /// already reads, that GitHub is throttling, until when, and which callers
    /// spent the last minute's budget.
    #[test]
    fn issue_monitor_status_reports_the_github_budget_state() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let ledger = gwt_core::github_budget::BudgetLedger::global();
        let now = chrono::Utc::now();
        let refusal = |at: chrono::DateTime<chrono::Utc>| gwt_core::github_quota::RateLimitBlock {
            resource: "graphql".to_string(),
            limit: 0,
            remaining: 0,
            reset_at: at + chrono::Duration::seconds(60),
        };
        ledger.record_block(
            &refusal(now - chrono::Duration::seconds(120)),
            now - chrono::Duration::seconds(120),
        );
        ledger.record_block(&refusal(now), now);
        ledger.record_spawn_from(
            gwt_core::github_quota::GitHubQuota::GraphQl,
            "gwt gh issue view",
            now - chrono::Duration::seconds(5),
        );
        ledger.record_spawn_from(
            gwt_core::github_quota::GitHubQuota::GraphQl,
            "gwt gh issue view",
            now - chrono::Duration::seconds(4),
        );
        ledger.record_spawn_from(
            gwt_core::github_quota::GitHubQuota::GraphQl,
            "gwtd gh pr list",
            now - chrono::Duration::seconds(3),
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        run(
            &mut env,
            IssueCommand::MonitorStatus { project_root: None },
            &mut out,
        )
        .expect("status");

        let status: serde_json::Value = serde_json::from_str(out.trim()).expect("status json");
        let graphql = &status["github_budget"]["graphql"];
        assert_eq!(graphql["throttled"], true, "{status}");
        assert!(
            graphql["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("github_rate_limited")),
            "{status}"
        );
        assert!(graphql["backoff_until"].is_string(), "{status}");
        assert!(
            graphql["retry_after_secs"]
                .as_i64()
                .is_some_and(|secs| (0..=120).contains(&secs)),
            "the second refusal in a row waits two minutes: {status}"
        );
        assert_eq!(graphql["consecutive_refusals"], 2, "{status}");
        assert_eq!(graphql["calls_last_minute"], 3, "{status}");
        assert_eq!(
            graphql["sources_last_minute"]["gwt gh issue view"], 2,
            "{status}"
        );
        assert_eq!(
            graphql["sources_last_minute"]["gwtd gh pr list"], 1,
            "{status}"
        );
        assert_eq!(
            status["github_budget"]["core"]["throttled"], false,
            "{status}"
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
                auto_close_merged_issues: None,
                launch_agent: None,
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
                auto_close_merged_issues: None,
                launch_agent: None,
            },
            &mut out,
        )
        .is_err());
        assert_eq!(std::fs::read(&prefs_path).expect("prefs bytes"), before);
    }

    fn pool_profile(agent_id: &str, prefer_for: &[&str]) -> crate::IssueMonitorLaunchProfile {
        crate::IssueMonitorLaunchProfile {
            agent_id: agent_id.to_string(),
            model: None,
            reasoning: None,
            version: None,
            session_mode: Default::default(),
            skip_permissions: false,
            codex_fast_mode: false,
            runtime_target: Default::default(),
            docker_service: None,
            docker_lifecycle_intent: Default::default(),
            windows_shell: None,
            prefer_for: prefer_for.iter().map(|tag| tag.to_string()).collect(),
        }
    }

    /// SPEC #3914 FR-011 / AC-8 / SC-6: the pool is written whole, mirrored
    /// into `launch_profile`, and read back with holds and the threshold.
    #[test]
    fn issue_monitor_profiles_set_replaces_the_pool_and_profiles_reads_it_back() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                launch_profile: Some(pool_profile("claude", &[])),
                provider_quota_holds: std::collections::BTreeMap::from([(
                    "codex".to_string(),
                    "2999-01-01T04:00:00Z".to_string(),
                )]),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorProfilesSet {
                project_root: Some(repo.clone()),
                profiles: vec![
                    pool_profile("codex", &[]),
                    pool_profile("claude", &["kind:spec"]),
                ],
                usage_threshold_percent: Some(70),
            },
            &mut out,
        )
        .expect("profiles set");
        assert_eq!(code, 0);

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        let pool = prefs.launch_profile_pool();
        assert_eq!(
            pool.iter()
                .map(|profile| profile.agent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex", "claude"]
        );
        assert_eq!(prefs.launch_profile.as_ref(), Some(&pool[0]));
        assert_eq!(pool[1].prefer_for, vec!["kind:spec".to_string()]);
        assert_eq!(prefs.launch_usage_threshold_percent, 70);
        assert_eq!(
            prefs.effect_authority_epoch, 1,
            "a pool change revokes prepared effects like the GUI save does"
        );
        assert_eq!(
            prefs.provider_quota_holds.get("codex").map(String::as_str),
            Some("2999-01-01T04:00:00Z"),
            "the write leaves holds untouched"
        );

        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorProfiles {
                project_root: Some(repo),
            },
            &mut out,
        )
        .expect("profiles read");
        assert_eq!(code, 0);
        let payload: serde_json::Value = serde_json::from_str(out.trim()).expect("profiles json");
        assert_eq!(payload["usage_threshold_percent"], 70);
        assert_eq!(payload["launch_profiles"].as_array().map(Vec::len), Some(2));
        assert_eq!(payload["launch_profiles"][0]["index"], 0);
        assert_eq!(payload["launch_profiles"][0]["agent_id"], "codex");
        assert_eq!(
            payload["launch_profiles"][0]["held_until"],
            "2999-01-01T04:00:00Z"
        );
        assert!(payload["launch_profiles"][0]["summary"]
            .as_str()
            .is_some_and(|summary| !summary.is_empty()));
        assert_eq!(payload["launch_profiles"][1]["prefer_for"][0], "kind:spec");
        assert!(payload["launch_profiles"][1].get("held_until").is_none());
        assert_eq!(payload["provider_quota_holds"][0]["provider"], "codex");
        assert!(payload["launch_profile_summary"]
            .as_str()
            .is_some_and(|summary| summary.starts_with("auto (2): ")));
    }

    #[test]
    fn issue_monitor_profiles_set_rejects_invalid_pools_without_writing() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                launch_profile: Some(pool_profile("claude", &[])),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let rejected: Vec<(&str, Vec<crate::IssueMonitorLaunchProfile>, Option<u8>)> = vec![
            ("empty pool", Vec::new(), None),
            (
                "duplicate provider",
                vec![pool_profile("codex", &[]), pool_profile("Codex", &[])],
                None,
            ),
            ("unknown agent", vec![pool_profile("nope", &[])], None),
            ("blank agent", vec![pool_profile("  ", &[])], None),
            (
                "tag without prefix",
                vec![pool_profile("codex", &["perf"])],
                None,
            ),
            (
                "uppercase tag",
                vec![pool_profile("codex", &["type:Perf"])],
                None,
            ),
            (
                "unknown tag prefix",
                vec![pool_profile("codex", &["repo:gwt"])],
                None,
            ),
            ("threshold zero", vec![pool_profile("codex", &[])], Some(0)),
            (
                "threshold over 100",
                vec![pool_profile("codex", &[])],
                Some(101),
            ),
        ];
        for (case, profiles, usage_threshold_percent) in rejected {
            let mut out = String::new();
            let result = run(
                &mut env,
                IssueCommand::MonitorProfilesSet {
                    project_root: Some(repo.clone()),
                    profiles,
                    usage_threshold_percent,
                },
                &mut out,
            );
            assert!(result.is_err(), "{case} must be rejected");
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs bytes"),
                before,
                "{case} must not touch prefs"
            );
        }
    }

    /// Issue #3814 AC-2/AC-3: registered PM identity must not open a hidden
    /// JSON path around the GUI-only ON boundary, and rejection is effect-free.
    #[test]
    fn issue_monitor_config_set_rejects_pm_on_direction_without_changing_status() {
        use gwt_core::test_support::ScopedEnvVar;

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
                enabled: false,
                autonomous_mode: false,
                max_active_agents: 3,
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let before = std::fs::read(&prefs_path).expect("prefs bytes");

        let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        crate::pm_registry::try_register_pm(
            &pm_prefs_path,
            crate::pm_registry::PmRegistration {
                session_id: "pm-session".to_string(),
                agent_id: "claude".to_string(),
                worktree_path: repo.to_string_lossy().into_owned(),
                created_at: None,
                consecutive_crashes: 0,
                next_not_before: None,
            },
            |_| false,
        )
        .expect("register PM");
        assert!(crate::pm_registry::session_is_registered_pm(
            &pm_prefs_path,
            "pm-session"
        ));
        let _pm = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        for (enabled, autonomous_mode) in [(Some(true), None), (None, Some(true))] {
            let mut out = String::new();
            let result = run(
                &mut env,
                IssueCommand::MonitorConfigSet {
                    project_root: Some(repo.clone()),
                    enabled,
                    autonomous_mode,
                    max_active: None,
                    auto_close_merged_issues: None,
                    launch_agent: None,
                },
                &mut out,
            );
            assert!(result.is_err(), "PM JSON ON request must be refused");
            assert!(out.is_empty(), "a refusal must not report applied state");
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs bytes after refusal"),
                before,
                "a refused ON request must not change persisted state"
            );
        }

        let mut status_out = String::new();
        run(
            &mut env,
            IssueCommand::MonitorStatus {
                project_root: Some(repo),
            },
            &mut status_out,
        )
        .expect("status after refusal");
        let status: serde_json::Value =
            serde_json::from_str(status_out.trim()).expect("status JSON");
        assert_eq!(status["enabled"], false);
        assert_eq!(status["autonomous_mode"], false);
        assert_eq!(status["max_active"], 3);
    }

    /// SPEC-3431 FR-033 / T-087b: the operation the PM actually calls.
    ///
    /// Drives the whole path a `gwtd` invocation takes — load prefs, evaluate
    /// the identity, commit or refuse — because the unit matrix on
    /// `IssueMonitorState` cannot catch a handler that writes prefs on a
    /// refusal or reports success it did not achieve.
    #[test]
    fn monitor_stop_revokes_the_launch_and_refuses_a_stale_identity() {
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
        let mut env = crate::cli::TestEnv::new(repo.clone());

        // A stale window id must change nothing on disk.
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo.clone()),
                number: 42,
                reason: "provider rate limit".to_string(),
                claim_id: None,
                delivery_id: None,
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
                reason: "provider rate limit".to_string(),
                claim_id: None,
                delivery_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"stopped\""), "{out}");
        assert!(out.contains("tab-1::agent-1"), "{out}");
        assert!(
            out.contains("pane.close"),
            "the caller must be told the pane is still theirs to close: {out}"
        );

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
                reason: "provider rate limit".to_string(),
                claim_id: None,
                delivery_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"already_stopped\""), "{out}");
    }

    /// SPEC-3431 FR-029〜031 / T-081: the failover the PM calls when a provider
    /// runs out of quota.
    #[test]
    fn monitor_failover_requeues_at_the_head_and_refuses_a_stale_identity() {
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
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorFailover {
                project_root: Some(repo.clone()),
                number: 42,
                reason: "codex rate limit".to_string(),
                claim_id: None,
                delivery_id: None,
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
                reason: "codex rate limit".to_string(),
                claim_id: None,
                delivery_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
            },
            &mut out,
        )
        .expect("failover runs");
        assert_eq!(code, 0);
        assert!(out.contains("\"status\":\"restarting\""), "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.launched_issues.is_empty(),
            "the old launch must be revoked on disk"
        );
        assert_eq!(
            prefs.priority_order.first().copied(),
            Some(42),
            "the failed-over issue must be first in line for the new profile"
        );
        assert!(
            prefs.failed_issues.is_empty(),
            "a failover is not a failure and must not leave a hold behind"
        );
    }

    /// Issue #3844: a wait declaration is only meaningful for a launch that is
    /// running right now, so a row without a live launch is refused with zero
    /// mutation instead of being silently accepted.
    #[test]
    fn monitor_wait_refuses_an_issue_without_a_live_launch() {
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
                    issue_number: 43,
                    window_id: "tab-1::agent-live".to_string(),
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorWait {
                project_root: Some(repo.clone()),
                number: Some(42),
                reason: Some("順番待ち".to_string()),
                resume_condition: Some("前の agent の完了".to_string()),
                clear: false,
            },
            &mut out,
        )
        .expect("wait runs");
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("\"status\":\"refused\""), "{out}");
        assert!(out.contains("not_launched"), "{out}");
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs bytes"),
            before,
            "a refused declaration must be zero-mutation"
        );
    }

    /// Issue #3844: the launch context names the owner Issue, so an agent may
    /// omit `number`; an explicit `number` still wins.
    #[test]
    fn monitor_wait_issue_number_falls_back_to_the_launch_context() {
        assert_eq!(
            resolve_monitor_wait_issue_number(Some(42), Some("3844")),
            Some(42)
        );
        assert_eq!(
            resolve_monitor_wait_issue_number(None, Some("3844")),
            Some(3844)
        );
        assert_eq!(resolve_monitor_wait_issue_number(None, Some(" ")), None);
        assert_eq!(resolve_monitor_wait_issue_number(None, None), None);
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
                autonomous_records: vec![{
                    let mut record = crate::AutonomousIssueRecord::new(42);
                    record.attempts = 3;
                    record
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
        let response: serde_json::Value =
            serde_json::from_str(out.trim()).expect("requeue response is JSON");
        assert_eq!(response["attempts_before"], 3);
        assert_eq!(response["attempts_after"], 0);

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
        assert_eq!(prefs.autonomous_records[0].attempts, 0);
        assert_eq!(prefs.requeue_audit.len(), 1);
        assert_eq!(prefs.requeue_audit[0].reason, "operator recovery");
        assert_eq!(prefs.requeue_audit[0].attempts_before, 3);
        assert_eq!(prefs.requeue_audit[0].attempts_after, 0);
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

    #[test]
    fn monitor_requeue_releases_a_stale_completion_hold_instead_of_refusing_not_held() {
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
                issue_completion_migration_version:
                    crate::issue_monitor::ISSUE_COMPLETION_MIGRATION_VERSION,
                completion_records: vec![crate::issue_monitor::IssueCompletionRecord {
                    issue_number: 42,
                    generation: 7,
                    state: crate::issue_monitor::IssueCompletionState::Completed,
                    issue_updated_at: Some("2026-08-15T00:00:00Z".to_string()),
                    evidence: crate::issue_monitor::IssueCompletionEvidence::LinkedPr,
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

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

        assert_eq!(code, 0);
        let response: serde_json::Value =
            serde_json::from_str(out.trim()).expect("requeue response is JSON");
        assert_eq!(response["status"], "requeued");
        assert_eq!(response["released_hold"], "completion");
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("persisted prefs");
        assert!(persisted.merged_issues.is_empty());
        assert_eq!(persisted.completion_records[0].generation, 8);
        assert_eq!(
            persisted.completion_records[0].state,
            crate::issue_monitor::IssueCompletionState::Reopened
        );
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
    fn stale_issue_cache_revalidates_and_surfaces_remote_state() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.state = IssueState::Open;
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.state = IssueState::Closed;
        remote.updated_at = UpdatedAt::new("2026-08-13T01:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("stale issue should revalidate");

        assert_eq!(loaded.snapshot.state, IssueState::Closed);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_unchanged_issue_renews_receipt_without_a_second_fetch() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut snapshot = sample_issue_snapshot();
        snapshot.body = "cached body must survive NotModified".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&snapshot)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &snapshot, "2020-01-01T00:00:00Z");
        let mut remote = snapshot.clone();
        remote.body = "remote body must not transfer on NotModified".to_string();
        env.client.seed(remote);

        let first = load_or_refresh_issue(&mut env, snapshot.number, false)
            .expect("stale issue should revalidate");
        let second = load_or_refresh_issue(&mut env, snapshot.number, false)
            .expect("renewed receipt should be fresh");

        assert_eq!(first.snapshot.body, snapshot.body);
        assert_eq!(second.snapshot.body, snapshot.body);
        assert_eq!(
            Cache::new(env.cache_root())
                .load_entry(snapshot.number)
                .expect("cached issue after NotModified")
                .snapshot
                .body,
            snapshot.body
        );
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_validation_sidecar_conditionally_revalidates_and_renews_generation() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.body = "cached complete body".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        let stale_generation =
            write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.body = "remote body must not replace NotModified cache".to_string();
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("stale validation should conditionally revalidate");

        assert_eq!(loaded.snapshot.body, cached.body);
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
        let receipt: serde_json::Value = serde_json::from_slice(
            &fs::read(
                temp.path()
                    .join(cached.number.0.to_string())
                    .join("issue-validation.json"),
            )
            .expect("renewed validation receipt"),
        )
        .expect("parse renewed receipt");
        assert_ne!(receipt["generation"], stale_generation);
    }

    #[test]
    fn stale_issue_comments_refresh_remote_changes() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.updated_at = UpdatedAt::new("2026-08-13T02:00:00Z");
        remote.comments = vec![CommentSnapshot {
            id: CommentId(9001),
            body: "fresh remote comment".to_string(),
            updated_at: remote.updated_at.clone(),
        }];
        env.client.seed(remote);

        let mut out = String::new();
        run(
            &mut env,
            IssueCommand::Comments {
                number: cached.number.0,
                refresh: false,
            },
            &mut out,
        )
        .expect("stale comments should revalidate");

        assert!(out.contains("fresh remote comment"));
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn cache_without_validation_sidecar_full_fetches_partial_comments() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut partial = sample_issue_snapshot();
        partial.comments.clear();
        Cache::new(env.cache_root())
            .write_snapshot(&partial)
            .expect("write bulk-like partial cache");

        let mut remote = partial.clone();
        remote.comments = vec![CommentSnapshot {
            id: CommentId(9002),
            body: "comment omitted by bulk list snapshot".to_string(),
            updated_at: remote.updated_at.clone(),
        }];
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, partial.number, false)
            .expect("unvalidated partial cache should full fetch");

        assert_eq!(loaded.snapshot.comments.len(), 1);
        assert_eq!(
            loaded.snapshot.comments[0].body,
            "comment omitted by bulk list snapshot"
        );
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_linked_pr_cache_refreshes_independently() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        write_linked_prs_cache(
            temp.path(),
            number,
            &[LinkedPrSummary {
                number: 100,
                title: "cached PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/100".to_string(),
                will_close_target: false,
                merged_at: None,
            }],
        )
        .expect("write linked PR cache");
        set_modified(&linked_prs_cache_path(temp.path(), number), stale_time());
        env.seed_linked_prs(
            number.0,
            vec![LinkedPrSummary {
                number: 101,
                title: "fresh PR".to_string(),
                state: "MERGED".to_string(),
                url: "https://example.test/101".to_string(),
                will_close_target: true,
                merged_at: None,
            }],
        );

        let linked = load_or_refresh_linked_prs(&mut env, number, false)
            .expect("stale linked PRs should refresh");

        assert_eq!(linked[0].number, 101);
        assert_eq!(env.linked_pr_calls(), vec![42]);
    }

    #[test]
    fn stale_linked_pr_revalidation_error_does_not_return_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        write_linked_prs_cache(
            temp.path(),
            number,
            &[LinkedPrSummary {
                number: 100,
                title: "stale cached PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/100".to_string(),
                will_close_target: false,
                merged_at: None,
            }],
        )
        .expect("write linked PR cache");
        let receipt = linked_prs_cache_path(temp.path(), number);
        let stale = stale_time();
        set_modified(&receipt, stale);
        env.seed_linked_pr_error(number.0, "linked PR refresh failed");

        let error = load_or_refresh_linked_prs(&mut env, number, false)
            .expect_err("failed linked PR refresh must fail closed");

        assert!(error.to_string().contains("linked PR refresh failed"));
        assert_eq!(env.linked_pr_calls(), vec![42]);
        assert!(!cache_resource_is_fresh(&receipt));
    }

    #[test]
    fn corrupt_linked_pr_cache_is_replaced_from_remote() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let number = IssueNumber(42);
        let receipt = linked_prs_cache_path(temp.path(), number);
        fs::create_dir_all(receipt.parent().expect("cache directory"))
            .expect("create cache directory");
        fs::write(&receipt, "{not-json").expect("write corrupt linked PR cache");
        env.seed_linked_prs(
            number.0,
            vec![LinkedPrSummary {
                number: 101,
                title: "recovered PR".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/101".to_string(),
                will_close_target: true,
                merged_at: None,
            }],
        );

        let linked = load_or_refresh_linked_prs(&mut env, number, false)
            .expect("corrupt linked PR cache should refresh");

        assert_eq!(linked[0].number, 101);
        assert_eq!(env.linked_pr_calls(), vec![42]);
        assert_eq!(
            read_linked_prs_cache(temp.path(), number)
                .expect("repaired linked PR cache")
                .expect("linked PR cache should exist")[0]
                .number,
            101
        );
    }

    #[test]
    fn future_dated_issue_receipt_is_revalidated() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2999-01-01T00:00:00Z");

        let mut remote = cached.clone();
        remote.title = "future receipt was revalidated".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T03:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, false)
            .expect("future receipt should revalidate");

        assert_eq!(loaded.snapshot.title, "future receipt was revalidated");
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn stale_issue_revalidation_error_does_not_return_cached_data() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");
        write_issue_validation_receipt(temp.path(), &cached, "2020-01-01T00:00:00Z");

        let error = load_or_refresh_issue(&mut env, cached.number, false)
            .expect_err("failed revalidation must fail closed");

        assert!(error.to_string().contains("not found"));
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
    }

    #[test]
    fn explicit_issue_refresh_bypasses_a_fresh_receipt() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write cached issue");

        let mut remote = cached.clone();
        remote.title = "explicitly refreshed".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T04:00:00Z");
        env.client.seed(remote);

        let loaded = load_or_refresh_issue(&mut env, cached.number, true)
            .expect("explicit refresh should fetch");

        assert_eq!(loaded.snapshot.title, "explicitly refreshed");
        assert_eq!(env.client.call_log(), vec!["fetch:#42".to_string()]);
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

    #[test]
    fn index_rebuild_failure_keeps_receipt_absent_and_next_read_retries() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let mut cached = sample_issue_snapshot();
        cached.title = "old cache".to_string();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");

        let mut remote = cached.clone();
        remote.title = "remote snapshot".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T05:00:00Z");
        env.client.seed(remote.clone());

        let mut rebuild_calls = 0;
        let first =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Err("injected rebuild failure".to_string())
            })
            .expect_err("first index rebuild should fail");
        assert!(first.to_string().contains("injected rebuild failure"));
        assert!(!Cache::new(env.cache_root())
            .validation_receipt_path(remote.number)
            .exists());

        let second =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Ok(())
            })
            .expect("unvalidated cache must retry index rebuild");
        assert_eq!(second.snapshot.title, remote.title);

        let third =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, |_| {
                rebuild_calls += 1;
                Ok(())
            })
            .expect("validated cache should be a warm hit");
        assert_eq!(third.snapshot.title, remote.title);
        assert_eq!(rebuild_calls, 2);
        assert_eq!(
            env.client.call_log(),
            vec!["fetch:#42".to_string(), "fetch:#42".to_string()]
        );
    }

    #[test]
    fn generation_change_during_rebuild_prevents_receipt_publication() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");
        let mut remote = cached.clone();
        remote.title = "remote snapshot".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T06:00:00Z");
        env.client.seed(remote.clone());
        let cache_root = env.cache_root();
        let mut concurrent = remote.clone();
        concurrent.title = "concurrent writer wins".to_string();

        let error =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, move |_| {
                Cache::new(cache_root.clone())
                    .write_snapshot(&concurrent)
                    .map_err(|error| error.to_string())
            })
            .expect_err("changed generation must reject receipt publication");

        assert!(error
            .to_string()
            .contains("changed before validation receipt"));
        let cache = Cache::new(env.cache_root());
        assert_eq!(
            cache
                .load_entry(remote.number)
                .expect("concurrent cache")
                .snapshot
                .title,
            "concurrent writer wins"
        );
        assert!(!cache.validation_receipt_path(remote.number).exists());
    }

    #[test]
    fn identical_snapshot_aba_during_rebuild_rejects_original_generation() {
        let temp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(temp.path().to_path_buf());
        let cached = sample_issue_snapshot();
        Cache::new(env.cache_root())
            .write_snapshot(&cached)
            .expect("write old cache");
        let mut remote = cached.clone();
        remote.title = "same bytes after concurrent commit".to_string();
        remote.updated_at = UpdatedAt::new("2026-08-13T07:00:00Z");
        env.client.seed(remote.clone());
        let cache_root = env.cache_root();
        let concurrent = remote.clone();

        let error =
            load_or_refresh_issue_with_index_rebuild(&mut env, remote.number, false, move |_| {
                Cache::new(cache_root.clone())
                    .write_snapshot(&concurrent)
                    .map_err(|error| error.to_string())
            })
            .expect_err("identical bytes with a different UUID must fail generation CAS");

        assert!(error
            .to_string()
            .contains("changed before validation receipt"));
        let cache = Cache::new(env.cache_root());
        let persisted = cache.load_entry(remote.number).unwrap().snapshot;
        assert_eq!(persisted.title, remote.title);
        assert_eq!(persisted.body, remote.body);
        assert_eq!(persisted.updated_at, remote.updated_at);
        assert_eq!(persisted.comments[0].body, remote.comments[0].body);
        assert!(!cache.validation_receipt_path(remote.number).exists());
    }

    // Issue #3873 AC-1: `issue.create` with the auto-merge label refuses a
    // body the Monitor's classifier cannot read, instead of creating an Issue
    // that silently lands in needs_human.
    #[test]
    fn issue_create_with_auto_merge_refuses_body_without_acceptance_block() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let mut out = String::new();
        let err = run(
            &mut env,
            IssueCommand::CreateBody {
                title: "fix: something".to_string(),
                body: "## 成功基準\n- [ ] AC-1: hidden under the wrong heading\n".to_string(),
                labels: vec!["auto-merge".to_string()],
            },
            &mut out,
        )
        .expect_err("auto-merge without a machine-checkable AC block must be refused");
        let message = err.to_string();
        assert!(
            message.contains("受け入れ基準") && message.contains("AC-"),
            "error must tell the author the required block shape, got: {message}"
        );
        assert!(
            !env.client
                .call_log()
                .iter()
                .any(|c| c.contains("create_issue")),
            "no Issue may be created when validation fails: {:?}",
            env.client.call_log()
        );
    }

    #[test]
    fn issue_create_with_auto_merge_accepts_classifier_readable_block() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::CreateBody {
                title: "fix: something".to_string(),
                body: "## 受け入れ基準\n- [ ] AC-1: cargo test is GREEN\n".to_string(),
                labels: vec!["Auto-Merge".to_string()],
            },
            &mut out,
        )
        .expect("well-formed AC block is accepted");
        assert_eq!(code, 0);
        assert!(out.contains("created issue #"), "out = {out}");
    }

    #[test]
    fn issue_create_without_auto_merge_does_not_require_acceptance_block() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::CreateBody {
                title: "docs: typo".to_string(),
                body: "free text, no criteria".to_string(),
                labels: vec!["documentation".to_string()],
            },
            &mut out,
        )
        .expect("plain issues keep today's behaviour");
        assert_eq!(code, 0);
    }

    /// Issue #3923 AC-5: the PM switches the saved profile's agent from the
    /// CLI; without a saved profile the switch is refused before anything is
    /// published or written.
    #[test]
    fn config_set_launch_agent_switches_the_saved_profile_or_refuses_without_one() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let mut out = String::new();
        assert!(run(
            &mut env,
            IssueCommand::MonitorConfigSet {
                project_root: Some(repo.clone()),
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                auto_close_merged_issues: None,
                launch_agent: Some("claude".to_string()),
            },
            &mut out,
        )
        .is_err());
        assert!(!prefs_path.exists(), "a refused switch writes nothing");

        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(crate::IssueMonitorLaunchProfile {
                    agent_id: "codex".to_string(),
                    model: Some("gpt-5.5".to_string()),
                    reasoning: Some("high".to_string()),
                    version: None,
                    session_mode: Default::default(),
                    skip_permissions: true,
                    codex_fast_mode: false,
                    runtime_target: Default::default(),
                    docker_service: None,
                    docker_lifecycle_intent: Default::default(),
                    windows_shell: None,
                    prefer_for: Vec::new(),
                }),
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorConfigSet {
                project_root: Some(repo),
                enabled: None,
                autonomous_mode: None,
                max_active: None,
                auto_close_merged_issues: None,
                launch_agent: Some("Claude".to_string()),
            },
            &mut out,
        )
        .expect("config set");
        assert_eq!(code, 0, "output: {out}");
        let result: serde_json::Value = serde_json::from_str(out.trim()).expect("result JSON");
        assert_eq!(result["launch_profile"], "claude / default / auto / host");
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        let profile = prefs.launch_profile.expect("profile survives");
        assert_eq!(profile.agent_id, "claude");
        assert_eq!(profile.model, None);
        assert_eq!(profile.reasoning, None);
        assert!(
            profile.skip_permissions,
            "the wizard's permission choice is kept"
        );
    }

    /// Issue #3923 AC-1 / AC-4: the PM lists a provider hold with its evidence
    /// and clears it by provider; the release is durable and readmits the
    /// issues the hold was holding.
    #[test]
    fn quota_hold_list_and_clear_roundtrip_releases_the_provider_hold() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let reset_at = (chrono::Utc::now() + chrono::Duration::days(3))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                priority_order: vec![42],
                provider_quota_holds: std::collections::BTreeMap::from([(
                    "codex".to_string(),
                    reset_at.clone(),
                )]),
                provider_quota_hold_evidence: std::collections::BTreeMap::from([(
                    "codex".to_string(),
                    crate::IssueMonitorProviderQuotaHoldEvidence {
                        recorded_at: "2026-09-02T09:01:00Z".to_string(),
                        source: "screen_notice".to_string(),
                        issue_number: Some(42),
                        window_id: Some("tab-1::agent-42".to_string()),
                        screen_text: Some("You've hit your usage limit".to_string()),
                        poller_state: Some("ok".to_string()),
                        poller_limit_reached: Some(false),
                        poller_windows: vec![crate::IssueMonitorProviderQuotaPollerWindow {
                            kind: "weekly".to_string(),
                            used_percent: 26,
                        }],
                    },
                )]),
                autonomous_records: vec![crate::AutonomousIssueRecord {
                    issue_number: 42,
                    retry_not_before: Some(reset_at.clone()),
                    retry_hold_reason: Some("Codex usage limit reached".to_string()),
                    retry_hold_provider: Some("codex".to_string()),
                    ..crate::AutonomousIssueRecord::new(42)
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("seed prefs");
        let mut env = crate::cli::TestEnv::new(repo);

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldList { project_root: None },
            &mut out,
        )
        .expect("list result");
        assert_eq!(code, 0);
        let listed: serde_json::Value = serde_json::from_str(out.trim()).expect("list JSON");
        assert_eq!(listed["provider_quota_holds"][0]["provider"], "codex");
        assert_eq!(listed["provider_quota_holds"][0]["reset_at"], reset_at);
        assert_eq!(
            listed["provider_quota_holds"][0]["evidence"]["screen_text"],
            "You've hit your usage limit"
        );
        assert_eq!(
            listed["provider_quota_holds"][0]["evidence"]["poller_windows"][0]["used_percent"],
            26
        );

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "codex".to_string(),
                reason: "PM: Codex is not rate limited".to_string(),
            },
            &mut out,
        )
        .expect("clear result");
        assert_eq!(code, 0, "clear output: {out}");
        let cleared: serde_json::Value = serde_json::from_str(out.trim()).expect("clear JSON");
        assert_eq!(cleared["provider"], "codex");
        assert_eq!(cleared["status"], "cleared");
        assert_eq!(cleared["released_reset_at"], reset_at);
        assert_eq!(cleared["released_issues"], serde_json::json!([42]));
        assert_eq!(cleared["provider_quota_holds"], serde_json::json!([]));
        // Issue #3961: no daemon owns the state here, so the durable prefs
        // are the authority and the response says which path was taken.
        assert_eq!(cleared["delivery"], "durable_prefs");

        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("persisted prefs");
        assert!(persisted.provider_quota_holds.is_empty());
        assert!(persisted.provider_quota_hold_evidence.is_empty());
        let release = persisted
            .provider_quota_hold_releases
            .get("codex")
            .expect("release fence is durable");
        assert_eq!(release.reason, "PM: Codex is not rate limited");
        assert_eq!(
            release.released_reset_at.as_deref(),
            Some(reset_at.as_str())
        );
        let record = persisted
            .autonomous_records
            .iter()
            .find(|record| record.issue_number == 42)
            .expect("the record survives");
        assert_eq!(record.retry_not_before, None);
        assert_eq!(record.retry_hold_provider, None);

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldList { project_root: None },
            &mut out,
        )
        .expect("list result");
        assert_eq!(code, 0);
        let listed: serde_json::Value = serde_json::from_str(out.trim()).expect("list JSON");
        assert_eq!(listed["provider_quota_holds"], serde_json::json!([]));

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "codex".to_string(),
                reason: "again".to_string(),
            },
            &mut out,
        )
        .expect("second clear result");
        assert_eq!(code, 0);
        let cleared: serde_json::Value = serde_json::from_str(out.trim()).expect("clear JSON");
        assert_eq!(cleared["status"], "not_held");

        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "   ".to_string(),
                reason: "x".to_string(),
            },
            &mut out,
        )
        .expect("unknown provider result");
        assert_eq!(code, 1);
        let refused: serde_json::Value = serde_json::from_str(out.trim()).expect("refusal JSON");
        assert_eq!(refused["status"], "refused");
        assert_eq!(refused["refusal"], "unknown_provider");
    }

    /// Issue #3961: one prefs file holding a live `codex` quota hold, as the
    /// PM sees it before a release.
    #[cfg(unix)]
    fn seed_codex_quota_hold(prefs_path: &Path) -> crate::IssueMonitorPrefs {
        let reset_at = (chrono::Utc::now() + chrono::Duration::days(3))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let seed = crate::IssueMonitorPrefs {
            enabled: true,
            provider_quota_holds: std::collections::BTreeMap::from([(
                "codex".to_string(),
                reset_at,
            )]),
            ..crate::IssueMonitorPrefs::default()
        };
        crate::save_issue_monitor_prefs(prefs_path, &seed).expect("seed prefs");
        seed
    }

    /// Issue #3961: a fake daemon that owns the control lane and answers one
    /// `quota_hold_clear` publish with `reply` without touching the prefs —
    /// the shape of a daemon that predates the control (rejects) and of one
    /// that acknowledges without adopting the release.
    #[cfg(unix)]
    fn spawn_quota_hold_clear_daemon(
        tmp: &Path,
        repo: &Path,
        reply: gwt_core::daemon::DaemonFrame,
    ) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
        let scope = gwt_core::daemon::RuntimeScope::from_project_root(
            repo,
            gwt_core::daemon::RuntimeTarget::Host,
        )
        .expect("runtime scope");
        let socket_path = tmp.join("quota-hold.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind live daemon");
        listener
            .set_nonblocking(true)
            .expect("nonblocking live daemon");
        let endpoint = gwt_core::daemon::DaemonEndpoint::new(
            scope.clone(),
            std::process::id(),
            socket_path.to_string_lossy().to_string(),
            "quota-hold-token".to_string(),
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
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !server_stop.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("accept control client: {error}"),
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking control client stream");
                let mut reader =
                    std::io::BufReader::new(stream.try_clone().expect("clone control stream"));
                let mut writer = stream;
                let mut line = String::new();
                reader.read_line(&mut line).expect("read control handshake");
                let request: gwt_core::daemon::IpcHandshakeRequest =
                    serde_json::from_str(line.trim_end()).expect("parse control handshake");
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
                    .expect("serialize control handshake")
                )
                .expect("write control handshake");
                line.clear();
                reader.read_line(&mut line).expect("read control publish");
                let gwt_core::daemon::ClientFrame::Publish { channel, payload } =
                    serde_json::from_str::<gwt_core::daemon::ClientFrame>(line.trim_end())
                        .expect("parse control publish")
                else {
                    panic!("expected a control publish, got {line}");
                };
                assert_eq!(
                    channel,
                    crate::runtime_daemon_events::ISSUE_MONITOR_CONTROL_CHANNEL
                );
                assert_eq!(payload["payload"]["quota_hold_clear"]["provider"], "codex");
                writeln!(
                    writer,
                    "{}",
                    serde_json::to_string(&reply).expect("serialize control reply")
                )
                .expect("write control reply");
                return;
            }
        });
        (stop, server)
    }

    /// Issue #3961 AC-4: the live daemon owns the authoritative hold
    /// state. When it rejects the release — a daemon that predates the control
    /// does exactly this — the CLI must neither fall back to a disk write the
    /// daemon would overwrite nor report `cleared`.
    #[test]
    #[cfg(unix)]
    fn quota_hold_clear_fails_closed_when_the_live_daemon_rejects_the_release() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let seed = seed_codex_quota_hold(&prefs_path);
        let (stop, server) = spawn_quota_hold_clear_daemon(
            tmp.path(),
            &repo,
            gwt_core::daemon::DaemonFrame::Error {
                message: "unknown issue monitor control".to_string(),
            },
        );

        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();
        let result = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "codex".to_string(),
                reason: "PM ruling".to_string(),
            },
            &mut out,
        );
        stop.store(true, Ordering::Release);
        server.join().expect("live daemon joins");
        let code = result.expect("clear result");
        assert_eq!(code, 1, "clear output: {out}");
        let refused: serde_json::Value = serde_json::from_str(out.trim()).expect("refusal JSON");
        assert_eq!(refused["status"], "refused");
        assert_eq!(refused["refusal"], "daemon_rejected");
        assert!(
            refused["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("unknown issue monitor control"),
            "the daemon's reason reaches the caller: {refused}"
        );
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("prefs");
        assert_eq!(
            persisted.provider_quota_holds, seed.provider_quota_holds,
            "a refused release must leave the durable hold untouched"
        );
        assert!(persisted.provider_quota_hold_releases.is_empty());
    }

    /// Issue #3961 AC-4: an acknowledgment is not adoption. When the daemon
    /// acks but the durable prefs still hold the provider and carry no release
    /// fence, `cleared` would be exactly the silent no-op this Issue is about.
    #[test]
    #[cfg(unix)]
    fn quota_hold_clear_refuses_when_an_acking_daemon_does_not_adopt_the_release() {
        let tmp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(tmp.path().join("home"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let seed = seed_codex_quota_hold(&prefs_path);
        let (stop, server) =
            spawn_quota_hold_clear_daemon(tmp.path(), &repo, gwt_core::daemon::DaemonFrame::Ack);

        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();
        let result = run(
            &mut env,
            IssueCommand::MonitorQuotaHoldClear {
                project_root: None,
                provider: "codex".to_string(),
                reason: "PM ruling".to_string(),
            },
            &mut out,
        );
        stop.store(true, Ordering::Release);
        server.join().expect("live daemon joins");
        let code = result.expect("clear result");
        assert_eq!(code, 1, "clear output: {out}");
        let refused: serde_json::Value = serde_json::from_str(out.trim()).expect("refusal JSON");
        assert_eq!(refused["status"], "refused");
        assert_eq!(refused["refusal"], "not_adopted");
        let persisted = crate::load_issue_monitor_prefs(&prefs_path).expect("prefs");
        assert_eq!(persisted.provider_quota_holds, seed.provider_quota_holds);
        assert!(persisted.provider_quota_hold_releases.is_empty());
    }
}

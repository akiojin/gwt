use std::{fs, io, path::PathBuf};

use gwt_github::{
    cache::write_atomic, client::ApiError, Cache, IssueClient, IssueNumber, IssueSnapshot,
    IssueState, SpecOpsError,
};

use crate::cli::{
    CliEnv, CliParseError, IssueCommand, IssueMonitorPriorityPosition, LinkedPrSummary,
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
    Ok(gwt_core::paths::resolve_current_worktree_root(&canonical))
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
        let status = serde_json::from_value::<crate::IssueMonitorAgentStatus>(status)
            .map_err(|error| io_as_api_error(io::Error::other(error)))?;
        out.push_str(
            &serde_json::to_string(&status)
                .map_err(|error| io_as_api_error(io::Error::other(error)))?,
        );
        out.push('\n');
        return Ok(0);
    }
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let mut monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&project_root);
    let candidates = crate::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root)
        .map_err(|error| io_as_api_error(io::Error::other(error)))?;
    crate::scan_issue_monitor_candidates(&mut monitor, &candidates, "gwtd-status");
    // Serialize through the same projection as the daemon branch above. The
    // offline fallback used to hand-roll an equivalent JSON object, so every
    // field added to the snapshot had to be added twice or the two branches
    // would silently disagree about what a caller can rely on.
    out.push_str(
        &serde_json::to_string(&monitor.agent_status())
            .map_err(|error| io_as_api_error(io::Error::other(error)))?,
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
/// driver re-reads) and ask the daemon for one immediate scan. The launch
/// itself stays on the Monitor's claim/slot path, so this cannot produce a
/// duplicate agent. Without a reachable daemon the reorder still lands and the
/// next scheduled scan picks it up; the response says which happened.
fn run_monitor_launch_now<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    number: u64,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let (prefs, ()) = crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
        prefs.priority_order.retain(|existing| *existing != number);
        prefs.priority_order.insert(0, number);
        Ok(())
    })
    .map_err(io_as_api_error)?;

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({ "scan_now": {} }),
        std::process::id(),
    );
    let scan_requested = publish_monitor_config_set(&project_root, payload).is_ok();

    out.push_str(
        &serde_json::json!({
            "number": number,
            "priority_order": prefs.priority_order,
            "scan_requested": scan_requested,
            "scan_delivery": if scan_requested { "immediate" } else { "next-scheduled-scan" },
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
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
        if !matches!(outcome, crate::IssueMonitorFailoverOutcome::Mismatch(_)) {
            *prefs = monitor.prefs();
        }
        Ok(outcome)
    })
    .map_err(io_as_api_error)?;

    let stopped_window_id = match outcome {
        crate::IssueMonitorFailoverOutcome::Restarting { stopped_window_id } => stopped_window_id,
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

    let payload = crate::runtime_daemon_events::issue_monitor_payload(
        "control",
        serde_json::json!({ "scan_now": {} }),
        std::process::id(),
    );
    let scan_requested = publish_monitor_config_set(&project_root, payload).is_ok();

    out.push_str(
        &serde_json::json!({
            "number": number,
            "status": "restarting",
            "reason": reason,
            "stopped_window_id": stopped_window_id,
            "priority_order": prefs.priority_order,
            "launch_profile": prefs.launch_profile.as_ref().map(|profile| &profile.agent_id),
            "scan_requested": scan_requested,
            "scan_delivery": if scan_requested { "immediate" } else { "next-scheduled-scan" },
            "pane_teardown": if stopped_window_id.is_some() {
                "close the returned window with pane.close — it is no longer bound to the issue, so the close cannot requeue it"
            } else {
                "none"
            },
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
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
        if let Some(existing) = index.get(&pr_number) {
            out[*existing].will_close_target |= will_close_target;
            continue;
        }
        index.insert(pr_number, out.len());
        out.push(LinkedPrSummary {
            number: pr_number,
            will_close_target,
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
             "source":{"__typename":"PullRequest","number":10,"title":"closes it","state":"MERGED","url":"u10","body":""}},
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
                "inbox": [
                    { "issue_number": 2, "state": "queued" },
                    { "issue_number": 1, "state": "queued" },
                ],
                "last_scan_at": "gwtd-status",
            })
        );
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

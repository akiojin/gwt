use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

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
            operation_id,
            reason,
            launch_generation,
            claim_id,
            claim_owner,
            delivery_id,
            materializer_window_id,
            window_id,
            launch_profile_fingerprint,
        } => run_monitor_pm_control(
            env,
            project_root.as_deref(),
            MonitorPmControlInput {
                number,
                legacy_generated_id: operation_id.is_none(),
                operation_id: operation_id.unwrap_or_else(generated_legacy_pm_control_id),
                reason,
                action: crate::IssueMonitorPmControlAction::Stop,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
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
            launch_profile_fingerprint,
        } => run_monitor_pm_control(
            env,
            project_root.as_deref(),
            MonitorPmControlInput {
                number,
                legacy_generated_id: operation_id.is_none(),
                operation_id: operation_id.unwrap_or_else(generated_legacy_pm_control_id),
                reason,
                action: crate::IssueMonitorPmControlAction::Failover,
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
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
            launch_profile_fingerprint,
        } => run_monitor_pm_control(
            env,
            project_root.as_deref(),
            MonitorPmControlInput {
                number,
                legacy_generated_id: false,
                operation_id,
                reason,
                action: crate::IssueMonitorPmControlAction::Recover,
                launch_generation: Some(launch_generation),
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
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
    Ok(gwt_core::paths::resolve_current_worktree_root(&canonical))
}

/// Select the projection with the newer durable PM-control revision. A stale
/// resident daemon must never hide a control target already committed to disk.
pub(crate) fn merge_issue_monitor_status_projection(
    resident: serde_json::Value,
    disk: serde_json::Value,
) -> serde_json::Value {
    fn revision(value: &serde_json::Value) -> u64 {
        value
            .get("pm_control")
            .and_then(|control| control.get("revision"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    }
    let runtime_inventory = resident
        .get("runtime_inventory")
        .filter(|inventory| issue_monitor_runtime_inventory_is_authoritative(inventory))
        .cloned();
    let mut merged = if revision(&disk) > revision(&resident) {
        disk
    } else {
        resident
    };
    if let Some(inventory) = runtime_inventory {
        apply_issue_monitor_runtime_inventory_projection(&mut merged, &inventory);
        merged["runtime_inventory"] = inventory;
    }
    if let Some(object) = merged.as_object_mut() {
        object.remove("pane_list");
        object.remove("elapsed_secs");
    }
    merged
}

fn issue_monitor_runtime_inventory_is_authoritative(inventory: &serde_json::Value) -> bool {
    let Some(object) = inventory.as_object() else {
        return false;
    };
    let allowed_available = [
        "availability",
        "project_scope",
        "runtime_instance_id",
        "revision",
        "observed_at",
        "windows",
    ];
    let allowed_unavailable = ["availability", "project_scope", "observed_at", "reason"];
    match object
        .get("availability")
        .and_then(serde_json::Value::as_str)
    {
        Some("available") => {
            object
                .keys()
                .all(|key| allowed_available.contains(&key.as_str()))
                && object
                    .get("project_scope")
                    .is_some_and(serde_json::Value::is_string)
                && object
                    .get("runtime_instance_id")
                    .is_some_and(serde_json::Value::is_string)
                && object
                    .get("revision")
                    .is_some_and(serde_json::Value::is_u64)
                && object
                    .get("observed_at")
                    .is_some_and(serde_json::Value::is_string)
                && object
                    .get("windows")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|windows| {
                        windows.iter().all(issue_monitor_runtime_window_is_typed)
                    })
        }
        Some("unavailable") => {
            object
                .keys()
                .all(|key| allowed_unavailable.contains(&key.as_str()))
                && ["project_scope", "observed_at", "reason"]
                    .iter()
                    .all(|field| object.get(*field).is_some_and(serde_json::Value::is_string))
        }
        _ => false,
    }
}

fn issue_monitor_runtime_window_is_typed(window: &serde_json::Value) -> bool {
    let Some(object) = window.as_object() else {
        return false;
    };
    if !object
        .keys()
        .all(|key| ["window_id", "pane_state", "wait_signal"].contains(&key.as_str()))
        || !object
            .get("window_id")
            .is_some_and(serde_json::Value::is_string)
        || !matches!(
            object.get("pane_state").and_then(serde_json::Value::as_str),
            Some("running" | "stopped" | "error" | "idle" | "waiting")
        )
    {
        return false;
    }
    match object.get("wait_signal") {
        None | Some(serde_json::Value::Null) => true,
        Some(wait) => {
            let Some(wait) = wait.as_object() else {
                return false;
            };
            match wait.get("kind").and_then(serde_json::Value::as_str) {
                Some("approval_required") => wait.len() == 1,
                Some("provider_usage_limit") => {
                    wait.keys()
                        .all(|key| ["kind", "provider", "resets_at"].contains(&key.as_str()))
                        && wait
                            .get("provider")
                            .is_some_and(serde_json::Value::is_string)
                        && wait
                            .get("resets_at")
                            .is_none_or(|value| value.is_null() || value.is_string())
                }
                _ => false,
            }
        }
    }
}

fn apply_issue_monitor_runtime_inventory_projection(
    status: &mut serde_json::Value,
    inventory: &serde_json::Value,
) {
    let Some(rows) = status
        .get_mut("inbox")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for row in rows {
        if row.get("state").and_then(serde_json::Value::as_str) != Some("launched") {
            continue;
        }
        let (pane_state, consistency, typed_wait, recover_reason) = if inventory
            .get("availability")
            .and_then(serde_json::Value::as_str)
            == Some("unavailable")
        {
            (None, "unavailable", None, "runtime_inventory_unavailable")
        } else {
            let target = row
                .get("launched_window_id")
                .and_then(serde_json::Value::as_str);
            let matching = inventory
                .get("windows")
                .and_then(serde_json::Value::as_array)
                .map(|windows| {
                    windows
                        .iter()
                        .filter(|window| {
                            window.get("window_id").and_then(serde_json::Value::as_str) == target
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match matching.as_slice() {
                [] => (None, "missing", None, "execution_unverified"),
                [window] => {
                    let pane_state = window.get("pane_state").and_then(serde_json::Value::as_str);
                    let terminal = matches!(pane_state, Some("stopped" | "error"));
                    (
                        pane_state,
                        if terminal { "terminal" } else { "consistent" },
                        window
                            .get("wait_signal")
                            .filter(|signal| !signal.is_null())
                            .cloned(),
                        if terminal {
                            "execution_unverified"
                        } else {
                            "runtime_live"
                        },
                    )
                }
                _ => (None, "ambiguous", None, "runtime_ambiguous"),
            }
        };
        if let Some(pane_state) = pane_state {
            row["pane_state"] = serde_json::json!(pane_state);
        } else if let Some(object) = row.as_object_mut() {
            object.remove("pane_state");
        }
        row["runtime_consistency"] = serde_json::json!(consistency);
        if let Some(typed_wait) = typed_wait {
            row["wait_reason"] = typed_wait;
        } else if row
            .get("wait_reason")
            .and_then(|wait| wait.get("kind"))
            .and_then(serde_json::Value::as_str)
            != Some("unresponsive")
        {
            row["wait_reason"] = serde_json::Value::Null;
        }
        row["control_ready"]["recover"]["ready"] = serde_json::Value::Bool(false);
        row["control_ready"]["recover"]["degraded_reason"] = serde_json::json!(recover_reason);
    }
}

fn issue_monitor_runtime_inventory_unavailable(
    project_root: &Path,
    reason: &str,
) -> serde_json::Value {
    serde_json::json!({
        "availability": "unavailable",
        "project_scope": gwt_core::paths::project_scope_hash(project_root).as_str(),
        "observed_at": chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "reason": reason,
    })
}

fn overlay_issue_monitor_runtime_inventory(
    status: &mut serde_json::Value,
    inventory: serde_json::Value,
) {
    apply_issue_monitor_runtime_inventory_projection(status, &inventory);
    status["runtime_inventory"] = inventory;
}

fn project_canonical_unresponsive_waits(
    status: &mut serde_json::Value,
    monitor: &crate::IssueMonitorState,
    now: &str,
) {
    let stuck = monitor
        .stuck_autonomous_issues(now)
        .into_iter()
        .collect::<HashSet<_>>();
    let fresh_inbox = monitor.agent_status().inbox;
    let Some(rows) = status
        .get_mut("inbox")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    for row in rows {
        let issue_number = row.get("issue_number").and_then(serde_json::Value::as_u64);
        let canonical = row.get("state").and_then(serde_json::Value::as_str) == Some("launched")
            && issue_number.is_some_and(|issue| stuck.contains(&issue));
        if canonical {
            let last_activity_at = issue_number
                .and_then(|issue| fresh_inbox.iter().find(|fresh| fresh.issue_number == issue))
                .and_then(|fresh| fresh.last_activity_at.clone())
                .map_or(serde_json::Value::Null, serde_json::Value::String);
            row["wait_reason"] = serde_json::json!({
                "kind": "unresponsive",
                "last_activity_at": last_activity_at,
            });
        } else if row
            .get("wait_reason")
            .and_then(|wait| wait.get("kind"))
            .and_then(serde_json::Value::as_str)
            == Some("unresponsive")
        {
            row["wait_reason"] = serde_json::Value::Null;
        }
    }
}

fn run_monitor_status<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    #[cfg(unix)]
    let resident_status = crate::daemon_publisher::read_issue_monitor_status(&project_root)
        .map_err(|error| io_as_api_error(io::Error::other(error.to_string())))?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let disk_monitor =
        crate::IssueMonitorState::with_prefs(crate::IssueMonitorConfig::default(), prefs.clone());
    let disk_status = serde_json::to_value(disk_monitor.agent_status())
        .map_err(|error| io_as_api_error(io::Error::other(error)))?;
    #[cfg(unix)]
    let has_resident_status = resident_status.is_some();
    #[cfg(not(unix))]
    let has_resident_status = false;
    let mut status = disk_status;
    #[cfg(unix)]
    if let Some(resident_status) = resident_status {
        status = merge_issue_monitor_status_projection(resident_status, status);
    }
    if !has_resident_status {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            prefs.clone(),
        );
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path_or_detached(&project_root);
        let candidates =
            crate::issue_monitor_worker::load_cached_issue_monitor_candidates(&cache_root)
                .map_err(|error| io_as_api_error(io::Error::other(error)))?;
        crate::scan_issue_monitor_candidates(&mut monitor, &candidates, "gwtd-status");
        status = serde_json::to_value(monitor.agent_status())
            .map_err(|error| io_as_api_error(io::Error::other(error)))?;
    }
    project_canonical_unresponsive_waits(&mut status, &disk_monitor, &now);
    let inventory = super::pane::request_issue_monitor_status_runtime_inventory(&project_root)
        .ok()
        .filter(issue_monitor_runtime_inventory_is_authoritative)
        .unwrap_or_else(|| {
            issue_monitor_runtime_inventory_unavailable(
                &project_root,
                "runtime_inventory_unavailable",
            )
        });
    overlay_issue_monitor_runtime_inventory(&mut status, inventory);
    // Serialize through the same projection as the daemon branch above. The
    // offline fallback used to hand-roll an equivalent JSON object, so every
    // field added to the snapshot had to be added twice or the two branches
    // would silently disagree about what a caller can rely on.
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

struct MonitorPmControlInput {
    number: u64,
    legacy_generated_id: bool,
    operation_id: String,
    reason: String,
    action: crate::IssueMonitorPmControlAction,
    launch_generation: Option<u64>,
    claim_id: Option<String>,
    claim_owner: Option<String>,
    delivery_id: Option<String>,
    materializer_window_id: Option<String>,
    window_id: Option<String>,
    launch_profile_fingerprint: Option<String>,
}

fn run_monitor_pm_control<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    input: MonitorPmControlInput,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let MonitorPmControlInput {
        number,
        legacy_generated_id,
        operation_id,
        reason,
        action,
        launch_generation,
        claim_id,
        claim_owner,
        delivery_id,
        materializer_window_id,
        window_id,
        launch_profile_fingerprint,
    } = input;
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
    let ambient_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let project_key = gwt_core::paths::project_scope_hash(&project_root)
        .as_str()
        .to_string();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let authorized =
        crate::pm_registry::with_registered_pm_authority(&pm_prefs_path, &ambient_session, |_| {
            Ok(())
        })
        .map_err(io_as_api_error)?;
    if authorized.is_none() {
        return write_pm_control_refusal(out, "unauthorized");
    }
    // Select the exact ECR/Session authority before Monitor admission and
    // release every PM lease while doing so. A replay consumes the durable
    // first target; a new admission captures the one current Active target.
    let (execution_target, replay_admission) = if action
        == crate::IssueMonitorPmControlAction::Recover
    {
        let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
        let existing_receipt = prefs
            .pm_control
            .receipts
            .iter()
            .find(|receipt| receipt.operation_id == operation_id)
            .cloned();
        match existing_receipt {
            Some(receipt) => (receipt.execution_target, true),
            None => match capture_exact_execution_settlement_target(
                &project_root,
                number,
                &operation_id,
            ) {
                Ok(Some(target)) => (Some(target), false),
                Ok(None) => return write_pm_control_refusal(out, "execution_target_unavailable"),
                Err(_) => return write_pm_control_refusal(out, "execution_target_unavailable"),
            },
        }
    } else {
        (None, false)
    };
    let admit = || {
        crate::pm_registry::with_registered_pm_authority(
            &pm_prefs_path,
            &ambient_session,
            |principal| {
                let prefs = crate::load_issue_monitor_prefs(&prefs_path)?;
                let monitor = crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(),
                    prefs,
                );
                let replay = monitor
                    .prefs()
                    .pm_control
                    .receipts
                    .iter()
                    .find(|receipt| receipt.operation_id == operation_id)
                    .cloned();
                let Some(mut target) = replay
                    .as_ref()
                    .map(|receipt| receipt.target.clone())
                    .or_else(|| monitor.pm_control_status(number).target)
                else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "PM control target is incomplete",
                    ));
                };
                let launch_generation = match launch_generation {
                    Some(generation) => generation,
                    None if legacy_generated_id => 0,
                    None => {
                        return Err(io::Error::other(
                            crate::IssueMonitorPmControlCommitError::Refused(
                                crate::IssueMonitorPmControlRefusal::TargetMismatch(
                                    crate::IssueMonitorPmControlTargetMismatch::Generation,
                                ),
                            ),
                        ));
                    }
                };
                target.issue_number = number;
                target.generation = launch_generation;
                target.claim_id = claim_id;
                target.claim_owner = claim_owner;
                target.delivery_id = delivery_id;
                target.materializer_window_id = materializer_window_id;
                target.launched_window_id = window_id;
                target.source_profile_fingerprint = launch_profile_fingerprint;
                let requested_profile = replay
                    .as_ref()
                    .and_then(|receipt| receipt.requested_profile.clone())
                    .or_else(|| match action {
                        crate::IssueMonitorPmControlAction::Stop => None,
                        crate::IssueMonitorPmControlAction::Failover => {
                            monitor.prefs().launch_profile.clone()
                        }
                        crate::IssueMonitorPmControlAction::Recover => {
                            target.source_profile.clone()
                        }
                    });
                let request = crate::IssueMonitorPmControlRequest {
                    operation_id: operation_id.clone(),
                    project_key,
                    action,
                    actor_session: principal.session_id().to_string(),
                    actor_registration_generation: principal.generation(),
                    target,
                    reason: reason.clone(),
                    requested_profile,
                    admitted_at: now,
                    admission_inventory_revision: 0,
                };
                crate::try_mutate_issue_monitor_pm_control_with_execution_target_classified(
                    &prefs_path,
                    request,
                    execution_target.clone(),
                )
                .map_err(io::Error::other)
            },
        )
    };
    let authorized = if action == crate::IssueMonitorPmControlAction::Recover && !replay_admission {
        let Some(identity) = execution_target
            .as_ref()
            .and_then(|target| target.session_identity.as_ref())
        else {
            return write_pm_control_refusal(out, "execution_target_unavailable");
        };
        match crate::cli::execution_state::with_current_active_session_execution_identity_global_lease(
            &gwt_core::paths::gwt_sessions_dir(),
            identity,
            admit,
        ) {
            Ok(Some(authorized)) => authorized,
            Ok(None) | Err(_) => {
                return write_pm_control_refusal(out, "execution_target_unavailable")
            }
        }
    } else {
        admit()
    };
    let authorized = match authorized {
        Ok(authorized) => authorized,
        Err(error) => return write_pm_control_error(out, &error),
    };
    let Some(committed) = authorized else {
        return write_pm_control_refusal(out, "unauthorized");
    };
    let (operation_id, outcome, reason, stopped_window_id, pending_prerequisites, exit_code) =
        match committed {
            crate::IssueMonitorPmControlCommit::Committed { receipt } => {
                let outcome = match &receipt.state {
                    crate::IssueMonitorPmControlReceiptState::HeldCleanupPending => {
                        "cleanup_pending"
                    }
                    crate::IssueMonitorPmControlReceiptState::PendingTeardown => "teardown_pending",
                    crate::IssueMonitorPmControlReceiptState::ClaimReleasePending => {
                        "claim_release_pending"
                    }
                    crate::IssueMonitorPmControlReceiptState::AutoMergeDisarmPending => {
                        "auto_merge_disarm_pending"
                    }
                    crate::IssueMonitorPmControlReceiptState::Terminal(_) => "settled",
                };
                (
                    receipt.operation_id,
                    outcome,
                    Some(receipt.reason),
                    receipt.target.launched_window_id,
                    receipt.pending_prerequisites,
                    0,
                )
            }
            crate::IssueMonitorPmControlCommit::OutcomeUnknown { operation_id } => {
                (operation_id, "outcome_unknown", None, None, Vec::new(), 1)
            }
        };
    out.push_str(
        &serde_json::json!({
            "operation_id": operation_id,
            "action": action,
            "outcome": outcome,
            "reason": reason,
            "stopped_window_id": stopped_window_id,
            "pending_prerequisites": pending_prerequisites,
            "scan_requested": false,
            "scan_delivery": "not_runnable",
        })
        .to_string(),
    );
    out.push('\n');
    Ok(exit_code)
}

fn generated_legacy_pm_control_id() -> String {
    format!("legacy-{}", uuid::Uuid::new_v4())
}

fn pm_control_target_mismatch_label(
    mismatch: crate::IssueMonitorPmControlTargetMismatch,
) -> &'static str {
    match mismatch {
        crate::IssueMonitorPmControlTargetMismatch::IssueNumber => "issue_number",
        crate::IssueMonitorPmControlTargetMismatch::Generation => "generation",
        crate::IssueMonitorPmControlTargetMismatch::ClaimId => "claim_id",
        crate::IssueMonitorPmControlTargetMismatch::ClaimOwner => "claim_owner",
        crate::IssueMonitorPmControlTargetMismatch::DeliveryId => "delivery_id",
        crate::IssueMonitorPmControlTargetMismatch::MaterializerId => "materializer_id",
        crate::IssueMonitorPmControlTargetMismatch::MaterializerPid => "materializer_pid",
        crate::IssueMonitorPmControlTargetMismatch::MaterializerWindow => "materializer_window",
        crate::IssueMonitorPmControlTargetMismatch::MaterializedWindow => "materialized_window",
        crate::IssueMonitorPmControlTargetMismatch::WorkspaceDurableWindow => {
            "workspace_durable_window"
        }
        crate::IssueMonitorPmControlTargetMismatch::LaunchedWindow => "launched_window",
        crate::IssueMonitorPmControlTargetMismatch::SourceProfile => "source_profile",
        crate::IssueMonitorPmControlTargetMismatch::SourceProfileFingerprint => {
            "source_profile_fingerprint"
        }
    }
}

fn write_pm_control_error(out: &mut String, error: &io::Error) -> Result<i32, SpecOpsError> {
    let commit_error = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<crate::IssueMonitorPmControlCommitError>());
    let (label, mismatch) = match commit_error {
        Some(crate::IssueMonitorPmControlCommitError::Refused(refusal)) => match refusal {
            crate::IssueMonitorPmControlRefusal::SourceProfileUnavailable => {
                ("source_profile_unavailable", None)
            }
            crate::IssueMonitorPmControlRefusal::IdentityIncomplete => {
                ("identity_incomplete", None)
            }
            crate::IssueMonitorPmControlRefusal::ReceiptConflict => ("receipt_conflict", None),
            crate::IssueMonitorPmControlRefusal::ReceiptCapacity => {
                ("receipt_capacity_exhausted", None)
            }
            crate::IssueMonitorPmControlRefusal::TargetMismatch(component) => (
                "target_mismatch",
                Some(pm_control_target_mismatch_label(*component)),
            ),
            crate::IssueMonitorPmControlRefusal::ControlRevisionExhausted
            | crate::IssueMonitorPmControlRefusal::GenerationExhausted
            | crate::IssueMonitorPmControlRefusal::EffectAuthorityExhausted => {
                ("authority_exhausted", None)
            }
        },
        Some(crate::IssueMonitorPmControlCommitError::PreRename(_)) => ("persistence_failed", None),
        None => ("identity_incomplete", None),
    };
    write_pm_control_refusal_with_mismatch(out, label, mismatch)
}

fn write_pm_control_refusal(out: &mut String, error: &str) -> Result<i32, SpecOpsError> {
    write_pm_control_refusal_with_mismatch(out, error, None)
}

fn write_pm_control_refusal_with_mismatch(
    out: &mut String,
    error: &str,
    mismatch: Option<&str>,
) -> Result<i32, SpecOpsError> {
    let mut response = serde_json::json!({ "status": "refused", "error": error });
    if let Some(mismatch) = mismatch {
        response["mismatch"] = serde_json::Value::String(mismatch.to_string());
    }
    out.push_str(&response.to_string());
    out.push('\n');
    Ok(1)
}

#[derive(Debug, Clone)]
struct ExactExecutionSettlementTarget {
    worktree: PathBuf,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    session: gwt_agent::SessionExecutionIdentity,
}

#[derive(Debug, Clone)]
struct ExactExecutionSettlementReceipt {
    target: ExactExecutionSettlementTarget,
    event: crate::cli::execution_state::GenerationLifecycleEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeInventoryFencePreflight {
    Ready,
    ReplacementFenceAdvanced,
}

fn exact_execution_settlement_candidate(
    worktree: &Path,
    issue_number: u64,
) -> io::Result<Option<ExactExecutionSettlementTarget>> {
    let Some(record) = crate::cli::execution_state::load(worktree)? else {
        return Ok(None);
    };
    if record.owner_number != issue_number {
        return Ok(None);
    }
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: record.owner_kind,
        number: record.owner_number,
    };
    let Some(ledger) = crate::cli::execution_state::load_generation_ledger(worktree, owner)? else {
        return Ok(None);
    };
    let Some(current) = ledger.current_generation() else {
        return Ok(None);
    };
    if ledger.current_effective_status()
        != Some(crate::cli::execution_state::ExecutionControlStatus::Active)
    {
        return Ok(None);
    }
    let session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", record.primary_session_id));
    let session = match gwt_agent::inspect_session_path(&session_path) {
        gwt_agent::SessionPathState::Present(session) => session,
        gwt_agent::SessionPathState::Missing | gwt_agent::SessionPathState::Error(_) => {
            return Ok(None)
        }
    };
    let Some(identity) = gwt_agent::SessionExecutionIdentity::from_session(&session)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    else {
        return Ok(None);
    };
    if identity.linked_issue_number != Some(issue_number)
        || identity.session_id != record.primary_session_id
        || !crate::cli::execution_state::execution_binding_authorizes_current_generation(
            worktree,
            owner,
            &identity.session_id,
            &identity.execution_binding.identity,
        )?
    {
        return Ok(None);
    }
    Ok(Some(ExactExecutionSettlementTarget {
        worktree: worktree.to_path_buf(),
        owner,
        session: identity,
    }))
}

fn exact_execution_project_worktrees(
    project_root: &Path,
    issue_number: u64,
) -> io::Result<HashSet<PathBuf>> {
    let project_scope = gwt_core::paths::project_scope_hash(project_root);
    let mut worktrees = HashSet::from([dunce::canonicalize(project_root)?]);
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    match fs::read_dir(&sessions_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.path().extension().and_then(|value| value.to_str()) != Some("toml") {
                    continue;
                }
                let Ok(session) = gwt_agent::Session::load(&entry.path()) else {
                    continue;
                };
                if session.linked_issue_number != Some(issue_number)
                    || gwt_core::paths::project_scope_hash(&session.worktree_path) != project_scope
                {
                    continue;
                }
                if let Ok(worktree) = dunce::canonicalize(&session.worktree_path) {
                    worktrees.insert(worktree);
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(worktrees)
}

fn capture_exact_execution_settlement_target(
    project_root: &Path,
    issue_number: u64,
    operation_id: &str,
) -> io::Result<Option<crate::IssueMonitorPmControlExecutionTarget>> {
    let mut candidates = Vec::new();
    for worktree in exact_execution_project_worktrees(project_root, issue_number)? {
        if let Some(candidate) = exact_execution_settlement_candidate(&worktree, issue_number)? {
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "matching execution generation is ambiguous",
        ));
    }
    let candidate = candidates.remove(0);
    Ok(Some(crate::IssueMonitorPmControlExecutionTarget {
        worktree_path: candidate.worktree,
        owner_kind: candidate.owner.kind.as_str().to_string(),
        owner_number: candidate.owner.number,
        generation_id: candidate
            .session
            .execution_binding
            .identity
            .generation_id
            .clone(),
        session_id: candidate.session.session_id.clone(),
        session_binding: candidate.session.execution_binding.clone(),
        session_identity: Some(candidate.session),
        settlement_operation_id: format!("issue-monitor-control:{operation_id}:exact-absence"),
    }))
}

fn resolve_exact_execution_settlement_target(
    project_root: &Path,
    pending: &crate::PendingIssueMonitorPmControl,
) -> io::Result<ExactExecutionSettlementTarget> {
    let durable = pending.execution_target.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pending control has no admission-bound execution target",
        )
    })?;
    let durable_identity = durable.session_identity.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pending control has no admission-bound full Session identity",
        )
    })?;
    let expected_operation_id = format!(
        "issue-monitor-control:{}:exact-absence",
        pending.operation_id
    );
    let owner_kind = match durable.owner_kind.as_str() {
        "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
        "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "admission-bound execution owner kind is invalid",
            ))
        }
    };
    let owner = crate::cli::execution_state::ExecutionOwnerKey {
        kind: owner_kind,
        number: durable.owner_number,
    };
    let worktree = dunce::canonicalize(&durable.worktree_path)?;
    if worktree != durable.worktree_path
        || durable.owner_number != pending.target.issue_number
        || durable.settlement_operation_id != expected_operation_id
        || durable.generation_id != durable.session_binding.identity.generation_id
        || durable.session_id != durable.session_binding.session_id
        || durable.owner_kind != durable.session_binding.owner_kind
        || durable.owner_number != durable.session_binding.owner_number
        || durable_identity.session_id != durable.session_id
        || durable_identity.execution_binding != durable.session_binding
        || durable_identity.linked_issue_number != Some(durable.owner_number)
        || dunce::canonicalize(&durable_identity.worktree_path)? != worktree
        || gwt_core::paths::project_scope_hash(&worktree)
            != gwt_core::paths::project_scope_hash(project_root)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "admission-bound execution identity is inconsistent",
        ));
    }
    let session_path =
        gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", durable.session_id));
    match gwt_agent::inspect_session_path(&session_path) {
        gwt_agent::SessionPathState::Present(session) => {
            let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "admission-bound execution Session has no binding",
                    )
                })?;
            if identity != *durable_identity {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "admission-bound execution Session was retargeted",
                ));
            }
        }
        gwt_agent::SessionPathState::Missing => {}
        gwt_agent::SessionPathState::Error(error) => return Err(error),
    }
    if !crate::cli::execution_state::execution_binding_authorizes_current_generation(
        &worktree,
        owner,
        &durable_identity.session_id,
        &durable_identity.execution_binding.identity,
    )? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "admission-bound execution generation is no longer current",
        ));
    }
    Ok(ExactExecutionSettlementTarget {
        worktree,
        owner,
        session: durable_identity.clone(),
    })
}

fn settle_exact_absent_execution_generation(
    project_root: &Path,
    pending: &crate::PendingIssueMonitorPmControl,
) -> io::Result<ExactExecutionSettlementReceipt> {
    let target = resolve_exact_execution_settlement_target(project_root, pending)?;
    let settlement = crate::cli::execution_state::settle_exact_absent_active_generation(
        &target.worktree,
        target.owner,
        pending
            .execution_target
            .as_ref()
            .map(|target| target.settlement_operation_id.as_str())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "pending control has no execution settlement operation",
                )
            })?,
        &gwt_core::paths::gwt_sessions_dir(),
        &target.session,
        "owning AppRuntime proved the Issue Monitor launch absent",
    )?;
    let event = match settlement {
        crate::cli::execution_state::ExactAbsentGenerationSettlement::Settled(event)
        | crate::cli::execution_state::ExactAbsentGenerationSettlement::Replayed(event) => event,
    };
    Ok(ExactExecutionSettlementReceipt { target, event })
}

fn run_monitor_control_reconcile<E: CliEnv>(
    env: &E,
    project_root: Option<&std::path::Path>,
    operation_id: &str,
    revoked_generation: u64,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let project_root = issue_monitor_project_root(env, project_root)?;
    let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&project_root);
    let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&project_root);
    let ambient_session = std::env::var(gwt_agent::GWT_SESSION_ID_ENV).unwrap_or_default();
    let authorized =
        crate::pm_registry::with_registered_pm_authority(&pm_prefs_path, &ambient_session, |_| {
            Ok(())
        })
        .map_err(io_as_api_error)?;
    if authorized.is_none() {
        return write_pm_control_refusal(out, "unauthorized");
    }

    let prefs = crate::load_issue_monitor_prefs(&prefs_path).map_err(io_as_api_error)?;
    let pending = prefs
        .pm_control
        .issues
        .values()
        .filter_map(|state| state.pending_control.as_ref())
        .find(|pending| pending.operation_id == operation_id)
        .cloned();
    let Some(pending) = pending else {
        return write_pm_control_refusal(out, "target_mismatch");
    };
    if pending.target.generation != revoked_generation {
        return write_pm_control_refusal(out, "target_mismatch");
    }
    if pending.teardown_settled {
        return write_reconcile_success(out, operation_id);
    }

    let proof = match super::pane::request_issue_monitor_runtime_inventory_proof(
        &project_root,
        operation_id,
        revoked_generation,
    ) {
        Ok(proof) => proof,
        Err(_) => return write_pm_control_refusal(out, "runtime_unavailable"),
    };
    match proof.observation {
        crate::RuntimeInventoryObservation::Present => {
            return write_pm_control_refusal(out, "still_present")
        }
        crate::RuntimeInventoryObservation::Unavailable => {
            return write_pm_control_refusal(out, "runtime_unavailable")
        }
        crate::RuntimeInventoryObservation::Ambiguous => {
            return write_pm_control_refusal(out, "runtime_ambiguous")
        }
        crate::RuntimeInventoryObservation::Absent => {}
    }
    if proof.project_key != pending.project_key
        || proof.generation != pending.target.generation
        || proof.materializer_id != pending.target.materializer_id
        || proof.materializer_pid != pending.target.materializer_pid
    {
        return write_pm_control_refusal(out, "target_mismatch");
    }
    let proof_runtime_id = match &proof.author {
        crate::RuntimeInventoryProofAuthor::OwningAppRuntime { runtime_id } => runtime_id,
        crate::RuntimeInventoryProofAuthor::CallerSupplied => {
            return write_pm_control_refusal(out, "target_mismatch")
        }
    };
    if pending
        .inventory_revision_fence
        .as_ref()
        .is_some_and(|fence| {
            fence.runtime_id == *proof_runtime_id
                && proof.inventory_revision <= fence.inventory_revision
        })
    {
        return write_pm_control_refusal(out, "inventory_stale");
    }

    // The runtime query can outlive the PM registration that authorized it.
    // Revalidate the exact pending proof under PM -> Issue locks. A proof from
    // an honest replacement runtime persists only its new revision fence;
    // the lease is then released without touching ECR. The same/newer-runtime
    // case is classified on a disposable state clone and persisted unchanged,
    // leaving ECR settlement strictly outside the PM lease hierarchy.
    let authorized =
        crate::pm_registry::with_registered_pm_authority(&pm_prefs_path, &ambient_session, |_| {
            crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                let before = prefs.clone();
                let mut monitor = crate::IssueMonitorState::with_prefs(
                    crate::IssueMonitorConfig::default(),
                    before.clone(),
                );
                match monitor.reconcile_pm_control_absence(operation_id, proof.clone()) {
                    crate::IssueMonitorAbsenceReconcileOutcome::TeardownSettled
                    | crate::IssueMonitorAbsenceReconcileOutcome::AlreadySettled => {
                        Ok(RuntimeInventoryFencePreflight::Ready)
                    }
                    crate::IssueMonitorAbsenceReconcileOutcome::Refused => {
                        let rebased = monitor.prefs();
                        if rebased == before {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "runtime inventory proof no longer matches the pending target",
                            ));
                        }
                        *prefs = rebased;
                        Ok(RuntimeInventoryFencePreflight::ReplacementFenceAdvanced)
                    }
                }
            })
            .map(|(_, preflight)| preflight)
        });
    let authorized = match authorized {
        Ok(authorized) => authorized,
        Err(_) => return write_pm_control_refusal(out, "target_mismatch"),
    };
    let Some(preflight) = authorized else {
        return write_pm_control_refusal(out, "unauthorized");
    };
    if preflight == RuntimeInventoryFencePreflight::ReplacementFenceAdvanced {
        return write_pm_control_refusal(out, "runtime_replacement_fenced");
    }

    const EXECUTION_SETTLEMENT_UNKNOWN: &str = "execution_settlement_unknown";
    let settlement_receipt = match settle_exact_absent_execution_generation(&project_root, &pending)
    {
        Ok(receipt) => receipt,
        Err(_) => return write_pm_control_refusal(out, EXECUTION_SETTLEMENT_UNKNOWN),
    };

    // Revalidate the exact Blocked lifecycle receipt under the canonical
    // global -> owner -> Session hierarchy, then keep those leases while PM
    // -> Issue commits. If a successor wins the gap after ECR settlement it
    // is observed before Monitor can consume the stale receipt.
    let authorized = match crate::cli::execution_state::with_exact_terminal_session_execution_identity_global_lease(
        &gwt_core::paths::gwt_sessions_dir(),
        &settlement_receipt.target.session,
        &settlement_receipt.event,
        || {
            crate::pm_registry::with_registered_pm_authority(
                &pm_prefs_path,
                &ambient_session,
                |_| {
                    crate::try_mutate_issue_monitor_prefs(&prefs_path, |prefs| {
                        let mut monitor = crate::IssueMonitorState::with_prefs(
                            crate::IssueMonitorConfig::default(),
                            prefs.clone(),
                        );
                        match monitor.reconcile_pm_control_absence(operation_id, proof) {
                            crate::IssueMonitorAbsenceReconcileOutcome::TeardownSettled
                            | crate::IssueMonitorAbsenceReconcileOutcome::AlreadySettled => {
                                *prefs = monitor.prefs();
                                Ok(())
                            }
                            crate::IssueMonitorAbsenceReconcileOutcome::Refused => {
                                Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "runtime inventory proof no longer matches the pending target",
                                ))
                            }
                        }
                    })
                    .map(|_| ())
                },
            )
        },
    ) {
        Ok(Some(authorized)) => authorized,
        Ok(None) | Err(_) => {
            return write_pm_control_refusal(out, EXECUTION_SETTLEMENT_UNKNOWN)
        }
    };
    let authorized = match authorized {
        Ok(authorized) => authorized,
        Err(error) if error.to_string().contains(EXECUTION_SETTLEMENT_UNKNOWN) => {
            return write_pm_control_refusal(out, EXECUTION_SETTLEMENT_UNKNOWN)
        }
        Err(_) => return write_pm_control_refusal(out, "target_mismatch"),
    };
    let Some(committed) = authorized else {
        return write_pm_control_refusal(out, "unauthorized");
    };
    let () = committed;
    write_reconcile_success(out, operation_id)
}

fn write_reconcile_success(out: &mut String, operation_id: &str) -> Result<i32, SpecOpsError> {
    out.push_str(
        &serde_json::json!({
            "operation_id": operation_id,
            "outcome": "teardown_settled",
        })
        .to_string(),
    );
    out.push('\n');
    Ok(0)
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

    use fs2::FileExt;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};
    use gwt_github::client::{IssueSnapshot, IssueState, UpdatedAt};
    use tempfile::TempDir;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn t250_pm_registration(session_id: &str) -> crate::pm_registry::PmRegistration {
        serde_json::from_value(serde_json::json!({
            "session_id": session_id,
            "agent_id": "codex",
            "worktree_path": "/tmp/pm-worktree",
            "created_at": "2026-08-15T00:00:00Z",
        }))
        .expect("PM registration fixture")
    }

    fn t250_seed_legacy_launched_issue(repo: &std::path::Path, window_id: &str) -> PathBuf {
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(repo);
        let launched = serde_json::from_value(serde_json::json!({
            "issue_number": 42,
            "window_id": window_id,
        }))
        .expect("legacy generation-0 launch fixture");
        crate::save_issue_monitor_prefs(
            &prefs_path,
            &crate::IssueMonitorPrefs {
                enabled: true,
                launched_issues: vec![launched],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save Issue Monitor prefs");
        prefs_path
    }

    fn t250_register_pm(repo: &std::path::Path, session_id: &str) {
        crate::pm_registry::try_register_pm(
            &crate::pm_registry::pm_prefs_path_for_repo_path(repo),
            t250_pm_registration(session_id),
            |_| false,
        )
        .expect("register PM fixture");
    }

    fn t250_optional_file_snapshot(path: &std::path::Path) -> Option<Vec<u8>> {
        match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("snapshot {}: {error}", path.display()),
        }
    }

    fn t250_assert_file_unchanged(label: &str, path: &std::path::Path, before: &Option<Vec<u8>>) {
        assert_eq!(
            &t250_optional_file_snapshot(path),
            before,
            "{label}: durable file changed: {}",
            path.display()
        );
    }

    fn t250_assert_unauthorized_refusal(label: &str, result: Result<i32, SpecOpsError>, out: &str) {
        let code = result.unwrap_or_else(|error| panic!("{label}: handler error: {error}"));
        assert_ne!(code, 0, "{label}: unauthorized request reported success");
        let response: serde_json::Value =
            serde_json::from_str(out.trim()).unwrap_or_else(|error| {
                panic!("{label}: refusal was not one JSON object: {error}; out={out}")
            });
        assert_eq!(
            response,
            serde_json::json!({
                "status": "refused",
                "error": "unauthorized",
            }),
            "{label}: unauthorized refusal disclosed extra fields"
        );
    }

    fn t259_launch_profile(agent_id: &str, model: &str) -> crate::IssueMonitorLaunchProfile {
        crate::IssueMonitorLaunchProfile {
            agent_id: agent_id.to_string(),
            model: Some(model.to_string()),
            reasoning: Some("high".to_string()),
            version: Some("phase-24-review".to_string()),
            session_mode: gwt_agent::SessionMode::Normal,
            skip_permissions: true,
            codex_fast_mode: false,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            windows_shell: None,
        }
    }

    fn t259_generated_launch() -> crate::IssueMonitorState {
        let mut monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..crate::IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                enabled: true,
                launch_profile: Some(t259_launch_profile("codex", "gpt-source")),
                ..crate::IssueMonitorPrefs::default()
            },
        );
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[crate::IssueMonitorIssue {
                number: 42,
                title: "Issue 42".to_string(),
                labels: vec!["auto-merge".to_string()],
                state: crate::IssueMonitorIssueState::Open,
                body: None,
                url: None,
                readiness: crate::IssueMonitorReadiness::NotApplicable,
            }],
            "2026-08-29T00:00:00Z",
        );
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42-generation-1".to_string(),
            "host/pm-session",
            "effect-42-generation-1",
            "2026-08-29T00:00:01Z",
        ));
        assert!(monitor.claim_launch_delivery(
            42,
            "launch:effect-42-generation-1",
            "app-runtime-a",
            4242,
            "tab-1::agent-42",
            |_| false,
        ));
        assert!(monitor.mark_launch_delivery_materialized(
            42,
            "launch:effect-42-generation-1",
            "app-runtime-a",
            "tab-1::agent-42",
        ));
        assert!(monitor.mark_launch_delivery_workspace_durable(
            42,
            "launch:effect-42-generation-1",
            "app-runtime-a",
            "tab-1::agent-42",
        ));
        assert!(monitor.complete_active_launch_delivery(
            42,
            "tab-1::agent-42",
            Some("launch:effect-42-generation-1"),
        ));
        monitor
    }

    fn t259_seed_generated_launch(
        repo: &std::path::Path,
    ) -> (PathBuf, crate::IssueMonitorPmControlTarget) {
        let monitor = t259_generated_launch();
        let target = monitor
            .pm_control_status(42)
            .target
            .expect("generated control target");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(repo);
        crate::save_issue_monitor_prefs(&prefs_path, &monitor.prefs())
            .expect("persist generated launch");
        (prefs_path, target)
    }

    fn t259_control_command(
        project_root: PathBuf,
        operation_id: Option<&str>,
        action: crate::IssueMonitorPmControlAction,
        target: &crate::IssueMonitorPmControlTarget,
    ) -> IssueCommand {
        let fields = (
            project_root,
            42,
            operation_id.map(str::to_string),
            "review-derived control".to_string(),
            Some(target.generation),
            target.claim_id.clone(),
            target.claim_owner.clone(),
            target.delivery_id.clone(),
            target.materializer_window_id.clone(),
            target.launched_window_id.clone(),
            target.source_profile_fingerprint.clone(),
        );
        match action {
            crate::IssueMonitorPmControlAction::Stop => IssueCommand::MonitorStop {
                project_root: Some(fields.0),
                number: fields.1,
                operation_id: fields.2,
                reason: fields.3,
                launch_generation: fields.4,
                claim_id: fields.5,
                claim_owner: fields.6,
                delivery_id: fields.7,
                materializer_window_id: fields.8,
                window_id: fields.9,
                launch_profile_fingerprint: fields.10,
            },
            crate::IssueMonitorPmControlAction::Failover => IssueCommand::MonitorFailover {
                project_root: Some(fields.0),
                number: fields.1,
                operation_id: fields.2,
                reason: fields.3,
                launch_generation: fields.4,
                claim_id: fields.5,
                claim_owner: fields.6,
                delivery_id: fields.7,
                materializer_window_id: fields.8,
                window_id: fields.9,
                launch_profile_fingerprint: fields.10,
            },
            crate::IssueMonitorPmControlAction::Recover => IssueCommand::MonitorRecover {
                project_root: Some(fields.0),
                number: fields.1,
                operation_id: fields.2.expect("recover operation id"),
                reason: fields.3,
                launch_generation: fields.4.expect("recover generation"),
                claim_id: fields.5,
                claim_owner: fields.6,
                delivery_id: fields.7,
                materializer_window_id: fields.8,
                window_id: fields.9,
                launch_profile_fingerprint: fields.10,
            },
        }
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

    /// SPEC-3431 Phase 24R / T496: the status handler returns one complete
    /// projection. Runtime facts come only from the owning AppRuntime
    /// inventory, while a newer disk control revision supplies the durable
    /// target; neither half may replace the other.
    #[test]
    fn t496_status_handler_projects_runtime_table_and_merges_newer_disk_control() {
        let target = serde_json::json!({
            "issue_number": 42,
            "generation": 7,
            "claim_id": "claim-42-generation-7",
            "claim_owner": "host/source-agent",
            "delivery_id": "launch:effect-42-generation-7",
            "materializer_id": "app-runtime-a",
            "materializer_pid": 4242,
            "materializer_window_id": "tab-1::agent-42",
            "materialized_window_id": "tab-1::agent-42",
            "workspace_durable_window_id": "tab-1::agent-42",
            "launched_window_id": "tab-1::agent-42",
            "source_profile": {
                "agent_id": "codex",
                "model": "gpt-source"
            },
            "source_profile_fingerprint": "fingerprint-42"
        });
        let durable_row = serde_json::json!({
            "issue_number": 42,
            "state": "launched",
            "launched_window_id": "tab-1::agent-42",
            "last_activity_at": "2026-08-29T00:00:00Z",
            "control_ready": {
                "stop": { "ready": true },
                "failover": { "ready": true },
                "recover": { "ready": false, "degraded_reason": "runtime_unverified" }
            }
        });
        let inventory = |windows: serde_json::Value| {
            serde_json::json!({
                "availability": "available",
                "project_scope": "scope-gwt",
                "runtime_instance_id": "runtime-a",
                "revision": 17,
                "observed_at": "2026-08-29T00:31:00Z",
                "windows": windows,
            })
        };
        let window = |pane_state: &str, wait_signal: serde_json::Value| {
            serde_json::json!({
                "window_id": "tab-1::agent-42",
                "pane_state": pane_state,
                "wait_signal": wait_signal,
            })
        };
        let cases = vec![
            (
                "running",
                inventory(serde_json::json!([window(
                    "running",
                    serde_json::Value::Null
                )])),
                Some("running"),
                "consistent",
                serde_json::Value::Null,
                "runtime_live",
                false,
            ),
            (
                "stopped",
                inventory(serde_json::json!([window(
                    "stopped",
                    serde_json::Value::Null
                )])),
                Some("stopped"),
                "terminal",
                serde_json::Value::Null,
                "execution_unverified",
                false,
            ),
            (
                "error",
                inventory(serde_json::json!([window(
                    "error",
                    serde_json::Value::Null
                )])),
                Some("error"),
                "terminal",
                serde_json::Value::Null,
                "execution_unverified",
                false,
            ),
            (
                "missing",
                inventory(serde_json::json!([])),
                None,
                "missing",
                serde_json::Value::Null,
                "execution_unverified",
                false,
            ),
            (
                "ambiguous",
                inventory(serde_json::json!([
                    window("running", serde_json::Value::Null),
                    window("stopped", serde_json::Value::Null),
                ])),
                None,
                "ambiguous",
                serde_json::Value::Null,
                "runtime_ambiguous",
                false,
            ),
            (
                "unavailable",
                serde_json::json!({
                    "availability": "unavailable",
                    "project_scope": "scope-gwt",
                    "observed_at": "2026-08-29T00:31:00Z",
                    "reason": "runtime_inventory_unavailable"
                }),
                None,
                "unavailable",
                serde_json::Value::Null,
                "runtime_inventory_unavailable",
                false,
            ),
            (
                "provider_limit",
                inventory(serde_json::json!([window(
                    "waiting",
                    serde_json::json!({
                        "kind": "provider_usage_limit",
                        "provider": "codex",
                        "resets_at": "2026-08-29T03:00:00Z"
                    })
                )])),
                Some("waiting"),
                "consistent",
                serde_json::json!({
                    "kind": "provider_usage_limit",
                    "provider": "codex",
                    "resets_at": "2026-08-29T03:00:00Z"
                }),
                "runtime_live",
                false,
            ),
            (
                "approval",
                inventory(serde_json::json!([window(
                    "waiting",
                    serde_json::json!({ "kind": "approval_required" })
                )])),
                Some("waiting"),
                "consistent",
                serde_json::json!({ "kind": "approval_required" }),
                "runtime_live",
                false,
            ),
        ];

        for (
            label,
            runtime_inventory,
            expected_pane_state,
            expected_consistency,
            expected_wait,
            expected_recover_reason,
            canonical_unresponsive,
        ) in cases
        {
            let mut row = durable_row.clone();
            if canonical_unresponsive {
                row["wait_reason"] = expected_wait.clone();
            }
            let resident = serde_json::json!({
                "queue": [],
                "active_launches": [42],
                "pm_control": { "revision": 40 },
                "inbox": [row],
                "runtime_inventory": runtime_inventory,
                // These deliberately tempting inputs are not authoritative.
                "pane_list": [{
                    "window_id": "tab-1::agent-42",
                    "pane_state": "running"
                }],
                "elapsed_secs": 999_999,
            });
            let disk = serde_json::json!({
                "queue": [],
                "active_launches": [42],
                "pm_control": {
                    "revision": 41,
                    "issues": [{
                        "issue_number": 42,
                        "target": target.clone()
                    }]
                },
                "inbox": [durable_row.clone()],
            });

            let merged = super::merge_issue_monitor_status_projection(resident, disk.clone());
            assert_eq!(
                merged["pm_control"], disk["pm_control"],
                "{label}: the complete newer durable target stays in the one response"
            );
            let projected = &merged["inbox"][0];
            assert_eq!(
                projected
                    .get("pane_state")
                    .and_then(serde_json::Value::as_str),
                expected_pane_state,
                "{label}: pane state comes from exact owning inventory"
            );
            assert_eq!(
                projected["runtime_consistency"], expected_consistency,
                "{label}: pure runtime consistency projection"
            );
            assert_eq!(
                projected.get("wait_reason").cloned().unwrap_or_default(),
                expected_wait,
                "{label}: only typed wait signals or the canonical predicate author a wait"
            );
            assert_eq!(
                projected["control_ready"]["recover"]["degraded_reason"], expected_recover_reason,
                "{label}: response is complete enough to decide Recover"
            );
        }
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
        t250_register_pm(&repo, "pm-session");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
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

    /// SPEC-3431 Phase 24 T250 / AS-PM-CONTROL-AUTH-001: authorization is
    /// derived only from the ambient Session and the durable PM principal of
    /// the resolved target project.  Every definitive refusal is zero-write,
    /// including a caller that was replaced between status and control.
    #[test]
    fn t250_monitor_stop_refuses_non_pm_principals_with_byte_identical_prefs() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));

        for (label, registered, ambient) in [
            ("ordinary-agent", Some("pm-session"), Some("agent-session")),
            ("missing-session", Some("pm-session"), None),
            ("missing-registration", None, Some("unregistered-session")),
            (
                "replaced-pm",
                Some("replacement-session"),
                Some("old-pm-session"),
            ),
        ] {
            let repo = temp.path().join(label);
            std::fs::create_dir_all(&repo).expect("repo dir");
            let prefs_path = t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");
            if let Some(session) = registered {
                t250_register_pm(&repo, session);
            }
            let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
            let _session =
                ambient.map(|session| ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, session));
            let _missing_session = ambient
                .is_none()
                .then(|| ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV));
            let issue_before = t250_optional_file_snapshot(&prefs_path);
            let pm_before = t250_optional_file_snapshot(&pm_prefs_path);
            if registered.is_none() {
                assert!(
                    pm_before.is_none(),
                    "missing-registration fixture must begin without pm.json"
                );
            }
            let mut env = crate::cli::TestEnv::new(repo.clone());
            let mut out = String::new();
            let result = run(
                &mut env,
                IssueCommand::MonitorStop {
                    project_root: Some(repo),
                    number: 42,
                    operation_id: None,
                    reason: "control requested".to_string(),
                    launch_generation: None,
                    claim_id: None,
                    claim_owner: None,
                    delivery_id: None,
                    materializer_window_id: None,
                    window_id: Some("tab-1::agent-42".to_string()),
                    launch_profile_fingerprint: None,
                },
                &mut out,
            );
            t250_assert_unauthorized_refusal(label, result, &out);
            t250_assert_file_unchanged(label, &prefs_path, &issue_before);
            t250_assert_file_unchanged(label, &pm_prefs_path, &pm_before);
        }

        // A PM registered for another project cannot lend its authority to an
        // explicit foreign project_root.
        let caller_repo = temp.path().join("caller-project");
        let foreign_repo = temp.path().join("foreign-project");
        std::fs::create_dir_all(&caller_repo).expect("caller repo");
        std::fs::create_dir_all(&foreign_repo).expect("foreign repo");
        t250_register_pm(&caller_repo, "pm-session");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let prefs_path = t250_seed_legacy_launched_issue(&foreign_repo, "tab-1::agent-42");
        let caller_pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&caller_repo);
        let target_pm_path = crate::pm_registry::pm_prefs_path_for_repo_path(&foreign_repo);
        let issue_before = t250_optional_file_snapshot(&prefs_path);
        let caller_pm_before = t250_optional_file_snapshot(&caller_pm_path);
        let target_pm_before = t250_optional_file_snapshot(&target_pm_path);
        let mut env = crate::cli::TestEnv::new(caller_repo);
        let mut out = String::new();
        let result = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(foreign_repo),
                number: 42,
                operation_id: None,
                reason: "control requested".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-42".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        );
        t250_assert_unauthorized_refusal("foreign-project", result, &out);
        t250_assert_file_unchanged("foreign-project issue", &prefs_path, &issue_before);
        t250_assert_file_unchanged(
            "foreign-project caller PM",
            &caller_pm_path,
            &caller_pm_before,
        );
        t250_assert_file_unchanged(
            "foreign-project target PM",
            &target_pm_path,
            &target_pm_before,
        );
    }

    /// Canonicalization happens before PM lookup: the registered PM succeeds
    /// through a symlink spelling of its own project.
    #[cfg(unix)]
    #[test]
    fn t250_registered_pm_succeeds_through_canonical_project_resolution() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("canonical-project");
        let alias = temp.path().join("project-alias");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::os::unix::fs::symlink(&repo, &alias).expect("project symlink");
        t250_register_pm(&repo, "pm-session");
        t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let mut env = crate::cli::TestEnv::new(repo);
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(alias),
                number: 42,
                operation_id: None,
                reason: "registered PM stop".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-42".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("registered PM control");

        assert_eq!(code, 0, "{out}");
        let response: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern stop response");
        assert_eq!(response["action"], "stop", "{out}");
        assert_eq!(response["outcome"], "cleanup_pending", "{out}");
        assert!(
            response["operation_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("legacy-")),
            "{out}"
        );
    }

    /// Authorization precedes target disclosure. An ordinary caller receives
    /// only `unauthorized` even when its legacy-compatible request also has a
    /// missing or stale target. Receipt precedence moves with the modern
    /// receipt/target state matrix owned by T251.
    #[test]
    fn t250_unauthorized_precedes_missing_and_stale_target_diagnostics_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "ordinary-session");
        for (label, claim_id, delivery_id, window_id) in [
            ("missing-target", None, None, None),
            (
                "stale-target",
                Some("stale-claim"),
                Some("stale-delivery"),
                Some("tab-1::stale-agent"),
            ),
        ] {
            let repo = temp.path().join(label);
            std::fs::create_dir_all(&repo).expect("repo dir");
            t250_register_pm(&repo, "pm-session");
            let prefs_path = t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");
            let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
            let issue_before = t250_optional_file_snapshot(&prefs_path);
            let pm_before = t250_optional_file_snapshot(&pm_prefs_path);
            let mut env = crate::cli::TestEnv::new(repo.clone());
            let mut out = String::new();
            let result = run(
                &mut env,
                IssueCommand::MonitorStop {
                    project_root: Some(repo),
                    number: 42,
                    operation_id: None,
                    reason: "control requested".to_string(),
                    launch_generation: None,
                    claim_id: claim_id.map(str::to_string),
                    claim_owner: None,
                    delivery_id: delivery_id.map(str::to_string),
                    materializer_window_id: None,
                    window_id: window_id.map(str::to_string),
                    launch_profile_fingerprint: None,
                },
                &mut out,
            );
            t250_assert_unauthorized_refusal(label, result, &out);
            t250_assert_file_unchanged(label, &prefs_path, &issue_before);
            t250_assert_file_unchanged(label, &pm_prefs_path, &pm_before);
        }
    }

    /// Authorization also precedes retained-receipt replay/conflict lookup.
    /// The same operation ID cannot disclose whether its fingerprint matched
    /// to a caller that is not the durable project PM.
    #[test]
    fn t250_unauthorized_precedes_retained_receipt_replay_and_conflict() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "ordinary-session");

        for (label, window_id) in [
            ("receipt-replay", "tab-1::agent-42"),
            ("receipt-conflict", "tab-1::stale-agent"),
        ] {
            let repo = temp.path().join(label);
            std::fs::create_dir_all(&repo).expect("repo dir");
            t250_register_pm(&repo, "pm-session");
            let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
            let prefs_path = t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");
            let mut prefs: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&prefs_path).expect("read prefs for receipt seed"),
            )
            .expect("parse prefs for receipt seed");
            prefs
                .as_object_mut()
                .expect("prefs object")
                .insert(
                    "pm_control_receipts".to_string(),
                    serde_json::json!([{
                        "schema_version": 1,
                        "operation_id": "retained-operation",
                        "action": "stop",
                        "actor_session_id": "pm-session",
                        "registration_generation": 1,
                        "issue_number": 42,
                        "target_fingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "target": {
                            "launch_generation": 0,
                            "claim_id": null,
                            "claim_owner": null,
                            "delivery_id": null,
                            "materializer_window_id": null,
                            "window_id": "tab-1::agent-42",
                            "launch_profile_fingerprint": null
                        },
                        "reason": "control requested",
                        "requested_at": "2026-08-15T00:00:00Z",
                        "settled_at": "2026-08-15T00:00:01Z",
                        "outcome": "held"
                    }]),
                );
            let encoded = serde_json::to_vec_pretty(&prefs).expect("encode receipt fixture");
            let roundtripped: serde_json::Value =
                serde_json::from_slice(&encoded).expect("roundtrip receipt fixture");
            std::fs::write(
                &prefs_path,
                serde_json::to_vec_pretty(&roundtripped).expect("re-encode receipt fixture"),
            )
            .expect("seed retained receipt");
            let issue_before = t250_optional_file_snapshot(&prefs_path);
            let pm_before = t250_optional_file_snapshot(&pm_prefs_path);
            let mut env = crate::cli::TestEnv::new(repo.clone());
            let mut out = String::new();
            let result = run(
                &mut env,
                IssueCommand::MonitorStop {
                    project_root: Some(repo),
                    number: 42,
                    operation_id: Some("retained-operation".to_string()),
                    reason: "control requested".to_string(),
                    launch_generation: Some(0),
                    claim_id: None,
                    claim_owner: None,
                    delivery_id: None,
                    materializer_window_id: None,
                    window_id: Some(window_id.to_string()),
                    launch_profile_fingerprint: None,
                },
                &mut out,
            );
            t250_assert_unauthorized_refusal(label, result, &out);
            t250_assert_file_unchanged(label, &prefs_path, &issue_before);
            t250_assert_file_unchanged(label, &pm_prefs_path, &pm_before);
        }
    }

    /// FR-128 narrows control-window equality to byte-exact comparison.  The
    /// general event compatibility rule (qualified `tab::agent` versus bare
    /// `agent`) must not authorize a destructive control request.
    #[test]
    fn t250_monitor_control_rejects_bare_window_for_qualified_target_without_mutation() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("project");
        std::fs::create_dir_all(&repo).expect("repo dir");
        t250_register_pm(&repo, "pm-session");
        let prefs_path = t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");
        let before = std::fs::read(&prefs_path).expect("prefs before mismatch");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();

        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo),
                number: 42,
                operation_id: None,
                reason: "stale bare window".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("agent-42".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("control refusal");

        assert_eq!(code, 1, "{out}");
        let response: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern mismatch response");
        assert_eq!(response["error"], "target_mismatch", "{out}");
        assert_eq!(response["mismatch"], "launched_window", "{out}");
        assert_eq!(
            std::fs::read(&prefs_path).expect("prefs after mismatch"),
            before,
            "qualified/bare compatibility must never mutate a control target"
        );
    }

    /// The global nesting order is PM prefs -> Issue Monitor prefs.  Holding
    /// the Issue lock forces control to wait while retaining its PM lease; a
    /// concurrent stale-registration replacement therefore cannot overtake
    /// the old principal and invalidate the commit authorization.
    #[test]
    fn t250_pm_registration_lock_is_held_until_the_issue_prefs_commit() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let repo = temp.path().join("project");
        std::fs::create_dir_all(&repo).expect("repo dir");
        t250_register_pm(&repo, "old-pm-session");
        let pm_prefs_path = crate::pm_registry::pm_prefs_path_for_repo_path(&repo);
        let issue_prefs_path = t250_seed_legacy_launched_issue(&repo, "tab-1::agent-42");

        // Stop the control transaction at its second lock.
        let issue_lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(issue_prefs_path.with_extension("lock"))
            .expect("open Issue prefs lock");
        issue_lock.lock_exclusive().expect("hold Issue prefs lock");

        let (lease_acquired_tx, lease_acquired_rx) = std::sync::mpsc::channel();
        let (control_done_tx, control_done_rx) = std::sync::mpsc::channel();
        let control_pm_path = pm_prefs_path.clone();
        let control_issue_path = issue_prefs_path.clone();
        let control = std::thread::spawn(move || {
            let result = crate::pm_registry::with_registered_pm_authority(
                &control_pm_path,
                "old-pm-session",
                |_authority| {
                    lease_acquired_tx
                        .send(())
                        .expect("signal acquired PM authority lock");
                    crate::try_mutate_issue_monitor_prefs(&control_issue_path, |_prefs| Ok(()))?;
                    Ok(())
                },
            );
            control_done_tx
                .send(result.is_ok())
                .expect("report control transaction result");
        });

        lease_acquired_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("control closure entered with PM authority lock held");

        // The closure signal makes this probe deterministic: the authority
        // API promises the PM lock is already held before invoking it.
        let pm_probe = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(pm_prefs_path.with_extension("lock"))
            .expect("open PM lock probe");
        pm_probe
            .try_lock_exclusive()
            .expect_err("PM lock must remain held while Issue mutation is blocked");

        let (replacement_started_tx, replacement_started_rx) = std::sync::mpsc::channel();
        let (replacement_tx, replacement_rx) = std::sync::mpsc::channel();
        let replacement_path = pm_prefs_path.clone();
        let replacement = std::thread::spawn(move || {
            replacement_started_tx
                .send(())
                .expect("signal replacement start");
            let result = crate::pm_registry::try_register_pm(
                &replacement_path,
                t250_pm_registration("replacement-session"),
                |_| false,
            )
            .is_ok();
            replacement_tx
                .send(result)
                .expect("report replacement result");
        });
        replacement_started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("replacement thread started");
        assert!(
            matches!(
                replacement_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "replacement must remain blocked behind the retained PM lock"
        );

        FileExt::unlock(&issue_lock).expect("release Issue prefs lock");
        let control_succeeded = control_done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("control completes after Issue unlock");
        let replacement_succeeded = replacement_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("replacement completes after control releases PM lock");
        control.join().expect("control thread");
        replacement.join().expect("replacement thread");

        assert!(control_succeeded, "old principal did not commit");
        assert!(
            replacement_succeeded,
            "replacement should commit after the control transaction"
        );
    }

    /// Review-confirmed compatibility contract: omitting operation_id from
    /// stop/failover generates a non-replayable ID but still uses the modern
    /// receipt/fence transaction. Omitting generation from an otherwise
    /// modern target is a mismatch, never an implicit generation-0 wildcard.
    #[test]
    fn t259_legacy_pm_control_ids_use_modern_receipts_and_generation_remains_exact() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");

        for action in [
            crate::IssueMonitorPmControlAction::Stop,
            crate::IssueMonitorPmControlAction::Failover,
        ] {
            let label = format!("generated-{action:?}").to_ascii_lowercase();
            let repo = temp.path().join(label);
            std::fs::create_dir_all(&repo).expect("repo dir");
            t250_register_pm(&repo, "pm-session");
            let (prefs_path, target) = t259_seed_generated_launch(&repo);
            let mut env = crate::cli::TestEnv::new(repo.clone());
            let mut out = String::new();
            let code = run(
                &mut env,
                t259_control_command(repo, None, action, &target),
                &mut out,
            )
            .expect("legacy-compatible control handler");
            assert_eq!(code, 0, "{action:?}: {out}");
            let response: serde_json::Value =
                serde_json::from_str(out.trim()).expect("modern control response");
            let generated_id = response["operation_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .expect("omitted operation_id receives a generated receipt ID");
            assert_eq!(response["action"], serde_json::json!(action));

            let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("reload control");
            let receipt = prefs
                .pm_control
                .receipts
                .iter()
                .find(|receipt| receipt.operation_id == generated_id)
                .expect("generated ID names a durable receipt");
            assert_eq!(receipt.action, action);
            assert_eq!(receipt.target.generation, target.generation);
            let pending = prefs
                .pm_control
                .issues
                .get(&42)
                .and_then(|state| state.pending_control.as_ref())
                .expect("control remains teardown/claim-cleanup fenced");
            assert_eq!(pending.operation_id, generated_id);
            assert!(!pending.teardown_settled);
        }

        let legacy_repo = temp.path().join("generation-zero");
        std::fs::create_dir_all(&legacy_repo).expect("legacy repo");
        t250_register_pm(&legacy_repo, "pm-session");
        let legacy_path = t250_seed_legacy_launched_issue(&legacy_repo, "tab-legacy::agent-42");
        let legacy_monitor = crate::IssueMonitorState::with_prefs(
            crate::IssueMonitorConfig::default(),
            crate::load_issue_monitor_prefs(&legacy_path).expect("legacy prefs"),
        );
        let legacy_target = legacy_monitor
            .pm_control_status(42)
            .target
            .expect("generation-0 proven target");
        assert_eq!(legacy_target.generation, 0);
        let mut env = crate::cli::TestEnv::new(legacy_repo.clone());
        let mut out = String::new();
        let code = run(
            &mut env,
            t259_control_command(
                legacy_repo,
                None,
                crate::IssueMonitorPmControlAction::Stop,
                &legacy_target,
            ),
            &mut out,
        )
        .expect("generation-0 compatibility control");
        assert_eq!(code, 0, "{out}");
        let legacy = crate::load_issue_monitor_prefs(&legacy_path).expect("legacy control prefs");
        assert_eq!(legacy.pm_control.receipts.len(), 1);
        assert_eq!(legacy.pm_control.receipts[0].target.generation, 0);

        let missing_repo = temp.path().join("modern-missing-generation");
        std::fs::create_dir_all(&missing_repo).expect("missing generation repo");
        t250_register_pm(&missing_repo, "pm-session");
        let (missing_path, target) = t259_seed_generated_launch(&missing_repo);
        let before = std::fs::read(&missing_path).expect("before missing generation");
        let mut command = t259_control_command(
            missing_repo.clone(),
            Some("modern-missing-generation"),
            crate::IssueMonitorPmControlAction::Stop,
            &target,
        );
        let IssueCommand::MonitorStop {
            launch_generation, ..
        } = &mut command
        else {
            unreachable!()
        };
        *launch_generation = None;
        let mut env = crate::cli::TestEnv::new(missing_repo);
        out.clear();
        let code = run(&mut env, command, &mut out).expect("typed generation refusal");
        assert_ne!(code, 0, "missing modern generation succeeded: {out}");
        let response: serde_json::Value = serde_json::from_str(out.trim()).expect("refusal JSON");
        assert_eq!(response["error"], "target_mismatch");
        assert_eq!(response["mismatch"], "generation");
        assert_eq!(
            std::fs::read(&missing_path).expect("after missing generation"),
            before,
            "modern generation omission must be byte-identical"
        );
    }

    /// Every exact-target disagreement has the same stable top-level error
    /// and an additive component code. Definitive mismatch is zero-write.
    #[test]
    fn t259_pm_control_target_mismatch_json_names_each_wire_component() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");

        for (component, expected) in [
            ("generation", "generation"),
            ("claim_id", "claim_id"),
            ("claim_owner", "claim_owner"),
            ("delivery_id", "delivery_id"),
            ("materializer_window", "materializer_window"),
            ("launched_window", "launched_window"),
            ("source_profile_fingerprint", "source_profile_fingerprint"),
        ] {
            let repo = temp.path().join(component);
            std::fs::create_dir_all(&repo).expect("repo dir");
            t250_register_pm(&repo, "pm-session");
            let (prefs_path, target) = t259_seed_generated_launch(&repo);
            let before = std::fs::read(&prefs_path).expect("prefs before mismatch");
            let mut command = t259_control_command(
                repo.clone(),
                Some(&format!("mismatch-{component}")),
                crate::IssueMonitorPmControlAction::Stop,
                &target,
            );
            let IssueCommand::MonitorStop {
                launch_generation,
                claim_id,
                claim_owner,
                delivery_id,
                materializer_window_id,
                window_id,
                launch_profile_fingerprint,
                ..
            } = &mut command
            else {
                unreachable!()
            };
            match component {
                "generation" => *launch_generation = Some(target.generation + 1),
                "claim_id" => *claim_id = Some("stale-claim".to_string()),
                "claim_owner" => *claim_owner = Some("stale/owner".to_string()),
                "delivery_id" => *delivery_id = Some("stale-delivery".to_string()),
                "materializer_window" => {
                    *materializer_window_id = Some("tab-stale::agent-42".to_string())
                }
                "launched_window" => *window_id = Some("tab-stale::agent-42".to_string()),
                "source_profile_fingerprint" => {
                    *launch_profile_fingerprint = Some("stale-profile-fingerprint".to_string())
                }
                _ => unreachable!(),
            }
            let mut env = crate::cli::TestEnv::new(repo);
            let mut out = String::new();
            let code = run(&mut env, command, &mut out).expect("typed mismatch response");
            assert_ne!(code, 0, "{component} mismatch succeeded: {out}");
            let response: serde_json::Value =
                serde_json::from_str(out.trim()).expect("mismatch response JSON");
            assert_eq!(response["status"], "refused", "{component}: {out}");
            assert_eq!(response["error"], "target_mismatch", "{component}: {out}");
            assert_eq!(response["mismatch"], expected, "{component}: {out}");
            assert_eq!(
                std::fs::read(&prefs_path).expect("prefs after mismatch"),
                before,
                "{component} mismatch mutated prefs"
            );
        }
    }

    /// A post-rename parent-sync failure means the durable outcome is
    /// ambiguous, not success. Handler replay converges the visible candidate
    /// without a second receipt, revision, or effect.
    #[test]
    fn t259_pm_control_handler_reports_post_rename_unknown_then_replays_once() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = TempDir::new().expect("tempdir");
        let _home = ScopedGwtHome::set(temp.path().join("home"));
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let repo = temp.path().join("post-rename-handler");
        std::fs::create_dir_all(&repo).expect("repo dir");
        t250_register_pm(&repo, "pm-session");
        let (prefs_path, target) = t259_seed_generated_launch(&repo);
        let operation_id = "handler-post-rename-unknown";
        let marker = prefs_path.with_extension("parent-sync-fail-once");
        std::fs::write(&marker, b"fail once").expect("arm parent sync fault");
        let fault = ScopedEnvVar::set(
            "GWT_TEST_FAIL_ISSUE_MONITOR_PREFS_PARENT_SYNC_ONCE",
            &prefs_path,
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let mut out = String::new();
        let code = run(
            &mut env,
            t259_control_command(
                repo.clone(),
                Some(operation_id),
                crate::IssueMonitorPmControlAction::Stop,
                &target,
            ),
            &mut out,
        )
        .expect("classified handler response");
        assert_ne!(code, 0, "post-rename ambiguity reported success: {out}");
        let response: serde_json::Value = serde_json::from_str(out.trim()).expect("unknown JSON");
        assert_eq!(response["operation_id"], operation_id);
        assert_eq!(response["outcome"], "outcome_unknown");
        let visible = crate::load_issue_monitor_prefs(&prefs_path)
            .expect("post-rename candidate remains visible");
        assert_eq!(visible.pm_control.receipts.len(), 1);
        let visible_revision = visible.pm_control.revision;
        let visible_effects = visible.pending_effects.len();
        drop(fault);

        out.clear();
        let code = run(
            &mut env,
            t259_control_command(
                repo,
                Some(operation_id),
                crate::IssueMonitorPmControlAction::Stop,
                &target,
            ),
            &mut out,
        )
        .expect("exact handler replay");
        assert_eq!(code, 0, "exact replay did not converge: {out}");
        let replayed = crate::load_issue_monitor_prefs(&prefs_path).expect("replayed prefs");
        assert_eq!(replayed.pm_control.revision, visible_revision);
        assert_eq!(replayed.pm_control.receipts.len(), 1);
        assert_eq!(replayed.pending_effects.len(), visible_effects);
        assert_eq!(
            replayed, visible,
            "replay duplicated or changed durable state"
        );
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
                    ..crate::IssueMonitorLaunchedIssue::default()
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        t250_register_pm(&repo, "pm-session");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        // A stale window id must change nothing on disk.
        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo.clone()),
                number: 42,
                operation_id: None,
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-9".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 1, "a refused stop is not a success");
        let refusal: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern stop refusal");
        assert_eq!(refusal["status"], "refused", "{out}");
        assert_eq!(refusal["error"], "target_mismatch", "{out}");
        assert_eq!(refusal["mismatch"], "launched_window", "{out}");
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
                operation_id: None,
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("stop runs");
        assert_eq!(code, 0);
        let accepted: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern stop admission");
        assert_eq!(accepted["action"], "stop", "{out}");
        assert_eq!(accepted["outcome"], "cleanup_pending", "{out}");
        assert_eq!(accepted["stopped_window_id"], "tab-1::agent-1", "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.launched_issues.is_empty(),
            "the slot must be released on disk"
        );
        assert_eq!(
            prefs
                .pm_control
                .receipts
                .last()
                .map(|receipt| receipt.reason.as_str()),
            Some("provider rate limit"),
            "the receipt reason must survive the prefs roundtrip"
        );

        // Omitted IDs are deliberately non-replayable: a second omission is a
        // distinct operation and cannot pass the existing pending fence.
        out.clear();
        let code = run(
            &mut env,
            IssueCommand::MonitorStop {
                project_root: Some(repo),
                number: 42,
                operation_id: None,
                reason: "provider rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("stop runs");
        assert_ne!(code, 0);
        let replay: serde_json::Value =
            serde_json::from_str(out.trim()).expect("non-replayable refusal");
        assert_eq!(replay["error"], "identity_incomplete", "{out}");
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
                launch_profile: Some(t259_launch_profile("codex", "gpt-failover")),
                priority_order: vec![43, 42],
                launched_issues: vec![crate::IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                    ..crate::IssueMonitorLaunchedIssue::default()
                }],
                ..crate::IssueMonitorPrefs::default()
            },
        )
        .expect("save prefs");
        t250_register_pm(&repo, "pm-session");
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let before = std::fs::read(&prefs_path).expect("prefs bytes");
        let mut out = String::new();
        let code = run(
            &mut env,
            IssueCommand::MonitorFailover {
                project_root: Some(repo.clone()),
                number: 42,
                operation_id: None,
                reason: "codex rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-9".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("failover runs");
        assert_eq!(code, 1);
        let refusal: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern failover refusal");
        assert_eq!(refusal["status"], "refused", "{out}");
        assert_eq!(refusal["error"], "target_mismatch", "{out}");
        assert_eq!(refusal["mismatch"], "launched_window", "{out}");
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
                operation_id: None,
                reason: "codex rate limit".to_string(),
                launch_generation: None,
                claim_id: None,
                claim_owner: None,
                delivery_id: None,
                materializer_window_id: None,
                window_id: Some("tab-1::agent-1".to_string()),
                launch_profile_fingerprint: None,
            },
            &mut out,
        )
        .expect("failover runs");
        assert_eq!(code, 0);
        let accepted: serde_json::Value =
            serde_json::from_str(out.trim()).expect("modern failover admission");
        assert_eq!(accepted["action"], "failover", "{out}");
        assert_eq!(accepted["outcome"], "teardown_pending", "{out}");

        let prefs = crate::load_issue_monitor_prefs(&prefs_path).expect("load prefs");
        assert!(
            prefs.launched_issues.is_empty(),
            "the old launch must be revoked on disk"
        );
        assert_eq!(
            prefs.priority_order,
            vec![43],
            "restart must remain fenced until teardown and claim cleanup settle"
        );
        assert!(
            prefs.failed_issues.is_empty(),
            "a failover is not a failure and must not leave a hold behind"
        );
        let control = prefs
            .pm_control
            .issues
            .get(&42)
            .expect("durable failover control state");
        assert!(
            control.pending_control.is_some(),
            "failover admission must retain a teardown fence"
        );
        assert_eq!(
            control.pinned_next_profile,
            Some(t259_launch_profile("codex", "gpt-failover")),
            "the requested failover profile must be pinned until settlement"
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

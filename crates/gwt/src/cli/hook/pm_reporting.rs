//! PM-only reporting obligations. Ordinary Agent producing obligations stay separate.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::HookOutput;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QaTurn {
    session_id: String,
    turn_id: String,
}

#[derive(Serialize, Deserialize)]
struct TurnReceipt {
    identity: QaTurn,
    requires_qa: bool,
    qa_report_id: Option<String>,
}

fn project_store(repo: &Path) -> PathBuf {
    repo.ancestors()
        .find_map(|ancestor| {
            crate::pm_registry::pm_worktree_store_dir(ancestor).or_else(|| {
                crate::pm_registry::is_canonical_pm_worktree(ancestor)
                    .then(|| ancestor.parent()?.parent().map(Path::to_path_buf))
                    .flatten()
            })
        })
        .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(repo))
}

fn receipt_path(repo: &Path, session: &str) -> PathBuf {
    project_store(repo)
        .join("project-state/pm-reporting")
        .join(format!(
            "{}.json",
            crate::pm_registry::pm_delivery_prompt_sha256(session)
        ))
}

fn read_receipt(repo: &Path, session: &str) -> io::Result<Option<TurnReceipt>> {
    match fs::read(receipt_path(repo, session)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_receipt(repo: &Path, receipt: &TurnReceipt) -> io::Result<()> {
    let bytes = serde_json::to_vec(receipt).map_err(io::Error::other)?;
    gwt_github::cache::write_atomic(&receipt_path(repo, &receipt.identity.session_id), &bytes)
}

fn with_receipt_lock<T>(
    repo: &Path,
    session: &str,
    action: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let path = receipt_path(repo, session).with_extension("lock");
    fs::create_dir_all(path.parent().expect("receipt has a parent"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    // A busy sidecar must not turn a foreground prompt into an unbounded wait.
    lock.try_lock_exclusive()?;
    action()
}

fn registered_pm_session(worktree: &Path, session: Option<&str>) -> Option<String> {
    if !super::is_resident_pm_worktree(worktree) {
        return None;
    }
    let session = session?.trim();
    (!session.is_empty()
        && crate::pm_registry::session_is_registered_pm(
            &project_store(worktree).join("project-state/pm.json"),
            session,
        ))
    .then(|| session.to_owned())
}

/// Every observed PM prompt gets a new local turn identity; native/client turn
/// hints never authorize completion of a different turn.
pub(crate) fn handle_user_prompt_submit(worktree: &Path, input: &str) -> io::Result<()> {
    let ambient = crate::cli::pm::ambient_session_id();
    let Some(session_id) = registered_pm_session(worktree, ambient.as_deref()) else {
        return Ok(());
    };
    let prompt = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let requires_qa = prompt.as_deref().is_some_and(|text| {
        !text.trim().is_empty() && !crate::pm_conversation::internal_prompt(text)
    });
    with_receipt_lock(worktree, &session_id, || {
        write_receipt(
            worktree,
            &TurnReceipt {
                identity: QaTurn {
                    session_id: session_id.clone(),
                    turn_id: uuid::Uuid::new_v4().to_string(),
                },
                requires_qa,
                qa_report_id: None,
            },
        )
    })
}

/// Capture before posting. A later prompt cannot be settled by a post that
/// started during an older turn.
pub(crate) fn capture_qa_turn(repo: &Path) -> io::Result<Option<QaTurn>> {
    let Some(session_id) = crate::cli::pm::ambient_session_id() else {
        return Ok(None);
    };
    with_receipt_lock(repo, &session_id, || {
        let path = receipt_path(repo, &session_id);
        let receipt: TurnReceipt = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(receipt) => receipt,
                Err(_) => {
                    // Preserve the exact damaged bytes before replacing the
                    // receipt. A failed report must leave the fresh obligation.
                    let quarantine =
                        path.with_extension(format!("corrupt-{}", uuid::Uuid::new_v4()));
                    gwt_github::cache::write_atomic(&quarantine, &bytes)?;
                    let receipt = TurnReceipt {
                        identity: QaTurn {
                            session_id: session_id.clone(),
                            turn_id: uuid::Uuid::new_v4().to_string(),
                        },
                        requires_qa: true,
                        qa_report_id: None,
                    };
                    write_receipt(repo, &receipt)?;
                    receipt
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(receipt)
            .filter(|receipt| {
                receipt.identity.session_id == session_id
                    && receipt.requires_qa
                    && receipt.qa_report_id.is_none()
            })
            .map(|receipt| receipt.identity))
    })
}

/// Called only after the joined report/instruction persistence path succeeds.
pub(crate) fn acknowledge_qa(repo: &Path, turn: &QaTurn, report_id: &str) -> io::Result<()> {
    if crate::cli::pm::ambient_session_id().as_deref() != Some(turn.session_id.as_str())
        || report_id.trim().is_empty()
    {
        return Ok(());
    }
    with_receipt_lock(repo, &turn.session_id, || {
        let Some(mut receipt) = read_receipt(repo, &turn.session_id)? else {
            return Ok(());
        };
        if receipt.identity == *turn && receipt.requires_qa {
            receipt.qa_report_id = Some(report_id.to_owned());
            write_receipt(repo, &receipt)?;
        }
        Ok(())
    })
}

pub(crate) fn handle_stop(worktree: &Path, session: Option<&str>) -> HookOutput {
    let Some(session) = registered_pm_session(worktree, session) else {
        return HookOutput::Silent;
    };
    match read_receipt(worktree, &session) {
        Ok(Some(receipt)) if receipt.identity.session_id == session && receipt.requires_qa && receipt.qa_report_id.is_none() => {
            HookOutput::stop_block("This human PM turn has no successful Q&A report. Post the user's question and your answer with JSON operation `pm.report.post`, `kind:\"qa\"`, before stopping. A progress post, failed post, or report from another session/turn does not satisfy this obligation.")
        }
        Err(error) => HookOutput::stop_block(format!("PM reporting receipt could not be read: {error}. Run JSON operation `pm.report.post` with `kind:\"qa\"` to preserve the corrupt bytes and arm a fresh required turn before posting. A successful Q&A post settles it; a failed post keeps the obligation.")),
        _ => HookOutput::Silent,
    }
}

pub(crate) fn reporting_diagnostic(repo: &Path, session_id: &str) -> serde_json::Value {
    match read_receipt(repo, session_id) {
        Ok(Some(receipt)) if receipt.identity.session_id == session_id => serde_json::json!({
            "turn_id": receipt.identity.turn_id,
            "requires_qa": receipt.requires_qa,
            "qa_report_id": receipt.qa_report_id,
        }),
        Ok(None) => serde_json::json!({
            "turn_id": null, "requires_qa": false, "qa_report_id": null,
        }),
        Ok(Some(_)) => serde_json::json!({
            "unreadable": true, "error": "receipt session identity does not match",
        }),
        Err(error) => serde_json::json!({"unreadable": true, "error": error.to_string()}),
    }
}

/// Context only: an unacknowledged Monitor change never becomes a Stop gate.
/// Recompute on every prompt, so a missed status update remains visible.
pub(crate) fn monitor_reminder(worktree: &Path) -> Option<String> {
    let ambient = crate::cli::pm::ambient_session_id();
    registered_pm_session(worktree, ambient.as_deref())?;
    let fingerprint = crate::pm_current::monitor_fingerprint(worktree)?;
    match crate::pm_current::latest_status(worktree) {
        Ok(Some(status)) if status.monitor_fingerprint.as_deref() == Some(fingerprint.as_str()) => {
            None
        }
        Ok(_) => Some(
            "The current Monitor facts have not been acknowledged in PM current status. \
             Reconcile the changes and update JSON operation `pm.status.set` in this turn. \
             This reminder repeats on the next prompt until status acknowledges these facts."
                .into(),
        ),
        Err(_) => Some(
            "PM current status could not be read, so acknowledgement of the Monitor facts is \
             unknown. Inspect the status store and retry `pm.status.set`; do not claim a \
             confirmed Monitor change from this read failure."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

    fn instruction_capture() -> Box<gwt_core::user_instruction::UserInstructionInput> {
        Box::new(gwt_core::user_instruction::UserInstructionInput {
            original: "状況は？".into(),
            summary: String::new(),
            answer: String::new(),
            actions: vec!["状況を報告".into()],
            issue_numbers: vec![],
            kind: gwt_core::user_instruction::UserInstructionKind::Question,
            supersedes: vec![],
        })
    }

    fn fixture(home: &Path) -> std::path::PathBuf {
        let repo = home.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
        std::fs::create_dir_all(&worktree).unwrap();
        crate::pm_registry::save_pm_prefs(
            &worktree
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("project-state/pm.json"),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "pm-session".into(),
                    agent_id: "codex".into(),
                    worktree_path: worktree.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        worktree
    }

    #[test]
    fn human_prompt_requires_a_successful_qa_in_the_same_session_and_turn() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況を教えて"}"#).unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        let first = capture_qa_turn(&worktree).unwrap().expect("armed PM turn");
        let other = QaTurn {
            session_id: "other-session".into(),
            ..first.clone()
        };
        acknowledge_qa(&worktree, &other, "wrong-session").unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        handle_user_prompt_submit(&worktree, r#"{"prompt":"次の質問です"}"#).unwrap();
        acknowledge_qa(&worktree, &first, "old-turn").unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        let current = capture_qa_turn(&worktree).unwrap().unwrap();
        acknowledge_qa(&worktree, &current, "saved-report").unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        assert_eq!(
            handle_stop(&worktree, Some("other-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn canonical_automatic_prompts_pass_but_similar_human_prose_requires_qa() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let tick = format!(
            "[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close). {} {} {}",
            crate::pm_registry::PM_STEERING_WAKE_CLAUSE,
            crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE,
            crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE,
        );
        let notification = "<task-notification>\n<task-id>background-1</task-id>\n<tool-use-id>tool-1</tool-use-id>\n<output-file>/tmp/result</output-file>\n<status>completed</status>\n<summary>Background task completed</summary>\n</task-notification>";
        let extended_notification = notification.replace("</task-notification>", "<note>Read the result</note>\n<result>Done</result>\n<usage><tool_uses>1</tool_uses></usage>\n</task-notification>");
        for prompt in [
            "$gwt-pm",
            tick.as_str(),
            notification,
            extended_notification.as_str(),
        ] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent
            );
        }
        handle_user_prompt_submit(&worktree, "{}").unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        for prompt in [
            "[gwt] Scheduled supervision tick について説明して".to_owned(),
            "$gwt-pm の使い方を教えて".to_owned(),
            format!("Explain this notification: {notification}"),
            format!("{notification}\n何が完了しましたか？"),
            format!("{notification}\nこの二つの通知を比較してください\n{notification}"),
        ] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert!(
                matches!(
                    handle_stop(&worktree, Some("pm-session")),
                    HookOutput::StopBlock { .. }
                ),
                "human question must still require QA: {prompt}"
            );
        }
    }

    #[test]
    fn ordinary_agent_prompts_do_not_create_pm_reporting_obligations() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-session");
        handle_user_prompt_submit(home.path(), r#"{"prompt":"修正して"}"#).unwrap();
        assert!(capture_qa_turn(home.path()).unwrap().is_none());
        assert_eq!(
            handle_stop(home.path(), Some("agent-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn only_successful_qa_report_posts_release_the_reporting_gate() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let mut env = crate::cli::TestEnv::new(home.path().join("repo"));
        let post = |kind, body: &str| crate::cli::pm::PmCommand::ReportPost {
            project_root: None,
            kind,
            question: Some("状況は？".into()),
            body: body.into(),
            instruction: (kind == gwt_core::pm_report::PmReportKind::Qa).then(instruction_capture),
        };
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        for (question, original) in [("$gwt-pm", "状況は？"), ("定期指示の照合", "$gwt-pm")]
        {
            let mut instruction = instruction_capture();
            instruction.original = original.into();
            let result = crate::cli::pm::run(
                &mut env,
                crate::cli::pm::PmCommand::ReportPost {
                    project_root: None,
                    kind: gwt_core::pm_report::PmReportKind::Qa,
                    question: Some(question.into()),
                    body: "変化なし".into(),
                    instruction: Some(instruction),
                },
                &mut String::new(),
            );
            assert!(result.is_err(), "automatic QA must be refused");
            assert!(gwt_core::pm_report::load_reports(&home.path().join("repo"))
                .unwrap()
                .is_empty());
            assert!(capture_qa_turn(&worktree).unwrap().is_some());
        }
        assert!(crate::cli::pm::run(
            &mut env,
            post(gwt_core::pm_report::PmReportKind::Qa, ""),
            &mut String::new()
        )
        .is_err());
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        crate::cli::pm::run(
            &mut env,
            post(gwt_core::pm_report::PmReportKind::Progress, "進行中"),
            &mut String::new(),
        )
        .unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        crate::cli::pm::run(
            &mut env,
            post(gwt_core::pm_report::PmReportKind::Qa, "待機中です"),
            &mut String::new(),
        )
        .unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn monitor_changes_remind_until_current_status_acknowledges_them() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let repo = home.path().join("repo");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let mut prefs = crate::IssueMonitorPrefs::default();
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "unacknowledged Monitor facts need context"
        );
        let fingerprint = crate::pm_current::monitor_fingerprint(&worktree).unwrap();
        let input = gwt_core::pm_status::PmStatusInput {
            items: vec![gwt_core::pm_status::PmStatusItem {
                kind: gwt_core::pm_status::PmStatusItemKind::Waiting,
                text: "Waiting for the queue".into(),
                started_at: "2026-10-13T09:00:00Z".parse().unwrap(),
            }],
            next_trigger: "The queue changes".into(),
            user_action: None,
            stale_after_minutes: 15,
        };
        gwt_core::pm_status::set_status(&repo, input.clone(), Some(fingerprint)).unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        prefs.priority_order.push(5349);
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
        assert!(monitor_reminder(&worktree)
            .unwrap()
            .contains("pm.status.set"));
        assert!(
            monitor_reminder(&worktree).is_some(),
            "a new prompt must retry the reminder until acknowledged"
        );
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent,
            "Monitor reminders never block Stop"
        );

        gwt_core::pm_status::set_status(
            &repo,
            input.clone(),
            crate::pm_current::monitor_fingerprint(&worktree),
        )
        .unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};
        let mut board = gwt_core::board_escalation::BoardEscalationStore::default();
        let blocked = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Blocked,
            "Symptom: waiting.\nCause: missing ruling.\nRequest: decide.\nResume: ruling received.",
            None,
            None,
            vec![],
            vec!["5349".into()],
        );
        let blocked_id = blocked.id.clone();
        board.apply_entry(&blocked);
        // Use the Board reader's canonical path contract, including its
        // repo-local fallback in this fixture without a Git repository.
        let index = gwt_core::coordination::coordination_escalations_path(&worktree);
        fs::create_dir_all(index.parent().unwrap()).unwrap();
        gwt_github::cache::write_atomic(&index, &serde_json::to_vec(&board).unwrap()).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "a Board-only blocked owner must trigger the next prompt reminder"
        );
        gwt_core::pm_status::set_status(
            &repo,
            input,
            crate::pm_current::monitor_fingerprint(&worktree),
        )
        .unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        let mut resolution = BoardEntry::new(
            AuthorKind::User,
            "You",
            BoardEntryKind::Decision,
            "The ruling is settled",
            None,
            None,
            vec![],
            vec!["5349".into()],
        );
        resolution.resolves_entry_ids = vec![blocked_id];
        board.apply_entry(&resolution);
        gwt_github::cache::write_atomic(&index, &serde_json::to_vec(&board).unwrap()).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "resolving a Board-only blocked owner must trigger the next prompt reminder"
        );
    }
    #[test]
    fn delivered_wakes_with_escalations_and_agent_idle_do_not_require_qa() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let suffix = format!(
            " {} {} {}",
            crate::pm_registry::PM_STEERING_WAKE_CLAUSE,
            crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE,
            crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE
        );
        let tick = format!("[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close).{suffix}");
        let delta = format!("[gwt] Monitor activity while the PM was idle (new needs_human). Reconcile now: fresh `issue.monitor.status`, triage new items, inventory PRs with `pr.list` (stale/SUPERSEDED/owner-closed rows: digest, never auto-close).{suffix}");
        let idle = "[gwt] Agent idle: 修正担当 (#3932) | [gwt] Agent idle: 次の担当 (#5349). Reconcile pane.list and issue.monitor.status; steer the idle agents.";
        let escalations = " UNRESOLVED BLOCKED ESCALATIONS (1) — decide fresh launch / spec ruling / tool fix for each, then post the resolution with params.resolves: #2338 params.resolves:[\"blocked-entry\"] 依頼: fresh launch を手配してほしい";
        let long_idle = format!("[gwt] Agent idle: {}. Reconcile pane.list and issue.monitor.status; steer the idle agents.", "担当".repeat(120));
        let mut truncated_idle = format!("{long_idle}{escalations}");
        // Match the physical idle-delivery cap, including a partially delivered escalation tail.
        let mut end = 959;
        while !truncated_idle.is_char_boundary(end) {
            end -= 1;
        }
        truncated_idle.truncate(end);
        for prompt in [
            format!("{tick}{escalations}\r"),
            format!("{delta}{escalations}\r"),
            format!("{tick}{escalations} quoted: UNRESOLVED BLOCKED ESCALATIONS (1)\r"),
            format!("{idle}\r"),
            format!("{idle}{escalations}\r"),
            format!(
                "{}{escalations}\r",
                idle.replace("修正担当", "UNRESOLVED BLOCKED ESCALATIONS (1) 修正担当")
            ),
            format!("{truncated_idle}\r"),
        ] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent,
                "physical automatic delivery must not arm QA: {prompt}"
            );
        }
        for prompt in ["[gwt] Agent idle: について説明して", "UNRESOLVED BLOCKED ESCALATIONS (1) は何ですか",
            "[gwt] Agent idle: 修正担当 (#3932). Reconcile pane.list and issue.monitor.status; steer the idle agents. これは何？"] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string()).unwrap();
            assert!(matches!(handle_stop(&worktree, Some("pm-session")), HookOutput::StopBlock { .. }), "similar human prose still needs QA: {prompt}");
        }
    }

    #[test]
    fn corrupt_receipt_is_quarantined_and_recovered_through_a_qa_post() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let repo = home.path().join("repo");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let post = |body: &str| crate::cli::pm::PmCommand::ReportPost {
            project_root: None,
            kind: gwt_core::pm_report::PmReportKind::Qa,
            question: Some("状況は？".into()),
            body: body.into(),
            instruction: Some(instruction_capture()),
        };
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        let old = capture_qa_turn(&repo).unwrap().unwrap();
        let path = receipt_path(&worktree, "pm-session");
        let corrupt = b"{broken receipt";
        fs::write(&path, corrupt).unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        assert_eq!(
            reporting_diagnostic(&repo, "pm-session")["unreadable"],
            true
        );
        let failure = crate::cli::pm::run(&mut env, post(""), &mut String::new()).unwrap_err();
        assert!(
            failure.to_string().contains("nonempty"),
            "QA capture must recover before report validation: {failure}"
        );
        let recovered = capture_qa_turn(&repo).unwrap().unwrap();
        assert_ne!(recovered, old);
        let diagnostic = reporting_diagnostic(&repo, "pm-session");
        assert_eq!(diagnostic["turn_id"], recovered.turn_id);
        assert_eq!(diagnostic["requires_qa"], true);
        assert!(diagnostic["qa_report_id"].is_null());
        let quarantined: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(fs::read(quarantined[0].path()).unwrap(), corrupt);
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        handle_user_prompt_submit(&worktree, r#"{"prompt":"新しい質問"}"#).unwrap();
        acknowledge_qa(&repo, &recovered, "older-success").unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        crate::cli::pm::run(&mut env, post("回復しました"), &mut String::new()).unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        assert!(reporting_diagnostic(&repo, "pm-session")["qa_report_id"].is_string());
    }
}

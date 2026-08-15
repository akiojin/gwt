//! Stop gate for terminal Work-event delivery settlement.

use std::path::Path;

use super::{envelope::stop_hook_active_from, HookOutput};

pub fn handle_with_input(
    worktree: &Path,
    input: &str,
    current_session: Option<&str>,
) -> HookOutput {
    if stop_hook_active_from(input) {
        return HookOutput::Silent;
    }
    let Some(current_session) = current_session
        .map(str::trim)
        .filter(|session| !session.is_empty())
    else {
        return HookOutput::Silent;
    };
    let resolved = gwt_core::paths::resolve_current_worktree_root(worktree);
    let record = match crate::cli::verification_record::load_work_event_settlement_record(&resolved)
    {
        Ok(Some(record)) => record,
        Ok(None) => return HookOutput::Silent,
        Err(error) => {
            tracing::warn!(%error, "work event settlement receipt is unreadable");
            return HookOutput::system_message(format!(
                "Warning: Work event settlement receipt is unreadable ({error}). Stop is not blocked by this infrastructure failure."
            ));
        }
    };
    if !record.obligation_open || record.session_id != current_session {
        return HookOutput::Silent;
    }
    let refreshed = match crate::cli::verification_record::save_work_event_settlement_record(
        &resolved,
        &record.session_id,
        false,
    ) {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(%error, "work event settlement could not be refreshed");
            return HookOutput::system_message(format!(
                "Warning: Work event settlement could not be refreshed ({error}). Stop is not blocked by this infrastructure failure."
            ));
        }
    };
    if !refreshed.obligation_open && refreshed.status.is_settled() {
        return HookOutput::Silent;
    }
    if refreshed.status.severity()
        == crate::cli::verification_record::WorkEventSettlementSeverity::Warning
    {
        let reason = match &refreshed.status {
            crate::cli::verification_record::WorkEventSettlementStatus::Blocked(blocker) => {
                crate::cli::verification_record::work_event_settlement_blocker_description(blocker)
            }
            _ => "Work event settlement is waiting on the environment.".to_string(),
        };
        tracing::warn!(%reason, "work event settlement degraded to warning");
        return HookOutput::system_message(format!(
            "Warning: {reason} Stop is not blocked because the current agent cannot repair this environment failure."
        ));
    }
    let reason = match &refreshed.status {
        crate::cli::verification_record::WorkEventSettlementStatus::PendingMutation {
            event_id,
            work_id,
            journal_entry_id,
            ..
        } => crate::cli::verification_record::work_event_settlement_pending_description(
            event_id,
            work_id,
            journal_entry_id,
        ),
        crate::cli::verification_record::WorkEventSettlementStatus::Blocked(blocker) => {
            crate::cli::verification_record::work_event_settlement_blocker_description(blocker)
        }
        crate::cli::verification_record::WorkEventSettlementStatus::Settled { .. } => {
            "Work event settlement refused: the trusted obligation is still open. Refresh the settlement state and retry Stop.".to_string()
        }
    };
    HookOutput::system_message(format!(
        "Warning: a terminal Work update is still awaiting delivery. {reason} gwt will not commit or push automatically, and Stop is not blocked by this bookkeeping state."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::verification_record::{
        load_work_event_settlement_record, prepare_work_event_settlement_record,
        save_work_event_settlement_record,
    };
    use gwt_core::{
        test_support::ScopedEnvVar,
        workspace_projection::{
            WorkEvent, WorkEventKind, WorkspaceJournalEntry, WorkspaceStatusCategory,
        },
    };

    #[test]
    fn pending_mutation_warns_before_the_tracked_event_exists() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        let updated_at = chrono::Utc::now();
        let mut event = WorkEvent::new(WorkEventKind::Done, "work-pending-stop", updated_at);
        event.agent_session_id = Some("session-pending-stop".to_string());
        let journal_entry = WorkspaceJournalEntry {
            id: "journal-pending-stop".to_string(),
            project_root: fixture.repo.clone(),
            title: None,
            status_category: Some(WorkspaceStatusCategory::Done),
            status_text: None,
            owner: None,
            next_action: None,
            summary: None,
            progress_summary: None,
            agent_session_id: Some("session-pending-stop".to_string()),
            agent_current_focus: None,
            agent_title_summary: None,
            updated_at,
        };
        prepare_work_event_settlement_record(
            &fixture.repo,
            "session-pending-stop",
            &event,
            &journal_entry,
        )
        .expect("prepare settlement receipt");

        let warning = handle_with_input(
            &fixture.repo,
            r#"{"stop_hook_active":false}"#,
            Some("session-pending-stop"),
        );
        let HookOutput::SystemMessage(reason) = warning else {
            panic!("a pending terminal mutation must warn without blocking Stop: {warning:?}");
        };
        assert!(reason.contains("has not been persisted"), "{reason}");
        assert!(reason.contains(&event.id), "{reason}");
    }

    #[test]
    fn open_obligation_warns_until_commit_and_push_readback() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        fixture.append_event("terminal-update-awaiting-delivery");
        let opened = save_work_event_settlement_record(&fixture.repo, "session-a", true)
            .expect("open settlement obligation");
        assert!(opened.obligation_open);

        let warning = handle_with_input(
            &fixture.repo,
            r#"{"stop_hook_active":false}"#,
            Some("session-a"),
        );
        let HookOutput::SystemMessage(reason) = warning else {
            panic!("open Work settlement obligation must warn without blocking Stop: {warning:?}");
        };
        assert!(reason.contains(".gwt/work/events.jsonl"), "{reason}");
        assert!(reason.contains("commit"), "{reason}");
        assert!(reason.contains("push"), "{reason}");

        assert_eq!(
            handle_with_input(
                &fixture.repo,
                r#"{"stop_hook_active":false}"#,
                Some("session-b"),
            ),
            HookOutput::Silent,
            "a foreign session must not inherit the author's Stop obligation"
        );
        assert_eq!(
            handle_with_input(
                &fixture.repo,
                r#"{"stop_hook_active":true}"#,
                Some("session-a"),
            ),
            HookOutput::Silent,
            "stop_hook_active must cap forced continuation at one cycle"
        );

        fixture.stage_events();
        fixture.commit("chore(work): settle terminal update");
        fixture.push();
        assert_eq!(
            handle_with_input(
                &fixture.repo,
                r#"{"stop_hook_active":false}"#,
                Some("session-a"),
            ),
            HookOutput::Silent,
            "remote containment must settle and release the obligation"
        );
        let settled = load_work_event_settlement_record(&fixture.repo)
            .expect("load settled receipt")
            .expect("settled receipt exists");
        assert!(!settled.obligation_open);
        assert_eq!(settled.session_id, "session-a");
    }

    #[test]
    fn missing_upstream_warns_without_blocking_stop() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("isolated gwt home");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let fixture = crate::cli::verification_record::tests::WorkEventGitFixture::tracked();
        fixture.append_event("terminal-update-without-upstream");
        save_work_event_settlement_record(&fixture.repo, "session-warning", true)
            .expect("open settlement obligation");
        assert!(gwt_core::process::hidden_command("git")
            .args(["branch", "--unset-upstream"])
            .current_dir(&fixture.repo)
            .status()
            .expect("unset fixture upstream")
            .success());

        let output = handle_with_input(
            &fixture.repo,
            r#"{"stop_hook_active":false}"#,
            Some("session-warning"),
        );

        let HookOutput::SystemMessage(message) = output else {
            panic!("missing upstream must warn without blocking Stop: {output:?}");
        };
        assert!(message.contains("Warning:"), "{message}");
        assert!(message.contains("upstream"), "{message}");
        assert!(message.contains("not blocked"), "{message}");
    }
}

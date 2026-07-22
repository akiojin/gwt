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
            return HookOutput::stop_block(format!(
                "Work event settlement receipt is unreadable ({error}). Repair the trusted store and confirm `.gwt/work/events.jsonl` is committed and pushed before stopping."
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
            return HookOutput::stop_block(format!(
                "Work event settlement could not be refreshed ({error}). Commit `.gwt/work/events.jsonl`, push HEAD to its configured upstream, and retry Stop."
            ));
        }
    };
    if !refreshed.obligation_open && refreshed.status.is_settled() {
        return HookOutput::Silent;
    }
    let reason = match &refreshed.status {
        crate::cli::verification_record::WorkEventSettlementStatus::Blocked(blocker) => {
            crate::cli::verification_record::work_event_settlement_blocker_description(blocker)
        }
        crate::cli::verification_record::WorkEventSettlementStatus::Settled { .. } => {
            "Work event settlement refused: the trusted obligation is still open. Refresh the settlement state and retry Stop.".to_string()
        }
    };
    HookOutput::stop_block(format!(
        "A terminal Work update is still awaiting delivery. {reason} gwt will not commit or push automatically."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::verification_record::{
        load_work_event_settlement_record, save_work_event_settlement_record,
    };
    use gwt_core::test_support::ScopedEnvVar;

    #[test]
    fn open_obligation_blocks_until_commit_and_push_readback() {
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

        let blocked = handle_with_input(
            &fixture.repo,
            r#"{"stop_hook_active":false}"#,
            Some("session-a"),
        );
        let HookOutput::StopBlock { reason } = blocked else {
            panic!("open Work settlement obligation must block Stop: {blocked:?}");
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
}

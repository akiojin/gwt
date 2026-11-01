//! GUI adapters for continuation requests. The project owner owns all
//! generation, Session, readiness, recovery, and physical cleanup writes.

use super::AppRuntime;
pub(super) use gwt::project_runtime::continuation::{
    classify_nonlocal_active_owner_liveness_batch_at, ActiveOwnerLiveness,
};

#[cfg(test)]
pub(super) use gwt::project_runtime::continuation::{
    abort_prepared_execution, canonical_continue_work_owner, configure_provider_continuation,
    configure_provider_continuation_with_grok_home, continuation_launch_config, handoff_context,
    nonlocal_runtime_index_scan_metrics, provider_conversation_availability,
    reset_nonlocal_runtime_index_scan_metrics, set_durable_continue_work_post_repair_hook_for_test,
    set_durable_continue_work_pre_work_commit_hook_for_test,
    set_fresh_execution_pre_work_commit_hook_for_test, set_missing_session_cleanup_hook_for_test,
    strict_projection_owner, ContinueWorkLaunchSeed, ProviderConversationAvailability,
};
#[cfg(test)]
pub(super) use gwt::project_runtime::launch_finalization::{
    compensate_terminalized_genesis_workspace_projection,
    resolve_split_workspace_state_external_commit,
};
pub(super) use gwt::session_launch::durable_launch_recovery_exists;
#[cfg(all(test, unix))]
pub(super) use gwt::session_launch::recovery::set_durable_launch_recovery_directory_sync_test_hook;
#[cfg(test)]
pub(super) use gwt::session_launch::recovery::{
    clear_durable_launch_recovery, durable_launch_recovery_session_identity,
    persist_durable_launch_recovery, DurableLaunchRecoveryKind,
};
#[cfg(test)]
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

impl AppRuntime {
    #[cfg(test)]
    pub(super) fn classify_nonlocal_active_owner_liveness(
        &self,
        session_id: &str,
    ) -> ActiveOwnerLiveness {
        gwt::project_runtime::continuation::classify_nonlocal_active_owner_liveness_at(
            &self.sessions_dir,
            session_id,
        )
    }

    #[cfg(test)]
    pub(super) fn classify_nonlocal_active_owner_liveness_batch<'a>(
        &self,
        session_ids: impl IntoIterator<Item = &'a str>,
    ) -> HashMap<String, ActiveOwnerLiveness> {
        classify_nonlocal_active_owner_liveness_batch_at(&self.sessions_dir, session_ids)
    }

    #[cfg(test)]
    pub(super) fn active_profile_grok_home_for_continuation(
        &self,
        source_session: &gwt_agent::Session,
        launch_cwd: &Path,
    ) -> Option<PathBuf> {
        let profile = self.profile_config_path().ok()?;
        gwt::project_runtime::continuation::active_profile_grok_home_for_continuation(
            &profile,
            source_session,
            launch_cwd,
        )
    }

    pub(crate) fn continue_work_events(
        &mut self,
        context: &super::ProjectContext,
        client_id: &str,
        operation_id: String,
        work_id: String,
    ) -> Vec<super::OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        self.queue_project_owner_continue(context, client_id, operation_id, work_id)
    }
}

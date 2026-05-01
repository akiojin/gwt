//! Workspace migration handler split out of `app_runtime/mod.rs` for
//! SPEC-2077 Phase D (arch-review handoff, 2026-05-01). Keeps the SPEC-1934
//! Normal Git → Bare+Worktree migration flow contained in a single module
//! while preserving the GUI-facing `BackendEvent::Migration*` envelopes and
//! `UserEvent::Migration*` lifecycle events.

use std::path::Path;

use gwt::ProjectKind;
use gwt_core::migration::{MigrationOptions, MigrationPhase, RecoveryState};

use crate::UserEvent;

use super::{
    load_restored_workspace_state, recovery_state_label, AppRuntime, BackendEvent, OutboundEvent,
    WorkspaceState,
};

impl AppRuntime {
    /// SPEC-1934 US-6: user accepted the Migration confirmation modal.
    /// Spawns `gwt::migration::execute_migration` on a background thread and
    /// streams progress / completion / error back through `UserEvent::Migration*`.
    pub(crate) fn start_migration_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return Vec::new();
        };
        let project_root = tab.project_root.clone();
        let proxy = self.proxy.clone();
        let tab_id_owned = tab_id.to_string();

        std::thread::spawn(move || {
            let progress_tab = tab_id_owned.clone();
            let progress_proxy = proxy.clone();
            let outcome = gwt::migration::execute_migration(
                &project_root,
                MigrationOptions::default(),
                move |phase, percent| {
                    progress_proxy.send(UserEvent::MigrationProgress {
                        tab_id: progress_tab.clone(),
                        phase,
                        percent,
                    });
                },
            );
            match outcome {
                Ok(result) => proxy.send(UserEvent::MigrationDone {
                    tab_id: tab_id_owned,
                    branch_worktree_path: result.branch_worktree_path,
                }),
                Err(error) => proxy.send(UserEvent::MigrationError {
                    tab_id: tab_id_owned,
                    phase: error.phase,
                    message: error.message,
                    recovery: error.recovery,
                }),
            }
        });

        Vec::new()
    }

    /// SPEC-1934 US-6.7: user dismissed the modal. Drop the in-memory flag so
    /// the rest of the GUI proceeds without further detection events.
    pub(crate) fn skip_migration_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.migration_pending = false;
        }
        Vec::new()
    }

    /// SPEC-1934 US-6.9: migration finished successfully. Re-point the project
    /// tab at the new branch worktree, reload its persisted workspace, and
    /// surface a [`BackendEvent::MigrationDone`] alongside a refreshed
    /// workspace_state broadcast.
    pub(crate) fn handle_migration_done(
        &mut self,
        tab_id: &str,
        branch_worktree_path: &Path,
    ) -> Vec<OutboundEvent> {
        let canonical = dunce::canonicalize(branch_worktree_path)
            .unwrap_or_else(|_| branch_worktree_path.to_path_buf());

        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.project_root = canonical.clone();
            tab.kind = ProjectKind::Git;
            tab.migration_pending = false;
            match load_restored_workspace_state(&canonical) {
                Ok(persisted) => tab.workspace = WorkspaceState::from_persisted(persisted),
                Err(error) => {
                    tracing::warn!(
                        target: "gwt::migration",
                        ?canonical,
                        %error,
                        "post-migration workspace reload failed; keeping current workspace state"
                    );
                }
            }
        }
        let _ = self.persist();

        vec![
            self.workspace_state_broadcast(),
            OutboundEvent::broadcast(BackendEvent::MigrationDone {
                tab_id: tab_id.to_string(),
                branch_worktree_path: canonical.display().to_string(),
            }),
        ]
    }

    /// SPEC-1934 US-6.6: migration failed. Drop the pending flag (the
    /// frontend offers Retry / Restore / Quit) and broadcast the failure.
    pub(crate) fn handle_migration_error(
        &mut self,
        tab_id: &str,
        phase: MigrationPhase,
        message: String,
        recovery: RecoveryState,
    ) -> Vec<OutboundEvent> {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.migration_pending = false;
        }
        vec![OutboundEvent::broadcast(BackendEvent::MigrationError {
            tab_id: tab_id.to_string(),
            phase: phase.as_str().to_string(),
            message,
            recovery: recovery_state_label(recovery).to_string(),
        })]
    }

    /// SPEC-1934 US-6.8: user chose Quit. Phase 10.4 will translate this into
    /// a `UserEvent::QuitApp` once the runtime helper lands (T-097); the
    /// frontend already closes the modal optimistically.
    pub(crate) fn quit_migration_events(&mut self, _tab_id: &str) -> Vec<OutboundEvent> {
        // TODO(T-097): proxy.send(UserEvent::QuitApp) once the dispatch
        // helper lands.
        Vec::new()
    }
}

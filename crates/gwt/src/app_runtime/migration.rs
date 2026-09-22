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
    ProjectContext, WindowCanvasState,
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
        let Some(context) = self.project_context(tab_id) else {
            return Vec::new();
        };
        let proxy = self.proxy.clone();

        std::thread::spawn(move || {
            let progress_context = context.clone();
            let progress_proxy = proxy.clone();
            let outcome = gwt::migration::execute_migration(
                &project_root,
                MigrationOptions::default(),
                move |phase, percent| {
                    progress_proxy.send(UserEvent::MigrationProgress {
                        context: progress_context.clone(),
                        phase,
                        percent,
                    });
                },
            );
            match outcome {
                Ok(result) => proxy.send(UserEvent::MigrationDone {
                    context,
                    branch_worktree_path: result.branch_worktree_path,
                }),
                Err(error) => proxy.send(UserEvent::MigrationError {
                    context,
                    phase: error.phase,
                    message: error.message,
                    recovery: error.recovery,
                }),
            }
        });

        Vec::new()
    }

    pub(crate) fn handle_migration_progress(
        &self,
        context: &ProjectContext,
        phase: MigrationPhase,
        percent: u8,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        vec![OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::MigrationProgress {
                tab_id: context.tab_id.clone(),
                phase: phase.as_str().to_string(),
                percent,
            },
        )]
    }

    /// SPEC-1934 US-6.7: user dismissed the modal. Drop the in-memory flag so
    /// the rest of the GUI proceeds without further detection events.
    pub(crate) fn skip_migration_events(&mut self, tab_id: &str) -> Vec<OutboundEvent> {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.migration_pending = false;
        }
        self.refresh_project_tab_incarnation(tab_id);
        Vec::new()
    }

    /// SPEC-1934 US-6.9: migration finished successfully. Re-point the project
    /// tab at the new branch worktree, reload its persisted workspace, and
    /// surface a [`BackendEvent::MigrationDone`] alongside a refreshed
    /// workspace_state broadcast.
    pub(crate) fn handle_migration_done(
        &mut self,
        context: &ProjectContext,
        branch_worktree_path: &Path,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.as_str();
        let canonical = dunce::canonicalize(branch_worktree_path)
            .unwrap_or_else(|_| branch_worktree_path.to_path_buf());
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.project_root = canonical.clone();
            tab.kind = ProjectKind::Git;
            tab.migration_pending = false;
            match load_restored_workspace_state(&canonical) {
                Ok(persisted) => tab.workspace = WindowCanvasState::from_persisted(persisted),
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
        self.refresh_project_tab_incarnation(tab_id);
        let _ = self.persist();

        // Deliver completion before the snapshot switches the client's project key.
        vec![
            OutboundEvent::project(
                context.project_key.clone(),
                BackendEvent::MigrationDone {
                    tab_id: tab_id.to_string(),
                    branch_worktree_path: canonical.display().to_string(),
                },
            ),
            OutboundEvent::project(
                context.project_key.clone(),
                BackendEvent::WindowCanvasState {
                    workspace: self
                        .project_state_view(
                            &self
                                .project_context(tab_id)
                                .expect("migrated project context"),
                        )
                        .expect("migrated project state"),
                },
            ),
            self.hub_state_broadcast(),
        ]
    }

    /// SPEC-1934 US-6.6: migration failed. Drop the pending flag (the
    /// frontend offers Retry / Restore / Quit) and broadcast the failure.
    pub(crate) fn handle_migration_error(
        &mut self,
        context: &ProjectContext,
        phase: MigrationPhase,
        message: String,
        recovery: RecoveryState,
    ) -> Vec<OutboundEvent> {
        if !self.project_context_is_current(context) {
            return Vec::new();
        }
        let tab_id = context.tab_id.as_str();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.migration_pending = false;
        }
        self.refresh_project_tab_incarnation(tab_id);
        vec![OutboundEvent::project(
            context.project_key.clone(),
            BackendEvent::MigrationError {
                tab_id: tab_id.to_string(),
                phase: phase.as_str().to_string(),
                message,
                recovery: recovery_state_label(recovery).to_string(),
            },
        )]
    }

    /// SPEC-1934 US-6.8: user chose Quit. Leave the repository untouched and
    /// ask the GUI event loop to exit through the normal shutdown path.
    pub(crate) fn quit_migration_events(&mut self, _tab_id: &str) -> Vec<OutboundEvent> {
        self.proxy.send(UserEvent::QuitApp {
            reason: crate::GuiShutdownReason::QuitApp,
        });
        Vec::new()
    }
}

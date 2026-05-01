//! Launch wizard handler split out of `app_runtime/mod.rs` for SPEC-2077
//! Phase F1 (arch-review handoff, 2026-05-01).
//!
//! Phase F1 scope is intentionally narrow: only the wizard state broadcast
//! / clear helpers move here so that the larger `handle_launch_wizard_action`
//! (~600 lines) and `spawn_wizard_shell_window*` (~525 lines) helpers can be
//! split in follow-up phases (F2 / F3) without merge conflicts.
//!
//! Owns:
//! - [`AppRuntime::launch_wizard_state_outbound`] — broadcast the current
//!   wizard view (or `None`) to all clients
//! - [`AppRuntime::launch_wizard_state_broadcast`] — broadcast a caller-
//!   provided view (used after the action handler mutates state)
//! - [`AppRuntime::clear_launch_wizard`] — drop the in-memory session state
//!   and return the previous session for any cleanup the caller needs
//!
//! [`LaunchWizardSession`] still lives in `mod.rs` because the larger wizard
//! handlers (Phase F2 / F3 scope) construct and mutate it; once those
//! phases land the struct can move here too.

use gwt::LaunchWizardView;

use super::{AppRuntime, BackendEvent, LaunchWizardSession, OutboundEvent};

impl AppRuntime {
    pub(crate) fn launch_wizard_state_outbound(&self) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: self
                .launch_wizard
                .as_ref()
                .map(|wizard| Box::new(wizard.wizard.view())),
        })
    }

    pub(crate) fn launch_wizard_state_broadcast(
        &self,
        wizard: Option<LaunchWizardView>,
    ) -> OutboundEvent {
        OutboundEvent::broadcast(BackendEvent::LaunchWizardState {
            wizard: wizard.map(Box::new),
        })
    }

    pub(crate) fn clear_launch_wizard(&mut self) -> Option<LaunchWizardSession> {
        self.launch_wizard.take()
    }
}

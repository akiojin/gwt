//! SPEC #2920 — Tray-only browser front door.
//!
//! Replaces the wry/tao native WebView GUI route with a tray-resident
//! process that owns the embedded server and exposes an `Open` menu entry
//! to launch the default browser. This module is the runtime entry point
//! when `gwt` is invoked with no CLI verb (FrontDoorRoute::Tray).
//!
//! Phase 1 ships only the type and module skeletons; the actual event loop
//! and EmbeddedServer integration land in Phase 4. Until then, `run()`
//! returns `Err(TrayError::NotYetImplemented)` so we never silently take
//! over the GUI route.

use std::net::{IpAddr, Ipv4Addr};

pub mod autostart;
pub mod lock;
pub mod menu;

/// CLI flags accepted by the tray-resident front door.
///
/// SPEC #2920 FR-013: `--no-tray` skips tray-icon creation (for CI /
/// Playwright). `--no-open` is preserved as a no-op for backward
/// compatibility — the tray menu `Open` action is what actually opens the
/// browser now, so the auto-open default is `false` regardless of this
/// flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayArgs {
    pub bind: IpAddr,
    pub port: u16,
    pub no_tray: bool,
    pub no_open: bool,
}

impl Default for TrayArgs {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            no_tray: false,
            no_open: false,
        }
    }
}

/// Errors surfaced by the tray-resident entry point.
#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    #[error("tray-resident process is not yet implemented (SPEC #2920 Phase 4)")]
    NotYetImplemented,
}

/// Entry point invoked by `main.rs` after `FrontDoorRoute::Tray` is
/// resolved. Phase 4 will replace the placeholder with the real event
/// loop + EmbeddedServer bootstrap.
pub fn run(_args: TrayArgs) -> Result<i32, TrayError> {
    Err(TrayError::NotYetImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_args_default_binds_loopback_and_random_port() {
        let args = TrayArgs::default();
        assert_eq!(args.bind, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(args.port, 0);
        assert!(!args.no_tray);
        assert!(!args.no_open);
    }

    #[test]
    fn tray_run_is_not_yet_implemented_in_phase_1() {
        // SPEC #2920 Phase 1 only ships skeletons; real bootstrap lands
        // in Phase 4 alongside the WebView removal.
        let err = run(TrayArgs::default()).expect_err("placeholder must error");
        assert!(matches!(err, TrayError::NotYetImplemented));
    }
}

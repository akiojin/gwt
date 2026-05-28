//! SPEC #2920 FR-004 — Tray menu skeleton.
//!
//! The minimal tray menu is `[Open] / [Quit] / About` (Q5 chosen during
//! `gwt-discussion`). Settings, autostart toggle, Copy URL, Logs, and
//! Update controls live in the browser UI Settings page (Phase 8 / FR-007).
//!
//! Phase 1 only declares the menu-item identifier surface that the Phase 4
//! event loop will hook into. The actual `tray-icon::Menu` construction
//! lives in Phase 4 because it requires a live event loop binding.

/// Stable identifiers for tray menu actions. Stored as `&'static str` so
/// the Phase 4 event loop can match on tray-icon `MenuEvent::id()` without
/// stringly-typed allocations.
pub mod ids {
    pub const OPEN: &str = "gwt.tray.open";
    pub const QUIT: &str = "gwt.tray.quit";
    /// `About` is rendered via the OS native About dialog; we still keep
    /// an id so tests can assert the action is wired up.
    pub const ABOUT: &str = "gwt.tray.about";
    /// SPEC #2920 Phase 8 / FR-005 + FR-007: "Start at Login" check
    /// item. Click → `AutostartManager::install()` / `uninstall()`.
    pub const AUTOSTART_TOGGLE: &str = "gwt.tray.autostart";
}

/// Logical menu action used by the Phase 4 event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    Quit,
    About,
    /// SPEC #2920 Phase 8: tray check item that mirrors
    /// `AutostartManager::status()`. Clicking the item toggles the OS
    /// autostart entry; the event loop handler reverts the check state
    /// on failure.
    ToggleAutostart,
}

impl MenuAction {
    /// Map a tray-icon `MenuEvent` id back to a typed action.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            ids::OPEN => Some(Self::Open),
            ids::QUIT => Some(Self::Quit),
            ids::ABOUT => Some(Self::About),
            ids::AUTOSTART_TOGGLE => Some(Self::ToggleAutostart),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_action_round_trip_through_ids() {
        assert_eq!(MenuAction::from_id(ids::OPEN), Some(MenuAction::Open));
        assert_eq!(MenuAction::from_id(ids::QUIT), Some(MenuAction::Quit));
        assert_eq!(MenuAction::from_id(ids::ABOUT), Some(MenuAction::About));
        assert_eq!(
            MenuAction::from_id(ids::AUTOSTART_TOGGLE),
            Some(MenuAction::ToggleAutostart)
        );
        assert_eq!(MenuAction::from_id("unknown"), None);
    }

    #[test]
    fn menu_action_ids_are_stable_and_namespaced() {
        // The Phase 4 event loop persists these ids into tray-icon Menu
        // entries; a rename would silently break click dispatch. Pin the
        // exact strings so future edits surface as test failures.
        assert_eq!(ids::OPEN, "gwt.tray.open");
        assert_eq!(ids::QUIT, "gwt.tray.quit");
        assert_eq!(ids::ABOUT, "gwt.tray.about");
        assert_eq!(ids::AUTOSTART_TOGGLE, "gwt.tray.autostart");
    }
}

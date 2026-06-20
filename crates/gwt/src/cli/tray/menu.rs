//! SPEC #2920 FR-004 — Tray menu skeleton.
//!
//! The minimal tray menu is `[Open in browser] / [Copy URL (<url>)] / [About GWT] / [Quit]`.
//! Settings, autostart toggle, Logs, and Update controls live in the browser UI
//! Settings page (Phase 8 / FR-007).
//!
//! Phase 1 only declares the menu-item identifier surface that the Phase 4
//! event loop will hook into. The actual `tray-icon::Menu` construction
//! lives in Phase 4 because it requires a live event loop binding.

/// Stable identifiers for tray menu actions. Stored as `&'static str` so
/// the Phase 4 event loop can match on tray-icon `MenuEvent::id()` without
/// stringly-typed allocations.
pub mod ids {
    pub const OPEN: &str = "gwt.tray.open";
    pub const COPY_URL: &str = "gwt.tray.copy_url";
    pub const QUIT: &str = "gwt.tray.quit";
    /// `About GWT` opens the browser About / Version surface.
    pub const ABOUT: &str = "gwt.tray.about";
}

/// Logical menu action used by the Phase 4 event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    CopyUrl,
    Quit,
    About,
}

impl MenuAction {
    /// Map a tray-icon `MenuEvent` id back to a typed action.
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            ids::OPEN => Some(Self::Open),
            ids::COPY_URL => Some(Self::CopyUrl),
            ids::QUIT => Some(Self::Quit),
            ids::ABOUT => Some(Self::About),
            _ => None,
        }
    }
}

/// Derive the browser About URL from the running embedded-server URL.
/// Existing fragments are replaced so repeated About clicks are stable.
pub fn about_url_for_browser_url(browser_url: &str) -> String {
    let base = browser_url
        .split_once('#')
        .map_or(browser_url, |(base, _)| base);
    format!("{base}#about")
}

/// Visible tray label for the Copy URL action.
///
/// The label includes the exact root browser URL copied by the handler so
/// the active ephemeral port is visible before the user clicks.
pub fn copy_url_label_for_browser_url(browser_url: &str) -> String {
    format!("Copy URL ({browser_url})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_action_round_trip_through_ids() {
        assert_eq!(MenuAction::from_id(ids::OPEN), Some(MenuAction::Open));
        assert_eq!(
            MenuAction::from_id(ids::COPY_URL),
            Some(MenuAction::CopyUrl)
        );
        assert_eq!(MenuAction::from_id(ids::QUIT), Some(MenuAction::Quit));
        assert_eq!(MenuAction::from_id(ids::ABOUT), Some(MenuAction::About));
        assert_eq!(MenuAction::from_id("unknown"), None);
    }

    #[test]
    fn menu_action_ids_are_stable_and_namespaced() {
        // The Phase 4 event loop persists these ids into tray-icon Menu
        // entries; a rename would silently break click dispatch. Pin the
        // exact strings so future edits surface as test failures.
        assert_eq!(ids::OPEN, "gwt.tray.open");
        assert_eq!(ids::COPY_URL, "gwt.tray.copy_url");
        assert_eq!(ids::QUIT, "gwt.tray.quit");
        assert_eq!(ids::ABOUT, "gwt.tray.about");
    }

    #[test]
    fn about_url_replaces_any_existing_fragment() {
        assert_eq!(
            about_url_for_browser_url("http://127.0.0.1:54321/"),
            "http://127.0.0.1:54321/#about"
        );
        assert_eq!(
            about_url_for_browser_url("http://127.0.0.1:54321/#old"),
            "http://127.0.0.1:54321/#about"
        );
    }

    #[test]
    fn copy_url_label_includes_browser_url() {
        assert_eq!(
            copy_url_label_for_browser_url("http://127.0.0.1:54321/"),
            "Copy URL (http://127.0.0.1:54321/)"
        );
    }
}

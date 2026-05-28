//! SPEC #2920 Phase 4 + 5: bundle identity constants for the
//! tray-resident front door. The wry / muda native menubar definitions
//! that previously lived in this file were removed when the tray-icon
//! path replaced the wry WebView. The remaining constants are pure
//! metadata (no platform code) and stay here so any future
//! distribution-level helper has a single place to consult bundle
//! identity, regardless of whether the heavier GUI crates are linked
//! in.

pub const APP_NAME: &str = "GWT";
pub const MACOS_BUNDLE_IDENTIFIER: &str = "io.github.akiojin.gwt";
pub const MACOS_APP_BUNDLE_NAME: &str = "GWT.app";
pub const GUI_FRONT_DOOR_BINARY_NAME: &str = "gwt";
pub const INTERNAL_DAEMON_BINARY_NAME: &str = "gwtd";

pub fn macos_bundle_identifier() -> &'static str {
    MACOS_BUNDLE_IDENTIFIER
}

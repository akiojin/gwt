//! SPEC #2920 Phase 1 (T-010): verify the tray-resident front-door module
//! skeleton is present and the public surface is wired into `cli.rs`.
//!
//! This test exists so that a future refactor cannot silently drop the
//! placeholder modules before Phase 4 fills them in. It only inspects the
//! source tree via `include_str!`; no runtime behaviour is exercised.

const CLI_ROOT: &str = include_str!("../src/cli.rs");
const TRAY_MOD: &str = include_str!("../src/cli/tray/mod.rs");
const TRAY_MENU: &str = include_str!("../src/cli/tray/menu.rs");
const TRAY_AUTOSTART: &str = include_str!("../src/cli/tray/autostart.rs");
const TRAY_LOCK: &str = include_str!("../src/cli/tray/lock.rs");
const OPEN_CLI: &str = include_str!("../src/cli/open.rs");

#[test]
fn cli_root_declares_tray_and_open_modules() {
    assert!(
        CLI_ROOT.contains("pub mod tray;"),
        "crates/gwt/src/cli.rs must declare `pub mod tray;`"
    );
    assert!(
        CLI_ROOT.contains("pub mod open;"),
        "crates/gwt/src/cli.rs must declare `pub mod open;`"
    );
}

#[test]
fn tray_module_exposes_phase_1_surface() {
    assert!(
        TRAY_MOD.contains("pub mod autostart;"),
        "tray/mod.rs must expose the autostart sub-module"
    );
    assert!(
        TRAY_MOD.contains("pub mod menu;"),
        "tray/mod.rs must expose the menu sub-module"
    );
    assert!(
        TRAY_MOD.contains("pub mod lock;"),
        "tray/mod.rs must expose the lock sub-module"
    );
    assert!(
        TRAY_MOD.contains("pub struct TrayArgs"),
        "tray/mod.rs must define the TrayArgs entry struct"
    );
    assert!(
        TRAY_MOD.contains("pub fn run"),
        "tray/mod.rs must expose `run` as the tray entry point"
    );
}

#[test]
fn tray_menu_pins_action_ids() {
    // Phase 4 event loop dispatches on these exact ids. Renaming them is
    // a breaking change for in-flight tray menu definitions and must be
    // caught at compile time.
    assert!(TRAY_MENU.contains(r#"pub const OPEN: &str = "gwt.tray.open";"#));
    assert!(TRAY_MENU.contains(r#"pub const QUIT: &str = "gwt.tray.quit";"#));
    assert!(TRAY_MENU.contains(r#"pub const ABOUT: &str = "gwt.tray.about";"#));
}

#[test]
fn tray_autostart_pins_status_surface() {
    assert!(TRAY_AUTOSTART.contains("pub struct AutostartStatus"));
    assert!(TRAY_AUTOSTART.contains("pub enum AutostartMechanism"));
    assert!(TRAY_AUTOSTART.contains("pub struct AutostartManager"));
    // Mechanism variants are part of the WebSocket protocol surface; pin
    // them so a casual rename does not break the Settings page contract
    // before Phase 8 ships.
    for mechanism in ["LoginItems", "LaunchAgent", "Registry", "XdgAutostart"] {
        assert!(
            TRAY_AUTOSTART.contains(mechanism),
            "AutostartMechanism must include {mechanism}"
        );
    }
}

#[test]
fn tray_lock_pins_payload_format() {
    assert!(TRAY_LOCK.contains("pub struct TrayLockFile"));
    for field in [
        "pub pid: u32",
        "pub url: String",
        "pub started_at",
        "pub version: String",
    ] {
        assert!(
            TRAY_LOCK.contains(field),
            "TrayLockFile must declare {field}"
        );
    }
    assert!(
        TRAY_LOCK.contains("pub fn lock_path"),
        "tray/lock.rs must expose lock_path resolver"
    );
}

#[test]
fn open_cli_skeleton_is_present() {
    assert!(OPEN_CLI.contains("pub struct OpenArgs"));
    assert!(OPEN_CLI.contains("pub fn parse_args"));
    assert!(OPEN_CLI.contains("pub fn run"));
}

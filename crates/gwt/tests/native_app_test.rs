use gwt::{
    macos_bundle_identifier, macos_native_menu_titles, native_launch_surface,
    native_menu_command_for_id, NativeMenuCommand, GUI_FRONT_DOOR_BINARY_NAME,
    INTERNAL_DAEMON_BINARY_NAME, MACOS_APP_BUNDLE_NAME, OPEN_PROJECT_MENU_ID, RELOAD_MENU_ID,
};

#[test]
fn macos_native_shell_uses_expected_bundle_identifier_and_menu_titles() {
    assert_eq!(macos_bundle_identifier(), "io.github.akiojin.gwt");
    assert_eq!(
        macos_native_menu_titles(),
        &["GWT", "File", "View", "Window"]
    );
}

#[test]
fn native_menu_command_maps_supported_ids_and_ignores_unknown_ids() {
    assert_eq!(
        native_menu_command_for_id(OPEN_PROJECT_MENU_ID),
        Some(NativeMenuCommand::OpenProject)
    );
    assert_eq!(
        native_menu_command_for_id(RELOAD_MENU_ID),
        Some(NativeMenuCommand::ReloadWebView)
    );
    assert_eq!(native_menu_command_for_id("view.unknown"), None);
}

#[test]
fn native_launch_surface_keeps_gui_primary_bundle_and_menu_contract() {
    let surface = native_launch_surface();

    assert_eq!(surface.app_name, "GWT");
    assert_eq!(surface.bundle_identifier, "io.github.akiojin.gwt");
    assert_eq!(surface.bundle_name, MACOS_APP_BUNDLE_NAME);
    assert_eq!(surface.front_door_binary, GUI_FRONT_DOOR_BINARY_NAME);
    assert_eq!(surface.daemon_binary, INTERNAL_DAEMON_BINARY_NAME);
    assert_eq!(surface.menu_titles, &["GWT", "File", "View", "Window"]);
    assert_eq!(
        surface.command_ids,
        &[OPEN_PROJECT_MENU_ID, RELOAD_MENU_ID],
        "native launch path should keep the Open Project / Reload surface stable"
    );
}

use gwt::{
    macos_bundle_identifier, macos_native_menu_titles, native_menu_command_for_id,
    NativeMenuCommand, OPEN_PROJECT_MENU_ID, RELOAD_MENU_ID,
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

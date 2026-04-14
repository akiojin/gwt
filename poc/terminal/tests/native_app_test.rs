use poc_terminal::{
    macos_bundle_identifier, macos_native_menu_titles, native_menu_command_for_id,
    NativeMenuCommand, RELOAD_MENU_ID,
};

#[test]
fn macos_native_shell_uses_expected_bundle_identifier_and_menu_titles() {
    assert_eq!(
        macos_bundle_identifier(),
        "io.github.akiojin.gwt.pocterminal"
    );
    assert_eq!(
        macos_native_menu_titles(),
        &["GWT Terminal PoC", "File", "View", "Window"]
    );
}

#[test]
fn native_menu_command_maps_reload_and_ignores_unknown_ids() {
    assert_eq!(
        native_menu_command_for_id(RELOAD_MENU_ID),
        Some(NativeMenuCommand::ReloadWebView)
    );
    assert_eq!(native_menu_command_for_id("view.unknown"), None);
}

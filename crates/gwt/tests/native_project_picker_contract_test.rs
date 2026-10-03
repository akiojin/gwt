//! The tray process has no NSWindow. rfd's macOS async fallback is therefore
//! synchronous (or panics on a worker); queued-worker tests cannot catch this.
#[test]
fn macos_folder_picker_is_parentless_and_nonmodal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let adapter =
        std::fs::read_to_string(root.join("src/app_runtime/native_project_picker.rs")).unwrap();
    assert!(
        adapter.contains("macos_picker::pick(deadline)"),
        "macOS must use a parentless completion adapter, not synchronous rfd"
    );
    let macos =
        std::fs::read_to_string(root.join("src/app_runtime/native_project_picker/macos.rs"))
            .unwrap();
    assert!(macos.contains("beginWithCompletionHandler("));
    for blocking in [".runModal(", "beginSheetModalForWindow", "rfd::"] {
        assert!(
            !macos.contains(blocking),
            "windowless picker cannot use {blocking}"
        );
    }
}

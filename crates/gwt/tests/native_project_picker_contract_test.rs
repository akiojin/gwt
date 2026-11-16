//! The tray process has no NSWindow. rfd's macOS async fallback is therefore
//! synchronous (or panics on a worker); queued-worker tests cannot catch this.

// Issue #7420: Dependabot updated only the general rfd dependency, leaving a
// second Windows version that rustc could not distinguish (E0464).
#[test]
fn rfd_has_one_version_declaration_with_windows_common_controls() {
    let manifest = include_str!("../Cargo.toml");
    let declarations: Vec<_> = manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("rfd ="))
        .collect();
    assert_eq!(declarations.len(), 1, "rfd must have one version to update");
    assert!(declarations[0].contains("\"common-controls-v6\""));
}

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

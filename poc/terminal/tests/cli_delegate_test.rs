use std::{fs, path::PathBuf};

use tempfile::tempdir;

use poc_terminal::{build_cli_delegate_invocation_from, resolve_canonical_cli_bin_from};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn touch(path: &PathBuf) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, b"#!/bin/sh\n").expect("write file");
}

#[test]
fn build_cli_delegate_invocation_ignores_non_cli_project_path() {
    let current_exe = PathBuf::from("/tmp/poc-terminal");
    let args = argv(&["poc-terminal", "/repo/path"]);

    let invocation =
        build_cli_delegate_invocation_from(&args, &current_exe).expect("build invocation");

    assert!(invocation.is_none());
}

#[test]
fn build_cli_delegate_invocation_for_hook_uses_resolved_cli_binary() {
    let dir = tempdir().expect("tempdir");
    let debug_dir = dir.path().join("target/debug");
    let current_exe = debug_dir.join("poc-terminal");
    let gwt_bin = debug_dir.join("gwt");
    touch(&gwt_bin);

    let invocation = build_cli_delegate_invocation_from(
        &argv(&["poc-terminal", "hook", "runtime-state", "PreToolUse"]),
        &current_exe,
    )
    .expect("build invocation")
    .expect("delegate invocation");

    assert_eq!(invocation.program, gwt_bin);
    assert_eq!(
        invocation.args,
        vec![
            "hook".to_string(),
            "runtime-state".to_string(),
            "PreToolUse".to_string()
        ]
    );
}

#[test]
fn resolve_canonical_cli_bin_from_falls_back_to_sibling_gwt_tui_when_gwt_missing() {
    let dir = tempdir().expect("tempdir");
    let debug_dir = dir.path().join("target/debug");
    let current_exe = debug_dir.join("poc-terminal");
    let gwt_tui_bin = debug_dir.join("gwt-tui");
    touch(&gwt_tui_bin);

    let resolved = resolve_canonical_cli_bin_from(&current_exe).expect("resolve cli bin");

    assert_eq!(resolved, gwt_tui_bin);
}

#[test]
fn resolve_canonical_cli_bin_from_finds_gwt_outside_macos_app_bundle() {
    let dir = tempdir().expect("tempdir");
    let debug_dir = dir.path().join("target/debug");
    let current_exe = debug_dir.join("bundle/osx/GWT Terminal PoC.app/Contents/MacOS/poc-terminal");
    let gwt_bin = debug_dir.join("gwt");
    touch(&gwt_bin);

    let resolved = resolve_canonical_cli_bin_from(&current_exe).expect("resolve cli bin");

    assert_eq!(resolved, gwt_bin);
}

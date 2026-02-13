use std::fs;
use std::path::{Path, PathBuf};

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn find_violations(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut violations = Vec::new();
    for path in files {
        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        if src.contains("Command::new(\"git\")")
            || src.contains("std::process::Command::new(\"git\")")
        {
            let rel = path.strip_prefix(root).unwrap_or(path);
            violations.push(normalize_path(rel));
        }
    }
    violations.sort();
    violations
}

fn find_process_constructor_violations(root: &Path, files: &[PathBuf]) -> Vec<String> {
    let mut violations = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_display = normalize_path(rel);
        if rel_display == "crates/gwt-core/src/process.rs"
            || rel_display == "crates/gwt-core/tests/no_direct_git_command.rs"
        {
            continue;
        }

        let Ok(src) = fs::read_to_string(path) else {
            continue;
        };
        if src.contains("Command::new(")
            || src.contains("std::process::Command::new(")
            || src.contains("tokio::process::Command::new(")
        {
            violations.push(rel_display);
        }
    }
    violations.sort();
    violations
}

#[test]
fn runtime_sources_do_not_invoke_git_directly() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_root
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root should exist");

    let mut files = Vec::new();
    collect_rs_files(&crate_root.join("src"), &mut files);
    collect_rs_files(
        &repo_root.join("crates").join("gwt-tauri").join("src"),
        &mut files,
    );

    let violations = find_violations(repo_root, &files);
    assert!(
        violations.is_empty(),
        "Direct git command invocation detected in source files: {}",
        violations.join(", ")
    );
}

#[test]
fn workspace_sources_use_process_helpers_for_external_commands() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_root
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root should exist");

    let mut files = Vec::new();
    collect_rs_files(&crate_root.join("src"), &mut files);
    collect_rs_files(&crate_root.join("tests"), &mut files);
    collect_rs_files(&crate_root.join("benches"), &mut files);
    collect_rs_files(
        &repo_root.join("crates").join("gwt-tauri").join("src"),
        &mut files,
    );

    let violations = find_process_constructor_violations(repo_root, &files);
    assert!(
        violations.is_empty(),
        "Direct process Command::new invocation detected in workspace Rust files: {}",
        violations.join(", ")
    );
}

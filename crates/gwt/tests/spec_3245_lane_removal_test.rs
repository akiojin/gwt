//! SPEC #3245 AC-4 — the lane mechanism is removed from non-test code.
//!
//! After Stage C no production source may reference the `GWT_SESSION_KIND`
//! environment variable or the `.gwt/session-kind.json` lane file. Test code
//! (files under `tests/`, and in-file `#[cfg(test)]` modules, which sit at the
//! end of their file by repository convention) is exempt while it pins the
//! removal behavior itself.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["GWT_SESSION_KIND", "session-kind.json", "session-kind"];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "tests" || name == "target" {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Strip the trailing `#[cfg(test)] mod …` module (repository convention keeps
/// it at the end of the file) so in-file unit tests may keep pinning the
/// removal. Plain `#[cfg(test)]` attributes on imports do NOT end the
/// production region — only the test module marker does.
fn non_test_source(contents: &str) -> &str {
    let mut search_from = 0;
    while let Some(relative) = contents[search_from..].find("#[cfg(test)]") {
        let index = search_from + relative;
        let after = contents[index..]
            .lines()
            .nth(1)
            .map(str::trim_start)
            .unwrap_or("");
        if after.starts_with("mod ") || after.starts_with("pub mod ") {
            return &contents[..index];
        }
        search_from = index + "#[cfg(test)]".len();
    }
    contents
}

#[test]
fn non_test_code_has_no_lane_mechanism_references() {
    let crates_root = workspace_root().join("crates");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates_root)
        .expect("read crates dir")
        .flatten()
    {
        let src = entry.path().join("src");
        if src.is_dir() {
            collect_rs_files(&src, &mut files);
        }
    }
    assert!(
        files.len() > 100,
        "sanity: the sweep must actually visit the workspace sources, got {}",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in files {
        let contents = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        let production = non_test_source(&contents);
        for needle in FORBIDDEN {
            if production.contains(needle) {
                offenders.push(format!("{} -> {needle}", file.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "lane mechanism references must be gone from non-test code (SPEC #3245 AC-4):\n{}",
        offenders.join("\n")
    );
}

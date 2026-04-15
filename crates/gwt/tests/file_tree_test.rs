use std::path::Path;

use gwt::{list_directory_entries, FileTreeEntryKind};
use tempfile::tempdir;

#[test]
fn list_directory_entries_filters_gitignored_and_builtin_skipped_paths() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("create src");
    std::fs::create_dir_all(dir.path().join("ignored")).expect("create ignored");
    std::fs::create_dir_all(dir.path().join(".git")).expect("create .git");
    std::fs::create_dir_all(dir.path().join(".gwt")).expect("create .gwt");
    std::fs::create_dir_all(dir.path().join("target")).expect("create target");
    std::fs::write(dir.path().join(".gitignore"), "ignored/\n*.log\n").expect("write gitignore");
    std::fs::write(dir.path().join("README.md"), "# demo").expect("write readme");
    std::fs::write(dir.path().join("debug.log"), "ignored").expect("write log");

    let entries = list_directory_entries(dir.path(), None).expect("root entries");

    let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
    assert!(paths.contains(&"src"));
    assert!(paths.contains(&"README.md"));
    assert!(!paths.contains(&"ignored"));
    assert!(!paths.contains(&".git"));
    assert!(!paths.contains(&".gwt"));
    assert!(!paths.contains(&"target"));
    assert!(!paths.contains(&"debug.log"));
}

#[test]
fn list_directory_entries_sorts_directories_before_files() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src").join("zeta")).expect("create zeta");
    std::fs::create_dir_all(dir.path().join("src").join("alpha")).expect("create alpha");
    std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("write main");
    std::fs::write(dir.path().join("src").join("lib.rs"), "").expect("write lib");

    let entries = list_directory_entries(dir.path(), Some(Path::new("src"))).expect("src entries");

    let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "zeta", "lib.rs", "main.rs"]);
    assert_eq!(entries[0].kind, FileTreeEntryKind::Directory);
    assert_eq!(entries[2].kind, FileTreeEntryKind::File);
}

#[test]
fn list_directory_entries_rejects_paths_outside_repository_root() {
    let dir = tempdir().expect("tempdir");
    let error = list_directory_entries(dir.path(), Some(Path::new("../outside")))
        .expect_err("path traversal should fail");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

use std::{fs, process::Command};

use gwt_core::index::path_policy::{build_project_ignore_matcher, default_index_path_policy};
use tempfile::tempdir;

#[test]
fn index_path_policy_honors_nested_gitignore() {
    let dir = tempdir().expect("tempdir");
    let app = dir.path().join("packages/app");
    fs::create_dir_all(&app).expect("create app");
    fs::write(app.join(".gitignore"), "*.generated\n").expect("write nested gitignore");
    fs::write(app.join("view.generated"), "ignored").expect("write ignored file");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    assert!(
        !policy.is_indexable_path(&matcher, dir.path(), &app.join("view.generated")),
        "nested .gitignore rules must be part of the index path policy"
    );
}

#[test]
fn index_path_policy_honors_git_info_exclude() {
    let dir = tempdir().expect("tempdir");
    Command::new("git")
        .arg("init")
        .arg(dir.path())
        .output()
        .expect("git init");
    fs::write(dir.path().join(".git/info/exclude"), "local-secret.txt\n")
        .expect("write info exclude");
    fs::write(dir.path().join("local-secret.txt"), "secret").expect("write excluded file");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    assert!(
        !policy.is_indexable_path(&matcher, dir.path(), &dir.path().join("local-secret.txt")),
        "$GIT_DIR/info/exclude must be part of the project-local index policy"
    );
}

#[test]
fn index_path_policy_ignores_global_gitignore_files() {
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    fs::create_dir_all(home.join(".config/git")).expect("create global git config dir");
    fs::write(home.join(".config/git/ignore"), "global-only.txt\n").expect("write global ignore");
    fs::write(dir.path().join("global-only.txt"), "keep searchable").expect("write file");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    assert!(
        policy.is_indexable_path(&matcher, dir.path(), &dir.path().join("global-only.txt")),
        "semantic index policy must not depend on user-global git ignore files"
    );
}

#[test]
fn index_path_policy_allowlists_shared_knowledge_files_only_under_tasks() {
    let dir = tempdir().expect("tempdir");
    let tasks = dir.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    assert!(policy.is_indexable_path(&matcher, dir.path(), &tasks.join("memory.md")));
    assert!(policy.is_indexable_path(&matcher, dir.path(), &tasks.join("discussions.md")));
    assert!(!policy.is_indexable_path(&matcher, dir.path(), &tasks.join("todo.md")));
    assert!(!policy.is_indexable_path(&matcher, dir.path(), &tasks.join("spec-1939/notes.md")));
}

#[test]
fn index_path_policy_denies_common_generated_directories() {
    let dir = tempdir().expect("tempdir");
    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    for rel in [
        "node_modules/pkg/index.js",
        "target/debug/app",
        ".venv/lib/python/site.py",
        ".pytest_cache/v/cache/nodeids",
        ".gradle/caches/modules-2.bin",
        ".terraform/providers/state.json",
        "coverage/lcov.info",
        "dist/bundle.js",
        "build/output.o",
    ] {
        assert!(
            !policy.is_indexable_path(&matcher, dir.path(), &dir.path().join(rel)),
            "built-in generated directory should be denied: {rel}"
        );
    }
}

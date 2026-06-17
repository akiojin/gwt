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
fn index_path_policy_scopes_nested_gitignore_to_own_directory() {
    let dir = tempdir().expect("tempdir");
    let husky = dir.path().join(".husky/_");
    fs::create_dir_all(&husky).expect("create husky dir");
    fs::write(husky.join(".gitignore"), "*\n").expect("write nested wildcard ignore");
    fs::write(husky.join("shim.sh"), "ignored").expect("write nested ignored file");
    fs::write(dir.path().join("README.md"), "visible").expect("write root file");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    assert!(
        policy.is_indexable_path(&matcher, dir.path(), &dir.path().join("README.md")),
        "nested wildcard .gitignore must not hide root files"
    );
    assert!(
        !policy.is_indexable_path(&matcher, dir.path(), &husky.join("shim.sh")),
        "nested wildcard .gitignore should still hide files in its own directory"
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
fn index_path_policy_allowlists_shared_knowledge_files_only_under_gwt_work() {
    let dir = tempdir().expect("tempdir");
    let work = dir.path().join(".gwt/work");
    fs::create_dir_all(&work).expect("create work dir");
    let tasks = dir.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks");

    let policy = default_index_path_policy();
    let matcher = build_project_ignore_matcher(dir.path());

    // The shared knowledge files now live under the tracked `.gwt/work/`
    // directory and are allowlisted out of the broad `.gwt` deny prefix.
    assert!(policy.is_indexable_path(&matcher, dir.path(), &work.join("memory.md")));
    assert!(policy.is_indexable_path(&matcher, dir.path(), &work.join("discussions.md")));
    // The Work event log under the same directory is not allowlisted.
    assert!(!policy.is_indexable_path(&matcher, dir.path(), &work.join("events.jsonl")));
    // Legacy `tasks/` knowledge files are no longer allowlisted.
    assert!(!policy.is_indexable_path(&matcher, dir.path(), &tasks.join("memory.md")));
    assert!(!policy.is_indexable_path(&matcher, dir.path(), &tasks.join("discussions.md")));
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

use gwt::gui_single_instance::{acquire_gui_instance_lock, gui_instance_lock_path};
use tempfile::tempdir;

#[test]
fn gui_instance_lock_rejects_second_owner_for_same_worktree() {
    let home = tempdir().expect("home");
    let project = tempdir().expect("project");

    let first = acquire_gui_instance_lock(home.path(), project.path()).expect("first lock");
    let second = acquire_gui_instance_lock(home.path(), project.path())
        .expect_err("second lock for the same worktree should fail");

    assert!(second.to_string().contains("already running"));
    assert!(second
        .to_string()
        .contains(&project.path().display().to_string()));
    drop(first);
}

#[test]
fn gui_instance_lock_is_scoped_by_worktree_path() {
    let home = tempdir().expect("home");
    let project_a = tempdir().expect("project-a");
    let project_b = tempdir().expect("project-b");

    let _first = acquire_gui_instance_lock(home.path(), project_a.path()).expect("first lock");
    let _second =
        acquire_gui_instance_lock(home.path(), project_b.path()).expect("different worktree lock");

    let lock_a = gui_instance_lock_path(home.path(), project_a.path()).expect("lock path a");
    let lock_b = gui_instance_lock_path(home.path(), project_b.path()).expect("lock path b");

    assert_ne!(lock_a, lock_b);
    assert!(lock_a
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".lock")));
    assert!(lock_b
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".lock")));
}

#[test]
fn gui_instance_lock_released_on_drop_allows_reacquire() {
    let home = tempdir().expect("home");
    let project = tempdir().expect("project");

    let first = acquire_gui_instance_lock(home.path(), project.path()).expect("first lock");
    drop(first);

    let second = acquire_gui_instance_lock(home.path(), project.path())
        .expect("reacquire after drop should succeed");
    drop(second);
}

#[test]
fn gui_instance_lock_path_rejects_relative_project_root() {
    let home = tempdir().expect("home");
    let result = gui_instance_lock_path(home.path(), std::path::Path::new("relative/path"));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("scope"));
}

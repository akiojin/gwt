use std::{
    fs,
    process::{Command, Stdio},
};

fn run_gwtd_in(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run gwtd")
}

fn work_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".gwt").join("work")
}

#[test]
fn memory_add_appends_typed_entry_to_existing_memory_file() {
    let repo = tempfile::tempdir().expect("repo");
    let work = work_dir(repo.path());
    fs::create_dir_all(&work).expect("create work dir");
    fs::write(work.join("memory.md"), "# Memory\n\n").expect("seed memory");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "memory",
            "add",
            "--date",
            "2026-05-20",
            "--type",
            "workflow",
            "--title",
            "hook reminder writer",
            "--context",
            "Hook reminders exposed memory but did not provide a writer.",
            "--learning",
            "A durable memory loop needs a supported gwt append command.",
            "--future-action",
            "Use gwtd memory add before reporting reusable learning as done.",
        ],
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized_stdout = stdout.replace('\\', "/");
    assert!(
        normalized_stdout.contains(".gwt/work/memory.md"),
        "stdout should name updated path, got: {stdout}"
    );
    let memory = fs::read_to_string(work.join("memory.md")).expect("read memory");
    assert!(memory.contains("## 2026-05-20 — hook reminder writer"));
    assert!(memory.contains("Type: workflow"));
    assert!(memory.contains("Context: Hook reminders exposed memory but did not provide a writer."));
    assert!(
        memory.contains("Learning: A durable memory loop needs a supported gwt append command.")
    );
    assert!(memory.contains(
        "Future Action: Use gwtd memory add before reporting reusable learning as done."
    ));
}

#[test]
fn lessons_add_alias_migrates_legacy_file_and_appends_entry() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Old Lessons\n\n## 2026-04-01 — old entry\n\nSome old content.\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "lessons",
            "add",
            "--date",
            "2026-05-20",
            "--title",
            "legacy alias writer",
            "--context",
            "Older prompts still say lessons.",
            "--learning",
            "The lessons alias should still write canonical memory.",
            "--future-action",
            "Keep new entries in .gwt/work/memory.md.",
        ],
    );

    assert!(
        output.status.success(),
        "lessons add alias should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory =
        fs::read_to_string(work_dir(repo.path()).join("memory.md")).expect("memory created");
    assert!(
        memory.contains("old entry"),
        "migrated content should be preserved"
    );
    assert!(memory.contains("## 2026-05-20 — legacy alias writer"));
    assert!(memory.contains("Type: lesson"));
    assert!(
        !tasks.join("lessons.md").exists(),
        "lessons.md should be removed after migration"
    );
}

#[test]
fn memory_add_migrates_legacy_tasks_memory_to_repo_local_work_dir() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Memory\n\n## 2026-03-01 — legacy tasks entry\n\nType: lesson\nContext: old\nLearning: old\nFuture Action: old\n";
    fs::write(tasks.join("memory.md"), legacy).expect("seed tasks memory");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "memory",
            "add",
            "--date",
            "2026-05-30",
            "--title",
            "entry after work-dir migration",
            "--context",
            "tasks/memory.md should move to .gwt/work/memory.md once.",
            "--learning",
            "The move preserves prior entries.",
            "--future-action",
            "Read .gwt/work/memory.md going forward.",
        ],
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let work = work_dir(repo.path());
    let memory = fs::read_to_string(work.join("memory.md")).expect("read memory");
    assert!(
        memory.contains("legacy tasks entry"),
        "prior tasks/memory.md content should be preserved via move"
    );
    assert!(memory.contains("## 2026-05-30 — entry after work-dir migration"));
    assert!(
        !tasks.join("memory.md").exists(),
        "tasks/memory.md should be moved, not duplicated"
    );
}

#[test]
fn memory_add_migrates_legacy_lessons_when_memory_absent() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Lessons\n\n## 2026-03-15 — prior knowledge\n\nType: lesson\nContext: old context\nLearning: old learning\nFuture Action: old action\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "memory",
            "add",
            "--date",
            "2026-05-24",
            "--title",
            "new entry after migration",
            "--context",
            "Testing migration path.",
            "--learning",
            "Legacy file should be renamed.",
            "--future-action",
            "Verify migration is automatic.",
        ],
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory = fs::read_to_string(work_dir(repo.path()).join("memory.md")).expect("read memory");
    assert!(
        memory.contains("prior knowledge"),
        "old entries should be preserved via rename"
    );
    assert!(memory.contains("## 2026-05-24 — new entry after migration"));
    assert!(
        !tasks.join("lessons.md").exists(),
        "lessons.md should not exist after migration"
    );
}

#[test]
fn memory_add_skips_migration_when_canonical_work_file_exists() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    let work = work_dir(repo.path());
    fs::create_dir_all(&tasks).expect("create tasks dir");
    fs::create_dir_all(&work).expect("create work dir");
    let legacy = "# Old Lessons\n";
    let legacy_tasks_memory = "# Legacy tasks memory\n";
    let existing = "# Memory\n\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons");
    fs::write(tasks.join("memory.md"), legacy_tasks_memory).expect("seed tasks memory");
    fs::write(work.join("memory.md"), existing).expect("seed canonical memory");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "memory",
            "add",
            "--date",
            "2026-05-24",
            "--title",
            "no migration needed",
            "--context",
            "Canonical work file already exists.",
            "--learning",
            "Migration should be skipped.",
            "--future-action",
            "Only append to .gwt/work/memory.md.",
        ],
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(tasks.join("lessons.md")).expect("read legacy"),
        legacy,
        "lessons.md must not be modified when canonical memory already exists"
    );
    assert_eq!(
        fs::read_to_string(tasks.join("memory.md")).expect("read legacy tasks memory"),
        legacy_tasks_memory,
        "tasks/memory.md must not be moved when canonical memory already exists"
    );
    let memory = fs::read_to_string(work.join("memory.md")).expect("read memory");
    assert!(memory.contains("## 2026-05-24 — no migration needed"));
}

#[test]
fn memory_add_rejects_empty_required_values_without_writing() {
    let repo = tempfile::tempdir().expect("repo");

    let output = run_gwtd_in(
        repo.path(),
        &[
            "memory",
            "add",
            "--date",
            "2026-05-20",
            "--title",
            "missing context",
            "--context",
            "   ",
            "--learning",
            "Learning",
            "--future-action",
            "Action",
        ],
    );

    assert!(!output.status.success(), "empty context should fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("must not be empty"),
        "stderr should explain validation failure, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !work_dir(repo.path()).join("memory.md").exists(),
        "invalid input must not create memory file"
    );
}

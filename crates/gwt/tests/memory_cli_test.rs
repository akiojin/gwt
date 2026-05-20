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

#[test]
fn memory_add_appends_typed_entry_to_existing_memory_file() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    fs::write(tasks.join("memory.md"), "# Memory\n\n").expect("seed memory");

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
    assert!(
        stdout.contains("tasks/memory.md"),
        "stdout should name updated path, got: {stdout}"
    );
    let memory = fs::read_to_string(tasks.join("memory.md")).expect("read memory");
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
fn lessons_add_alias_creates_memory_file_without_appending_legacy_stub() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Moved to tasks/memory.md\n\nlegacy pointer only\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons stub");

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
            "Keep new entries in tasks/memory.md.",
        ],
    );

    assert!(
        output.status.success(),
        "lessons add alias should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory = fs::read_to_string(tasks.join("memory.md")).expect("memory created");
    assert!(memory.contains("# Memory"));
    assert!(memory.contains("## 2026-05-20 — legacy alias writer"));
    assert!(memory.contains("Type: lesson"));
    assert_eq!(
        fs::read_to_string(tasks.join("lessons.md")).expect("read legacy"),
        legacy,
        "legacy lessons stub must not be appended"
    );
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
        !repo.path().join("tasks/memory.md").exists(),
        "invalid input must not create memory file"
    );
}

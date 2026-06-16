use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn run_gwtd_json(root: &std::path::Path, payload: serde_json::Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");
    child
        .stdin
        .as_mut()
        .expect("gwtd stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write gwtd JSON");
    child.wait_with_output().expect("wait gwtd")
}

#[test]
fn discussion_update_creates_single_canonical_discussions_file() {
    let repo = tempfile::tempdir().expect("repo");

    let output = run_gwtd_json(
        repo.path(),
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "date": "2026-05-22",
                "title": "Workspace / Work / Discussion terminology",
                "status": "active",
                "topics": ["workspace", "work"],
                "related_specs": [2359],
                "summary": "Workspace is being split into Project State, Work, Agent, Discussion, and Branch.",
                "decisions": [
                    "Discussion is not Work.",
                    "Discussions are saved in .gwt/work/discussions.md."
                ],
                "open_questions": ["How should Topic Stack resume across sessions?"],
                "next": "Define Project State migration."
            }
        }),
    );

    assert!(
        output.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized_stdout = stdout.replace('\\', "/");
    assert!(
        normalized_stdout.contains(".gwt/work/discussions.md"),
        "stdout should name updated path, got: {stdout}"
    );
    let content =
        fs::read_to_string(repo.path().join(".gwt/work/discussions.md")).expect("read discussions");
    assert!(content.contains("# Discussions"));
    assert!(content.contains("## 2026-05-22 — Workspace / Work / Discussion terminology"));
    assert!(content.contains("Status: active"));
    assert!(content.contains("Topics: workspace, work"));
    assert!(content.contains("Related SPECs: #2359"));
    assert!(content.contains("- Discussion is not Work."));
    assert!(content.contains("- Discussions are saved in .gwt/work/discussions.md."));
    assert!(content.contains("- How should Topic Stack resume across sessions?"));
    assert!(content.contains("Define Project State migration."));
}

#[test]
fn discussion_update_rewrites_existing_section_instead_of_appending_duplicate() {
    let repo = tempfile::tempdir().expect("repo");

    for summary in ["First summary", "Updated summary"] {
        let output = run_gwtd_json(
            repo.path(),
            serde_json::json!({
                "schema_version": 1,
                "operation": "discussion.update",
                "params": {
                    "date": "2026-05-22",
                    "title": "Workspace terminology",
                    "status": "active",
                    "summary": summary,
                    "decisions": [summary],
                    "next": "Continue"
                }
            }),
        );
        assert!(
            output.status.success(),
            "discussion update should succeed, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let content =
        fs::read_to_string(repo.path().join(".gwt/work/discussions.md")).expect("read discussions");
    assert_eq!(
        content
            .matches("## 2026-05-22 — Workspace terminology")
            .count(),
        1,
        "active discussion should keep one canonical section"
    );
    assert!(!content.contains("First summary"));
    assert!(content.contains("Updated summary"));
}

#[test]
fn discussion_update_migrates_legacy_tasks_discussions_to_work_dir() {
    let repo = tempfile::tempdir().expect("repo");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Discussions\n\n## 2026-04-01 — legacy discussion\n\nStatus: completed\nTopics: legacy\nRelated SPECs:\nRelated Works:\nPromoted To:\n\nSummary:\nOld discussion preserved.\n\nDecisions:\n\nOpen Questions:\n\nNext:\nNothing.\n";
    fs::write(tasks.join("discussions.md"), legacy).expect("seed legacy discussions");

    let output = run_gwtd_json(
        repo.path(),
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "date": "2026-05-30",
                "title": "entry after work-dir migration",
                "status": "active",
                "summary": "New discussion after move.",
                "next": "Continue."
            }
        }),
    );

    assert!(
        output.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content =
        fs::read_to_string(repo.path().join(".gwt/work/discussions.md")).expect("read discussions");
    assert!(
        content.contains("legacy discussion"),
        "prior tasks/discussions.md content should be preserved via move"
    );
    assert!(content.contains("## 2026-05-30 — entry after work-dir migration"));
    assert!(
        !tasks.join("discussions.md").exists(),
        "tasks/discussions.md should be moved, not duplicated"
    );
}

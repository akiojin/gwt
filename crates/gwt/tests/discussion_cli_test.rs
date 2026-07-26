use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use gwt_core::process::hidden_command;

/// Run gwtd with an isolated HOME so `discussion.update` writes into the
/// machine-local work-notes scratch of this test only (SPEC-3214 FR-007).
fn run_gwtd_json(
    root: &std::path::Path,
    home: &std::path::Path,
    payload: serde_json::Value,
) -> std::process::Output {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
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

/// Run the real Stop hook dispatcher against the isolated discussion log.
fn run_gwtd_stop_hook(root: &Path, home: &Path) -> std::process::Output {
    let mut child = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("GWT_SESSION_ID")
        .env_remove("GWT_SESSION_RUNTIME_PATH")
        .args(["hook", "event", "Stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd Stop hook");
    child
        .stdin
        .as_mut()
        .expect("gwtd Stop stdin")
        .write_all(br#"{"stop_hook_active":false}"#)
        .expect("write Stop hook JSON");
    child.wait_with_output().expect("wait gwtd Stop hook")
}

/// Locate the single machine-local work-notes discussions file under the
/// isolated HOME (`<home>/.gwt/projects/<repo-hash>/work-notes/discussions.md`).
fn home_discussions_path(home: &Path) -> PathBuf {
    let projects = home.join(".gwt").join("projects");
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(&projects) {
        for entry in entries.filter_map(Result::ok) {
            let candidate = entry.path().join("work-notes").join("discussions.md");
            if candidate.is_file() {
                found.push(candidate);
            }
        }
    }
    assert!(
        found.len() == 1,
        "expected exactly one home work-notes discussions file, got {found:?}"
    );
    found.pop().expect("home discussions file")
}

#[test]
fn discussion_update_creates_single_canonical_discussions_file() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
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
                    "Discussions are saved in the machine-local work-notes log."
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
        normalized_stdout.contains("work-notes/discussions.md"),
        "stdout should name the machine-local path, got: {stdout}"
    );
    let content = fs::read_to_string(home_discussions_path(home.path())).expect("read discussions");
    assert!(content.contains("# Discussions"));
    assert!(content.contains("## 2026-05-22 — Workspace / Work / Discussion terminology"));
    assert!(content.contains("Status: active"));
    assert!(content.contains("Topics: workspace, work"));
    assert!(content.contains("Related SPECs: #2359"));
    assert!(content.contains("- Discussion is not Work."));
    assert!(content.contains("- How should Topic Stack resume across sessions?"));
    assert!(content.contains("Define Project State migration."));
    // SPEC-3214 FR-007: no repo-local (git-tracked) discussions file appears.
    assert!(
        !repo.path().join(".gwt/work/discussions.md").exists(),
        "discussion.update must not create the repo-local .gwt/work/discussions.md"
    );
}

#[test]
fn discussion_update_rewrites_existing_section_instead_of_appending_duplicate() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    for summary in ["First summary", "Updated summary"] {
        let output = run_gwtd_json(
            repo.path(),
            home.path(),
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

    let content = fs::read_to_string(home_discussions_path(home.path())).expect("read discussions");
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
fn discussion_update_keeps_nested_h2_inside_entry_and_preserves_next_dated_entry() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    let update = |title: &str, summary: &str| {
        run_gwtd_json(
            repo.path(),
            home.path(),
            serde_json::json!({
                "schema_version": 1,
                "operation": "discussion.update",
                "params": {
                    "date": "2026-07-26",
                    "title": title,
                    "status": "active",
                    "summary": summary,
                    "next": "Continue"
                }
            }),
        )
    };

    let first = update(
        "Discussion boundary",
        "First summary.\n\n## 2026- 7-06 — Nested note\n\nKeep this nested H2.\n\n## Discussion TODO\n\n### Proposal 3248-E - Continue work [active]\n- Next Question: Old question.",
    );
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );

    let later = update("Later proposal", "This entry must remain unchanged.");
    assert!(
        later.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&later.stderr)
    );

    let discussions_path = home_discussions_path(home.path());
    let mut legacy_duplicate =
        fs::read_to_string(&discussions_path).expect("read discussions before duplicate seed");
    let later_heading = "## 2026-07-26 — Later proposal";
    legacy_duplicate = legacy_duplicate.replacen(
        later_heading,
        &format!(
            "## Legacy gwt-discussion state\n\n\
             Status: active\n\n\
             Summary:\n\
             Migrated from legacy .gwt/discussion.md.\n\n\
             ## Discussion TODO\n\n\
             ### Proposal Legacy - Resume [parked]\n\
             - Next Question: Preserve this legacy state.\n\n\
             {later_heading}"
        ),
        1,
    );
    legacy_duplicate.push_str(
        "\n## 2026-07-26 — Discussion boundary\n\nStatus: active\n\nSummary:\nLegacy duplicate.\n\nNext:\nContinue\n",
    );
    fs::write(&discussions_path, legacy_duplicate).expect("seed legacy duplicate entry");

    let replacement = update(
        "Discussion boundary",
        "Updated summary.\n\n## 2026- 7-06 — Nested note\n\nKeep this nested H2.\n\n## Discussion TODO\n\n### Proposal 3248-E - Continue work [active]\n- Next Question: New question.",
    );
    assert!(
        replacement.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&replacement.stderr)
    );

    let content = fs::read_to_string(home_discussions_path(home.path())).expect("read discussions");
    assert_eq!(
        content
            .matches("## 2026-07-26 — Discussion boundary")
            .count(),
        1,
        "the updated proposal must keep one dated entry"
    );
    assert_eq!(
        content.matches("## Discussion TODO").count(),
        2,
        "the current nested TODO and standalone migrated TODO must each remain once"
    );
    assert_eq!(
        content.matches("## 2026- 7-06 — Nested note").count(),
        1,
        "a non-canonical date-like H2 must remain entry body content"
    );
    assert_eq!(
        content
            .matches("### Proposal 3248-E - Continue work [active]")
            .count(),
        1,
        "the Stop hook parser must see one current proposal"
    );
    assert!(!content.contains("First summary."));
    assert!(!content.contains("- Next Question: Old question."));
    assert!(!content.contains("Legacy duplicate."));
    assert!(content.contains("Updated summary."));
    assert_eq!(content.matches("- Next Question: New question.").count(), 1);
    assert_eq!(
        content.matches("## Legacy gwt-discussion state").count(),
        1,
        "a standalone migrated legacy entry must survive the dated update"
    );
    assert!(content.contains("Migrated from legacy .gwt/discussion.md."));
    assert!(content.contains("### Proposal Legacy - Resume [parked]"));
    assert!(content.contains("- Next Question: Preserve this legacy state."));
    assert_eq!(
        content.matches("## 2026-07-26 — Later proposal").count(),
        1,
        "the next dated entry must be preserved"
    );
    assert!(content.contains("This entry must remain unchanged."));

    let stop = run_gwtd_stop_hook(repo.path(), home.path());
    assert!(
        stop.status.success(),
        "Stop hook should succeed, stderr: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let stop_stdout = String::from_utf8_lossy(&stop.stdout);
    assert!(
        stop_stdout.contains("Discussion is still [active]"),
        "real Stop hook should surface the active proposal: {stop_stdout}"
    );
    assert_eq!(
        stop_stdout.matches("New question.").count(),
        1,
        "real Stop hook should surface only the current question: {stop_stdout}"
    );
    assert!(
        !stop_stdout.contains("Old question."),
        "real Stop hook must not surface the replaced question: {stop_stdout}"
    );
}

#[test]
fn discussion_update_migrates_legacy_tasks_discussions_to_home() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Discussions\n\n## 2026-04-01 — legacy discussion\n\nStatus: completed\nTopics: legacy\nRelated SPECs:\nRelated Works:\nPromoted To:\n\nSummary:\nOld discussion preserved.\n\nDecisions:\n\nOpen Questions:\n\nNext:\nNothing.\n";
    fs::write(tasks.join("discussions.md"), legacy).expect("seed legacy discussions");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "date": "2026-05-30",
                "title": "entry after work-notes migration",
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
    let content = fs::read_to_string(home_discussions_path(home.path())).expect("read discussions");
    assert!(
        content.contains("legacy discussion"),
        "prior tasks/discussions.md content should be preserved via move"
    );
    assert!(content.contains("## 2026-05-30 — entry after work-notes migration"));
    assert!(
        !tasks.join("discussions.md").exists(),
        "tasks/discussions.md should be moved, not duplicated"
    );
}

/// SPEC-3214 FR-007: a pre-migration repo-local `.gwt/work/discussions.md`
/// is imported (copied) into the home file on the first write; the
/// git-tracked source stays intact.
#[test]
fn discussion_update_imports_repo_local_work_file_and_keeps_source() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let work = repo.path().join(".gwt").join("work");
    fs::create_dir_all(&work).expect("create work dir");
    let repo_local = "# Discussions\n\n## 2026-04-10 — repo-local discussion\n\nStatus: completed\nTopics: legacy\n\nSummary:\nRepo-local content.\n\nNext:\nNothing.\n";
    fs::write(work.join("discussions.md"), repo_local).expect("seed repo-local discussions");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "date": "2026-07-03",
                "title": "entry after home import",
                "status": "active",
                "summary": "Written after the one-time import.",
                "next": "Continue."
            }
        }),
    );

    assert!(
        output.status.success(),
        "discussion update should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = fs::read_to_string(home_discussions_path(home.path())).expect("read discussions");
    assert!(content.contains("repo-local discussion"));
    assert!(content.contains("## 2026-07-03 — entry after home import"));
    assert_eq!(
        fs::read_to_string(work.join("discussions.md")).expect("read repo-local"),
        repo_local,
        "repo-local discussions.md must be left intact (copy, not move)"
    );
}

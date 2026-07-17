use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};

use gwt_core::process::hidden_command;

/// Run gwtd with an isolated HOME so `memory.add` writes into the
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

fn memory_add_payload(
    date: &str,
    memory_type: Option<&str>,
    title: &str,
    context: &str,
    learning: &str,
    future_action: &str,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "date": date,
        "title": title,
        "context": context,
        "learning": learning,
        "future_action": future_action,
    });
    if let Some(memory_type) = memory_type {
        params["type"] = serde_json::json!(memory_type);
    }
    serde_json::json!({
        "schema_version": 1,
        "operation": "memory.add",
        "params": params,
    })
}

fn work_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".gwt").join("work")
}

/// Locate the single machine-local work-notes memory file under the isolated
/// HOME (`<home>/.gwt/projects/<repo-hash>/work-notes/memory.md`). The repo
/// hash is computed by gwtd from its cwd, so the test discovers it by walking
/// the projects dir instead of recomputing the hash.
fn home_memory_path(home: &Path) -> Option<PathBuf> {
    let projects = home.join(".gwt").join("projects");
    let entries = fs::read_dir(&projects).ok()?;
    let mut found = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let candidate = entry.path().join("work-notes").join("memory.md");
        if candidate.is_file() {
            found.push(candidate);
        }
    }
    assert!(
        found.len() <= 1,
        "expected at most one work-notes memory file, got {found:?}"
    );
    found.pop()
}

#[test]
fn memory_add_writes_to_machine_local_work_notes() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-20",
            Some("workflow"),
            "hook reminder writer",
            "Hook reminders exposed memory but did not provide a writer.",
            "A durable memory loop needs a supported gwt append command.",
            "Use memory.add before reporting reusable learning as done.",
        ),
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let normalized_stdout = stdout.replace('\\', "/");
    assert!(
        normalized_stdout.contains("work-notes/memory.md"),
        "stdout should name the machine-local path, got: {stdout}"
    );
    let memory_path = home_memory_path(home.path()).expect("home memory file created");
    let memory = fs::read_to_string(memory_path).expect("read memory");
    assert!(memory.contains("## 2026-05-20 — hook reminder writer"));
    assert!(memory.contains("Type: workflow"));
    assert!(memory.contains("Context: Hook reminders exposed memory but did not provide a writer."));
    assert!(
        memory.contains("Learning: A durable memory loop needs a supported gwt append command.")
    );
    assert!(memory
        .contains("Future Action: Use memory.add before reporting reusable learning as done."));
    // SPEC-3214 FR-007: no repo-local (git-tracked) memory file is created.
    assert!(
        !work_dir(repo.path()).join("memory.md").exists(),
        "memory.add must not create the repo-local .gwt/work/memory.md"
    );
}

#[test]
fn memory_add_imports_repo_local_work_file_and_keeps_source() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let work = work_dir(repo.path());
    fs::create_dir_all(&work).expect("create work dir");
    let repo_local = "# Memory\n\n## 2026-04-10 — repo-local entry\n\nType: lesson\nContext: old\nLearning: old\nFuture Action: old\n";
    fs::write(work.join("memory.md"), repo_local).expect("seed repo-local memory");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-30",
            None,
            "entry after home migration",
            "Repo-local memory should be imported into the home scratch once.",
            "The import preserves prior entries.",
            "Write ~/.gwt/projects/<hash>/work-notes/memory.md going forward.",
        ),
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory_path = home_memory_path(home.path()).expect("home memory file created");
    let memory = fs::read_to_string(memory_path).expect("read memory");
    assert!(
        memory.contains("repo-local entry"),
        "repo-local content should be imported into the home file"
    );
    assert!(memory.contains("## 2026-05-30 — entry after home migration"));
    // The repo-local file is git-tracked in target repositories; the import
    // must copy, not delete, to avoid dirtying the user's working tree.
    assert_eq!(
        fs::read_to_string(work.join("memory.md")).expect("read repo-local"),
        repo_local,
        "repo-local memory.md must be left intact (copy, not move)"
    );
}

#[test]
fn memory_add_migrates_legacy_tasks_memory_to_home() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Memory\n\n## 2026-03-01 — legacy tasks entry\n\nType: lesson\nContext: old\nLearning: old\nFuture Action: old\n";
    fs::write(tasks.join("memory.md"), legacy).expect("seed tasks memory");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-30",
            None,
            "entry after work-notes migration",
            "tasks/memory.md should move to the home work-notes once.",
            "The move preserves prior entries.",
            "Read the home work-notes going forward.",
        ),
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory_path = home_memory_path(home.path()).expect("home memory file created");
    let memory = fs::read_to_string(memory_path).expect("read memory");
    assert!(
        memory.contains("legacy tasks entry"),
        "prior tasks/memory.md content should be preserved via migration"
    );
    assert!(memory.contains("## 2026-05-30 — entry after work-notes migration"));
    assert!(
        !tasks.join("memory.md").exists(),
        "tasks/memory.md should be moved, not duplicated"
    );
}

#[test]
fn memory_add_migrates_legacy_lessons_when_memory_absent() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let tasks = repo.path().join("tasks");
    fs::create_dir_all(&tasks).expect("create tasks dir");
    let legacy = "# Lessons\n\n## 2026-03-15 — prior knowledge\n\nType: lesson\nContext: old context\nLearning: old learning\nFuture Action: old action\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-24",
            None,
            "new entry after migration",
            "Testing migration path.",
            "Legacy file should be imported.",
            "Verify migration is automatic.",
        ),
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let memory_path = home_memory_path(home.path()).expect("home memory file created");
    let memory = fs::read_to_string(memory_path).expect("read memory");
    assert!(
        memory.contains("prior knowledge"),
        "old entries should be preserved via migration"
    );
    assert!(memory.contains("## 2026-05-24 — new entry after migration"));
    assert!(
        !tasks.join("lessons.md").exists(),
        "lessons.md should not exist after migration"
    );
}

#[test]
fn memory_add_skips_migration_when_home_file_exists() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    // Seed an existing home memory file first, while no legacy source exists.
    let seed = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-23",
            None,
            "seed home entry",
            "Seed the home file.",
            "Home file exists.",
            "No migration afterwards.",
        ),
    );
    assert!(seed.status.success());
    let memory_path = home_memory_path(home.path()).expect("home memory file created");

    // Legacy sources appearing later must not be re-imported.
    let tasks = repo.path().join("tasks");
    let work = work_dir(repo.path());
    fs::create_dir_all(&tasks).expect("create tasks dir");
    fs::create_dir_all(&work).expect("create work dir");
    let legacy = "# Old Lessons\n";
    let legacy_tasks_memory = "# Legacy tasks memory\n";
    let repo_local = "# Repo-local memory\n";
    fs::write(tasks.join("lessons.md"), legacy).expect("seed lessons");
    fs::write(tasks.join("memory.md"), legacy_tasks_memory).expect("seed tasks memory");
    fs::write(work.join("memory.md"), repo_local).expect("seed repo-local memory");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-24",
            None,
            "no migration needed",
            "Home work-notes file already exists.",
            "Migration should be skipped.",
            "Only append to the home file.",
        ),
    );

    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(tasks.join("lessons.md")).expect("read legacy"),
        legacy,
        "lessons.md must not be modified when the home file already exists"
    );
    assert_eq!(
        fs::read_to_string(tasks.join("memory.md")).expect("read legacy tasks memory"),
        legacy_tasks_memory,
        "tasks/memory.md must not be moved when the home file already exists"
    );
    let memory = fs::read_to_string(&memory_path).expect("read memory");
    assert!(memory.contains("## 2026-05-24 — no migration needed"));
    assert!(
        !memory.contains("Repo-local memory"),
        "repo-local content must not be re-imported once the home file exists"
    );
}

#[test]
fn memory_add_rejects_empty_required_values_without_writing() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");

    let output = run_gwtd_json(
        repo.path(),
        home.path(),
        memory_add_payload(
            "2026-05-20",
            None,
            "missing context",
            "   ",
            "Learning",
            "Action",
        ),
    );

    assert!(!output.status.success(), "empty context should fail");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing required flag: context"),
        "stderr should explain validation failure, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        home_memory_path(home.path()).is_none(),
        "invalid input must not create a memory file"
    );
    assert!(
        !work_dir(repo.path()).join("memory.md").exists(),
        "invalid input must not create a repo-local memory file"
    );
}

/// SPEC-3214 acceptance scenario 3: the same notes are readable from two
/// different worktrees of one repository without any git merge.
#[test]
fn memory_added_from_one_worktree_is_visible_from_another() {
    let tmp = tempfile::tempdir().expect("tmp");
    let home = tempfile::tempdir().expect("home");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo");
    let git = |args: &[&str], cwd: &Path| {
        let output = hidden_command("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init"], &repo);
    git(&["config", "user.email", "gwt@example.invalid"], &repo);
    git(&["config", "user.name", "gwt"], &repo);
    git(&["commit", "--allow-empty", "-m", "seed"], &repo);
    git(
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/memory-sharing.git",
        ],
        &repo,
    );
    let linked = tmp.path().join("linked");
    git(
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
        &repo,
    );

    let output = run_gwtd_json(
        &repo,
        home.path(),
        memory_add_payload(
            "2026-07-03",
            None,
            "cross-worktree note",
            "Written from the main worktree.",
            "Notes are machine-local and branch-independent.",
            "Read from any worktree.",
        ),
    );
    assert!(
        output.status.success(),
        "memory add should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = run_gwtd_json(
        &linked,
        home.path(),
        memory_add_payload(
            "2026-07-03",
            None,
            "second worktree note",
            "Written from the linked worktree.",
            "Both worktrees share one home file.",
            "No git merge involved.",
        ),
    );
    assert!(
        output.status.success(),
        "memory add from linked worktree should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let memory_path = home_memory_path(home.path()).expect("one shared home memory file");
    let memory = fs::read_to_string(memory_path).expect("read memory");
    assert!(memory.contains("cross-worktree note"));
    assert!(memory.contains("second worktree note"));
}

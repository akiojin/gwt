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
        .env_remove(gwt_agent::GWT_SESSION_ID_ENV)
        .env_remove(gwt_agent::GWT_RECOVERY_ID_ENV)
        .env_remove(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
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

fn run_gwtd_json_for_recovery(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    payload: serde_json::Value,
) -> std::process::Output {
    let repo_id = gwt_core::paths::project_scope_hash(root).to_string();
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(
        home.join(".gwt").join("projects").join(repo_id),
    );
    let provider_context = store.load(recovery_id).ok().flatten().map(|record| {
        let provider_identity = record.provider_root.map(|root| root.root_id);
        (record.provider, provider_identity)
    });
    let (provider, provider_identity) =
        provider_context.unwrap_or_else(|| ("codex".to_string(), None));
    run_gwtd_json_for_recovery_with_provider_environment(
        root,
        home,
        recovery_id,
        session_id,
        &provider,
        provider_identity.as_deref(),
        payload,
    )
}

fn run_gwtd_json_for_recovery_with_provider_identity(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    provider_identity: Option<&str>,
    payload: serde_json::Value,
) -> std::process::Output {
    run_gwtd_json_for_recovery_with_provider_environment(
        root,
        home,
        recovery_id,
        session_id,
        "codex",
        provider_identity,
        payload,
    )
}

fn run_gwtd_json_for_recovery_with_provider_environment(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    provider: &str,
    provider_identity: Option<&str>,
    payload: serde_json::Value,
) -> std::process::Output {
    run_gwtd_json_for_recovery_with_provider_environment_and_permit(
        root,
        home,
        recovery_id,
        session_id,
        provider,
        provider_identity,
        None,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_gwtd_json_for_recovery_with_provider_environment_and_permit(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    provider: &str,
    provider_identity: Option<&str>,
    checkpoint_permit: Option<&str>,
    payload: serde_json::Value,
) -> std::process::Output {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    command
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env(gwt_agent::GWT_RECOVERY_ID_ENV, recovery_id)
        .env(gwt_agent::GWT_SESSION_ID_ENV, session_id)
        .env_remove(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_CODE_AGENT_ID")
        .env_remove("CLAUDE_CODE_AGENT_TYPE")
        .env_remove("CLAUDE_AGENT_ID")
        .env_remove("CLAUDE_AGENT_TYPE")
        .env_remove("CLAUDE_CODE_IS_SIDECHAIN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !provider.to_ascii_lowercase().contains("claude") {
        if let Some(provider_identity) = provider_identity {
            command.env("CODEX_THREAD_ID", provider_identity);
        }
    }
    if let Some(checkpoint_permit) = checkpoint_permit {
        command.env("GWT_INTAKE_CHECKPOINT_PERMIT", checkpoint_permit);
    } else {
        command.env_remove("GWT_INTAKE_CHECKPOINT_PERMIT");
    }
    let mut child = command.spawn().expect("run recovery-scoped gwtd");
    child
        .stdin
        .as_mut()
        .expect("gwtd stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write gwtd JSON");
    child.wait_with_output().expect("wait gwtd")
}

fn mint_claude_checkpoint_permit(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    operation: &str,
) -> String {
    let session = gwt_agent::Session::load(
        &home
            .join(".gwt")
            .join("sessions")
            .join(format!("{session_id}.toml")),
    )
    .expect("load managed Claude Session for hook provenance");
    let provider_session_id = session
        .agent_session_id
        .as_deref()
        .expect("managed Claude Session provider identity");
    let hook_input = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "session_id": provider_session_id,
        "tool_name": "Bash",
        "tool_input": {
            "command": format!("gwtd <<'JSON'\n{{\"operation\":\"{operation}\"}}\nJSON")
        }
    });
    let output = hidden_command(env!("CARGO_BIN_EXE_gwtd"))
        .args(["hook", "event", "PreToolUse"])
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env(gwt_agent::GWT_RECOVERY_ID_ENV, recovery_id)
        .env(gwt_agent::GWT_SESSION_ID_ENV, session_id)
        .env_remove(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove("GWT_INTAKE_CHECKPOINT_PERMIT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("hook stdin")
                .write_all(hook_input.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .expect("run root PreToolUse hook");
    assert!(
        output.status.success(),
        "root PreToolUse hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("updatedInput hook envelope");
    let command = envelope["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .expect("updated checkpoint command");
    command
        .strip_prefix("export GWT_INTAKE_CHECKPOINT_PERMIT=")
        .and_then(|tail| tail.split_once(';'))
        .map(|(token, _)| token.to_string())
        .expect("one-shot checkpoint permit")
}

fn run_gwtd_json_for_claude_recovery_with_hook_permit(
    root: &Path,
    home: &Path,
    recovery_id: &str,
    session_id: &str,
    operation: &str,
    payload: serde_json::Value,
) -> std::process::Output {
    let permit = mint_claude_checkpoint_permit(root, home, recovery_id, session_id, operation);
    run_gwtd_json_for_recovery_with_provider_environment_and_permit(
        root,
        home,
        recovery_id,
        session_id,
        "claude-code",
        None,
        Some(&permit),
        payload,
    )
}

fn run_gwtd_json_for_legacy_session(
    root: &Path,
    home: &Path,
    session_id: &str,
    provider_root_id: Option<&str>,
    payload: serde_json::Value,
) -> std::process::Output {
    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwtd"));
    command
        .current_dir(root)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CODEX_HOME", home.join(".codex"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env(gwt_agent::GWT_SESSION_ID_ENV, session_id)
        .env_remove(gwt_agent::GWT_RECOVERY_ID_ENV)
        .env_remove(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_URL_ENV)
        .env_remove(gwt_agent::GWT_HOOK_FORWARD_TOKEN_ENV)
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("CLAUDE_CODE_AGENT_ID")
        .env_remove("CLAUDE_CODE_AGENT_TYPE")
        .env_remove("CLAUDE_AGENT_ID")
        .env_remove("CLAUDE_AGENT_TYPE")
        .env_remove("CLAUDE_CODE_IS_SIDECHAIN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(provider_root_id) = provider_root_id {
        command.env("CODEX_THREAD_ID", provider_root_id);
    }
    let mut child = command.spawn().expect("run legacy session-scoped gwtd");
    child
        .stdin
        .as_mut()
        .expect("gwtd stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write gwtd JSON");
    child.wait_with_output().expect("wait gwtd")
}

fn parse_board_snapshot(
    output: &std::process::Output,
) -> gwt_core::coordination::CoordinationSnapshot {
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
            panic!(
                "parse gwtd response: {err}; stdout={}",
                String::from_utf8_lossy(&output.stdout)
            )
        });
    assert_eq!(
        envelope.get("ok").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let rendered = envelope
        .get("output")
        .and_then(serde_json::Value::as_str)
        .expect("board.show output");
    serde_json::from_str(rendered).expect("parse Board snapshot")
}

fn parse_intake_checkpoint_state(output: &std::process::Output) -> serde_json::Value {
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse checkpoint envelope");
    let rendered = envelope
        .get("output")
        .and_then(serde_json::Value::as_str)
        .expect("checkpoint current output");
    let state = rendered
        .split_once(" state=")
        .map(|(_, state)| state.trim())
        .expect("checkpoint current must expose mergeable state");
    serde_json::from_str(state).expect("parse checkpoint current state")
}

fn init_repo_with_origin(path: &Path) {
    assert!(hidden_command("git")
        .args(["init", "-q"])
        .arg(path)
        .status()
        .expect("git init")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(path)
        .args([
            "remote",
            "add",
            "origin",
            "https://github.com/example/intake-checkpoint-test.git",
        ])
        .status()
        .expect("git remote add")
        .success());
}

fn commit_repo_head(path: &Path) -> String {
    fs::write(path.join("README.md"), "legacy recovery fixture\n").expect("write fixture");
    assert!(hidden_command("git")
        .arg("-C")
        .arg(path)
        .args(["add", "README.md"])
        .status()
        .expect("git add")
        .success());
    assert!(hidden_command("git")
        .arg("-C")
        .arg(path)
        .args([
            "-c",
            "user.name=gwt test",
            "-c",
            "user.email=gwt@example.invalid",
            "commit",
            "-qm",
            "test: seed legacy intake",
        ])
        .status()
        .expect("git commit")
        .success());
    let output = hidden_command("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("HEAD utf-8")
        .trim()
        .to_string()
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
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse gwtd response");
    let stdout = response
        .get("output")
        .and_then(serde_json::Value::as_str)
        .expect("discussion.update output");
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

#[test]
fn discussion_update_projects_one_privacy_safe_intake_checkpoint_and_board_milestone() {
    use chrono::Utc;
    use gwt_core::recovery::{
        BindingQuality, CreateRecovery, ProviderRootBinding, RecoverySessionKind, RecoveryStore,
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let project_dir = home.join(".gwt").join("projects").join(&repo_id);
    let store = RecoveryStore::for_project_dir(&project_dir);
    let recovery_id = "recovery-discussion-projection";
    let session_id = "session-discussion-projection";
    let root_id = "root-discussion-projection";
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: session_id.to_string(),
                repo_id,
                session_kind: RecoverySessionKind::Intake,
                worktree_path: repo.clone(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "a".repeat(40),
                launch_head_oid: "a".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Investigate missing Intake durability".to_string(),
                created_at: Utc::now(),
            },
            "create-discussion-projection",
        )
        .expect("create recovery");
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: root_id.to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-discussion-projection",
        )
        .expect("bind root");
    store
        .record_root_input(
            recovery_id,
            root_id,
            "turn-privacy-safe",
            "private user input must not be copied into Board",
            "root-input-discussion-projection",
        )
        .expect("record root input");

    let payload = serde_json::json!({
        "schema_version": 1,
        "operation": "discussion.update",
        "params": {
            "date": "2026-07-16",
            "title": "Current Intake durability",
            "status": "active",
            "topics": ["intake", "recovery"],
            "related_specs": [3214],
            "summary": "Structured discussion milestones survive a crash.",
            "decisions": ["Project safe fields without copying the transcript."],
            "open_questions": ["How should legacy roots be confirmed?"],
            "next": "Verify Stop boundary enforcement.",
            "raw_transcript": "PRIVATE_TRANSCRIPT_SENTINEL",
            "bridge_visible_text": "PRIVATE_BRIDGE_SENTINEL"
        }
    });

    let denied = run_gwtd_json_for_recovery_with_provider_identity(
        &repo,
        &home,
        recovery_id,
        session_id,
        Some("child-discussion-projection"),
        payload.clone(),
    );
    assert!(
        !denied.status.success(),
        "a child provider root must fail closed"
    );
    assert_eq!(
        store
            .load(recovery_id)
            .unwrap()
            .unwrap()
            .checkpoint_revision,
        0
    );
    assert!(
        !project_dir.join("work-notes/discussions.md").exists(),
        "root authorization must happen before the work-notes projection"
    );

    let first = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload.clone());
    assert!(
        first.status.success(),
        "discussion projection failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_record = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(first_record.checkpoint_revision, 1);
    assert!(first_record.board_outbox.is_empty());
    assert_eq!(first_record.board_entry_ids.len(), 1);
    let checkpoint = first_record.checkpoint.as_ref().expect("checkpoint");
    assert_eq!(
        checkpoint.summary,
        "Structured discussion milestones survive a crash."
    );
    assert_eq!(
        checkpoint.as_of_turn_id.as_deref(),
        Some("turn-privacy-safe")
    );
    assert!(checkpoint.attachment_refs.is_empty());
    assert_eq!(checkpoint.visible_items.len(), 1);

    let serialized_record = serde_json::to_string(&first_record).expect("serialize recovery");
    let notes =
        fs::read_to_string(project_dir.join("work-notes/discussions.md")).expect("read work notes");
    assert!(!serialized_record.contains("PRIVATE_TRANSCRIPT_SENTINEL"));
    assert!(!serialized_record.contains("PRIVATE_BRIDGE_SENTINEL"));
    assert!(!notes.contains("PRIVATE_TRANSCRIPT_SENTINEL"));
    assert!(!notes.contains("PRIVATE_BRIDGE_SENTINEL"));

    let retry = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload.clone());
    assert!(retry.status.success());
    assert_eq!(
        store
            .load(recovery_id)
            .unwrap()
            .unwrap()
            .checkpoint_revision,
        1
    );

    let supplemental = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.update",
            "params": {
                "expected_revision": 1,
                "title": "Current Intake durability",
                "summary": "Structured discussion milestones survive a crash.",
                "related_specs": [3214],
                "decisions": ["Project safe fields without copying the transcript."],
                "open_questions": ["How should legacy roots be confirmed?"],
                "next": "Verify Stop boundary enforcement.",
                "visible_items": [{
                    "role": "assistant",
                    "kind": "plan",
                    "text": "Completed public plan only.",
                    "partial": false
                }]
            }
        }),
    );
    assert!(
        supplemental.status.success(),
        "supplemental explicit checkpoint failed: {}",
        String::from_utf8_lossy(&supplemental.stderr)
    );
    let supplemented = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(supplemented.checkpoint_revision, 2);
    assert_eq!(supplemented.board_entry_ids.len(), 1);
    assert_eq!(
        supplemented.checkpoint.as_ref().unwrap().visible_items[0].text,
        "Completed public plan only."
    );

    let post_supplement_retry =
        run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload);
    assert!(post_supplement_retry.status.success());
    let converged = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(converged.checkpoint_revision, 2);
    assert_eq!(converged.board_entry_ids.len(), 1);
    assert_eq!(
        converged.checkpoint.as_ref().unwrap().visible_items[0].text,
        "Completed public plan only.",
        "automatic structured retries must preserve explicit visible items"
    );

    let board = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "board.show",
            "params": { "all": true }
        }),
    );
    let snapshot = parse_board_snapshot(&board);
    assert_eq!(snapshot.board.entries.len(), 1);
    let board_entry = &snapshot.board.entries[0];
    assert!(board_entry.body.contains("Status: active"));
    assert!(board_entry
        .body
        .contains("Structured discussion milestones survive a crash."));
    assert!(!board_entry.body.contains("private user input"));
    assert!(!board_entry.body.contains("PRIVATE_TRANSCRIPT_SENTINEL"));
    assert!(!board_entry.body.contains("PRIVATE_BRIDGE_SENTINEL"));
}

#[test]
fn discussion_update_resolves_current_legacy_intake_without_recovery_environment() {
    use gwt_agent::{AgentId, AgentStatus, Session};

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let head = commit_repo_head(&repo);
    let session_id = "legacy-current-intake";
    let provider_root_id = "legacy-current-root";
    let sessions_dir = home.join(".gwt").join("sessions");
    let mut session = Session::new(&repo, "", AgentId::Codex);
    session.id = session_id.to_string();
    session.status = AgentStatus::Interrupted;
    session.session_kind = Some(gwt_skills::SessionKind::Intake);
    session.is_ephemeral = true;
    session.launch_base_oid = Some(head);
    session.agent_session_id = Some(provider_root_id.to_string());
    session.recovery_id = None;
    session.recovery_launch_stage = None;
    session.provider_root_role = None;
    session.provider_binding_quality = None;
    session
        .save(&sessions_dir)
        .expect("save legacy Session ledger");

    let output = run_gwtd_json_for_legacy_session(
        &repo,
        &home,
        session_id,
        Some(provider_root_id),
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "title": "Legacy current Intake",
                "status": "completed",
                "topics": ["intake", "migration"],
                "summary": "The exact Session ledger selects the current Intake.",
                "decisions": ["Import metadata only and bind the recorded root."],
                "open_questions": [],
                "next": "Continue from the durable checkpoint."
            }
        }),
    );
    assert!(
        output.status.success(),
        "legacy current Intake projection failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let synced = Session::load(&sessions_dir.join(format!("{session_id}.toml")))
        .expect("reload synced Session ledger");
    let recovery_id = synced.recovery_id.expect("legacy recovery id");
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(
        home.join(".gwt").join("projects").join(repo_id),
    );
    let record = store
        .load(&recovery_id)
        .expect("load imported recovery")
        .expect("imported recovery");
    assert_eq!(record.session_id, session_id);
    assert_eq!(record.checkpoint_revision, 1);
    assert_eq!(
        record
            .provider_root
            .as_ref()
            .map(|root| root.root_id.as_str()),
        Some(provider_root_id)
    );
    assert_eq!(record.board_entry_ids.len(), 1);
    let serialized = serde_json::to_string(&record).expect("serialize imported recovery");
    assert!(!serialized.contains("private"));
}

#[test]
fn discussion_update_keeps_ambiguous_legacy_intake_in_attention_without_private_backfill() {
    use gwt_agent::{AgentId, AgentStatus, Session};

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let head = commit_repo_head(&repo);
    let session_id = "legacy-attention-intake";
    let sessions_dir = home.join(".gwt").join("sessions");
    let mut session = Session::new(&repo, "", AgentId::Codex);
    session.id = session_id.to_string();
    session.status = AgentStatus::Interrupted;
    session.session_kind = Some(gwt_skills::SessionKind::Intake);
    session.is_ephemeral = true;
    session.launch_base_oid = Some(head);
    session.agent_session_id = None;
    session.session_history.clear();
    session.recovery_id = None;
    session.recovery_launch_stage = None;
    session.provider_root_role = None;
    session.provider_binding_quality = None;
    session
        .save(&sessions_dir)
        .expect("save ambiguous legacy Session");

    let output = run_gwtd_json_for_legacy_session(
        &repo,
        &home,
        session_id,
        None,
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "title": "Must not guess",
                "status": "active",
                "summary": "PRIVATE_BACKFILL_SENTINEL",
                "next": "Wait for explicit root confirmation."
            }
        }),
    );
    assert!(
        !output.status.success(),
        "missing root evidence must fail closed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Recovery Center"), "{stderr}");
    assert!(stderr.contains("Attention"), "{stderr}");

    let synced = Session::load(&sessions_dir.join(format!("{session_id}.toml")))
        .expect("reload attention Session");
    let recovery_id = synced.recovery_id.expect("attention recovery id");
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let store = gwt_core::recovery::RecoveryStore::for_project_dir(
        home.join(".gwt").join("projects").join(repo_id),
    );
    let record = store
        .load(&recovery_id)
        .expect("load attention recovery")
        .expect("attention recovery");
    assert_eq!(record.checkpoint_revision, 0);
    assert!(record.checkpoint.is_none());
    assert!(record.board_outbox.is_empty());
    assert!(record.board_entry_ids.is_empty());
    assert!(!serde_json::to_string(&record)
        .expect("serialize attention recovery")
        .contains("PRIVATE_BACKFILL_SENTINEL"));
    assert!(
        !home
            .join(".gwt")
            .join("projects")
            .join(gwt_core::paths::project_scope_hash(&repo).as_str())
            .join("work-notes/discussions.md")
            .exists(),
        "failed root resolution must not write the discussion memo"
    );
}

#[test]
fn intake_checkpoint_update_enforces_session_cas_copies_attachments_and_acks_board() {
    use chrono::Utc;
    use gwt_core::recovery::{
        BindingQuality, CreateRecovery, ProviderRootBinding, RecoverySessionKind, RecoveryStore,
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let project_dir = home.join(".gwt").join("projects").join(&repo_id);
    let store = RecoveryStore::for_project_dir(&project_dir);
    let recovery_id = "recovery-cli-cas";
    let session_id = "session-cli-cas";
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: session_id.to_string(),
                repo_id,
                session_kind: RecoverySessionKind::Intake,
                worktree_path: repo.clone(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "1".repeat(40),
                launch_head_oid: "1".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Design crash-safe Intake recovery".to_string(),
                created_at: Utc::now(),
            },
            "create-cli-cas",
        )
        .expect("create recovery");
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: "root-cli-cas".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-cli-cas",
        )
        .expect("bind root");
    let drop_files = repo.join(".gwt").join("drop-files");
    fs::create_dir_all(&drop_files).expect("drop files");
    let attachment = drop_files.join("board-gap.png");
    fs::write(&attachment, b"screenshot evidence").expect("attachment");

    let current = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        }),
    );
    assert!(current.status.success());
    let current_stdout = String::from_utf8_lossy(&current.stdout);
    assert!(current_stdout.contains("revision=0"), "{current_stdout}");
    assert!(current_stdout.contains(recovery_id), "{current_stdout}");
    let initial_state = parse_intake_checkpoint_state(&current);
    assert_eq!(initial_state["revision"], 0);
    assert!(initial_state["checkpoint"].is_null());

    let denied_current = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        "session-child-or-other",
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        }),
    );
    assert!(!denied_current.status.success());
    assert!(String::from_utf8_lossy(&denied_current.stderr).contains("belongs to session"));

    let child_provider_current = run_gwtd_json_for_recovery_with_provider_identity(
        &repo,
        &home,
        recovery_id,
        session_id,
        Some("child-provider-root"),
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        }),
    );
    assert!(!child_provider_current.status.success());
    assert!(
        String::from_utf8_lossy(&child_provider_current.stderr)
            .contains("not caller provider identity"),
        "{}",
        String::from_utf8_lossy(&child_provider_current.stderr)
    );

    let unknown_provider_current = run_gwtd_json_for_recovery_with_provider_identity(
        &repo,
        &home,
        recovery_id,
        session_id,
        None,
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        }),
    );
    assert!(!unknown_provider_current.status.success());
    assert!(
        String::from_utf8_lossy(&unknown_provider_current.stderr).contains("CODEX_THREAD_ID"),
        "{}",
        String::from_utf8_lossy(&unknown_provider_current.stderr)
    );

    let payload = serde_json::json!({
        "schema_version": 1,
        "operation": "intake.checkpoint.update",
        "params": {
            "expected_revision": 0,
            "title": "Unified recovery",
            "summary": "Exact resume and semantic fallback are both required.",
            "next": "Implement and verify the recovery lifecycle.",
            "related_specs": [1921, 3214],
            "decisions": ["Recovery Center shows every candidate."],
            "open_questions": [],
            "visible_items": [{
                "role": "assistant",
                "kind": "plan",
                "text": "Implement the approved recovery plan.",
                "partial": false
            }],
            "attachment_paths": [".gwt/drop-files/board-gap.png"]
        }
    });
    let denied = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        "session-child-or-other",
        payload.clone(),
    );
    assert!(
        !denied.status.success(),
        "another session must not replace the checkpoint"
    );
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("belongs to session"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert_eq!(
        store
            .load(recovery_id)
            .unwrap()
            .unwrap()
            .checkpoint_revision,
        0
    );

    let output = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload.clone());
    assert!(
        output.status.success(),
        "checkpoint update failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(record.checkpoint_revision, 1);
    assert!(record.board_outbox.is_empty());
    assert_eq!(record.board_entry_ids.len(), 1);
    let copied = &record.checkpoint.as_ref().unwrap().attachment_refs[0];
    assert_eq!(copied.file_name, "board-gap.png");
    assert_eq!(
        store
            .read_attachment_bytes(copied, gwt_core::recovery::MAX_RECOVERY_ATTACHMENT_BYTES)
            .unwrap(),
        b"screenshot evidence"
    );

    let retry = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload.clone());
    assert!(
        retry.status.success(),
        "exact checkpoint retry failed: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let after_retry = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(after_retry.checkpoint_revision, 1);
    assert_eq!(after_retry.board_entry_ids.len(), 1);
    assert_eq!(
        after_retry
            .checkpoint
            .as_ref()
            .expect("checkpoint after retry")
            .attachment_refs
            .len(),
        1
    );

    let mut stale_payload = payload;
    stale_payload["params"]["summary"] =
        serde_json::Value::String("A conflicting stale replacement.".to_string());
    let stale = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, stale_payload);
    assert!(!stale.status.success(), "stale CAS must fail");
    assert!(
        String::from_utf8_lossy(&stale.stderr).contains("revision mismatch"),
        "{}",
        String::from_utf8_lossy(&stale.stderr)
    );
    assert_eq!(
        store
            .load(recovery_id)
            .unwrap()
            .unwrap()
            .checkpoint_revision,
        1
    );

    let board = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "board.show",
            "params": { "all": true }
        }),
    );
    assert!(
        board.status.success(),
        "Board snapshot failed: {}",
        String::from_utf8_lossy(&board.stderr)
    );
    let snapshot = parse_board_snapshot(&board);
    assert_eq!(snapshot.board.entries.len(), 1);
    let entry = &snapshot.board.entries[0];
    assert_eq!(entry.id, record.board_entry_ids[0]);
    assert_eq!(
        entry.title.as_deref(),
        Some("Intake checkpoint: Unified recovery")
    );
    assert!(entry
        .body
        .contains("Exact resume and semantic fallback are both required."));
    assert_eq!(entry.origin_session_kind.as_deref(), Some("intake"));
    assert_eq!(entry.origin_recovery_id.as_deref(), Some(recovery_id));
    assert_eq!(entry.origin_session_id.as_deref(), Some(session_id));
    assert_eq!(entry.origin_branch, None);

    let current_after_update = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        }),
    );
    assert!(current_after_update.status.success());
    let mergeable_state = parse_intake_checkpoint_state(&current_after_update);
    assert_eq!(mergeable_state["revision"], 1);
    assert_eq!(
        mergeable_state["checkpoint"]["summary"],
        "Exact resume and semantic fallback are both required."
    );
    assert_eq!(
        mergeable_state["checkpoint"]["confirmed_decisions"][0],
        "Recovery Center shows every candidate."
    );
    let retained_refs = mergeable_state["checkpoint"]["attachment_refs"].clone();
    assert_eq!(retained_refs.as_array().map(Vec::len), Some(1));

    let replacement = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.update",
            "params": {
                "expected_revision": 1,
                "title": "Unified recovery follow-up",
                "summary": "The next complete checkpoint retains required evidence.",
                "next": "Continue implementation.",
                "related_specs": [1921, 3214],
                "decisions": ["Retain the existing screenshot by content id."],
                "open_questions": [],
                "retained_attachment_refs": retained_refs
            }
        }),
    );
    assert!(
        replacement.status.success(),
        "retained attachment update failed: {}",
        String::from_utf8_lossy(&replacement.stderr)
    );
    let after_replacement = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(after_replacement.checkpoint_revision, 2);
    assert_eq!(
        after_replacement
            .checkpoint
            .as_ref()
            .expect("replacement checkpoint")
            .attachment_refs,
        record
            .checkpoint
            .as_ref()
            .expect("first checkpoint")
            .attachment_refs
    );
}

#[test]
fn intake_checkpoint_accepts_claude_root_and_rejects_unknown_or_subagent_store_roles() {
    use chrono::Utc;
    use gwt_agent::{AgentId, Session};
    use gwt_core::recovery::{
        BindingQuality, CreateRecovery, ProviderRootBinding, ProviderRootRole, RecoveryLaunchStage,
        RecoverySessionKind, RecoveryStore,
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let project_dir = home.join(".gwt").join("projects").join(&repo_id);
    let store = RecoveryStore::for_project_dir(&project_dir);
    let sessions_dir = home.join(".gwt").join("sessions");
    let create = |recovery_id: &str, session_id: &str| {
        store
            .create(
                CreateRecovery {
                    recovery_id: recovery_id.to_string(),
                    session_id: session_id.to_string(),
                    repo_id: repo_id.clone(),
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: repo.clone(),
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "6".repeat(40),
                    launch_head_oid: "6".repeat(40),
                    provider: "claude-code".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Prove root-only Claude checkpoint authority".to_string(),
                    created_at: Utc::now(),
                },
                format!("create-{recovery_id}"),
            )
            .expect("create recovery");
        let mut session = Session::new(&repo, "intake/root", AgentId::ClaudeCode);
        session.id = session_id.to_string();
        session.recovery_id = Some(recovery_id.to_string());
        session.session_kind = Some(gwt_skills::SessionKind::Intake);
        session.is_ephemeral = true;
        session.agent_session_id = Some(format!("provider-{session_id}"));
        session.save(&sessions_dir).expect("save Claude Session");
    };
    let current_payload = || {
        serde_json::json!({
            "schema_version": 1,
            "operation": "intake.checkpoint.current",
            "params": {}
        })
    };

    let root_recovery = "recovery-claude-root";
    let root_session = "session-claude-root";
    create(root_recovery, root_session);
    store
        .bind_root(
            root_recovery,
            ProviderRootBinding {
                root_id: format!("provider-{root_session}"),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-claude-root",
        )
        .expect("bind Claude root");
    let missing_permit =
        run_gwtd_json_for_recovery(&repo, &home, root_recovery, root_session, current_payload());
    assert!(!missing_permit.status.success());
    assert!(String::from_utf8_lossy(&missing_permit.stderr).contains("authorization is missing"));
    let root = run_gwtd_json_for_claude_recovery_with_hook_permit(
        &repo,
        &home,
        root_recovery,
        root_session,
        "intake.checkpoint.current",
        current_payload(),
    );
    assert!(
        root.status.success(),
        "Claude root checkpoint read failed: {}",
        String::from_utf8_lossy(&root.stderr)
    );

    for (recovery_id, session_id, role) in [
        (
            "recovery-claude-unknown",
            "session-claude-unknown",
            ProviderRootRole::Unknown,
        ),
        (
            "recovery-claude-subagent",
            "session-claude-subagent",
            ProviderRootRole::Subagent,
        ),
    ] {
        create(recovery_id, session_id);
        if role == ProviderRootRole::Subagent {
            store
                .advance_launch_stage(
                    recovery_id,
                    RecoveryLaunchStage::WorktreeMaterialized,
                    Some(role),
                    format!("observe-{recovery_id}"),
                )
                .expect("record subagent role");
        }
        let denied =
            run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, current_payload());
        assert!(!denied.status.success(), "role {role:?} must fail closed");
        let stderr = String::from_utf8_lossy(&denied.stderr);
        assert!(stderr.contains("rejected caller role"), "{stderr}");
        assert!(stderr.contains(&format!("{role:?}")), "{stderr}");
    }
}

#[test]
fn intake_checkpoint_retry_flushes_a_failed_board_post_exactly_once() {
    use chrono::Utc;
    use gwt_core::recovery::{
        BindingQuality, CreateRecovery, ProviderRootBinding, RecoverySessionKind, RecoveryStore,
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let project_dir = home.join(".gwt").join("projects").join(&repo_id);
    let store = RecoveryStore::for_project_dir(&project_dir);
    let recovery_id = "recovery-cli-board-retry";
    let session_id = "session-cli-board-retry";
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: session_id.to_string(),
                repo_id,
                session_kind: RecoverySessionKind::Intake,
                worktree_path: repo.clone(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "2".repeat(40),
                launch_head_oid: "2".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Prove Board outbox retry durability".to_string(),
                created_at: Utc::now(),
            },
            "create-cli-board-retry",
        )
        .expect("create recovery");
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: "root-cli-board-retry".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-cli-board-retry",
        )
        .expect("bind root");

    let payload = serde_json::json!({
        "schema_version": 1,
        "operation": "intake.checkpoint.update",
        "params": {
            "expected_revision": 0,
            "title": "Durable Board retry",
            "summary": "The checkpoint survives while Board storage is unavailable.",
            "next": "Replay the pending milestone.",
            "decisions": ["Retry the same semantic checkpoint without duplication."],
            "open_questions": []
        }
    });

    let coordination_path = project_dir.join("coordination");
    fs::write(&coordination_path, b"block Board directory creation")
        .expect("create Board storage obstruction");
    let first = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload.clone());
    assert!(
        first.status.success(),
        "Board delivery must not roll back a durable semantic checkpoint: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    assert!(
        first_stdout.contains("board_pending=true"),
        "{first_stdout}"
    );
    assert!(first_stdout.contains("delivery_error="), "{first_stdout}");
    let pending = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(pending.checkpoint_revision, 1);
    assert_eq!(pending.board_outbox.len(), 1);
    assert!(pending.board_entry_ids.is_empty());
    assert!(pending.board_delivery_error.is_some());
    assert!(
        pending
            .board_delivery_error
            .as_deref()
            .unwrap()
            .chars()
            .count()
            <= gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS
    );

    fs::remove_file(&coordination_path).expect("remove Board storage obstruction");
    let retry = run_gwtd_json_for_recovery(&repo, &home, recovery_id, session_id, payload);
    assert!(
        retry.status.success(),
        "same-content retry failed: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(String::from_utf8_lossy(&retry.stdout).contains("revision 1"));

    let recovered = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(recovered.checkpoint_revision, 1);
    assert!(recovered.board_outbox.is_empty());
    assert_eq!(recovered.board_entry_ids.len(), 1);
    assert_eq!(recovered.board_delivery_error, None);

    let board = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "board.show",
            "params": { "all": true }
        }),
    );
    assert!(board.status.success());
    let snapshot = parse_board_snapshot(&board);
    assert_eq!(snapshot.board.entries.len(), 1);
    let entry = &snapshot.board.entries[0];
    assert_eq!(entry.id, recovered.board_entry_ids[0]);
    assert_eq!(
        entry.title.as_deref(),
        Some("Intake checkpoint: Durable Board retry")
    );
    assert!(entry
        .body
        .contains("The checkpoint survives while Board storage is unavailable."));
    assert_eq!(entry.origin_session_kind.as_deref(), Some("intake"));
    assert_eq!(entry.origin_recovery_id.as_deref(), Some(recovery_id));
    assert_eq!(entry.origin_branch, None);
}

#[test]
fn recovery_board_outbox_replay_converges_after_post_before_ack_crash() {
    use chrono::Utc;
    use gwt_core::{
        coordination::{
            load_snapshot, post_entry_idempotent, AuthorKind, BoardEntryDraft, BoardEntryKind,
            BoardOrigin,
        },
        recovery::{
            BindingQuality, BoardMilestoneIntent, CreateRecovery, ProviderRootBinding,
            RecoverySessionKind, RecoveryStore, SemanticCheckpoint,
        },
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    let store = RecoveryStore::new(temp.path().join("recovery-store"));
    let recovery_id = "recovery-post-before-ack";
    let entry_id = "intake-post-before-ack";
    let title = "Intake checkpoint: Post before ack";
    let body = "The Board append survived but its outbox acknowledgement did not.";
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: "session-post-before-ack".to_string(),
                repo_id: "repo-post-before-ack".to_string(),
                session_kind: RecoverySessionKind::Intake,
                worktree_path: repo.clone(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "3".repeat(40),
                launch_head_oid: "3".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Prove the post-before-ack boundary".to_string(),
                created_at: Utc::now(),
            },
            "create-post-before-ack",
        )
        .expect("create recovery");
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: "root-post-before-ack".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-post-before-ack",
        )
        .expect("bind recovery");
    store
        .replace_checkpoint(
            recovery_id,
            "root-post-before-ack",
            0,
            SemanticCheckpoint {
                summary: "Post survived".to_string(),
                board_intents: vec![BoardMilestoneIntent {
                    entry_id: entry_id.to_string(),
                    title: title.to_string(),
                    body: body.to_string(),
                    queued_at: Utc::now(),
                }],
                ..Default::default()
            },
            "checkpoint-post-before-ack",
        )
        .expect("queue Board intent");

    let mut draft = BoardEntryDraft::new(
        AuthorKind::Agent,
        "gwt-discussion",
        BoardEntryKind::Status,
        body,
    );
    draft.title = Some(title.to_string());
    draft.origin = BoardOrigin::new("", "session-post-before-ack", "codex")
        .with_session_kind("intake")
        .with_recovery_id(recovery_id);
    let mut entry = draft.finalize().expect("finalize Board entry");
    entry.id = entry_id.to_string();
    post_entry_idempotent(&repo, entry).expect("simulate durable Board append");

    let crashed = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(
        crashed.board_outbox.len(),
        1,
        "ack did not happen before crash"
    );
    assert!(crashed.board_entry_ids.is_empty());
    assert_eq!(
        load_snapshot(&repo)
            .unwrap()
            .board
            .entries
            .iter()
            .filter(|entry| entry.id == entry_id)
            .count(),
        1
    );

    gwt::cli::flush_recovery_board_outbox(&repo, &store, recovery_id)
        .expect("restart replay must idempotently post then ack");

    let replayed = store.load(recovery_id).unwrap().unwrap();
    assert!(replayed.board_outbox.is_empty());
    assert_eq!(replayed.board_entry_ids, [entry_id]);
    assert_eq!(
        load_snapshot(&repo)
            .unwrap()
            .board
            .entries
            .iter()
            .filter(|entry| entry.id == entry_id)
            .count(),
        1,
        "post-before-ack replay must converge to one durable Board entry"
    );
}

#[test]
fn recovery_board_outbox_fails_closed_for_remote_providers_without_caller_id_idempotency() {
    use chrono::Utc;
    use gwt_config::{BoardProviderKind, ProjectBoardConfig};
    use gwt_core::recovery::{
        BindingQuality, BoardMilestoneIntent, CreateRecovery, ProviderRootBinding,
        RecoverySessionKind, RecoveryStore, SemanticCheckpoint,
    };

    for (provider, provider_name) in [
        (BoardProviderKind::Slack, "slack"),
        (BoardProviderKind::Teams, "teams"),
    ] {
        let temp = tempfile::tempdir().expect("temp");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("repo");
        ProjectBoardConfig {
            provider: Some(provider),
            channel: Some("test-channel".to_string()),
            ..Default::default()
        }
        .save_to_work_dir(&gwt_core::paths::gwt_repo_local_work_dir(&repo))
        .expect("select remote provider");
        let store = RecoveryStore::new(temp.path().join("recovery-store"));
        let recovery_id = format!("recovery-remote-{provider_name}");
        store
            .create(
                CreateRecovery {
                    recovery_id: recovery_id.clone(),
                    session_id: format!("session-remote-{provider_name}"),
                    repo_id: format!("repo-remote-{provider_name}"),
                    session_kind: RecoverySessionKind::Intake,
                    worktree_path: repo.clone(),
                    launch_base_ref: Some("origin/develop".to_string()),
                    launch_base_oid: "4".repeat(40),
                    launch_head_oid: "4".repeat(40),
                    provider: "codex".to_string(),
                    model: None,
                    runtime: "host".to_string(),
                    initial_prompt: "Keep remote Board delivery pending".to_string(),
                    created_at: Utc::now(),
                },
                format!("create-remote-{provider_name}"),
            )
            .expect("create remote recovery");
        store
            .bind_root(
                &recovery_id,
                ProviderRootBinding {
                    root_id: format!("root-remote-{provider_name}"),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                format!("bind-remote-{provider_name}"),
            )
            .expect("bind remote recovery");
        store
            .replace_checkpoint(
                &recovery_id,
                &format!("root-remote-{provider_name}"),
                0,
                SemanticCheckpoint {
                    board_intents: vec![BoardMilestoneIntent {
                        entry_id: format!("entry-remote-{provider_name}"),
                        title: "Intake checkpoint: Pending remote delivery".to_string(),
                        body: "Do not acknowledge a local-only append.".to_string(),
                        queued_at: Utc::now(),
                    }],
                    ..Default::default()
                },
                format!("checkpoint-remote-{provider_name}"),
            )
            .expect("queue remote Board intent");

        let error = gwt::cli::flush_recovery_board_outbox(&repo, &store, &recovery_id)
            .expect_err("remote delivery without caller-ID idempotency must fail closed");
        assert!(error.contains(provider_name), "{error}");
        assert!(error.contains("pending"), "{error}");
        let retained = store.load(&recovery_id).unwrap().unwrap();
        assert_eq!(retained.board_outbox.len(), 1);
        assert!(retained.board_entry_ids.is_empty());
        assert!(retained
            .board_delivery_error
            .as_deref()
            .is_some_and(|error| error.contains(provider_name)));
        assert!(
            !gwt_core::coordination::coordination_dir(&repo).exists(),
            "remote selection must not silently append to the local Board"
        );
    }
}

#[test]
fn remote_board_delivery_is_non_blocking_and_preserves_multiple_checkpoint_intents() {
    use chrono::Utc;
    use gwt_config::{BoardProviderKind, ProjectBoardConfig};
    use gwt_core::recovery::{
        BindingQuality, CreateRecovery, ProviderRootBinding, RecoverySessionKind, RecoveryStore,
    };

    let temp = tempfile::tempdir().expect("temp");
    let repo = temp.path().join("repo");
    let home = temp.path().join("home");
    fs::create_dir_all(&repo).expect("repo dir");
    fs::create_dir_all(&home).expect("home dir");
    init_repo_with_origin(&repo);
    ProjectBoardConfig {
        provider: Some(BoardProviderKind::Slack),
        channel: Some("checkpoint-channel".to_string()),
        ..Default::default()
    }
    .save_to_work_dir(&gwt_core::paths::gwt_repo_local_work_dir(&repo))
    .expect("select Slack Board");

    let repo_id = gwt_core::paths::project_scope_hash(&repo).to_string();
    let project_dir = home.join(".gwt").join("projects").join(&repo_id);
    let store = RecoveryStore::for_project_dir(&project_dir);
    let recovery_id = "recovery-remote-checkpoints";
    let session_id = "session-remote-checkpoints";
    store
        .create(
            CreateRecovery {
                recovery_id: recovery_id.to_string(),
                session_id: session_id.to_string(),
                repo_id,
                session_kind: RecoverySessionKind::Intake,
                worktree_path: repo.clone(),
                launch_base_ref: Some("origin/develop".to_string()),
                launch_base_oid: "5".repeat(40),
                launch_head_oid: "5".repeat(40),
                provider: "codex".to_string(),
                model: None,
                runtime: "host".to_string(),
                initial_prompt: "Keep checkpoints moving during Board outage".to_string(),
                created_at: Utc::now(),
            },
            "create-remote-checkpoints",
        )
        .expect("create recovery");
    store
        .bind_root(
            recovery_id,
            ProviderRootBinding {
                root_id: "root-remote-checkpoints".to_string(),
                session_tree_id: None,
                quality: BindingQuality::Verified,
                bound_at: Utc::now(),
            },
            "bind-remote-checkpoints",
        )
        .expect("bind recovery");

    for revision in 0..2 {
        let output = run_gwtd_json_for_recovery(
            &repo,
            &home,
            recovery_id,
            session_id,
            serde_json::json!({
                "schema_version": 1,
                "operation": "intake.checkpoint.update",
                "params": {
                    "expected_revision": revision,
                    "title": format!("Remote checkpoint {}", revision + 1),
                    "summary": format!("Semantic checkpoint {} remains durable.", revision + 1),
                    "next": format!("Continue from checkpoint {}.", revision + 1),
                    "decisions": ["Board delivery is non-blocking."],
                    "open_questions": []
                }
            }),
        );
        assert!(
            output.status.success(),
            "revision {} must commit while remote delivery is pending: {}",
            revision + 1,
            String::from_utf8_lossy(&output.stderr)
        );
        let envelope: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse checkpoint response");
        let rendered = envelope["output"].as_str().expect("checkpoint output");
        assert!(rendered.contains(&format!("revision {}", revision + 1)));
        assert!(rendered.contains("board_pending=true"));
        assert!(rendered.contains(&format!("board_pending_count={}", revision + 1)));
        assert!(rendered.contains("delivery_error="));
    }

    let automatic = run_gwtd_json_for_recovery(
        &repo,
        &home,
        recovery_id,
        session_id,
        serde_json::json!({
            "schema_version": 1,
            "operation": "discussion.update",
            "params": {
                "title": "Remote structured checkpoint 3",
                "status": "active",
                "topics": ["intake"],
                "summary": "Automatic discussion durability also survives remote Board outage.",
                "decisions": ["Keep the deterministic remote intent pending."],
                "open_questions": [],
                "next": "Continue without falsely acknowledging Board."
            }
        }),
    );
    assert!(
        automatic.status.success(),
        "automatic discussion checkpoint must commit while remote Board is pending: {}",
        String::from_utf8_lossy(&automatic.stderr)
    );
    let rendered: serde_json::Value =
        serde_json::from_slice(&automatic.stdout).expect("parse automatic response");
    let rendered = rendered["output"].as_str().expect("automatic output");
    assert!(rendered.contains("revision 3"), "{rendered}");
    assert!(rendered.contains("board_pending_count=3"), "{rendered}");
    assert!(rendered.contains("delivery_error="), "{rendered}");

    let record = store.load(recovery_id).unwrap().unwrap();
    assert_eq!(record.checkpoint_revision, 3);
    assert_eq!(record.board_outbox.len(), 3);
    assert!(record.board_entry_ids.is_empty());
    let delivery_error = record
        .board_delivery_error
        .as_deref()
        .expect("bounded remote delivery diagnostic");
    assert!(delivery_error.contains("slack"));
    assert!(delivery_error.contains("pending"));
    assert!(delivery_error.chars().count() <= gwt_core::recovery::MAX_BOARD_DELIVERY_ERROR_CHARS);
    assert!(
        !project_dir.join("coordination").exists(),
        "remote delivery must never acknowledge a local-only Board append"
    );
}

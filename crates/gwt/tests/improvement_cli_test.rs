use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct Fixture {
    home: TempDir,
    project: TempDir,
}

fn fixture() -> Fixture {
    Fixture {
        home: tempfile::tempdir().expect("home"),
        project: tempfile::tempdir().expect("project"),
    }
}

fn run_gwtd_json(fixture: &Fixture, payload: Value) -> Value {
    run_gwtd_json_in(fixture.home.path(), fixture.project.path(), payload)
}

fn run_gwtd_json_in(home: &Path, project: &Path, payload: Value) -> Value {
    let output = run_gwtd_json_raw_in(home, project, payload);
    assert!(
        output.status.success(),
        "gwtd should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "parse gwtd response: {err}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn run_gwtd_json_raw(fixture: &Fixture, payload: Value) -> std::process::Output {
    run_gwtd_json_raw_in(fixture.home.path(), fixture.project.path(), payload)
}

fn run_gwtd_json_raw_in(home: &Path, project: &Path, payload: Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gwtd"))
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run gwtd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.to_string().as_bytes())
        .expect("write JSON");
    child.wait_with_output().expect("wait gwtd")
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository_with_linked_worktree(root: &Path, name: &str, remote: &str) -> (PathBuf, PathBuf) {
    let repository = root.join(name);
    let worktree = root.join(format!("{name}-worktree"));
    fs::create_dir_all(&repository).expect("repository directory");
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.email", "test@example.com"]);
    run_git(&repository, &["config", "user.name", "Test User"]);
    run_git(&repository, &["remote", "add", "origin", remote]);
    fs::write(repository.join("README.md"), "fixture\n").expect("fixture readme");
    run_git(&repository, &["add", "README.md"]);
    run_git(&repository, &["commit", "-m", "test: initialize fixture"]);
    run_git(
        &repository,
        &[
            "worktree",
            "add",
            "--detach",
            worktree.to_str().expect("worktree path"),
            "HEAD",
        ],
    );
    (repository, worktree)
}

fn operation_output(response: &Value) -> Value {
    assert_eq!(
        response.get("ok").and_then(Value::as_bool),
        Some(true),
        "operation should succeed: {response}"
    );
    let output = response
        .get("output")
        .and_then(Value::as_str)
        .expect("output string");
    serde_json::from_str(output.trim())
        .unwrap_or_else(|err| panic!("operation output must be JSON: {err}; output={output}"))
}

fn capture_payload(dedupe_key: &str, summary: &str) -> Value {
    json!({
        "schema_version": 1,
        "operation": "improvement.capture",
        "params": {
            "source": "agent-failure",
            "target_artifact": "skill",
            "classification": "gwt-caused",
            "confidence": "high",
            "summary": summary,
            "details": "Codex followed stale instructions from /Users/alice/private-repo/AGENTS.md with token ghp_1234567890abcdef.",
            "evidence_digest": "Stop hook allowed completion without skill update evidence.",
            "dedupe_key": dedupe_key,
            "local_evidence": [
                {
                    "kind": "transcript",
                    "path": "/Users/alice/private-repo/.gwt/transcript.jsonl"
                }
            ]
        }
    })
}

fn low_confidence_capture_payload(dedupe_key: &str, summary: &str) -> Value {
    let mut payload = capture_payload(dedupe_key, summary);
    payload["params"]["confidence"] = json!("low");
    payload
}

fn legacy_candidate_store(project: &Path) -> PathBuf {
    project
        .join(".gwt")
        .join("improvements")
        .join("candidates.json")
}

fn canonical_candidate_store(home: &Path, project: &Path) -> PathBuf {
    let repo_hash = gwt_core::paths::project_scope_hash(project);
    home.join(".gwt")
        .join("projects")
        .join(repo_hash.as_str())
        .join("improvements")
        .join("candidates.json")
}

fn legacy_candidate(index: usize, occurrences: u64, evidence_path: Option<&Path>) -> Value {
    json!({
        "id": format!("impr-legacy-{index:02}"),
        "created_at": "2026-07-07T00:00:00Z",
        "updated_at": "2026-07-07T00:00:00Z",
        "source": "agent-failure",
        "target_artifact": "issue-spec-workflow",
        "classification": "gwt-caused",
        "confidence": "high",
        "state": "pending",
        "dedupe_key": format!("legacy:workflow-failure-{index:02}"),
        "occurrences": occurrences,
        "sanitized_summary": format!("Legacy workflow failure {index:02}"),
        "sanitized_details": null,
        "evidence_digest": null,
        "local_evidence": evidence_path.map(|path| vec![json!({
            "kind": "transcript",
            "path": path,
        })]).unwrap_or_default(),
        "linked_issue": null,
        "dismissed_reason": null,
    })
}

fn write_legacy_store(project: &Path, candidates: Vec<Value>) -> PathBuf {
    let path = legacy_candidate_store(project);
    fs::create_dir_all(path.parent().expect("legacy store parent"))
        .expect("legacy store directory");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({ "candidates": candidates }))
            .expect("serialize legacy store"),
    )
    .expect("write legacy store");
    path
}

fn save_session_path(home: &Path, project: &Path, repo_hash: &str) {
    let mut session = gwt_agent::Session::new(project, "test", gwt_agent::AgentId::Codex);
    session.repo_hash = Some(repo_hash.to_string());
    session
        .save(&home.join(".gwt").join("sessions"))
        .expect("save session");
}

#[test]
fn improvement_store_is_shared_by_worktrees_and_isolated_by_repository() {
    let fixture = fixture();
    let repositories = tempfile::tempdir().expect("repository root");
    let (repository_a, worktree_a) = repository_with_linked_worktree(
        repositories.path(),
        "repository-a",
        "https://github.com/example/repository-a.git",
    );
    let (repository_b, _) = repository_with_linked_worktree(
        repositories.path(),
        "repository-b",
        "https://github.com/example/repository-b.git",
    );

    let captured = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository_a,
        capture_payload(
            "skill:canonical-store",
            "Canonical repository store is required",
        ),
    ));
    let captured_id = captured["id"].as_str().expect("captured id");

    let linked_worktree_list = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &worktree_a,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(
        linked_worktree_list["candidates"][0]["id"], captured_id,
        "linked worktrees of one repository must share the canonical store"
    );

    let other_repository_list = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository_b,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(
        other_repository_list["candidates"],
        json!([]),
        "different repositories must remain isolated"
    );
    assert!(
        !legacy_candidate_store(&repository_a).exists()
            && !legacy_candidate_store(&worktree_a).exists(),
        "canonical storage must not leave worktree-local candidate stores"
    );
}

#[test]
fn improvement_store_imports_nineteen_legacy_candidates_without_inventing_occurrences() {
    let fixture = fixture();
    let repositories = tempfile::tempdir().expect("repository root");
    let (repository, linked_worktree) = repository_with_linked_worktree(
        repositories.path(),
        "legacy-repository",
        "https://github.com/example/legacy-repository.git",
    );
    let first_source = write_legacy_store(
        &repository,
        (0..2)
            .map(|index| legacy_candidate(index, (index + 1) as u64, None))
            .collect(),
    );
    let second_source = write_legacy_store(
        &linked_worktree,
        (2..19)
            .map(|index| legacy_candidate(index, (index + 1) as u64, None))
            .collect(),
    );

    let first_list = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(first_list["candidates"].as_array().unwrap().len(), 19);

    let canonical_path = canonical_candidate_store(fixture.home.path(), &repository);
    let first_store: Value =
        serde_json::from_slice(&fs::read(&canonical_path).expect("canonical store"))
            .expect("parse canonical store");
    let candidates = first_store["candidates"].as_array().expect("candidates");
    assert_eq!(first_store["schema_version"], 1);
    assert!(candidates.iter().all(|candidate| {
        candidate["state"] == "needs-evidence"
            && candidate["occurrences"] == 0
            && candidate["legacy_occurrence_count"]
                .as_u64()
                .unwrap_or_default()
                > 0
            && candidate["fingerprint"]
                .as_str()
                .is_some_and(|fingerprint| fingerprint.starts_with("legacy:"))
    }));
    let fingerprints = candidates
        .iter()
        .map(|candidate| candidate["fingerprint"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    let second_list = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &linked_worktree,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(second_list["candidates"].as_array().unwrap().len(), 19);
    let second_store: Value =
        serde_json::from_slice(&fs::read(&canonical_path).expect("canonical store"))
            .expect("parse canonical store");
    assert_eq!(second_store["candidates"].as_array().unwrap().len(), 19);
    assert_eq!(
        second_store["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| candidate["fingerprint"].as_str().unwrap().to_string())
            .collect::<Vec<_>>(),
        fingerprints,
        "legacy fingerprints and imports must be deterministic and idempotent"
    );

    write_legacy_store(
        &repository,
        (0..2)
            .map(|index| legacy_candidate(index, (index + 1) as u64, None))
            .chain(std::iter::once(legacy_candidate(19, 20, None)))
            .collect(),
    );
    let discovered_later = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(discovered_later["candidates"].as_array().unwrap().len(), 20);
    let updated_store: Value =
        serde_json::from_slice(&fs::read(&canonical_path).expect("updated store"))
            .expect("parse updated store");
    let unchanged = updated_store["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["dedupe_key"] == "legacy:workflow-failure-00")
        .expect("existing legacy candidate");
    assert_eq!(
        unchanged["legacy_occurrence_count"], 1,
        "rescanning changed source content must not re-add an existing aggregate"
    );
    assert!(
        first_source.exists() && second_source.exists(),
        "legacy source files must remain read-only and must not be deleted"
    );
}

#[cfg(unix)]
#[test]
fn improvement_store_rescans_session_sources_copies_evidence_and_records_diagnostics() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let fixture = fixture();
    let repositories = tempfile::tempdir().expect("repository root");
    let (repository, _) = repository_with_linked_worktree(
        repositories.path(),
        "session-repository",
        "https://github.com/example/session-repository.git",
    );
    let repo_hash = gwt_core::paths::project_scope_hash(&repository);

    let before_discovery = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(before_discovery["candidates"], json!([]));

    let session_source = repositories.path().join("session-source");
    fs::create_dir_all(&session_source).expect("session source");
    let evidence = session_source.join("failure.log");
    fs::write(&evidence, b"typed local evidence\n").expect("evidence");
    let legacy_source = write_legacy_store(
        &session_source,
        vec![legacy_candidate(20, 7, Some(&evidence))],
    );
    save_session_path(fixture.home.path(), &session_source, repo_hash.as_str());

    let evidence_digest = hex::encode(Sha256::digest(b"typed local evidence\n"));
    let canonical_evidence = canonical_candidate_store(fixture.home.path(), &repository)
        .parent()
        .expect("improvements directory")
        .join("evidence")
        .join(&evidence_digest);
    fs::create_dir_all(canonical_evidence.parent().unwrap()).expect("canonical evidence dir");
    fs::write(&canonical_evidence, b"typed local evidence\n")
        .expect("simulate interrupted evidence copy");

    let alias_source = repositories.path().join("session-source-alias");
    symlink(&session_source, &alias_source).expect("source alias");
    save_session_path(fixture.home.path(), &alias_source, repo_hash.as_str());

    let missing_source = repositories.path().join("missing-session-source");
    save_session_path(fixture.home.path(), &missing_source, repo_hash.as_str());

    let malformed_source = repositories.path().join("malformed-session-source");
    let malformed_store = legacy_candidate_store(&malformed_source);
    fs::create_dir_all(malformed_store.parent().unwrap()).expect("malformed parent");
    fs::write(&malformed_store, b"{not-json").expect("malformed store");
    save_session_path(fixture.home.path(), &malformed_source, repo_hash.as_str());

    let symlink_source = repositories.path().join("symlink-session-source");
    let symlink_store = legacy_candidate_store(&symlink_source);
    fs::create_dir_all(symlink_store.parent().unwrap()).expect("symlink parent");
    let symlink_target = repositories.path().join("symlink-target.json");
    fs::write(&symlink_target, b"{\"candidates\":[]}").expect("symlink target");
    symlink(&symlink_target, &symlink_store).expect("symlink store");
    save_session_path(fixture.home.path(), &symlink_source, repo_hash.as_str());

    let permission_source = repositories.path().join("permission-session-source");
    let permission_store =
        write_legacy_store(&permission_source, vec![legacy_candidate(21, 1, None)]);
    let original_permissions = fs::metadata(&permission_store)
        .expect("permission metadata")
        .permissions();
    fs::set_permissions(&permission_store, fs::Permissions::from_mode(0o000))
        .expect("deny store read");
    let permission_is_enforced = fs::read(&permission_store).is_err();
    save_session_path(fixture.home.path(), &permission_source, repo_hash.as_str());

    let response = run_gwtd_json_in(
        fixture.home.path(),
        &repository,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    );
    fs::set_permissions(&permission_store, original_permissions).expect("restore permissions");
    let list = operation_output(&response);
    let imported_count = list["candidates"].as_array().unwrap().len();
    assert_eq!(imported_count, if permission_is_enforced { 1 } else { 2 });

    let canonical_path = canonical_candidate_store(fixture.home.path(), &repository);
    let store: Value = serde_json::from_slice(&fs::read(&canonical_path).expect("canonical store"))
        .expect("parse canonical store");
    let candidate = store["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["legacy_occurrence_count"] == 7)
        .expect("session-source candidate");
    assert_eq!(candidate["legacy_occurrence_count"], 7);
    assert_eq!(candidate["local_evidence"][0]["path"], Value::Null);
    assert_eq!(candidate["local_evidence"][0]["digest"], evidence_digest);
    assert_eq!(
        fs::read(&canonical_evidence).expect("copied evidence"),
        b"typed local evidence\n"
    );
    let diagnostic_codes = store["legacy_import"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["code"].as_str())
        .collect::<Vec<_>>();
    let mut expected_diagnostics = vec![
        "alias-source",
        "missing-source",
        "malformed-json",
        "symlink-source",
    ];
    if permission_is_enforced {
        expected_diagnostics.push("permission-denied");
    }
    for expected in expected_diagnostics {
        assert!(
            diagnostic_codes.contains(&expected),
            "missing {expected} diagnostic: {diagnostic_codes:?}"
        );
    }

    let retry = operation_output(&run_gwtd_json_in(
        fixture.home.path(),
        &repository,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(
        retry["candidates"].as_array().unwrap().len(),
        imported_count + usize::from(permission_is_enforced),
        "a previously unreadable source should import after it becomes readable"
    );
    assert!(legacy_source.exists() && evidence.exists());
}

#[test]
fn improvement_store_serializes_concurrent_cross_process_capture() {
    use std::sync::{Arc, Barrier};

    const WORKERS: usize = 32;
    let fixture = fixture();
    let barrier = Arc::new(Barrier::new(WORKERS));
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(WORKERS);
        for worker in 0..WORKERS {
            let barrier = Arc::clone(&barrier);
            let home = fixture.home.path().to_path_buf();
            let project = fixture.project.path().to_path_buf();
            handles.push(scope.spawn(move || {
                barrier.wait();
                operation_output(&run_gwtd_json_in(
                    &home,
                    &project,
                    low_confidence_capture_payload(
                        "coordination:concurrent-capture",
                        &format!("Concurrent capture {worker}"),
                    ),
                ))
            }));
        }
        for handle in handles {
            handle.join().expect("capture worker");
        }
    });

    let store_bytes = fs::read(canonical_candidate_store(
        fixture.home.path(),
        fixture.project.path(),
    ))
    .expect("canonical store");
    let store: Value = serde_json::from_slice(&store_bytes).expect("store remains valid JSON");
    let candidates = store["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1, "concurrent upserts must not duplicate");
    assert_eq!(
        candidates[0]["occurrences"], WORKERS as u64,
        "cross-process read-modify-write must not lose occurrences"
    );
    assert_eq!(store["schema_version"], 1);
}

#[test]
fn improvement_store_rejects_unknown_future_schema_without_rewriting_it() {
    let fixture = fixture();
    let store_path = canonical_candidate_store(fixture.home.path(), fixture.project.path());
    fs::create_dir_all(store_path.parent().unwrap()).expect("store directory");
    let future_store = b"{\"schema_version\":999,\"candidates\":[]}";
    fs::write(&store_path, future_store).expect("future store");

    let output = run_gwtd_json_raw(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("unsupported improvement store schema version: 999"));
    assert_eq!(
        fs::read(&store_path).expect("future store remains"),
        future_store,
        "a future schema must fail closed without being rewritten"
    );
}

#[test]
fn improvement_capture_sanitizes_and_persists_pending_candidate() {
    let fixture = fixture();
    let response = run_gwtd_json(
        &fixture,
        capture_payload(
            "skill:gwt-discussion:stale-instruction",
            "Skill update missing for /Users/alice/private-repo failure",
        ),
    );
    let body = operation_output(&response);
    assert_eq!(body["state"], "pending");
    assert_eq!(body["occurrences"], 1);
    assert!(body["id"].as_str().unwrap_or_default().starts_with("impr-"));

    let stored = fs::read_to_string(canonical_candidate_store(
        fixture.home.path(),
        fixture.project.path(),
    ))
    .expect("candidate store");
    assert!(
        !stored.contains("/Users/alice"),
        "public candidate store fields must not contain absolute private paths: {stored}"
    );
    assert!(
        !stored.contains("ghp_1234567890abcdef"),
        "candidate store must redact token-like secrets: {stored}"
    );
    assert!(
        stored.contains("[redacted-path]"),
        "redacted path marker should be visible: {stored}"
    );
}

#[test]
fn improvement_capture_rejects_invalid_enum_value() {
    let fixture = fixture();
    let output = run_gwtd_json_raw(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.capture",
            "params": {
                "source": "agent-failure",
                "target_artifact": "skill",
                "classification": "bad",
                "confidence": "high",
                "summary": "bad enum"
            }
        }),
    );
    assert!(
        !output.status.success(),
        "invalid enum should fail, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value for classification: bad"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        !canonical_candidate_store(fixture.home.path(), fixture.project.path()).exists(),
        "invalid capture should not create a candidate store"
    );
}

#[test]
fn improvement_capture_dedupes_and_list_returns_single_updated_candidate() {
    let fixture = fixture();
    let first = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload("coordination:title-summary-drift", "Title summary drift"),
    ));
    let second = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload(
            "coordination:title-summary-drift",
            "Title summary drift again",
        ),
    ));
    assert_eq!(
        first["id"], second["id"],
        "dedupe should reuse candidate id"
    );
    assert_eq!(second["occurrences"], 2);

    let list = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {"state": "pending"}
        }),
    ));
    let candidates = list["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0]["dedupe_key"],
        "coordination:title-summary-drift"
    );
    assert_eq!(candidates[0]["occurrences"], 2);
}

#[test]
fn improvement_dismiss_and_link_issue_update_lifecycle() {
    let fixture = fixture();
    let captured = operation_output(&run_gwtd_json(
        &fixture,
        capture_payload(
            "verification:user-skip-regression",
            "Verification skip regression",
        ),
    ));
    let id = captured["id"].as_str().expect("id");

    let linked = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.link_issue",
            "params": {
                "id": id,
                "number": 3164,
                "url": "https://github.com/akiojin/gwt/issues/3164",
                "repository": "akiojin/gwt"
            }
        }),
    ));
    assert_eq!(linked["state"], "linked");
    assert_eq!(linked["linked_issue"]["number"], 3164);

    let dismissed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.dismiss",
            "params": {
                "id": id,
                "reason": "covered by existing SPEC"
            }
        }),
    ));
    assert_eq!(dismissed["state"], "dismissed");
}

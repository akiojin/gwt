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
    let output = run_gwtd_json_raw_in_with_session(home, project, None, payload);
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

fn run_gwtd_json_with_session(fixture: &Fixture, session_id: &str, payload: Value) -> Value {
    run_gwtd_json_in_with_session(
        fixture.home.path(),
        fixture.project.path(),
        session_id,
        payload,
    )
}

fn run_gwtd_json_in_with_session(
    home: &Path,
    project: &Path,
    session_id: &str,
    payload: Value,
) -> Value {
    let output = run_gwtd_json_raw_in_with_session(home, project, Some(session_id), payload);
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
    run_gwtd_json_raw_in_with_session(home, project, None, payload)
}

fn run_gwtd_json_raw_in_with_session(
    home: &Path,
    project: &Path,
    session_id: Option<&str>,
    payload: Value,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gwtd"));
    command
        .current_dir(project)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove(gwt_agent::GWT_SESSION_ID_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(session_id) = session_id {
        command.env(gwt_agent::GWT_SESSION_ID_ENV, session_id);
    }
    let mut child = command.spawn().expect("run gwtd");
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

fn typed_capture_payload() -> Value {
    json!({
        "schema_version": 1,
        "operation": "improvement.capture",
        "params": {
            "source": "agent-failure",
            "target_artifact": "coordination",
            "classification": "gwt-caused",
            "confidence": "high",
            "subsystem": "coordination",
            "contract_id": "coordination.board-status",
            "contract_schema_revision": 1,
            "failure_code": "STATUS_NOT_POSTED",
            "evidence": {
                "expected_outcome": "BOARD_STATUS_POSTED",
                "observed_outcome": "BOARD_STATUS_MISSING"
            },
            "summary": "Free-form local context that is not identity",
            "evidence_digest": "caller-controlled-digest",
            "dedupe_key": "caller-controlled-dedupe"
        }
    })
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

fn lifecycle_candidate(id: &str, state: &str, updated_at: &str) -> Value {
    json!({
        "schema_version": 3,
        "id": id,
        "created_at": "2026-07-14T00:00:00Z",
        "updated_at": updated_at,
        "source": "agent-failure",
        "target_artifact": "coordination",
        "classification": "gwt-caused",
        "confidence": "high",
        "state": state,
        "blocked_reason": null,
        "failure_subcode": null,
        "retry": null,
        "owner": null,
        "resolver_snapshot": null,
        "dedupe_key": format!("lifecycle:{id}"),
        "occurrences": 1,
        "legacy_occurrence_count": null,
        "fingerprint": format!("v2:{id:0>64}"),
        "eligibility": "deterministic",
        "typed_evidence": null,
        "distinct_occurrences": [],
        "capture_status_generation": 0,
        "capture_status_delivered_generation": 0,
        "sanitized_summary": format!("Lifecycle {id}"),
        "sanitized_details": null,
        "evidence_digest": null,
        "local_evidence": [],
        "linked_issue": null,
        "dismissed_reason": null,
        "legacy_provenance": [],
        "attempt": null
    })
}

fn write_canonical_store(
    home: &Path,
    project: &Path,
    schema_version: u64,
    candidates: Vec<Value>,
) -> PathBuf {
    let path = canonical_candidate_store(home, project);
    fs::create_dir_all(path.parent().expect("canonical store parent"))
        .expect("canonical store directory");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": schema_version,
            "source_scope_nonce": "11".repeat(32),
            "candidates": candidates,
            "legacy_import": {}
        }))
        .expect("serialize canonical store"),
    )
    .expect("write canonical store");
    path
}

fn save_session_path(home: &Path, project: &Path, repo_hash: &str) {
    let mut session = gwt_agent::Session::new(project, "test", gwt_agent::AgentId::Codex);
    session.repo_hash = Some(repo_hash.to_string());
    session
        .save(&home.join(".gwt").join("sessions"))
        .expect("save session");
}

fn save_verified_session(home: &Path, project: &Path) -> String {
    save_session_with_repo_hash(
        home,
        project,
        &gwt_core::paths::project_scope_hash(project).to_string(),
    )
}

fn save_session_with_repo_hash(home: &Path, project: &Path, repo_hash: &str) -> String {
    let mut session = gwt_agent::Session::new(project, "test", gwt_agent::AgentId::Codex);
    session.repo_hash = Some(repo_hash.to_string());
    let id = session.id.clone();
    session
        .save(&home.join(".gwt").join("sessions"))
        .expect("save verified session");
    id
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
    assert_eq!(first_store["schema_version"], 3);
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

#[test]
fn improvement_store_scrubs_typed_provenance_from_legacy_sources() {
    let fixture = fixture();
    let mut candidate = legacy_candidate(0, 1, None);
    candidate["schema_version"] = json!(2);
    candidate["eligibility"] = json!("deterministic");
    candidate["typed_evidence"] = json!({
        "subsystem": "coordination",
        "contract_id": "coordination.board-status",
        "contract_schema_revision": 1,
        "failure_code": "STATUS_NOT_POSTED",
        "target_artifact": "coordination",
        "expected_outcome": "BOARD_STATUS_POSTED",
        "observed_outcome": "BOARD_STATUS_MISSING"
    });
    candidate["distinct_occurrences"] = json!([{
        "opaque_key": "occ:v1:forged",
        "evidence_digest": "forged",
        "captured_at": "2026-07-13T00:00:00Z",
        "origin": "deterministic",
        "qualifies_unattended": true,
        "producer_id": "forged.producer",
        "producer_registry_revision": 999,
        "routing_basis_revision": 999
    }]);
    candidate["capture_status_generation"] = json!(999);
    candidate["capture_status_delivered_generation"] = json!(998);
    write_legacy_store(fixture.project.path(), vec![candidate]);

    operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));

    let canonical: Value = serde_json::from_slice(
        &fs::read(canonical_candidate_store(
            fixture.home.path(),
            fixture.project.path(),
        ))
        .expect("canonical store"),
    )
    .expect("parse canonical store");
    let candidate = &canonical["candidates"][0];
    assert_eq!(candidate["eligibility"], "needs-evidence");
    assert_eq!(candidate["typed_evidence"], Value::Null);
    assert_eq!(candidate["distinct_occurrences"], json!([]));
    assert_eq!(candidate["capture_status_generation"], 0);
    assert_eq!(candidate["capture_status_delivered_generation"], 0);
    assert_eq!(candidate["occurrences"], 0);
    assert_eq!(candidate["legacy_occurrence_count"], 1);
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
    let sessions = (0..WORKERS)
        .map(|_| save_verified_session(fixture.home.path(), fixture.project.path()))
        .collect::<Vec<_>>();
    let barrier = Arc::new(Barrier::new(WORKERS));
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(WORKERS);
        for (worker, session_id) in sessions.into_iter().enumerate() {
            let barrier = Arc::clone(&barrier);
            let home = fixture.home.path().to_path_buf();
            let project = fixture.project.path().to_path_buf();
            handles.push(scope.spawn(move || {
                barrier.wait();
                let mut payload = typed_capture_payload();
                payload["params"]["confidence"] = json!("low");
                payload["params"]["summary"] = json!(format!("Concurrent capture {worker}"));
                operation_output(&run_gwtd_json_in_with_session(
                    &home,
                    &project,
                    &session_id,
                    payload,
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
    assert_eq!(store["schema_version"], 3);
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
fn improvement_store_rejects_malformed_source_nonce_without_rewriting_it() {
    let fixture = fixture();
    let store_path = canonical_candidate_store(fixture.home.path(), fixture.project.path());
    fs::create_dir_all(store_path.parent().unwrap()).expect("store directory");
    let malformed_store =
        b"{\"schema_version\":1,\"source_scope_nonce\":\"repo-derived\",\"candidates\":[]}";
    fs::write(&store_path, malformed_store).expect("malformed nonce store");

    let output = run_gwtd_json_raw(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid improvement source scope nonce")
    );
    assert_eq!(
        fs::read(&store_path).expect("malformed store remains"),
        malformed_store,
        "a malformed nonce must fail closed without store repair"
    );
}

#[test]
fn improvement_store_migrates_v1_occurrences_to_legacy_aggregate() {
    let fixture = fixture();
    let store_path = canonical_candidate_store(fixture.home.path(), fixture.project.path());
    fs::create_dir_all(store_path.parent().unwrap()).expect("store directory");
    fs::write(
        &store_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "candidates": [legacy_candidate(0, 3, None)]
        }))
        .expect("serialize v1 store"),
    )
    .expect("write v1 store");

    let listed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(listed["candidates"].as_array().unwrap().len(), 1);

    let migrated: Value = serde_json::from_slice(&fs::read(&store_path).expect("migrated store"))
        .expect("parse migrated store");
    assert_eq!(migrated["schema_version"], 3);
    assert_eq!(migrated["source_scope_nonce"].as_str().unwrap().len(), 64);
    let candidate = &migrated["candidates"][0];
    assert_eq!(candidate["schema_version"], 3);
    assert_eq!(candidate["state"], "needs-evidence");
    assert_eq!(candidate["occurrences"], 0);
    assert_eq!(candidate["legacy_occurrence_count"], 3);
    assert!(candidate["fingerprint"]
        .as_str()
        .is_some_and(|fingerprint| fingerprint.starts_with("legacy:")));

    let first_migration = fs::read(&store_path).expect("first migration bytes");
    operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(
        fs::read(&store_path).expect("second migration bytes"),
        first_migration,
        "v1 migration must be idempotent"
    );
}

#[test]
fn improvement_store_rejects_v2_without_source_nonce_without_rewriting_it() {
    let fixture = fixture();
    let store_path = canonical_candidate_store(fixture.home.path(), fixture.project.path());
    fs::create_dir_all(store_path.parent().unwrap()).expect("store directory");
    let missing_nonce = b"{\"schema_version\":2,\"candidates\":[]}";
    fs::write(&store_path, missing_nonce).expect("missing nonce store");

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
        .contains("improvement source scope nonce is missing"));
    assert_eq!(
        fs::read(&store_path).expect("missing nonce store remains"),
        missing_nonce,
        "a current store with missing identity must fail closed"
    );
}

#[test]
fn improvement_legacy_capture_sanitizes_and_persists_needs_evidence_candidate() {
    let fixture = fixture();
    let response = run_gwtd_json(
        &fixture,
        capture_payload(
            "skill:gwt-discussion:stale-instruction",
            "Skill update missing for /Users/alice/private-repo failure",
        ),
    );
    let body = operation_output(&response);
    assert_eq!(body["state"], "needs-evidence");
    assert_eq!(body["eligibility"], "needs-evidence");
    assert_eq!(body["occurrences"], 0);
    assert_eq!(body["improvement_contract_version"], 2);
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
fn improvement_legacy_capture_dedupes_without_inventing_typed_occurrences() {
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
    assert_eq!(second["occurrences"], 0);
    assert_eq!(second["updated"], true);

    let list = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {"state": "needs-evidence"}
        }),
    ));
    let candidates = list["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].get("dedupe_key").is_none());
    assert_eq!(candidates[0]["occurrences"], 0);
    let persisted: Value = serde_json::from_slice(
        &fs::read(canonical_candidate_store(
            fixture.home.path(),
            fixture.project.path(),
        ))
        .expect("candidate store"),
    )
    .expect("candidate store JSON");
    assert_eq!(
        persisted["candidates"][0]["dedupe_key"], "coordination:title-summary-drift",
        "legacy dedupe remains local for migration compatibility"
    );
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

#[test]
fn improvement_capture_v2_requires_two_distinct_verified_sessions() {
    let fixture = fixture();
    let session_a = save_verified_session(fixture.home.path(), fixture.project.path());
    let session_b = save_verified_session(fixture.home.path(), fixture.project.path());

    let first = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session_a,
        typed_capture_payload(),
    ));
    assert_eq!(first["improvement_contract_version"], 2);
    assert_eq!(first["state"], "needs-evidence");
    assert_eq!(first["eligibility"], "needs-evidence");
    assert_eq!(first["occurrences"], 1);
    let fingerprint = first["fingerprint"]
        .as_str()
        .expect("typed fingerprint")
        .to_string();

    let mut replay_payload = typed_capture_payload();
    replay_payload["params"]["summary"] = json!("Changed local summary");
    replay_payload["params"]["dedupe_key"] = json!("changed-caller-dedupe");
    replay_payload["params"]["evidence_digest"] = json!("changed-caller-digest");
    let replay = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session_a,
        replay_payload,
    ));
    assert_eq!(replay["id"], first["id"]);
    assert_eq!(replay["occurrences"], 1, "same-session replay is one");
    assert_eq!(replay["state"], "needs-evidence");

    let mut corroborating_payload = typed_capture_payload();
    corroborating_payload["params"]["subsystem"] = json!(" Coordination ");
    corroborating_payload["params"]["contract_id"] = json!("Coordination.Board-Status");
    corroborating_payload["params"]["failure_code"] = json!("status_not_posted");
    corroborating_payload["params"]["evidence"]["expected_outcome"] = json!("board_status_posted");
    corroborating_payload["params"]["evidence"]["observed_outcome"] = json!("board_status_missing");
    let corroborated = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session_b,
        corroborating_payload,
    ));
    assert_eq!(corroborated["id"], first["id"]);
    assert_eq!(corroborated["fingerprint"], fingerprint);
    assert_eq!(corroborated["occurrences"], 2);
    assert_eq!(corroborated["eligibility"], "interpretive-corroboration");
    assert_eq!(corroborated["state"], "owner-resolving");

    let store: Value = serde_json::from_slice(
        &fs::read(canonical_candidate_store(
            fixture.home.path(),
            fixture.project.path(),
        ))
        .expect("typed candidate store"),
    )
    .expect("parse typed candidate store");
    let nonce = store["source_scope_nonce"]
        .as_str()
        .expect("source scope nonce");
    assert_eq!(nonce.len(), 64, "source scope nonce must be 256 bits");
    assert!(nonce.chars().all(|character| character.is_ascii_hexdigit()));
    let candidate = &store["candidates"][0];
    assert_eq!(
        candidate["distinct_occurrences"].as_array().unwrap().len(),
        2
    );
    assert_ne!(candidate["evidence_digest"], "caller-controlled-digest");
    assert_ne!(candidate["dedupe_key"], "caller-controlled-dedupe");
}

#[test]
fn improvement_capture_v2_rejects_conflicting_same_session_replay_without_mutation() {
    let fixture = fixture();
    let session = save_verified_session(fixture.home.path(), fixture.project.path());
    operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session,
        typed_capture_payload(),
    ));
    let store_path = canonical_candidate_store(fixture.home.path(), fixture.project.path());
    let before = fs::read(&store_path).expect("store before conflict");

    let mut conflicting = typed_capture_payload();
    conflicting["params"]["evidence"]["observed_outcome"] = json!("BOARD_STATUS_POSTED_TOO_LATE");
    let output = run_gwtd_json_raw_in_with_session(
        fixture.home.path(),
        fixture.project.path(),
        Some(&session),
        conflicting,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("conflicting improvement occurrence replay"));
    assert_eq!(
        fs::read(&store_path).expect("store after conflict"),
        before,
        "conflicting replay must not rewrite candidate evidence"
    );
}

#[test]
fn improvement_legacy_capture_cannot_overwrite_typed_fingerprint_namespace() {
    let fixture = fixture();
    let session = save_verified_session(fixture.home.path(), fixture.project.path());
    let typed = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session,
        typed_capture_payload(),
    ));
    let typed_id = typed["id"].as_str().expect("typed id").to_string();
    let fingerprint = typed["fingerprint"].as_str().expect("typed fingerprint");

    let mut legacy_payload = capture_payload(
        &format!("fingerprint:{fingerprint}"),
        "Caller-controlled legacy collision",
    );
    legacy_payload["params"]["evidence_digest"] = json!("legacy-controlled-digest");
    let legacy = operation_output(&run_gwtd_json(&fixture, legacy_payload));
    assert_ne!(legacy["id"], typed_id, "legacy and typed ids must differ");

    let store: Value = serde_json::from_slice(
        &fs::read(canonical_candidate_store(
            fixture.home.path(),
            fixture.project.path(),
        ))
        .expect("candidate store"),
    )
    .expect("parse candidate store");
    assert_eq!(store["candidates"].as_array().unwrap().len(), 2);
    let typed_candidate = store["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == typed_id)
        .expect("typed candidate remains");
    assert!(typed_candidate["typed_evidence"].is_object());
    assert_ne!(
        typed_candidate["evidence_digest"],
        "legacy-controlled-digest"
    );
    assert_eq!(typed_candidate["occurrences"], 1);
}

#[test]
fn improvement_capture_v2_rejects_public_deterministic_identity_claims() {
    for (key, value) in [
        ("producer_id", json!("gate.intake")),
        ("source_event_id", json!("event-1")),
        ("session_id", json!("forged-session")),
        ("routing_basis_revision", json!(7)),
        ("budget_profile", json!("strict-stop")),
        ("fingerprint", json!("caller-fingerprint")),
        ("occurrence_key", json!("caller-occurrence")),
    ] {
        let fixture = fixture();
        let session = save_verified_session(fixture.home.path(), fixture.project.path());
        let mut payload = typed_capture_payload();
        payload["params"][key] = value;
        let output = run_gwtd_json_raw_in_with_session(
            fixture.home.path(),
            fixture.project.path(),
            Some(&session),
            payload,
        );

        assert!(
            !output.status.success(),
            "public deterministic claim {key} must be rejected"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("untrusted identity field"),
            "unexpected error for {key}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !canonical_candidate_store(fixture.home.path(), fixture.project.path()).exists(),
            "rejected {key} must not mutate the candidate store"
        );
    }
}

#[test]
fn improvement_capture_v2_validates_typed_evidence_before_mutation() {
    let invalid_payloads = [
        {
            let mut payload = typed_capture_payload();
            payload["params"]
                .as_object_mut()
                .expect("params")
                .remove("subsystem");
            payload
        },
        {
            let mut payload = typed_capture_payload();
            payload["params"]["contract_schema_revision"] = json!(0);
            payload
        },
        {
            let mut payload = typed_capture_payload();
            payload["params"]["evidence"] = json!({
                "observed_outcome": "EXPECTED_OUTCOME_MISSING"
            });
            payload
        },
        {
            let mut payload = typed_capture_payload();
            payload["params"]["evidence"]["raw_log"] = json!("not allowlisted");
            payload
        },
        {
            let mut payload = typed_capture_payload();
            payload["params"]["evidence"]["expected_outcome"] =
                json!("caller controlled prose is not a machine token");
            payload
        },
    ];

    for payload in invalid_payloads {
        let fixture = fixture();
        let session = save_verified_session(fixture.home.path(), fixture.project.path());
        let output = run_gwtd_json_raw_in_with_session(
            fixture.home.path(),
            fixture.project.path(),
            Some(&session),
            payload,
        );
        assert!(
            !output.status.success(),
            "invalid typed evidence must fail before capture"
        );
        assert!(
            !canonical_candidate_store(fixture.home.path(), fixture.project.path()).exists(),
            "invalid typed evidence must not create a candidate store"
        );
    }
}

#[test]
fn improvement_capture_v2_without_a_verified_session_remains_needs_evidence() {
    let fixture = fixture();
    let captured = operation_output(&run_gwtd_json(&fixture, typed_capture_payload()));
    assert_eq!(captured["improvement_contract_version"], 2);
    assert_eq!(captured["state"], "needs-evidence");
    assert_eq!(captured["eligibility"], "needs-evidence");
    assert_eq!(captured["occurrences"], 0);
}

#[test]
fn improvement_capture_v2_rejects_missing_or_cross_repository_session_evidence() {
    let missing = fixture();
    let missing_id = uuid::Uuid::new_v4().to_string();
    let captured = operation_output(&run_gwtd_json_with_session(
        &missing,
        &missing_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["state"], "needs-evidence");
    assert_eq!(captured["occurrences"], 0);

    let cross_repository = fixture();
    let cross_repository_id = save_session_with_repo_hash(
        cross_repository.home.path(),
        cross_repository.project.path(),
        "different-repository-hash",
    );
    let captured = operation_output(&run_gwtd_json_with_session(
        &cross_repository,
        &cross_repository_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["state"], "needs-evidence");
    assert_eq!(captured["occurrences"], 0);

    let id_mismatch = fixture();
    let stored_id = save_verified_session(id_mismatch.home.path(), id_mismatch.project.path());
    let forged_id = uuid::Uuid::new_v4().to_string();
    let sessions_dir = id_mismatch.home.path().join(".gwt").join("sessions");
    fs::rename(
        sessions_dir.join(format!("{stored_id}.toml")),
        sessions_dir.join(format!("{forged_id}.toml")),
    )
    .expect("rename session to mismatched id");
    let captured = operation_output(&run_gwtd_json_with_session(
        &id_mismatch,
        &forged_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["state"], "needs-evidence");
    assert_eq!(captured["occurrences"], 0);
}

#[test]
fn improvement_capture_v2_accepts_native_local_session_scope_only_for_its_project() {
    let fixture = fixture();
    run_git(fixture.project.path(), &["init"]);
    let session = gwt_agent::Session::new(
        fixture.project.path(),
        "local-only",
        gwt_agent::AgentId::Codex,
    );
    assert_eq!(
        session.repo_hash, None,
        "fixture must cover local/no-origin"
    );
    let session_id = session.id.clone();
    session
        .save(&fixture.home.path().join(".gwt").join("sessions"))
        .expect("save native local session");

    let captured = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["occurrences"], 1);
    assert_eq!(captured["state"], "needs-evidence");
    let candidate_id = captured["id"].clone();

    let foreign_project = tempfile::tempdir().expect("foreign project");
    run_git(foreign_project.path(), &["init"]);
    let foreign = gwt_agent::Session::new(
        foreign_project.path(),
        "foreign-local-only",
        gwt_agent::AgentId::Codex,
    );
    assert_eq!(foreign.repo_hash, None);
    let foreign_id = foreign.id.clone();
    foreign
        .save(&fixture.home.path().join(".gwt").join("sessions"))
        .expect("save foreign local session");
    let captured = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &foreign_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["id"], candidate_id);
    assert_eq!(captured["occurrences"], 1, "foreign session must not count");
}

#[cfg(unix)]
#[test]
fn improvement_capture_v2_rejects_symlinked_session_evidence() {
    use std::os::unix::fs::symlink;

    let fixture = fixture();
    let session_id = save_verified_session(fixture.home.path(), fixture.project.path());
    let sessions_dir = fixture.home.path().join(".gwt").join("sessions");
    let session_path = sessions_dir.join(format!("{session_id}.toml"));
    let target_path = sessions_dir.join("session-target.toml");
    fs::rename(&session_path, &target_path).expect("move session behind symlink");
    symlink(&target_path, &session_path).expect("symlink session");

    let captured = operation_output(&run_gwtd_json_with_session(
        &fixture,
        &session_id,
        typed_capture_payload(),
    ));
    assert_eq!(captured["state"], "needs-evidence");
    assert_eq!(captured["occurrences"], 0);
}

#[test]
fn memory_add_alone_does_not_create_or_advance_an_improvement_candidate() {
    let fixture = fixture();
    let response = run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "memory.add",
            "params": {
                "type": "lesson",
                "title": "Self improvement lesson",
                "context": "A workflow contract failed.",
                "learning": "Use typed failure evidence.",
                "future_action": "Capture evidence through the explicit operation."
            }
        }),
    );
    assert_eq!(response["ok"], true);

    let listed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(listed["candidates"], json!([]));
}

#[test]
fn improvement_list_v2_filters_typed_lifecycle_and_owner_fields() {
    let fixture = fixture();
    let mut blocked = lifecycle_candidate("blocked", "blocked", "2026-07-14T04:00:00Z");
    blocked["blocked_reason"] = json!("search");
    blocked["failure_subcode"] = json!("partial-page");
    blocked["retry"] = json!({
        "retryable": true,
        "remediation": "REFRESH_OWNER_CORPUS",
        "failed_at": "2026-07-14T04:00:00Z"
    });

    let mut linked = lifecycle_candidate("linked", "linked", "2026-07-14T03:00:00Z");
    let linked_fingerprint = linked["fingerprint"].clone();
    linked["owner"] = json!({
        "number": 42,
        "kind": "issue",
        "title": "Existing typed owner",
        "active": true,
        "url": "https://github.com/akiojin/gwt/issues/42",
        "fingerprint": linked_fingerprint,
        "readback_verified_at": "2026-07-14T03:00:00Z"
    });
    linked["linked_issue"] = json!({
        "number": 42,
        "url": "https://github.com/akiojin/gwt/issues/42",
        "repository": "akiojin/gwt"
    });

    let mut created = lifecycle_candidate("created", "created", "2026-07-14T02:00:00Z");
    created["confidence"] = json!("medium");
    let created_fingerprint = created["fingerprint"].clone();
    created["owner"] = json!({
        "number": 84,
        "kind": "issue",
        "title": "New typed owner",
        "active": true,
        "url": "https://github.com/akiojin/gwt/issues/84",
        "fingerprint": created_fingerprint,
        "readback_verified_at": "2026-07-14T02:00:00Z"
    });
    created["linked_issue"] = json!({
        "number": 84,
        "url": "https://github.com/akiojin/gwt/issues/84",
        "repository": "akiojin/gwt"
    });

    let mut needs = lifecycle_candidate("needs-evidence", "needs-evidence", "2026-07-14T01:00:00Z");
    needs["classification"] = json!("ambiguous");
    needs["confidence"] = json!("low");
    needs["eligibility"] = json!("needs-evidence");
    needs["occurrences"] = json!(0);

    write_canonical_store(
        fixture.home.path(),
        fixture.project.path(),
        3,
        vec![created, needs, linked, blocked],
    );

    let list = |params: Value| {
        operation_output(&run_gwtd_json(
            &fixture,
            json!({
                "schema_version": 1,
                "operation": "improvement.list",
                "params": params
            }),
        ))
    };

    let newest = list(json!({"limit": 2}));
    assert_eq!(newest["improvement_contract_version"], 2);
    assert_eq!(newest["candidates"][0]["id"], "blocked");
    assert_eq!(newest["candidates"][1]["id"], "linked");
    assert_eq!(newest["candidates"][0]["resolution_state"], "blocked");
    assert_eq!(newest["candidates"][0]["blocked_reason"], "search");
    assert_eq!(newest["candidates"][0]["failure_subcode"], "partial-page");
    assert_eq!(newest["candidates"][0]["retry"]["retryable"], true);

    assert_eq!(
        list(json!({"state": "promoted"}))["candidates"][0]["id"],
        "created"
    );
    assert_eq!(
        list(json!({"blocked_reason": "search"}))["candidates"][0]["id"],
        "blocked"
    );
    assert_eq!(
        list(json!({"failure_subcode": "partial-page"}))["candidates"][0]["id"],
        "blocked"
    );
    assert_eq!(
        list(json!({"classification": "ambiguous"}))["candidates"][0]["id"],
        "needs-evidence"
    );
    assert_eq!(
        list(json!({"confidence": "medium"}))["candidates"][0]["id"],
        "created"
    );
    let owned = list(json!({"owner_number": 42}));
    assert_eq!(owned["candidates"][0]["id"], "linked");
    assert_eq!(owned["candidates"][0]["owner"]["number"], 42);
}

#[test]
fn improvement_list_v2_reads_legacy_states_and_remote_retry_metadata() {
    let fixture = fixture();
    let states = [
        ("pending", "pending"),
        ("promoted", "created"),
        ("linked", "linked"),
        ("parked", "parked"),
        ("dismissed", "dismissed"),
        ("remote", "remote-outcome-unknown"),
    ];
    let candidates = states
        .iter()
        .enumerate()
        .map(|(index, (id, state))| {
            let mut candidate =
                lifecycle_candidate(id, state, &format!("2026-07-14T00:{index:02}:00Z"));
            candidate["schema_version"] = json!(2);
            if matches!(*id, "promoted" | "linked") {
                candidate["linked_issue"] = json!({
                    "number": index as u64 + 1,
                    "url": format!("https://github.com/akiojin/gwt/issues/{}", index + 1),
                    "repository": "akiojin/gwt"
                });
            }
            if *state == "dismissed" {
                candidate["dismissed_reason"] = json!("legacy dismissal");
            }
            if *state == "remote-outcome-unknown" {
                candidate["retry"] = json!({
                    "retryable": true,
                    "remediation": "REFRESH_OWNER_CORPUS",
                    "failed_at": "2026-07-14T00:05:00Z"
                });
            }
            candidate
        })
        .collect();
    let store_path =
        write_canonical_store(fixture.home.path(), fixture.project.path(), 2, candidates);

    let listed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {}
        }),
    ));
    assert_eq!(listed["improvement_contract_version"], 2);
    let candidates = listed["candidates"].as_array().unwrap();
    for (id, resolution_state) in states {
        let candidate = candidates
            .iter()
            .find(|candidate| candidate["id"] == id)
            .expect("legacy candidate");
        assert_eq!(candidate["resolution_state"], resolution_state);
        if id == "promoted" {
            assert_eq!(candidate["state"], "promoted");
        }
    }
    let remote = candidates
        .iter()
        .find(|candidate| candidate["id"] == "remote")
        .unwrap();
    assert_eq!(remote["retry"]["remediation"], "REFRESH_OWNER_CORPUS");
    let migrated: Value = serde_json::from_slice(&fs::read(store_path).expect("migrated store"))
        .expect("parse migrated store");
    assert_eq!(migrated["schema_version"], 3);
}

#[test]
fn improvement_list_v2_rejects_invalid_filters_and_downstream_states_without_rewrite() {
    for (params, expected) in [
        (json!({"state": "queue"}), "invalid improvement state"),
        (
            json!({"blocked_reason": "unknown"}),
            "invalid blocked reason",
        ),
        (
            json!({"failure_subcode": "unknown"}),
            "invalid failure subcode",
        ),
        (
            json!({"classification": "unknown"}),
            "invalid classification",
        ),
        (json!({"confidence": "unknown"}), "invalid confidence"),
        (
            json!({"owner_number": 0}),
            "owner_number must be greater than zero",
        ),
        (json!({"limit": 0}), "limit must be greater than zero"),
    ] {
        let fixture = fixture();
        let path =
            write_canonical_store(fixture.home.path(), fixture.project.path(), 2, Vec::new());
        let before = fs::read(&path).expect("store before invalid filter");
        let output = run_gwtd_json_raw(
            &fixture,
            json!({
                "schema_version": 1,
                "operation": "improvement.list",
                "params": params
            }),
        );
        assert!(!output.status.success(), "invalid filter must fail");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "unexpected error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(path).expect("unchanged store"), before);
    }

    for forbidden_state in ["queue", "in-progress", "verification-pending", "resolved"] {
        let fixture = fixture();
        let path = write_canonical_store(
            fixture.home.path(),
            fixture.project.path(),
            3,
            vec![lifecycle_candidate(
                "forbidden",
                forbidden_state,
                "2026-07-14T00:00:00Z",
            )],
        );
        let before = fs::read(&path).expect("store before invalid state");
        let output = run_gwtd_json_raw(
            &fixture,
            json!({
                "schema_version": 1,
                "operation": "improvement.list",
                "params": {}
            }),
        );
        assert!(!output.status.success(), "{forbidden_state} must not load");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unknown variant"));
        assert_eq!(fs::read(path).expect("invalid state store remains"), before);
    }
}

#[test]
fn improvement_list_v2_never_returns_raw_taint_inputs() {
    let fixture = fixture();
    let private_root = fixture.project.path().display().to_string();
    let raw_summary = format!("Customer customer-8675309 failed in {private_root}");
    let captured = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.capture",
            "params": {
                "source": "agent-failure",
                "target_artifact": "coordination",
                "classification": "gwt-caused",
                "confidence": "high",
                "subsystem": "coordination",
                "contract_id": "coordination.board-status",
                "contract_schema_revision": 1,
                "failure_code": "STATUS_NOT_POSTED",
                "evidence": {
                    "expected_outcome": "BOARD_STATUS_POSTED",
                    "observed_outcome": "BOARD_STATUS_MISSING"
                },
                "summary": raw_summary,
                "details": "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz",
                "evidence_digest": "alice@example.com",
                "dedupe_key": "private-repo:customer-8675309",
                "local_evidence": [{
                    "kind": "transcript",
                    "path": format!("{private_root}/trace.log")
                }]
            }
        }),
    ));
    let listed = operation_output(&run_gwtd_json(
        &fixture,
        json!({
            "schema_version": 1,
            "operation": "improvement.list",
            "params": {"state": captured["state"]}
        }),
    ));
    let candidate = &listed["candidates"][0];
    let encoded = serde_json::to_string(candidate).expect("candidate JSON");
    assert_eq!(candidate["summary"], "coordination: STATUS_NOT_POSTED");
    assert!(candidate.get("details").is_none());
    assert!(candidate.get("dedupe_key").is_none());
    assert!(candidate.get("evidence_digest").is_none());
    assert!(!encoded.contains("customer-8675309"));
    assert!(!encoded.contains(&private_root));
    assert!(!encoded.contains("ghp_"));
    assert!(!encoded.contains("alice@example.com"));
    assert!(candidate["issue_preview"]["body"]
        .as_str()
        .is_some_and(|body| body.contains("BOARD_STATUS_MISSING")));
}

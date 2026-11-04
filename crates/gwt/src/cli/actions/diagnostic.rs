//! Fixed historical Windows diagnosis; UUIDs and digests never grant authority.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use fs2::FileExt;
use gwt_github::SpecOpsError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::{CliEnv, CliParseError};

pub const PROFILE_ID: &str = "windows-4777-restore-f351-v1";
pub const WORKFLOW: &str = "windows-diagnostic.yml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: String,
    pub profile_id: String,
    pub expected_profile_digest: String,
    pub preexecution_of: Option<String>,
    pub expected_predecessor_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observe {
    pub request_id: Option<String>,
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub request_id: String,
    pub expected_run_id: u64,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCommand {
    Request(Request),
    Observe(Observe),
    Cancel(Cancel),
}

fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, CliParseError> {
    serde_json::from_value(value.clone()).map_err(|_| invalid())
}

fn invalid() -> CliParseError {
    CliParseError::InvalidJson(
        "invalid fixed diagnostic contract; use the published profile and exact typed fields"
            .into(),
    )
}

pub(crate) fn parse(operation: &str, value: &Value) -> Result<DiagnosticCommand, CliParseError> {
    match operation {
        "actions.diagnostic.request" => {
            let request: Request = decode(value)?;
            if !uuid(&request.request_id)
                || request.profile_id != PROFILE_ID
                || !sha256(&request.expected_profile_digest)
                || match (
                    &request.preexecution_of,
                    request.expected_predecessor_revision,
                ) {
                    (None, None) => false,
                    (Some(id), Some(revision)) => {
                        !uuid(id) || id == &request.request_id || revision == 0
                    }
                    _ => true,
                }
            {
                return Err(invalid());
            }
            Ok(DiagnosticCommand::Request(request))
        }
        "actions.diagnostic.observe" => {
            let observe: Observe = decode(value)?;
            if !matches!((&observe.request_id, &observe.profile_id),
                (Some(id), None) if uuid(id))
                && !matches!((&observe.request_id, &observe.profile_id),
                    (None, Some(id)) if id == PROFILE_ID)
            {
                return Err(invalid());
            }
            Ok(DiagnosticCommand::Observe(observe))
        }
        "actions.diagnostic.cancel" => {
            let cancel: Cancel = decode(value)?;
            if !uuid(&cancel.request_id)
                || cancel.expected_run_id == 0
                || cancel.expected_revision == 0
            {
                return Err(invalid());
            }
            Ok(DiagnosticCommand::Cancel(cancel))
        }
        _ => Err(invalid()),
    }
}

pub(crate) fn run<E: CliEnv>(
    env: &mut E,
    command: DiagnosticCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let value = env
        .actions_diagnostic(command)
        .map_err(super::super::io_as_api_error)?;
    out.push_str(&serde_json::to_string(&value).map_err(super::super::serde_as_api_error)?);
    out.push('\n');
    Ok(0)
}

pub(crate) fn unavailable() -> io::Error {
    io::Error::other(
        "diagnostic runtime unavailable; inspect the published profile before requesting execution",
    )
}

const PROFILE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../.github/diagnostics/windows/4777-restore-f351-v1/profile.json"
));
const PATCH_SHA256: &str = "8cf3285aa97e628d0d52bd95924bec5ce71461302e3ada5de4daa5e39cd3fc98";
const REPOSITORY: &str = "akiojin/gwt";
const OVERLAY_TREE: &str = "21bdb55935bb3223f391d505fcc2e619e4015d0c";
const WORKFLOW_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../.github/workflows/windows-diagnostic.yml"
));
const HARNESS_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../.github/diagnostics/windows/4777-restore-f351-v1/run.ps1"
));

fn digest(bytes: &[u8]) -> String {
    gwt_core::pm_authorization::digest(bytes)
}
fn commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Published {
    repository: String,
    default_branch: String,
    workflow_commit: String,
    workflow_id: u64,
    profile_sha256: String,
    patch_sha256: String,
    workflow_sha256: Option<String>,
    harness_sha256: Option<String>,
    published: bool,
}

impl Published {
    fn valid(&self) -> bool {
        self.repository == REPOSITORY
            && self.published
            && self.workflow_id > 0
            && !self.default_branch.is_empty()
            && !self.default_branch.chars().any(char::is_control)
            && commit(&self.workflow_commit)
            && self.profile_sha256 == digest(PROFILE_BYTES)
            && self.patch_sha256 == PATCH_SHA256
            && self.workflow_sha256.as_deref() == Some(digest(WORKFLOW_BYTES).as_str())
            && self.harness_sha256.as_deref() == Some(digest(HARNESS_BYTES).as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    request: Request,
    repository: String,
    actor: super::ReleasePolicyContext,
    published: Published,
    created_at: String,
    revision: u64,
    state: String,
    run_id: Option<u64>,
    cancel_requested: bool,
    cancel_outcome: Option<String>,
    no_start_verified: bool,
    result: Option<Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    content_hash: String,
    receipt: Receipt,
}

fn root(repo: &Path) -> PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo)
        .join("project-state/actions-diagnostic-requests")
}
fn record_path(root: &Path, id: &str) -> PathBuf {
    root.join(id).join("receipt.json")
}

fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn durable_dir(path: &Path) -> io::Result<()> {
    if !path.exists() {
        let parent = path.parent().ok_or_else(unavailable)?;
        durable_dir(parent)?;
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        sync_dir(parent)?;
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(unavailable());
    }
    sync_dir(path)
}

fn publish(path: &Path, bytes: &[u8], create: bool) -> io::Result<()> {
    let parent = path.parent().ok_or_else(unavailable)?;
    durable_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, MOVEFILE_REPLACE_EXISTING,
                MOVEFILE_WRITE_THROUGH,
            },
        };
        let from: Vec<u16> = temp
            .path()
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = if create {
            MOVEFILE_WRITE_THROUGH
        } else {
            MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING
        };
        // Both NUL-terminated buffers outlive the calls. As in tempfile's
        // Windows publisher, remove FILE_ATTRIBUTE_TEMPORARY before moving.
        // A same-directory write-through move preserves create-new/replace
        // behavior and never uses COPY_ALLOWED or delayed publication.
        unsafe {
            SetFileAttributesW(PCWSTR(from.as_ptr()), FILE_ATTRIBUTE_NORMAL)
                .map_err(|_| unavailable())?;
            MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), flags)
                .map_err(|_| unavailable())?;
        }
        temp.disable_cleanup(true);
        Ok(())
    }
    #[cfg(not(windows))]
    if create {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        temp.persist(path).map_err(|e| e.error)?;
    }
    #[cfg(not(windows))]
    sync_dir(parent)
}

fn save(root: &Path, record: &Receipt, create: bool) -> io::Result<()> {
    let bytes = serde_json::to_vec(record).map_err(|_| unavailable())?;
    let stored = Stored {
        content_hash: digest(&bytes),
        receipt: record.clone(),
    };
    let encoded = serde_json::to_vec(&stored).map_err(|_| unavailable())?;
    if create {
        publish(
            &root.join(&record.request.request_id).join("initial.json"),
            &encoded,
            true,
        )?;
    }
    publish(
        &record_path(root, &record.request.request_id),
        &encoded,
        create,
    )
}

fn load(root: &Path, id: &str) -> io::Result<Option<Receipt>> {
    let path = record_path(root, id);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if meta.file_type().is_file() && meta.len() <= 1024 * 1024 => {}
        _ => return Err(unavailable()),
    }
    let stored: Stored = serde_json::from_slice(&fs::read(path)?).map_err(|_| unavailable())?;
    let record = stored.receipt;
    let initial_path = root.join(id).join("initial.json");
    let meta = fs::symlink_metadata(&initial_path)?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err(unavailable());
    }
    let initial: Stored =
        serde_json::from_slice(&fs::read(initial_path)?).map_err(|_| unavailable())?;
    if stored.content_hash != digest(&serde_json::to_vec(&record).map_err(|_| unavailable())?)
        || initial.content_hash
            != digest(&serde_json::to_vec(&initial.receipt).map_err(|_| unavailable())?)
        || initial.receipt.request != record.request
        || initial.receipt.actor != record.actor
        || initial.receipt.published != record.published
        || initial.receipt.created_at != record.created_at
        || record.schema_version != 1
        || record.request.request_id != id
        || !uuid(id)
        || record.repository != REPOSITORY
        || !record.published.valid()
        || record.revision == 0
        || !matches!(
            record.state.as_str(),
            "Prepared"
                | "AttemptCommitted"
                | "DispatchUnknown"
                | "AckBound"
                | "BoundQueued"
                | "Running"
                | "TerminalObserved"
                | "ResultAccepted"
                | "Rejected"
                | "CleanupUnobserved"
                | "ExpiredUnknown"
        )
    {
        return Err(unavailable());
    }
    Ok(Some(record))
}

fn response(state: &str, reason: &str) -> Value {
    json!({"schema_version":1,"state":state,"reason_enum":reason,
        "diagnostic_only":true,"ready_evidence":false,"uncertainty":state.contains("Unknown") || state == "CleanupUnobserved",
        "requirement":{"registered_pm":true,"host_proof":"fresh_same_issuer_session_generation","repository":REPOSITORY,"workflow":WORKFLOW,"profile_id":PROFILE_ID,"profile_digest":digest(PROFILE_BYTES),"source_observations":1,"request_posts":1},
        "current_observation":{},
        "recovery":{"operation":"actions.diagnostic.observe","authority":"pm.capabilities; use the current registered repository PM and canonical Host context","workflow_allowlist":{"status_operation":"actions.diagnostic.policy.status","add_operation":"actions.diagnostic.policy.add","owner_action":"Read diagnostic policy status, then use its current token and exact scoped instruction_id or native approval_id with a fresh request_id. This permits only akiojin/gwt/windows-diagnostic.yml; workflow submission remains separate."},"unknown":"preserve the request and inspect its exact run; never resend or reset the source budget"}})
}

fn view(record: &Receipt) -> Value {
    let state = if record.state == "AttemptCommitted" {
        "DispatchUnknown"
    } else {
        &record.state
    };
    let mut value = response(
        state,
        if state == "DispatchUnknown" {
            "DispatchUnconfirmed"
        } else {
            "Observed"
        },
    );
    value["request_id"] = json!(record.request.request_id);
    value["revision"] = json!(record.revision);
    value["binding"] = json!({"repository":record.repository,"workflow_commit":record.published.workflow_commit,"run_id":record.run_id});
    value["cancel_requested"] = json!(record.cancel_requested);
    value["cancel_outcome"] = json!(record.cancel_outcome);
    if record.cancel_outcome.as_deref() == Some("Unknown") {
        value["uncertainty"] = json!(true);
    }
    value["result"] = json!(record.result);
    value
}

fn refusal(record: &Receipt, reason: &str) -> Value {
    let mut value = view(record);
    value["state"] = json!("Refused");
    value["reason_enum"] = json!(reason);
    value["current_observation"] = json!({"revision":record.revision,"run_id":record.run_id});
    value
}

fn uncertain(record: &Receipt, reason: &str) -> Value {
    let mut value = refusal(record, reason);
    value["state"] = json!("ExpiredUnknown");
    value["uncertainty"] = json!(true);
    value
}

fn allowed() -> io::Result<bool> {
    let path = gwt_core::paths::gwt_config_path();
    if !path.exists() {
        return Ok(false);
    }
    let settings = gwt_config::Settings::load_from_path(&path).map_err(|_| unavailable())?;
    Ok(settings
        .allowed_workflows
        .get(REPOSITORY)
        .is_some_and(|items| items.iter().any(|item| item == WORKFLOW)))
}

fn published<E: CliEnv + ?Sized>(env: &mut E) -> io::Result<Published> {
    let (owner, repo) = env.github_repository().map_err(|_| unavailable())?;
    if format!("{owner}/{repo}") != REPOSITORY {
        return Err(unavailable());
    }
    let published: Published = serde_json::from_value(
        env.diagnostic_workflow_context()
            .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    if !published.valid() {
        return Err(unavailable());
    }
    Ok(published)
}

pub(crate) fn execute<E: CliEnv + ?Sized>(
    env: &mut E,
    command: DiagnosticCommand,
) -> io::Result<Value> {
    if let DiagnosticCommand::Observe(Observe {
        profile_id: Some(_),
        ..
    }) = command
    {
        let context = env.release_policy_context();
        let remote = published(env);
        let permission = allowed();
        let mut value = response("ProfileObserved", "ProfileObserved");
        value["current_observation"] = json!({"host_verified":context.is_ok(),"allowed":permission.ok(),"published":remote.as_ref().ok().map(|p|p.published),"workflow_commit":remote.as_ref().ok().map(|p| &p.workflow_commit)});
        return Ok(value);
    }
    let actor = match env.release_policy_context() {
        Ok(actor) => actor,
        Err(_) => return Ok(response("Refused", "PermissionRequired")),
    };
    let root = root(env.repo_path());
    durable_dir(&root).map_err(|_| unavailable())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("intent.lock"))
        .map_err(|_| unavailable())?;
    lock.lock_exclusive().map_err(|_| unavailable())?;
    match command {
        DiagnosticCommand::Request(request) => {
            let remote = match published(env) {
                Ok(remote) => remote,
                Err(_) => return Ok(response("Refused", "RuntimeUnavailable")),
            };
            if !allowed().unwrap_or(false) {
                return Ok(response("Refused", "WorkflowNotAllowed"));
            }
            submit(env, &root, actor, remote, request)
        }
        DiagnosticCommand::Observe(observe) => {
            let id = observe.request_id.as_deref().ok_or_else(unavailable)?;
            let Some(record) = load(&root, id)? else {
                return Ok(response("NotFound", "RequestUnknown"));
            };
            observe_request(env, &root, record)
        }
        DiagnosticCommand::Cancel(cancel) => cancel_request(env, &root, &actor, cancel),
    }
}

fn submit<E: CliEnv + ?Sized>(
    env: &mut E,
    root: &Path,
    actor: super::ReleasePolicyContext,
    remote: Published,
    request: Request,
) -> io::Result<Value> {
    if request.expected_profile_digest != digest(PROFILE_BYTES) {
        return Ok(response("Refused", "ProfileMismatch"));
    }
    let existing = load(root, &request.request_id)?;
    if let Some(record) = &existing {
        if record.request != request {
            return Ok(refusal(record, "RequestConflict"));
        }
        if record.state != "Prepared" {
            return Ok(view(record));
        }
        if record.actor != actor {
            return Ok(response("Refused", "AuthorityChanged"));
        }
    }
    let budget_path = root.join("intent.json");
    let mut budget: Vec<String> = match fs::read(&budget_path) {
        Ok(bytes) if bytes.len() <= 4096 => {
            serde_json::from_slice(&bytes).map_err(|_| unavailable())?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        _ => return Err(unavailable()),
    };
    if budget.len() > 2 || budget.iter().any(|id| !uuid(id)) {
        return Err(unavailable());
    }
    if !budget.contains(&request.request_id) {
        let correction = budget.len() == 1
            && request.preexecution_of.as_ref() == budget.first()
            && load(root, &budget[0])?.is_some_and(|record| {
                record.no_start_verified
                    && record.revision == request.expected_predecessor_revision.unwrap_or(0)
            });
        if (!budget.is_empty() && !correction)
            || (budget.is_empty() && request.preexecution_of.is_some())
        {
            let mut value = response(
                "Refused",
                if budget.len() == 2 {
                    "BudgetExhausted"
                } else {
                    "SemanticDuplicate"
                },
            );
            value["current_observation"] =
                json!({"reserved_request_ids":budget,"correction_used":budget.len()==2});
            return Ok(value);
        }
        budget.push(request.request_id.clone());
    }
    if published(env).ok().as_ref() != Some(&remote) {
        return Ok(response("Refused", "RefDrift"));
    }
    // Revalidate authority after the remote reads. This is the acceptance
    // point, before reserving any source budget or committing a POST attempt.
    if env.release_policy_context().ok().as_ref() != Some(&actor) || !allowed().unwrap_or(false) {
        return Ok(response("Refused", "AuthorityChanged"));
    }
    let mut record = existing.unwrap_or_else(|| Receipt {
        schema_version: 1,
        request,
        repository: REPOSITORY.into(),
        actor: actor.clone(),
        published: remote.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        revision: 1,
        state: "Prepared".into(),
        run_id: None,
        cancel_requested: false,
        cancel_outcome: None,
        no_start_verified: false,
        result: None,
    });
    if !record_path(root, &record.request.request_id).exists() {
        save(root, &record, true).map_err(|_| unavailable())?;
    }
    publish(
        &budget_path,
        &serde_json::to_vec(&budget).map_err(|_| unavailable())?,
        !budget_path.exists(),
    )
    .map_err(|_| unavailable())?;
    // The initial workflow tuple remains immutable across a Prepared replay.
    // A current branch advancement with identical trusted assets is handled
    // by the wrapper's RefDrift/NoStart result, using this original expected D.
    record.revision += 1;
    record.state = "AttemptCommitted".into();
    save(root, &record, false).map_err(|_| unavailable())?;
    let inputs = json!({"request_id":record.request.request_id,"expected_profile_digest":record.request.expected_profile_digest,"expected_workflow_commit":record.published.workflow_commit});
    match env.dispatch_diagnostic_workflow(
        &record.published.default_branch,
        inputs.as_object().expect("fixed inputs object"),
    ) {
        Ok(ack)
            if ack.workflow_run_id > 0
                && ack.run_url
                    == format!(
                        "https://api.github.com/repos/{REPOSITORY}/actions/runs/{}",
                        ack.workflow_run_id
                    )
                && ack.html_url
                    == format!(
                        "https://github.com/{REPOSITORY}/actions/runs/{}",
                        ack.workflow_run_id
                    ) =>
        {
            record.run_id = Some(ack.workflow_run_id);
            record.state = "AckBound".into();
        }
        _ => record.state = "DispatchUnknown".into(),
    }
    record.revision += 1;
    if save(root, &record, false).is_err() {
        record.run_id = None;
        record.state = "DispatchUnknown".into();
    }
    Ok(view(&record))
}

fn run_matches(snapshot: &Value, record: &Receipt) -> bool {
    let run = &snapshot["run"];
    run["id"].as_u64() == record.run_id
        && run["workflow_id"].as_u64() == Some(record.published.workflow_id)
        && run["run_attempt"].as_u64() == Some(1)
        && run["event"].as_str() == Some("workflow_dispatch")
        && run["path"].as_str().is_some_and(|p| {
            p.split('@').next() == Some(".github/workflows/windows-diagnostic.yml")
        })
        && run["repository"]["full_name"].as_str() == Some(REPOSITORY)
        && run["head_sha"].as_str().is_some_and(commit)
}

fn observe_request<E: CliEnv + ?Sized>(
    env: &mut E,
    root: &Path,
    mut record: Receipt,
) -> io::Result<Value> {
    let Some(run_id) = record.run_id else {
        return Ok(view(&record));
    };
    let snapshot = match env.diagnostic_run_snapshot(run_id) {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok(uncertain(&record, "RunUnavailable")),
    };
    if !run_matches(&snapshot, &record) {
        return Ok(refusal(&record, "RunIdentityMismatch"));
    }
    let next = match snapshot["run"]["status"].as_str() {
        Some("queued" | "waiting" | "pending" | "requested") => "BoundQueued",
        Some("in_progress") => "Running",
        Some("completed") => "TerminalObserved",
        _ => return Ok(uncertain(&record, "RunUnavailable")),
    };
    if record.result.is_some() {
        return Ok(view(&record));
    }
    if record.state != next {
        record.state = next.into();
        record.revision += 1;
        save(root, &record, false).map_err(|_| unavailable())?;
    }
    if next == "TerminalObserved" {
        if !trusted_assets(&snapshot) {
            return Ok(refusal(&record, "WorkflowIdentityMismatch"));
        }
        let name = format!("windows-diagnostic-{}", record.request.request_id);
        let Some(artifacts) = snapshot["artifacts"].as_array() else {
            return Ok(uncertain(&record, "ArtifactUnavailable"));
        };
        let matching: Vec<_> = artifacts
            .iter()
            .filter(|a| a["name"].as_str() == Some(&name))
            .collect();
        if matching.len() != 1 || matching[0]["expired"].as_bool() != Some(false) {
            return Ok(uncertain(&record, "ArtifactUnavailable"));
        }
        let Some(artifact_id) = matching[0]["id"].as_u64().filter(|id| *id > 0) else {
            return Ok(refusal(&record, "ArtifactRejected"));
        };
        let bytes = match env.diagnostic_artifact(run_id, artifact_id) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(uncertain(&record, "ArtifactUnavailable")),
        };
        let result = match read_result(&bytes, &record, &snapshot) {
            Ok(result) => result,
            Err(_) => return Ok(refusal(&record, "ArtifactRejected")),
        };
        record.no_start_verified = result.no_start_verified;
        record.state = if result.cleanup == "Unobserved" {
            "CleanupUnobserved"
        } else {
            "ResultAccepted"
        }
        .into();
        let result = serde_json::to_value(result).map_err(|_| unavailable())?;
        record.result = Some(
            json!({"receipt":result,"artifact_id":artifact_id,"artifact_bytes":bytes.len(),"artifact_sha256":digest(&bytes)}),
        );
        record.revision += 1;
        save(root, &record, false).map_err(|_| unavailable())?;
    }
    Ok(view(&record))
}

fn trusted_assets(snapshot: &Value) -> bool {
    snapshot["profile_sha256"].as_str() == Some(digest(PROFILE_BYTES).as_str())
        && snapshot["patch_sha256"].as_str() == Some(PATCH_SHA256)
        && snapshot["workflow_sha256"].as_str() == Some(digest(WORKFLOW_BYTES).as_str())
        && snapshot["harness_sha256"].as_str() == Some(digest(HARNESS_BYTES).as_str())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDocument {
    schema_version: u32,
    request_id: String,
    profile_id: String,
    profile_digest: String,
    repository: String,
    workflow_commit: String,
    run_id: u64,
    run_attempt: u32,
    source_commit: Option<String>,
    source_tree: Option<String>,
    source_lock_sha256: Option<String>,
    overlay_tree: Option<String>,
    observation_started: bool,
    phase: String,
    reason: String,
    cleanup: String,
    restore: Option<Restore>,
    cases: Vec<Case>,
    binaries: Vec<Binary>,
    image: Image,
    no_start_verified: bool,
    diagnostic_only: bool,
    ready_evidence: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    before_bytes: Option<u64>,
    before_sha256: Option<String>,
    after_bytes: Option<u64>,
    after_sha256: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    provider: String,
    case: String,
    outcome: String,
    exit_code: Option<i64>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binary {
    kind: String,
    sha256: String,
    source_commit: String,
    source_tree: String,
    overlay_tree: String,
    target: String,
    profile: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    os: String,
    arch: Option<String>,
    rust_version: Option<String>,
    node_major: Option<u64>,
    node_version: Option<String>,
    runner_label: String,
    image_version: Option<String>,
    os_build: Option<u64>,
}

fn read_result(bytes: &[u8], record: &Receipt, snapshot: &Value) -> io::Result<ResultDocument> {
    use std::io::Read;
    const LIMIT: usize = 1024 * 1024;
    if bytes.len() > LIMIT {
        return Err(unavailable());
    }
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| unavailable())?;
    if archive.len() != 1 {
        return Err(unavailable());
    }
    let file = archive.by_index(0).map_err(|_| unavailable())?;
    if !file.name().is_ok_and(|name| name == "result.json")
        || file.enclosed_name().is_none()
        || file.size() > LIMIT as u64
        || file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 != 0o100000 && mode & 0o170000 != 0)
    {
        return Err(unavailable());
    }
    let mut raw = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| unavailable())?;
    if raw.len() > LIMIT {
        return Err(unavailable());
    }
    let doc: ResultDocument = serde_json::from_slice(&raw).map_err(|_| unavailable())?;
    if !result_valid(&doc, record, snapshot) {
        return Err(unavailable());
    }
    Ok(doc)
}

fn result_valid(doc: &ResultDocument, record: &Receipt, snapshot: &Value) -> bool {
    let profile: Value = serde_json::from_slice(PROFILE_BYTES).expect("embedded fixed profile");
    let matches_optional = |actual: &Option<String>, expected: &Value| {
        actual
            .as_ref()
            .is_none_or(|a| expected.as_str() == Some(a.as_str()))
    };
    if doc.schema_version != 1
        || doc.request_id != record.request.request_id
        || doc.profile_id != PROFILE_ID
        || doc.profile_digest != digest(PROFILE_BYTES)
        || doc.repository != REPOSITORY
        || doc.run_id != record.run_id.unwrap_or(0)
        || doc.run_attempt != 1
        || snapshot["run"]["head_sha"].as_str() != Some(doc.workflow_commit.as_str())
        || !doc.diagnostic_only
        || doc.ready_evidence
        || !matches_optional(&doc.source_commit, &profile["source_commit"])
        || !matches_optional(&doc.source_tree, &profile["source_tree"])
        || !matches_optional(&doc.source_lock_sha256, &profile["source_lock"]["sha256"])
        || doc
            .overlay_tree
            .as_deref()
            .is_some_and(|tree| tree != OVERLAY_TREE)
        || !matches!(
            doc.phase.as_str(),
            "setup" | "compile" | "contracts" | "providers" | "restore" | "result"
        )
        || !matches!(
            doc.reason.as_str(),
            "Completed"
                | "RefDrift"
                | "InputMismatch"
                | "SetupNotStarted"
                | "SetupFailed"
                | "CompileFailed"
                | "ContractsFailed"
                | "CasesFailed"
                | "WrapperTimeout"
                | "CleanupUnobserved"
        )
        || !matches!(
            doc.cleanup.as_str(),
            "ObservedClean" | "Unobserved" | "NotStarted"
        )
    {
        return false;
    }
    let image = &doc.image;
    if image.os != "windows"
        || image.runner_label != "windows-latest"
        || image
            .arch
            .as_deref()
            .is_some_and(|a| !matches!(a, "x86_64" | "aarch64"))
        || image.rust_version.as_deref().is_some_and(|v| v != "1.99.1")
        || image.node_major.is_some_and(|n| n != 22)
        || image.node_version.as_ref().is_some_and(|v| {
            v.len() > 32
                || !semver::Version::parse(v)
                    .is_ok_and(|v| v.major == 22 && v.pre.is_empty() && v.build.is_empty())
        })
        || image.image_version.as_ref().is_some_and(|v| {
            v.is_empty() || v.len() > 64 || !v.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
    {
        return false;
    }
    if doc.cases.len() != 4 {
        return false;
    }
    let mut cases = std::collections::BTreeSet::new();
    for case in &doc.cases {
        if !matches!(case.provider.as_str(), "codex" | "claude")
            || !matches!(case.case.as_str(), "official_launch" | "restored_close")
            || !matches!(
                case.outcome.as_str(),
                "pass" | "fail" | "not_run" | "unknown"
            )
            || !cases.insert((&case.provider, &case.case))
            || case
                .exit_code
                .is_some_and(|code| code < i32::MIN as i64 || code > u32::MAX as i64)
            || (case.outcome == "not_run" && case.exit_code.is_some())
        {
            return false;
        }
    }
    for provider in ["codex", "claude"] {
        let mut cases = doc.cases.iter().filter(|case| case.provider == provider);
        if cases.next().map(|case| case.exit_code) != cases.next().map(|case| case.exit_code) {
            return false;
        }
    }
    let restored = doc.restore.as_ref().is_some_and(|r| {
        r.before_bytes == Some(101079)
            && r.after_bytes == Some(101079)
            && r.before_sha256.as_deref() == profile["before"]["sha256"].as_str()
            && r.after_sha256 == r.before_sha256
    });
    if let Some(restore) = &doc.restore {
        if restore
            .before_sha256
            .as_deref()
            .is_some_and(|hash| !sha256(hash))
            || restore
                .after_sha256
                .as_deref()
                .is_some_and(|hash| !sha256(hash))
        {
            return false;
        }
    }
    if doc.binaries.len() > 3 {
        return false;
    }
    let mut binaries = std::collections::BTreeSet::new();
    for binary in &doc.binaries {
        if !matches!(binary.kind.as_str(), "gwt" | "gwtd" | "test")
            || !binaries.insert(&binary.kind)
            || !sha256(&binary.sha256)
            || Some(binary.source_commit.as_str()) != profile["source_commit"].as_str()
            || Some(binary.source_tree.as_str()) != profile["source_tree"].as_str()
            || binary.overlay_tree != OVERLAY_TREE
            || !matches!(
                binary.target.as_str(),
                "x86_64-pc-windows-msvc" | "aarch64-pc-windows-msvc"
            )
            || binary.profile != "test"
        {
            return false;
        }
    }
    if doc.no_start_verified {
        if doc.observation_started
            || doc.phase != "setup"
            || doc.cleanup != "NotStarted"
            || !doc.binaries.is_empty()
            || doc.cases.iter().any(|case| case.outcome != "not_run")
            || !matches!(
                doc.reason.as_str(),
                "RefDrift" | "InputMismatch" | "SetupNotStarted"
            )
        {
            return false;
        }
    } else if doc.workflow_commit != record.published.workflow_commit {
        return false;
    }
    if doc.cleanup == "NotStarted" && !doc.no_start_verified {
        return false;
    }
    if doc.cleanup == "ObservedClean" && doc.observation_started && !restored {
        return false;
    }
    if matches!(doc.reason.as_str(), "Completed" | "CasesFailed")
        && (!doc.observation_started
            || doc.cleanup != "ObservedClean"
            || !restored
            || doc.binaries.len() != 3
            || doc.source_commit.is_none()
            || doc.source_tree.is_none()
            || doc.source_lock_sha256.is_none()
            || doc.overlay_tree.is_none()
            || image.arch.is_none()
            || image.rust_version.is_none()
            || image.node_version.is_none()
            || (doc.reason == "Completed"
                && doc
                    .cases
                    .iter()
                    .any(|case| case.outcome != "pass" || case.exit_code != Some(0))))
    {
        return false;
    }
    true
}

fn cancel_request<E: CliEnv + ?Sized>(
    env: &mut E,
    root: &Path,
    actor: &super::ReleasePolicyContext,
    cancel: Cancel,
) -> io::Result<Value> {
    let Some(mut record) = load(root, &cancel.request_id)? else {
        return Ok(response("NotFound", "RequestUnknown"));
    };
    if record.revision != cancel.expected_revision {
        return Ok(refusal(&record, "StaleRevision"));
    }
    if record.run_id != Some(cancel.expected_run_id) {
        return Ok(refusal(&record, "RunIdentityMismatch"));
    }
    if record.cancel_requested {
        return Ok(view(&record));
    }
    let snapshot = match env.diagnostic_run_snapshot(cancel.expected_run_id) {
        Ok(snapshot) => snapshot,
        Err(_) => return Ok(uncertain(&record, "RunUnavailable")),
    };
    if !run_matches(&snapshot, &record) {
        return Ok(refusal(&record, "RunIdentityMismatch"));
    }
    if !matches!(
        snapshot["run"]["status"].as_str(),
        Some("queued" | "waiting" | "pending" | "requested" | "in_progress")
    ) {
        return Ok(refusal(&record, "RunNotLive"));
    }
    if env.release_policy_context().ok().as_ref() != Some(actor) {
        return Ok(refusal(&record, "AuthorityChanged"));
    }
    record.cancel_requested = true;
    record.cancel_outcome = Some("Unknown".into());
    record.revision += 1;
    save(root, &record, false).map_err(|_| unavailable())?;
    if env.cancel_actions(cancel.expected_run_id).is_ok() {
        record.cancel_outcome = Some("Requested".into());
        record.revision += 1;
        if save(root, &record, false).is_err() {
            record.cancel_outcome = Some("Unknown".into());
        }
    }
    Ok(view(&record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(windows)]
    #[test]
    fn windows_publisher_preserves_create_replace_and_normal_file_attributes() {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                GetFileAttributesW, FILE_ATTRIBUTE_TEMPORARY, INVALID_FILE_ATTRIBUTES,
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        publish(&path, b"prepared", true).unwrap();
        assert!(publish(&path, b"conflicting", true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"prepared");
        publish(&path, b"attempt-committed", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"attempt-committed");
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // The live NUL-terminated buffer names this test's own file.
        let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        assert_ne!(attributes, INVALID_FILE_ATTRIBUTES);
        assert_eq!(attributes & FILE_ATTRIBUTE_TEMPORARY.0, 0);
    }

    fn request(id: &str) -> DiagnosticCommand {
        DiagnosticCommand::Request(Request {
            request_id: id.into(),
            profile_id: PROFILE_ID.into(),
            expected_profile_digest: gwt_core::pm_authorization::digest(include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.github/diagnostics/windows/4777-restore-f351-v1/profile.json"
            ))),
            preexecution_of: None,
            expected_predecessor_revision: None,
        })
    }

    fn permitted(repo: &std::path::Path) -> crate::cli::TestEnv {
        let config = gwt_core::paths::gwt_config_path();
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            "[allowed_workflows]\n\"akiojin/gwt\"=[\"windows-diagnostic.yml\"]\n",
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(repo.to_path_buf());
        env.diagnostic_repository = Some(("akiojin".into(), "gwt".into()));
        env.release_policy_context_hook = Some(Box::new(|| {
            Ok(crate::cli::ReleasePolicyContext {
                host_instance_id: "fixture-host".into(),
                session_id: "fixture-pm".into(),
                capability_generation: 0,
                worktree_digest: "a".repeat(64),
            })
        }));
        env.diagnostic_context = Some(
            json!({"repository":"akiojin/gwt","default_branch":"develop", "workflow_commit":"a".repeat(40), "workflow_id":123,
            "profile_sha256":gwt_core::pm_authorization::digest(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.github/diagnostics/windows/4777-restore-f351-v1/profile.json"))),
            "patch_sha256":"8cf3285aa97e628d0d52bd95924bec5ce71461302e3ada5de4daa5e39cd3fc98", "workflow_sha256":digest(WORKFLOW_BYTES),"harness_sha256":digest(HARNESS_BYTES), "published":true}),
        );
        env
    }

    #[test]
    fn lost_dispatch_ack_is_durable_and_never_resubmitted_under_a_new_uuid() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = permitted(repo.path());
        let id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4e";
        env.diagnostic_dispatch_results
            .push_back(Err(io::Error::other("private-provider-sentinel")));
        let result = execute(&mut env, request(id)).unwrap();
        assert_eq!(result["state"], "DispatchUnknown");
        // A crash after committing the attempt but before persisting its ACK
        // leaves this durable marker. Recovery must not try the POST again.
        let request_root = root(repo.path());
        let mut committed = load(&request_root, id).unwrap().unwrap();
        committed.state = "AttemptCommitted".into();
        committed.revision = 2;
        committed.run_id = None;
        save(&request_root, &committed, false).unwrap();
        let mut restarted = permitted(repo.path());
        let replay = execute(&mut restarted, request(id)).unwrap();
        assert_eq!(replay["state"], "DispatchUnknown");
        assert_eq!(
            execute(
                &mut restarted,
                request("d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4f")
            )
            .unwrap()["reason_enum"],
            "SemanticDuplicate"
        );
        assert!(restarted.diagnostic_dispatches.is_empty());
        assert_eq!(env.diagnostic_dispatches.len(), 1);
        assert!(!result.to_string().contains("private-provider-sentinel"));
    }

    #[test]
    fn legal_pm_dispatch_binds_the_direct_ack_and_refuses_authority_drift() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = permitted(repo.path());
        env.diagnostic_dispatch_results.push_back(Ok(
            gwt_github::client::http::DiagnosticDispatchAck {
                workflow_run_id: 77,
                run_url: "https://api.github.com/repos/akiojin/gwt/actions/runs/77".into(),
                html_url: "https://github.com/akiojin/gwt/actions/runs/77".into(),
            },
        ));
        let id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4e";
        let result = execute(&mut env, request(id)).unwrap();
        assert_eq!(result["state"], "AckBound");
        assert_eq!(result["binding"]["run_id"], 77);
        assert_eq!(
            execute(&mut env, request(id)).unwrap()["binding"]["run_id"],
            77
        );
        assert_eq!(env.diagnostic_dispatches.len(), 1);
        let other_repo = tempfile::tempdir().unwrap();
        let mut revoked = permitted(other_repo.path());
        let mut calls = 0;
        revoked.release_policy_context_hook = Some(Box::new(move || {
            calls += 1;
            if calls == 1 {
                Ok(crate::cli::ReleasePolicyContext {
                    host_instance_id: "fixture-host".into(),
                    session_id: "fixture-pm".into(),
                    capability_generation: 0,
                    worktree_digest: "a".repeat(64),
                })
            } else {
                Err(io::Error::other("private-provider-sentinel"))
            }
        }));
        assert_eq!(
            execute(&mut revoked, request(id)).unwrap()["reason_enum"],
            "AuthorityChanged"
        );
        assert!(revoked.diagnostic_dispatches.is_empty());
        let mut recovered = permitted(other_repo.path());
        assert_eq!(
            execute(
                &mut recovered,
                request("d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4f")
            )
            .unwrap()["state"],
            "DispatchUnknown"
        );
        assert_eq!(recovered.diagnostic_dispatches.len(), 1);
    }

    #[test]
    fn exact_live_run_can_be_observed_and_cancelled_once_with_a_current_revision() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = permitted(repo.path());
        let id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4e";
        env.diagnostic_dispatch_results.push_back(Ok(
            gwt_github::client::http::DiagnosticDispatchAck {
                workflow_run_id: 77,
                run_url: "https://api.github.com/repos/akiojin/gwt/actions/runs/77".into(),
                html_url: "https://github.com/akiojin/gwt/actions/runs/77".into(),
            },
        ));
        let bound = execute(&mut env, request(id)).unwrap();
        for command in [
            DiagnosticCommand::Observe(Observe {
                request_id: Some(id.into()),
                profile_id: None,
            }),
            DiagnosticCommand::Cancel(Cancel {
                request_id: id.into(),
                expected_run_id: 77,
                expected_revision: bound["revision"].as_u64().unwrap(),
            }),
        ] {
            let unavailable = execute(&mut env, command).unwrap();
            assert_eq!(unavailable["state"], "ExpiredUnknown");
            assert_eq!(unavailable["request_id"], id);
            assert_eq!(unavailable["revision"], bound["revision"]);
            assert_eq!(unavailable["binding"], bound["binding"]);
            assert_eq!(unavailable["uncertainty"], true);
        }
        assert_eq!(
            load(&root(repo.path()), id).unwrap().unwrap().revision,
            bound["revision"].as_u64().unwrap()
        );
        assert!(env.cancel_call_log.is_empty());
        env.diagnostic_runs.insert(77, json!({"run":{"id":77,"workflow_id":123,"event":"workflow_dispatch","path":".github/workflows/windows-diagnostic.yml","head_sha":"a".repeat(40),"run_attempt":1,"status":"in_progress","repository":{"full_name":REPOSITORY}},"artifacts":[]}));
        let running = execute(
            &mut env,
            DiagnosticCommand::Observe(Observe {
                request_id: Some(id.into()),
                profile_id: None,
            }),
        )
        .unwrap();
        assert_eq!(running["state"], "Running");
        let revision = running["revision"].as_u64().unwrap();
        let stale = execute(
            &mut env,
            DiagnosticCommand::Cancel(Cancel {
                request_id: id.into(),
                expected_run_id: 77,
                expected_revision: revision - 1,
            }),
        )
        .unwrap();
        assert_eq!(stale["reason_enum"], "StaleRevision");
        let cancelled = execute(
            &mut env,
            DiagnosticCommand::Cancel(Cancel {
                request_id: id.into(),
                expected_run_id: 77,
                expected_revision: revision,
            }),
        )
        .unwrap();
        assert_eq!(cancelled["cancel_requested"], true);
        execute(
            &mut env,
            DiagnosticCommand::Cancel(Cancel {
                request_id: id.into(),
                expected_run_id: 77,
                expected_revision: cancelled["revision"].as_u64().unwrap(),
            }),
        )
        .unwrap();
        assert_eq!(env.cancel_call_log, [77]);
        env.diagnostic_runs.get_mut(&77).unwrap()["run"]["repository"]["full_name"] =
            json!("foreign/repo");
        let foreign = execute(
            &mut env,
            DiagnosticCommand::Observe(Observe {
                request_id: Some(id.into()),
                profile_id: None,
            }),
        )
        .unwrap();
        assert_eq!(foreign["reason_enum"], "RunIdentityMismatch");
        assert_eq!(env.cancel_call_log, [77]);
    }

    #[test]
    fn sealed_no_start_result_is_accepted_once_and_permits_only_one_correction() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = permitted(repo.path());
        let id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4e";
        env.diagnostic_dispatch_results.push_back(Ok(
            gwt_github::client::http::DiagnosticDispatchAck {
                workflow_run_id: 77,
                run_url: format!("https://api.github.com/repos/{REPOSITORY}/actions/runs/77"),
                html_url: format!("https://github.com/{REPOSITORY}/actions/runs/77"),
            },
        ));
        execute(&mut env, request(id)).unwrap();
        let mut cases = Vec::new();
        for provider in ["codex", "claude"] {
            for case in ["official_launch", "restored_close"] {
                cases.push(
                    json!({"provider":provider,"case":case,"outcome":"not_run","exit_code":null}),
                );
            }
        }
        let result = json!({"schema_version":1,"request_id":id,"profile_id":PROFILE_ID,"profile_digest":digest(PROFILE_BYTES),"repository":REPOSITORY,"workflow_commit":"b".repeat(40),"run_id":77,"run_attempt":1,
            "source_commit":null,"source_tree":null,"source_lock_sha256":null,"overlay_tree":null,"observation_started":false,"phase":"setup","reason":"RefDrift","cleanup":"NotStarted","restore":null,"cases":cases,"binaries":[],
            "image":{"os":"windows","arch":null,"rust_version":null,"node_major":null,"node_version":null,"runner_label":"windows-latest","image_version":null,"os_build":null},"no_start_verified":true,"diagnostic_only":true,"ready_evidence":false});
        let zip_result = |name: &str, result: &Value| {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&serde_json::to_vec(result).unwrap()).unwrap();
            zip.finish().unwrap().into_inner()
        };
        let bytes = zip_result("result.json", &result);
        env.diagnostic_artifacts.insert(99, bytes);
        env.diagnostic_runs.insert(77,json!({"run":{"id":77,"workflow_id":123,"event":"workflow_dispatch","path":".github/workflows/windows-diagnostic.yml","head_sha":"b".repeat(40),"run_attempt":1,"status":"completed","conclusion":"success","repository":{"full_name":REPOSITORY}},"artifacts":[{"id":99,"name":format!("windows-diagnostic-{id}"),"expired":false}],"profile_sha256":digest(PROFILE_BYTES),"patch_sha256":PATCH_SHA256,"workflow_sha256":digest(WORKFLOW_BYTES),"harness_sha256":digest(HARNESS_BYTES)}));
        let record = load(&root(repo.path()), id).unwrap().unwrap();
        let snapshot = &env.diagnostic_runs[&77];
        assert!(read_result(&zip_result("../result.json", &result), &record, snapshot).is_err());
        assert!(read_result(&vec![0; 1024 * 1024 + 1], &record, snapshot).is_err());
        let mut unsafe_result = result.clone();
        unsafe_result["private-sentinel"] = json!("private-provider-sentinel");
        let error = read_result(
            &zip_result("result.json", &unsafe_result),
            &record,
            snapshot,
        )
        .unwrap_err();
        assert!(!error.to_string().contains("private-provider-sentinel"));
        unsafe_result = result.clone();
        unsafe_result["request_id"] = json!("foreign");
        assert!(read_result(
            &zip_result("result.json", &unsafe_result),
            &record,
            snapshot
        )
        .is_err());
        unsafe_result = result.clone();
        unsafe_result["observation_started"] = json!(true);
        assert!(read_result(
            &zip_result("result.json", &unsafe_result),
            &record,
            snapshot
        )
        .is_err());
        // Both tests share one provider process. A passing case can carry101
        // because its sibling failed; that observed process exit is retained.
        let mut failed_cases = result.clone();
        let profile: Value = serde_json::from_slice(PROFILE_BYTES).unwrap();
        for key in ["source_commit", "source_tree"] {
            failed_cases[key] = profile[key].clone();
        }
        failed_cases["source_lock_sha256"] = profile["source_lock"]["sha256"].clone();
        failed_cases["overlay_tree"] = json!(OVERLAY_TREE);
        failed_cases["workflow_commit"] = json!("a".repeat(40));
        failed_cases["observation_started"] = json!(true);
        failed_cases["no_start_verified"] = json!(false);
        failed_cases["phase"] = json!("result");
        failed_cases["reason"] = json!("CasesFailed");
        failed_cases["cleanup"] = json!("ObservedClean");
        failed_cases["restore"] = json!({"before_bytes":101079,"after_bytes":101079,"before_sha256":profile["before"]["sha256"],"after_sha256":profile["before"]["sha256"]});
        failed_cases["image"]["arch"] = json!("x86_64");
        failed_cases["image"]["rust_version"] = json!("1.99.1");
        failed_cases["image"]["node_major"] = json!(22);
        failed_cases["image"]["node_version"] = json!("22.1.0");
        failed_cases["binaries"] = json!(["gwt","gwtd","test"].map(|kind|json!({"kind":kind,"sha256":"a".repeat(64),"source_commit":profile["source_commit"],"source_tree":profile["source_tree"],"overlay_tree":OVERLAY_TREE,"target":"x86_64-pc-windows-msvc","profile":"test"})));
        for (index, case) in failed_cases["cases"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            case["outcome"] = json!(if index % 2 == 0 { "pass" } else { "fail" });
            case["exit_code"] = json!(101);
        }
        let mut failure_snapshot = snapshot.clone();
        failure_snapshot["run"]["head_sha"] = json!("a".repeat(40));
        assert!(read_result(
            &zip_result("result.json", &failed_cases),
            &record,
            &failure_snapshot
        )
        .is_ok());
        let mut passed_cases = failed_cases.clone();
        passed_cases["reason"] = json!("Completed");
        for case in passed_cases["cases"].as_array_mut().unwrap() {
            case["outcome"] = json!("pass");
            case["exit_code"] = json!(0);
        }
        assert!(read_result(
            &zip_result("result.json", &passed_cases),
            &record,
            &failure_snapshot
        )
        .is_ok());
        let observe = || {
            DiagnosticCommand::Observe(Observe {
                request_id: Some(id.into()),
                profile_id: None,
            })
        };
        let exact_snapshot = env.diagnostic_runs[&77].clone();
        env.diagnostic_runs.get_mut(&77).unwrap()["run"]["run_attempt"] = json!(2);
        assert_eq!(
            execute(&mut env, observe()).unwrap()["reason_enum"],
            "RunIdentityMismatch"
        );
        env.diagnostic_runs.insert(77, exact_snapshot.clone());
        env.diagnostic_runs.get_mut(&77).unwrap()["profile_sha256"] = json!("0".repeat(64));
        assert_eq!(
            execute(&mut env, observe()).unwrap()["reason_enum"],
            "WorkflowIdentityMismatch"
        );
        env.diagnostic_runs.insert(77, exact_snapshot.clone());
        env.diagnostic_runs.get_mut(&77).unwrap()["artifacts"] = json!([]);
        assert_eq!(
            execute(&mut env, observe()).unwrap()["state"],
            "ExpiredUnknown"
        );
        env.diagnostic_runs.insert(77, exact_snapshot.clone());
        env.diagnostic_runs.get_mut(&77).unwrap()["artifacts"][0]["expired"] = json!(true);
        assert_eq!(
            execute(&mut env, observe()).unwrap()["state"],
            "ExpiredUnknown"
        );
        assert!(
            !load(&root(repo.path()), id)
                .unwrap()
                .unwrap()
                .no_start_verified
        );
        assert_eq!(
            execute(&mut env, request("d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4f")).unwrap()
                ["reason_enum"],
            "SemanticDuplicate"
        );
        assert_eq!(env.diagnostic_dispatches.len(), 1);
        env.diagnostic_runs.insert(77, exact_snapshot);
        let observed = execute(
            &mut env,
            DiagnosticCommand::Observe(Observe {
                request_id: Some(id.into()),
                profile_id: None,
            }),
        )
        .unwrap();
        assert_eq!(observed["state"], "ResultAccepted");
        let revision = observed["revision"].as_u64().unwrap();
        assert_eq!(
            execute(
                &mut env,
                DiagnosticCommand::Observe(Observe {
                    request_id: Some(id.into()),
                    profile_id: None
                })
            )
            .unwrap()["revision"],
            revision
        );
        let next_id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4f";
        let DiagnosticCommand::Request(mut correction) = request(next_id) else {
            unreachable!()
        };
        correction.preexecution_of = Some(id.into());
        correction.expected_predecessor_revision = Some(revision);
        env.diagnostic_dispatch_results
            .push_back(Err(io::Error::other("private-provider-sentinel")));
        assert_eq!(
            execute(&mut env, DiagnosticCommand::Request(correction)).unwrap()["state"],
            "DispatchUnknown"
        );
        assert_eq!(
            execute(&mut env, request("d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad40")).unwrap()
                ["reason_enum"],
            "BudgetExhausted"
        );
        assert_eq!(env.diagnostic_dispatches.len(), 2);
    }

    #[test]
    fn prepared_crash_recovery_keeps_the_original_workflow_binding_after_ref_drift() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = permitted(repo.path());
        let id = "d703dbb3-88e3-4e2d-a5b8-6a5d30a1ad4e";
        let DiagnosticCommand::Request(request) = request(id) else {
            unreachable!()
        };
        let prepared = Receipt {
            schema_version: 1,
            request: request.clone(),
            repository: REPOSITORY.into(),
            actor: env.release_policy_context().unwrap(),
            published: published(&mut env).unwrap(),
            created_at: "2026-11-04T00:00:00Z".into(),
            revision: 1,
            state: "Prepared".into(),
            run_id: None,
            cancel_requested: false,
            cancel_outcome: None,
            no_start_verified: false,
            result: None,
        };
        let root = root(repo.path());
        save(&root, &prepared, true).unwrap();
        publish(
            &root.join("intent.json"),
            &serde_json::to_vec(&vec![id]).unwrap(),
            true,
        )
        .unwrap();
        let initial = fs::read(root.join(id).join("initial.json")).unwrap();
        env.diagnostic_context.as_mut().unwrap()["workflow_commit"] = json!("b".repeat(40));
        env.diagnostic_dispatch_results.push_back(Ok(
            gwt_github::client::http::DiagnosticDispatchAck {
                workflow_run_id: 77,
                run_url: format!("https://api.github.com/repos/{REPOSITORY}/actions/runs/77"),
                html_url: format!("https://github.com/{REPOSITORY}/actions/runs/77"),
            },
        ));
        let result = execute(&mut env, DiagnosticCommand::Request(request)).unwrap();
        assert_eq!(result["state"], "AckBound");
        assert_eq!(
            load(&root, id).unwrap().unwrap().published.workflow_commit,
            "a".repeat(40)
        );
        assert_eq!(
            fs::read(root.join(id).join("initial.json")).unwrap(),
            initial
        );
        assert_eq!(
            env.diagnostic_dispatches[0]["expected_workflow_commit"],
            "a".repeat(40)
        );
        assert_eq!(env.diagnostic_dispatches.len(), 1);
    }

    #[test]
    fn profile_observation_reports_missing_authority_without_leaking_provider_errors() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let repo = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(repo.path().to_path_buf());
        env.release_policy_context_hook = Some(Box::new(|| {
            Err(io::Error::other("private-provider-sentinel"))
        }));
        let result = execute(
            &mut env,
            DiagnosticCommand::Observe(Observe {
                profile_id: Some(PROFILE_ID.into()),
                request_id: None,
            }),
        )
        .unwrap();
        assert_eq!(result["requirement"]["registered_pm"], true);
        assert_eq!(result["current_observation"]["host_verified"], false);
        assert_eq!(result["current_observation"]["published"], Value::Null);
        assert_eq!(
            result["recovery"]["operation"],
            "actions.diagnostic.observe"
        );
        assert!(!result.to_string().contains("private-provider-sentinel"));
        assert!(env.diagnostic_dispatches.is_empty());
        assert_eq!(json!(result["diagnostic_only"]), true);
        assert_eq!(
            response("CleanupUnobserved", "Observed")["uncertainty"],
            true
        );
    }
}

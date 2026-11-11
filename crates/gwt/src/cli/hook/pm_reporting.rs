//! PM-only reporting obligations. Ordinary Agent producing obligations stay separate.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

use super::HookOutput;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QaTurn {
    session_id: String,
    turn_id: String,
}

#[derive(Serialize, Deserialize)]
struct TurnReceipt {
    identity: QaTurn,
    requires_qa: bool,
    qa_report_id: Option<String>,
    #[serde(default)]
    prompt_sha256: Option<String>,
    #[serde(default)]
    stop_reminder_sent: bool,
    #[serde(default)]
    unverified_intake_candidate_id: Option<String>,
}

fn project_store(repo: &Path) -> PathBuf {
    repo.ancestors()
        .find_map(|ancestor| {
            crate::pm_registry::pm_worktree_store_dir(ancestor).or_else(|| {
                crate::pm_registry::is_canonical_pm_worktree(ancestor)
                    .then(|| ancestor.parent()?.parent().map(Path::to_path_buf))
                    .flatten()
            })
        })
        .unwrap_or_else(|| gwt_core::paths::gwt_project_dir_for_repo_path(repo))
}

fn receipt_path(repo: &Path, session: &str) -> PathBuf {
    project_store(repo)
        .join("project-state/pm-reporting")
        .join(format!(
            "{}.json",
            crate::pm_registry::pm_delivery_prompt_sha256(session)
        ))
}

fn read_receipt(repo: &Path, session: &str) -> io::Result<Option<TurnReceipt>> {
    match fs::read(receipt_path(repo, session)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn write_receipt(repo: &Path, receipt: &TurnReceipt) -> io::Result<()> {
    let bytes = serde_json::to_vec(receipt).map_err(io::Error::other)?;
    gwt_github::cache::write_atomic(&receipt_path(repo, &receipt.identity.session_id), &bytes)
}

/// Hook text is an observation, never proof of human authorship. Preserve it
/// before replacing the latest QA receipt, without selecting a native turn.
fn write_unverified_intake_candidate(
    repo: &Path,
    receipt: &TurnReceipt,
    original: &str,
) -> io::Result<()> {
    let project = project_store(repo);
    let project_key = project
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("PM intake project identity is unavailable"))?;
    let directory = project
        .join("project-state/pm-reporting/intake-candidates")
        .join(crate::pm_registry::pm_delivery_prompt_sha256(
            &receipt.identity.session_id,
        ));
    gwt_config::atomic::create_dir_all_durable(&directory).map_err(io::Error::other)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Protect the raw bytes before the shared writer creates its temp file.
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join(format!("{}.json", receipt.identity.turn_id));
    let bytes = serde_json::to_string(&serde_json::json!({
        "schema_version": 1,
        "state": "unverified",
        "project_key": project_key,
        "observation": receipt.identity,
        "original": original,
        "prompt_sha256": receipt.prompt_sha256,
    }))
    .map_err(io::Error::other)?;
    gwt_config::atomic::write_atomic(&path, &bytes).map_err(io::Error::other)?;
    if fs::read(&path)? != bytes.as_bytes() {
        return Err(io::Error::other(
            "PM intake candidate persistence unconfirmed; retain the native input for recovery",
        ));
    }
    Ok(())
}

pub(crate) fn candidate_digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn candidate_id_valid(value: &str) -> bool {
    value.split_once('/').is_some_and(|(session, turn)| {
        candidate_digest_valid(session)
            && uuid::Uuid::parse_str(turn).is_ok_and(|id| id.to_string() == turn)
    })
}

/// Inspect fixed directories without following a candidate path outside storage.
fn candidate_directory(project: &Path) -> io::Result<Option<PathBuf>> {
    let mut directory = project.to_path_buf();
    for component in ["project-state", "pm-reporting", "intake-candidates"] {
        directory.push(component);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(io::Error::other("invalid PM candidate directory")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(directory))
}

/// Only the shared atomic writer's unpublished sibling is not a candidate.
fn candidate_temporary_name(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|name| name.split_once(".json.tmp."))
        .is_some_and(|(turn, suffix)| {
            uuid::Uuid::parse_str(turn).is_ok_and(|id| id.to_string() == turn)
                && suffix.split_once('.').is_some_and(|(pid, timestamp)| {
                    pid.parse::<u32>().is_ok() && timestamp.parse::<u128>().is_ok()
                })
        })
}

/// The token describes observed immutable IDs, never human provenance or an
/// atomic snapshot. A later cycle must start discovery again for new inputs.
pub(crate) fn candidate_inventory(project: &Path) -> io::Result<(bool, Vec<String>, String)> {
    let directory = candidate_directory(project)?;
    let mut ids = Vec::new();
    if let Some(directory) = &directory {
        for session in fs::read_dir(directory)? {
            let session = session?;
            let session_id = session
                .file_name()
                .into_string()
                .map_err(|_| io::Error::other("invalid PM candidate session name"))?;
            if !candidate_digest_valid(&session_id) || !session.file_type()?.is_dir() {
                return Err(io::Error::other("invalid PM candidate session directory"));
            }
            for entry in fs::read_dir(session.path())? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| io::Error::other("invalid PM candidate file name"))?;
                if candidate_temporary_name(&name) {
                    continue;
                }
                let id = format!(
                    "{}/{}",
                    session_id,
                    name.strip_suffix(".json").unwrap_or("")
                );
                if !candidate_id_valid(&id) || !entry.file_type()?.is_file() {
                    return Err(io::Error::other("invalid PM candidate file"));
                }
                ids.push(id);
            }
        }
    }
    ids.sort();
    let bytes = serde_json::to_vec(&ids).map_err(io::Error::other)?;
    let digest = gwt_core::pm_authorization::digest(&bytes);
    Ok((directory.is_some(), ids, digest))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IntakeCandidate {
    schema_version: u32,
    state: String,
    project_key: String,
    observation: IntakeObservation,
    original: String,
    prompt_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IntakeObservation {
    session_id: String,
    turn_id: String,
}

// Match the existing PM native-conversation record budget. This limits the
// serialized record, including JSON escaping, rather than the decoded prompt.
pub(crate) const MAX_CANDIDATE_RECORD_BYTES: u64 = 4 * 1024 * 1024;
// Base64 expands this to at most the existing PM report text budget (64 KiB).
const CANDIDATE_CHUNK_BYTES: u64 = 48 * 1024;

fn open_intake_candidate(project: &Path, id: &str) -> io::Result<fs::File> {
    if !candidate_id_valid(id) {
        return Err(io::Error::other("invalid PM candidate identity"));
    }
    let directory = candidate_directory(project)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PM candidate not found"))?;
    let (session, turn) = id.split_once('/').expect("validated composite ID");
    let directory = directory.join(session);
    if !fs::symlink_metadata(&directory)?.is_dir() {
        return Err(io::Error::other("invalid PM candidate session directory"));
    }
    let path = directory.join(format!("{turn}.json"));
    if !fs::symlink_metadata(&path)?.is_file() {
        return Err(io::Error::other("invalid PM candidate file"));
    }
    fs::File::open(path)
}

fn read_candidate_record(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CANDIDATE_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CANDIDATE_RECORD_BYTES {
        return Err(io::Error::from(io::ErrorKind::FileTooLarge));
    }
    Ok(bytes)
}

pub(crate) fn read_intake_candidate(project: &Path, id: &str) -> io::Result<serde_json::Value> {
    let file = open_intake_candidate(project, id)?;
    if file.metadata()?.len() > MAX_CANDIDATE_RECORD_BYTES {
        return Err(io::Error::from(io::ErrorKind::FileTooLarge));
    }
    let (session, turn) = id.split_once('/').expect("validated composite ID");
    let candidate: IntakeCandidate =
        serde_json::from_slice(&read_candidate_record(file)?).map_err(io::Error::other)?;
    if candidate.schema_version != 1
        || candidate.state != "unverified"
        || Some(candidate.project_key.as_str())
            != project.file_name().and_then(|name| name.to_str())
        || candidate.observation.turn_id != turn
        || crate::pm_registry::pm_delivery_prompt_sha256(&candidate.observation.session_id)
            != session
        || candidate.prompt_sha256
            != gwt_core::pm_authorization::digest(candidate.original.as_bytes())
    {
        return Err(io::Error::other(
            "PM candidate identity or content does not match",
        ));
    }
    serde_json::to_value(candidate).map_err(io::Error::other)
}

/// Recovery bytes only: no candidate schema/hash validation or snapshot claim.
pub(crate) fn read_intake_candidate_chunk(
    project: &Path,
    id: &str,
    offset: u64,
) -> io::Result<serde_json::Value> {
    use base64::Engine;
    let mut file = open_intake_candidate(project, id)?;
    let observed_total_bytes = file.metadata()?.len();
    if offset > observed_total_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "candidate offset exceeds observed file length; restart at offset 0",
        ));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(CANDIDATE_CHUNK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let more = bytes.len() as u64 > CANDIDATE_CHUNK_BYTES;
    bytes.truncate(CANDIDATE_CHUNK_BYTES as usize);
    Ok(serde_json::json!({
        "schema_version": 1, "ok": true, "state": "unverified_raw", "id": id,
        "offset": offset, "observed_total_bytes": observed_total_bytes,
        "next_offset": more.then_some(offset + bytes.len() as u64),
        "bytes_base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
    }))
}

fn with_receipt_lock<T>(
    repo: &Path,
    session: &str,
    action: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let path = receipt_path(repo, session).with_extension("lock");
    fs::create_dir_all(path.parent().expect("receipt has a parent"))?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    // A busy sidecar must not turn a foreground prompt into an unbounded wait.
    lock.try_lock_exclusive()?;
    action()
}

fn registered_pm_session(worktree: &Path, session: Option<&str>) -> Option<String> {
    if !super::is_resident_pm_worktree(worktree) {
        return None;
    }
    let session = session?.trim();
    (!session.is_empty()
        && crate::pm_registry::session_is_registered_pm(
            &project_store(worktree).join("project-state/pm.json"),
            session,
        ))
    .then(|| session.to_owned())
}

/// Every observed PM prompt gets a new local turn identity; native/client turn
/// hints never authorize completion of a different turn.
pub(crate) fn handle_user_prompt_submit(worktree: &Path, input: &str) -> io::Result<()> {
    let ambient = crate::cli::pm::ambient_session_id();
    let Some(session_id) = registered_pm_session(worktree, ambient.as_deref()) else {
        return Ok(());
    };
    let prompt = serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let requires_qa = prompt.as_deref().is_some_and(|text| {
        !text.trim().is_empty() && !crate::pm_conversation::internal_prompt(text)
    });
    with_receipt_lock(worktree, &session_id, || {
        let identity = QaTurn {
            session_id: session_id.clone(),
            turn_id: uuid::Uuid::new_v4().to_string(),
        };
        let receipt = TurnReceipt {
            unverified_intake_candidate_id: requires_qa.then(|| identity.turn_id.clone()),
            identity,
            requires_qa,
            qa_report_id: None,
            stop_reminder_sent: false,
            prompt_sha256: prompt
                .as_deref()
                .filter(|_| requires_qa)
                .map(|text| gwt_core::pm_authorization::digest(text.as_bytes())),
        };
        if let Some(original) = prompt.as_deref().filter(|_| requires_qa) {
            write_unverified_intake_candidate(worktree, &receipt, original)?;
        }
        write_receipt(worktree, &receipt)
    })
}

/// Capture before posting. A later prompt cannot be settled by a post that
/// started during an older turn.
pub(crate) fn capture_qa_turn(repo: &Path) -> io::Result<Option<QaTurn>> {
    let Some(session_id) = crate::cli::pm::ambient_session_id() else {
        return Ok(None);
    };
    with_receipt_lock(repo, &session_id, || {
        let path = receipt_path(repo, &session_id);
        let receipt: TurnReceipt = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(receipt) => receipt,
                Err(_) => {
                    // Preserve the exact damaged bytes before replacing the
                    // receipt. A failed report must leave the fresh obligation.
                    let quarantine =
                        path.with_extension(format!("corrupt-{}", uuid::Uuid::new_v4()));
                    gwt_github::cache::write_atomic(&quarantine, &bytes)?;
                    let receipt = TurnReceipt {
                        identity: QaTurn {
                            session_id: session_id.clone(),
                            turn_id: uuid::Uuid::new_v4().to_string(),
                        },
                        requires_qa: true,
                        qa_report_id: None,
                        prompt_sha256: None,
                        stop_reminder_sent: false,
                        unverified_intake_candidate_id: None,
                    };
                    write_receipt(repo, &receipt)?;
                    receipt
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(receipt)
            .filter(|receipt| receipt.identity.session_id == session_id && receipt.requires_qa)
            .map(|receipt| receipt.identity))
    })
}

/// A PM-authored instruction is authority only when its original matches the
/// current human turn captured by the managed prompt boundary.
pub(crate) fn instruction_prompt_identity(
    repo: &Path,
    turn: &QaTurn,
    original: &str,
) -> io::Result<(String, String)> {
    let receipt = read_receipt(repo, &turn.session_id)?
        .filter(|receipt| receipt.identity == *turn && receipt.requires_qa
            && receipt.prompt_sha256.as_deref() == Some(gwt_core::pm_authorization::digest(original.as_bytes()).as_str()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied,
            "permission_stage=instruction: instruction.original must exactly match the current recorded human prompt; use pm.approval.request when no applicable instruction exists"))?;
    native_prompt_identity(&receipt)
}

/// Stop must not demand a report that instruction capture cannot authorize.
fn native_prompt_identity(receipt: &TurnReceipt) -> io::Result<(String, String)> {
    // Hook stdin is callable by agents. It is an obligation boundary, not a
    // human-authorship proof. Read the registered Session's native source too.
    gwt_agent::validate_session_id_path_component(&receipt.identity.session_id)
        .map_err(io::Error::other)?;
    let session = gwt_agent::Session::load_and_migrate(
        &gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", receipt.identity.session_id)),
    )
    .map_err(io::Error::other)?;
    let conversation = crate::pm_conversation::read_for_session(&session);
    let message = conversation.messages.iter().rev()
        .find(|message| message.role == crate::pm_conversation::PmConversationRole::User)
        .filter(|message| !crate::pm_conversation::internal_prompt(&message.text)
            && receipt.prompt_sha256.as_deref() == Some(gwt_core::pm_authorization::digest(message.text.as_bytes()).as_str()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied,
            "permission_stage=instruction: the latest native human message must match instruction.original; use pm.approval.request otherwise"))?;
    // Native message identity stays stable even if a hook is replayed.
    Ok((receipt.identity.session_id.clone(), message.id.clone()))
}

/// Called only after the joined report/instruction persistence path succeeds.
pub(crate) fn acknowledge_qa(repo: &Path, turn: &QaTurn, report_id: &str) -> io::Result<()> {
    if crate::cli::pm::ambient_session_id().as_deref() != Some(turn.session_id.as_str())
        || report_id.trim().is_empty()
    {
        return Ok(());
    }
    with_receipt_lock(repo, &turn.session_id, || {
        let Some(mut receipt) = read_receipt(repo, &turn.session_id)? else {
            return Ok(());
        };
        if receipt.identity == *turn && receipt.requires_qa {
            receipt.qa_report_id = Some(report_id.to_owned());
            write_receipt(repo, &receipt)?;
        }
        Ok(())
    })
}

pub(crate) fn handle_stop(worktree: &Path, session: Option<&str>) -> HookOutput {
    let Some(session) = registered_pm_session(worktree, session) else {
        return HookOutput::Silent;
    };
    let result = with_receipt_lock(worktree, &session, || {
        let Some(mut receipt) = read_receipt(worktree, &session)? else {
            return Ok(HookOutput::Silent);
        };
        if receipt.identity.session_id != session
            || !receipt.requires_qa
            || receipt.qa_report_id.is_some()
            || receipt.stop_reminder_sent
            || native_prompt_identity(&receipt).is_err()
        {
            return Ok(HookOutput::Silent);
        }
        // One reminder per recorded turn, including providers without a
        // stop_hook_active flag. This is not a successful report or authority.
        receipt.stop_reminder_sent = true;
        write_receipt(worktree, &receipt)?;
        Ok(HookOutput::stop_block("This human PM turn has no successful Q&A report. Post the user's question and your answer with JSON operation `pm.report.post`, `kind:\"qa\"`, before stopping. This reminder is issued once per turn; an unresolved report does not prevent the next Stop."))
    });
    result.unwrap_or_else(|error| {
        tracing::warn!(%error, "PM reporting Stop reminder failed open");
        HookOutput::Silent
    })
}

pub(crate) fn reporting_diagnostic(repo: &Path, session_id: &str) -> serde_json::Value {
    match read_receipt(repo, session_id) {
        Ok(Some(receipt)) if receipt.identity.session_id == session_id => serde_json::json!({
            "turn_id": receipt.identity.turn_id,
            "requires_qa": receipt.requires_qa,
            "qa_report_id": receipt.qa_report_id,
            "stop_reminder_sent": receipt.stop_reminder_sent,
            "unverified_intake_candidate_id": receipt.unverified_intake_candidate_id,
        }),
        Ok(None) => serde_json::json!({
            "turn_id": null, "requires_qa": false, "qa_report_id": null,
        }),
        Ok(Some(_)) => serde_json::json!({
            "unreadable": true, "error": "receipt session identity does not match",
        }),
        Err(error) => serde_json::json!({"unreadable": true, "error": error.to_string()}),
    }
}

/// Context only: an unacknowledged Monitor change never becomes a Stop gate.
/// Recompute on every prompt, so a missed status update remains visible.
pub(crate) fn monitor_reminder(worktree: &Path) -> Option<String> {
    let ambient = crate::cli::pm::ambient_session_id();
    registered_pm_session(worktree, ambient.as_deref())?;
    let fingerprint = crate::pm_current::monitor_fingerprint(worktree)?;
    match crate::pm_current::latest_status(worktree) {
        Ok(Some(status)) if status.monitor_fingerprint.as_deref() == Some(fingerprint.as_str()) => {
            None
        }
        Ok(_) => Some(
            "The current Monitor facts have not been acknowledged in PM current status. \
             Reconcile the changes and update JSON operation `pm.status.set` in this turn. \
             This reminder repeats on the next prompt until status acknowledges these facts."
                .into(),
        ),
        Err(_) => Some(
            "PM current status could not be read, so acknowledgement of the Monitor facts is \
             unknown. Inspect the status store and retry `pm.status.set`; do not claim a \
             confirmed Monitor change from this read failure."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::test_support::{ScopedEnvVar, ScopedGwtHome};

    #[test]
    fn candidate_record_read_bounds_growth_after_metadata_check() {
        let mut source = io::Cursor::new(vec![b' '; MAX_CANDIDATE_RECORD_BYTES as usize]);
        assert_eq!(
            read_candidate_record(&mut source).unwrap().len() as u64,
            MAX_CANDIDATE_RECORD_BYTES
        );
        // Deterministic append after a successful length check, without racing
        // a filesystem thread or depending on scheduler timing.
        source.get_mut().extend_from_slice(b"  ");
        source.set_position(0);
        let error = read_candidate_record(&mut source)
            .map(|bytes| bytes.len())
            .expect_err("growth must be limited during the read");
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert_eq!(source.position(), MAX_CANDIDATE_RECORD_BYTES + 1);
    }

    fn instruction_capture() -> Box<gwt_core::user_instruction::UserInstructionInput> {
        Box::new(gwt_core::user_instruction::UserInstructionInput {
            original: "状況は？".into(),
            summary: String::new(),
            answer: String::new(),
            actions: vec!["状況を報告".into()],
            issue_numbers: vec![],
            kind: gwt_core::user_instruction::UserInstructionKind::Question,
            supersedes: vec![],
            authorizations: vec![],
        })
    }

    fn fixture(home: &Path) -> std::path::PathBuf {
        let repo = home.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let worktree = crate::pm_registry::pm_worktree_path_for_repo_path(&repo);
        std::fs::create_dir_all(&worktree).unwrap();
        crate::pm_registry::save_pm_prefs(
            &worktree
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("project-state/pm.json"),
            &crate::pm_registry::PmPrefs {
                registration: Some(crate::pm_registry::PmRegistration {
                    session_id: "pm-session".into(),
                    agent_id: "codex".into(),
                    worktree_path: worktree.display().to_string(),
                    created_at: None,
                    consecutive_crashes: 0,
                    next_not_before: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        worktree
    }

    fn native_human_fixture(home: &Path, worktree: &Path, original: &str) -> ScopedEnvVar {
        let native_home = home.join("qa-native");
        let native = ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &native_home);
        let mut session = gwt_agent::Session::new(worktree, "pm", gwt_agent::AgentId::ClaudeCode);
        session.id = "pm-session".into();
        session.agent_session_id = Some("qa-conversation".into());
        fs::create_dir_all(gwt_core::paths::gwt_sessions_dir()).unwrap();
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let directory = native_home.join("projects/project");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("qa-conversation.jsonl"), format!("{}\n", serde_json::json!({
            "type":"user","uuid":"native-qa-turn","sessionId":"qa-conversation","cwd":worktree,
            "message":{"role":"user","content":original}
        }))).unwrap();
        native
    }

    #[test]
    fn intake_candidates_preserve_unverified_inputs_before_native_arrival() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let directory = project_store(&worktree)
            .join("project-state/pm-reporting/intake-candidates")
            .join(crate::pm_registry::pm_delivery_prompt_sha256("pm-session"));
        let prompt = "元の依頼を直して\n条件を保持してください";
        let mut saved = Vec::new();
        let mut _native = None;
        for _ in 0..2 {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            let receipt = read_receipt(&worktree, "pm-session").unwrap().unwrap();
            let path = directory.join(format!("{}.json", receipt.identity.turn_id));
            let bytes =
                fs::read(&path).expect("retain each unverified observation before answering");
            let candidate: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(candidate["state"], "unverified");
            assert_eq!(candidate["schema_version"], 1);
            assert_eq!(candidate["observation"]["session_id"], "pm-session");
            assert_eq!(candidate["original"], prompt);
            assert_eq!(
                candidate["prompt_sha256"],
                gwt_core::pm_authorization::digest(prompt.as_bytes())
            );
            assert!(candidate.get("authority").is_none());
            assert!(candidate.get("source_id").is_none());
            assert_eq!(
                candidate["project_key"],
                project_store(&worktree)
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                assert_eq!(
                    fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                    0o700,
                    "make raw input private before the atomic writer creates its temp file"
                );
            }
            assert_eq!(
                reporting_diagnostic(&worktree, "pm-session")["unverified_intake_candidate_id"],
                receipt.identity.turn_id
            );
            saved.push((path, bytes));
            // The second observation sees only the prior identical native ID.
            if _native.is_none() {
                _native = Some(native_human_fixture(home.path(), &worktree, prompt));
            }
        }
        assert_ne!(saved[0].0, saved[1].0);
        for input in [r#"{"prompt":"$gwt-pm"}"#, r#"{"prompt":" "}"#, "{}"] {
            handle_user_prompt_submit(&worktree, input).unwrap();
        }
        {
            let _foreign = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "ordinary");
            handle_user_prompt_submit(&worktree, r#"{"prompt":"foreign"}"#).unwrap();
        }
        handle_user_prompt_submit(home.path(), r#"{"prompt":"ordinary"}"#).unwrap();
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        assert_eq!(
            fs::read_dir(directory.parent().unwrap()).unwrap().count(),
            1
        );
        // Registration replacement cannot erase the previous PM's candidates.
        let prefs_path = project_store(&worktree).join("project-state/pm.json");
        let mut prefs = crate::pm_registry::load_pm_prefs(&prefs_path).unwrap();
        prefs.registration.as_mut().unwrap().session_id = "next-pm".into();
        crate::pm_registry::save_pm_prefs(&prefs_path, &prefs).unwrap();
        let _next = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "next-pm");
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        for (path, bytes) in saved {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert!(!project_store(&worktree)
            .join("project-state/request-tasks")
            .exists());
    }

    #[test]
    fn intake_candidates_fail_honestly_before_and_after_receipt_publication() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        handle_user_prompt_submit(&worktree, r#"{"prompt":"$gwt-pm"}"#).unwrap();
        let receipt = receipt_path(&worktree, "pm-session");
        let before = fs::read(&receipt).unwrap();
        let directory = project_store(&worktree)
            .join("project-state/pm-reporting/intake-candidates")
            .join(crate::pm_registry::pm_delivery_prompt_sha256("pm-session"));
        fs::create_dir_all(directory.parent().unwrap()).unwrap();
        fs::write(&directory, b"blocked candidate directory").unwrap();
        assert!(handle_user_prompt_submit(&worktree, r#"{"prompt":"保全して"}"#).is_err());
        assert_eq!(fs::read(&receipt).unwrap(), before);
        fs::remove_file(&directory).unwrap();
        fs::remove_file(&receipt).unwrap();
        fs::create_dir(&receipt).unwrap();
        assert!(handle_user_prompt_submit(&worktree, r#"{"prompt":"保全して"}"#).is_err());
        let candidates: Vec<_> = fs::read_dir(directory).unwrap().collect();
        assert_eq!(candidates.len(), 1, "candidate precedes the latest receipt");
        let candidate: serde_json::Value =
            serde_json::from_slice(&fs::read(candidates[0].as_ref().unwrap().path()).unwrap())
                .unwrap();
        assert_eq!(candidate["original"], "保全して");
        assert_eq!(candidate["state"], "unverified");
    }

    #[test]
    fn unpostable_automatic_tool_answer_and_mid_turn_prompts_do_not_block_stop() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        let path = home
            .path()
            .join("qa-native/projects/project/qa-conversation.jsonl");
        let previous_human = fs::read_to_string(&path).unwrap();
        for (prompt, record) in [
            (
                "定期巡回を続けて",
                serde_json::json!({
                    "isMeta":true, "message":{"role":"user","content":"定期巡回を続けて"}
                }),
            ),
            (
                "はい",
                serde_json::json!({"message":{"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"ask-user","content":"はい"}
                ]}}),
            ),
            (
                "ターミナルがスクロールできない問題は？",
                serde_json::json!({
                    "message":{"role":"assistant","content":"調査中です"}
                }),
            ),
        ] {
            let mut record = record;
            record["type"] = record["message"]["role"].clone();
            record["uuid"] = "not-current-human".into();
            record["sessionId"] = "qa-conversation".into();
            record["cwd"] = worktree.display().to_string().into();
            fs::write(&path, format!("{previous_human}{record}\n")).unwrap();
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            let turn = capture_qa_turn(&worktree).unwrap().unwrap();
            assert!(instruction_prompt_identity(&worktree, &turn, prompt).is_err());
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent,
                "Stop cannot require a Q&A the posting path rejects: {prompt}"
            );
        }
    }

    #[test]
    fn unresolved_human_qa_stop_reminder_is_bounded_per_turn() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        for _ in 0..2 {
            handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
            assert!(matches!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::StopBlock { .. }
            ));
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent
            );
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent
            );
            let turn = capture_qa_turn(&worktree).unwrap().unwrap();
            assert_eq!(
                reporting_diagnostic(&worktree, "pm-session")["stop_reminder_sent"],
                true
            );
            assert!(
                instruction_prompt_identity(&worktree, &turn, "別の質問").is_err(),
                "fail-open never grants instruction authority"
            );
            assert!(reporting_diagnostic(&worktree, "pm-session")["qa_report_id"].is_null());
        }
    }

    #[test]
    fn scoped_instruction_capture_requires_the_native_human_message() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let native_home = home.path().join("claude");
        let _native = ScopedEnvVar::set("CLAUDE_CONFIG_DIR", &native_home);
        let worktree = fixture(home.path());
        let original = "Enable automatic updates.";
        let mut env = crate::cli::TestEnv::new(home.path().join("repo"));
        let post = || {
            let mut instruction = instruction_capture();
            instruction.original = original.into();
            instruction.kind = gwt_core::user_instruction::UserInstructionKind::OneOff;
            instruction.authorizations = vec![gwt_core::pm_authorization::OperationIntent {
                operation: "update.settings.set".into(),
                params: serde_json::json!({"auto_apply_updates":true}),
            }];
            crate::cli::pm::PmCommand::ReportPost {
                project_root: None,
                kind: gwt_core::pm_report::PmReportKind::Qa,
                question: Some(original.into()),
                body: "I will enable updates and report the result.".into(),
                instruction: Some(instruction),
            }
        };
        handle_user_prompt_submit(
            &worktree,
            &serde_json::json!({"prompt":original}).to_string(),
        )
        .unwrap();
        assert!(
            crate::cli::pm::run(&mut env, post(), &mut String::new()).is_err(),
            "a synthetic hook alone is not human authority"
        );
        let mut session = gwt_agent::Session::new(&worktree, "pm", gwt_agent::AgentId::ClaudeCode);
        session.id = "pm-session".into();
        session.agent_session_id = Some("human-conversation".into());
        std::fs::create_dir_all(gwt_core::paths::gwt_sessions_dir()).unwrap();
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let native_dir = native_home.join("projects/project");
        std::fs::create_dir_all(&native_dir).unwrap();
        let transcript = |text: &str| {
            format!(
                "{}\n",
                serde_json::json!({"type":"user","uuid":"native-human-message","sessionId":"human-conversation","cwd":worktree,"message":{"role":"user","content":text}})
            )
        };
        std::fs::write(
            native_dir.join("human-conversation.jsonl"),
            transcript("Unrelated human question"),
        )
        .unwrap();
        assert!(crate::cli::pm::run(&mut env, post(), &mut String::new()).is_err());
        std::fs::write(
            native_dir.join("human-conversation.jsonl"),
            transcript(original),
        )
        .unwrap();
        let mut output = String::new();
        crate::cli::pm::run(&mut env, post(), &mut output).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(
            saved["instruction"]["authority"]["turn_id"],
            "native-human-message"
        );
        assert_eq!(
            saved["instruction"]["authorizations"][0]["operation"],
            "update.settings.set"
        );
        let mut ordinary = post();
        let crate::cli::pm::PmCommand::ReportPost { instruction, .. } = &mut ordinary else {
            unreachable!()
        };
        instruction.as_mut().unwrap().authorizations.clear();
        assert!(
            crate::cli::pm::run(&mut env, ordinary, &mut String::new()).is_err(),
            "same native turn cannot mint a second capture through the ordinary path"
        );
        assert_eq!(
            gwt_core::user_instruction::load(&home.path().join("repo"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unbound_human_answers_are_saved_as_display_only_qa_without_authority() {
        // Issue #6950: a question that arrives mid-work or after its response
        // deadline has no current human turn. Its answer must still reach
        // Reports, but it must never mint instruction authority.
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        let repo = home.path().join("repo");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let post = |original: &str, authorize: bool| {
            let mut instruction = instruction_capture();
            instruction.original = original.into();
            if authorize {
                instruction.authorizations = vec![gwt_core::pm_authorization::OperationIntent {
                    operation: "update.settings.set".into(),
                    params: serde_json::json!({"auto_apply_updates":true}),
                }];
            }
            crate::cli::pm::PmCommand::ReportPost {
                project_root: None,
                kind: gwt_core::pm_report::PmReportKind::Qa,
                question: Some(original.into()),
                body: "回答しました".into(),
                instruction: Some(instruction),
            }
        };
        // Expired request: no recorded human turn at all.
        crate::cli::pm::run(
            &mut env,
            post("期限切れの依頼は？", false),
            &mut String::new(),
        )
        .unwrap();
        // Mid-work question: the current turn belongs to another prompt.
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        let mut output = String::new();
        crate::cli::pm::run(&mut env, post("作業中の質問は？", false), &mut output).unwrap();
        let saved: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(saved["report"]["kind"], "qa");
        assert!(saved.get("instruction").is_none());
        assert!(
            crate::cli::pm::run(&mut env, post("作業中の承認は？", true), &mut String::new())
                .is_err(),
            "an unbound answer never carries authorizations"
        );
        let questions: Vec<_> = gwt_core::pm_report::load_reports(&repo)
            .unwrap()
            .into_iter()
            .filter(|report| report.kind == gwt_core::pm_report::PmReportKind::Qa)
            .filter_map(|report| report.question)
            .collect();
        assert_eq!(questions, ["期限切れの依頼は？", "作業中の質問は？"]);
        assert!(gwt_core::user_instruction::load(&repo).unwrap().is_empty());
        assert!(
            matches!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::StopBlock { .. }
            ),
            "an unbound answer does not settle the current human turn"
        );
    }

    #[test]
    fn human_qa_response_loss_retries_one_joined_capture_after_acknowledgement() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        let repo = home.path().join("repo");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        let post = || crate::cli::pm::PmCommand::ReportPost {
            project_root: None,
            kind: gwt_core::pm_report::PmReportKind::Qa,
            question: Some("状況は？".into()),
            body: "確認しました".into(),
            instruction: Some(instruction_capture()),
        };
        let mut first = String::new();
        crate::cli::pm::run(&mut env, post(), &mut first).unwrap();
        let mut retried = String::new();
        crate::cli::pm::run(&mut env, post(), &mut retried).unwrap();
        let first: serde_json::Value = serde_json::from_str(&first).unwrap();
        let retried: serde_json::Value = serde_json::from_str(&retried).unwrap();
        assert_eq!(first["instruction"]["id"], retried["instruction"]["id"]);
        let source = gwt_core::user_instruction::instructions_path(&repo);
        let before = fs::read(&source).unwrap();
        let mut changed = post();
        let crate::cli::pm::PmCommand::ReportPost { body, .. } = &mut changed else {
            unreachable!()
        };
        *body = "回答を変更しました".into();
        assert!(crate::cli::pm::run(&mut env, changed, &mut String::new()).is_err());
        let mut changed = post();
        let crate::cli::pm::PmCommand::ReportPost { question, .. } = &mut changed else {
            unreachable!()
        };
        *question = Some("変更した質問要約".into());
        assert!(crate::cli::pm::run(&mut env, changed, &mut String::new()).is_err());
        assert_eq!(fs::read(source).unwrap(), before);
        assert_eq!(gwt_core::user_instruction::load(&repo).unwrap().len(), 1);
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn human_qa_acknowledgement_is_scoped_to_the_same_session_and_turn() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況を教えて");
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況を教えて"}"#).unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        let first = capture_qa_turn(&worktree).unwrap().expect("armed PM turn");
        let other = QaTurn {
            session_id: "other-session".into(),
            ..first.clone()
        };
        acknowledge_qa(&worktree, &other, "wrong-session").unwrap();
        assert!(reporting_diagnostic(&worktree, "pm-session")["qa_report_id"].is_null());
        let _next_native = native_human_fixture(home.path(), &worktree, "次の質問です");
        handle_user_prompt_submit(&worktree, r#"{"prompt":"次の質問です"}"#).unwrap();
        acknowledge_qa(&worktree, &first, "old-turn").unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        let current = capture_qa_turn(&worktree).unwrap().unwrap();
        acknowledge_qa(&worktree, &current, "saved-report").unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        assert_eq!(
            handle_stop(&worktree, Some("other-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn canonical_automatic_prompts_pass_but_similar_human_prose_requires_qa() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let tick = format!(
            "[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close). {} {} {}",
            crate::pm_registry::PM_STEERING_WAKE_CLAUSE,
            crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE,
            crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE,
        );
        let portable_tick = format!(
            "[gwt] Scheduled supervision tick: reconcile now: read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close). {} {} {}",
            crate::pm_registry::PM_STEERING_WAKE_CLAUSE,
            crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE,
            crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE,
        );
        let notification = "<task-notification>\n<task-id>background-1</task-id>\n<tool-use-id>tool-1</tool-use-id>\n<output-file>/tmp/result</output-file>\n<status>completed</status>\n<summary>Background task completed</summary>\n</task-notification>";
        let extended_notification = notification.replace("</task-notification>", "<note>Read the result</note>\n<result>Done</result>\n<usage><tool_uses>1</tool_uses></usage>\n</task-notification>");
        for prompt in [
            "$gwt-pm",
            tick.as_str(),
            portable_tick.as_str(),
            notification,
            extended_notification.as_str(),
        ] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent
            );
        }
        handle_user_prompt_submit(&worktree, "{}").unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        for prompt in [
            "[gwt] Scheduled supervision tick について説明して".to_owned(),
            format!("Explain this notification: {portable_tick}"),
            format!("{portable_tick}\nこれは何を意味しますか？"),
            "$gwt-pm の使い方を教えて".to_owned(),
            format!("Explain this notification: {notification}"),
            format!("{notification}\n何が完了しましたか？"),
            format!("{notification}\nこの二つの通知を比較してください\n{notification}"),
        ] {
            let _native = native_human_fixture(home.path(), &worktree, &prompt);
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert!(
                matches!(
                    handle_stop(&worktree, Some("pm-session")),
                    HookOutput::StopBlock { .. }
                ),
                "human question must still require QA: {prompt}"
            );
        }
    }

    #[test]
    fn ordinary_agent_prompts_do_not_create_pm_reporting_obligations() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "agent-session");
        handle_user_prompt_submit(home.path(), r#"{"prompt":"修正して"}"#).unwrap();
        assert!(capture_qa_turn(home.path()).unwrap().is_none());
        assert_eq!(
            handle_stop(home.path(), Some("agent-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn only_successful_qa_report_posts_release_the_reporting_gate() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        let mut env = crate::cli::TestEnv::new(home.path().join("repo"));
        let post = |kind, body: &str| crate::cli::pm::PmCommand::ReportPost {
            project_root: None,
            kind,
            question: Some("状況は？".into()),
            body: body.into(),
            instruction: (kind == gwt_core::pm_report::PmReportKind::Qa).then(instruction_capture),
        };
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        for (question, original) in [("$gwt-pm", "状況は？"), ("定期指示の照合", "$gwt-pm")]
        {
            let mut instruction = instruction_capture();
            instruction.original = original.into();
            let result = crate::cli::pm::run(
                &mut env,
                crate::cli::pm::PmCommand::ReportPost {
                    project_root: None,
                    kind: gwt_core::pm_report::PmReportKind::Qa,
                    question: Some(question.into()),
                    body: "変化なし".into(),
                    instruction: Some(instruction),
                },
                &mut String::new(),
            );
            assert!(result.is_err(), "automatic QA must be refused");
            assert!(gwt_core::pm_report::load_reports(&home.path().join("repo"))
                .unwrap()
                .is_empty());
            assert!(capture_qa_turn(&worktree).unwrap().is_some());
        }
        assert!(crate::cli::pm::run(
            &mut env,
            post(gwt_core::pm_report::PmReportKind::Qa, ""),
            &mut String::new()
        )
        .is_err());
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        assert!(
            crate::cli::pm::run(
                &mut env,
                post(gwt_core::pm_report::PmReportKind::Progress, "進行中"),
                &mut String::new(),
            )
            .is_err(),
            "registered PM cannot bypass typed reporting"
        );
        let input = serde_json::from_value(serde_json::json!({
            "schema_version":1,"request_id":"human-turn-progress","events":[{
                "episode_id":"work","event_id":"status","expected_revision":0,"kind":"progress",
                "state":{"status":"active","severity":"routine","impact":"local"},
                "summary":{"result":"Work advanced"},"details":"Full evidence","refs":[]
            }]
        }))
        .unwrap();
        let batch = crate::cli::pm::PmCommand::ReportBatch {
            project_root: None,
            submission: Box::new(input),
        };
        crate::cli::pm::run(&mut env, batch.clone(), &mut String::new()).unwrap();
        crate::cli::pm::run(&mut env, batch, &mut String::new()).unwrap();
        assert!(reporting_diagnostic(&worktree, "pm-session")["qa_report_id"].is_null());
        crate::cli::pm::run(
            &mut env,
            post(gwt_core::pm_report::PmReportKind::Qa, "待機中です"),
            &mut String::new(),
        )
        .unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
    }

    #[test]
    fn monitor_changes_remind_until_current_status_acknowledges_them() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let repo = home.path().join("repo");
        let prefs_path = crate::issue_monitor_prefs_path_for_repo_path(&repo);
        let mut prefs = crate::IssueMonitorPrefs::default();
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "unacknowledged Monitor facts need context"
        );
        let fingerprint = crate::pm_current::monitor_fingerprint(&worktree).unwrap();
        let input = gwt_core::pm_status::PmStatusInput {
            items: vec![gwt_core::pm_status::PmStatusItem {
                kind: gwt_core::pm_status::PmStatusItemKind::Waiting,
                text: "Waiting for the queue".into(),
                started_at: "2026-10-13T09:00:00Z".parse().unwrap(),
            }],
            next_trigger: "The queue changes".into(),
            user_action: None,
            stale_after_minutes: 15,
        };
        gwt_core::pm_status::set_status(&repo, input.clone(), Some(fingerprint)).unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        prefs.priority_order.push(5349);
        crate::save_issue_monitor_prefs(&prefs_path, &prefs).unwrap();
        assert!(monitor_reminder(&worktree)
            .unwrap()
            .contains("pm.status.set"));
        assert!(
            monitor_reminder(&worktree).is_some(),
            "a new prompt must retry the reminder until acknowledged"
        );
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent,
            "Monitor reminders never block Stop"
        );

        gwt_core::pm_status::set_status(
            &repo,
            input.clone(),
            crate::pm_current::monitor_fingerprint(&worktree),
        )
        .unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        use gwt_core::coordination::{AuthorKind, BoardEntry, BoardEntryKind};
        let mut board = gwt_core::board_escalation::BoardEscalationStore::default();
        let blocked = BoardEntry::new(
            AuthorKind::Agent,
            "Codex",
            BoardEntryKind::Blocked,
            "Symptom: waiting.\nCause: missing ruling.\nRequest: decide.\nResume: ruling received.",
            None,
            None,
            vec![],
            vec!["5349".into()],
        );
        let blocked_id = blocked.id.clone();
        board.apply_entry(&blocked);
        // Use the Board reader's canonical path contract, including its
        // repo-local fallback in this fixture without a Git repository.
        let index = gwt_core::coordination::coordination_escalations_path(&worktree);
        fs::create_dir_all(index.parent().unwrap()).unwrap();
        gwt_github::cache::write_atomic(&index, &serde_json::to_vec(&board).unwrap()).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "a Board-only blocked owner must trigger the next prompt reminder"
        );
        gwt_core::pm_status::set_status(
            &repo,
            input,
            crate::pm_current::monitor_fingerprint(&worktree),
        )
        .unwrap();
        assert!(monitor_reminder(&worktree).is_none());
        let mut resolution = BoardEntry::new(
            AuthorKind::User,
            "You",
            BoardEntryKind::Decision,
            "The ruling is settled",
            None,
            None,
            vec![],
            vec!["5349".into()],
        );
        resolution.resolves_entry_ids = vec![blocked_id];
        board.apply_entry(&resolution);
        gwt_github::cache::write_atomic(&index, &serde_json::to_vec(&board).unwrap()).unwrap();
        assert!(
            monitor_reminder(&worktree).is_some(),
            "resolving a Board-only blocked owner must trigger the next prompt reminder"
        );
    }
    #[test]
    fn delivered_wakes_with_escalations_and_agent_idle_do_not_require_qa() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let suffix = format!(
            " {} {} {}",
            crate::pm_registry::PM_STEERING_WAKE_CLAUSE,
            crate::pm_registry::PM_GWTD_EXECUTION_WAKE_CLAUSE,
            crate::pm_registry::PM_CYCLE_REPORTING_CLAUSE
        );
        let tick = format!("[gwt] Scheduled supervision tick: reconcile now — read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close).{suffix}");
        let portable_tick = format!("[gwt] Scheduled supervision tick: reconcile now: read a fresh `issue.monitor.status` snapshot and inventory open PRs with `pr.list` (stale / SUPERSEDED / owner-Issue-closed rows: digest escalations, never auto-close).{suffix}");
        let delta = format!("[gwt] Monitor activity while the PM was idle (new needs_human). Reconcile now: fresh `issue.monitor.status`, triage new items, inventory PRs with `pr.list` (stale/SUPERSEDED/owner-closed rows: digest, never auto-close).{suffix}");
        let idle = "[gwt] Agent idle: 修正担当 (#3932) | [gwt] Agent idle: 次の担当 (#5349). Reconcile pane.list and issue.monitor.status; steer the idle agents.";
        let escalations = " UNRESOLVED BLOCKED ESCALATIONS (1) — decide fresh launch / spec ruling / tool fix for each, then post the resolution with params.resolves: #2338 params.resolves:[\"blocked-entry\"] 依頼: fresh launch を手配してほしい";
        let long_idle = format!("[gwt] Agent idle: {}. Reconcile pane.list and issue.monitor.status; steer the idle agents.", "担当".repeat(120));
        let mut truncated_idle = format!("{long_idle}{escalations}");
        // Match the physical idle-delivery cap, including a partially delivered escalation tail.
        let mut end = 959;
        while !truncated_idle.is_char_boundary(end) {
            end -= 1;
        }
        truncated_idle.truncate(end);
        for prompt in [
            format!("{tick}{escalations}\r"),
            format!("{portable_tick}{escalations}\r"),
            format!("{delta}{escalations}\r"),
            format!("{tick}{escalations} quoted: UNRESOLVED BLOCKED ESCALATIONS (1)\r"),
            format!("{idle}\r"),
            format!("{idle}{escalations}\r"),
            format!(
                "{}{escalations}\r",
                idle.replace("修正担当", "UNRESOLVED BLOCKED ESCALATIONS (1) 修正担当")
            ),
            format!("{truncated_idle}\r"),
        ] {
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string())
                .unwrap();
            assert_eq!(
                handle_stop(&worktree, Some("pm-session")),
                HookOutput::Silent,
                "physical automatic delivery must not arm QA: {prompt}"
            );
        }
        for prompt in ["[gwt] Agent idle: について説明して", "UNRESOLVED BLOCKED ESCALATIONS (1) は何ですか",
            "[gwt] Agent idle: 修正担当 (#3932). Reconcile pane.list and issue.monitor.status; steer the idle agents. これは何？"] {
            let _native = native_human_fixture(home.path(), &worktree, prompt);
            handle_user_prompt_submit(&worktree, &serde_json::json!({"prompt":prompt}).to_string()).unwrap();
            assert!(matches!(handle_stop(&worktree, Some("pm-session")), HookOutput::StopBlock { .. }), "similar human prose still needs QA: {prompt}");
        }
    }

    #[test]
    fn corrupt_receipt_is_quarantined_and_recovered_through_a_qa_post() {
        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session = ScopedEnvVar::set(gwt_agent::GWT_SESSION_ID_ENV, "pm-session");
        let worktree = fixture(home.path());
        let _native = native_human_fixture(home.path(), &worktree, "状況は？");
        let repo = home.path().join("repo");
        let mut env = crate::cli::TestEnv::new(repo.clone());
        let post = |body: &str| crate::cli::pm::PmCommand::ReportPost {
            project_root: None,
            kind: gwt_core::pm_report::PmReportKind::Qa,
            question: Some("状況は？".into()),
            body: body.into(),
            instruction: Some(instruction_capture()),
        };
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        let old = capture_qa_turn(&repo).unwrap().unwrap();
        let path = receipt_path(&worktree, "pm-session");
        let corrupt = b"{broken receipt";
        fs::write(&path, corrupt).unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        assert_eq!(
            reporting_diagnostic(&repo, "pm-session")["unreadable"],
            true
        );
        let failure = crate::cli::pm::run(&mut env, post(""), &mut String::new()).unwrap_err();
        assert!(
            failure.to_string().contains("nonempty"),
            "QA capture must recover before report validation: {failure}"
        );
        let recovered = capture_qa_turn(&repo).unwrap().unwrap();
        assert_ne!(recovered, old);
        let diagnostic = reporting_diagnostic(&repo, "pm-session");
        assert_eq!(diagnostic["turn_id"], recovered.turn_id);
        assert_eq!(diagnostic["requires_qa"], true);
        assert!(diagnostic["qa_report_id"].is_null());
        let quarantined: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(fs::read(quarantined[0].path()).unwrap(), corrupt);
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        handle_user_prompt_submit(&worktree, r#"{"prompt":"状況は？"}"#).unwrap();
        acknowledge_qa(&repo, &recovered, "older-success").unwrap();
        assert!(matches!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::StopBlock { .. }
        ));
        crate::cli::pm::run(&mut env, post("回復しました"), &mut String::new()).unwrap();
        assert_eq!(
            handle_stop(&worktree, Some("pm-session")),
            HookOutput::Silent
        );
        assert!(reporting_diagnostic(&repo, "pm-session")["qa_report_id"].is_string());
    }
}

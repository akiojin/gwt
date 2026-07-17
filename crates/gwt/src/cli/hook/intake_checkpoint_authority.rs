//! One-shot official-hook authority for managed Claude Intake checkpoints.
//!
//! This is a workflow-integrity boundary between provider-declared root and
//! subagent hook events. It is not an OS sandbox against arbitrary code that
//! already runs as the same user and can rewrite gwt's Session store.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration as StdDuration, SystemTime},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const INTAKE_CHECKPOINT_PERMIT_ENV: &str = "GWT_INTAKE_CHECKPOINT_PERMIT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntakeCheckpointOperation {
    DiscussionUpdate,
    CheckpointCurrent,
    CheckpointUpdate,
}

impl IntakeCheckpointOperation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DiscussionUpdate => "discussion.update",
            Self::CheckpointCurrent => "intake.checkpoint.current",
            Self::CheckpointUpdate => "intake.checkpoint.update",
        }
    }

    pub(crate) fn from_command(
        command: &str,
    ) -> Result<Option<Self>, IntakeCheckpointAuthorityError> {
        let structured = structured_operation_fields(command);
        if !structured.is_empty() {
            if structured.len() != 1 {
                return Err(IntakeCheckpointAuthorityError::Mismatch);
            }
            return Ok(structured[0].as_deref().and_then(Self::from_operation_name));
        }

        let mut found = [
            (
                Self::DiscussionUpdate,
                compatibility_comment_contains(command, Self::DiscussionUpdate.as_str()),
            ),
            (
                Self::CheckpointCurrent,
                compatibility_comment_contains(command, Self::CheckpointCurrent.as_str()),
            ),
            (
                Self::CheckpointUpdate,
                compatibility_comment_contains(command, Self::CheckpointUpdate.as_str()),
            ),
        ]
        .into_iter()
        .filter_map(|(operation, present)| present.then_some(operation));
        let first = found.next();
        if found.next().is_some() {
            return Err(IntakeCheckpointAuthorityError::Mismatch);
        }
        Ok(first)
    }

    fn from_operation_name(value: &str) -> Option<Self> {
        [
            Self::DiscussionUpdate,
            Self::CheckpointCurrent,
            Self::CheckpointUpdate,
        ]
        .into_iter()
        .find(|operation| operation.as_str() == value)
    }
}

fn structured_operation_fields(command: &str) -> Vec<Option<String>> {
    let mut fields = Vec::new();
    let mut cursor = 0;
    while cursor < command.len() {
        let Some(relative_start) = command[cursor..].find(['{', '[']) else {
            break;
        };
        let start = cursor + relative_start;
        let mut values =
            serde_json::Deserializer::from_str(&command[start..]).into_iter::<serde_json::Value>();
        let Some(Ok(value)) = values.next() else {
            cursor = start + 1;
            continue;
        };
        let consumed = values.byte_offset().max(1);
        cursor = start.saturating_add(consumed);
        if let Some(object) = value.as_object() {
            if let Some(operation) = object.get("operation") {
                fields.push(operation.as_str().map(ToOwned::to_owned));
            }
        }
    }
    fields
}

fn compatibility_comment_contains(command: &str, operation: &str) -> bool {
    command.lines().any(|line| {
        let Some((_, comment)) = line.split_once('#') else {
            return false;
        };
        comment.match_indices(operation).any(|(start, _)| {
            let before = comment[..start].chars().next_back();
            let after = comment[start + operation.len()..].chars().next();
            !before.is_some_and(is_operation_identifier_character)
                && !after.is_some_and(is_operation_identifier_character)
        })
    })
}

fn is_operation_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum IntakeCheckpointAuthorityError {
    #[error("root Intake checkpoint authorization is missing")]
    Missing,
    #[error("root Intake checkpoint authorization is invalid")]
    Invalid,
    #[error("root Intake checkpoint authorization expired")]
    Expired,
    #[error("root Intake checkpoint authorization does not match this operation")]
    Mismatch,
    #[error("root Intake checkpoint authorization storage is unavailable")]
    Storage,
}

const PERMIT_VERSION: u8 = 1;
const PERMIT_TTL_SECONDS: i64 = 60;
const MAX_PERMIT_BYTES: u64 = 4 * 1024;
const MAX_AUTHORITY_ENTRIES: usize = 128;
const AUTHORITY_DIR_NAME: &str = ".intake-checkpoint-authority";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntakeCheckpointPermit {
    version: u8,
    session_id: String,
    recovery_id: String,
    operation: String,
    token_digest: String,
    issued_at_millis: i64,
    expires_at_millis: i64,
}

fn issue_permit_in(
    authority_dir: &Path,
    session_id: &str,
    recovery_id: &str,
    operation: IntakeCheckpointOperation,
    now: DateTime<Utc>,
) -> Result<String, IntakeCheckpointAuthorityError> {
    validate_ledger_identity(session_id)?;
    validate_ledger_identity(recovery_id)?;
    ensure_private_authority_dir(authority_dir)?;
    prune_expired_permits(authority_dir)?;

    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_digest = token_digest(&token);
    let permit = IntakeCheckpointPermit {
        version: PERMIT_VERSION,
        session_id: session_id.to_string(),
        recovery_id: recovery_id.to_string(),
        operation: operation.as_str().to_string(),
        token_digest: token_digest.clone(),
        issued_at_millis: now.timestamp_millis(),
        expires_at_millis: (now + Duration::seconds(PERMIT_TTL_SECONDS)).timestamp_millis(),
    };
    let body = serde_json::to_vec(&permit).map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
    if body.len() as u64 > MAX_PERMIT_BYTES {
        return Err(IntakeCheckpointAuthorityError::Storage);
    }
    let destination = authority_dir.join(format!("{token_digest}.json"));
    let temporary = authority_dir.join(format!(
        ".{token_digest}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let write_result = (|| {
        let mut file = open_private_new_file(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(IntakeCheckpointAuthorityError::Storage);
    }
    Ok(token)
}

fn consume_permit_in(
    authority_dir: &Path,
    token: &str,
    session_id: &str,
    recovery_id: &str,
    operation: IntakeCheckpointOperation,
    now: DateTime<Utc>,
) -> Result<(), IntakeCheckpointAuthorityError> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(IntakeCheckpointAuthorityError::Invalid);
    }
    let digest = token_digest(token);
    let source = authority_dir.join(format!("{digest}.json"));
    let consuming = authority_dir.join(format!(
        ".consuming-{digest}.{}.json",
        uuid::Uuid::new_v4().simple()
    ));
    match fs::rename(&source, &consuming) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(IntakeCheckpointAuthorityError::Missing)
        }
        Err(_) => return Err(IntakeCheckpointAuthorityError::Storage),
    }

    let result = read_and_validate_consumed_permit(
        &consuming,
        &digest,
        session_id,
        recovery_id,
        operation,
        now,
    );
    let _ = fs::remove_file(&consuming);
    result
}

pub(crate) fn issue_for_current_managed_claude(
    operation: IntakeCheckpointOperation,
    hook_provider_session_id: &str,
) -> Result<Option<String>, IntakeCheckpointAuthorityError> {
    let Some(session_id) = std::env::var(gwt_agent::GWT_SESSION_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    validate_ledger_identity(&session_id)?;
    let sessions_dir = current_sessions_dir();
    let session = gwt_agent::Session::load(&sessions_dir.join(format!("{session_id}.toml")))
        .map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
    if session.id != session_id {
        return Err(IntakeCheckpointAuthorityError::Invalid);
    }
    if !session
        .agent_id
        .to_string()
        .to_ascii_lowercase()
        .contains("claude")
    {
        return Ok(None);
    }
    let hook_provider_session_id = hook_provider_session_id.trim();
    if hook_provider_session_id.is_empty()
        || session.agent_session_id.as_deref() != Some(hook_provider_session_id)
    {
        return Err(IntakeCheckpointAuthorityError::Mismatch);
    }
    // Current ledgers name the Intake lane explicitly. Pre-upgrade ledgers
    // may retain only the ephemeral bit; accepting either is the same bounded
    // compatibility rule used by legacy Intake import.
    if session.session_kind != Some(gwt_skills::SessionKind::Intake) && !session.is_ephemeral {
        return Ok(None);
    }
    let environment_recovery_id = std::env::var(gwt_agent::GWT_RECOVERY_ID_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let deterministic_legacy_recovery_id = format!("legacy-{session_id}");
    let recovery_id = match (
        session.recovery_id.as_deref(),
        environment_recovery_id.as_deref(),
    ) {
        (Some(expected), Some(actual)) if expected == actual => expected.to_string(),
        (Some(expected), None) if expected == deterministic_legacy_recovery_id => {
            expected.to_string()
        }
        (None, None) => {
            // The bounded legacy importer deterministically assigns this id
            // to the exact Session ledger before the checkpoint is written.
            deterministic_legacy_recovery_id
        }
        (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) => {
            return Err(IntakeCheckpointAuthorityError::Mismatch)
        }
    };
    validate_ledger_identity(&recovery_id)?;
    issue_permit_in(
        &authority_dir(&sessions_dir),
        &session_id,
        &recovery_id,
        operation,
        Utc::now(),
    )
    .map(Some)
}

pub(crate) fn consume_for_current_claude(
    session_id: &str,
    recovery_id: &str,
    operation: IntakeCheckpointOperation,
) -> Result<(), IntakeCheckpointAuthorityError> {
    let token = std::env::var(INTAKE_CHECKPOINT_PERMIT_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or(IntakeCheckpointAuthorityError::Missing)?;
    consume_permit_in(
        &authority_dir(&current_sessions_dir()),
        &token,
        session_id,
        recovery_id,
        operation,
        Utc::now(),
    )
}

fn read_and_validate_consumed_permit(
    path: &Path,
    expected_digest: &str,
    session_id: &str,
    recovery_id: &str,
    operation: IntakeCheckpointOperation,
    now: DateTime<Utc>,
) -> Result<(), IntakeCheckpointAuthorityError> {
    let body = gwt_core::bounded_file::read_bounded_regular_file(
        path,
        MAX_PERMIT_BYTES,
        "Intake checkpoint permit",
    )
    .map_err(|_| IntakeCheckpointAuthorityError::Invalid)?;
    let permit: IntakeCheckpointPermit =
        serde_json::from_slice(&body).map_err(|_| IntakeCheckpointAuthorityError::Invalid)?;
    if permit.version != PERMIT_VERSION
        || permit.token_digest != expected_digest
        || permit.session_id != session_id
        || permit.recovery_id != recovery_id
        || permit.operation != operation.as_str()
    {
        return Err(IntakeCheckpointAuthorityError::Mismatch);
    }
    let now_millis = now.timestamp_millis();
    if permit.issued_at_millis > now_millis.saturating_add(5_000) {
        return Err(IntakeCheckpointAuthorityError::Invalid);
    }
    if now_millis > permit.expires_at_millis {
        return Err(IntakeCheckpointAuthorityError::Expired);
    }
    Ok(())
}

fn current_sessions_dir() -> PathBuf {
    // `GWT_SESSION_RUNTIME_PATH` is agent-controlled input. The canonical
    // machine Session store is the only authority; accepting an arbitrary
    // three-parent path here would let a forged ledger select its own permit
    // directory.
    gwt_core::paths::gwt_sessions_dir()
}

fn authority_dir(sessions_dir: &Path) -> PathBuf {
    sessions_dir.join(AUTHORITY_DIR_NAME)
}

fn token_digest(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn validate_ledger_identity(value: &str) -> Result<(), IntakeCheckpointAuthorityError> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    valid
        .then_some(())
        .ok_or(IntakeCheckpointAuthorityError::Invalid)
}

fn ensure_private_authority_dir(path: &Path) -> Result<(), IntakeCheckpointAuthorityError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(IntakeCheckpointAuthorityError::Storage)
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_dir(path).map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
        }
        Err(_) => return Err(IntakeCheckpointAuthorityError::Storage),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)
    }
}

fn open_private_new_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn prune_expired_permits(path: &Path) -> Result<(), IntakeCheckpointAuthorityError> {
    let entries = fs::read_dir(path).map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
    let mut fresh = 0_usize;
    for entry in entries.take(MAX_AUTHORITY_ENTRIES + 1) {
        let entry = entry.map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| IntakeCheckpointAuthorityError::Storage)?;
        if !metadata.file_type().is_file() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > StdDuration::from_secs((PERMIT_TTL_SECONDS * 4) as u64));
        if stale {
            let _ = fs::remove_file(entry.path());
        } else {
            fresh = fresh.saturating_add(1);
        }
    }
    if fresh > MAX_AUTHORITY_ENTRIES {
        return Err(IntakeCheckpointAuthorityError::Storage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use tempfile::TempDir;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).single().unwrap()
    }

    #[test]
    fn command_operation_comes_from_the_structured_field_not_payload_text() {
        let command = r#"gwtd <<'JSON'
{"operation":"discussion.update","params":{"summary":"Compared intake.checkpoint.current with intake.checkpoint.update."}}
JSON"#;

        assert_eq!(
            IntakeCheckpointOperation::from_command(command),
            Ok(Some(IntakeCheckpointOperation::DiscussionUpdate))
        );
    }

    #[test]
    fn command_operation_rejects_multiple_structured_operation_fields() {
        let command = r#"gwtd <<'JSON'
{"operation":"discussion.update"}
{"operation":"intake.checkpoint.update"}
JSON"#;

        assert_eq!(
            IntakeCheckpointOperation::from_command(command),
            Err(IntakeCheckpointAuthorityError::Mismatch)
        );
    }

    #[test]
    fn command_operation_keeps_the_bounded_comment_compatibility_marker() {
        assert_eq!(
            IntakeCheckpointOperation::from_command(
                "$GWT_BIN < checkpoint.json # intake.checkpoint.current"
            ),
            Ok(Some(IntakeCheckpointOperation::CheckpointCurrent))
        );
    }

    #[test]
    fn one_shot_permit_is_operation_and_recovery_bound_without_plaintext_storage() {
        let temp = TempDir::new().expect("tempdir");
        let token = issue_permit_in(
            temp.path(),
            "session-root",
            "recovery-root",
            IntakeCheckpointOperation::DiscussionUpdate,
            now(),
        )
        .expect("issue permit");
        assert_eq!(token.len(), 64);
        let stored = std::fs::read_dir(temp.path())
            .expect("permit inventory")
            .map(|entry| {
                let path = entry.expect("entry").path();
                std::fs::read_to_string(path).expect("permit body")
            })
            .collect::<String>();
        assert!(
            !stored.contains(&token),
            "plaintext token must not be stored"
        );

        consume_permit_in(
            temp.path(),
            &token,
            "session-root",
            "recovery-root",
            IntakeCheckpointOperation::DiscussionUpdate,
            now() + Duration::seconds(1),
        )
        .expect("consume permit");
        assert_eq!(
            consume_permit_in(
                temp.path(),
                &token,
                "session-root",
                "recovery-root",
                IntakeCheckpointOperation::DiscussionUpdate,
                now() + Duration::seconds(2),
            ),
            Err(IntakeCheckpointAuthorityError::Missing),
            "a replay must fail closed"
        );
    }

    #[test]
    fn permit_rejects_operation_recovery_and_expiry_mismatches() {
        let temp = TempDir::new().expect("tempdir");
        let mismatch = issue_permit_in(
            temp.path(),
            "session-root",
            "recovery-root",
            IntakeCheckpointOperation::CheckpointCurrent,
            now(),
        )
        .expect("issue mismatch permit");
        assert_eq!(
            consume_permit_in(
                temp.path(),
                &mismatch,
                "session-root",
                "other-recovery",
                IntakeCheckpointOperation::CheckpointCurrent,
                now() + Duration::seconds(1),
            ),
            Err(IntakeCheckpointAuthorityError::Mismatch)
        );

        let expired = issue_permit_in(
            temp.path(),
            "session-root",
            "recovery-root",
            IntakeCheckpointOperation::CheckpointUpdate,
            now(),
        )
        .expect("issue expiring permit");
        assert_eq!(
            consume_permit_in(
                temp.path(),
                &expired,
                "session-root",
                "recovery-root",
                IntakeCheckpointOperation::CheckpointUpdate,
                now() + Duration::minutes(2),
            ),
            Err(IntakeCheckpointAuthorityError::Expired)
        );
    }
}

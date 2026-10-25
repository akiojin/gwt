//! Durable launch-recovery receipts shared by project runtime owners.
//!
//! Persistence and recovery retain exact Session identity, monotonic receipt
//! strengthening, and the existing lock and directory durability barriers.

use std::path::{Path, PathBuf};

use fs2::FileExt;

use super::{durable_launch_recovery_dir, durable_launch_recovery_path};

#[cfg(any(test, feature = "test-gh-guard"))]
type DurableLaunchRecoveryDirectorySyncHook = Box<dyn Fn(&Path) -> std::io::Result<()> + 'static>;

#[cfg(any(test, feature = "test-gh-guard"))]
thread_local! {
    static DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK:
        std::cell::RefCell<Option<DurableLaunchRecoveryDirectorySyncHook>> =
            std::cell::RefCell::new(None);
}

pub const DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum DurableLaunchRecoveryKind {
    Genesis,
    FreshSuccessor { operation_id: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DurableLaunchRecoveryRecord {
    pub schema_version: u32,
    pub kind: DurableLaunchRecoveryKind,
    pub session_id: String,
    pub project_root: PathBuf,
    pub worktree_path: PathBuf,
    pub repo_hash: String,
    pub owner_kind: String,
    pub owner_number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_binding: Option<gwt_agent::SessionExecutionBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_agent_id: Option<gwt_agent::AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_session_identity: Option<gwt_agent::SessionExecutionIdentity>,
}

impl DurableLaunchRecoveryRecord {
    pub fn owner(&self) -> Result<crate::cli::execution_state::ExecutionOwnerKey, String> {
        let kind = match self.owner_kind.as_str() {
            "spec" => crate::cli::execution_state::ExecutionOwnerKind::Spec,
            "issue" => crate::cli::execution_state::ExecutionOwnerKind::Issue,
            _ => return Err("launch recovery owner kind is not canonical".to_string()),
        };
        if self.schema_version != DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION
            || self.owner_number == 0
            || self.repo_hash.trim().is_empty()
        {
            return Err("launch recovery record schema or owner is invalid".to_string());
        }
        gwt_agent::validate_session_id_path_component(&self.session_id)?;
        if let DurableLaunchRecoveryKind::FreshSuccessor { operation_id } = &self.kind {
            if !canonical_public_id(operation_id, 256) {
                return Err("launch recovery operation id is not canonical".to_string());
            }
        }
        if let Some(binding) = self.expected_binding.as_ref() {
            if binding.schema_version != gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION
                || binding.session_id != self.session_id
                || binding.repo_hash != self.repo_hash
                || binding.owner_kind != self.owner_kind
                || binding.owner_number != self.owner_number
                || binding.capability_generation == 0
                || !canonical_public_id(&binding.identity.generation_id, 512)
                || !canonical_public_id(&binding.identity.binding_id, 512)
                || !canonical_public_id(&binding.identity.ledger_head_hash, 512)
            {
                return Err("launch recovery expected Session binding is not canonical".to_string());
            }
        }
        if self.expected_binding.is_some() != self.expected_agent_id.is_some()
            || self.expected_binding.is_some() != self.expected_session_identity.is_some()
        {
            return Err("launch recovery exact Session identity proof is incomplete".to_string());
        }
        if let Some(identity) = self.expected_session_identity.as_ref() {
            if identity.session_id != self.session_id
                || identity.worktree_path != self.worktree_path
                || identity.project_state_root.as_deref() != Some(self.project_root.as_path())
                || identity.repo_hash.as_deref() != Some(self.repo_hash.as_str())
                || identity.linked_issue_number != Some(self.owner_number)
                || self.expected_binding.as_ref() != Some(&identity.execution_binding)
                || self.expected_agent_id.as_ref() != Some(&identity.agent_id)
            {
                return Err(
                    "launch recovery expected Session identity is not canonical".to_string()
                );
            }
        }
        Ok(crate::cli::execution_state::ExecutionOwnerKey {
            kind,
            number: self.owner_number,
        })
    }
}

fn durable_launch_recovery_lock_path(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<PathBuf, String> {
    gwt_agent::validate_session_id_path_component(session_id)?;
    Ok(durable_launch_recovery_dir(sessions_dir).join(format!("{session_id}.lock")))
}

#[cfg(any(test, feature = "test-gh-guard"))]
// Only the `#[cfg(unix)]` directory-sync tests install this hook.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn set_durable_launch_recovery_directory_sync_test_hook(
    hook: Option<DurableLaunchRecoveryDirectorySyncHook>,
) {
    DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK.with(|slot| {
        *slot.borrow_mut() = hook;
    });
}

// `directory` is only read by the test hook and the `#[cfg(unix)]` fsync.
#[cfg_attr(not(any(test, unix)), allow(unused_variables))]
fn sync_durable_launch_recovery_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(any(test, feature = "test-gh-guard"))]
    if let Some(result) = DURABLE_LAUNCH_RECOVERY_DIRECTORY_SYNC_HOOK
        .with(|slot| slot.borrow().as_ref().map(|hook| hook(directory)))
    {
        return result;
    }

    #[cfg(unix)]
    {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

fn create_durable_launch_recovery_directory(directory: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut cursor = directory;
    while !cursor.as_os_str().is_empty() && !cursor.exists() {
        missing.push(cursor.to_path_buf());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    std::fs::create_dir_all(directory)?;

    if missing.is_empty() {
        sync_durable_launch_recovery_directory(directory)?;
        if let Some(parent) = directory
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_durable_launch_recovery_directory(parent)?;
        }
        return Ok(());
    }

    for created in missing.iter().rev() {
        sync_durable_launch_recovery_directory(created)?;
        if let Some(parent) = created
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            sync_durable_launch_recovery_directory(parent)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn persist_durable_launch_recovery(
    sessions_dir: &Path,
    kind: DurableLaunchRecoveryKind,
    session_id: &str,
    project_root: &Path,
    worktree_path: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    expected_agent_id: Option<&gwt_agent::AgentId>,
) -> Result<(), String> {
    let expected_session_identity = match (expected_binding, expected_agent_id) {
        (None, None) => None,
        (Some(binding), Some(agent_id)) => {
            let session_path = sessions_dir.join(format!("{session_id}.toml"));
            let session = match std::fs::symlink_metadata(&session_path) {
                Ok(_) => gwt_agent::Session::load(&session_path).map_err(|error| {
                    format!("launch recovery Session identity could not be read: {error}")
                })?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let branch = gwt_git::Repository::discover(worktree_path)
                        .and_then(|repository| repository.current_branch())
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            "launch recovery Session identity has no canonical branch".to_string()
                        })?;
                    let mut session =
                        gwt_agent::Session::new(worktree_path, branch, agent_id.clone());
                    session.id = session_id.to_string();
                    session.project_state_root = Some(project_root.to_path_buf());
                    session.repo_hash = Some(binding.repo_hash.clone());
                    session.linked_issue_number = Some(owner.number);
                    session
                }
                Err(error) => return Err(error.to_string()),
            };
            if &session.agent_id != agent_id {
                return Err("launch recovery Session Agent identity changed".to_string());
            }
            Some(gwt_agent::SessionExecutionIdentity::for_binding(
                &session, binding,
            )?)
        }
        _ => return Err("launch recovery exact Session identity proof is incomplete".to_string()),
    };
    persist_durable_launch_recovery_with_identity(
        sessions_dir,
        kind,
        session_id,
        project_root,
        worktree_path,
        owner,
        expected_binding,
        expected_agent_id,
        expected_session_identity.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn persist_durable_launch_recovery_with_identity(
    sessions_dir: &Path,
    kind: DurableLaunchRecoveryKind,
    session_id: &str,
    project_root: &Path,
    worktree_path: &Path,
    owner: crate::cli::execution_state::ExecutionOwnerKey,
    expected_binding: Option<&gwt_agent::SessionExecutionBinding>,
    expected_agent_id: Option<&gwt_agent::AgentId>,
    expected_session_identity: Option<&gwt_agent::SessionExecutionIdentity>,
) -> Result<(), String> {
    let record = DurableLaunchRecoveryRecord {
        schema_version: DURABLE_LAUNCH_RECOVERY_SCHEMA_VERSION,
        kind,
        session_id: session_id.to_string(),
        project_root: project_root.to_path_buf(),
        worktree_path: worktree_path.to_path_buf(),
        repo_hash: gwt_core::repo_hash::detect_repo_hash(worktree_path)
            .ok_or_else(|| "launch recovery worktree has no canonical repository hash".to_string())?
            .to_string(),
        owner_kind: owner.kind.as_str().to_string(),
        owner_number: owner.number,
        expected_binding: expected_binding.cloned(),
        expected_agent_id: expected_agent_id.cloned(),
        expected_session_identity: expected_session_identity.cloned(),
    };
    record.owner()?;
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "launch recovery path has no parent".to_string())?;
    create_durable_launch_recovery_directory(parent).map_err(|error| error.to_string())?;
    let lock_path = durable_launch_recovery_lock_path(sessions_dir, session_id)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| error.to_string())?;
    lock.lock_exclusive().map_err(|error| error.to_string())?;
    let existing = match std::fs::read(&path) {
        Ok(bytes) => Some(
            serde_json::from_slice::<DurableLaunchRecoveryRecord>(&bytes).map_err(|error| {
                format!("existing launch recovery receipt is malformed: {error}")
            })?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if let Some(existing) = existing.as_ref() {
        existing.owner()?;
        let same_operation = existing.kind == record.kind
            && existing.session_id == record.session_id
            && existing.project_root == record.project_root
            && existing.worktree_path == record.worktree_path
            && existing.repo_hash == record.repo_hash
            && existing.owner_kind == record.owner_kind
            && existing.owner_number == record.owner_number;
        let monotonic = existing == &record
            || (same_operation
                && existing.expected_binding.is_none()
                && record.expected_binding.is_some());
        if same_operation
            && existing.expected_binding.is_some()
            && record.expected_binding.is_none()
        {
            // A retry begins from the base receipt even when a previous
            // response-loss attempt already advanced this same operation to
            // an exact bound receipt. Keep the stronger evidence byte-for-byte
            // and let the coordinator replay its Prepared attempt.
            return Ok(());
        }
        if !monotonic {
            return Err("launch recovery receipt cannot be downgraded or retargeted".to_string());
        }
        if existing == &record {
            return Ok(());
        }
    }
    let bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    gwt_github::cache::write_atomic(&path, &bytes).map_err(|error| error.to_string())?;
    sync_durable_launch_recovery_directory(parent).map_err(|error| error.to_string())
}

pub fn clear_durable_launch_recovery(sessions_dir: &Path, session_id: &str) -> Result<(), String> {
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    let parent = path
        .parent()
        .ok_or_else(|| "launch recovery path has no parent".to_string())?;
    if !parent.exists() {
        return Ok(());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(durable_launch_recovery_lock_path(sessions_dir, session_id)?)
        .map_err(|error| error.to_string())?;
    lock.lock_exclusive().map_err(|error| error.to_string())?;
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let durability_barrier = if parent.exists() {
        Some(parent)
    } else {
        parent.parent().filter(|ancestor| ancestor.exists())
    };
    durability_barrier.map_or(Ok(()), |directory| {
        sync_durable_launch_recovery_directory(directory).map_err(|error| error.to_string())
    })
}

pub fn durable_launch_recovery_session_identity(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<Option<gwt_agent::SessionExecutionIdentity>, String> {
    let path = durable_launch_recovery_path(sessions_dir, session_id)?;
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let record: DurableLaunchRecoveryRecord =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    record.owner()?;
    if record.session_id != session_id {
        return Err("launch recovery Session id changed".to_string());
    }
    Ok(record.expected_session_identity)
}

pub fn bind_durable_launch_recovery_session_identity(
    sessions_dir: &Path,
    session: &gwt_agent::Session,
    binding: &gwt_agent::SessionExecutionBinding,
) -> Result<gwt_agent::SessionExecutionIdentity, String> {
    let path = durable_launch_recovery_path(sessions_dir, &session.id)?;
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let record: DurableLaunchRecoveryRecord =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let owner = record.owner()?;
    if record.session_id != session.id
        || record.worktree_path != session.worktree_path
        || session.project_state_root.as_deref() != Some(record.project_root.as_path())
        || session.repo_hash.as_deref() != Some(record.repo_hash.as_str())
        || session.linked_issue_number != Some(owner.number)
        || binding.owner_kind != owner.kind.as_str()
        || binding.owner_number != owner.number
    {
        return Err(
            "launch recovery candidate does not match its durable base receipt".to_string(),
        );
    }
    let identity = gwt_agent::SessionExecutionIdentity::for_binding(session, binding)?;
    persist_durable_launch_recovery_with_identity(
        sessions_dir,
        record.kind,
        &record.session_id,
        &record.project_root,
        &record.worktree_path,
        owner,
        Some(binding),
        Some(&session.agent_id),
        Some(&identity),
    )?;
    Ok(identity)
}

pub fn durable_launch_recovery_records(
    sessions_dir: &Path,
) -> Vec<(PathBuf, DurableLaunchRecoveryRecord)> {
    let Ok(entries) = std::fs::read_dir(durable_launch_recovery_dir(sessions_dir)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                return None;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt could not be read");
                    return None;
                }
            };
            let record = match serde_json::from_slice::<DurableLaunchRecoveryRecord>(&bytes) {
                Ok(record) => record,
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt is malformed");
                    return None;
                }
            };
            if let Err(error) = record.owner() {
                tracing::warn!(path = %path.display(), error = %error, "launch recovery receipt is invalid");
                return None;
            }
            if path.file_stem().and_then(|value| value.to_str())
                != Some(record.session_id.as_str())
            {
                tracing::warn!(path = %path.display(), "launch recovery receipt filename does not match its Session identity");
                return None;
            }
            match durable_launch_recovery_repo_identity_matches(&record) {
                Some(true) => {}
                Some(false) => {
                    match archive_mismatched_launch_recovery(sessions_dir, &path, &record, &bytes) {
                        Ok(()) => tracing::info!(session_id = %record.session_id, "archived launch recovery receipt after repository identity mismatch"),
                        Err(error) => tracing::warn!(session_id = %record.session_id, error = %error, "launch recovery receipt archive remains pending"),
                    }
                    return None;
                }
                None => {
                    tracing::warn!(session_id = %record.session_id, "retained launch recovery receipt while repository identity is unavailable");
                    return None;
                }
            }
            Some((path, record))
        })
        .collect()
}

fn archive_mismatched_launch_recovery(
    sessions_dir: &Path,
    path: &Path,
    record: &DurableLaunchRecoveryRecord,
    expected_bytes: &[u8],
) -> Result<(), String> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(durable_launch_recovery_lock_path(
            sessions_dir,
            &record.session_id,
        )?)
        .map_err(|error| error.to_string())?;
    lock.lock_exclusive().map_err(|error| error.to_string())?;
    // A concurrent launch may have strengthened or replaced this receipt.
    // Never archive evidence other than the exact mismatch we inspected.
    if std::fs::read(path).map_err(|error| error.to_string())? != expected_bytes
        || durable_launch_recovery_repo_identity_matches(record) != Some(false)
    {
        return Err("launch recovery evidence changed before archival".to_string());
    }
    let parent = durable_launch_recovery_dir(sessions_dir);
    let archive = parent.join("archive");
    create_durable_launch_recovery_directory(&archive).map_err(|error| error.to_string())?;
    let destination = archive.join(format!(
        "{}-{}.json",
        record.session_id,
        uuid::Uuid::new_v4()
    ));
    std::fs::rename(path, destination).map_err(|error| error.to_string())?;
    sync_durable_launch_recovery_directory(&archive).map_err(|error| error.to_string())?;
    sync_durable_launch_recovery_directory(&parent).map_err(|error| error.to_string())
}

// None means unavailable evidence, not proof that this repository changed.
fn durable_launch_recovery_repo_identity_matches(
    record: &DurableLaunchRecoveryRecord,
) -> Option<bool> {
    let repo_hash = gwt_core::repo_hash::detect_repo_hash(&record.worktree_path)?;
    if repo_hash.to_string() != record.repo_hash {
        return Some(false);
    }
    gwt_git::worktree::main_worktree_root(&record.worktree_path)
        .ok()
        .map(|anchor| path_matches(&anchor, &record.project_root))
}

fn canonical_public_id(value: &str, max_len: usize) -> bool {
    value.trim() == value
        && !value.is_empty()
        && value.len() <= max_len
        && !value.chars().any(char::is_control)
}

fn path_matches(left: &Path, right: &Path) -> bool {
    match (dunce::canonicalize(left), dunce::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            gwt_core::paths::normalize_windows_child_process_path(&left)
                == gwt_core::paths::normalize_windows_child_process_path(&right)
        }
        _ => false,
    }
}

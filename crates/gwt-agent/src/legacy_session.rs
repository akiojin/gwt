use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use chrono::{TimeZone, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    resolve_agent_id, AgentSessionHistoryEntry, AgentStatus, DockerLifecycleIntent,
    LaunchRuntimeTarget, Session, SessionCreateOutcome, SessionMode, SessionOrigin,
};

const MAX_LEGACY_FILES: usize = 256;
const MAX_LEGACY_DIRECTORY_ENTRIES: usize = 4_096;
const MAX_LEGACY_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LEGACY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_LEGACY_HISTORY_ENTRIES: usize = 10_000;
const LEGACY_SESSION_ID_DOMAIN: &[u8] = b"gwt/legacy-json/session-id";
const LEGACY_SESSION_ID_VERSION: &str = "v1";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegacySessionImportReport {
    pub discovered: usize,
    pub imported: usize,
    pub unchanged: usize,
    pub collisions: usize,
    pub skipped: usize,
    pub diagnostics: Vec<LegacySessionImportDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySessionImportDiagnostic {
    pub source: PathBuf,
    pub entry_index: Option<usize>,
    pub message: String,
}

static PREWARMED_SESSION_DIRS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct LegacySessionData {
    #[serde(default, alias = "lastWorktreePath")]
    last_worktree_path: Option<PathBuf>,
    #[serde(default, alias = "lastBranch")]
    last_branch: Option<String>,
    #[serde(default, alias = "lastUsedTool")]
    last_used_tool: Option<String>,
    #[serde(default, alias = "lastSessionId")]
    last_session_id: Option<String>,
    #[serde(default, alias = "toolLabel")]
    tool_label: Option<String>,
    #[serde(default, alias = "toolVersion")]
    tool_version: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    timestamp: i64,
    #[serde(alias = "repositoryRoot")]
    repository_root: PathBuf,
    #[serde(default)]
    history: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacySessionEntry {
    #[serde(default)]
    branch: String,
    #[serde(default, alias = "worktreePath")]
    worktree_path: Option<PathBuf>,
    #[serde(default, alias = "toolId")]
    tool_id: String,
    #[serde(default, alias = "toolLabel")]
    tool_label: String,
    #[serde(default, alias = "sessionId")]
    session_id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, alias = "reasoningLevel")]
    reasoning_level: Option<String>,
    #[serde(default, alias = "skipPermissions")]
    skip_permissions: Option<bool>,
    #[serde(default, alias = "toolVersion")]
    tool_version: Option<String>,
    #[serde(default, alias = "collaborationModes")]
    collaboration_modes: Option<bool>,
    #[serde(default, alias = "dockerService")]
    docker_service: Option<String>,
    #[serde(default, alias = "dockerForceHost")]
    docker_force_host: Option<bool>,
    #[serde(default, alias = "dockerRecreate")]
    docker_recreate: Option<bool>,
    #[serde(default, alias = "dockerBuild")]
    docker_build: Option<bool>,
    #[serde(default, alias = "dockerKeep")]
    docker_keep: Option<bool>,
    #[serde(default, alias = "dockerContainerName")]
    docker_container_name: Option<String>,
    #[serde(default, alias = "dockerComposeArgs")]
    docker_compose_args: Option<Vec<String>>,
    #[serde(default)]
    timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LaunchFingerprint {
    branch: String,
    worktree_path: Option<PathBuf>,
    tool_id: String,
    mode: Option<String>,
    model: Option<String>,
    reasoning_level: Option<String>,
    skip_permissions: Option<bool>,
    tool_version: Option<String>,
    docker_service: Option<String>,
    docker_force_host: Option<bool>,
    docker_recreate: Option<bool>,
}

impl LegacySessionEntry {
    fn fingerprint(&self) -> LaunchFingerprint {
        LaunchFingerprint {
            branch: self.branch.clone(),
            worktree_path: self.worktree_path.clone(),
            tool_id: self.tool_id.clone(),
            mode: self.mode.clone(),
            model: self.model.clone(),
            reasoning_level: self.reasoning_level.clone(),
            skip_permissions: self.skip_permissions,
            tool_version: self.tool_version.clone(),
            docker_service: self.docker_service.clone(),
            docker_force_host: self.docker_force_host,
            docker_recreate: self.docker_recreate,
        }
    }

    fn has_unsupported_metadata(&self) -> bool {
        self.collaboration_modes.is_some()
            || self.docker_build.is_some()
            || self.docker_keep.is_some()
            || self.docker_container_name.is_some()
            || self.docker_compose_args.is_some()
    }
}

#[derive(Debug)]
struct LogicalLegacyLaunch {
    start_index: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    end_index: usize,
    start: LegacySessionEntry,
    end: LegacySessionEntry,
}

/// Import every pre-ledger aggregate JSON file in `legacy_dir` into immutable
/// per-launch Session TOMLs under `sessions_dir`.
///
/// A missing legacy directory is an expected successful no-op. Individual
/// malformed, unsafe, or colliding inputs are isolated and reported without
/// preventing other valid imports.
pub fn import_legacy_sessions_from_dir(
    legacy_dir: &Path,
    sessions_dir: &Path,
) -> LegacySessionImportReport {
    let mut report = LegacySessionImportReport::default();
    let metadata = match fs::symlink_metadata(legacy_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return report,
        Err(error) => {
            skip(
                &mut report,
                legacy_dir,
                None,
                &format!("legacy directory metadata could not be read: {error}"),
            );
            return report;
        }
    };
    if metadata.file_type().is_symlink() {
        skip(
            &mut report,
            legacy_dir,
            None,
            "legacy directory symlink is not allowed",
        );
        return report;
    }
    if !metadata.is_dir() {
        skip(
            &mut report,
            legacy_dir,
            None,
            "legacy source is not a directory",
        );
        return report;
    }
    let entries = match fs::read_dir(legacy_dir) {
        Ok(entries) => entries,
        Err(error) => {
            skip(
                &mut report,
                legacy_dir,
                None,
                &format!("legacy directory could not be read: {error}"),
            );
            return report;
        }
    };
    let mut paths = Vec::new();
    for (index, entry) in entries.enumerate() {
        if index >= MAX_LEGACY_DIRECTORY_ENTRIES {
            skip(
                &mut report,
                legacy_dir,
                None,
                "legacy directory entry limit exceeded",
            );
            break;
        }
        let Ok(entry) = entry else {
            skip(
                &mut report,
                legacy_dir,
                None,
                "legacy directory entry could not be read",
            );
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut total_bytes = 0_u64;
    for (file_index, path) in paths.into_iter().enumerate() {
        report.discovered += 1;
        if file_index >= MAX_LEGACY_FILES {
            skip(&mut report, &path, None, "legacy file limit exceeded");
            continue;
        }
        let remaining_bytes = MAX_LEGACY_TOTAL_BYTES.saturating_sub(total_bytes);
        let bytes = match read_bounded_regular_file(&path, remaining_bytes) {
            Ok(bytes) => bytes,
            Err(error) => {
                skip(
                    &mut report,
                    &path,
                    None,
                    &format!("unsafe or unreadable source: {error}"),
                );
                continue;
            }
        };
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_LEGACY_TOTAL_BYTES {
            skip(&mut report, &path, None, "legacy total byte limit exceeded");
            continue;
        }
        let data: LegacySessionData = match serde_json::from_slice(&bytes) {
            Ok(data) => data,
            Err(error) => {
                skip(
                    &mut report,
                    &path,
                    None,
                    &format!("malformed JSON: {error}"),
                );
                continue;
            }
        };
        import_source(&path, data, sessions_dir, &mut report);
    }
    report
}

/// Resolve the canonical legacy directory from the current Session ledger
/// directory (`<home>/.gwt/sessions` -> `<home>/.config/gwt/sessions`) and
/// import it. Non-standard Session directories return an empty report rather
/// than guessing another user's home.
pub fn import_legacy_sessions(sessions_dir: &Path) -> LegacySessionImportReport {
    let Some(gwt_home) = sessions_dir.parent() else {
        return LegacySessionImportReport::default();
    };
    if gwt_home.file_name().and_then(|value| value.to_str()) != Some(".gwt") {
        return LegacySessionImportReport::default();
    }
    let Some(home) = gwt_home.parent() else {
        return LegacySessionImportReport::default();
    };
    import_legacy_sessions_from_dir(
        &home.join(".config").join("gwt").join("sessions"),
        sessions_dir,
    )
}

/// Load the current Session ledger after performing at most one legacy JSON
/// prewarm for this sessions directory in the current process.
pub fn load_sessions_with_legacy_import(sessions_dir: &Path) -> Vec<Session> {
    prewarm_legacy_sessions(sessions_dir);
    let Ok(entries) = fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
        .filter_map(|path| Session::load_and_migrate(&path).ok())
        .collect()
}

/// Ensure the legacy source for this ledger was considered once. Import
/// diagnostics are logged without source content; loading current Sessions is
/// never blocked by a legacy failure.
pub fn prewarm_legacy_sessions(sessions_dir: &Path) {
    let key = absolute_lexical_path(sessions_dir);
    let prewarmed = PREWARMED_SESSION_DIRS.get_or_init(|| Mutex::new(HashSet::new()));
    let report = {
        let mut guard = prewarmed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.contains(&key) {
            return;
        }
        // Hold the small process-wide prewarm lock through the bounded import
        // so a concurrent ledger reader cannot observe the directory as done
        // before its imported TOMLs have been published.
        let report = import_legacy_sessions(sessions_dir);
        guard.insert(key);
        report
    };
    for diagnostic in report.diagnostics {
        tracing::warn!(
            source = %diagnostic.source.display(),
            entry_index = diagnostic.entry_index,
            reason = %diagnostic.message,
            "legacy Session import diagnostic"
        );
    }
}

fn absolute_lexical_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn import_source(
    source_path: &Path,
    mut data: LegacySessionData,
    sessions_dir: &Path,
    report: &mut LegacySessionImportReport,
) {
    if data.history.len() > MAX_LEGACY_HISTORY_ENTRIES {
        skip(
            report,
            source_path,
            None,
            "legacy history entry limit exceeded",
        );
        return;
    }
    let raw_history = std::mem::take(&mut data.history);
    let mut history = Vec::with_capacity(raw_history.len().max(1));
    if raw_history.is_empty() {
        history.push((
            0,
            LegacySessionEntry {
                branch: data.last_branch.clone().unwrap_or_default(),
                worktree_path: data.last_worktree_path.clone(),
                tool_id: data.last_used_tool.clone().unwrap_or_default(),
                tool_label: data.tool_label.clone().unwrap_or_default(),
                session_id: data.last_session_id.clone(),
                mode: None,
                model: data.model.clone(),
                reasoning_level: None,
                skip_permissions: None,
                tool_version: data.tool_version.clone(),
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: data.timestamp,
            },
        ));
    } else {
        for (entry_index, value) in raw_history.into_iter().enumerate() {
            match serde_json::from_value::<LegacySessionEntry>(value) {
                Ok(entry) => history.push((entry_index, entry)),
                Err(_) => skip(
                    report,
                    source_path,
                    Some(entry_index),
                    "malformed legacy history entry",
                ),
            }
        }
    }

    for logical in logical_launches_with_original_indices(&history).0 {
        if logical.start.has_unsupported_metadata() || logical.end.has_unsupported_metadata() {
            report.diagnostics.push(LegacySessionImportDiagnostic {
                source: source_path.to_path_buf(),
                entry_index: Some(logical.start_index),
                message: "unsupported legacy-only launch metadata was ignored".to_string(),
            });
        }
        let Some(session) = convert_logical_launch(source_path, &data, &logical) else {
            skip(
                report,
                source_path,
                Some(logical.start_index),
                "missing agent identity",
            );
            continue;
        };
        match session.save_if_absent(sessions_dir) {
            Ok(SessionCreateOutcome::Created) => report.imported += 1,
            Ok(SessionCreateOutcome::Unchanged) => report.unchanged += 1,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                report.collisions += 1;
                report.diagnostics.push(LegacySessionImportDiagnostic {
                    source: source_path.to_path_buf(),
                    entry_index: Some(logical.start_index),
                    message: "deterministic Session id collision; existing ledger record preserved"
                        .to_string(),
                });
            }
            Err(error) => skip(
                report,
                source_path,
                Some(logical.start_index),
                &format!("Session publication failed: {error}"),
            ),
        }
    }
}

#[cfg(test)]
fn logical_launches(entries: &[LegacySessionEntry]) -> Vec<LogicalLegacyLaunch> {
    logical_launches_with_operation_count(entries).0
}

#[cfg(test)]
fn logical_launches_with_operation_count(
    entries: &[LegacySessionEntry],
) -> (Vec<LogicalLegacyLaunch>, usize) {
    let indexed_entries = entries.iter().cloned().enumerate().collect::<Vec<_>>();
    logical_launches_with_original_indices(&indexed_entries)
}

fn logical_launches_with_original_indices(
    entries: &[(usize, LegacySessionEntry)],
) -> (Vec<LogicalLegacyLaunch>, usize) {
    let mut paired_starts = HashMap::<usize, usize>::new();
    let mut paired_exits = HashSet::<usize>::new();
    let mut unmatched_starts = HashMap::<LaunchFingerprint, Vec<usize>>::new();
    let mut operations = 0;
    let mut previous_original_index = None;

    for (position, (original_index, entry)) in entries.iter().enumerate() {
        operations += 1;
        if previous_original_index.is_some_and(|previous| previous + 1 != *original_index) {
            // A skipped malformed entry may itself have been a matching
            // launch observation. Never pair across that unknown boundary.
            unmatched_starts.clear();
        }
        previous_original_index = Some(*original_index);
        let fingerprint = entry.fingerprint();
        if normalized_session_id(entry.session_id.as_deref()).is_none() {
            unmatched_starts
                .entry(fingerprint)
                .or_default()
                .push(position);
            continue;
        }
        let start_position = unmatched_starts
            .get_mut(&fingerprint)
            .and_then(|starts| (starts.len() == 1).then(|| starts.pop()).flatten());
        if let Some(start_position) = start_position {
            unmatched_starts.remove(&fingerprint);
            paired_starts.insert(start_position, position);
            paired_exits.insert(position);
        }
    }

    let launches = entries
        .iter()
        .enumerate()
        .filter_map(|(position, (original_index, entry))| {
            if paired_exits.contains(&position) {
                return None;
            }
            let end_position = paired_starts.get(&position).copied().unwrap_or(position);
            let (end_index, end) = &entries[end_position];
            Some(LogicalLegacyLaunch {
                start_index: *original_index,
                end_index: *end_index,
                start: entry.clone(),
                end: end.clone(),
            })
        })
        .collect();
    (launches, operations)
}

fn effective_branch(data: &LegacySessionData, logical: &LogicalLegacyLaunch) -> String {
    nonempty(&logical.end.branch)
        .or_else(|| nonempty(&logical.start.branch))
        .or_else(|| {
            data.last_branch
                .clone()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "main".to_string())
}

fn convert_logical_launch(
    source_path: &Path,
    data: &LegacySessionData,
    logical: &LogicalLegacyLaunch,
) -> Option<Session> {
    let raw_agent = if logical.end.tool_id.trim().is_empty() {
        logical.end.tool_label.as_str()
    } else {
        logical.end.tool_id.as_str()
    };
    let agent_id = resolve_agent_id(raw_agent)?;
    let worktree_path = logical
        .end
        .worktree_path
        .clone()
        .or_else(|| logical.start.worktree_path.clone())
        .unwrap_or_else(|| data.repository_root.clone());
    let branch = effective_branch(data, logical);
    let started_at = timestamp(logical.start.timestamp);
    let ended_at = timestamp(logical.end.timestamp);
    let exact_id = normalized_session_id(logical.end.session_id.as_deref())
        .or_else(|| normalized_session_id(logical.start.session_id.as_deref()))
        .map(str::to_string);

    let mut session = Session::new(worktree_path, branch, agent_id);
    session.id = deterministic_session_id(source_path, data, logical);
    session.origin = SessionOrigin::LegacyJson;
    session.project_state_root = Some(data.repository_root.clone());
    if session.repo_hash.is_none() {
        session.repo_hash = gwt_core::repo_hash::detect_repo_hash(&data.repository_root)
            .map(|hash| hash.as_str().to_string());
    }
    session.agent_session_id = exact_id.clone();
    session.session_history = exact_id
        .map(|agent_session_id| {
            vec![AgentSessionHistoryEntry {
                agent_session_id,
                started_at,
            }]
        })
        .unwrap_or_default();
    session.status = AgentStatus::Stopped;
    session.restore_window_on_startup = false;
    session.tool_version = logical
        .end
        .tool_version
        .clone()
        .or_else(|| logical.start.tool_version.clone());
    session.model = logical
        .end
        .model
        .clone()
        .or_else(|| logical.start.model.clone());
    session.reasoning_level = logical
        .end
        .reasoning_level
        .clone()
        .or_else(|| logical.start.reasoning_level.clone());
    session.session_mode = parse_session_mode(
        logical
            .end
            .mode
            .as_deref()
            .or(logical.start.mode.as_deref()),
    );
    session.skip_permissions = logical
        .end
        .skip_permissions
        .or(logical.start.skip_permissions)
        .unwrap_or(false);
    let force_host = logical
        .end
        .docker_force_host
        .or(logical.start.docker_force_host)
        .unwrap_or(false);
    session.docker_service = logical
        .end
        .docker_service
        .clone()
        .or_else(|| logical.start.docker_service.clone());
    session.runtime_target = if !force_host && session.docker_service.is_some() {
        LaunchRuntimeTarget::Docker
    } else {
        LaunchRuntimeTarget::Host
    };
    session.docker_lifecycle_intent = if logical
        .end
        .docker_recreate
        .or(logical.start.docker_recreate)
        .unwrap_or(false)
    {
        DockerLifecycleIntent::Recreate
    } else {
        DockerLifecycleIntent::Connect
    };
    let label = nonempty(&logical.end.tool_label).or_else(|| nonempty(&logical.start.tool_label));
    if let Some(label) = label {
        session.display_name = label;
    }
    session.launch_command = session.agent_id.command().to_string();
    session.launch_args.clear();
    session.schema_version = Session::CURRENT_SCHEMA_VERSION;
    session.created_at = started_at;
    session.updated_at = ended_at.max(started_at);
    session.last_activity_at = session.updated_at;
    Some(session)
}

fn deterministic_session_id(
    source_path: &Path,
    data: &LegacySessionData,
    logical: &LogicalLegacyLaunch,
) -> String {
    let mut hash = Sha256::new();
    hash_component(&mut hash, b"domain", Some(LEGACY_SESSION_ID_DOMAIN));
    hash_component(
        &mut hash,
        b"version",
        Some(LEGACY_SESSION_ID_VERSION.as_bytes()),
    );
    let source_name = source_path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();
    hash_component(&mut hash, b"source_name", Some(source_name.as_bytes()));
    let repository_root = data.repository_root.to_string_lossy();
    hash_component(
        &mut hash,
        b"repository_root",
        Some(repository_root.as_bytes()),
    );
    let branch = effective_branch(data, logical);
    hash_component(&mut hash, b"effective_branch", Some(branch.as_bytes()));
    hash_legacy_entry(&mut hash, b"start", &logical.start);
    hash_legacy_entry(&mut hash, b"end", &logical.end);
    let digest = format!("{:x}", hash.finalize());
    format!("legacy-json-{LEGACY_SESSION_ID_VERSION}-{}", &digest[..32])
}

fn hash_legacy_entry(hash: &mut Sha256, role: &[u8], entry: &LegacySessionEntry) {
    hash_component(hash, b"entry_role", Some(role));
    hash_component(hash, b"branch", Some(entry.branch.as_bytes()));
    let worktree_path = entry
        .worktree_path
        .as_ref()
        .map(|path| path.to_string_lossy());
    hash_component(
        hash,
        b"worktree_path",
        worktree_path.as_deref().map(str::as_bytes),
    );
    hash_component(hash, b"tool_id", Some(entry.tool_id.as_bytes()));
    hash_component(hash, b"tool_label", Some(entry.tool_label.as_bytes()));
    hash_component(
        hash,
        b"session_id",
        entry.session_id.as_deref().map(str::as_bytes),
    );
    hash_component(hash, b"mode", entry.mode.as_deref().map(str::as_bytes));
    hash_component(hash, b"model", entry.model.as_deref().map(str::as_bytes));
    hash_component(
        hash,
        b"reasoning_level",
        entry.reasoning_level.as_deref().map(str::as_bytes),
    );
    hash_optional_bool(hash, b"skip_permissions", entry.skip_permissions);
    hash_component(
        hash,
        b"tool_version",
        entry.tool_version.as_deref().map(str::as_bytes),
    );
    hash_optional_bool(hash, b"collaboration_modes", entry.collaboration_modes);
    hash_component(
        hash,
        b"docker_service",
        entry.docker_service.as_deref().map(str::as_bytes),
    );
    hash_optional_bool(hash, b"docker_force_host", entry.docker_force_host);
    hash_optional_bool(hash, b"docker_recreate", entry.docker_recreate);
    hash_optional_bool(hash, b"docker_build", entry.docker_build);
    hash_optional_bool(hash, b"docker_keep", entry.docker_keep);
    hash_component(
        hash,
        b"docker_container_name",
        entry.docker_container_name.as_deref().map(str::as_bytes),
    );
    match entry.docker_compose_args.as_deref() {
        None => hash_component(hash, b"docker_compose_args", None),
        Some(args) => {
            let count = (args.len() as u64).to_le_bytes();
            hash_component(hash, b"docker_compose_args", Some(&count));
            for arg in args {
                hash_component(hash, b"docker_compose_arg", Some(arg.as_bytes()));
            }
        }
    }
    hash_component(hash, b"timestamp", Some(&entry.timestamp.to_le_bytes()));
}

fn hash_optional_bool(hash: &mut Sha256, label: &[u8], value: Option<bool>) {
    let encoded = value.map(|value| [u8::from(value)]);
    hash_component(hash, label, encoded.as_ref().map(|value| value.as_slice()));
}

fn hash_component(hash: &mut Sha256, label: &[u8], value: Option<&[u8]>) {
    hash.update((label.len() as u64).to_le_bytes());
    hash.update(label);
    match value {
        Some(value) => {
            hash.update([1]);
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value);
        }
        None => hash.update([0]),
    }
}

fn parse_session_mode(mode: Option<&str>) -> SessionMode {
    match mode.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("continue") => SessionMode::Continue,
        Some("resume") => SessionMode::Resume,
        _ => SessionMode::Normal,
    }
}

fn timestamp(value: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH)
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalized_session_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn skip(
    report: &mut LegacySessionImportReport,
    source: &Path,
    entry_index: Option<usize>,
    message: &str,
) {
    report.skipped += 1;
    report.diagnostics.push(LegacySessionImportDiagnostic {
        source: source.to_path_buf(),
        entry_index,
        message: message.to_string(),
    });
}

fn read_bounded_regular_file(path: &Path, remaining_total_bytes: u64) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source is not a regular file",
        ));
    }
    if metadata.len() > MAX_LEGACY_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source exceeds file-size limit",
        ));
    }
    if metadata.len() > remaining_total_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source exceeds remaining total-byte limit",
        ));
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open the link itself instead of its
        // target so the regular-file check below fails closed.
        options.custom_flags(0x0020_0000);
    }
    let mut file = options.open(path)?;
    ensure_opened_regular_file(&file)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let read_limit = MAX_LEGACY_FILE_BYTES.min(remaining_total_bytes);
    file.by_ref().take(read_limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LEGACY_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source grew beyond file-size limit",
        ));
    }
    if bytes.len() as u64 > remaining_total_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "source grew beyond remaining total-byte limit",
        ));
    }
    ensure_opened_regular_file(&file)?;
    Ok(bytes)
}

fn ensure_opened_regular_file(file: &File) -> io::Result<()> {
    if file.metadata()?.file_type().is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "opened source is not a regular file",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn write_legacy_file(legacy_dir: &Path, name: &str, body: &serde_json::Value) -> Vec<u8> {
        std::fs::create_dir_all(legacy_dir).expect("legacy dir");
        let bytes = serde_json::to_vec_pretty(body).expect("serialize fixture");
        std::fs::write(legacy_dir.join(name), &bytes).expect("write fixture");
        bytes
    }

    fn imported_sessions(sessions_dir: &Path) -> Vec<crate::Session> {
        let mut sessions = std::fs::read_dir(sessions_dir)
            .expect("sessions dir")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("toml"))
            .map(|path| crate::Session::load(&path).expect("load imported session"))
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.created_at);
        sessions
    }

    #[test]
    fn imports_one_unambiguous_start_exit_pair_as_one_stopped_session() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");

        let source = json!({
            "repositoryRoot": repo,
            "timestamp": 1_710_000_001_000_i64,
            "history": [
                {
                    "branch": "work/issue-1681",
                    "worktreePath": worktree,
                    "toolId": "codex-cli",
                    "toolLabel": "Codex",
                    "model": "gpt-5",
                    "reasoningLevel": "high",
                    "skipPermissions": true,
                    "toolVersion": "1.2.3",
                    "timestamp": 1_710_000_000_000_i64
                },
                {
                    "branch": "work/issue-1681",
                    "worktreePath": worktree,
                    "toolId": "codex-cli",
                    "toolLabel": "Codex",
                    "sessionId": "legacy-codex-session",
                    "model": "gpt-5",
                    "reasoningLevel": "high",
                    "skipPermissions": true,
                    "toolVersion": "1.2.3",
                    "timestamp": 1_710_000_001_000_i64
                }
            ]
        });
        let source_bytes = write_legacy_file(&legacy_dir, "repo.json", &source);

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.skipped, 0, "{report:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert!(session.id.starts_with("legacy-json-"));
        assert_eq!(session.branch, "work/issue-1681");
        assert_eq!(session.worktree_path, worktree);
        assert_eq!(session.agent_id, crate::AgentId::Codex);
        assert_eq!(
            session.agent_session_id.as_deref(),
            Some("legacy-codex-session")
        );
        assert_eq!(session.session_history.len(), 1);
        assert_eq!(
            session.session_history[0].started_at,
            Utc.timestamp_millis_opt(1_710_000_000_000_i64)
                .single()
                .unwrap()
        );
        assert_eq!(session.status, crate::AgentStatus::Stopped);
        assert!(!session.restore_window_on_startup);
        assert_eq!(
            session.schema_version,
            crate::Session::CURRENT_SCHEMA_VERSION
        );
        assert_eq!(session.origin, crate::SessionOrigin::LegacyJson);
        let resume = crate::AgentLaunchBuilder::new(session.agent_id.clone())
            .session_mode(crate::SessionMode::Resume)
            .resume_session_id(
                session
                    .exact_resume_session_id()
                    .expect("legacy exact resume id"),
            )
            .build();
        assert_eq!(resume.command, "codex");
        assert!(resume
            .args
            .windows(2)
            .any(|pair| pair[0] == "resume" && pair[1] == "legacy-codex-session"));
        assert_eq!(
            std::fs::read(legacy_dir.join("repo.json")).expect("source still present"),
            source_bytes
        );
    }

    #[test]
    fn ambiguous_pending_starts_are_not_coalesced_with_the_exit_observation() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");

        let entry = |timestamp: i64, session_id: Option<&str>| {
            let mut value = json!({
                "branch": "main",
                "worktree_path": worktree,
                "tool_id": "claude-code",
                "tool_label": "Claude Code",
                "timestamp": timestamp
            });
            if let Some(session_id) = session_id {
                value["session_id"] = json!(session_id);
            }
            value
        };
        write_legacy_file(
            &legacy_dir,
            "ambiguous.json",
            &json!({
                "repository_root": repo,
                "timestamp": 1_710_000_003_000_i64,
                "history": [
                    entry(1_710_000_000_000, None),
                    entry(1_710_000_001_000, None),
                    entry(1_710_000_002_000, Some("claude-session"))
                ]
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 3, "{report:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 3);
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.agent_session_id.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn repeated_nonempty_session_id_observations_remain_distinct_launches() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");

        write_legacy_file(
            &legacy_dir,
            "resume-writer-shape.json",
            &json!({
                "repositoryRoot": repo,
                "timestamp": 1_710_000_001_000_i64,
                "history": [
                    {
                        "branch": "main",
                        "worktreePath": worktree,
                        "toolId": "codex",
                        "toolLabel": "Codex",
                        "sessionId": "same-resumed-session",
                        "timestamp": 1_710_000_000_000_i64
                    },
                    {
                        "branch": "main",
                        "worktreePath": worktree,
                        "toolId": "codex",
                        "toolLabel": "Codex",
                        "sessionId": "same-resumed-session",
                        "timestamp": 1_710_000_001_000_i64
                    }
                ]
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 2, "{report:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 2);
        assert_ne!(sessions[0].id, sessions[1].id);
        assert!(sessions.iter().all(|session| {
            session.agent_session_id.as_deref() == Some("same-resumed-session")
        }));
        assert_eq!(
            sessions[0].created_at,
            Utc.timestamp_millis_opt(1_710_000_000_000_i64)
                .single()
                .unwrap()
        );
        assert_eq!(
            sessions[1].created_at,
            Utc.timestamp_millis_opt(1_710_000_001_000_i64)
                .single()
                .unwrap()
        );
    }

    #[test]
    fn rolling_history_front_drop_preserves_surviving_launch_ids() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");

        let launch = |timestamp: i64, session_id: &str| {
            vec![
                json!({
                    "branch": "main",
                    "worktreePath": repo,
                    "toolId": "codex",
                    "toolLabel": "Codex",
                    "timestamp": timestamp
                }),
                json!({
                    "branch": "main",
                    "worktreePath": repo,
                    "toolId": "codex",
                    "toolLabel": "Codex",
                    "sessionId": session_id,
                    "timestamp": timestamp + 1_000
                }),
            ]
        };
        let mut history = Vec::new();
        history.extend(launch(1_710_000_000_000, "oldest"));
        history.extend(launch(1_710_000_002_000, "survivor-one"));
        history.extend(launch(1_710_000_004_000, "survivor-two"));
        write_legacy_file(
            &legacy_dir,
            "rolling.json",
            &json!({
                "repositoryRoot": repo,
                "timestamp": 1_710_000_005_000_i64,
                "history": history
            }),
        );

        let first = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        assert_eq!(first.imported, 3, "{first:?}");
        let survivor_snapshots = imported_sessions(&sessions_dir)
            .into_iter()
            .filter(|session| session.agent_session_id.as_deref() != Some("oldest"))
            .map(|session| {
                let ledger_content =
                    std::fs::read(sessions_dir.join(format!("{}.toml", session.id)))
                        .expect("survivor ledger content");
                (
                    session.agent_session_id.expect("survivor exact id"),
                    session.id,
                    ledger_content,
                )
            })
            .collect::<Vec<_>>();

        write_legacy_file(
            &legacy_dir,
            "rolling.json",
            &json!({
                "repositoryRoot": repo,
                "timestamp": 1_710_000_005_000_i64,
                "history": history.into_iter().skip(2).collect::<Vec<_>>()
            }),
        );

        let second = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        assert_eq!(second.imported, 0, "{second:?}");
        assert_eq!(second.unchanged, 2, "{second:?}");
        assert_eq!(second.collisions, 0, "{second:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 3, "front-drop must not duplicate survivors");
        for (agent_session_id, session_id, ledger_content) in survivor_snapshots {
            let matching = sessions
                .iter()
                .filter(|session| {
                    session.agent_session_id.as_deref() == Some(agent_session_id.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "duplicate survivor {agent_session_id}");
            assert_eq!(matching[0].id, session_id);
            assert_eq!(
                std::fs::read(sessions_dir.join(format!("{session_id}.toml")))
                    .expect("unchanged survivor ledger content"),
                ledger_content
            );
        }
    }

    #[test]
    fn deterministic_id_includes_effective_top_level_branch() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let source = |last_branch: &str| {
            json!({
                "repositoryRoot": repo,
                "lastBranch": last_branch,
                "timestamp": 1_710_000_000_000_i64,
                "history": [{
                    "branch": "",
                    "worktreePath": repo,
                    "toolId": "codex",
                    "toolLabel": "Codex",
                    "sessionId": "branch-fallback-session",
                    "timestamp": 1_710_000_000_000_i64
                }]
            })
        };
        write_legacy_file(&legacy_dir, "branch-fallback.json", &source("main"));

        let first = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        assert_eq!(first.imported, 1, "{first:?}");
        assert_eq!(imported_sessions(&sessions_dir)[0].branch, "main");

        write_legacy_file(
            &legacy_dir,
            "branch-fallback.json",
            &source("work/alternate"),
        );
        let second = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(second.imported, 1, "{second:?}");
        assert_eq!(second.collisions, 0, "{second:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 2);
        let main = sessions
            .iter()
            .find(|session| session.branch == "main")
            .expect("main fallback branch");
        let alternate = sessions
            .iter()
            .find(|session| session.branch == "work/alternate")
            .expect("alternate fallback branch");
        assert_ne!(main.id, alternate.id);
    }

    #[test]
    fn deterministic_session_id_v1_has_stable_golden_vector() {
        let data: LegacySessionData = serde_json::from_value(json!({
            "repositoryRoot": "fixed/repository",
            "lastBranch": "work/golden",
            "timestamp": 1_710_000_001_000_i64
        }))
        .expect("fixed legacy data");
        let entry = |session_id: Option<&str>, timestamp: i64| {
            let mut value = json!({
                "branch": "",
                "worktreePath": "fixed/worktree",
                "toolId": "codex-cli",
                "toolLabel": "Codex β",
                "mode": "continue",
                "model": "gpt-5",
                "reasoningLevel": "high",
                "skipPermissions": true,
                "toolVersion": "1.2.3",
                "collaborationModes": false,
                "dockerService": "app",
                "dockerForceHost": false,
                "dockerRecreate": true,
                "dockerBuild": true,
                "dockerKeep": false,
                "dockerContainerName": "golden-container",
                "dockerComposeArgs": ["--profile", "固定"],
                "timestamp": timestamp
            });
            if let Some(session_id) = session_id {
                value["sessionId"] = json!(session_id);
            }
            serde_json::from_value(value).expect("fixed legacy entry")
        };
        let logical = LogicalLegacyLaunch {
            start_index: 41,
            end_index: 42,
            start: entry(None, 1_710_000_000_000),
            end: entry(Some("golden-session"), 1_710_000_001_000),
        };

        assert_eq!(
            deterministic_session_id(
                Path::new("fixed/legacy/golden-history.json"),
                &data,
                &logical,
            ),
            "legacy-json-v1-d1787911f06eb70b919e72523d94a9b3"
        );
    }

    #[test]
    fn whitespace_only_session_id_is_treated_as_an_unfinished_start() {
        let entries = vec![
            LegacySessionEntry {
                branch: "main".to_string(),
                worktree_path: Some(PathBuf::from("/tmp/worktree")),
                tool_id: "codex-cli".to_string(),
                tool_label: "Codex".to_string(),
                session_id: Some("   ".to_string()),
                mode: None,
                model: None,
                reasoning_level: None,
                skip_permissions: None,
                tool_version: None,
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: 1,
            },
            LegacySessionEntry {
                branch: "main".to_string(),
                worktree_path: Some(PathBuf::from("/tmp/worktree")),
                tool_id: "codex-cli".to_string(),
                tool_label: "Codex".to_string(),
                session_id: Some("exact-id".to_string()),
                mode: None,
                model: None,
                reasoning_level: None,
                skip_permissions: None,
                tool_version: None,
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: 2,
            },
        ];

        let launches = logical_launches(&entries);

        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].start_index, 0);
        assert_eq!(launches[0].end_index, 1);
    }

    #[test]
    fn logical_pairing_work_is_linear_at_the_history_limit() {
        let mut entries = Vec::with_capacity(MAX_LEGACY_HISTORY_ENTRIES);
        for index in 0..MAX_LEGACY_HISTORY_ENTRIES {
            entries.push(LegacySessionEntry {
                branch: "main".to_string(),
                worktree_path: Some(PathBuf::from("/tmp/worktree")),
                tool_id: "codex-cli".to_string(),
                tool_label: "Codex".to_string(),
                session_id: (index >= MAX_LEGACY_HISTORY_ENTRIES / 2)
                    .then(|| format!("exact-{index}")),
                mode: None,
                model: None,
                reasoning_level: None,
                skip_permissions: None,
                tool_version: None,
                collaboration_modes: None,
                docker_service: None,
                docker_force_host: None,
                docker_recreate: None,
                docker_build: None,
                docker_keep: None,
                docker_container_name: None,
                docker_compose_args: None,
                timestamp: index as i64,
            });
        }

        let (_, operations) = logical_launches_with_operation_count(&entries);

        assert!(
            operations <= MAX_LEGACY_HISTORY_ENTRIES * 2,
            "pairing performed {operations} operations"
        );
    }

    #[test]
    fn repeated_import_is_idempotent_and_keeps_existing_collision_content() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "one.json",
            &json!({
                "repositoryRoot": repo,
                "lastBranch": "main",
                "lastUsedTool": "codex",
                "lastSessionId": "legacy-id",
                "toolLabel": "Codex",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }),
        );

        let first = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        let second = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        assert_eq!(first.imported, 1, "{first:?}");
        assert_eq!(second.unchanged, 1, "{second:?}");

        let mut session = imported_sessions(&sessions_dir).pop().expect("session");
        session.display_name = "Locally preserved".to_string();
        session
            .save(&sessions_dir)
            .expect("replace fixture for collision");

        let collision = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);
        assert_eq!(collision.collisions, 1, "{collision:?}");
        assert_eq!(
            imported_sessions(&sessions_dir)[0].display_name,
            "Locally preserved"
        );
    }

    #[test]
    fn maps_supported_metadata_without_fabricating_exact_resume_identity() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "metadata.json",
            &json!({
                "repositoryRoot": repo,
                "timestamp": 1_710_000_000_000_i64,
                "history": [{
                    "branch": "main",
                    "worktreePath": repo,
                    "toolId": "codex",
                    "toolLabel": "Codex Legacy",
                    "mode": "continue",
                    "model": "gpt-5",
                    "reasoningLevel": "high",
                    "skipPermissions": true,
                    "toolVersion": "1.2.3",
                    "dockerService": "app",
                    "dockerRecreate": true,
                    "dockerComposeArgs": ["--profile", "private-value"],
                    "timestamp": 1_710_000_000_000_i64
                }]
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.diagnostics.len(), 1, "{report:?}");
        assert!(!report.diagnostics[0].message.contains("private-value"));
        let session = imported_sessions(&sessions_dir).pop().expect("session");
        assert_eq!(session.agent_session_id, None);
        assert!(session.session_history.is_empty());
        assert_eq!(session.session_mode, crate::SessionMode::Continue);
        assert_eq!(session.model.as_deref(), Some("gpt-5"));
        assert_eq!(session.reasoning_level.as_deref(), Some("high"));
        assert!(session.skip_permissions);
        assert_eq!(session.tool_version.as_deref(), Some("1.2.3"));
        assert_eq!(session.runtime_target, crate::LaunchRuntimeTarget::Docker);
        assert_eq!(session.docker_service.as_deref(), Some("app"));
        assert_eq!(
            session.docker_lifecycle_intent,
            crate::DockerLifecycleIntent::Recreate
        );
        assert!(session.launch_args.is_empty());
    }

    #[test]
    fn malformed_history_entry_isolated_without_dropping_valid_siblings() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "partially-malformed.json",
            &json!({
                "repositoryRoot": repo,
                "history": [
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "sessionId": "valid-before",
                        "timestamp": 1_710_000_000_000_i64
                    },
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "sessionId": "invalid",
                        "timestamp": "not-a-number"
                    },
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "sessionId": "valid-after",
                        "timestamp": 1_710_000_002_000_i64
                    }
                ]
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 2, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.entry_index == Some(1)
                && diagnostic
                    .message
                    .contains("malformed legacy history entry")
        }));
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions
                .iter()
                .filter_map(|session| session.agent_session_id.as_deref())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["valid-before", "valid-after"])
        );
    }

    #[test]
    fn malformed_history_entry_is_a_conservative_pairing_barrier() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "pairing-barrier.json",
            &json!({
                "repositoryRoot": repo,
                "history": [
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "timestamp": 1_710_000_000_000_i64
                    },
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "timestamp": "unknown-entry"
                    },
                    {
                        "branch": "main",
                        "worktreePath": repo,
                        "toolId": "codex-cli",
                        "sessionId": "valid-after-barrier",
                        "timestamp": 1_710_000_002_000_i64
                    }
                ]
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 2, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        let sessions = imported_sessions(&sessions_dir);
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions
                .iter()
                .filter(|session| session.agent_session_id.is_none())
                .count(),
            1,
            "the start before an unknown entry must remain a separate launch"
        );
        assert_eq!(
            sessions
                .iter()
                .filter(|session| {
                    session.agent_session_id.as_deref() == Some("valid-after-barrier")
                })
                .count(),
            1
        );
    }

    #[test]
    fn scan_budgets_isolate_oversized_file_and_history_without_blocking_valid_input() {
        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&legacy_dir).expect("legacy dir");
        std::fs::create_dir_all(&repo).expect("repo");
        std::fs::write(
            legacy_dir.join("oversized.json"),
            vec![b' '; MAX_LEGACY_FILE_BYTES as usize + 1],
        )
        .expect("oversized fixture");
        let repeated_entry = json!({
            "branch": "main",
            "worktreePath": repo,
            "toolId": "codex",
            "timestamp": 1_710_000_000_000_i64
        });
        write_legacy_file(
            &legacy_dir,
            "too-many-entries.json",
            &json!({
                "repositoryRoot": repo,
                "history": vec![repeated_entry; MAX_LEGACY_HISTORY_ENTRIES + 1]
            }),
        );
        write_legacy_file(
            &legacy_dir,
            "valid.json",
            &json!({
                "repositoryRoot": repo,
                "lastBranch": "main",
                "lastUsedTool": "codex",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }),
        );

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.skipped, 2, "{report:?}");
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("file-size limit")),
            "{report:?}"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("history entry limit")),
            "{report:?}"
        );
        assert_eq!(imported_sessions(&sessions_dir).len(), 1);
    }

    #[test]
    fn non_directory_legacy_source_reports_a_diagnostic() {
        let temp = tempdir().expect("tempdir");
        let legacy_source = temp.path().join("sessions");
        std::fs::write(&legacy_source, b"not a directory").expect("legacy source file");

        let report = import_legacy_sessions_from_dir(&legacy_source, &temp.path().join("ledger"));

        assert_eq!(report.skipped, 1, "{report:?}");
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("directory")));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_legacy_directory_is_rejected_without_scanning_target() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let real_dir = temp.path().join("real-legacy");
        let linked_dir = temp.path().join("linked-legacy");
        let sessions_dir = temp.path().join("ledger");
        let repo = temp.path().join("repo");
        write_legacy_file(
            &real_dir,
            "outside.json",
            &json!({
                "repositoryRoot": repo,
                "lastBranch": "main",
                "lastUsedTool": "codex-cli",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }),
        );
        symlink(&real_dir, &linked_dir).expect("legacy directory symlink");

        let report = import_legacy_sessions_from_dir(&linked_dir, &sessions_dir);

        assert_eq!(report.imported, 0, "{report:?}");
        assert_eq!(report.skipped, 1, "{report:?}");
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("symlink")));
        assert!(!sessions_dir.exists());
    }

    #[test]
    fn ledger_load_prewarms_the_canonical_legacy_directory_once() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path().join(".gwt").join("sessions");
        let legacy_dir = temp.path().join(".config").join("gwt").join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "canonical.json",
            &json!({
                "repositoryRoot": repo,
                "lastBranch": "main",
                "lastUsedTool": "codex",
                "lastSessionId": "canonical-resume-id",
                "toolLabel": "Codex",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }),
        );

        let first = load_sessions_with_legacy_import(&sessions_dir);
        let second = load_sessions_with_legacy_import(&sessions_dir);

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(
            first[0].exact_resume_session_id(),
            Some("canonical-resume-id")
        );
        assert_eq!(
            std::fs::read_dir(&sessions_dir)
                .expect("sessions")
                .flatten()
                .filter(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("toml")
                )
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlink_and_malformed_sources_without_blocking_valid_files() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let legacy_dir = temp.path().join("legacy");
        let sessions_dir = temp.path().join("sessions");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        write_legacy_file(
            &legacy_dir,
            "valid.json",
            &json!({
                "repositoryRoot": repo,
                "lastBranch": "main",
                "lastUsedTool": "codex",
                "timestamp": 1_710_000_000_000_i64,
                "history": []
            }),
        );
        std::fs::write(legacy_dir.join("malformed.json"), b"{").expect("malformed");
        symlink(
            legacy_dir.join("valid.json"),
            legacy_dir.join("linked.json"),
        )
        .expect("symlink");

        let report = import_legacy_sessions_from_dir(&legacy_dir, &sessions_dir);

        assert_eq!(report.imported, 1, "{report:?}");
        assert_eq!(report.skipped, 2, "{report:?}");
        assert_eq!(imported_sessions(&sessions_dir).len(), 1);
    }
}

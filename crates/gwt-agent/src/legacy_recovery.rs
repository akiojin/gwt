//! Conservative import of pre-recovery-store agent session ledgers.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    fs::File,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Duration, Utc};
use gwt_core::recovery::{
    BindingQuality, CreateRecovery, ProviderRootBinding, ProviderRootCandidate, RecoveryLifecycle,
    RecoverySessionKind, RecoveryStore, RecoveryStoreError,
};
use serde::Deserialize;

use crate::session::{discover_session_toml_paths, SessionInventoryReadBudget};
use crate::{LaunchRuntimeTarget, Session};

/// Why an imported legacy session cannot be restored automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyRecoveryAttentionReason {
    /// The durable Session proves this was an ephemeral Intake, but its
    /// generated worktree disappeared and must be recreated from the pin.
    MissingIntakeWorktree,
    MissingExactResumeSessionId,
    PlaceholderResumeSessionId,
    MultipleResumeSessionIds {
        candidates: Vec<String>,
    },
    /// Only workspace/window projection metadata survived. It is never
    /// treated as a provider conversation binding.
    PlaceholderOnlyEvidence,
    /// Provider-native metadata found one plausible root, but its timestamp
    /// does not match the original gwt launch window closely enough to bind it
    /// without a user choice.
    ProviderNativeCandidateNeedsConfirmation,
    /// A current-format Session names a Recovery but never crossed a durable
    /// provider/lifecycle boundary. A leftover provider id alone must not
    /// turn stale history into an automatic startup launch.
    MissingLifecycleEvidence,
    SessionKindAmbiguous,
}

/// Provider-native metadata roots used only while migrating legacy ledgers.
///
/// The scanner reads bounded JSON metadata and never copies messages,
/// prompts, argv, or native history paths into the recovery store.
#[derive(Debug, Clone, Default)]
struct LegacyProviderHistoryRoots {
    codex_sessions: Option<PathBuf>,
    claude_projects: Option<PathBuf>,
    claude_sessions: Option<PathBuf>,
}

impl LegacyProviderHistoryRoots {
    fn from_environment() -> Self {
        let home = gwt_config::Settings::global_config_path()
            .and_then(|path| path.parent()?.parent().map(Path::to_path_buf));
        let codex_home = std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".codex")));
        let claude_home = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".claude")));
        Self {
            codex_sessions: codex_home.map(|root| root.join("sessions")),
            claude_projects: claude_home.as_ref().map(|root| root.join("projects")),
            claude_sessions: claude_home.map(|root| root.join("sessions")),
        }
    }
}

#[derive(Debug, Clone)]
struct NativeProviderRootObservation {
    root_id: String,
    evidence: BTreeSet<String>,
    observed_at: DateTime<Utc>,
    strong_launch_match: bool,
}

#[derive(Debug, Clone)]
struct NativeProviderRootDiscovery {
    observations: Vec<NativeProviderRootObservation>,
    scan_complete: bool,
}

impl Default for NativeProviderRootDiscovery {
    fn default() -> Self {
        Self {
            observations: Vec::new(),
            scan_complete: true,
        }
    }
}

impl NativeProviderRootDiscovery {
    fn provider_candidates(&self) -> Vec<ProviderRootCandidate> {
        self.observations
            .iter()
            .map(|observation| {
                let mut evidence = observation.evidence.clone();
                if !self.scan_complete {
                    evidence.insert(
                        "Provider-native metadata scan stopped at the global safety budget"
                            .to_string(),
                    );
                }
                ProviderRootCandidate {
                    root_id: observation.root_id.clone(),
                    evidence: evidence.into_iter().collect(),
                    observed_at: observation.observed_at,
                }
            })
            .collect()
    }
}

// One importer-wide budget is shared by every legacy Session. Directory
// enumeration and candidate files are bounded for every provider; only Codex
// structured metadata envelopes consume the line/byte budgets. Claude message
// transcripts are never opened by recovery discovery.
const MAX_NATIVE_METADATA_FILES: usize = 256;
const MAX_NATIVE_METADATA_LINES: usize = 2048;
const MAX_NATIVE_METADATA_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_NATIVE_METADATA_DIRECTORY_ENTRIES: usize = 4096;

#[derive(Debug, Clone)]
struct NativeMetadataScanBudget {
    files_remaining: usize,
    lines_remaining: usize,
    bytes_remaining: usize,
    directory_entries_remaining: usize,
    incomplete: bool,
}

impl Default for NativeMetadataScanBudget {
    fn default() -> Self {
        Self::new(
            MAX_NATIVE_METADATA_FILES,
            MAX_NATIVE_METADATA_LINES,
            MAX_NATIVE_METADATA_TOTAL_BYTES,
        )
    }
}

impl NativeMetadataScanBudget {
    fn new(files: usize, lines: usize, bytes: usize) -> Self {
        Self {
            files_remaining: files,
            lines_remaining: lines,
            bytes_remaining: bytes,
            directory_entries_remaining: MAX_NATIVE_METADATA_DIRECTORY_ENTRIES,
            incomplete: false,
        }
    }

    fn begin_file(&mut self) -> bool {
        if self.files_remaining == 0 {
            self.incomplete = true;
            return false;
        }
        self.files_remaining -= 1;
        true
    }

    fn begin_line(&mut self) -> bool {
        if self.lines_remaining == 0 {
            self.incomplete = true;
            return false;
        }
        self.lines_remaining -= 1;
        true
    }

    fn consume_bytes(&mut self, bytes: usize) -> bool {
        if bytes > self.bytes_remaining {
            self.bytes_remaining = 0;
            self.incomplete = true;
            return false;
        }
        self.bytes_remaining -= bytes;
        true
    }

    fn mark_incomplete(&mut self) {
        self.incomplete = true;
    }

    fn begin_directory_entry(&mut self) -> bool {
        if self.directory_entries_remaining == 0 {
            self.incomplete = true;
            return false;
        }
        self.directory_entries_remaining -= 1;
        true
    }

    fn ensure_capacity(&mut self) -> bool {
        if self.files_remaining == 0 || self.lines_remaining == 0 || self.bytes_remaining == 0 {
            self.incomplete = true;
            return false;
        }
        true
    }
}

/// Durable legacy source from which a placeholder candidate was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LegacyRecoveryPlaceholderSource {
    /// A persisted GUI window without a matching Session TOML.
    WindowState,
    /// A Workspace/Work agent projection without a matching Session TOML.
    WorkspaceProjection,
}

/// Whether a legacy projection entry represents a recoverable agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyRecoveryPlaceholderKind {
    Agent,
    Shell,
    NonAgent,
    Invalid,
}

/// Minimal, public-safe evidence needed to retain a legacy placeholder.
///
/// Callers deliberately cannot provide transcript/current-focus text or a
/// provider root id. Projection-only evidence always imports as Attention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecoveryPlaceholder {
    pub source: LegacyRecoveryPlaceholderSource,
    pub source_id: String,
    pub source_path: PathBuf,
    pub session_id: Option<String>,
    pub provider: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub session_kind: Option<RecoverySessionKind>,
    pub kind: LegacyRecoveryPlaceholderKind,
    pub runtime_is_live: bool,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

/// One legacy session imported with an exact, verified provider root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecoveryImportedExact {
    pub session_id: String,
    pub recovery_id: String,
    pub provider_root_id: String,
}

/// One legacy session retained in an explicit attention state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecoveryImportedAttention {
    pub session_id: String,
    pub recovery_id: String,
    pub provider_root_id: Option<String>,
    pub reasons: Vec<LegacyRecoveryAttentionReason>,
}

/// Why an otherwise discoverable legacy session was not imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyRecoverySkipReason {
    TerminalSession,
    PersistedSessionExists,
    DuplicatePlaceholder,
    ShellPlaceholder,
    NonAgentPlaceholder,
    LiveAgentPlaceholder,
    InvalidPlaceholder,
    MissingPlaceholderIdentity,
    MissingWorktree,
    WorktreeNotDirectory,
    RepoScopeMismatch {
        expected: String,
        actual: Option<String>,
    },
    ExistingRecord,
    ExistingTombstone,
    HeadOidUnavailable,
}

/// One skipped legacy session. Its source TOML and provider data remain intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecoverySkipped {
    pub session_id: String,
    pub recovery_id: Option<String>,
    pub reason: LegacyRecoverySkipReason,
}

/// Stage at which a legacy candidate could not be inspected or persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyRecoveryImportStage {
    ReadSessionsDirectory,
    ReadDirectoryEntry,
    LoadSession,
    ValidateRecoveryIdentity,
    InspectRecoveryStore,
    CreateRecovery,
    PinRecoveryBase,
    BindProviderRoot,
    RecordProviderRootCandidates,
    SetLifecycle,
    SyncSession,
    SyncPlaceholder,
}

/// A per-candidate import failure. Other candidates continue to be processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRecoveryImportError {
    pub session_path: PathBuf,
    pub session_id: Option<String>,
    pub stage: LegacyRecoveryImportStage,
    pub message: String,
}

/// Complete, non-fail-fast result of one legacy recovery import scan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyRecoveryImportReport {
    pub imported_exact: Vec<LegacyRecoveryImportedExact>,
    pub imported_attention: Vec<LegacyRecoveryImportedAttention>,
    pub skipped: Vec<LegacyRecoverySkipped>,
    pub errors: Vec<LegacyRecoveryImportError>,
}

/// Import eligible legacy Session TOMLs into one project-scoped recovery store.
pub fn import_legacy_recovery_sessions(
    sessions_dir: &Path,
    store: &RecoveryStore,
    expected_repo_id: &str,
) -> LegacyRecoveryImportReport {
    import_legacy_recovery_sessions_with_history_roots(
        sessions_dir,
        store,
        expected_repo_id,
        &LegacyProviderHistoryRoots::from_environment(),
    )
}

fn import_legacy_recovery_sessions_with_history_roots(
    sessions_dir: &Path,
    store: &RecoveryStore,
    expected_repo_id: &str,
    history_roots: &LegacyProviderHistoryRoots,
) -> LegacyRecoveryImportReport {
    let mut report = LegacyRecoveryImportReport::default();
    let mut native_scan_budget = NativeMetadataScanBudget::default();
    let mut session_read_budget = SessionInventoryReadBudget::default();
    let session_paths = match discover_session_toml_paths(sessions_dir) {
        Ok(paths) => paths,
        Err(error) => {
            report.errors.push(import_error(
                sessions_dir,
                None,
                LegacyRecoveryImportStage::ReadSessionsDirectory,
                error,
            ));
            return report;
        }
    };

    for session_path in session_paths {
        let session =
            match Session::load_with_inventory_budget(&session_path, &mut session_read_budget) {
                Ok(session) => session,
                Err(error) => {
                    report.errors.push(import_error(
                        &session_path,
                        None,
                        LegacyRecoveryImportStage::LoadSession,
                        error,
                    ));
                    continue;
                }
            };
        import_session(
            &session_path,
            &session,
            store,
            expected_repo_id,
            history_roots,
            &mut native_scan_budget,
            &mut report,
        );
    }

    report
}

/// Import agent placeholders that survived without a Session TOML.
///
/// The caller supplies the complete set of persisted Session ids discovered
/// during the same startup inventory. Those ids always win over projections.
/// Window and Workspace entries with the same real session id are imported at
/// most once, preferring window evidence because it can carry an explicit lane.
pub fn import_legacy_recovery_placeholders(
    placeholders: &[LegacyRecoveryPlaceholder],
    persisted_session_ids: &BTreeSet<String>,
    store: &RecoveryStore,
    expected_repo_id: &str,
) -> LegacyRecoveryImportReport {
    let mut report = LegacyRecoveryImportReport::default();
    if placeholders.is_empty() {
        return report;
    }
    let mut existing_recoveries = BTreeMap::new();
    match store.list() {
        Ok(records) => {
            for record in records {
                existing_recoveries
                    .entry(record.session_id)
                    .or_insert(record.recovery_id);
            }
        }
        Err(error) => {
            report.errors.push(import_error(
                &placeholders[0].source_path,
                placeholders[0].session_id.as_deref(),
                LegacyRecoveryImportStage::InspectRecoveryStore,
                error,
            ));
            return report;
        }
    }
    let persisted_session_ids = persisted_session_ids
        .iter()
        .map(|session_id| session_id.trim())
        .filter(|session_id| !session_id.is_empty())
        .collect::<BTreeSet<_>>();
    let mut ordered = placeholders.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        placeholder_dedup_key(left)
            .cmp(&placeholder_dedup_key(right))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| right.observed_at.cmp(&left.observed_at))
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    let mut seen = BTreeSet::new();

    for candidate in ordered {
        import_placeholder(
            candidate,
            &persisted_session_ids,
            &existing_recoveries,
            &mut seen,
            store,
            expected_repo_id,
            &mut report,
        );
    }

    report
}

fn import_placeholder(
    candidate: &LegacyRecoveryPlaceholder,
    persisted_session_ids: &BTreeSet<&str>,
    existing_recoveries: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    store: &RecoveryStore,
    expected_repo_id: &str,
    report: &mut LegacyRecoveryImportReport,
) {
    let source_id = candidate.source_id.trim();
    let real_session_id = candidate
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty());
    let report_session_id = real_session_id.unwrap_or(source_id).to_string();

    // A bare GUI window id is allocated from the lowest free slot and can be
    // reused by an unrelated window. Without a Session or projection identity
    // there is no stable owner to recover or to mark terminal after discard.
    // Treating the window id as a synthetic recovery identity would therefore
    // either resurrect it forever or suppress a future, unrelated pane.
    if candidate.source == LegacyRecoveryPlaceholderSource::WindowState && real_session_id.is_none()
    {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: None,
            reason: LegacyRecoverySkipReason::MissingPlaceholderIdentity,
        });
        return;
    }

    let immediate_skip = match candidate.kind {
        LegacyRecoveryPlaceholderKind::Shell => Some(LegacyRecoverySkipReason::ShellPlaceholder),
        LegacyRecoveryPlaceholderKind::NonAgent => {
            Some(LegacyRecoverySkipReason::NonAgentPlaceholder)
        }
        LegacyRecoveryPlaceholderKind::Invalid => {
            Some(LegacyRecoverySkipReason::InvalidPlaceholder)
        }
        LegacyRecoveryPlaceholderKind::Agent if candidate.runtime_is_live => {
            Some(LegacyRecoverySkipReason::LiveAgentPlaceholder)
        }
        LegacyRecoveryPlaceholderKind::Agent => None,
    };
    if let Some(reason) = immediate_skip {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: None,
            reason,
        });
        return;
    }

    if real_session_id.is_none() && source_id.is_empty() {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: String::new(),
            recovery_id: None,
            reason: LegacyRecoverySkipReason::MissingPlaceholderIdentity,
        });
        return;
    }
    if real_session_id.is_some_and(|session_id| persisted_session_ids.contains(session_id)) {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: None,
            reason: LegacyRecoverySkipReason::PersistedSessionExists,
        });
        return;
    }
    if let Some(existing_recovery_id) =
        real_session_id.and_then(|session_id| existing_recoveries.get(session_id))
    {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(existing_recovery_id.clone()),
            reason: LegacyRecoverySkipReason::ExistingRecord,
        });
        return;
    }

    let dedup_key = placeholder_dedup_key(candidate);
    let recovery_id = placeholder_recovery_id(expected_repo_id, &dedup_key);

    match store.load_tombstone(&recovery_id) {
        Ok(Some(tombstone)) => {
            if candidate.source == LegacyRecoveryPlaceholderSource::WorkspaceProjection {
                if let Err(error) =
                    sync_terminal_workspace_projection(candidate, tombstone.purged_at)
                {
                    report.errors.push(import_error(
                        &candidate.source_path,
                        real_session_id,
                        LegacyRecoveryImportStage::SyncPlaceholder,
                        error,
                    ));
                    return;
                }
            }
            report.skipped.push(LegacyRecoverySkipped {
                session_id: report_session_id,
                recovery_id: Some(recovery_id),
                reason: LegacyRecoverySkipReason::ExistingTombstone,
            });
            return;
        }
        Ok(None) => {}
        Err(error) => {
            report.errors.push(import_error(
                &candidate.source_path,
                real_session_id,
                LegacyRecoveryImportStage::InspectRecoveryStore,
                error,
            ));
            return;
        }
    }
    if !seen.insert(dedup_key) {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: None,
            reason: LegacyRecoverySkipReason::DuplicatePlaceholder,
        });
        return;
    }
    match store.load(&recovery_id) {
        Ok(Some(_)) => {
            report.skipped.push(LegacyRecoverySkipped {
                session_id: report_session_id,
                recovery_id: Some(recovery_id),
                reason: LegacyRecoverySkipReason::ExistingRecord,
            });
            return;
        }
        Ok(None) => {}
        Err(error) => {
            report.errors.push(import_error(
                &candidate.source_path,
                real_session_id,
                LegacyRecoveryImportStage::InspectRecoveryStore,
                error,
            ));
            return;
        }
    }

    let Some(worktree_path) = candidate.worktree_path.as_deref() else {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(recovery_id),
            reason: LegacyRecoverySkipReason::MissingWorktree,
        });
        return;
    };
    if !worktree_path.exists() {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(recovery_id),
            reason: LegacyRecoverySkipReason::MissingWorktree,
        });
        return;
    }
    if !worktree_path.is_dir() {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(recovery_id),
            reason: LegacyRecoverySkipReason::WorktreeNotDirectory,
        });
        return;
    }
    let measured_repo_id = gwt_core::paths::project_scope_hash(worktree_path).to_string();
    if measured_repo_id != expected_repo_id {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(recovery_id),
            reason: LegacyRecoverySkipReason::RepoScopeMismatch {
                expected: expected_repo_id.to_string(),
                actual: Some(measured_repo_id),
            },
        });
        return;
    }
    let Some(head_oid) = resolve_git_head_oid(worktree_path) else {
        report.skipped.push(LegacyRecoverySkipped {
            session_id: report_session_id,
            recovery_id: Some(recovery_id),
            reason: LegacyRecoverySkipReason::HeadOidUnavailable,
        });
        return;
    };

    let mut attention_reasons = vec![LegacyRecoveryAttentionReason::PlaceholderOnlyEvidence];
    let session_kind = candidate.session_kind.unwrap_or_else(|| {
        if is_legacy_intake_path(worktree_path) && is_detached_head(worktree_path) == Some(true) {
            RecoverySessionKind::Intake
        } else {
            attention_reasons.push(LegacyRecoveryAttentionReason::SessionKindAmbiguous);
            RecoverySessionKind::Execution
        }
    });
    let provider = candidate
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let request = CreateRecovery {
        recovery_id: recovery_id.clone(),
        session_id: real_session_id
            .map(str::to_string)
            .unwrap_or_else(|| placeholder_synthetic_session_id(candidate)),
        repo_id: expected_repo_id.to_string(),
        session_kind,
        worktree_path: worktree_path.to_path_buf(),
        launch_base_ref: None,
        launch_base_oid: head_oid.clone(),
        launch_head_oid: head_oid,
        provider,
        model: None,
        runtime: "unknown".to_string(),
        initial_prompt: String::new(),
        created_at: candidate.observed_at,
    };
    if let Err(error) = store.create(request, placeholder_operation_id(&recovery_id, "create")) {
        report.errors.push(import_error(
            &candidate.source_path,
            real_session_id,
            LegacyRecoveryImportStage::CreateRecovery,
            error,
        ));
        return;
    }
    let lifecycle_reason = format!(
        "legacy_placeholder_attention:{}",
        attention_reasons
            .iter()
            .map(attention_reason_code)
            .collect::<Vec<_>>()
            .join(",")
    );
    if let Err(error) = store.set_lifecycle(
        &recovery_id,
        RecoveryLifecycle::Attention,
        Some(lifecycle_reason),
        placeholder_operation_id(&recovery_id, "set-lifecycle"),
    ) {
        report.errors.push(import_error(
            &candidate.source_path,
            real_session_id,
            LegacyRecoveryImportStage::SetLifecycle,
            error,
        ));
        return;
    }

    report
        .imported_attention
        .push(LegacyRecoveryImportedAttention {
            session_id: real_session_id
                .map(str::to_string)
                .unwrap_or_else(|| placeholder_synthetic_session_id(candidate)),
            recovery_id,
            provider_root_id: None,
            reasons: attention_reasons,
        });
}

fn sync_terminal_workspace_projection(
    candidate: &LegacyRecoveryPlaceholder,
    terminal_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let projection =
        gwt_core::workspace_projection::load_workspace_projection_from_path(&candidate.source_path)
            .map_err(|error| format!("load legacy Workspace projection: {error}"))?
            .ok_or_else(|| {
                "legacy Workspace projection disappeared before terminal sync".to_string()
            })?;
    let project_root = projection.project_root;
    let matched = gwt_core::workspace_projection::mutate_workspace_projection_at(
        &candidate.source_path,
        &project_root,
        |projection| {
            let mut matched = 0usize;
            for agent in &mut projection.agents {
                if !workspace_projection_agent_matches(candidate, agent) {
                    continue;
                }
                agent.status_category =
                    gwt_core::workspace_projection::WorkspaceStatusCategory::Done;
                // A terminal migration needs only the existing public identity.
                // Do not retain legacy discussion text as suppression evidence.
                agent.current_focus = None;
                agent.updated_at = agent.updated_at.max(terminal_at);
                matched += 1;
            }
            if matched > 0 {
                projection.updated_at = projection.updated_at.max(terminal_at);
            }
            Ok(matched)
        },
    )
    .map_err(|error| format!("persist terminal Workspace projection marker: {error}"))?;
    if matched == 0 {
        return Err("legacy Workspace projection identity disappeared before terminal sync".into());
    }
    Ok(())
}

fn workspace_projection_agent_matches(
    candidate: &LegacyRecoveryPlaceholder,
    agent: &gwt_core::workspace_projection::WorkspaceAgentSummary,
) -> bool {
    let candidate_session_id = candidate
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty());
    if candidate_session_id.is_some_and(|session_id| agent.session_id.trim() == session_id) {
        return true;
    }

    let source_id = candidate.source_id.trim();
    if agent.window_id.as_deref().is_some_and(|window_id| {
        !window_id.trim().is_empty()
            && (source_id == window_id || source_id.ends_with(&format!("::{window_id}")))
    }) {
        return true;
    }
    source_id.ends_with(&format!("::projection::{}", agent.session_id.trim()))
}

fn placeholder_dedup_key(candidate: &LegacyRecoveryPlaceholder) -> String {
    if let Some(session_id) = candidate
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        return format!("session:{session_id}");
    }
    format!("source:{}", candidate.source_id.trim())
}

fn placeholder_synthetic_session_id(candidate: &LegacyRecoveryPlaceholder) -> String {
    format!(
        "legacy-placeholder:{}:{}",
        placeholder_source_code(candidate.source),
        candidate.source_id.trim()
    )
}

fn placeholder_source_code(source: LegacyRecoveryPlaceholderSource) -> &'static str {
    match source {
        LegacyRecoveryPlaceholderSource::WindowState => "window",
        LegacyRecoveryPlaceholderSource::WorkspaceProjection => "projection",
    }
}

fn placeholder_recovery_id(expected_repo_id: &str, dedup_key: &str) -> String {
    let encoded_key = dedup_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let digest = gwt_core::repo_hash::compute_repo_hash(&format!(
        "legacy-placeholder.invalid/{expected_repo_id}/{encoded_key}"
    ));
    format!("legacy-placeholder-{}", digest.as_str())
}

fn placeholder_operation_id(recovery_id: &str, mutation: &str) -> String {
    format!("legacy-placeholder-import-v1:{recovery_id}:{mutation}")
}

fn import_session(
    session_path: &Path,
    session: &Session,
    store: &RecoveryStore,
    expected_repo_id: &str,
    history_roots: &LegacyProviderHistoryRoots,
    native_scan_budget: &mut NativeMetadataScanBudget,
    report: &mut LegacyRecoveryImportReport,
) {
    if session
        .recovery_launch_stage
        .is_some_and(crate::session::RecoveryLaunchStage::is_terminal)
    {
        report.skipped.push(skipped(
            session,
            session.recovery_id.clone(),
            LegacyRecoverySkipReason::TerminalSession,
        ));
        return;
    }
    let recovery_id = session
        .recovery_id
        .clone()
        .unwrap_or_else(|| format!("legacy-{}", session.id));

    match store.load_tombstone(&recovery_id) {
        Ok(Some(tombstone)) => {
            if let Err(error) =
                sync_terminal_recovery_session(session_path, session, tombstone.lifecycle)
            {
                report.errors.push(import_error(
                    session_path,
                    Some(&session.id),
                    LegacyRecoveryImportStage::SyncSession,
                    error,
                ));
                return;
            }
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::ExistingTombstone,
            ));
            return;
        }
        Ok(None) => {}
        Err(RecoveryStoreError::InvalidRecoveryId(_)) => {
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::ValidateRecoveryIdentity,
                format!("invalid recovery identity: {recovery_id}"),
            ));
            return;
        }
        Err(error) => {
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::InspectRecoveryStore,
                error,
            ));
            return;
        }
    }
    match store.load(&recovery_id) {
        Ok(Some(mut record)) => {
            let expected_runtime = match session.runtime_target {
                LaunchRuntimeTarget::Host => "host",
                LaunchRuntimeTarget::Docker => "docker",
            };
            let (expected_kind, _) = classify_session_kind(session);
            if record.session_id != session.id
                || record.repo_id != expected_repo_id
                || !same_legacy_worktree_path(&record.worktree_path, &session.worktree_path)
                || record.provider != session.agent_id.to_string()
                || record.runtime != expected_runtime
                || record.session_kind != expected_kind
            {
                report.errors.push(import_error(
                    session_path,
                    Some(&session.id),
                    LegacyRecoveryImportStage::ValidateRecoveryIdentity,
                    format!(
                        "recovery {recovery_id} owner metadata does not match Session {}",
                        session.id
                    ),
                ));
                return;
            }
            if record.provider_root.is_none()
                && !matches!(
                    record.lifecycle,
                    RecoveryLifecycle::Resolved | RecoveryLifecycle::Discarded
                )
            {
                let native_discovery =
                    discover_native_provider_roots(session, history_roots, native_scan_budget);
                let root_assessment = assess_provider_root(session, &native_discovery);
                let candidates = provider_root_candidates_for_attention(
                    session,
                    &native_discovery,
                    &root_assessment,
                );
                if !candidates.is_empty() {
                    let candidates_operation_id =
                        provider_root_candidates_operation_id(&recovery_id, &candidates);
                    if let Err(error) = store.record_provider_root_candidates(
                        &recovery_id,
                        candidates,
                        candidates_operation_id,
                    ) {
                        report.errors.push(import_error(
                            session_path,
                            Some(&session.id),
                            LegacyRecoveryImportStage::RecordProviderRootCandidates,
                            error,
                        ));
                        return;
                    }
                }
                let (_, kind_attention) = classify_session_kind(session);
                let mut attention_reasons = Vec::new();
                match root_assessment {
                    ProviderRootAssessment::Exact {
                        root_id,
                        observed_at,
                    } => {
                        if let Err(error) = store.bind_root(
                            &recovery_id,
                            ProviderRootBinding {
                                root_id: root_id.clone(),
                                session_tree_id: None,
                                quality: BindingQuality::Verified,
                                bound_at: observed_at,
                            },
                            native_operation_id(&recovery_id, "bind-root", &root_id),
                        ) {
                            report.errors.push(import_error(
                                session_path,
                                Some(&session.id),
                                LegacyRecoveryImportStage::BindProviderRoot,
                                error,
                            ));
                            return;
                        }
                    }
                    ProviderRootAssessment::Attention(reason) => attention_reasons.push(reason),
                }
                if let Some(reason) = kind_attention {
                    attention_reasons.push(reason);
                }
                let (lifecycle, lifecycle_reason) = legacy_import_lifecycle(&attention_reasons);
                if let Err(error) = store.set_lifecycle(
                    &recovery_id,
                    lifecycle,
                    Some(lifecycle_reason.clone()),
                    native_operation_id(&recovery_id, "set-lifecycle", &lifecycle_reason),
                ) {
                    report.errors.push(import_error(
                        session_path,
                        Some(&session.id),
                        LegacyRecoveryImportStage::SetLifecycle,
                        error,
                    ));
                    return;
                }
                match store.load(&recovery_id) {
                    Ok(Some(updated)) => record = updated,
                    Ok(None) => {
                        report.errors.push(import_error(
                            session_path,
                            Some(&session.id),
                            LegacyRecoveryImportStage::InspectRecoveryStore,
                            "recovery disappeared after provider-native metadata update",
                        ));
                        return;
                    }
                    Err(error) => {
                        report.errors.push(import_error(
                            session_path,
                            Some(&session.id),
                            LegacyRecoveryImportStage::InspectRecoveryStore,
                            error,
                        ));
                        return;
                    }
                }
            }
            let metadata = LegacyRecoverySessionMetadata {
                recovery_id: &record.recovery_id,
                repo_id: &record.repo_id,
                session_kind: record.session_kind,
                provider_root_id: record
                    .provider_root
                    .as_ref()
                    .map(|root| root.root_id.as_str()),
                provider_binding_quality: record.provider_root.as_ref().map(|root| root.quality),
                checkpoint_revision: record.checkpoint_revision,
            };
            if let Err(error) =
                sync_legacy_session_recovery_metadata(session_path, session, metadata)
            {
                report.errors.push(import_error(
                    session_path,
                    Some(&session.id),
                    LegacyRecoveryImportStage::SyncSession,
                    error,
                ));
                return;
            }
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::ExistingRecord,
            ));
            return;
        }
        Ok(None) => {}
        Err(error) => {
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::InspectRecoveryStore,
                error,
            ));
            return;
        }
    }

    let missing_intake = !session.worktree_path.exists();
    let (head_oid, session_kind, kind_attention, pin_repo) = if missing_intake {
        let explicitly_intake =
            session.session_kind == Some(gwt_skills::SessionKind::Intake) || session.is_ephemeral;
        let Some(project_root) = session
            .project_state_root
            .as_deref()
            .filter(|path| path.is_dir())
        else {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::MissingWorktree,
            ));
            return;
        };
        let measured_repo_id = gwt_core::paths::project_scope_hash(project_root).to_string();
        // A missing explicit Intake still has stronger project ownership
        // evidence than its pre-upgrade repo_hash: the surviving canonical
        // project state root and the validated generated Intake target. Accept
        // the measured project and repair stale persisted scope metadata just
        // as the live-worktree path below does.
        if !explicitly_intake
            || measured_repo_id != expected_repo_id
            || gwt_git::recovery::validate_recovery_intake_target_path(
                project_root,
                &session.worktree_path,
            )
            .is_err()
        {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                if measured_repo_id != expected_repo_id {
                    LegacyRecoverySkipReason::RepoScopeMismatch {
                        expected: expected_repo_id.to_string(),
                        actual: Some(measured_repo_id),
                    }
                } else {
                    LegacyRecoverySkipReason::MissingWorktree
                },
            ));
            return;
        }
        let Some(recorded_oid) = session
            .launch_base_oid
            .as_deref()
            .map(str::trim)
            .filter(|oid| !oid.is_empty())
            .and_then(|oid| resolve_git_commit_oid(project_root, oid))
        else {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::HeadOidUnavailable,
            ));
            return;
        };
        (
            recorded_oid,
            RecoverySessionKind::Intake,
            None,
            project_root.to_path_buf(),
        )
    } else {
        if !session.worktree_path.is_dir() {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::WorktreeNotDirectory,
            ));
            return;
        }
        let measured_repo_id =
            gwt_core::paths::project_scope_hash(&session.worktree_path).to_string();
        // A live worktree is stronger evidence than legacy persisted scope
        // metadata. Reject an actually different repository, but repair a
        // stale repo_hash after the measured repository has been accepted.
        if measured_repo_id != expected_repo_id {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::RepoScopeMismatch {
                    expected: expected_repo_id.to_string(),
                    actual: Some(measured_repo_id),
                },
            ));
            return;
        }
        let Some(head_oid) = resolve_git_head_oid(&session.worktree_path) else {
            report.skipped.push(skipped(
                session,
                Some(recovery_id),
                LegacyRecoverySkipReason::HeadOidUnavailable,
            ));
            return;
        };
        let (session_kind, kind_attention) = classify_session_kind(session);
        (
            head_oid,
            session_kind,
            kind_attention,
            session.worktree_path.clone(),
        )
    };
    let native_discovery =
        discover_native_provider_roots(session, history_roots, native_scan_budget);
    let root_assessment = assess_provider_root(session, &native_discovery);
    let provider_root_id = match &root_assessment {
        ProviderRootAssessment::Exact { root_id, .. } => Some(root_id.clone()),
        ProviderRootAssessment::Attention(_) => None,
    };
    let mut attention_reasons = Vec::new();
    if let ProviderRootAssessment::Attention(reason) = &root_assessment {
        attention_reasons.push(reason.clone());
    }
    if let Some(reason) = kind_attention {
        attention_reasons.push(reason);
    }
    if missing_intake {
        attention_reasons.push(LegacyRecoveryAttentionReason::MissingIntakeWorktree);
    }
    if current_recovery_lifecycle_is_unproven(session) {
        attention_reasons.push(LegacyRecoveryAttentionReason::MissingLifecycleEvidence);
    }

    let launch_base_oid = session
        .launch_base_oid
        .as_deref()
        .map(str::trim)
        .filter(|oid| !oid.is_empty())
        .unwrap_or(&head_oid)
        .to_string();
    let request = CreateRecovery {
        recovery_id: recovery_id.clone(),
        session_id: session.id.clone(),
        repo_id: expected_repo_id.to_string(),
        session_kind,
        worktree_path: session.worktree_path.clone(),
        launch_base_ref: session.ephemeral_base_ref.clone(),
        launch_base_oid: launch_base_oid.clone(),
        launch_head_oid: head_oid,
        provider: session.agent_id.to_string(),
        model: session.model.clone(),
        runtime: match session.runtime_target {
            LaunchRuntimeTarget::Host => "host",
            LaunchRuntimeTarget::Docker => "docker",
        }
        .to_string(),
        // Legacy argv is deliberately not interpreted as user-authored input.
        initial_prompt: String::new(),
        created_at: session.created_at,
    };
    if let Err(error) = store.create(request, operation_id(&recovery_id, "create")) {
        report.errors.push(import_error(
            session_path,
            Some(&session.id),
            LegacyRecoveryImportStage::CreateRecovery,
            error,
        ));
        return;
    }

    if session_kind == RecoverySessionKind::Intake {
        if let Err(error) =
            gwt_git::recovery::ensure_recovery_base_pin(&pin_repo, &recovery_id, &launch_base_oid)
        {
            let reason = format!("legacy_import_attention:base_pin_failed:{error}");
            let _ = store.set_lifecycle(
                &recovery_id,
                RecoveryLifecycle::Attention,
                Some(reason),
                operation_id(&recovery_id, "base-pin-failed"),
            );
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::PinRecoveryBase,
                error,
            ));
            return;
        }
    }

    if let ProviderRootAssessment::Exact {
        root_id,
        observed_at,
    } = &root_assessment
    {
        let binding = ProviderRootBinding {
            root_id: root_id.clone(),
            session_tree_id: None,
            quality: BindingQuality::Verified,
            bound_at: *observed_at,
        };
        if let Err(error) = store.bind_root(
            &recovery_id,
            binding,
            operation_id(&recovery_id, "bind-root"),
        ) {
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::BindProviderRoot,
                error,
            ));
            return;
        }
    }

    let provider_root_candidates =
        provider_root_candidates_for_attention(session, &native_discovery, &root_assessment);
    if !provider_root_candidates.is_empty() {
        let candidates_operation_id =
            provider_root_candidates_operation_id(&recovery_id, &provider_root_candidates);
        if let Err(error) = store.record_provider_root_candidates(
            &recovery_id,
            provider_root_candidates,
            candidates_operation_id,
        ) {
            report.errors.push(import_error(
                session_path,
                Some(&session.id),
                LegacyRecoveryImportStage::RecordProviderRootCandidates,
                error,
            ));
            return;
        }
    }

    let (lifecycle, lifecycle_reason) = legacy_import_lifecycle(&attention_reasons);
    if let Err(error) = store.set_lifecycle(
        &recovery_id,
        lifecycle,
        Some(lifecycle_reason),
        operation_id(&recovery_id, "set-lifecycle"),
    ) {
        report.errors.push(import_error(
            session_path,
            Some(&session.id),
            LegacyRecoveryImportStage::SetLifecycle,
            error,
        ));
        return;
    }

    let metadata = LegacyRecoverySessionMetadata {
        recovery_id: &recovery_id,
        repo_id: expected_repo_id,
        session_kind,
        provider_root_id: provider_root_id.as_deref(),
        provider_binding_quality: provider_root_id.as_ref().map(|_| BindingQuality::Verified),
        checkpoint_revision: 0,
    };
    if let Err(error) = sync_legacy_session_recovery_metadata(session_path, session, metadata) {
        report.errors.push(import_error(
            session_path,
            Some(&session.id),
            LegacyRecoveryImportStage::SyncSession,
            error,
        ));
        return;
    }

    if attention_reasons.is_empty() {
        report.imported_exact.push(LegacyRecoveryImportedExact {
            session_id: session.id.clone(),
            recovery_id,
            provider_root_id: provider_root_id
                .expect("an exact import must have a verified provider root"),
        });
    } else {
        report
            .imported_attention
            .push(LegacyRecoveryImportedAttention {
                session_id: session.id.clone(),
                recovery_id,
                provider_root_id,
                reasons: attention_reasons,
            });
    }
}

fn same_legacy_worktree_path(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn sync_terminal_recovery_session(
    session_path: &Path,
    source: &Session,
    lifecycle: RecoveryLifecycle,
) -> io::Result<()> {
    let stage = match lifecycle {
        RecoveryLifecycle::Resolved => crate::session::RecoveryLaunchStage::Resolved,
        RecoveryLifecycle::Discarded => crate::session::RecoveryLaunchStage::Discarded,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovery tombstone lifecycle is not terminal",
            ));
        }
    };
    let sessions_dir = session_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "legacy Session path has no parent directory",
        )
    })?;
    let ledger_id = session_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "legacy Session path has no UTF-8 ledger id",
            )
        })?;
    crate::update_session(sessions_dir, ledger_id, |current| {
        if current.id != source.id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal recovery Session identity changed during update",
            ));
        }
        current.recovery_launch_stage = Some(stage);
        current.recovery_lease = None;
        current.restore_window_on_startup = false;
        current.startup_restore_intent_recorded = true;
        current.status = crate::AgentStatus::Stopped;
        current.updated_at = chrono::Utc::now();
        Ok(())
    })
    .map(|_| ())
}

struct LegacyRecoverySessionMetadata<'a> {
    recovery_id: &'a str,
    repo_id: &'a str,
    session_kind: RecoverySessionKind,
    provider_root_id: Option<&'a str>,
    provider_binding_quality: Option<BindingQuality>,
    checkpoint_revision: u64,
}

fn sync_legacy_session_recovery_metadata(
    session_path: &Path,
    source: &Session,
    metadata: LegacyRecoverySessionMetadata<'_>,
) -> io::Result<()> {
    let sessions_dir = session_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "legacy Session path has no parent directory",
        )
    })?;
    let ledger_id = session_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "legacy Session path has no UTF-8 ledger id",
            )
        })?;

    crate::update_session(sessions_dir, ledger_id, |current| {
        if current.id != source.id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy Session ledger id changed from {} to {}",
                    source.id, current.id
                ),
            ));
        }
        current.recovery_id = Some(metadata.recovery_id.to_string());
        current.repo_hash = Some(metadata.repo_id.to_string());
        current.session_kind = Some(match metadata.session_kind {
            RecoverySessionKind::Intake => gwt_skills::SessionKind::Intake,
            RecoverySessionKind::Execution => gwt_skills::SessionKind::Execution,
        });
        if metadata.session_kind == RecoverySessionKind::Intake {
            current.is_ephemeral = true;
        }
        current.checkpoint_revision = metadata.checkpoint_revision;

        if let Some(quality) = metadata.provider_binding_quality {
            current.observe_provider_root_role(crate::session::ProviderRootRole::Root)?;
            current.provider_binding_quality = Some(if quality.is_authoritative() {
                crate::session::ProviderBindingQuality::Verified
            } else {
                crate::session::ProviderBindingQuality::Inferred
            });
            if quality.is_authoritative()
                && current.recovery_launch_stage != Some(crate::session::RecoveryLaunchStage::Ready)
            {
                current.advance_recovery_launch_stage(
                    crate::session::RecoveryLaunchStage::ProviderBound,
                )?;
            }
            if quality.is_authoritative() {
                if let Some(root_id) = metadata
                    .provider_root_id
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                {
                    if current.agent_session_id.as_deref() != Some(root_id) {
                        if let Some(previous_root) = current
                            .agent_session_id
                            .as_deref()
                            .filter(|id| current.is_resumable_conversation(id))
                            .map(str::to_string)
                        {
                            if !current
                                .session_history
                                .iter()
                                .any(|entry| entry.agent_session_id == previous_root)
                            {
                                current.session_history.push(
                                    crate::session::AgentSessionHistoryEntry {
                                        agent_session_id: previous_root,
                                        started_at: current.updated_at,
                                    },
                                );
                            }
                        }
                        if !current
                            .session_history
                            .iter()
                            .any(|entry| entry.agent_session_id == root_id)
                        {
                            current.session_history.push(
                                crate::session::AgentSessionHistoryEntry {
                                    agent_session_id: root_id.to_string(),
                                    started_at: source.updated_at,
                                },
                            );
                        }
                        current.agent_session_id = Some(root_id.to_string());
                    }
                }
            }
        } else {
            current.provider_binding_quality = None;
        }
        Ok(())
    })
    .map(|_| ())
}

enum ProviderRootAssessment {
    Exact {
        root_id: String,
        observed_at: DateTime<Utc>,
    },
    Attention(LegacyRecoveryAttentionReason),
}

fn ledger_provider_root_candidates(session: &Session) -> Vec<ProviderRootCandidate> {
    let mut candidates = BTreeMap::<String, (BTreeSet<String>, DateTime<Utc>)>::new();
    if let Some(root_id) = session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|root_id| session.is_resumable_conversation(root_id))
    {
        let entry = candidates
            .entry(root_id.to_string())
            .or_insert_with(|| (BTreeSet::new(), session.updated_at));
        entry
            .0
            .insert("Legacy Session current provider id".to_string());
        entry.1 = entry.1.max(session.updated_at);
    }
    for history in &session.session_history {
        let root_id = history.agent_session_id.trim();
        if !session.is_resumable_conversation(root_id) {
            continue;
        }
        let entry = candidates
            .entry(root_id.to_string())
            .or_insert_with(|| (BTreeSet::new(), history.started_at));
        entry.0.insert("Legacy Session history".to_string());
        entry.1 = entry.1.max(history.started_at);
    }
    candidates
        .into_iter()
        .map(|(root_id, (evidence, observed_at))| ProviderRootCandidate {
            root_id,
            evidence: evidence.into_iter().collect(),
            observed_at,
        })
        .collect()
}

fn provider_root_candidates_for_attention(
    session: &Session,
    native_discovery: &NativeProviderRootDiscovery,
    assessment: &ProviderRootAssessment,
) -> Vec<ProviderRootCandidate> {
    if !matches!(assessment, ProviderRootAssessment::Attention(_)) {
        return Vec::new();
    }
    let ledger_candidates = ledger_provider_root_candidates(session);
    if ledger_candidates.len() > 1 {
        ledger_candidates
    } else {
        native_discovery.provider_candidates()
    }
}

fn assess_provider_root(
    session: &Session,
    native_discovery: &NativeProviderRootDiscovery,
) -> ProviderRootAssessment {
    let mut candidates = BTreeSet::new();
    if let Some(candidate) = session.agent_session_id.as_deref() {
        if session.is_resumable_conversation(candidate) {
            candidates.insert(candidate.trim().to_string());
        }
    }
    for history in &session.session_history {
        if session.is_resumable_conversation(&history.agent_session_id) {
            candidates.insert(history.agent_session_id.trim().to_string());
        }
    }

    if candidates.len() > 1 {
        return ProviderRootAssessment::Attention(
            LegacyRecoveryAttentionReason::MultipleResumeSessionIds {
                candidates: candidates.into_iter().collect(),
            },
        );
    }
    if let Some(exact) = session.exact_resume_session_id() {
        return ProviderRootAssessment::Exact {
            root_id: exact.to_string(),
            observed_at: session.updated_at,
        };
    }
    if native_discovery.observations.len() > 1 {
        return ProviderRootAssessment::Attention(
            LegacyRecoveryAttentionReason::MultipleResumeSessionIds {
                candidates: native_discovery
                    .observations
                    .iter()
                    .map(|observation| observation.root_id.clone())
                    .collect(),
            },
        );
    }
    if let Some(observation) = native_discovery.observations.first() {
        if observation.strong_launch_match && native_discovery.scan_complete {
            return ProviderRootAssessment::Exact {
                root_id: observation.root_id.clone(),
                observed_at: observation.observed_at,
            };
        }
        return ProviderRootAssessment::Attention(
            LegacyRecoveryAttentionReason::ProviderNativeCandidateNeedsConfirmation,
        );
    }
    let has_placeholder = session
        .agent_session_id
        .as_deref()
        .into_iter()
        .chain(
            session
                .session_history
                .iter()
                .map(|entry| entry.agent_session_id.as_str()),
        )
        .any(|candidate| {
            !candidate.trim().is_empty() && !session.is_resumable_conversation(candidate)
        });
    if has_placeholder {
        ProviderRootAssessment::Attention(LegacyRecoveryAttentionReason::PlaceholderResumeSessionId)
    } else {
        ProviderRootAssessment::Attention(
            LegacyRecoveryAttentionReason::MissingExactResumeSessionId,
        )
    }
}

fn legacy_import_lifecycle(
    attention_reasons: &[LegacyRecoveryAttentionReason],
) -> (RecoveryLifecycle, String) {
    if attention_reasons.is_empty() {
        (
            RecoveryLifecycle::Interrupted,
            "legacy_import_exact".to_string(),
        )
    } else {
        (
            RecoveryLifecycle::Attention,
            format!(
                "legacy_import_attention:{}",
                attention_reasons
                    .iter()
                    .map(attention_reason_code)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )
    }
}

fn current_recovery_lifecycle_is_unproven(session: &Session) -> bool {
    session
        .recovery_id
        .as_deref()
        .is_some_and(|recovery_id| !recovery_id.starts_with("legacy-"))
        && !matches!(
            session.recovery_launch_stage,
            Some(
                crate::session::RecoveryLaunchStage::ProcessSpawned
                    | crate::session::RecoveryLaunchStage::ProviderBound
                    | crate::session::RecoveryLaunchStage::Ready
            )
        )
        && session.last_hook_event_at.is_none()
        && session.last_completed_stop_at.is_none()
}

const MAX_NATIVE_METADATA_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct CodexHistoryEnvelope {
    #[serde(rename = "type")]
    event_type: Option<String>,
    payload: Option<CodexSessionMetadata>,
}

#[derive(Debug, Deserialize)]
struct CodexSessionMetadata {
    id: Option<String>,
    cwd: Option<PathBuf>,
    timestamp: Option<DateTime<Utc>>,
    source: Option<serde_json::Value>,
}

fn discover_native_provider_roots(
    session: &Session,
    roots: &LegacyProviderHistoryRoots,
    budget: &mut NativeMetadataScanBudget,
) -> NativeProviderRootDiscovery {
    if session.exact_resume_session_id().is_some()
        || ledger_provider_root_candidates(session).len() > 1
    {
        return NativeProviderRootDiscovery::default();
    }
    discover_native_provider_roots_with_shared_budget(session, roots, budget)
}

#[cfg(test)]
fn discover_native_provider_roots_with_budget(
    session: &Session,
    roots: &LegacyProviderHistoryRoots,
    mut budget: NativeMetadataScanBudget,
) -> NativeProviderRootDiscovery {
    discover_native_provider_roots_with_shared_budget(session, roots, &mut budget)
}

fn discover_native_provider_roots_with_shared_budget(
    session: &Session,
    roots: &LegacyProviderHistoryRoots,
    budget: &mut NativeMetadataScanBudget,
) -> NativeProviderRootDiscovery {
    if session.runtime_target != LaunchRuntimeTarget::Host {
        return NativeProviderRootDiscovery::default();
    }
    let mut observations = BTreeMap::<String, NativeProviderRootObservation>::new();
    match session.agent_id {
        crate::AgentId::Codex => {
            if let Some(root) = roots.codex_sessions.as_deref() {
                discover_codex_history_roots(root, session, &mut observations, budget);
            }
        }
        crate::AgentId::ClaudeCode => {
            if let Some(root) = roots.claude_projects.as_deref() {
                discover_claude_history_roots(root, session, &mut observations, budget);
            }
            if let Some(root) = roots.claude_sessions.as_deref() {
                discover_claude_active_roots(root, session, &mut observations, budget);
            }
        }
        _ => {}
    }
    NativeProviderRootDiscovery {
        observations: observations.into_values().collect(),
        scan_complete: !budget.incomplete,
    }
}

fn retain_bounded_path(paths: &mut BTreeSet<PathBuf>, path: PathBuf, limit: usize) {
    if limit == 0 {
        return;
    }
    paths.insert(path);
    if paths.len() > limit {
        // Keep a deterministic lexical prefix. This is only scan ordering;
        // an over-budget scan is non-authoritative regardless of which
        // candidates were encountered and can therefore never select a root.
        let _ = paths.pop_last();
    }
}

fn discover_codex_history_roots(
    sessions_root: &Path,
    session: &Session,
    observations: &mut BTreeMap<String, NativeProviderRootObservation>,
    budget: &mut NativeMetadataScanBudget,
) {
    if !budget.ensure_capacity() {
        return;
    }
    let path_limit = budget.files_remaining.saturating_add(1);
    let mut session_files = BTreeSet::new();
    'days: for day_offset in [-1_i64, 0, 1] {
        let day = session.created_at + Duration::days(day_offset);
        let day_dir = sessions_root
            .join(day.format("%Y").to_string())
            .join(day.format("%m").to_string())
            .join(day.format("%d").to_string());
        let entries = match fs::read_dir(&day_dir) {
            Ok(entries) => entries,
            Err(_) if !day_dir.exists() => continue,
            Err(_) => {
                budget.mark_incomplete();
                continue;
            }
        };
        for entry in entries {
            if !budget.begin_directory_entry() {
                break 'days;
            }
            let Ok(entry) = entry else {
                budget.mark_incomplete();
                continue;
            };
            let path = entry.path();
            if entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
            {
                retain_bounded_path(&mut session_files, path, path_limit);
            }
        }
    }

    for path in session_files {
        if !budget.begin_file() {
            break;
        }
        let Some(line) = read_first_bounded_metadata_line(&path, budget) else {
            continue;
        };
        let Ok(envelope) = serde_json::from_slice::<CodexHistoryEnvelope>(&line) else {
            continue;
        };
        if envelope.event_type.as_deref() != Some("session_meta") {
            continue;
        }
        let Some(metadata) = envelope.payload else {
            continue;
        };
        if metadata.source.as_ref().and_then(serde_json::Value::as_str) != Some("cli") {
            // Object-valued sources are Codex subagents. Unknown source kinds
            // are also excluded instead of being guessed to be a root.
            continue;
        }
        let Some(root_id) = metadata
            .id
            .as_deref()
            .map(str::trim)
            .filter(|root_id| session.is_resumable_conversation(root_id))
        else {
            continue;
        };
        let filename_matches_root = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.ends_with(root_id));
        if !filename_matches_root {
            continue;
        }
        let (Some(cwd), Some(observed_at)) = (metadata.cwd.as_deref(), metadata.timestamp) else {
            continue;
        };
        if !same_legacy_worktree_path(cwd, &session.worktree_path)
            || !provider_timestamp_is_plausible(session, observed_at)
        {
            continue;
        }
        let strong_launch_match = provider_timestamp_is_strong(session, observed_at);
        let mut evidence = BTreeSet::from([
            "Codex native metadata identifies a CLI root, not a subagent".to_string(),
            "Codex native metadata cwd matches the Session worktree".to_string(),
        ]);
        evidence.insert(provider_timestamp_evidence("Codex", strong_launch_match));
        merge_native_observation(
            observations,
            root_id,
            evidence,
            observed_at,
            strong_launch_match,
        );
    }
}

fn discover_claude_history_roots(
    projects_root: &Path,
    session: &Session,
    observations: &mut BTreeMap<String, NativeProviderRootObservation>,
    budget: &mut NativeMetadataScanBudget,
) {
    if budget.files_remaining == 0 {
        budget.mark_incomplete();
        return;
    }
    let project_entries = match fs::read_dir(projects_root) {
        Ok(entries) => entries,
        Err(_) if !projects_root.exists() => return,
        Err(_) => {
            budget.mark_incomplete();
            return;
        }
    };
    let path_limit = budget.files_remaining.saturating_add(1);
    let mut root_files = BTreeSet::new();
    'projects: for project_entry in project_entries {
        if !budget.begin_directory_entry() {
            break;
        }
        let Ok(project_entry) = project_entry else {
            budget.mark_incomplete();
            continue;
        };
        if !project_entry
            .file_type()
            .is_ok_and(|file_type| file_type.is_dir())
        {
            continue;
        }
        if !claude_project_path_matches_worktree(&project_entry.path(), &session.worktree_path) {
            continue;
        }
        let entries = match fs::read_dir(project_entry.path()) {
            Ok(entries) => entries,
            Err(_) => {
                budget.mark_incomplete();
                continue;
            }
        };
        for entry in entries {
            if !budget.begin_directory_entry() {
                break 'projects;
            }
            let Ok(entry) = entry else {
                budget.mark_incomplete();
                continue;
            };
            let path = entry.path();
            if entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
            {
                retain_bounded_path(&mut root_files, path, path_limit);
            }
        }
    }

    for path in root_files {
        if !budget.begin_file() {
            break;
        }
        let Some(root_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|root_id| session.is_resumable_conversation(root_id))
        else {
            continue;
        };
        // Claude project JSONL files are user/assistant message transcripts,
        // not metadata envelopes. Never open them during recovery discovery.
        // The direct-child filename is the root identity, the parent directory
        // is Claude's encoded cwd, and filesystem mtime supplies the bounded
        // launch-window evidence.
        let observed_at = match fs::symlink_metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map(DateTime::<Utc>::from)
        {
            Ok(observed_at) => observed_at,
            Err(_) => {
                budget.mark_incomplete();
                continue;
            }
        };
        if !provider_timestamp_is_plausible(session, observed_at) {
            continue;
        }
        let strong_launch_match = provider_timestamp_is_strong(session, observed_at);
        let mut evidence = BTreeSet::from([
            "Claude root identity comes from the direct transcript filename without reading message content"
                .to_string(),
            "Claude encoded project path matches the Session worktree".to_string(),
        ]);
        evidence.insert(provider_timestamp_evidence(
            "Claude filesystem",
            strong_launch_match,
        ));
        merge_native_observation(
            observations,
            root_id,
            evidence,
            observed_at,
            strong_launch_match,
        );
    }
}

fn claude_project_path_matches_worktree(project_dir: &Path, worktree: &Path) -> bool {
    let Some(actual) = project_dir.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let mut candidates = BTreeSet::from([claude_project_path_key(worktree)]);
    if let Ok(canonical) = dunce::canonicalize(worktree) {
        candidates.insert(claude_project_path_key(&canonical));
    }
    candidates.contains(actual)
}

fn claude_project_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn discover_claude_active_roots(
    sessions_root: &Path,
    session: &Session,
    observations: &mut BTreeMap<String, NativeProviderRootObservation>,
    budget: &mut NativeMetadataScanBudget,
) {
    if budget.files_remaining == 0 {
        budget.mark_incomplete();
        return;
    }
    let entries = match fs::read_dir(sessions_root) {
        Ok(entries) => entries,
        Err(_) if !sessions_root.exists() => return,
        Err(_) => {
            budget.mark_incomplete();
            return;
        }
    };
    let path_limit = budget.files_remaining.saturating_add(1);
    let mut paths = BTreeSet::new();
    for entry in entries {
        if !budget.begin_directory_entry() {
            break;
        }
        let Ok(entry) = entry else {
            budget.mark_incomplete();
            continue;
        };
        let path = entry.path();
        if entry.file_type().is_ok_and(|file_type| file_type.is_file())
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            retain_bounded_path(&mut paths, path, path_limit);
        }
    }
    for path in paths {
        if !budget.begin_file() {
            break;
        }
        let Some(root_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|root_id| session.is_resumable_conversation(root_id))
        else {
            continue;
        };
        // A flat active-session marker does not encode cwd, so it may only
        // corroborate a root already discovered under the matching encoded
        // project directory. Its JSON body is never opened.
        if !observations.contains_key(root_id) {
            continue;
        }
        let observed_at = match fs::symlink_metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map(DateTime::<Utc>::from)
        {
            Ok(observed_at) => observed_at,
            Err(_) => {
                budget.mark_incomplete();
                continue;
            }
        };
        if !provider_timestamp_is_plausible(session, observed_at) {
            continue;
        }
        let strong_launch_match = provider_timestamp_is_strong(session, observed_at);
        let mut evidence = BTreeSet::from([
            "Claude active-session filename corroborates the project-scoped root without reading its JSON body"
                .to_string(),
        ]);
        evidence.insert(provider_timestamp_evidence(
            "Claude active-session filesystem",
            strong_launch_match,
        ));
        merge_native_observation(
            observations,
            root_id,
            evidence,
            observed_at,
            strong_launch_match,
        );
    }
}

fn merge_native_observation(
    observations: &mut BTreeMap<String, NativeProviderRootObservation>,
    root_id: &str,
    evidence: BTreeSet<String>,
    observed_at: DateTime<Utc>,
    strong_launch_match: bool,
) {
    let entry =
        observations
            .entry(root_id.to_string())
            .or_insert_with(|| NativeProviderRootObservation {
                root_id: root_id.to_string(),
                evidence: BTreeSet::new(),
                observed_at,
                strong_launch_match: false,
            });
    entry.evidence.extend(evidence);
    entry.observed_at = entry.observed_at.min(observed_at);
    entry.strong_launch_match |= strong_launch_match;
}

fn provider_timestamp_is_plausible(session: &Session, observed_at: DateTime<Utc>) -> bool {
    let latest_activity = [
        session.updated_at,
        session.last_activity_at,
        session.last_hook_event_at.unwrap_or(session.created_at),
        session.last_completed_stop_at.unwrap_or(session.created_at),
    ]
    .into_iter()
    .max()
    .unwrap_or(session.created_at);
    observed_at >= session.created_at - Duration::minutes(2)
        && observed_at <= latest_activity + Duration::minutes(10)
}

fn provider_timestamp_is_strong(session: &Session, observed_at: DateTime<Utc>) -> bool {
    observed_at >= session.created_at - Duration::seconds(30)
        && observed_at <= session.created_at + Duration::minutes(10)
}

fn provider_timestamp_evidence(provider: &str, strong_launch_match: bool) -> String {
    if strong_launch_match {
        format!("{provider} native metadata timestamp matches the Session launch window")
    } else {
        format!("{provider} native metadata timestamp is within the Session activity window")
    }
}

fn read_first_bounded_metadata_line(
    path: &Path,
    budget: &mut NativeMetadataScanBudget,
) -> Option<Vec<u8>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => {
            budget.mark_incomplete();
            return None;
        }
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    match read_bounded_line(&mut reader, &mut line, budget) {
        Ok(Some(true)) => Some(line),
        Ok(Some(false) | None) => None,
        Err(_) => {
            budget.mark_incomplete();
            None
        }
    }
}

/// Read one JSONL record while keeping transcript-sized lines out of memory.
/// `Some(false)` means the record exceeded the metadata limit and was drained.
fn read_bounded_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
    budget: &mut NativeMetadataScanBudget,
) -> io::Result<Option<bool>> {
    line.clear();
    let mut overflowed = false;
    if reader.fill_buf()?.is_empty() {
        return Ok(None);
    }
    if !budget.begin_line() {
        return Ok(None);
    }
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if overflowed {
                budget.mark_incomplete();
            }
            return Ok(Some(!overflowed));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !budget.consume_bytes(consumed) {
            return Ok(None);
        }
        if !overflowed {
            let remaining = MAX_NATIVE_METADATA_LINE_BYTES.saturating_sub(line.len());
            let copied = remaining.min(consumed);
            line.extend_from_slice(&available[..copied]);
            overflowed = copied < consumed || line.len() == MAX_NATIVE_METADATA_LINE_BYTES;
        }
        reader.consume(consumed);
        if newline.is_some() {
            while line
                .last()
                .is_some_and(|byte| matches!(*byte, b'\n' | b'\r'))
            {
                line.pop();
            }
            if overflowed {
                budget.mark_incomplete();
            }
            return Ok(Some(!overflowed));
        }
    }
}

fn classify_session_kind(
    session: &Session,
) -> (RecoverySessionKind, Option<LegacyRecoveryAttentionReason>) {
    match session.session_kind {
        Some(gwt_skills::SessionKind::Intake) => (RecoverySessionKind::Intake, None),
        Some(gwt_skills::SessionKind::Execution) => (RecoverySessionKind::Execution, None),
        None if session.is_ephemeral => (RecoverySessionKind::Intake, None),
        None if is_legacy_intake_path(&session.worktree_path)
            && is_detached_head(&session.worktree_path) == Some(true) =>
        {
            (RecoverySessionKind::Intake, None)
        }
        None => (
            RecoverySessionKind::Execution,
            Some(LegacyRecoveryAttentionReason::SessionKindAmbiguous),
        ),
    }
}

fn is_legacy_intake_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(".intake-"))
}

fn resolve_git_head_oid(worktree_path: &Path) -> Option<String> {
    let mut command = gwt_core::process::hidden_command("git");
    command
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .current_dir(worktree_path);
    gwt_core::process::scrub_git_env(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let oid = String::from_utf8(output.stdout).ok()?;
    let oid = oid.trim();
    (!oid.is_empty()).then(|| oid.to_string())
}

fn resolve_git_commit_oid(repo_path: &Path, recorded_oid: &str) -> Option<String> {
    let recorded_oid = recorded_oid.trim();
    if !matches!(recorded_oid.len(), 40 | 64)
        || !recorded_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let revision = format!("{recorded_oid}^{{commit}}");
    let mut command = gwt_core::process::hidden_command("git");
    command
        .args(["rev-parse", "--verify", &revision])
        .current_dir(repo_path);
    gwt_core::process::scrub_git_env(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let resolved = String::from_utf8(output.stdout).ok()?;
    let resolved = resolved.trim().to_ascii_lowercase();
    (resolved == recorded_oid.to_ascii_lowercase()).then_some(resolved)
}

fn is_detached_head(worktree_path: &Path) -> Option<bool> {
    let mut command = gwt_core::process::hidden_command("git");
    command
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .current_dir(worktree_path);
    gwt_core::process::scrub_git_env(&mut command);
    let output = command.output().ok()?;
    if output.status.success() {
        Some(false)
    } else if output.status.code() == Some(1) {
        Some(true)
    } else {
        None
    }
}

fn operation_id(recovery_id: &str, mutation: &str) -> String {
    format!("legacy-import-v1:{recovery_id}:{mutation}")
}

fn native_operation_id(recovery_id: &str, mutation: &str, subject: &str) -> String {
    let digest = gwt_core::repo_hash::compute_repo_hash(subject);
    format!(
        "legacy-native-import-v1:{recovery_id}:{mutation}:{}",
        digest.as_str()
    )
}

fn provider_root_candidates_operation_id(
    recovery_id: &str,
    candidates: &[ProviderRootCandidate],
) -> String {
    let signature = candidates
        .iter()
        .map(|candidate| {
            format!(
                "{}|{}|{}",
                candidate.root_id,
                candidate.evidence.join("+"),
                candidate.observed_at.to_rfc3339()
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let digest = gwt_core::repo_hash::compute_repo_hash(&signature);
    format!(
        "legacy-import-v1:{recovery_id}:record-root-candidates:{}",
        digest.as_str()
    )
}

fn attention_reason_code(reason: &LegacyRecoveryAttentionReason) -> &'static str {
    match reason {
        LegacyRecoveryAttentionReason::MissingIntakeWorktree => "missing_intake_worktree",
        LegacyRecoveryAttentionReason::MissingExactResumeSessionId => "missing_provider_root",
        LegacyRecoveryAttentionReason::PlaceholderResumeSessionId => "placeholder_provider_root",
        LegacyRecoveryAttentionReason::MultipleResumeSessionIds { .. } => "multiple_provider_roots",
        LegacyRecoveryAttentionReason::PlaceholderOnlyEvidence => "placeholder_only_evidence",
        LegacyRecoveryAttentionReason::ProviderNativeCandidateNeedsConfirmation => {
            "provider_native_candidate_needs_confirmation"
        }
        LegacyRecoveryAttentionReason::MissingLifecycleEvidence => "missing_lifecycle_evidence",
        LegacyRecoveryAttentionReason::SessionKindAmbiguous => "ambiguous_session_kind",
    }
}

fn skipped(
    session: &Session,
    recovery_id: Option<String>,
    reason: LegacyRecoverySkipReason,
) -> LegacyRecoverySkipped {
    LegacyRecoverySkipped {
        session_id: session.id.clone(),
        recovery_id,
        reason,
    }
}

fn import_error(
    session_path: &Path,
    session_id: Option<&str>,
    stage: LegacyRecoveryImportStage,
    error: impl std::fmt::Display,
) -> LegacyRecoveryImportError {
    LegacyRecoveryImportError {
        session_path: session_path.to_path_buf(),
        session_id: session_id.map(str::to_string),
        stage,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::Utc;
    use gwt_core::{
        paths::project_scope_hash,
        process::{hidden_command, scrub_git_env},
        recovery::{
            BindingQuality, CreateRecovery, RecoveryLifecycle, RecoverySessionKind, RecoveryStore,
        },
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{AgentId, AgentSessionHistoryEntry, Session};

    const ORIGIN: &str = "https://example.com/acme/legacy-recovery.git";

    #[test]
    fn legacy_import_fails_closed_before_reading_oversized_session_toml() {
        let temp = TempDir::new().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir(&sessions_dir).unwrap();
        let oversized = sessions_dir.join("oversized.toml");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(crate::MAX_SESSION_TOML_BYTES + 1)
            .unwrap();
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, "repo");

        assert!(report.imported_exact.is_empty());
        assert!(report.imported_attention.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].stage,
            LegacyRecoveryImportStage::ReadSessionsDirectory
        );
        assert!(report.errors[0].message.contains("size limit"));
    }

    #[test]
    fn imports_one_strong_codex_native_root_and_excludes_subagents() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let session = legacy_session_at("native-codex-exact", &repo, launch_at);
        session
            .save(&sessions_dir)
            .expect("save legacy Session without a provider id");
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-root-exact",
            launch_at + Duration::seconds(8),
            serde_json::json!("cli"),
            "private transcript marker must not be imported",
        );
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-subagent-closer",
            launch_at + Duration::seconds(2),
            serde_json::json!({
                "subagent": {
                    "thread_spawn": {
                        "parent_thread_id": "codex-root-exact",
                        "depth": 1
                    }
                }
            }),
            "subagent private transcript marker",
        );

        let report = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &roots,
        );

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_attention, Vec::new());
        assert_eq!(report.imported_exact.len(), 1);
        assert_eq!(
            report.imported_exact[0].provider_root_id,
            "codex-root-exact"
        );
        let record = store
            .load("legacy-native-codex-exact")
            .expect("load imported recovery")
            .expect("imported recovery");
        assert_eq!(
            record
                .provider_root
                .as_ref()
                .map(|root| root.root_id.as_str()),
            Some("codex-root-exact")
        );
        assert_eq!(
            record.provider_root.as_ref().map(|root| root.quality),
            Some(BindingQuality::Verified)
        );
        assert!(record.provider_root_candidates.is_empty());
        assert_eq!(record.initial_prompt, "");
        assert!(!serde_json::to_string(&record)
            .expect("serialize recovery")
            .contains("private transcript marker"));
    }

    #[test]
    fn upgrades_an_existing_attention_record_when_one_strong_native_root_appears() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let launch_at = Utc::now() - Duration::minutes(1);
        let session = legacy_session_at("native-existing", &repo, launch_at);
        session.save(&sessions_dir).expect("save legacy Session");

        let first = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &LegacyProviderHistoryRoots::default(),
        );
        assert_eq!(first.imported_attention.len(), 1);
        assert_eq!(
            store
                .load("legacy-native-existing")
                .expect("load initial recovery")
                .expect("initial recovery")
                .lifecycle,
            RecoveryLifecycle::Attention
        );

        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-root-late-metadata",
            launch_at + Duration::seconds(8),
            serde_json::json!("cli"),
            "private marker",
        );
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions),
            ..LegacyProviderHistoryRoots::default()
        };
        let repeated = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &roots,
        );

        assert_eq!(repeated.errors, Vec::new());
        assert_eq!(repeated.skipped.len(), 1);
        assert_eq!(
            repeated.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingRecord
        );
        let upgraded = store
            .load("legacy-native-existing")
            .expect("load upgraded recovery")
            .expect("upgraded recovery");
        assert_eq!(upgraded.lifecycle, RecoveryLifecycle::Interrupted);
        assert_eq!(
            upgraded
                .provider_root
                .as_ref()
                .map(|root| (root.root_id.as_str(), root.quality)),
            Some(("codex-root-late-metadata", BindingQuality::Verified))
        );
        let synced = Session::load(&sessions_dir.join("native-existing.toml"))
            .expect("load upgraded Session");
        assert_eq!(
            synced.agent_session_id.as_deref(),
            Some("codex-root-late-metadata")
        );
        assert_eq!(
            synced.provider_binding_quality,
            Some(crate::session::ProviderBindingQuality::Verified)
        );
    }

    #[test]
    fn keeps_all_ambiguous_codex_native_roots_with_evidence_without_latest_guessing() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let session = legacy_session_at("native-codex-multiple", &repo, launch_at);
        session.save(&sessions_dir).expect("save legacy Session");
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-root-earlier",
            launch_at + Duration::seconds(12),
            serde_json::json!("cli"),
            "first private marker",
        );
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-root-later",
            launch_at + Duration::seconds(40),
            serde_json::json!("cli"),
            "second private marker",
        );
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-subagent-newest",
            launch_at + Duration::seconds(50),
            serde_json::json!({"subagent": {"thread_spawn": {"depth": 1}}}),
            "subagent private marker",
        );

        let report = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &roots,
        );

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(
            attention_reasons(&report, "native-codex-multiple"),
            &[LegacyRecoveryAttentionReason::MultipleResumeSessionIds {
                candidates: vec![
                    "codex-root-earlier".to_string(),
                    "codex-root-later".to_string(),
                ],
            }]
        );
        let record = store
            .load("legacy-native-codex-multiple")
            .expect("load ambiguous recovery")
            .expect("ambiguous recovery");
        assert_eq!(record.lifecycle, RecoveryLifecycle::Attention);
        assert_eq!(record.provider_root, None);
        assert_eq!(
            record
                .provider_root_candidates
                .iter()
                .map(|candidate| candidate.root_id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex-root-earlier", "codex-root-later"]
        );
        assert!(record.provider_root_candidates.iter().all(|candidate| {
            candidate
                .evidence
                .iter()
                .any(|evidence| evidence.contains("not a subagent"))
                && candidate
                    .evidence
                    .iter()
                    .any(|evidence| evidence.contains("cwd matches"))
                && candidate
                    .evidence
                    .iter()
                    .any(|evidence| evidence.contains("launch window"))
        }));
        let serialized = serde_json::to_string(&record).expect("serialize recovery");
        assert!(!serialized.contains("private marker"));
        assert!(!serialized.contains("codex-subagent-newest"));
    }

    #[test]
    fn leaves_one_weak_native_timestamp_for_explicit_confirmation() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::hours(2);
        let mut session = legacy_session_at("native-codex-weak", &repo, launch_at);
        session.last_activity_at = launch_at + Duration::hours(1);
        session.updated_at = session.last_activity_at;
        session.save(&sessions_dir).expect("save legacy Session");
        write_codex_metadata(
            &codex_sessions,
            &session,
            "codex-root-needs-confirmation",
            launch_at + Duration::minutes(30),
            serde_json::json!("cli"),
            "private marker",
        );

        let report = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &roots,
        );

        assert_eq!(
            attention_reasons(&report, "native-codex-weak"),
            &[LegacyRecoveryAttentionReason::ProviderNativeCandidateNeedsConfirmation]
        );
        let record = store
            .load("legacy-native-codex-weak")
            .expect("load weak recovery")
            .expect("weak recovery");
        assert_eq!(record.provider_root, None);
        assert_eq!(record.provider_root_candidates.len(), 1);
        assert!(record.provider_root_candidates[0]
            .evidence
            .iter()
            .any(|evidence| evidence.contains("activity window")));
    }

    #[test]
    fn global_file_budget_never_promotes_a_partially_scanned_strong_root() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let session = legacy_session_at("native-file-budget", &repo, launch_at);
        write_codex_metadata(
            &codex_sessions,
            &session,
            "000-strong-root",
            launch_at + Duration::seconds(5),
            serde_json::json!("cli"),
            "private marker",
        );
        write_codex_metadata(
            &codex_sessions,
            &session,
            "100-known-subagent",
            launch_at + Duration::seconds(6),
            serde_json::json!({"subagent": {"thread_spawn": {"depth": 1}}}),
            "private marker",
        );
        write_codex_metadata(
            &codex_sessions,
            &session,
            "200-unscanned-root",
            launch_at + Duration::seconds(7),
            serde_json::json!("cli"),
            "private marker",
        );

        let discovery = discover_native_provider_roots_with_budget(
            &session,
            &roots,
            NativeMetadataScanBudget::new(2, 10, 1024 * 1024),
        );

        assert!(!discovery.scan_complete);
        assert_eq!(discovery.observations.len(), 1);
        assert!(matches!(
            assess_provider_root(&session, &discovery),
            ProviderRootAssessment::Attention(
                LegacyRecoveryAttentionReason::ProviderNativeCandidateNeedsConfirmation
            )
        ));
        let candidates = discovery.provider_candidates();
        assert_eq!(candidates[0].root_id, "000-strong-root");
        assert!(candidates[0]
            .evidence
            .iter()
            .any(|evidence| evidence.contains("global safety budget")));
    }

    #[test]
    fn claude_transcript_discovery_never_consumes_message_line_or_byte_budget() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let claude_projects = temp.path().join("native-claude").join("projects");
        let roots = LegacyProviderHistoryRoots {
            claude_projects: Some(claude_projects.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let mut session = legacy_session_at("native-line-budget", &repo, launch_at);
        session.agent_id = AgentId::ClaudeCode;
        write_claude_history(
            &claude_projects,
            &session,
            "claude-root-after-budget",
            launch_at + Duration::seconds(5),
            false,
            "private transcript marker",
        );

        let discovery = discover_native_provider_roots_with_budget(
            &session,
            &roots,
            NativeMetadataScanBudget::new(4, 0, 0),
        );

        assert!(discovery.scan_complete);
        assert_eq!(discovery.observations.len(), 1);
        assert_eq!(
            discovery.observations[0].root_id,
            "claude-root-after-budget"
        );
    }

    #[test]
    fn native_scan_budget_is_shared_across_legacy_sessions() {
        let temp = TempDir::new().expect("tempdir");
        let first_repo = temp.path().join("first-repo");
        let second_repo = temp.path().join("second-repo");
        init_repo(&first_repo, ORIGIN, true);
        init_repo(&second_repo, ORIGIN, true);
        let codex_sessions = temp.path().join("native-codex").join("sessions");
        let roots = LegacyProviderHistoryRoots {
            codex_sessions: Some(codex_sessions.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let first = legacy_session_at("native-global-first", &first_repo, launch_at);
        let second = legacy_session_at("native-global-second", &second_repo, launch_at);
        write_codex_metadata(
            &codex_sessions,
            &first,
            "only-budgeted-root",
            launch_at + Duration::seconds(5),
            serde_json::json!("cli"),
            "private marker",
        );
        let mut budget = NativeMetadataScanBudget::new(1, 4, 1024 * 1024);

        let first_discovery = discover_native_provider_roots(&first, &roots, &mut budget);
        let second_discovery = discover_native_provider_roots(&second, &roots, &mut budget);

        assert!(first_discovery.scan_complete);
        assert_eq!(first_discovery.observations.len(), 1);
        assert!(!second_discovery.scan_complete);
        assert!(second_discovery.observations.is_empty());
    }

    #[test]
    fn imports_claude_root_from_path_metadata_without_reading_message_jsonl() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let claude_projects = temp.path().join("native-claude").join("projects");
        let roots = LegacyProviderHistoryRoots {
            claude_projects: Some(claude_projects.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let mut session = legacy_session_at("native-claude-exact", &repo, launch_at);
        session.agent_id = AgentId::ClaudeCode;
        session.save(&sessions_dir).expect("save Claude Session");
        write_claude_history(
            &claude_projects,
            &session,
            "claude-root-exact",
            launch_at + Duration::seconds(5),
            false,
            "root private transcript marker",
        );
        write_claude_history(
            &claude_projects,
            &session,
            "claude-sidechain-closer",
            launch_at + Duration::seconds(1),
            true,
            "sidechain private transcript marker",
        );

        let report = import_legacy_recovery_sessions_with_history_roots(
            &sessions_dir,
            &store,
            &expected_repo_id,
            &roots,
        );

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_exact.len(), 1);
        assert_eq!(
            report.imported_exact[0].provider_root_id,
            "claude-root-exact"
        );
        let record = store
            .load("legacy-native-claude-exact")
            .expect("load Claude recovery")
            .expect("Claude recovery");
        assert_eq!(
            record
                .provider_root
                .as_ref()
                .map(|root| root.root_id.as_str()),
            Some("claude-root-exact")
        );
        let serialized = serde_json::to_string(&record).expect("serialize recovery");
        assert!(!serialized.contains("private transcript marker"));
        assert!(!serialized.contains("claude-sidechain-closer"));
    }

    #[cfg(unix)]
    #[test]
    fn discovers_claude_root_when_transcript_bytes_are_unreadable() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let claude_projects = temp.path().join("native-claude").join("projects");
        let roots = LegacyProviderHistoryRoots {
            claude_projects: Some(claude_projects.clone()),
            ..LegacyProviderHistoryRoots::default()
        };
        let launch_at = Utc::now() - Duration::minutes(1);
        let mut session = legacy_session_at("native-claude-unreadable", &repo, launch_at);
        session.agent_id = AgentId::ClaudeCode;
        write_claude_history(
            &claude_projects,
            &session,
            "claude-root-unreadable",
            launch_at + Duration::seconds(5),
            false,
            "bytes cannot be opened",
        );
        let transcript = claude_projects
            .join(claude_project_path_key(&session.worktree_path))
            .join("claude-root-unreadable.jsonl");
        fs::set_permissions(&transcript, fs::Permissions::from_mode(0o000))
            .expect("make transcript unreadable");
        if File::open(&transcript).is_ok() {
            // Privileged/root test runners can bypass mode bits. The invalid
            // JSON sentinel test above still proves content independence.
            return;
        }

        let discovery = discover_native_provider_roots_with_budget(
            &session,
            &roots,
            NativeMetadataScanBudget::new(4, 0, 0),
        );

        assert!(discovery.scan_complete);
        assert_eq!(discovery.observations.len(), 1);
        assert_eq!(discovery.observations[0].root_id, "claude-root-unreadable");
    }

    #[test]
    fn imports_exact_root_with_measured_head_and_syncs_recovery_metadata() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        let mut session = legacy_session("exact", &repo);
        session.agent_session_id = Some("provider-root-exact".to_string());
        session.session_history.push(AgentSessionHistoryEntry {
            agent_session_id: "provider-root-exact".to_string(),
            started_at: Utc::now(),
        });
        session.launch_args = vec!["--prompt".into(), "must-not-be-imported".into()];
        session.save(&sessions_dir).expect("save session");
        let session_path = sessions_dir.join("exact.toml");
        let measured_head = git_stdout(&repo, &["rev-parse", "--verify", "HEAD^{commit}"]);

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_attention, Vec::new());
        assert_eq!(report.imported_exact.len(), 1);
        assert_eq!(
            report.imported_exact[0].provider_root_id,
            "provider-root-exact"
        );
        let record = store
            .load("legacy-exact")
            .expect("load imported recovery")
            .expect("imported recovery");
        assert_eq!(record.launch_head_oid, measured_head);
        assert_eq!(record.launch_base_oid, record.launch_head_oid);
        assert_eq!(record.initial_prompt, "");
        assert_eq!(record.lifecycle, RecoveryLifecycle::Interrupted);
        assert_eq!(
            record.provider_root.as_ref().map(|root| root.quality),
            Some(BindingQuality::Verified)
        );
        let synced = Session::load(&session_path).expect("synced legacy Session");
        assert_eq!(synced.recovery_id.as_deref(), Some("legacy-exact"));
        assert_eq!(
            synced.recovery_launch_stage,
            Some(crate::session::RecoveryLaunchStage::ProviderBound)
        );
        assert_eq!(
            synced.provider_root_role,
            Some(crate::session::ProviderRootRole::Root)
        );
        assert_eq!(
            synced.provider_binding_quality,
            Some(crate::session::ProviderBindingQuality::Verified)
        );
        assert_eq!(
            synced.session_kind,
            Some(gwt_skills::SessionKind::Execution)
        );
        assert_eq!(synced.agent_session_id, session.agent_session_id);
        assert_eq!(synced.session_history, session.session_history);
        assert_eq!(synced.launch_args, session.launch_args);

        let mut metadata_lost = synced;
        metadata_lost.recovery_id = None;
        metadata_lost.recovery_launch_stage = None;
        metadata_lost.provider_root_role = None;
        metadata_lost.provider_binding_quality = None;
        metadata_lost.agent_session_id = Some("stale-provider-root".to_string());
        metadata_lost
            .save(&sessions_dir)
            .expect("simulate interrupted Session metadata sync");

        let repeated = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);
        assert!(repeated.imported_exact.is_empty());
        assert_eq!(repeated.skipped.len(), 1);
        assert_eq!(
            repeated.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingRecord
        );
        assert_eq!(store.list().expect("list recoveries").len(), 1);
        let resynced = Session::load(&session_path).expect("resynced existing recovery metadata");
        assert_eq!(resynced.recovery_id.as_deref(), Some("legacy-exact"));
        assert_eq!(
            resynced.agent_session_id.as_deref(),
            Some("provider-root-exact")
        );
        assert!(resynced
            .session_history
            .iter()
            .any(|entry| entry.agent_session_id == "stale-provider-root"));
        assert_eq!(
            resynced.provider_binding_quality,
            Some(crate::session::ProviderBindingQuality::Verified)
        );
    }

    #[test]
    fn expired_tombstone_does_not_reimport_a_terminal_session() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let mut session = legacy_session("terminal", &repo);
        session.recovery_id = Some("legacy-terminal".to_string());
        session.agent_session_id = Some("provider-terminal".to_string());
        session.save(&sessions_dir).expect("save source Session");
        let head = git_stdout(&repo, &["rev-parse", "--verify", "HEAD^{commit}"]);
        let purged_at = Utc::now() - chrono::Duration::days(31);
        store
            .create(
                CreateRecovery {
                    recovery_id: "legacy-terminal".to_string(),
                    session_id: session.id.clone(),
                    repo_id: expected_repo_id.clone(),
                    session_kind: RecoverySessionKind::Execution,
                    worktree_path: repo.clone(),
                    launch_base_ref: None,
                    launch_base_oid: head.clone(),
                    launch_head_oid: head,
                    provider: session.agent_id.to_string(),
                    model: session.model.clone(),
                    runtime: "host".to_string(),
                    initial_prompt: String::new(),
                    created_at: purged_at,
                },
                "create-terminal",
            )
            .expect("create terminal recovery");
        store
            .finalize_and_purge(
                "legacy-terminal",
                RecoveryLifecycle::Resolved,
                purged_at,
                "resolve-terminal",
            )
            .expect("resolve terminal recovery");

        let first = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);
        assert_eq!(first.errors, Vec::new());
        assert_eq!(first.skipped.len(), 1);
        assert_eq!(
            first.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingTombstone
        );
        let terminal =
            Session::load(&sessions_dir.join("terminal.toml")).expect("terminal Session");
        assert_eq!(
            terminal.recovery_launch_stage,
            Some(crate::session::RecoveryLaunchStage::Resolved)
        );
        assert_eq!(store.remove_expired_tombstones(Utc::now()).unwrap(), 1);

        let second = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);
        assert_eq!(second.errors, Vec::new());
        assert_eq!(second.imported_exact, Vec::new());
        assert_eq!(second.imported_attention, Vec::new());
        assert_eq!(second.skipped.len(), 1);
        assert_eq!(
            second.skipped[0].reason,
            LegacyRecoverySkipReason::TerminalSession
        );

        let window = LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WindowState,
            source_id: "tab-terminal::agent-1".to_string(),
            source_path: temp.path().join("workspace.json"),
            session_id: Some("terminal".to_string()),
            provider: Some("codex".to_string()),
            worktree_path: Some(repo),
            session_kind: Some(RecoverySessionKind::Execution),
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };
        let persisted_session_ids = BTreeSet::from(["terminal".to_string()]);
        let window_report = import_legacy_recovery_placeholders(
            &[window],
            &persisted_session_ids,
            &store,
            &expected_repo_id,
        );
        assert_eq!(window_report.errors, Vec::new());
        assert_eq!(window_report.imported_attention, Vec::new());
        assert_eq!(
            window_report.skipped[0].reason,
            LegacyRecoverySkipReason::PersistedSessionExists
        );
        assert!(store.load("legacy-terminal").unwrap().is_none());
    }

    #[test]
    fn current_created_session_without_lifecycle_evidence_requires_attention() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let mut session = Session::new(&repo, "work/current-created", AgentId::Codex);
        session.id = "current-created".to_string();
        session.agent_session_id = Some("provider-current-created".to_string());
        session.update_status(crate::AgentStatus::Running);
        let recovery_id = session
            .recovery_id
            .clone()
            .expect("current Session recovery id");
        session.save(&sessions_dir).expect("save current Session");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(report.imported_attention.len(), 1);
        assert_eq!(
            attention_reasons(&report, "current-created"),
            &[LegacyRecoveryAttentionReason::MissingLifecycleEvidence]
        );
        let record = store
            .load(&recovery_id)
            .expect("load current recovery")
            .expect("current recovery");
        assert_eq!(record.lifecycle, RecoveryLifecycle::Attention);
        assert_eq!(
            record.lifecycle_reason.as_deref(),
            Some("legacy_import_attention:missing_lifecycle_evidence")
        );
    }

    #[test]
    fn imports_missing_placeholder_and_multiple_roots_as_typed_attention() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        let missing = legacy_session("missing", &repo);
        missing.save(&sessions_dir).expect("save missing session");

        let mut placeholder = legacy_session("placeholder", &repo);
        placeholder.agent_session_id = Some("agent-session".to_string());
        placeholder
            .save(&sessions_dir)
            .expect("save placeholder session");

        let mut multiple = legacy_session("multiple", &repo);
        multiple.agent_session_id = Some("provider-b".to_string());
        multiple.session_history = vec![
            AgentSessionHistoryEntry {
                agent_session_id: "provider-a".to_string(),
                started_at: Utc::now(),
            },
            AgentSessionHistoryEntry {
                agent_session_id: "provider-b".to_string(),
                started_at: Utc::now(),
            },
        ];
        multiple.save(&sessions_dir).expect("save multiple session");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(report.imported_attention.len(), 3);
        assert_eq!(
            attention_reasons(&report, "missing"),
            &[LegacyRecoveryAttentionReason::MissingExactResumeSessionId]
        );
        assert_eq!(
            attention_reasons(&report, "placeholder"),
            &[LegacyRecoveryAttentionReason::PlaceholderResumeSessionId]
        );
        assert_eq!(
            attention_reasons(&report, "multiple"),
            &[LegacyRecoveryAttentionReason::MultipleResumeSessionIds {
                candidates: vec!["provider-a".to_string(), "provider-b".to_string()],
            }]
        );
        for id in ["missing", "placeholder", "multiple"] {
            let record = store
                .load(&format!("legacy-{id}"))
                .expect("load attention recovery")
                .expect("attention recovery");
            assert_eq!(record.lifecycle, RecoveryLifecycle::Attention);
            assert_eq!(record.provider_root, None);
            let synced = Session::load(&sessions_dir.join(format!("{id}.toml")))
                .expect("load synced attention Session");
            assert_eq!(synced.recovery_id, Some(format!("legacy-{id}")));
            assert_eq!(
                synced.session_kind,
                Some(gwt_skills::SessionKind::Execution)
            );
            assert_eq!(synced.provider_binding_quality, None);
        }
        let multiple_record = store
            .load("legacy-multiple")
            .expect("load multiple-root recovery")
            .expect("multiple-root recovery");
        assert_eq!(
            multiple_record
                .provider_root_candidates
                .iter()
                .map(|candidate| candidate.root_id.as_str())
                .collect::<Vec<_>>(),
            vec!["provider-a", "provider-b"]
        );
        assert_eq!(
            multiple_record.provider_root_candidates[0].evidence,
            vec!["Legacy Session history"]
        );
        assert_eq!(
            multiple_record.provider_root_candidates[1].evidence,
            vec![
                "Legacy Session current provider id".to_string(),
                "Legacy Session history".to_string(),
            ]
        );

        let repeated = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);
        assert!(repeated.errors.is_empty());
        assert_eq!(repeated.skipped.len(), 3);
        assert_eq!(
            store
                .load("legacy-multiple")
                .expect("reload multiple-root recovery")
                .expect("multiple-root recovery")
                .provider_root_candidates
                .len(),
            2
        );
    }

    #[test]
    fn does_not_sync_existing_recovery_with_mismatched_owner_metadata() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        let other_worktree = temp.path().join("other-worktree");
        init_repo(&repo, ORIGIN, true);
        init_repo(&other_worktree, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        let mut session = legacy_session("owner-mismatch", &repo);
        session.agent_session_id = Some("source-provider-root".to_string());
        session.save(&sessions_dir).expect("save source Session");

        let mut request = create_request(
            "legacy-owner-mismatch",
            "owner-mismatch",
            &other_worktree,
            &expected_repo_id,
        );
        request.provider = "Claude Code".to_string();
        request.runtime = "docker".to_string();
        store
            .create(request, "create-foreign-owner")
            .expect("create mismatched recovery");
        store
            .bind_root(
                "legacy-owner-mismatch",
                ProviderRootBinding {
                    root_id: "foreign-provider-root".to_string(),
                    session_tree_id: None,
                    quality: BindingQuality::Verified,
                    bound_at: Utc::now(),
                },
                "bind-foreign-owner",
            )
            .expect("bind mismatched recovery");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert!(report.skipped.is_empty());
        assert_eq!(report.errors.len(), 1);
        assert_eq!(
            report.errors[0].stage,
            LegacyRecoveryImportStage::ValidateRecoveryIdentity
        );
        let untouched = Session::load(&sessions_dir.join("owner-mismatch.toml"))
            .expect("load untouched source Session");
        assert_eq!(untouched.recovery_id, None);
        assert_eq!(
            untouched.agent_session_id.as_deref(),
            Some("source-provider-root")
        );
    }

    #[test]
    fn classifies_detached_dot_intake_and_keeps_unknown_kind_in_attention() {
        let temp = TempDir::new().expect("tempdir");
        let intake_repo = temp.path().join(".intake-old");
        init_repo(&intake_repo, ORIGIN, true);
        git(&intake_repo, &["checkout", "--detach"]);
        let execution_repo = temp.path().join("ordinary-worktree");
        init_repo(&execution_repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&intake_repo).to_string();
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        let mut intake = legacy_session("intake", &intake_repo);
        intake.session_kind = None;
        intake.agent_session_id = Some("intake-root".to_string());
        intake.save(&sessions_dir).expect("save intake session");

        let mut unknown = legacy_session("unknown-kind", &execution_repo);
        unknown.session_kind = None;
        unknown.agent_session_id = Some("unknown-root".to_string());
        unknown.save(&sessions_dir).expect("save unknown session");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.imported_exact.len(), 1);
        assert_eq!(report.imported_exact[0].session_id, "intake");
        assert_eq!(report.imported_attention.len(), 1);
        assert_eq!(
            attention_reasons(&report, "unknown-kind"),
            &[LegacyRecoveryAttentionReason::SessionKindAmbiguous]
        );
        assert_eq!(
            store
                .load("legacy-intake")
                .expect("load intake")
                .expect("intake")
                .session_kind,
            RecoverySessionKind::Intake
        );
        let unknown_record = store
            .load("legacy-unknown-kind")
            .expect("load unknown")
            .expect("unknown");
        assert_eq!(unknown_record.session_kind, RecoverySessionKind::Execution);
        assert_eq!(
            unknown_record
                .provider_root
                .as_ref()
                .map(|root| (root.root_id.as_str(), root.quality)),
            Some(("unknown-root", BindingQuality::Verified))
        );
    }

    #[test]
    fn skips_ineligible_candidates_and_reports_corrupt_toml_without_deleting_sources() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let no_head_repo = temp.path().join("no-head");
        init_repo(&no_head_repo, ORIGIN, false);
        let other_repo = temp.path().join("other");
        init_repo(&other_repo, "https://example.com/acme/unrelated.git", true);
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));

        legacy_session("missing-worktree", &temp.path().join("gone"))
            .save(&sessions_dir)
            .expect("save missing worktree session");
        legacy_session("no-head", &no_head_repo)
            .save(&sessions_dir)
            .expect("save no-head session");
        legacy_session("wrong-repo", &other_repo)
            .save(&sessions_dir)
            .expect("save wrong-repo session");

        let existing = legacy_session("existing", &repo);
        existing.save(&sessions_dir).expect("save existing session");
        store
            .create(
                create_request("legacy-existing", "existing", &repo, &expected_repo_id),
                "preexisting:create",
            )
            .expect("create existing recovery");

        let tombstoned = legacy_session("tombstoned", &repo);
        tombstoned
            .save(&sessions_dir)
            .expect("save tombstoned session");
        store
            .create(
                create_request("legacy-tombstoned", "tombstoned", &repo, &expected_repo_id),
                "tombstone:create",
            )
            .expect("create tombstone source");
        store
            .set_lifecycle(
                "legacy-tombstoned",
                RecoveryLifecycle::Resolved,
                None,
                "tombstone:resolve",
            )
            .expect("resolve tombstone source");
        store
            .finalize_and_purge(
                "legacy-tombstoned",
                RecoveryLifecycle::Resolved,
                Utc::now(),
                "tombstone:purge",
            )
            .expect("purge tombstone source");

        fs::create_dir_all(&sessions_dir).expect("sessions directory");
        let corrupt_path = sessions_dir.join("corrupt.toml");
        fs::write(&corrupt_path, b"not = [valid").expect("write corrupt TOML");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(report.imported_attention, Vec::new());
        assert_eq!(
            skip_reason(&report, "missing-worktree"),
            &LegacyRecoverySkipReason::MissingWorktree
        );
        assert_eq!(
            skip_reason(&report, "no-head"),
            &LegacyRecoverySkipReason::HeadOidUnavailable
        );
        assert!(matches!(
            skip_reason(&report, "wrong-repo"),
            LegacyRecoverySkipReason::RepoScopeMismatch { .. }
        ));
        assert_eq!(
            skip_reason(&report, "existing"),
            &LegacyRecoverySkipReason::ExistingRecord
        );
        assert_eq!(
            skip_reason(&report, "tombstoned"),
            &LegacyRecoverySkipReason::ExistingTombstone
        );
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].session_path, corrupt_path);
        assert_eq!(
            report.errors[0].stage,
            LegacyRecoveryImportStage::LoadSession
        );
        for id in [
            "missing-worktree",
            "no-head",
            "wrong-repo",
            "existing",
            "tombstoned",
        ] {
            assert!(sessions_dir.join(format!("{id}.toml")).is_file());
        }
    }

    #[test]
    fn imports_missing_explicit_intake_from_recorded_base_without_recreating_user_paths() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let launch_base_oid = git_stdout(&repo, &["rev-parse", "HEAD"]);
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let missing_intake = temp.path().join(".intake-5");

        let mut intake = legacy_session("missing-intake", &missing_intake);
        intake.session_kind = Some(gwt_skills::SessionKind::Intake);
        intake.is_ephemeral = true;
        intake.project_state_root = Some(repo.clone());
        intake.repo_hash = Some(expected_repo_id.clone());
        intake.launch_base_oid = Some(launch_base_oid.clone());
        intake.agent_session_id = Some("provider-root".to_string());
        intake
            .save(&sessions_dir)
            .expect("save missing Intake Session");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(report.imported_attention.len(), 1);
        assert_eq!(
            attention_reasons(&report, "missing-intake"),
            &[LegacyRecoveryAttentionReason::MissingIntakeWorktree]
        );
        let record = store
            .load("legacy-missing-intake")
            .expect("load recovery")
            .expect("missing Intake recovery");
        assert_eq!(record.session_kind, RecoverySessionKind::Intake);
        assert_eq!(record.worktree_path, missing_intake);
        assert_eq!(record.launch_base_oid, launch_base_oid);
        assert_eq!(record.lifecycle, RecoveryLifecycle::Attention);
        assert!(!record.worktree_path.exists());

        let user_path = temp.path().join("work").join("20260716-0100");
        let mut execution = legacy_session("missing-execution", &user_path);
        execution.project_state_root = Some(repo);
        execution.repo_hash = Some(expected_repo_id);
        execution.launch_base_oid = Some(record.launch_base_oid);
        execution
            .save(&sessions_dir)
            .expect("save missing Execution Session");
        let second = import_legacy_recovery_sessions(
            &sessions_dir,
            &store,
            &project_scope_hash(temp.path().join("repo").as_path()).to_string(),
        );
        assert_eq!(
            skip_reason(&second, "missing-execution"),
            &LegacyRecoverySkipReason::MissingWorktree
        );
        assert!(!user_path.exists());
    }

    #[test]
    fn imports_missing_explicit_intake_when_persisted_repo_hash_is_stale() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let launch_base_oid = git_stdout(&repo, &["rev-parse", "HEAD"]);
        let sessions_dir = temp.path().join("sessions");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let missing_intake = temp.path().join(".intake-55");

        let mut intake = legacy_session("missing-intake-stale-scope", &missing_intake);
        intake.session_kind = Some(gwt_skills::SessionKind::Intake);
        intake.is_ephemeral = true;
        intake.project_state_root = Some(repo);
        intake.repo_hash = Some("stale-pre-project-scope-hash".to_string());
        intake.launch_base_oid = Some(launch_base_oid);
        intake.agent_session_id = Some("provider-root".to_string());
        intake.save(&sessions_dir).expect("save legacy Intake");

        let report = import_legacy_recovery_sessions(&sessions_dir, &store, &expected_repo_id);

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_attention.len(), 1);
        assert_eq!(
            attention_reasons(&report, "missing-intake-stale-scope"),
            &[LegacyRecoveryAttentionReason::MissingIntakeWorktree]
        );
        let record = store
            .load("legacy-missing-intake-stale-scope")
            .expect("load recovery")
            .expect("missing Intake recovery");
        assert_eq!(record.repo_id, expected_repo_id);
        let synced = Session::load(&sessions_dir.join("missing-intake-stale-scope.toml"))
            .expect("load migrated Session");
        assert_eq!(synced.repo_hash.as_deref(), Some(record.repo_id.as_str()));
    }

    #[test]
    fn imports_projection_only_placeholder_as_attention_without_copying_projection_content() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let source_path = temp.path().join("project-state/current.json");
        fs::create_dir_all(source_path.parent().expect("projection parent"))
            .expect("projection parent");
        fs::write(&source_path, br#"{"current_focus":"must-not-be-imported"}"#)
            .expect("projection fixture");
        let source_before = fs::read(&source_path).expect("projection before import");
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let candidate = LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WorkspaceProjection,
            source_id: "workspace-agent-1".to_string(),
            source_path: source_path.clone(),
            session_id: Some("projection-only".to_string()),
            provider: Some("codex".to_string()),
            worktree_path: Some(repo.clone()),
            session_kind: None,
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };

        let report = import_legacy_recovery_placeholders(
            std::slice::from_ref(&candidate),
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_exact, Vec::new());
        assert_eq!(report.imported_attention.len(), 1);
        assert_eq!(
            report.imported_attention[0].reasons,
            vec![
                LegacyRecoveryAttentionReason::PlaceholderOnlyEvidence,
                LegacyRecoveryAttentionReason::SessionKindAmbiguous,
            ]
        );
        let recovery_id = &report.imported_attention[0].recovery_id;
        let record = store
            .load(recovery_id)
            .expect("load placeholder recovery")
            .expect("placeholder recovery");
        let measured_head = git_stdout(&repo, &["rev-parse", "--verify", "HEAD^{commit}"]);
        assert_eq!(record.session_id, "projection-only");
        assert_eq!(record.provider, "codex");
        assert_eq!(record.provider_root, None);
        assert_eq!(record.launch_base_oid, measured_head);
        assert_eq!(record.launch_head_oid, measured_head);
        assert_eq!(record.initial_prompt, "");
        assert_eq!(record.lifecycle, RecoveryLifecycle::Attention);
        assert_eq!(
            fs::read(&source_path).expect("projection after import"),
            source_before
        );

        let repeated = import_legacy_recovery_placeholders(
            &[candidate],
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );
        assert!(repeated.imported_attention.is_empty());
        assert_eq!(repeated.skipped.len(), 1);
        assert_eq!(
            repeated.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingRecord
        );
        assert_eq!(store.list().expect("list recoveries").len(), 1);
    }

    #[test]
    fn skips_idless_window_without_a_stable_recovery_identity() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let candidate = LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WindowState,
            source_id: "tab-legacy::agent-7".to_string(),
            source_path: temp.path().join("workspace.json"),
            session_id: None,
            provider: Some("claude".to_string()),
            worktree_path: Some(repo),
            session_kind: Some(RecoverySessionKind::Execution),
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };

        let report = import_legacy_recovery_placeholders(
            std::slice::from_ref(&candidate),
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );

        assert_eq!(report.errors, Vec::new());
        assert_eq!(report.imported_attention, Vec::new());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(
            report.skipped[0].reason,
            LegacyRecoverySkipReason::MissingPlaceholderIdentity
        );
        assert!(store.list().expect("list recoveries").is_empty());
    }

    #[test]
    fn placeholder_import_deduplicates_session_window_and_tombstone_sources() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let source_path = temp.path().join("workspace.json");
        fs::write(&source_path, b"unchanged").expect("window fixture");

        let window = LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WindowState,
            source_id: "tab-1::agent-1".to_string(),
            source_path: source_path.clone(),
            session_id: Some("shared-session".to_string()),
            provider: Some("codex".to_string()),
            worktree_path: Some(repo.clone()),
            session_kind: Some(RecoverySessionKind::Intake),
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };
        let projection = LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WorkspaceProjection,
            source_id: "projection-agent-1".to_string(),
            source_path: temp.path().join("current.json"),
            session_id: Some("shared-session".to_string()),
            provider: Some("codex".to_string()),
            worktree_path: Some(repo.clone()),
            session_kind: None,
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };

        store
            .create(
                create_request(
                    "session-import-owned",
                    "store-owned-session",
                    &repo,
                    &expected_repo_id,
                ),
                "session-import:create",
            )
            .expect("create Session-import-owned recovery");
        let mut store_owned = window.clone();
        store_owned.session_id = Some("store-owned-session".to_string());
        store_owned.source_id = "store-owned-window".to_string();
        let already_imported = import_legacy_recovery_placeholders(
            &[store_owned],
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );
        assert!(already_imported.imported_attention.is_empty());
        assert_eq!(
            already_imported.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingRecord,
        );
        assert_eq!(
            already_imported.skipped[0].recovery_id.as_deref(),
            Some("session-import-owned"),
        );

        let mut persisted_session_ids = BTreeSet::new();
        persisted_session_ids.insert("shared-session".to_string());
        let ledger_owned = import_legacy_recovery_placeholders(
            &[projection.clone(), window.clone()],
            &persisted_session_ids,
            &store,
            &expected_repo_id,
        );
        assert!(ledger_owned.imported_attention.is_empty());
        assert_eq!(ledger_owned.skipped.len(), 2);
        assert!(ledger_owned
            .skipped
            .iter()
            .all(|skipped| skipped.reason == LegacyRecoverySkipReason::PersistedSessionExists));

        let imported = import_legacy_recovery_placeholders(
            &[projection, window.clone()],
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );
        assert_eq!(imported.imported_attention.len(), 1);
        assert_eq!(
            imported.skipped[0].reason,
            LegacyRecoverySkipReason::DuplicatePlaceholder
        );
        let recovery_id = imported.imported_attention[0].recovery_id.clone();
        assert_eq!(
            store
                .load(&recovery_id)
                .expect("load imported window")
                .expect("imported window")
                .session_kind,
            RecoverySessionKind::Intake,
        );
        store
            .finalize_and_purge(
                &recovery_id,
                RecoveryLifecycle::Discarded,
                Utc::now(),
                "test:discard-placeholder",
            )
            .expect("discard placeholder");

        let tombstoned = import_legacy_recovery_placeholders(
            &[window],
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );
        assert!(tombstoned.imported_attention.is_empty());
        assert_eq!(
            tombstoned.skipped[0].reason,
            LegacyRecoverySkipReason::ExistingTombstone
        );
    }

    #[test]
    fn placeholder_import_skips_shell_non_agent_live_and_invalid_candidates() {
        let temp = TempDir::new().expect("tempdir");
        let repo = temp.path().join("repo");
        init_repo(&repo, ORIGIN, true);
        let expected_repo_id = project_scope_hash(&repo).to_string();
        let other_repo = temp.path().join("other");
        init_repo(
            &other_repo,
            "https://example.com/acme/other-placeholder.git",
            true,
        );
        let store = RecoveryStore::for_project_dir(temp.path().join("project"));
        let candidate = |source_id: &str| LegacyRecoveryPlaceholder {
            source: LegacyRecoveryPlaceholderSource::WorkspaceProjection,
            source_id: source_id.to_string(),
            source_path: temp.path().join(format!("{source_id}.json")),
            session_id: Some(source_id.to_string()),
            provider: Some("codex".to_string()),
            worktree_path: Some(repo.clone()),
            session_kind: None,
            kind: LegacyRecoveryPlaceholderKind::Agent,
            runtime_is_live: false,
            observed_at: Utc::now(),
        };
        let mut shell = candidate("shell");
        shell.kind = LegacyRecoveryPlaceholderKind::Shell;
        let mut non_agent = candidate("board");
        non_agent.kind = LegacyRecoveryPlaceholderKind::NonAgent;
        let mut live = candidate("live");
        live.runtime_is_live = true;
        let mut invalid = candidate("done");
        invalid.kind = LegacyRecoveryPlaceholderKind::Invalid;
        let mut missing_identity = candidate("");
        missing_identity.session_id = None;
        let mut wrong_repo = candidate("wrong-repo");
        wrong_repo.worktree_path = Some(other_repo);

        let report = import_legacy_recovery_placeholders(
            &[
                shell,
                non_agent,
                live,
                invalid,
                missing_identity,
                wrong_repo,
            ],
            &BTreeSet::new(),
            &store,
            &expected_repo_id,
        );

        assert!(report.imported_attention.is_empty());
        assert_eq!(report.errors, Vec::new());
        assert_eq!(
            skip_reason(&report, "shell"),
            &LegacyRecoverySkipReason::ShellPlaceholder
        );
        assert_eq!(
            skip_reason(&report, "board"),
            &LegacyRecoverySkipReason::NonAgentPlaceholder
        );
        assert_eq!(
            skip_reason(&report, "live"),
            &LegacyRecoverySkipReason::LiveAgentPlaceholder
        );
        assert_eq!(
            skip_reason(&report, "done"),
            &LegacyRecoverySkipReason::InvalidPlaceholder
        );
        assert_eq!(
            skip_reason(&report, ""),
            &LegacyRecoverySkipReason::MissingPlaceholderIdentity
        );
        assert!(matches!(
            skip_reason(&report, "wrong-repo"),
            LegacyRecoverySkipReason::RepoScopeMismatch { .. }
        ));
        assert!(store.list().expect("list recoveries").is_empty());
    }

    fn legacy_session_at(id: &str, worktree: &Path, created_at: DateTime<Utc>) -> Session {
        let mut session = legacy_session(id, worktree);
        session.created_at = created_at;
        session.updated_at = created_at;
        session.last_activity_at = created_at;
        session
    }

    fn write_codex_metadata(
        sessions_root: &Path,
        session: &Session,
        root_id: &str,
        observed_at: DateTime<Utc>,
        source: serde_json::Value,
        private_marker: &str,
    ) {
        let day_dir = sessions_root
            .join(session.created_at.format("%Y").to_string())
            .join(session.created_at.format("%m").to_string())
            .join(session.created_at.format("%d").to_string());
        fs::create_dir_all(&day_dir).expect("Codex fixture day directory");
        let envelope = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "id": root_id,
                "cwd": session.worktree_path,
                "timestamp": observed_at,
                "source": source,
                "base_instructions": private_marker
            }
        });
        let path = day_dir.join(format!("rollout-fixture-{root_id}.jsonl"));
        fs::write(
            path,
            format!(
                "{}\n",
                serde_json::to_string(&envelope).expect("serialize Codex metadata")
            ),
        )
        .expect("write Codex fixture");
    }

    fn write_claude_history(
        projects_root: &Path,
        session: &Session,
        root_id: &str,
        _observed_at: DateTime<Utc>,
        is_sidechain: bool,
        private_marker: &str,
    ) {
        let project_dir = projects_root.join(claude_project_path_key(&session.worktree_path));
        fs::create_dir_all(&project_dir).expect("Claude fixture project directory");
        let path = if is_sidechain {
            project_dir
                .join("session-tree")
                .join("subagents")
                .join(format!("{root_id}.jsonl"))
        } else {
            project_dir.join(format!("{root_id}.jsonl"))
        };
        fs::create_dir_all(path.parent().expect("Claude fixture parent"))
            .expect("Claude fixture parent directory");
        // Deliberately not JSON. Discovery must succeed from filename, parent
        // path, and mtime without opening or parsing message/transcript bytes.
        fs::write(
            path,
            format!("UNREADABLE-TRANSCRIPT-SENTINEL:{private_marker}\n"),
        )
        .expect("write Claude fixture");
    }

    fn legacy_session(id: &str, worktree: &Path) -> Session {
        let mut session = Session::new(worktree, "legacy", AgentId::Codex);
        session.id = id.to_string();
        session.recovery_id = None;
        session.agent_session_id = None;
        session.session_history.clear();
        session.session_kind = Some(gwt_skills::SessionKind::Execution);
        session.launch_base_oid = None;
        session
    }

    fn create_request(
        recovery_id: &str,
        session_id: &str,
        repo: &Path,
        repo_id: &str,
    ) -> CreateRecovery {
        let head = git_stdout(repo, &["rev-parse", "HEAD"]);
        CreateRecovery {
            recovery_id: recovery_id.to_string(),
            session_id: session_id.to_string(),
            repo_id: repo_id.to_string(),
            session_kind: RecoverySessionKind::Execution,
            worktree_path: repo.to_path_buf(),
            launch_base_ref: None,
            launch_base_oid: head.clone(),
            launch_head_oid: head,
            provider: "Codex".to_string(),
            model: None,
            runtime: "host".to_string(),
            initial_prompt: String::new(),
            created_at: Utc::now(),
        }
    }

    fn attention_reasons<'a>(
        report: &'a LegacyRecoveryImportReport,
        session_id: &str,
    ) -> &'a [LegacyRecoveryAttentionReason] {
        &report
            .imported_attention
            .iter()
            .find(|item| item.session_id == session_id)
            .expect("attention item")
            .reasons
    }

    fn skip_reason<'a>(
        report: &'a LegacyRecoveryImportReport,
        session_id: &str,
    ) -> &'a LegacyRecoverySkipReason {
        &report
            .skipped
            .iter()
            .find(|item| item.session_id == session_id)
            .expect("skipped item")
            .reason
    }

    fn init_repo(path: &Path, origin: &str, commit: bool) {
        fs::create_dir_all(path).expect("repo directory");
        git(path, &["init"]);
        git(path, &["remote", "add", "origin", origin]);
        if commit {
            git(path, &["config", "user.email", "legacy@example.com"]);
            git(path, &["config", "user.name", "Legacy Test"]);
            fs::write(path.join("tracked.txt"), b"tracked\n").expect("tracked file");
            git(path, &["add", "tracked.txt"]);
            git(path, &["commit", "-m", "test: seed"]);
        }
    }

    fn git(path: &Path, args: &[&str]) {
        let mut command = hidden_command("git");
        command.current_dir(path).args(args);
        scrub_git_env(&mut command);
        let output = command.output().expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(path: &Path, args: &[&str]) -> String {
        let mut command = hidden_command("git");
        command.current_dir(path).args(args);
        scrub_git_env(&mut command);
        let output = command.output().expect("run git");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout)
            .expect("utf8 git output")
            .trim()
            .to_string()
    }
}

//! Managed hook health read model.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, PoisonError,
    },
    time::SystemTime,
};

use gwt_agent::PendingDiscussionResume;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLOW_HANDLER_THRESHOLD_MS: f64 = 1000.0;
const SELF_HEALED_MARKER: &str = ".gwt/managed-hook-self-healed";
/// How long a hook failure keeps showing in `hook.health` (Issue #3541).
const HOOK_FAILURE_RETENTION_HOURS: i64 = 24;
const MANAGED_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHookHealthStatus {
    Ready,
    NeedsAttention,
    SelfHealed,
    Degraded,
    Inactive,
    WaitingForFirstHookEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingHookGoal {
    pub proposal_label: String,
    pub proposal_title: String,
    pub condition: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookProfileEvidence {
    pub event: String,
    pub handler: String,
    pub status: String,
    pub duration_ms: f64,
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManagedHookHealth {
    pub status: ManagedHookHealthStatus,
    pub last_event: Option<String>,
    pub last_event_at: Option<String>,
    pub pending_discussion: Option<PendingDiscussionResume>,
    pub pending_goal: Option<PendingHookGoal>,
    pub slow_handlers: Vec<HookProfileEvidence>,
    pub issues: Vec<String>,
    /// Advisory conflicts never change managed hook readiness.
    pub external_skill_conflicts: super::external_skill_conflicts::ConflictReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHookHealthInput {
    pub worktree_root: PathBuf,
    pub runtime_state_path: Option<PathBuf>,
    pub profile_path: Option<PathBuf>,
    pub expected_hook_bin: Option<String>,
}

impl ManagedHookHealthInput {
    pub fn new(worktree_root: impl AsRef<Path>) -> Self {
        Self {
            worktree_root: worktree_root.as_ref().to_path_buf(),
            runtime_state_path: std::env::var_os(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV)
                .map(PathBuf::from),
            profile_path: None,
            expected_hook_bin: std::env::var("GWT_HOOK_BIN")
                .ok()
                .filter(|value| !value.is_empty()),
        }
    }

    pub fn with_runtime_state_path(mut self, path: impl AsRef<Path>) -> Self {
        self.runtime_state_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_profile_path(mut self, path: impl AsRef<Path>) -> Self {
        self.profile_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_expected_hook_bin(mut self, bin: impl Into<String>) -> Self {
        self.expected_hook_bin = Some(bin.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedHookRepairOutcome {
    pub repaired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookDoctorPurpose {
    ManagedBrowser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightContext {
    Current,
    Missing,
    Unreadable,
    Foreign,
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightCompletion {
    Normal,
    Blocked,
    Missing,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightConvergence {
    Converged,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightResult {
    Proceed,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightReason {
    SourceMissing,
    SourceUnreadable,
    SourceForeign,
    SourceChanged,
    AuthorityUnavailable,
    CompletionMissing,
    CompletionBlocked,
    CompletionStale,
    TargetUnconverged,
    LedgerUnavailable,
    UnattributedFailure,
    UnresolvedFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetainedHookClassification {
    Recovered,
    Unresolved,
    FailOpen,
    Foreign,
    Unattributed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetainedHookDiagnostic {
    pub id: String,
    pub classification: RetainedHookClassification,
}

/// Point-in-time diagnosis. No path, policy payload, or reusable authority is
/// included; native operations retain their own authorization checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedBrowserPreflight {
    pub schema_version: u8,
    pub context: PreflightContext,
    pub normal_completion: PreflightCompletion,
    pub convergence: PreflightConvergence,
    pub diagnostics: Vec<RetainedHookDiagnostic>,
    pub result: PreflightResult,
    pub reasons: Vec<PreflightReason>,
}

pub fn read_managed_browser_preflight(input: &ManagedHookHealthInput) -> ManagedBrowserPreflight {
    let mut input = input.clone();
    input.worktree_root = gwt_core::paths::resolve_current_worktree_root(&input.worktree_root);
    read_managed_browser_preflight_with(&input, &ManagedHookFailureSnapshot::read(), || {
        super::runtime_state::current_hook_context(&input.worktree_root)
    })
}

fn read_managed_browser_preflight_with(
    input: &ManagedHookHealthInput,
    failures: &ManagedHookFailureSnapshot,
    mut current: impl FnMut() -> Result<
        (
            super::runtime_state::CurrentHookContext,
            gwt_agent::SessionRuntimeState,
        ),
        super::runtime_state::HookContextError,
    >,
) -> ManagedBrowserPreflight {
    use super::runtime_state::HookContextError;
    use gwt_agent::session::HookCompletionOutcome;
    let mut result = ManagedBrowserPreflight {
        schema_version: 1,
        context: PreflightContext::Missing,
        normal_completion: PreflightCompletion::Missing,
        convergence: PreflightConvergence::Unknown,
        diagnostics: Vec::new(),
        result: PreflightResult::Unknown,
        reasons: Vec::new(),
    };
    let (before, _) = match current() {
        Ok(pair) => pair,
        Err(error) => {
            (result.context, result.reasons) = match error {
                HookContextError::Missing => (
                    PreflightContext::Missing,
                    vec![PreflightReason::SourceMissing],
                ),
                HookContextError::Unreadable => (
                    PreflightContext::Unreadable,
                    vec![PreflightReason::SourceUnreadable],
                ),
                HookContextError::Foreign => (
                    PreflightContext::Foreign,
                    vec![PreflightReason::SourceForeign],
                ),
            };
            return result;
        }
    };
    result.context = PreflightContext::Current;
    let authority =
        crate::cli::verification_record::caller_has_current_bound_verification_authority(
            &input.worktree_root,
            &before.identity.execution_binding,
        );
    // Audit the target's config/binary/trust without treating retained source
    // diagnostics or a fabricated target runtime as target convergence.
    let surface = cached_surface_audit(input, failures);
    let mut target = ManagedHookHealth {
        status: surface.status,
        issues: surface.issues,
        external_skill_conflicts: surface.external_skill_conflicts,
        last_event: None,
        last_event_at: None,
        pending_discussion: None,
        pending_goal: None,
        slow_handlers: Vec::new(),
    };
    super::append_codex_trust_health_for_doctor(input, &mut target);
    // A logical fallback is optional only when the managed runtime indirection
    // passed the surface audit and this real daemon is executable. Merely
    // spelling "gwtd" or omitting it from PATH is not positive evidence.
    let executable_runtime = std::env::current_exe().ok().is_some_and(|path| {
        path.file_stem().is_some_and(|name| name == "gwtd") && which::which(&path).is_ok()
    });
    let optional_logical_fallback = input.expected_hook_bin.as_deref() == Some("gwtd")
        && executable_runtime
        && !target.issues.is_empty()
        && target.issues.iter().all(|issue| {
            issue.starts_with("managed hook binary missing: ") && issue.ends_with(" uses gwtd")
        });
    if (target.status == ManagedHookHealthStatus::Ready && target.issues.is_empty())
        || optional_logical_fallback
    {
        result.convergence = PreflightConvergence::Converged;
    } else {
        result.convergence = PreflightConvergence::Blocked;
        result.reasons.push(PreflightReason::TargetUnconverged);
    }
    let (after, runtime) = match current() {
        Ok(pair) if pair.0 == before => pair,
        _ => {
            result.context = PreflightContext::Changed;
            result.reasons.push(PreflightReason::SourceChanged);
            return result;
        }
    };
    if !authority
        || !crate::cli::verification_record::caller_has_current_bound_verification_authority(
            &input.worktree_root,
            &after.identity.execution_binding,
        )
    {
        result.reasons.push(PreflightReason::AuthorityUnavailable);
    }
    if !failures.ledger_readable {
        result.reasons.push(PreflightReason::LedgerUnavailable);
    }
    let completed_at = runtime.last_completed_hook_event_at;
    match (runtime.last_completed_hook_outcome, completed_at) {
        (Some(HookCompletionOutcome::Blocked), _) => {
            result.normal_completion = PreflightCompletion::Blocked;
            result.reasons.push(PreflightReason::CompletionBlocked);
        }
        (Some(HookCompletionOutcome::Normal), Some(at))
            if at >= runtime.updated_at && at <= chrono::Utc::now() =>
        {
            result.normal_completion = PreflightCompletion::Normal;
        }
        (Some(HookCompletionOutcome::Normal), Some(_)) => {
            result.normal_completion = PreflightCompletion::Stale;
            result.reasons.push(PreflightReason::CompletionStale);
        }
        _ => {
            result.reasons.push(PreflightReason::CompletionMissing);
        }
    }
    if let Some(rows) = failures
        .by_worktree
        .get(&comparable_path(&input.worktree_root))
    {
        for row in &rows.retained {
            let classification = if row.context.get("fail_open").map(String::as_str) == Some("true")
            {
                RetainedHookClassification::FailOpen
            } else {
                match row.target.session_id.as_deref().filter(|id| !id.is_empty()) {
                    Some(id) if id == before.identity.session_id => {
                        if runtime.last_completed_hook_outcome
                            == Some(HookCompletionOutcome::Normal)
                            && completed_at.is_some_and(|at| at > row.recorded_at)
                        {
                            RetainedHookClassification::Recovered
                        } else {
                            result.reasons.push(PreflightReason::UnresolvedFailure);
                            if result.normal_completion == PreflightCompletion::Normal {
                                result.normal_completion = PreflightCompletion::Stale;
                            }
                            RetainedHookClassification::Unresolved
                        }
                    }
                    Some(_) => RetainedHookClassification::Foreign,
                    None => {
                        result.reasons.push(PreflightReason::UnattributedFailure);
                        RetainedHookClassification::Unattributed
                    }
                }
            };
            result.diagnostics.push(RetainedHookDiagnostic {
                id: row.id.clone(),
                classification,
            });
        }
    }
    result.reasons.dedup();
    result.result = if result.reasons.is_empty() {
        PreflightResult::Proceed
    } else if result.normal_completion == PreflightCompletion::Blocked
        || result.convergence == PreflightConvergence::Blocked
        || result.reasons.contains(&PreflightReason::UnresolvedFailure)
    {
        PreflightResult::Blocked
    } else {
        PreflightResult::Unknown
    };
    result
}

#[derive(Debug, Deserialize)]
struct RuntimeStateReadModel {
    pub status: String,
    pub updated_at: String,
    // Keep the required string in the input contract even though health does not
    // read it; removing it would accept missing or incorrectly typed runtime state.
    #[allow(dead_code)]
    pub last_activity_at: String,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

/// Hook failures read once and reused for a whole projection.
///
/// Issue #4172: `read_managed_hook_health` is called once per Work row, and
/// each call used to scan the whole host error ledger. With a real ledger the
/// scan alone cost seconds per projection and grew with the recorded volume.
/// One projection now takes a single snapshot and every row reads its own
/// worktree's failures out of it, so the cost stops scaling with Work count.
///
/// The snapshot is deliberately not cached beyond the projection that holds
/// it: the next projection takes a fresh one, so newly recorded failures still
/// surface on the next refresh.
#[derive(Debug, Clone, Default)]
pub struct ManagedHookFailureSnapshot {
    ledger_readable: bool,
    /// Latest hard / fail-open failure per canonicalized worktree root.
    ///
    /// Folding at snapshot time keeps the per-worktree read out of the ledger
    /// size entirely: the rows are canonicalized and reduced once, so a Work
    /// row only looks its own worktree up.
    by_worktree: HashMap<PathBuf, WorktreeHookFailures>,
    /// Surface audits this snapshot refreshed or reused (Issue #4370).
    surface_audits: Arc<SurfaceAuditCounters>,
    /// Issue #4257: bare hook binaries resolved during this projection.
    hook_binaries: HookBinaryResolutionCache,
    /// Session selection, health and recovery share one runtime read per projection.
    runtime_states: RefCell<HashMap<PathBuf, Arc<CachedRuntimeState>>>,
}

/// Memo of bare hook-binary resolution for one projection.
///
/// Issue #4257: resolving a bare fallback such as `gwtd` walks the whole
/// PATH (~11ms per lookup on a 59-entry Windows PATH) and depends only on
/// process-wide state, yet every managed command of every event of every
/// Work row asked again. The GUI builds the projection on its event loop,
/// so that repetition held pane replies for tens of seconds.
#[derive(Debug, Clone, Default)]
struct HookBinaryResolutionCache {
    bare_resolved: RefCell<HashMap<String, Option<PathBuf>>>,
}

impl HookBinaryResolutionCache {
    fn resolve_bare_hook_binary(&self, actual: &str) -> Option<PathBuf> {
        if let Some(resolved) = self.bare_resolved.borrow().get(actual) {
            return resolved.clone();
        }
        let resolved = resolve_bare_hook_binary(actual);
        self.bare_resolved
            .borrow_mut()
            .insert(actual.to_string(), resolved.clone());
        resolved
    }
}

/// Surface audit cache activity for one [`ManagedHookFailureSnapshot`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SurfaceAuditStats {
    /// Rows whose managed hook surface was audited from disk.
    pub refreshed: usize,
    /// Rows whose unchanged surface reused the cached audit.
    pub reused: usize,
}

#[derive(Debug, Default)]
struct SurfaceAuditCounters {
    refreshed: AtomicUsize,
    reused: AtomicUsize,
}

#[derive(Debug, Clone, Default)]
struct WorktreeHookFailures {
    latest_hard: Option<gwt_core::error_ledger::ErrorRecord>,
    latest_fail_open: Option<gwt_core::error_ledger::ErrorRecord>,
    retained: Vec<gwt_core::error_ledger::ErrorRecord>,
}

impl ManagedHookFailureSnapshot {
    /// Read the retained hook failures from the host error ledger once.
    ///
    /// A ledger that cannot be read yields an empty snapshot, matching the
    /// fail-open behaviour the per-row read had.
    pub fn read() -> Self {
        use gwt_core::error_ledger::ErrorKind;

        let since = chrono::Utc::now() - chrono::Duration::hours(HOOK_FAILURE_RETENTION_HOURS);
        let mut by_worktree: HashMap<PathBuf, WorktreeHookFailures> = HashMap::new();
        // `list_since` returns rows oldest first, so the last match wins.
        let rows = gwt_core::error_ledger::list_since(Some(since));
        let ledger_readable = rows.as_ref().is_ok_and(|rows| {
            !rows
                .iter()
                .any(|row| row.kind == ErrorKind::LedgerCorruption)
        });
        for row in rows
            .unwrap_or_default()
            .into_iter()
            .filter(|row| row.kind == ErrorKind::HookFailure)
        {
            let Some(project_root) = row.target.project_root.as_deref() else {
                continue;
            };
            let key = comparable_path(Path::new(project_root));
            let entry = by_worktree.entry(key).or_default();
            entry.retained.push(row.clone());
            if row.context.get("fail_open").map(String::as_str) == Some("true") {
                entry.latest_fail_open = Some(row);
            } else {
                entry.latest_hard = Some(row);
            }
        }
        Self {
            ledger_readable,
            by_worktree,
            surface_audits: Arc::default(),
            hook_binaries: HookBinaryResolutionCache::default(),
            runtime_states: RefCell::default(),
        }
    }

    /// Number of distinct bare hook binaries this projection resolved.
    pub fn resolved_hook_binaries(&self) -> usize {
        self.hook_binaries.bare_resolved.borrow().len()
    }

    /// Project managed hook health for one worktree out of this snapshot.
    pub fn read_health(&self, input: &ManagedHookHealthInput) -> ManagedHookHealth {
        read_managed_hook_health_with(input, self)
    }

    /// How many rows this snapshot audited from disk versus reused.
    pub fn surface_audit_stats(&self) -> SurfaceAuditStats {
        SurfaceAuditStats {
            refreshed: self.surface_audits.refreshed.load(Ordering::Relaxed),
            reused: self.surface_audits.reused.load(Ordering::Relaxed),
        }
    }

    /// Session ordering uses the same JSON snapshot as its eventual health read.
    pub fn runtime_state_updated_at(
        &self,
        path: &Path,
    ) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        self.runtime_state(path).updated_at
    }

    fn runtime_state(&self, path: &Path) -> Arc<CachedRuntimeState> {
        if let Some(state) = self.runtime_states.borrow().get(path) {
            return Arc::clone(state);
        }
        let state = cached_runtime_state(path);
        self.runtime_states
            .borrow_mut()
            .insert(path.to_path_buf(), Arc::clone(&state));
        state
    }
}

pub fn read_managed_hook_health(input: &ManagedHookHealthInput) -> ManagedHookHealth {
    read_managed_hook_health_with(input, &ManagedHookFailureSnapshot::read())
}

fn read_managed_hook_health_with(
    input: &ManagedHookHealthInput,
    failures: &ManagedHookFailureSnapshot,
) -> ManagedHookHealth {
    let mut health = ManagedHookHealth {
        status: ManagedHookHealthStatus::Ready,
        last_event: None,
        last_event_at: None,
        pending_discussion: None,
        pending_goal: crate::discussion_resume::load_pending_goal_from_worktree_files(
            &input.worktree_root,
        )
        .ok()
        .flatten()
        .map(|goal| PendingHookGoal {
            proposal_label: goal.proposal_label,
            proposal_title: goal.proposal_title,
            condition: goal.condition,
        }),
        slow_handlers: Vec::new(),
        issues: Vec::new(),
        external_skill_conflicts: Default::default(),
    };

    let surface = cached_surface_audit(input, failures);
    health.status = surface.status;
    health.issues = surface.issues;
    health.external_skill_conflicts = surface.external_skill_conflicts;
    audit_hook_profile(input, &mut health);
    audit_hook_failures(input, failures, &mut health);

    let Some(runtime_state_path) = input.runtime_state_path.as_ref() else {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::Inactive;
        }
        apply_self_healed_marker(input, &mut health);
        return health;
    };

    let runtime = failures.runtime_state(runtime_state_path);
    let Some(state) = runtime.state.as_ref() else {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
        }
        apply_self_healed_marker(input, &mut health);
        return health;
    };

    match state {
        Ok(runtime_state) => {
            if let Some(source_event) = runtime_state.source_event.as_ref() {
                health.last_event = Some(source_event.clone());
                health.last_event_at = Some(runtime_state.updated_at.clone());
            } else if health.status == ManagedHookHealthStatus::Ready {
                health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
            }
            health.pending_discussion = runtime_state.pending_discussion.clone();
            if runtime_state.status == "Stopped" && health.status == ManagedHookHealthStatus::Ready
            {
                health.status = ManagedHookHealthStatus::Inactive;
            }
        }
        Err(error) => {
            health.status = ManagedHookHealthStatus::Degraded;
            health
                .issues
                .push(format!("runtime state could not be read: {}", error));
        }
    }

    apply_self_healed_marker(input, &mut health);
    health
}

fn apply_self_healed_marker(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    if input.worktree_root.join(SELF_HEALED_MARKER).is_file()
        && matches!(
            health.status,
            ManagedHookHealthStatus::Ready
                | ManagedHookHealthStatus::Inactive
                | ManagedHookHealthStatus::WaitingForFirstHookEvent
        )
    {
        health.status = ManagedHookHealthStatus::SelfHealed;
    }
}

pub fn record_managed_hook_self_healed(worktree_root: &Path) -> io::Result<()> {
    let marker = worktree_root.join(SELF_HEALED_MARKER);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(marker, chrono::Utc::now().to_rfc3339())
}

fn audit_hook_profile(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    let Some(profile_path) = input.profile_path.as_ref() else {
        return;
    };
    if !profile_path.exists() {
        return;
    }

    let Ok(raw) = fs::read_to_string(profile_path) else {
        needs_attention(
            health,
            format!("hook profile could not be read: {}", profile_path.display()),
        );
        return;
    };

    for (index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            needs_attention(
                health,
                format!(
                    "hook profile line {} is not valid JSON: {}",
                    index + 1,
                    profile_path.display()
                ),
            );
            continue;
        };
        let Some(duration_ms) = record.get("duration_ms").and_then(Value::as_f64) else {
            continue;
        };
        if duration_ms < SLOW_HANDLER_THRESHOLD_MS {
            continue;
        }
        let event = record
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let handler = record
            .get("handler")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let status = record
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let occurred_at = record
            .get("occurred_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        health.slow_handlers.push(HookProfileEvidence {
            event: event.clone(),
            handler: handler.clone(),
            status,
            duration_ms,
            occurred_at,
        });
        needs_attention(
            health,
            format!("slow managed hook handler: {event}/{handler} took {duration_ms:.1}ms"),
        );
    }
}

/// Issue #3541: project recent hook failures from the host error ledger so a
/// later successful event turns them into "recovered" evidence instead of
/// erasing them. Unresolved handler failures degrade health; recovered and
/// fail-open (live forwarding) failures need attention until they age out.
fn audit_hook_failures(
    input: &ManagedHookHealthInput,
    failures: &ManagedHookFailureSnapshot,
    health: &mut ManagedHookHealth,
) {
    let worktree = comparable_path(&input.worktree_root);
    let Some(matching) = failures.by_worktree.get(&worktree) else {
        return;
    };

    if let Some(row) = matching.latest_hard.as_ref() {
        let completed_at = input
            .runtime_state_path
            .as_deref()
            .and_then(|path| failures.runtime_state(path).completed_at);
        let recovered = completed_at.is_some_and(|at| at > row.recorded_at);
        let state = if recovered { "recovered" } else { "unresolved" };
        let issue = describe_hook_failure(row, state);
        if recovered {
            needs_attention(health, issue);
        } else {
            degraded(health, issue);
        }
    }
    if let Some(row) = matching.latest_fail_open.as_ref() {
        needs_attention(health, describe_hook_failure(row, "fail-open"));
    }
}

fn describe_hook_failure(row: &gwt_core::error_ledger::ErrorRecord, state: &str) -> String {
    let field = |key: &str| {
        row.context
            .get(key)
            .map(String::as_str)
            .unwrap_or("unknown")
            .to_string()
    };
    format!(
        "managed hook failure: {}/{} state={state} recorded_at={} (errors.list id={})",
        field("event"),
        field("handler"),
        row.recorded_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        row.id
    )
}

fn comparable_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Environment that decides how a bare hook binary such as `gwtd` resolves.
const SURFACE_AUDIT_ENV: &[&str] = &["PATH", "GWT_BIN_PATH", "HOME", "USERPROFILE"];

/// Managed hook surface audits reused across projections (Issue #4370).
///
/// The Active Work projection reads hook health once per Work row, and the
/// surface audit spawns `git` (tracked-config and hooks-path probes) and parses
/// every hook config. At 592 rows that cost 72-77 s per build although almost
/// no worktree's surface changes between builds. An audit is now kept per
/// worktree and expected binary, and reused while every file it depends on
/// keeps the same length and modification time.
///
/// Inputs the fingerprint cannot see — a config becoming git-tracked without a
/// content change, or `core.hooksPath` being set where it was unset — are
/// picked up the next time any watched file changes.
static SURFACE_AUDITS: OnceLock<Mutex<HashMap<SurfaceAuditKey, CachedSurfaceAudit>>> =
    OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SurfaceAuditKey {
    worktree: PathBuf,
    expected_hook_bin: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedSurfaceAudit {
    fingerprint: SurfaceFingerprint,
    /// Paths outside the fixed surface the audit consulted: hook binaries and
    /// the `core.hooksPath` hook scripts.
    dependencies: Vec<PathBuf>,
    audit: SurfaceAudit,
}

#[derive(Debug, Clone)]
struct SurfaceAudit {
    status: ManagedHookHealthStatus,
    issues: Vec<String>,
    external_skill_conflicts: super::external_skill_conflicts::ConflictReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SurfaceFingerprint {
    env: Vec<Option<OsString>>,
    files: Vec<Option<FileStamp>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn cached_surface_audit(
    input: &ManagedHookHealthInput,
    snapshot: &ManagedHookFailureSnapshot,
) -> SurfaceAudit {
    let counters = &snapshot.surface_audits;
    let cache = SURFACE_AUDITS.get_or_init(Mutex::default);
    let key = SurfaceAuditKey {
        worktree: input.worktree_root.clone(),
        expected_hook_bin: input.expected_hook_bin.clone(),
    };
    let surface = watched_surface_paths(&input.worktree_root);
    let cached = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .cloned();
    if let Some(cached) = cached {
        if surface_fingerprint(&surface, &cached.dependencies) == cached.fingerprint {
            counters.reused.fetch_add(1, Ordering::Relaxed);
            return cached.audit;
        }
    }

    // Stamp the surface before auditing it, so a write that lands mid-audit
    // mismatches on the next read instead of being cached as seen.
    let mut fingerprint = surface_fingerprint(&surface, &[]);
    let mut scratch = ManagedHookHealth {
        status: ManagedHookHealthStatus::Ready,
        last_event: None,
        last_event_at: None,
        pending_discussion: None,
        pending_goal: None,
        slow_handlers: Vec::new(),
        issues: Vec::new(),
        external_skill_conflicts: Default::default(),
    };
    let mut dependencies = Vec::new();
    audit_managed_hook_configs(
        input,
        &snapshot.hook_binaries,
        &mut scratch,
        &mut dependencies,
    );
    scratch.external_skill_conflicts =
        super::external_skill_conflicts::scan_worktree(&input.worktree_root);
    dependencies.extend(
        scratch
            .external_skill_conflicts
            .watched_paths
            .iter()
            .cloned(),
    );
    fingerprint
        .files
        .extend(dependencies.iter().map(|path| file_stamp(path)));
    let audit = SurfaceAudit {
        status: scratch.status,
        issues: scratch.issues,
        external_skill_conflicts: scratch.external_skill_conflicts,
    };
    counters.refreshed.fetch_add(1, Ordering::Relaxed);
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(
        key,
        CachedSurfaceAudit {
            fingerprint,
            dependencies,
            audit: audit.clone(),
        },
    );
    audit
}

/// File inputs for the Work projection's hook health snapshot. Runtime state
/// and the error ledger are fingerprinted separately by the projection.
pub fn managed_hook_health_dependency_paths(worktree: &Path) -> Vec<PathBuf> {
    let mut paths = watched_surface_paths(worktree);
    paths.extend([
        worktree.join(SELF_HEALED_MARKER),
        gwt_core::paths::gwt_work_notes_discussions_path(worktree),
        worktree.join(crate::discussion_resume::CANONICAL_DISCUSSIONS_DISPLAY_PATH),
        worktree.join(crate::discussion_resume::DISCUSSION_RELATIVE_PATH),
    ]);
    let input = ManagedHookHealthInput::new(worktree);
    let key = SurfaceAuditKey {
        worktree: input.worktree_root,
        expected_hook_bin: input.expected_hook_bin,
    };
    if let Some(cache) = SURFACE_AUDITS.get() {
        if let Some(cached) = cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            paths.extend(cached.dependencies.iter().cloned());
        }
    }
    paths
}

/// Every path whose presence or content decides the surface audit.
fn watched_surface_paths(worktree: &Path) -> Vec<PathBuf> {
    let external_paths = super::external_skill_conflicts::discovery_paths(worktree);
    let runtime = crate::pm_registry::pm_runtime_dir_for_pm_worktree(worktree);
    let worktree = runtime.as_deref().unwrap_or(worktree);
    let mut paths = vec![
        worktree.join(".claude"),
        worktree.join(".claude/settings.local.json"),
        worktree.join(".codex"),
        worktree.join(".codex/hooks.json"),
    ];
    paths.extend(external_paths);
    for hooks in crate::managed_assets::managed_codex_hook_paths(worktree) {
        paths.push(codex_root_of(&hooks));
        paths.push(hooks);
    }
    for (root, artifact) in provider_hook_surfaces(worktree) {
        paths.push(root);
        paths.push(artifact);
    }
    paths
}

fn surface_fingerprint(surface: &[PathBuf], dependencies: &[PathBuf]) -> SurfaceFingerprint {
    SurfaceFingerprint {
        env: SURFACE_AUDIT_ENV.iter().map(std::env::var_os).collect(),
        files: surface
            .iter()
            .chain(dependencies)
            .map(|path| file_stamp(path))
            .collect(),
    }
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Provider plugin surfaces as `(provider root, generated hook artifact)`.
fn provider_hook_surfaces(worktree: &Path) -> [(PathBuf, PathBuf); 3] {
    [
        (
            worktree.join(".gwt/opencode"),
            worktree.join(".gwt/opencode/plugins/gwt-hooks.js"),
        ),
        (
            worktree.join(".gwt/openclaw"),
            worktree.join(".gwt/openclaw/plugins/gwt-hook-bridge/plugin.ts"),
        ),
        (
            worktree.join(".gwt/hermes"),
            worktree.join(".gwt/hermes/agent-hooks/gwt-hook.sh"),
        ),
    ]
}

fn audit_managed_hook_configs(
    input: &ManagedHookHealthInput,
    binaries: &HookBinaryResolutionCache,
    health: &mut ManagedHookHealth,
    dependencies: &mut Vec<PathBuf>,
) {
    // Provider configuration follows PM runtime discovery. Session identity
    // and Git hook dependencies remain rooted in the canonical checkout.
    let runtime = crate::pm_registry::pm_runtime_dir_for_pm_worktree(&input.worktree_root);
    let worktree = runtime.as_deref().unwrap_or(&input.worktree_root);
    let claude_dir = worktree.join(".claude");
    let claude_settings = worktree.join(".claude/settings.local.json");
    // #3474: audit every `.codex/hooks.json` the self-heal writer owns, not
    // just the worktree-local one. For a linked worktree the writer targets the
    // repo-root (workspace-home) copy that newer Codex reads, so auditing only
    // the worktree-local copy reported a file nothing would ever rewrite.
    let codex_hooks_paths = crate::managed_assets::managed_codex_hook_paths(worktree);
    let provider_hooks = provider_hook_surfaces(worktree);

    // Whether this worktree has a gwt surface at all stays a worktree-local
    // question: the workspace-home copy is shared by every worktree, so it must
    // never make an unmaterialized one report hook health.
    let codex_dir = worktree.join(".codex");
    let has_surface = claude_dir.exists()
        || claude_settings.exists()
        || codex_dir.exists()
        || worktree.join(".codex/hooks.json").exists()
        || provider_hooks
            .iter()
            .any(|(root, artifact)| root.exists() || artifact.exists());
    if !has_surface {
        health.status = ManagedHookHealthStatus::Inactive;
        return;
    }

    if claude_dir.exists() && !claude_settings.exists() {
        needs_attention(
            health,
            "managed hook config missing: .claude/settings.local.json",
        );
    }
    for hooks in &codex_hooks_paths {
        if codex_root_of(hooks).exists() && !hooks.exists() {
            needs_attention(
                health,
                format!("managed hook config missing: {}", hooks.display()),
            );
        }
    }

    let expected_hook_bin = input.expected_hook_bin.as_deref();
    if claude_settings.exists() {
        audit_hook_json_config(
            &claude_settings,
            expected_hook_bin,
            binaries,
            health,
            dependencies,
        );
    }
    for hooks in &codex_hooks_paths {
        if hooks.exists() {
            audit_hook_json_config(hooks, expected_hook_bin, binaries, health, dependencies);
        }
    }

    for (root, artifact) in provider_hooks {
        if artifact.exists() {
            audit_provider_hook_config(
                &artifact,
                expected_hook_bin,
                binaries,
                health,
                dependencies,
            );
        } else if root.exists() {
            needs_attention(
                health,
                format!("managed hook config missing: {}", artifact.display()),
            );
        }
    }

    audit_managed_git_hooks(&input.worktree_root, health, dependencies);
}

/// Issue #4339: a `core.hooksPath` aimed at a directory with no hook in it
/// makes Git skip commitlint and the commit/push gates without a word. Report
/// it so the skip is visible and the self-heal pass can materialize them.
fn audit_managed_git_hooks(
    worktree: &Path,
    health: &mut ManagedHookHealth,
    dependencies: &mut Vec<PathBuf>,
) {
    let Some(plan) = gwt_skills::plan_managed_git_hooks(worktree) else {
        return;
    };
    let missing_hooks = plan.missing_hooks();
    dependencies.push(plan.hooks_dir);
    for hook in plan.hooks {
        dependencies.push(hook.source);
        dependencies.push(hook.target);
    }
    for missing in missing_hooks {
        needs_attention(
            health,
            format!(
                "managed git hook missing: {} (core.hooksPath is configured but the hook is not materialized)",
                missing.display()
            ),
        );
    }
}

/// The binary a config at `path` is expected to fall back to.
///
/// Use the same destination policy as generation and trust registration.
/// Otherwise a canonical shared-root config looks skewed against the writer's
/// install pin, causing startup to self-heal it on every boot (#3567, #5270).
fn expected_hook_bin_for_config_path(path: &Path, configured: Option<&str>) -> Option<String> {
    configured.map(|bin| gwt_skills::settings_local::sanitize_hook_bin_for_config_path(path, bin))
}

fn audit_hook_json_config(
    path: &Path,
    expected_hook_bin: Option<&str>,
    binaries: &HookBinaryResolutionCache,
    health: &mut ManagedHookHealth,
    dependencies: &mut Vec<PathBuf>,
) {
    let expected_hook_bin = expected_hook_bin_for_config_path(path, expected_hook_bin);
    let Ok(raw) = fs::read_to_string(path) else {
        degraded(
            health,
            format!("managed hook config could not be read: {}", path.display()),
        );
        return;
    };
    let Ok(root) = serde_json::from_str::<Value>(&raw) else {
        degraded(
            health,
            format!("managed hook config is not valid JSON: {}", path.display()),
        );
        return;
    };

    for event in MANAGED_EVENTS {
        let raw_commands = hook_commands_for_event(&root, event);
        for command in &raw_commands {
            if is_managed_event_command(command, event)
                && command.starts_with("powershell ")
                && command.contains(" -Command \"")
            {
                needs_attention(
                    health,
                    format!(
                        "managed hook PowerShell transport unsafe: {}",
                        path.display()
                    ),
                );
            }
        }
        let commands: Vec<_> = raw_commands
            .into_iter()
            .map(|command| {
                gwt_skills::decode_powershell_encoded_command(&command).unwrap_or(command)
            })
            .collect();
        if !commands
            .iter()
            .any(|command| is_managed_event_command(command, event))
        {
            needs_attention(
                health,
                format!(
                    "managed hook event missing: {} in {}",
                    event,
                    path.display()
                ),
            );
        }
        for command in &commands {
            if !is_managed_event_command(command, event) {
                continue;
            }
            if !command.contains("GWT_BIN_PATH") {
                needs_attention(
                    health,
                    format!("managed hook runtime resolver missing: {}", path.display()),
                );
            }
            if !has_runtime_guard(command) {
                needs_attention(
                    health,
                    format!("managed hook runtime guard missing: {}", path.display()),
                );
            }
        }
        if let Some(expected) = expected_hook_bin.as_deref() {
            for command in commands {
                if !is_managed_event_command(&command, event) {
                    continue;
                }
                let Some(actual) = hook_command_binary_fallback(&command) else {
                    continue;
                };
                audit_hook_binary(
                    path,
                    &actual,
                    Some(expected),
                    binaries,
                    health,
                    dependencies,
                );
            }
        } else {
            for command in commands {
                if !is_managed_event_command(&command, event) {
                    continue;
                }
                if let Some(actual) = hook_command_binary_fallback(&command) {
                    audit_hook_binary(path, &actual, None, binaries, health, dependencies);
                }
            }
        }
    }
}

fn audit_provider_hook_config(
    path: &Path,
    expected_hook_bin: Option<&str>,
    binaries: &HookBinaryResolutionCache,
    health: &mut ManagedHookHealth,
    dependencies: &mut Vec<PathBuf>,
) {
    // Provider generators keep explicit pins; only Claude/Codex JSON writers
    // use the destination sanitizer. Preserve the provider audit contract.
    let expected_hook_bin = expected_hook_bin.map(|bin| {
        if gwt_skills::managed_hook_config_is_git_tracked(path) {
            gwt_skills::CANONICAL_HOOK_BIN
        } else {
            bin
        }
    });
    let Ok(raw) = fs::read_to_string(path) else {
        degraded(
            health,
            format!("managed hook config could not be read: {}", path.display()),
        );
        return;
    };
    if !raw.contains("GWT_BIN_PATH") {
        needs_attention(
            health,
            format!("managed hook runtime resolver missing: {}", path.display()),
        );
    }
    let Some(actual) = hook_command_binary_fallback(&raw) else {
        needs_attention(
            health,
            format!("managed hook runtime resolver missing: {}", path.display()),
        );
        return;
    };
    audit_hook_binary(
        path,
        &actual,
        expected_hook_bin,
        binaries,
        health,
        dependencies,
    );
}

fn audit_hook_binary(
    path: &Path,
    actual: &str,
    expected_hook_bin: Option<&str>,
    binaries: &HookBinaryResolutionCache,
    health: &mut ManagedHookHealth,
    dependencies: &mut Vec<PathBuf>,
) {
    let actual_path = Path::new(actual);
    let explicitly_pinned = expected_hook_bin.is_some_and(|expected| expected == actual);
    if crate::managed_assets::is_worktree_local_build_binary(actual_path) && !explicitly_pinned {
        degraded(
            health,
            format!(
                "managed hook worktree-local binary: {} uses {}",
                path.display(),
                actual
            ),
        );
        return;
    }
    if let Some(expected) = expected_hook_bin {
        if actual != expected {
            degraded(
                health,
                format!(
                    "managed hook binary skew: {} uses {}, expected {}",
                    path.display(),
                    actual,
                    expected
                ),
            );
        }
    }
    if looks_absolute(actual) {
        dependencies.push(actual_path.to_path_buf());
        if !actual_path.is_file() {
            degraded(
                health,
                format!(
                    "managed hook binary missing: {} uses {}",
                    path.display(),
                    actual
                ),
            );
        } else if which::which(actual).is_err() {
            degraded(
                health,
                format!(
                    "managed hook binary not executable: {} uses {}",
                    path.display(),
                    actual
                ),
            );
        }
    } else if let Some(resolved) = binaries.resolve_bare_hook_binary(actual) {
        dependencies.push(resolved);
    } else {
        degraded(
            health,
            format!(
                "managed hook binary missing: {} uses {}",
                path.display(),
                actual
            ),
        );
    }
}

/// Whether a bare-name hook fallback such as `gwtd` resolves to a real binary.
///
/// #3474 root cause 4: `which` searches the *calling process's* PATH. A gwt GUI
/// launched from Finder or the Dock inherits launchd's PATH, which lacks
/// `/Applications/GWT.app/Contents/MacOS`, so the same fallback that resolves
/// in a terminal — and always resolves for a gwt-launched agent, which gets
/// `GWT_BIN_PATH` injected and its directory prepended to PATH — was reported
/// as missing and turned every Work card red. Fall back to gwt's own
/// PATH-independent resolver, and only accept a hit that actually names the
/// binary the hook asks for.
fn resolve_bare_hook_binary(actual: &str) -> Option<PathBuf> {
    if let Ok(resolved) = which::which(actual) {
        return Some(resolved);
    }
    crate::cli::gwtd_resolver::resolve_gwtd_path()
        .filter(|resolved| binary_names_match(resolved, actual))
}

fn binary_names_match(resolved: &Path, actual: &str) -> bool {
    let resolved = resolved.file_name().and_then(|name| name.to_str());
    resolved.is_some_and(|resolved| {
        strip_exe_suffix(resolved).eq_ignore_ascii_case(strip_exe_suffix(actual))
    })
}

fn strip_exe_suffix(value: &str) -> &str {
    value
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(value, |(stem, _)| stem)
}

fn codex_root_of(hooks_path: &Path) -> PathBuf {
    hooks_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

/// Whether a generated managed event command degrades to a no-op when its
/// binary cannot be resolved, instead of hard-failing the agent's hook.
///
/// #3474: the template committed before `b8fa26c04` / `2c660f11e` invoked
/// `"$gwt_bin"` unconditionally, so a Codex started outside gwt (no
/// `GWT_BIN_PATH`, no `gwtd` on PATH) failed every hook with
/// `command not found`. The current POSIX template guards the call with
/// `command -v`, and the PowerShell template wraps it in `try`/`catch`. A
/// missing guard is its own issue class so the startup self-heal loop breaker —
/// which skips a worktree whose issues are *only* `managed hook binary
/// missing:` — can never strand a legacy config (root cause 3).
fn has_runtime_guard(command: &str) -> bool {
    command.contains("command -v ") || command.contains("catch {")
}

fn hook_commands_for_event(root: &Value, event: &str) -> Vec<String> {
    let Some(groups) = root
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flat_map(|hooks| hooks.iter())
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn is_managed_event_command(command: &str, event: &str) -> bool {
    command.contains(&format!("hook event {event}"))
}

fn hook_command_binary_fallback(command: &str) -> Option<String> {
    if let Some(value) = posix_shell_assignment_value(command, "gwt_fallback") {
        return Some(value);
    }
    if let Some(rest) = command
        .split_once("gwt_bin=\"${GWT_BIN_PATH:-}\"")
        .map(|(_, rest)| rest)
    {
        if let Some(value) = posix_shell_assignment_value(rest, "gwt_bin") {
            return Some(value);
        }
    }
    if let Some(rest) = command.split_once("${GWT_BIN_PATH:-").map(|(_, rest)| rest) {
        let value = rest.split_once('}')?.0;
        return nonempty_unquoted(value);
    }
    if let Some(rest) = command
        .split_once("process.env.GWT_BIN_PATH ||")
        .map(|(_, rest)| rest.trim_start())
    {
        let value = rest.split_once(';').map_or(rest, |(value, _)| value).trim();
        if let Ok(value) = serde_json::from_str::<String>(value) {
            return (!value.is_empty()).then_some(value);
        }
        return nonempty_unquoted(value);
    }
    if let Some(rest) = command.split_once("else {").map(|(_, rest)| rest) {
        let value = rest.split_once('}')?.0;
        return nonempty_powershell_quoted(value);
    }
    let (prefix, _) = command.split_once(" hook ")?;
    nonempty_unquoted(prefix)
}

fn posix_shell_assignment_value(command: &str, variable: &str) -> Option<String> {
    let assignment = format!("{variable}=");
    let rest = command.split_once(&assignment)?.1.trim_start();
    posix_shell_word(rest)
}

fn posix_shell_word(value: &str) -> Option<String> {
    if value.starts_with('\'') {
        let mut cursor = 1;
        loop {
            let closing = cursor + value.get(cursor..)?.find('\'')?;
            if value
                .get(closing + 1..)
                .is_some_and(|rest| rest.starts_with("\\''"))
            {
                cursor = closing + 4;
                continue;
            }
            let token = value.get(..=closing)?;
            let decoded = token
                .strip_prefix('\'')?
                .strip_suffix('\'')?
                .replace(r"'\''", "'");
            return (!decoded.is_empty()).then_some(decoded);
        }
    }

    let end = value
        .find(|character: char| character == ';' || character.is_whitespace())
        .unwrap_or(value.len());
    nonempty_unquoted(value.get(..end)?)
}

fn nonempty_powershell_quoted(value: &str) -> Option<String> {
    nonempty_unquoted(value).map(|value| value.replace("''", "'"))
}

fn nonempty_unquoted(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .replace("\\\"", "\"")
        .replace("\\$", "$")
        .replace("\\`", "`")
        .replace("\\\\", "\\");
    (!value.is_empty()).then_some(value)
}

fn looks_absolute(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn needs_attention(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    if health.status == ManagedHookHealthStatus::Ready
        || health.status == ManagedHookHealthStatus::Inactive
        || health.status == ManagedHookHealthStatus::WaitingForFirstHookEvent
    {
        health.status = ManagedHookHealthStatus::NeedsAttention;
    }
    push_unique_issue(health, issue.into());
}

fn degraded(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    health.status = ManagedHookHealthStatus::Degraded;
    push_unique_issue(health, issue.into());
}

fn push_unique_issue(health: &mut ManagedHookHealth, issue: String) {
    if !health.issues.contains(&issue) {
        health.issues.push(issue);
    }
}

/// Issue #5120: unchanged runtime files reuse their decoded result across projections.
static RUNTIME_STATES: OnceLock<Mutex<HashMap<PathBuf, Arc<CachedRuntimeState>>>> = OnceLock::new();

#[derive(Debug)]
struct CachedRuntimeState {
    stamp: Option<FileStamp>,
    updated_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    completed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Missing files remain distinct from malformed or unreadable files.
    state: Option<Result<RuntimeStateReadModel, String>>,
}

fn cached_runtime_state(path: &Path) -> Arc<CachedRuntimeState> {
    // Stamp before reading so a concurrent rewrite is noticed next projection.
    let stamp = file_stamp(path);
    let cache = RUNTIME_STATES.get_or_init(Mutex::default);
    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(path)
    {
        if cached.stamp == stamp {
            return Arc::clone(cached);
        }
    }
    let mut runtime = CachedRuntimeState {
        stamp,
        updated_at: None,
        completed_at: None,
        state: None,
    };
    if stamp.is_some() {
        let value = fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|raw| {
                serde_json::from_slice::<Value>(&raw).map_err(|error| error.to_string())
            });
        runtime.state = Some(value.and_then(|value| {
            // Preserve selection/recovery timestamps even when typed health is invalid.
            let timestamp = |field| {
                value
                    .get(field)
                    .and_then(Value::as_str)
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            };
            runtime.updated_at = timestamp("updated_at");
            runtime.completed_at =
                timestamp("last_completed_hook_event_at").map(|at| at.with_timezone(&chrono::Utc));
            serde_json::from_value(value).map_err(|error| error.to_string())
        }));
    }
    let runtime = Arc::new(runtime);
    cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.to_path_buf(), Arc::clone(&runtime));
    runtime
}

pub fn repair_managed_hook_configs(worktree_root: &Path) -> io::Result<ManagedHookRepairOutcome> {
    let runtime = crate::pm_registry::pm_runtime_dir_for_pm_worktree(worktree_root);
    let assets = runtime.as_deref().unwrap_or(worktree_root);
    let claude_surface =
        assets.join(".claude").exists() || assets.join(".claude/settings.local.json").exists();
    let codex_surface = assets.join(".codex").exists() || assets.join(".codex/hooks.json").exists();
    let provider_surface = assets.join(".gwt/opencode").exists()
        || assets.join(".gwt/openclaw").exists()
        || assets.join(".gwt/hermes").exists();
    // #4339: the Git hook directory belongs to the repository, not to any agent
    // provider, so repair it even in a worktree with no agent surface at all.
    let mut repaired = !gwt_skills::materialize_managed_git_hooks(worktree_root)?.is_empty();

    if claude_surface || codex_surface || provider_surface {
        crate::managed_assets::regenerate_existing_managed_hook_configs(worktree_root)?;
        record_managed_hook_self_healed(worktree_root)?;
        repaired = true;
    }

    Ok(ManagedHookRepairOutcome { repaired })
}

#[cfg(test)]
mod tests {
    use super::hook_command_binary_fallback;

    #[test]
    fn shared_root_codex_health_expects_the_canonical_fallback() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let path = repo.path().join(".codex/hooks.json");
        assert_eq!(
            super::expected_hook_bin_for_config_path(&path, Some("/installed/gwtd")).as_deref(),
            Some(gwt_skills::CANONICAL_HOOK_BIN)
        );
        assert!(super::expected_hook_bin_for_config_path(&path, None).is_none());
    }

    #[test]
    fn posix_two_stage_runtime_fallback_decodes_embedded_apostrophes() {
        let command = r#"gwt_bin="${GWT_BIN_PATH:-}"; if [ -z "$gwt_bin" ]; then gwt_bin='/opt/GWT O'\''Brien}/gwtd'; fi; if command -v "$gwt_bin" >/dev/null 2>&1; then "$gwt_bin" hook event Stop; else true; fi"#;

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("/opt/GWT O'Brien}/gwtd")
        );
    }

    #[test]
    fn posix_named_runtime_fallback_decodes_embedded_apostrophes() {
        let command = r#"gwt_fallback='/opt/GWT O'\''Brien}/gwtd'; gwt_bin="${GWT_BIN_PATH:-$gwt_fallback}"; "$gwt_bin" hook event Stop"#;

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("/opt/GWT O'Brien}/gwtd")
        );
    }

    #[test]
    fn powershell_runtime_fallback_decodes_embedded_apostrophes() {
        let command = "powershell -NoProfile -Command \"& { $gwtBin = if ($env:GWT_BIN_PATH) { $env:GWT_BIN_PATH } else { 'C:\\Users\\O''Brien\\GWT\\gwtd.exe' }; & $gwtBin hook event Stop }\"";

        assert_eq!(
            hook_command_binary_fallback(command).as_deref(),
            Some("C:\\Users\\O'Brien\\GWT\\gwtd.exe")
        );
    }
}

#[cfg(test)]
mod browser_preflight_tests {
    use super::*;
    use gwt_agent::session::HookCompletionOutcome::{Blocked, Normal};
    use gwt_agent::{
        AgentId, AgentStatus, Session, SessionExecutionBinding, SessionExecutionIdentity,
        SessionRuntimeState,
    };
    use gwt_core::{
        error_ledger::{ErrorKind, ErrorRecord, ErrorTarget},
        test_support::{ScopedEnvVar, ScopedGwtHome},
    };

    fn with_source(
        test: impl FnOnce(&ManagedHookHealthInput, &Session, &Path, &SessionRuntimeState),
    ) {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(root.path().join("home"));
        let worktree = root.path().join("repo");
        fs::create_dir(&worktree).unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(&worktree);
        let sessions = gwt_core::paths::gwt_sessions_dir();
        let mut session = Session::new(&worktree, "work/issue-6622", AgentId::ClaudeCode);
        session.linked_issue_number = Some(6622);
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
            number: 6622,
        };
        crate::cli::execution_state::materialize_at_launch(
            &worktree,
            owner.kind,
            owner.number,
            &session.id,
            "gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::execution_state::ensure_generation_ledger(
            &worktree,
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let binding = crate::cli::execution_state::current_execution_binding(&worktree, owner)
            .unwrap()
            .unwrap();
        session
            .set_execution_binding(Some(SessionExecutionBinding {
                schema_version: 1,
                session_id: session.id.clone(),
                repo_hash: session.repo_hash.clone().unwrap(),
                owner_kind: "spec".into(),
                owner_number: 6622,
                identity: binding,
                capability_generation: 1,
            }))
            .unwrap();
        session.save(&sessions).unwrap();
        let identity = SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let pid = std::process::id();
        let started = crate::process::host_process_start_time(pid).unwrap();
        let mut runtime = SessionRuntimeState::for_execution_process(
            AgentStatus::Running,
            &identity,
            17,
            started,
            pid,
            started,
        );
        runtime.updated_at = chrono::Utc::now() - chrono::Duration::seconds(5);
        runtime.source_event = Some("PostToolUse".into());
        runtime.last_completed_hook_event_at =
            Some(chrono::Utc::now() - chrono::Duration::seconds(1));
        runtime.last_completed_hook_outcome = Some(Normal);
        let path = gwt_agent::runtime_state_path(&sessions, &session.id);
        runtime.save(&path).unwrap();
        let _sid = ScopedEnvVar::set("GWT_SESSION_ID", &session.id);
        let _runtime = ScopedEnvVar::set("GWT_SESSION_RUNTIME_PATH", &path);
        let bin = std::env::current_exe().unwrap();
        let _bin = gwt_skills::settings_local::ScopedHookBin::set(&bin);
        gwt_skills::generate_settings_local(&worktree).unwrap();
        let input =
            ManagedHookHealthInput::new(&worktree).with_expected_hook_bin(bin.to_string_lossy());
        test(&input, &session, &path, &runtime);
    }

    fn failure(
        input: &ManagedHookHealthInput,
        sid: Option<&str>,
        at: chrono::DateTime<chrono::Utc>,
    ) -> ErrorRecord {
        let mut row = ErrorRecord::new(
            ErrorKind::HookFailure,
            "PRIVATE_POLICY_PAYLOAD",
            ErrorTarget {
                project_root: Some(input.worktree_root.display().to_string()),
                session_id: sid.map(str::to_string),
                ..Default::default()
            },
        );
        row.recorded_at = at;
        row
    }

    #[test]
    fn managed_browser_nested_cwd_completes_only_for_the_current_git_worktree() {
        with_source(|input, _, path, runtime| {
            let nested = input.worktree_root.join("src/deep");
            fs::create_dir_all(&nested).unwrap();
            let completion = super::super::runtime_state::capture_hook_completion(&nested)
                .expect("same worktree subdirectory is a current source");
            completion.record(Normal);
            let completed = SessionRuntimeState::load(path).unwrap();
            assert!(completed.last_completed_hook_event_at > runtime.last_completed_hook_event_at);
            assert_eq!(completed.last_completed_hook_outcome, Some(Normal));

            let foreign = input.worktree_root.join("vendor/other");
            fs::create_dir_all(&foreign).unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(&foreign);
            let foreign_nested = foreign.join("src");
            fs::create_dir(&foreign_nested).unwrap();
            assert_eq!(
                super::super::runtime_state::current_hook_context(&foreign_nested).unwrap_err(),
                super::super::runtime_state::HookContextError::Foreign
            );
            assert!(
                super::super::runtime_state::capture_hook_completion(&foreign_nested).is_none()
            );
            let after = SessionRuntimeState::load(path).unwrap();
            assert_eq!(
                after.last_completed_hook_event_at,
                completed.last_completed_hook_event_at
            );
            assert_eq!(
                after.last_completed_hook_outcome,
                completed.last_completed_hook_outcome
            );
        });
    }

    #[test]
    fn managed_browser_nested_cwd_doctor_audits_and_repairs_the_current_git_worktree() {
        with_source(|input, _, _, _| {
            let nested = input.worktree_root.join("src/deep");
            fs::create_dir_all(&nested).unwrap();
            let mut nested_input = input.clone();
            nested_input.worktree_root = nested.clone();
            assert_eq!(
                read_managed_browser_preflight(&nested_input).result,
                PreflightResult::Proceed
            );

            fs::remove_file(input.worktree_root.join(".claude/settings.local.json")).unwrap();
            let mut env = crate::cli::env::TestEnv::new(nested.clone());
            let (code, output) = crate::cli::run_collect(
                &mut env,
                crate::cli::CliCommand::Hook(crate::cli::HookCommand::Doctor {
                    purpose: Some(HookDoctorPurpose::ManagedBrowser),
                    runtime_state_path: None,
                    profile_path: None,
                    expected_hook_bin: input.expected_hook_bin.clone(),
                    repair: true,
                }),
            )
            .unwrap();
            assert_eq!(code, 0);
            let payload: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(payload["preflight"]["result"], "proceed", "{payload}");
            assert!(input
                .worktree_root
                .join(".claude/settings.local.json")
                .is_file());
            assert!(!nested.join(".claude").exists());

            let foreign = input.worktree_root.join("other");
            fs::create_dir(&foreign).unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(&foreign);
            nested_input.worktree_root = foreign;
            assert_eq!(
                read_managed_browser_preflight(&nested_input).context,
                PreflightContext::Foreign
            );
        });
    }

    #[test]
    fn normal_recovery_proceeds_while_legacy_health_and_failure_bytes_are_retained() {
        with_source(|input, session, _, runtime| {
            let row = failure(input, Some(&session.id), runtime.updated_at);
            gwt_core::error_ledger::record(row.clone()).unwrap();
            let before = gwt_core::error_ledger::list_since(None).unwrap();
            let health = read_managed_hook_health(input);
            assert_eq!(health.status, ManagedHookHealthStatus::NeedsAttention);
            assert!(health.issues.iter().any(|s| s.contains("state=recovered")));
            let result = read_managed_browser_preflight(input);
            assert_eq!(result.result, PreflightResult::Proceed, "{result:?}");
            assert_eq!(
                result.diagnostics,
                vec![RetainedHookDiagnostic {
                    id: row.id,
                    classification: RetainedHookClassification::Recovered
                }]
            );
            assert_eq!(gwt_core::error_ledger::list_since(None).unwrap(), before);
            let wire = serde_json::to_string(&result).unwrap();
            assert!(!wire.contains("PRIVATE_POLICY_PAYLOAD"));
            assert!(!wire.contains(&input.worktree_root.display().to_string()));
            assert!(!wire.contains(&session.id));
            assert!(!wire.contains("execution_binding"));
        });
    }

    #[test]
    fn blocked_missing_old_equal_and_backward_completion_cannot_proceed() {
        with_source(|input, session, path, runtime| {
            let at = runtime.last_completed_hook_event_at.unwrap();
            gwt_core::error_ledger::record(failure(input, Some(&session.id), at)).unwrap();
            for (outcome, completed) in [
                (Some(Blocked), Some(at + chrono::Duration::seconds(1))),
                (None, Some(at)),
                (Some(Normal), None),
                (Some(Normal), Some(at)),
                (Some(Normal), Some(at - chrono::Duration::seconds(1))),
                (
                    Some(Normal),
                    Some(runtime.updated_at - chrono::Duration::seconds(1)),
                ),
            ] {
                let mut state = runtime.clone();
                state.last_completed_hook_outcome = outcome;
                state.last_completed_hook_event_at = completed;
                state.save(path).unwrap();
                assert_ne!(
                    read_managed_browser_preflight(input).result,
                    PreflightResult::Proceed
                );
            }
        });
    }

    #[test]
    fn foreign_rows_never_hide_related_failure_and_unattributed_rows_remain_unknown() {
        with_source(|input, session, _, runtime| {
            let at = runtime.last_completed_hook_event_at.unwrap();
            let related = failure(input, Some(&session.id), at);
            let foreign = failure(
                input,
                Some("foreign-session"),
                at + chrono::Duration::seconds(1),
            );
            gwt_core::error_ledger::record(related).unwrap();
            gwt_core::error_ledger::record(foreign).unwrap();
            let result = read_managed_browser_preflight(input);
            assert_eq!(result.result, PreflightResult::Blocked);
            assert!(result
                .diagnostics
                .iter()
                .any(|d| d.classification == RetainedHookClassification::Foreign));
        });
        with_source(|input, _, _, runtime| {
            gwt_core::error_ledger::record(failure(input, None, runtime.updated_at)).unwrap();
            let result = read_managed_browser_preflight(input);
            assert_eq!(result.result, PreflightResult::Unknown);
            assert!(result
                .reasons
                .contains(&PreflightReason::UnattributedFailure));
        });
    }

    #[test]
    fn missing_unreadable_foreign_and_old_schema_source_fail_closed() {
        with_source(|input, session, path, runtime| {
            assert_eq!(
                read_managed_browser_preflight(input).result,
                PreflightResult::Proceed
            );
            for wire in [
                "{".to_string(),
                {
                    let mut wire = serde_json::to_value(runtime).unwrap();
                    wire.as_object_mut()
                        .unwrap()
                        .remove("last_completed_hook_outcome");
                    wire.to_string()
                },
                {
                    let mut wire = serde_json::to_value(runtime).unwrap();
                    wire["last_completed_hook_outcome"] = serde_json::json!("future_unknown");
                    wire.to_string()
                },
            ] {
                fs::write(path, wire).unwrap();
                assert_ne!(
                    read_managed_browser_preflight(input).result,
                    PreflightResult::Proceed
                );
            }
            fs::remove_file(path).unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).context,
                PreflightContext::Missing
            );
            runtime.save(path).unwrap();
            let arbitrary = input.worktree_root.join("arbitrary-runtime.json");
            runtime.save(&arbitrary).unwrap();
            {
                let _path = ScopedEnvVar::set("GWT_SESSION_RUNTIME_PATH", &arbitrary);
                assert_eq!(
                    read_managed_browser_preflight(input).context,
                    PreflightContext::Foreign
                );
            }
            let mut foreign = runtime.clone();
            foreign.execution_identity.as_mut().unwrap().session_id = "foreign".into();
            foreign.save(path).unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).context,
                PreflightContext::Foreign
            );
            runtime.save(path).unwrap();
            let mut other = input.clone();
            other.worktree_root = input.worktree_root.parent().unwrap().to_path_buf();
            assert_eq!(
                read_managed_browser_preflight(&other).context,
                PreflightContext::Foreign
            );
            let session_path =
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id));
            fs::write(session_path, "invalid TOML {").unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).context,
                PreflightContext::Unreadable
            );
            fs::remove_file(
                gwt_core::paths::gwt_sessions_dir().join(format!("{}.toml", session.id)),
            )
            .unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).context,
                PreflightContext::Missing
            );
        });
    }

    #[test]
    fn source_replacement_during_read_and_late_predecessor_completion_are_rejected() {
        with_source(|input, _, path, runtime| {
            let completion =
                super::super::runtime_state::capture_hook_completion(&input.worktree_root).unwrap();
            let snapshot = ManagedHookFailureSnapshot::read();
            let mut reads = 0;
            let result = read_managed_browser_preflight_with(input, &snapshot, || {
                reads += 1;
                if reads == 2 {
                    let mut replacement = runtime.clone();
                    replacement.runtime_incarnation = Some(18);
                    replacement.last_completed_hook_event_at = None;
                    replacement.last_completed_hook_outcome = None;
                    replacement.save(path).unwrap();
                }
                super::super::runtime_state::current_hook_context(&input.worktree_root)
            });
            assert_eq!(result.context, PreflightContext::Changed);
            assert_eq!(result.result, PreflightResult::Unknown);
            let before = fs::read(path).unwrap();
            completion.record(Normal);
            assert_eq!(fs::read(path).unwrap(), before);
            let current =
                super::super::runtime_state::capture_hook_completion(&input.worktree_root).unwrap();
            current.record(Blocked);
            let state = SessionRuntimeState::load(path).unwrap();
            assert_eq!(state.last_completed_hook_outcome, Some(Blocked));
            assert!(state.last_completed_hook_event_at.is_some());
        });
        with_source(|input, session, path, _| {
            let completion =
                super::super::runtime_state::capture_hook_completion(&input.worktree_root).unwrap();
            let snapshot = ManagedHookFailureSnapshot::read();
            let mut reads = 0;
            let result = read_managed_browser_preflight_with(input, &snapshot, || {
                reads += 1;
                if reads == 2 {
                    let mut replacement = session.clone();
                    replacement
                        .execution_binding
                        .as_mut()
                        .unwrap()
                        .capability_generation += 1;
                    replacement
                        .save(&gwt_core::paths::gwt_sessions_dir())
                        .unwrap();
                }
                super::super::runtime_state::current_hook_context(&input.worktree_root)
            });
            assert_eq!(result.context, PreflightContext::Changed);
            let before = fs::read(path).unwrap();
            completion.record(Normal);
            assert_eq!(fs::read(path).unwrap(), before);
        });
    }

    #[test]
    fn legacy_verification_compatibility_does_not_supply_bound_authority() {
        with_source(|input, session, _, _| {
            let binding = session.execution_binding.as_ref().unwrap();
            assert!(
                crate::cli::verification_record::caller_has_current_bound_verification_authority(
                    &input.worktree_root,
                    binding
                )
            );
            let empty = tempfile::tempdir().unwrap();
            crate::cli::trusted_store::init_git_repo_with_origin(empty.path());
            assert!(
                crate::cli::verification_record::caller_has_verification_authority(
                    empty.path(),
                    &session.id
                )
            );
            assert!(
                !crate::cli::verification_record::caller_has_current_bound_verification_authority(
                    empty.path(),
                    binding
                )
            );
            // Native verification accepts an authentic same-Session lifecycle
            // prefix; doctor must retain that legitimate positive case.
            crate::cli::execution_state::settle(
                &input.worktree_root,
                &session.id,
                crate::cli::execution_state::ExecutionSettlement::Blocked {
                    reason: "fixture recovery".into(),
                    missing_verification: None,
                },
            )
            .unwrap();
            let current = crate::cli::execution_state::current_execution_binding(
                &input.worktree_root,
                crate::cli::execution_state::ExecutionOwnerKey {
                    kind: crate::cli::execution_state::ExecutionOwnerKind::Spec,
                    number: 6622,
                },
            )
            .unwrap()
            .unwrap();
            assert_ne!(current.ledger_head_hash, binding.identity.ledger_head_hash);
            assert!(
                crate::cli::verification_record::caller_has_verification_authority(
                    &input.worktree_root,
                    &session.id
                )
            );
            assert!(
                crate::cli::verification_record::caller_has_current_bound_verification_authority(
                    &input.worktree_root,
                    binding
                )
            );
        });
    }

    #[test]
    fn target_config_binary_and_trust_must_converge_and_repair_does_not_stamp_source() {
        with_source(|input, _, path, _| {
            let before = fs::read(path).unwrap();
            let settings = input.worktree_root.join(".claude/settings.local.json");
            fs::write(&settings, "{}").unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).convergence,
                PreflightConvergence::Blocked
            );
            repair_managed_hook_configs(&input.worktree_root).unwrap();
            assert_eq!(fs::read(path).unwrap(), before);
            assert_eq!(
                read_managed_browser_preflight(input).result,
                PreflightResult::Proceed
            );
            let wrong = input
                .clone()
                .with_expected_hook_bin("/missing/fixture/gwtd");
            assert_eq!(
                read_managed_browser_preflight(&wrong).convergence,
                PreflightConvergence::Blocked
            );
            let codex = tempfile::tempdir().unwrap();
            let _codex = ScopedEnvVar::set("CODEX_HOME", codex.path());
            gwt_skills::generate_codex_hooks(&input.worktree_root).unwrap();
            assert_eq!(
                read_managed_browser_preflight(input).convergence,
                PreflightConvergence::Blocked
            );
        });
    }
}

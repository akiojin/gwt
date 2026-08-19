use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Condvar, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{
    protocol::{IndexSearchMatchMode, IndexSearchResult, IndexSearchScope, IndexSearchTarget},
    worktree_inventory,
};

const INDEX_SEARCH_LIMIT: usize = 50;
const SEARCH_ATTEMPT_HARD_LIMIT_MS: u64 = 30_000;
const RUNNER_DIAGNOSTIC_MAX_BYTES: usize = 512;

/// Exit code for retryable "index not ready" search failures (Phase 70
/// FR-388): missing / corrupt scopes that did not repair within the wait
/// window must never degrade into a silent empty success.
pub const INDEX_NOT_READY_EXIT_CODE: i32 = 75;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectIndexSearchOutcome {
    pub results: Vec<IndexSearchResult>,
    pub suggestions: Vec<IndexSearchResult>,
    /// Scopes whose results came from a healthy but stale generation
    /// (FR-387 stale-while-revalidate).
    pub stale_scopes: Vec<String>,
    /// True when a single-flight refresh was queued for the stale scopes.
    pub refresh_queued: bool,
}

/// Typed retry information for FR-388 `INDEX_NOT_READY` failures.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexSearchNotReady {
    pub reason: String,
    pub affected_scopes: Vec<String>,
    pub waited_ms: u64,
    pub retry_after_ms: u64,
}

/// Non-retryable query failure against a scope whose canonical health check
/// still reports a usable store (Phase 70a FR-400).
#[derive(Debug, Clone, PartialEq)]
pub struct IndexSearchFailed {
    pub reason: String,
    pub affected_scopes: Vec<String>,
}

/// Retryable infrastructure failure of an internal search attempt (Phase 70d
/// FR-097/FR-103). This is intentionally crate-private: the public error enum
/// predates this classification and downstream exhaustive matches must remain
/// source compatible.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct IndexSearchUnavailable {
    pub(crate) reason: String,
    pub(crate) retry_after_ms: u64,
}

/// Search error surface (Phase 70 FR-388). `NotReady` is retryable and maps
/// to exit code 75 / `error_code=INDEX_NOT_READY` on the CLI surface.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexSearchError {
    NotReady(IndexSearchNotReady),
    SearchFailed(IndexSearchFailed),
    Other(String),
}

impl IndexSearchError {
    pub fn exit_code(&self) -> i32 {
        match self {
            IndexSearchError::NotReady(_) => INDEX_NOT_READY_EXIT_CODE,
            IndexSearchError::SearchFailed(_) | IndexSearchError::Other(_) => 1,
        }
    }

    pub fn error_code(&self) -> Option<&'static str> {
        match self {
            IndexSearchError::NotReady(_) => Some("INDEX_NOT_READY"),
            IndexSearchError::SearchFailed(_) => Some("SEARCH_FAILED"),
            IndexSearchError::Other(_) => None,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, IndexSearchError::NotReady(_))
    }

    /// Retry delay hint carried by the retryable variants (FR-098 metadata).
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            IndexSearchError::NotReady(not_ready) => Some(not_ready.retry_after_ms),
            IndexSearchError::SearchFailed(_) | IndexSearchError::Other(_) => None,
        }
    }
}

impl std::fmt::Display for IndexSearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexSearchError::NotReady(not_ready) => write!(
                f,
                "index not ready for scopes [{}] after {} ms: {} (retry in {} ms)",
                not_ready.affected_scopes.join(", "),
                not_ready.waited_ms,
                not_ready.reason,
                not_ready.retry_after_ms,
            ),
            IndexSearchError::SearchFailed(failed) => f.write_str(&failed.reason),
            IndexSearchError::Other(message) => f.write_str(message),
        }
    }
}

impl From<String> for IndexSearchError {
    fn from(message: String) -> Self {
        IndexSearchError::Other(message)
    }
}

/// Crate-internal attempt taxonomy. CLI and Knowledge Bridge consume this
/// typed path so retry metadata never depends on parsing display strings.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum IndexSearchAttemptError {
    Public(IndexSearchError),
    Unavailable(IndexSearchUnavailable),
}

impl IndexSearchAttemptError {
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            Self::Public(error) => error.exit_code(),
            Self::Unavailable(_) => 1,
        }
    }

    pub(crate) fn error_code(&self) -> Option<&'static str> {
        match self {
            Self::Public(error) => error.error_code(),
            Self::Unavailable(_) => Some("SEARCH_UNAVAILABLE"),
        }
    }

    pub(crate) fn retryable(&self) -> bool {
        match self {
            Self::Public(error) => error.retryable(),
            Self::Unavailable(_) => true,
        }
    }

    pub(crate) fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::Public(error) => error.retry_after_ms(),
            Self::Unavailable(unavailable) => Some(unavailable.retry_after_ms),
        }
    }

    fn into_public(self) -> IndexSearchError {
        match self {
            Self::Public(error) => error,
            Self::Unavailable(_) => IndexSearchError::Other(
                "project index search is temporarily unavailable".to_string(),
            ),
        }
    }
}

impl std::fmt::Display for IndexSearchAttemptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public(error) => error.fmt(f),
            Self::Unavailable(unavailable) => write!(
                f,
                "search unavailable: {} (retry in {} ms)",
                unavailable.reason, unavailable.retry_after_ms,
            ),
        }
    }
}

impl From<IndexSearchError> for IndexSearchAttemptError {
    fn from(error: IndexSearchError) -> Self {
        Self::Public(error)
    }
}

impl From<String> for IndexSearchAttemptError {
    fn from(message: String) -> Self {
        Self::Public(IndexSearchError::Other(message))
    }
}

/// `auto_build`: `false` for GUI interactive search (the watcher owns index
/// builds; never block on inline rebuilds), `true` for JSON / agent search
/// (`search`, SPEC-1942 FR-107) where no watcher exists and the runner
/// must self-heal missing or stale indexes inline.
pub fn search_project_index(
    project_root: &Path,
    query: &str,
    scopes: &[IndexSearchScope],
    selected_worktree_hash: Option<&str>,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
) -> Result<ProjectIndexSearchOutcome, IndexSearchError> {
    search_project_index_attempt(
        project_root,
        query,
        scopes,
        selected_worktree_hash,
        match_mode,
        auto_build,
    )
    .map_err(IndexSearchAttemptError::into_public)
}

pub(crate) fn search_project_index_attempt(
    project_root: &Path,
    query: &str,
    scopes: &[IndexSearchScope],
    selected_worktree_hash: Option<&str>,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
) -> Result<ProjectIndexSearchOutcome, IndexSearchAttemptError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(ProjectIndexSearchOutcome::default());
    }
    // One absolute attempt budget covers runtime ensure/provisioning, its
    // cross-process lock, every health probe, repair polling, and the final
    // runner. Nested callers retain an earlier ambient deadline.
    let _attempt_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        Instant::now() + Duration::from_millis(search_attempt_deadline_ms()),
    );
    let git_context = crate::index_worker::project_index_git_context_for_search(project_root)
        .map_err(|_| search_unavailable_error("project index git context unavailable"))?
        .ok_or_else(|| "project index search requires a git origin remote".to_string())?;
    let index_repo_root = git_context.repo_root.clone();
    let repo_hash = crate::index_worker::detect_repo_hash(&index_repo_root)
        .ok_or_else(|| "project index search requires a git origin remote".to_string())?;
    let repo_search_root =
        crate::index_worker::default_project_index_worktree_root_for_search(&git_context)
            .map_err(|_| search_unavailable_error("project index git context unavailable"))?;
    gwt_core::runtime::ensure_project_index_runtime()
        .map_err(|_| search_unavailable_error("project index runtime unavailable"))?;

    let effective_scopes = if scopes.is_empty() {
        default_index_search_scopes()
    } else {
        scopes.to_vec()
    };
    let board_scope = crate::board_audience::gui_default_board_scope(project_root)
        .unwrap_or(gwt_core::coordination::BoardAudienceScope::All);
    let file_worktree = if effective_scopes.iter().any(|scope| is_file_scope(*scope)) {
        Some(resolve_file_search_worktree(
            &index_repo_root,
            &repo_search_root,
            selected_worktree_hash,
        )?)
    } else {
        None
    };

    // Phase 70 FR-384 / AS-2: every scope — repo-shared and worktree file
    // scopes alike — goes through ONE versioned `search-multi` request: one
    // runner tree, one model load, one query encode.
    let per_scope_limit = per_scope_limit(effective_scopes.len());
    let worktree_hash_arg = file_worktree.as_ref().map(|worktree| worktree.hash.clone());
    let run_batch = || -> Result<Value, IndexSearchAttemptError> {
        match run_batch_scope_search(
            &repo_search_root,
            repo_hash.as_str(),
            &effective_scopes,
            worktree_hash_arg.as_deref(),
            query,
            per_scope_limit,
            match_mode,
        ) {
            Err(IndexSearchAttemptError::Public(IndexSearchError::NotReady(not_ready))) => {
                let broken = not_ready
                    .affected_scopes
                    .iter()
                    .map(|scope| (scope.clone(), "not-ready".to_string()))
                    .collect::<Vec<_>>();
                queue_scope_rebuilds(project_root, &broken, worktree_hash_arg.as_deref());
                Err(IndexSearchError::NotReady(not_ready).into())
            }
            outcome => outcome,
        }
    };

    let repair_deadline = Duration::from_millis(search_repair_wait_ms());
    let started = std::time::Instant::now();
    let mut payload = run_batch()?;
    let mut broken = broken_scopes(&payload);
    if !broken.is_empty() {
        // FR-388: missing / corrupt scopes never degrade into a silent
        // empty success. FR-097 (T-IDX-417): every caller queues the
        // coordinated repair first — the host-wide coordinator coalesces
        // concurrent admissions into one job. With auto_build the caller
        // then joins that repair and waits up to the deadline; without it
        // (GUI, watcher owns builds) the typed retryable error returns
        // promptly while the repair proceeds in the background.
        queue_scope_rebuilds(project_root, &broken, worktree_hash_arg.as_deref());
        if !auto_build {
            return Err(build_not_ready_error(&broken, 0).into());
        }
        loop {
            let elapsed = started.elapsed();
            if elapsed >= repair_deadline {
                return Err(build_not_ready_error(&broken, elapsed.as_millis() as u64).into());
            }
            let remaining = repair_deadline - elapsed;
            sleep_with_attempt_deadline(remaining.min(Duration::from_secs(1)))?;
            // PR #3301 review: poll repair progress through the model-free
            // status action; the full batch search (one model load) runs
            // only after the broken scopes report healthy again.
            if broken_scopes_still_unhealthy(
                &repo_search_root,
                repo_hash.as_str(),
                &broken,
                worktree_hash_arg.as_deref(),
            )? {
                continue;
            }
            payload = run_batch()?;
            broken = broken_scopes(&payload);
            if broken.is_empty() {
                break;
            }
        }
    }

    // FR-387 stale-while-revalidate: verified results return immediately;
    // one refresh is queued per stale scope (the coordinator coalesces
    // concurrent refreshes host-wide into a single flight).
    let stale_scopes: Vec<String> = payload
        .get("stale_scopes")
        .and_then(Value::as_array)
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let refresh_queued = if stale_scopes.is_empty() {
        false
    } else {
        let stale_pairs: Vec<(String, String)> = stale_scopes
            .iter()
            .map(|scope| (scope.clone(), "stale".to_string()))
            .collect();
        queue_scope_rebuilds(project_root, &stale_pairs, worktree_hash_arg.as_deref());
        true
    };

    let mut results = Vec::new();
    let mut suggestions = Vec::new();
    for scope in &effective_scopes {
        let sub_payload = scope_subpayload(&payload, *scope);
        append_scope_results(&mut results, *scope, sub_payload, &board_scope);
        append_scope_suggestions(&mut suggestions, *scope, sub_payload, &board_scope);
    }

    results.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    suggestions.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    results.truncate(INDEX_SEARCH_LIMIT);
    suggestions.truncate(INDEX_SEARCH_LIMIT);
    Ok(ProjectIndexSearchOutcome {
        results,
        suggestions,
        stale_scopes,
        refresh_queued,
    })
}

/// Default (and env-shortenable) wait for missing / corrupt scope repair
/// before returning `INDEX_NOT_READY` (FR-388: 30 seconds).
fn search_repair_wait_ms() -> u64 {
    std::env::var("GWT_INDEX_SEARCH_REPAIR_WAIT_MS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(30_000)
}

/// Hard attempt budget. Fault-injection tests may shorten it, but environment
/// input can never extend the production 30-second ceiling.
fn search_attempt_deadline_ms() -> u64 {
    std::env::var("GWT_INDEX_SEARCH_RUNNER_DEADLINE_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(SEARCH_ATTEMPT_HARD_LIMIT_MS)
        .min(SEARCH_ATTEMPT_HARD_LIMIT_MS)
}

fn sleep_with_attempt_deadline(duration: Duration) -> Result<(), IndexSearchAttemptError> {
    let sleep_for = match gwt_core::operation_deadline::current() {
        Some(deadline) => {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(search_unavailable_error("project index search timed out"));
            }
            duration.min(remaining)
        }
        None => duration,
    };
    thread::sleep(sleep_for);
    gwt_core::operation_deadline::ensure_remaining("project index repair wait")
        .map_err(|_| search_unavailable_error("project index search timed out"))?;
    Ok(())
}

const SEARCH_RETRY_AFTER_MS: u64 = 5_000;

/// Extract `(scope, state)` pairs whose state blocks searching
/// (missing / corrupt) from the batch payload's `scopes` classification.
fn broken_scopes(payload: &Value) -> Vec<(String, String)> {
    payload
        .get("scopes")
        .and_then(Value::as_object)
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|(scope, status)| {
                    let state = status.get("state").and_then(Value::as_str)?;
                    matches!(state, "missing" | "corrupt")
                        .then(|| (scope.clone(), state.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Light repair-progress probe (PR #3301 review): checks the broken scopes
/// through the model-free `status` action so the repair wait does not pay a
/// model load per poll. Fails closed: any probe error keeps waiting.
fn broken_scopes_still_unhealthy(
    project_root: &Path,
    repo_hash: &str,
    broken: &[(String, String)],
    worktree_hash: Option<&str>,
) -> Result<bool, IndexSearchAttemptError> {
    let mut args = vec![
        gwt_core::runtime::project_index_runner_path().into_os_string(),
        OsString::from("--action"),
        OsString::from("status"),
        OsString::from("--repo-hash"),
        OsString::from(repo_hash),
    ];
    if let Some(hash) = worktree_hash {
        args.extend([OsString::from("--worktree-hash"), OsString::from(hash)]);
    }
    let output = match gwt_core::process_console::spawn_logged_blocking(
        &gwt_core::process_console::ProcessConsoleHub::new(),
        gwt_core::process_console::ProcessKind::IndexRunner,
        crate::index_worker::project_index_python_path(),
        &args,
        gwt_core::process_console::SpawnOptions::new("project index status")
            .current_dir(project_root)
            .forward_output(false),
    ) {
        Ok(output) if output.success() => output,
        Ok(_) => {
            return Err(search_unavailable_error(
                "project index status probe unavailable",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            return Err(search_unavailable_error("project index search timed out"));
        }
        Err(_) => {
            return Err(search_unavailable_error(
                "project index status probe unavailable",
            ));
        }
    };
    let Ok(payload) = serde_json::from_str::<Value>(&output.stdout) else {
        return Ok(true);
    };
    let status = payload.get("status").cloned().unwrap_or(Value::Null);
    Ok(broken.iter().any(|(scope, _)| {
        let ready = status
            .get(scope.as_str())
            .map(|entry| {
                let healthy = entry
                    .get("healthy")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let repair_required = entry
                    .get("repair_required")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                healthy && !repair_required
            })
            .unwrap_or(false);
        !ready
    }))
}

fn build_not_ready_error(broken: &[(String, String)], waited_ms: u64) -> IndexSearchError {
    let reason = broken
        .iter()
        .map(|(scope, state)| format!("{scope} index is {state}"))
        .collect::<Vec<_>>()
        .join("; ");
    IndexSearchError::NotReady(IndexSearchNotReady {
        reason,
        affected_scopes: broken.iter().map(|(scope, _)| scope.clone()).collect(),
        waited_ms,
        retry_after_ms: SEARCH_RETRY_AFTER_MS,
    })
}

fn rebuild_scope_for_name(name: &str) -> Option<crate::index_worker::IndexRebuildScope> {
    use crate::index_worker::IndexRebuildScope;
    Some(match name {
        "issues" => IndexRebuildScope::Issues,
        "specs" => IndexRebuildScope::Specs,
        "memory" => IndexRebuildScope::Memory,
        "discussions" => IndexRebuildScope::Discussions,
        "board" => IndexRebuildScope::Board,
        "works" => IndexRebuildScope::Works,
        "files" => IndexRebuildScope::Files,
        "files-docs" => IndexRebuildScope::FilesDocs,
        _ => return None,
    })
}

/// Queue one coordinated background rebuild per scope. The host-wide
/// coordinator coalesces concurrent requests for the same target, which is
/// what makes the stale refresh single-flight (FR-387) and the repair join
/// shared (FR-382).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RepairKey {
    repo_root: PathBuf,
    scope: String,
    worktree_hash: Option<String>,
}

#[derive(Default)]
struct RepairTracker {
    active: Mutex<HashSet<RepairKey>>,
    settled: Condvar,
}

static REPAIR_TRACKER: OnceLock<RepairTracker> = OnceLock::new();

fn repair_tracker() -> &'static RepairTracker {
    REPAIR_TRACKER.get_or_init(RepairTracker::default)
}

struct RepairLease {
    key: Option<RepairKey>,
}

impl RepairLease {
    fn admit(key: RepairKey) -> Option<Self> {
        let tracker = repair_tracker();
        let mut active = tracker
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !active.insert(key.clone()) {
            return None;
        }
        Some(Self { key: Some(key) })
    }
}

impl Drop for RepairLease {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let tracker = repair_tracker();
        let mut active = tracker
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        active.remove(&key);
        tracker.settled.notify_all();
    }
}

type RepairTask = Box<dyn FnOnce() + Send + 'static>;

fn spawn_tracked_repair_with(
    key: RepairKey,
    job: impl FnOnce() + Send + 'static,
    spawn: impl FnOnce(RepairTask) -> std::io::Result<()>,
) -> bool {
    let Some(lease) = RepairLease::admit(key) else {
        return false;
    };
    // Thread-local operation deadlines do not cross `std::thread::spawn`.
    // Capture the absolute expiry before handing the task to the spawner so
    // queueing delay cannot grant a fresh budget. Repairs queued outside a
    // search still receive the same finite hard ceiling.
    let deadline = gwt_core::operation_deadline::current()
        .unwrap_or_else(|| Instant::now() + Duration::from_millis(search_attempt_deadline_ms()));
    let task: RepairTask = Box::new(move || {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(deadline);
        let _lease = lease;
        job();
    });
    // If spawning fails, `spawn` drops `task`; the captured lease rolls the
    // admission back and wakes bounded fixture waits.
    spawn(task).is_ok()
}

/// Wait until all search-triggered repair tasks have released their tracked
/// leases. Test fixtures use this before restoring scoped environment values.
#[cfg(test)]
pub(crate) fn wait_for_index_search_repairs(timeout: Duration) -> bool {
    let tracker = repair_tracker();
    let active = tracker
        .active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active.is_empty() {
        return true;
    }
    let (active, _) = tracker
        .settled
        .wait_timeout_while(active, timeout, |active| !active.is_empty())
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    active.is_empty()
}

fn queue_scope_rebuilds(
    project_root: &Path,
    scopes: &[(String, String)],
    worktree_hash: Option<&str>,
) {
    let repair_repo_root = crate::index_worker::resolve_project_index_repo_root(project_root)
        .unwrap_or_else(|| project_root.to_path_buf());
    let repair_repo_root = dunce::canonicalize(&repair_repo_root).unwrap_or(repair_repo_root);
    for (scope_name, _) in scopes {
        let Some(rebuild_scope) = rebuild_scope_for_name(scope_name) else {
            continue;
        };
        let project_root = project_root.to_path_buf();
        let worktree = rebuild_scope
            .requires_worktree_hash()
            .then(|| worktree_hash.map(str::to_string))
            .flatten();
        let scope_label = scope_name.clone();
        let key = RepairKey {
            repo_root: repair_repo_root.clone(),
            scope: scope_label.clone(),
            worktree_hash: worktree.clone(),
        };
        let _ = spawn_tracked_repair_with(
            key,
            move || {
                if let Err(error) = crate::index_worker::default_rebuild_runner(
                    &project_root,
                    rebuild_scope,
                    worktree.as_deref(),
                ) {
                    let _ = error;
                    tracing::debug!(
                        target: "gwt::index",
                        scope = %scope_label,
                        "search-triggered index repair failed"
                    );
                }
            },
            |task| {
                std::thread::Builder::new()
                    .name("gwt-index-search-repair".to_string())
                    .spawn(task)
                    .map(|_| ())
            },
        );
    }
}

/// Per-scope sub-payload of a batch response; falls back to the merged
/// legacy top-level keys for older runner payloads (FR-398 compatibility).
fn scope_subpayload(payload: &Value, scope: IndexSearchScope) -> &Value {
    payload
        .get("scope_results")
        .and_then(|scopes| scopes.get(scope.as_str()))
        .unwrap_or(payload)
}

fn is_file_scope(scope: IndexSearchScope) -> bool {
    matches!(scope, IndexSearchScope::Files | IndexSearchScope::FilesDocs)
}

/// Curated scopes consulted by the Start Work duplicate-work advisory
/// (SPEC-2359 US-80): past Work (`works`) plus the durable owners a prior
/// effort would have been anchored to.
const WORK_ADVISORY_SCOPES: &[IndexSearchScope] = &[
    IndexSearchScope::Works,
    IndexSearchScope::Issues,
    IndexSearchScope::Specs,
    IndexSearchScope::Board,
];

/// Maximum semantic distance for an advisory hit to count as a "strong match".
/// Beyond this, hits are dropped so Start Work stays quiet instead of always
/// claiming "related work" (alarm-fatigue guard, SPEC-2359 FR-414).
pub const WORK_ADVISORY_DISTANCE_THRESHOLD: f64 = 0.25;

/// Maximum advisory hits surfaced at Start Work.
const WORK_ADVISORY_LIMIT: usize = 5;

/// Keep only strong-match advisory hits: a present distance within `threshold`.
/// Returns them nearest-first, capped at `limit`. An empty result means
/// "no strong match" — the advisory panel stays empty (SPEC-2359 AS-2).
pub fn filter_strong_advisory_matches(
    mut results: Vec<IndexSearchResult>,
    threshold: f64,
    limit: usize,
) -> Vec<IndexSearchResult> {
    results.retain(|item| item.distance.is_some_and(|distance| distance <= threshold));
    results.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    results.truncate(limit);
    results
}

/// Run the Start Work duplicate-work advisory (SPEC-2359 US-80): semantic search
/// across past Work and the durable owners, keeping only strong matches. Never
/// blocks Start Work; an error or empty corpus yields an empty advisory.
///
/// Uses `auto_build = true` so the advisory self-heals the `works` index on
/// first use: unlike the long-lived `issues` / `specs` / `board` scopes, the
/// `works` scope is not (yet) maintained by the index watcher, so in a freshly
/// upgraded project it would not exist and the advisory would always come back
/// empty until the user manually ran a works search. Self-healing backfills past
/// Work from `work_items.json` on first advisory. This runs on a background
/// task with a visible loading indicator, so a one-time inline build is
/// acceptable here even though the interactive search window uses `false`.
pub fn work_advisory(project_root: &Path, query: &str) -> Result<Vec<IndexSearchResult>, String> {
    // Try the full curated set first. With auto_build the per-scope actions
    // hard-fail on an empty corpus (e.g. an issue cache that was never synced
    // for this repo), and a single peripheral failure would otherwise blank the
    // whole advisory. Fall back to past Work alone — the scope that actually
    // matters for duplicate-work detection — so a broken issues/specs/board
    // source never hides similar prior Work.
    let outcome = match search_project_index(
        project_root,
        query,
        WORK_ADVISORY_SCOPES,
        None,
        IndexSearchMatchMode::Semantic,
        true,
    ) {
        Ok(outcome) => outcome,
        Err(_) => search_project_index(
            project_root,
            query,
            &[IndexSearchScope::Works],
            None,
            IndexSearchMatchMode::Semantic,
            true,
        )
        .map_err(|error| error.to_string())?,
    };
    Ok(filter_strong_advisory_matches(
        outcome.results,
        WORK_ADVISORY_DISTANCE_THRESHOLD,
        WORK_ADVISORY_LIMIT,
    ))
}

fn per_scope_limit(scope_count: usize) -> usize {
    if scope_count <= 1 {
        INDEX_SEARCH_LIMIT
    } else {
        INDEX_SEARCH_LIMIT.div_ceil(scope_count).max(12)
    }
}

fn default_index_search_scopes() -> Vec<IndexSearchScope> {
    vec![
        IndexSearchScope::Issues,
        IndexSearchScope::Specs,
        IndexSearchScope::Memory,
        IndexSearchScope::Discussions,
        IndexSearchScope::Board,
        IndexSearchScope::Works,
        IndexSearchScope::Files,
        IndexSearchScope::FilesDocs,
    ]
}

struct FileSearchWorktree {
    hash: String,
}

fn resolve_file_search_worktree(
    index_repo_root: &Path,
    default_worktree_root: &Path,
    selected_worktree_hash: Option<&str>,
) -> Result<FileSearchWorktree, String> {
    if let Some(hash) = selected_worktree_hash
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
    {
        let entries =
            worktree_inventory::enumerate_worktrees(index_repo_root, Some(default_worktree_root))
                .map_err(|error| error.to_string())?;
        let entry = entries
            .into_iter()
            .find(|entry| entry.id == hash)
            .ok_or_else(|| format!("worktree with hash {hash} not found"))?;
        if matches!(entry.kind, worktree_inventory::WorktreeEntryKind::BareMain) {
            return Err("file search requires a non-bare worktree".to_string());
        }
        return Ok(FileSearchWorktree {
            hash: hash.to_string(),
        });
    }
    let worktree_root = default_worktree_root;
    if worktree_root == index_repo_root {
        let entries = worktree_inventory::enumerate_worktrees(index_repo_root, None)
            .map_err(|error| error.to_string())?;
        if let Some(entry) = entries
            .into_iter()
            .find(|entry| matches!(entry.kind, worktree_inventory::WorktreeEntryKind::Workspace))
        {
            let hash = gwt_core::worktree_hash::compute_worktree_hash(&entry.path)
                .map_err(|error| error.to_string())?
                .to_string();
            return Ok(FileSearchWorktree { hash });
        }
    }
    let hash = gwt_core::worktree_hash::compute_worktree_hash(worktree_root)
        .map_err(|error| error.to_string())?
        .to_string();
    Ok(FileSearchWorktree { hash })
}

fn search_unavailable_error(reason: impl Into<String>) -> IndexSearchAttemptError {
    IndexSearchAttemptError::Unavailable(IndexSearchUnavailable {
        reason: sanitize_runner_diagnostic(&reason.into()),
        retry_after_ms: SEARCH_RETRY_AFTER_MS,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_batch_scope_search(
    project_root: &Path,
    repo_hash: &str,
    scopes: &[IndexSearchScope],
    worktree_hash: Option<&str>,
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
) -> Result<Value, IndexSearchAttemptError> {
    let args = batch_scope_search_command_args(
        project_root,
        repo_hash,
        scopes,
        worktree_hash,
        query,
        limit,
        match_mode,
    );
    // FR-103 (T-IDX-419): the interactive semantic attempt runs through the
    // shared process lifecycle boundary — captured output without terminal
    // forwarding, one hard deadline, and full process-tree termination and
    // reaping on expiry. Spawn failure and deadline expiry are retryable
    // SEARCH_UNAVAILABLE, never a raw error string.
    let output = gwt_core::process_console::spawn_logged_blocking(
        &gwt_core::process_console::ProcessConsoleHub::new(),
        gwt_core::process_console::ProcessKind::IndexRunner,
        crate::index_worker::project_index_python_path(),
        &args,
        gwt_core::process_console::SpawnOptions::new("project index search-multi")
            .current_dir(project_root)
            .forward_output(false),
    )
    .map_err(|_| search_unavailable_error("project index runner unavailable"))?;
    if !output.success() {
        return Err(classify_failed_runner_output(&output));
    }
    classify_successful_runner_output(&output)
}

fn classify_successful_runner_output(
    output: &gwt_core::process_console::SpawnOutput,
) -> Result<Value, IndexSearchAttemptError> {
    let payload = parse_runner_payload(output.stdout.as_bytes()).map_err(|_| {
        IndexSearchAttemptError::Public(IndexSearchError::Other(
            "malformed project index search response".to_string(),
        ))
    })?;
    if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(runner_payload_error(&payload));
    }
    const ERROR_ONLY_FIELDS: &[&str] = &[
        "error",
        "error_code",
        "reason",
        "retryable",
        "affected_scopes",
        "waited_ms",
        "retry_after_ms",
    ];
    if ERROR_ONLY_FIELDS
        .iter()
        .any(|field| payload.get(*field).is_some())
    {
        return Err(IndexSearchError::Other(
            "contradictory project index search response".to_string(),
        )
        .into());
    }
    Ok(payload)
}

/// Classify a non-zero runner exit (T-IDX-419): a structured stdout
/// diagnostic wins over stderr progress noise; an unstructured termination
/// is a retryable `SEARCH_UNAVAILABLE`.
fn classify_failed_runner_output(
    output: &gwt_core::process_console::SpawnOutput,
) -> IndexSearchAttemptError {
    if let Ok(payload) = parse_runner_payload(output.stdout.as_bytes()) {
        if payload.get("ok").is_some()
            || payload.get("error").is_some()
            || payload.get("error_code").is_some()
        {
            return runner_payload_error(&payload);
        }
    }
    let stderr = output.stderr.trim();
    let detail = if stderr.is_empty() {
        output.stdout.trim()
    } else {
        stderr
    };
    let exit = output
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "?".to_string());
    let detail = sanitize_runner_diagnostic(detail);
    search_unavailable_error(if detail.is_empty() {
        format!("project index runner exited with {exit}")
    } else {
        format!("project index runner exited with {exit}: {detail}")
    })
}

/// One versioned `search-multi` request covering every scope (FR-384):
/// interactive QoS thread caps, worktree hash for file scopes, no inline
/// auto-build (the Rust caller owns repair through the coordinator).
fn batch_scope_search_command_args(
    project_root: &Path,
    repo_hash: &str,
    scopes: &[IndexSearchScope],
    worktree_hash: Option<&str>,
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
) -> Vec<OsString> {
    let mut args = vec![
        gwt_core::runtime::project_index_runner_path().into_os_string(),
        OsString::from("--action"),
        OsString::from("search-multi"),
        OsString::from("--repo-hash"),
        OsString::from(repo_hash),
        OsString::from("--project-root"),
        project_root.as_os_str().to_os_string(),
        OsString::from("--query"),
        OsString::from(query),
        OsString::from("--n-results"),
        OsString::from(limit.to_string()),
        OsString::from("--match-mode"),
        OsString::from(match_mode.as_str()),
        OsString::from("--qos"),
        OsString::from("interactive"),
        OsString::from("--scopes"),
        OsString::from(
            scopes
                .iter()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
    ];
    if let Some(hash) = worktree_hash {
        args.push(OsString::from("--worktree-hash"));
        args.push(OsString::from(hash));
    }
    args
}

fn append_scope_results(
    out: &mut Vec<IndexSearchResult>,
    scope: IndexSearchScope,
    payload: &Value,
    board_scope: &gwt_core::coordination::BoardAudienceScope,
) {
    let key = match scope {
        IndexSearchScope::Issues => "issueResults",
        IndexSearchScope::Specs => "specResults",
        IndexSearchScope::Memory => "memoryResults",
        IndexSearchScope::Discussions => "discussionResults",
        IndexSearchScope::Board => "boardResults",
        IndexSearchScope::Works => "workResults",
        IndexSearchScope::Files | IndexSearchScope::FilesDocs => "results",
    };
    let Some(items) = payload.get(key).and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let result = match scope {
            IndexSearchScope::Issues => issue_result(item),
            IndexSearchScope::Specs => spec_result(item),
            IndexSearchScope::Memory => memory_result(item),
            IndexSearchScope::Discussions => discussion_result(item),
            IndexSearchScope::Board => board_result(item, board_scope),
            IndexSearchScope::Works => work_result(item),
            IndexSearchScope::Files | IndexSearchScope::FilesDocs => file_result(scope, item),
        };
        if let Some(result) = result {
            out.push(result);
        }
    }
}

fn append_scope_suggestions(
    out: &mut Vec<IndexSearchResult>,
    scope: IndexSearchScope,
    payload: &Value,
    board_scope: &gwt_core::coordination::BoardAudienceScope,
) {
    let Some(suggestions) = payload.get("suggestions") else {
        return;
    };
    let items = suggestions
        .get(scope.as_str())
        .or_else(|| suggestions.as_array().map(|_| suggestions))
        .and_then(Value::as_array);
    let Some(items) = items else {
        return;
    };
    for item in items {
        let result = match scope {
            IndexSearchScope::Issues => issue_result(item),
            IndexSearchScope::Specs => spec_result(item),
            IndexSearchScope::Memory => memory_result(item),
            IndexSearchScope::Discussions => discussion_result(item),
            IndexSearchScope::Board => board_result(item, board_scope),
            IndexSearchScope::Works => work_result(item),
            IndexSearchScope::Files | IndexSearchScope::FilesDocs => file_result(scope, item),
        };
        if let Some(result) = result {
            out.push(result);
        }
    }
}

fn issue_result(item: &Value) -> Option<IndexSearchResult> {
    let number = value_u64(item.get("number")?)?;
    let title = value_str(item.get("title")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Issues,
        title: format!("#{number} {title}"),
        subtitle: value_str(item.get("state")).unwrap_or_else(|| "issue".to_string()),
        preview: labels_preview(item),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Issue { number },
    })
}

fn spec_result(item: &Value) -> Option<IndexSearchResult> {
    let spec_id = value_u64(item.get("spec_id")?)?;
    let title = value_str(item.get("title")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Specs,
        title: format!("SPEC #{spec_id} {title}"),
        subtitle: value_str(item.get("phase"))
            .filter(|phase| !phase.is_empty())
            .unwrap_or_else(|| "spec".to_string()),
        preview: value_str(item.get("matched_section")).unwrap_or_default(),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Spec { spec_id },
    })
}

fn memory_result(item: &Value) -> Option<IndexSearchResult> {
    let heading = value_str(item.get("heading"))?;
    let title = value_str(item.get("title")).unwrap_or_else(|| heading.clone());
    let date = value_str(item.get("date")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Memory,
        title,
        subtitle: if date.is_empty() {
            "memory".to_string()
        } else {
            format!("memory · {date}")
        },
        preview: heading.clone(),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Memory { heading, date },
    })
}

fn work_result(item: &Value) -> Option<IndexSearchResult> {
    let work_id = value_str(item.get("work_id"))?;
    let title = value_str(item.get("title")).unwrap_or_else(|| work_id.clone());
    let status = value_str(item.get("status")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Works,
        title,
        subtitle: if status.is_empty() {
            "work".to_string()
        } else {
            format!("work · {status}")
        },
        preview: value_str(item.get("intent")).unwrap_or_default(),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Work { work_id },
    })
}

fn discussion_result(item: &Value) -> Option<IndexSearchResult> {
    let heading = value_str(item.get("heading"))?;
    let title = value_str(item.get("title")).unwrap_or_else(|| heading.clone());
    let date = value_str(item.get("date")).unwrap_or_default();
    let status = value_str(item.get("status")).unwrap_or_else(|| "discussion".to_string());
    Some(IndexSearchResult {
        scope: IndexSearchScope::Discussions,
        title,
        subtitle: if date.is_empty() {
            status
        } else {
            format!("{status} · {date}")
        },
        preview: heading.clone(),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Discussion { heading, date },
    })
}

fn board_result(
    item: &Value,
    scope: &gwt_core::coordination::BoardAudienceScope,
) -> Option<IndexSearchResult> {
    if !board_item_visible_for_scope(item, scope) {
        return None;
    }
    let entry_id = value_str(item.get("entry_id"))?;
    let title = value_str(item.get("title_summary"))
        .filter(|value| !value.is_empty())
        .or_else(|| value_str(item.get("body_preview")))
        .unwrap_or_else(|| "Board entry".to_string());
    let kind = value_str(item.get("kind")).unwrap_or_else(|| "board".to_string());
    let author = value_str(item.get("author")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Board,
        title,
        subtitle: if author.is_empty() {
            kind
        } else {
            format!("{kind} · {author}")
        },
        preview: value_str(item.get("body_preview")).unwrap_or_default(),
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::Board { entry_id },
    })
}

fn file_result(scope: IndexSearchScope, item: &Value) -> Option<IndexSearchResult> {
    let path = value_str(item.get("path"))?;
    let description = value_str(item.get("description")).unwrap_or_default();
    let file_type = value_str(item.get("fileType")).unwrap_or_default();
    Some(IndexSearchResult {
        scope,
        title: path.clone(),
        subtitle: if file_type.is_empty() {
            scope.as_str().to_string()
        } else {
            file_type
        },
        preview: description,
        distance: item.get("distance").and_then(Value::as_f64),
        match_mode: item_match_mode(item),
        matched_terms: value_string_array(item.get("matched_terms")),
        missing_terms: value_string_array(item.get("missing_terms")),
        target: IndexSearchTarget::File { path },
    })
}

fn item_match_mode(item: &Value) -> Option<IndexSearchMatchMode> {
    match item.get("match_mode").and_then(Value::as_str) {
        Some("all_terms") => Some(IndexSearchMatchMode::AllTerms),
        Some("semantic") => Some(IndexSearchMatchMode::Semantic),
        _ => None,
    }
}

fn value_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value_str(Some(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn board_item_visible_for_scope(
    item: &Value,
    scope: &gwt_core::coordination::BoardAudienceScope,
) -> bool {
    let audience: Vec<String> = item
        .get("audience")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value_str(Some(value)))
                .collect()
        })
        .unwrap_or_default();
    match scope {
        gwt_core::coordination::BoardAudienceScope::All => true,
        gwt_core::coordination::BoardAudienceScope::Broadcast => audience.is_empty(),
        gwt_core::coordination::BoardAudienceScope::Workspace(workspace_id) => {
            audience.is_empty() || audience.iter().any(|value| value == workspace_id)
        }
    }
}

fn labels_preview(item: &Value) -> String {
    item.get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|value| value_str(Some(value)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn value_str(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
}

fn distance_key(result: &IndexSearchResult) -> f64 {
    result.distance.unwrap_or(f64::INFINITY)
}

fn payload_error(payload: &Value) -> String {
    sanitize_runner_diagnostic(
        payload
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| payload.get("reason").and_then(Value::as_str))
            .unwrap_or("project index search failed"),
    )
}

fn runner_payload_error(payload: &Value) -> IndexSearchAttemptError {
    let malformed = || IndexSearchError::Other(payload_error(payload)).into();
    if payload.get("ok").and_then(Value::as_bool) != Some(false) {
        return malformed();
    }
    match payload.get("error_code").and_then(Value::as_str) {
        Some("INDEX_NOT_READY") => {
            let Some(reason) = typed_payload_reason(payload) else {
                return malformed();
            };
            let Some(affected_scopes) = typed_affected_scopes(payload) else {
                return malformed();
            };
            let Some(waited_ms) = payload.get("waited_ms").and_then(Value::as_u64) else {
                return malformed();
            };
            let Some(retry_after_ms) = typed_positive_retry_after(payload) else {
                return malformed();
            };
            if payload.get("retryable").and_then(Value::as_bool) != Some(true) {
                return malformed();
            }
            IndexSearchError::NotReady(IndexSearchNotReady {
                reason,
                affected_scopes,
                waited_ms,
                retry_after_ms,
            })
            .into()
        }
        Some("SEARCH_FAILED") => {
            let Some(reason) = typed_payload_reason(payload) else {
                return malformed();
            };
            let Some(affected_scopes) = typed_affected_scopes(payload) else {
                return malformed();
            };
            if payload.get("retryable").and_then(Value::as_bool) != Some(false) {
                return malformed();
            }
            IndexSearchError::SearchFailed(IndexSearchFailed {
                reason,
                affected_scopes,
            })
            .into()
        }
        Some("SEARCH_UNAVAILABLE") => {
            let Some(reason) = typed_payload_reason(payload) else {
                return malformed();
            };
            let Some(retry_after_ms) = typed_positive_retry_after(payload) else {
                return malformed();
            };
            if payload.get("retryable").and_then(Value::as_bool) != Some(true)
                || (payload.get("affected_scopes").is_some()
                    && typed_affected_scopes(payload).is_none())
            {
                return malformed();
            }
            IndexSearchAttemptError::Unavailable(IndexSearchUnavailable {
                reason,
                retry_after_ms,
            })
        }
        // Unknown, absent, malformed, and legacy structured diagnostics are
        // intentionally non-retryable. Only the exact three codes above
        // participate in the typed retry protocol.
        _ => IndexSearchError::Other(payload_error(payload)).into(),
    }
}

fn typed_payload_reason(payload: &Value) -> Option<String> {
    let raw = match (payload.get("reason"), payload.get("error")) {
        (Some(Value::String(reason)), None) | (None, Some(Value::String(reason))) => reason,
        _ => return None,
    };
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > RUNNER_DIAGNOSTIC_MAX_BYTES {
        return None;
    }
    let sanitized = sanitize_runner_diagnostic(raw);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

fn typed_affected_scopes(payload: &Value) -> Option<Vec<String>> {
    let values = payload.get("affected_scopes")?.as_array()?;
    if values.is_empty() {
        return None;
    }
    let mut scopes = Vec::with_capacity(values.len());
    for value in values {
        let scope = value.as_str()?.trim();
        if scope.is_empty()
            || scope.len() > 64
            || rebuild_scope_for_name(scope).is_none()
            || scopes.iter().any(|existing| existing == scope)
        {
            return None;
        }
        scopes.push(scope.to_string());
    }
    Some(scopes)
}

fn typed_positive_retry_after(payload: &Value) -> Option<u64> {
    payload
        .get("retry_after_ms")
        .and_then(Value::as_u64)
        .filter(|retry_after_ms| *retry_after_ms > 0)
}

fn sanitize_runner_diagnostic(raw: &str) -> String {
    let stripped = gwt_core::process_console::strip_ansi(raw);
    let redacted = gwt_core::process_console::redact_line(&stripped);
    let mut end = redacted.len().min(RUNNER_DIAGNOSTIC_MAX_BYTES);
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    redacted[..end].to_string()
}

fn parse_runner_payload(stdout: &[u8]) -> Result<Value, String> {
    match serde_json::from_slice(stdout) {
        Ok(payload) => Ok(payload),
        Err(full_error) => {
            let text = String::from_utf8_lossy(stdout);
            for line in text.lines().rev() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(payload) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if payload.get("ok").is_some()
                    || payload.get("error").is_some()
                    || payload.get("error_code").is_some()
                {
                    return Ok(payload);
                }
            }
            Err(format!("parse project index search result: {full_error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::coordination::BoardAudienceScope;
    use serde_json::json;
    use std::path::PathBuf;

    fn run_git_at(path: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?}: {err}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn make_bare_workspace_with_worktree(home: &Path) -> PathBuf {
        let bare = home.join("gwt.git");
        let bootstrap = home.join(".bootstrap");
        let develop = home.join("develop");
        std::fs::create_dir_all(home).expect("workspace home");
        run_git_at(home, &["init", "--bare", bare.to_str().unwrap()]);
        run_git_at(
            &bare,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/gwt.git",
            ],
        );
        run_git_at(home, &["clone", bare.to_str().unwrap(), ".bootstrap"]);
        run_git_at(&bootstrap, &["config", "user.email", "test@example.com"]);
        run_git_at(&bootstrap, &["config", "user.name", "Test User"]);
        run_git_at(&bootstrap, &["checkout", "-b", "develop"]);
        run_git_at(&bootstrap, &["commit", "--allow-empty", "-m", "init"]);
        run_git_at(&bootstrap, &["push", "origin", "develop"]);
        run_git_at(
            &bare,
            &["worktree", "add", develop.to_str().unwrap(), "develop"],
        );
        std::fs::remove_dir_all(&bootstrap).expect("remove bootstrap");
        develop
    }

    fn canonical(path: &Path) -> PathBuf {
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    #[test]
    fn empty_index_search_query_returns_no_results_without_runtime() {
        let outcome = search_project_index(
            Path::new("/definitely/not/a/repo"),
            "   ",
            &[],
            None,
            IndexSearchMatchMode::Semantic,
            false,
        )
        .expect("empty query should short-circuit");

        assert!(outcome.results.is_empty());
        assert!(outcome.suggestions.is_empty());
    }

    #[test]
    fn default_index_search_scopes_cover_all_user_visible_sources() {
        assert_eq!(
            default_index_search_scopes(),
            vec![
                IndexSearchScope::Issues,
                IndexSearchScope::Specs,
                IndexSearchScope::Memory,
                IndexSearchScope::Discussions,
                IndexSearchScope::Board,
                IndexSearchScope::Works,
                IndexSearchScope::Files,
                IndexSearchScope::FilesDocs,
            ]
        );
    }

    fn advisory_item(
        scope: IndexSearchScope,
        title: &str,
        distance: Option<f64>,
    ) -> IndexSearchResult {
        IndexSearchResult {
            scope,
            title: title.to_string(),
            subtitle: String::new(),
            preview: String::new(),
            distance,
            match_mode: None,
            matched_terms: Vec::new(),
            missing_terms: Vec::new(),
            target: IndexSearchTarget::Work {
                work_id: title.to_string(),
            },
        }
    }

    #[test]
    fn advisory_keeps_only_strong_matches_sorted_nearest_first() {
        // SPEC-2359 FR-414 / AS-1: strong matches survive, weak ones drop, and
        // hits arrive nearest-first.
        let input = vec![
            advisory_item(IndexSearchScope::Works, "far", Some(0.40)),
            advisory_item(IndexSearchScope::Works, "near", Some(0.05)),
            advisory_item(IndexSearchScope::Issues, "mid", Some(0.20)),
            advisory_item(IndexSearchScope::Works, "no-distance", None),
        ];
        let out = filter_strong_advisory_matches(input, 0.25, 5);
        let titles: Vec<_> = out.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, vec!["near", "mid"]);
    }

    #[test]
    fn advisory_is_empty_when_no_strong_match() {
        // SPEC-2359 AS-2: nothing within threshold => quiet (empty) advisory.
        let input = vec![
            advisory_item(IndexSearchScope::Issues, "weak-a", Some(0.6)),
            advisory_item(IndexSearchScope::Specs, "weak-b", Some(0.9)),
        ];
        assert!(filter_strong_advisory_matches(input, 0.25, 5).is_empty());
    }

    #[test]
    fn advisory_caps_at_limit() {
        let input: Vec<_> = (1..=10)
            .map(|i| {
                advisory_item(
                    IndexSearchScope::Works,
                    &i.to_string(),
                    Some(0.01 * f64::from(i)),
                )
            })
            .collect();
        assert_eq!(filter_strong_advisory_matches(input, 1.0, 3).len(), 3);
    }

    #[test]
    fn append_scope_results_formats_work_target() {
        // SPEC-2359 US-80: a `works` scope result must locate a prior Work by
        // work_id and surface its title/intent/status for the advisory panel.
        let mut results = Vec::new();
        let board_scope = BoardAudienceScope::All;
        append_scope_results(
            &mut results,
            IndexSearchScope::Works,
            &json!({
                "workResults": [{
                    "work_id": "work-feature-auth-abc123",
                    "title": "ログイン認証のバグ修正",
                    "intent": "login auth bug",
                    "status": "done",
                    "distance": 0.07,
                }]
            }),
            &board_scope,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].scope, IndexSearchScope::Works);
        assert_eq!(results[0].title, "ログイン認証のバグ修正");
        assert_eq!(results[0].subtitle, "work · done");
        assert_eq!(results[0].preview, "login auth bug");
        assert!(matches!(
            results[0].target,
            IndexSearchTarget::Work { ref work_id } if work_id == "work-feature-auth-abc123"
        ));
    }

    #[test]
    fn work_result_without_work_id_is_dropped() {
        let mut results = Vec::new();
        let board_scope = BoardAudienceScope::All;
        append_scope_results(
            &mut results,
            IndexSearchScope::Works,
            &json!({ "workResults": [{ "title": "no id" }] }),
            &board_scope,
        );
        assert!(results.is_empty());
    }

    #[test]
    fn append_scope_results_formats_issue_spec_memory_discussion_and_file_targets() {
        let mut results = Vec::new();
        let board_scope = BoardAudienceScope::All;

        append_scope_results(
            &mut results,
            IndexSearchScope::Issues,
            &json!({
                "issueResults": [{
                    "number": "42",
                    "title": "Search index",
                    "state": "open",
                    "labels": ["enhancement", "index"],
                    "distance": 0.4
                }]
            }),
            &board_scope,
        );
        append_scope_results(
            &mut results,
            IndexSearchScope::Specs,
            &json!({
                "specResults": [{
                    "spec_id": 1939,
                    "title": "Semantic search",
                    "phase": "Phase 15",
                    "matched_section": "Dedicated Index window",
                    "distance": 0.2
                }]
            }),
            &board_scope,
        );
        append_scope_results(
            &mut results,
            IndexSearchScope::Memory,
            &json!({
                "memoryResults": [{
                    "heading": "Always verify index routes",
                    "title": "Index verification",
                    "date": "2026-05-20",
                    "distance": 0.3
                }]
            }),
            &board_scope,
        );
        append_scope_results(
            &mut results,
            IndexSearchScope::Discussions,
            &json!({
                "discussionResults": [{
                    "heading": "## 2026-05-22 — Workspace terminology",
                    "title": "Workspace terminology",
                    "date": "2026-05-22",
                    "status": "active",
                    "distance": 0.25
                }]
            }),
            &board_scope,
        );
        append_scope_results(
            &mut results,
            IndexSearchScope::FilesDocs,
            &json!({
                "results": [{
                    "path": "README.md",
                    "description": "Index usage docs",
                    "fileType": "Markdown",
                    "distance": 0.1
                }]
            }),
            &board_scope,
        );

        assert_eq!(results.len(), 5);
        assert_eq!(results[0].title, "#42 Search index");
        assert_eq!(results[0].preview, "enhancement, index");
        assert!(matches!(
            results[0].target,
            IndexSearchTarget::Issue { number: 42 }
        ));
        assert_eq!(results[1].title, "SPEC #1939 Semantic search");
        assert_eq!(results[1].preview, "Dedicated Index window");
        assert!(matches!(
            results[1].target,
            IndexSearchTarget::Spec { spec_id: 1939 }
        ));
        assert_eq!(results[2].subtitle, "memory · 2026-05-20");
        assert!(matches!(
            results[2].target,
            IndexSearchTarget::Memory { .. }
        ));
        assert_eq!(results[3].subtitle, "active · 2026-05-22");
        assert!(matches!(
            results[3].target,
            IndexSearchTarget::Discussion { .. }
        ));
        assert_eq!(results[4].title, "README.md");
        assert_eq!(results[4].subtitle, "Markdown");
        assert!(matches!(results[4].target, IndexSearchTarget::File { .. }));
    }

    #[test]
    fn append_scope_results_filters_board_entries_to_workspace_audience() {
        let mut results = Vec::new();
        let board_scope = BoardAudienceScope::Workspace("workspace-a".to_string());

        append_scope_results(
            &mut results,
            IndexSearchScope::Board,
            &json!({
                "boardResults": [
                    {
                        "entry_id": "broadcast",
                        "kind": "status",
                        "author": "Codex",
                        "title_summary": "Broadcast entry",
                        "body_preview": "Visible to everyone",
                        "audience": [],
                        "distance": 0.2
                    },
                    {
                        "entry_id": "workspace-a",
                        "kind": "decision",
                        "author": "Claude Code",
                        "title_summary": "",
                        "body_preview": "Visible to workspace A",
                        "audience": ["workspace-a"],
                        "distance": 0.1
                    },
                    {
                        "entry_id": "workspace-b",
                        "kind": "status",
                        "author": "Codex",
                        "title_summary": "Hidden entry",
                        "body_preview": "Visible to workspace B",
                        "audience": ["workspace-b"],
                        "distance": 0.3
                    }
                ]
            }),
            &board_scope,
        );

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Broadcast entry");
        assert_eq!(results[0].subtitle, "status · Codex");
        assert!(matches!(
            results[0].target,
            IndexSearchTarget::Board { ref entry_id } if entry_id == "broadcast"
        ));
        assert_eq!(results[1].title, "Visible to workspace A");
        assert_eq!(results[1].subtitle, "decision · Claude Code");
    }

    #[test]
    fn append_scope_suggestions_preserves_match_evidence() {
        let mut suggestions = Vec::new();
        let board_scope = BoardAudienceScope::All;

        append_scope_suggestions(
            &mut suggestions,
            IndexSearchScope::Issues,
            &json!({
                "suggestions": {
                    "issues": [{
                        "number": 77,
                        "title": "Workspace only",
                        "state": "open",
                        "labels": ["index"],
                        "distance": 0.35,
                        "match_mode": "all_terms",
                        "matched_terms": ["Workspace"],
                        "missing_terms": ["置き換え"]
                    }]
                }
            }),
            &board_scope,
        );

        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].title, "#77 Workspace only");
        assert_eq!(
            suggestions[0].match_mode,
            Some(IndexSearchMatchMode::AllTerms)
        );
        assert_eq!(suggestions[0].matched_terms, vec!["Workspace"]);
        assert_eq!(suggestions[0].missing_terms, vec!["置き換え"]);
    }

    #[test]
    fn board_visibility_supports_all_broadcast_and_workspace_modes() {
        let broadcast = json!({ "audience": [] });
        let workspace = json!({ "audience": ["workspace-a"] });

        assert!(board_item_visible_for_scope(
            &workspace,
            &BoardAudienceScope::All
        ));
        assert!(board_item_visible_for_scope(
            &broadcast,
            &BoardAudienceScope::Broadcast
        ));
        assert!(!board_item_visible_for_scope(
            &workspace,
            &BoardAudienceScope::Broadcast
        ));
        assert!(board_item_visible_for_scope(
            &broadcast,
            &BoardAudienceScope::Workspace("workspace-a".to_string())
        ));
        assert!(board_item_visible_for_scope(
            &workspace,
            &BoardAudienceScope::Workspace("workspace-a".to_string())
        ));
        assert!(!board_item_visible_for_scope(
            &workspace,
            &BoardAudienceScope::Workspace("workspace-b".to_string())
        ));
    }

    #[test]
    fn file_search_default_worktree_uses_workspace_entry_for_workspace_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let develop = make_bare_workspace_with_worktree(temp.path());
        let index_repo_root = crate::index_worker::resolve_project_index_repo_root(temp.path())
            .expect("index repo root");
        let default_worktree_root =
            crate::index_worker::default_project_index_worktree_root(temp.path())
                .expect("default worktree root");

        let resolved = resolve_file_search_worktree(&index_repo_root, &default_worktree_root, None)
            .expect("file search worktree");

        // The batch search identifies the worktree store purely by hash;
        // resolving to the develop workspace hash proves the right worktree
        // was selected (canonical() keeps Windows paths comparable).
        assert_eq!(
            resolved.hash,
            gwt_core::worktree_hash::compute_worktree_hash(&canonical(&develop))
                .expect("worktree hash")
                .to_string()
        );
    }

    #[test]
    fn parse_runner_payload_accepts_jsonl_progress_before_final_result() {
        let payload = parse_runner_payload(
            br#"{"phase":"indexing","scope":"board","done":0,"total":0}
{"phase":"complete","scope":"board","total":0}
{"ok":true,"boardResults":[{"entry_id":"entry-1"}]}"#,
        )
        .expect("final ok payload");

        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(
            payload
                .get("boardResults")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn batch_scope_search_command_args_cover_all_scopes_in_one_request() {
        // Phase 70 FR-384 / AS-2: one search-multi request, interactive QoS,
        // worktree hash for file scopes, no inline auto-build.
        let args = batch_scope_search_command_args(
            Path::new("/repo"),
            "repo-hash",
            &[
                IndexSearchScope::Issues,
                IndexSearchScope::Specs,
                IndexSearchScope::Board,
                IndexSearchScope::Files,
                IndexSearchScope::FilesDocs,
            ],
            Some("wt-hash"),
            "Git",
            12,
            crate::protocol::IndexSearchMatchMode::AllTerms,
        );

        assert!(args.iter().any(|arg| arg == "search-multi"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--scopes"
                    && pair[1] == "issues,specs,board,files,files-docs"),
            "every requested scope must share the single batch request"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--worktree-hash" && pair[1] == "wt-hash"),
            "file scopes carry the worktree hash"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--qos" && pair[1] == "interactive"),
            "search runs at interactive QoS (FR-385)"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--match-mode" && pair[1] == "all_terms"),
            "batch search forwards the requested match mode"
        );
        assert!(
            !args.iter().any(|arg| arg == "--no-auto-build"),
            "search-multi never auto-builds; the Rust caller owns repair"
        );
    }

    #[test]
    fn broken_scopes_extracts_missing_and_corrupt_states() {
        let payload = json!({
            "ok": true,
            "scopes": {
                "issues": {"state": "fresh"},
                "specs": {"state": "stale"},
                "files": {"state": "missing"},
                "files-docs": {"state": "corrupt"},
            },
        });
        let mut broken = broken_scopes(&payload);
        broken.sort();
        assert_eq!(
            broken,
            vec![
                ("files".to_string(), "missing".to_string()),
                ("files-docs".to_string(), "corrupt".to_string()),
            ]
        );
    }

    #[test]
    fn broken_scopes_extracts_missing_issue_and_corrupt_spec_states() {
        // T-IDX-416 (SPEC #1939 Phase 70d, bundled-required by SPEC #3170
        // FR-097): the Knowledge Bridge consumer scopes classify exactly like
        // the file scopes — missing / corrupt block searching, stale does not.
        let payload = json!({
            "ok": true,
            "scopes": {
                "issues": {"state": "missing"},
                "specs": {"state": "corrupt"},
                "board": {"state": "fresh"},
                "works": {"state": "stale"},
            },
        });
        let mut broken = broken_scopes(&payload);
        broken.sort();
        assert_eq!(
            broken,
            vec![
                ("issues".to_string(), "missing".to_string()),
                ("specs".to_string(), "corrupt".to_string()),
            ]
        );
    }

    #[test]
    fn issue_and_spec_not_ready_errors_carry_prompt_retry_contract() {
        // T-IDX-416: the non-blocking (GUI) path reports waited_ms = 0 — a
        // prompt typed failure — while keeping the mandatory retry delay.
        let error = build_not_ready_error(
            &[
                ("issues".to_string(), "missing".to_string()),
                ("specs".to_string(), "corrupt".to_string()),
            ],
            0,
        );
        assert_eq!(error.exit_code(), INDEX_NOT_READY_EXIT_CODE);
        assert_eq!(error.error_code(), Some("INDEX_NOT_READY"));
        assert!(error.retryable());
        match error {
            IndexSearchError::NotReady(not_ready) => {
                assert_eq!(
                    not_ready.affected_scopes,
                    vec!["issues".to_string(), "specs".to_string()]
                );
                assert_eq!(not_ready.waited_ms, 0);
                assert!(not_ready.retry_after_ms > 0);
                assert!(not_ready.reason.contains("issues index is missing"));
                assert!(not_ready.reason.contains("specs index is corrupt"));
            }
            other => panic!("expected NotReady, got {other:?}"),
        }
    }

    #[test]
    fn scope_subpayload_prefers_per_scope_results_over_legacy_merge() {
        let payload = json!({
            "ok": true,
            "results": [{"path": "legacy.rs"}],
            "scope_results": {
                "files": {"results": [{"path": "scoped.rs"}]},
            },
        });
        let sub = scope_subpayload(&payload, IndexSearchScope::Files);
        assert_eq!(
            sub.get("results")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("path"))
                .and_then(Value::as_str),
            Some("scoped.rs")
        );
        // Legacy payloads without scope_results keep working (FR-398).
        let legacy = json!({"ok": true, "results": [{"path": "legacy.rs"}]});
        let sub = scope_subpayload(&legacy, IndexSearchScope::Files);
        assert!(sub.get("results").is_some());
    }

    #[test]
    fn not_ready_error_reports_retry_contract() {
        let error = build_not_ready_error(&[("files".to_string(), "missing".to_string())], 30_100);
        assert_eq!(error.exit_code(), 75);
        assert_eq!(error.error_code(), Some("INDEX_NOT_READY"));
        assert!(error.retryable());
        match error {
            IndexSearchError::NotReady(not_ready) => {
                assert_eq!(not_ready.affected_scopes, vec!["files".to_string()]);
                assert_eq!(not_ready.waited_ms, 30_100);
                assert!(not_ready.retry_after_ms > 0);
                assert!(not_ready.reason.contains("files"));
            }
            other => panic!("expected NotReady, got {other:?}"),
        }
    }

    fn runner_output(
        exit_code: i32,
        stdout: &str,
        stderr: &str,
    ) -> gwt_core::process_console::SpawnOutput {
        gwt_core::process_console::SpawnOutput {
            exit_code: Some(exit_code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            stdout_lines: stdout.lines().count() as u64,
            stderr_lines: stderr.lines().count() as u64,
        }
    }

    #[test]
    fn structured_runner_errors_restore_only_the_three_known_codes() {
        let not_ready = runner_payload_error(&json!({
            "ok": false,
            "error_code": "INDEX_NOT_READY",
            "retryable": true,
            "reason": "issues missing",
            "affected_scopes": ["issues"],
            "waited_ms": 7,
            "retry_after_ms": 11,
        }));
        assert!(matches!(
            not_ready,
            IndexSearchAttemptError::Public(IndexSearchError::NotReady(_))
        ));

        let failed = runner_payload_error(&json!({
            "ok": false,
            "error_code": "SEARCH_FAILED",
            "retryable": false,
            "error": "bad query",
            "affected_scopes": ["issues"],
        }));
        assert!(matches!(
            failed,
            IndexSearchAttemptError::Public(IndexSearchError::SearchFailed(_))
        ));

        let unavailable = runner_payload_error(&json!({
            "ok": false,
            "error_code": "SEARCH_UNAVAILABLE",
            "retryable": true,
            "reason": "runner busy",
            "retry_after_ms": 123,
        }));
        assert!(matches!(
            unavailable,
            IndexSearchAttemptError::Unavailable(_)
        ));

        for payload in [
            json!({"ok": false, "error_code": "UNKNOWN", "error": "legacy"}),
            json!({"ok": false, "error": "untyped"}),
        ] {
            assert!(matches!(
                runner_payload_error(&payload),
                IndexSearchAttemptError::Public(IndexSearchError::Other(_))
            ));
        }
    }

    #[test]
    fn malformed_known_code_payload_matrix_is_never_retryable() {
        let oversized_reason = "x".repeat(RUNNER_DIAGNOSTIC_MAX_BYTES + 1);
        let rows = vec![
            (
                "not-ready missing ok",
                json!({
                    "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready successful payload",
                json!({
                    "ok": true, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready retryable false",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": false,
                    "reason": "missing", "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready zero retry delay",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 0,
                }),
            ),
            (
                "not-ready non-string reason",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": 7, "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready ambiguous reason fields",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "error": "other", "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready non-array scopes",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": "issues",
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready empty scopes",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": [],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready unknown scope",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": ["future-scope"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready missing waited duration",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": "missing", "affected_scopes": ["issues"],
                    "retry_after_ms": 5_000,
                }),
            ),
            (
                "not-ready oversized reason",
                json!({
                    "ok": false, "error_code": "INDEX_NOT_READY", "retryable": true,
                    "reason": oversized_reason, "affected_scopes": ["issues"],
                    "waited_ms": 0, "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable successful payload",
                json!({
                    "ok": true, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": "busy", "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable missing retryable flag",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE",
                    "reason": "busy", "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable retryable false",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": false,
                    "reason": "busy", "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable zero retry delay",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": "busy", "retry_after_ms": 0,
                }),
            ),
            (
                "unavailable non-string reason",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": ["busy"], "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable ansi-only reason",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": "\u{1b}[31m\u{1b}[0m", "retry_after_ms": 5_000,
                }),
            ),
            (
                "unavailable missing retry delay",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": "busy",
                }),
            ),
            (
                "unavailable malformed optional scopes",
                json!({
                    "ok": false, "error_code": "SEARCH_UNAVAILABLE", "retryable": true,
                    "reason": "busy", "retry_after_ms": 5_000,
                    "affected_scopes": [7],
                }),
            ),
            (
                "search-failed retryable true",
                json!({
                    "ok": false, "error_code": "SEARCH_FAILED", "retryable": true,
                    "error": "bad query", "affected_scopes": ["issues"],
                }),
            ),
            (
                "search-failed missing scopes",
                json!({
                    "ok": false, "error_code": "SEARCH_FAILED", "retryable": false,
                    "error": "bad query",
                }),
            ),
            (
                "search-failed non-string scope",
                json!({
                    "ok": false, "error_code": "SEARCH_FAILED", "retryable": false,
                    "error": "bad query", "affected_scopes": [7],
                }),
            ),
            (
                "search-failed missing reason",
                json!({
                    "ok": false, "error_code": "SEARCH_FAILED", "retryable": false,
                    "affected_scopes": ["issues"],
                }),
            ),
        ];

        for (case, payload) in rows {
            let error = runner_payload_error(&payload);
            assert!(
                matches!(
                    error,
                    IndexSearchAttemptError::Public(IndexSearchError::Other(_))
                ),
                "{case} unexpectedly restored a typed error: {error:?}"
            );
            assert!(!error.retryable(), "{case} unexpectedly became retryable");
            assert_eq!(error.error_code(), None, "{case}");
        }
    }

    #[test]
    fn exit_zero_malformed_and_unknown_payloads_are_not_retryable() {
        let malformed = classify_successful_runner_output(&runner_output(0, "not json", ""))
            .expect_err("malformed success payload must fail");
        assert!(matches!(
            malformed,
            IndexSearchAttemptError::Public(IndexSearchError::Other(_))
        ));
        assert!(!malformed.retryable());

        let unknown = classify_successful_runner_output(&runner_output(
            0,
            r#"{"ok":false,"error_code":"FUTURE_CODE","error":"future"}"#,
            "",
        ))
        .expect_err("unknown typed payload must fail closed");
        assert!(matches!(
            unknown,
            IndexSearchAttemptError::Public(IndexSearchError::Other(_))
        ));
        assert!(!unknown.retryable());
    }

    #[test]
    fn exit_zero_payload_cannot_mix_success_with_retryable_error_fields() {
        let cases = [
            ("error", serde_json::json!({"error": "failed"})),
            (
                "error_code",
                serde_json::json!({"error_code": "INDEX_NOT_READY"}),
            ),
            ("reason", serde_json::json!({"reason": "not ready"})),
            ("retryable", serde_json::json!({"retryable": true})),
            (
                "affected_scopes",
                serde_json::json!({"affected_scopes": ["issues"]}),
            ),
            ("waited_ms", serde_json::json!({"waited_ms": 0})),
            (
                "retry_after_ms",
                serde_json::json!({"retry_after_ms": 5_000}),
            ),
        ];
        for (field, extra) in cases {
            let mut payload = serde_json::json!({"ok": true});
            payload
                .as_object_mut()
                .expect("success payload object")
                .extend(extra.as_object().expect("extra fields").clone());
            let output = runner_output(0, &payload.to_string(), "");

            let error = classify_successful_runner_output(&output)
                .expect_err("contradictory success payload must fail closed");

            assert!(
                matches!(
                    error,
                    IndexSearchAttemptError::Public(IndexSearchError::Other(_))
                ),
                "{field}"
            );
            assert!(!error.retryable(), "{field}");
        }
    }

    #[test]
    fn nonzero_unstructured_output_is_search_unavailable() {
        let error = classify_failed_runner_output(&runner_output(
            9,
            "progress only",
            "temporary transport failure",
        ));
        assert!(matches!(error, IndexSearchAttemptError::Unavailable(_)));
        assert!(error.retryable());
        assert_eq!(error.error_code(), Some("SEARCH_UNAVAILABLE"));
    }

    #[test]
    fn public_mapping_hides_internal_unavailable_diagnostics() {
        let raw = format!(
            "\u{1b}[31msecret runner path ghp_abcdef0123456789ABCDEF\u{1b}[0m {}",
            "x".repeat(RUNNER_DIAGNOSTIC_MAX_BYTES * 4)
        );
        let public_error = IndexSearchAttemptError::Unavailable(IndexSearchUnavailable {
            reason: raw.clone(),
            retry_after_ms: SEARCH_RETRY_AFTER_MS,
        })
        .into_public();

        let IndexSearchError::Other(message) = public_error else {
            panic!("internal unavailability must map to the legacy public Other variant");
        };
        assert_eq!(message, "project index search is temporarily unavailable");
        assert!(!message.contains(&raw));
        assert!(!message.contains('\u{1b}'));
        assert!(!message.contains("ghp_"));
        assert!(message.len() < RUNNER_DIAGNOSTIC_MAX_BYTES);
    }

    #[test]
    fn runner_diagnostic_is_ansi_free_redacted_and_unicode_safely_bounded() {
        let token = "ghp_abcdef0123456789ABCDEF";
        let raw = format!("\u{1b}[31m{token}\u{1b}[0m {}", "界".repeat(800));
        let sanitized = sanitize_runner_diagnostic(&raw);

        assert!(!sanitized.contains('\u{1b}'));
        assert!(!sanitized.contains(token));
        assert!(sanitized.contains(gwt_core::process_console::REDACTED));
        assert!(sanitized.len() <= RUNNER_DIAGNOSTIC_MAX_BYTES);
        assert!(sanitized.len() > RUNNER_DIAGNOSTIC_MAX_BYTES - 4);
        assert!(std::str::from_utf8(sanitized.as_bytes()).is_ok());
    }

    #[test]
    fn search_attempt_environment_can_only_shorten_the_hard_limit() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _long = ScopedEnvVar::set("GWT_INDEX_SEARCH_RUNNER_DEADLINE_MS", "60000");
        assert_eq!(search_attempt_deadline_ms(), SEARCH_ATTEMPT_HARD_LIMIT_MS);
        drop(_long);
        let _short = ScopedEnvVar::set("GWT_INDEX_SEARCH_RUNNER_DEADLINE_MS", "125");
        assert_eq!(search_attempt_deadline_ms(), 125);
    }

    #[test]
    fn hanging_git_context_probe_is_bounded_and_maps_to_search_unavailable() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _hang = ScopedEnvVar::set("GWT_INDEX_TEST_GIT_HANG", "1");
        let _pre_spawn = ScopedEnvVar::set("GWT_INDEX_TEST_GIT_PRESPAWN_MS", "100");
        let _deadline = ScopedEnvVar::set("GWT_INDEX_SEARCH_RUNNER_DEADLINE_MS", "150");

        let started = Instant::now();
        let error = search_project_index_attempt(
            temp.path(),
            "deadline git context",
            &[IndexSearchScope::Issues],
            None,
            IndexSearchMatchMode::Semantic,
            false,
        )
        .expect_err("a hanging git context probe must not escape the attempt deadline");

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "git context probe exceeded the absolute search budget: {:?}",
            started.elapsed()
        );
        assert!(
            matches!(error, IndexSearchAttemptError::Unavailable(_)),
            "{error:?}"
        );
        assert!(error.retryable());
    }

    #[test]
    fn git_context_spawn_failure_maps_to_search_unavailable() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let _spawn_failure = ScopedEnvVar::set("GWT_INDEX_TEST_GIT_SPAWN_FAILURE", "1");

        let error = search_project_index_attempt(
            temp.path(),
            "unavailable git context",
            &[IndexSearchScope::Issues],
            None,
            IndexSearchMatchMode::Semantic,
            false,
        )
        .expect_err("git spawn failure must be retryable infrastructure failure");

        assert!(
            matches!(error, IndexSearchAttemptError::Unavailable(_)),
            "{error:?}"
        );
        assert!(error.retryable());
    }

    #[test]
    fn repair_spawn_failure_rolls_back_admission_and_notifies_waiters() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = RepairKey {
            repo_root: PathBuf::from("spawn-failure-test"),
            scope: "issues".to_string(),
            worktree_hash: None,
        };
        let spawned = spawn_tracked_repair_with(
            key.clone(),
            || panic!("failed spawner must never run the repair"),
            |task| {
                drop(task);
                Err(std::io::Error::other("injected spawn failure"))
            },
        );

        assert!(!spawned);
        assert!(wait_for_index_search_repairs(Duration::from_millis(50)));
        let lease = RepairLease::admit(key).expect("spawn failure released the repair key");
        drop(lease);
    }

    #[test]
    fn repair_admission_is_single_flight_per_repo_scope_and_worktree() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = RepairKey {
            repo_root: PathBuf::from("single-flight-test"),
            scope: "files".to_string(),
            worktree_hash: Some("worktree-a".to_string()),
        };
        let first = RepairLease::admit(key.clone()).expect("first admission");
        assert!(RepairLease::admit(key.clone()).is_none());
        assert!(RepairLease::admit(RepairKey {
            worktree_hash: Some("worktree-b".to_string()),
            ..key.clone()
        })
        .is_some());
        drop(first);
        assert!(RepairLease::admit(key).is_some());
    }

    #[test]
    fn repair_inherits_absolute_deadline_releases_lease_and_can_be_readmitted() {
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = RepairKey {
            repo_root: PathBuf::from("deadline-repair-test"),
            scope: "issues".to_string(),
            worktree_hash: None,
        };
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let expires_at = Instant::now() + Duration::from_millis(150);
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(expires_at);

        let spawned = spawn_tracked_repair_with(
            key.clone(),
            move || {
                #[cfg(windows)]
                let (program, args) = (
                    OsString::from("powershell.exe"),
                    vec![
                        OsString::from("-NoProfile"),
                        OsString::from("-Command"),
                        OsString::from("Start-Sleep -Seconds 4"),
                    ],
                );
                #[cfg(not(windows))]
                let (program, args) = (
                    OsString::from("sh"),
                    vec![OsString::from("-c"), OsString::from("sleep 4")],
                );
                let result = gwt_core::process_console::spawn_logged_blocking(
                    &gwt_core::process_console::ProcessConsoleHub::new(),
                    gwt_core::process_console::ProcessKind::IndexRunner,
                    program,
                    &args,
                    gwt_core::process_console::SpawnOptions::new("deadline repair fixture")
                        .forward_output(false),
                )
                .map_err(|error| error.kind());
                let _ = result_tx.send(result);
            },
            |task| {
                // Start close to the parent's absolute expiry. A relative
                // deadline installed inside the worker would incorrectly
                // grant a fresh budget here.
                std::thread::sleep(Duration::from_millis(100));
                std::thread::Builder::new()
                    .name("deadline-repair-fixture".to_string())
                    .spawn(task)
                    .map(|_| ())
            },
        );

        assert!(spawned);
        let bounded_result = result_rx.recv_timeout(Duration::from_secs(2));
        if bounded_result.is_err() {
            // Let the pre-fix unbounded child finish before failing so the
            // process and global repair tracker cannot leak into other tests.
            let _ = result_rx.recv_timeout(Duration::from_secs(3));
        }
        let process_result =
            bounded_result.expect("repair process must settle at the inherited deadline");
        assert!(
            matches!(process_result, Err(std::io::ErrorKind::TimedOut)),
            "repair process must be killed by the deadline: {process_result:?}"
        );
        assert!(wait_for_index_search_repairs(Duration::from_millis(100)));
        let lease = RepairLease::admit(key).expect("settled repair key must be re-admitted");
        drop(lease);
    }

    #[test]
    fn queued_rebuild_runner_hang_settles_and_allows_re_admission() {
        use gwt_core::test_support::ScopedEnvVar;
        let _lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo directory");
        run_git_at(&repo, &["init", "-q", "-b", "develop"]);
        // The production coordinator is host-wide and keys repo-shared jobs
        // by the origin-derived repo hash. Give every fixture invocation its
        // own identity so an earlier run or another checkout cannot coalesce
        // this repair before the runner closure reaches the marker.
        let remote_suffix = temp
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("tempdir name");
        let remote = format!("https://example.com/deadline-repair-{remote_suffix}.git");
        run_git_at(&repo, &["remote", "add", "origin", remote.as_str()]);
        let marker = temp.path().join("rebuild-started.marker");
        let _hang = ScopedEnvVar::set("GWT_INDEX_TEST_REBUILD_HANG", "1");
        let _marker = ScopedEnvVar::set("GWT_INDEX_TEST_REBUILD_MARKER", &marker);
        // Coverage instrumentation makes startup of the child test binary
        // materially slower than an idle production runner. Leave enough
        // startup budget to reach the fixture while keeping the inherited
        // deadline shorter than the fixture's deliberate hang.
        let inherited_budget = Duration::from_secs(8);
        let expires_at = Instant::now() + inherited_budget;
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(expires_at);

        queue_scope_rebuilds(
            &repo,
            &[("issues".to_string(), "missing".to_string())],
            None,
        );

        let marker_wait_started = Instant::now();
        while !marker.exists()
            && marker_wait_started.elapsed() < inherited_budget + Duration::from_secs(1)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        let settlement_wait =
            expires_at.saturating_duration_since(Instant::now()) + Duration::from_secs(3);
        let settled = wait_for_index_search_repairs(settlement_wait);
        if !settled {
            // Let the pre-fix raw child finish before failing, keeping the
            // singleton tracker clean for the rest of the test process.
            let _ = wait_for_index_search_repairs(Duration::from_secs(20));
        }

        assert!(
            marker.exists(),
            "the production rebuild runner path was not reached"
        );
        assert!(
            settled,
            "queued rebuild outlived the inherited attempt deadline"
        );
        let key = RepairKey {
            repo_root: dunce::canonicalize(&repo).unwrap_or(repo),
            scope: "issues".to_string(),
            worktree_hash: None,
        };
        let lease = RepairLease::admit(key).expect("settled queued repair must be re-admitted");
        drop(lease);
    }
}

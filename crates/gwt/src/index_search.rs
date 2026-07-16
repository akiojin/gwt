use std::{ffi::OsString, path::Path, thread, time::Duration};

use serde_json::Value;

use crate::{
    protocol::{IndexSearchMatchMode, IndexSearchResult, IndexSearchScope, IndexSearchTarget},
    worktree_inventory,
};

const INDEX_SEARCH_LIMIT: usize = 50;

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

/// Search error surface (Phase 70 FR-388). `NotReady` is retryable and maps
/// to exit code 75 / `error_code=INDEX_NOT_READY` on the CLI surface.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexSearchError {
    NotReady(IndexSearchNotReady),
    Other(String),
}

impl IndexSearchError {
    pub fn exit_code(&self) -> i32 {
        match self {
            IndexSearchError::NotReady(_) => INDEX_NOT_READY_EXIT_CODE,
            IndexSearchError::Other(_) => 1,
        }
    }

    pub fn error_code(&self) -> Option<&'static str> {
        match self {
            IndexSearchError::NotReady(_) => Some("INDEX_NOT_READY"),
            IndexSearchError::Other(_) => None,
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, IndexSearchError::NotReady(_))
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
            IndexSearchError::Other(message) => f.write_str(message),
        }
    }
}

impl From<String> for IndexSearchError {
    fn from(message: String) -> Self {
        IndexSearchError::Other(message)
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
    let query = query.trim();
    if query.is_empty() {
        return Ok(ProjectIndexSearchOutcome::default());
    }
    let index_repo_root = crate::index_worker::resolve_project_index_repo_root(project_root)
        .ok_or_else(|| "project index search requires a git origin remote".to_string())?;
    let repo_hash = crate::index_worker::detect_repo_hash(&index_repo_root)
        .ok_or_else(|| "project index search requires a git origin remote".to_string())?;
    let repo_search_root = crate::index_worker::default_project_index_worktree_root(project_root)
        .unwrap_or_else(|| index_repo_root.clone());
    gwt_core::runtime::ensure_project_index_runtime().map_err(|error| error.to_string())?;

    let effective_scopes = if scopes.is_empty() {
        default_index_search_scopes()
    } else {
        scopes.to_vec()
    };
    let board_scope = crate::board_audience::gui_default_board_scope(project_root)
        .unwrap_or(gwt_core::coordination::BoardAudienceScope::All);
    let file_worktree = if effective_scopes.iter().any(|scope| is_file_scope(*scope)) {
        Some(resolve_file_search_worktree(
            project_root,
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
    let run_batch = || -> Result<Value, String> {
        run_batch_scope_search(
            &repo_search_root,
            repo_hash.as_str(),
            &effective_scopes,
            worktree_hash_arg.as_deref(),
            query,
            per_scope_limit,
            match_mode,
        )
    };

    let repair_deadline = Duration::from_millis(search_repair_wait_ms());
    let started = std::time::Instant::now();
    let mut payload = run_batch()?;
    let mut broken = broken_scopes(&payload);
    if !broken.is_empty() {
        // FR-388: missing / corrupt scopes never degrade into a silent
        // empty success. With auto_build the caller joins the coordinated
        // repair and waits up to the deadline; without it (GUI, watcher owns
        // builds) the typed retryable error returns immediately.
        if !auto_build {
            return Err(build_not_ready_error(&broken, 0));
        }
        queue_scope_rebuilds(project_root, &broken, worktree_hash_arg.as_deref());
        loop {
            let elapsed = started.elapsed();
            if elapsed >= repair_deadline {
                return Err(build_not_ready_error(&broken, elapsed.as_millis() as u64));
            }
            let remaining = repair_deadline - elapsed;
            thread::sleep(remaining.min(Duration::from_secs(1)));
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

/// Default (and env-overridable) wait for missing / corrupt scope repair
/// before returning `INDEX_NOT_READY` (FR-388: 30 seconds).
fn search_repair_wait_ms() -> u64 {
    std::env::var("GWT_INDEX_SEARCH_REPAIR_WAIT_MS")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(30_000)
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
fn queue_scope_rebuilds(
    project_root: &Path,
    scopes: &[(String, String)],
    worktree_hash: Option<&str>,
) {
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
        let _ = std::thread::Builder::new()
            .name("gwt-index-search-repair".to_string())
            .spawn(move || {
                if let Err(error) = crate::index_worker::default_rebuild_runner(
                    &project_root,
                    rebuild_scope,
                    worktree.as_deref(),
                ) {
                    tracing::warn!(
                        target: "gwt::index",
                        scope = %scope_label,
                        error = %error,
                        "search-triggered index repair failed"
                    );
                }
            });
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
    project_root: &Path,
    selected_worktree_hash: Option<&str>,
) -> Result<FileSearchWorktree, String> {
    let index_repo_root = crate::index_worker::resolve_project_index_repo_root(project_root)
        .unwrap_or_else(|| project_root.to_path_buf());
    if let Some(hash) = selected_worktree_hash
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
    {
        let active_root = crate::index_worker::default_project_index_worktree_root(project_root);
        let entries =
            worktree_inventory::enumerate_worktrees(&index_repo_root, active_root.as_deref())
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
    let worktree_root = crate::index_worker::default_project_index_worktree_root(project_root)
        .ok_or_else(|| "file search requires a git worktree".to_string())?;
    if worktree_root == index_repo_root {
        let entries = worktree_inventory::enumerate_worktrees(&index_repo_root, None)
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
    let hash = gwt_core::worktree_hash::compute_worktree_hash(&worktree_root)
        .map_err(|error| error.to_string())?
        .to_string();
    Ok(FileSearchWorktree { hash })
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
) -> Result<Value, String> {
    let output =
        gwt_core::process::hidden_command(crate::index_worker::project_index_python_path())
            .args(batch_scope_search_command_args(
                project_root,
                repo_hash,
                scopes,
                worktree_hash,
                query,
                limit,
                match_mode,
            ))
            .current_dir(project_root)
            .output()
            .map_err(|error| format!("run project index search: {error}"))?;
    if !output.status.success() {
        return Err(format_runner_failure(&output));
    }
    let payload = parse_runner_payload(&output.stdout)?;
    if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(payload_error(&payload));
    }
    Ok(payload)
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
    payload
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("project index search failed")
        .to_string()
}

fn format_runner_failure(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    format!("runner exited with {}", output.status)
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
                if payload.get("ok").is_some() || payload.get("error").is_some() {
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

        let resolved =
            resolve_file_search_worktree(temp.path(), None).expect("file search worktree");

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
            IndexSearchError::Other(_) => panic!("expected NotReady"),
        }
    }
}

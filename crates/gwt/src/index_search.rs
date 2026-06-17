use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    thread,
};

use serde_json::Value;

use crate::{
    protocol::{IndexSearchMatchMode, IndexSearchResult, IndexSearchScope, IndexSearchTarget},
    worktree_inventory,
};

const INDEX_SEARCH_LIMIT: usize = 50;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectIndexSearchOutcome {
    pub results: Vec<IndexSearchResult>,
    pub suggestions: Vec<IndexSearchResult>,
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
) -> Result<ProjectIndexSearchOutcome, String> {
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
    let file_worktree = if effective_scopes
        .iter()
        .any(|scope| matches!(scope, IndexSearchScope::Files | IndexSearchScope::FilesDocs))
    {
        Some(resolve_file_search_worktree(
            project_root,
            selected_worktree_hash,
        )?)
    } else {
        None
    };

    let mut results = Vec::new();
    let mut suggestions = Vec::new();
    let per_scope_limit = per_scope_limit(effective_scopes.len());
    let mut scope_jobs = Vec::new();
    let mut repo_scopes = Vec::new();
    for scope in effective_scopes {
        if is_file_scope(scope) {
            let file_worktree = file_worktree
                .as_ref()
                .ok_or_else(|| "file search worktree was not resolved".to_string())?;
            scope_jobs.push(ScopeSearchJob {
                search_root: file_worktree.path.clone(),
                worktree_hash: Some(file_worktree.hash.clone()),
                scope,
            });
        } else if auto_build {
            // The runner's `search-multi` action hardcodes no_auto_build
            // (interactive GUI contract). CLI search must self-heal missing
            // or stale indexes (SPEC-1942 FR-107), so repo scopes go through
            // the per-scope actions, which also surface the EMPTY_CORPUS
            // diagnostic for agents (Issue #2979).
            scope_jobs.push(ScopeSearchJob {
                search_root: repo_search_root.clone(),
                worktree_hash: None,
                scope,
            });
        } else {
            repo_scopes.push(scope);
        }
    }
    if !repo_scopes.is_empty() {
        let payload = run_repo_scope_search(
            &repo_search_root,
            repo_hash.as_str(),
            &repo_scopes,
            query,
            per_scope_limit,
            match_mode,
            auto_build,
        )?;
        for scope in repo_scopes {
            append_scope_results(&mut results, scope, &payload, &board_scope);
            append_scope_suggestions(&mut suggestions, scope, &payload, &board_scope);
        }
    }
    for outcome in run_scope_search_jobs(
        scope_jobs,
        repo_hash.as_str(),
        query,
        per_scope_limit,
        match_mode,
        |root, hash, worktree, scope, job_query, job_limit, job_match_mode| {
            run_scope_search(
                root,
                hash,
                worktree,
                scope,
                job_query,
                job_limit,
                job_match_mode,
                auto_build,
            )
        },
    )? {
        append_scope_results(&mut results, outcome.scope, &outcome.payload, &board_scope);
        append_scope_suggestions(
            &mut suggestions,
            outcome.scope,
            &outcome.payload,
            &board_scope,
        );
    }

    results.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    suggestions.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    results.truncate(INDEX_SEARCH_LIMIT);
    suggestions.truncate(INDEX_SEARCH_LIMIT);
    Ok(ProjectIndexSearchOutcome {
        results,
        suggestions,
    })
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
pub fn work_advisory(project_root: &Path, query: &str) -> Result<Vec<IndexSearchResult>, String> {
    let outcome = search_project_index(
        project_root,
        query,
        WORK_ADVISORY_SCOPES,
        None,
        IndexSearchMatchMode::Semantic,
        true,
    )?;
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

struct ScopeSearchJob {
    search_root: PathBuf,
    worktree_hash: Option<String>,
    scope: IndexSearchScope,
}

struct ScopeSearchOutcome {
    scope: IndexSearchScope,
    payload: Value,
}

fn run_scope_search_jobs<F>(
    jobs: Vec<ScopeSearchJob>,
    repo_hash: &str,
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
    runner: F,
) -> Result<Vec<ScopeSearchOutcome>, String>
where
    F: Fn(
            &Path,
            &str,
            Option<&str>,
            IndexSearchScope,
            &str,
            usize,
            IndexSearchMatchMode,
        ) -> Result<Value, String>
        + Sync,
{
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(jobs.len());
        for job in jobs {
            let runner = &runner;
            handles.push(scope.spawn(move || {
                runner(
                    job.search_root.as_path(),
                    repo_hash,
                    job.worktree_hash.as_deref(),
                    job.scope,
                    query,
                    limit,
                    match_mode,
                )
                .map(|payload| ScopeSearchOutcome {
                    scope: job.scope,
                    payload,
                })
                .map_err(|error| format!("{} search failed: {error}", job.scope.as_str()))
            }));
        }

        let mut outcomes = Vec::with_capacity(handles.len());
        let mut first_error = None;
        for handle in handles {
            match handle.join() {
                Ok(Ok(outcome)) => outcomes.push(outcome),
                Ok(Err(error)) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                Err(_) => {
                    if first_error.is_none() {
                        first_error = Some("project index search worker panicked".to_string());
                    }
                }
            }
        }
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(outcomes)
        }
    })
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
    path: PathBuf,
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
            path: entry.path,
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
            return Ok(FileSearchWorktree {
                path: entry.path,
                hash,
            });
        }
    }
    let hash = gwt_core::worktree_hash::compute_worktree_hash(&worktree_root)
        .map_err(|error| error.to_string())?
        .to_string();
    Ok(FileSearchWorktree {
        path: worktree_root,
        hash,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_scope_search(
    project_root: &Path,
    repo_hash: &str,
    worktree_hash: Option<&str>,
    scope: IndexSearchScope,
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
) -> Result<Value, String> {
    let output =
        gwt_core::process::hidden_command(crate::index_worker::project_index_python_path())
            .args(scope_search_command_args(
                project_root,
                repo_hash,
                worktree_hash,
                scope,
                query,
                limit,
                match_mode,
                auto_build,
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

#[allow(clippy::too_many_arguments)]
fn run_repo_scope_search(
    project_root: &Path,
    repo_hash: &str,
    scopes: &[IndexSearchScope],
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
) -> Result<Value, String> {
    let output =
        gwt_core::process::hidden_command(crate::index_worker::project_index_python_path())
            .args(repo_scope_search_command_args(
                project_root,
                repo_hash,
                scopes,
                query,
                limit,
                match_mode,
                auto_build,
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

#[allow(clippy::too_many_arguments)]
fn scope_search_command_args(
    project_root: &Path,
    repo_hash: &str,
    worktree_hash: Option<&str>,
    scope: IndexSearchScope,
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
) -> Vec<OsString> {
    let mut args = vec![
        gwt_core::runtime::project_index_runner_path().into_os_string(),
        OsString::from("--action"),
        OsString::from(search_action(scope)),
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
    ];
    if !auto_build {
        args.push(OsString::from("--no-auto-build"));
    }
    if let Some(hash) = worktree_hash {
        args.push(OsString::from("--worktree-hash"));
        args.push(OsString::from(hash));
    }
    args
}

fn repo_scope_search_command_args(
    project_root: &Path,
    repo_hash: &str,
    scopes: &[IndexSearchScope],
    query: &str,
    limit: usize,
    match_mode: IndexSearchMatchMode,
    auto_build: bool,
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
        OsString::from("--scopes"),
        OsString::from(
            scopes
                .iter()
                .map(|scope| scope.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
    ];
    if !auto_build {
        args.push(OsString::from("--no-auto-build"));
    }
    args
}

fn search_action(scope: IndexSearchScope) -> &'static str {
    match scope {
        IndexSearchScope::Issues => "search-issues",
        IndexSearchScope::Specs => "search-specs",
        IndexSearchScope::Memory => "search-memory",
        IndexSearchScope::Discussions => "search-discussions",
        IndexSearchScope::Board => "search-board",
        IndexSearchScope::Works => "search-works",
        IndexSearchScope::Files => "search-files",
        IndexSearchScope::FilesDocs => "search-files-docs",
    }
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

        assert_eq!(canonical(&resolved.path), canonical(&develop));
        assert_eq!(
            resolved.hash,
            gwt_core::worktree_hash::compute_worktree_hash(&develop)
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
    fn scope_search_command_args_disable_auto_build_for_interactive_search() {
        let args = scope_search_command_args(
            Path::new("/repo"),
            "repo-hash",
            None,
            IndexSearchScope::Issues,
            "Git",
            50,
            crate::protocol::IndexSearchMatchMode::AllTerms,
            false,
        );

        assert!(
            args.iter().any(|arg| arg == "--no-auto-build"),
            "interactive Index search must not block on auto-index rebuilds"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--match-mode" && pair[1] == "all_terms"),
            "runner args should carry the requested match mode"
        );
    }

    #[test]
    fn scope_search_command_args_enable_auto_build_for_cli_search() {
        // SPEC-1942 FR-107: `search` has no watcher, so the runner must
        // self-heal missing or stale (source_cache_changed) indexes inline.
        let args = scope_search_command_args(
            Path::new("/repo"),
            "repo-hash",
            None,
            IndexSearchScope::Issues,
            "Git",
            50,
            crate::protocol::IndexSearchMatchMode::Semantic,
            true,
        );

        assert!(
            !args.iter().any(|arg| arg == "--no-auto-build"),
            "CLI search must allow the runner auto-build fallback"
        );
    }

    #[test]
    fn repo_scope_search_command_args_use_single_multi_scope_runner() {
        let args = repo_scope_search_command_args(
            Path::new("/repo"),
            "repo-hash",
            &[
                IndexSearchScope::Issues,
                IndexSearchScope::Specs,
                IndexSearchScope::Board,
                IndexSearchScope::Discussions,
                IndexSearchScope::Memory,
            ],
            "Git",
            12,
            crate::protocol::IndexSearchMatchMode::AllTerms,
            false,
        );

        assert!(args.iter().any(|arg| arg == "search-multi"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--scopes"
                    && pair[1] == "issues,specs,board,discussions,memory"),
            "repo-scoped searches should share one runner process"
        );
        assert!(args.iter().any(|arg| arg == "--no-auto-build"));
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--match-mode" && pair[1] == "all_terms"),
            "repo-scoped searches should forward match mode to search-multi"
        );

        // SPEC-1942 FR-107: the CLI variant drops --no-auto-build so the
        // runner can rebuild stale repo-scope indexes inline.
        let cli_args = repo_scope_search_command_args(
            Path::new("/repo"),
            "repo-hash",
            &[IndexSearchScope::Issues],
            "Git",
            12,
            crate::protocol::IndexSearchMatchMode::Semantic,
            true,
        );
        assert!(!cli_args.iter().any(|arg| arg == "--no-auto-build"));
    }

    #[test]
    fn run_scope_search_jobs_executes_selected_scopes_in_parallel() {
        use std::{
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
            thread,
            time::Duration,
        };

        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let jobs = vec![
            ScopeSearchJob {
                search_root: PathBuf::from("/repo"),
                worktree_hash: None,
                scope: IndexSearchScope::Issues,
            },
            ScopeSearchJob {
                search_root: PathBuf::from("/repo"),
                worktree_hash: None,
                scope: IndexSearchScope::Specs,
            },
            ScopeSearchJob {
                search_root: PathBuf::from("/repo"),
                worktree_hash: None,
                scope: IndexSearchScope::Board,
            },
            ScopeSearchJob {
                search_root: PathBuf::from("/repo"),
                worktree_hash: None,
                scope: IndexSearchScope::Discussions,
            },
            ScopeSearchJob {
                search_root: PathBuf::from("/repo"),
                worktree_hash: None,
                scope: IndexSearchScope::Memory,
            },
        ];

        let results = run_scope_search_jobs(
            jobs,
            "repo",
            "Git",
            3,
            IndexSearchMatchMode::Semantic,
            |_, _, _, scope, _, _, _| {
                let active_now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(active_now, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(120));
                active.fetch_sub(1, Ordering::SeqCst);
                let key = match scope {
                    IndexSearchScope::Issues => "issueResults",
                    IndexSearchScope::Specs => "specResults",
                    IndexSearchScope::Board => "boardResults",
                    IndexSearchScope::Discussions => "discussionResults",
                    IndexSearchScope::Memory => "memoryResults",
                    IndexSearchScope::Works => "workResults",
                    IndexSearchScope::Files | IndexSearchScope::FilesDocs => "results",
                };
                let mut payload = serde_json::Map::new();
                payload.insert("ok".to_string(), Value::Bool(true));
                payload.insert(key.to_string(), Value::Array(Vec::new()));
                Ok(Value::Object(payload))
            },
        )
        .expect("parallel scope search should succeed");

        assert_eq!(results.len(), 5);
        assert!(
            peak.load(Ordering::SeqCst) > 1,
            "expected selected scope searches to overlap"
        );
    }
}

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gwt_core::paths::gwt_cache_dir;
use gwt_github::{Cache, CacheEntry, IssueState, SectionName};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::issue_cache::{
    issue_cache_root_for_repo_path, issue_cache_root_for_repo_path_or_detached,
    sync_issue_cache_from_remote, sync_issue_cache_from_remote_if_stale, ISSUE_CACHE_TTL,
};

const SPEC_LABEL: &str = "gwt-spec";
const KNOWLEDGE_SEARCH_RESULT_LIMIT: usize = 50;

/// Canonical SPEC phase labels in lifecycle order.
///
/// `phase/<value>` labels matching one of these values map to the canonical
/// phase. Any other `phase/*` label is reported as unknown/legacy via
/// [`ExtractedPhase::has_unknown_phase`] and not promoted to a column.
pub const KNOWLEDGE_PHASE_LABELS: &[&str] =
    &["draft", "planning", "implementation", "review", "done"];

/// Result of [`extract_phase`]: the canonical phase (if any), whether any
/// unknown `phase/*` label is present, and whether the entry is a SPEC
/// (`gwt-spec` label).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractedPhase {
    pub phase: Option<String>,
    pub has_unknown_phase: bool,
    pub is_spec: bool,
}

/// Extract the canonical phase from an Issue's labels, plus auxiliary flags
/// used by Kanban grouping.
///
/// - `phase` is `Some("<canonical>")` when exactly one of `phase/draft`,
///   `phase/planning`, `phase/implementation`, `phase/review`, `phase/done`
///   appears. The first canonical match wins; further canonical or legacy
///   `phase/*` labels also raise `has_unknown_phase` so the UI can surface a
///   warning for malformed input.
/// - `has_unknown_phase` is `true` when any `phase/*` label outside the
///   canonical set is present, OR when more than one canonical phase label
///   is present.
/// - `is_spec` mirrors the `gwt-spec` label.
pub fn extract_phase(labels: &[String]) -> ExtractedPhase {
    let mut phase: Option<String> = None;
    let mut has_unknown_phase = false;
    let mut is_spec = false;

    for label in labels {
        if label == SPEC_LABEL {
            is_spec = true;
            continue;
        }
        let Some(rest) = label.strip_prefix("phase/") else {
            continue;
        };
        if KNOWLEDGE_PHASE_LABELS.contains(&rest) {
            if phase.is_none() {
                phase = Some(rest.to_string());
            } else {
                has_unknown_phase = true;
            }
        } else {
            has_unknown_phase = true;
        }
    }

    ExtractedPhase {
        phase,
        has_unknown_phase,
        is_spec,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeKind {
    Issue,
    Spec,
    Pr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeListItem {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub meta: String,
    pub labels: Vec<String>,
    pub linked_branch_count: usize,
    pub match_score: Option<u8>,
    /// Canonical phase value (`"draft"`, `"planning"`, `"implementation"`,
    /// `"review"`, `"done"`) when a `phase/*` label is present, otherwise
    /// `None`. Used by the Kanban view for column grouping.
    #[serde(default)]
    pub phase: Option<String>,
    /// `true` when an unknown / legacy `phase/*` label is present (or when
    /// more than one canonical phase label is set). The UI shows a warning
    /// indicator for these entries.
    #[serde(default)]
    pub has_unknown_phase: bool,
    /// `true` when the entry carries the `gwt-spec` label. Plain Issues are
    /// always grouped into the Backlog column and are not draggable.
    #[serde(default)]
    pub is_spec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDetailSection {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDetailView {
    pub number: Option<u64>,
    pub title: String,
    pub subtitle: String,
    pub state: String,
    pub labels: Vec<String>,
    pub sections: Vec<KnowledgeDetailSection>,
    pub launch_issue_number: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeBridgeView {
    pub kind: KnowledgeKind,
    pub entries: Vec<KnowledgeListItem>,
    pub selected_number: Option<u64>,
    pub empty_message: Option<String>,
    pub refresh_enabled: bool,
    pub detail: KnowledgeDetailView,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchHit {
    pub number: u64,
    pub distance: Option<f64>,
}

pub trait SemanticSearchClient {
    fn search(
        &self,
        repo_path: &Path,
        kind: KnowledgeKind,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticSearchHit>, String>;
}

#[derive(Debug, Default)]
struct RunnerSemanticSearchClient;

impl SemanticSearchClient for RunnerSemanticSearchClient {
    fn search(
        &self,
        repo_path: &Path,
        kind: KnowledgeKind,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SemanticSearchHit>, String> {
        let action = match kind {
            KnowledgeKind::Issue => "search-issues",
            KnowledgeKind::Spec => "search-specs",
            KnowledgeKind::Pr => return Ok(Vec::new()),
        };
        let repo_hash = crate::index_worker::detect_repo_hash(repo_path)
            .ok_or_else(|| "semantic search requires a git origin remote".to_string())?;
        gwt_core::runtime::ensure_project_index_runtime().map_err(|error| error.to_string())?;
        let output =
            gwt_core::process::hidden_command(crate::index_worker::project_index_python_path())
                .arg(gwt_core::paths::gwt_runtime_runner_path())
                .arg("--action")
                .arg(action)
                .arg("--repo-hash")
                .arg(repo_hash.as_str())
                .arg("--project-root")
                .arg(repo_path)
                .arg("--query")
                .arg(query)
                .arg("--n-results")
                .arg(limit.to_string())
                .current_dir(repo_path)
                .output()
                .map_err(|error| format!("run semantic search: {error}"))?;
        if !output.status.success() {
            return Err(format_runner_failure(&output));
        }
        let payload: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("parse semantic search result: {error}"))?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(payload_error(&payload));
        }
        Ok(match kind {
            KnowledgeKind::Issue => payload
                .get("issueResults")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            Some(SemanticSearchHit {
                                number: value_u64(item.get("number")?)?,
                                distance: item.get("distance").and_then(Value::as_f64),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            KnowledgeKind::Spec => payload
                .get("specResults")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            Some(SemanticSearchHit {
                                number: value_u64(item.get("spec_id")?)?,
                                distance: item.get("distance").and_then(Value::as_f64),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            KnowledgeKind::Pr => Vec::new(),
        })
    }
}

pub fn load_knowledge_bridge(
    repo_path: &Path,
    kind: KnowledgeKind,
    selected_number: Option<u64>,
    refresh: bool,
) -> Result<KnowledgeBridgeView, String> {
    if !repo_path.is_dir() {
        return Err(format!(
            "project root is not available: {}",
            repo_path.display()
        ));
    }

    if matches!(kind, KnowledgeKind::Pr) {
        return Ok(disabled_pr_view());
    }

    if issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(non_repo_view(kind));
    }

    if refresh {
        refresh_knowledge_bridge_cache(repo_path, true)?;
    }
    let entries = load_local_cache_entries_for_repo(repo_path)?;
    let linked_branches = load_linked_branches(repo_path);
    Ok(match kind {
        KnowledgeKind::Issue => build_issue_view(entries, linked_branches, selected_number),
        KnowledgeKind::Spec => build_spec_view(entries, linked_branches, selected_number),
        KnowledgeKind::Pr => disabled_pr_view(),
    })
}

pub fn refresh_knowledge_bridge_cache(repo_path: &Path, force: bool) -> Result<bool, String> {
    if !repo_path.is_dir() || issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(false);
    }
    let cache_root = issue_cache_root_for_repo_path_or_detached(repo_path);
    if force {
        sync_issue_cache_from_remote(repo_path, &cache_root)?;
        Ok(true)
    } else {
        sync_issue_cache_from_remote_if_stale(repo_path, &cache_root, ISSUE_CACHE_TTL)
    }
}

pub fn search_knowledge_bridge(
    repo_path: &Path,
    kind: KnowledgeKind,
    query: &str,
    selected_number: Option<u64>,
) -> Result<KnowledgeBridgeView, String> {
    search_knowledge_bridge_with_client(
        repo_path,
        kind,
        query,
        selected_number,
        &RunnerSemanticSearchClient,
    )
}

/// SPEC-2017 US-8 — Apply a Kanban phase change to the GitHub Issue
/// owning `issue_number` and return the freshly-rebuilt
/// [`KnowledgeListItem`].
///
/// `target_phase` semantics:
/// - `None` → remove every `phase/*` label (Backlog drop)
/// - `Some(canonical)` → ensure exactly the matching `phase/<canonical>`
///   label is set, removing any other `phase/*` labels first
///
/// The function shells out to `gh issue edit --add-label / --remove-label`
/// (matching the existing `sync_issue_cache_from_remote` pattern) and
/// updates the local Issue cache via [`Cache::apply_phase_change`] so
/// subsequent [`load_knowledge_bridge`] calls reflect the change without
/// waiting for a full refresh.
///
/// Returns the rebuilt [`KnowledgeListItem`] on success, or a human-
/// readable error string on failure (network, permission, unknown phase,
/// missing cache entry).
pub fn update_knowledge_phase(
    repo_path: &Path,
    issue_number: u64,
    target_phase: Option<&str>,
) -> Result<KnowledgeListItem, String> {
    update_knowledge_phase_with_label_writer(
        repo_path,
        issue_number,
        target_phase,
        |labels_to_add, labels_to_remove| {
            crate::issue_cache::write_issue_labels_via_gh(
                repo_path,
                issue_number,
                labels_to_add,
                labels_to_remove,
            )
        },
    )
}

/// Internal seam that lets unit tests substitute a fake label writer
/// for the gh CLI shell-out. Production callers go through
/// [`update_knowledge_phase`] which always wires up the gh writer.
pub(crate) fn update_knowledge_phase_with_label_writer<F>(
    repo_path: &Path,
    issue_number: u64,
    target_phase: Option<&str>,
    label_writer: F,
) -> Result<KnowledgeListItem, String>
where
    F: FnOnce(&[String], &[String]) -> Result<(), String>,
{
    if let Some(value) = target_phase {
        if !KNOWLEDGE_PHASE_LABELS.contains(&value) {
            return Err(format!(
                "unknown phase '{value}' (expected one of {:?})",
                KNOWLEDGE_PHASE_LABELS
            ));
        }
    }
    let cache_root = issue_cache_root_for_repo_path_or_detached(repo_path);
    let cache = Cache::new(cache_root);
    let entry = cache
        .load_entry(gwt_github::IssueNumber(issue_number))
        .ok_or_else(|| format!("Issue #{issue_number} not in local cache"))?;
    let target_label = target_phase.map(|value| format!("phase/{value}"));
    let labels_to_remove: Vec<String> = entry
        .snapshot
        .labels
        .iter()
        .filter(|label| {
            label.starts_with("phase/") && Some(label.as_str()) != target_label.as_deref()
        })
        .cloned()
        .collect();
    let labels_to_add: Vec<String> = target_label
        .as_ref()
        .filter(|target| !entry.snapshot.labels.iter().any(|label| label == *target))
        .cloned()
        .into_iter()
        .collect();
    if !labels_to_add.is_empty() || !labels_to_remove.is_empty() {
        label_writer(&labels_to_add, &labels_to_remove)?;
    }
    let mut updated_labels: Vec<String> = entry
        .snapshot
        .labels
        .iter()
        .filter(|label| !labels_to_remove.contains(label))
        .cloned()
        .collect();
    for label in &labels_to_add {
        if !updated_labels.contains(label) {
            updated_labels.push(label.clone());
        }
    }
    cache
        .apply_phase_change(gwt_github::IssueNumber(issue_number), updated_labels)
        .map_err(|error| format!("apply phase change to cache: {error}"))?;
    let refreshed = cache
        .load_entry(gwt_github::IssueNumber(issue_number))
        .ok_or_else(|| format!("Issue #{issue_number} disappeared after cache update"))?;
    let linked_branches: HashMap<u64, Vec<String>> = HashMap::new();
    Ok(
        if refreshed.snapshot.labels.contains(&SPEC_LABEL.to_string()) {
            spec_list_item(&refreshed, &linked_branches, None)
        } else {
            issue_list_item(&refreshed, &linked_branches, None)
        },
    )
}

pub(crate) fn search_knowledge_bridge_with_client<C: SemanticSearchClient + ?Sized>(
    repo_path: &Path,
    kind: KnowledgeKind,
    query: &str,
    selected_number: Option<u64>,
    client: &C,
) -> Result<KnowledgeBridgeView, String> {
    let query = query.trim();
    if query.is_empty() {
        return load_knowledge_bridge(repo_path, kind, selected_number, false);
    }
    if !repo_path.is_dir() {
        return Err(format!(
            "project root is not available: {}",
            repo_path.display()
        ));
    }
    if matches!(kind, KnowledgeKind::Pr) {
        return Ok(disabled_pr_view());
    }
    if issue_cache_root_for_repo_path(repo_path).is_none() {
        return Ok(non_repo_view(kind));
    }

    let mut entries = load_local_cache_entries_for_repo(repo_path)?
        .into_iter()
        .filter(|entry| candidate_matches_kind(entry, kind))
        .collect::<Vec<_>>();
    entries.sort_by(issue_entry_sort);
    let linked_branches = load_linked_branches(repo_path);
    let hits = client.search(repo_path, kind, query, KNOWLEDGE_SEARCH_RESULT_LIMIT)?;

    let mut seen = HashSet::new();
    let mut list_items = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| is_exact_search_match(entry, query))
    {
        if seen.insert(entry.snapshot.number.0) {
            list_items.push(list_item_for_kind(kind, entry, &linked_branches, Some(100)));
        }
    }

    let entries_by_number = entries
        .iter()
        .map(|entry| (entry.snapshot.number.0, entry))
        .collect::<HashMap<_, _>>();
    for hit in hits {
        if !seen.insert(hit.number) {
            continue;
        }
        let Some(entry) = entries_by_number.get(&hit.number) else {
            continue;
        };
        list_items.push(list_item_for_kind(
            kind,
            entry,
            &linked_branches,
            hit.distance.map(distance_to_match_score),
        ));
        if list_items.len() >= KNOWLEDGE_SEARCH_RESULT_LIMIT {
            break;
        }
    }

    let selected_number = selected_number
        .filter(|selected| list_items.iter().any(|entry| entry.number == *selected))
        .or_else(|| list_items.first().map(|entry| entry.number));
    let detail = selected_number
        .and_then(|selected| entries_by_number.get(&selected).copied())
        .map(|entry| detail_for_kind(kind, entry, &linked_branches))
        .unwrap_or_else(|| empty_detail(search_empty_title(kind), "No semantic matches found."));

    Ok(KnowledgeBridgeView {
        kind,
        entries: list_items,
        selected_number,
        empty_message: if selected_number.is_none() {
            Some("No semantic matches found.".to_string())
        } else {
            None
        },
        refresh_enabled: true,
        detail,
    })
}

fn load_local_cache_entries_for_repo(repo_path: &Path) -> Result<Vec<CacheEntry>, String> {
    let cache_root = issue_cache_root_for_repo_path_or_detached(repo_path);
    let cache = Cache::new(cache_root);
    load_cache_entries(&cache)
}

fn load_cache_entries(cache: &Cache) -> Result<Vec<CacheEntry>, String> {
    match cache.list_entries() {
        Ok(entries) => Ok(entries),
        Err(gwt_github::CacheError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(Vec::new())
        }
        Err(error) => Err(format!("failed to read issue cache: {error}")),
    }
}

fn build_issue_view(
    mut entries: Vec<CacheEntry>,
    linked_branches: HashMap<u64, Vec<String>>,
    selected_number: Option<u64>,
) -> KnowledgeBridgeView {
    entries.retain(|entry| !is_spec_entry(entry));
    entries.sort_by(issue_entry_sort);

    let list_items = entries
        .iter()
        .map(|entry| issue_list_item(entry, &linked_branches, None))
        .collect::<Vec<_>>();
    let selected_number = resolve_selected_number(&entries, selected_number);
    let detail = entries
        .iter()
        .find(|entry| Some(entry.snapshot.number.0) == selected_number)
        .map(|entry| issue_detail_view(entry, linked_branches.get(&entry.snapshot.number.0)))
        .unwrap_or_else(|| empty_detail("Issue Bridge", "No cached issues available."));

    KnowledgeBridgeView {
        kind: KnowledgeKind::Issue,
        entries: list_items,
        selected_number,
        empty_message: if selected_number.is_none() {
            Some("No cached issues. Use Refresh to sync the cache.".to_string())
        } else {
            None
        },
        refresh_enabled: true,
        detail,
    }
}

fn build_spec_view(
    mut entries: Vec<CacheEntry>,
    linked_branches: HashMap<u64, Vec<String>>,
    selected_number: Option<u64>,
) -> KnowledgeBridgeView {
    entries.retain(is_spec_entry);
    entries.sort_by(issue_entry_sort);

    let list_items = entries
        .iter()
        .map(|entry| spec_list_item(entry, &linked_branches, None))
        .collect::<Vec<_>>();
    let selected_number = resolve_selected_number(&entries, selected_number);
    let detail = entries
        .iter()
        .find(|entry| Some(entry.snapshot.number.0) == selected_number)
        .map(spec_detail_view)
        .unwrap_or_else(|| empty_detail("SPEC Bridge", "No cached SPECs available."));

    KnowledgeBridgeView {
        kind: KnowledgeKind::Spec,
        entries: list_items,
        selected_number,
        empty_message: if selected_number.is_none() {
            Some("No cached SPECs. Use Refresh to sync the cache.".to_string())
        } else {
            None
        },
        refresh_enabled: true,
        detail,
    }
}

fn disabled_pr_view() -> KnowledgeBridgeView {
    KnowledgeBridgeView {
        kind: KnowledgeKind::Pr,
        entries: Vec::new(),
        selected_number: None,
        empty_message: Some(
            "PR Bridge is waiting for cache-backed PR list support before it can render data."
                .to_string(),
        ),
        refresh_enabled: false,
        detail: KnowledgeDetailView {
            number: None,
            title: "PR Bridge".to_string(),
            subtitle: "Unavailable".to_string(),
            state: "unavailable".to_string(),
            labels: Vec::new(),
            sections: vec![KnowledgeDetailSection {
                title: "Status".to_string(),
                body: "PR Bridge is waiting for cache-backed PR list support before it can render data."
                    .to_string(),
            }],
            launch_issue_number: None,
        },
    }
}

fn non_repo_view(kind: KnowledgeKind) -> KnowledgeBridgeView {
    let title = match kind {
        KnowledgeKind::Issue => "Issue Bridge",
        KnowledgeKind::Spec => "SPEC Bridge",
        KnowledgeKind::Pr => "PR Bridge",
    };
    KnowledgeBridgeView {
        kind,
        entries: Vec::new(),
        selected_number: None,
        empty_message: Some("Knowledge Bridge is available only for Git projects.".to_string()),
        refresh_enabled: false,
        detail: empty_detail(
            title,
            "Knowledge Bridge is available only for Git projects.",
        ),
    }
}

fn empty_detail(title: &str, body: &str) -> KnowledgeDetailView {
    KnowledgeDetailView {
        number: None,
        title: title.to_string(),
        subtitle: String::new(),
        state: "idle".to_string(),
        labels: Vec::new(),
        sections: vec![KnowledgeDetailSection {
            title: "Status".to_string(),
            body: body.to_string(),
        }],
        launch_issue_number: None,
    }
}

fn candidate_matches_kind(entry: &CacheEntry, kind: KnowledgeKind) -> bool {
    match kind {
        KnowledgeKind::Issue => !is_spec_entry(entry),
        KnowledgeKind::Spec => is_spec_entry(entry),
        KnowledgeKind::Pr => false,
    }
}

fn list_item_for_kind(
    kind: KnowledgeKind,
    entry: &CacheEntry,
    linked_branches: &HashMap<u64, Vec<String>>,
    match_score: Option<u8>,
) -> KnowledgeListItem {
    match kind {
        KnowledgeKind::Issue => issue_list_item(entry, linked_branches, match_score),
        KnowledgeKind::Spec => spec_list_item(entry, linked_branches, match_score),
        KnowledgeKind::Pr => unreachable!("PR bridge has no list items"),
    }
}

fn issue_list_item(
    entry: &CacheEntry,
    linked_branches: &HashMap<u64, Vec<String>>,
    match_score: Option<u8>,
) -> KnowledgeListItem {
    let phase_info = extract_phase(&entry.snapshot.labels);
    KnowledgeListItem {
        number: entry.snapshot.number.0,
        title: entry.snapshot.title.clone(),
        state: issue_state_label(entry.snapshot.state),
        meta: format!("Updated {}", short_updated_at(&entry.snapshot.updated_at.0)),
        labels: entry.snapshot.labels.clone(),
        linked_branch_count: linked_branches
            .get(&entry.snapshot.number.0)
            .map(Vec::len)
            .unwrap_or_default(),
        match_score,
        phase: phase_info.phase,
        has_unknown_phase: phase_info.has_unknown_phase,
        is_spec: phase_info.is_spec,
    }
}

fn spec_list_item(
    entry: &CacheEntry,
    linked_branches: &HashMap<u64, Vec<String>>,
    match_score: Option<u8>,
) -> KnowledgeListItem {
    let phase_info = extract_phase(&entry.snapshot.labels);
    KnowledgeListItem {
        number: entry.snapshot.number.0,
        title: entry.snapshot.title.clone(),
        state: issue_state_label(entry.snapshot.state),
        meta: spec_list_meta(entry),
        labels: entry.snapshot.labels.clone(),
        linked_branch_count: linked_branches
            .get(&entry.snapshot.number.0)
            .map(Vec::len)
            .unwrap_or_default(),
        match_score,
        phase: phase_info.phase,
        has_unknown_phase: phase_info.has_unknown_phase,
        is_spec: phase_info.is_spec,
    }
}

fn detail_for_kind(
    kind: KnowledgeKind,
    entry: &CacheEntry,
    linked_branches: &HashMap<u64, Vec<String>>,
) -> KnowledgeDetailView {
    match kind {
        KnowledgeKind::Issue => {
            issue_detail_view(entry, linked_branches.get(&entry.snapshot.number.0))
        }
        KnowledgeKind::Spec => spec_detail_view(entry),
        KnowledgeKind::Pr => disabled_pr_view().detail,
    }
}

fn is_exact_search_match(entry: &CacheEntry, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    let query_lower = query.to_lowercase();
    let number = entry.snapshot.number.0.to_string();
    if query_lower.strip_prefix('#') == Some(number.as_str()) || query_lower == number {
        return true;
    }
    if entry.snapshot.title.to_lowercase() == query_lower {
        return true;
    }
    entry
        .snapshot
        .labels
        .iter()
        .any(|label| label.to_lowercase() == query_lower)
}

fn distance_to_match_score(distance: f64) -> u8 {
    ((1.0 - distance) * 100.0).round().clamp(0.0, 100.0) as u8
}

fn search_empty_title(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Issue => "Issue Search",
        KnowledgeKind::Spec => "SPEC Search",
        KnowledgeKind::Pr => "PR Bridge",
    }
}

fn issue_detail_view(
    entry: &CacheEntry,
    linked_branches: Option<&Vec<String>>,
) -> KnowledgeDetailView {
    let mut sections = Vec::new();
    let body = entry.snapshot.body.trim();
    if !body.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Description".to_string(),
            body: body.to_string(),
        });
    }
    for (index, comment) in entry.snapshot.comments.iter().enumerate() {
        let comment_body = comment.body.trim();
        if comment_body.is_empty() {
            continue;
        }
        sections.push(KnowledgeDetailSection {
            title: format!("Comment {}", index + 1),
            body: comment_body.to_string(),
        });
    }
    if let Some(branches) = linked_branches.filter(|branches| !branches.is_empty()) {
        sections.push(KnowledgeDetailSection {
            title: "Linked branches".to_string(),
            body: linked_branches_markdown(branches),
        });
    }
    if sections.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Status".to_string(),
            body: "No cached issue details available.".to_string(),
        });
    }

    KnowledgeDetailView {
        number: Some(entry.snapshot.number.0),
        title: entry.snapshot.title.clone(),
        subtitle: format!(
            "#{} · {} · Updated {}",
            entry.snapshot.number.0,
            issue_state_label(entry.snapshot.state),
            short_updated_at(&entry.snapshot.updated_at.0)
        ),
        state: issue_state_label(entry.snapshot.state),
        labels: entry.snapshot.labels.clone(),
        sections,
        launch_issue_number: Some(entry.snapshot.number.0),
    }
}

fn linked_branches_markdown(branches: &[String]) -> String {
    branches
        .iter()
        .map(|branch| format!("- `{}`", branch.replace('`', "\\`")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn spec_detail_view(entry: &CacheEntry) -> KnowledgeDetailView {
    let mut sections = Vec::new();
    for name in ["spec", "plan", "tasks"] {
        if let Some(body) = entry.spec_body.sections.get(&SectionName(name.to_string())) {
            if !body.trim().is_empty() {
                sections.push(KnowledgeDetailSection {
                    title: name.to_string(),
                    body: body.trim().to_string(),
                });
            }
        }
    }
    for (name, body) in &entry.spec_body.sections {
        if matches!(name.0.as_str(), "spec" | "plan" | "tasks") || body.trim().is_empty() {
            continue;
        }
        sections.push(KnowledgeDetailSection {
            title: name.0.clone(),
            body: body.trim().to_string(),
        });
    }
    if sections.is_empty() {
        sections.push(KnowledgeDetailSection {
            title: "Status".to_string(),
            body: "No cached SPEC sections available.".to_string(),
        });
    }

    let phase = entry
        .snapshot
        .labels
        .iter()
        .find(|label| label.starts_with("phase/"))
        .cloned()
        .unwrap_or_else(|| "phase/unspecified".to_string());
    KnowledgeDetailView {
        number: Some(entry.snapshot.number.0),
        title: entry.snapshot.title.clone(),
        subtitle: format!(
            "#{} · {} · Updated {}",
            entry.snapshot.number.0,
            phase,
            short_updated_at(&entry.snapshot.updated_at.0)
        ),
        state: issue_state_label(entry.snapshot.state),
        labels: entry.snapshot.labels.clone(),
        sections,
        launch_issue_number: Some(entry.snapshot.number.0),
    }
}

fn spec_list_meta(entry: &CacheEntry) -> String {
    let phase = entry
        .snapshot
        .labels
        .iter()
        .find(|label| label.starts_with("phase/"))
        .cloned()
        .unwrap_or_else(|| "phase/unspecified".to_string());
    format!(
        "{phase} · Updated {}",
        short_updated_at(&entry.snapshot.updated_at.0)
    )
}

fn resolve_selected_number(entries: &[CacheEntry], selected_number: Option<u64>) -> Option<u64> {
    selected_number
        .filter(|selected| {
            entries
                .iter()
                .any(|entry| entry.snapshot.number.0 == *selected)
        })
        .or_else(|| entries.first().map(|entry| entry.snapshot.number.0))
}

fn issue_entry_sort(left: &CacheEntry, right: &CacheEntry) -> std::cmp::Ordering {
    let left_state = if left.snapshot.state == IssueState::Open {
        0
    } else {
        1
    };
    let right_state = if right.snapshot.state == IssueState::Open {
        0
    } else {
        1
    };
    left_state
        .cmp(&right_state)
        .then_with(|| right.snapshot.updated_at.0.cmp(&left.snapshot.updated_at.0))
        .then_with(|| left.snapshot.number.0.cmp(&right.snapshot.number.0))
}

fn issue_state_label(state: IssueState) -> String {
    match state {
        IssueState::Open => "open".to_string(),
        IssueState::Closed => "closed".to_string(),
    }
}

fn short_updated_at(updated_at: &str) -> String {
    updated_at.get(..10).unwrap_or(updated_at).to_string()
}

fn is_spec_entry(entry: &CacheEntry) -> bool {
    entry
        .snapshot
        .labels
        .iter()
        .any(|label| label == SPEC_LABEL)
}

fn value_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
}

fn payload_error(payload: &Value) -> String {
    payload
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| payload.get("error_code").and_then(Value::as_str))
        .unwrap_or("semantic search failed")
        .to_string()
}

fn format_runner_failure(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    if detail.is_empty() {
        format!("semantic search runner exited with {}", output.status)
    } else {
        format!(
            "semantic search runner exited with {}: {detail}",
            output.status
        )
    }
}

#[derive(Debug, Default, Deserialize)]
struct IssueBranchLinkStore {
    #[serde(default)]
    branches: HashMap<String, u64>,
}

fn load_linked_branches(repo_path: &Path) -> HashMap<u64, Vec<String>> {
    let Some(repo_hash) = crate::index_worker::detect_repo_hash(repo_path) else {
        return HashMap::new();
    };
    let path = gwt_cache_dir()
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let Ok(store) = serde_json::from_slice::<IssueBranchLinkStore>(&bytes) else {
        return HashMap::new();
    };

    let mut linked = HashMap::<u64, Vec<String>>::new();
    for (branch, issue_number) in store.branches {
        linked.entry(issue_number).or_default().push(branch);
    }
    for branches in linked.values_mut() {
        branches.sort();
    }
    linked
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, ffi::OsString, fs};

    use gwt_github::{
        client::{CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt},
        Cache,
    };

    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn init_repo(repo: &Path) {
        fs::create_dir_all(repo).expect("create repo");
        let mut init_cmd = gwt_core::process::hidden_command("git");
        init_cmd.args(["init", "--quiet"]).current_dir(repo);
        gwt_core::process::scrub_git_env(&mut init_cmd);
        let init = init_cmd.output().expect("git init");
        assert!(init.status.success(), "git init failed");

        let mut remote_cmd = gwt_core::process::hidden_command("git");
        remote_cmd
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/example/repo.git",
            ])
            .current_dir(repo);
        gwt_core::process::scrub_git_env(&mut remote_cmd);
        let remote = remote_cmd.output().expect("git remote add");
        assert!(remote.status.success(), "git remote add failed");
    }

    fn issue_snapshot(
        number: u64,
        title: &str,
        body: &str,
        labels: &[&str],
        state: IssueState,
    ) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state,
            updated_at: UpdatedAt::new("2026-04-20T12:34:56Z"),
            comments: vec![CommentSnapshot {
                id: CommentId(41),
                body: "Follow-up detail".to_string(),
                updated_at: UpdatedAt::new("2026-04-20T12:35:00Z"),
            }],
        }
    }

    fn spec_snapshot(number: u64) -> IssueSnapshot {
        issue_snapshot(
            number,
            "Coverage SPEC",
            r#"<!-- gwt-spec id=2001 version=1 -->
<!-- sections:
spec=body
plan=body
tasks=body
notes=body
-->
<!-- artifact:spec BEGIN -->
Raise project coverage to 90%.
<!-- artifact:spec END -->

<!-- artifact:plan BEGIN -->
1. Add tests.
<!-- artifact:plan END -->

<!-- artifact:tasks BEGIN -->
- [ ] Add push-time gate.
<!-- artifact:tasks END -->

<!-- artifact:notes BEGIN -->
Extra context.
<!-- artifact:notes END -->
"#,
            &["gwt-spec", "phase/in-progress"],
            IssueState::Open,
        )
    }

    fn write_issue_links(repo_path: &Path, links: &[(&str, u64)]) {
        let repo_hash = crate::index_worker::detect_repo_hash(repo_path).expect("repo hash");
        let path = gwt_cache_dir()
            .join("issue-links")
            .join(format!("{}.json", repo_hash.as_str()));
        fs::create_dir_all(path.parent().expect("issue links dir"))
            .expect("create issue-links dir");
        let branches = links
            .iter()
            .map(|(branch, issue)| ((*branch).to_string(), *issue))
            .collect::<HashMap<_, _>>();
        let bytes = serde_json::to_vec(&serde_json::json!({ "branches": branches }))
            .expect("serialize links");
        fs::write(path, bytes).expect("write links");
    }

    #[test]
    fn load_knowledge_bridge_returns_non_repo_and_disabled_pr_views() {
        let dir = tempfile::tempdir().expect("tempdir");

        let issue_view = load_knowledge_bridge(dir.path(), KnowledgeKind::Issue, None, false)
            .expect("issue view");
        assert_eq!(issue_view.kind, KnowledgeKind::Issue);
        assert!(!issue_view.refresh_enabled);
        assert_eq!(
            issue_view.empty_message.as_deref(),
            Some("Knowledge Bridge is available only for Git projects.")
        );

        let pr_view =
            load_knowledge_bridge(dir.path(), KnowledgeKind::Pr, Some(12), false).expect("pr view");
        assert_eq!(pr_view.kind, KnowledgeKind::Pr);
        assert!(!pr_view.refresh_enabled);
        assert_eq!(pr_view.detail.title, "PR Bridge");
        assert_eq!(pr_view.detail.state, "unavailable");
    }

    #[test]
    fn load_knowledge_bridge_builds_issue_and_spec_views_from_cache() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());

        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Coverage bug",
                "Need more tests.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write issue snapshot");
        cache
            .write_snapshot(&spec_snapshot(22))
            .expect("write spec snapshot");
        write_issue_links(
            &repo,
            &[
                ("feature/coverage", 11),
                ("feature/coverage-followup", 11),
                ("spec/coverage", 22),
            ],
        );

        let issue_view = load_knowledge_bridge(&repo, KnowledgeKind::Issue, Some(11), false)
            .expect("issue bridge");
        let issue_entry = issue_view
            .entries
            .iter()
            .find(|entry| entry.number == 11)
            .expect("issue entry");
        assert_eq!(issue_entry.linked_branch_count, 2);
        assert_eq!(issue_view.selected_number, Some(11));
        assert_eq!(issue_view.detail.launch_issue_number, Some(11));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Description" && section.body == "Need more tests."));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Comment 1" && section.body == "Follow-up detail"));
        assert!(issue_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "Linked branches"
                && section.body == "- `feature/coverage`\n- `feature/coverage-followup`"));

        let spec_view = load_knowledge_bridge(&repo, KnowledgeKind::Spec, Some(22), false)
            .expect("spec bridge");
        let spec_entry = spec_view
            .entries
            .iter()
            .find(|entry| entry.number == 22)
            .expect("spec entry");
        assert_eq!(spec_entry.linked_branch_count, 1);
        assert!(spec_entry.meta.contains("phase/in-progress"));
        assert_eq!(spec_view.detail.launch_issue_number, Some(22));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "spec"
                && section.body.contains("Raise project coverage")));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "plan"));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "tasks"));
        assert!(spec_view
            .detail
            .sections
            .iter()
            .any(|section| section.title == "notes"));
    }

    #[derive(Debug, Default)]
    struct FakeSemanticSearchClient {
        hits: Vec<SemanticSearchHit>,
    }

    impl SemanticSearchClient for FakeSemanticSearchClient {
        fn search(
            &self,
            _repo_path: &Path,
            _kind: KnowledgeKind,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<SemanticSearchHit>, String> {
            Ok(self.hits.clone())
        }
    }

    #[test]
    fn semantic_issue_search_filters_specs_and_scores_open_and_closed_results() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Open semantic issue",
                "Need semantic search.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write open issue");
        cache
            .write_snapshot(&issue_snapshot(
                12,
                "Closed semantic issue",
                "Already fixed.",
                &["bug"],
                IssueState::Closed,
            ))
            .expect("write closed issue");
        cache
            .write_snapshot(&spec_snapshot(22))
            .expect("write spec snapshot");

        let view = search_knowledge_bridge_with_client(
            &repo,
            KnowledgeKind::Issue,
            "semantic search",
            None,
            &FakeSemanticSearchClient {
                hits: vec![
                    SemanticSearchHit {
                        number: 22,
                        distance: Some(0.01),
                    },
                    SemanticSearchHit {
                        number: 12,
                        distance: Some(0.02),
                    },
                    SemanticSearchHit {
                        number: 11,
                        distance: Some(0.2),
                    },
                ],
            },
        )
        .expect("search view");

        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.entries[0].number, 12);
        assert_eq!(view.entries[0].state, "closed");
        assert_eq!(view.entries[0].match_score, Some(98));
        assert_eq!(view.entries[1].number, 11);
        assert_eq!(view.entries[1].state, "open");
        assert_eq!(view.entries[1].match_score, Some(80));
        assert_eq!(view.selected_number, Some(12));
    }

    #[test]
    fn semantic_issue_search_reads_cache_without_stale_remote_sync() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Open semantic issue",
                "Need semantic search.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write issue");

        let marker = home.path().join("gh-was-called");
        let fake_gh = home.path().join("fake-gh");
        fs::write(
            &fake_gh,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .expect("write fake gh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755))
                .expect("chmod fake gh");
        }
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);

        let view = search_knowledge_bridge_with_client(
            &repo,
            KnowledgeKind::Issue,
            "semantic search",
            None,
            &FakeSemanticSearchClient {
                hits: vec![SemanticSearchHit {
                    number: 11,
                    distance: Some(0.1),
                }],
            },
        )
        .expect("search view");

        assert_eq!(view.entries.len(), 1);
        assert!(
            !marker.exists(),
            "interactive semantic search must not invoke stale remote cache sync"
        );
    }

    #[test]
    fn load_knowledge_bridge_reads_local_cache_without_stale_remote_sync() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Open cache issue",
                "Opening the bridge should read this cached entry immediately.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write issue");

        let marker = home.path().join("gh-was-called");
        let fake_gh = home.path().join("fake-gh");
        fs::write(
            &fake_gh,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
        )
        .expect("write fake gh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&fake_gh, fs::Permissions::from_mode(0o755))
                .expect("chmod fake gh");
        }
        let _gh = ScopedEnvVar::set("GWT_TEST_GH", &fake_gh);

        let view = load_knowledge_bridge(&repo, KnowledgeKind::Issue, Some(11), false)
            .expect("issue bridge");

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.selected_number, Some(11));
        assert!(
            !marker.exists(),
            "opening a knowledge bridge must not invoke stale remote cache sync"
        );
    }

    #[test]
    fn semantic_spec_search_prioritizes_exact_matches_and_removes_duplicates() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);

        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("repo cache root");
        let cache = Cache::new(cache_root);
        cache
            .write_snapshot(&issue_snapshot(
                11,
                "Plain issue",
                "Not a spec.",
                &["bug"],
                IssueState::Open,
            ))
            .expect("write issue");
        cache
            .write_snapshot(&spec_snapshot(22))
            .expect("write spec");

        let view = search_knowledge_bridge_with_client(
            &repo,
            KnowledgeKind::Spec,
            "#22",
            None,
            &FakeSemanticSearchClient {
                hits: vec![
                    SemanticSearchHit {
                        number: 11,
                        distance: Some(0.01),
                    },
                    SemanticSearchHit {
                        number: 22,
                        distance: Some(0.18),
                    },
                    SemanticSearchHit {
                        number: 22,
                        distance: Some(0.2),
                    },
                ],
            },
        )
        .expect("search view");

        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].number, 22);
        assert_eq!(view.entries[0].match_score, Some(100));
        assert_eq!(view.selected_number, Some(22));
    }

    #[test]
    fn extract_phase_recognizes_canonical_phase_labels() {
        let cases = [
            ("phase/draft", "draft"),
            ("phase/planning", "planning"),
            ("phase/implementation", "implementation"),
            ("phase/review", "review"),
            ("phase/done", "done"),
        ];
        for (label, expected) in cases {
            let extracted = extract_phase(&[label.to_string()]);
            assert_eq!(
                extracted.phase.as_deref(),
                Some(expected),
                "label={}",
                label
            );
            assert!(!extracted.has_unknown_phase, "label={}", label);
            assert!(!extracted.is_spec, "label={}", label);
        }
    }

    #[test]
    fn extract_phase_returns_none_when_no_phase_labels() {
        let extracted = extract_phase(&["bug".to_string(), "documentation".to_string()]);
        assert!(extracted.phase.is_none());
        assert!(!extracted.has_unknown_phase);
        assert!(!extracted.is_spec);
    }

    #[test]
    fn extract_phase_flags_unknown_phase_label_as_warning() {
        let extracted = extract_phase(&["phase/legacy".to_string()]);
        assert!(extracted.phase.is_none());
        assert!(extracted.has_unknown_phase);
        assert!(!extracted.is_spec);
    }

    #[test]
    fn extract_phase_detects_gwt_spec_label() {
        let extracted = extract_phase(&["gwt-spec".to_string(), "phase/planning".to_string()]);
        assert_eq!(extracted.phase.as_deref(), Some("planning"));
        assert!(!extracted.has_unknown_phase);
        assert!(extracted.is_spec);
    }

    #[test]
    fn extract_phase_keeps_first_canonical_when_multiple_phase_labels() {
        let extracted = extract_phase(&[
            "phase/draft".to_string(),
            "phase/implementation".to_string(),
        ]);
        // first canonical wins; second triggers unknown flag because two
        // canonical labels at once is malformed input
        assert_eq!(extracted.phase.as_deref(), Some("draft"));
        assert!(extracted.has_unknown_phase);
    }

    // SPEC-2017 T-027 — phase write-back orchestration coverage. The
    // tests use `update_knowledge_phase_with_label_writer` to inject a
    // closure that captures (and optionally fails) the gh CLI call so
    // we don't need a live `gh` binary on the test runner.

    #[test]
    fn update_knowledge_phase_replaces_existing_phase_label() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("cache root");
        Cache::new(cache_root)
            .write_snapshot(&issue_snapshot(
                100,
                "Coverage spec",
                "Body",
                &["gwt-spec", "phase/draft"],
                IssueState::Open,
            ))
            .expect("write snapshot");

        let captured: std::cell::RefCell<Option<(Vec<String>, Vec<String>)>> =
            std::cell::RefCell::new(None);
        let result = update_knowledge_phase_with_label_writer(
            &repo,
            100,
            Some("implementation"),
            |add, remove| {
                *captured.borrow_mut() = Some((add.to_vec(), remove.to_vec()));
                Ok(())
            },
        )
        .expect("update phase");
        let snapshot = captured.into_inner().expect("label writer called");
        assert_eq!(snapshot.0, vec!["phase/implementation".to_string()]);
        assert_eq!(snapshot.1, vec!["phase/draft".to_string()]);
        assert_eq!(result.phase.as_deref(), Some("implementation"));
        assert!(result.is_spec);
        assert!(!result.has_unknown_phase);
    }

    #[test]
    fn update_knowledge_phase_to_backlog_removes_every_phase_label() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("cache root");
        Cache::new(cache_root)
            .write_snapshot(&issue_snapshot(
                200,
                "Spec to backlog",
                "Body",
                &["gwt-spec", "phase/review"],
                IssueState::Open,
            ))
            .expect("write snapshot");

        let captured: std::cell::RefCell<Option<(Vec<String>, Vec<String>)>> =
            std::cell::RefCell::new(None);
        let result = update_knowledge_phase_with_label_writer(&repo, 200, None, |add, remove| {
            *captured.borrow_mut() = Some((add.to_vec(), remove.to_vec()));
            Ok(())
        })
        .expect("update phase");
        let snapshot = captured.into_inner().expect("label writer called");
        assert!(
            snapshot.0.is_empty(),
            "Backlog drop must not add any phase label"
        );
        assert_eq!(snapshot.1, vec!["phase/review".to_string()]);
        assert!(result.phase.is_none());
    }

    #[test]
    fn update_knowledge_phase_rejects_unknown_target() {
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        let result = update_knowledge_phase_with_label_writer(
            &repo,
            999,
            Some("legacy"),
            |_add, _remove| panic!("label writer must not be invoked"),
        );
        let err = result.expect_err("unknown phase target should error");
        assert!(err.contains("unknown phase"), "got: {err}");
    }

    #[test]
    fn update_knowledge_phase_propagates_label_writer_failure() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("cache root");
        Cache::new(cache_root)
            .write_snapshot(&issue_snapshot(
                300,
                "Failing spec",
                "Body",
                &["gwt-spec", "phase/draft"],
                IssueState::Open,
            ))
            .expect("write snapshot");

        let result = update_knowledge_phase_with_label_writer(
            &repo,
            300,
            Some("planning"),
            |_add, _remove| Err("gh issue edit #300: 422 Unprocessable Entity".to_string()),
        );
        let err = result.expect_err("label writer failure must surface");
        assert!(err.contains("422"), "got: {err}");
        // Cache must NOT be updated when the GitHub call failed —
        // otherwise the local cache drifts away from the source of truth.
        let cache_root =
            crate::issue_cache::issue_cache_root_for_repo_path(&repo).expect("cache root");
        let entry = Cache::new(cache_root)
            .load_entry(gwt_github::IssueNumber(300))
            .expect("entry exists");
        assert_eq!(
            entry.snapshot.labels,
            vec!["gwt-spec".to_string(), "phase/draft".to_string()],
            "labels must remain unchanged after writer failure",
        );
    }

    #[test]
    fn update_knowledge_phase_reports_missing_cache_entry() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().expect("tempdir");
        let _home = ScopedEnvVar::set("HOME", home.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.path());
        let repo = home.path().join("repo");
        init_repo(&repo);
        let result =
            update_knowledge_phase_with_label_writer(&repo, 404, Some("draft"), |_add, _remove| {
                panic!("label writer must not run when cache miss")
            });
        let err = result.expect_err("missing cache entry must error");
        assert!(err.contains("not in local cache"), "got: {err}");
    }
}

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{
    protocol::{IndexSearchResult, IndexSearchScope, IndexSearchTarget},
    worktree_inventory,
};

const INDEX_SEARCH_LIMIT: usize = 50;

pub fn search_project_index(
    project_root: &Path,
    query: &str,
    scopes: &[IndexSearchScope],
    selected_worktree_hash: Option<&str>,
) -> Result<Vec<IndexSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let repo_hash = crate::index_worker::detect_repo_hash(project_root)
        .ok_or_else(|| "project index search requires a git origin remote".to_string())?;
    gwt_core::runtime::ensure_project_index_runtime().map_err(|error| error.to_string())?;

    let effective_scopes = if scopes.is_empty() {
        default_index_search_scopes()
    } else {
        scopes.to_vec()
    };
    let file_worktree = resolve_file_search_worktree(project_root, selected_worktree_hash)?;
    let board_scope = crate::board_audience::gui_default_board_scope(project_root)
        .unwrap_or(gwt_core::coordination::BoardAudienceScope::All);

    let mut results = Vec::new();
    let per_scope_limit = INDEX_SEARCH_LIMIT;
    for scope in effective_scopes {
        let (search_root, worktree_hash) = match scope {
            IndexSearchScope::Files | IndexSearchScope::FilesDocs => (
                file_worktree.path.as_path(),
                Some(file_worktree.hash.as_str()),
            ),
            _ => (project_root, None),
        };
        let payload = run_scope_search(
            search_root,
            repo_hash.as_str(),
            worktree_hash,
            scope,
            query,
            per_scope_limit,
        )?;
        append_scope_results(&mut results, scope, &payload, &board_scope);
    }

    results.sort_by(|left, right| distance_key(left).total_cmp(&distance_key(right)));
    results.truncate(INDEX_SEARCH_LIMIT);
    Ok(results)
}

fn default_index_search_scopes() -> Vec<IndexSearchScope> {
    vec![
        IndexSearchScope::Issues,
        IndexSearchScope::Specs,
        IndexSearchScope::Lessons,
        IndexSearchScope::Board,
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
    if let Some(hash) = selected_worktree_hash
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
    {
        let entries = worktree_inventory::enumerate_worktrees(project_root, Some(project_root))
            .map_err(|error| error.to_string())?;
        let entry = entries
            .into_iter()
            .find(|entry| entry.id == hash)
            .ok_or_else(|| format!("worktree with hash {hash} not found"))?;
        return Ok(FileSearchWorktree {
            path: entry.path,
            hash: hash.to_string(),
        });
    }
    let hash = gwt_core::worktree_hash::compute_worktree_hash(project_root)
        .map_err(|error| error.to_string())?
        .to_string();
    Ok(FileSearchWorktree {
        path: project_root.to_path_buf(),
        hash,
    })
}

fn run_scope_search(
    project_root: &Path,
    repo_hash: &str,
    worktree_hash: Option<&str>,
    scope: IndexSearchScope,
    query: &str,
    limit: usize,
) -> Result<Value, String> {
    let output =
        gwt_core::process::hidden_command(crate::index_worker::project_index_python_path())
            .arg(gwt_core::paths::gwt_runtime_runner_path())
            .arg("--action")
            .arg(search_action(scope))
            .arg("--repo-hash")
            .arg(repo_hash)
            .arg("--project-root")
            .arg(project_root)
            .arg("--query")
            .arg(query)
            .arg("--n-results")
            .arg(limit.to_string())
            .args(
                worktree_hash
                    .map(|hash| vec!["--worktree-hash".to_string(), hash.to_string()])
                    .unwrap_or_default(),
            )
            .current_dir(project_root)
            .output()
            .map_err(|error| format!("run project index search: {error}"))?;
    if !output.status.success() {
        return Err(format_runner_failure(&output));
    }
    let payload: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("parse project index search result: {error}"))?;
    if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Err(payload_error(&payload));
    }
    Ok(payload)
}

fn search_action(scope: IndexSearchScope) -> &'static str {
    match scope {
        IndexSearchScope::Issues => "search-issues",
        IndexSearchScope::Specs => "search-specs",
        IndexSearchScope::Lessons => "search-lessons",
        IndexSearchScope::Board => "search-board",
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
        IndexSearchScope::Lessons => "lessonResults",
        IndexSearchScope::Board => "boardResults",
        IndexSearchScope::Files | IndexSearchScope::FilesDocs => "results",
    };
    let Some(items) = payload.get(key).and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let result = match scope {
            IndexSearchScope::Issues => issue_result(item),
            IndexSearchScope::Specs => spec_result(item),
            IndexSearchScope::Lessons => lesson_result(item),
            IndexSearchScope::Board => board_result(item, board_scope),
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
        target: IndexSearchTarget::Spec { spec_id },
    })
}

fn lesson_result(item: &Value) -> Option<IndexSearchResult> {
    let heading = value_str(item.get("heading"))?;
    let title = value_str(item.get("title")).unwrap_or_else(|| heading.clone());
    let date = value_str(item.get("date")).unwrap_or_default();
    Some(IndexSearchResult {
        scope: IndexSearchScope::Lessons,
        title,
        subtitle: if date.is_empty() {
            "lesson".to_string()
        } else {
            format!("lesson · {date}")
        },
        preview: heading.clone(),
        distance: item.get("distance").and_then(Value::as_f64),
        target: IndexSearchTarget::Lesson { heading, date },
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
        target: IndexSearchTarget::File { path },
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::coordination::BoardAudienceScope;
    use serde_json::json;

    #[test]
    fn empty_index_search_query_returns_no_results_without_runtime() {
        let results = search_project_index(Path::new("/definitely/not/a/repo"), "   ", &[], None)
            .expect("empty query should short-circuit");

        assert!(results.is_empty());
    }

    #[test]
    fn default_index_search_scopes_cover_all_user_visible_sources() {
        assert_eq!(
            default_index_search_scopes(),
            vec![
                IndexSearchScope::Issues,
                IndexSearchScope::Specs,
                IndexSearchScope::Lessons,
                IndexSearchScope::Board,
                IndexSearchScope::Files,
                IndexSearchScope::FilesDocs,
            ]
        );
    }

    #[test]
    fn append_scope_results_formats_issue_spec_lesson_and_file_targets() {
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
            IndexSearchScope::Lessons,
            &json!({
                "lessonResults": [{
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

        assert_eq!(results.len(), 4);
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
        assert_eq!(results[2].subtitle, "lesson · 2026-05-20");
        assert!(matches!(
            results[2].target,
            IndexSearchTarget::Lesson { .. }
        ));
        assert_eq!(results[3].title, "README.md");
        assert_eq!(results[3].subtitle, "Markdown");
        assert!(matches!(results[3].target, IndexSearchTarget::File { .. }));
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
}

//! Project version history (tag-based) summarization for the GUI.
//!
//! This feature does NOT read CHANGELOG.md. It derives version ranges from Git tags
//! (v*), generates a simple grouped changelog from commit subjects, and (when AI is
//! configured) generates a summary in the configured language.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use gwt_core::{
    ai::{format_error_for_display, AIClient, AIError, ChatMessage},
    config::ProfilesConfig,
    git::Remote,
    StructuredError,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tracing::instrument;

use crate::{
    commands::project::resolve_repo_path_for_project_root,
    state::{AppState, VersionHistoryCacheEntry},
};

/// Compute the cache file path for a given repo path.
///
/// Path: `~/.gwt/cache/version-history/{hash}.json`
/// where `hash` = first 16 hex chars of SHA-256(canonical repo path).
fn cache_file_path(repo_path: &Path) -> PathBuf {
    let canonical = dunce::canonicalize(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hash = format!("{digest:x}");
    let short_hash = &hash[..16];

    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".gwt")
        .join("cache")
        .join("version-history")
        .join(format!("{short_hash}.json"))
}

/// Load disk cache for a repo. Returns None if the file does not exist or contains invalid JSON.
fn load_disk_cache(repo_path: &Path) -> Option<HashMap<String, VersionHistoryCacheEntry>> {
    let path = cache_file_path(repo_path);
    let data = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save the in-memory cache entries to disk for a repo, creating directories as needed.
fn save_disk_cache(repo_path: &Path, entries: &HashMap<String, VersionHistoryCacheEntry>) {
    let path = cache_file_path(repo_path);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(&path, json);
    }
}

const VERSION_ID_UNRELEASED: &str = "unreleased";
const RANGE_OID_UNBORN_HEAD: &str = "UNBORN_HEAD";

const MAX_SUBJECTS_FOR_CHANGELOG: usize = 400;
const MAX_SUBJECTS_FOR_AI: usize = 120;
const MAX_CHANGELOG_LINES_PER_GROUP: usize = 20;
const MAX_PROMPT_CHARS: usize = 12000;

fn normalize_version_history_language(value: &str) -> &'static str {
    match value.trim() {
        "ja" => "ja",
        _ => "en", // "en" or "auto" (or any invalid value) -> English
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectVersions {
    pub items: Vec<VersionItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionItem {
    pub id: String, // "unreleased" | "vX.Y.Z"
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String, // "HEAD" | "vX.Y.Z"
    pub commit_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionHistoryResult {
    /// "ok" | "generating" | "error" | "disabled"
    pub status: String,
    pub version_id: String,
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String,
    pub commit_count: u32,
    pub summary_markdown: Option<String>,
    pub changelog_markdown: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionHistoryUpdatedPayload {
    pub project_path: String,
    pub version_id: String,
    pub result: VersionHistoryResult,
}

#[instrument(skip_all, fields(command = "list_project_versions", project_path))]
#[tauri::command]
pub fn list_project_versions(
    project_path: String,
    limit: usize,
) -> Result<ProjectVersions, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_project_versions"))?;

    let limit = limit.max(1);
    let tag_limit = limit.saturating_sub(1);

    // Best-effort tag refresh to avoid stale local tag lists.
    // Failures (offline/no remote/etc.) are intentionally ignored.
    refresh_version_tags_from_default_remote(&repo_path);

    // Fetch an extra tag so each displayed tag can have a "previous" range bound.
    let tags = list_version_tags(&repo_path, Some(tag_limit.saturating_add(1)))
        .map_err(|e| StructuredError::internal(&e, "list_project_versions"))?;

    let mut items = Vec::new();

    // Unreleased range: latest tag -> HEAD (or entire history if no tags exist)
    {
        let latest = tags.first().cloned();
        let commit_count = rev_list_count(&repo_path, latest.as_deref(), "HEAD")
            .map_err(|e| StructuredError::internal(&e, "list_project_versions"))?;
        items.push(VersionItem {
            id: VERSION_ID_UNRELEASED.to_string(),
            label: "Unreleased (HEAD)".to_string(),
            range_from: latest,
            range_to: "HEAD".to_string(),
            commit_count,
        });
    }

    for (idx, tag) in tags.iter().take(tag_limit).enumerate() {
        let prev = tags.get(idx + 1).cloned();
        let commit_count = rev_list_count(&repo_path, prev.as_deref(), tag.as_str())
            .map_err(|e| StructuredError::internal(&e, "list_project_versions"))?;
        items.push(VersionItem {
            id: tag.to_string(),
            label: tag.to_string(),
            range_from: prev,
            range_to: tag.to_string(),
            commit_count,
        });
    }

    Ok(ProjectVersions { items })
}

#[instrument(
    skip_all,
    fields(command = "get_project_version_history", project_path)
)]
#[tauri::command]
pub fn get_project_version_history(
    project_path: String,
    version_id: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<VersionHistoryResult, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_project_version_history"))?;
    let repo_key = repo_path.to_string_lossy().to_string();

    let profiles = ProfilesConfig::load()
        .map_err(|e| StructuredError::internal(&e.to_string(), "get_project_version_history"))?;
    let settings = match resolve_version_history_ai_settings(&profiles) {
        Some(settings) => settings,
        None => return Ok(disabled_version_history_result(&version_id)),
    };

    // Resolve the requested version into a concrete git range.
    let (label, range_from, range_to) = resolve_range_for_version(&repo_path, &version_id)
        .map_err(|e| StructuredError::internal(&e, "get_project_version_history"))?;
    let commit_count = rev_list_count(&repo_path, range_from.as_deref(), &range_to)
        .map_err(|e| StructuredError::internal(&e, "get_project_version_history"))?;

    let range_from_oid = match &range_from {
        Some(r) => Some(
            rev_parse(&repo_path, r)
                .map_err(|e| StructuredError::internal(&e, "get_project_version_history"))?,
        ),
        None => None,
    };
    let range_to_oid = match rev_parse(&repo_path, &range_to) {
        Ok(v) => v,
        Err(err) => {
            if range_to == "HEAD" && is_unborn_head(&repo_path) {
                RANGE_OID_UNBORN_HEAD.to_string()
            } else {
                return Err(StructuredError::internal(
                    &err,
                    "get_project_version_history",
                ));
            }
        }
    };

    let language = normalize_version_history_language(&settings.language);

    // Cache hit
    if let Some(hit) = get_cached_version_history(
        &state,
        &repo_path,
        &repo_key,
        &version_id,
        range_from_oid.as_deref(),
        &range_to_oid,
        language,
    ) {
        return Ok(VersionHistoryResult {
            status: "ok".to_string(),
            version_id,
            label: hit.label,
            range_from: hit.range_from,
            range_to: hit.range_to,
            commit_count: hit.commit_count,
            summary_markdown: Some(hit.summary_markdown),
            changelog_markdown: Some(hit.changelog_markdown),
            error: None,
        });
    }

    // Build simple changelog for immediate "generating" response (FR-008).
    let generating_changelog = git_log_subjects(
        &repo_path,
        range_from.as_deref(),
        &range_to,
        MAX_SUBJECTS_FOR_CHANGELOG,
    )
    .ok()
    .map(|subjects| build_simple_changelog_markdown(&subjects, language));

    // Start background job (best-effort)
    // Include language so switching output language can spawn a fresh job
    // without being blocked by an in-flight request for a different language.
    let inflight_key = format!("{repo_key}::{version_id}::{language}");
    let should_spawn = match state.project_version_history_inflight.lock() {
        Ok(mut set) => {
            if set.contains(&inflight_key) {
                false
            } else {
                set.insert(inflight_key.clone());
                true
            }
        }
        Err(_) => false,
    };

    if should_spawn {
        let app_handle_clone = app_handle.clone();
        let project_path_clone = project_path.clone();
        let version_id_clone = version_id.clone();
        let repo_key_clone = repo_key.clone();
        let range_from_clone = range_from.clone();
        let range_to_clone = range_to.clone();
        let label_clone = label.clone();
        let range_from_oid_clone = range_from_oid.clone();
        let range_to_oid_clone = range_to_oid.clone();
        let semaphore = state.version_history_semaphore.clone();

        tauri::async_runtime::spawn(async move {
            // Acquire a semaphore permit to limit concurrent AI generation.
            let _permit = semaphore.acquire().await;

            let _ = tauri::async_runtime::spawn_blocking(move || {
                let state = app_handle_clone.state::<AppState>();

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    generate_and_cache_version_history(
                        &repo_path,
                        &repo_key_clone,
                        &version_id_clone,
                        &label_clone,
                        range_from_clone.as_deref(),
                        &range_to_clone,
                        commit_count,
                        range_from_oid_clone.as_deref(),
                        &range_to_oid_clone,
                        settings,
                        &state,
                    )
                }))
                .unwrap_or_else(|_| VersionHistoryResult {
                    status: "error".to_string(),
                    version_id: version_id_clone.clone(),
                    label: label_clone.clone(),
                    range_from: range_from_clone.clone(),
                    range_to: range_to_clone.clone(),
                    commit_count,
                    summary_markdown: None,
                    changelog_markdown: None,
                    error: Some("Internal error".to_string()),
                });

                if let Ok(mut set) = state.project_version_history_inflight.lock() {
                    set.remove(&inflight_key);
                }

                let payload = VersionHistoryUpdatedPayload {
                    project_path: project_path_clone,
                    version_id: version_id_clone,
                    result: result.clone(),
                };
                let _ = app_handle_clone.emit("project-version-history-updated", &payload);
            })
            .await;
        });
    }

    Ok(VersionHistoryResult {
        status: "generating".to_string(),
        version_id,
        label,
        range_from,
        range_to,
        commit_count,
        summary_markdown: None,
        changelog_markdown: generating_changelog,
        error: None,
    })
}

fn disabled_version_history_result(version_id: &str) -> VersionHistoryResult {
    VersionHistoryResult {
        status: "disabled".to_string(),
        version_id: version_id.to_string(),
        label: version_label(version_id),
        range_from: None,
        range_to: String::new(),
        commit_count: 0,
        summary_markdown: None,
        changelog_markdown: None,
        error: None,
    }
}

fn resolve_version_history_ai_settings(
    profiles: &ProfilesConfig,
) -> Option<gwt_core::config::ResolvedAISettings> {
    let ai = profiles.resolve_active_ai_settings();
    if !ai.ai_enabled {
        return None;
    }
    ai.resolved
}

fn validate_cache_entry(
    entry: &VersionHistoryCacheEntry,
    range_from_oid: Option<&str>,
    range_to_oid: &str,
    language: &str,
) -> bool {
    if entry.language != language {
        return false;
    }
    let from_ok = match (entry.range_from_oid.as_deref(), range_from_oid) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    if !from_ok {
        return false;
    }
    entry.range_to_oid == range_to_oid
}

fn get_cached_version_history(
    state: &AppState,
    repo_path: &Path,
    repo_key: &str,
    version_id: &str,
    range_from_oid: Option<&str>,
    range_to_oid: &str,
    language: &str,
) -> Option<VersionHistoryCacheEntry> {
    // 1. In-memory cache check
    {
        let guard = state.project_version_history_cache.lock().ok()?;
        if let Some(repo_map) = guard.get(repo_key) {
            if let Some(entry) = repo_map.get(version_id) {
                if validate_cache_entry(entry, range_from_oid, range_to_oid, language) {
                    return Some(entry.clone());
                }
            }
        }
    }

    // 2. Disk cache fallback
    let disk_map = load_disk_cache(repo_path)?;
    // Restore disk cache into in-memory cache
    if let Ok(mut guard) = state.project_version_history_cache.lock() {
        let repo_entry = guard.entry(repo_key.to_string()).or_default();
        for (k, v) in &disk_map {
            repo_entry.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    let entry = disk_map.get(version_id)?;
    if !validate_cache_entry(entry, range_from_oid, range_to_oid, language) {
        return None;
    }

    Some(entry.clone())
}

#[allow(clippy::too_many_arguments)]
fn generate_and_cache_version_history(
    repo_path: &Path,
    repo_key: &str,
    version_id: &str,
    label: &str,
    range_from: Option<&str>,
    range_to: &str,
    commit_count: u32,
    range_from_oid: Option<&str>,
    range_to_oid: &str,
    settings: gwt_core::config::ResolvedAISettings,
    state: &AppState,
) -> VersionHistoryResult {
    let language = normalize_version_history_language(&settings.language);
    let subjects =
        match git_log_subjects(repo_path, range_from, range_to, MAX_SUBJECTS_FOR_CHANGELOG) {
            Ok(v) => v,
            Err(err) => {
                return VersionHistoryResult {
                    status: "error".to_string(),
                    version_id: version_id.to_string(),
                    label: label.to_string(),
                    range_from: range_from.map(|s| s.to_string()),
                    range_to: range_to.to_string(),
                    commit_count,
                    summary_markdown: None,
                    changelog_markdown: None,
                    error: Some(err),
                };
            }
        };

    let simple_changelog = build_simple_changelog_markdown(&subjects, language);

    let client = match AIClient::new(settings.clone()) {
        Ok(client) => client,
        Err(err) => {
            return VersionHistoryResult {
                status: "error".to_string(),
                version_id: version_id.to_string(),
                label: label.to_string(),
                range_from: range_from.map(|s| s.to_string()),
                range_to: range_to.to_string(),
                commit_count,
                summary_markdown: None,
                changelog_markdown: Some(simple_changelog),
                error: Some(format_error_for_display(&err)),
            };
        }
    };

    let ai_input = build_ai_input(
        label,
        range_from,
        range_to,
        commit_count,
        &simple_changelog,
        &subjects,
    );

    let summary_markdown = match generate_ai_summary(&client, &ai_input, language) {
        Ok(md) => md,
        Err(err) => {
            return VersionHistoryResult {
                status: "error".to_string(),
                version_id: version_id.to_string(),
                label: label.to_string(),
                range_from: range_from.map(|s| s.to_string()),
                range_to: range_to.to_string(),
                commit_count,
                summary_markdown: None,
                changelog_markdown: Some(simple_changelog),
                error: Some(format_error_for_display(&err)),
            };
        }
    };

    // Cache only successful results.
    let new_entry = VersionHistoryCacheEntry {
        label: label.to_string(),
        range_from: range_from.map(|s| s.to_string()),
        range_to: range_to.to_string(),
        range_from_oid: range_from_oid.map(|s| s.to_string()),
        range_to_oid: range_to_oid.to_string(),
        commit_count,
        language: language.to_string(),
        summary_markdown: summary_markdown.clone(),
        changelog_markdown: simple_changelog.clone(),
    };

    if let Ok(mut guard) = state.project_version_history_cache.lock() {
        let repo_entry = guard.entry(repo_key.to_string()).or_default();
        repo_entry.insert(version_id.to_string(), new_entry);

        // Write all entries for this repo to disk.
        save_disk_cache(repo_path, repo_entry);
    }

    VersionHistoryResult {
        status: "ok".to_string(),
        version_id: version_id.to_string(),
        label: label.to_string(),
        range_from: range_from.map(|s| s.to_string()),
        range_to: range_to.to_string(),
        commit_count,
        summary_markdown: Some(summary_markdown),
        changelog_markdown: Some(simple_changelog),
        error: None,
    }
}

fn build_ai_input(
    label: &str,
    range_from: Option<&str>,
    range_to: &str,
    commit_count: u32,
    simple_changelog: &str,
    subjects: &[String],
) -> String {
    let range = match range_from {
        Some(from) => format!("{from}..{range_to}"),
        None => range_to.to_string(),
    };

    let mut raw_subjects = String::new();
    for s in subjects.iter().take(MAX_SUBJECTS_FOR_AI) {
        let line = s.trim();
        if line.is_empty() {
            continue;
        }
        raw_subjects.push_str("- ");
        raw_subjects.push_str(line);
        raw_subjects.push('\n');
    }

    let content = format!(
        "Version: {label}\nRange: {range}\nCommits: {commit_count}\n\nSimple Changelog:\n{simple_changelog}\n\nRaw Commit Subjects (sample):\n{raw_subjects}"
    );

    sample_text(&content, MAX_PROMPT_CHARS)
}

fn generate_ai_summary(client: &AIClient, input: &str, language: &str) -> Result<String, AIError> {
    let system = if language == "ja" {
        [
            "You are a release notes assistant.",
            "Write concise Japanese for end users.",
            "Do NOT list commit hashes or raw git commands.",
            "Do NOT copy commit subjects verbatim unless necessary.",
            "Output MUST be Markdown with these sections in this order:",
            "## 要約",
            "## ハイライト",
            "Highlights MUST be 3-5 bullet points.",
            "Keep it short and practical.",
        ]
        .join("\n")
    } else {
        [
            "You are a release notes assistant.",
            "Write concise English for end users.",
            "Do NOT list commit hashes or raw git commands.",
            "Do NOT copy commit subjects verbatim unless necessary.",
            "Output MUST be Markdown with these sections in this order:",
            "## Summary",
            "## Highlights",
            "Highlights MUST be 3-5 bullet points.",
            "Keep it short and practical.",
        ]
        .join("\n")
    };

    let user = if language == "ja" {
        format!("次のプロジェクト変更点を、このバージョンのリリースノートとして要約してください。\n\n{input}\n")
    } else {
        format!("Summarize the following project changes for this version.\n\n{input}\n")
    };

    let out = client.create_response(vec![
        ChatMessage {
            role: "system".to_string(),
            content: system,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user,
        },
    ])?;

    let markdown = normalize_ai_summary_markdown(out.trim(), language);
    validate_ai_summary_markdown(&markdown, language)?;
    Ok(markdown)
}

fn validate_ai_summary_markdown(markdown: &str, language: &str) -> Result<(), AIError> {
    let mut has_summary = false;
    let mut has_highlights = false;
    let mut highlight_bullets = 0usize;
    let mut in_highlights = false;

    let want_ja = language == "ja";

    for line in markdown.lines() {
        let t = line.trim();
        if (!want_ja && t.eq_ignore_ascii_case("## summary")) || (want_ja && t == "## 要約") {
            has_summary = true;
            in_highlights = false;
            continue;
        }
        if (!want_ja && t.eq_ignore_ascii_case("## highlights"))
            || (want_ja && t == "## ハイライト")
        {
            has_highlights = true;
            in_highlights = true;
            continue;
        }
        if t.starts_with("## ") {
            in_highlights = false;
        }
        if in_highlights && is_bullet_line(t) {
            highlight_bullets += 1;
        }
    }

    if has_summary && has_highlights && highlight_bullets >= 1 {
        Ok(())
    } else {
        Err(AIError::IncompleteSummary)
    }
}

fn normalize_ai_summary_markdown(markdown: &str, language: &str) -> String {
    let want_ja = language == "ja";
    let mut out = String::with_capacity(markdown.len());
    let lines: Vec<&str> = markdown.lines().collect();
    let last_idx = lines.len().saturating_sub(1);

    for (idx, line) in lines.iter().enumerate() {
        let line = *line;
        let trimmed = line.trim_start();
        if let Some(title) = trimmed.strip_prefix("## ") {
            let title = title.trim();
            let title_l = title.to_ascii_lowercase();

            if title == "要約" || title == "概要" || title_l == "summary" {
                out.push_str(if want_ja { "## 要約" } else { "## Summary" });
                if idx < last_idx {
                    out.push('\n');
                }
                continue;
            }
            if title == "ハイライト" || title_l == "highlights" {
                out.push_str(if want_ja {
                    "## ハイライト"
                } else {
                    "## Highlights"
                });
                if idx < last_idx {
                    out.push('\n');
                }
                continue;
            }
        }

        out.push_str(line);
        if idx < last_idx {
            out.push('\n');
        }
    }

    out
}

fn is_bullet_line(line: &str) -> bool {
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("•")
        || strip_ordered_prefix(line).is_some()
}

fn strip_ordered_prefix(line: &str) -> Option<&str> {
    // Matches "1. " / "1) "
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i + 1 >= bytes.len() {
        return None;
    }
    if (bytes[i] == b'.' || bytes[i] == b')') && bytes[i + 1] == b' ' {
        return Some(&line[i + 2..]);
    }
    None
}

fn build_simple_changelog_markdown(subjects: &[String], language: &str) -> String {
    let want_ja = language == "ja";
    let mut groups: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for s in subjects {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        let group = classify_subject_group(s);
        let entry = normalize_subject_for_changelog(s);
        groups.entry(group).or_default().push(entry);
    }

    let order = [
        "Features",
        "Bug Fixes",
        "Documentation",
        "Performance",
        "Refactor",
        "Styling",
        "Testing",
        "Miscellaneous Tasks",
        "Other",
    ];

    let mut out = String::new();
    for name in order {
        let Some(entries) = groups.get(name) else {
            continue;
        };
        if entries.is_empty() {
            continue;
        }
        out.push_str("### ");
        out.push_str(if want_ja {
            translate_changelog_group(name)
        } else {
            name
        });
        out.push('\n');
        let mut shown = 0usize;
        for e in entries.iter().take(MAX_CHANGELOG_LINES_PER_GROUP) {
            out.push_str("- ");
            out.push_str(e);
            out.push('\n');
            shown += 1;
        }
        if entries.len() > shown {
            out.push_str(&format!(
                "- (+{} {})\n",
                entries.len() - shown,
                if want_ja { "件" } else { "more" }
            ));
        }
        out.push('\n');
    }

    if out.trim().is_empty() {
        if want_ja {
            "(コミットなし)".to_string()
        } else {
            "(No commits)".to_string()
        }
    } else {
        out.trim_end().to_string()
    }
}

fn translate_changelog_group(name: &str) -> &str {
    match name {
        "Features" => "機能",
        "Bug Fixes" => "バグ修正",
        "Documentation" => "ドキュメント",
        "Performance" => "パフォーマンス",
        "Refactor" => "リファクタ",
        "Styling" => "スタイル",
        "Testing" => "テスト",
        "Miscellaneous Tasks" => "その他タスク",
        "Other" => "その他",
        _ => name,
    }
}

fn classify_subject_group(subject: &str) -> &'static str {
    let s = subject.trim().to_ascii_lowercase();
    if s.starts_with("feat") {
        return "Features";
    }
    if s.starts_with("fix") {
        return "Bug Fixes";
    }
    if s.starts_with("docs") || s.starts_with("doc") {
        return "Documentation";
    }
    if s.starts_with("perf") {
        return "Performance";
    }
    if s.starts_with("refactor") {
        return "Refactor";
    }
    if s.starts_with("style") {
        return "Styling";
    }
    if s.starts_with("test") {
        return "Testing";
    }
    if s.starts_with("chore") {
        return "Miscellaneous Tasks";
    }
    "Other"
}

fn normalize_subject_for_changelog(subject: &str) -> String {
    let s = subject.trim();
    let Some((prefix, rest)) = s.split_once(':') else {
        return s.to_string();
    };

    let msg = rest.trim();
    if msg.is_empty() {
        return s.to_string();
    }

    let mut prefix = prefix.trim();
    if prefix.ends_with('!') {
        prefix = prefix.trim_end_matches('!');
    }

    let (typ, scope) = if let Some((t, rest)) = prefix.split_once('(') {
        let t = t.trim();
        let scope = rest.trim_end_matches(')').trim();
        (t, if scope.is_empty() { None } else { Some(scope) })
    } else {
        (prefix, None)
    };

    let typ_l = typ.to_ascii_lowercase();
    let known = matches!(
        typ_l.as_str(),
        "feat" | "fix" | "docs" | "doc" | "perf" | "refactor" | "style" | "test" | "chore"
    );
    if !known {
        return s.to_string();
    }

    if let Some(scope) = scope {
        format!("**{}:** {}", scope, msg)
    } else {
        msg.to_string()
    }
}

fn sample_text(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    let head_chars = max_chars * 2 / 5; // 40%
    let separator = "\n...[truncated]...\n";
    let tail_chars = max_chars.saturating_sub(head_chars + separator.len());

    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}{separator}{tail}")
}

fn version_label(version_id: &str) -> String {
    if version_id == VERSION_ID_UNRELEASED {
        "Unreleased (HEAD)".to_string()
    } else {
        version_id.to_string()
    }
}

fn resolve_range_for_version(
    repo_path: &Path,
    version_id: &str,
) -> Result<(String, Option<String>, String), String> {
    let tags = list_version_tags(repo_path, None)?;

    if version_id == VERSION_ID_UNRELEASED {
        let from = tags.first().cloned();
        return Ok((version_label(version_id), from, "HEAD".to_string()));
    }

    let idx = tags
        .iter()
        .position(|t| t == version_id)
        .ok_or_else(|| format!("Version tag not found: {}", version_id))?;
    let prev = tags.get(idx + 1).cloned();
    Ok((version_id.to_string(), prev, version_id.to_string()))
}

fn refresh_version_tags_from_default_remote(repo_path: &Path) {
    let remote_name = match Remote::default_name(repo_path) {
        Ok(Some(name)) => name,
        _ => return,
    };

    let _ = gwt_core::process::command("git")
        .args([
            "fetch",
            remote_name.as_str(),
            "--tags",
            "--prune",
            "--no-recurse-submodules",
        ])
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();
}

fn list_version_tags(repo_path: &Path, max: Option<usize>) -> Result<Vec<String>, String> {
    let args = vec!["tag".to_string(), "--list".to_string(), "v*".to_string()];
    let out = git_output(repo_path, &args)?;
    let mut tags = parse_and_sort_version_tags(&out);

    if let Some(n) = max {
        tags.truncate(n);
    }
    Ok(tags)
}

fn parse_and_sort_version_tags(raw: &str) -> Vec<String> {
    let mut tags: Vec<String> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .filter(|t| t.starts_with('v') && t.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
        .collect();

    tags.sort_by(|a, b| compare_version_tag_desc(a, b));
    tags
}

fn compare_version_tag_desc(a: &str, b: &str) -> Ordering {
    match (parse_semver_tag(a), parse_semver_tag(b)) {
        (Some(ver_a), Some(ver_b)) => ver_b.cmp(&ver_a).then_with(|| b.cmp(a)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b.cmp(a),
    }
}

fn parse_semver_tag(tag: &str) -> Option<semver::Version> {
    let raw = tag.strip_prefix('v')?;
    semver::Version::parse(raw).ok()
}

fn rev_parse(repo_path: &Path, rev: &str) -> Result<String, String> {
    let args = vec!["rev-parse".to_string(), rev.to_string()];
    let out = git_output(repo_path, &args)?;
    let hash = out.lines().next().unwrap_or("").trim().to_string();
    if hash.is_empty() {
        return Err(format!("Failed to resolve revision: {}", rev));
    }
    Ok(hash)
}

fn rev_list_count(repo_path: &Path, from: Option<&str>, to: &str) -> Result<u32, String> {
    if to == "HEAD" && is_unborn_head(repo_path) {
        return Ok(0);
    }
    let range = match from {
        Some(f) if !f.trim().is_empty() => format!("{f}..{to}"),
        _ => to.to_string(),
    };
    let args = vec!["rev-list".to_string(), "--count".to_string(), range];
    let out = git_output(repo_path, &args)?;
    let text = out.lines().next().unwrap_or("").trim();
    text.parse::<u32>()
        .map_err(|_| format!("Failed to parse rev-list count: {}", text))
}

fn git_log_subjects(
    repo_path: &Path,
    from: Option<&str>,
    to: &str,
    max: usize,
) -> Result<Vec<String>, String> {
    if to == "HEAD" && is_unborn_head(repo_path) {
        return Ok(Vec::new());
    }
    let range = match from {
        Some(f) if !f.trim().is_empty() => format!("{f}..{to}"),
        _ => to.to_string(),
    };

    let args = vec![
        "log".to_string(),
        "--no-merges".to_string(),
        "--pretty=format:%s".to_string(),
        "-n".to_string(),
        max.to_string(),
        range,
    ];
    let out = git_output(repo_path, &args)?;
    Ok(out
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect())
}

fn git_output(repo_path: &Path, args: &[String]) -> Result<String, String> {
    let output = gwt_core::process::command("git")
        .args(args)
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Failed to execute git: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "Git command failed".to_string()
        } else {
            stderr
        })
    }
}

/// Identify versions that are not yet cached and need prefetching.
///
/// Returns version IDs that need background generation.
#[cfg(test)]
fn find_uncached_versions(
    repo_path: &Path,
    repo_key: &str,
    language: &str,
    versions: &[VersionItem],
    state: &AppState,
) -> Vec<String> {
    let mut uncached = Vec::new();

    for version in versions {
        let (_, range_from, range_to) = match resolve_range_for_version(repo_path, &version.id) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let range_from_oid = match &range_from {
            Some(r) => match rev_parse(repo_path, r) {
                Ok(v) => Some(v),
                Err(_) => continue,
            },
            None => None,
        };
        let range_to_oid = match rev_parse(repo_path, &range_to) {
            Ok(v) => v,
            Err(_) => {
                if range_to == "HEAD" && is_unborn_head(repo_path) {
                    continue; // Unborn HEAD has no commits to summarize
                } else {
                    continue;
                }
            }
        };

        if get_cached_version_history(
            state,
            repo_path,
            repo_key,
            &version.id,
            range_from_oid.as_deref(),
            &range_to_oid,
            language,
        )
        .is_some()
        {
            continue;
        }

        uncached.push(version.id.clone());
    }

    uncached
}

/// Inner prefetch logic, callable from both the Tauri command and the project open hook.
pub fn prefetch_version_history_inner(project_path: &str, app_handle: &AppHandle) {
    let started = Instant::now();
    let _span = tracing::info_span!(
        "startup.prefetch_version_history_inner",
        project_path = %project_path
    )
    .entered();
    let profiles = match ProfilesConfig::load() {
        Ok(p) => p,
        Err(_) => return,
    };
    if resolve_version_history_ai_settings(&profiles).is_none() {
        return; // AI disabled, nothing to prefetch
    }

    let versions = match list_project_versions(project_path.to_string(), 20) {
        Ok(v) => v,
        Err(_) => return,
    };

    let tauri_state: State<AppState> = app_handle.state::<AppState>();

    for version in versions.items {
        // get_project_version_history handles cache check, inflight dedup,
        // and semaphore-controlled background generation internally.
        let _ = get_project_version_history(
            project_path.to_string(),
            version.id,
            tauri_state.clone(),
            app_handle.clone(),
        );
    }

    tracing::info!(
        category = "project_start",
        command = "prefetch_version_history_inner",
        project_path = %project_path,
        elapsed_ms = started.elapsed().as_millis(),
        "Version history prefetch queued"
    );
}

/// Prefetch version history for all uncached versions in a project.
///
/// This command is fire-and-forget: uncached versions are generated in the background
/// using the existing `get_project_version_history` flow (which handles inflight
/// deduplication and semaphore-based concurrency limiting).
#[instrument(skip_all, fields(command = "prefetch_version_history", project_path))]
#[tauri::command]
pub fn prefetch_version_history(
    project_path: String,
    _state: State<AppState>,
    app_handle: AppHandle,
) {
    let app_handle_bg = app_handle.clone();

    tauri::async_runtime::spawn_blocking(move || {
        prefetch_version_history_inner(&project_path, &app_handle_bg);
    });
}

fn is_unborn_head(repo_path: &Path) -> bool {
    let output = gwt_core::process::command("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();

    match output {
        Ok(out) => !out.status.success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::Path};

    use gwt_core::config::{AISettings, Profile};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::{TestEnvGuard, ENV_LOCK};

    fn init_git_repo(path: &Path) {
        let out = gwt_core::process::command("git")
            .args(["init"])
            .current_dir(path)
            .output();
        assert!(out.is_ok());
        assert!(out.unwrap().status.success());
        let _ = gwt_core::process::command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output();
        let _ = gwt_core::process::command("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output();
    }

    fn commit_file(path: &Path, name: &str, content: &str, msg: &str) {
        fs::write(path.join(name), content).unwrap();
        let _ = gwt_core::process::command("git")
            .args(["add", "."])
            .current_dir(path)
            .output();
        let out = gwt_core::process::command("git")
            .args(["commit", "-m", msg])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    fn tag(path: &Path, name: &str) {
        let out = gwt_core::process::command("git")
            .args(["tag", name])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    fn run_git(path: &Path, args: &[&str]) {
        let out = gwt_core::process::command("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn parse_and_sort_version_tags_orders_semver_descending() {
        let raw = "v7.9.0\nv7.12.6\nv7.10.0\nv7.12.5\n";
        let sorted = parse_and_sort_version_tags(raw);
        assert_eq!(
            sorted,
            vec![
                "v7.12.6".to_string(),
                "v7.12.5".to_string(),
                "v7.10.0".to_string(),
                "v7.9.0".to_string(),
            ]
        );
    }

    #[test]
    fn parse_and_sort_version_tags_keeps_non_semver_tags_stable_and_last() {
        let raw = "v1.0.0\nv1.0\nv2.0.0\nv1.0.1\n";
        let sorted = parse_and_sort_version_tags(raw);
        assert_eq!(
            sorted,
            vec![
                "v2.0.0".to_string(),
                "v1.0.1".to_string(),
                "v1.0.0".to_string(),
                "v1.0".to_string(),
            ]
        );
    }

    #[test]
    fn list_project_versions_includes_unreleased_and_tags() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        commit_file(repo.path(), "a.txt", "1", "feat: first");
        tag(repo.path(), "v1.0.0");
        commit_file(repo.path(), "b.txt", "2", "fix: second");
        tag(repo.path(), "v1.0.1");
        commit_file(repo.path(), "c.txt", "3", "chore: third");

        let out = list_project_versions(repo.path().to_string_lossy().to_string(), 10).unwrap();
        assert!(!out.items.is_empty());
        assert_eq!(out.items[0].id, "unreleased");
        assert_eq!(out.items[0].range_to, "HEAD");
        assert_eq!(out.items[1].id, "v1.0.1");
        assert_eq!(out.items[2].id, "v1.0.0");
        assert!(out.items.iter().any(|i| i.id == "v1.0.1"));
        assert!(out.items.iter().any(|i| i.id == "v1.0.0"));
    }

    #[test]
    fn list_project_versions_refreshes_tags_from_default_remote() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let origin = TempDir::new().unwrap();
        run_git(origin.path(), &["init", "--bare"]);

        let seed = TempDir::new().unwrap();
        init_git_repo(seed.path());
        commit_file(seed.path(), "a.txt", "1", "feat: first");
        run_git(seed.path(), &["branch", "-M", "main"]);
        tag(seed.path(), "v1.0.0");
        run_git(
            seed.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        run_git(seed.path(), &["push", "-u", "origin", "main"]);
        run_git(seed.path(), &["push", "origin", "--tags"]);

        let local = TempDir::new().unwrap();
        let out = gwt_core::process::command("git")
            .args([
                "clone",
                origin.path().to_str().unwrap(),
                local.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(out.status.success());

        commit_file(seed.path(), "b.txt", "2", "fix: second");
        tag(seed.path(), "v1.1.0");
        run_git(seed.path(), &["push", "origin", "main"]);
        run_git(seed.path(), &["push", "origin", "--tags"]);

        let local_before = list_version_tags(local.path(), None).unwrap();
        assert!(
            !local_before.iter().any(|t| t == "v1.1.0"),
            "clone should be stale before tag refresh"
        );

        let out = list_project_versions(local.path().to_string_lossy().to_string(), 10).unwrap();
        assert_eq!(out.items[1].id, "v1.1.0");
        assert!(out.items.iter().any(|i| i.id == "v1.0.0"));
    }

    #[test]
    fn list_project_versions_handles_unborn_head() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());

        let out = list_project_versions(repo.path().to_string_lossy().to_string(), 10).unwrap();
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].id, "unreleased");
        assert_eq!(out.items[0].range_to, "HEAD");
        assert_eq!(out.items[0].commit_count, 0);
    }

    #[test]
    fn simple_changelog_groups_and_strips_prefixes() {
        let subjects = vec![
            "feat(ui): add button".to_string(),
            "fix: crash".to_string(),
            "docs: update readme".to_string(),
            "random message".to_string(),
        ];
        let md = build_simple_changelog_markdown(&subjects, "en");
        assert!(md.contains("### Features"));
        assert!(md.contains("**ui:** add button"));
        assert!(md.contains("### Bug Fixes"));
        assert!(md.contains("- crash"));
        assert!(md.contains("### Documentation"));
        assert!(md.contains("- update readme"));
        assert!(md.contains("### Other"));
        assert!(md.contains("- random message"));
    }

    #[test]
    fn simple_changelog_translates_group_headings_in_japanese() {
        let subjects = vec!["feat: add".to_string(), "fix: bug".to_string()];
        let md = build_simple_changelog_markdown(&subjects, "ja");
        assert!(md.contains("### 機能"));
        assert!(md.contains("### バグ修正"));
    }

    fn ai_settings(enabled: bool) -> AISettings {
        AISettings {
            endpoint: if enabled {
                "https://api.openai.com/v1".to_string()
            } else {
                String::new()
            },
            api_key: String::new(),
            model: if enabled {
                "gpt-5.2-codex".to_string()
            } else {
                String::new()
            },
            language: "en".to_string(),
            summary_enabled: true,
        }
    }

    fn make_cache_entry(range_to_oid: &str, language: &str) -> VersionHistoryCacheEntry {
        VersionHistoryCacheEntry {
            label: "v1.0.0".to_string(),
            range_from: None,
            range_to: "v1.0.0".to_string(),
            range_from_oid: None,
            range_to_oid: range_to_oid.to_string(),
            commit_count: 3,
            language: language.to_string(),
            summary_markdown: "## Summary\nTest".to_string(),
            changelog_markdown: "### Features\n- test".to_string(),
        }
    }

    #[test]
    fn cache_file_path_same_path_produces_same_hash() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let p1 = cache_file_path(repo.path());
        let p2 = cache_file_path(repo.path());
        assert_eq!(p1, p2);
    }

    #[test]
    fn cache_file_path_different_paths_produce_different_hashes() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo_a = TempDir::new().unwrap();
        let repo_b = TempDir::new().unwrap();
        let p1 = cache_file_path(repo_a.path());
        let p2 = cache_file_path(repo_b.path());
        assert_ne!(p1, p2);
    }

    #[test]
    fn disk_cache_roundtrip() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let mut entries = HashMap::new();
        entries.insert("v1.0.0".to_string(), make_cache_entry("abc123", "en"));

        save_disk_cache(repo.path(), &entries);
        let loaded = load_disk_cache(repo.path());
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 1);
        let entry = loaded.get("v1.0.0").unwrap();
        assert_eq!(entry.range_to_oid, "abc123");
        assert_eq!(entry.language, "en");
        assert_eq!(entry.summary_markdown, "## Summary\nTest");
    }

    #[test]
    fn disk_cache_invalid_json_returns_none() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let path = cache_file_path(repo.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ invalid json }}}").unwrap();

        let loaded = load_disk_cache(repo.path());
        assert!(loaded.is_none());
    }

    #[test]
    fn get_cached_version_history_falls_back_to_disk() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        // Write cache to disk (but NOT to in-memory)
        let mut entries = HashMap::new();
        entries.insert("v1.0.0".to_string(), make_cache_entry("abc123", "en"));
        save_disk_cache(repo.path(), &entries);

        // Should find via disk fallback
        let hit = get_cached_version_history(
            &state,
            repo.path(),
            &repo_key,
            "v1.0.0",
            None,
            "abc123",
            "en",
        );
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().range_to_oid, "abc123");

        // After disk fallback, the entry should be in in-memory cache too
        let guard = state.project_version_history_cache.lock().unwrap();
        assert!(guard.get(&repo_key).unwrap().contains_key("v1.0.0"));
    }

    #[test]
    fn cache_miss_on_oid_mismatch() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        let mut entries = HashMap::new();
        entries.insert("v1.0.0".to_string(), make_cache_entry("abc123", "en"));
        save_disk_cache(repo.path(), &entries);

        let hit = get_cached_version_history(
            &state,
            repo.path(),
            &repo_key,
            "v1.0.0",
            None,
            "different_oid",
            "en",
        );
        assert!(hit.is_none());
    }

    #[test]
    fn cache_miss_on_language_mismatch() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        let mut entries = HashMap::new();
        entries.insert("v1.0.0".to_string(), make_cache_entry("abc123", "en"));
        save_disk_cache(repo.path(), &entries);

        let hit = get_cached_version_history(
            &state,
            repo.path(),
            &repo_key,
            "v1.0.0",
            None,
            "abc123",
            "ja",
        );
        assert!(hit.is_none());
    }

    #[test]
    fn cache_miss_still_hydrates_disk_entries_for_future_save() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        let mut entries = HashMap::new();
        entries.insert("v1.0.0".to_string(), make_cache_entry("old_oid", "en"));
        entries.insert("v1.0.1".to_string(), make_cache_entry("stable_oid", "en"));
        save_disk_cache(repo.path(), &entries);

        // Requested version misses validation, but disk entries should still hydrate.
        let hit = get_cached_version_history(
            &state,
            repo.path(),
            &repo_key,
            "v1.0.0",
            None,
            "new_oid",
            "en",
        );
        assert!(hit.is_none());

        // Simulate successful regeneration and persist all in-memory entries.
        {
            let mut guard = state.project_version_history_cache.lock().unwrap();
            let repo_map = guard
                .get_mut(&repo_key)
                .expect("disk entries should hydrate after fallback");
            assert!(repo_map.contains_key("v1.0.1"));
            repo_map.insert("v1.0.0".to_string(), make_cache_entry("new_oid", "en"));
            save_disk_cache(repo.path(), repo_map);
        }

        let loaded = load_disk_cache(repo.path()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("v1.0.1").unwrap().range_to_oid, "stable_oid");
        assert_eq!(loaded.get("v1.0.0").unwrap().range_to_oid, "new_oid");
    }

    fn make_cache_entry_with_from(
        range_from_oid: Option<&str>,
        range_to_oid: &str,
        language: &str,
    ) -> VersionHistoryCacheEntry {
        VersionHistoryCacheEntry {
            label: "v1.0.0".to_string(),
            range_from: None,
            range_to: "v1.0.0".to_string(),
            range_from_oid: range_from_oid.map(|s| s.to_string()),
            range_to_oid: range_to_oid.to_string(),
            commit_count: 3,
            language: language.to_string(),
            summary_markdown: "## Summary\nTest".to_string(),
            changelog_markdown: "### Features\n- test".to_string(),
        }
    }

    #[test]
    fn prefetch_skips_cached_versions() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        commit_file(repo.path(), "a.txt", "1", "feat: first");
        tag(repo.path(), "v1.0.0");
        commit_file(repo.path(), "b.txt", "2", "fix: second");

        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        // Get versions list
        let versions =
            list_project_versions(repo.path().to_string_lossy().to_string(), 10).unwrap();

        // Resolve the v1.0.0 OID for cache entry
        let v100_oid = rev_parse(repo.path(), "v1.0.0").unwrap();

        // Write a valid cache entry for v1.0.0 to disk
        let mut entries = HashMap::new();
        entries.insert(
            "v1.0.0".to_string(),
            make_cache_entry_with_from(None, &v100_oid, "en"),
        );
        save_disk_cache(repo.path(), &entries);

        let uncached =
            find_uncached_versions(repo.path(), &repo_key, "en", &versions.items, &state);

        // v1.0.0 is cached, so only unreleased should be uncached
        assert!(
            !uncached.contains(&"v1.0.0".to_string()),
            "v1.0.0 should be skipped (cached)"
        );
        assert!(
            uncached.contains(&"unreleased".to_string()),
            "unreleased should be in uncached list"
        );
    }

    #[test]
    fn prefetch_includes_uncached_versions() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        commit_file(repo.path(), "a.txt", "1", "feat: first");
        tag(repo.path(), "v1.0.0");
        commit_file(repo.path(), "b.txt", "2", "fix: second");
        tag(repo.path(), "v1.0.1");
        commit_file(repo.path(), "c.txt", "3", "chore: third");

        let state = AppState::new();
        let repo_key = repo.path().to_string_lossy().to_string();

        // No cache at all
        let versions =
            list_project_versions(repo.path().to_string_lossy().to_string(), 10).unwrap();
        let uncached =
            find_uncached_versions(repo.path(), &repo_key, "en", &versions.items, &state);

        // All versions should be uncached
        assert!(
            uncached.contains(&"unreleased".to_string()),
            "unreleased should need prefetch"
        );
        assert!(
            uncached.contains(&"v1.0.1".to_string()),
            "v1.0.1 should need prefetch"
        );
        assert!(
            uncached.contains(&"v1.0.0".to_string()),
            "v1.0.0 should need prefetch"
        );
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_disabled_when_summary_disabled() {
        let mut profiles = HashMap::new();
        let mut default = Profile::new("default");
        default.ai = Some(ai_settings(false));
        profiles.insert("default".to_string(), default);
        let config = ProfilesConfig {
            version: 1,
            active: Some("default".to_string()),
            profiles,
        };

        let out = resolve_version_history_ai_settings(&config);
        assert!(out.is_none(), "summary disabled should skip AI generation");
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_disabled_when_ai_not_configured() {
        let mut profiles = HashMap::new();
        profiles.insert("default".to_string(), Profile::new("default"));
        let config = ProfilesConfig {
            version: 1,
            active: Some("default".to_string()),
            profiles,
        };

        let out = resolve_version_history_ai_settings(&config);
        assert!(out.is_none(), "missing AI config should skip AI generation");
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_settings_when_enabled() {
        let mut profiles = HashMap::new();
        let mut default = Profile::new("default");
        default.ai = Some(ai_settings(true));
        profiles.insert("default".to_string(), default);
        let config = ProfilesConfig {
            version: 1,
            active: Some("default".to_string()),
            profiles,
        };

        let settings = resolve_version_history_ai_settings(&config)
            .expect("enabled AI should provide settings");
        assert_eq!(settings.endpoint, "https://api.openai.com/v1");
        assert_eq!(settings.model, "gpt-5.2-codex");
    }
}

//! Project version history (tag-based) summarization for the GUI.
//!
//! This feature does NOT read CHANGELOG.md. It derives version ranges from Git tags
//! (v*), generates a simple grouped changelog from commit subjects, and (when AI is
//! configured) generates an English summary.

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::{AppState, VersionHistoryCacheEntry};
use gwt_core::ai::{format_error_for_display, AIClient, AIError, ChatMessage};
use gwt_core::config::ProfilesConfig;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tauri::Manager;
use tauri::{AppHandle, Emitter, State};

const VERSION_ID_UNRELEASED: &str = "unreleased";
const RANGE_OID_UNBORN_HEAD: &str = "UNBORN_HEAD";

const MAX_SUBJECTS_FOR_CHANGELOG: usize = 400;
const MAX_SUBJECTS_FOR_AI: usize = 120;
const MAX_CHANGELOG_LINES_PER_GROUP: usize = 20;
const MAX_PROMPT_CHARS: usize = 12000;

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

#[tauri::command]
pub fn list_project_versions(
    project_path: String,
    limit: usize,
) -> Result<ProjectVersions, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let limit = limit.max(1);
    let tag_limit = limit.saturating_sub(1);

    // Fetch an extra tag so each displayed tag can have a "previous" range bound.
    let tags = list_version_tags(&repo_path, Some(tag_limit.saturating_add(1)))?;

    let mut items = Vec::new();

    // Unreleased range: latest tag -> HEAD (or entire history if no tags exist)
    {
        let latest = tags.first().cloned();
        let commit_count = rev_list_count(&repo_path, latest.as_deref(), "HEAD")?;
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
        let commit_count = rev_list_count(&repo_path, prev.as_deref(), tag.as_str())?;
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

#[tauri::command]
pub fn get_project_version_history(
    project_path: String,
    version_id: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<VersionHistoryResult, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let repo_key = repo_path.to_string_lossy().to_string();

    let profiles = ProfilesConfig::load().map_err(|e| e.to_string())?;
    let settings = match resolve_version_history_ai_settings(&profiles) {
        Some(settings) => settings,
        None => return Ok(disabled_version_history_result(&version_id)),
    };

    // Resolve the requested version into a concrete git range.
    let (label, range_from, range_to) = resolve_range_for_version(&repo_path, &version_id)?;
    let commit_count = rev_list_count(&repo_path, range_from.as_deref(), &range_to)?;

    let range_from_oid = match &range_from {
        Some(r) => Some(rev_parse(&repo_path, r)?),
        None => None,
    };
    let range_to_oid = match rev_parse(&repo_path, &range_to) {
        Ok(v) => v,
        Err(err) => {
            if range_to == "HEAD" && is_unborn_head(&repo_path) {
                RANGE_OID_UNBORN_HEAD.to_string()
            } else {
                return Err(err);
            }
        }
    };

    // Cache hit
    if let Some(hit) = get_cached_version_history(
        &state,
        &repo_key,
        &version_id,
        range_from_oid.as_deref(),
        &range_to_oid,
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

    // Start background job (best-effort)
    let inflight_key = format!("{repo_key}::{version_id}");
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

        tauri::async_runtime::spawn_blocking(move || {
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
        changelog_markdown: None,
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
    if !ai.summary_enabled {
        return None;
    }
    ai.resolved
}

fn get_cached_version_history(
    state: &AppState,
    repo_key: &str,
    version_id: &str,
    range_from_oid: Option<&str>,
    range_to_oid: &str,
) -> Option<VersionHistoryCacheEntry> {
    let guard = state.project_version_history_cache.lock().ok()?;
    let repo_map = guard.get(repo_key)?;
    let entry = repo_map.get(version_id)?.clone();

    let from_ok = match (entry.range_from_oid.as_deref(), range_from_oid) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    if !from_ok {
        return None;
    }
    if entry.range_to_oid != range_to_oid {
        return None;
    }
    Some(entry)
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

    let simple_changelog = build_simple_changelog_markdown(&subjects);

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

    let summary_markdown = match generate_ai_summary(&client, &ai_input) {
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
    if let Ok(mut guard) = state.project_version_history_cache.lock() {
        let repo_entry = guard.entry(repo_key.to_string()).or_default();
        repo_entry.insert(
            version_id.to_string(),
            VersionHistoryCacheEntry {
                label: label.to_string(),
                range_from: range_from.map(|s| s.to_string()),
                range_to: range_to.to_string(),
                range_from_oid: range_from_oid.map(|s| s.to_string()),
                range_to_oid: range_to_oid.to_string(),
                commit_count,
                summary_markdown: summary_markdown.clone(),
                changelog_markdown: simple_changelog.clone(),
            },
        );
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

fn generate_ai_summary(client: &AIClient, input: &str) -> Result<String, AIError> {
    let system = [
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
    .join("\n");

    let user = format!("Summarize the following project changes for this version.\n\n{input}\n");

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

    let markdown = out.trim().to_string();
    validate_ai_summary_markdown(&markdown)?;
    Ok(markdown)
}

fn validate_ai_summary_markdown(markdown: &str) -> Result<(), AIError> {
    let mut has_summary = false;
    let mut has_highlights = false;
    let mut highlight_bullets = 0usize;
    let mut in_highlights = false;

    for line in markdown.lines() {
        let t = line.trim();
        if t.eq_ignore_ascii_case("## summary") {
            has_summary = true;
            in_highlights = false;
            continue;
        }
        if t.eq_ignore_ascii_case("## highlights") {
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

fn build_simple_changelog_markdown(subjects: &[String]) -> String {
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
        out.push_str(name);
        out.push('\n');
        let mut shown = 0usize;
        for e in entries.iter().take(MAX_CHANGELOG_LINES_PER_GROUP) {
            out.push_str("- ");
            out.push_str(e);
            out.push('\n');
            shown += 1;
        }
        if entries.len() > shown {
            out.push_str(&format!("- (+{} more)\n", entries.len() - shown));
        }
        out.push('\n');
    }

    if out.trim().is_empty() {
        "(No commits)".to_string()
    } else {
        out.trim_end().to_string()
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

fn list_version_tags(repo_path: &Path, max: Option<usize>) -> Result<Vec<String>, String> {
    let args = vec![
        "tag".to_string(),
        "--list".to_string(),
        "v*".to_string(),
        "--sort=-v:refname".to_string(),
    ];
    let out = git_output(repo_path, &args)?;
    let mut tags: Vec<String> = out
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .filter(|t| t.starts_with('v') && t.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
        .collect();

    if let Some(n) = max {
        tags.truncate(n);
    }
    Ok(tags)
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
    let output = Command::new("git")
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

fn is_unborn_head(repo_path: &Path) -> bool {
    let output = Command::new("git")
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
    use super::*;
    use crate::commands::{TestEnvGuard, ENV_LOCK};
    use gwt_core::config::AISettings;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_git_repo(path: &Path) {
        let out = Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output();
        assert!(out.is_ok());
        assert!(out.unwrap().status.success());
        let _ = Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output();
        let _ = Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(path)
            .output();
    }

    fn commit_file(path: &Path, name: &str, content: &str, msg: &str) {
        fs::write(path.join(name), content).unwrap();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output();
        let out = Command::new("git")
            .args(["commit", "-m", msg])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    fn tag(path: &Path, name: &str) {
        let out = Command::new("git")
            .args(["tag", name])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn list_project_versions_includes_unreleased_and_tags() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        assert!(out.items.iter().any(|i| i.id == "v1.0.1"));
        assert!(out.items.iter().any(|i| i.id == "v1.0.0"));
    }

    #[test]
    fn list_project_versions_handles_unborn_head() {
        let _lock = ENV_LOCK.lock().unwrap();
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
        let md = build_simple_changelog_markdown(&subjects);
        assert!(md.contains("### Features"));
        assert!(md.contains("**ui:** add button"));
        assert!(md.contains("### Bug Fixes"));
        assert!(md.contains("- crash"));
        assert!(md.contains("### Documentation"));
        assert!(md.contains("- update readme"));
        assert!(md.contains("### Other"));
        assert!(md.contains("- random message"));
    }

    fn ai_settings(summary_enabled: bool) -> AISettings {
        AISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: "gpt-5.2-codex".to_string(),
            summary_enabled,
        }
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_disabled_when_summary_disabled() {
        let config = ProfilesConfig {
            version: 1,
            active: None,
            default_ai: Some(ai_settings(false)),
            profiles: HashMap::new(),
        };

        let out = resolve_version_history_ai_settings(&config);
        assert!(out.is_none(), "summary disabled should skip AI generation");
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_disabled_when_ai_not_configured() {
        let config = ProfilesConfig {
            version: 1,
            active: None,
            default_ai: None,
            profiles: HashMap::new(),
        };

        let out = resolve_version_history_ai_settings(&config);
        assert!(out.is_none(), "missing AI config should skip AI generation");
    }

    #[test]
    fn resolve_version_history_ai_settings_returns_settings_when_enabled() {
        let config = ProfilesConfig {
            version: 1,
            active: None,
            default_ai: Some(ai_settings(true)),
            profiles: HashMap::new(),
        };

        let settings = resolve_version_history_ai_settings(&config)
            .expect("enabled AI should provide settings");
        assert_eq!(settings.endpoint, "https://api.openai.com/v1");
        assert_eq!(settings.model, "gpt-5.2-codex");
    }
}

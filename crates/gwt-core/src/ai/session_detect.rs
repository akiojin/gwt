//! Session ID detection for coding agents.
//!
//! Scans each agent's session files to find the latest session ID
//! for a given worktree path.  Migrated from gwt-cli `main.rs`.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::{fs, time::SystemTime};

use super::claude_paths::encode_claude_project_path;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect the latest session ID for the given agent tool at the worktree path.
///
/// Scans agent-specific session files under the user home directory.
/// Returns `None` when the agent is unknown or no session file is found.
pub fn detect_session_id_for_tool(tool_id: &str, worktree_path: &Path) -> Option<String> {
    let home = dirs::home_dir()?;
    detect_session_id_for_tool_at(&home, tool_id, worktree_path)
}

/// Testable variant that accepts an explicit home directory.
pub(crate) fn detect_session_id_for_tool_at(
    home: &Path,
    tool_id: &str,
    worktree_path: &Path,
) -> Option<String> {
    let lower = tool_id.to_lowercase();
    if lower.contains("codex") {
        return detect_codex_session_id(home, worktree_path);
    }
    if lower.contains("claude") {
        return detect_claude_session_id(home, worktree_path);
    }
    if lower.contains("gemini") {
        return detect_gemini_session_id(home);
    }
    if lower.contains("opencode") || lower.contains("open-code") {
        return detect_opencode_session_id(home);
    }
    None
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct ClaudeHistoryEntry {
    #[allow(dead_code)]
    timestamp: u64,
    project: String,
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn parse_claude_history(home: &Path) -> Vec<ClaudeHistoryEntry> {
    let path = home.join(".claude").join("history.jsonl");
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

fn detect_claude_session_id(home: &Path, worktree_path: &Path) -> Option<String> {
    // Primary: history.jsonl with project→sessionId mapping
    let history = parse_claude_history(home);
    let worktree_str = worktree_path.to_string_lossy();
    let from_history = history
        .iter()
        .filter(|e| e.project == *worktree_str || worktree_str.ends_with(&e.project))
        .max_by_key(|e| e.timestamp)
        .map(|e| e.session_id.clone());
    if from_history.is_some() {
        return from_history;
    }

    // Fallback: scan projects/{encoded_path}/*.jsonl
    let project_dir = home
        .join(".claude")
        .join("projects")
        .join(encode_claude_project_path(worktree_path));
    if !project_dir.exists() {
        return None;
    }
    latest_jsonl_stem(&project_dir)
}

// ---------------------------------------------------------------------------
// Codex CLI
// ---------------------------------------------------------------------------

fn parse_codex_session_meta(path: &Path) -> Option<(String, Option<String>)> {
    let file = fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        let payload = value.get("payload")?;
        let id = payload.get("id")?.as_str()?.to_string();
        let cwd = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Some((id, cwd));
    }
    None
}

fn detect_codex_session_id(home: &Path, worktree_path: &Path) -> Option<String> {
    let root = home.join(".codex").join("sessions");
    if !root.exists() {
        // Fallback: history.jsonl
        return detect_codex_from_history(home);
    }

    let target_str = worktree_path.to_string_lossy().to_string();
    let target_canon = fs::canonicalize(worktree_path).ok();
    let mut latest_match: Option<(SystemTime, String)> = None;
    let mut latest_any: Option<(SystemTime, String)> = None;

    for path in walk_files(&root, "jsonl") {
        let modified = path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let Some((id, cwd)) = parse_codex_session_meta(&path) else {
            continue;
        };
        update_latest(&mut latest_any, modified, &id);

        if let Some(cwd) = cwd {
            let matches = cwd == target_str
                || matches!(
                    (fs::canonicalize(&cwd).ok(), target_canon.as_ref()),
                    (Some(a), Some(b)) if a == *b
                );
            if matches {
                update_latest(&mut latest_match, modified, &id);
            }
        }
    }

    latest_match
        .or(latest_any)
        .map(|(_, id)| id)
        .or_else(|| detect_codex_from_history(home))
}

#[derive(Debug, serde::Deserialize)]
struct CodexHistoryEntry {
    session_id: String,
    ts: u64,
}

fn detect_codex_from_history(home: &Path) -> Option<String> {
    let path = home.join(".codex").join("history.jsonl");
    let file = fs::File::open(&path).ok()?;
    std::io::BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<CodexHistoryEntry>(&line).ok())
        .max_by_key(|e| e.ts)
        .map(|e| e.session_id)
}

// ---------------------------------------------------------------------------
// Gemini CLI
// ---------------------------------------------------------------------------

fn detect_gemini_session_id(home: &Path) -> Option<String> {
    let root = home.join(".gemini").join("tmp");
    if !root.exists() {
        return None;
    }
    let path = latest_file(&root, "json", Some("chats"))?;
    parse_generic_session_id(&path).or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })
}

// ---------------------------------------------------------------------------
// OpenCode
// ---------------------------------------------------------------------------

fn detect_opencode_session_id(home: &Path) -> Option<String> {
    let root = home.join(".local").join("share").join("opencode");
    if !root.exists() {
        return None;
    }
    let json = latest_file(&root, "json", None);
    let jsonl = latest_file(&root, "jsonl", None);
    let path = match (json, jsonl) {
        (Some(a), Some(b)) => {
            let ma = a.metadata().ok().and_then(|m| m.modified().ok());
            let mb = b.metadata().ok().and_then(|m| m.modified().ok());
            if ma >= mb {
                a
            } else {
                b
            }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    parse_generic_session_id(&path).or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn parse_generic_session_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let keys = [
        "session_id",
        "sessionId",
        "id",
        "chat_id",
        "chatId",
        "conversation_id",
        "conversationId",
    ];
    for key in keys {
        if let Some(val) = value.get(key) {
            if let Some(text) = val.as_str() {
                return Some(text.to_string());
            }
            if let Some(num) = val.as_i64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

/// Walk a directory tree collecting files with the given extension.
fn walk_files(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                result.push(path);
            }
        }
    }
    result
}

/// Find the most recently modified file with given extension under root.
/// Optionally require `path_contains` substring in the path.
fn latest_file(root: &Path, ext: &str, path_contains: Option<&str>) -> Option<PathBuf> {
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for path in walk_files(root, ext) {
        if let Some(sub) = path_contains {
            if !path.to_string_lossy().contains(sub) {
                continue;
            }
        }
        let modified = path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let dominated = latest.as_ref().is_some_and(|(t, _)| modified <= *t);
        if !dominated {
            latest = Some((modified, path));
        }
    }
    latest.map(|(_, p)| p)
}

/// Find the most recently modified .jsonl file stem in a directory (non-recursive).
fn latest_jsonl_stem(dir: &Path) -> Option<String> {
    let mut latest: Option<(SystemTime, String)> = None;
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let stem = path.file_stem().and_then(|s| s.to_str())?.to_string();
        let dominated = latest.as_ref().is_some_and(|(t, _)| modified <= *t);
        if !dominated {
            latest = Some((modified, stem));
        }
    }
    latest.map(|(_, s)| s)
}

fn update_latest(slot: &mut Option<(SystemTime, String)>, modified: SystemTime, id: &str) {
    let dominated = slot.as_ref().is_some_and(|(t, _)| modified <= *t);
    if !dominated {
        *slot = Some((modified, id.to_string()));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_home(tmp: &tempfile::TempDir) -> PathBuf {
        tmp.path().to_path_buf()
    }

    // -- Claude --

    #[test]
    fn claude_history_jsonl_matches_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let claude_dir = home.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let history = r#"{"display":"test","pastedContents":null,"timestamp":1000,"project":"/repo/feat","sessionId":"sess-abc"}
{"display":"test","pastedContents":null,"timestamp":2000,"project":"/repo/feat","sessionId":"sess-def"}"#;
        fs::write(claude_dir.join("history.jsonl"), history).unwrap();

        let result = detect_session_id_for_tool_at(&home, "claude", Path::new("/repo/feat"));
        assert_eq!(result.as_deref(), Some("sess-def"));
    }

    #[test]
    fn claude_history_no_match_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let claude_dir = home.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let history = r#"{"display":"test","pastedContents":null,"timestamp":1000,"project":"/other","sessionId":"sess-abc"}"#;
        fs::write(claude_dir.join("history.jsonl"), history).unwrap();

        let result = detect_session_id_for_tool_at(&home, "claude", Path::new("/repo/feat"));
        assert_eq!(result, None);
    }

    #[test]
    fn claude_fallback_to_project_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let worktree = Path::new("/repo/feat");
        let encoded = encode_claude_project_path(worktree);
        let project_dir = home.join(".claude").join("projects").join(&encoded);
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("my-session-id.jsonl"), "{}").unwrap();

        let result = detect_session_id_for_tool_at(&home, "claude", worktree);
        assert_eq!(result.as_deref(), Some("my-session-id"));
    }

    #[test]
    fn claude_no_files_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let result = detect_session_id_for_tool_at(&home, "claude", Path::new("/repo/feat"));
        assert_eq!(result, None);
    }

    // -- Codex --

    #[test]
    fn codex_session_cwd_match() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let sessions = home.join(".codex").join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let content = r#"{"payload":{"id":"codex-sess-1","cwd":"/repo/feat"}}"#;
        fs::write(sessions.join("s1.jsonl"), content).unwrap();

        let result = detect_session_id_for_tool_at(&home, "codex", Path::new("/repo/feat"));
        assert_eq!(result.as_deref(), Some("codex-sess-1"));
    }

    #[test]
    fn codex_fallback_to_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let codex_dir = home.join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        let history = r#"{"session_id":"hist-sess","ts":100,"text":"test"}"#;
        fs::write(codex_dir.join("history.jsonl"), history).unwrap();

        let result = detect_session_id_for_tool_at(&home, "codex", Path::new("/repo/feat"));
        assert_eq!(result.as_deref(), Some("hist-sess"));
    }

    #[test]
    fn codex_no_files_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let result = detect_session_id_for_tool_at(&home, "codex", Path::new("/repo/feat"));
        assert_eq!(result, None);
    }

    // -- Gemini --

    #[test]
    fn gemini_finds_latest_chat_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let chats = home.join(".gemini").join("tmp").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(chats.join("gem-sess.json"), r#"{"session_id":"gem-sess"}"#).unwrap();

        let result = detect_session_id_for_tool_at(&home, "gemini", Path::new("/repo"));
        assert_eq!(result.as_deref(), Some("gem-sess"));
    }

    #[test]
    fn gemini_no_files_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let result = detect_session_id_for_tool_at(&home, "gemini", Path::new("/repo"));
        assert_eq!(result, None);
    }

    // -- OpenCode --

    #[test]
    fn opencode_finds_latest_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let oc_dir = home.join(".local").join("share").join("opencode");
        fs::create_dir_all(&oc_dir).unwrap();
        fs::write(oc_dir.join("sess.json"), r#"{"id":"oc-sess-1"}"#).unwrap();

        let result = detect_session_id_for_tool_at(&home, "opencode", Path::new("/repo"));
        assert_eq!(result.as_deref(), Some("oc-sess-1"));
    }

    #[test]
    fn opencode_no_files_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let result = detect_session_id_for_tool_at(&home, "opencode", Path::new("/repo"));
        assert_eq!(result, None);
    }

    // -- Unknown agent --

    #[test]
    fn unknown_agent_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = make_home(&tmp);
        let result = detect_session_id_for_tool_at(&home, "unknown-tool", Path::new("/repo"));
        assert_eq!(result, None);
    }
}

//! Claude Code session parser.

use super::{
    find_session_file, parse_jsonl_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};
use chrono::{DateTime, Utc};
use crate::ai::claude_paths::encode_claude_project_path;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ClaudeSessionParser {
    home_dir: PathBuf,
}

impl ClaudeSessionParser {
    pub fn new(home_dir: PathBuf) -> Self {
        Self { home_dir }
    }

    pub fn with_default_home() -> Option<Self> {
        dirs::home_dir().map(Self::new)
    }

    fn base_dir(&self) -> PathBuf {
        self.home_dir.join(".claude").join("projects")
    }
}

impl SessionParser for ClaudeSessionParser {
    fn parse(&self, session_id: &str) -> Result<super::ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        parse_jsonl_session(&path, session_id, AgentType::ClaudeCode)
    }

    fn agent_type(&self) -> AgentType {
        AgentType::ClaudeCode
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        let root = self.base_dir();
        if let Some(found) = find_session_file(&root, session_id, &["jsonl", "json"]) {
            return found;
        }
        root.join(format!("{}.jsonl", session_id))
    }

    fn list_sessions(&self, worktree_path: Option<&Path>) -> Vec<SessionListEntry> {
        let root = self.base_dir();
        if !root.exists() {
            return vec![];
        }

        let mut entries = if let Some(wt_path) = worktree_path {
            self.collect_sessions_for_worktree(&root, wt_path)
        } else {
            self.collect_all_sessions(&root)
        };

        // Sort by last_updated (newest first)
        entries.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        entries
    }
}

impl ClaudeSessionParser {
    fn collect_sessions_for_worktree(
        &self,
        root: &Path,
        worktree_path: &Path,
    ) -> Vec<SessionListEntry> {
        // Claude Code stores sessions under:
        // `~/.claude/projects/{encoded-worktree-path}/{session-id}.jsonl`
        let encoded = encode_claude_project_path(worktree_path);
        let dir = root.join(encoded);
        if !dir.is_dir() {
            return Vec::new();
        }
        self.collect_sessions_from_dir(&dir)
    }

    fn collect_all_sessions(&self, root: &Path) -> Vec<SessionListEntry> {
        let mut results = Vec::new();
        let entries = match fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => return results,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(self.collect_sessions_from_dir(&path));
            }
        }

        results
    }

    fn collect_sessions_from_dir(&self, dir: &Path) -> Vec<SessionListEntry> {
        let mut results = Vec::new();
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return results,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "jsonl" && ext != "json" {
                continue;
            }

            // Extract session ID from filename
            let session_id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            if session_id.is_empty() {
                continue;
            }

            // Get file metadata for last_updated
            let last_updated = fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from);

            // Count lines for approximate message count (JSONL format)
            let message_count = fs::read_to_string(&path)
                .map(|content| content.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(0);

            results.push(SessionListEntry {
                session_id,
                last_updated,
                message_count,
                file_path: path,
            });
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn list_sessions_filters_by_encoded_worktree_dir() {
        let dir = tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let parser = ClaudeSessionParser::new(home.clone());

        let worktree = PathBuf::from("/repo/worktrees/feature-x");
        let encoded = encode_claude_project_path(&worktree);
        let project_dir = home.join(".claude").join("projects").join(encoded);
        fs::create_dir_all(&project_dir).unwrap();

        // Session for target worktree
        fs::write(project_dir.join("sess-1.jsonl"), r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-02-09T00:00:00.000Z"}"#).unwrap();

        // Another project dir should not be included when filtering.
        let other_dir = home
            .join(".claude")
            .join("projects")
            .join("other-project");
        fs::create_dir_all(&other_dir).unwrap();
        fs::write(other_dir.join("sess-2.jsonl"), r#"{"type":"user","message":{"role":"user","content":"nope"},"timestamp":"2026-02-09T00:00:00.000Z"}"#).unwrap();

        let filtered = parser.list_sessions(Some(&worktree));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "sess-1");

        let all = parser.list_sessions(None);
        assert_eq!(all.len(), 2);
    }
}

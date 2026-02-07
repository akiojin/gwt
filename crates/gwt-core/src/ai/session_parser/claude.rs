//! Claude Code session parser.

use super::{
    find_session_file, parse_jsonl_session, AgentType, SessionListEntry, SessionParseError,
    SessionParser,
};
use chrono::{DateTime, Utc};
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

        let mut entries = Vec::new();

        // If worktree_path is specified, prioritize that project directory
        if let Some(wt_path) = worktree_path {
            // Claude Code stores sessions under .claude/projects/<project-hash>/
            // Try to find the project directory for this worktree
            if let Some(project_entries) = self.find_sessions_for_worktree(&root, wt_path) {
                entries.extend(project_entries);
            }
        }

        // If no worktree-specific sessions found, search all projects
        if entries.is_empty() {
            entries = self.collect_all_sessions(&root);
        }

        // Sort by last_updated (newest first)
        entries.sort_by(|a, b| b.last_updated.cmp(&a.last_updated));
        entries
    }
}

impl ClaudeSessionParser {
    fn find_sessions_for_worktree(
        &self,
        root: &Path,
        worktree_path: &Path,
    ) -> Option<Vec<SessionListEntry>> {
        // Claude Code uses a hash of the project path as directory name
        // We need to search through project directories to find matching ones
        let entries = fs::read_dir(root).ok()?;
        let mut results = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Check if this project directory contains sessions related to our worktree
            // by checking if the directory name matches or contains worktree info
            let sessions = self.collect_sessions_from_dir(&path);
            for session in sessions {
                // Check if session file path contains worktree path component
                if session
                    .file_path
                    .to_string_lossy()
                    .contains(&worktree_path.to_string_lossy().to_string())
                {
                    results.push(session);
                }
            }
        }

        if results.is_empty() {
            None
        } else {
            Some(results)
        }
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

//! Compatibility shim for APIs that were in the old monolithic gwt-core
//! but have not yet been migrated to the domain crates (gwt-git, gwt-agent,
//! gwt-config, gwt-ai, etc.).
//!
//! Each stub is marked with a `TODO:` comment indicating which domain crate
//! should eventually own the implementation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// git::*  (TODO: move to gwt-git)
// ---------------------------------------------------------------------------

/// A GitHub issue summary.
#[derive(Debug, Clone)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub updated_at: String,
    pub html_url: String,
    pub body: Option<String>,
    pub labels: Vec<GitHubLabel>,
}

#[derive(Debug, Clone)]
pub struct GitHubLabel {
    pub name: String,
}

/// Fetch a single GitHub issue via `gh issue view`.
pub fn fetch_issue_detail(repo_root: &Path, issue_number: u64) -> Result<GitHubIssue, String> {
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "view",
            &issue_number.to_string(),
            "--json",
            "number,title,state,updatedAt,url,body,labels",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("failed to run gh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue view failed: {stderr}"));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("parse error: {e}"))?;

    Ok(GitHubIssue {
        number: json["number"].as_u64().unwrap_or(issue_number),
        title: json["title"].as_str().unwrap_or("").to_string(),
        state: json["state"].as_str().unwrap_or("OPEN").to_string(),
        updated_at: json["updatedAt"].as_str().unwrap_or("").to_string(),
        html_url: json["url"].as_str().unwrap_or("").to_string(),
        body: json["body"].as_str().map(String::from),
        labels: json["labels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v["name"].as_str().map(|n| GitHubLabel { name: n.to_string() }))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Find a branch name associated with an issue number.
pub fn find_branch_for_issue(repo_root: &Path, issue_number: u64) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["branch", "--list", &format!("*{issue_number}*")])
        .current_dir(repo_root)
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|l| l.trim().trim_start_matches("* ").to_string())
        .find(|l| !l.is_empty())
}

/// Generate a branch name from a prefix and issue number.
pub fn generate_branch_name(prefix: &str, issue_number: u64) -> String {
    format!("{prefix}{issue_number}")
}

// ---------------------------------------------------------------------------
// git::issue_cache  (TODO: move to gwt-git)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IssueExactCacheEntry {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub fetched_at: u64,
}

/// GitHub issue as returned by `gh`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<FetchedLabel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchedLabel {
    pub name: String,
}

/// Result of fetching issues.
pub struct FetchIssuesResult {
    pub issues: Vec<FetchedIssue>,
}

/// Fetch issues via `gh issue list`.
pub fn fetch_issues_with_options(
    repo_root: &Path,
    _page: usize,
    limit: usize,
    state: &str,
    _include_prs: bool,
    _issue_type: &str,
) -> Result<FetchIssuesResult, String> {
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "list",
            "--state",
            state,
            "--limit",
            &limit.to_string(),
            "--json",
            "number,title,state,labels",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("failed to run gh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh issue list failed: {stderr}"));
    }

    let issues: Vec<FetchedIssue> =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("parse: {e}"))?;
    Ok(FetchIssuesResult { issues })
}

/// Simple issue cache backed by JSON file.
#[derive(Debug, Clone, Default)]
pub struct IssueExactCache {
    path: PathBuf,
    entries: HashMap<u64, IssueExactCacheEntry>,
}

impl IssueExactCache {
    pub fn load(repo_root: &Path) -> Self {
        let path = repo_root.join(".gwt").join("issue-cache.json");
        let list: Vec<IssueExactCacheEntry> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let entries = list.into_iter().map(|e| (e.number, e)).collect();
        Self { path, entries }
    }

    pub fn upsert(&mut self, entry: IssueExactCacheEntry) {
        self.entries.insert(entry.number, entry);
    }

    pub fn all_entries(&self) -> &HashMap<u64, IssueExactCacheEntry> {
        &self.entries
    }

    pub fn get(&self, number: u64) -> Option<&IssueExactCacheEntry> {
        self.entries.get(&number)
    }

    pub fn entry_from_github_issue(issue: &FetchedIssue) -> IssueExactCacheEntry {
        IssueExactCacheEntry {
            number: issue.number,
            title: issue.title.clone(),
            state: issue.state.clone(),
            labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
            ..Default::default()
        }
    }

    pub fn save(&self, repo_root: &Path) -> Result<(), String> {
        let path = repo_root.join(".gwt").join("issue-cache.json");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let list: Vec<&IssueExactCacheEntry> = self.entries.values().collect();
        let json =
            serde_json::to_string_pretty(&list).map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("write: {e}"))
    }
}

// ---------------------------------------------------------------------------
// git::issue_linkage  (TODO: move to gwt-git)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum LinkSource {
    BranchParse,
    Manual,
}

/// Worktree-to-issue link store (stub).
pub struct WorktreeIssueLinkStore;

impl WorktreeIssueLinkStore {
    pub fn load(_repo_root: &Path) -> Self {
        Self
    }

    pub fn link(&mut self, _branch: &str, _issue: u64, _source: LinkSource) {}
    pub fn issue_for_branch(&self, _branch: &str) -> Option<u64> {
        None
    }
    pub fn get_link(&self, _branch: &str) -> Option<u64> {
        None
    }
}

// ---------------------------------------------------------------------------
// git::Branch / PrCache  (TODO: already in gwt-git, adapt)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub has_remote: bool,
    pub commit_timestamp: Option<i64>,
}

impl Branch {
    pub fn list(repo_root: &Path) -> Vec<Branch> {
        let output = std::process::Command::new("git")
            .args(["branch", "-a", "--format=%(refname:short)%(if)%(upstream:short)%(then) -> %(upstream:short)%(end)"])
            .current_dir(repo_root)
            .output()
            .ok();

        match output {
            Some(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                stdout
                    .lines()
                    .filter(|l| !l.is_empty() && !l.contains("HEAD"))
                    .map(|l| {
                        let (name, upstream) = if let Some((n, u)) = l.split_once(" -> ") {
                            (n.to_string(), Some(u.to_string()))
                        } else {
                            (l.to_string(), None)
                        };
                        Branch {
                            name,
                            is_current: false,
                            upstream,
                            ahead: 0,
                            behind: 0,
                            has_remote: false,
                            commit_timestamp: None,
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn list_remote(repo_root: &Path) -> Vec<Branch> {
        Self::list(repo_root)
            .into_iter()
            .filter(|b| b.name.starts_with("origin/"))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct PrCache {
    pub entries: HashMap<String, PrCacheEntry>,
}

#[derive(Debug, Clone)]
pub struct PrCacheEntry {
    pub number: u64,
    pub title: String,
    pub state: String,
}

impl PrCache {
    pub fn new(_repo_root: &Path) -> Self {
        Self::default()
    }

    pub fn load(_repo_root: &Path) -> Self {
        Self::default()
    }

    pub fn get(&self, branch: &str) -> Option<&PrCacheEntry> {
        self.entries.get(branch)
    }

    pub fn populate(&mut self, _repo_root: &Path) {
        // Stub: PR cache population not yet implemented
    }
}

// ---------------------------------------------------------------------------
// worktree::*  (TODO: move to gwt-git/worktree)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeStatus {
    Clean,
    Modified,
    Unknown,
    Active,
    Locked,
    Prunable,
    Missing,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub status: WorktreeStatus,
    pub has_changes: bool,
    pub has_unpushed: bool,
}

/// Worktree manager (stub).
pub struct WorktreeManager {
    repo_root: PathBuf,
}

impl WorktreeManager {
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
        }
    }

    pub fn list(&self) -> Option<Vec<Worktree>> {
        let output = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.repo_root)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut worktrees = Vec::new();
        let mut current_path = None;
        let mut current_branch = None;
        for line in stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_path = Some(PathBuf::from(path));
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                current_branch = Some(branch.to_string());
            } else if line.is_empty() {
                if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                    worktrees.push(Worktree {
                        path,
                        branch,
                        status: WorktreeStatus::Unknown,
                        has_changes: false,
                        has_unpushed: false,
                    });
                }
                current_path = None;
                current_branch = None;
            }
        }
        if let (Some(path), Some(branch)) = (current_path, current_branch) {
            worktrees.push(Worktree {
                path,
                branch,
                status: WorktreeStatus::Unknown,
                has_changes: false,
                has_unpushed: false,
            });
        }
        Some(worktrees)
    }

    pub fn create_new_branch(
        &self,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<PathBuf, gwt_core::error::GwtError> {
        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch_name,
                branch_name,
                base_branch,
            ])
            .current_dir(&self.repo_root)
            .output()
            .map_err(|e| gwt_core::error::GwtError::Git(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(gwt_core::error::GwtError::Git(stderr.to_string()));
        }

        Ok(self.repo_root.join(branch_name))
    }
}

// ---------------------------------------------------------------------------
// config::*  (TODO: move to gwt-config)
// ---------------------------------------------------------------------------

/// Session entry for tool history tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolSessionEntry {
    #[serde(default)]
    pub tool_id: String,
    #[serde(default)]
    pub branch_name: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub agent_label: String,
    #[serde(default)]
    pub tool_label: Option<String>,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning_level: Option<String>,
    #[serde(default)]
    pub skip_permissions: Option<bool>,
    #[serde(default)]
    pub tool_version: Option<String>,
    #[serde(default)]
    pub collaboration_modes: Option<Vec<String>>,
    #[serde(default)]
    pub docker_service: Option<String>,
    #[serde(default)]
    pub docker_force_host: Option<bool>,
    #[serde(default)]
    pub docker_recreate: Option<bool>,
    #[serde(default)]
    pub docker_build: Option<bool>,
    #[serde(default)]
    pub docker_keep: Option<bool>,
    #[serde(default)]
    pub docker_container_name: Option<String>,
    #[serde(default)]
    pub docker_compose_args: Option<Vec<String>>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

impl ToolSessionEntry {
    pub fn format_tool_usage(&self) -> String {
        format!("{} ({})", self.agent_label, self.tool_id)
    }
}

/// Save a session entry (stub).
pub fn save_session_entry(
    _repo_root: &Path,
    _entry: ToolSessionEntry,
) -> Result<(), String> {
    // Stub: persistence not yet implemented in split crates
    Ok(())
}

/// Get last tool usage map (stub).
pub fn get_last_tool_usage_map(_repo_root: &Path) -> HashMap<String, ToolSessionEntry> {
    HashMap::new()
}

/// Get branch tool history (stub).
pub fn get_branch_tool_history_for_worktree(
    _repo_root: &Path,
    _branch_name: &str,
) -> Vec<ToolSessionEntry> {
    Vec::new()
}

/// Check if codex hooks need update (stub).
pub fn codex_hooks_needs_update(_codex_root: &Path) -> bool {
    false
}

// ---------------------------------------------------------------------------
// config::skill_registration  (TODO: move to gwt-skills)
// ---------------------------------------------------------------------------

pub mod skill_registration {
    use std::path::Path;

    #[derive(Debug, Clone, Copy)]
    pub enum SkillAgentType {
        Claude,
        Codex,
    }

    impl SkillAgentType {
        pub fn from_agent_id(agent_id: &str) -> Option<Self> {
            match agent_id {
                "claude" => Some(Self::Claude),
                "codex" => Some(Self::Codex),
                _ => None,
            }
        }
    }

    pub fn install_skills_if_needed(_repo_root: &Path) -> Result<(), String> {
        Ok(())
    }

    pub fn installed_skill_names(_repo_root: &Path) -> Vec<String> {
        Vec::new()
    }

    pub fn register_agent_skills_with_settings_at_project_root(
        _repo_root: &Path,
        _agent_type: SkillAgentType,
    ) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// agent::launch  (TODO: move to gwt-agent)
// ---------------------------------------------------------------------------

pub mod agent_launch {
    use gwt_agent::types::AgentColor;
    use std::collections::HashMap;
    use std::path::PathBuf;

    pub fn agent_color_for(_agent_id: &str) -> AgentColor {
        AgentColor::Green
    }

    pub fn find_agent_def(agent_id: &str) -> Option<AgentDef> {
        Some(AgentDef {
            id: agent_id.to_string(),
            display_name: agent_id.to_string(),
        })
    }

    #[derive(Debug, Clone)]
    pub struct AgentDef {
        pub id: String,
        pub display_name: String,
    }

    /// Shell launch builder (stub).
    pub struct ShellLaunchBuilder {
        pub working_dir: PathBuf,
        pub branch_name: String,
        pub env_vars: HashMap<String, String>,
    }

    impl ShellLaunchBuilder {
        pub fn new(working_dir: PathBuf) -> Self {
            Self {
                working_dir,
                branch_name: String::new(),
                env_vars: HashMap::new(),
            }
        }

        pub fn branch_name(mut self, name: &str) -> Self {
            self.branch_name = name.to_string();
            self
        }

        pub fn env_vars(mut self, vars: HashMap<String, String>) -> Self {
            self.env_vars = vars;
            self
        }

        pub fn build(self) -> LaunchResult {
            LaunchResult {
                command: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
                args: Vec::new(),
                env: self.env_vars,
                working_dir: self.working_dir,
            }
        }
    }

    /// Agent launch builder (stub).
    pub struct AgentLaunchBuilder {
        pub agent_id: String,
        pub working_dir: PathBuf,
        pub branch_name: String,
        pub spec_id: Option<String>,
        pub env_vars: HashMap<String, String>,
        pub session_mode: SessionMode,
        pub model: Option<String>,
    }

    #[derive(Debug, Clone, Copy, Default)]
    pub enum SessionMode {
        #[default]
        Normal,
        Resume,
    }

    impl AgentLaunchBuilder {
        pub fn new(agent_id: &str, working_dir: PathBuf) -> Self {
            Self {
                agent_id: agent_id.to_string(),
                working_dir,
                branch_name: String::new(),
                spec_id: None,
                env_vars: HashMap::new(),
                session_mode: SessionMode::Normal,
                model: None,
            }
        }

        pub fn branch_name(mut self, name: &str) -> Self {
            self.branch_name = name.to_string();
            self
        }

        pub fn spec_id(mut self, id: &str) -> Self {
            self.spec_id = Some(id.to_string());
            self
        }

        pub fn env_vars(mut self, vars: HashMap<String, String>) -> Self {
            self.env_vars = vars;
            self
        }

        pub fn session_mode(mut self, mode: SessionMode) -> Self {
            self.session_mode = mode;
            self
        }

        pub fn model(mut self, model: &str) -> Self {
            self.model = Some(model.to_string());
            self
        }

        pub fn skip_permissions(self, _skip: bool) -> Self {
            self
        }

        pub fn reasoning_level(self, _level: &str) -> Self {
            self
        }

        pub fn resume_session_id(self, _id: &str) -> Self {
            self
        }

        pub fn agent_version(self, _version: &str) -> Self {
            self
        }

        pub fn fast_mode(self, _fast: bool) -> Self {
            self
        }

        pub fn build(self) -> LaunchResult {
            LaunchResult {
                command: self.agent_id,
                args: Vec::new(),
                env: self.env_vars,
                working_dir: self.working_dir,
            }
        }
    }

    /// Result of building a launch configuration.
    pub struct LaunchResult {
        pub command: String,
        pub args: Vec<String>,
        pub env: HashMap<String, String>,
        pub working_dir: PathBuf,
    }
}

// ---------------------------------------------------------------------------
// ai::*  (TODO: move to gwt-ai)
// ---------------------------------------------------------------------------

pub fn detect_session_id_for_tool(_tool_id: &str, _working_dir: &Path) -> Option<String> {
    None
}

// ---------------------------------------------------------------------------
// git::hooks  (TODO: move to gwt-git)
// ---------------------------------------------------------------------------

pub mod git_hooks {
    use std::path::Path;

    pub fn is_develop_guard_installed(_repo_root: &Path) -> bool {
        true
    }

    pub fn install_pre_commit_hook(_repo_root: &Path) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// logging::*  (TODO: move to gwt-notification or separate logging crate)
// ---------------------------------------------------------------------------

pub mod logging {
    use std::path::{Path, PathBuf};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LogEntry {
        #[serde(default)]
        pub timestamp: String,
        #[serde(default)]
        pub level: String,
        #[serde(default, rename = "message")]
        msg: String,
        #[serde(default)]
        pub target: String,
        #[serde(default)]
        pub fields: serde_json::Value,
    }

    impl LogEntry {
        pub fn message(&self) -> &str {
            &self.msg
        }

        pub fn category(&self) -> Option<&str> {
            self.fields.get("category").and_then(|v| v.as_str())
        }

        pub fn event(&self) -> Option<&str> {
            self.fields.get("event").and_then(|v| v.as_str())
        }

        pub fn result(&self) -> Option<&str> {
            self.fields.get("result").and_then(|v| v.as_str())
        }

        pub fn workspace(&self) -> Option<&str> {
            self.fields.get("workspace").and_then(|v| v.as_str())
        }

        pub fn error_code(&self) -> Option<&str> {
            self.fields.get("error_code").and_then(|v| v.as_str())
        }

        pub fn error_detail(&self) -> Option<&str> {
            self.fields.get("error_detail").and_then(|v| v.as_str())
        }
    }

    /// Log reader for structured logs.
    pub struct LogReader {
        log_dir: PathBuf,
    }

    impl LogReader {
        pub fn new(log_dir: &Path) -> Self {
            Self {
                log_dir: log_dir.to_path_buf(),
            }
        }

        /// Read entries from a log file with offset and limit.
        pub fn read_entries(
            path: &Path,
            _offset: usize,
            limit: usize,
        ) -> Result<(Vec<LogEntry>, usize), String> {
            let content =
                std::fs::read_to_string(path).map_err(|e| format!("read log: {e}"))?;
            let mut entries = Vec::new();
            let mut total = 0usize;
            for line in content.lines() {
                total += 1;
                if entries.len() >= limit {
                    continue; // count total but skip collection
                }
                if let Ok(entry) = serde_json::from_str::<LogEntry>(line) {
                    entries.push(entry);
                }
            }
            Ok((entries, total))
        }

        pub fn list_files(&self) -> Result<Vec<PathBuf>, String> {
            let mut files = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&self.log_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("log")
                        || path.extension().and_then(|e| e.to_str()) == Some("json")
                    {
                        files.push(path);
                    }
                }
            }
            files.sort();
            files.reverse();
            Ok(files)
        }

        pub fn read_file(&self, path: &Path) -> Result<Vec<LogEntry>, String> {
            let content =
                std::fs::read_to_string(path).map_err(|e| format!("read log: {e}"))?;
            let mut entries = Vec::new();
            for line in content.lines() {
                if let Ok(entry) = serde_json::from_str::<LogEntry>(line) {
                    entries.push(entry);
                }
            }
            Ok(entries)
        }
    }
}

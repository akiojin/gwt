//! Compatibility shims bridging old gwt_core monolith types to new domain crates.
//!
//! The monolithic gwt-core was split into gwt-git, gwt-config, gwt-agent,
//! gwt-terminal, and gwt-ai. This module provides standalone types matching
//! the old API surface so the TUI can compile without rewriting every consumer.

#![allow(unused_variables, dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ===========================================================================
// compat::git
// ===========================================================================

pub mod git {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Branch {
        pub name: String,
        pub is_current: bool,
        pub has_remote: bool,
        pub upstream: Option<String>,
        pub commit: String,
        pub ahead: u32,
        pub behind: u32,
        pub commit_timestamp: Option<i64>,
        pub is_gone: bool,
    }

    impl Branch {
        pub fn list(repo_root: &Path) -> Vec<Branch> {
            gwt_git::branch::list_branches(repo_root)
                .unwrap_or_default()
                .into_iter()
                .filter(|b| b.is_local)
                .map(|b| Branch {
                    is_current: b.is_head,
                    has_remote: b.upstream.is_some(),
                    upstream: b.upstream,
                    commit: String::new(),
                    ahead: b.ahead,
                    behind: b.behind,
                    commit_timestamp: parse_iso_timestamp(b.last_commit_date.as_deref()),
                    is_gone: false,
                    name: b.name,
                })
                .collect()
        }

        pub fn list_remote(repo_root: &Path) -> Vec<Branch> {
            gwt_git::branch::list_branches(repo_root)
                .unwrap_or_default()
                .into_iter()
                .filter(|b| b.is_remote)
                .map(|b| Branch {
                    is_current: false,
                    has_remote: true,
                    upstream: None,
                    commit: String::new(),
                    ahead: 0,
                    behind: 0,
                    commit_timestamp: parse_iso_timestamp(b.last_commit_date.as_deref()),
                    is_gone: false,
                    name: b.name,
                })
                .collect()
        }
    }

    fn parse_iso_timestamp(date: Option<&str>) -> Option<i64> {
        date.and_then(|s| {
            chrono::DateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S %z").ok()
        })
        .map(|dt| dt.timestamp())
    }

    #[derive(Debug, Default)]
    pub struct PrCache {
        entries: HashMap<String, PrEntry>,
    }

    #[derive(Debug, Clone)]
    pub struct PrEntry {
        pub title: String,
        pub number: u64,
        pub state: String,
    }

    impl PrCache {
        pub fn new() -> Self { Self::default() }
        pub fn populate(&mut self, _repo_root: &Path) {}
        pub fn get(&self, branch: &str) -> Option<&PrEntry> { self.entries.get(branch) }
    }

    pub mod issue_cache {
        use super::*;

        #[derive(Debug, Default, Clone)]
        pub struct IssueExactCache {
            entries: HashMap<u64, IssueExactCacheEntry>,
        }

        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct IssueExactCacheEntry {
            pub number: u64,
            pub title: String,
            pub url: String,
            pub state: String,
            pub labels: Vec<String>,
            pub updated_at: String,
            pub fetched_at: u64,
        }

        impl IssueExactCache {
            pub fn load(repo_root: &Path) -> Self {
                let path = repo_root.join(".gwt").join("issue-cache.json");
                if let Ok(c) = std::fs::read_to_string(&path) {
                    if let Ok(entries) = serde_json::from_str::<Vec<IssueExactCacheEntry>>(&c) {
                        return Self { entries: entries.into_iter().map(|e| (e.number, e)).collect() };
                    }
                }
                Self::default()
            }

            pub fn get(&self, number: u64) -> Option<&IssueExactCacheEntry> { self.entries.get(&number) }
            pub fn upsert(&mut self, entry: IssueExactCacheEntry) { self.entries.insert(entry.number, entry); }

            pub fn save(&self, repo_root: &Path) -> Result<(), String> {
                let cache_dir = repo_root.join(".gwt");
                std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
                let entries: Vec<&IssueExactCacheEntry> = self.entries.values().collect();
                let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
                std::fs::write(cache_dir.join("issue-cache.json"), json).map_err(|e| e.to_string())?;
                Ok(())
            }

            pub fn all_entries(&self) -> Vec<&IssueExactCacheEntry> { self.entries.values().collect() }

            pub fn entry_from_github_issue(issue: &super::GitHubIssue) -> IssueExactCacheEntry {
                IssueExactCacheEntry {
                    number: issue.number,
                    title: issue.title.clone(),
                    url: issue.url.clone(),
                    state: issue.state.clone(),
                    labels: issue.labels.iter().map(|l| l.name.clone()).collect(),
                    updated_at: issue.updated_at.clone(),
                    fetched_at: 0,
                }
            }
        }
    }

    pub mod issue_linkage {
        use super::*;

        #[derive(Debug, Default)]
        pub struct WorktreeIssueLinkStore {
            links: HashMap<String, IssueLink>,
        }

        #[derive(Debug, Clone)]
        pub struct IssueLink { pub issue_number: u64, pub source: LinkSource }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum LinkSource { BranchParse, Manual }

        impl WorktreeIssueLinkStore {
            pub fn load(_repo_root: &Path) -> Self { Self::default() }
            pub fn get_link(&self, branch: &str) -> Option<&IssueLink> { self.links.get(branch) }
            pub fn set_link(&mut self, branch: &str, issue_number: u64, source: LinkSource) {
                self.links.insert(branch.to_string(), IssueLink { issue_number, source });
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct GitHubIssue {
        pub number: u64,
        pub title: String,
        pub body: Option<String>,
        pub state: String,
        pub labels: Vec<GitHubLabel>,
        pub url: String,
        pub html_url: String,
        pub created_at: String,
        pub updated_at: String,
        pub comments: Vec<GitHubComment>,
    }

    #[derive(Debug, Clone)]
    pub struct GitHubLabel { pub name: String }

    #[derive(Debug, Clone)]
    pub struct GitHubComment { pub author: String, pub body: String, pub created_at: String }

    pub fn fetch_issue_detail(_repo_root: &Path, _number: u64) -> Result<GitHubIssue, String> {
        Err("Issue detail not yet available".to_string())
    }

    pub fn find_branch_for_issue(_repo_root: &Path, _issue_number: u64) -> Option<String> { None }

    pub fn generate_branch_name(prefix: &str, issue_number: u64) -> String {
        format!("{prefix}issue-{issue_number}")
    }

    pub fn fetch_issues_with_options(_repo_root: &Path, _state: &str, _limit: usize) -> Vec<GitHubIssue> {
        Vec::new()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RepoType { Normal, Worktree, Empty, NonRepo }

    pub fn detect_repo_type(path: &Path) -> RepoType {
        let git_dir = path.join(".git");
        if git_dir.is_dir() { RepoType::Normal }
        else if git_dir.is_file() { RepoType::Worktree }
        else { RepoType::NonRepo }
    }

    #[derive(Debug, Clone)]
    pub struct SpecSection { pub label: String, pub content: String }

    #[derive(Debug, Clone)]
    pub struct LocalSpecDetail {
        pub id: String,
        pub title: String,
        pub status: String,
        pub phase: String,
        pub spec_md: Option<String>,
        pub plan_md: Option<String>,
        pub tasks_md: Option<String>,
        pub sections: Vec<SpecSection>,
    }

    pub fn get_local_spec_detail(repo_root: &Path, spec_id: &str) -> Result<LocalSpecDetail, String> {
        let spec_dir = repo_root.join("specs").join(format!("SPEC-{spec_id}"));
        if !spec_dir.is_dir() { return Err(format!("SPEC-{spec_id} not found")); }
        let metadata_path = spec_dir.join("metadata.json");
        let (title, status, phase) = if metadata_path.exists() {
            let content = std::fs::read_to_string(&metadata_path).map_err(|e| e.to_string())?;
            let v: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
            (
                v["title"].as_str().unwrap_or("Untitled").to_string(),
                v["status"].as_str().unwrap_or("unknown").to_string(),
                v["phase"].as_str().unwrap_or("unknown").to_string(),
            )
        } else {
            ("Untitled".into(), "unknown".into(), "unknown".into())
        };
        let spec_md = std::fs::read_to_string(spec_dir.join("spec.md")).ok();
        let plan_md = std::fs::read_to_string(spec_dir.join("plan.md")).ok();
        let tasks_md = std::fs::read_to_string(spec_dir.join("tasks.md")).ok();
        let mut sections = Vec::new();
        if let Some(ref s) = spec_md { sections.push(SpecSection { label: "Spec".into(), content: s.clone() }); }
        if let Some(ref s) = plan_md { sections.push(SpecSection { label: "Plan".into(), content: s.clone() }); }
        if let Some(ref s) = tasks_md { sections.push(SpecSection { label: "Tasks".into(), content: s.clone() }); }
        Ok(LocalSpecDetail { id: spec_id.to_string(), title, status, phase, spec_md, plan_md, tasks_md, sections })
    }

    pub mod hooks {
        use std::path::Path;
        pub fn is_develop_guard_installed(_repo_root: &Path) -> bool { false }
        pub fn install_pre_commit_hook(_repo_root: &Path) -> Result<(), String> { Ok(()) }
    }
}

// ===========================================================================
// compat::worktree
// ===========================================================================

pub mod worktree {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Worktree {
        pub path: PathBuf,
        pub branch: Option<String>,
        pub commit: String,
        pub status: WorktreeStatus,
        pub is_main: bool,
        pub has_changes: bool,
        pub has_unpushed: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WorktreeStatus { Active, Locked, Prunable, Missing }

    pub struct WorktreeManager { repo_root: PathBuf }

    impl WorktreeManager {
        pub fn new(repo_root: &Path) -> Result<Self, String> {
            Ok(Self { repo_root: repo_root.to_path_buf() })
        }
        pub fn list(&self) -> Result<Vec<Worktree>, String> {
            let manager = gwt_git::WorktreeManager::new(&self.repo_root);
            let infos = manager.list().map_err(|e| e.to_string())?;
            Ok(infos.into_iter().map(|w| Worktree {
                is_main: w.branch.as_deref().is_some_and(|b| b == "main" || b == "master"),
                has_changes: false, has_unpushed: false, commit: String::new(),
                status: if w.prunable { WorktreeStatus::Prunable }
                        else if w.locked { WorktreeStatus::Locked }
                        else { WorktreeStatus::Active },
                path: w.path, branch: w.branch,
            }).collect())
        }
        pub fn create_for_branch(&self, branch: &str, path: &Path) -> Result<(), String> {
            gwt_git::WorktreeManager::new(&self.repo_root).create(branch, path).map_err(|e| e.to_string())
        }
        pub fn create_new_branch(&self, _branch: &str, _base: &str, _path: &Path) -> Result<(), String> {
            Err("create_new_branch not yet implemented".into())
        }
        pub fn get_by_branch(&self, branch: &str) -> Option<Worktree> {
            self.list().ok()?.into_iter().find(|w| w.branch.as_deref() == Some(branch))
        }
    }
}

// ===========================================================================
// compat::config
// ===========================================================================

pub mod config {
    use super::*;

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Profile {
        #[serde(default)]
        pub name: String,
        #[serde(default)]
        pub description: String,
        #[serde(default)]
        pub env: HashMap<String, String>,
        #[serde(default)]
        pub disabled_env: Vec<String>,
        #[serde(default)]
        pub ai: Option<AISettings>,
        #[serde(default)]
        pub ai_enabled: Option<bool>,
    }

    impl Profile {
        pub fn new(name: &str) -> Self {
            Self { name: name.to_string(), ..Default::default() }
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct AISettings {
        #[serde(default)]
        pub endpoint: String,
        #[serde(default)]
        pub api_key: Option<String>,
        #[serde(default)]
        pub model: String,
        #[serde(default)]
        pub summary_enabled: bool,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ProfilesConfig {
        #[serde(default)]
        pub profiles: HashMap<String, Profile>,
        #[serde(default)]
        pub active: Option<String>,
        #[serde(default)]
        pub version: Option<u32>,
    }

    impl ProfilesConfig {
        pub fn load() -> Result<Self, String> {
            match gwt_config::Settings::load() {
                Ok(settings) => Ok(Self::from_gwt_config(&settings.profiles)),
                Err(_) => Ok(Self::default()),
            }
        }

        fn from_gwt_config(pc: &gwt_config::ProfilesConfig) -> Self {
            let mut profiles = HashMap::new();
            for p in &pc.profiles {
                profiles.insert(p.name.clone(), Profile {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    env: p.env_vars.clone(),
                    disabled_env: p.disabled_env.clone(),
                    ai: p.ai_settings.as_ref().map(|a| AISettings {
                        endpoint: a.endpoint.clone(),
                        api_key: a.api_key.clone(),
                        model: a.model.clone(),
                        summary_enabled: a.summary_enabled,
                    }),
                    ai_enabled: None,
                });
            }
            Self { profiles, active: pc.active.clone(), version: None }
        }

        pub fn set_active(&mut self, name: &str) -> Result<(), String> {
            if self.profiles.contains_key(name) {
                self.active = Some(name.to_string());
                Ok(())
            } else {
                Err(format!("profile '{}' not found", name))
            }
        }

        pub fn active_profile(&self) -> Option<&Profile> {
            self.active.as_ref().and_then(|name| self.profiles.get(name))
        }

        pub fn resolve_active_ai_settings(&self) -> Option<&AISettings> {
            self.active_profile().and_then(|p| p.ai.as_ref())
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct AgentSettings {
        pub default_agent: Option<String>,
        pub auto_install_deps: bool,
        pub claude_path: Option<PathBuf>,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct Settings {
        #[serde(default)]
        pub default_base_branch: String,
        #[serde(default)]
        pub debug: bool,
        #[serde(default)]
        pub log_retention_days: u32,
        #[serde(default)]
        pub worktree_root: String,
        #[serde(default)]
        pub protected_branches: Vec<String>,
        #[serde(default)]
        pub tools: ToolsConfig,
        #[serde(default)]
        pub profiles: ProfilesConfig,
        #[serde(default)]
        pub agent: AgentSettings,
    }

    impl Settings {
        pub fn load_global() -> Result<Self, String> {
            match gwt_config::Settings::load() {
                Ok(s) => Ok(Self {
                    default_base_branch: s.default_base_branch,
                    debug: s.debug,
                    log_retention_days: 30,
                    worktree_root: s.worktree_root.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    protected_branches: s.protected_branches,
                    tools: ToolsConfig::default(),
                    profiles: ProfilesConfig::from_gwt_config(&s.profiles),
                    agent: AgentSettings {
                        default_agent: s.agent.default_agent,
                        auto_install_deps: s.agent.auto_install_deps,
                        claude_path: None,
                    },
                }),
                Err(e) => Err(e.to_string()),
            }
        }

        pub fn load(_path: &Path) -> Result<Self, String> { Self::load_global() }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ToolsConfig {
        #[serde(default)]
        pub custom_coding_agents: Vec<CustomCodingAgent>,
    }

    impl ToolsConfig {
        pub fn empty() -> Self { Self::default() }
        pub fn add_agent(&mut self, agent: CustomCodingAgent) -> bool {
            if self.custom_coding_agents.iter().any(|a| a.id == agent.id) { return false; }
            self.custom_coding_agents.push(agent);
            true
        }
        pub fn update_agent(&mut self, agent: CustomCodingAgent) -> bool {
            if let Some(pos) = self.custom_coding_agents.iter().position(|a| a.id == agent.id) {
                self.custom_coding_agents[pos] = agent;
                true
            } else { false }
        }
        pub fn remove_agent(&mut self, id: &str) -> bool {
            let before = self.custom_coding_agents.len();
            self.custom_coding_agents.retain(|a| a.id != id);
            self.custom_coding_agents.len() < before
        }
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct CustomCodingAgent {
        pub id: String,
        pub display_name: String,
        pub agent_type: AgentType,
        pub command: String,
        #[serde(default)]
        pub default_args: Vec<String>,
        #[serde(default)]
        pub mode_args: Option<serde_json::Value>,
        #[serde(default)]
        pub permission_skip_args: Vec<String>,
        #[serde(default)]
        pub env: HashMap<String, String>,
        #[serde(default)]
        pub models: Vec<String>,
        #[serde(default)]
        pub version_command: Option<String>,
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum AgentType { #[default] Command, Path, Bunx }

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    pub struct ToolSessionEntry {
        pub tool_id: String,
        pub tool_label: String,
        pub tool_version: String,
        pub branch: String,
        pub session_id: Option<String>,
        pub working_dir: PathBuf,
        pub worktree_path: Option<PathBuf>,
        pub model: Option<String>,
        pub mode: Option<String>,
        pub skip_permissions: bool,
        pub reasoning_level: Option<String>,
        pub collaboration_modes: Vec<String>,
        pub docker_build: bool,
        pub docker_compose_args: Vec<String>,
        pub docker_container_name: Option<String>,
        pub docker_force_host: bool,
        pub docker_keep: bool,
        pub docker_recreate: bool,
        pub docker_service: Option<String>,
        #[serde(default)]
        pub timestamp: Option<String>,
    }

    impl ToolSessionEntry {
        pub fn format_tool_usage(&self) -> String {
            if self.tool_version.is_empty() { self.tool_label.clone() }
            else { format!("{}@{}", self.tool_label, self.tool_version) }
        }
    }

    pub fn save_session_entry(_repo_root: &Path, _entry: ToolSessionEntry) -> Result<(), String> { Ok(()) }
    pub fn get_last_tool_usage_map(_repo_root: &Path) -> HashMap<String, ToolSessionEntry> { HashMap::new() }
    pub fn get_branch_tool_history_for_worktree(_repo_root: &Path, _branch: &str, _limit: usize) -> Vec<ToolSessionEntry> { Vec::new() }
    pub fn codex_hooks_needs_update(_codex_root: &Path) -> bool { false }

    pub mod skill_registration {
        pub struct SkillManifest { pub name: String, pub skills: Vec<SkillEntry> }
        pub struct SkillEntry { pub name: String }
        pub fn load_manifests(_working_dir: &std::path::Path) -> Vec<SkillManifest> { Vec::new() }
        pub fn register_agent_skills_with_settings_at_project_root(_root: &std::path::Path) {}
        #[derive(Debug, Clone, Copy)]
        pub enum SkillAgentType { Claude, Codex, Custom }
    }
}

// ===========================================================================
// compat::terminal
// ===========================================================================

pub mod terminal {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum AgentColor { Green, Blue, Cyan, Yellow, Magenta, Gray, White }

    pub mod manager {
        use std::collections::HashMap;
        use std::path::PathBuf;
        use super::AgentColor;

        pub struct PaneManager { inner: gwt_terminal::PaneManager }

        impl PaneManager {
            pub fn new() -> Self { Self { inner: gwt_terminal::PaneManager::new(80, 24) } }

            pub fn panes(&self) -> Vec<PaneRef<'_>> {
                self.inner.list_panes().into_iter().filter_map(|id| {
                    self.inner.get_pane(id).map(|pane| PaneRef { id: id.to_string(), pane })
                }).collect()
            }

            pub fn panes_mut(&mut self) -> Vec<PaneMutRef<'_>> {
                let ids: Vec<String> = self.inner.list_panes().iter().map(|s| s.to_string()).collect();
                let mut result = Vec::new();
                for id in ids {
                    let ptr = &mut self.inner as *mut gwt_terminal::PaneManager;
                    unsafe {
                        if let Some(pane) = (*ptr).get_pane_mut(&id) {
                            result.push(PaneMutRef { id: id.clone(), pane });
                        }
                    }
                }
                result
            }

            pub fn pane_mut_by_id(&mut self, id: &str) -> Option<&mut gwt_terminal::Pane> {
                self.inner.get_pane_mut(id)
            }

            pub fn close_pane(&mut self, index: usize) -> Result<(), String> {
                let ids: Vec<String> = self.inner.list_panes().iter().map(|s| s.to_string()).collect();
                if let Some(id) = ids.get(index) {
                    self.inner.close_pane(id).map_err(|e| e.to_string())
                } else {
                    Err("pane index out of range".into())
                }
            }

            pub fn add_pane(&mut self, _pane: TerminalPane) -> Result<(), String> { Ok(()) }

            pub fn spawn_shell(&mut self, cwd: PathBuf, env: HashMap<String, String>) -> Result<String, String> {
                self.inner.spawn_shell(cwd, env).map_err(|e| e.to_string())
            }

            pub fn launch_agent(&mut self, config: gwt_terminal::manager::LaunchConfig) -> Result<String, String> {
                self.inner.launch_agent(config).map_err(|e| e.to_string())
            }

            pub fn resize_all(&mut self, cols: u16, rows: u16) -> Result<(), String> {
                self.inner.resize_all(cols, rows).map_err(|e| e.to_string())
            }

            pub fn get_pane(&self, id: &str) -> Option<&gwt_terminal::Pane> { self.inner.get_pane(id) }
        }

        impl std::fmt::Debug for PaneManager {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("PaneManager").finish()
            }
        }

        pub struct PaneRef<'a> { pub id: String, pub pane: &'a gwt_terminal::Pane }
        impl PaneRef<'_> {
            pub fn pane_id(&self) -> &str { &self.id }
            pub fn take_reader(&self) -> Result<Box<dyn std::io::Read + Send>, String> {
                self.pane.reader().map_err(|e| e.to_string())
            }
        }

        pub struct PaneMutRef<'a> { pub id: String, pub pane: &'a mut gwt_terminal::Pane }
        impl PaneMutRef<'_> {
            pub fn pane_id(&self) -> &str { &self.id }
            pub fn check_status(&mut self) -> Result<&gwt_terminal::PaneStatus, String> {
                self.pane.check_status().map_err(|e| e.to_string())
            }
            pub fn mark_error(&mut self, message: String) { self.pane.mark_error(message); }
        }

        #[derive(Debug, Clone)]
        pub struct PaneConfig {
            pub pane_id: String,
            pub command: String,
            pub args: Vec<String>,
            pub working_dir: PathBuf,
            pub branch_name: String,
            pub agent_name: String,
            pub agent_color: AgentColor,
            pub rows: u16,
            pub cols: u16,
            pub env_vars: HashMap<String, String>,
            pub terminal_shell: Option<String>,
            pub interactive: bool,
            pub windows_force_utf8: bool,
            pub project_root: PathBuf,
        }

        pub struct TerminalPane { pub pane_id: String, pub inner: gwt_terminal::Pane }

        impl TerminalPane {
            pub fn take_reader(&self) -> Result<Box<dyn std::io::Read + Send>, String> {
                self.inner.reader().map_err(|e| e.to_string())
            }

            pub fn new(config: PaneConfig) -> Result<Self, String> {
                let pane = gwt_terminal::Pane::new(
                    config.pane_id.clone(), config.command, config.args,
                    config.cols, config.rows, config.env_vars, Some(config.working_dir),
                ).map_err(|e| e.to_string())?;
                Ok(Self { pane_id: config.pane_id, inner: pane })
            }
        }
    }

    pub mod pane {
        pub use gwt_terminal::PaneStatus;
        pub use super::manager::{PaneConfig, TerminalPane};
    }
}

// ===========================================================================
// compat::agent
// ===========================================================================

pub mod agent {
    pub mod launch {
        use std::collections::HashMap;
        use std::path::PathBuf;
        use crate::compat::terminal::AgentColor;

        pub struct ShellLaunchBuilder {
            pub working_dir: PathBuf,
            pub env: HashMap<String, String>,
            pub branch: Option<String>,
        }

        impl ShellLaunchBuilder {
            pub fn new() -> Self {
                Self { working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")), env: HashMap::new(), branch: None }
            }
            pub fn working_dir(mut self, dir: PathBuf) -> Self { self.working_dir = dir; self }
            pub fn env(mut self, env: HashMap<String, String>) -> Self { self.env = env; self }
            pub fn branch(mut self, name: &str) -> Self { self.branch = Some(name.to_string()); self }
            pub fn build(self) -> ShellLaunchConfig {
                ShellLaunchConfig { working_dir: self.working_dir, env: self.env, branch: self.branch }
            }
        }

        pub struct ShellLaunchConfig {
            pub working_dir: PathBuf,
            pub env: HashMap<String, String>,
            pub branch: Option<String>,
        }

        pub struct AgentLaunchBuilder {
            pub agent_id: String,
            pub working_dir: PathBuf,
            pub env: HashMap<String, String>,
            pub branch: Option<String>,
            pub model_name: Option<String>,
            pub mode: SessionMode,
            pub skip_perms: bool,
            pub session_id: Option<String>,
            pub agent_ver: Option<String>,
            pub reasoning: Option<String>,
            pub branch_display: Option<String>,
        }

        impl AgentLaunchBuilder {
            pub fn new(agent_id: &str) -> Self {
                Self {
                    agent_id: agent_id.to_string(),
                    working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
                    env: HashMap::new(), branch: None, model_name: None, mode: SessionMode::Normal,
                    skip_perms: false, session_id: None, agent_ver: None, reasoning: None, branch_display: None,
                }
            }
            pub fn working_dir(mut self, dir: PathBuf) -> Self { self.working_dir = dir; self }
            pub fn env(mut self, env: HashMap<String, String>) -> Self { self.env = env; self }
            pub fn branch(mut self, name: &str) -> Self { self.branch = Some(name.to_string()); self }
            pub fn branch_name(mut self, name: &str) -> Self { self.branch_display = Some(name.to_string()); self }
            pub fn model(mut self, model: &str) -> Self { self.model_name = Some(model.to_string()); self }
            pub fn mode(mut self, mode: SessionMode) -> Self { self.mode = mode; self }
            pub fn skip_permissions(mut self, skip: bool) -> Self { self.skip_perms = skip; self }
            pub fn session_id(mut self, id: &str) -> Self { self.session_id = Some(id.to_string()); self }
            pub fn agent_version(mut self, ver: &str) -> Self { self.agent_ver = Some(ver.to_string()); self }
            pub fn reasoning_level(mut self, level: &str) -> Self { self.reasoning = Some(level.to_string()); self }
        }

        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub enum SessionMode { #[default] Normal, Continue, Resume }

        pub fn agent_color_for(_agent_id: &str) -> AgentColor { AgentColor::Gray }

        pub struct AgentDef { pub display_name: String }
        pub fn find_agent_def(_agent_id: &str) -> Option<AgentDef> { None }
    }
}

// ===========================================================================
// compat::ai
// ===========================================================================

pub mod ai {
    pub use gwt_ai::{AIClient, AIError, ChatMessage};

    pub fn format_error_for_display(err: &AIError) -> String { format!("{err}") }
    pub fn detect_session_id_for_tool(_tool_id: &str, _working_dir: &std::path::Path) -> Option<String> { None }
}

// ===========================================================================
// compat::logging
// ===========================================================================

pub mod logging {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Default)]
    pub struct LogConfig;

    pub fn init_logger(_config: &LogConfig) -> Result<(), String> { Ok(()) }

    pub struct LogReader;
    impl LogReader {
        pub fn new() -> Result<Self, String> { Ok(Self) }
        pub fn read_entries() -> Vec<serde_json::Value> { Vec::new() }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct LogEntry {
        pub timestamp: Option<String>,
        pub level: Option<String>,
        pub fields: Option<serde_json::Value>,
        pub target: Option<String>,
    }
}

// ===========================================================================
// compat::process
// ===========================================================================

pub mod process {
    pub fn command(cmd: &str) -> CommandCheck { CommandCheck(cmd.to_string()) }

    pub struct CommandCheck(String);
    impl CommandCheck {
        pub fn exists(&self) -> bool { which::which(&self.0).is_ok() }
        pub fn args(&self, _args: &[&str]) -> &Self { self }
    }
}

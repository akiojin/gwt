use std::{
    collections::{BTreeMap, VecDeque},
    fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};

use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

use crate::{knowledge_launch_target_branch_name, LaunchWizardPreviousProfile, LinkedIssueKind};

const GITHUB_AUTH_SETUP_MESSAGE: &str = concat!(
    "GitHub authentication is required before automatic Issue Monitor launches can claim Issues. ",
    "Configure it on the host terminal with: ",
    "gh auth login --hostname github.com --git-protocol https --scopes repo,read:org; ",
    "gh auth setup-git. ",
    "Then verify: gh auth status --hostname github.com; git ls-remote origin HEAD. ",
    "gwt does not store GitHub credentials; it uses the host gh/Git credential setup."
);

const GIT_HTTPS_AUTH_SETUP_PREFIX: &str = concat!(
    "Git HTTPS credentials are required before Issue Monitor can create work branches. ",
    "Configure the host terminal with: ",
    "gh auth login --hostname github.com --git-protocol https --scopes repo,read:org; ",
    "gh auth setup-git. ",
    "Then verify: git ls-remote origin HEAD."
);

pub fn github_auth_setup_message() -> &'static str {
    GITHUB_AUTH_SETUP_MESSAGE
}

pub fn is_git_https_auth_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("could not read username for 'https://github.com'")
        || lower.contains("could not read username for \"https://github.com")
        || (lower.contains("terminal prompts disabled") && lower.contains("github.com"))
}

pub fn git_https_auth_setup_message(original_error: &str) -> String {
    format!(
        "{GIT_HTTPS_AUTH_SETUP_PREFIX} Original error: {}",
        original_error.trim()
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorConfig {
    pub enabled: bool,
    pub trigger_label: String,
    pub poll_interval_secs: u64,
    pub claim_heartbeat_secs: u64,
    pub claim_ttl_secs: u64,
    pub max_active: usize,
    pub queue_when_gui_absent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorPrefs {
    pub enabled: bool,
    pub max_active_agents: usize,
    pub priority_order: Vec<u64>,
    #[serde(default)]
    pub launch_profile: Option<IssueMonitorLaunchProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launched_issues: Vec<IssueMonitorLaunchedIssue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_issues: Vec<IssueMonitorFailedIssue>,
}

impl Default for IssueMonitorPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            max_active_agents: 1,
            priority_order: Vec::new(),
            launch_profile: None,
            launched_issues: Vec::new(),
            failed_issues: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchedIssue {
    pub issue_number: u64,
    pub window_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorFailedIssue {
    pub issue_number: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchProfile {
    pub agent_id: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub session_mode: gwt_agent::SessionMode,
    #[serde(default)]
    pub skip_permissions: bool,
    #[serde(default)]
    pub codex_fast_mode: bool,
    #[serde(default)]
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    #[serde(default)]
    pub docker_service: Option<String>,
    #[serde(default)]
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    #[serde(default)]
    pub windows_shell: Option<gwt_agent::WindowsShellKind>,
}

impl From<&gwt_agent::LaunchConfig> for IssueMonitorLaunchProfile {
    fn from(config: &gwt_agent::LaunchConfig) -> Self {
        Self {
            agent_id: config.agent_id.command().to_string(),
            model: config.model.clone(),
            reasoning: config.reasoning_level.clone(),
            version: config.tool_version.clone(),
            session_mode: config.session_mode,
            skip_permissions: config.skip_permissions,
            codex_fast_mode: config.fast_mode || config.codex_fast_mode,
            runtime_target: config.runtime_target,
            docker_service: config.docker_service.clone(),
            docker_lifecycle_intent: config.docker_lifecycle_intent,
            windows_shell: config.windows_shell,
        }
    }
}

impl From<IssueMonitorLaunchProfile> for LaunchWizardPreviousProfile {
    fn from(profile: IssueMonitorLaunchProfile) -> Self {
        Self {
            agent_id: profile.agent_id,
            model: profile.model,
            reasoning: profile.reasoning,
            version: profile.version,
            session_mode: profile.session_mode,
            skip_permissions: profile.skip_permissions,
            codex_fast_mode: profile.codex_fast_mode,
            runtime_target: profile.runtime_target,
            docker_service: profile.docker_service,
            docker_lifecycle_intent: profile.docker_lifecycle_intent,
            windows_shell: profile.windows_shell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorLaunchProfileSource {
    Saved,
    LastSettings,
    Default,
}

impl IssueMonitorLaunchProfileSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "Saved",
            Self::LastSettings => "Last settings",
            Self::Default => "Default",
        }
    }
}

pub fn issue_monitor_launch_profile_summary(profile: &LaunchWizardPreviousProfile) -> String {
    let model = profile.model.as_deref().unwrap_or("default");
    let reasoning = profile.reasoning.as_deref().unwrap_or("auto");
    format!(
        "{} / {} / {} / {}",
        profile.agent_id,
        model,
        reasoning,
        issue_monitor_runtime_label(profile.runtime_target)
    )
}

fn issue_monitor_runtime_label(target: gwt_agent::LaunchRuntimeTarget) -> &'static str {
    match target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    }
}

impl Default for IssueMonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger_label: "auto-improve".to_string(),
            poll_interval_secs: 300,
            claim_heartbeat_secs: 300,
            claim_ttl_secs: 1800,
            max_active: 1,
            queue_when_gui_absent: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueMonitorIssueState {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorIssue {
    pub number: u64,
    pub title: String,
    pub labels: Vec<String>,
    pub state: IssueMonitorIssueState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorInboxState {
    Queued,
    Launching,
    Launched,
    LaunchFailed,
    AgentFailed,
    BlockedByClaim,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorInboxItem {
    pub issue: IssueMonitorIssue,
    pub state: MonitorInboxState,
    pub claim_id: Option<String>,
    pub blocked_by_owner: Option<String>,
    pub claim_expires_at: Option<String>,
    pub launched_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_plan: Option<IssueMonitorLaunchPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchRequest {
    pub issue_number: u64,
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchPlan {
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorStatusView {
    pub enabled: bool,
    pub state: String,
    pub queue_len: usize,
    pub active_count: usize,
    pub max_active_agents: usize,
    pub total_candidates: usize,
    pub active_issue_number: Option<u64>,
    pub last_scan_at: Option<String>,
    pub last_error: Option<String>,
    pub launch_profile_source: IssueMonitorLaunchProfileSource,
    pub launch_profile_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorState {
    pub config: IssueMonitorConfig,
    pub gui_connected: bool,
    pub inbox: Vec<IssueMonitorInboxItem>,
    last_scan_at: Option<String>,
    last_error: Option<String>,
    launch_auth_required: bool,
    active_launches: Vec<u64>,
    priority_order: Vec<u64>,
    launch_profile: Option<IssueMonitorLaunchProfile>,
    launched_windows: BTreeMap<u64, String>,
    failed_issues: BTreeMap<u64, String>,
    queue: VecDeque<u64>,
    pending_launches: VecDeque<IssueMonitorLaunchRequest>,
}

pub fn is_auto_improve_candidate(issue: &IssueMonitorIssue, config: &IssueMonitorConfig) -> bool {
    let _ = config;
    issue.state == IssueMonitorIssueState::Open
}

pub fn issue_monitor_linked_issue_kind(issue: &IssueMonitorIssue) -> LinkedIssueKind {
    if issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("gwt-spec"))
    {
        LinkedIssueKind::Spec
    } else {
        LinkedIssueKind::Issue
    }
}

pub fn issue_monitor_launch_prompt(kind: LinkedIssueKind, number: u64) -> String {
    match kind {
        LinkedIssueKind::Spec => format!("$gwt-build-spec SPEC-{number}"),
        LinkedIssueKind::Issue => format!("$gwt-fix-issue #{number}"),
    }
}

pub fn issue_monitor_launch_plan(issue: &IssueMonitorIssue) -> IssueMonitorLaunchPlan {
    let linked_issue_kind = issue_monitor_linked_issue_kind(issue);
    IssueMonitorLaunchPlan {
        branch_name: knowledge_launch_target_branch_name(linked_issue_kind, issue.number),
        linked_issue_kind,
        prompt: issue_monitor_launch_prompt(linked_issue_kind, issue.number),
    }
}

pub fn load_issue_monitor_prefs(path: &Path) -> io::Result<IssueMonitorPrefs> {
    if !path.exists() {
        return Ok(IssueMonitorPrefs::default());
    }
    let content = fs::read_to_string(path)?;
    serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub fn save_issue_monitor_prefs(path: &Path, prefs: &IssueMonitorPrefs) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content.as_bytes())?;
    fs::rename(tmp, path)
}

pub fn issue_monitor_prefs_path_for_repo_path(repo_path: &Path) -> std::path::PathBuf {
    gwt_core::paths::gwt_project_dir_for_repo_path(repo_path)
        .join("project-state/issue-monitor.json")
}

impl IssueMonitorState {
    pub fn new(config: IssueMonitorConfig) -> Self {
        Self {
            config,
            gui_connected: false,
            inbox: Vec::new(),
            last_scan_at: None,
            last_error: None,
            launch_auth_required: false,
            active_launches: Vec::new(),
            priority_order: Vec::new(),
            launch_profile: None,
            launched_windows: BTreeMap::new(),
            failed_issues: BTreeMap::new(),
            queue: VecDeque::new(),
            pending_launches: VecDeque::new(),
        }
    }

    pub fn with_prefs(mut config: IssueMonitorConfig, prefs: IssueMonitorPrefs) -> Self {
        config.enabled = prefs.enabled;
        config.max_active = prefs.max_active_agents.max(1);
        let mut state = Self::new(config);
        state.priority_order = prefs.priority_order;
        state.launch_profile = prefs.launch_profile;
        for launched in prefs.launched_issues {
            if launched.window_id.is_empty() {
                continue;
            }
            state
                .launched_windows
                .insert(launched.issue_number, launched.window_id);
            if !state.active_launches.contains(&launched.issue_number) {
                state.active_launches.push(launched.issue_number);
            }
        }
        for failed in prefs.failed_issues {
            if failed.message.trim().is_empty() {
                continue;
            }
            state
                .failed_issues
                .insert(failed.issue_number, failed.message);
        }
        state
    }

    pub fn prefs(&self) -> IssueMonitorPrefs {
        IssueMonitorPrefs {
            enabled: self.config.enabled,
            max_active_agents: self.config.max_active.max(1),
            priority_order: self.priority_order.clone(),
            launch_profile: self.launch_profile.clone(),
            launched_issues: self
                .launched_windows
                .iter()
                .map(|(issue_number, window_id)| IssueMonitorLaunchedIssue {
                    issue_number: *issue_number,
                    window_id: window_id.clone(),
                })
                .collect(),
            failed_issues: self
                .failed_issues
                .iter()
                .map(|(issue_number, message)| IssueMonitorFailedIssue {
                    issue_number: *issue_number,
                    message: message.clone(),
                })
                .collect(),
        }
    }

    pub fn set_gui_connected(&mut self, connected: bool) {
        self.gui_connected = connected;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        self.launch_auth_required = false;
        if !enabled {
            self.active_launches.clear();
            self.pending_launches.clear();
        }
    }

    pub fn set_max_active_agents(&mut self, max_active_agents: usize) {
        self.config.max_active = max_active_agents.max(1);
    }

    pub fn record_scan_error(&mut self, now: impl Into<String>, error: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(error.into());
        self.launch_auth_required = false;
    }

    pub fn record_launch_auth_required(&mut self, now: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(github_auth_setup_message().to_string());
        self.launch_auth_required = true;
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn active_issue_number(&self) -> Option<u64> {
        self.active_launches.first().copied()
    }

    pub fn active_count(&self) -> usize {
        self.active_launches.len()
    }

    pub fn has_launch_profile(&self) -> bool {
        self.launch_profile.is_some()
    }

    pub fn status_view(&self) -> IssueMonitorStatusView {
        let last_error = self.last_error.clone().or_else(|| {
            self.failed_issues
                .iter()
                .next()
                .map(|(issue_number, message)| format!("issue #{issue_number}: {message}"))
        });
        IssueMonitorStatusView {
            enabled: self.config.enabled,
            state: if !self.config.enabled {
                "disabled".to_string()
            } else if last_error.is_some() {
                "error".to_string()
            } else if !self.active_launches.is_empty() {
                if self
                    .active_launches
                    .iter()
                    .all(|issue_number| self.launched_windows.contains_key(issue_number))
                {
                    "active".to_string()
                } else {
                    "launching".to_string()
                }
            } else if self.launch_auth_required {
                "auth_required".to_string()
            } else if self.launch_profile.is_none()
                && !self.queue.is_empty()
                && self.active_launches.is_empty()
            {
                "settings_required".to_string()
            } else {
                "idle".to_string()
            },
            queue_len: self.queue.len(),
            active_count: self.active_launches.len(),
            max_active_agents: self.config.max_active,
            total_candidates: self.inbox.len(),
            active_issue_number: self.active_issue_number(),
            last_scan_at: self.last_scan_at.clone(),
            last_error,
            launch_profile_source: self
                .launch_profile
                .as_ref()
                .map(|_| IssueMonitorLaunchProfileSource::Saved)
                .unwrap_or(IssueMonitorLaunchProfileSource::Default),
            launch_profile_summary: self
                .launch_profile
                .clone()
                .map(LaunchWizardPreviousProfile::from)
                .as_ref()
                .map(issue_monitor_launch_profile_summary)
                .unwrap_or_else(|| "configure to override".to_string()),
        }
    }

    pub fn inbox_item(&self, issue_number: u64) -> Option<&IssueMonitorInboxItem> {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
    }

    pub fn record_claimed(&mut self, issue: IssueMonitorIssue, claim_id: impl Into<String>) {
        let issue_number = issue.number;
        let error_message = self.failed_issues.get(&issue_number).cloned();
        let launched_window_id = if error_message.is_some() {
            None
        } else {
            self.launched_windows.get(&issue_number).cloned()
        };
        let state = if error_message.is_some() {
            MonitorInboxState::AgentFailed
        } else if launched_window_id.is_some() {
            MonitorInboxState::Launched
        } else if self.active_launches.contains(&issue_number) {
            MonitorInboxState::Launching
        } else {
            MonitorInboxState::Queued
        };
        let item = IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state,
            claim_id: Some(claim_id.into()),
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id,
            error_message,
        };
        self.upsert_inbox(item);
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
        }
        self.apply_priority_order_to_inbox();
    }

    pub fn record_candidate(&mut self, issue: IssueMonitorIssue) {
        let issue_number = issue.number;
        let existing = self.inbox_item(issue_number).cloned();
        let error_message = self.failed_issues.get(&issue_number).cloned().or_else(|| {
            existing.as_ref().and_then(|item| {
                if matches!(
                    item.state,
                    MonitorInboxState::AgentFailed | MonitorInboxState::LaunchFailed
                ) {
                    item.error_message.clone()
                } else {
                    None
                }
            })
        });
        let launched_window_id = if error_message.is_some() {
            None
        } else {
            self.launched_windows.get(&issue_number).cloned()
        };
        let state = if error_message.is_some() {
            existing
                .as_ref()
                .filter(|item| item.state == MonitorInboxState::LaunchFailed)
                .map(|item| item.state)
                .unwrap_or(MonitorInboxState::AgentFailed)
        } else if launched_window_id.is_some() {
            MonitorInboxState::Launched
        } else {
            existing
                .as_ref()
                .map(|item| item.state)
                .unwrap_or(MonitorInboxState::Queued)
        };
        let item = IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state,
            claim_id: existing.as_ref().and_then(|item| item.claim_id.clone()),
            blocked_by_owner: existing
                .as_ref()
                .and_then(|item| item.blocked_by_owner.clone()),
            claim_expires_at: existing
                .as_ref()
                .and_then(|item| item.claim_expires_at.clone()),
            launched_window_id: launched_window_id.or_else(|| {
                existing
                    .as_ref()
                    .and_then(|item| item.launched_window_id.clone())
            }),
            error_message,
        };
        self.upsert_inbox(item);
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        self.apply_priority_order_to_inbox();
    }

    pub fn record_blocked_by_claim(
        &mut self,
        issue: IssueMonitorIssue,
        owner: impl Into<String>,
        expires_at: impl Into<String>,
    ) {
        self.queue.retain(|queued| *queued != issue.number);
        self.upsert_inbox(IssueMonitorInboxItem {
            launch_plan: Some(issue_monitor_launch_plan(&issue)),
            issue,
            state: MonitorInboxState::BlockedByClaim,
            claim_id: None,
            blocked_by_owner: Some(owner.into()),
            claim_expires_at: Some(expires_at.into()),
            launched_window_id: None,
            error_message: None,
        });
        self.apply_priority_order_to_inbox();
    }

    pub fn reorder_queued_issues(&mut self, issue_numbers: &[u64]) {
        self.priority_order = issue_numbers.to_vec();
        self.apply_priority_order_to_queue();
        self.apply_priority_order_to_inbox();
    }

    pub fn set_priority_order(&mut self, issue_numbers: Vec<u64>) {
        self.priority_order = issue_numbers;
        self.apply_priority_order_to_queue();
        self.apply_priority_order_to_inbox();
    }

    fn apply_priority_order_to_queue(&mut self) {
        let mut remaining: Vec<u64> = self.queue.iter().copied().collect();
        let mut reordered = VecDeque::new();
        for number in &self.priority_order {
            if self.active_launches.contains(number) {
                continue;
            }
            if let Some(index) = remaining.iter().position(|queued| queued == number) {
                reordered.push_back(*number);
                remaining.remove(index);
            }
        }
        for number in remaining {
            reordered.push_back(number);
        }
        self.queue = reordered;
    }

    fn apply_priority_order_to_inbox(&mut self) {
        if self.priority_order.is_empty() || self.inbox.len() < 2 {
            return;
        }
        let order = self.priority_order.clone();
        self.inbox.sort_by(|left, right| {
            let left_index = order.iter().position(|number| *number == left.issue.number);
            let right_index = order
                .iter()
                .position(|number| *number == right.issue.number);
            match (left_index, right_index) {
                (Some(left_index), Some(right_index)) => left_index.cmp(&right_index),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    pub fn next_launch_request(&mut self) -> Option<IssueMonitorLaunchRequest> {
        let max_active = self.config.max_active.max(1);
        if !self.gui_connected || self.active_launches.len() >= max_active {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        let linked_issue_kind = if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launching;
            issue_monitor_linked_issue_kind(&item.issue)
        } else {
            LinkedIssueKind::Issue
        };
        Some(IssueMonitorLaunchRequest {
            issue_number,
            branch_name: knowledge_launch_target_branch_name(linked_issue_kind, issue_number),
            linked_issue_kind,
        })
    }

    pub fn claim_next_launch_requests<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
    ) -> Vec<IssueMonitorLaunchRequest> {
        self.claim_next_launch_requests_with_active_cap(
            client,
            owner,
            now,
            self.config.max_active.max(1),
        )
    }

    pub fn claim_next_launch_requests_with_active_cap<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
        active_cap: usize,
    ) -> Vec<IssueMonitorLaunchRequest> {
        let mut launches = Vec::new();
        let max_active = self.config.max_active.max(1).min(active_cap);
        if max_active == 0 {
            return launches;
        }
        while self.config.enabled && self.gui_connected && self.active_launches.len() < max_active {
            let Some(issue_number) = self.queue.front().copied() else {
                break;
            };
            let Some(issue) = self.inbox_item(issue_number).map(|item| item.issue.clone()) else {
                self.queue.pop_front();
                continue;
            };
            let kind = issue_monitor_linked_issue_kind(&issue);
            let branch_name = knowledge_launch_target_branch_name(kind, issue.number);
            let claim = ClaimComment {
                comment_id: None,
                claim_id: format!("gwt-auto-improve:{owner}:{}:{now}", issue.number),
                owner: owner.to_string(),
                issue_number: issue.number,
                status: ClaimStatus::Active,
                heartbeat_at: now.to_string(),
                expires_at: expiry_from_now_lexical(now, self.config.claim_ttl_secs),
                launched_work_id: Some(branch_name),
            };

            match acquire_claim(client, IssueNumber(issue.number), claim, now) {
                Ok(ClaimAcquireOutcome::Acquired(claim)) => {
                    self.record_claimed(issue, claim.claim_id);
                    if let Some(request) = self.next_launch_request() {
                        self.pending_launches.push_back(request.clone());
                        launches.push(request);
                    }
                }
                Ok(ClaimAcquireOutcome::Blocked(claim)) => {
                    self.record_blocked_by_claim(issue, claim.owner, claim.expires_at);
                }
                Ok(ClaimAcquireOutcome::Lost { winning_claim, .. }) => {
                    self.record_blocked_by_claim(
                        issue,
                        winning_claim.owner,
                        winning_claim.expires_at,
                    );
                }
                Err(error) => {
                    self.last_error = Some(format!("issue #{}: {error}", issue.number));
                    break;
                }
            }
        }
        launches
    }

    pub fn take_pending_launch_requests(&mut self) -> Vec<IssueMonitorLaunchRequest> {
        self.pending_launches.drain(..).collect()
    }

    pub fn complete_active_launch(&mut self, issue_number: u64, window_id: impl Into<String>) {
        let window_id = window_id.into();
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        self.launched_windows
            .insert(issue_number, window_id.clone());
        self.failed_issues.remove(&issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launched;
            item.launched_window_id = Some(window_id);
            item.error_message = None;
        }
    }

    pub fn record_launch_failed(&mut self, issue_number: u64, message: impl Into<String>) {
        self.record_failed_issue(issue_number, message, MonitorInboxState::LaunchFailed);
    }

    pub fn record_agent_window_failed(
        &mut self,
        window_id: &str,
        message: impl Into<String>,
    ) -> Option<u64> {
        let issue_number = self
            .launched_windows
            .iter()
            .find_map(|(issue_number, launched_window_id)| {
                issue_monitor_window_ids_match(launched_window_id, window_id)
                    .then_some(*issue_number)
            })
            .or_else(|| {
                self.inbox.iter().find_map(|item| {
                    item.launched_window_id
                        .as_deref()
                        .filter(|launched_window_id| {
                            issue_monitor_window_ids_match(launched_window_id, window_id)
                        })
                        .map(|_| item.issue.number)
                })
            })?;
        self.record_failed_issue(issue_number, message, MonitorInboxState::AgentFailed);
        Some(issue_number)
    }

    pub fn record_agent_issue_failed(&mut self, issue_number: u64, message: impl Into<String>) {
        self.record_failed_issue(issue_number, message, MonitorInboxState::AgentFailed);
    }

    fn record_failed_issue(
        &mut self,
        issue_number: u64,
        message: impl Into<String>,
        state: MonitorInboxState,
    ) {
        let message = message.into();
        self.active_launches
            .retain(|active| *active != issue_number);
        self.launched_windows.remove(&issue_number);
        self.failed_issues.insert(issue_number, message.clone());
        self.queue.retain(|queued| *queued != issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
        self.launch_auth_required = false;
        self.last_error = Some(format!("issue #{issue_number}: {message}"));
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = state;
            item.launched_window_id = None;
            item.error_message = Some(message);
        }
    }

    fn upsert_inbox(&mut self, item: IssueMonitorInboxItem) {
        if let Some(existing) = self
            .inbox
            .iter_mut()
            .find(|existing| existing.issue.number == item.issue.number)
        {
            *existing = item;
        } else {
            self.inbox.push(item);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorScanSummary {
    pub scanned: usize,
    pub claimed: usize,
    pub blocked: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

pub fn scan_issue_monitor_candidates(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    now: &str,
) -> IssueMonitorScanSummary {
    let mut summary = IssueMonitorScanSummary::default();
    monitor.last_scan_at = Some(now.to_string());
    monitor.last_error = None;
    monitor.launch_auth_required = false;

    for issue in issues {
        summary.scanned += 1;
        if !is_auto_improve_candidate(issue, &monitor.config) {
            summary.skipped += 1;
            continue;
        }

        monitor.record_candidate(issue.clone());
    }

    summary
}

fn expiry_from_now_lexical(now: &str, ttl_secs: u64) -> String {
    chrono::DateTime::parse_from_rfc3339(now)
        .map(|time| {
            (time + chrono::Duration::seconds(ttl_secs as i64))
                .to_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| now.to_string())
}

fn issue_monitor_window_ids_match(stored: &str, incoming: &str) -> bool {
    if stored == incoming {
        return true;
    }
    let stored_raw = stored.rsplit("::").next().unwrap_or(stored);
    let incoming_raw = incoming.rsplit("::").next().unwrap_or(incoming);
    !stored_raw.is_empty() && stored_raw == incoming_raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: Vec::new(),
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
        }
    }

    #[test]
    fn scan_keeps_queue_visible_when_processing_is_stopped() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: false,
            ..IssueMonitorConfig::default()
        });

        let summary = scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-06-23T10:00:00Z",
        );

        assert_eq!(summary.scanned, 2);
        assert_eq!(monitor.queue_len(), 2);
        assert_eq!(monitor.status_view().state, "disabled");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
    }

    #[test]
    fn launched_issue_from_prefs_stays_active_and_is_not_requeued() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 1,
                launched_issues: vec![IssueMonitorLaunchedIssue {
                    issue_number: 42,
                    window_id: "tab-1::agent-1".to_string(),
                }],
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);

        let summary = scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-06-23T10:00:00Z",
        );

        assert_eq!(summary.scanned, 2);
        assert_eq!(monitor.status_view().state, "active");
        assert_eq!(monitor.active_count(), 1);
        assert_eq!(monitor.queue_len(), 1);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launched)
        );
        assert_eq!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.launched_window_id.as_deref()),
            Some("tab-1::agent-1")
        );
        assert!(
            monitor.next_launch_request().is_none(),
            "max_active=1 must keep the next queued issue waiting while launched work is active"
        );
    }
}

use std::{collections::VecDeque, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{knowledge_launch_target_branch_name, LinkedIssueKind};
use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorConfig {
    pub enabled: bool,
    #[serde(default)]
    pub trigger_label: String,
    pub poll_interval_secs: u64,
    pub claim_heartbeat_secs: u64,
    pub claim_ttl_secs: u64,
    pub max_active_agents: usize,
    #[serde(default)]
    pub priority_order: Vec<u64>,
    pub queue_when_gui_absent: bool,
}

impl Default for IssueMonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger_label: "auto-improve".to_string(),
            poll_interval_secs: 300,
            claim_heartbeat_secs: 300,
            claim_ttl_secs: 1800,
            max_active_agents: 1,
            priority_order: Vec::new(),
            queue_when_gui_absent: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorProjectSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub priority_order: Vec<u64>,
    #[serde(default = "default_max_active_agents")]
    pub max_active_agents: usize,
}

impl Default for IssueMonitorProjectSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            priority_order: Vec::new(),
            max_active_agents: default_max_active_agents(),
        }
    }
}

fn default_max_active_agents() -> usize {
    1
}

pub fn load_issue_monitor_settings_from_path(
    path: &Path,
) -> Result<IssueMonitorProjectSettings, String> {
    if !path.exists() {
        return Ok(IssueMonitorProjectSettings::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read issue monitor settings {}: {error}", path.display()))?;
    serde_json::from_str(&raw)
        .map(|mut settings: IssueMonitorProjectSettings| {
            settings.max_active_agents = settings.max_active_agents.max(1);
            settings
        })
        .map_err(|error| format!("parse issue monitor settings {}: {error}", path.display()))
}

pub fn save_issue_monitor_settings_to_path(
    path: &Path,
    settings: &IssueMonitorProjectSettings,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create issue monitor settings directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let normalized = IssueMonitorProjectSettings {
        enabled: settings.enabled,
        priority_order: settings.priority_order.clone(),
        max_active_agents: settings.max_active_agents.max(1),
    };
    let raw = serde_json::to_string_pretty(&normalized)
        .map_err(|error| format!("serialize issue monitor settings: {error}"))?;
    fs::write(path, raw)
        .map_err(|error| format!("write issue monitor settings {}: {error}", path.display()))
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
}

impl IssueMonitorIssue {
    pub fn linked_issue_kind(&self) -> LinkedIssueKind {
        if self.labels.iter().any(|label| label == "gwt-spec") {
            LinkedIssueKind::Spec
        } else {
            LinkedIssueKind::Issue
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorInboxState {
    Queued,
    Launching,
    Launched,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchRequest {
    pub issue_number: u64,
    pub branch_name: String,
    pub linked_issue_kind: LinkedIssueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorStatusView {
    pub enabled: bool,
    pub state: String,
    pub queue_len: usize,
    pub active_issue_number: Option<u64>,
    #[serde(default)]
    pub active_issue_numbers: Vec<u64>,
    #[serde(default = "default_max_active_agents")]
    pub max_active_agents: usize,
    pub last_scan_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorState {
    pub config: IssueMonitorConfig,
    pub gui_connected: bool,
    pub inbox: Vec<IssueMonitorInboxItem>,
    last_scan_at: Option<String>,
    last_error: Option<String>,
    active_launches: Vec<u64>,
    queue: VecDeque<u64>,
}

pub fn is_issue_monitor_candidate(issue: &IssueMonitorIssue, _config: &IssueMonitorConfig) -> bool {
    issue.state == IssueMonitorIssueState::Open
}

pub fn is_auto_improve_candidate(issue: &IssueMonitorIssue, config: &IssueMonitorConfig) -> bool {
    is_issue_monitor_candidate(issue, config)
}

impl IssueMonitorState {
    pub fn new(mut config: IssueMonitorConfig) -> Self {
        config.max_active_agents = config.max_active_agents.max(1);
        Self {
            config,
            gui_connected: false,
            inbox: Vec::new(),
            last_scan_at: None,
            last_error: None,
            active_launches: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    pub fn set_gui_connected(&mut self, connected: bool) {
        self.gui_connected = connected;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        if !enabled {
            self.active_launches.clear();
        }
    }

    pub fn set_max_active_agents(&mut self, max_active_agents: usize) {
        self.config.max_active_agents = max_active_agents.max(1);
    }

    pub fn set_priority_order(&mut self, priority_order: Vec<u64>) {
        self.config.priority_order = priority_order;
        self.sort_queue_by_priority();
    }

    pub fn project_settings(&self) -> IssueMonitorProjectSettings {
        IssueMonitorProjectSettings {
            enabled: self.config.enabled,
            priority_order: self.config.priority_order.clone(),
            max_active_agents: self.config.max_active_agents,
        }
    }

    pub fn record_scan_error(&mut self, now: impl Into<String>, error: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(error.into());
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn queued_issue_numbers(&self) -> Vec<u64> {
        self.queue.iter().copied().collect()
    }

    pub fn active_issue_number(&self) -> Option<u64> {
        self.active_launches.first().copied()
    }

    pub fn active_issue_numbers(&self) -> Vec<u64> {
        self.active_launches.clone()
    }

    pub fn status_view(&self) -> IssueMonitorStatusView {
        IssueMonitorStatusView {
            enabled: self.config.enabled,
            state: if !self.active_launches.is_empty() {
                "launching".to_string()
            } else if !self.config.enabled {
                "disabled".to_string()
            } else {
                "idle".to_string()
            },
            queue_len: self.queue.len(),
            active_issue_number: self.active_issue_number(),
            active_issue_numbers: self.active_issue_numbers(),
            max_active_agents: self.config.max_active_agents,
            last_scan_at: self.last_scan_at.clone(),
            last_error: self.last_error.clone(),
        }
    }

    pub fn inbox_item(&self, issue_number: u64) -> Option<&IssueMonitorInboxItem> {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
    }

    pub fn record_candidate(&mut self, issue: IssueMonitorIssue) {
        let issue_number = issue.number;
        let existing_state = self.inbox_item(issue_number).map(|item| item.state);
        let state = match existing_state {
            Some(MonitorInboxState::Launching) | Some(MonitorInboxState::Launched) => {
                existing_state.expect("checked above")
            }
            _ => MonitorInboxState::Queued,
        };
        let item = IssueMonitorInboxItem {
            issue,
            state,
            claim_id: None,
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id: None,
        };
        self.upsert_inbox(item);
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
            self.sort_queue_by_priority();
        }
    }

    pub fn record_claimed(&mut self, issue: IssueMonitorIssue, claim_id: impl Into<String>) {
        let issue_number = issue.number;
        let state = if self.active_launches.contains(&issue_number) {
            MonitorInboxState::Launching
        } else {
            MonitorInboxState::Queued
        };
        let item = IssueMonitorInboxItem {
            issue,
            state,
            claim_id: Some(claim_id.into()),
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id: None,
        };
        self.upsert_inbox(item);
        if state == MonitorInboxState::Queued
            && !self.queue.contains(&issue_number)
            && !self.active_launches.contains(&issue_number)
        {
            self.queue.push_back(issue_number);
            self.sort_queue_by_priority();
        }
    }

    pub fn record_blocked_by_claim(
        &mut self,
        issue: IssueMonitorIssue,
        owner: impl Into<String>,
        expires_at: impl Into<String>,
    ) {
        self.queue.retain(|queued| *queued != issue.number);
        self.upsert_inbox(IssueMonitorInboxItem {
            issue,
            state: MonitorInboxState::BlockedByClaim,
            claim_id: None,
            blocked_by_owner: Some(owner.into()),
            claim_expires_at: Some(expires_at.into()),
            launched_window_id: None,
        });
    }

    pub fn next_launch_request(&mut self) -> Option<IssueMonitorLaunchRequest> {
        if !self.can_start_more_launches() {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        self.mark_launching(issue_number, None)
    }

    pub fn complete_active_launch(&mut self, issue_number: u64, window_id: impl Into<String>) {
        self.active_launches
            .retain(|active| *active != issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launched;
            item.launched_window_id = Some(window_id.into());
        }
    }

    fn can_start_more_launches(&self) -> bool {
        self.config.enabled
            && self.gui_connected
            && self.active_launches.len() < self.config.max_active_agents.max(1)
    }

    fn pop_next_launch_issue(&mut self) -> Option<IssueMonitorIssue> {
        if !self.can_start_more_launches() {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        self.inbox_item(issue_number).map(|item| item.issue.clone())
    }

    fn mark_launching(
        &mut self,
        issue_number: u64,
        claim_id: Option<String>,
    ) -> Option<IssueMonitorLaunchRequest> {
        let item = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)?;
        let linked_issue_kind = item.issue.linked_issue_kind();
        item.state = MonitorInboxState::Launching;
        item.claim_id = claim_id;
        item.blocked_by_owner = None;
        item.claim_expires_at = None;
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        Some(IssueMonitorLaunchRequest {
            issue_number,
            branch_name: knowledge_launch_target_branch_name(linked_issue_kind, issue_number),
            linked_issue_kind,
        })
    }

    fn record_launch_error(&mut self, issue: IssueMonitorIssue, message: String) {
        self.last_error = Some(message);
        self.queue.retain(|queued| *queued != issue.number);
        self.upsert_inbox(IssueMonitorInboxItem {
            issue,
            state: MonitorInboxState::Skipped,
            claim_id: None,
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id: None,
        });
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

    fn sort_queue_by_priority(&mut self) {
        let order = &self.config.priority_order;
        let mut deduped = Vec::new();
        for issue_number in self.queue.drain(..) {
            if !deduped.contains(&issue_number) {
                deduped.push(issue_number);
            }
        }
        let original_positions = deduped.clone();
        deduped.sort_by_key(|issue_number| {
            let priority = order
                .iter()
                .position(|ordered| ordered == issue_number)
                .unwrap_or(usize::MAX);
            let original = original_positions
                .iter()
                .position(|original| original == issue_number)
                .unwrap_or(usize::MAX);
            (priority, original)
        });
        self.queue = deduped.into_iter().collect();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorScanSummary {
    pub scanned: usize,
    pub queued: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchBatch {
    pub requests: Vec<IssueMonitorLaunchRequest>,
    pub claimed: usize,
    pub blocked: usize,
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

    if !monitor.config.enabled {
        summary.skipped = issues.len();
        return summary;
    }

    for issue in issues {
        summary.scanned += 1;
        if !is_issue_monitor_candidate(issue, &monitor.config) {
            summary.skipped += 1;
            monitor.queue.retain(|queued| *queued != issue.number);
            continue;
        }
        summary.queued += 1;
        monitor.record_candidate(issue.clone());
    }

    summary
}

pub fn prepare_issue_monitor_launches<C: IssueClient>(
    monitor: &mut IssueMonitorState,
    client: &C,
    owner: &str,
    now: &str,
) -> IssueMonitorLaunchBatch {
    let mut batch = IssueMonitorLaunchBatch::default();
    while monitor.can_start_more_launches() {
        let Some(issue) = monitor.pop_next_launch_issue() else {
            break;
        };
        let linked_issue_kind = issue.linked_issue_kind();
        let branch_name = knowledge_launch_target_branch_name(linked_issue_kind, issue.number);
        let claim = ClaimComment {
            comment_id: None,
            claim_id: format!("gwt-auto-improve:{owner}:{}:{now}", issue.number),
            owner: owner.to_string(),
            issue_number: issue.number,
            status: ClaimStatus::Active,
            heartbeat_at: now.to_string(),
            expires_at: expiry_from_now_lexical(now, monitor.config.claim_ttl_secs),
            launched_work_id: Some(branch_name),
        };

        match acquire_claim(client, IssueNumber(issue.number), claim, now) {
            Ok(ClaimAcquireOutcome::Acquired(claim)) => {
                if let Some(request) = monitor.mark_launching(issue.number, Some(claim.claim_id)) {
                    batch.claimed += 1;
                    batch.requests.push(request);
                }
            }
            Ok(ClaimAcquireOutcome::Blocked(claim)) => {
                batch.blocked += 1;
                monitor.record_blocked_by_claim(issue, claim.owner, claim.expires_at);
            }
            Ok(ClaimAcquireOutcome::Lost { winning_claim, .. }) => {
                batch.blocked += 1;
                monitor.record_blocked_by_claim(
                    issue,
                    winning_claim.owner,
                    winning_claim.expires_at,
                );
            }
            Err(error) => {
                let message = format!("issue #{}: {error}", issue.number);
                batch.errors.push(message.clone());
                monitor.record_launch_error(issue, message);
            }
        }
    }
    batch
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

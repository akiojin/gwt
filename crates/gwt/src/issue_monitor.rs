use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorStatusView {
    pub enabled: bool,
    pub state: String,
    pub queue_len: usize,
    pub active_issue_number: Option<u64>,
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
    active_launch: Option<u64>,
    queue: VecDeque<u64>,
}

pub fn is_auto_improve_candidate(issue: &IssueMonitorIssue, config: &IssueMonitorConfig) -> bool {
    issue.state == IssueMonitorIssueState::Open
        && issue
            .labels
            .iter()
            .any(|label| label == &config.trigger_label)
}

impl IssueMonitorState {
    pub fn new(config: IssueMonitorConfig) -> Self {
        Self {
            config,
            gui_connected: false,
            inbox: Vec::new(),
            last_scan_at: None,
            last_error: None,
            active_launch: None,
            queue: VecDeque::new(),
        }
    }

    pub fn set_gui_connected(&mut self, connected: bool) {
        self.gui_connected = connected;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
        if !enabled {
            self.active_launch = None;
        }
    }

    pub fn record_scan_error(&mut self, now: impl Into<String>, error: impl Into<String>) {
        self.last_scan_at = Some(now.into());
        self.last_error = Some(error.into());
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub fn active_issue_number(&self) -> Option<u64> {
        self.active_launch
    }

    pub fn status_view(&self) -> IssueMonitorStatusView {
        IssueMonitorStatusView {
            enabled: self.config.enabled,
            state: if self.active_launch.is_some() {
                "launching".to_string()
            } else if !self.config.enabled {
                "disabled".to_string()
            } else {
                "idle".to_string()
            },
            queue_len: self.queue.len(),
            active_issue_number: self.active_launch,
            last_scan_at: self.last_scan_at.clone(),
            last_error: self.last_error.clone(),
        }
    }

    pub fn inbox_item(&self, issue_number: u64) -> Option<&IssueMonitorInboxItem> {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
    }

    pub fn record_claimed(&mut self, issue: IssueMonitorIssue, claim_id: impl Into<String>) {
        let issue_number = issue.number;
        let state = if self.active_launch == Some(issue_number) {
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
        if !self.queue.contains(&issue_number) && self.active_launch != Some(issue_number) {
            self.queue.push_back(issue_number);
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
        if !self.gui_connected || self.active_launch.is_some() {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        self.active_launch = Some(issue_number);
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launching;
        }
        Some(IssueMonitorLaunchRequest {
            issue_number,
            branch_name: format!("work/issue-{issue_number}"),
        })
    }

    pub fn complete_active_launch(&mut self, issue_number: u64, window_id: impl Into<String>) {
        if self.active_launch == Some(issue_number) {
            self.active_launch = None;
        }
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::Launched;
            item.launched_window_id = Some(window_id.into());
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

pub fn scan_issue_monitor_candidates<C: IssueClient>(
    monitor: &mut IssueMonitorState,
    client: &C,
    issues: &[IssueMonitorIssue],
    owner: &str,
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
        if !is_auto_improve_candidate(issue, &monitor.config) {
            summary.skipped += 1;
            continue;
        }

        let claim = ClaimComment {
            comment_id: None,
            claim_id: format!("gwt-auto-improve:{owner}:{}:{now}", issue.number),
            owner: owner.to_string(),
            issue_number: issue.number,
            status: ClaimStatus::Active,
            heartbeat_at: now.to_string(),
            expires_at: expiry_from_now_lexical(now, monitor.config.claim_ttl_secs),
            launched_work_id: Some(format!("work/issue-{}", issue.number)),
        };

        match acquire_claim(client, IssueNumber(issue.number), claim, now) {
            Ok(ClaimAcquireOutcome::Acquired(claim)) => {
                summary.claimed += 1;
                monitor.record_claimed(issue.clone(), claim.claim_id);
            }
            Ok(ClaimAcquireOutcome::Blocked(claim)) => {
                summary.blocked += 1;
                monitor.record_blocked_by_claim(issue.clone(), claim.owner, claim.expires_at);
            }
            Ok(ClaimAcquireOutcome::Lost { winning_claim, .. }) => {
                summary.blocked += 1;
                monitor.record_blocked_by_claim(
                    issue.clone(),
                    winning_claim.owner,
                    winning_claim.expires_at,
                );
            }
            Err(error) => {
                let message = format!("issue #{}: {error}", issue.number);
                summary.errors.push(message.clone());
                monitor.last_error = Some(message);
            }
        }
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

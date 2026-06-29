use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use gwt_git::branch_protection::BranchProtectionStatus;
use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

use crate::issue_monitor_review::{ReviewerIdentity, VerdictOutcome};
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
    /// Issues whose work PR merged. Persisted so completed work is not
    /// auto-relaunched while its GitHub Issue remains open until release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_issues: Vec<u64>,
    /// SPEC #3200 stage-one opt-in. When `false` (the default), the existing
    /// SPEC #3165 human gate is fully preserved and every other autonomous
    /// field is inert.
    #[serde(default)]
    pub autonomous_mode: bool,
    /// SPEC #3200 configurable decision-boundary parameters (FR-030).
    #[serde(default)]
    pub autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 per-issue autonomous lifecycle records (attempt counter,
    /// phase, acceptance snapshot, gated SHA, audit). Persisted across restarts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub autonomous_issues: BTreeMap<u64, AutonomousIssueRecord>,
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
            merged_issues: Vec::new(),
            autonomous_mode: false,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_issues: BTreeMap::new(),
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
    /// Work PR merged into the base branch — the agent's work is done and the
    /// active slot is freed. The GitHub Issue may still be open (gwt closes
    /// Issues at release time), so this is distinct from `Released`.
    Merged,
    /// The GitHub Issue was closed (e.g. at release). Final terminal state.
    Released,
    LaunchFailed,
    AgentFailed,
    BlockedByClaim,
    Skipped,
    /// Autonomous mode determined the issue cannot be resolved unattended (gate
    /// fail-closed, terminal review FAIL, max attempts, merge-watch timeout,
    /// snapshot tampering, or unverifiable branch protection). Terminal until a
    /// human explicitly clears it (SPEC #3200, FR-027).
    NeedsHuman,
}

impl MonitorInboxState {
    /// A terminal state whose meaning must not be overwritten by a later
    /// window/project close (which only re-queues still-active work).
    fn is_terminal(self) -> bool {
        matches!(
            self,
            MonitorInboxState::Merged
                | MonitorInboxState::Released
                | MonitorInboxState::LaunchFailed
                | MonitorInboxState::AgentFailed
                | MonitorInboxState::NeedsHuman
        )
    }
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

// ============================================================================
// SPEC #3200 — Autonomous Mode foundational data model
//
// These types are inert while `autonomous_mode` is OFF (the default): the
// existing SPEC #3165 lifecycle is preserved byte-for-byte. They become active
// only for two-stage opt-in eligible issues. 🔒 markers in the SPEC denote
// threat-model controls; their structural enforcement lands in Phase 2.
// ============================================================================

/// The GitHub label that, together with `autonomous_mode`, forms the two-stage
/// opt-in required for an issue to be autonomous-eligible (FR-003 (ii)).
pub const AUTO_MERGE_LABEL: &str = "auto-merge";

/// Whether an issue carries the `auto-merge` opt-in label.
pub fn has_auto_merge_label(issue: &IssueMonitorIssue) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(AUTO_MERGE_LABEL))
}

/// Configurable decision-boundary parameters for autonomous mode (FR-030).
///
/// Every field has a documented default and loads back-compat via
/// `#[serde(default)]`, so a prefs file written before this feature deserializes
/// to the documented defaults. The backoff factor is an integer multiplier
/// (not a float) so the surrounding prefs can keep `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousTuning {
    /// Max failure attempts `N` before escalating to needs-human (default 3).
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    /// Base backoff for the first transient retry, seconds (default 60).
    #[serde(default = "default_backoff_base_secs")]
    pub backoff_base_secs: u64,
    /// Integer backoff growth factor per retry (default 2).
    #[serde(default = "default_backoff_factor")]
    pub backoff_factor: u32,
    /// Backoff ceiling, seconds (default 1800).
    #[serde(default = "default_backoff_max_secs")]
    pub backoff_max_secs: u64,
    /// Idle/stuck timeout before slot recovery, seconds (default 900).
    #[serde(default = "default_stuck_idle_timeout_secs")]
    pub stuck_idle_timeout_secs: u64,
    /// Heartbeat emission interval for autonomous agents, seconds (default 60).
    #[serde(default = "default_heartbeat_interval_secs")]
    pub heartbeat_interval_secs: u64,
    /// Bound on watching for `merged_at` after Deliver handoff, seconds
    /// (default 3600).
    #[serde(default = "default_merge_watch_timeout_secs")]
    pub merge_watch_timeout_secs: u64,
    /// Deliver Fix-loop iteration cap, mapped onto the attempt counter and never
    /// exceeding `max_attempts` (default 3).
    #[serde(default = "default_deliver_fix_loop_cap")]
    pub deliver_fix_loop_cap: u32,
}

fn default_max_attempts() -> u32 {
    3
}
fn default_backoff_base_secs() -> u64 {
    60
}
fn default_backoff_factor() -> u32 {
    2
}
fn default_backoff_max_secs() -> u64 {
    1800
}
fn default_stuck_idle_timeout_secs() -> u64 {
    900
}
fn default_heartbeat_interval_secs() -> u64 {
    60
}
fn default_merge_watch_timeout_secs() -> u64 {
    3600
}
fn default_deliver_fix_loop_cap() -> u32 {
    3
}

impl Default for AutonomousTuning {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            backoff_base_secs: default_backoff_base_secs(),
            backoff_factor: default_backoff_factor(),
            backoff_max_secs: default_backoff_max_secs(),
            stuck_idle_timeout_secs: default_stuck_idle_timeout_secs(),
            heartbeat_interval_secs: default_heartbeat_interval_secs(),
            merge_watch_timeout_secs: default_merge_watch_timeout_secs(),
            deliver_fix_loop_cap: default_deliver_fix_loop_cap(),
        }
    }
}

impl AutonomousTuning {
    /// Bounded backoff delay (seconds) for the `attempt`-th retry (1-based):
    /// `base * factor^(attempt-1)`, saturating-clamped to `backoff_max_secs`.
    pub fn backoff_delay_secs(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let mut delay = self.backoff_base_secs;
        for _ in 1..attempt {
            delay = delay.saturating_mul(self.backoff_factor as u64);
            if delay >= self.backoff_max_secs {
                return self.backoff_max_secs;
            }
        }
        delay.min(self.backoff_max_secs)
    }
}

/// Kind of an acceptance criterion derived at pre-launch parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceCriterionKind {
    Behavioral,
    Visual,
}

/// A single acceptance criterion with a stable id (FR-012 / FR-003 (iii)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub text: String,
    pub kind: AcceptanceCriterionKind,
}

/// Immutable acceptance-criteria snapshot captured at launch (FR-012).
///
/// The independent review verifies against this snapshot, never a live re-read
/// of the Issue body, so an implementation agent cannot rewrite its own rubric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriteriaSnapshot {
    pub issue_number: u64,
    pub captured_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_etag: Option<String>,
    pub criteria: Vec<AcceptanceCriterion>,
    pub content_hash: String,
    /// `true` iff a structured block with ≥1 well-formed criterion was found.
    pub machine_checkable: bool,
    /// `true` iff any criterion is a visual surface (review-time encodability is
    /// judged separately, FR-014).
    pub visual_surface: bool,
}

impl AcceptanceCriteriaSnapshot {
    /// Stable criterion ids, used to bind the review verdict 1:1 to the rubric.
    pub fn criterion_ids(&self) -> Vec<String> {
        self.criteria.iter().map(|c| c.id.clone()).collect()
    }
}

fn acceptance_content_hash(criteria: &[AcceptanceCriterion]) -> String {
    let mut hasher = Sha256::new();
    for criterion in criteria {
        hasher.update(criterion.id.as_bytes());
        hasher.update([0u8]);
        hasher.update(criterion.text.as_bytes());
        hasher.update([0u8]);
        hasher.update(match criterion.kind {
            AcceptanceCriterionKind::Behavioral => b"b".as_slice(),
            AcceptanceCriterionKind::Visual => b"v".as_slice(),
        });
        hasher.update([b'\n']);
    }
    format!("{:x}", hasher.finalize())
}

fn is_acceptance_heading(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return false;
    }
    let title = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
    title.starts_with("acceptance criteria") || title.contains("受け入れ基準")
}

fn parse_acceptance_criterion_line(line: &str) -> Option<AcceptanceCriterion> {
    let trimmed = line.trim();
    let rest = trimmed
        .strip_prefix("- [")
        .or_else(|| trimmed.strip_prefix("* ["))?;
    let (_mark, after) = rest.split_once(']')?;
    let after = after.trim();
    let (id_part, text) = after.split_once(':')?;
    let id_part = id_part.trim();
    let (id, kind) = if let Some(paren) = id_part.find('(') {
        let id = id_part[..paren].trim();
        let tag = id_part[paren..].to_ascii_lowercase();
        let kind = if tag.contains("visual") {
            AcceptanceCriterionKind::Visual
        } else {
            AcceptanceCriterionKind::Behavioral
        };
        (id, kind)
    } else {
        (id_part, AcceptanceCriterionKind::Behavioral)
    };
    let text = text.trim();
    if id.is_empty()
        || text.is_empty()
        || id.split_whitespace().count() != 1
        || !id.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(AcceptanceCriterion {
        id: id.to_string(),
        text: text.to_string(),
        kind,
    })
}

/// Deterministically derive an acceptance-criteria snapshot from an Issue body
/// (FR-003 (iii) pre-launch gate, FR-014 visual-surface flagging).
///
/// Looks for an `Acceptance Criteria` / `受け入れ基準` heading, then collects
/// stable-id checklist items (`- [ ] AC-1: text`, `- [ ] AC-2 (visual): text`)
/// until the next heading. No agent is invoked: `machine_checkable` is `true`
/// iff the block exists with ≥1 well-formed criterion. An absent/unparseable
/// block yields `machine_checkable = false` (ineligible (iii) ⇒ needs-human).
pub fn parse_acceptance_criteria(
    issue_number: u64,
    body: &str,
    captured_at: impl Into<String>,
    source_etag: Option<String>,
) -> AcceptanceCriteriaSnapshot {
    let mut criteria = Vec::new();
    let mut in_block = false;
    for line in body.lines() {
        if is_acceptance_heading(line) {
            in_block = true;
            continue;
        }
        if in_block && line.trim_start().starts_with('#') {
            break;
        }
        if in_block {
            if let Some(criterion) = parse_acceptance_criterion_line(line) {
                criteria.push(criterion);
            }
        }
    }
    let machine_checkable = !criteria.is_empty();
    let visual_surface = criteria
        .iter()
        .any(|criterion| criterion.kind == AcceptanceCriterionKind::Visual);
    let content_hash = acceptance_content_hash(&criteria);
    AcceptanceCriteriaSnapshot {
        issue_number,
        captured_at: captured_at.into(),
        source_etag,
        criteria,
        content_hash,
        machine_checkable,
        visual_surface,
    }
}

/// Per-issue autonomous lifecycle phase (data-model #3). Terminal phases are
/// `Merged` and `NeedsHuman`; the latter only exits via an explicit human reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousPhase {
    #[default]
    Idle,
    Implementing,
    Gating,
    Gated,
    Delivering,
    Fixing,
    Merged,
    NeedsHuman,
}

impl AutonomousPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, AutonomousPhase::Merged | AutonomousPhase::NeedsHuman)
    }

    /// The legal autonomous phase transition table (data-model #3).
    pub fn can_transition_to(self, next: AutonomousPhase) -> bool {
        use AutonomousPhase::*;
        matches!(
            (self, next),
            (Idle, Implementing)
                | (Implementing, Implementing) // transient retry self-loop
                | (Implementing, Gating)
                | (Implementing, NeedsHuman)
                | (Gating, Gated)
                | (Gating, Fixing)
                | (Gating, NeedsHuman)
                | (Fixing, Gating)
                | (Fixing, NeedsHuman)
                | (Gated, Delivering)
                | (Gated, Gating) // HEAD advanced past reviewed SHA → re-gate
                | (Gated, NeedsHuman)
                | (Delivering, Merged)
                | (Delivering, Gating) // HEAD advance → re-gate
                | (Delivering, NeedsHuman)
                | (NeedsHuman, Idle) // human reset only
        )
    }
}

/// A link from a privileged skip token / audit record back to the gate
/// evaluation that produced it (data-model #7 / #12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateEvidenceRef {
    pub reviewed_sha: String,
    pub ci_pass: bool,
    pub matrix_pass: bool,
    pub review_pass: bool,
}

/// Reviewed-SHA binding for TOCTOU prevention (data-model #7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatedSha {
    pub pr_number: u64,
    pub reviewed_sha: String,
    pub base_ref: String,
    pub gated_at: String,
    pub gate_evidence: GateEvidenceRef,
}

/// Failure taxonomy classes (data-model #13, FR-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailureClass {
    /// CI red / test fail / reviewer-comment → Deliver Fix bounded retry.
    Remediable,
    /// Independent-review adversarial FAIL → immediate needs-human, no retry.
    Terminal,
    /// Crash / network / abnormal exit → bounded backoff retry.
    TransientInfra,
    /// Vacuous green / unverified branch protection / fail-closed → needs-human.
    GateUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureOutcome {
    pub class: GateFailureClass,
    pub reason: String,
    pub at: String,
}

/// Outcome of a single gate element, recorded for audit (FR-031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateElementOutcome {
    Pass,
    Fail,
    Unavailable,
}

/// The decision recorded for an autonomous merge judgement (FR-031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditDecision {
    Merged,
    NeedsHuman,
    Aborted,
}

/// Persisted audit record for one autonomous merge judgement (FR-031).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousMergeAuditRecord {
    pub issue_number: u64,
    pub pr_number: u64,
    pub gate_ci: GateElementOutcome,
    pub gate_matrix: GateElementOutcome,
    pub gate_review: GateElementOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<ReviewerIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict_overall: Option<VerdictOutcome>,
    pub reviewed_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_sha: Option<String>,
    pub attempt_count: u32,
    pub decision: AuditDecision,
    pub timestamp: String,
}

/// The privileged `skipped(autonomous-mode)` token (data-model #9, FR-017).
///
/// 🔒 Only the monitor control plane may sign this, bound to `reviewed_sha`. The
/// keyed signature plumbing is implemented in Phase 2 (T-091); this is the value
/// shape the verification-result schema carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousSkipToken {
    pub issue_number: u64,
    pub pr_number: u64,
    pub reviewed_sha: String,
    pub gate_evidence: GateEvidenceRef,
    pub signed_by: String,
    pub signature: String,
    pub signed_at: String,
}

/// PR verification-result schema understood by gwt-verify / gwt-manage-pr
/// Deliver (data-model #9, FR-017). `SkippedAutonomousMode` is distinct from a
/// human `SkippedHuman { reason }` and from `MonitorInboxState::Skipped`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PrVerificationResult {
    Confirmed,
    Pending,
    SkippedHuman { reason: String },
    SkippedAutonomousMode(AutonomousSkipToken),
}

/// Per-issue autonomous lifecycle record — the single typed container the
/// worker consumes (data-model #3, first-class entity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIssueRecord {
    pub issue_number: u64,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub phase: AutonomousPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_snapshot: Option<AcceptanceCriteriaSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gated: Option<GatedSha>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<FailureOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_model_identity: Option<ReviewerIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit: Vec<AutonomousMergeAuditRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_launch_id: Option<String>,
}

impl AutonomousIssueRecord {
    pub fn new(issue_number: u64) -> Self {
        Self {
            issue_number,
            attempt_count: 0,
            phase: AutonomousPhase::Idle,
            acceptance_snapshot: None,
            gated: None,
            last_failure: None,
            backoff_until: None,
            review_model_identity: None,
            audit: Vec::new(),
            needs_human_reason: None,
            active_launch_id: None,
        }
    }

    /// Attempt a phase transition; returns `false` (leaving the phase unchanged)
    /// for an illegal transition.
    pub fn try_transition(&mut self, next: AutonomousPhase) -> bool {
        if self.phase.can_transition_to(next) {
            self.phase = next;
            if next != AutonomousPhase::NeedsHuman {
                self.needs_human_reason = None;
            }
            true
        } else {
            false
        }
    }

    /// Idempotency guard (data-model #3, FR-026): claim the launch only when no
    /// other autonomous launch is active for this issue. Re-claiming the SAME id
    /// is a no-op success; a different id while one is active is rejected.
    pub fn claim_active_launch(&mut self, launch_id: &str) -> bool {
        match &self.active_launch_id {
            Some(existing) if existing == launch_id => true,
            Some(_) => false,
            None => {
                self.active_launch_id = Some(launch_id.to_string());
                true
            }
        }
    }

    pub fn release_active_launch(&mut self) {
        self.active_launch_id = None;
    }

    /// Escalate to the terminal needs-human phase with a reason (FR-023/FR-027).
    pub fn escalate_needs_human(&mut self, reason: impl Into<String>) {
        self.phase = AutonomousPhase::NeedsHuman;
        self.needs_human_reason = Some(reason.into());
        self.active_launch_id = None;
    }

    /// Human-driven exit from needs-human / attempt reset (FR-027).
    pub fn reset_for_human(&mut self) {
        self.phase = AutonomousPhase::Idle;
        self.needs_human_reason = None;
        self.attempt_count = 0;
        self.active_launch_id = None;
        self.last_failure = None;
        self.backoff_until = None;
    }
}

/// How an ineligible issue is routed (data-model #10): two-stage opt-in
/// negatives keep the existing #3165 human gate; missing preconditions surface
/// as needs-human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IneligibleRoute {
    HumanGate,
    NeedsHuman,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityDecision {
    Eligible,
    Ineligible {
        route: IneligibleRoute,
        reason: String,
    },
}

impl EligibilityDecision {
    pub fn is_eligible(&self) -> bool {
        matches!(self, EligibilityDecision::Eligible)
    }
}

/// Inputs to the pure eligibility predicate (data-model #10, FR-003).
pub struct EligibilityInputs<'a> {
    pub autonomous_mode: bool,
    pub has_auto_merge_label: bool,
    pub acceptance: Option<&'a AcceptanceCriteriaSnapshot>,
    pub branch_protection: &'a BranchProtectionStatus,
    pub record: Option<&'a AutonomousIssueRecord>,
    pub max_attempts: u32,
}

/// Pure two-stage-opt-in eligibility predicate (FR-003/004/005).
///
/// Routing (data-model #10): missing (i) `autonomous_mode` or (ii) `auto-merge`
/// label ⇒ `HumanGate` (the existing #3165 gate, unchanged). Missing (iii)
/// machine-checkable acceptance criteria, (iv) verified branch protection, or
/// (v) not-needs-human ∧ attempt<N ⇒ `NeedsHuman` with a surfaced reason.
pub fn autonomous_eligibility(inputs: &EligibilityInputs) -> EligibilityDecision {
    if !inputs.autonomous_mode {
        return EligibilityDecision::Ineligible {
            route: IneligibleRoute::HumanGate,
            reason: "autonomous_mode is OFF".to_string(),
        };
    }
    if !inputs.has_auto_merge_label {
        return EligibilityDecision::Ineligible {
            route: IneligibleRoute::HumanGate,
            reason: "auto-merge label is not present".to_string(),
        };
    }
    match inputs.acceptance {
        Some(snapshot) if snapshot.machine_checkable => {}
        _ => {
            return EligibilityDecision::Ineligible {
                route: IneligibleRoute::NeedsHuman,
                reason: "no machine-checkable acceptance criteria".to_string(),
            };
        }
    }
    if !inputs.branch_protection.verified {
        return EligibilityDecision::Ineligible {
            route: IneligibleRoute::NeedsHuman,
            reason: inputs
                .branch_protection
                .unavailable_reason()
                .unwrap_or_else(|| "branch protection is unverified".to_string()),
        };
    }
    if let Some(record) = inputs.record {
        if record.phase == AutonomousPhase::NeedsHuman {
            return EligibilityDecision::Ineligible {
                route: IneligibleRoute::NeedsHuman,
                reason: record
                    .needs_human_reason
                    .clone()
                    .unwrap_or_else(|| "issue is in needs-human state".to_string()),
            };
        }
        if record.attempt_count >= inputs.max_attempts {
            return EligibilityDecision::Ineligible {
                route: IneligibleRoute::NeedsHuman,
                reason: format!("attempt counter reached the max of {}", inputs.max_attempts),
            };
        }
    }
    EligibilityDecision::Eligible
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
    /// issue → work branch for currently launched Issues, used to look up the
    /// PR when checking whether the work has merged.
    launched_branches: BTreeMap<u64, String>,
    /// Issues whose work PR merged (state `Merged`). Persisted so the monitor
    /// does not auto-relaunch completed work even while the Issue stays open.
    merged_issues: BTreeSet<u64>,
    failed_issues: BTreeMap<u64, String>,
    queue: VecDeque<u64>,
    pending_launches: VecDeque<IssueMonitorLaunchRequest>,
    /// SPEC #3200 stage-one opt-in mirror of the prefs flag.
    #[serde(default)]
    autonomous_mode: bool,
    /// SPEC #3200 configurable decision boundaries (FR-030).
    #[serde(default)]
    autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 per-issue autonomous lifecycle records.
    #[serde(default)]
    autonomous_issues: BTreeMap<u64, AutonomousIssueRecord>,
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
            launched_branches: BTreeMap::new(),
            merged_issues: BTreeSet::new(),
            failed_issues: BTreeMap::new(),
            queue: VecDeque::new(),
            pending_launches: VecDeque::new(),
            autonomous_mode: false,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_issues: BTreeMap::new(),
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
        state.merged_issues = prefs.merged_issues.into_iter().collect();
        state.autonomous_mode = prefs.autonomous_mode;
        state.autonomous_tuning = prefs.autonomous_tuning;
        state.autonomous_issues = prefs.autonomous_issues;
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
            merged_issues: self.merged_issues.iter().copied().collect(),
            autonomous_mode: self.autonomous_mode,
            autonomous_tuning: self.autonomous_tuning.clone(),
            autonomous_issues: self.autonomous_issues.clone(),
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

    // ----- SPEC #3200 autonomous mode (foundational state API) --------------

    /// Stage-one opt-in flag (FR-001/002).
    pub fn autonomous_mode(&self) -> bool {
        self.autonomous_mode
    }

    /// Toggle the stage-one opt-in. The kill-switch side effects (disarming
    /// in-flight auto-merge) are wired into the daemon control plane (Phase 2);
    /// this only flips the runtime flag.
    pub fn set_autonomous_mode(&mut self, enabled: bool) {
        self.autonomous_mode = enabled;
    }

    /// Configurable decision boundaries (FR-030).
    pub fn autonomous_tuning(&self) -> &AutonomousTuning {
        &self.autonomous_tuning
    }

    /// The per-issue autonomous record, if one exists.
    pub fn autonomous_record(&self, issue_number: u64) -> Option<&AutonomousIssueRecord> {
        self.autonomous_issues.get(&issue_number)
    }

    fn autonomous_record_mut(&mut self, issue_number: u64) -> &mut AutonomousIssueRecord {
        self.autonomous_issues
            .entry(issue_number)
            .or_insert_with(|| AutonomousIssueRecord::new(issue_number))
    }

    /// Persisted per-issue failure attempt counter (FR-021).
    pub fn attempt_count(&self, issue_number: u64) -> u32 {
        self.autonomous_issues
            .get(&issue_number)
            .map(|record| record.attempt_count)
            .unwrap_or(0)
    }

    /// Increment and return the per-issue attempt counter (FR-021).
    pub fn bump_attempt(&mut self, issue_number: u64) -> u32 {
        let record = self.autonomous_record_mut(issue_number);
        record.attempt_count = record.attempt_count.saturating_add(1);
        record.attempt_count
    }

    /// Idempotently claim an autonomous launch for an issue (FR-026): rejects a
    /// second concurrent launch with a different id.
    pub fn claim_autonomous_launch(&mut self, issue_number: u64, launch_id: &str) -> bool {
        self.autonomous_record_mut(issue_number)
            .claim_active_launch(launch_id)
    }

    /// Capture the immutable acceptance-criteria snapshot at launch (FR-012).
    /// Returns whether the issue is machine-checkable (FR-003 (iii)).
    pub fn capture_acceptance_snapshot(
        &mut self,
        issue: &IssueMonitorIssue,
        captured_at: &str,
    ) -> bool {
        let snapshot = parse_acceptance_criteria(
            issue.number,
            issue.body.as_deref().unwrap_or(""),
            captured_at,
            None,
        );
        let machine_checkable = snapshot.machine_checkable;
        self.autonomous_record_mut(issue.number).acceptance_snapshot = Some(snapshot);
        machine_checkable
    }

    /// Detect divergence between the captured snapshot and the live Issue body
    /// (FR-012 / FR-018): an implementation agent rewriting its own rubric.
    pub fn acceptance_snapshot_diverged(&self, issue_number: u64, live_body: &str) -> bool {
        let Some(snapshot) = self
            .autonomous_issues
            .get(&issue_number)
            .and_then(|record| record.acceptance_snapshot.as_ref())
        else {
            return false;
        };
        let live = parse_acceptance_criteria(issue_number, live_body, &snapshot.captured_at, None);
        live.content_hash != snapshot.content_hash
    }

    /// Append an autonomous merge audit record (FR-031).
    pub fn record_merge_audit(&mut self, issue_number: u64, audit: AutonomousMergeAuditRecord) {
        self.autonomous_record_mut(issue_number).audit.push(audit);
    }

    /// Evaluate two-stage-opt-in eligibility for an issue against the supplied
    /// branch-protection status (FR-003/006). Uses the captured snapshot when
    /// present, else derives one from the live body for the pre-launch gate.
    pub fn evaluate_eligibility(
        &self,
        issue: &IssueMonitorIssue,
        branch_protection: &BranchProtectionStatus,
        now: &str,
    ) -> EligibilityDecision {
        let record = self.autonomous_issues.get(&issue.number);
        let snapshot = match record.and_then(|record| record.acceptance_snapshot.clone()) {
            Some(snapshot) => snapshot,
            None => parse_acceptance_criteria(
                issue.number,
                issue.body.as_deref().unwrap_or(""),
                now,
                None,
            ),
        };
        let inputs = EligibilityInputs {
            autonomous_mode: self.autonomous_mode,
            has_auto_merge_label: has_auto_merge_label(issue),
            acceptance: Some(&snapshot),
            branch_protection,
            record,
            max_attempts: self.autonomous_tuning.max_attempts,
        };
        autonomous_eligibility(&inputs)
    }

    /// Escalate an issue to the terminal needs-human state (FR-023/027): frees
    /// the active slot, removes it from the queue, records the reason on both
    /// the autonomous record and the inbox surface, and stops re-launch.
    pub fn escalate_needs_human(&mut self, issue_number: u64, reason: impl Into<String>) {
        let reason = reason.into();
        self.autonomous_record_mut(issue_number)
            .escalate_needs_human(reason.clone());
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.set_inbox_needs_human(issue_number, reason);
    }

    /// Human-driven exit from needs-human / attempt reset (FR-027). Clears the
    /// record back to idle and returns the issue to the eligible set on the next
    /// scan.
    pub fn reset_autonomous_issue(&mut self, issue_number: u64) {
        if let Some(record) = self.autonomous_issues.get_mut(&issue_number) {
            record.reset_for_human();
        }
    }

    fn set_inbox_needs_human(&mut self, issue_number: u64, reason: String) {
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = MonitorInboxState::NeedsHuman;
            item.launched_window_id = None;
            item.error_message = Some(reason);
        }
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
        // A terminal autonomous NeedsHuman item is preserved verbatim across
        // scans (SPEC #3200 FR-027 / Sc 21): never re-queued or re-launched, and
        // its surfaced reason is retained, until a human resets it.
        if let Some(item) = existing
            .as_ref()
            .filter(|item| item.state == MonitorInboxState::NeedsHuman)
        {
            let reason = item.error_message.clone().or_else(|| {
                self.autonomous_issues
                    .get(&issue_number)
                    .and_then(|record| record.needs_human_reason.clone())
            });
            let mut refreshed = item.clone();
            refreshed.launch_plan = Some(issue_monitor_launch_plan(&issue));
            refreshed.issue = issue;
            refreshed.error_message = reason;
            refreshed.launched_window_id = None;
            self.upsert_inbox(refreshed);
            self.apply_priority_order_to_inbox();
            return;
        }
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
        let merged = self.merged_issues.contains(&issue_number);
        let launched_window_id = if error_message.is_some() || merged {
            None
        } else {
            self.launched_windows.get(&issue_number).cloned()
        };
        let state = if merged {
            // Completed work stays Merged and is never re-queued while its Issue
            // remains open until release.
            MonitorInboxState::Merged
        } else if error_message.is_some() {
            existing
                .as_ref()
                .filter(|item| item.state == MonitorInboxState::LaunchFailed)
                .map(|item| item.state)
                .unwrap_or(MonitorInboxState::AgentFailed)
        } else if launched_window_id.is_some() {
            MonitorInboxState::Launched
        } else {
            match existing.as_ref().map(|item| item.state) {
                // A reopened Issue previously marked Released/Merged (but no
                // longer tracked as merged) returns to the queue.
                Some(MonitorInboxState::Released) | Some(MonitorInboxState::Merged) | None => {
                    MonitorInboxState::Queued
                }
                Some(other) => other,
            }
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
        if let Some(branch) = self
            .inbox_item(issue_number)
            .and_then(|item| item.launch_plan.as_ref())
            .map(|plan| plan.branch_name.clone())
        {
            self.launched_branches.insert(issue_number, branch);
        }
        // A fresh launch supersedes any prior Merged completion (e.g. manual
        // Launch Now of already-merged work).
        self.merged_issues.remove(&issue_number);
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

    /// Reverse-lookup the Issue associated with a launched agent `window_id`.
    pub fn launched_window_issue(&self, window_id: &str) -> Option<u64> {
        self.launched_windows
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
            })
    }

    fn clear_active_tracking(&mut self, issue_number: u64) {
        self.active_launches
            .retain(|active| *active != issue_number);
        self.launched_windows.remove(&issue_number);
        self.launched_branches.remove(&issue_number);
        self.pending_launches
            .retain(|pending| pending.issue_number != issue_number);
    }

    fn set_inbox_state(&mut self, issue_number: u64, state: MonitorInboxState) {
        if let Some(item) = self
            .inbox
            .iter_mut()
            .find(|item| item.issue.number == issue_number)
        {
            item.state = state;
            item.launched_window_id = None;
            item.error_message = None;
        }
    }

    /// Record that the launched work for `issue_number` merged into the base
    /// branch. Frees the active slot and marks the Issue `Merged` (persisted so
    /// completed work is not auto-relaunched while the Issue stays open until
    /// release).
    pub fn record_merged(&mut self, issue_number: u64) {
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.merged_issues.insert(issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Merged);
    }

    /// Record that the GitHub Issue for `issue_number` was closed (released).
    pub fn record_released(&mut self, issue_number: u64) {
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Released);
    }

    /// issue → work branch for every currently active (launched) Issue. Uses
    /// the stored launch branch, falling back to the inbox launch plan.
    pub fn active_launched_branches(&self) -> Vec<(u64, String)> {
        self.active_launches
            .iter()
            .filter_map(|number| {
                let branch = self.launched_branches.get(number).cloned().or_else(|| {
                    self.inbox_item(*number)
                        .and_then(|item| item.launch_plan.as_ref())
                        .map(|plan| plan.branch_name.clone())
                })?;
                Some((*number, branch))
            })
            .collect()
    }

    /// Mark any active launched Issue whose work branch has a merged PR as
    /// `Merged`, freeing the active slot. Returns the affected Issue numbers.
    pub fn reconcile_merged_branches(&mut self, merged_branches: &BTreeSet<String>) -> Vec<u64> {
        let to_merge: Vec<u64> = self
            .active_launched_branches()
            .into_iter()
            .filter(|(_, branch)| merged_branches.contains(branch))
            .map(|(number, _)| number)
            .collect();
        for number in &to_merge {
            self.record_merged(*number);
        }
        to_merge
    }

    /// An agent window closed without the work completing. Frees the active
    /// slot and returns the Issue to pending (`Queued`) — never a fabricated
    /// "done" state. Terminal states (Merged/Released/failed) are preserved.
    /// Returns the affected Issue number when the window mapped to an active
    /// launch that was re-queued.
    pub fn requeue_window(&mut self, window_id: &str) -> Option<u64> {
        let issue_number = self.launched_window_issue(window_id)?;
        if self.merged_issues.contains(&issue_number) {
            return None;
        }
        if self
            .inbox_item(issue_number)
            .is_some_and(|item| item.state.is_terminal())
        {
            return None;
        }
        self.clear_active_tracking(issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
        if !self.queue.contains(&issue_number) {
            self.queue.push_back(issue_number);
            self.apply_priority_order_to_queue();
        }
        Some(issue_number)
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

    fn launched_monitor(number: u64, window_id: &str) -> IssueMonitorState {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(number)], "2026-06-26T00:00:00Z");
        monitor.complete_active_launch(number, window_id);
        assert_eq!(monitor.active_count(), 1);
        monitor
    }

    #[test]
    fn record_merged_frees_slot_marks_done_and_is_not_requeued() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_merged(42);
        assert_eq!(monitor.active_count(), 0, "Merged frees the active slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        // A later scan must keep it Merged (not re-queued) while the Issue is
        // still open.
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-06-26T01:00:00Z");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
        assert_eq!(monitor.queue_len(), 0);
        assert_eq!(monitor.active_count(), 0);
    }

    #[test]
    fn requeue_window_returns_unmerged_issue_to_pending() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let requeued = monitor.requeue_window("tab-1::agent-1");
        assert_eq!(requeued, Some(42));
        assert_eq!(monitor.active_count(), 0, "closing frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "closing an unmerged window returns to pending, never a fake done state"
        );
        assert_eq!(monitor.queue_len(), 1);
    }

    #[test]
    fn requeue_window_does_not_revert_merged() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_merged(42);
        assert_eq!(monitor.requeue_window("tab-1::agent-1"), None);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
    }

    #[test]
    fn record_released_marks_released_and_frees_slot() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_released(42);
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Released)
        );
    }

    #[test]
    fn reconcile_merged_branches_marks_merged_and_frees_slot() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let branch = monitor
            .active_launched_branches()
            .into_iter()
            .find(|(number, _)| *number == 42)
            .map(|(_, branch)| branch)
            .expect("launched branch");
        let merged: BTreeSet<String> = [branch].into_iter().collect();
        assert_eq!(monitor.reconcile_merged_branches(&merged), vec![42]);
        assert_eq!(monitor.active_count(), 0, "merged work frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged)
        );
    }

    #[test]
    fn reconcile_merged_branches_ignores_unmerged_branches() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        let merged: BTreeSet<String> = ["work/some-other-branch".to_string()].into_iter().collect();
        assert!(monitor.reconcile_merged_branches(&merged).is_empty());
        assert_eq!(monitor.active_count(), 1, "unmerged work stays launched");
    }

    #[test]
    fn merged_issues_survive_prefs_roundtrip_and_block_relaunch() {
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_merged(42);
        let prefs = monitor.prefs();
        assert_eq!(prefs.merged_issues, vec![42]);

        let mut restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-06-26T02:00:00Z");
        assert_eq!(
            restored.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged),
            "restored monitor must not re-launch already-merged work"
        );
        assert_eq!(restored.queue_len(), 0);
    }

    // ===== SPEC #3200 — Autonomous Mode foundational tests ==================

    const ACCEPTANCE_BODY: &str = "# Issue\n\nDescription.\n\n## Acceptance Criteria\n\n- [ ] AC-1: the endpoint returns success\n- [ ] AC-2: the active slot is freed\n\n## Notes\nunrelated section\n";

    fn auto_merge_issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some(ACCEPTANCE_BODY.to_string()),
            url: None,
        }
    }

    fn protected() -> BranchProtectionStatus {
        BranchProtectionStatus::fully_protected("main")
    }

    fn machine_checkable_snapshot(number: u64) -> AcceptanceCriteriaSnapshot {
        parse_acceptance_criteria(
            number,
            "## Acceptance Criteria\n- [ ] AC-1: works\n",
            "2026-06-29T00:00:00Z",
            None,
        )
    }

    // --- T-010: prefs defaults + back-compat fixture ---

    #[test]
    fn autonomous_prefs_default_off_with_documented_tuning() {
        let prefs = IssueMonitorPrefs::default();
        assert!(!prefs.autonomous_mode);
        assert!(prefs.autonomous_issues.is_empty());
        let tuning = &prefs.autonomous_tuning;
        assert_eq!(tuning.max_attempts, 3);
        assert_eq!(tuning.backoff_base_secs, 60);
        assert_eq!(tuning.backoff_factor, 2);
        assert_eq!(tuning.backoff_max_secs, 1800);
        assert_eq!(tuning.stuck_idle_timeout_secs, 900);
        assert_eq!(tuning.heartbeat_interval_secs, 60);
        assert_eq!(tuning.merge_watch_timeout_secs, 3600);
        assert_eq!(tuning.deliver_fix_loop_cap, 3);
    }

    #[test]
    fn pre_autonomous_prefs_fixture_loads_with_defaults() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/issue_monitor_prefs_pre_autonomous.json"
        );
        let raw = std::fs::read_to_string(path).expect("fixture readable");
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(&raw).expect("pre-autonomous prefs deserialize (back-compat)");
        // Existing #3165 fields preserved.
        assert!(prefs.enabled);
        assert_eq!(prefs.max_active_agents, 2);
        assert_eq!(prefs.merged_issues, vec![204]);
        // New SPEC #3200 fields fall back to documented defaults.
        assert!(!prefs.autonomous_mode);
        assert!(prefs.autonomous_issues.is_empty());
        assert_eq!(prefs.autonomous_tuning, AutonomousTuning::default());
    }

    #[test]
    fn backoff_delay_is_bounded() {
        let tuning = AutonomousTuning::default();
        assert_eq!(tuning.backoff_delay_secs(0), 0);
        assert_eq!(tuning.backoff_delay_secs(1), 60);
        assert_eq!(tuning.backoff_delay_secs(2), 120);
        assert_eq!(tuning.backoff_delay_secs(3), 240);
        // Saturates at the configured ceiling.
        assert_eq!(tuning.backoff_delay_secs(20), 1800);
    }

    // --- T-011: NeedsHuman terminal + attempt counter ---

    #[test]
    fn needs_human_is_terminal_and_not_revived() {
        assert!(MonitorInboxState::NeedsHuman.is_terminal());
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.escalate_needs_human(42, "independent review FAIL: AC-2 unmet");
        assert_eq!(monitor.active_count(), 0, "needs-human frees the slot");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert_eq!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.error_message.clone()),
            Some("independent review FAIL: AC-2 unmet".to_string())
        );
        // Window close must NOT revive a terminal needs-human item.
        assert_eq!(monitor.requeue_window("tab-1::agent-1"), None);
        // A later scan must NOT re-queue it, and must preserve the reason.
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-06-29T00:00:00Z");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert_eq!(monitor.queue_len(), 0);
        assert_eq!(
            monitor
                .inbox_item(42)
                .and_then(|item| item.error_message.clone()),
            Some("independent review FAIL: AC-2 unmet".to_string())
        );
    }

    #[test]
    fn attempt_counter_persists_across_reload() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert_eq!(monitor.bump_attempt(7), 1);
        assert_eq!(monitor.bump_attempt(7), 2);
        assert_eq!(monitor.attempt_count(7), 2);
        let prefs = monitor.prefs();
        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        assert_eq!(
            restored.attempt_count(7),
            2,
            "attempt counter survives save/reload"
        );
    }

    #[test]
    fn human_reset_clears_needs_human_and_attempts() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.bump_attempt(9);
        monitor.escalate_needs_human(9, "max attempts reached");
        assert_eq!(
            monitor.autonomous_record(9).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
        monitor.reset_autonomous_issue(9);
        assert_eq!(
            monitor.autonomous_record(9).map(|record| record.phase),
            Some(AutonomousPhase::Idle)
        );
        assert_eq!(monitor.attempt_count(9), 0);
    }

    // --- T-012: eligibility truth table + routing + snapshot + audit + token ---

    #[test]
    fn eligibility_two_stage_optin_truth_table() {
        let bp = protected();
        let snapshot = machine_checkable_snapshot(1);
        let inputs = |mode: bool, label: bool| EligibilityInputs {
            autonomous_mode: mode,
            has_auto_merge_label: label,
            acceptance: Some(&snapshot),
            branch_protection: &bp,
            record: None,
            max_attempts: 3,
        };
        // Both ON ⇒ eligible.
        assert!(autonomous_eligibility(&inputs(true, true)).is_eligible());
        // Either opt-in missing ⇒ HumanGate (#3165 preserved), never NeedsHuman.
        for (mode, label) in [(false, true), (true, false), (false, false)] {
            match autonomous_eligibility(&inputs(mode, label)) {
                EligibilityDecision::Ineligible { route, .. } => {
                    assert_eq!(route, IneligibleRoute::HumanGate)
                }
                other => panic!("expected HumanGate for ({mode},{label}), got {other:?}"),
            }
        }
    }

    #[test]
    fn eligibility_missing_preconditions_route_to_needs_human() {
        let bp = protected();
        let snapshot = machine_checkable_snapshot(1);
        let is_needs_human = |decision: EligibilityDecision| {
            matches!(
                decision,
                EligibilityDecision::Ineligible {
                    route: IneligibleRoute::NeedsHuman,
                    ..
                }
            )
        };

        // (iii) no machine-checkable acceptance criteria.
        let no_ac = parse_acceptance_criteria(1, "prose, no structured criteria", "now", None);
        assert!(is_needs_human(autonomous_eligibility(&EligibilityInputs {
            autonomous_mode: true,
            has_auto_merge_label: true,
            acceptance: Some(&no_ac),
            branch_protection: &bp,
            record: None,
            max_attempts: 3,
        })));

        // (iv) unverified branch protection.
        let absent = BranchProtectionStatus::absent("main");
        assert!(is_needs_human(autonomous_eligibility(&EligibilityInputs {
            autonomous_mode: true,
            has_auto_merge_label: true,
            acceptance: Some(&snapshot),
            branch_protection: &absent,
            record: None,
            max_attempts: 3,
        })));

        // (v) issue already in needs-human.
        let mut needs_human = AutonomousIssueRecord::new(1);
        needs_human.escalate_needs_human("prior terminal fail");
        assert!(is_needs_human(autonomous_eligibility(&EligibilityInputs {
            autonomous_mode: true,
            has_auto_merge_label: true,
            acceptance: Some(&snapshot),
            branch_protection: &bp,
            record: Some(&needs_human),
            max_attempts: 3,
        })));

        // (v) attempt counter at the max.
        let mut maxed = AutonomousIssueRecord::new(1);
        maxed.attempt_count = 3;
        assert!(is_needs_human(autonomous_eligibility(&EligibilityInputs {
            autonomous_mode: true,
            has_auto_merge_label: true,
            acceptance: Some(&snapshot),
            branch_protection: &bp,
            record: Some(&maxed),
            max_attempts: 3,
        })));
    }

    #[test]
    fn evaluate_eligibility_via_state_uses_label_and_body() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_mode(true);
        let bp = protected();
        let issue = auto_merge_issue(50);
        assert!(monitor
            .evaluate_eligibility(&issue, &bp, "now")
            .is_eligible());
        monitor.set_autonomous_mode(false);
        match monitor.evaluate_eligibility(&issue, &bp, "now") {
            EligibilityDecision::Ineligible { route, .. } => {
                assert_eq!(route, IneligibleRoute::HumanGate)
            }
            other => panic!("OFF must route to HumanGate, got {other:?}"),
        }
    }

    #[test]
    fn acceptance_snapshot_divergence_detected() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let issue = auto_merge_issue(60);
        assert!(monitor.capture_acceptance_snapshot(&issue, "2026-06-29T00:00:00Z"));
        // Identical body ⇒ no divergence.
        assert!(!monitor.acceptance_snapshot_diverged(60, issue.body.as_deref().unwrap()));
        // Tampered criteria ⇒ divergence (agent rewrote its own rubric).
        let tampered = "## Acceptance Criteria\n- [ ] AC-1: anything passes now\n";
        assert!(monitor.acceptance_snapshot_diverged(60, tampered));
    }

    #[test]
    fn audit_record_roundtrips() {
        let audit = AutonomousMergeAuditRecord {
            issue_number: 1,
            pr_number: 9,
            gate_ci: GateElementOutcome::Pass,
            gate_matrix: GateElementOutcome::Pass,
            gate_review: GateElementOutcome::Pass,
            reviewer: Some(ReviewerIdentity {
                agent_id: "codex".into(),
                model: "gpt-5".into(),
                provider: "openai".into(),
                same_model_fallback: false,
            }),
            verdict_overall: Some(VerdictOutcome::Pass),
            reviewed_sha: "abc".into(),
            merged_sha: Some("abc".into()),
            attempt_count: 1,
            decision: AuditDecision::Merged,
            timestamp: "2026-06-29T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&audit).unwrap();
        let back: AutonomousMergeAuditRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(audit, back);
    }

    #[test]
    fn skipped_autonomous_mode_is_distinct_privileged_value() {
        let token = AutonomousSkipToken {
            issue_number: 1,
            pr_number: 2,
            reviewed_sha: "sha-a".into(),
            gate_evidence: GateEvidenceRef {
                reviewed_sha: "sha-a".into(),
                ci_pass: true,
                matrix_pass: true,
                review_pass: true,
            },
            signed_by: "monitor-control-plane".into(),
            signature: "sig".into(),
            signed_at: "2026-06-29T00:00:00Z".into(),
        };
        let auto = PrVerificationResult::SkippedAutonomousMode(token);
        let human = PrVerificationResult::SkippedHuman {
            reason: "looks good".into(),
        };
        assert_ne!(auto, human);
        let json = serde_json::to_string(&auto).unwrap();
        assert!(json.contains("\"result\":\"skipped_autonomous_mode\""));
        assert!(json.contains("\"reviewed_sha\":\"sha-a\""));
        let back: PrVerificationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, auto);
        // Distinct concept from the inbox Skipped state.
        assert_ne!(
            serde_json::to_string(&MonitorInboxState::Skipped).unwrap(),
            json
        );
    }

    // --- T-020: AutonomousIssueRecord phase + idempotency ---

    #[test]
    fn autonomous_phase_transitions_enforced() {
        use AutonomousPhase::*;
        for (from, to) in [
            (Idle, Implementing),
            (Implementing, Gating),
            (Implementing, Implementing),
            (Gating, Gated),
            (Gating, Fixing),
            (Fixing, Gating),
            (Gated, Delivering),
            (Delivering, Merged),
            (Gating, NeedsHuman),
            (Gated, Gating),
            (Delivering, Gating),
            (NeedsHuman, Idle),
        ] {
            assert!(
                from.can_transition_to(to),
                "expected legal {from:?}->{to:?}"
            );
        }
        for (from, to) in [
            (Idle, Gated),
            (Idle, Merged),
            (Merged, Gating),
            (Merged, Idle),
            (Gating, Merged),
            (Implementing, Merged),
            (NeedsHuman, Gating),
        ] {
            assert!(
                !from.can_transition_to(to),
                "expected illegal {from:?}->{to:?}"
            );
        }
        assert!(Merged.is_terminal() && NeedsHuman.is_terminal());
        assert!(!Gating.is_terminal());
    }

    #[test]
    fn try_transition_rejects_illegal() {
        let mut record = AutonomousIssueRecord::new(1);
        assert!(record.try_transition(AutonomousPhase::Implementing));
        assert!(!record.try_transition(AutonomousPhase::Merged));
        assert_eq!(record.phase, AutonomousPhase::Implementing);
    }

    #[test]
    fn active_launch_id_idempotency_guard() {
        let mut record = AutonomousIssueRecord::new(1);
        assert!(record.claim_active_launch("launch-A"));
        assert!(
            record.claim_active_launch("launch-A"),
            "re-claiming the same id is idempotent"
        );
        assert!(
            !record.claim_active_launch("launch-B"),
            "a second concurrent launch is rejected"
        );
        record.release_active_launch();
        assert!(record.claim_active_launch("launch-B"));
    }

    #[test]
    fn state_claim_autonomous_launch_is_idempotent() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(monitor.claim_autonomous_launch(5, "L1"));
        assert!(!monitor.claim_autonomous_launch(5, "L2"));
    }

    #[test]
    fn autonomous_record_full_roundtrip() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let issue = auto_merge_issue(77);
        monitor.capture_acceptance_snapshot(&issue, "2026-06-29T00:00:00Z");
        monitor.bump_attempt(77);
        assert!(monitor.claim_autonomous_launch(77, "L1"));
        monitor.record_merge_audit(
            77,
            AutonomousMergeAuditRecord {
                issue_number: 77,
                pr_number: 5,
                gate_ci: GateElementOutcome::Pass,
                gate_matrix: GateElementOutcome::Pass,
                gate_review: GateElementOutcome::Pass,
                reviewer: None,
                verdict_overall: None,
                reviewed_sha: "abc".into(),
                merged_sha: None,
                attempt_count: 1,
                decision: AuditDecision::Aborted,
                timestamp: "2026-06-29T00:00:00Z".into(),
            },
        );
        let prefs = monitor.prefs();
        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let record = restored
            .autonomous_record(77)
            .expect("autonomous record survives save/reload");
        assert_eq!(record.attempt_count, 1);
        assert_eq!(record.active_launch_id.as_deref(), Some("L1"));
        assert!(record.acceptance_snapshot.is_some());
        assert_eq!(record.audit.len(), 1);
    }

    // --- T-021: pre-launch acceptance-criteria classifier ---

    #[test]
    fn acceptance_classifier_detects_machine_checkable_and_visual() {
        let body = "## Acceptance Criteria\n- [ ] AC-1: backend returns 200\n- [ ] AC-2 (visual): the button is centered\n## Notes\nx\n";
        let snapshot = parse_acceptance_criteria(1, body, "now", None);
        assert!(snapshot.machine_checkable);
        assert!(snapshot.visual_surface);
        assert_eq!(snapshot.criteria.len(), 2);
        assert_eq!(snapshot.criterion_ids(), vec!["AC-1", "AC-2"]);
        assert_eq!(snapshot.criteria[1].kind, AcceptanceCriterionKind::Visual);
    }

    #[test]
    fn acceptance_classifier_behavioral_only() {
        let snapshot = parse_acceptance_criteria(
            1,
            "### Acceptance Criteria\n- [x] CHK-1: it works\n",
            "now",
            None,
        );
        assert!(snapshot.machine_checkable);
        assert!(!snapshot.visual_surface);
    }

    #[test]
    fn acceptance_classifier_absent_block_is_not_machine_checkable() {
        let snapshot =
            parse_acceptance_criteria(1, "# Title\nSome prose, no criteria.", "now", None);
        assert!(!snapshot.machine_checkable);
        assert!(snapshot.criteria.is_empty());
    }

    #[test]
    fn acceptance_classifier_japanese_heading() {
        let snapshot =
            parse_acceptance_criteria(1, "## 受け入れ基準\n- [ ] AC-1: 期待挙動\n", "now", None);
        assert!(snapshot.machine_checkable);
    }

    #[test]
    fn acceptance_classifier_ignores_malformed_lines() {
        let body = "## Acceptance Criteria\n- [ ] no id colon here\n- bullet without checkbox\n- [ ] a whole sentence: not an id\n- [ ] AC-9: the valid one\n";
        let snapshot = parse_acceptance_criteria(1, body, "now", None);
        assert_eq!(snapshot.criterion_ids(), vec!["AC-9"]);
    }
}

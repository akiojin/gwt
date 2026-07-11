use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};

use gwt_github::{
    issue_auto_claim::{acquire_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus},
    IssueClient, IssueNumber,
};

use crate::{
    has_gwt_spec_label, knowledge_launch_target_branch_name, LaunchWizardPreviousProfile,
    LinkedIssueKind,
};

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

/// SPEC #3200 FR-030: tunable bounds for autonomous (unattended) operation.
/// Every field has a documented default so older prefs deserialize cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousTuning {
    /// Max failed attempts per issue before escalating to `NeedsHuman`
    /// (FR-021). Bounds the auto-relaunch / Deliver-fix loop.
    pub max_attempts: u32,
    /// An active agent with no liveness progress for this long is considered
    /// stuck; its active slot is recovered (FR-025).
    pub stuck_timeout_secs: u64,
    /// Heartbeat freshness window used by stuck/idle detection (FR-025).
    pub heartbeat_interval_secs: u64,
    /// Max time to watch a PR toward merge before treating it as stuck
    /// (FR-018 merge-watch).
    pub merge_watch_timeout_secs: u64,
    /// Max Deliver Fix-loop iterations within one attempt before the attempt
    /// counts as a failure.
    pub deliver_fix_loop_cap: u32,
    /// Base backoff seconds for transient-failure retry (FR-022/FR-024).
    pub retry_backoff_base_secs: u64,
    /// Upper bound for the (exponential) retry backoff.
    pub retry_backoff_cap_secs: u64,
    /// SPEC #3200 FR-015: the model the INDEPENDENT review agent runs on. When
    /// set (and different from the implementer's model) the review is forced onto
    /// it so the verdict is not a self-grade. `None` falls back to the saved
    /// launch profile's model (still a fresh, adversarial session).
    #[serde(default)]
    pub review_model: Option<String>,
}

impl Default for AutonomousTuning {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            stuck_timeout_secs: 1800,
            heartbeat_interval_secs: 120,
            merge_watch_timeout_secs: 3600,
            deliver_fix_loop_cap: 5,
            retry_backoff_base_secs: 60,
            retry_backoff_cap_secs: 1800,
            review_model: None,
        }
    }
}

/// SPEC #3200 FR-015: pick the model an independent review should run on, given
/// the implementer's model and the configured `review_model`. Returns the
/// configured model only when it is set AND genuinely different from the
/// implementer's (avoids a self-grade); otherwise `None` (caller keeps the saved
/// profile model — still a fresh adversarial session).
pub fn resolve_review_model(
    implementer_model: Option<&str>,
    configured_review_model: Option<&str>,
) -> Option<String> {
    let configured = configured_review_model?.trim();
    if configured.is_empty() {
        return None;
    }
    match implementer_model {
        Some(impl_model) if impl_model.eq_ignore_ascii_case(configured) => None,
        _ => Some(configured.to_string()),
    }
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
    /// Issue #3222: claims whose agent window is not bound yet (`Launching`).
    /// Persisted so an in-flight claim survives the per-handler prefs
    /// roundtrip — otherwise a rescan re-claims the same issue (same-owner
    /// renewal) and spawns a duplicate window past `max_active`.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_launching_issues"
    )]
    pub launching_issues: Vec<IssueMonitorLaunchingIssue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_issues: Vec<IssueMonitorFailedIssue>,
    /// Issues whose work PR merged. Persisted so completed work is not
    /// auto-relaunched while its GitHub Issue remains open until release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_issues: Vec<u64>,
    /// SPEC #3200: opt-in autonomous (unattended) resolution mode. Default
    /// `false` preserves SPEC #3165 human-gated behavior exactly (FR-001).
    #[serde(default)]
    pub autonomous_mode: bool,
    /// SPEC #3200 FR-030: tunable bounds for unattended operation.
    #[serde(default)]
    pub autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 T-016/T-022: per-issue autonomous state (attempt counter,
    /// phase, in-flight launch id, acceptance snapshot). Persisted so an
    /// in-flight attempt survives a daemon restart.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autonomous_records: Vec<AutonomousIssueRecord>,
}

impl Default for IssueMonitorPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            max_active_agents: 1,
            priority_order: Vec::new(),
            launch_profile: None,
            launched_issues: Vec::new(),
            launching_issues: Vec::new(),
            failed_issues: Vec::new(),
            merged_issues: Vec::new(),
            autonomous_mode: false,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_records: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchedIssue {
    pub issue_number: u64,
    pub window_id: String,
}

/// #3223 follow-up: one claimed-but-unbound launch with its claim anchor.
/// `claimed_at` lets a restored claim EXPIRE after `claim_ttl_secs` instead of
/// holding a max-active slot forever when the process died before the ACK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorLaunchingIssue {
    pub issue_number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
}

/// Backward-compat: the first shipped shape was a bare id array. A parse
/// failure here would `unwrap_or_default()` into a full prefs wipe, so both
/// shapes must deserialize.
fn deserialize_launching_issues<'de, D>(
    deserializer: D,
) -> Result<Vec<IssueMonitorLaunchingIssue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Bare(u64),
        Full(IssueMonitorLaunchingIssue),
    }
    let entries = Vec::<Compat>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|entry| match entry {
            Compat::Bare(issue_number) => IssueMonitorLaunchingIssue {
                issue_number,
                claimed_at: None,
            },
            Compat::Full(full) => full,
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueMonitorFailedIssue {
    pub issue_number: u64,
    pub message: String,
    /// #3165 error-window lifecycle: the agent window that was on the canvas
    /// when this issue failed. Persisted so an explicit Launch Now (even after a
    /// daemon/GUI restart) can close the stale window before relaunching. `None`
    /// for failures that never opened a window (e.g. pre-launch errors).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
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
    /// SPEC #3200 FR-027: autonomous resolution exhausted its bounded retries,
    /// hit a terminal review failure, or could not verify its safety gates, and
    /// has been handed back to a human. Terminal: scan / requeue / window-close
    /// must never revive it; only an explicit human reset exits it.
    NeedsHuman,
}

impl MonitorInboxState {
    /// A terminal state whose meaning must not be overwritten by a later
    /// window/project close (which only re-queues still-active work) or by a
    /// scan re-queue.
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
    /// SPEC #3200 T-048/FR-001: whether unattended autonomous mode is enabled.
    #[serde(default)]
    pub autonomous_mode: bool,
    /// SPEC #3200 T-048/FR-033: per-issue autonomous lifecycle summary, so every
    /// decision boundary (phase, attempts, needs-human) is observable.
    #[serde(default)]
    pub autonomous_issues: Vec<AutonomousIssueSummary>,
}

/// SPEC #3200 T-048: status-view summary of one issue's autonomous lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIssueSummary {
    pub issue_number: u64,
    pub phase: AutonomousPhase,
    pub attempts: u32,
    pub needs_human: bool,
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
    /// SPEC #3200 FR-001: opt-in autonomous (unattended) resolution mode.
    autonomous_mode: bool,
    /// SPEC #3200 FR-030: tunable bounds for unattended operation.
    autonomous_tuning: AutonomousTuning,
    /// SPEC #3200 T-016/T-022: per-issue autonomous lifecycle records keyed by
    /// issue number (attempt counter, phase, in-flight launch id, snapshot).
    autonomous_records: BTreeMap<u64, AutonomousIssueRecord>,
    failed_issues: BTreeMap<u64, String>,
    /// #3165 error-window lifecycle: the stale agent window id retained per
    /// failed issue, so an explicit Launch Now can close it before relaunching.
    failed_windows: BTreeMap<u64, String>,
    queue: VecDeque<u64>,
    pending_launches: VecDeque<IssueMonitorLaunchRequest>,
    /// SPEC #3200 Option A: review-agent spawn requests produced by the
    /// orchestration loop, drained by the daemon→GUI payload builder.
    pending_review_dispatches: VecDeque<AutonomousReviewDispatch>,
    /// SPEC #3200 FR-034 (T-111): operator notices produced by autonomous
    /// lifecycle transitions (merged / needs-human / retry / auto-merge armed).
    /// Drained into `toast` payloads by the daemon→GUI payload builder; retained
    /// (bounded) while no GUI is connected so unattended events surface on the
    /// next connect.
    pending_autonomous_notices: VecDeque<AutonomousNotice>,
    /// #3223 follow-up: claim anchors for unbound launches (issue → RFC3339).
    launching_claimed_at: BTreeMap<u64, String>,
}

pub fn is_auto_improve_candidate(issue: &IssueMonitorIssue, config: &IssueMonitorConfig) -> bool {
    let _ = config;
    issue.state == IssueMonitorIssueState::Open
}

/// SPEC #3200 FR-003/004/005: routing decision for whether an open Issue may be
/// resolved by the autonomous (unattended) path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityDecision {
    /// Two-stage opt-in satisfied and every safety precondition holds — the
    /// Issue may be resolved autonomously.
    Eligible,
    /// The two-stage opt-in is NOT satisfied (autonomous_mode off OR no
    /// `auto-merge` label) — fall back to the existing SPEC #3165 human-gated
    /// flow unchanged.
    HumanGate(String),
    /// Two-stage opt-in IS satisfied but a safety precondition failed (no
    /// machine-checkable criteria, unverified branch protection, already
    /// needs-human, or attempts exhausted) — hand to a human; never auto-run.
    NeedsHuman(String),
}

/// Pure autonomous-eligibility predicate (SPEC #3200 FR-003).
///
/// Routing (the negatives matter as much as the positive):
/// - missing (i) `autonomous_mode` or (ii) the `auto-merge` label ⇒ `HumanGate`
///   — these two-stage-opt-in negatives use the existing #3165 gate, NOT
///   `NeedsHuman`.
/// - already needs-human, attempts exhausted, missing (iii) machine-checkable
///   criteria, or (iv) verified branch protection ⇒ `NeedsHuman(reason)`.
/// - all satisfied ⇒ `Eligible`.
#[allow(clippy::too_many_arguments)]
pub fn autonomous_eligibility(
    autonomous_mode: bool,
    has_auto_merge_label: bool,
    criteria: &crate::issue_monitor_gate::AcceptanceCriteria,
    protection: &gwt_git::branch_protection::BranchProtectionStatus,
    is_needs_human: bool,
    attempt_count: u32,
    max_attempts: u32,
) -> EligibilityDecision {
    // Stage 1 — two-stage opt-in. Either negative falls back to the existing
    // human-gated #3165 behavior, NOT to needs-human.
    if !autonomous_mode {
        return EligibilityDecision::HumanGate("autonomous_mode is off".to_string());
    }
    if !has_auto_merge_label {
        return EligibilityDecision::HumanGate("issue lacks the auto-merge label".to_string());
    }
    // Stage 2 — safety preconditions. Opt-in is satisfied, so failures here are
    // NeedsHuman (the user asked for autonomy but it cannot run safely).
    if is_needs_human {
        return EligibilityDecision::NeedsHuman("already escalated to needs-human".to_string());
    }
    if attempt_count >= max_attempts {
        return EligibilityDecision::NeedsHuman(format!(
            "autonomous attempts exhausted ({attempt_count}/{max_attempts})"
        ));
    }
    if !criteria.machine_checkable {
        return EligibilityDecision::NeedsHuman(
            "no machine-checkable acceptance criteria block".to_string(),
        );
    }
    if !protection.is_verified() {
        let reason = match protection {
            gwt_git::branch_protection::BranchProtectionStatus::Unreadable(detail) => {
                format!("branch protection could not be verified (permissions): {detail}")
            }
            _ => "branch protection absent or structurally insufficient".to_string(),
        };
        return EligibilityDecision::NeedsHuman(reason);
    }
    EligibilityDecision::Eligible
}

/// SPEC #3200 T-022: lifecycle phase of one issue's current autonomous attempt.
/// Observable via the status view so every decision boundary is testable
/// (FR-033). `Idle` is the resting state between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousPhase {
    /// No attempt in flight (never launched, or reset after merge/escalation).
    #[default]
    Idle,
    /// Implementation agent launched and producing changes.
    Implementing,
    /// Implementation complete; independent review / strong gate in flight.
    Reviewing,
    /// Gate passed; Deliver is driving the PR to merge.
    Delivering,
    /// Work merged — terminal success for the autonomous path.
    Merged,
    /// Escalated to a human (bounded retries exhausted / gate-unavailable).
    NeedsHuman,
}

/// SPEC #3200 T-022/T-016/T-018: the typed container for one issue's autonomous
/// state. Single source of truth for the attempt counter (FR-026), the current
/// lifecycle phase, the launch id binding the in-flight attempt (TOCTOU /
/// stuck-detection anchor, FR-013), and the launch-time acceptance snapshot
/// (FR-014). Persisted via [`IssueMonitorPrefs`] so a daemon restart never
/// resets an in-flight attempt's counter or loses its snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIssueRecord {
    pub issue_number: u64,
    #[serde(default)]
    pub phase: AutonomousPhase,
    /// Launch / window id binding the CURRENT attempt. `None` between attempts.
    #[serde(default)]
    pub active_launch_id: Option<String>,
    /// Failed/started attempts so far (the persisted attempt counter).
    #[serde(default)]
    pub attempts: u32,
    /// Acceptance-criteria snapshot captured at launch; compared at gate time.
    #[serde(default)]
    pub acceptance_snapshot: Option<crate::issue_monitor_gate::AcceptanceSnapshot>,
    /// SPEC #3200 T-043/FR-029: earliest RFC3339 time the issue may relaunch
    /// after a transient retry was scheduled (bounded backoff). `None` ⇒ ready.
    #[serde(default)]
    pub retry_not_before: Option<String>,
    /// SPEC #3200 T-044/T-045/FR-013: RFC3339 of the last observed liveness
    /// signal from the launched agent — the anchor for stuck/idle detection.
    #[serde(default)]
    pub last_heartbeat: Option<String>,
    /// SPEC #3200: the open PR number produced by the implementation agent, set
    /// when the loop transitions Implementing→Reviewing. `None` until a PR exists.
    #[serde(default)]
    pub pr_number: Option<u64>,
    /// SPEC #3200 FR-016: the SHA the independent review evaluated and the gate is
    /// bound to (TOCTOU anchor). Set at Reviewing; checked against the merged SHA.
    #[serde(default)]
    pub reviewed_sha: Option<String>,
    /// SPEC #3200 FR-015: the independent-review verdict for `reviewed_sha`.
    /// `None` while review is in flight; `Some(true/false)` once it returns.
    #[serde(default)]
    pub review_passed: Option<bool>,
}

/// SPEC #3200 T-043/FR-029: bounded exponential backoff (seconds) for the
/// `attempt`-th transient retry. attempt 1 ⇒ `base_secs`, doubling each
/// subsequent attempt, clamped to `cap_secs`. Saturating arithmetic so large
/// attempt counts never overflow or panic on shift.
pub fn autonomous_retry_backoff_secs(attempt: u32, base_secs: u64, cap_secs: u64) -> u64 {
    let exponent = attempt.saturating_sub(1).min(32);
    let scaled = base_secs.saturating_mul(1u64 << exponent);
    scaled.min(cap_secs)
}

/// Add `secs` to an RFC3339 instant, returning the new RFC3339 string. `None`
/// when `now` is not parseable as RFC3339.
fn rfc3339_plus_secs(now: &str, secs: u64) -> Option<String> {
    // Guard the u64→i64 cast: an absurd magnitude (only possible via corrupted
    // tuning) fails closed to None rather than wrapping negative.
    let secs = i64::try_from(secs).ok()?;
    let parsed = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    let later = parsed + chrono::Duration::seconds(secs);
    Some(later.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Whole seconds elapsed from `earlier` to `now` (both RFC3339). `None` when
/// either is unparseable. Negative when `now` precedes `earlier`.
fn rfc3339_elapsed_secs(earlier: &str, now: &str) -> Option<i64> {
    let a = chrono::DateTime::parse_from_rfc3339(earlier).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(now).ok()?;
    Some((b - a).num_seconds())
}

/// SPEC #3200 Option A: a request for the GUI to spawn an independent review
/// agent. The GUI launches a fresh-session, different-model agent with a prompt
/// built from `required_criteria` + `diff`, bound to `reviewed_sha`; that agent
/// returns its verdict via the `ReviewVerdict` control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousReviewDispatch {
    pub issue_number: u64,
    pub pr_number: u64,
    pub reviewed_sha: String,
    pub required_criteria: Vec<String>,
    pub diff: String,
    /// SPEC #3200 Option A: the work branch kind, so the GUI spawns the review
    /// agent in the implementation agent's existing work-branch worktree.
    #[serde(default)]
    pub linked_issue_kind: LinkedIssueKind,
}

/// SPEC #3200 FR-034 (T-111): one operator notice for an unattended autonomous
/// lifecycle transition. Surfaced to the GUI as an `issue_monitor_toast`
/// (transient surface toast + persistent scrollable notification stack).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousNotice {
    /// Toast level: `info` | `warn` | `error` | `done`.
    pub level: String,
    pub issue_number: u64,
    pub message: String,
}

/// Bound for [`IssueMonitorState::pending_autonomous_notices`]: unattended
/// operation with no GUI connected must not grow the queue without limit.
const AUTONOMOUS_NOTICE_CAP: usize = 100;

/// SPEC #3200 T-042: how an autonomous attempt's failure should be routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Transient (launch failure / network / abnormal exit): retry with bounded
    /// backoff until the per-issue attempt counter reaches `max_attempts`.
    Transient,
    /// Terminal for autonomous resolution (independent-review rejected, criteria
    /// unsatisfiable, gate structurally unavailable): escalate to `NeedsHuman`
    /// immediately — another attempt cannot fix it.
    Terminal,
}

/// SPEC #3200 T-042: the routing outcome of dispatching an autonomous failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousFailureOutcome {
    /// Re-queued for another attempt; carries the new attempt count.
    Retry { attempt: u32 },
    /// Escalated to `NeedsHuman`; carries the human-facing reason.
    Escalated(String),
}

impl AutonomousIssueRecord {
    fn new(issue_number: u64) -> Self {
        Self {
            issue_number,
            phase: AutonomousPhase::Idle,
            active_launch_id: None,
            attempts: 0,
            acceptance_snapshot: None,
            retry_not_before: None,
            last_heartbeat: None,
            pr_number: None,
            reviewed_sha: None,
            review_passed: None,
        }
    }
}

/// SPEC #3200 FR-004: the GitHub label that, together with project-level
/// `autonomous_mode`, opts an issue into unattended autonomous resolution.
pub const AUTO_MERGE_LABEL: &str = "auto-merge";

/// Whether `issue` carries the [`AUTO_MERGE_LABEL`] (case-insensitive).
pub fn issue_has_auto_merge_label(issue: &IssueMonitorIssue) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case(AUTO_MERGE_LABEL))
}

pub fn issue_monitor_linked_issue_kind(issue: &IssueMonitorIssue) -> LinkedIssueKind {
    if has_gwt_spec_label(&issue.labels) {
        LinkedIssueKind::Spec
    } else {
        LinkedIssueKind::Issue
    }
}

pub fn issue_monitor_launch_prompt(_kind: LinkedIssueKind, number: u64) -> String {
    format!("$gwt-execute #{number}")
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

/// Per-process-unique scratch path for the atomic prefs write, placed in the
/// same directory as `path` (so the final `rename` stays on one filesystem and
/// is atomic). The daemon (`gwtd`) and GUI (`gwt`) processes both write this same
/// prefs file; a fixed `*.json.tmp` name let their concurrent writes open and
/// truncate the SAME scratch file and interleave into torn JSON, which
/// `load_issue_monitor_prefs` then silently reset to default (adversarial
/// review). Scoping the scratch name to `{pid}-{uuid}` gives every writer its own
/// file. Mirrors the gwt-core atomic-write convention.
fn unique_prefs_tmp_path(path: &Path) -> std::path::PathBuf {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("issue-monitor.json");
    parent.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

pub fn save_issue_monitor_prefs(path: &Path, prefs: &IssueMonitorPrefs) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let content = serde_json::to_string_pretty(prefs).map_err(io::Error::other)?;
    let tmp = unique_prefs_tmp_path(path);
    fs::write(&tmp, content.as_bytes())?;
    fs::rename(&tmp, path)
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
            autonomous_mode: false,
            autonomous_tuning: AutonomousTuning::default(),
            autonomous_records: BTreeMap::new(),
            failed_issues: BTreeMap::new(),
            failed_windows: BTreeMap::new(),
            queue: VecDeque::new(),
            pending_launches: VecDeque::new(),
            pending_review_dispatches: VecDeque::new(),
            pending_autonomous_notices: VecDeque::new(),
            launching_claimed_at: BTreeMap::new(),
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
        // Issue #3222: restore claimed-but-unbound launches so a reload (every
        // GUI handler) still sees the in-flight claim and cannot re-claim it.
        for entry in prefs.launching_issues {
            if !state.active_launches.contains(&entry.issue_number) {
                state.active_launches.push(entry.issue_number);
            }
            if let Some(claimed_at) = entry.claimed_at {
                state
                    .launching_claimed_at
                    .insert(entry.issue_number, claimed_at);
            }
        }
        for failed in prefs.failed_issues {
            if failed.message.trim().is_empty() {
                continue;
            }
            state
                .failed_issues
                .insert(failed.issue_number, failed.message);
            if let Some(window_id) = failed.window_id.filter(|id| !id.is_empty()) {
                state.failed_windows.insert(failed.issue_number, window_id);
            }
        }
        state.merged_issues = prefs.merged_issues.into_iter().collect();
        state.autonomous_mode = prefs.autonomous_mode;
        state.autonomous_tuning = prefs.autonomous_tuning;
        for record in prefs.autonomous_records {
            state.autonomous_records.insert(record.issue_number, record);
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
            launching_issues: self
                .active_launches
                .iter()
                .filter(|issue_number| !self.launched_windows.contains_key(issue_number))
                .map(|issue_number| IssueMonitorLaunchingIssue {
                    issue_number: *issue_number,
                    claimed_at: self.launching_claimed_at.get(issue_number).cloned(),
                })
                .collect(),
            failed_issues: self
                .failed_issues
                .iter()
                .map(|(issue_number, message)| IssueMonitorFailedIssue {
                    issue_number: *issue_number,
                    message: message.clone(),
                    window_id: self.failed_windows.get(issue_number).cloned(),
                })
                .collect(),
            merged_issues: self.merged_issues.iter().copied().collect(),
            autonomous_mode: self.autonomous_mode,
            autonomous_tuning: self.autonomous_tuning.clone(),
            autonomous_records: self.autonomous_records.values().cloned().collect(),
        }
    }

    /// SPEC #3200 T-022: read-only access to an issue's autonomous record.
    pub fn autonomous_record(&self, issue_number: u64) -> Option<&AutonomousIssueRecord> {
        self.autonomous_records.get(&issue_number)
    }

    fn autonomous_record_mut(&mut self, issue_number: u64) -> &mut AutonomousIssueRecord {
        self.autonomous_records
            .entry(issue_number)
            .or_insert_with(|| AutonomousIssueRecord::new(issue_number))
    }

    /// SPEC #3200 T-016 / FR-026: failed/started attempts recorded for an issue.
    pub fn attempt_count(&self, issue_number: u64) -> u32 {
        self.autonomous_records
            .get(&issue_number)
            .map(|record| record.attempts)
            .unwrap_or(0)
    }

    /// SPEC #3200 T-016 / FR-026: increment the per-issue attempt counter,
    /// returning the new count. Drives max-attempts escalation to `NeedsHuman`.
    pub fn record_attempt(&mut self, issue_number: u64) -> u32 {
        let record = self.autonomous_record_mut(issue_number);
        record.attempts = record.attempts.saturating_add(1);
        record.attempts
    }

    /// SPEC #3200 T-022: set the lifecycle phase of an issue's current attempt.
    pub fn set_autonomous_phase(&mut self, issue_number: u64, phase: AutonomousPhase) {
        self.autonomous_record_mut(issue_number).phase = phase;
    }

    /// SPEC #3200 T-022 / FR-013: bind (or clear) the launch id of the in-flight
    /// attempt — the anchor for stuck detection and reviewed-SHA binding.
    pub fn set_active_launch_id(&mut self, issue_number: u64, launch_id: Option<String>) {
        self.autonomous_record_mut(issue_number).active_launch_id = launch_id;
    }

    /// SPEC #3200 T-018 / FR-014: capture the launch-time acceptance snapshot,
    /// compared against the re-classified Issue body at gate time.
    pub fn capture_acceptance_snapshot(
        &mut self,
        issue_number: u64,
        snapshot: crate::issue_monitor_gate::AcceptanceSnapshot,
    ) {
        self.autonomous_record_mut(issue_number).acceptance_snapshot = Some(snapshot);
    }

    /// SPEC #3200 T-016/T-022: drop an issue's autonomous record (resets the
    /// attempt counter) once the work merges or is otherwise resolved.
    pub fn clear_autonomous_record(&mut self, issue_number: u64) {
        self.autonomous_records.remove(&issue_number);
    }

    /// SPEC #3200 T-042/T-033 / FR-026/FR-027: dispatch an autonomous attempt
    /// failure. Counts the attempt, then either re-queues for a bounded retry
    /// (transient AND still under `max_attempts`) or escalates to `NeedsHuman`
    /// (terminal failure, OR transient with attempts exhausted). The retry path
    /// frees the slot and returns the issue to `Queued` for resume — never a
    /// fabricated "done" state.
    pub fn record_autonomous_failure(
        &mut self,
        issue_number: u64,
        class: FailureClass,
        message: impl Into<String>,
        now: &str,
    ) -> AutonomousFailureOutcome {
        let message = message.into();
        let attempt = self.record_attempt(issue_number);
        let max = self.autonomous_tuning.max_attempts;
        let exhausted = attempt >= max;
        if matches!(class, FailureClass::Terminal) || exhausted {
            let reason = if matches!(class, FailureClass::Terminal) {
                format!("autonomous resolution failed terminally: {message}")
            } else {
                format!("autonomous attempts exhausted ({attempt}/{max}): {message}")
            };
            self.escalate_to_needs_human(issue_number, reason.clone());
            AutonomousFailureOutcome::Escalated(reason)
        } else {
            let backoff = autonomous_retry_backoff_secs(
                attempt,
                self.autonomous_tuning.retry_backoff_base_secs,
                self.autonomous_tuning.retry_backoff_cap_secs,
            );
            // FR-034: surface the transient retry (attempt + reason) so an
            // unattended failure loop is visible to the operator.
            self.push_autonomous_notice(
                "warn",
                issue_number,
                format!("Issue #{issue_number} attempt {attempt}/{max} failed (retry scheduled): {message}"),
            );
            self.clear_active_tracking(issue_number);
            self.set_autonomous_phase(issue_number, AutonomousPhase::Idle);
            self.set_active_launch_id(issue_number, None);
            self.autonomous_record_mut(issue_number).retry_not_before =
                rfc3339_plus_secs(now, backoff);
            self.set_inbox_state(issue_number, MonitorInboxState::Queued);
            if !self.queue.contains(&issue_number) {
                self.queue.push_back(issue_number);
                self.apply_priority_order_to_queue();
            }
            AutonomousFailureOutcome::Retry { attempt }
        }
    }

    /// SPEC #3200 T-043/FR-029: whether `issue_number` may relaunch now. `true`
    /// when no retry backoff is pending or the backoff window has elapsed. An
    /// unparseable clock fails open so a glitch never permanently blocks a retry.
    pub fn retry_ready(&self, issue_number: u64, now: &str) -> bool {
        let Some(not_before) = self
            .autonomous_records
            .get(&issue_number)
            .and_then(|record| record.retry_not_before.as_deref())
        else {
            return true;
        };
        match (
            chrono::DateTime::parse_from_rfc3339(now),
            chrono::DateTime::parse_from_rfc3339(not_before),
        ) {
            (Ok(now_t), Ok(nb_t)) => now_t >= nb_t,
            _ => true,
        }
    }

    /// SPEC #3200 T-045/FR-013: record an observed liveness signal from the
    /// launched agent for `issue_number`. Resets the stuck-detection window.
    pub fn record_autonomous_heartbeat(&mut self, issue_number: u64, now: &str) {
        self.autonomous_record_mut(issue_number).last_heartbeat = Some(now.to_string());
    }

    /// SPEC #3200 T-044/T-035/FR-013: launched autonomous issues whose agent has
    /// shown no liveness for longer than `stuck_timeout_secs`. Pipeline-in-flight
    /// phases are excluded because they self-heal without a liveness signal:
    /// `Reviewing` is resumed on a daemon restart (see
    /// [`resume_inflight_reviews_after_restart`](Self::resume_inflight_reviews_after_restart)),
    /// and `Delivering` re-polls the persisted PR for its merge commit. Terminal
    /// phases are excluded too. Issues with no heartbeat yet are conservatively
    /// NOT judged stuck (no liveness data).
    pub fn stuck_autonomous_issues(&self, now: &str) -> Vec<u64> {
        let timeout = self.autonomous_tuning.stuck_timeout_secs as i64;
        self.autonomous_records
            .values()
            .filter(|record| self.active_launches.contains(&record.issue_number))
            .filter(|record| {
                matches!(
                    record.phase,
                    AutonomousPhase::Idle | AutonomousPhase::Implementing
                )
            })
            .filter(|record| {
                record
                    .last_heartbeat
                    .as_deref()
                    .and_then(|hb| rfc3339_elapsed_secs(hb, now))
                    .is_some_and(|elapsed| elapsed >= timeout)
            })
            .map(|record| record.issue_number)
            .collect()
    }

    /// SPEC #3200 T-044/T-045/FR-013: reclaim every stuck autonomous slot,
    /// dispatching each as a transient failure (retry-with-backoff, or escalate
    /// to `NeedsHuman` when attempts are exhausted). Idempotent: a reclaimed
    /// issue is no longer launched, so a second pass finds nothing.
    pub fn recover_stuck_autonomous(&mut self, now: &str) -> Vec<(u64, AutonomousFailureOutcome)> {
        // Fail-closed gate: never mutate autonomous state when the mode is off
        // (default), so the SPEC #3165 path is untouched.
        if !self.autonomous_mode {
            return Vec::new();
        }
        self.stuck_autonomous_issues(now)
            .into_iter()
            .map(|issue_number| {
                let outcome = self.record_autonomous_failure(
                    issue_number,
                    FailureClass::Transient,
                    "stuck/idle timeout: agent made no progress within stuck_timeout_secs",
                    now,
                );
                (issue_number, outcome)
            })
            .collect()
    }

    /// SPEC #3200 (review follow-up): restore self-healing for a `Reviewing`
    /// record whose review-agent dispatch was lost across a daemon restart.
    ///
    /// The review-agent spawn request (`pending_review_dispatches`) is NOT
    /// persisted, but the record's phase IS. A record reloaded in `Reviewing`
    /// therefore waits forever for a verdict that no agent will produce (the
    /// review agent was never re-spawned) — `advance_autonomous_in_flight`'s
    /// `Reviewing` branch only waits, and the `Implementing` branch (which
    /// re-detects the open PR and re-issues `begin_review` + the review dispatch)
    /// is never reached. Resetting the phase to `Implementing` restores exactly
    /// the pre-persist self-healing (a restart used to revert the in-memory
    /// phase): the next scan rebuilds the launch plan, re-detects the PR, and
    /// re-dispatches the review, binding to the current head SHA.
    ///
    /// `Delivering` is intentionally left untouched: its watch loop polls the
    /// persisted `pr_number` for the merge commit, so it self-heals across a
    /// restart on its own — and its GitHub auto-merge is already armed, so
    /// re-driving it would double-work and could invalidate the armed merge.
    ///
    /// The resumed record's `last_heartbeat` is refreshed to `now`: a restart is
    /// not a failed attempt, but the reset to `Implementing` makes the record
    /// eligible for stuck/idle detection (`stuck_autonomous_issues` covers
    /// `Idle | Implementing`), which runs BEFORE the re-dispatch on the next scan.
    /// Without the refresh, a persisted stale `last_heartbeat` (e.g. a review that
    /// ran longer than `stuck_timeout_secs` before the restart) would trip
    /// `recover_stuck_autonomous` and wrongly count a failed attempt / backoff
    /// (or escalate to `NeedsHuman` at the cap) before the review is even
    /// re-issued. The fresh stamp gives the resumed record a full window to reach
    /// `Reviewing` again.
    ///
    /// Idempotent and safe to call once right after loading persisted prefs; it
    /// only touches records already parked in `Reviewing`.
    pub fn resume_inflight_reviews_after_restart(&mut self, now: &str) -> Vec<u64> {
        let mut resumed = Vec::new();
        for record in self.autonomous_records.values_mut() {
            if record.phase == AutonomousPhase::Reviewing {
                record.phase = AutonomousPhase::Implementing;
                record.review_passed = None;
                record.last_heartbeat = Some(now.to_string());
                resumed.push(record.issue_number);
            }
        }
        resumed
    }

    /// SPEC #3200 FR-027: escalate an issue to the terminal `NeedsHuman` state —
    /// frees the slot, records the reason, marks the autonomous phase, and never
    /// auto-relaunches. Reused by the strong-gate path when review rejects.
    pub fn escalate_to_needs_human(&mut self, issue_number: u64, reason: impl Into<String>) {
        let reason = reason.into();
        // FR-034: an unattended escalation is exactly what the operator must see.
        self.push_autonomous_notice(
            "error",
            issue_number,
            format!("Issue #{issue_number} needs human: {reason}"),
        );
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.set_autonomous_phase(issue_number, AutonomousPhase::NeedsHuman);
        self.set_active_launch_id(issue_number, None);
        self.failed_issues.insert(issue_number, reason.clone());
        self.last_error = Some(format!("issue #{issue_number}: {reason}"));
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

    /// Issue #3222: refresh the GUI-owned prefs fields (`launch_profile`,
    /// `autonomous_tuning`) from a freshly-loaded on-disk snapshot. The daemon
    /// loads prefs once at startup and has no control frame for these fields, so
    /// without this refresh a profile the GUI saves later stays invisible
    /// (`has_launch_profile()==false` ⇒ active cap 0 ⇒ the daemon never refills
    /// slots and cannot act as the single launch driver).
    pub fn refresh_gui_owned_prefs(&mut self, disk: &IssueMonitorPrefs) {
        self.launch_profile = disk.launch_profile.clone();
        self.autonomous_tuning = disk.autonomous_tuning.clone();
    }

    /// #3223 follow-up (codex P1): absorb the OTHER process's in-flight launch
    /// accounting from disk. The GUI and the daemon both claim launches; the
    /// daemon only refreshed profile/tuning, so GUI-written `launching`/
    /// `launched` entries were invisible to it — it saw free slots (over-cap
    /// claims) and its next persist dropped the GUI's in-flight claims.
    /// Union-merge: entries already known in memory win; removals propagate via
    /// the existing control frames (Launched / LaunchFailed / WindowClosed).
    pub fn merge_inflight_launches_from_disk(&mut self, disk: &IssueMonitorPrefs) {
        for launched in &disk.launched_issues {
            if launched.window_id.is_empty() {
                continue;
            }
            self.launched_windows
                .entry(launched.issue_number)
                .or_insert_with(|| launched.window_id.clone());
            if !self.active_launches.contains(&launched.issue_number) {
                self.active_launches.push(launched.issue_number);
            }
        }
        for entry in &disk.launching_issues {
            if !self.active_launches.contains(&entry.issue_number) {
                self.active_launches.push(entry.issue_number);
                if let Some(claimed_at) = &entry.claimed_at {
                    self.launching_claimed_at
                        .insert(entry.issue_number, claimed_at.clone());
                }
            }
        }
    }

    /// #3223 follow-up (codex P2 / coderabbit): release claimed-but-unbound
    /// launches whose claim anchor is older than `claim_ttl_secs`. A crash
    /// between the claim-save and the launch ACK would otherwise hold a
    /// max-active slot forever. Entries restored without an anchor (legacy
    /// bare-id shape) are stamped `now` so their clock starts here. Released
    /// issues return to `Queued` and re-enter the queue so the next scan can
    /// relaunch them (mirroring the expired GitHub claim, which lapses after
    /// the same TTL).
    pub fn expire_stale_unbound_launches(&mut self, now: &str) -> Vec<u64> {
        let ttl = self.config.claim_ttl_secs as i64;
        let unbound: Vec<u64> = self
            .active_launches
            .iter()
            .filter(|issue_number| !self.launched_windows.contains_key(issue_number))
            .copied()
            .collect();
        let mut expired = Vec::new();
        for issue_number in unbound {
            match self.launching_claimed_at.get(&issue_number) {
                Some(claimed_at) => {
                    let stale =
                        rfc3339_elapsed_secs(claimed_at, now).is_some_and(|elapsed| elapsed >= ttl);
                    if stale {
                        self.active_launches
                            .retain(|active| *active != issue_number);
                        self.launching_claimed_at.remove(&issue_number);
                        self.set_inbox_state(issue_number, MonitorInboxState::Queued);
                        if !self.queue.contains(&issue_number) {
                            self.queue.push_back(issue_number);
                            self.apply_priority_order_to_queue();
                        }
                        expired.push(issue_number);
                    }
                }
                None => {
                    // Legacy entry without an anchor: start its clock now.
                    self.launching_claimed_at
                        .insert(issue_number, now.to_string());
                }
            }
        }
        expired
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
                .unwrap_or_else(|| "configure before auto start".to_string()),
            autonomous_mode: self.autonomous_mode,
            autonomous_issues: self
                .autonomous_records
                .values()
                .map(|record| AutonomousIssueSummary {
                    issue_number: record.issue_number,
                    phase: record.phase,
                    attempts: record.attempts,
                    needs_human: record.phase == AutonomousPhase::NeedsHuman,
                })
                .collect(),
        }
    }

    /// SPEC #3200 T-001/FR-001: read the opt-in autonomous mode flag.
    pub fn autonomous_mode(&self) -> bool {
        self.autonomous_mode
    }

    /// SPEC #3200 T-047/FR-001: toggle unattended autonomous mode. Default OFF
    /// keeps the SPEC #3165 human-gated behavior exactly.
    pub fn set_autonomous_mode(&mut self, enabled: bool) {
        self.autonomous_mode = enabled;
    }

    /// SPEC #3200 T-032/FR-003/004: the pure two-stage opt-in pre-gate — an issue
    /// is an autonomous candidate ONLY when autonomous mode is on AND the issue
    /// carries the `auto-merge` label. Branch-protection / acceptance-criteria /
    /// attempt safety preconditions are applied later by [`autonomous_eligibility`].
    pub fn is_autonomous_two_stage_candidate(&self, issue: &IssueMonitorIssue) -> bool {
        self.autonomous_mode && issue_has_auto_merge_label(issue)
    }

    /// #3165/#3200 error-window lifecycle: decide whether a just-failed agent
    /// window should be auto-closed. An autonomous (two-stage opt-in) issue
    /// auto-closes its stale window so the bounded retry relaunches into a clean
    /// canvas; a default (non-autonomous) issue KEEPS its failed window so the
    /// human can inspect the error output and relaunch explicitly via Launch Now.
    /// The issue is looked up in the inbox, where a freshly recorded failure
    /// still carries the issue and its labels.
    pub fn should_autoclose_failed_window(&self, issue_number: u64) -> bool {
        self.inbox
            .iter()
            .find(|item| item.issue.number == issue_number)
            .map(|item| self.is_autonomous_two_stage_candidate(&item.issue))
            .unwrap_or(false)
    }

    /// #3165 error-window lifecycle: remove and return the stale agent window id
    /// retained for a failed issue, so an explicit Launch Now (default mode) can
    /// close it before relaunching into a fresh window. `None` when no stale
    /// window was recorded for the issue.
    pub fn take_failed_window(&mut self, issue_number: u64) -> Option<String> {
        self.failed_windows.remove(&issue_number)
    }

    /// SPEC #3200 T-041 (FR-003..FR-010): pre-launch autonomous decision + state
    /// capture for one candidate, given the freshly fetched base-branch
    /// `branch_protection`. Composes the pure [`autonomous_eligibility`] predicate
    /// with the issue body's acceptance criteria and the persisted attempt count,
    /// then applies the side effects:
    ///
    /// - non-two-stage candidate ⇒ `HumanGate` (caller uses the existing #3165
    ///   human-gated launch path, no autonomous state created);
    /// - `NeedsHuman` ⇒ escalate (terminal, removed from the launch queue);
    /// - `Eligible` ⇒ capture the acceptance snapshot + set `Implementing` phase
    ///   (idempotent: only on a fresh, not-yet-launched candidate).
    ///
    /// Returns the [`EligibilityDecision`] so the caller knows whether to launch.
    /// Default `autonomous_mode` OFF makes this a no-op `HumanGate` for every
    /// issue, preserving SPEC #3165 behavior exactly.
    pub fn prepare_autonomous_candidate(
        &mut self,
        issue: &IssueMonitorIssue,
        branch_protection: &gwt_git::branch_protection::BranchProtectionStatus,
        now: &str,
    ) -> EligibilityDecision {
        if !self.is_autonomous_two_stage_candidate(issue) {
            return EligibilityDecision::HumanGate("not an autonomous candidate".to_string());
        }
        let number = issue.number;
        // Idempotency: a candidate already in flight is left alone.
        if self.active_launches.contains(&number) {
            return EligibilityDecision::Eligible;
        }
        // SPEC #3200 T-043/FR-029: honor the transient-retry backoff — a candidate
        // whose backoff window has not elapsed is skipped this scan (no capture,
        // no escalation) so the exponential backoff is actually enforced.
        if !self.retry_ready(number, now) {
            return EligibilityDecision::HumanGate("retry backoff window not elapsed".to_string());
        }
        let criteria = crate::issue_monitor_gate::classify_acceptance_criteria(
            issue.body.as_deref().unwrap_or(""),
        );
        let attempt_count = self.attempt_count(number);
        let is_needs_human = self
            .autonomous_record(number)
            .map(|record| record.phase == AutonomousPhase::NeedsHuman)
            .unwrap_or(false);
        let decision = autonomous_eligibility(
            self.autonomous_mode,
            issue_has_auto_merge_label(issue),
            &criteria,
            branch_protection,
            is_needs_human,
            attempt_count,
            self.autonomous_tuning.max_attempts,
        );
        match &decision {
            EligibilityDecision::Eligible => {
                self.capture_acceptance_snapshot(number, criteria.snapshot());
                self.set_autonomous_phase(number, AutonomousPhase::Implementing);
                // The launch consumes the scheduled retry, so the backoff marker
                // is cleared to avoid stale state on the in-flight attempt.
                self.autonomous_record_mut(number).retry_not_before = None;
                // SPEC #3200 T-045/FR-025: seed the liveness baseline at launch so
                // stuck detection actually fires for an agent that hangs without
                // producing a PR within stuck_timeout_secs. Real progress (a
                // heartbeat, or the Implementing→Reviewing transition) resets it.
                self.record_autonomous_heartbeat(number, now);
            }
            EligibilityDecision::NeedsHuman(reason) => {
                self.escalate_to_needs_human(number, reason.clone());
            }
            EligibilityDecision::HumanGate(_) => {}
        }
        decision
    }

    /// SPEC #3200: autonomous issues currently in flight (phase Implementing /
    /// Reviewing / Delivering) — the set the daemon orchestration loop advances
    /// each tick. Terminal/Idle phases are excluded.
    pub fn autonomous_in_flight_issues(&self) -> Vec<u64> {
        self.autonomous_records
            .values()
            .filter(|record| Self::is_in_flight_phase(record.phase))
            .map(|record| record.issue_number)
            .collect()
    }

    fn is_in_flight_phase(phase: AutonomousPhase) -> bool {
        matches!(
            phase,
            AutonomousPhase::Implementing
                | AutonomousPhase::Reviewing
                | AutonomousPhase::Delivering
        )
    }

    /// SPEC #3200 (review follow-up): true when `issue_number` has an autonomous
    /// record actively in flight (`Implementing` / `Reviewing` / `Delivering`).
    /// A launch/agent failure for such an issue must be routed through the
    /// autonomous retry/backoff/escalation machinery rather than the plain
    /// human-gated launch-failed path, or the record strands in a non-`Idle`
    /// phase forever (e.g. `Reviewing` after a failed review-agent spawn, where
    /// the daemon waits for a verdict that will never arrive).
    pub fn is_autonomous_in_flight(&self, issue_number: u64) -> bool {
        self.autonomous_records
            .get(&issue_number)
            .is_some_and(|record| Self::is_in_flight_phase(record.phase))
    }

    /// SPEC #3200: transition Implementing→Reviewing once the implementation
    /// agent has produced an open PR. Binds the PR number and the reviewed SHA
    /// (the TOCTOU anchor) and clears any prior verdict.
    pub fn begin_review(
        &mut self,
        issue_number: u64,
        pr_number: u64,
        reviewed_sha: impl Into<String>,
    ) {
        let record = self.autonomous_record_mut(issue_number);
        record.phase = AutonomousPhase::Reviewing;
        record.pr_number = Some(pr_number);
        record.reviewed_sha = Some(reviewed_sha.into());
        record.review_passed = None;
    }

    /// SPEC #3200 FR-015: record the independent-review verdict for the in-flight
    /// reviewed SHA. The gate is evaluated on the next tick.
    pub fn record_review_verdict(&mut self, issue_number: u64, passed: bool) {
        self.autonomous_record_mut(issue_number).review_passed = Some(passed);
    }

    /// SPEC #3200 FR-015/FR-016: apply a raw review verdict reported by the
    /// (untrusted) review agent. The verdict is parsed and judged HERE (the
    /// trusted daemon), not by the agent — and only accepted when its
    /// `reviewed_sha` matches the SHA this issue is actually under review for
    /// (a stale / wrong-SHA verdict is rejected). Returns `None` when rejected
    /// (no record / SHA mismatch), else `Some(passed)`.
    pub fn apply_review_verdict(
        &mut self,
        issue_number: u64,
        reviewed_sha: &str,
        verdict_raw: &str,
    ) -> Option<bool> {
        let record = self.autonomous_records.get(&issue_number)?;
        // Reject a verdict that is not for the SHA we are reviewing.
        if record.reviewed_sha.as_deref() != Some(reviewed_sha) {
            return None;
        }
        let required = record
            .acceptance_snapshot
            .as_ref()
            .map(|snapshot| snapshot.ids.clone())
            .unwrap_or_default();
        let outcome = crate::issue_monitor_review::evaluate_review_verdict(verdict_raw, &required);
        let passed = matches!(
            outcome,
            crate::issue_monitor_review::ReviewGateOutcome::Pass
        );
        self.record_review_verdict(issue_number, passed);
        Some(passed)
    }

    /// SPEC #3200: transition Reviewing→Delivering once the strong gate passes
    /// (the auto-merge is being armed).
    pub fn begin_delivering(&mut self, issue_number: u64) {
        self.set_autonomous_phase(issue_number, AutonomousPhase::Delivering);
    }

    /// SPEC #3200 FR-034 (codex #3217 review): announce a SUCCESSFUL auto-merge
    /// arm. Called by the worker only after `gh pr merge --auto` actually
    /// succeeded — never before — so the operator toast cannot claim an arm
    /// that failed (the merge helper's fail-closed contract).
    pub fn record_auto_merge_armed(&mut self, issue_number: u64) {
        self.push_autonomous_notice(
            "info",
            issue_number,
            format!("Issue #{issue_number} gate passed — auto-merge armed"),
        );
    }

    /// SPEC #3200 FR-009..FR-016: assemble the strong-gate inputs for an issue
    /// under review, from the record (reviewed SHA + review verdict + acceptance
    /// snapshot) and freshly-fetched signals (branch protection, CI rollup JSON,
    /// the current HEAD SHA, the current Issue body). Returns `None` when the
    /// review verdict has not yet arrived (gate not ready ⇒ caller waits).
    pub fn autonomous_gate_inputs(
        &self,
        issue_number: u64,
        branch_protection: gwt_git::branch_protection::BranchProtectionStatus,
        ci_rollup_json: &str,
        current_head_sha: &str,
        current_issue_body: &str,
    ) -> Option<crate::issue_monitor_gate::AutonomousGateInputs> {
        use crate::issue_monitor_gate::{classify_acceptance_criteria, classify_ci_rollup};
        use crate::issue_monitor_review::ReviewGateOutcome;
        let record = self.autonomous_records.get(&issue_number)?;
        let reviewed_sha = record.reviewed_sha.clone()?;
        // Review must have returned a verdict; otherwise the gate is not ready.
        let review_passed = record.review_passed?;
        let required_checks = match &branch_protection {
            gwt_git::branch_protection::BranchProtectionStatus::Verified { required_checks } => {
                required_checks.clone()
            }
            _ => Vec::new(),
        };
        let ci = classify_ci_rollup(ci_rollup_json, &required_checks);
        let acceptance_unchanged = record
            .acceptance_snapshot
            .as_ref()
            .map(|snapshot| snapshot.matches(&classify_acceptance_criteria(current_issue_body)))
            .unwrap_or(false);
        let review = if review_passed {
            ReviewGateOutcome::Pass
        } else {
            ReviewGateOutcome::Fail("independent review rejected".to_string())
        };
        Some(crate::issue_monitor_gate::AutonomousGateInputs {
            branch_protection,
            ci,
            review,
            acceptance_unchanged,
            reviewed_sha,
            head_sha: current_head_sha.to_string(),
        })
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
        } else if self.active_launches.contains(&issue_number) {
            // Issue #3222: a claimed launch whose window is not bound yet stays
            // visibly in-flight; the queue-push guard below then skips it.
            MonitorInboxState::Launching
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

    pub fn next_launch_request(&mut self, now: &str) -> Option<IssueMonitorLaunchRequest> {
        let max_active = self.config.max_active.max(1);
        if !self.gui_connected || self.active_launches.len() >= max_active {
            return None;
        }
        let issue_number = self.queue.pop_front()?;
        if !self.active_launches.contains(&issue_number) {
            self.active_launches.push(issue_number);
        }
        // #3223 follow-up: anchor the claim so a restored-but-never-acked
        // launch can expire after claim_ttl_secs instead of leaking the slot.
        self.launching_claimed_at
            .insert(issue_number, now.to_string());
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
        self.claim_next_launch_requests_with_probe(client, owner, now, active_cap, |_| false)
    }

    /// Issue #3225: claim queued candidates, skipping issues whose fix is
    /// already completed. `completed_probe` answers "does this issue have a
    /// merged linked PR?" from GitHub — the instance-local `merged_issues`
    /// memory is not enough because a fresh monitor (new machine / isolated
    /// HOME / wiped prefs) would otherwise re-launch already-finished work
    /// that stays open until release. Positives are recorded `Merged`
    /// (persisted) and the slot goes to the next queued candidate. The probe
    /// fails open: an error/false keeps the issue launchable.
    pub fn claim_next_launch_requests_with_probe<C: IssueClient>(
        &mut self,
        client: &C,
        owner: &str,
        now: &str,
        active_cap: usize,
        completed_probe: impl Fn(u64) -> bool,
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
            if completed_probe(issue.number) {
                tracing::info!(
                    issue = issue.number,
                    "issue monitor: skipping candidate — a linked PR is already merged"
                );
                self.record_merged(issue.number);
                continue;
            }
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
                    if let Some(request) = self.next_launch_request(now) {
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

    /// SPEC #3200 Option A: queue a review-agent spawn request (orchestration
    /// loop → GUI). Deduped on issue number so repeated ticks don't pile up.
    pub fn push_review_dispatch(&mut self, dispatch: AutonomousReviewDispatch) {
        self.pending_review_dispatches
            .retain(|pending| pending.issue_number != dispatch.issue_number);
        self.pending_review_dispatches.push_back(dispatch);
    }

    /// Drain queued review-agent spawn requests for emission to the GUI.
    pub fn take_pending_review_dispatches(&mut self) -> Vec<AutonomousReviewDispatch> {
        self.pending_review_dispatches.drain(..).collect()
    }

    /// SPEC #3200 FR-034 (T-111): queue an operator notice for an unattended
    /// autonomous transition. Fail-closed: a no-op unless autonomous mode is on,
    /// so the default-OFF human-gated flow (#3165) emits nothing extra. Bounded:
    /// oldest notices are dropped past [`AUTONOMOUS_NOTICE_CAP`] so a
    /// disconnected-GUI window never grows the queue without limit.
    fn push_autonomous_notice(
        &mut self,
        level: &str,
        issue_number: u64,
        message: impl Into<String>,
    ) {
        if !self.autonomous_mode {
            return;
        }
        while self.pending_autonomous_notices.len() >= AUTONOMOUS_NOTICE_CAP {
            self.pending_autonomous_notices.pop_front();
        }
        self.pending_autonomous_notices.push_back(AutonomousNotice {
            level: level.to_string(),
            issue_number,
            message: message.into(),
        });
    }

    /// Drain queued autonomous operator notices for emission as `toast`
    /// payloads. Call only when a GUI is connected so unattended-window notices
    /// are retained until someone can see them.
    pub fn take_autonomous_notices(&mut self) -> Vec<AutonomousNotice> {
        self.pending_autonomous_notices.drain(..).collect()
    }

    /// Queue an operator notice that must surface even though autonomous mode is
    /// already OFF — the kill-switch disarm results. Bypasses the fail-closed
    /// mode gate deliberately: these notices are feedback ABOUT turning the mode
    /// off, so gating them on the mode would silence exactly the events the
    /// operator just asked for.
    fn push_kill_switch_notice(&mut self, level: &str, issue_number: u64, message: String) {
        while self.pending_autonomous_notices.len() >= AUTONOMOUS_NOTICE_CAP {
            self.pending_autonomous_notices.pop_front();
        }
        self.pending_autonomous_notices.push_back(AutonomousNotice {
            level: level.to_string(),
            issue_number,
            message,
        });
    }

    /// SPEC #3200 kill switch (codex #3217/#3219 review): with autonomous mode
    /// OFF, every record still in `Delivering` has an armed GitHub auto-merge
    /// that must be ACTIVELY cancelled (`gh pr merge --disable-auto`), not just
    /// abandoned locally. Returns the `(issue_number, pr_number)` pairs to
    /// disarm WITHOUT mutating any record: a record leaves `Delivering` only
    /// after the disarm actually SUCCEEDS
    /// ([`record_kill_switch_disarm_result`](Self::record_kill_switch_disarm_result)),
    /// so a transient `gh` failure keeps it targeted and the next scan retries.
    /// A failed disarm must never strand a live armed auto-merge behind a
    /// NeedsHuman screen. No-op while the mode is ON.
    pub fn kill_switch_disarm_targets(&self) -> Vec<(u64, u64)> {
        if self.autonomous_mode {
            return Vec::new();
        }
        self.autonomous_records
            .values()
            .filter(|record| record.phase == AutonomousPhase::Delivering)
            .filter_map(|record| record.pr_number.map(|pr| (record.issue_number, pr)))
            .collect()
    }

    /// Record the outcome of a kill-switch auto-merge disarm attempt.
    ///
    /// - **Success**: the delivery is halted for good — escalate to `NeedsHuman`
    ///   (visible, never silently resumed) and emit a warn notice.
    /// - **Failure**: emit an error notice but LEAVE the record in `Delivering`
    ///   so [`kill_switch_disarm_targets`](Self::kill_switch_disarm_targets)
    ///   returns it again and the next scan retries the disarm (codex #3219: a
    ///   failed disarm must stay retryable while the remote auto-merge is live).
    ///
    /// Notices are ungated: the mode is OFF by definition here, and these are
    /// the operator's feedback for turning it off.
    pub fn record_kill_switch_disarm_result(
        &mut self,
        issue_number: u64,
        pr_number: u64,
        disarmed: bool,
    ) {
        if disarmed {
            self.escalate_to_needs_human(
                issue_number,
                "autonomous mode disabled — delivery halted; auto-merge disarmed",
            );
            self.push_kill_switch_notice(
                "warn",
                issue_number,
                format!(
                    "Issue #{issue_number}: auto-merge on PR #{pr_number} disarmed (kill switch)"
                ),
            );
        } else {
            self.push_kill_switch_notice(
                "error",
                issue_number,
                format!(
                    "Issue #{issue_number}: failed to disarm auto-merge on PR #{pr_number} — still armed on GitHub; will retry next scan"
                ),
            );
        }
    }

    pub fn complete_active_launch(&mut self, issue_number: u64, window_id: impl Into<String>) {
        let window_id = window_id.into();
        self.launching_claimed_at.remove(&issue_number);
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
        self.failed_windows.remove(&issue_number);
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
        self.launching_claimed_at.remove(&issue_number);
        self.launched_windows.remove(&issue_number);
        self.launched_branches.remove(&issue_number);
        // #3165 error-window lifecycle: terminal transitions (merged / released /
        // needs-human) and retry all funnel through here; drop any retained stale
        // failed-window id so it never orphans (and never persists into prefs)
        // when the issue ends without an explicit Launch Now relaunch.
        self.failed_windows.remove(&issue_number);
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
        // FR-034: notify the operator when an issue that went through the
        // autonomous loop completes (checked BEFORE the record is cleared).
        if self.autonomous_records.contains_key(&issue_number) {
            self.push_autonomous_notice(
                "done",
                issue_number,
                format!("Issue #{issue_number} merged autonomously"),
            );
        }
        self.clear_active_tracking(issue_number);
        self.queue.retain(|queued| *queued != issue_number);
        self.merged_issues.insert(issue_number);
        self.set_inbox_state(issue_number, MonitorInboxState::Merged);
        // SPEC #3200 T-022: completion resets the autonomous lifecycle (attempts,
        // phase, snapshot, in-flight launch id) so a future reopen starts clean.
        self.clear_autonomous_record(issue_number);
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
        // SPEC #3200 (review follow-up): a failure for an in-flight autonomous
        // issue (e.g. the independent review agent could not spawn, leaving the
        // record in `Reviewing`) must funnel through the autonomous
        // retry/backoff/escalation machinery — otherwise the record strands in a
        // non-`Idle` phase forever and the daemon waits for a verdict that will
        // never arrive. The plain human-gated `LaunchFailed`/`AgentFailed` path
        // below is preserved for every non-autonomous issue.
        if self.autonomous_mode && self.is_autonomous_in_flight(issue_number) {
            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            self.record_autonomous_failure(issue_number, FailureClass::Transient, message, &now);
            return;
        }
        self.active_launches
            .retain(|active| *active != issue_number);
        // #3165 error-window lifecycle: retain the stale agent window id so an
        // explicit Launch Now can close it before relaunching. Prefer the
        // tracked launched window; fall back to the inbox item's window id.
        let stale_window = self.launched_windows.remove(&issue_number).or_else(|| {
            self.inbox
                .iter()
                .find(|item| item.issue.number == issue_number)
                .and_then(|item| item.launched_window_id.clone())
        });
        if let Some(window_id) = stale_window {
            self.failed_windows.insert(issue_number, window_id);
        }
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
    fn launch_plan_uses_unified_execute_prompt_and_work_issue_branch() {
        let mut spec_issue = issue(3164);
        spec_issue.labels.push("gwt-spec".to_string());
        let spec_plan = issue_monitor_launch_plan(&spec_issue);

        assert_eq!(spec_plan.branch_name, "work/issue-3164");
        assert_eq!(spec_plan.prompt, "$gwt-execute #3164");
        assert_eq!(spec_plan.linked_issue_kind, LinkedIssueKind::Spec);

        let plain_plan = issue_monitor_launch_plan(&issue(42));
        assert_eq!(plain_plan.branch_name, "work/issue-42");
        assert_eq!(plain_plan.prompt, "$gwt-execute #42");
        assert_eq!(plain_plan.linked_issue_kind, LinkedIssueKind::Issue);
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
            monitor
                .next_launch_request("2026-07-02T00:00:00Z")
                .is_none(),
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
    fn launching_claims_survive_prefs_roundtrip_and_are_not_reclaimed() {
        // Issue #3222: a claimed-but-not-yet-acked launch (Launching, no window
        // yet) must survive the prefs roundtrip. The GUI rebuilds the monitor
        // from disk on every handler call, so an unpersisted claim was invisible
        // to the next handler / the launch ACK's rescan, which re-claimed the
        // same issue (same-owner renewal) and spawned a DUPLICATE agent window
        // (observed live: Max 5 ⇒ 10 windows).
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-02T00:00:00Z");
        monitor.set_gui_connected(true);
        let request = monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .expect("claimed for launch");
        assert_eq!(request.issue_number, 42);
        assert_eq!(monitor.active_count(), 1, "claim holds an active slot");

        // Reload from prefs (what every GUI handler does).
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        restored.set_gui_connected(true);
        assert_eq!(
            restored.active_count(),
            1,
            "in-flight Launching claim survives the roundtrip"
        );
        // A rescan must not re-queue the in-flight issue…
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-07-02T00:00:30Z");
        assert_eq!(
            restored.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Launching),
            "rescan shows the in-flight claim as Launching, not Queued"
        );
        assert_eq!(restored.queue_len(), 0, "not re-queued");
        // …and must not hand out a second launch request for it.
        assert!(
            restored
                .next_launch_request("2026-07-02T00:00:00Z")
                .is_none(),
            "no duplicate launch request for an in-flight claim"
        );
    }

    #[test]
    fn launching_prefs_accept_legacy_bare_ids_and_timestamped_entries() {
        // #3223 follow-up (codex P2): the launching entries gained a
        // `claimed_at` anchor. Files written by the first shipped shape (bare
        // ids) must still parse — a parse failure would `unwrap_or_default()`
        // into a FULL prefs wipe.
        let legacy = r#"{"enabled":true,"max_active_agents":2,"priority_order":[],"launching_issues":[42,43]}"#;
        let prefs: IssueMonitorPrefs = serde_json::from_str(legacy).expect("legacy parses");
        assert_eq!(
            prefs
                .launching_issues
                .iter()
                .map(|entry| entry.issue_number)
                .collect::<Vec<_>>(),
            vec![42, 43]
        );
        let timed = r#"{"enabled":true,"max_active_agents":2,"priority_order":[],"launching_issues":[{"issue_number":7,"claimed_at":"2026-07-02T00:00:00Z"}]}"#;
        let prefs: IssueMonitorPrefs = serde_json::from_str(timed).expect("timed parses");
        assert_eq!(prefs.launching_issues[0].issue_number, 7);
        assert_eq!(
            prefs.launching_issues[0].claimed_at.as_deref(),
            Some("2026-07-02T00:00:00Z")
        );
    }

    #[test]
    fn stale_unbound_launching_claims_expire_after_claim_ttl() {
        // #3223 follow-up (codex P2 / coderabbit): a crash between the
        // claim-save and the launch ACK leaves a restored `Launching` claim
        // with no window. Without an expiry it holds a max-active slot
        // forever. After claim_ttl_secs it must be released so the next scan
        // can re-queue and relaunch the issue.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-02T00:00:00Z");
        monitor.set_gui_connected(true);
        assert!(monitor
            .next_launch_request("2026-07-02T00:00:00Z")
            .is_some());
        assert_eq!(monitor.active_count(), 1);

        // Restart (roundtrip) mid-launch: claim restored, still unbound.
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        restored.set_gui_connected(true);
        assert_eq!(restored.active_count(), 1);

        // Before the TTL: retained.
        let expired = restored.expire_stale_unbound_launches("2026-07-02T00:10:00Z");
        assert!(expired.is_empty(), "not expired before claim_ttl_secs");
        assert_eq!(restored.active_count(), 1);

        // After the TTL (default 1800s): released and re-queueable.
        let expired = restored.expire_stale_unbound_launches("2026-07-02T00:31:00Z");
        assert_eq!(expired, vec![42], "stale unbound claim expires");
        assert_eq!(restored.active_count(), 0, "slot released");
        scan_issue_monitor_candidates(&mut restored, &[issue(42)], "2026-07-02T00:31:10Z");
        assert!(
            restored
                .next_launch_request("2026-07-02T00:31:20Z")
                .is_some(),
            "the issue is claimable again after expiry"
        );
        // Bound launches never expire this way.
        monitor.complete_active_launch(42, "tab-1::agent-1");
        assert!(monitor
            .expire_stale_unbound_launches("2026-07-03T00:00:00Z")
            .is_empty());
    }

    #[test]
    fn merge_inflight_launches_from_disk_unifies_cross_process_accounting() {
        // #3223 follow-up (codex P1): the daemon only refreshed profile/tuning
        // from disk, so GUI-written launching/launched claims were invisible —
        // the daemon saw free slots (over-cap claims) and its next persist
        // dropped the GUI's in-flight entries. The merge must absorb both.
        let mut daemon = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                max_active: 2,
                ..IssueMonitorConfig::default()
            },
            IssueMonitorPrefs {
                enabled: true,
                max_active_agents: 2,
                ..IssueMonitorPrefs::default()
            },
        );
        let disk = IssueMonitorPrefs {
            launching_issues: vec![IssueMonitorLaunchingIssue {
                issue_number: 42,
                claimed_at: Some("2026-07-02T00:00:00Z".to_string()),
            }],
            launched_issues: vec![IssueMonitorLaunchedIssue {
                issue_number: 43,
                window_id: "tab-1::agent-2".to_string(),
            }],
            ..IssueMonitorPrefs::default()
        };
        daemon.merge_inflight_launches_from_disk(&disk);
        assert_eq!(daemon.active_count(), 2, "both in-flight claims absorbed");
        assert!(daemon.launched_window_issue("tab-1::agent-2").is_some());
        // The daemon persist now round-trips them instead of dropping them.
        let prefs = daemon.prefs();
        assert!(prefs
            .launching_issues
            .iter()
            .any(|entry| entry.issue_number == 42
                && entry.claimed_at.as_deref() == Some("2026-07-02T00:00:00Z")));
        assert!(prefs
            .launched_issues
            .iter()
            .any(|entry| entry.issue_number == 43));
    }

    #[test]
    fn refresh_gui_owned_prefs_updates_profile_and_tuning_from_disk() {
        // Issue #3222: the daemon loads prefs once at startup, so a launch
        // profile the GUI saves later stays invisible (has_launch_profile=false
        // ⇒ cap 0 ⇒ the daemon never refills slots and the GUI's re-entrant
        // scan became the de-facto driver). The scan must refresh GUI-owned
        // fields from disk.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(!monitor.has_launch_profile());
        let disk = IssueMonitorPrefs {
            launch_profile: Some(IssueMonitorLaunchProfile {
                agent_id: "claude".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: Default::default(),
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: Default::default(),
                docker_service: None,
                docker_lifecycle_intent: Default::default(),
                windows_shell: None,
            }),
            autonomous_tuning: AutonomousTuning {
                max_attempts: 9,
                ..AutonomousTuning::default()
            },
            ..IssueMonitorPrefs::default()
        };
        monitor.refresh_gui_owned_prefs(&disk);
        assert!(monitor.has_launch_profile(), "profile refreshed from disk");
        assert_eq!(monitor.autonomous_tuning.max_attempts, 9);
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
    fn autonomous_mode_defaults_false_and_back_compat_deserializes() {
        // SPEC #3200 FR-001/FR-030, Sc 23: pre-autonomous prefs (no
        // autonomous_mode / tuning fields) deserialize with documented defaults
        // and existing fields are preserved.
        let legacy = r#"{"enabled":true,"max_active_agents":1,"priority_order":[101,102],"merged_issues":[42]}"#;
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(legacy).expect("legacy prefs deserialize");
        assert!(!prefs.autonomous_mode, "autonomous_mode defaults to false");
        assert_eq!(prefs.autonomous_tuning, AutonomousTuning::default());
        assert_eq!(prefs.autonomous_tuning.max_attempts, 3);
        assert_eq!(prefs.merged_issues, vec![42], "existing fields preserved");
        assert!(!IssueMonitorPrefs::default().autonomous_mode);
    }

    #[test]
    fn resolve_review_model_prefers_different_configured_model() {
        // Configured + different from implementer ⇒ use it (no self-grade).
        assert_eq!(
            resolve_review_model(Some("claude-opus"), Some("claude-sonnet")),
            Some("claude-sonnet".to_string()),
        );
        // Configured == implementer ⇒ None (would be a self-grade).
        assert_eq!(
            resolve_review_model(Some("claude-opus"), Some("claude-opus")),
            None
        );
        assert_eq!(
            resolve_review_model(Some("OPUS"), Some("opus")),
            None,
            "case-insensitive"
        );
        // Unset / empty ⇒ None (fall back to saved model, still fresh session).
        assert_eq!(resolve_review_model(Some("claude-opus"), None), None);
        assert_eq!(resolve_review_model(Some("claude-opus"), Some("  ")), None);
        // No implementer model known ⇒ use the configured one.
        assert_eq!(
            resolve_review_model(None, Some("claude-sonnet")),
            Some("claude-sonnet".to_string()),
        );
    }

    #[test]
    fn pre_autonomous_prefs_fixture_file_round_trips() {
        // SPEC #3200 FR-001/FR-023, Sc 23: the committed pre-autonomous prefs
        // fixture (no autonomous_mode / tuning / records fields) must deserialize
        // with documented defaults and preserve all existing fields.
        let fixture = include_str!("../tests/fixtures/issue_monitor_prefs_pre_autonomous.json");
        let prefs: IssueMonitorPrefs =
            serde_json::from_str(fixture).expect("pre-autonomous fixture deserializes");
        assert!(prefs.enabled);
        assert_eq!(prefs.priority_order, vec![101, 102]);
        assert_eq!(prefs.merged_issues, vec![42]);
        assert!(!prefs.autonomous_mode, "autonomous_mode defaults false");
        assert_eq!(prefs.autonomous_tuning, AutonomousTuning::default());
        assert!(
            prefs.autonomous_records.is_empty(),
            "no records in a pre-autonomous prefs file",
        );
    }

    #[test]
    fn autonomous_eligibility_truth_table() {
        // SPEC #3200 FR-003/004/005, Sc 2/3/4: two-stage-opt-in negatives →
        // HumanGate; safety-precondition failures → NeedsHuman; all → Eligible.
        use crate::issue_monitor_gate::AcceptanceCriteria;
        use gwt_git::branch_protection::BranchProtectionStatus;
        let ok = AcceptanceCriteria {
            ids: vec!["AC-1".to_string()],
            machine_checkable: true,
            visual_surface: false,
        };
        let no_criteria = AcceptanceCriteria {
            ids: vec![],
            machine_checkable: false,
            visual_surface: false,
        };
        let verified = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let absent = BranchProtectionStatus::Absent;
        let unreadable = BranchProtectionStatus::Unreadable("403".to_string());

        assert_eq!(
            autonomous_eligibility(true, true, &ok, &verified, false, 0, 3),
            EligibilityDecision::Eligible
        );
        // (i)/(ii) opt-in negatives → HumanGate (NOT NeedsHuman).
        assert!(matches!(
            autonomous_eligibility(false, true, &ok, &verified, false, 0, 3),
            EligibilityDecision::HumanGate(_)
        ));
        assert!(matches!(
            autonomous_eligibility(true, false, &ok, &verified, false, 0, 3),
            EligibilityDecision::HumanGate(_)
        ));
        // (iii)/(iv)/(v) safety preconditions → NeedsHuman.
        assert!(matches!(
            autonomous_eligibility(true, true, &no_criteria, &verified, false, 0, 3),
            EligibilityDecision::NeedsHuman(_)
        ));
        assert!(matches!(
            autonomous_eligibility(true, true, &ok, &absent, false, 0, 3),
            EligibilityDecision::NeedsHuman(_)
        ));
        match autonomous_eligibility(true, true, &ok, &unreadable, false, 0, 3) {
            EligibilityDecision::NeedsHuman(reason) => {
                assert!(reason.contains("permissions"), "distinct reason: {reason}")
            }
            other => panic!("expected NeedsHuman, got {other:?}"),
        }
        assert!(matches!(
            autonomous_eligibility(true, true, &ok, &verified, true, 0, 3),
            EligibilityDecision::NeedsHuman(_)
        ));
        assert!(matches!(
            autonomous_eligibility(true, true, &ok, &verified, false, 3, 3),
            EligibilityDecision::NeedsHuman(_)
        ));
    }

    #[test]
    fn needs_human_is_terminal_and_not_revived_by_requeue() {
        // SPEC #3200 FR-027, Sc 12/21: NeedsHuman is terminal and a window-close
        // requeue must never revive it.
        assert!(MonitorInboxState::NeedsHuman.is_terminal());
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        if let Some(item) = monitor.inbox.iter_mut().find(|i| i.issue.number == 42) {
            item.state = MonitorInboxState::NeedsHuman;
        }
        assert_eq!(
            monitor.requeue_window("tab-1::agent-1"),
            None,
            "requeue must not revive a terminal NeedsHuman item"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
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

    #[test]
    fn prefs_tmp_path_is_process_unique_not_a_shared_fixed_name() {
        // adversarial review (shared *.json.tmp race): the daemon and GUI both
        // write this prefs file, so the atomic-write scratch path must be unique
        // per writer — never the old fixed `*.json.tmp` that concurrent writers
        // could truncate into torn JSON.
        let path = std::path::Path::new("/x/y/issue-monitor.json");
        let a = super::unique_prefs_tmp_path(path);
        let b = super::unique_prefs_tmp_path(path);
        assert_ne!(a, b, "each write gets a distinct scratch path (uuid)");
        assert_ne!(
            a,
            path.with_extension("json.tmp"),
            "not the old shared fixed name"
        );
        assert!(
            a.to_string_lossy()
                .contains(&std::process::id().to_string()),
            "scratch path is scoped to the writing process: {}",
            a.display()
        );
        assert_eq!(
            a.parent(),
            path.parent(),
            "scratch stays in the target's dir so the rename is atomic"
        );
    }

    #[test]
    fn save_issue_monitor_prefs_round_trips_and_leaves_no_scratch_file() {
        // The unique-scratch atomic write still round-trips and cleans up (the
        // rename consumes the temp), leaving only the target file.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let prefs = IssueMonitorPrefs {
            merged_issues: vec![7, 9],
            ..IssueMonitorPrefs::default()
        };
        save_issue_monitor_prefs(&path, &prefs).expect("save");

        let loaded = load_issue_monitor_prefs(&path).expect("load");
        assert_eq!(loaded.merged_issues, vec![7, 9]);

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "issue-monitor.json")
            .collect();
        assert!(
            leftovers.is_empty(),
            "no scratch file left behind: {leftovers:?}"
        );
    }

    #[test]
    fn issue_monitor_launch_profile_round_trips_all_launch_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("issue-monitor.json");
        let config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .model("gpt-5.5")
            .reasoning_level("high")
            .version("0.121.0")
            .session_mode(gwt_agent::SessionMode::Resume)
            .skip_permissions(true)
            .fast_mode(true)
            .runtime_target(gwt_agent::LaunchRuntimeTarget::Docker)
            .docker_service("app")
            .docker_lifecycle_intent(gwt_agent::DockerLifecycleIntent::Restart)
            .windows_shell(gwt_agent::WindowsShellKind::PowerShell7)
            .build();
        let prefs = IssueMonitorPrefs {
            launch_profile: Some(IssueMonitorLaunchProfile::from(&config)),
            ..IssueMonitorPrefs::default()
        };

        save_issue_monitor_prefs(&path, &prefs).expect("save");
        let loaded = load_issue_monitor_prefs(&path).expect("load");
        let profile = loaded.launch_profile.expect("launch profile");

        assert_eq!(profile.agent_id, "codex");
        assert_eq!(profile.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(profile.reasoning.as_deref(), Some("high"));
        assert_eq!(profile.version.as_deref(), Some("0.121.0"));
        assert_eq!(profile.session_mode, gwt_agent::SessionMode::Resume);
        assert!(profile.skip_permissions);
        assert!(profile.codex_fast_mode);
        assert_eq!(
            profile.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(profile.docker_service.as_deref(), Some("app"));
        assert_eq!(
            profile.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        assert_eq!(
            profile.windows_shell,
            Some(gwt_agent::WindowsShellKind::PowerShell7)
        );

        let previous = LaunchWizardPreviousProfile::from(profile);
        assert_eq!(previous.agent_id, "codex");
        assert_eq!(previous.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(previous.reasoning.as_deref(), Some("high"));
        assert_eq!(previous.version.as_deref(), Some("0.121.0"));
        assert_eq!(previous.session_mode, gwt_agent::SessionMode::Resume);
        assert!(previous.skip_permissions);
        assert!(previous.codex_fast_mode);
        assert_eq!(
            previous.runtime_target,
            gwt_agent::LaunchRuntimeTarget::Docker
        );
        assert_eq!(previous.docker_service.as_deref(), Some("app"));
        assert_eq!(
            previous.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        assert_eq!(
            previous.windows_shell,
            Some(gwt_agent::WindowsShellKind::PowerShell7)
        );
    }

    #[test]
    fn autonomous_phase_defaults_idle() {
        // SPEC #3200 T-022: an issue with no autonomous record reports no record,
        // and a freshly created record starts Idle.
        assert_eq!(AutonomousPhase::default(), AutonomousPhase::Idle);
        let monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(monitor.autonomous_record(42).is_none());
        assert_eq!(monitor.attempt_count(42), 0);
    }

    #[test]
    fn attempt_counter_increments_and_clears() {
        // SPEC #3200 T-016 / FR-026: a per-issue attempt counter increments on
        // each attempt and resets when the record is cleared (success/merge).
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        assert_eq!(monitor.record_attempt(7), 1);
        assert_eq!(monitor.record_attempt(7), 2);
        assert_eq!(monitor.attempt_count(7), 2);
        assert_eq!(monitor.attempt_count(8), 0, "other issues are independent");
        monitor.clear_autonomous_record(7);
        assert_eq!(monitor.attempt_count(7), 0, "clear resets the counter");
        assert!(monitor.autonomous_record(7).is_none());
    }

    #[test]
    fn autonomous_record_tracks_phase_launch_id_and_snapshot() {
        // SPEC #3200 T-022/T-018: phase, the active launch id binding the current
        // attempt, and the acceptance snapshot are all tracked per issue.
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_phase(9, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(9, Some("tab-1::agent-9".to_string()));
        let snapshot =
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot();
        monitor.capture_acceptance_snapshot(9, snapshot.clone());

        let record = monitor.autonomous_record(9).expect("record exists");
        assert_eq!(record.phase, AutonomousPhase::Implementing);
        assert_eq!(record.active_launch_id.as_deref(), Some("tab-1::agent-9"));
        assert_eq!(record.acceptance_snapshot.as_ref(), Some(&snapshot));

        monitor.set_active_launch_id(9, None);
        assert_eq!(
            monitor
                .autonomous_record(9)
                .and_then(|r| r.active_launch_id.clone()),
            None,
            "active launch id clears when the attempt's launch ends"
        );
    }

    #[test]
    fn transient_failure_under_cap_retries_and_counts() {
        // SPEC #3200 T-042/FR-026: a transient failure below max_attempts
        // re-queues the issue for resume and increments the attempt counter.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(42, Some("tab-1::agent-1".to_string()));

        assert_eq!(
            monitor.record_autonomous_failure(
                42,
                FailureClass::Transient,
                "network blip",
                "2026-06-29T00:00:00Z"
            ),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "transient retry re-queues (never a fake done state)"
        );
        assert_eq!(monitor.attempt_count(42), 1);
        assert_eq!(monitor.active_count(), 0, "slot freed for the retry");
        assert_eq!(
            monitor
                .autonomous_record(42)
                .map(|r| r.active_launch_id.clone()),
            Some(None),
            "the in-flight launch id is cleared on retry"
        );
        // T-043: the retry is scheduled for the future (bounded backoff), so the
        // issue is not eligible to relaunch immediately, but is once time passes.
        assert!(
            !monitor.retry_ready(42, "2026-06-29T00:00:00Z"),
            "not relaunchable before the backoff elapses"
        );
        assert!(
            monitor.retry_ready(42, "2026-06-29T01:00:00Z"),
            "relaunchable once the backoff window passes"
        );
    }

    #[test]
    fn transient_failure_at_cap_escalates_to_needs_human() {
        // SPEC #3200 T-033/FR-027, Sc 12: once the attempt counter reaches
        // max_attempts the issue escalates to NeedsHuman and is not relaunched.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.autonomous_tuning.max_attempts = 2;
        assert_eq!(
            monitor.record_autonomous_failure(
                42,
                FailureClass::Transient,
                "fail 1",
                "2026-06-29T00:00:00Z"
            ),
            AutonomousFailureOutcome::Retry { attempt: 1 }
        );
        // Re-launch the retried attempt, then fail again at the cap.
        monitor.complete_active_launch(42, "tab-1::agent-1b");
        match monitor.record_autonomous_failure(
            42,
            FailureClass::Transient,
            "fail 2",
            "2026-06-29T00:30:00Z",
        ) {
            AutonomousFailureOutcome::Escalated(reason) => {
                assert!(
                    reason.contains("exhausted"),
                    "reason names exhaustion: {reason}"
                )
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert_eq!(
            monitor.autonomous_record(42).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
        assert_eq!(monitor.active_count(), 0, "slot freed on escalation");
        // Terminal: a window-close requeue must not revive it.
        assert_eq!(monitor.requeue_window("tab-1::agent-1b"), None);
    }

    #[test]
    fn terminal_failure_escalates_immediately_regardless_of_attempts() {
        // SPEC #3200 T-042: a terminal failure (retry cannot fix) escalates on
        // the first attempt without exhausting the counter.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        match monitor.record_autonomous_failure(
            42,
            FailureClass::Terminal,
            "review rejected",
            "2026-06-29T00:00:00Z",
        ) {
            AutonomousFailureOutcome::Escalated(reason) => {
                assert!(
                    reason.contains("terminal"),
                    "reason names terminal: {reason}"
                )
            }
            other => panic!("expected escalation, got {other:?}"),
        }
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert_eq!(monitor.attempt_count(42), 1, "the attempt is still counted");
    }

    #[test]
    fn retry_backoff_is_exponential_and_capped() {
        // SPEC #3200 T-043/FR-029: the transient-retry delay grows exponentially
        // per attempt and is clamped to the configured cap.
        assert_eq!(autonomous_retry_backoff_secs(1, 60, 1800), 60);
        assert_eq!(autonomous_retry_backoff_secs(2, 60, 1800), 120);
        assert_eq!(autonomous_retry_backoff_secs(3, 60, 1800), 240);
        assert_eq!(
            autonomous_retry_backoff_secs(6, 60, 1800),
            1800,
            "clamped to cap"
        );
        assert_eq!(
            autonomous_retry_backoff_secs(100, 60, 1800),
            1800,
            "no overflow at large attempt counts"
        );
        assert_eq!(
            autonomous_retry_backoff_secs(0, 60, 1800),
            60,
            "attempt 0 floors at base"
        );
    }

    #[test]
    fn retry_ready_defaults_true_without_a_schedule() {
        // An issue with no pending retry schedule is always relaunch-ready, and an
        // unparseable clock fails open (never permanently blocks a retry).
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        assert!(monitor.retry_ready(42, "2026-06-29T00:00:00Z"));
        monitor.record_autonomous_failure(
            42,
            FailureClass::Transient,
            "blip",
            "2026-06-29T00:00:00Z",
        );
        assert!(
            monitor.retry_ready(42, "not-a-timestamp"),
            "unparseable now fails open"
        );
    }

    fn stuck_monitor(number: u64, launched_at: &str) -> IssueMonitorState {
        let mut monitor = launched_monitor(number, "tab-1::agent-1");
        // Stuck recovery is an autonomous-only feature (guarded by autonomous_mode).
        monitor.set_autonomous_mode(true);
        monitor.autonomous_tuning.stuck_timeout_secs = 1800;
        monitor.set_autonomous_phase(number, AutonomousPhase::Implementing);
        monitor.set_active_launch_id(number, Some("tab-1::agent-1".to_string()));
        monitor.record_autonomous_heartbeat(number, launched_at);
        monitor
    }

    #[test]
    fn stuck_detection_flags_idle_agent_past_timeout() {
        // SPEC #3200 T-044/T-035/FR-013: a launched autonomous agent with no
        // heartbeat past stuck_timeout_secs is stuck; a fresh heartbeat is not.
        let monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        // 20 min later (< 30 min timeout) ⇒ not yet stuck.
        assert!(monitor
            .stuck_autonomous_issues("2026-06-29T00:20:00Z")
            .is_empty());
        // 31 min later (> 30 min timeout) ⇒ stuck.
        assert_eq!(
            monitor.stuck_autonomous_issues("2026-06-29T00:31:00Z"),
            vec![42]
        );
    }

    #[test]
    fn stuck_detection_ignores_pipeline_in_flight() {
        // SPEC #3200 T-044: once review / Deliver is in flight, the merge-watch
        // timeout governs — a stale agent heartbeat must NOT reclaim the slot.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.set_autonomous_phase(42, AutonomousPhase::Reviewing);
        assert!(
            monitor
                .stuck_autonomous_issues("2026-06-29T02:00:00Z")
                .is_empty(),
            "Reviewing is pipeline-in-flight, not stuck"
        );
    }

    #[test]
    fn recover_stuck_returns_to_queued_and_is_idempotent() {
        // SPEC #3200 T-044/T-045: recovery reclaims the stuck slot and resumes
        // (Queued); a second pass finds nothing (idempotent).
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T01:00:00Z");
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            recovered[0],
            (42, AutonomousFailureOutcome::Retry { attempt: 1 })
        ));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued)
        );
        assert_eq!(monitor.active_count(), 0, "stuck slot reclaimed");
        assert!(
            monitor
                .recover_stuck_autonomous("2026-06-29T01:05:00Z")
                .is_empty(),
            "no longer launched ⇒ idempotent"
        );
    }

    #[test]
    fn recover_stuck_escalates_when_attempts_exhausted() {
        // SPEC #3200 T-044: a stuck agent on the last attempt escalates to
        // NeedsHuman rather than looping.
        let mut monitor = stuck_monitor(42, "2026-06-29T00:00:00Z");
        monitor.autonomous_tuning.max_attempts = 1;
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T01:00:00Z");
        assert!(matches!(
            recovered.as_slice(),
            [(42, AutonomousFailureOutcome::Escalated(_))]
        ));
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    #[test]
    fn status_view_surfaces_autonomous_mode_and_per_issue_summary() {
        // SPEC #3200 T-048/FR-033: autonomous_mode and per-issue phase / attempts
        // / needs_human are observable in the status view.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        monitor.record_attempt(42);
        monitor.set_autonomous_phase(42, AutonomousPhase::Reviewing);
        monitor.escalate_to_needs_human(43, "gate unavailable");

        let view = monitor.status_view();
        assert!(view.autonomous_mode, "autonomous_mode surfaced");
        let summary_42 = view
            .autonomous_issues
            .iter()
            .find(|s| s.issue_number == 42)
            .expect("issue 42 summarized");
        assert_eq!(summary_42.phase, AutonomousPhase::Reviewing);
        assert_eq!(summary_42.attempts, 1);
        assert!(!summary_42.needs_human);
        let summary_43 = view
            .autonomous_issues
            .iter()
            .find(|s| s.issue_number == 43)
            .expect("issue 43 summarized");
        assert!(summary_43.needs_human, "escalated issue marked needs_human");
        assert_eq!(summary_43.phase, AutonomousPhase::NeedsHuman);
    }

    #[test]
    fn two_stage_candidate_requires_mode_and_label() {
        // SPEC #3200 T-032/FR-003/004: the pure pre-gate filter requires BOTH
        // autonomous_mode ON and the auto-merge label. Either missing ⇒ not a
        // candidate (falls back to the human-gated path).
        let labelled = IssueMonitorIssue {
            labels: vec!["auto-merge".to_string()],
            ..issue(42)
        };
        let unlabelled = issue(43);

        let mut off = IssueMonitorState::new(IssueMonitorConfig::default());
        assert!(
            !off.is_autonomous_two_stage_candidate(&labelled),
            "mode off"
        );

        off.set_autonomous_mode(true);
        assert!(
            off.is_autonomous_two_stage_candidate(&labelled),
            "mode on + label ⇒ candidate"
        );
        assert!(
            !off.is_autonomous_two_stage_candidate(&unlabelled),
            "mode on but no label ⇒ not a candidate"
        );
    }

    #[test]
    fn autoclose_failed_window_only_for_autonomous_candidates() {
        // #3165/#3200 error-window lifecycle: a failed autonomous issue
        // (autonomous_mode ON + auto-merge label) auto-closes its stale window so
        // the retry relaunches clean; default issues keep theirs for inspection.
        let now = "2026-06-30T00:00:00Z";

        let mut auto = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        scan_issue_monitor_candidates(&mut auto, &[auto_issue(42, "b")], now);
        auto.record_agent_issue_failed(42, "boom");
        assert!(
            auto.should_autoclose_failed_window(42),
            "autonomous candidate failure ⇒ auto-close the stale window"
        );
        assert!(
            !auto.should_autoclose_failed_window(999),
            "unknown issue ⇒ no close"
        );

        // autonomous_mode OFF ⇒ keep the window (default human-gated path).
        let mut def = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut def, &[auto_issue(42, "b")], now);
        def.record_agent_issue_failed(42, "boom");
        assert!(
            !def.should_autoclose_failed_window(42),
            "autonomous_mode off ⇒ keep the failed window"
        );

        // autonomous_mode ON but no auto-merge label ⇒ keep the window.
        let mut nolabel = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        );
        scan_issue_monitor_candidates(&mut nolabel, &[issue(43)], now);
        nolabel.record_agent_issue_failed(43, "boom");
        assert!(
            !nolabel.should_autoclose_failed_window(43),
            "no auto-merge label ⇒ keep the failed window"
        );
    }

    #[test]
    fn failed_window_is_retained_persisted_and_cleared_on_relaunch() {
        // #3165 error-window lifecycle: a failed agent window id is retained per
        // issue (and persisted) so an explicit Launch Now can close the stale
        // window before relaunching; a successful relaunch clears it.
        let mut monitor = launched_monitor(42, "tab-1::agent-42");
        assert_eq!(
            monitor.record_agent_window_failed("tab-1::agent-42", "boom"),
            Some(42)
        );

        // Persisted across a prefs round-trip (daemon/GUI restart).
        let mut restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        assert_eq!(
            restored.take_failed_window(42).as_deref(),
            Some("tab-1::agent-42"),
            "stale window id retained + persisted for Launch Now"
        );
        // take is one-shot.
        assert_eq!(restored.take_failed_window(42), None);

        // A successful (re)launch clears any retained stale window.
        let mut relaunch = launched_monitor(43, "old::agent-43");
        relaunch.record_agent_window_failed("old::agent-43", "boom");
        relaunch.complete_active_launch(43, "new::agent-43");
        assert_eq!(
            relaunch.take_failed_window(43),
            None,
            "relaunch clears the stale window so it is not double-closed"
        );

        // A pre-launch failure with no window records nothing to close.
        let mut no_window = IssueMonitorState::new(IssueMonitorConfig::default());
        scan_issue_monitor_candidates(&mut no_window, &[issue(44)], "2026-06-30T00:00:00Z");
        no_window.record_launch_failed(44, "could not create branch");
        assert_eq!(no_window.take_failed_window(44), None);
    }

    #[test]
    fn failed_window_does_not_orphan_on_terminal_transition() {
        // Adversarial-review fix: a failed issue that ends WITHOUT an explicit
        // Launch Now relaunch (merged / released — both funnel through
        // clear_active_tracking) must not orphan its retained failed-window id,
        // which would otherwise persist into prefs unbounded.
        for terminal in ["merged", "released"] {
            let mut monitor = launched_monitor(42, "tab-1::agent-42");
            monitor.record_agent_window_failed("tab-1::agent-42", "boom");
            assert!(
                monitor
                    .prefs()
                    .failed_issues
                    .iter()
                    .any(|f| f.window_id.as_deref() == Some("tab-1::agent-42")),
                "failed window retained before {terminal}"
            );

            match terminal {
                "merged" => monitor.record_merged(42),
                "released" => monitor.record_released(42),
                _ => unreachable!(),
            }

            assert_eq!(
                monitor.take_failed_window(42),
                None,
                "{terminal} must clear the stale failed window (no orphan)"
            );
            assert!(
                monitor
                    .prefs()
                    .failed_issues
                    .iter()
                    .all(|f| f.window_id.is_none()),
                "{terminal} must not persist an orphaned window id"
            );
        }
    }

    #[test]
    fn autonomous_records_survive_prefs_roundtrip() {
        // SPEC #3200 T-016/T-022: attempt counter + phase + launch id + snapshot
        // persist through a prefs round-trip so a daemon restart does not lose an
        // in-flight autonomous attempt (and does not reset attempts to zero).
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.record_attempt(11);
        monitor.record_attempt(11);
        monitor.set_autonomous_phase(11, AutonomousPhase::Reviewing);
        monitor.set_active_launch_id(11, Some("tab-2::agent-11".to_string()));
        monitor.capture_acceptance_snapshot(
            11,
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot(),
        );

        let prefs = monitor.prefs();
        assert_eq!(prefs.autonomous_records.len(), 1);

        let restored = IssueMonitorState::with_prefs(IssueMonitorConfig::default(), prefs);
        let record = restored.autonomous_record(11).expect("record restored");
        assert_eq!(record.attempts, 2);
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
        assert_eq!(record.active_launch_id.as_deref(), Some("tab-2::agent-11"));
        assert_eq!(restored.attempt_count(11), 2);
        assert!(record.acceptance_snapshot.is_some());
    }

    fn auto_issue(number: u64, body: &str) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some(body.to_string()),
            url: None,
        }
    }

    fn autonomous_state() -> IssueMonitorState {
        IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            IssueMonitorPrefs {
                autonomous_mode: true,
                ..IssueMonitorPrefs::default()
            },
        )
    }

    #[test]
    fn prepare_autonomous_candidate_non_candidate_is_human_gate_noop() {
        // SPEC #3200 FR-001/003: autonomous_mode OFF (or no label) ⇒ no autonomous
        // state created; the issue uses the existing human-gated path.
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert!(matches!(decision, EligibilityDecision::HumanGate(_)));
        assert!(monitor.autonomous_record(50).is_none());
    }

    #[test]
    fn prepare_autonomous_candidate_eligible_captures_snapshot_and_phase() {
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert_eq!(decision, EligibilityDecision::Eligible);
        let record = monitor.autonomous_record(50).expect("record");
        assert_eq!(record.phase, AutonomousPhase::Implementing);
        assert!(
            record.acceptance_snapshot.is_some(),
            "snapshot captured at launch"
        );
    }

    #[test]
    fn prepare_autonomous_candidate_unverified_protection_escalates() {
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Absent;
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert!(matches!(decision, EligibilityDecision::NeedsHuman(_)));
        assert_eq!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
            "ineligible candidate is escalated, not launched",
        );
    }

    #[test]
    fn prepare_autonomous_candidate_without_criteria_escalates() {
        // SPEC #3200 FR-014: no machine-checkable acceptance criteria ⇒ NeedsHuman.
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let decision = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "free text, no criteria"),
            &bp,
            "2026-06-29T00:00:00Z",
        );
        assert!(matches!(decision, EligibilityDecision::NeedsHuman(_)));
        assert_eq!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::NeedsHuman),
        );
    }

    #[test]
    fn prepare_autonomous_candidate_respects_retry_backoff() {
        // SPEC #3200 T-043/FR-029: a candidate whose transient-retry backoff has
        // not elapsed is skipped (no capture/escalation); once it elapses it is
        // processed normally.
        let mut monitor = autonomous_state();
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        // Schedule a backoff: a transient failure sets retry_not_before.
        monitor.record_attempt(50); // ensure a record exists
        monitor.record_autonomous_failure(
            50,
            FailureClass::Transient,
            "blip",
            "2026-06-29T00:00:00Z",
        );
        // Still inside the backoff window ⇒ skipped (HumanGate, not captured).
        let blocked = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T00:00:30Z",
        );
        assert!(matches!(blocked, EligibilityDecision::HumanGate(_)));
        assert_ne!(
            monitor.autonomous_record(50).map(|r| r.phase),
            Some(AutonomousPhase::Implementing),
            "not launched while backing off",
        );
        // After the backoff window ⇒ eligible and prepared.
        let ready = monitor.prepare_autonomous_candidate(
            &auto_issue(50, "## Acceptance Criteria\n- [ ] AC-1: x\n"),
            &bp,
            "2026-06-29T02:00:00Z",
        );
        assert_eq!(ready, EligibilityDecision::Eligible);
        assert_eq!(
            monitor
                .autonomous_record(50)
                .and_then(|r| r.retry_not_before.clone()),
            None,
            "launching clears the consumed backoff marker",
        );
    }

    #[test]
    fn recover_stuck_autonomous_is_noop_when_mode_off() {
        // Fail-closed: with autonomous_mode OFF, stuck recovery never mutates
        // state (defends the SPEC #3165 path against the runtime toggle).
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(false);
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.record_autonomous_heartbeat(42, "2026-06-29T00:00:00Z");
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T05:00:00Z");
        assert!(recovered.is_empty(), "off ⇒ no recovery");
        assert_eq!(monitor.active_count(), 1, "slot untouched when mode off");
        assert_eq!(monitor.attempt_count(42), 0, "no attempt recorded when off");
    }

    #[test]
    fn resume_after_restart_reverts_reviewing_to_implementing_for_redispatch() {
        // review follow-up: a record persisted mid-review reloads in `Reviewing`,
        // but its (non-persisted) review dispatch is gone, so it would wait forever
        // for a verdict. Resetting it to `Implementing` lets the next scan re-detect
        // the PR and re-issue the review — restoring the pre-persist self-healing.
        // The record is round-tripped through prefs to model an actual restart.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123"); // → Reviewing, verdict pending
        let restored_prefs = monitor.prefs();

        let mut restarted =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), restored_prefs);
        assert_eq!(
            restarted.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing),
            "reloads in Reviewing (the strand)"
        );

        let resumed = restarted.resume_inflight_reviews_after_restart("2026-06-29T00:00:00Z");

        assert_eq!(resumed, vec![7], "the stranded Reviewing record is resumed");
        let record = restarted.autonomous_record(7).expect("record retained");
        assert_eq!(
            record.phase,
            AutonomousPhase::Implementing,
            "reset to Implementing so the next scan re-detects the PR + re-dispatches"
        );
        assert_eq!(
            record.review_passed, None,
            "verdict cleared for the re-review"
        );
        // Still counted in-flight (holds its slot); no attempt is spent on a restart.
        assert_eq!(restarted.autonomous_in_flight_issues(), vec![7]);
        assert_eq!(
            restarted.attempt_count(7),
            0,
            "a restart is not a failed attempt"
        );
    }

    #[test]
    fn resume_after_restart_leaves_delivering_and_other_phases_untouched() {
        // Delivering self-heals on its own (its watch polls the persisted pr_number
        // for the merge commit) and has an armed GitHub auto-merge, so it must NOT
        // be re-driven. Idle/Implementing/terminal phases are also left alone.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(1, AutonomousPhase::Delivering);
        monitor.set_autonomous_phase(2, AutonomousPhase::Implementing);
        monitor.set_autonomous_phase(3, AutonomousPhase::NeedsHuman);
        monitor.set_autonomous_phase(4, AutonomousPhase::Idle);

        let resumed = monitor.resume_inflight_reviews_after_restart("2026-06-29T00:00:00Z");

        assert!(resumed.is_empty(), "no Reviewing records ⇒ nothing resumed");
        assert_eq!(
            monitor.autonomous_record(1).map(|r| r.phase),
            Some(AutonomousPhase::Delivering),
            "Delivering is left to its own merge-watch self-heal"
        );
        assert_eq!(
            monitor.autonomous_record(2).map(|r| r.phase),
            Some(AutonomousPhase::Implementing)
        );
        assert_eq!(
            monitor.autonomous_record(3).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
    }

    #[test]
    fn autonomous_transitions_emit_notices_for_the_operator() {
        // SPEC #3200 FR-034 (T-109/T-111, Sc 24): unattended autonomous lifecycle
        // transitions must surface operator notices — merged, needs-human, and
        // transient retry — so fully-unattended operation is observable. The
        // notices queue is drained by the daemon worker into `toast` payloads.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-07-02T00:00:00Z");
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);

        // Transient retry → warn notice naming the attempt.
        monitor.record_autonomous_failure(
            7,
            FailureClass::Transient,
            "review spawn blip",
            "2026-07-02T00:10:00Z",
        );
        // Gate pass → Delivering; the info notice fires only once the arm
        // actually SUCCEEDS (codex #3217: no success toast for a failed arm).
        monitor.set_autonomous_phase(7, AutonomousPhase::Reviewing);
        monitor.begin_delivering(7);
        monitor.record_auto_merge_armed(7);
        // Merge completion → done notice.
        monitor.record_merged(7);
        // A second issue escalates → error notice.
        scan_issue_monitor_candidates(&mut monitor, &[issue(8)], "2026-07-02T00:20:00Z");
        monitor.record_attempt(8);
        monitor.escalate_to_needs_human(8, "review rejected");

        let notices = monitor.take_autonomous_notices();
        let summary: Vec<(String, u64)> = notices
            .iter()
            .map(|notice| (notice.level.clone(), notice.issue_number))
            .collect();
        assert!(
            summary.contains(&("warn".to_string(), 7)),
            "transient retry emits a warn notice: {summary:?}"
        );
        assert!(
            summary.contains(&("info".to_string(), 7)),
            "auto-merge arming emits an info notice: {summary:?}"
        );
        assert!(
            summary.contains(&("done".to_string(), 7)),
            "merge completion emits a done notice: {summary:?}"
        );
        assert!(
            summary.contains(&("error".to_string(), 8)),
            "needs-human escalation emits an error notice: {summary:?}"
        );
        let retry = notices
            .iter()
            .find(|notice| notice.level == "warn")
            .expect("retry notice");
        assert!(
            retry.message.contains("review spawn blip"),
            "retry notice carries the failure reason: {}",
            retry.message
        );
        // Drained: a second take returns nothing.
        assert!(monitor.take_autonomous_notices().is_empty());
    }

    #[test]
    fn default_off_transitions_emit_no_autonomous_notices() {
        // FR-004 non-regression: with autonomous_mode OFF (default), the human-
        // gated #3165 flow emits no autonomous notices — merges and failures are
        // already visible via inbox state without extra toasts.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_merged(42);
        assert!(
            monitor.take_autonomous_notices().is_empty(),
            "default-OFF merge emits no autonomous notice"
        );
    }

    #[test]
    fn kill_switch_retries_failed_disarms_until_success() {
        // codex #3217/#3219 review: turning autonomous mode OFF must actively
        // disarm GitHub auto-merges armed by the monitor — and a FAILED disarm
        // must stay retryable. A record leaves Delivering only after the disarm
        // succeeds; otherwise the armed auto-merge would stay live on GitHub
        // behind a NeedsHuman screen with nothing retrying it.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-07-02T00:00:00Z");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");
        monitor.begin_delivering(7);

        // Mode ON ⇒ no targets (deliveries are still owned by the loop).
        assert!(monitor.kill_switch_disarm_targets().is_empty());

        monitor.set_autonomous_mode(false);
        assert_eq!(
            monitor.kill_switch_disarm_targets(),
            vec![(7, 99)],
            "delivering PR targeted for disarm"
        );

        // FAILED disarm: error notice, record STAYS Delivering ⇒ re-targeted.
        monitor.record_kill_switch_disarm_result(7, 99, false);
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Delivering),
            "failed disarm keeps the record in Delivering for retry"
        );
        assert_eq!(
            monitor.kill_switch_disarm_targets(),
            vec![(7, 99)],
            "next scan retries the disarm"
        );

        // SUCCESSFUL disarm: escalates to NeedsHuman (visible, never silently
        // resumed) and stops being targeted.
        monitor.record_kill_switch_disarm_result(7, 99, true);
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman)
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
        assert!(monitor.kill_switch_disarm_targets().is_empty());

        // Both outcomes surfaced even though the mode is OFF (ungated notices).
        let notices = monitor.take_autonomous_notices();
        assert!(notices
            .iter()
            .any(|n| n.level == "error" && n.message.contains("retry next scan")));
        assert!(notices
            .iter()
            .any(|n| n.level == "warn" && n.message.contains("disarmed")));
    }

    #[test]
    fn autonomous_notices_queue_is_bounded() {
        // Unattended operation with a disconnected GUI must not grow the queue
        // without limit: oldest notices are dropped past the cap.
        let mut monitor = autonomous_state();
        for n in 0..200u64 {
            monitor.record_attempt(n);
            monitor.escalate_to_needs_human(n, "boom");
        }
        let notices = monitor.take_autonomous_notices();
        assert!(
            notices.len() <= 100,
            "queue bounded to 100, got {}",
            notices.len()
        );
        assert_eq!(
            notices.last().map(|notice| notice.issue_number),
            Some(199),
            "newest notice retained when the cap drops oldest"
        );
    }

    #[test]
    fn resume_after_restart_refreshes_heartbeat_so_stuck_recovery_does_not_fail_it() {
        // review follow-up (codex #3210): on restart the persisted active slot and
        // a STALE last_heartbeat are restored. Resetting Reviewing → Implementing
        // makes the record eligible for stuck/idle detection, which runs BEFORE the
        // re-dispatch on the next scan. Without refreshing the heartbeat, a review
        // that ran longer than stuck_timeout_secs before the restart would be
        // wrongly counted as a failed attempt. The refresh must prevent that.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.set_autonomous_mode(true);
        monitor.set_autonomous_phase(42, AutonomousPhase::Implementing);
        monitor.begin_review(42, 99, "abc123"); // → Reviewing (holds the active slot)
        monitor.record_autonomous_heartbeat(42, "2026-06-29T00:00:00Z"); // stale pre-restart

        // Resume, then run stuck recovery in the same scan order as the daemon.
        let resumed = monitor.resume_inflight_reviews_after_restart("2026-06-29T05:00:00Z");
        assert_eq!(resumed, vec![42]);
        let recovered = monitor.recover_stuck_autonomous("2026-06-29T05:00:00Z");

        assert!(
            recovered.is_empty(),
            "the resumed record's fresh heartbeat keeps it out of stuck recovery"
        );
        assert_eq!(
            monitor.autonomous_record(42).map(|r| r.phase),
            Some(AutonomousPhase::Implementing),
            "still Implementing, ready for the next scan to re-dispatch review"
        );
        assert_eq!(
            monitor.attempt_count(42),
            0,
            "a restart is not counted as a failed attempt"
        );
    }

    #[test]
    fn autonomous_loop_transitions_track_pr_sha_and_verdict() {
        // SPEC #3200: Implementing → Reviewing (bind PR + reviewed SHA) →
        // verdict recorded → Delivering. autonomous_in_flight_issues tracks it.
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        assert_eq!(monitor.autonomous_in_flight_issues(), vec![7]);

        monitor.begin_review(7, 99, "abc123");
        let record = monitor.autonomous_record(7).expect("record");
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
        assert_eq!(record.pr_number, Some(99));
        assert_eq!(record.reviewed_sha.as_deref(), Some("abc123"));
        assert_eq!(record.review_passed, None, "verdict pending");

        monitor.record_review_verdict(7, true);
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(true)
        );

        monitor.begin_delivering(7);
        assert_eq!(
            monitor.autonomous_record(7).unwrap().phase,
            AutonomousPhase::Delivering
        );
        assert_eq!(monitor.autonomous_in_flight_issues(), vec![7]);

        // Completion clears the whole record.
        monitor.record_merged(7);
        assert!(monitor.autonomous_record(7).is_none());
        assert!(monitor.autonomous_in_flight_issues().is_empty());
    }

    #[test]
    fn launch_failure_routes_inflight_autonomous_issue_through_retry() {
        // SPEC #3200 (review follow-up): a launch/agent failure for an in-flight
        // autonomous issue — e.g. the independent review agent could not spawn —
        // must funnel through the autonomous retry machinery (count an attempt,
        // schedule backoff, re-queue) instead of stranding the record in
        // `Reviewing` forever, waiting for a verdict that will never arrive.
        let mut monitor = autonomous_state();
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123"); // Implementing → Reviewing
        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing)
        );
        assert!(monitor.is_autonomous_in_flight(7));

        monitor.record_launch_failed(7, "Independent review could not start");

        let record = monitor.autonomous_record(7).expect("record retained");
        assert_eq!(
            record.phase,
            AutonomousPhase::Idle,
            "routed back to Idle for retry, not stranded in Reviewing"
        );
        assert_eq!(monitor.attempt_count(7), 1, "the failed attempt is counted");
        assert!(
            record.retry_not_before.is_some(),
            "a retry backoff is scheduled"
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "re-queued for automatic relaunch (not parked in LaunchFailed)"
        );
    }

    #[test]
    fn launch_failure_at_cap_escalates_inflight_autonomous_issue_to_needs_human() {
        // SPEC #3200 (review follow-up): once the in-flight autonomous issue's
        // attempts are exhausted, a further launch failure escalates to
        // NeedsHuman through the same routing rather than silently retrying.
        let mut monitor = autonomous_state();
        monitor.autonomous_tuning.max_attempts = 1;
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        monitor.complete_active_launch(7, "tab-1::agent-7");
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");

        monitor.record_launch_failed(7, "review spawn failed at cap");

        assert_eq!(
            monitor.autonomous_record(7).map(|r| r.phase),
            Some(AutonomousPhase::NeedsHuman),
            "attempts exhausted ⇒ escalated, not retried"
        );
        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::NeedsHuman)
        );
    }

    #[test]
    fn launch_failure_for_non_autonomous_issue_keeps_plain_failed_state() {
        // Non-regression: with no in-flight autonomous record, the launch failure
        // stays on the human-gated LaunchFailed path (SPEC #3165), untouched by
        // the autonomous routing.
        let mut monitor = autonomous_state(); // mode on, but no record for #7
        scan_issue_monitor_candidates(&mut monitor, &[issue(7)], "2026-06-30T00:00:00Z");
        assert!(!monitor.is_autonomous_in_flight(7));

        monitor.record_launch_failed(7, "binary missing");

        assert_eq!(
            monitor.inbox_item(7).map(|item| item.state),
            Some(MonitorInboxState::LaunchFailed),
            "plain launch-failed path is preserved when no autonomous attempt is in flight"
        );
        assert_eq!(
            monitor.attempt_count(7),
            0,
            "no autonomous attempt is counted"
        );
    }

    #[test]
    fn autonomous_gate_inputs_assemble_into_a_pass_and_detect_drift() {
        // SPEC #3200 FR-009..FR-016: assembled inputs run through the real gate.
        use crate::issue_monitor_gate::{
            classify_acceptance_criteria, evaluate_autonomous_gate, GateDecision,
        };
        use gwt_git::branch_protection::BranchProtectionStatus;
        let body = "## Acceptance Criteria\n- [ ] AC-1: x\n";
        let mut monitor = autonomous_state();
        monitor.capture_acceptance_snapshot(7, classify_acceptance_criteria(body).snapshot());
        monitor.begin_review(7, 99, "abc123");
        monitor.record_review_verdict(7, true);

        let bp = BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        let rollup = r#"[{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}]"#;

        // All conditions hold at the reviewed SHA ⇒ gate Pass.
        let inputs = monitor
            .autonomous_gate_inputs(7, bp.clone(), rollup, "abc123", body)
            .expect("gate ready");
        assert_eq!(evaluate_autonomous_gate(&inputs), GateDecision::Pass);

        // Issue body edited after launch ⇒ acceptance drift ⇒ gate Fail.
        let drifted = monitor
            .autonomous_gate_inputs(
                7,
                bp.clone(),
                rollup,
                "abc123",
                "## Acceptance Criteria\n- [ ] AC-2: new\n",
            )
            .expect("gate ready");
        assert!(matches!(
            evaluate_autonomous_gate(&drifted),
            GateDecision::Fail(_)
        ));

        // HEAD advanced past reviewed SHA ⇒ TOCTOU ⇒ gate Fail.
        let advanced = monitor
            .autonomous_gate_inputs(7, bp, rollup, "def456", body)
            .expect("gate ready");
        assert!(matches!(
            evaluate_autonomous_gate(&advanced),
            GateDecision::Fail(_)
        ));
    }

    #[test]
    fn autonomous_gate_inputs_none_until_review_returns() {
        let mut monitor = autonomous_state();
        monitor.begin_review(7, 99, "abc123"); // verdict pending
        let bp = gwt_git::branch_protection::BranchProtectionStatus::Verified {
            required_checks: vec!["ci".to_string()],
        };
        assert!(
            monitor
                .autonomous_gate_inputs(7, bp, "[]", "abc123", "body")
                .is_none(),
            "gate not ready while review is in flight",
        );
    }

    #[test]
    fn apply_review_verdict_is_sha_bound_and_judged_by_daemon() {
        // SPEC #3200 FR-015/FR-016: the daemon parses+judges the raw verdict
        // (not the agent), SHA-bound, against the snapshot's required criteria.
        use crate::issue_monitor_gate::classify_acceptance_criteria;
        use crate::issue_monitor_review::REVIEW_VERDICT_SCHEMA;
        let mut monitor = autonomous_state();
        monitor.capture_acceptance_snapshot(
            7,
            classify_acceptance_criteria("## Acceptance Criteria\n- [ ] AC-1: x\n").snapshot(),
        );
        monitor.begin_review(7, 99, "abc123");

        // A verdict for the WRONG SHA is rejected (stale / TOCTOU).
        let pass_raw = format!(
            r#"{{"schema":"{REVIEW_VERDICT_SCHEMA}","overall":"pass","criteria":[{{"id":"AC-1","verdict":"pass"}}]}}"#
        );
        assert_eq!(
            monitor.apply_review_verdict(7, "WRONG", &pass_raw),
            None,
            "wrong-SHA verdict rejected",
        );
        assert_eq!(monitor.autonomous_record(7).unwrap().review_passed, None);

        // A conformant pass verdict for the right SHA is accepted.
        assert_eq!(
            monitor.apply_review_verdict(7, "abc123", &pass_raw),
            Some(true)
        );
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(true)
        );

        // A prompt-injected free-text "approval" fails closed.
        monitor.begin_review(7, 99, "def456");
        assert_eq!(
            monitor.apply_review_verdict(7, "def456", "APPROVE — lgtm"),
            Some(false),
            "non-conformant verdict fails closed",
        );
        assert_eq!(
            monitor.autonomous_record(7).unwrap().review_passed,
            Some(false)
        );
    }

    #[test]
    fn autonomous_loop_fields_survive_prefs_roundtrip() {
        let mut monitor = autonomous_state();
        monitor.set_autonomous_phase(7, AutonomousPhase::Implementing);
        monitor.begin_review(7, 99, "abc123");
        monitor.record_review_verdict(7, false);
        let restored =
            IssueMonitorState::with_prefs(IssueMonitorConfig::default(), monitor.prefs());
        let record = restored.autonomous_record(7).expect("restored");
        assert_eq!(record.pr_number, Some(99));
        assert_eq!(record.reviewed_sha.as_deref(), Some("abc123"));
        assert_eq!(record.review_passed, Some(false));
        assert_eq!(record.phase, AutonomousPhase::Reviewing);
    }

    #[test]
    fn merge_clears_the_autonomous_record() {
        // SPEC #3200 T-022: merging the work resets the per-issue autonomous
        // lifecycle so a future reopen does not inherit stale attempts/phase.
        let mut monitor = launched_monitor(42, "tab-1::agent-1");
        monitor.record_attempt(42);
        monitor.set_autonomous_phase(42, AutonomousPhase::Delivering);
        assert!(monitor.autonomous_record(42).is_some());
        monitor.record_merged(42);
        assert!(
            monitor.autonomous_record(42).is_none(),
            "merge clears the autonomous record",
        );
        assert_eq!(monitor.attempt_count(42), 0);
    }
}

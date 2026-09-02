//! Pull Request status tracking via GitHub CLI

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use gwt_core::github_budget::{self, BudgetLedger, ThrottlePolicy};
use gwt_core::github_quota::GitHubQuota;
use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Pull Request state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "OPEN"),
            Self::Closed => write!(f, "CLOSED"),
            Self::Merged => write!(f, "MERGED"),
        }
    }
}

/// Status of a Pull Request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrStatus {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub url: String,
    pub created_at: Option<DateTime<Utc>>,
    /// Overall CI status: "SUCCESS", "FAILURE", "PENDING", or "UNKNOWN".
    pub ci_status: String,
    /// Raw `mergeable` field from GitHub: "MERGEABLE", "CONFLICTING", "UNKNOWN".
    pub mergeable: String,
    /// Raw `mergeStateStatus` field from GitHub: "CLEAN", "BEHIND", "UNKNOWN", etc.
    pub merge_state_status: String,
    /// Review verdict: "APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED", or "UNKNOWN".
    pub review_status: String,
}

impl PrStatus {
    /// Return the merge state label that CLI summaries should display.
    pub fn effective_merge_status(&self) -> &str {
        effective_merge_status_label(&self.mergeable, &self.merge_state_status)
    }

    pub fn requires_update_branch(&self) -> bool {
        self.merge_state_status == "BEHIND"
    }
}

/// Hours without an `updatedAt` bump after which an open PR is stale for the
/// PM inventory (Issue #3781 AC-2). Overridable per call through
/// [`PrInventoryOptions::stale_after_hours`] (Issue #3868 AC-5).
pub const PR_STALE_AFTER_HOURS: i64 = 72;

/// Consecutive `pr.list` observations with identical real data after which a
/// row is `escalation_due` (Issue #3868 AC-6). Counted from the second
/// observation, so the default flags a PR on its fourth unchanged cycle. One
/// `pr.list` read is one observation, so the PM reads the inventory once per
/// resident cycle.
pub const PR_ESCALATE_AFTER_UNCHANGED_CYCLES: u32 = 3;

/// The PM fallback attached to rows whose `default_action` cannot be executed
/// through the Issue Monitor (Issue #3868 AC-2). The order is fixed: triage
/// first, rerun a flake, fresh-launch a regression, escalate when neither
/// is possible.
pub const PR_FALLBACK_WHEN_NOT_EXECUTABLE: &str = "PM triages the failure (#3790) → flake: \
arrange a rerun → regression: arrange a fresh launch → neither possible: escalate to a human now";

/// Thresholds that shape the PM inventory (Issue #3868 AC-5 / AC-6) and the
/// budget behaviour of the read itself (Issue #3891).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrInventoryOptions {
    /// Hours without an `updated_at` bump before a row is `stale`.
    pub stale_after_hours: i64,
    /// Unchanged consecutive observations before a row is `escalation_due`.
    pub escalate_after_cycles: u32,
    /// Bypass the TTL cache and the budget throttle: the caller needs the live
    /// state for a decision. Default `false` (periodic, non-essential read).
    pub refresh: bool,
    /// Heavy per-PR fields to hydrate on top of the light list query.
    pub include: PrInventoryInclude,
}

impl Default for PrInventoryOptions {
    fn default() -> Self {
        Self {
            stale_after_hours: PR_STALE_AFTER_HOURS,
            escalate_after_cycles: PR_ESCALATE_AFTER_UNCHANGED_CYCLES,
            refresh: false,
            include: PrInventoryInclude::default(),
        }
    }
}

/// Heavy fields `pr.list` fetches per PR instead of in the bulk list query
/// (Issue #3891 AC-2). The list query itself never carries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInventoryInclude {
    /// `statusCheckRollup` — the input of CI-RED / MERGE-CANDIDATE. Costs
    /// ~100 GraphQL points in a 100-row list, ~2 per single `pr view`.
    pub checks: bool,
    /// The PR body — only the "superseded" heuristic reads it.
    pub body: bool,
}

impl Default for PrInventoryInclude {
    fn default() -> Self {
        Self {
            checks: true,
            body: false,
        }
    }
}

impl PrInventoryInclude {
    /// Whether data hydrated with `self` satisfies a caller wanting `wanted`.
    fn covers(self, wanted: Self) -> bool {
        (!wanted.checks || self.checks) && (!wanted.body || self.body)
    }

    fn is_empty(self) -> bool {
        !self.checks && !self.body
    }

    /// `gh pr view --json` field list for the hydration call.
    fn view_json_fields(self) -> String {
        let mut fields = Vec::new();
        if self.checks {
            fields.push("statusCheckRollup");
        }
        if self.body {
            fields.push("body");
        }
        fields.join(",")
    }
}

/// Lifecycle class the PM uses to pick a default action (Issue #3781 AC-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrLifecycleClass {
    MergeCandidate,
    Conflicted,
    Behind,
    CiRed,
    Superseded,
    InProgress,
    /// GitHub has not computed mergeability (`UNKNOWN`), so no definite class
    /// can be claimed. `pr.list` holds the previous class when the PR's real
    /// data has not changed (Issue #3868 lifecycle stability).
    Undetermined,
}

impl PrLifecycleClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MergeCandidate => "MERGE-CANDIDATE",
            Self::Conflicted => "CONFLICTED",
            Self::Behind => "BEHIND",
            Self::CiRed => "CI-RED",
            Self::Superseded => "SUPERSEDED",
            Self::InProgress => "IN-PROGRESS",
            Self::Undetermined => "UNDETERMINED",
        }
    }

    fn parse(label: &str) -> Option<Self> {
        [
            Self::MergeCandidate,
            Self::Conflicted,
            Self::Behind,
            Self::CiRed,
            Self::Superseded,
            Self::InProgress,
            Self::Undetermined,
        ]
        .into_iter()
        .find(|class| class.as_str() == label)
    }

    /// Whether the default action relaunches the owner Issue.
    fn relaunches_owner(self) -> bool {
        matches!(self, Self::Conflicted | Self::CiRed)
    }
}

/// Closing Issue referenced by an open PR, when `gh pr list` exposes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrClosingIssue {
    pub number: u64,
    pub state: Option<String>,
}

/// Fields needed to classify one open PR without the derived lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrInventoryFields {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    /// PR head branch (`headRefName`). A head sitting on the owner's launch
    /// ref (`work/issue-<owner>`) is exactly what the Issue Monitor refuses
    /// to fresh-launch over (unique commits), so it decides executability.
    pub head_ref_name: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub mergeable: String,
    pub merge_state_status: String,
    pub ci_status: String,
    pub review_status: String,
    pub body: String,
    pub closing_issues: Vec<PrClosingIssue>,
}

/// Result of classifying one open PR for the PM inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrLifecycleDecision {
    pub class: PrLifecycleClass,
    pub stale: bool,
    pub owner_issue_closed: bool,
    /// The Issue the PM would relaunch or triage for: the first closing
    /// Issue, else the Issue named by the launch ref the head sits on.
    pub owner_issue: Option<u64>,
    pub default_action: String,
    /// Hours since `updated_at` (Issue #3868 AC-4); `None` without a timestamp.
    pub dwell_hours: Option<i64>,
    /// Whether the PM can execute `default_action` through JSON operations.
    pub default_action_executable: bool,
    /// Why the owner cannot be relaunched, when known (Issue #3868 AC-1).
    pub blocker: Option<String>,
    /// The fallback order to apply when `default_action` is not executable.
    pub fallback: Option<String>,
}

/// One open-PR inventory row returned by `pr.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInventoryItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    #[serde(default)]
    pub head_ref_name: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub mergeable: String,
    pub merge_state_status: String,
    pub ci_status: String,
    pub review_status: String,
    pub body: String,
    pub closing_issues: Vec<PrClosingIssue>,
    pub lifecycle: String,
    /// `observed` (classified from this read), `held` (previous class kept
    /// while mergeability is `UNKNOWN` and the real data is unchanged), or
    /// `undetermined` (unknown mergeability with no usable history).
    #[serde(default = "lifecycle_source_observed")]
    pub lifecycle_source: String,
    pub stale: bool,
    pub owner_issue_closed: bool,
    #[serde(default)]
    pub owner_issue: Option<u64>,
    pub default_action: String,
    #[serde(default)]
    pub dwell_hours: Option<i64>,
    #[serde(default = "default_stale_after_hours")]
    pub stale_after_hours: i64,
    #[serde(default = "default_true")]
    pub default_action_executable: bool,
    #[serde(default)]
    pub blocker: Option<String>,
    #[serde(default)]
    pub fallback: Option<String>,
    /// Consecutive `pr.list` observations whose real data did not change.
    #[serde(default)]
    pub unchanged_cycles: u32,
    #[serde(default = "default_escalate_after_cycles")]
    pub escalate_after_cycles: u32,
    /// `stale` or `unchanged_cycles >= escalate_after_cycles`: the row must
    /// reach the human with what was done or why nothing could be done.
    #[serde(default)]
    pub escalation_due: bool,
}

fn lifecycle_source_observed() -> String {
    "observed".to_string()
}

fn default_stale_after_hours() -> i64 {
    PR_STALE_AFTER_HOURS
}

fn default_escalate_after_cycles() -> u32 {
    PR_ESCALATE_AFTER_UNCHANGED_CYCLES
}

fn default_true() -> bool {
    true
}

/// Per-PR memory between `pr.list` calls (Issue #3868 AC-6 and lifecycle
/// stability). Lives in the machine-local project dir, never in the repo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInventoryHistory {
    #[serde(default)]
    pub entries: BTreeMap<u64, PrInventoryHistoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInventoryHistoryEntry {
    pub updated_at: Option<DateTime<Utc>>,
    pub lifecycle: String,
    pub default_action: String,
    pub unchanged_cycles: u32,
    pub last_seen_at: DateTime<Utc>,
}

impl PrInventoryHistory {
    /// Read the history; a missing or corrupt file is an empty history so an
    /// inventory read never fails because of its own bookkeeping.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rendered = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, rendered)?;
        std::fs::rename(&tmp, path)
    }

    /// Fold this read into the history: hold a previous class while GitHub
    /// reports `UNKNOWN` mergeability for unchanged real data, count
    /// unchanged cycles, and forget PRs that left the inventory.
    pub fn observe(
        &mut self,
        items: &mut [PrInventoryItem],
        now: DateTime<Utc>,
        options: &PrInventoryOptions,
    ) {
        let mut next: BTreeMap<u64, PrInventoryHistoryEntry> = BTreeMap::new();
        for item in items.iter_mut() {
            let previous = self.entries.get(&item.number);
            let same_real_data = previous.is_some_and(|entry| entry.updated_at == item.updated_at);
            if item.lifecycle_source == "undetermined" && same_real_data {
                if let Some(entry) = previous {
                    if let Some(class) = PrLifecycleClass::parse(&entry.lifecycle) {
                        if class != PrLifecycleClass::Undetermined {
                            item.apply_held_class(class);
                        }
                    }
                }
            }
            item.unchanged_cycles = if same_real_data {
                previous.map_or(0, |entry| entry.unchanged_cycles.saturating_add(1))
            } else {
                0
            };
            item.escalate_after_cycles = options.escalate_after_cycles;
            item.escalation_due =
                item.stale || item.unchanged_cycles >= options.escalate_after_cycles;
            next.insert(
                item.number,
                PrInventoryHistoryEntry {
                    updated_at: item.updated_at,
                    lifecycle: item.lifecycle.clone(),
                    default_action: item.default_action.clone(),
                    unchanged_cycles: item.unchanged_cycles,
                    last_seen_at: now,
                },
            );
        }
        self.entries = next;
    }
}

impl PrInventoryItem {
    fn fields(&self) -> PrInventoryFields {
        PrInventoryFields {
            number: self.number,
            title: self.title.clone(),
            url: self.url.clone(),
            is_draft: self.is_draft,
            head_ref_name: self.head_ref_name.clone(),
            updated_at: self.updated_at,
            mergeable: self.mergeable.clone(),
            merge_state_status: self.merge_state_status.clone(),
            ci_status: self.ci_status.clone(),
            review_status: self.review_status.clone(),
            body: self.body.clone(),
            closing_issues: self.closing_issues.clone(),
        }
    }

    fn apply_held_class(&mut self, class: PrLifecycleClass) {
        let decision = decide_for_class(&self.fields(), class);
        self.lifecycle = class.as_str().to_string();
        self.lifecycle_source = "held".to_string();
        self.default_action = decision.default_action;
        self.default_action_executable = decision.default_action_executable;
        self.blocker = decision.blocker;
        self.fallback = decision.fallback;
    }
}

fn looks_superseded(title: &str, body: &str) -> bool {
    let mut haystack = String::with_capacity(title.len() + body.len() + 1);
    haystack.push_str(title);
    haystack.push('\n');
    haystack.push_str(body);
    haystack.to_ascii_lowercase().contains("superseded")
}

fn owner_issue_is_closed(issues: &[PrClosingIssue]) -> bool {
    if issues.is_empty() {
        return false;
    }
    issues.iter().all(|issue| {
        issue
            .state
            .as_deref()
            .is_some_and(|state| state.eq_ignore_ascii_case("CLOSED"))
    })
}

fn pr_dwell_hours(updated_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<i64> {
    updated_at.map(|updated| (now - updated).num_hours().max(0))
}

fn mergeability_unknown(fields: &PrInventoryFields) -> bool {
    fields.mergeable.eq_ignore_ascii_case("UNKNOWN")
        || fields.merge_state_status.eq_ignore_ascii_case("UNKNOWN")
}

/// Issue number encoded in a gwt launch ref (`work/issue-<n>` or `issue-<n>`).
fn launch_ref_issue(head_ref_name: &str) -> Option<u64> {
    let tail = head_ref_name
        .strip_prefix("work/issue-")
        .or_else(|| head_ref_name.strip_prefix("issue-"))?;
    (!tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit())).then(|| tail.parse().ok())?
}

/// The Issue the PM would relaunch: the first closing Issue, else the Issue
/// named by the launch ref the PR head sits on.
fn owner_issue(fields: &PrInventoryFields) -> Option<u64> {
    fields
        .closing_issues
        .first()
        .map(|issue| issue.number)
        .or_else(|| launch_ref_issue(&fields.head_ref_name))
}

/// Whether the PR head is the launch ref the Issue Monitor would fresh-launch
/// the owner from — the exact case its unique-commits guard refuses.
fn head_is_owner_launch_ref(fields: &PrInventoryFields) -> bool {
    launch_ref_issue(&fields.head_ref_name).is_some_and(|head_issue| {
        fields.closing_issues.is_empty()
            || fields
                .closing_issues
                .iter()
                .any(|issue| issue.number == head_issue)
    })
}

/// Classify one open PR into the PM inventory taxonomy with default thresholds.
pub fn classify_pr_lifecycle(
    fields: &PrInventoryFields,
    now: DateTime<Utc>,
) -> PrLifecycleDecision {
    classify_pr_lifecycle_with(fields, now, &PrInventoryOptions::default())
}

/// Classify one open PR into the PM inventory taxonomy.
pub fn classify_pr_lifecycle_with(
    fields: &PrInventoryFields,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
) -> PrLifecycleDecision {
    let owner_issue_closed = owner_issue_is_closed(&fields.closing_issues);
    let class = if looks_superseded(&fields.title, &fields.body) || owner_issue_closed {
        PrLifecycleClass::Superseded
    } else if mergeability_unknown(fields) {
        PrLifecycleClass::Undetermined
    } else if fields.mergeable.eq_ignore_ascii_case("CONFLICTING")
        || fields.merge_state_status.eq_ignore_ascii_case("DIRTY")
    {
        PrLifecycleClass::Conflicted
    } else if fields.merge_state_status.eq_ignore_ascii_case("BEHIND") {
        PrLifecycleClass::Behind
    } else if fields.ci_status.eq_ignore_ascii_case("FAILURE") {
        PrLifecycleClass::CiRed
    } else if fields.mergeable.eq_ignore_ascii_case("MERGEABLE")
        && fields.ci_status.eq_ignore_ascii_case("SUCCESS")
        && (fields.merge_state_status.eq_ignore_ascii_case("CLEAN")
            || fields.merge_state_status.is_empty())
    {
        PrLifecycleClass::MergeCandidate
    } else {
        PrLifecycleClass::InProgress
    };
    let mut decision = decide_for_class(fields, class);
    decision.dwell_hours = pr_dwell_hours(fields.updated_at, now);
    decision.stale = decision
        .dwell_hours
        .is_some_and(|hours| hours >= options.stale_after_hours);
    if class == PrLifecycleClass::InProgress && decision.stale {
        decision.default_action = format!("escalate: no update for {}h", options.stale_after_hours);
    }
    decision
}

/// Default action and executability for an already-decided class. `stale`
/// and `dwell_hours` are filled by the caller, which owns the clock.
fn decide_for_class(fields: &PrInventoryFields, class: PrLifecycleClass) -> PrLifecycleDecision {
    let owner_issue_closed = owner_issue_is_closed(&fields.closing_issues);
    let default_action = match (class, fields.is_draft) {
        (PrLifecycleClass::MergeCandidate, true) => "mark ready".to_string(),
        (PrLifecycleClass::MergeCandidate, false) => "propose merge".to_string(),
        (PrLifecycleClass::Conflicted, _) => "relaunch owner to resolve conflict".to_string(),
        (PrLifecycleClass::Behind, _) => "update-branch".to_string(),
        (PrLifecycleClass::CiRed, _) => "relaunch owner to fix CI".to_string(),
        (PrLifecycleClass::Superseded, _) => {
            "propose close in digest (never auto-close)".to_string()
        }
        (PrLifecycleClass::InProgress, _) => "leave in progress".to_string(),
        (PrLifecycleClass::Undetermined, _) => {
            "hold: mergeability not computed yet, re-read next cycle".to_string()
        }
    };
    let owner = owner_issue(fields);
    let blocker = if owner_issue_closed {
        Some("owner_issue_closed")
    } else if class.relaunches_owner() {
        if owner.is_none() {
            Some("owner_unknown")
        } else if head_is_owner_launch_ref(fields) {
            Some("owner_relaunch_refused_unique_commits")
        } else {
            None
        }
    } else {
        None
    };
    let default_action_executable = !(class.relaunches_owner() && blocker.is_some());
    let fallback =
        (!default_action_executable).then(|| PR_FALLBACK_WHEN_NOT_EXECUTABLE.to_string());
    PrLifecycleDecision {
        class,
        stale: false,
        owner_issue_closed,
        owner_issue: owner,
        default_action,
        dwell_hours: None,
        default_action_executable,
        blocker: blocker.map(str::to_string),
        fallback,
    }
}

fn inventory_item_from_fields(
    fields: PrInventoryFields,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
) -> PrInventoryItem {
    let decision = classify_pr_lifecycle_with(&fields, now, options);
    let lifecycle_source = if decision.class == PrLifecycleClass::Undetermined {
        "undetermined"
    } else {
        "observed"
    };
    PrInventoryItem {
        number: fields.number,
        title: fields.title,
        url: fields.url,
        is_draft: fields.is_draft,
        head_ref_name: fields.head_ref_name,
        updated_at: fields.updated_at,
        mergeable: fields.mergeable,
        merge_state_status: fields.merge_state_status,
        ci_status: fields.ci_status,
        review_status: fields.review_status,
        body: fields.body,
        closing_issues: fields.closing_issues,
        lifecycle: decision.class.as_str().to_string(),
        lifecycle_source: lifecycle_source.to_string(),
        stale: decision.stale,
        owner_issue_closed: decision.owner_issue_closed,
        owner_issue: decision.owner_issue,
        default_action: decision.default_action,
        dwell_hours: decision.dwell_hours,
        stale_after_hours: options.stale_after_hours,
        default_action_executable: decision.default_action_executable,
        blocker: decision.blocker,
        fallback: decision.fallback,
        unchanged_cycles: 0,
        escalate_after_cycles: options.escalate_after_cycles,
        escalation_due: decision.stale,
    }
}

fn parse_closing_issues(value: &serde_json::Value) -> Vec<PrClosingIssue> {
    let nodes = match value {
        serde_json::Value::Array(items) => items.as_slice(),
        serde_json::Value::Object(map) => match map.get("nodes").and_then(|nodes| nodes.as_array())
        {
            Some(items) => items.as_slice(),
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    nodes
        .iter()
        .filter_map(|node| {
            let number = node.get("number").and_then(serde_json::Value::as_u64)?;
            let state = node
                .get("state")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            Some(PrClosingIssue { number, state })
        })
        .collect()
}

fn inventory_item_from_value(
    value: &serde_json::Value,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
) -> Result<PrInventoryItem> {
    let single_json = serde_json::to_string(value).map_err(|e| GwtError::Other(e.to_string()))?;
    let status = parse_pr_status_json(&single_json)?;
    let fields = PrInventoryFields {
        number: status.number,
        title: status.title,
        url: status.url,
        is_draft: value
            .get("isDraft")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        head_ref_name: value
            .get("headRefName")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        updated_at: parse_github_timestamp(
            value.get("updatedAt").and_then(serde_json::Value::as_str),
        )
        .or(status.created_at),
        mergeable: status.mergeable,
        merge_state_status: status.merge_state_status,
        ci_status: status.ci_status,
        review_status: status.review_status,
        body: value
            .get("body")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        closing_issues: value
            .get("closingIssuesReferences")
            .map(parse_closing_issues)
            .unwrap_or_default(),
    };
    Ok(inventory_item_from_fields(fields, now, options))
}

/// Parse `gh pr list --json` output into classified inventory rows.
pub fn parse_pr_inventory_json(json: &str, now: DateTime<Utc>) -> Result<Vec<PrInventoryItem>> {
    parse_pr_inventory_json_with(json, now, &PrInventoryOptions::default())
}

/// Parse `gh pr list --json` output with explicit thresholds.
pub fn parse_pr_inventory_json_with(
    json: &str,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
) -> Result<Vec<PrInventoryItem>> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;
    arr.iter()
        .map(|value| inventory_item_from_value(value, now, options))
        .collect()
}

/// The bulk list query (Issue #3891 AC-2): no `body`, no `statusCheckRollup`.
/// Both are hydrated per PR, and only when that PR needs it.
const INVENTORY_LIGHT_JSON_FIELDS: &str = "number,title,url,isDraft,headRefName,createdAt,updatedAt,mergeable,mergeStateStatus,reviewDecision,closingIssuesReferences";
const INVENTORY_LIGHT_JSON_FIELDS_WITHOUT_CLOSING: &str =
    "number,title,url,isDraft,headRefName,createdAt,updatedAt,mergeable,mergeStateStatus,reviewDecision";

/// File under the machine-local project dir that remembers the previous
/// `pr.list` observation per PR (Issue #3868 AC-6).
pub const PR_INVENTORY_HISTORY_FILE: &str = "pr-inventory-history.json";

/// File under the machine-local project dir that holds the last `pr.list`
/// snapshot (Issue #3891 AC-1). Shared by every worktree and session of the
/// repository on this machine, so N readers cost one fetch per TTL.
pub const PR_INVENTORY_CACHE_FILE: &str = "pr-inventory-cache.json";

/// How long a snapshot answers `pr.list` without touching GitHub. One PM
/// cycle: repeated reads inside a cycle are free, and a cycle never sees data
/// older than the previous cycle.
pub const PR_INVENTORY_CACHE_TTL_SECS: i64 = 300;

/// Per-PR hydration calls allowed in one read. Bounds the spawn burst on a
/// cold cache; the rest hydrate on the next read.
const PR_INVENTORY_HYDRATION_CAP: usize = 30;

/// Heavy fields of one PR, keyed by the `updated_at` they were fetched for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrInventoryHeavy {
    pub updated_at: Option<DateTime<Utc>>,
    pub status_check_rollup: Option<serde_json::Value>,
    pub body: Option<String>,
    pub hydrated_at: DateTime<Utc>,
}

/// The persisted `pr.list` snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrInventoryCache {
    pub fetched_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub include: PrInventoryInclude,
    #[serde(default)]
    pub rows: Vec<serde_json::Value>,
    #[serde(default)]
    pub heavy: BTreeMap<u64, PrInventoryHeavy>,
}

impl PrInventoryCache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rendered = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
        std::fs::write(&tmp, rendered)?;
        std::fs::rename(&tmp, path)
    }

    fn age_secs(&self, now: DateTime<Utc>) -> Option<i64> {
        self.fetched_at
            .map(|fetched_at| (now - fetched_at).num_seconds())
    }

    /// Classified rows from the snapshot, exposing only the heavy fields the
    /// caller asked for.
    fn items(
        &self,
        now: DateTime<Utc>,
        options: &PrInventoryOptions,
    ) -> Result<Vec<PrInventoryItem>> {
        self.rows
            .iter()
            .map(|row| {
                let mut row = row.clone();
                let number = row.get("number").and_then(serde_json::Value::as_u64);
                if let Some(heavy) = number.and_then(|number| self.heavy.get(&number)) {
                    if let Some(map) = row.as_object_mut() {
                        if options.include.checks {
                            if let Some(rollup) = &heavy.status_check_rollup {
                                map.insert("statusCheckRollup".to_string(), rollup.clone());
                            }
                        }
                        if options.include.body {
                            if let Some(body) = &heavy.body {
                                map.insert(
                                    "body".to_string(),
                                    serde_json::Value::String(body.clone()),
                                );
                            }
                        }
                    }
                }
                inventory_item_from_value(&row, now, options)
            })
            .collect()
    }
}

/// Result of one `pr.list` read: the rows plus where they came from and what
/// the read cost (Issue #3891 AC-1 / AC-4 observability).
#[derive(Debug, Clone)]
pub struct PrInventoryRead {
    pub items: Vec<PrInventoryItem>,
    /// `github` (fetched now), `cache` (inside the TTL), or `stale-cache`
    /// (served because the budget throttled the fetch).
    pub source: &'static str,
    pub fetched_at: Option<DateTime<Utc>>,
    pub cache_age_secs: Option<i64>,
    /// Why the live fetch was skipped, when it was.
    pub throttled: Option<String>,
    /// Budget-spending `gh` calls this read made (the free probe excluded).
    pub github_calls: u32,
}

/// Fetch open PRs, classify them, and fold the read into the per-project
/// history so `unchanged_cycles`, `escalation_due`, and held classes are
/// meaningful across resident PM cycles. Cache-first and budget-aware
/// (Issue #3891): see [`PrInventoryOptions::refresh`].
pub fn fetch_pr_inventory_tracked(
    repo_path: &Path,
    history_path: &Path,
    cache_path: &Path,
    options: &PrInventoryOptions,
) -> Result<PrInventoryRead> {
    let now = Utc::now();
    let mut read = fetch_pr_inventory_cached_with(
        repo_path,
        cache_path,
        &BudgetLedger::global(),
        now,
        options,
        run_gh_command,
    )?;
    let mut history = PrInventoryHistory::load(history_path);
    history.observe(&mut read.items, now, options);
    if let Err(error) = history.save(history_path) {
        // Bookkeeping never fails the read; the next call simply starts the
        // counters over. gwt-git has no tracing sink, so stderr is the
        // channel envelope callers already collect.
        eprintln!(
            "warning: pr inventory history at {} could not be saved: {error}",
            history_path.display()
        );
    }
    Ok(read)
}

/// The cache-first, budget-aware read behind `pr.list`.
///
/// 1. Inside the TTL (and the cached hydration covers the request) the
///    snapshot answers without any GitHub call.
/// 2. Otherwise, unless `refresh` is set, the shared budget ledger decides
///    whether this non-essential read may spend: below the reserve, in an
///    active refusal window, or during a local burst, the last snapshot is
///    served and the skip is reported; with no snapshot the inventory is
///    unobservable, not empty.
/// 3. A live read is the light list query plus per-PR hydration of the
///    requested heavy fields, only for PRs whose real data changed or whose
///    CI is not final yet.
fn fetch_pr_inventory_cached_with<F>(
    repo_path: &Path,
    cache_path: &Path,
    ledger: &BudgetLedger,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
    mut run_gh: F,
) -> Result<PrInventoryRead>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let mut cache = PrInventoryCache::load(cache_path);
    let cache_age = cache.age_secs(now);
    if !options.refresh {
        if let Some(age) = cache_age {
            if (0..PR_INVENTORY_CACHE_TTL_SECS).contains(&age)
                && cache.include.covers(options.include)
            {
                return Ok(PrInventoryRead {
                    items: cache.items(now, options)?,
                    source: "cache",
                    fetched_at: cache.fetched_at,
                    cache_age_secs: Some(age),
                    throttled: None,
                    github_calls: 0,
                });
            }
        }
        if let Some(reason) = periodic_read_throttle(repo_path, ledger, now, &mut run_gh) {
            if cache.fetched_at.is_some() {
                return Ok(PrInventoryRead {
                    items: cache.items(now, options)?,
                    source: "stale-cache",
                    fetched_at: cache.fetched_at,
                    cache_age_secs: cache_age,
                    throttled: Some(reason),
                    github_calls: 0,
                });
            }
            return Err(GwtError::Git(format!(
                "pr inventory unobservable: the GitHub budget throttled this read and no \
                 cached snapshot exists ({reason}); pass refresh:true only if the decision \
                 at hand needs the live inventory"
            )));
        }
    }

    let (rows, mut github_calls) = fetch_light_inventory_rows_with(repo_path, &mut run_gh)?;
    let mut heavy = BTreeMap::new();
    let mut hydrated = 0usize;
    for row in &rows {
        let Some(number) = row.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let updated_at =
            parse_github_timestamp(row.get("updatedAt").and_then(serde_json::Value::as_str));
        let previous = cache.heavy.remove(&number);
        let mut entry = previous.clone();
        if let Some(fields) = hydration_needed(previous.as_ref(), updated_at, options.include, now)
        {
            if hydrated < PR_INVENTORY_HYDRATION_CAP {
                hydrated += 1;
                github_calls += 1;
                if let Ok((rollup, body)) = hydrate_pr(repo_path, number, fields, &mut run_gh) {
                    entry = Some(PrInventoryHeavy {
                        updated_at,
                        status_check_rollup: rollup.or_else(|| {
                            previous
                                .as_ref()
                                .and_then(|p| p.status_check_rollup.clone())
                        }),
                        body: body.or_else(|| previous.as_ref().and_then(|p| p.body.clone())),
                        hydrated_at: now,
                    });
                }
            }
        }
        if let Some(entry) = entry {
            heavy.insert(number, entry);
        }
    }
    cache = PrInventoryCache {
        fetched_at: Some(now),
        include: options.include,
        rows,
        heavy,
    };
    if let Err(error) = cache.save(cache_path) {
        eprintln!(
            "warning: pr inventory cache at {} could not be saved: {error}",
            cache_path.display()
        );
    }
    Ok(PrInventoryRead {
        items: cache.items(now, options)?,
        source: "github",
        fetched_at: Some(now),
        cache_age_secs: Some(0),
        throttled: None,
        github_calls,
    })
}

/// Issue #3891 AC-4: the reason a periodic read must not spend right now.
/// Refreshes the shared probe first when it is stale — `gh api rate_limit`
/// is free — so the decision rests on a current primary window.
fn periodic_read_throttle<F>(
    repo_path: &Path,
    ledger: &BudgetLedger,
    now: DateTime<Utc>,
    run_gh: &mut F,
) -> Option<String>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let policy = ThrottlePolicy::default();
    let mut snapshot = ledger.snapshot(now);
    if github_budget::probe_is_stale(&snapshot, &policy) {
        if let Ok(output) = run_gh(repo_path, &["api", "rate_limit"]) {
            if output.success {
                if let Some(probe) = github_budget::parse_rate_limit_probe_all(&output.stdout, now)
                {
                    ledger.record_probe(&probe);
                    snapshot = ledger.snapshot(now);
                }
            }
        }
    }
    github_budget::throttle_reason(&snapshot, GitHubQuota::GraphQl, &policy, now)
}

/// Which heavy fields PR `number` must (re-)fetch, or `None` when the cached
/// entry still answers: unchanged real data, final CI, body already held.
fn hydration_needed(
    previous: Option<&PrInventoryHeavy>,
    updated_at: Option<DateTime<Utc>>,
    wanted: PrInventoryInclude,
    now: DateTime<Utc>,
) -> Option<PrInventoryInclude> {
    if wanted.is_empty() {
        return None;
    }
    let Some(previous) = previous else {
        return Some(wanted);
    };
    if previous.updated_at != updated_at {
        return Some(wanted);
    }
    let checks = wanted.checks
        && match &previous.status_check_rollup {
            None => true,
            Some(rollup) => {
                !ci_is_final(&ci_status_from_rollup(Some(rollup)))
                    && (now - previous.hydrated_at).num_seconds() >= PR_INVENTORY_CACHE_TTL_SECS
            }
        };
    let body = wanted.body && previous.body.is_none();
    let needed = PrInventoryInclude { checks, body };
    (!needed.is_empty()).then_some(needed)
}

fn ci_is_final(ci_status: &str) -> bool {
    ci_status.eq_ignore_ascii_case("SUCCESS") || ci_status.eq_ignore_ascii_case("FAILURE")
}

/// One `gh pr view <n> --json <fields>` for the heavy fields.
fn hydrate_pr<F>(
    repo_path: &Path,
    number: u64,
    fields: PrInventoryInclude,
    run_gh: &mut F,
) -> Result<(Option<serde_json::Value>, Option<String>)>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    let json_fields = fields.view_json_fields();
    let output = run_gh(repo_path, &["pr", "view", &number, "--json", &json_fields])?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr view {number} {json_fields}: {}",
            output.stderr.trim()
        )));
    }
    let value: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Other(format!("gh pr view {number} JSON: {error}")))?;
    Ok((
        value.get("statusCheckRollup").cloned(),
        value
            .get("body")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    ))
}

/// The light bulk query, with the pre-#3891 fallback for a `gh` that does not
/// know `closingIssuesReferences`. Returns the raw rows and the number of
/// budget-spending calls made.
fn fetch_light_inventory_rows_with<F>(
    repo_path: &Path,
    run_gh: &mut F,
) -> Result<(Vec<serde_json::Value>, u32)>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let primary = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            INVENTORY_LIGHT_JSON_FIELDS,
        ],
    )?;
    if primary.success {
        return Ok((parse_inventory_rows(&primary.stdout)?, 1));
    }
    let fallback = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            INVENTORY_LIGHT_JSON_FIELDS_WITHOUT_CLOSING,
        ],
    )?;
    if !fallback.success {
        return Err(GwtError::Git(format!(
            "gh pr list inventory: {}",
            fallback.stderr.trim()
        )));
    }
    Ok((parse_inventory_rows(&fallback.stdout)?, 2))
}

fn parse_inventory_rows(json: &str) -> Result<Vec<serde_json::Value>> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;
    match value {
        serde_json::Value::Array(rows) => Ok(rows),
        _ => Err(GwtError::Other(
            "gh pr list JSON: expected an array".to_string(),
        )),
    }
}

/// Uncached light read: list rows classified without any heavy hydration.
#[cfg(test)]
fn fetch_pr_inventory_with<F>(
    repo_path: &Path,
    now: DateTime<Utc>,
    options: &PrInventoryOptions,
    mut run_gh: F,
) -> Result<Vec<PrInventoryItem>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let (rows, _) = fetch_light_inventory_rows_with(repo_path, &mut run_gh)?;
    rows.iter()
        .map(|row| inventory_item_from_value(row, now, options))
        .collect()
}

/// Fetch the status of a PR by number using `gh pr view --json`.
///
/// The `repo_slug` should be in "owner/repo" format.
pub fn fetch_pr_status(repo_slug: &str, number: u64) -> Result<PrStatus> {
    let hub = gwt_core::process_console::global();
    let args = [
        "pr",
        "view",
        &number.to_string(),
        "--repo",
        repo_slug,
        "--json",
        "number,title,state,url,createdAt,mergeable,mergeStateStatus,statusCheckRollup,reviewDecision",
    ];
    let label = format!("gh pr view {}", number);
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &args,
        gwt_core::process_console::SpawnOptions::new(label),
    )
    .map_err(|e| GwtError::Git(format!("gh pr view: {e}")))?;

    if !output.success() {
        return Err(GwtError::Git(format!("gh pr view: {}", output.stderr)));
    }

    parse_pr_status_json(&output.stdout)
}

/// Parse `gh pr view --json` output.
pub fn parse_pr_status_json(json: &str) -> Result<PrStatus> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(e.to_string()))?;

    let number = v["number"].as_u64().unwrap_or(0);
    let title = v["title"].as_str().unwrap_or("").to_string();
    let state_str = v["state"].as_str().unwrap_or("OPEN");
    let state = match state_str {
        "MERGED" => PrState::Merged,
        "CLOSED" => PrState::Closed,
        _ => PrState::Open,
    };
    let url = v["url"].as_str().unwrap_or("").to_string();
    let created_at = parse_github_timestamp(v.get("createdAt").and_then(|value| value.as_str()));
    let mergeable = v["mergeable"].as_str().unwrap_or("UNKNOWN").to_string();
    let merge_state_status = v["mergeStateStatus"]
        .as_str()
        .unwrap_or("UNKNOWN")
        .to_string();

    let ci_status = ci_status_from_rollup(v.get("statusCheckRollup"));

    let review_status = v["reviewDecision"]
        .as_str()
        .unwrap_or("UNKNOWN")
        .to_string();

    Ok(PrStatus {
        number,
        title,
        state,
        url,
        created_at,
        ci_status,
        mergeable,
        merge_state_status,
        review_status,
    })
}

/// Determine CI status from a `statusCheckRollup` array.
fn ci_status_from_rollup(rollup: Option<&serde_json::Value>) -> String {
    rollup
        .and_then(serde_json::Value::as_array)
        .map(|checks| {
            if checks.is_empty() {
                return "UNKNOWN".to_string();
            }
            let any_failure = checks.iter().any(|c| {
                c["conclusion"].as_str() == Some("FAILURE")
                    || c["conclusion"].as_str() == Some("failure")
            });
            let any_pending = checks.iter().any(|c| {
                c["status"].as_str() == Some("IN_PROGRESS")
                    || c["status"].as_str() == Some("QUEUED")
                    || c["conclusion"].is_null()
            });
            if any_failure {
                "FAILURE".to_string()
            } else if any_pending {
                "PENDING".to_string()
            } else {
                "SUCCESS".to_string()
            }
        })
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

fn parse_github_timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

pub fn latest_pr_by_created_at(prs: impl IntoIterator<Item = PrStatus>) -> Option<PrStatus> {
    prs.into_iter().max_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.number.cmp(&right.number))
    })
}

fn effective_merge_status_label<'a>(mergeable: &'a str, merge_state_status: &'a str) -> &'a str {
    match merge_state_status {
        "BEHIND" => "BEHIND",
        "" | "UNKNOWN" => {
            if mergeable.is_empty() {
                "UNKNOWN"
            } else {
                mergeable
            }
        }
        _ if mergeable.is_empty() || mergeable == "UNKNOWN" => merge_state_status,
        _ => mergeable,
    }
}

/// Fetch a list of PRs for the repository at `repo_path`.
///
/// Uses the GitHub CLI's `pr list --json` surface as the primary path and
/// falls back to the REST pulls endpoint when that surface is unavailable.
pub fn fetch_pr_list(repo_path: &Path) -> Result<Vec<PrStatus>> {
    fetch_pr_list_with(repo_path, run_gh_command)
}

/// Parse `gh pr list --json` output (a JSON array) into a `Vec<PrStatus>`.
pub fn parse_pr_list_json(json: &str) -> Result<Vec<PrStatus>> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;

    let mut results = Vec::with_capacity(arr.len());
    for v in &arr {
        // Reuse the single-PR parser by serializing back to string
        let single_json = serde_json::to_string(v).map_err(|e| GwtError::Other(e.to_string()))?;
        results.push(parse_pr_status_json(&single_json)?);
    }
    Ok(results)
}

/// SPEC-3075: map each branch (PR head ref) to its PR title, fetched in ONE
/// `gh pr list` call (the GitHub API may paginate). A PR title is the
/// human-written purpose of the work, so it is the top-priority "what work was
/// running" summary for the Workspace rail. Returns an empty map offline / when
/// `gh` is unavailable. When a branch has several PRs the most recent (highest
/// number) wins.
pub fn fetch_pr_titles_by_branch(
    repo_path: &Path,
) -> Result<std::collections::HashMap<String, String>> {
    fetch_pr_titles_by_branch_with(repo_path, run_gh_command)
}

fn fetch_pr_titles_by_branch_with<F>(
    repo_path: &Path,
    mut run_gh: F,
) -> Result<std::collections::HashMap<String, String>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let output = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--json",
            "number,title,headRefName,state",
            "--state",
            "all",
            "--limit",
            "999",
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr list titles: {}",
            output.stderr.trim()
        )));
    }
    parse_pr_titles_by_branch(&output.stdout)
}

/// Parse `gh pr list --json number,title,headRefName,...` into a
/// `branch -> PR title` map. The highest PR number per branch wins.
pub fn parse_pr_titles_by_branch(json: &str) -> Result<std::collections::HashMap<String, String>> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;
    let mut best: std::collections::HashMap<String, (u64, String)> =
        std::collections::HashMap::new();
    for value in &arr {
        let branch = value
            .get("headRefName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let title = value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let number = value
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let (Some(branch), Some(title)) = (branch, title) else {
            continue;
        };
        match best.get(branch) {
            Some((seen, _)) if *seen >= number => {}
            _ => {
                best.insert(branch.to_string(), (number, title.to_string()));
            }
        }
    }
    Ok(best
        .into_iter()
        .map(|(branch, (_, title))| (branch, title))
        .collect())
}

/// Branches (PR head refs) whose PR has merged, fetched in ONE `gh pr list`
/// call. A transient failure returns an `Err` (the caller keeps work as
/// launched) rather than an empty set, so closing the active slot only happens
/// on a positive merge signal.
pub fn fetch_merged_pr_branches(repo_path: &Path) -> Result<std::collections::BTreeSet<String>> {
    fetch_merged_pr_branches_with(repo_path, run_gh_command)
}

fn fetch_merged_pr_branches_with<F>(
    repo_path: &Path,
    mut run_gh: F,
) -> Result<std::collections::BTreeSet<String>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let output = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--json",
            "headRefName,state",
            "--state",
            "merged",
            "--limit",
            "999",
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr list merged: {}",
            output.stderr.trim()
        )));
    }
    parse_merged_pr_branches(&output.stdout)
}

/// Parse `gh pr list --json headRefName,state` into the set of branches whose
/// PR state is `MERGED`.
pub fn parse_merged_pr_branches(json: &str) -> Result<std::collections::BTreeSet<String>> {
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr list JSON: {e}")))?;
    let mut branches = std::collections::BTreeSet::new();
    for value in &arr {
        let merged = value
            .get("state")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("merged"));
        if !merged {
            continue;
        }
        if let Some(branch) = value
            .get("headRefName")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            branches.insert(branch.to_string());
        }
    }
    Ok(branches)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GhCliOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Authoritative remote state used to reconcile an auto-merge effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrAutoMergeRemoteState {
    Open {
        head_sha: String,
        auto_merge_requested: bool,
    },
    Merged {
        head_sha: String,
    },
    Closed {
        head_sha: String,
    },
}

/// Result of a single auto-merge mutation attempt.
///
/// `RemoteOutcomeUnknown` is deliberately distinct from `PreSubmit`: callers
/// must read the remote PR state before retrying an attempt whose process may
/// have submitted the mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoMergeMutationOutcome {
    Confirmed,
    AlreadyTargetState,
    PreSubmit(String),
    RemoteOutcomeUnknown(String),
    HeadChanged { expected: String, actual: String },
    AuthorityMismatch(String),
}

impl AutoMergeMutationOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Confirmed | Self::AlreadyTargetState)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoMergeCommandError {
    PreSubmit(String),
    RemoteOutcomeUnknown(String),
}

fn fetch_pr_list_with<F>(repo_path: &Path, mut run_gh: F) -> Result<Vec<PrStatus>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let primary = run_gh(
        repo_path,
        &[
            "pr",
            "list",
            "--json",
            "number,title,state,url,createdAt,statusCheckRollup,mergeable,mergeStateStatus,reviewDecision",
            "--state",
            "all",
            "--limit",
            "20",
        ],
    );

    if let Ok(output) = primary {
        if output.success {
            if let Ok(prs) = parse_pr_list_json(&output.stdout) {
                return Ok(prs);
            }
        }
    }

    let rest = run_gh(
        repo_path,
        &["api", "repos/{owner}/{repo}/pulls?state=all&per_page=20"],
    )?;
    if !rest.success {
        return Err(GwtError::Git(format!(
            "gh api pulls: {}",
            rest.stderr.trim()
        )));
    }
    parse_rest_pr_list_json(&rest.stdout)
}

fn run_gh_command(repo_path: &Path, args: &[&str]) -> Result<GhCliOutput> {
    run_gh_command_with(repo_path, args, spawn_gh_command)
}

fn run_gh_command_with<F>(repo_path: &Path, args: &[&str], mut spawn_gh: F) -> Result<GhCliOutput>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let command_root =
        crate::worktree::main_worktree_root(repo_path).unwrap_or_else(|_| repo_path.to_path_buf());
    spawn_gh(&command_root, args)
}

fn spawn_gh_command(repo_path: &Path, args: &[&str]) -> Result<GhCliOutput> {
    let hub = gwt_core::process_console::global();
    let label = format!("gh {}", args.join(" "));
    let options =
        gwt_core::process_console::SpawnOptions::new(label.clone()).current_dir(repo_path);
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        args,
        options,
    )
    .map_err(|e| GwtError::Git(format!("{label}: {e}")))?;

    Ok(GhCliOutput {
        success: output.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn parse_rest_pr_list_json(json: &str) -> Result<Vec<PrStatus>> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| GwtError::Other(format!("gh api pulls JSON: {e}")))?;

    Ok(arr
        .into_iter()
        .map(|v| {
            let state = if v
                .get("merged_at")
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty())
            {
                PrState::Merged
            } else {
                match v.get("state").and_then(|s| s.as_str()).unwrap_or("open") {
                    "closed" => PrState::Closed,
                    "merged" => PrState::Merged,
                    _ => PrState::Open,
                }
            };
            PrStatus {
                number: v
                    .get("number")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                title: v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                state,
                url: v
                    .get("html_url")
                    .or_else(|| v.get("url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                created_at: parse_github_timestamp(
                    v.get("created_at").and_then(|value| value.as_str()),
                ),
                ci_status: "UNKNOWN".to_string(),
                mergeable: "UNKNOWN".to_string(),
                merge_state_status: "UNKNOWN".to_string(),
                review_status: "UNKNOWN".to_string(),
            }
        })
        .collect())
}

// ── Extended PR check report ──

/// PR status check states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Passing,
    Failing,
    Pending,
    Unknown,
}

/// Merge readiness states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    Ready,
    Behind,
    Blocked,
    Conflicts,
    Unknown,
}

/// Review states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Approved,
    ChangesRequested,
    Pending,
    Unknown,
}

/// Extended PR status report.
#[derive(Debug, Clone)]
pub struct PrCheckReport {
    pub ci: CiStatus,
    pub merge: MergeStatus,
    pub review: ReviewStatus,
    pub summary: String,
}

/// Generate an extended PR status report by inspecting the repository.
///
/// Runs `gh pr view` to gather CI, merge, and review states. Falls back
/// to `Unknown` states when `gh` is unavailable or the repo has no open PR.
pub fn pr_check_report(repo_path: &Path) -> Result<PrCheckReport> {
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &[
            "pr",
            "view",
            "--json",
            "statusCheckRollup,mergeable,mergeStateStatus,reviewDecision,state,title",
        ],
        gwt_core::process_console::SpawnOptions::new("gh pr view --json").current_dir(repo_path),
    )
    .map_err(|e| GwtError::Git(format!("gh pr view: {e}")))?;

    if !output.success() {
        return Ok(PrCheckReport {
            ci: CiStatus::Unknown,
            merge: MergeStatus::Unknown,
            review: ReviewStatus::Unknown,
            summary: format!("No open PR or gh error: {}", output.stderr.trim()),
        });
    }

    parse_pr_check_report_json(&output.stdout)
}

/// Parse `gh pr view --json` output into an extended PR check report.
pub fn parse_pr_check_report_json(json: &str) -> Result<PrCheckReport> {
    let json: serde_json::Value =
        serde_json::from_str(json).map_err(|e| GwtError::Other(format!("gh pr view JSON: {e}")))?;

    let ci = match json.get("statusCheckRollup") {
        Some(serde_json::Value::Array(checks)) => {
            let all_pass = checks.iter().all(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "SUCCESS" || s == "NEUTRAL" || s == "SKIPPED")
            });
            let any_fail = checks.iter().any(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "FAILURE" || s == "CANCELLED" || s == "TIMED_OUT")
            });
            if checks.is_empty() {
                CiStatus::Pending
            } else if any_fail {
                CiStatus::Failing
            } else if all_pass {
                CiStatus::Passing
            } else {
                CiStatus::Pending
            }
        }
        _ => CiStatus::Unknown,
    };

    let mergeable = json
        .get("mergeable")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let merge_state_status = json
        .get("mergeStateStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let merge = match effective_merge_status_label(mergeable, merge_state_status) {
        "MERGEABLE" => MergeStatus::Ready,
        "BEHIND" => MergeStatus::Behind,
        "CONFLICTING" | "DIRTY" => MergeStatus::Conflicts,
        "UNKNOWN" => MergeStatus::Unknown,
        _ => MergeStatus::Blocked,
    };

    let review = match json.get("reviewDecision").and_then(|v| v.as_str()) {
        Some("APPROVED") => ReviewStatus::Approved,
        Some("CHANGES_REQUESTED") => ReviewStatus::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewStatus::Pending,
        _ => ReviewStatus::Unknown,
    };

    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(untitled)");

    let summary = format!("PR: {title} | CI: {ci:?} | Merge: {merge:?} | Review: {review:?}");

    Ok(PrCheckReport {
        ci,
        merge,
        review,
        summary,
    })
}

// ---------------------------------------------------------------------------
// SPEC #3200 — gate-input adapters for the autonomous strong gate.
//
// These feed the PURE gate functions (classify_ci_rollup / build_review_prompt /
// evaluate_autonomous_gate). They are FAIL-CLOSED: a missing PR, a gh failure,
// or unparseable output yields `None`/empty so the gate treats the input as
// unavailable rather than as a pass.
// ---------------------------------------------------------------------------

/// Parse `gh pr list --head <branch> --json number` into the open PR number for
/// that work branch. Returns the highest number when several exist (the most
/// recent reopen). `None` when the array is empty or unparseable.
pub fn parse_open_pr_number(json: &str) -> Option<u64> {
    let arr: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    arr.iter()
        .filter_map(|value| value.get("number").and_then(serde_json::Value::as_u64))
        .max()
}

/// Parse `gh pr view <n> --json headRefOid` into the PR head SHA. `None` when
/// absent/empty/unparseable.
pub fn parse_pr_head_sha(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .get("headRefOid")
        .and_then(serde_json::Value::as_str)
        .filter(|sha| !sha.trim().is_empty())
        .map(str::to_string)
}

/// Extract the `statusCheckRollup` array (as a JSON string) from a
/// `gh pr view <n> --json statusCheckRollup` body, ready to hand to
/// `classify_ci_rollup`. A missing/null rollup becomes `"[]"` (which the
/// classifier treats as no checks → vacuous → fail-closed).
pub fn extract_status_check_rollup(json: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(json).ok();
    match parsed
        .as_ref()
        .and_then(|value| value.get("statusCheckRollup"))
    {
        Some(rollup) if rollup.is_array() => rollup.to_string(),
        _ => "[]".to_string(),
    }
}

/// Find the open PR number for a work `branch` (fail-closed `Option`).
pub fn fetch_open_pr_number_for_branch(repo_path: &Path, branch: &str) -> Option<u64> {
    try_fetch_open_pr_number_for_branch(repo_path, branch)
        .ok()
        .flatten()
}

/// Checked variant used by scan transactions that must preserve transport and
/// deadline failures instead of conflating them with "no open PR".
pub fn try_fetch_open_pr_number_for_branch(repo_path: &Path, branch: &str) -> Result<Option<u64>> {
    try_fetch_open_pr_number_for_branch_with(repo_path, branch, run_gh_command)
}

fn try_fetch_open_pr_number_for_branch_with<F>(
    repo_path: &Path,
    branch: &str,
    mut run_gh: F,
) -> Result<Option<u64>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let output = run_gh(
        repo_path,
        &[
            "pr", "list", "--head", branch, "--state", "open", "--json", "number",
        ],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr list open branch: {}",
            output.stderr.trim()
        )));
    }
    let _: Vec<serde_json::Value> = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Other(format!("gh pr list open branch JSON: {error}")))?;
    Ok(parse_open_pr_number(&output.stdout))
}

#[cfg(test)]
fn fetch_open_pr_number_for_branch_with<F>(repo_path: &Path, branch: &str, run_gh: F) -> Option<u64>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    try_fetch_open_pr_number_for_branch_with(repo_path, branch, run_gh)
        .ok()
        .flatten()
}

/// Fetch a PR's head SHA — the SHA the gate binds the review/merge to
/// (fail-closed `Option`).
pub fn fetch_pr_head_sha(repo_path: &Path, number: u64) -> Option<String> {
    try_fetch_pr_head_sha(repo_path, number).ok().flatten()
}

/// Checked variant used by deadline-integral scans.
pub fn try_fetch_pr_head_sha(repo_path: &Path, number: u64) -> Result<Option<String>> {
    try_fetch_pr_head_sha_with(repo_path, number, run_gh_command)
}

fn try_fetch_pr_head_sha_with<F>(
    repo_path: &Path,
    number: u64,
    mut run_gh: F,
) -> Result<Option<String>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    let output = run_gh(repo_path, &["pr", "view", &number, "--json", "headRefOid"])?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr view headRefOid: {}",
            output.stderr.trim()
        )));
    }
    let _: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Other(format!("gh pr view headRefOid JSON: {error}")))?;
    Ok(parse_pr_head_sha(&output.stdout))
}

/// Fetch a PR's `statusCheckRollup` array JSON (fail-closed: `"[]"` on any
/// failure so the gate treats CI as vacuous, never a pass).
pub fn fetch_pr_status_check_rollup(repo_path: &Path, number: u64) -> String {
    try_fetch_pr_status_check_rollup(repo_path, number).unwrap_or_else(|_| "[]".to_string())
}

/// Checked variant used by deadline-integral scans.
pub fn try_fetch_pr_status_check_rollup(repo_path: &Path, number: u64) -> Result<String> {
    try_fetch_pr_status_check_rollup_with(repo_path, number, run_gh_command)
}

fn try_fetch_pr_status_check_rollup_with<F>(
    repo_path: &Path,
    number: u64,
    mut run_gh: F,
) -> Result<String>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    let output = run_gh(
        repo_path,
        &["pr", "view", &number, "--json", "statusCheckRollup"],
    )?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr view statusCheckRollup: {}",
            output.stderr.trim()
        )));
    }
    let _: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Other(format!("gh pr view statusCheckRollup JSON: {error}")))?;
    Ok(extract_status_check_rollup(&output.stdout))
}

#[cfg(test)]
fn fetch_pr_status_check_rollup_with<F>(repo_path: &Path, number: u64, run_gh: F) -> String
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    try_fetch_pr_status_check_rollup_with(repo_path, number, run_gh)
        .unwrap_or_else(|_| "[]".to_string())
}

/// Fetch a PR's unified diff for the independent review agent. Capped to
/// `max_bytes` (truncated with a marker) so a huge diff never blows the prompt.
/// `None` on any gh failure (the review then runs without a diff and, being
/// adversarial + fail-closed, will reject).
pub fn fetch_pr_diff(repo_path: &Path, number: u64, max_bytes: usize) -> Option<String> {
    try_fetch_pr_diff(repo_path, number, max_bytes)
        .ok()
        .flatten()
}

/// Checked variant used by deadline-integral scans.
pub fn try_fetch_pr_diff(
    repo_path: &Path,
    number: u64,
    max_bytes: usize,
) -> Result<Option<String>> {
    try_fetch_pr_diff_with(repo_path, number, max_bytes, run_gh_command)
}

fn try_fetch_pr_diff_with<F>(
    repo_path: &Path,
    number: u64,
    max_bytes: usize,
    mut run_gh: F,
) -> Result<Option<String>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    let output = run_gh(repo_path, &["pr", "diff", &number])?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr diff: {}",
            output.stderr.trim()
        )));
    }
    Ok(Some(truncate_diff(&output.stdout, max_bytes)))
}

#[cfg(test)]
fn fetch_pr_diff_with<F>(
    repo_path: &Path,
    number: u64,
    max_bytes: usize,
    run_gh: F,
) -> Option<String>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    try_fetch_pr_diff_with(repo_path, number, max_bytes, run_gh)
        .ok()
        .flatten()
}

fn truncate_diff(diff: &str, max_bytes: usize) -> String {
    if diff.len() <= max_bytes {
        return diff.to_string();
    }
    // Truncate on a char boundary to keep valid UTF-8.
    let mut end = max_bytes;
    while end > 0 && !diff.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n... [diff truncated at {max_bytes} bytes] ...",
        &diff[..end]
    )
}

/// SPEC #3200: arm auto-merge for an autonomous PR. This is the IRREVERSIBLE
/// action — callers MUST have run the strong gate first. Uses GitHub's native
/// `--auto` so the merge only proceeds once branch-protection's required checks
/// pass (a second, GitHub-enforced layer behind our gate). Returns `false` on
/// any gh failure (fail-closed: never report a merge armed when it was not).
///
/// `reviewed_head_sha` binds the arm to the exact head the gate reviewed via
/// `--match-head-commit`: if the PR head advances between review and merge,
/// GitHub REFUSES to merge the unreviewed commit. This is prevention at the
/// merge boundary (vs the post-merge layer-4 detection), closing the
/// review→merge TOCTOU window.
pub fn merge_pr_auto(repo_path: &Path, number: u64, reviewed_head_sha: &str) -> bool {
    merge_pr_auto_with(repo_path, number, reviewed_head_sha, run_gh_command)
}

/// Fetch the fields needed to reconcile an auto-merge effect after an
/// ambiguous attempt or daemon restart.
pub fn fetch_pr_auto_merge_remote_state(
    repo_path: &Path,
    number: u64,
) -> Option<PrAutoMergeRemoteState> {
    let number = number.to_string();
    let output = run_gh_command(
        repo_path,
        &[
            "pr",
            "view",
            &number,
            "--json",
            "state,headRefOid,autoMergeRequest,mergeCommit",
        ],
    )
    .ok()?;
    if !output.success {
        return None;
    }
    parse_pr_auto_merge_remote_state(&output.stdout)
}

/// Parse an authoritative `gh pr view` response for auto-merge reconciliation.
pub fn parse_pr_auto_merge_remote_state(json: &str) -> Option<PrAutoMergeRemoteState> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let head_sha = value
        .get("headRefOid")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|sha| !sha.is_empty())?
        .to_string();
    let state = value.get("state").and_then(serde_json::Value::as_str)?;

    if state.eq_ignore_ascii_case("open") {
        Some(PrAutoMergeRemoteState::Open {
            head_sha,
            auto_merge_requested: value
                .get("autoMergeRequest")
                .is_some_and(|request| !request.is_null()),
        })
    } else if state.eq_ignore_ascii_case("merged") {
        Some(PrAutoMergeRemoteState::Merged { head_sha })
    } else if state.eq_ignore_ascii_case("closed") {
        Some(PrAutoMergeRemoteState::Closed { head_sha })
    } else {
        None
    }
}

/// Arm auto-merge only when the current remote HEAD still matches the SHA that
/// passed review. The caller supplies a fresh remote readback so retries can
/// first distinguish an already-applied target state.
pub fn arm_pr_auto_merge(
    repo_path: &Path,
    number: u64,
    reviewed_head_sha: &str,
    remote: &PrAutoMergeRemoteState,
) -> AutoMergeMutationOutcome {
    arm_pr_auto_merge_with(
        repo_path,
        number,
        reviewed_head_sha,
        remote,
        run_auto_merge_command,
    )
}

fn arm_pr_auto_merge_with<F>(
    repo_path: &Path,
    number: u64,
    reviewed_head_sha: &str,
    remote: &PrAutoMergeRemoteState,
    mut run_gh: F,
) -> AutoMergeMutationOutcome
where
    F: FnMut(&Path, &[&str]) -> std::result::Result<GhCliOutput, AutoMergeCommandError>,
{
    let (head_sha, auto_merge_requested) = match remote {
        PrAutoMergeRemoteState::Open {
            head_sha,
            auto_merge_requested,
        } => (head_sha, *auto_merge_requested),
        PrAutoMergeRemoteState::Merged { head_sha } => {
            if head_sha != reviewed_head_sha {
                return AutoMergeMutationOutcome::HeadChanged {
                    expected: reviewed_head_sha.to_string(),
                    actual: head_sha.clone(),
                };
            }
            return AutoMergeMutationOutcome::AlreadyTargetState;
        }
        PrAutoMergeRemoteState::Closed { .. } => {
            return AutoMergeMutationOutcome::AuthorityMismatch(
                "pull request is closed".to_string(),
            );
        }
    };
    if head_sha != reviewed_head_sha {
        return AutoMergeMutationOutcome::HeadChanged {
            expected: reviewed_head_sha.to_string(),
            actual: head_sha.clone(),
        };
    }
    if auto_merge_requested {
        return AutoMergeMutationOutcome::AlreadyTargetState;
    }

    let number = number.to_string();
    classify_auto_merge_command(run_gh(
        repo_path,
        &[
            "pr",
            "merge",
            &number,
            "--auto",
            "--squash",
            "--match-head-commit",
            reviewed_head_sha,
        ],
    ))
}

fn merge_pr_auto_with<F>(
    repo_path: &Path,
    number: u64,
    reviewed_head_sha: &str,
    mut run_gh: F,
) -> bool
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    matches!(
        run_gh(
            repo_path,
            &[
                "pr",
                "merge",
                &number,
                "--auto",
                "--squash",
                "--match-head-commit",
                reviewed_head_sha,
            ],
        ),
        Ok(output) if output.success
    )
}

/// Disarm a previously-armed auto-merge (SPEC #3200 FR-024: HEAD advanced past
/// the reviewed SHA ⇒ revoke). Fail-closed `bool`.
pub fn disable_pr_auto_merge(repo_path: &Path, number: u64) -> bool {
    disable_pr_auto_merge_with(repo_path, number, run_gh_command)
}

/// Disarm auto-merge idempotently from a fresh remote state readback.
pub fn disarm_pr_auto_merge(
    repo_path: &Path,
    number: u64,
    remote: &PrAutoMergeRemoteState,
) -> AutoMergeMutationOutcome {
    disarm_pr_auto_merge_with(repo_path, number, remote, run_auto_merge_command)
}

fn disarm_pr_auto_merge_with<F>(
    repo_path: &Path,
    number: u64,
    remote: &PrAutoMergeRemoteState,
    mut run_gh: F,
) -> AutoMergeMutationOutcome
where
    F: FnMut(&Path, &[&str]) -> std::result::Result<GhCliOutput, AutoMergeCommandError>,
{
    match remote {
        PrAutoMergeRemoteState::Open {
            auto_merge_requested: false,
            ..
        }
        | PrAutoMergeRemoteState::Closed { .. } => {
            return AutoMergeMutationOutcome::AlreadyTargetState;
        }
        PrAutoMergeRemoteState::Merged { .. } => {
            return AutoMergeMutationOutcome::AuthorityMismatch(
                "pull request merged before kill-switch disarm was confirmed".to_string(),
            );
        }
        PrAutoMergeRemoteState::Open {
            auto_merge_requested: true,
            ..
        } => {}
    }

    let number = number.to_string();
    classify_auto_merge_command(run_gh(
        repo_path,
        &["pr", "merge", &number, "--disable-auto"],
    ))
}

fn classify_auto_merge_command(
    result: std::result::Result<GhCliOutput, AutoMergeCommandError>,
) -> AutoMergeMutationOutcome {
    match result {
        Ok(output) if output.success => AutoMergeMutationOutcome::Confirmed,
        Ok(output) => AutoMergeMutationOutcome::RemoteOutcomeUnknown(output.stderr),
        Err(AutoMergeCommandError::PreSubmit(message)) => {
            AutoMergeMutationOutcome::PreSubmit(message)
        }
        Err(AutoMergeCommandError::RemoteOutcomeUnknown(message)) => {
            AutoMergeMutationOutcome::RemoteOutcomeUnknown(message)
        }
    }
}

fn run_auto_merge_command(
    repo_path: &Path,
    args: &[&str],
) -> std::result::Result<GhCliOutput, AutoMergeCommandError> {
    let hub = gwt_core::process_console::global();
    let label = format!("gh {}", args.join(" "));
    let options =
        gwt_core::process_console::SpawnOptions::new(label.clone()).current_dir(repo_path);
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        args,
        options,
    )
    .map_err(|error| {
        let message = format!("{label}: {error}");
        match error.kind() {
            std::io::ErrorKind::NotFound
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::InvalidInput => AutoMergeCommandError::PreSubmit(message),
            _ => AutoMergeCommandError::RemoteOutcomeUnknown(message),
        }
    })?;

    Ok(GhCliOutput {
        success: output.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn disable_pr_auto_merge_with<F>(repo_path: &Path, number: u64, mut run_gh: F) -> bool
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    matches!(
        run_gh(repo_path, &["pr", "merge", &number, "--disable-auto"]),
        Ok(output) if output.success
    )
}

/// Parse `gh pr view <n> --json mergeCommit` into the merge commit SHA. Used for
/// the SPEC #3200 layer-4 check (merged SHA must equal the reviewed SHA). `None`
/// until the PR has actually merged.
pub fn parse_pr_merge_commit_sha(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    value
        .get("mergeCommit")
        .and_then(|mc| mc.get("oid"))
        .and_then(serde_json::Value::as_str)
        .filter(|sha| !sha.trim().is_empty())
        .map(str::to_string)
}

/// Fetch a merged PR's merge-commit SHA (fail-closed `Option`).
pub fn fetch_pr_merge_commit_sha(repo_path: &Path, number: u64) -> Option<String> {
    try_fetch_pr_merge_commit_sha(repo_path, number)
        .ok()
        .flatten()
}

/// Checked variant used by deadline-integral scans.
pub fn try_fetch_pr_merge_commit_sha(repo_path: &Path, number: u64) -> Result<Option<String>> {
    try_fetch_pr_merge_commit_sha_with(repo_path, number, run_gh_command)
}

fn try_fetch_pr_merge_commit_sha_with<F>(
    repo_path: &Path,
    number: u64,
    mut run_gh: F,
) -> Result<Option<String>>
where
    F: FnMut(&Path, &[&str]) -> Result<GhCliOutput>,
{
    let number = number.to_string();
    let output = run_gh(repo_path, &["pr", "view", &number, "--json", "mergeCommit"])?;
    if !output.success {
        return Err(GwtError::Git(format!(
            "gh pr view mergeCommit: {}",
            output.stderr.trim()
        )));
    }
    let _: serde_json::Value = serde_json::from_str(&output.stdout)
        .map_err(|error| GwtError::Other(format!("gh pr view mergeCommit JSON: {error}")))?;
    Ok(parse_pr_merge_commit_sha(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_pr_readbacks_preserve_runner_failures() {
        let failure = || Err(GwtError::Git("operation deadline expired".to_string()));

        assert!(try_fetch_open_pr_number_for_branch_with(
            Path::new("/repo"),
            "work/issue-42",
            |_, _| failure(),
        )
        .expect_err("open PR readback must preserve failure")
        .to_string()
        .contains("deadline"));
        assert!(
            try_fetch_pr_head_sha_with(Path::new("/repo"), 42, |_, _| failure())
                .expect_err("head SHA readback must preserve failure")
                .to_string()
                .contains("deadline")
        );
        assert!(
            try_fetch_pr_status_check_rollup_with(Path::new("/repo"), 42, |_, _| failure(),)
                .expect_err("rollup readback must preserve failure")
                .to_string()
                .contains("deadline")
        );
        assert!(
            try_fetch_pr_diff_with(Path::new("/repo"), 42, 1024, |_, _| failure())
                .expect_err("diff readback must preserve failure")
                .to_string()
                .contains("deadline")
        );
        assert!(
            try_fetch_pr_merge_commit_sha_with(Path::new("/repo"), 42, |_, _| failure())
                .expect_err("merge readback must preserve failure")
                .to_string()
                .contains("deadline")
        );
    }

    #[test]
    fn parse_pr_titles_by_branch_maps_head_ref_to_title_and_prefers_latest() {
        let json = r#"[
            {"number": 10, "title": "feat: old work on A", "headRefName": "work/a", "state": "CLOSED"},
            {"number": 42, "title": "feat: latest work on A", "headRefName": "work/a", "state": "MERGED"},
            {"number": 7,  "title": "fix: work on B", "headRefName": "work/b", "state": "OPEN"},
            {"number": 8,  "title": "", "headRefName": "work/c", "state": "OPEN"}
        ]"#;
        let map = parse_pr_titles_by_branch(json).unwrap();
        // Highest PR number wins for a branch with several PRs.
        assert_eq!(
            map.get("work/a").map(String::as_str),
            Some("feat: latest work on A")
        );
        assert_eq!(
            map.get("work/b").map(String::as_str),
            Some("fix: work on B")
        );
        // Empty titles are skipped.
        assert!(!map.contains_key("work/c"));
    }

    #[test]
    fn parse_merged_pr_branches_collects_only_merged_head_refs() {
        let json = r#"[
            {"headRefName": "work/a", "state": "MERGED"},
            {"headRefName": "work/b", "state": "OPEN"},
            {"headRefName": "work/c", "state": "merged"},
            {"headRefName": "", "state": "MERGED"}
        ]"#;
        let branches = parse_merged_pr_branches(json).unwrap();
        assert!(branches.contains("work/a"));
        assert!(
            branches.contains("work/c"),
            "state match is case-insensitive"
        );
        assert!(!branches.contains("work/b"), "open PRs are excluded");
        assert_eq!(branches.len(), 2, "empty head refs are skipped");
    }

    #[test]
    fn run_gh_command_uses_child_bare_repo_cwd_for_workspace_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo = tmp.path().join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        // `main_worktree_root` canonicalizes and then strips the Windows
        // verbatim (`\\?\`) prefix, because a child process cannot always
        // consume a verbatim path. Assert against that same contract so the
        // expectation holds on Windows as well as on unix.
        let expected = gwt_core::paths::normalize_windows_child_process_path(
            &std::fs::canonicalize(&bare_repo).expect("canonical bare repo"),
        );
        let mut observed_cwd = None;

        let output = run_gh_command_with(tmp.path(), &["pr", "list"], |cwd, args| {
            observed_cwd = Some(cwd.to_path_buf());
            assert_eq!(args, ["pr", "list"]);
            Ok(GhCliOutput {
                success: true,
                stdout: "[]".to_string(),
                stderr: String::new(),
            })
        })
        .expect("run fake gh command");

        assert!(output.success);
        assert_eq!(observed_cwd.as_deref(), Some(expected.as_path()));
    }

    #[test]
    fn parse_pr_status_open() {
        let json = r#"{
            "number": 123,
            "title": "Add feature",
            "state": "OPEN",
            "url": "https://github.com/owner/repo/pull/123",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"}
            ],
            "reviewDecision": "APPROVED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.number, 123);
        assert_eq!(pr.title, "Add feature");
        assert_eq!(pr.state, PrState::Open);
        assert_eq!(pr.ci_status, "SUCCESS");
        assert_eq!(pr.mergeable, "MERGEABLE");
        assert_eq!(pr.merge_state_status, "CLEAN");
        assert_eq!(pr.effective_merge_status(), "MERGEABLE");
        assert_eq!(pr.review_status, "APPROVED");
    }

    #[test]
    fn parse_pr_status_merged() {
        let json = r#"{
            "number": 456,
            "title": "Fix bug",
            "state": "MERGED",
            "url": "https://github.com/owner/repo/pull/456",
            "mergeable": "UNKNOWN",
            "mergeStateStatus": "UNKNOWN",
            "statusCheckRollup": [],
            "reviewDecision": "APPROVED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.state, PrState::Merged);
        assert_eq!(pr.ci_status, "UNKNOWN");
    }

    #[test]
    fn parse_pr_status_ci_failure() {
        let json = r#"{
            "number": 789,
            "title": "Broken PR",
            "state": "OPEN",
            "url": "",
            "mergeable": "CONFLICTING",
            "mergeStateStatus": "DIRTY",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "lint", "status": "COMPLETED", "conclusion": "FAILURE"}
            ],
            "reviewDecision": "CHANGES_REQUESTED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.ci_status, "FAILURE");
        assert_eq!(pr.mergeable, "CONFLICTING");
        assert_eq!(pr.review_status, "CHANGES_REQUESTED");
    }

    #[test]
    fn parse_pr_status_branch_behind_prefers_merge_state_status_for_cli() {
        let json = r#"{
            "number": 102,
            "title": "Update branch required",
            "state": "OPEN",
            "url": "",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "BEHIND",
            "statusCheckRollup": [],
            "reviewDecision": "REVIEW_REQUIRED"
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.mergeable, "MERGEABLE");
        assert_eq!(pr.merge_state_status, "BEHIND");
        assert_eq!(pr.effective_merge_status(), "BEHIND");
        assert!(pr.requires_update_branch());
    }

    #[test]
    fn parse_pr_status_ci_pending() {
        let json = r#"{
            "number": 101,
            "title": "WIP",
            "state": "OPEN",
            "url": "",
            "mergeable": "UNKNOWN",
            "mergeStateStatus": "UNKNOWN",
            "statusCheckRollup": [
                {"name": "ci", "status": "IN_PROGRESS", "conclusion": null}
            ],
            "reviewDecision": ""
        }"#;

        let pr = parse_pr_status_json(json).unwrap();
        assert_eq!(pr.ci_status, "PENDING");
    }

    #[test]
    fn parse_pr_status_invalid_json() {
        let result = parse_pr_status_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_pr_check_report_structured_statuses() {
        let json = r#"{
            "title": "Add feature",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "reviewDecision": "APPROVED",
            "statusCheckRollup": [
                {"name": "ci", "status": "COMPLETED", "conclusion": "SUCCESS"},
                {"name": "lint", "status": "COMPLETED", "conclusion": "NEUTRAL"}
            ]
        }"#;

        let report = parse_pr_check_report_json(json).unwrap();

        assert_eq!(report.ci, CiStatus::Passing);
        assert_eq!(report.merge, MergeStatus::Ready);
        assert_eq!(report.review, ReviewStatus::Approved);
        assert_eq!(
            report.summary,
            "PR: Add feature | CI: Passing | Merge: Ready | Review: Approved"
        );
    }

    #[test]
    fn parse_pr_check_report_branch_behind_uses_merge_state_status() {
        let json = r#"{
            "title": "Update branch required",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "BEHIND",
            "reviewDecision": "REVIEW_REQUIRED",
            "statusCheckRollup": []
        }"#;

        let report = parse_pr_check_report_json(json).unwrap();

        assert_eq!(report.ci, CiStatus::Pending);
        assert_eq!(report.merge, MergeStatus::Behind);
        assert_eq!(report.review, ReviewStatus::Pending);
        assert_eq!(
            report.summary,
            "PR: Update branch required | CI: Pending | Merge: Behind | Review: Pending"
        );
    }

    #[test]
    fn parse_pr_check_report_empty_checks() {
        let json = r#"{
            "title": "Waiting on CI",
            "mergeable": "CONFLICTING",
            "mergeStateStatus": "DIRTY",
            "reviewDecision": "REVIEW_REQUIRED",
            "statusCheckRollup": []
        }"#;

        let report = parse_pr_check_report_json(json).unwrap();

        assert_eq!(report.ci, CiStatus::Pending);
        assert_eq!(report.merge, MergeStatus::Conflicts);
        assert_eq!(report.review, ReviewStatus::Pending);
        assert_eq!(
            report.summary,
            "PR: Waiting on CI | CI: Pending | Merge: Conflicts | Review: Pending"
        );
    }

    #[test]
    fn parse_pr_list_empty() {
        let prs = parse_pr_list_json("[]").unwrap();
        assert!(prs.is_empty());
    }

    #[test]
    fn parse_pr_list_multiple() {
        let json = r#"[
            {
                "number": 1,
                "title": "First PR",
                "state": "OPEN",
                "url": "https://github.com/o/r/pull/1",
                "mergeable": "MERGEABLE",
                "mergeStateStatus": "CLEAN",
                "statusCheckRollup": [],
                "reviewDecision": "APPROVED"
            },
            {
                "number": 2,
                "title": "Second PR",
                "state": "OPEN",
                "url": "https://github.com/o/r/pull/2",
                "mergeable": "CONFLICTING",
                "mergeStateStatus": "DIRTY",
                "statusCheckRollup": [
                    {"name": "ci", "status": "COMPLETED", "conclusion": "FAILURE"}
                ],
                "reviewDecision": "CHANGES_REQUESTED"
            }
        ]"#;

        let prs = parse_pr_list_json(json).unwrap();
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 1);
        assert_eq!(prs[0].title, "First PR");
        assert_eq!(prs[1].number, 2);
        assert_eq!(prs[1].ci_status, "FAILURE");
        assert_eq!(prs[0].merge_state_status, "CLEAN");
        assert_eq!(prs[1].merge_state_status, "DIRTY");
    }

    #[test]
    fn parse_pr_status_records_created_at() {
        let json = r#"{
            "number": 2538,
            "title": "Active Work title",
            "state": "OPEN",
            "url": "https://github.com/akiojin/gwt/pull/2538",
            "createdAt": "2026-05-07T08:12:00Z",
            "mergeable": "MERGEABLE",
            "mergeStateStatus": "CLEAN",
            "statusCheckRollup": [],
            "reviewDecision": "APPROVED"
        }"#;

        let pr = parse_pr_status_json(json).expect("parse pr");

        assert_eq!(
            pr.created_at.expect("created_at").to_rfc3339(),
            "2026-05-07T08:12:00+00:00"
        );
    }

    #[test]
    fn latest_pr_by_created_at_prefers_newest_pr() {
        let older = PrStatus {
            number: 2537,
            title: "Older PR".to_string(),
            state: PrState::Closed,
            url: "https://github.com/akiojin/gwt/pull/2537".to_string(),
            created_at: Some("2026-05-07T08:05:00Z".parse().expect("older time")),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        };
        let newer = PrStatus {
            number: 2538,
            title: "Newer PR".to_string(),
            state: PrState::Open,
            url: "https://github.com/akiojin/gwt/pull/2538".to_string(),
            created_at: Some("2026-05-07T08:20:00Z".parse().expect("newer time")),
            ci_status: "PENDING".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: "UNKNOWN".to_string(),
            review_status: "REVIEW_REQUIRED".to_string(),
        };

        let latest = latest_pr_by_created_at(vec![older, newer]).expect("latest pr");

        assert_eq!(latest.number, 2538);
        assert_eq!(latest.title, "Newer PR");
    }

    #[test]
    fn parse_pr_list_invalid_json() {
        assert!(parse_pr_list_json("not json").is_err());
    }

    #[test]
    fn parse_rest_pr_list_json_sets_missing_ci_merge_review_fields_to_unknown() {
        let json = r#"[
            {
                "number": 11,
                "title": "REST fallback PR",
                "state": "open",
                "html_url": "https://github.com/o/r/pull/11"
            }
        ]"#;

        let prs = parse_rest_pr_list_json(json).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 11);
        assert_eq!(prs[0].title, "REST fallback PR");
        assert_eq!(prs[0].state, PrState::Open);
        assert_eq!(prs[0].url, "https://github.com/o/r/pull/11");
        assert_eq!(prs[0].ci_status, "UNKNOWN");
        assert_eq!(prs[0].mergeable, "UNKNOWN");
        assert_eq!(prs[0].merge_state_status, "UNKNOWN");
        assert_eq!(prs[0].review_status, "UNKNOWN");
    }

    #[test]
    fn fetch_pr_list_with_uses_primary_pr_list_when_available() {
        let repo_path = Path::new("/tmp/repo");
        let mut calls = Vec::new();

        let prs = fetch_pr_list_with(repo_path, |path, args| {
            assert_eq!(path, repo_path);
            calls.push(args[..2].join(" "));
            match args {
                ["pr", "list", ..] => Ok(GhCliOutput {
                    success: true,
                    stdout: r#"[
                        {
                            "number": 7,
                            "title": "Primary transport",
                            "state": "OPEN",
                            "url": "https://github.com/o/r/pull/7",
                            "mergeable": "MERGEABLE",
                            "mergeStateStatus": "CLEAN",
                            "statusCheckRollup": [],
                            "reviewDecision": "APPROVED"
                        }
                    ]"#
                    .to_string(),
                    stderr: String::new(),
                }),
                other => panic!("unexpected gh invocation: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(calls, vec!["pr list"]);
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 7);
        assert_eq!(prs[0].review_status, "APPROVED");
    }

    #[test]
    fn fetch_pr_list_with_falls_back_to_rest_when_pr_list_call_fails() {
        let repo_path = Path::new("/tmp/repo");
        let mut calls = Vec::new();

        let prs = fetch_pr_list_with(repo_path, |path, args| {
            assert_eq!(path, repo_path);
            calls.push(args[..2].join(" "));
            match args {
                ["pr", "list", ..] => Ok(GhCliOutput {
                    success: false,
                    stdout: String::new(),
                    stderr: "pr list unavailable".to_string(),
                }),
                ["api", "repos/{owner}/{repo}/pulls?state=all&per_page=20"] => Ok(GhCliOutput {
                    success: true,
                    stdout: r#"[
                        {
                            "number": 21,
                            "title": "REST fallback",
                            "state": "open",
                            "html_url": "https://github.com/o/r/pull/21"
                        }
                    ]"#
                    .to_string(),
                    stderr: String::new(),
                }),
                other => panic!("unexpected gh invocation: {other:?}"),
            }
        })
        .unwrap();

        assert_eq!(
            calls,
            vec![
                "pr list",
                "api repos/{owner}/{repo}/pulls?state=all&per_page=20"
            ]
        );
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].number, 21);
        assert_eq!(prs[0].ci_status, "UNKNOWN");
    }

    #[test]
    fn pr_state_display() {
        assert_eq!(PrState::Open.to_string(), "OPEN");
        assert_eq!(PrState::Closed.to_string(), "CLOSED");
        assert_eq!(PrState::Merged.to_string(), "MERGED");
    }

    // --- SPEC #3200 gate-input adapters ---

    #[test]
    fn parse_open_pr_number_returns_highest_and_handles_empty() {
        assert_eq!(
            parse_open_pr_number(r#"[{"number":12},{"number":34}]"#),
            Some(34),
            "most recent (highest) open PR",
        );
        assert_eq!(parse_open_pr_number("[]"), None, "no open PR");
        assert_eq!(parse_open_pr_number("not json"), None, "unparseable → none");
    }

    #[test]
    fn parse_pr_head_sha_extracts_or_fails_closed() {
        assert_eq!(
            parse_pr_head_sha(r#"{"headRefOid":"abc123"}"#),
            Some("abc123".to_string()),
        );
        assert_eq!(parse_pr_head_sha(r#"{"headRefOid":""}"#), None, "empty SHA");
        assert_eq!(parse_pr_head_sha("{}"), None, "missing field");
        assert_eq!(parse_pr_head_sha("nope"), None, "unparseable");
    }

    #[test]
    fn extract_status_check_rollup_returns_array_or_empty() {
        let body = r#"{"statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]}"#;
        let rollup = extract_status_check_rollup(body);
        assert!(rollup.contains("\"build\""));
        assert!(rollup.starts_with('['));
        // null / missing / unparseable → "[]" (fail-closed → vacuous).
        assert_eq!(
            extract_status_check_rollup(r#"{"statusCheckRollup":null}"#),
            "[]"
        );
        assert_eq!(extract_status_check_rollup("{}"), "[]");
        assert_eq!(extract_status_check_rollup("nope"), "[]");
    }

    #[test]
    fn fetch_open_pr_number_for_branch_with_parses_and_fails_closed() {
        let repo = Path::new("/tmp/repo");
        let found = fetch_open_pr_number_for_branch_with(repo, "work/issue-42", |_p, args| {
            assert_eq!(args[0], "pr");
            assert!(args.contains(&"work/issue-42"));
            Ok(GhCliOutput {
                success: true,
                stdout: r#"[{"number":7}]"#.to_string(),
                stderr: String::new(),
            })
        });
        assert_eq!(found, Some(7));

        let failed = fetch_open_pr_number_for_branch_with(repo, "b", |_p, _args| {
            Ok(GhCliOutput {
                success: false,
                stdout: String::new(),
                stderr: "boom".to_string(),
            })
        });
        assert_eq!(failed, None, "gh failure → fail-closed None");
    }

    #[test]
    fn fetch_pr_status_check_rollup_with_fails_closed_to_empty_array() {
        let repo = Path::new("/tmp/repo");
        let ok = fetch_pr_status_check_rollup_with(repo, 7, |_p, _args| {
            Ok(GhCliOutput {
                success: true,
                stdout: r#"{"statusCheckRollup":[{"name":"ci","state":"SUCCESS"}]}"#.to_string(),
                stderr: String::new(),
            })
        });
        assert!(ok.contains("\"ci\""));
        let failed = fetch_pr_status_check_rollup_with(repo, 7, |_p, _args| {
            Err(GwtError::Git("nope".to_string()))
        });
        assert_eq!(failed, "[]", "any failure → vacuous, never a pass");
    }

    #[test]
    fn merge_pr_auto_with_arms_bound_to_head_sha_and_fails_closed() {
        let repo = Path::new("/tmp/repo");
        let armed = merge_pr_auto_with(repo, 7, "abc123", |_p, args| {
            // The arm MUST bind to the reviewed head SHA via --match-head-commit
            // so a head that advanced past review cannot merge unreviewed.
            assert_eq!(
                args,
                [
                    "pr",
                    "merge",
                    "7",
                    "--auto",
                    "--squash",
                    "--match-head-commit",
                    "abc123",
                ]
            );
            Ok(GhCliOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        assert!(armed);
        let failed = merge_pr_auto_with(repo, 7, "abc123", |_p, _a| {
            Ok(GhCliOutput {
                success: false,
                stdout: String::new(),
                stderr: "nope".to_string(),
            })
        });
        assert!(!failed, "gh failure ⇒ not armed (fail-closed)");
        let errored = merge_pr_auto_with(repo, 7, "abc123", |_p, _a| {
            Err(GwtError::Git("x".to_string()))
        });
        assert!(!errored);
    }

    #[test]
    fn disable_pr_auto_merge_with_disarms() {
        let repo = Path::new("/tmp/repo");
        let ok = disable_pr_auto_merge_with(repo, 7, |_p, args| {
            assert_eq!(args, ["pr", "merge", "7", "--disable-auto"]);
            Ok(GhCliOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        assert!(ok);
    }

    // SPEC #3200 Phase 7 / T-143: remote auto-merge state must be observable
    // independently from the mutation command so an ambiguous result can be
    // reconciled after a daemon restart instead of being collapsed to `false`.
    #[test]
    fn parse_pr_auto_merge_remote_state_distinguishes_open_armed_merged_and_closed() {
        assert_eq!(
            parse_pr_auto_merge_remote_state(
                r#"{"state":"OPEN","headRefOid":"abc123","autoMergeRequest":{"enabledAt":"2026-07-27T00:00:00Z"},"mergeCommit":null}"#,
            ),
            Some(PrAutoMergeRemoteState::Open {
                head_sha: "abc123".to_string(),
                auto_merge_requested: true,
            })
        );
        assert_eq!(
            parse_pr_auto_merge_remote_state(
                r#"{"state":"OPEN","headRefOid":"abc123","autoMergeRequest":null,"mergeCommit":null}"#,
            ),
            Some(PrAutoMergeRemoteState::Open {
                head_sha: "abc123".to_string(),
                auto_merge_requested: false,
            })
        );
        assert_eq!(
            parse_pr_auto_merge_remote_state(
                r#"{"state":"MERGED","headRefOid":"abc123","autoMergeRequest":null,"mergeCommit":{"oid":"merge456"}}"#,
            ),
            Some(PrAutoMergeRemoteState::Merged {
                head_sha: "abc123".to_string(),
            })
        );
        assert_eq!(
            parse_pr_auto_merge_remote_state(
                r#"{"state":"CLOSED","headRefOid":"abc123","autoMergeRequest":null,"mergeCommit":null}"#,
            ),
            Some(PrAutoMergeRemoteState::Closed {
                head_sha: "abc123".to_string(),
            })
        );
        assert_eq!(parse_pr_auto_merge_remote_state("not json"), None);
        assert_eq!(
            parse_pr_auto_merge_remote_state(r#"{"state":"OPEN","headRefOid":""}"#),
            None,
            "missing HEAD is not an authoritative readback"
        );
    }

    #[test]
    fn arm_pr_auto_merge_preserves_confirmed_already_target_and_head_changed_outcomes() {
        let repo = Path::new("/tmp/repo");
        let mut calls = 0;
        let already_armed = arm_pr_auto_merge_with(
            repo,
            7,
            "abc123",
            &PrAutoMergeRemoteState::Open {
                head_sha: "abc123".to_string(),
                auto_merge_requested: true,
            },
            |_p, _args| {
                calls += 1;
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(already_armed, AutoMergeMutationOutcome::AlreadyTargetState);
        assert!(already_armed.is_success());
        assert_eq!(calls, 0, "already-armed readback must not re-submit");

        let already_merged = arm_pr_auto_merge_with(
            repo,
            7,
            "abc123",
            &PrAutoMergeRemoteState::Merged {
                head_sha: "abc123".to_string(),
            },
            |_p, _args| {
                calls += 1;
                unreachable!("already-merged readback must not re-submit")
            },
        );
        assert_eq!(already_merged, AutoMergeMutationOutcome::AlreadyTargetState);
        assert!(already_merged.is_success());
        assert_eq!(
            calls, 0,
            "same-head merged readback is already the target state"
        );

        let merged_head_changed = arm_pr_auto_merge_with(
            repo,
            7,
            "abc123",
            &PrAutoMergeRemoteState::Merged {
                head_sha: "def456".to_string(),
            },
            |_p, _args| {
                calls += 1;
                unreachable!("changed merged HEAD must reject before mutation")
            },
        );
        assert_eq!(
            merged_head_changed,
            AutoMergeMutationOutcome::HeadChanged {
                expected: "abc123".to_string(),
                actual: "def456".to_string(),
            }
        );
        assert!(!merged_head_changed.is_success());
        assert_eq!(calls, 0, "changed merged HEAD must not re-submit");

        let head_changed = arm_pr_auto_merge_with(
            repo,
            7,
            "abc123",
            &PrAutoMergeRemoteState::Open {
                head_sha: "def456".to_string(),
                auto_merge_requested: false,
            },
            |_p, _args| {
                calls += 1;
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(
            head_changed,
            AutoMergeMutationOutcome::HeadChanged {
                expected: "abc123".to_string(),
                actual: "def456".to_string(),
            }
        );
        assert!(!head_changed.is_success());
        assert_eq!(calls, 0, "changed HEAD must reject before mutation");

        let confirmed = arm_pr_auto_merge_with(
            repo,
            7,
            "abc123",
            &PrAutoMergeRemoteState::Open {
                head_sha: "abc123".to_string(),
                auto_merge_requested: false,
            },
            |_p, args| {
                calls += 1;
                assert_eq!(
                    args,
                    [
                        "pr",
                        "merge",
                        "7",
                        "--auto",
                        "--squash",
                        "--match-head-commit",
                        "abc123",
                    ]
                );
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(confirmed, AutoMergeMutationOutcome::Confirmed);
        assert!(confirmed.is_success());
        assert_eq!(calls, 1);
    }

    #[test]
    fn arm_pr_auto_merge_preserves_pre_submit_and_remote_unknown_failures() {
        let repo = Path::new("/tmp/repo");
        let remote = PrAutoMergeRemoteState::Open {
            head_sha: "abc123".to_string(),
            auto_merge_requested: false,
        };

        let pre_submit = arm_pr_auto_merge_with(repo, 7, "abc123", &remote, |_p, _args| {
            Err(AutoMergeCommandError::PreSubmit(
                "gh executable unavailable".to_string(),
            ))
        });
        assert_eq!(
            pre_submit,
            AutoMergeMutationOutcome::PreSubmit("gh executable unavailable".to_string())
        );

        let ambiguous = arm_pr_auto_merge_with(repo, 7, "abc123", &remote, |_p, _args| {
            Err(AutoMergeCommandError::RemoteOutcomeUnknown(
                "deadline expired after process start".to_string(),
            ))
        });
        assert_eq!(
            ambiguous,
            AutoMergeMutationOutcome::RemoteOutcomeUnknown(
                "deadline expired after process start".to_string()
            )
        );
    }

    #[test]
    fn disarm_pr_auto_merge_treats_already_disarmed_as_success_and_preserves_failures() {
        let repo = Path::new("/tmp/repo");
        let mut calls = 0;
        let already_disarmed = disarm_pr_auto_merge_with(
            repo,
            7,
            &PrAutoMergeRemoteState::Open {
                head_sha: "abc123".to_string(),
                auto_merge_requested: false,
            },
            |_p, _args| {
                calls += 1;
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(
            already_disarmed,
            AutoMergeMutationOutcome::AlreadyTargetState
        );
        assert!(already_disarmed.is_success());
        assert_eq!(calls, 0, "already-disarmed PR must not be mutated again");

        let closed = disarm_pr_auto_merge_with(
            repo,
            7,
            &PrAutoMergeRemoteState::Closed {
                head_sha: "abc123".to_string(),
            },
            |_p, _args| {
                calls += 1;
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(closed, AutoMergeMutationOutcome::AlreadyTargetState);
        assert!(closed.is_success());
        assert_eq!(calls, 0);

        let merged = disarm_pr_auto_merge_with(
            repo,
            7,
            &PrAutoMergeRemoteState::Merged {
                head_sha: "abc123".to_string(),
            },
            |_p, _args| {
                calls += 1;
                Ok(GhCliOutput {
                    success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            },
        );
        assert_eq!(
            merged,
            AutoMergeMutationOutcome::AuthorityMismatch(
                "pull request merged before kill-switch disarm was confirmed".to_string()
            )
        );
        assert!(!merged.is_success());
        assert_eq!(calls, 0, "a merged PR cannot be disarmed after the fact");

        let armed = PrAutoMergeRemoteState::Open {
            head_sha: "abc123".to_string(),
            auto_merge_requested: true,
        };
        let confirmed = disarm_pr_auto_merge_with(repo, 7, &armed, |_p, args| {
            calls += 1;
            assert_eq!(args, ["pr", "merge", "7", "--disable-auto"]);
            Ok(GhCliOutput {
                success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        assert_eq!(confirmed, AutoMergeMutationOutcome::Confirmed);
        assert_eq!(calls, 1);

        let pre_submit = disarm_pr_auto_merge_with(repo, 7, &armed, |_p, _args| {
            Err(AutoMergeCommandError::PreSubmit("not started".to_string()))
        });
        assert_eq!(
            pre_submit,
            AutoMergeMutationOutcome::PreSubmit("not started".to_string())
        );

        let ambiguous = disarm_pr_auto_merge_with(repo, 7, &armed, |_p, _args| {
            Err(AutoMergeCommandError::RemoteOutcomeUnknown(
                "response lost".to_string(),
            ))
        });
        assert_eq!(
            ambiguous,
            AutoMergeMutationOutcome::RemoteOutcomeUnknown("response lost".to_string())
        );
    }

    #[test]
    fn parse_pr_merge_commit_sha_extracts_or_fails_closed() {
        assert_eq!(
            parse_pr_merge_commit_sha(r#"{"mergeCommit":{"oid":"deadbeef"}}"#),
            Some("deadbeef".to_string()),
        );
        assert_eq!(
            parse_pr_merge_commit_sha(r#"{"mergeCommit":null}"#),
            None,
            "not merged yet"
        );
        assert_eq!(parse_pr_merge_commit_sha("{}"), None);
        assert_eq!(parse_pr_merge_commit_sha("nope"), None);
    }

    #[test]
    fn fetch_pr_diff_with_truncates_and_fails_closed() {
        let repo = Path::new("/tmp/repo");
        let big = "x".repeat(5000);
        let diff = fetch_pr_diff_with(repo, 7, 100, {
            let big = big.clone();
            move |_p, args| {
                assert_eq!(args, ["pr", "diff", "7"]);
                Ok(GhCliOutput {
                    success: true,
                    stdout: big.clone(),
                    stderr: String::new(),
                })
            }
        })
        .expect("diff present");
        assert!(diff.contains("diff truncated"));
        assert!(diff.len() < 5000);

        let failed = fetch_pr_diff_with(repo, 7, 100, |_p, _a| {
            Ok(GhCliOutput {
                success: false,
                stdout: String::new(),
                stderr: "no pr".to_string(),
            })
        });
        assert_eq!(failed, None, "gh failure → None");
    }

    fn sample_inventory_fields() -> PrInventoryFields {
        PrInventoryFields {
            number: 1,
            title: "feat: example".to_string(),
            url: "https://github.com/o/r/pull/1".to_string(),
            is_draft: false,
            head_ref_name: "work/issue-10".to_string(),
            updated_at: Some("2026-08-30T00:00:00Z".parse().expect("now")),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            ci_status: "SUCCESS".to_string(),
            review_status: "APPROVED".to_string(),
            body: "Closes #10".to_string(),
            closing_issues: vec![],
        }
    }

    #[test]
    fn classifies_merge_candidate_when_clean_and_green() {
        let decision = classify_pr_lifecycle(
            &sample_inventory_fields(),
            "2026-08-30T00:00:00Z".parse().expect("now"),
        );
        assert_eq!(decision.class, PrLifecycleClass::MergeCandidate);
        assert!(!decision.stale);
        assert!(!decision.owner_issue_closed);
        assert_eq!(decision.default_action, "propose merge");
    }

    #[test]
    fn classifies_draft_merge_candidate_as_mark_ready() {
        let mut fields = sample_inventory_fields();
        fields.is_draft = true;
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::MergeCandidate);
        assert_eq!(decision.default_action, "mark ready");
    }

    #[test]
    fn classifies_conflicted_ahead_of_ci_red() {
        let mut fields = sample_inventory_fields();
        fields.mergeable = "CONFLICTING".to_string();
        fields.ci_status = "FAILURE".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::Conflicted);
        assert_eq!(
            decision.default_action,
            "relaunch owner to resolve conflict"
        );
    }

    #[test]
    fn classifies_behind_from_merge_state() {
        let mut fields = sample_inventory_fields();
        fields.merge_state_status = "BEHIND".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::Behind);
        assert_eq!(decision.default_action, "update-branch");
    }

    #[test]
    fn classifies_ci_red() {
        let mut fields = sample_inventory_fields();
        fields.ci_status = "FAILURE".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::CiRed);
        assert_eq!(decision.default_action, "relaunch owner to fix CI");
    }

    #[test]
    fn classifies_superseded_from_title_or_body_ahead_of_conflict() {
        let mut fields = sample_inventory_fields();
        fields.title = "feat: old path (superseded by #99)".to_string();
        fields.mergeable = "CONFLICTING".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::Superseded);
        assert_eq!(
            decision.default_action,
            "propose close in digest (never auto-close)"
        );

        fields.title = "feat: example".to_string();
        fields.body = "This PR is superseded by #100".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::Superseded);
    }

    #[test]
    fn classifies_owner_issue_closed_as_superseded_close_proposal() {
        let mut fields = sample_inventory_fields();
        fields.closing_issues = vec![PrClosingIssue {
            number: 10,
            state: Some("CLOSED".to_string()),
        }];
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::Superseded);
        assert!(decision.owner_issue_closed);
        assert_eq!(
            decision.default_action,
            "propose close in digest (never auto-close)"
        );
    }

    #[test]
    fn owner_issue_stays_open_when_any_closing_issue_is_open() {
        let mut fields = sample_inventory_fields();
        fields.closing_issues = vec![
            PrClosingIssue {
                number: 10,
                state: Some("CLOSED".to_string()),
            },
            PrClosingIssue {
                number: 11,
                state: Some("OPEN".to_string()),
            },
        ];
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert!(!decision.owner_issue_closed);
        assert_eq!(decision.class, PrLifecycleClass::MergeCandidate);
    }

    #[test]
    fn classifies_in_progress_for_pending_ci_or_unknown_merge() {
        let mut fields = sample_inventory_fields();
        fields.ci_status = "PENDING".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert_eq!(decision.class, PrLifecycleClass::InProgress);
        assert_eq!(decision.default_action, "leave in progress");
    }

    #[test]
    fn marks_pr_stale_after_72_hours_without_update() {
        let mut fields = sample_inventory_fields();
        fields.updated_at = Some("2026-08-26T23:59:59Z".parse().expect("old"));
        fields.ci_status = "PENDING".to_string();
        let decision = classify_pr_lifecycle(&fields, "2026-08-30T00:00:00Z".parse().expect("now"));
        assert!(decision.stale);
        assert_eq!(decision.class, PrLifecycleClass::InProgress);
        assert_eq!(decision.default_action, "escalate: no update for 72h");
    }

    #[test]
    fn parse_pr_inventory_json_classifies_rows() {
        let json = r#"[
            {
                "number": 42,
                "title": "feat: ready",
                "url": "https://github.com/o/r/pull/42",
                "isDraft": false,
                "updatedAt": "2026-08-30T00:00:00Z",
                "mergeable": "MERGEABLE",
                "mergeStateStatus": "CLEAN",
                "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
                "reviewDecision": "APPROVED",
                "body": "Closes #7",
                "closingIssuesReferences": [{"number": 7, "state": "OPEN"}]
            }
        ]"#;
        let items = parse_pr_inventory_json(json, "2026-08-30T00:00:00Z".parse().expect("now"))
            .expect("parse inventory");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].number, 42);
        assert_eq!(items[0].lifecycle, "MERGE-CANDIDATE");
        assert_eq!(items[0].default_action, "propose merge");
        assert!(!items[0].stale);
        assert_eq!(items[0].closing_issues[0].number, 7);
    }

    #[test]
    fn parse_pr_inventory_json_accepts_nested_closing_issue_nodes() {
        let json = r#"[
            {
                "number": 8,
                "title": "fix: leftover",
                "url": "https://github.com/o/r/pull/8",
                "isDraft": true,
                "updatedAt": "2026-08-01T00:00:00Z",
                "mergeable": "MERGEABLE",
                "mergeStateStatus": "CLEAN",
                "statusCheckRollup": [],
                "body": "superseded by #9",
                "closingIssuesReferences": {"nodes": [{"number": 3, "state": "CLOSED"}]}
            }
        ]"#;
        let items = parse_pr_inventory_json(json, "2026-08-30T00:00:00Z".parse().expect("now"))
            .expect("parse nested closing issues");
        assert_eq!(items[0].lifecycle, "SUPERSEDED");
        assert!(items[0].owner_issue_closed);
        assert!(items[0].stale);
        assert!(items[0].is_draft);
    }

    #[test]
    fn fetch_pr_inventory_with_lists_open_prs_and_classifies() {
        let repo_path = Path::new("/tmp/repo");
        let mut calls = Vec::new();
        let items = fetch_pr_inventory_with(
            repo_path,
            "2026-08-30T00:00:00Z".parse().expect("now"),
            &PrInventoryOptions::default(),
            |path, args| {
                assert_eq!(path, repo_path);
                calls.push(args.join(" "));
                match args {
                    ["pr", "list", ..] => {
                        assert!(args.contains(&"--state"));
                        assert!(args.contains(&"open"));
                        Ok(GhCliOutput {
                            success: true,
                            stdout: r#"[{
                                "number": 3,
                                "title": "feat: behind",
                                "url": "https://github.com/o/r/pull/3",
                                "isDraft": false,
                                "updatedAt": "2026-08-30T00:00:00Z",
                                "mergeable": "MERGEABLE",
                                "mergeStateStatus": "BEHIND",
                                "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
                                "body": ""
                            }]"#
                            .to_string(),
                            stderr: String::new(),
                        })
                    }
                    other => panic!("unexpected gh invocation: {other:?}"),
                }
            },
        )
        .expect("inventory");
        assert!(calls[0].starts_with("pr list"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].lifecycle, "BEHIND");
        assert_eq!(items[0].default_action, "update-branch");
    }

    // ---- Issue #3868: fallback detection, dwell time, no-progress counting ----

    fn now_3868() -> DateTime<Utc> {
        "2026-09-01T12:00:00Z".parse().expect("now")
    }

    #[test]
    fn inventory_row_reports_dwell_hours_from_updated_at() {
        let mut fields = sample_inventory_fields();
        fields.updated_at = Some("2026-08-29T12:00:00Z".parse().expect("updated"));
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.dwell_hours, Some(72));
        assert!(decision.stale);

        fields.updated_at = None;
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.dwell_hours, None);
    }

    #[test]
    fn stale_threshold_is_configurable_with_a_default_of_72_hours() {
        let options = PrInventoryOptions::default();
        assert_eq!(options.stale_after_hours, PR_STALE_AFTER_HOURS);
        assert_eq!(
            options.escalate_after_cycles,
            PR_ESCALATE_AFTER_UNCHANGED_CYCLES
        );

        let mut fields = sample_inventory_fields();
        fields.ci_status = "PENDING".to_string();
        fields.updated_at = Some("2026-08-31T11:00:00Z".parse().expect("updated"));
        let default = classify_pr_lifecycle(&fields, now_3868());
        assert!(!default.stale, "25h is under the 72h default");

        let tight = PrInventoryOptions {
            stale_after_hours: 24,
            ..PrInventoryOptions::default()
        };
        let decision = classify_pr_lifecycle_with(&fields, now_3868(), &tight);
        assert!(decision.stale, "25h exceeds a 24h threshold");
        assert_eq!(decision.default_action, "escalate: no update for 24h");
    }

    #[test]
    fn relaunch_actions_on_the_owner_launch_ref_are_not_executable() {
        let mut fields = sample_inventory_fields();
        fields.mergeable = "CONFLICTING".to_string();
        fields.head_ref_name = "work/issue-10".to_string();
        fields.closing_issues = vec![PrClosingIssue {
            number: 10,
            state: Some("OPEN".to_string()),
        }];
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::Conflicted);
        assert!(!decision.default_action_executable);
        assert_eq!(
            decision.blocker.as_deref(),
            Some("owner_relaunch_refused_unique_commits")
        );
        assert_eq!(
            decision.fallback.as_deref(),
            Some(PR_FALLBACK_WHEN_NOT_EXECUTABLE)
        );

        fields.mergeable = "MERGEABLE".to_string();
        fields.ci_status = "FAILURE".to_string();
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::CiRed);
        assert!(!decision.default_action_executable);
        assert_eq!(
            decision.blocker.as_deref(),
            Some("owner_relaunch_refused_unique_commits")
        );
    }

    #[test]
    fn relaunch_actions_on_a_launch_ref_without_closing_issues_name_the_owner_from_the_head() {
        // #3726 / #3598 / #3593 in the wild: no `Closes #N`, head on
        // `work/issue-<n>`. The launch ref itself names the owner and is what
        // the Monitor's unique-commits guard refuses.
        let mut fields = sample_inventory_fields();
        fields.mergeable = "CONFLICTING".to_string();
        fields.head_ref_name = "work/issue-3712".to_string();
        fields.closing_issues = vec![];
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.owner_issue, Some(3712));
        assert!(!decision.default_action_executable);
        assert_eq!(
            decision.blocker.as_deref(),
            Some("owner_relaunch_refused_unique_commits")
        );
    }

    #[test]
    fn relaunch_actions_without_a_known_owner_are_not_executable() {
        let mut fields = sample_inventory_fields();
        fields.ci_status = "FAILURE".to_string();
        fields.head_ref_name = "feature/manual".to_string();
        fields.closing_issues = vec![];
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::CiRed);
        assert_eq!(decision.owner_issue, None);
        assert!(!decision.default_action_executable);
        assert_eq!(decision.blocker.as_deref(), Some("owner_unknown"));
        assert!(decision.fallback.is_some());
    }

    #[test]
    fn relaunch_actions_stay_executable_off_the_owner_launch_ref() {
        let mut fields = sample_inventory_fields();
        fields.ci_status = "FAILURE".to_string();
        fields.head_ref_name = "feature/other-branch".to_string();
        fields.closing_issues = vec![PrClosingIssue {
            number: 10,
            state: Some("OPEN".to_string()),
        }];
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert!(decision.default_action_executable);
        assert_eq!(decision.blocker, None);
        assert_eq!(decision.fallback, None);
    }

    #[test]
    fn executable_actions_carry_no_blocker() {
        let decision = classify_pr_lifecycle(&sample_inventory_fields(), now_3868());
        assert!(decision.default_action_executable);
        assert_eq!(decision.blocker, None);
        assert_eq!(decision.fallback, None);

        let mut fields = sample_inventory_fields();
        fields.closing_issues = vec![PrClosingIssue {
            number: 10,
            state: Some("CLOSED".to_string()),
        }];
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::Superseded);
        assert!(
            decision.default_action_executable,
            "a close proposal is something the PM can do"
        );
        assert_eq!(decision.blocker.as_deref(), Some("owner_issue_closed"));
    }

    #[test]
    fn unknown_mergeability_is_undetermined_instead_of_a_fake_class() {
        let mut fields = sample_inventory_fields();
        fields.mergeable = "UNKNOWN".to_string();
        fields.ci_status = "FAILURE".to_string();
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::Undetermined);
        assert_eq!(decision.class.as_str(), "UNDETERMINED");
        assert_eq!(
            decision.default_action,
            "hold: mergeability not computed yet, re-read next cycle"
        );

        fields.mergeable = "MERGEABLE".to_string();
        fields.merge_state_status = "UNKNOWN".to_string();
        fields.ci_status = "SUCCESS".to_string();
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(decision.class, PrLifecycleClass::Undetermined);

        fields.title = "superseded by #99".to_string();
        let decision = classify_pr_lifecycle(&fields, now_3868());
        assert_eq!(
            decision.class,
            PrLifecycleClass::Superseded,
            "supersession does not depend on mergeability"
        );
    }

    fn sample_item(number: u64, updated_at: &str, mergeable: &str, ci: &str) -> PrInventoryItem {
        let mut fields = sample_inventory_fields();
        fields.number = number;
        fields.updated_at = Some(updated_at.parse().expect("updated"));
        fields.mergeable = mergeable.to_string();
        fields.ci_status = ci.to_string();
        inventory_item_from_fields(fields, now_3868(), &PrInventoryOptions::default())
    }

    #[test]
    fn history_holds_the_previous_lifecycle_while_mergeability_is_unknown() {
        let mut history = PrInventoryHistory::default();
        let options = PrInventoryOptions::default();

        let mut first = vec![sample_item(
            3726,
            "2026-08-21T04:01:24Z",
            "CONFLICTING",
            "FAILURE",
        )];
        history.observe(&mut first, now_3868(), &options);
        assert_eq!(first[0].lifecycle, "CONFLICTED");
        assert_eq!(first[0].lifecycle_source, "observed");

        let mut second = vec![sample_item(
            3726,
            "2026-08-21T04:01:24Z",
            "UNKNOWN",
            "FAILURE",
        )];
        assert_eq!(second[0].lifecycle, "UNDETERMINED");
        history.observe(&mut second, now_3868(), &options);
        assert_eq!(
            second[0].lifecycle, "CONFLICTED",
            "same real data → held class"
        );
        assert_eq!(second[0].lifecycle_source, "held");
        assert_eq!(
            second[0].default_action,
            "relaunch owner to resolve conflict"
        );

        let mut moved = vec![sample_item(
            3726,
            "2026-09-01T00:00:00Z",
            "UNKNOWN",
            "FAILURE",
        )];
        history.observe(&mut moved, now_3868(), &options);
        assert_eq!(
            moved[0].lifecycle, "UNDETERMINED",
            "real data changed → previous class is not reused"
        );
        assert_eq!(moved[0].lifecycle_source, "undetermined");
    }

    #[test]
    fn history_counts_unchanged_cycles_and_flags_escalation_at_the_threshold() {
        let mut history = PrInventoryHistory::default();
        let options = PrInventoryOptions {
            escalate_after_cycles: 2,
            ..PrInventoryOptions::default()
        };
        let mut items = vec![sample_item(
            3847,
            "2026-09-01T10:15:00Z",
            "MERGEABLE",
            "FAILURE",
        )];
        history.observe(&mut items, now_3868(), &options);
        assert_eq!(items[0].unchanged_cycles, 0);
        assert!(!items[0].escalation_due);
        assert_eq!(items[0].escalate_after_cycles, 2);

        let mut items = vec![sample_item(
            3847,
            "2026-09-01T10:15:00Z",
            "MERGEABLE",
            "FAILURE",
        )];
        history.observe(&mut items, now_3868(), &options);
        assert_eq!(items[0].unchanged_cycles, 1);
        assert!(!items[0].escalation_due);

        let mut items = vec![sample_item(
            3847,
            "2026-09-01T10:15:00Z",
            "MERGEABLE",
            "FAILURE",
        )];
        history.observe(&mut items, now_3868(), &options);
        assert_eq!(items[0].unchanged_cycles, 2);
        assert!(
            items[0].escalation_due,
            "threshold reached → immediate escalation"
        );

        let mut items = vec![sample_item(
            3847,
            "2026-09-01T11:00:00Z",
            "MERGEABLE",
            "FAILURE",
        )];
        history.observe(&mut items, now_3868(), &options);
        assert_eq!(items[0].unchanged_cycles, 0, "an update resets the counter");
        assert!(!items[0].escalation_due);
    }

    #[test]
    fn stale_rows_are_escalation_due_regardless_of_the_cycle_counter() {
        let mut history = PrInventoryHistory::default();
        let mut items = vec![sample_item(
            3598,
            "2026-08-15T13:15:05Z",
            "CONFLICTING",
            "FAILURE",
        )];
        history.observe(&mut items, now_3868(), &PrInventoryOptions::default());
        assert!(items[0].stale);
        assert_eq!(items[0].unchanged_cycles, 0);
        assert!(items[0].escalation_due);
    }

    #[test]
    fn history_forgets_prs_that_left_the_inventory_and_round_trips_through_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("pr-inventory-history.json");
        let options = PrInventoryOptions::default();

        let mut history = PrInventoryHistory::load(&path);
        let mut items = vec![
            sample_item(1, "2026-09-01T00:00:00Z", "MERGEABLE", "SUCCESS"),
            sample_item(2, "2026-09-01T00:00:00Z", "MERGEABLE", "SUCCESS"),
        ];
        history.observe(&mut items, now_3868(), &options);
        history.save(&path).expect("save history");

        let mut reloaded = PrInventoryHistory::load(&path);
        let mut items = vec![sample_item(
            2,
            "2026-09-01T00:00:00Z",
            "MERGEABLE",
            "SUCCESS",
        )];
        reloaded.observe(&mut items, now_3868(), &options);
        assert_eq!(
            items[0].unchanged_cycles, 1,
            "counter survived the disk round trip"
        );
        assert!(
            !reloaded.entries.contains_key(&1),
            "a PR that left the inventory is forgotten"
        );

        std::fs::write(&path, "not json").expect("corrupt");
        let recovered = PrInventoryHistory::load(&path);
        assert!(
            recovered.entries.is_empty(),
            "corrupt history is treated as empty"
        );
    }

    #[test]
    fn parse_pr_inventory_json_reads_head_ref_and_dwell_fields() {
        let json = r#"[
            {
                "number": 3847,
                "title": "fix(pane): scope operations",
                "url": "https://github.com/o/r/pull/3847",
                "isDraft": true,
                "headRefName": "work/issue-3830",
                "updatedAt": "2026-09-01T10:15:00Z",
                "mergeable": "MERGEABLE",
                "mergeStateStatus": "BLOCKED",
                "statusCheckRollup": [{"conclusion": "FAILURE", "status": "COMPLETED"}],
                "body": "",
                "closingIssuesReferences": [{"number": 3830, "state": "OPEN"}]
            }
        ]"#;
        let items = parse_pr_inventory_json_with(json, now_3868(), &PrInventoryOptions::default())
            .expect("parse inventory");
        assert_eq!(items[0].head_ref_name, "work/issue-3830");
        assert_eq!(items[0].owner_issue, Some(3830));
        assert_eq!(items[0].lifecycle, "CI-RED");
        assert_eq!(items[0].dwell_hours, Some(1));
        assert_eq!(items[0].stale_after_hours, 72);
        assert!(!items[0].default_action_executable);
        assert_eq!(
            items[0].blocker.as_deref(),
            Some("owner_relaunch_refused_unique_commits")
        );
        assert_eq!(items[0].lifecycle_source, "observed");
        assert_eq!(items[0].unchanged_cycles, 0);
    }

    // ---- Issue #3891: TTL cache, light query, per-PR hydration, throttle ----

    use gwt_core::github_budget::{BudgetLedger, ProbeSnapshot, ResourceWindow};

    fn now_3891() -> DateTime<Utc> {
        "2026-09-02T00:00:00Z".parse().expect("now")
    }

    fn healthy_probe_payload() -> String {
        r#"{"resources":{"core":{"limit":5000,"remaining":4990,"reset":1756777200},
            "graphql":{"limit":5000,"remaining":4800,"reset":1756777200}}}"#
            .to_string()
    }

    fn light_row(number: u64, updated_at: &str, merge_state: &str) -> serde_json::Value {
        serde_json::json!({
            "number": number,
            "title": format!("feat: pr {number}"),
            "url": format!("https://github.com/o/r/pull/{number}"),
            "isDraft": false,
            "headRefName": format!("work/issue-{number}"),
            "updatedAt": updated_at,
            "mergeable": "MERGEABLE",
            "mergeStateStatus": merge_state,
            "reviewDecision": "APPROVED"
        })
    }

    fn rollup(conclusion: &str) -> String {
        format!(
            r#"{{"statusCheckRollup":[{{"conclusion":"{conclusion}","status":"COMPLETED"}}],"body":""}}"#
        )
    }

    /// A scripted `gh` that records every argv and answers list / view /
    /// rate_limit from the given rows.
    struct FakeGh {
        rows: Vec<serde_json::Value>,
        views: BTreeMap<u64, String>,
        calls: Vec<String>,
        probe_remaining: u64,
    }

    impl FakeGh {
        fn new(rows: Vec<serde_json::Value>) -> Self {
            Self {
                rows,
                views: BTreeMap::new(),
                calls: Vec::new(),
                probe_remaining: 4800,
            }
        }

        fn run(&mut self, args: &[&str]) -> Result<GhCliOutput> {
            self.calls.push(args.join(" "));
            let ok = |stdout: String| {
                Ok(GhCliOutput {
                    success: true,
                    stdout,
                    stderr: String::new(),
                })
            };
            match args {
                ["api", "rate_limit"] => ok(healthy_probe_payload().replace(
                    "\"remaining\":4800",
                    &format!("\"remaining\":{}", self.probe_remaining),
                )),
                ["pr", "list", ..] => {
                    let json_fields = args[args.len() - 1];
                    assert!(
                        !json_fields.contains("statusCheckRollup") && !json_fields.contains("body"),
                        "the list query must stay light (AC-2): {json_fields}"
                    );
                    ok(serde_json::Value::Array(self.rows.clone()).to_string())
                }
                ["pr", "view", number, "--json", _fields] => {
                    let number: u64 = number.parse().expect("number");
                    ok(self
                        .views
                        .get(&number)
                        .cloned()
                        .unwrap_or_else(|| rollup("SUCCESS")))
                }
                other => panic!("unexpected gh invocation: {other:?}"),
            }
        }
    }

    fn cached_read(
        tmp: &Path,
        ledger: &BudgetLedger,
        gh: &mut FakeGh,
        now: DateTime<Utc>,
        options: &PrInventoryOptions,
    ) -> Result<PrInventoryRead> {
        fetch_pr_inventory_cached_with(
            Path::new("/tmp/repo"),
            &tmp.join(PR_INVENTORY_CACHE_FILE),
            ledger,
            now,
            options,
            |_, args| gh.run(args),
        )
    }

    fn light_list_call() -> String {
        format!("pr list --state open --limit 100 --json {INVENTORY_LIGHT_JSON_FIELDS}")
    }

    #[test]
    fn inventory_default_query_is_light_and_hydrates_checks_per_pr() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![
            light_row(3, "2026-09-01T00:00:00Z", "CLEAN"),
            light_row(4, "2026-09-01T00:00:00Z", "CLEAN"),
        ]);
        gh.views.insert(4, rollup("FAILURE"));

        let read = cached_read(
            tmp.path(),
            &ledger,
            &mut gh,
            now_3891(),
            &PrInventoryOptions::default(),
        )
        .expect("read");

        assert_eq!(
            gh.calls,
            vec![
                "api rate_limit".to_string(),
                light_list_call(),
                "pr view 3 --json statusCheckRollup".to_string(),
                "pr view 4 --json statusCheckRollup".to_string(),
            ]
        );
        assert_eq!(read.source, "github");
        assert_eq!(read.github_calls, 3, "the free probe is not a budget call");
        assert_eq!(read.throttled, None);
        assert_eq!(read.items[0].lifecycle, "MERGE-CANDIDATE");
        assert_eq!(read.items[1].lifecycle, "CI-RED");
    }

    #[test]
    fn inventory_include_body_requests_body_only_when_asked() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(5, "2026-09-01T00:00:00Z", "CLEAN")]);
        gh.views.insert(
            5,
            r#"{"statusCheckRollup":[],"body":"Superseded by #6"}"#.to_string(),
        );
        let options = PrInventoryOptions {
            include: PrInventoryInclude {
                checks: true,
                body: true,
            },
            ..PrInventoryOptions::default()
        };
        let read = cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("read");
        assert!(
            gh.calls
                .iter()
                .any(|call| call == "pr view 5 --json statusCheckRollup,body"),
            "{:?}",
            gh.calls
        );
        assert_eq!(read.items[0].lifecycle, "SUPERSEDED");
        assert_eq!(read.items[0].body, "Superseded by #6");
    }

    #[test]
    fn inventory_include_nothing_skips_hydration_entirely() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(5, "2026-09-01T00:00:00Z", "CLEAN")]);
        let options = PrInventoryOptions {
            include: PrInventoryInclude {
                checks: false,
                body: false,
            },
            ..PrInventoryOptions::default()
        };
        let read = cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("read");
        assert!(
            !gh.calls.iter().any(|call| call.starts_with("pr view")),
            "{:?}",
            gh.calls
        );
        assert_eq!(read.github_calls, 1);
        assert_eq!(read.items[0].ci_status, "UNKNOWN");
    }

    /// AC-1: a second read inside the TTL is served from the cache and spends
    /// no GitHub budget at all.
    #[test]
    fn inventory_read_within_ttl_makes_no_github_call() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(3, "2026-09-01T00:00:00Z", "CLEAN")]);
        let options = PrInventoryOptions::default();
        let first = cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("first");
        gh.calls.clear();

        let later = now_3891() + chrono::Duration::seconds(60);
        let second = cached_read(tmp.path(), &ledger, &mut gh, later, &options).expect("second");

        assert!(
            gh.calls.is_empty(),
            "cache hit must not spawn gh: {:?}",
            gh.calls
        );
        assert_eq!(second.source, "cache");
        assert_eq!(second.github_calls, 0);
        assert_eq!(second.cache_age_secs, Some(60));
        assert_eq!(second.items[0].lifecycle, first.items[0].lifecycle);
        assert_eq!(second.items[0].ci_status, "SUCCESS");
    }

    /// AC-2: heavy fields are re-fetched only for PRs whose real data changed
    /// or whose CI is not final yet.
    #[test]
    fn inventory_hydrates_only_changed_or_pending_prs_after_ttl() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![
            light_row(3, "2026-09-01T00:00:00Z", "CLEAN"),
            light_row(4, "2026-09-01T00:00:00Z", "BLOCKED"),
        ]);
        gh.views.insert(
            4,
            r#"{"statusCheckRollup":[{"conclusion":null,"status":"IN_PROGRESS"}]}"#.to_string(),
        );
        let options = PrInventoryOptions::default();
        cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("first");
        gh.calls.clear();

        // TTL elapsed, nothing changed: only the pending PR is re-checked.
        let second_at = now_3891() + chrono::Duration::seconds(PR_INVENTORY_CACHE_TTL_SECS + 1);
        cached_read(tmp.path(), &ledger, &mut gh, second_at, &options).expect("second");
        assert_eq!(
            gh.calls,
            vec![
                light_list_call(),
                "pr view 4 --json statusCheckRollup".to_string(),
            ],
            "the probe is still fresh and PR 3 is final, so neither is re-read"
        );
        gh.calls.clear();

        // PR 3 got a new commit: its updatedAt moved, so it is hydrated again.
        gh.rows[0] = light_row(3, "2026-09-02T00:30:00Z", "CLEAN");
        let third_at = second_at + chrono::Duration::seconds(PR_INVENTORY_CACHE_TTL_SECS + 1);
        let third = cached_read(tmp.path(), &ledger, &mut gh, third_at, &options).expect("third");
        assert!(
            gh.calls
                .contains(&"pr view 3 --json statusCheckRollup".to_string()),
            "{:?}",
            gh.calls
        );
        assert_eq!(third.items[0].lifecycle, "MERGE-CANDIDATE");
    }

    /// AC-4 / AC-7: with the budget below the reserve, a periodic read is
    /// thinned out — the last snapshot is served, the skip and its reason are
    /// visible, and no GitHub budget is spent.
    #[test]
    fn inventory_below_reserve_serves_stale_cache_and_reports_the_throttle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(3, "2026-09-01T00:00:00Z", "CLEAN")]);
        let options = PrInventoryOptions::default();
        cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("warm");
        gh.calls.clear();

        // The account's GraphQL budget is nearly gone (observed by any process).
        let exhausted_at = now_3891() + chrono::Duration::seconds(PR_INVENTORY_CACHE_TTL_SECS + 1);
        let mut resources = BTreeMap::new();
        resources.insert(
            "graphql".to_string(),
            ResourceWindow {
                limit: 5000,
                remaining: 120,
                reset_at: exhausted_at + chrono::Duration::minutes(30),
            },
        );
        ledger.record_probe(&ProbeSnapshot {
            probed_at: exhausted_at,
            resources,
        });

        let read = cached_read(tmp.path(), &ledger, &mut gh, exhausted_at, &options).expect("read");
        assert!(
            gh.calls.is_empty(),
            "throttled read must not spawn gh: {:?}",
            gh.calls
        );
        assert_eq!(read.source, "stale-cache");
        assert_eq!(read.github_calls, 0);
        let reason = read.throttled.expect("throttle reason");
        assert!(reason.contains("budget_reserve"), "{reason}");
        assert!(reason.contains("remaining=120"), "{reason}");
        assert_eq!(read.items[0].number, 3);
        assert_eq!(read.items[0].ci_status, "SUCCESS");
    }

    #[test]
    fn inventory_below_reserve_without_any_cache_is_unobservable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(3, "2026-09-01T00:00:00Z", "CLEAN")]);
        gh.probe_remaining = 50;
        let error = cached_read(
            tmp.path(),
            &ledger,
            &mut gh,
            now_3891(),
            &PrInventoryOptions::default(),
        )
        .expect_err("no snapshot to serve");
        let message = error.to_string();
        assert!(message.contains("unobservable"), "{message}");
        assert!(message.contains("budget_reserve"), "{message}");
        assert_eq!(gh.calls, vec!["api rate_limit".to_string()]);
    }

    #[test]
    fn inventory_refresh_bypasses_cache_and_throttle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ledger = BudgetLedger::at(&tmp.path().join("budget"));
        let mut gh = FakeGh::new(vec![light_row(3, "2026-09-01T00:00:00Z", "CLEAN")]);
        gh.probe_remaining = 50;
        let options = PrInventoryOptions {
            refresh: true,
            ..PrInventoryOptions::default()
        };
        let read = cached_read(tmp.path(), &ledger, &mut gh, now_3891(), &options).expect("read");
        assert_eq!(read.source, "github");
        assert!(
            gh.calls.iter().any(|call| call.starts_with("pr list")),
            "{:?}",
            gh.calls
        );
        assert!(
            !gh.calls.iter().any(|call| call == "api rate_limit"),
            "an explicit refresh is essential and never probes to throttle itself: {:?}",
            gh.calls
        );
    }
}

use std::{collections::BTreeMap, fmt, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    IssueMonitorCandidateSource, IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorReadiness, IssueMonitorScanSummary, IssueMonitorState, MonitorInboxState,
};
use gwt_github::{Cache, CacheEntry, IssueNumber, IssueState, SectionName};

pub(crate) const ISSUE_MONITOR_TARGETED_REFRESH_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueMonitorDaemonPayload {
    pub event: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedIssueMonitorCandidates {
    pub issues: Vec<IssueMonitorIssue>,
    pub source: IssueMonitorCandidateSource,
    /// The live-list failure that forced a cache fallback, or targeted
    /// readiness-refresh errors attached to an otherwise complete live list.
    /// Kept alongside the read model so failures never render as healthy.
    pub live_error: Option<String>,
}

impl LoadedIssueMonitorCandidates {
    pub fn authorizes_remote_effects(&self) -> bool {
        self.source == IssueMonitorCandidateSource::Live
    }
}

/// External stage owned by one side-effect-free Issue Monitor scan proposal.
/// The stage is retained in errors so operators and tests can distinguish the
/// boundary that failed instead of observing a lossy bool/None/default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorScanStage {
    RemoteResolution,
    CandidateLoad,
    MergeReconciliation,
    DefaultBaseBranch,
    BranchProtection,
    OpenPrReadback,
    HeadShaReadback,
    PrDiffReadback,
    StatusCheckReadback,
    MergeCommitReadback,
    ClaimCompletionReadback,
    /// Issue #3917: PR body / Issue comment readback for delegation evidence.
    MergedIssueSettlementReadback,
    ProposalReturn,
}

impl IssueMonitorScanStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoteResolution => "remote-resolution",
            Self::CandidateLoad => "candidate-load",
            Self::MergeReconciliation => "merge-reconciliation",
            Self::DefaultBaseBranch => "default-base-branch",
            Self::BranchProtection => "branch-protection",
            Self::OpenPrReadback => "open-pr-readback",
            Self::HeadShaReadback => "head-sha-readback",
            Self::PrDiffReadback => "pr-diff-readback",
            Self::StatusCheckReadback => "status-check-readback",
            Self::MergeCommitReadback => "merge-commit-readback",
            Self::ClaimCompletionReadback => "claim-completion-readback",
            Self::MergedIssueSettlementReadback => "merged-issue-settlement-readback",
            Self::ProposalReturn => "proposal-return",
        }
    }
}

impl fmt::Display for IssueMonitorScanStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorScanFailure {
    pub stage: IssueMonitorScanStage,
    pub detail: String,
}

impl IssueMonitorScanFailure {
    pub fn new(stage: IssueMonitorScanStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for IssueMonitorScanFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "issue monitor scan failed at {} stage: {}",
            self.stage, self.detail
        )
    }
}

impl std::error::Error for IssueMonitorScanFailure {}

/// Issue #3933: how a scan stage failed *without* discarding the scan.
///
/// A stage failure used to be fatal to the whole pass, so one slow `gh` call
/// took the launch stage down with it and free agent slots sat idle. These
/// dispositions name what the scan fell back on instead, so a reader can tell a
/// degraded scan (launch still ran) from a stopped one (launch never ran).
///
/// The `last_error` vocabulary is fixed (Issue #3963 AC-4): a line carrying one
/// of these `continued_with_*` tokens means the launch stage still ran, and a
/// line carrying `launch_suppressed` means the scan aborted before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorScanContinuation {
    /// A per-candidate readback exceeded its own budget. That candidate keeps
    /// the value the last successful readback gave it until a later scan reads
    /// it back again; every other candidate is unaffected.
    StaleReadback,
    /// A shared prerequisite failed, so the stage kept the previous scan's
    /// successful result rather than throwing this pass away.
    PreviousCandidates,
    /// Issue #3928 AC-2: the pre-launch readback of a candidate was refused by
    /// GitHub's rate limit. That candidate is left unconfirmed — it cannot be
    /// claimed from cache alone — and stays queued for the scan after the
    /// backoff window; every other candidate and stage is unaffected.
    DeferredCandidates,
}

impl IssueMonitorScanContinuation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StaleReadback => "continued_with_stale_readback",
            Self::PreviousCandidates => "continued_with_previous_candidates",
            Self::DeferredCandidates => "continued_with_deferred_candidates",
        }
    }
}

/// Issue #3928: whether a stage failure is GitHub's rate limit — either the
/// identified refusal (`github_rate_limited: … reset_at=…`) or GitHub's own
/// wording — as opposed to a transport or lookup failure. A rate limit is a
/// wait, not a fault, so the scan degrades the stage instead of aborting.
pub fn is_rate_limit_failure(detail: &str) -> bool {
    detail.contains(gwt_core::github_quota::RATE_LIMITED_ERROR_CODE)
        || gwt_core::github_quota::is_rate_limit_stderr(detail)
}

impl fmt::Display for IssueMonitorScanContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One survived stage failure, kept whole so the status surface can name the
/// stage, the branch it was reading, and what the scan did instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorScanDegradation {
    pub stage: IssueMonitorScanStage,
    /// The branch, base branch, or issue whose value is now unknown.
    pub target: Option<String>,
    pub continuation: IssueMonitorScanContinuation,
    pub detail: String,
}

impl IssueMonitorScanDegradation {
    pub fn new(
        failure: IssueMonitorScanFailure,
        target: Option<String>,
        continuation: IssueMonitorScanContinuation,
    ) -> Self {
        Self {
            stage: failure.stage,
            target,
            continuation,
            detail: failure.detail,
        }
    }
}

impl fmt::Display for IssueMonitorScanDegradation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "scan continued: {} failed", self.stage)?;
        if let Some(target) = &self.target {
            write!(formatter, " for {target}")?;
        }
        write!(formatter, " ({}): {}", self.continuation, self.detail)
    }
}

/// Render every survived failure into the single operator-facing line that
/// `last_error` carries (Issue #3933 AC-4). `None` when the scan was clean.
pub fn issue_monitor_scan_degradation_summary(
    degradations: &[IssueMonitorScanDegradation],
) -> Option<String> {
    if degradations.is_empty() {
        return None;
    }
    Some(
        degradations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// Issue #3933 AC-3: what one per-candidate readback may spend.
///
/// Deliberately a per-call budget rather than a share of the scan deadline. The
/// readbacks are a serial fan-out over candidate branches, so a single shared
/// budget is consumed front-to-back and the last branch inherits whatever is
/// left — in the production incident, nothing.
const ISSUE_MONITOR_READBACK_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

/// The scan budget held back for the launch stage. The readback loop stops
/// issuing new calls once less than this remains, so a long fan-out degrades its
/// own findings instead of starving the launch that fills free slots.
const ISSUE_MONITOR_LAUNCH_RESERVE: std::time::Duration = std::time::Duration::from_secs(10);

pub fn issue_monitor_readback_budget() -> std::time::Duration {
    #[cfg(test)]
    if let Some(budget_ms) = std::env::var_os("GWT_TEST_ISSUE_MONITOR_READBACK_BUDGET_MS")
        .and_then(|value| value.to_string_lossy().parse::<u64>().ok())
        .filter(|budget_ms| *budget_ms <= 60_000)
    {
        return std::time::Duration::from_millis(budget_ms);
    }
    ISSUE_MONITOR_READBACK_BUDGET
}

/// Whether the scan still has budget to start another per-candidate readback
/// and leave the launch stage something to run on. An absent ambient deadline
/// (direct callers, tests) imposes no limit of its own.
fn readback_fan_out_has_budget() -> bool {
    gwt_core::operation_deadline::current().is_none_or(|deadline| {
        deadline.saturating_duration_since(std::time::Instant::now()) > ISSUE_MONITOR_LAUNCH_RESERVE
    })
}

pub fn ensure_scan_deadline(stage: IssueMonitorScanStage) -> Result<(), IssueMonitorScanFailure> {
    gwt_core::operation_deadline::ensure_remaining(stage.as_str())
        .map(|_| ())
        .map_err(|error| IssueMonitorScanFailure::new(stage, error.to_string()))
}

/// Run one per-candidate readback under a budget of its own (Issue #3933 AC-3).
///
/// [`run_scan_stage`] hands the call whatever is left of the shared scan
/// deadline, so in a serial fan-out the first slow branch can spend the entire
/// window and every later branch inherits an already-expired one. Entering a
/// per-call budget caps each branch instead. The budget still nests inside the
/// scan deadline, and the loop refuses to start a candidate without
/// [`ISSUE_MONITOR_LAUNCH_RESERVE`] left, so a call that does start always has a
/// real budget and can never overrun the scan.
pub(crate) fn run_budgeted_readback_stage<T, E>(
    stage: IssueMonitorScanStage,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, IssueMonitorScanFailure>
where
    E: fmt::Display,
{
    let _budget = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
        std::time::Instant::now() + issue_monitor_readback_budget(),
    );
    operation().map_err(|error| IssueMonitorScanFailure::new(stage, error.to_string()))
}

pub(crate) fn run_scan_stage<T, E>(
    stage: IssueMonitorScanStage,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, IssueMonitorScanFailure>
where
    E: fmt::Display,
{
    ensure_scan_deadline(stage)?;
    let value =
        operation().map_err(|error| IssueMonitorScanFailure::new(stage, error.to_string()))?;
    ensure_scan_deadline(stage)?;
    Ok(value)
}

pub fn issue_monitor_daemon_payloads(
    monitor: &mut IssueMonitorState,
    gui_connected: bool,
) -> Vec<IssueMonitorDaemonPayload> {
    monitor.set_gui_connected(gui_connected);
    let mut payloads = Vec::new();
    if gui_connected {
        for request in monitor.take_pending_launch_requests() {
            let delivery_id = request.delivery_id.clone();
            payloads.push(IssueMonitorDaemonPayload {
                event: "launch_request".to_string(),
                payload: serde_json::json!({
                    "issue_number": request.issue_number,
                    "branch_name": request.branch_name,
                    "linked_issue_kind": request.linked_issue_kind,
                    "delivery_id": delivery_id,
                    "launch_session_strategy": request.launch_session_strategy,
                }),
            });
            if request.delivery_id.is_none() {
                payloads.push(IssueMonitorDaemonPayload {
                    event: "toast".to_string(),
                    payload: serde_json::json!({
                        "level": "info",
                        "message": "Issue Monitor launch requested",
                        "issue_number": request.issue_number,
                    }),
                });
            }
        }
        // SPEC #3200 Option A: surface review-agent spawn requests to the GUI.
        for dispatch in monitor.take_pending_review_dispatches() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "review_dispatch".to_string(),
                payload: serde_json::to_value(&dispatch).expect("review dispatch serializes"),
            });
        }
        // SPEC #3200 FR-034 (T-111): surface unattended autonomous lifecycle
        // transitions (merged / needs-human / retry / auto-merge armed) as
        // toasts. Drained only while a GUI is connected so notices queued during
        // a fully-unattended window still reach the operator on the next connect.
        for notice in monitor.take_autonomous_notices() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "toast".to_string(),
                payload: serde_json::json!({
                    "level": notice.level,
                    "message": notice.message,
                    "issue_number": notice.issue_number,
                }),
            });
        }
    }

    payloads.extend(issue_monitor_read_only_daemon_payloads(monitor));
    payloads
}

/// Build the recovery-safe projection without consuming any delivery-bearing
/// outbox. Recovery-blocked daemons use this path because they may expose
/// status and inbox state, but cannot authorize launch/review/toast delivery.
pub fn issue_monitor_read_only_daemon_payloads(
    monitor: &IssueMonitorState,
) -> Vec<IssueMonitorDaemonPayload> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    vec![
        IssueMonitorDaemonPayload {
            event: "status".to_string(),
            payload: serde_json::to_value(monitor.status_view_at(&now))
                .expect("issue monitor status serializes"),
        },
        IssueMonitorDaemonPayload {
            event: "inbox".to_string(),
            payload: serde_json::to_value(monitor.inbox.clone())
                .expect("issue monitor inbox serializes"),
        },
    ]
}

pub fn load_open_issue_monitor_candidates(
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    let issues = gwt_git::issue::fetch_issues(owner, repo).map_err(|error| error.to_string())?;
    Ok(issues
        .into_iter()
        .map(|issue| issue_monitor_candidate(issue, IssueMonitorReadiness::NotApplicable))
        .collect())
}

fn issue_monitor_candidate(
    issue: gwt_git::issue::Issue,
    readiness: IssueMonitorReadiness,
) -> IssueMonitorIssue {
    IssueMonitorIssue {
        number: issue.number,
        title: issue.title,
        labels: issue.labels,
        state: if issue.state.eq_ignore_ascii_case("closed") {
            IssueMonitorIssueState::Closed
        } else {
            IssueMonitorIssueState::Open
        },
        body: issue.body,
        url: (!issue.url.is_empty()).then_some(issue.url),
        readiness,
        updated_at: issue.updated_at,
    }
}

/// Issue #3930 AC-4: the text the acceptance-criteria classifier reads for a
/// gwt-spec Issue. The storage layer may route the `spec` section to a
/// comment (#3864's shape), in which case the Issue body no longer carries the
/// `## 受け入れ基準` block; the assembled section is appended so the Monitor
/// sees the same block regardless of where it is stored. A body-resident
/// section is already inside `body`, so nothing is duplicated.
fn acceptance_source_text(body: &str, entry: &CacheEntry) -> String {
    let spec = SectionName("spec".to_string());
    let comment_resident = matches!(
        entry.spec_body.sections_index.0.get(&spec),
        Some(gwt_github::SectionLocation::Comments(_))
    );
    match entry.spec_body.sections.get(&spec) {
        Some(content) if comment_resident && !content.trim().is_empty() => {
            format!("{body}\n\n{content}")
        }
        _ => body.to_string(),
    }
}

fn spec_cache_entry_readiness(entry: &CacheEntry) -> IssueMonitorReadiness {
    let section = |name: &str| {
        entry
            .spec_body
            .sections
            .get(&SectionName(name.to_string()))
            .map(String::as_str)
            .filter(|content| !content.trim().is_empty())
    };
    let (Some(_plan), Some(tasks)) = (section("plan"), section("tasks")) else {
        return IssueMonitorReadiness::NotReady;
    };
    let mut open_fence = None;
    let checkbox_states = tasks.lines().filter_map(|line| {
        let content = line.trim_start_matches([' ', '\t']);
        let indentation = &line[..line.len() - content.len()];
        if indentation.len() > 3 || indentation.contains('\t') {
            return markdown_list_item(content)
                .filter(|item| item.starts_with('['))
                .map(|_| None);
        }
        let fence = markdown_fence(content);
        if let Some((open_marker, open_length)) = open_fence {
            if fence.is_some_and(|(marker, length, suffix)| {
                marker == open_marker && length >= open_length && suffix.trim().is_empty()
            }) {
                open_fence = None;
            }
            return None;
        }
        if let Some((marker, length, _)) = fence {
            open_fence = Some((marker, length));
            return None;
        }
        let item = markdown_list_item(content)?;
        if item.starts_with("[ ]") {
            Some(Some(false))
        } else if item.starts_with("[x]") || item.starts_with("[X]") {
            Some(Some(true))
        } else if item.starts_with('[') {
            // A checkbox-like task with an unknown marker must never turn a
            // partially parsed task list into Issue-wide completion.
            Some(None)
        } else {
            None
        }
    });
    let mut saw_checkbox = false;
    let mut saw_open = false;
    for checked in checkbox_states {
        if let Some(checked) = checked {
            saw_checkbox = true;
            saw_open |= !checked;
        } else {
            saw_open = true;
        }
    }
    if saw_open {
        IssueMonitorReadiness::ReadyWithOpenTasks
    } else if saw_checkbox {
        IssueMonitorReadiness::ReadyWithCompletedTasks
    } else {
        IssueMonitorReadiness::Ready
    }
}

fn markdown_fence(line: &str) -> Option<(u8, usize, &str)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = line
        .bytes()
        .take_while(|candidate| *candidate == marker)
        .count();
    (length >= 3).then_some((marker, length, &line[length..]))
}

fn markdown_list_item(line: &str) -> Option<&str> {
    if let Some(item) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some(item.trim_start());
    }
    let (marker, item) = line.split_once(char::is_whitespace)?;
    let ordered = marker
        .strip_suffix('.')
        .or_else(|| marker.strip_suffix(')'))?;
    (!ordered.is_empty() && ordered.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(item.trim_start())
}

fn issue_monitor_candidates_with_readiness<F>(
    issues: Vec<gwt_git::issue::Issue>,
    cache_root: &Path,
    refresh: F,
) -> (Vec<IssueMonitorIssue>, Vec<String>)
where
    F: FnMut(IssueNumber) -> Result<(), String>,
{
    issue_monitor_candidates_with_readiness_and_refresh_limit(
        issues,
        cache_root,
        ISSUE_MONITOR_TARGETED_REFRESH_LIMIT,
        refresh,
    )
}

fn issue_monitor_candidates_with_readiness_and_refresh_limit<F>(
    issues: Vec<gwt_git::issue::Issue>,
    cache_root: &Path,
    refresh_limit: usize,
    mut refresh: F,
) -> (Vec<IssueMonitorIssue>, Vec<String>)
where
    F: FnMut(IssueNumber) -> Result<(), String>,
{
    let cache = Cache::new(cache_root.to_path_buf());
    let mut candidates = Vec::with_capacity(issues.len());
    let mut errors = Vec::new();
    let mut refresh_count = 0;

    for issue in issues {
        let is_spec = issue
            .labels
            .iter()
            .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
        if !is_spec {
            candidates.push(issue_monitor_candidate(
                issue,
                IssueMonitorReadiness::NotApplicable,
            ));
            continue;
        }

        let number = IssueNumber(issue.number);
        let cached = cache.load_entry(number);
        let cache_matches_live = issue.updated_at.as_ref().is_some_and(|updated_at| {
            cached
                .as_ref()
                .is_some_and(|entry| entry.snapshot.updated_at.0 == *updated_at)
        });
        let entry = if cache_matches_live {
            cached
        } else if refresh_count >= refresh_limit {
            errors.push(format!(
                "issue #{} targeted refresh skipped: per-scan limit {} reached",
                issue.number, refresh_limit
            ));
            None
        } else {
            refresh_count += 1;
            match refresh(number) {
                Ok(()) => {
                    let refreshed = cache.load_entry(number);
                    match refreshed {
                        Some(entry)
                            if issue.updated_at.as_ref().is_some_and(|updated_at| {
                                entry.snapshot.updated_at.0 == *updated_at
                            }) =>
                        {
                            Some(entry)
                        }
                        Some(entry) => {
                            errors.push(format!(
                                "issue #{} targeted refresh generation mismatch: live={}, cache={}",
                                issue.number,
                                issue.updated_at.as_deref().unwrap_or("missing"),
                                entry.snapshot.updated_at.0
                            ));
                            None
                        }
                        None => {
                            errors.push(format!(
                                "issue #{} targeted refresh parse failed",
                                issue.number
                            ));
                            None
                        }
                    }
                }
                Err(error) => {
                    errors.push(format!(
                        "issue #{} targeted refresh failed: {error}",
                        issue.number
                    ));
                    None
                }
            }
        };
        let readiness = entry
            .as_ref()
            .map(spec_cache_entry_readiness)
            .unwrap_or(IssueMonitorReadiness::NotReady);
        let mut issue = issue;
        if let Some(entry) = entry.as_ref() {
            // Issue #3930 AC-4: the generation-matched cache entry knows where
            // the `spec` section lives; a comment-resident block must reach
            // the acceptance classifier too.
            let body = issue.body.as_deref().unwrap_or(&entry.snapshot.body);
            issue.body = Some(acceptance_source_text(body, entry));
        }
        candidates.push(issue_monitor_candidate(issue, readiness));
    }

    (candidates, errors)
}

pub fn load_open_issue_monitor_candidates_for_repo_path(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    load_open_issue_monitor_candidates_for_repo_path_with_provenance(repo_path, owner, repo)
        .map(|loaded| loaded.issues)
}

/// Load live candidates when available, retaining typed provenance for capped
/// (therefore incomplete) live lists and cache fallbacks. The existing
/// Vec-returning API above remains a compatibility wrapper.
pub fn load_open_issue_monitor_candidates_for_repo_path_with_provenance(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<LoadedIssueMonitorCandidates, String> {
    let live_error = match gwt_git::issue::fetch_issues(owner, repo) {
        Ok(raw_issues) => {
            let cache_root = crate::issue_cache::issue_cache_root_for_repo_path(repo_path)
                .unwrap_or_else(|| crate::issue_cache::issue_cache_root_for_repo_slug(owner, repo));
            let (issues, readiness_errors) =
                issue_monitor_candidates_with_readiness(raw_issues, &cache_root, |number| {
                    crate::issue_cache::refresh_issue_cache_entry_from_remote(
                        repo_path,
                        &cache_root,
                        number,
                    )
                });
            let source = live_candidate_source(issues.len());
            return Ok(LoadedIssueMonitorCandidates {
                issues,
                source,
                live_error: (!readiness_errors.is_empty()).then(|| readiness_errors.join("; ")),
            });
        }
        Err(error) => error.to_string(),
    };
    let cache_roots = [
        crate::issue_cache::issue_cache_root_for_repo_path(repo_path),
        Some(crate::issue_cache::issue_cache_root_for_repo_slug(
            owner, repo,
        )),
    ];
    let cache_results = cache_roots.into_iter().flatten().map(|cache_root| {
        let result = load_cached_issue_monitor_candidates(&cache_root);
        if let Err(error) = &result {
            tracing::warn!(
                "issue monitor cache fallback failed for {}: {error}",
                cache_root.display()
            );
        }
        result
    });
    resolve_loaded_issue_monitor_candidates(Err(live_error), cache_results)
}

fn resolve_loaded_issue_monitor_candidates<I>(
    live_result: Result<Vec<IssueMonitorIssue>, String>,
    cache_results: I,
) -> Result<LoadedIssueMonitorCandidates, String>
where
    I: IntoIterator<Item = Result<Vec<IssueMonitorIssue>, String>>,
{
    match live_result {
        Ok(issues) => {
            let source = live_candidate_source(issues.len());
            Ok(LoadedIssueMonitorCandidates {
                issues,
                source,
                live_error: None,
            })
        }
        Err(live_error) => {
            for issues in cache_results.into_iter().flatten() {
                if !issues.is_empty() {
                    return Ok(LoadedIssueMonitorCandidates {
                        issues,
                        source: IssueMonitorCandidateSource::Cache,
                        live_error: Some(live_error),
                    });
                }
            }
            Err(live_error)
        }
    }
}

fn live_candidate_source(issue_count: usize) -> IssueMonitorCandidateSource {
    let configured_limit = gwt_git::issue::GITHUB_ISSUE_LIST_LIMIT
        .parse::<usize>()
        .unwrap_or(usize::MAX);
    if issue_count < configured_limit {
        IssueMonitorCandidateSource::Live
    } else {
        IssueMonitorCandidateSource::LiveIncomplete
    }
}

/// Shared loader-to-state transition. Cache snapshots still follow the normal
/// candidate scan, but only Live provenance can unlock the one-shot historical
/// failure migration in the canonical core transition.
pub fn scan_loaded_issue_monitor_candidates(
    monitor: &mut IssueMonitorState,
    loaded: &LoadedIssueMonitorCandidates,
    repo_path: &Path,
    now: &str,
) -> IssueMonitorScanSummary {
    scan_loaded_issue_monitor_candidates_for_project_tab(monitor, loaded, repo_path, None, now)
}

/// GUI-bound variant of the shared loader transition. The project tab id is
/// optional because daemon scans have repository provenance but no GUI tab
/// identity; they still reconcile repo-absent in-flight rows on Live input.
pub fn scan_loaded_issue_monitor_candidates_for_project_tab(
    monitor: &mut IssueMonitorState,
    loaded: &LoadedIssueMonitorCandidates,
    repo_path: &Path,
    expected_project_tab_id: Option<&str>,
    now: &str,
) -> IssueMonitorScanSummary {
    let summary =
        crate::issue_monitor::scan_issue_monitor_candidates_for_project_tab_with_provenance(
            monitor,
            &loaded.issues,
            loaded.source,
            repo_path,
            expected_project_tab_id,
            now,
        );
    // Issue #3964 AC-1: every scan — daemon or GUI fallback — asks the owner
    // ledger whether a generation-conflict hold still protects anything. The
    // reaper released 29 of the 45 stranded production generations and their
    // rows stayed `agent_failed` regardless, because nothing told the monitor.
    monitor.release_stranded_generation_failures(now, |issue_number| {
        match crate::cli::execution_state::owner_generation_hold_for_project(
            repo_path,
            issue_number,
        ) {
            Ok(hold) => hold,
            Err(error) => {
                tracing::debug!(
                    issue = issue_number,
                    %error,
                    "owner generation ledger could not be read; the hold stays in place"
                );
                None
            }
        }
    });
    if let Some(error) = &loaded.live_error {
        let message = if loaded.source == IssueMonitorCandidateSource::Cache {
            format!("issue list failed; using cache fallback: {error}")
        } else {
            format!("issue readiness refresh failed: {error}")
        };
        monitor.record_scan_error(now, message);
    }
    summary
}

/// Issue #3933 AC-2 (review follow-up): read one candidate's authoritative state
/// before the launch stage acts on a previous scan's result.
///
/// [`try_issue_completed_by_merged_pr`] decides an ordinary Issue purely from the
/// loaded row's own `state`, so a previous-result row that GitHub has since
/// closed would still read as launchable. A targeted single-issue refresh is far
/// cheaper than the issue list whose failure put the scan on this path, and an
/// unreadable state is reported as a failure rather than assumed Open.
pub fn try_refresh_issue_monitor_candidate(
    repo_path: &Path,
    owner: &str,
    repo: &str,
    issue: &IssueMonitorIssue,
) -> Result<IssueMonitorIssue, IssueMonitorScanFailure> {
    let cache_root = crate::issue_cache::issue_cache_root_for_repo_path(repo_path)
        .unwrap_or_else(|| crate::issue_cache::issue_cache_root_for_repo_slug(owner, repo));
    let number = IssueNumber(issue.number);
    run_budgeted_readback_stage(IssueMonitorScanStage::CandidateLoad, || {
        crate::issue_cache::refresh_issue_cache_entry_from_remote(repo_path, &cache_root, number)
    })?;
    let entry = Cache::new(cache_root).load_entry(number).ok_or_else(|| {
        IssueMonitorScanFailure::new(
            IssueMonitorScanStage::CandidateLoad,
            format!(
                "issue #{number} has no cache entry after a targeted refresh",
                number = issue.number
            ),
        )
    })?;
    Ok(IssueMonitorIssue {
        state: match entry.snapshot.state {
            IssueState::Open => IssueMonitorIssueState::Open,
            IssueState::Closed => IssueMonitorIssueState::Closed,
        },
        updated_at: Some(entry.snapshot.updated_at.0),
        ..issue.clone()
    })
}

/// Issue #3225 / #3832: GitHub-derived completion probe for the claim loop.
/// Ordinary Issues are terminal only when GitHub reports `Closed`; linked PR
/// evidence remains delivery evidence and cannot suppress an Open Issue.
/// SPECs retain their structured-task plus merged-PR gate. Fails open (false)
/// on remote errors so a transient gh failure never blocks real work.
pub fn issue_completed_by_merged_pr(owner: &str, repo: &str, issue: &IssueMonitorIssue) -> bool {
    match try_issue_completed_by_merged_pr(owner, repo, issue) {
        Ok(completed) => completed,
        Err(error) => {
            tracing::debug!(
                issue = issue.number,
                error = %error,
                "issue monitor completion probe failed (fail-open)"
            );
            false
        }
    }
}

/// Checked completion probe used by scan proposal transactions.
pub fn try_issue_completed_by_merged_pr(
    owner: &str,
    repo: &str,
    issue: &IssueMonitorIssue,
) -> Result<bool, IssueMonitorScanFailure> {
    try_issue_completed_by_merged_pr_classified(owner, repo, issue)
        .map_err(IssueMonitorCompletionProbeFailure::into_failure)
}

/// Issue #3528 (SPEC #3200 FR-059): how a completion probe failed. The two
/// arms carry different contracts — an expired observation deadline is
/// fail-closed for the whole claim proposal, an ordinary readback error keeps
/// #3165's fail-open compatibility — so a caller must be able to tell them
/// apart without parsing the failure text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueMonitorCompletionProbeFailure {
    /// The observation deadline expired before or right after the readback.
    Deadline(IssueMonitorScanFailure),
    /// The readback itself failed while the deadline was still valid.
    Operation(IssueMonitorScanFailure),
}

impl IssueMonitorCompletionProbeFailure {
    pub fn into_failure(self) -> IssueMonitorScanFailure {
        match self {
            Self::Deadline(failure) | Self::Operation(failure) => failure,
        }
    }
}

/// [`try_issue_completed_by_merged_pr`] with the deadline expiry told apart
/// from an ordinary readback failure.
pub fn try_issue_completed_by_merged_pr_classified(
    owner: &str,
    repo: &str,
    issue: &IssueMonitorIssue,
) -> Result<bool, IssueMonitorCompletionProbeFailure> {
    let is_spec = issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
    if !is_spec {
        return Ok(issue.state == IssueMonitorIssueState::Closed);
    }
    let stage = IssueMonitorScanStage::ClaimCompletionReadback;
    ensure_scan_deadline(stage).map_err(IssueMonitorCompletionProbeFailure::Deadline)?;
    let prs = crate::cli::issue::fetch_linked_prs_via_gh(
        owner,
        repo,
        gwt_github::IssueNumber(issue.number),
    )
    .map_err(|error| {
        IssueMonitorCompletionProbeFailure::Operation(IssueMonitorScanFailure::new(
            stage,
            error.to_string(),
        ))
    })?;
    ensure_scan_deadline(stage).map_err(IssueMonitorCompletionProbeFailure::Deadline)?;
    Ok(linked_pr_completion_is_fresh_for_issue(issue, &prs))
}

pub fn linked_pr_completion_is_fresh_for_issue(
    issue: &IssueMonitorIssue,
    prs: &[crate::cli::LinkedPrSummary],
) -> bool {
    let is_spec = issue
        .labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
    if !is_spec {
        return issue.state == IssueMonitorIssueState::Closed;
    }
    if issue
        .updated_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_none()
        || issue.readiness != IssueMonitorReadiness::ReadyWithCompletedTasks
    {
        return false;
    }
    let closing_merged = prs
        .iter()
        .filter(|pr| pr.will_close_target && pr.state.eq_ignore_ascii_case("merged"));
    closing_merged.into_iter().next().is_some()
}

/// Reconcile work branches that have merged, freeing the slot and delegating
/// Issue-wide completion vs requeue to the domain policy. Returns query
/// failures so the scan owner can surface them after its final state rebase.
///
/// Two entrances, one query. The tracked one matches `active_launched_branches`
/// and is the ordinary path. The untracked one (Issue #3645 AC-3/AC-4) exists
/// because emptying `launched_issues` — the repair the 2026-08-17 outage
/// required — makes every launch that was in flight invisible to the first: the
/// work merges, nothing records it, and the Issue stays a relaunch candidate
/// forever. Candidates there are nominated locally from the merged-branch list
/// and confirmed by [`try_issue_completed_by_merged_pr`]. Ordinary Open rows
/// never become terminal through this path; complete SPECs retain their linked
/// PR gate. Bounding the probe to branch matches keeps it off the whole queue.
pub fn reconcile_issue_monitor_merges(
    monitor: &mut IssueMonitorState,
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> gwt_core::Result<IssueMonitorMergeReconciliation> {
    if monitor.active_launched_branches().is_empty()
        && !monitor.has_open_launch_plan_candidates()
        && !monitor.has_merged_issue_settlement_prospects()
    {
        return Ok(IssueMonitorMergeReconciliation::default());
    }
    let merged_prs = gwt_git::pr_status::fetch_merged_pr_deliveries(repo_path)?;
    let merged_branches = merged_prs.branches;
    let deliveries = merged_prs
        .deliveries
        .into_iter()
        .map(|(branch, delivery)| {
            (
                branch,
                crate::MergedIssueDelivery {
                    pr_number: delivery.number,
                    merge_sha: delivery.merge_sha,
                    merged_at: delivery.merged_at,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut merged = monitor.reconcile_merged_branches(&merged_branches);
    if !merged.is_empty() {
        tracing::info!(
            issues = ?merged,
            "issue monitor reconciled merged work deliveries and freed active slots"
        );
    }

    for issue in monitor.untracked_merged_branch_candidates(&merged_branches) {
        let issue_number = issue.number;
        // A probe failure keeps the issue launchable, matching the claim-path
        // policy: an unreachable GitHub must never mint a terminal completion.
        match try_issue_completed_by_merged_pr(owner, repo, &issue) {
            Ok(true) => {
                if monitor.record_untracked_completion(issue_number) {
                    tracing::info!(
                        issue = issue_number,
                        "issue monitor recovered a completion its launch tracking had lost"
                    );
                    merged.push(issue_number);
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    issue = issue_number,
                    error = %error,
                    "issue monitor untracked completion probe failed; issue stays launchable"
                );
            }
        }
    }
    Ok(IssueMonitorMergeReconciliation { merged, deliveries })
}

/// Result of one merged-branch reconciliation: the Issues whose slots were
/// freed plus, keyed by head branch, the latest merged delivery the same
/// query returned (Issue #3917).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueMonitorMergeReconciliation {
    pub merged: Vec<u64>,
    pub deliveries: BTreeMap<String, crate::MergedIssueDelivery>,
}

/// Delete the remote `work/issue-*` branches whose delivery this scan just
/// confirmed (Issue #3970 AC-1).
///
/// The trigger is a *new* reconciliation, not the merged-PR list: that list
/// keeps naming branches long after their heads are gone, so pruning off it
/// every scan would push hundreds of no-op deletions forever. Gating on
/// `reconciliation.merged` makes the pass cost nothing on an ordinary scan and
/// run exactly when a delivery lands.
///
/// The pass then sweeps every `work/issue-*` branch the remote still has, not
/// only the one that just merged, so a backlog that accumulated before this
/// existed drains through the same safe rules instead of needing a separate
/// migration.
pub fn prune_delivered_work_branches(
    repo_path: &Path,
    base_branch: &str,
    reconciliation: &IssueMonitorMergeReconciliation,
) -> gwt_git::merged_branch_prune::PruneReport {
    let mut report = gwt_git::merged_branch_prune::PruneReport::default();
    if reconciliation.merged.is_empty() {
        return report;
    }
    // Mirrors the gh command root: a workspace home resolves to its child bare
    // repo, anything else runs where the caller pointed us.
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let repo_path = git_root.as_path();
    if let Err(error) = gwt_git::merged_branch_prune::refresh_remote_refs(repo_path) {
        report.skipped_reason = Some(format!("git fetch origin --prune failed: {error}"));
        return report;
    }
    let branches = match gwt_git::merged_branch_prune::list_remote_work_branches(repo_path) {
        Ok(branches) => branches,
        Err(error) => {
            report.skipped_reason = Some(format!("git ls-remote failed: {error}"));
            return report;
        }
    };
    // The merged-PR inventory this scan already paid for is reused, so the
    // prune adds one `gh` call (the open-PR inventory), not two.
    let merged = reconciliation
        .deliveries
        .iter()
        .map(|(branch, delivery)| (branch.clone(), delivery.pr_number))
        .collect::<BTreeMap<_, _>>();
    let mut environment =
        gwt_git::merged_branch_prune::GitPruneEnvironment::new(repo_path, base_branch)
            .with_merged_prs(merged);
    prune_delivered_work_branches_with(&mut environment, reconciliation, &branches)
}

/// Injectable core of [`prune_delivered_work_branches`].
pub fn prune_delivered_work_branches_with<E: gwt_git::merged_branch_prune::PruneEnvironment>(
    env: &mut E,
    reconciliation: &IssueMonitorMergeReconciliation,
    remote_branches: &[String],
) -> gwt_git::merged_branch_prune::PruneReport {
    if reconciliation.merged.is_empty() {
        return gwt_git::merged_branch_prune::PruneReport::default();
    }
    gwt_git::merged_branch_prune::prune_merged_branches(env, remote_branches, false)
}

/// Issue #3917: propose settling every delivered Issue whose work branch
/// merged. Side-effect free like the rest of the scan: it only prepares
/// `SettleMergedIssue` effects for the durable executor. Delegation
/// evidence (PR body, Issue comments) is read only when unchecked criteria
/// remain; a failed readback defers that Issue to the next scan instead of
/// escalating it. Returns the Issue numbers proposed this scan.
pub fn propose_merged_issue_settlements(
    monitor: &mut IssueMonitorState,
    repo_path: &Path,
    owner: &str,
    repo: &str,
    deliveries: &BTreeMap<String, crate::MergedIssueDelivery>,
) -> Vec<u64> {
    if !monitor.config.enabled || deliveries.is_empty() {
        return Vec::new();
    }
    let auto_close = monitor.auto_close_merged_issues_enabled();
    let mut proposed = Vec::new();
    for (issue, delivery) in monitor.merged_issue_settlement_candidates(deliveries) {
        let issue_number = issue.number;
        let pr_number = delivery.pr_number;
        let evidence = |_unmet: &[String]| -> Option<bool> {
            match fetch_delegation_evidence(repo_path, owner, repo, issue_number, pr_number) {
                Ok(texts) => Some(crate::delegation_recorded(texts.iter().map(String::as_str))),
                Err(error) => {
                    tracing::warn!(
                        issue = issue_number,
                        pr = pr_number,
                        error = %error,
                        "merged issue settlement evidence readback failed; deferring to the next scan"
                    );
                    None
                }
            }
        };
        let Some(action) = crate::decide_merged_issue_settlement(&issue, auto_close, evidence)
        else {
            continue;
        };
        if monitor.propose_merged_issue_settlement(issue_number, &delivery, action) {
            proposed.push(issue_number);
        }
    }
    proposed
}

/// PR body plus Issue comment bodies, the two places a delegation record may
/// live (Issue #3917 AC-2).
fn fetch_delegation_evidence(
    repo_path: &Path,
    owner: &str,
    repo: &str,
    issue_number: u64,
    pr_number: u64,
) -> Result<Vec<String>, IssueMonitorScanFailure> {
    let mut texts = Vec::new();
    if let Some(body) =
        run_scan_stage(IssueMonitorScanStage::MergedIssueSettlementReadback, || {
            gwt_git::pr_status::try_fetch_pr_body(repo_path, pr_number)
        })?
    {
        texts.push(body);
    }
    texts.extend(run_scan_stage(
        IssueMonitorScanStage::MergedIssueSettlementReadback,
        || gwt_git::issue::fetch_issue_comment_bodies(owner, repo, issue_number),
    )?);
    Ok(texts)
}

/// Parse `git symbolic-ref --short refs/remotes/origin/HEAD` output (e.g.
/// `origin/main`) into the bare default branch name. Fail-closed to `main`.
pub fn parse_default_base_branch(symbolic_ref_stdout: &str) -> String {
    let trimmed = symbolic_ref_stdout.trim();
    let name = trimmed.strip_prefix("origin/").unwrap_or(trimmed);
    if name.is_empty() {
        "main".to_string()
    } else {
        name.to_string()
    }
}

/// Branch assumed when `origin/HEAD` cannot be read.
const DEFAULT_BASE_BRANCH_FALLBACK: &str = "main";

/// Resolve the repo's default base branch (the branch autonomous PRs merge
/// into) via `origin/HEAD`. Fail-closed to `main` on any failure.
pub fn resolve_default_base_branch(repo_path: &Path) -> String {
    try_resolve_default_base_branch(repo_path).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "issue monitor default base branch unavailable");
        DEFAULT_BASE_BRANCH_FALLBACK.to_string()
    })
}

/// Checked default-base resolution used by deadline-integral scans.
///
/// The scan deadline is the only condition that may abort the caller here.
/// `origin/HEAD` is an optional local hint — a repository that never configured
/// it (a bare `git init` under a workspace home, for example) is a normal
/// state, not a scan failure. Treating it as one disabled the whole autonomous
/// progress loop before it could reach `gh` (Issue #3348), so every
/// non-deadline outcome keeps the historical `main` fallback.
pub fn try_resolve_default_base_branch(
    repo_path: &Path,
) -> Result<String, IssueMonitorScanFailure> {
    ensure_scan_deadline(IssueMonitorScanStage::DefaultBaseBranch)?;
    // Mirrors the gh command root: a workspace home resolves to its child bare
    // repo, anything else runs where the caller pointed us.
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        gwt_core::process_console::SpawnOptions::new("git symbolic-ref origin/HEAD")
            .current_dir(&git_root),
    );
    if let Err(error) = &output {
        if error.kind() == std::io::ErrorKind::TimedOut {
            return Err(IssueMonitorScanFailure::new(
                IssueMonitorScanStage::DefaultBaseBranch,
                format!("git symbolic-ref origin/HEAD exceeded the scan deadline: {error}"),
            ));
        }
    }
    ensure_scan_deadline(IssueMonitorScanStage::DefaultBaseBranch)?;
    match output {
        Ok(output) if output.success() && !output.stdout.trim().is_empty() => {
            let branch = parse_default_base_branch(&output.stdout);
            if branch.trim().is_empty() {
                Ok(DEFAULT_BASE_BRANCH_FALLBACK.to_string())
            } else {
                Ok(branch)
            }
        }
        _ => Ok(DEFAULT_BASE_BRANCH_FALLBACK.to_string()),
    }
}

/// SPEC #3200 T-041: apply the pre-launch autonomous eligibility gate to every
/// two-stage candidate before the scan claims/launches them. For each candidate
/// it fetches the base-branch protection and runs
/// [`IssueMonitorState::prepare_autonomous_candidate`], which escalates
/// ineligible issues to `NeedsHuman` (removing them from the launch queue) and
/// captures the acceptance snapshot + `Implementing` phase for eligible ones.
/// A no-op unless autonomous mode is on (default OFF preserves SPEC #3165).
pub fn apply_autonomous_eligibility(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    now: &str,
) {
    if let Err(error) = try_apply_autonomous_eligibility(monitor, issues, repo_slug, repo_path, now)
    {
        tracing::warn!(error = %error, "issue monitor autonomous eligibility scan failed");
    }
}

pub fn try_apply_autonomous_eligibility(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    now: &str,
) -> Result<Vec<IssueMonitorScanDegradation>, IssueMonitorScanFailure> {
    if !monitor.config.enabled || !monitor.autonomous_mode() {
        return Ok(Vec::new());
    }
    // Only fetch branch protection for candidates whose transient-retry backoff
    // window has elapsed (retry_ready) — a backed-off issue is skipped this scan
    // without a network call (SPEC #3200 T-043/FR-029).
    let candidates = autonomous_eligibility_candidates(monitor, issues, now);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let base_branch = try_resolve_default_base_branch(repo_path)?;
    // Issue #3933 AC-2: an unreadable protection snapshot is not a reason to
    // throw the pass away. Every candidate keeps the eligibility decision the
    // last successful scan gave it, and the launch stage still runs — inventing
    // a decision from an absent snapshot is the one thing that stays forbidden.
    let protection = match run_scan_stage(IssueMonitorScanStage::BranchProtection, || {
        gwt_git::branch_protection::try_fetch_branch_protection(repo_slug, &base_branch)
    }) {
        Ok(protection) => protection,
        Err(failure) => {
            return Ok(vec![IssueMonitorScanDegradation::new(
                failure,
                Some(base_branch),
                IssueMonitorScanContinuation::PreviousCandidates,
            )]);
        }
    };
    for issue in candidates {
        let _ = monitor.prepare_autonomous_candidate(issue, &protection, now);
    }
    Ok(Vec::new())
}

fn autonomous_eligibility_candidates<'a>(
    monitor: &IssueMonitorState,
    issues: &'a [IssueMonitorIssue],
    now: &str,
) -> Vec<&'a IssueMonitorIssue> {
    issues
        .iter()
        .filter(|issue| monitor.is_autonomous_two_stage_candidate(issue))
        .filter(|issue| {
            monitor
                .inbox_item(issue.number)
                .is_some_and(|item| item.state == MonitorInboxState::Queued)
        })
        .filter(|issue| monitor.retry_ready(issue.number, now))
        .collect()
}

/// SPEC #3200 Option A (daemon-direct + token): advance every in-flight
/// autonomous issue one step through the loop, using freshly-fetched signals.
///
/// - **Implementing** → detect the implementation agent's open PR; on discovery
///   bind it (`begin_review`) and emit a `review_dispatch` so the GUI spawns the
///   independent review agent.
/// - **Reviewing** → once the verdict has arrived, assemble the strong-gate
///   inputs and route: `Deliver` arms the auto-merge (after minting an audit
///   token); `Remediate` re-queues (bounded); `Escalate` → NeedsHuman; `WaitForCi`
///   waits.
/// - **Delivering** → watch for the merge; on merge verify `merged_sha ==
///   reviewed_sha` (TOCTOU layer-4) before completing, else escalate.
///
/// No-op unless autonomous mode is on. Review-dispatch requests are queued on the
/// monitor ([`IssueMonitorState::take_pending_review_dispatches`]) for the GUI to
/// spawn the review agents.
pub fn advance_autonomous_in_flight(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    daemon_secret: &[u8],
    now: &str,
) {
    if let Err(error) =
        try_advance_autonomous_in_flight(monitor, issues, repo_slug, repo_path, daemon_secret, now)
    {
        tracing::warn!(error = %error, "issue monitor autonomous progress scan failed");
    }
}

pub fn try_advance_autonomous_in_flight(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    daemon_secret: &[u8],
    now: &str,
) -> Result<Vec<IssueMonitorScanDegradation>, IssueMonitorScanFailure> {
    if !monitor.config.enabled || !monitor.autonomous_mode() {
        return Ok(Vec::new());
    }
    let base_branch = try_resolve_default_base_branch(repo_path)?;
    let mut degradations = Vec::new();
    let in_flight = monitor.autonomous_in_flight_issues();
    // Issue #3963 AC-2: the open-PR readback is ONE inventory per scan. The
    // eligibility gate marks every eligible queued candidate Implementing before
    // it is launched, so the in-flight set is queue-sized, not slot-sized, and a
    // `gh pr list --head` per candidate grew with the queue until the fan-out
    // could not finish inside any scan budget.
    let open_prs = read_open_pr_inventory(monitor, &in_flight, repo_path, &mut degradations);
    for issue_number in in_flight {
        // The branch this candidate's readbacks are about, named up front so a
        // failure can say which one went unknown even when it failed before the
        // phase arm resolved it (Issue #3933 AC-4).
        let target = autonomous_readback_target(monitor, issue_number);
        // Issue #3933: the fan-out is bounded by the scan budget, not by the
        // first slow call. Once too little is left to also run the launch
        // stage, the remaining candidates stay unknown rather than starving the
        // launch that fills free agent slots.
        if !readback_fan_out_has_budget() {
            degradations.push(IssueMonitorScanDegradation {
                stage: IssueMonitorScanStage::OpenPrReadback,
                target,
                continuation: IssueMonitorScanContinuation::StaleReadback,
                detail: "scan budget reserved for the launch stage".to_string(),
            });
            continue;
        }
        // Issue #3933 AC-1: one candidate's failed readback degrades that
        // candidate alone. Its record is left exactly as the last successful
        // scan wrote it, and the loop moves on to the next candidate.
        if let Err(failure) = advance_one_autonomous_issue(
            monitor,
            issues,
            repo_slug,
            repo_path,
            &base_branch,
            open_prs.as_ref(),
            daemon_secret,
            issue_number,
            now,
        ) {
            tracing::warn!(
                issue = issue_number,
                stage = %failure.stage,
                error = %failure.detail,
                "issue monitor readback degraded; keeping the previous value"
            );
            degradations.push(IssueMonitorScanDegradation::new(
                failure,
                target,
                IssueMonitorScanContinuation::StaleReadback,
            ));
        }
    }
    Ok(degradations)
}

/// The launch branch a candidate's readbacks address, falling back to the issue
/// itself when no launch plan has been recorded yet.
fn autonomous_readback_target(monitor: &IssueMonitorState, issue_number: u64) -> Option<String> {
    Some(
        monitor
            .inbox_item(issue_number)
            .and_then(|item| item.launch_plan.as_ref())
            .map(|plan| plan.branch_name.clone())
            .unwrap_or_else(|| format!("issue #{issue_number}")),
    )
}

/// Issue #3963 AC-2/AC-3: the one open-PR inventory a scan reads for its
/// Implementing candidates, keyed by head branch.
///
/// `None` when no candidate needs it or the readback degraded. A degraded
/// inventory is recorded once, naming how many candidates it left unknown; the
/// Implementing arm then leaves every one of them on the phase the last
/// successful scan gave it, and the launch stage still runs.
fn read_open_pr_inventory(
    monitor: &IssueMonitorState,
    in_flight: &[u64],
    repo_path: &Path,
    degradations: &mut Vec<IssueMonitorScanDegradation>,
) -> Option<std::collections::HashMap<String, u64>> {
    let implementing = in_flight
        .iter()
        .filter(|issue_number| {
            monitor
                .autonomous_record(**issue_number)
                .is_some_and(|record| record.phase == crate::AutonomousPhase::Implementing)
        })
        .count();
    if implementing == 0 {
        return None;
    }
    let target = Some(if implementing == 1 {
        "1 implementing candidate".to_string()
    } else {
        format!("{implementing} implementing candidates")
    });
    if !readback_fan_out_has_budget() {
        degradations.push(IssueMonitorScanDegradation {
            stage: IssueMonitorScanStage::OpenPrReadback,
            target,
            continuation: IssueMonitorScanContinuation::StaleReadback,
            detail: "scan budget reserved for the launch stage".to_string(),
        });
        return None;
    }
    match run_budgeted_readback_stage(IssueMonitorScanStage::OpenPrReadback, || {
        gwt_git::pr_status::try_fetch_open_pr_numbers_by_branch(repo_path)
    }) {
        Ok(index) => Some(index),
        Err(failure) => {
            tracing::warn!(
                stage = %failure.stage,
                error = %failure.detail,
                implementing,
                "issue monitor open-PR inventory degraded; keeping the previous phases"
            );
            degradations.push(IssueMonitorScanDegradation::new(
                failure,
                target,
                IssueMonitorScanContinuation::StaleReadback,
            ));
            None
        }
    }
}

/// Advance one in-flight autonomous issue by a single step. Every remote
/// readback here runs under its own budget ([`run_budgeted_readback_stage`]), so
/// a slow candidate cannot spend the budget the next candidate needs. The
/// open-PR lookup itself is served from `open_prs`, the scan's one inventory.
#[allow(clippy::too_many_arguments)]
fn advance_one_autonomous_issue(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    base_branch: &str,
    open_prs: Option<&std::collections::HashMap<String, u64>>,
    daemon_secret: &[u8],
    issue_number: u64,
    now: &str,
) -> Result<(), IssueMonitorScanFailure> {
    let Some(record) = monitor.autonomous_record(issue_number).cloned() else {
        return Ok(());
    };
    match record.phase {
        crate::AutonomousPhase::Implementing => {
            let Some(branch) = monitor
                .inbox_item(issue_number)
                .and_then(|item| item.launch_plan.as_ref())
                .map(|plan| plan.branch_name.clone())
            else {
                return Ok(());
            };
            // Issue #3963 AC-2/AC-3: resolved from the scan's one inventory. No
            // inventory (the readback degraded) and no row (no open PR) both
            // leave the phase where the last successful readback put it.
            if let Some(pr) = open_prs.and_then(|index| index.get(&branch).copied()) {
                if let Some(sha) =
                    run_budgeted_readback_stage(IssueMonitorScanStage::HeadShaReadback, || {
                        gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
                    })?
                {
                    let criteria = issues
                        .iter()
                        .find(|issue| issue.number == issue_number)
                        .and_then(|issue| issue.body.clone())
                        .map(|body| {
                            crate::issue_monitor_gate::classify_acceptance_criteria(&body).ids
                        })
                        .unwrap_or_default();
                    // Issue #3933 (review follow-up): every readback the dispatch
                    // needs runs BEFORE the record is moved out of Implementing.
                    // A degraded readback now leaves the phase alone; committing
                    // `begin_review` first would strand the candidate in
                    // Reviewing with no dispatch and therefore no verdict to
                    // wait for, until the stuck sweep eventually reset it.
                    let diff =
                        run_budgeted_readback_stage(IssueMonitorScanStage::PrDiffReadback, || {
                            gwt_git::pr_status::try_fetch_pr_diff(repo_path, pr, 200_000)
                        })?
                        .unwrap_or_default();
                    monitor.begin_review(issue_number, pr, &sha);
                    let linked_issue_kind = issues
                        .iter()
                        .find(|issue| issue.number == issue_number)
                        .map(crate::issue_monitor::issue_monitor_linked_issue_kind)
                        .unwrap_or_default();
                    monitor.push_review_dispatch(crate::AutonomousReviewDispatch {
                        issue_number,
                        pr_number: pr,
                        reviewed_sha: sha,
                        required_criteria: criteria,
                        diff,
                        linked_issue_kind,
                    });
                }
            }
        }
        crate::AutonomousPhase::Reviewing => {
            let Some(pr) = record.pr_number else {
                return Ok(());
            };
            let protection =
                run_budgeted_readback_stage(IssueMonitorScanStage::BranchProtection, || {
                    gwt_git::branch_protection::try_fetch_branch_protection(repo_slug, base_branch)
                })?;
            let rollup =
                run_budgeted_readback_stage(IssueMonitorScanStage::StatusCheckReadback, || {
                    gwt_git::pr_status::try_fetch_pr_status_check_rollup(repo_path, pr)
                })?;
            let head = run_budgeted_readback_stage(IssueMonitorScanStage::HeadShaReadback, || {
                gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
            })?
            .unwrap_or_default();
            let body = issues
                .iter()
                .find(|issue| issue.number == issue_number)
                .and_then(|issue| issue.body.clone())
                .unwrap_or_default();
            let Some(inputs) =
                monitor.autonomous_gate_inputs(issue_number, protection, &rollup, &head, &body)
            else {
                return Ok(()); // verdict not back yet → wait
            };
            match crate::issue_monitor_gate::route_autonomous_gate(&inputs) {
                crate::issue_monitor_gate::GateAction::Deliver => {
                    // Audit: a daemon-signed authorization record bound to the
                    // reviewed SHA (control-plane proof the gate authorized it).
                    let token = crate::issue_monitor_authz::sign_merge_authorization(
                        daemon_secret,
                        issue_number,
                        &inputs.reviewed_sha,
                        base_branch,
                    );
                    tracing::info!(
                        issue = issue_number,
                        pr,
                        reviewed_sha = %inputs.reviewed_sha,
                        token = %token,
                        "autonomous gate PASS — preparing auto-merge authority"
                    );
                    // The scan is a side-effect-free proposal builder. It
                    // must never submit `gh pr merge --auto` from a cloned
                    // monitor because the daemon may reject this result as
                    // stale after a concurrent OFF/control transition. The
                    // driver persists Prepared, fences Attempting in a
                    // second transaction, then the serialized executor owns
                    // the remote mutation and result reconciliation.
                    let epoch = monitor.effect_authority_epoch();
                    monitor.prepare_effect(crate::PendingIssueMonitorEffect {
                        effect_id: format!(
                            "arm:{issue_number}:{pr}:{}:{epoch}",
                            inputs.reviewed_sha
                        ),
                        authority_epoch: epoch,
                        attempt: 0,
                        state: crate::IssueMonitorEffectState::Prepared,
                        payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                            issue_number,
                            pr_number: pr,
                            reviewed_sha: inputs.reviewed_sha,
                        },
                    });
                }
                crate::issue_monitor_gate::GateAction::WaitForCi => {}
                crate::issue_monitor_gate::GateAction::Remediate(reason) => {
                    monitor.record_autonomous_failure(issue_number, reason, now);
                }
                // Issue #3944 AC-1: an environment hold is not a human
                // decision — keep the PR in flight, ask the PM to steer,
                // and re-evaluate on the next scan.
                crate::issue_monitor_gate::GateAction::Hold(reason) => {
                    monitor.request_autonomous_steering(issue_number, reason, now);
                }
            }
        }
        crate::AutonomousPhase::Delivering => {
            let Some(pr) = record.pr_number else {
                return Ok(());
            };
            // Merge completion is detected by the presence of a merge commit.
            // The layer-4 identity check then compares the reviewed SHA to the
            // PR's HEAD SHA (`headRefOid`) — NOT the merge commit oid: a squash
            // / merge-commit produces a NEW oid, while `headRefOid` is the head
            // tip that was actually merged (== reviewed SHA when HEAD did not
            // advance). Live-verified against real GitHub (SPEC #3200 layer-4).
            if run_budgeted_readback_stage(IssueMonitorScanStage::MergeCommitReadback, || {
                gwt_git::pr_status::try_fetch_pr_merge_commit_sha(repo_path, pr)
            })?
            .is_some()
            {
                let reviewed = record.reviewed_sha.clone().unwrap_or_default();
                let merged_head =
                    run_budgeted_readback_stage(IssueMonitorScanStage::HeadShaReadback, || {
                        gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
                    })?
                    .unwrap_or_default();
                if crate::issue_monitor_authz::merged_sha_matches_reviewed(&reviewed, &merged_head)
                {
                    monitor.record_merged(issue_number);
                } else {
                    tracing::error!(
                        issue = issue_number,
                        reviewed_sha = %reviewed,
                        merged_head = %merged_head,
                        "SECURITY: merged head SHA != reviewed SHA — escalating"
                    );
                    // Issue #3944 AC-5: the merge already happened, so the
                    // only remaining decision is the human's: approve the
                    // unreviewed merge or revert it.
                    monitor.escalate_to_needs_human(
                        issue_number,
                        crate::NeedsHumanKind::DestructiveChangeApproval,
                        format!(
                            "approve or revert the merge of PR #{pr}: merged head SHA {merged_head} does not match the reviewed SHA {reviewed}"
                        ),
                    );
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn load_cached_issue_monitor_candidates(
    cache_root: &Path,
) -> Result<Vec<IssueMonitorIssue>, String> {
    if !cache_root.is_dir() {
        return Ok(Vec::new());
    }
    let cache = Cache::new(cache_root.to_path_buf());
    let mut issues = cache
        .list_entries()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|entry| {
            let is_spec = entry
                .snapshot
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case("gwt-spec"));
            let (readiness, body) = if is_spec {
                (
                    spec_cache_entry_readiness(&entry),
                    acceptance_source_text(&entry.snapshot.body, &entry),
                )
            } else {
                (
                    IssueMonitorReadiness::NotApplicable,
                    entry.snapshot.body.clone(),
                )
            };
            IssueMonitorIssue {
                readiness,
                number: entry.snapshot.number.0,
                title: entry.snapshot.title,
                labels: entry.snapshot.labels,
                state: match entry.snapshot.state {
                    IssueState::Open => IssueMonitorIssueState::Open,
                    IssueState::Closed => IssueMonitorIssueState::Closed,
                },
                body: (!body.is_empty()).then_some(body),
                url: None,
                updated_at: Some(entry.snapshot.updated_at.0),
            }
        })
        .collect::<Vec<_>>();
    issues.sort_by_key(|issue| issue.number);
    Ok(issues)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRemoteResolutionError {
    CommandSpawnFailed(String),
    GitCommandFailed {
        status_code: Option<i32>,
        stderr: String,
    },
    OriginNotConfigured(String),
    NonGitHubOrigin(String),
    InvalidGitHubOrigin(String),
}

impl fmt::Display for GitHubRemoteResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandSpawnFailed(error) => {
                write!(f, "git remote get-url origin could not be started: {error}")
            }
            Self::GitCommandFailed {
                status_code,
                stderr,
            } => {
                let status = status_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                write!(
                    f,
                    "git remote get-url origin failed with exit status {status}: {stderr}"
                )
            }
            Self::OriginNotConfigured(detail) => {
                write!(f, "Git origin remote is not configured: {detail}")
            }
            Self::NonGitHubOrigin(remote_url) => {
                write!(f, "Git origin remote is not a GitHub URL: {remote_url}")
            }
            Self::InvalidGitHubOrigin(remote_url) => {
                write!(f, "GitHub origin remote URL is invalid: {remote_url}")
            }
        }
    }
}

impl std::error::Error for GitHubRemoteResolutionError {}

pub fn github_remote_owner_and_repo(
    repo_path: &Path,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    github_remote_owner_and_repo_with_program(repo_path, "git")
}

fn github_remote_owner_and_repo_with_program(
    repo_path: &Path,
    program: impl Into<std::ffi::OsString>,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let output = gwt_core::process_console::spawn_logged_blocking(
        &gwt_core::process_console::global(),
        gwt_core::process_console::ProcessKind::Git,
        program,
        &["remote", "get-url", "origin"],
        gwt_core::process_console::SpawnOptions::new("git remote get-url origin")
            .current_dir(&git_root)
            .forward_output(false),
    )
    .map_err(|error| GitHubRemoteResolutionError::CommandSpawnFailed(error.to_string()))?;
    github_remote_owner_and_repo_from_get_url_output(
        output.success(),
        output.exit_code,
        &output.stdout,
        &output.stderr,
    )
}

pub fn parse_github_remote_url(remote_url: &str) -> Option<(String, String)> {
    let path = remote_url
        .strip_prefix("https://github.com/")
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("git@github.com:"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn github_remote_owner_and_repo_from_get_url_output(
    success: bool,
    status_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let stdout = stdout.trim();
    let stderr = cleaned_process_text(stderr);
    if !success {
        if stderr.to_ascii_lowercase().contains("no such remote") && stderr.contains("origin") {
            return Err(GitHubRemoteResolutionError::OriginNotConfigured(stderr));
        }
        return Err(GitHubRemoteResolutionError::GitCommandFailed {
            status_code,
            stderr,
        });
    }
    if stdout.is_empty() {
        return Err(GitHubRemoteResolutionError::OriginNotConfigured(
            "git remote get-url origin returned an empty URL".to_string(),
        ));
    }
    if let Some(owner_repo) = parse_github_remote_url(stdout) {
        return Ok(owner_repo);
    }
    if has_supported_github_remote_prefix(stdout) {
        return Err(GitHubRemoteResolutionError::InvalidGitHubOrigin(
            stdout.to_string(),
        ));
    }
    Err(GitHubRemoteResolutionError::NonGitHubOrigin(
        stdout.to_string(),
    ))
}

fn cleaned_process_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "no stderr".to_string()
    } else {
        trimmed.to_string()
    }
}

fn has_supported_github_remote_prefix(remote_url: &str) -> bool {
    [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ]
    .iter()
    .any(|prefix| remote_url.starts_with(prefix))
}

#[allow(dead_code)]
fn _assert_inbox_item_is_send_sync(_: IssueMonitorInboxItem) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueMonitorConfig, MonitorInboxState};
    use gwt_github::{
        Cache, CommentId, CommentSnapshot, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState,
        UpdatedAt,
    };
    use std::path::PathBuf;

    fn issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-improve".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: Some("2026-08-15T00:00:00Z".to_string()),
        }
    }

    fn github_issue(number: u64) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: vec![],
        }
    }

    fn live_issue(number: u64, labels: &[&str], updated_at: Option<&str>) -> gwt_git::issue::Issue {
        gwt_git::issue::Issue {
            number,
            title: format!("Issue {number}"),
            state: "OPEN".to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            assignee: None,
            body: None,
            url: format!("https://github.com/example/repo/issues/{number}"),
            updated_at: updated_at.map(str::to_string),
        }
    }

    fn structured_spec(number: u64, updated_at: &str, plan: &str, tasks: &str) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("SPEC {number}"),
            body: format!(
                "<!-- gwt-spec id={number} version=1 -->\n\
                 <!-- sections:\n\
                 spec=body\n\
                 plan=body\n\
                 tasks=body\n\
                 -->\n\n\
                 <!-- artifact:spec BEGIN -->\nSpec body\n<!-- artifact:spec END -->\n\n\
                 <!-- artifact:plan BEGIN -->\n{plan}\n<!-- artifact:plan END -->\n\n\
                 <!-- artifact:tasks BEGIN -->\n{tasks}\n<!-- artifact:tasks END -->"
            ),
            labels: vec!["gwt-spec".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new(updated_at),
            comments: Vec::new(),
        }
    }

    #[test]
    fn live_spec_readiness_reuses_only_matching_cache_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        cache
            .write_snapshot(&structured_spec(
                42,
                "2026-08-05T10:00:00Z",
                "Plan body",
                "- [ ] T-001",
            ))
            .expect("write matching spec cache");
        cache
            .write_snapshot(&structured_spec(
                43,
                "2026-08-05T10:00:00Z",
                "   ",
                "- [ ] T-001",
            ))
            .expect("write empty-plan spec cache");
        cache
            .write_snapshot(&structured_spec(
                44,
                "2026-08-05T10:00:00Z",
                "Plan body",
                "\n\t",
            ))
            .expect("write empty-tasks spec cache");
        let mut refreshes = 0;

        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![
                live_issue(42, &["GWT-SPEC"], Some("2026-08-05T10:00:00Z")),
                live_issue(43, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
                live_issue(44, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
            ],
            dir.path(),
            |_| {
                refreshes += 1;
                Ok(())
            },
        );

        assert!(errors.is_empty());
        assert_eq!(refreshes, 0, "matching generations must not hit GitHub");
        assert_eq!(
            issues[0].readiness,
            crate::IssueMonitorReadiness::ReadyWithOpenTasks
        );
        assert_eq!(issues[1].readiness, crate::IssueMonitorReadiness::NotReady);
        assert_eq!(issues[2].readiness, crate::IssueMonitorReadiness::NotReady);
    }

    #[test]
    fn structured_spec_task_completion_is_explicit_and_fail_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        for (number, tasks) in [
            (41, "- [x] T-001\n- [X] T-002"),
            (42, "- [x] T-001\n- [ ] T-002"),
            (43, "Tasks are described without checkboxes"),
            (44, "- [x] T-001\n- [?] malformed task marker"),
            (
                45,
                "```markdown\n- [x] example only\n```\n1. [x] T-001\n2. [ ] T-002",
            ),
            (46, "```markdown\n~~~\n- [x] example only\n~~~\n```"),
            (47, "    - [x] indented code only"),
            (48, "- [x] T-001\n    - [ ] T-002"),
        ] {
            cache
                .write_snapshot(&structured_spec(number, "t1", "Plan", tasks))
                .expect("write spec");
        }
        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![
                live_issue(41, &["gwt-spec"], Some("t1")),
                live_issue(42, &["gwt-spec"], Some("t1")),
                live_issue(43, &["gwt-spec"], Some("t1")),
                live_issue(44, &["gwt-spec"], Some("t1")),
                live_issue(45, &["gwt-spec"], Some("t1")),
                live_issue(46, &["gwt-spec"], Some("t1")),
                live_issue(47, &["gwt-spec"], Some("t1")),
                live_issue(48, &["gwt-spec"], Some("t1")),
            ],
            dir.path(),
            |_| panic!("matching cache must not refresh"),
        );

        assert!(errors.is_empty());
        assert_eq!(
            issues
                .iter()
                .map(|issue| issue.readiness)
                .collect::<Vec<_>>(),
            vec![
                IssueMonitorReadiness::ReadyWithCompletedTasks,
                IssueMonitorReadiness::ReadyWithOpenTasks,
                IssueMonitorReadiness::Ready,
                IssueMonitorReadiness::ReadyWithOpenTasks,
                IssueMonitorReadiness::ReadyWithOpenTasks,
                IssueMonitorReadiness::Ready,
                IssueMonitorReadiness::ReadyWithOpenTasks,
                IssueMonitorReadiness::ReadyWithOpenTasks,
            ]
        );
    }

    #[test]
    fn linked_pr_completion_only_short_circuits_closed_ordinary_issues() {
        let ordinary = IssueMonitorIssue {
            updated_at: Some("2026-08-10T00:00:00Z".to_string()),
            ..issue(42)
        };
        let merged = |merged_at: Option<&str>| crate::cli::LinkedPrSummary {
            number: 99,
            title: "fix".to_string(),
            state: "MERGED".to_string(),
            url: "https://example.test/pull/99".to_string(),
            will_close_target: true,
            merged_at: merged_at.map(str::to_string),
        };

        assert!(!linked_pr_completion_is_fresh_for_issue(
            &ordinary,
            &[merged(Some("2026-08-11T00:00:00Z"))]
        ));
        assert!(!linked_pr_completion_is_fresh_for_issue(
            &ordinary,
            &[merged(Some("2026-08-09T00:00:00Z"))]
        ));
        assert!(!linked_pr_completion_is_fresh_for_issue(
            &ordinary,
            &[merged(None)]
        ));
        let closed_ordinary = IssueMonitorIssue {
            state: IssueMonitorIssueState::Closed,
            ..ordinary.clone()
        };
        assert!(linked_pr_completion_is_fresh_for_issue(
            &closed_ordinary,
            &[]
        ));

        let complete_spec = IssueMonitorIssue {
            labels: vec!["gwt-spec".to_string()],
            readiness: IssueMonitorReadiness::ReadyWithCompletedTasks,
            ..ordinary.clone()
        };
        let incomplete_spec = IssueMonitorIssue {
            readiness: IssueMonitorReadiness::ReadyWithOpenTasks,
            ..complete_spec.clone()
        };
        assert!(linked_pr_completion_is_fresh_for_issue(
            &complete_spec,
            &[merged(Some("2026-08-09T00:00:00Z"))],
        ));
        assert!(!linked_pr_completion_is_fresh_for_issue(
            &incomplete_spec,
            &[merged(Some("2026-08-11T00:00:00Z"))],
        ));
        let missing_revision_spec = IssueMonitorIssue {
            updated_at: None,
            ..complete_spec
        };
        assert!(!linked_pr_completion_is_fresh_for_issue(
            &missing_revision_spec,
            &[merged(Some("2026-08-11T00:00:00Z"))],
        ));
    }

    #[test]
    fn stale_or_missing_live_spec_cache_is_refreshed_per_issue_and_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        cache
            .write_snapshot(&structured_spec(
                42,
                "2026-08-05T09:00:00Z",
                "stale plan",
                "stale tasks",
            ))
            .expect("write stale spec cache");
        let refreshed_cache = Cache::new(dir.path().to_path_buf());
        let mut refreshed_numbers = Vec::new();

        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![
                live_issue(42, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
                live_issue(43, &["gwt-spec"], None),
                live_issue(44, &["bug"], Some("2026-08-05T10:00:00Z")),
            ],
            dir.path(),
            |number| {
                refreshed_numbers.push(number.0);
                match number.0 {
                    42 => refreshed_cache
                        .write_snapshot(&structured_spec(
                            42,
                            "2026-08-05T10:00:00Z",
                            "fresh plan",
                            "- [ ] T-002",
                        ))
                        .map_err(|error| error.to_string()),
                    43 => Err("targeted refresh unavailable".to_string()),
                    other => panic!("ordinary issue #{other} must not refresh"),
                }
            },
        );

        assert_eq!(refreshed_numbers, vec![42, 43]);
        assert_eq!(
            issues[0].readiness,
            crate::IssueMonitorReadiness::ReadyWithOpenTasks
        );
        assert_eq!(issues[1].readiness, crate::IssueMonitorReadiness::NotReady);
        assert_eq!(
            issues[2].readiness,
            crate::IssueMonitorReadiness::NotApplicable
        );
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("#43"));
        assert!(errors[0].contains("targeted refresh unavailable"));
    }

    #[test]
    fn targeted_refresh_limit_bounds_serial_remote_work_and_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let mut refreshed_numbers = Vec::new();

        let (issues, errors) = issue_monitor_candidates_with_readiness_and_refresh_limit(
            vec![
                live_issue(42, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
                live_issue(43, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
            ],
            dir.path(),
            1,
            |number| {
                refreshed_numbers.push(number.0);
                cache
                    .write_snapshot(&structured_spec(
                        number.0,
                        "2026-08-05T10:00:00Z",
                        "Plan body",
                        "- [ ] T-001",
                    ))
                    .map_err(|error| error.to_string())
            },
        );

        assert_eq!(refreshed_numbers, vec![42]);
        assert_eq!(
            issues[0].readiness,
            IssueMonitorReadiness::ReadyWithOpenTasks
        );
        assert_eq!(issues[1].readiness, IssueMonitorReadiness::NotReady);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("#43"));
        assert!(errors[0].contains("per-scan limit 1 reached"));
    }

    #[test]
    fn targeted_refresh_with_unparseable_spec_remains_not_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());

        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![live_issue(45, &["gwt-spec"], Some("2026-08-05T10:00:00Z"))],
            dir.path(),
            |_| {
                let mut malformed = github_issue(45);
                malformed.labels = vec!["gwt-spec".to_string()];
                malformed.updated_at = UpdatedAt::new("2026-08-05T10:00:00Z");
                malformed.body = "<!-- gwt-spec id=45 version=1 -->\n<!-- sections: plan=comment:999 tasks=body -->".to_string();
                cache
                    .write_snapshot(&malformed)
                    .map_err(|error| error.to_string())
            },
        );

        assert_eq!(issues[0].readiness, crate::IssueMonitorReadiness::NotReady);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("#45"));
        assert!(errors[0].contains("parse"));
    }

    #[test]
    fn targeted_refresh_requires_a_known_matching_live_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![
                live_issue(46, &["gwt-spec"], Some("2026-08-05T10:00:00Z")),
                live_issue(47, &["gwt-spec"], None),
            ],
            dir.path(),
            |number| {
                let updated_at = if number.0 == 46 {
                    "2026-08-05T09:00:00Z"
                } else {
                    "2026-08-05T10:00:00Z"
                };
                cache
                    .write_snapshot(&structured_spec(
                        number.0,
                        updated_at,
                        "Plan body",
                        "- [ ] T-001",
                    ))
                    .map_err(|error| error.to_string())
            },
        );

        assert_eq!(issues[0].readiness, IssueMonitorReadiness::NotReady);
        assert_eq!(issues[1].readiness, IssueMonitorReadiness::NotReady);
        assert_eq!(errors.len(), 2);
        assert!(errors
            .iter()
            .all(|error| error.contains("generation mismatch")));
    }

    #[test]
    fn targeted_refresh_composes_comment_resident_plan_before_readiness() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let (issues, errors) = issue_monitor_candidates_with_readiness(
            vec![live_issue(48, &["gwt-spec"], Some("2026-08-05T10:00:00Z"))],
            dir.path(),
            |_| {
                cache
                    .write_snapshot(&IssueSnapshot {
                        number: IssueNumber(48),
                        title: "Comment-resident plan".to_string(),
                        body: "<!-- gwt-spec id=48 version=1 -->\n\
                               <!-- sections:\n\
                               spec=body\n\
                               plan=comment:700\n\
                               tasks=body\n\
                               -->\n\n\
                               <!-- artifact:spec BEGIN -->\nSpec\n<!-- artifact:spec END -->\n\n\
                               <!-- artifact:tasks BEGIN -->\n- [ ] T-001\n<!-- artifact:tasks END -->"
                            .to_string(),
                        labels: vec!["gwt-spec".to_string()],
                        state: IssueState::Open,
                        updated_at: UpdatedAt::new("2026-08-05T10:00:00Z"),
                        comments: vec![CommentSnapshot {
                            id: CommentId(700),
                            body: "<!-- artifact:plan BEGIN -->\nPlan body\n<!-- artifact:plan END -->"
                                .to_string(),
                            updated_at: UpdatedAt::new("2026-08-05T10:00:00Z"),
                        }],
                    })
                    .map_err(|error| error.to_string())
            },
        );

        assert!(errors.is_empty());
        assert_eq!(
            issues[0].readiness,
            IssueMonitorReadiness::ReadyWithOpenTasks
        );
    }

    #[test]
    fn needs_human_is_visible_in_read_only_projection_without_gui() {
        // SPEC-3431 T-040 (FR-011): the PM agent consumes NeedsHuman through
        // the always-emitted status/inbox projection and must never depend on
        // toasts, which are drained only while a GUI is connected. Escalation
        // therefore has to be visible with gui_connected=false, and no toast
        // may be the only carrier.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.inbox.push(crate::IssueMonitorInboxItem {
            issue: issue(42),
            state: MonitorInboxState::Launched,
            claim_id: None,
            blocked_by_owner: None,
            claim_expires_at: None,
            launched_window_id: Some("window-1".to_string()),
            launch_plan: None,
            error_message: None,
            exclusion_reason: None,
        });
        monitor.record_attempt(42);
        monitor.escalate_to_needs_human(
            42,
            crate::NeedsHumanKind::UserChoiceRequired,
            "review rejected",
        );

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        let status = payloads
            .iter()
            .find(|payload| payload.event == "status")
            .expect("status projection is emitted without a GUI");
        let summary = status
            .payload
            .get("autonomous_issues")
            .and_then(|value| value.as_array())
            .and_then(|summaries| {
                summaries.iter().find(|summary| {
                    summary.get("issue_number").and_then(|v| v.as_u64()) == Some(42)
                })
            })
            .expect("escalated issue appears in autonomous_issues");
        assert_eq!(
            summary.get("needs_human").and_then(|v| v.as_bool()),
            Some(true)
        );

        let inbox = payloads
            .iter()
            .find(|payload| payload.event == "inbox")
            .expect("inbox projection is emitted without a GUI");
        let item = inbox
            .payload
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("issue")
                        .and_then(|issue| issue.get("number"))
                        .and_then(|v| v.as_u64())
                        == Some(42)
                })
            })
            .expect("escalated issue appears in the inbox projection");
        assert_eq!(
            item.get("state").and_then(|v| v.as_str()),
            Some("needs_human")
        );
        assert!(item
            .get("error_message")
            .and_then(|v| v.as_str())
            .is_some_and(|message| message.contains("review rejected")));

        assert!(
            payloads.iter().all(|payload| payload.event != "toast"),
            "toasts must not be emitted while no GUI is connected"
        );
    }

    #[test]
    fn payloads_surface_autonomous_notices_as_toasts_when_gui_is_connected() {
        // SPEC #3200 FR-034 (T-111): daemon-side autonomous transitions queue
        // operator notices; the worker drains them into `toast` payloads so the
        // GUI's issue_monitor_toast pipe (surface toast + persistent autonomous
        // notification stack) receives them.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        monitor.record_attempt(42);
        monitor.escalate_to_needs_human(
            42,
            crate::NeedsHumanKind::UserChoiceRequired,
            "review rejected",
        );

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);

        let toast = payloads
            .iter()
            .find(|payload| {
                payload.event == "toast"
                    && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)
            })
            .expect("autonomous notice surfaces as a toast payload");
        assert_eq!(
            toast.payload.get("level").and_then(|v| v.as_str()),
            Some("error")
        );
        assert!(toast
            .payload
            .get("message")
            .and_then(|v| v.as_str())
            .is_some_and(|message| message.contains("review rejected")));
        // Drained: a second pass emits no duplicate.
        let again = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(!again.iter().any(|payload| {
            payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)
        }));
    }

    #[test]
    fn payloads_retain_autonomous_notices_while_no_gui_is_connected() {
        // Fully-unattended window: with no GUI connected the notices stay queued
        // (bounded) instead of being dropped, and surface on the next connect.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.record_attempt(42);
        monitor.escalate_to_needs_human(42, crate::NeedsHumanKind::UserChoiceRequired, "boom");

        let offline = issue_monitor_daemon_payloads(&mut monitor, false);
        assert!(
            !offline.iter().any(|payload| payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)),
            "no toast emitted while the GUI is disconnected"
        );

        let online = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(
            online.iter().any(|payload| payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)),
            "queued notice surfaces once a GUI connects"
        );
    }

    #[test]
    fn payloads_surface_stalled_scan_through_status_error() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            poll_interval_secs: 10,
            ..IssueMonitorConfig::default()
        });
        crate::scan_issue_monitor_candidates(&mut monitor, &[], "2000-01-01T00:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        let status = payloads
            .iter()
            .find(|payload| payload.event == "status")
            .expect("status payload");
        assert_eq!(status.payload["state"], "error");
        assert_eq!(
            status.payload["last_error"],
            "Issue Monitor scan stalled; last scan at 2000-01-01T00:00:00Z"
        );
        assert_eq!(status.payload["last_scan_at"], "2000-01-01T00:00:00Z");
    }

    #[test]
    fn claim_skips_closed_issue_but_keeps_open_issue_launchable_despite_merged_pr_evidence() {
        // Issue #3225 negative control, replaced by Issue #3832: GitHub Closed
        // is terminal, while an ordinary Open issue remains launchable even
        // when the linked-PR probe reports closing merged evidence.
        let mut monitor = IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            max_active: 1,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        let closed = IssueMonitorIssue {
            state: IssueMonitorIssueState::Closed,
            ..issue(42)
        };
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[closed, issue(43)],
            "2026-07-02T00:00:00Z",
        );
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));

        let launches = monitor.claim_next_launch_requests_with_probe(
            &client,
            "host:1",
            "2026-07-02T00:00:10Z",
            1,
            |_| true,
        );

        assert_eq!(
            launches.iter().map(|l| l.issue_number).collect::<Vec<_>>(),
            vec![43],
            "Closed is terminal, but Open merged evidence must not consume the slot"
        );
        assert!(monitor.inbox_item(42).is_none());
        assert!(monitor.prefs().closure_records.iter().any(|record| {
            record.issue_number == 42
                && record.state == crate::issue_monitor::IssueClosureState::Closed
        }));
        assert!(!monitor.prefs().merged_issues.contains(&43));
    }

    #[test]
    fn payloads_keep_queue_when_no_gui_is_connected() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_claimed(issue(42), "claim-a");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        assert!(payloads.iter().any(|payload| payload.event == "status"));
        assert!(payloads.iter().any(|payload| payload.event == "inbox"));
        assert!(!payloads
            .iter()
            .any(|payload| payload.event == "launch_request"));
        assert_eq!(monitor.queue_len(), 1);
        assert_eq!(monitor.active_issue_number(), None);
    }

    #[test]
    fn read_only_payloads_do_not_drain_recovery_blocked_deliveries() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        let read_only = issue_monitor_read_only_daemon_payloads(&monitor);

        assert_eq!(
            read_only
                .iter()
                .map(|payload| payload.event.as_str())
                .collect::<Vec<_>>(),
            vec!["status", "inbox"],
        );
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);

        let after_recovery = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(after_recovery.iter().any(|payload| {
            payload.event == "launch_request"
                && payload.payload["delivery_id"] == "launch:effect-42"
        }));
    }

    #[test]
    fn payloads_emit_launch_request_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);

        assert!(payloads.iter().any(|payload| {
            payload.event == "launch_request" && payload.payload["issue_number"] == 42
        }));
        assert_eq!(monitor.active_issue_number(), Some(42));
        assert_eq!(
            monitor.inbox_item(42).expect("inbox item").state,
            MonitorInboxState::Launching
        );
    }

    /// Issue #3964 AC-1: the shared scan transition — the one path both the
    /// daemon scan and the GUI fallback scan run — asks the owner ledger
    /// whether a generation-conflict hold still protects anything, so a
    /// reaped generation returns its Issue to the queue on the next scan
    /// without an operator.
    #[test]
    fn scan_transition_releases_generation_conflict_holds_from_the_owner_ledger() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", home.path());
        let _session_env = [
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_ID_ENV),
            gwt_core::test_support::ScopedEnvVar::unset(gwt_agent::GWT_SESSION_RUNTIME_PATH_ENV),
        ];
        let worktree = tempfile::tempdir().unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree.path());
        let owner = crate::cli::execution_state::ExecutionOwnerKey {
            kind: crate::cli::execution_state::ExecutionOwnerKind::Issue,
            number: 42,
        };
        let session_id = "scan-transition-reaped-holder";
        crate::cli::execution_state::materialize_at_launch(
            worktree.path(),
            owner.kind,
            owner.number,
            session_id,
            "gwt-execute",
            false,
        )
        .unwrap();
        crate::cli::execution_state::ensure_generation_ledger(
            worktree.path(),
            owner,
            crate::cli::execution_state::LegacyActiveDisposition::Live,
        )
        .unwrap();
        let binding =
            crate::cli::execution_state::current_execution_binding(worktree.path(), owner)
                .unwrap()
                .unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let mut session =
            gwt_agent::Session::new(worktree.path(), "work/issue-42", gwt_agent::AgentId::Codex);
        session.id = session_id.to_string();
        session.linked_issue_number = Some(owner.number);
        session.execution_binding = Some(gwt_agent::SessionExecutionBinding {
            schema_version: gwt_agent::SessionExecutionBinding::CURRENT_SCHEMA_VERSION,
            session_id: session.id.clone(),
            repo_hash: session.repo_hash.clone().unwrap(),
            owner_kind: owner.kind.as_str().to_string(),
            owner_number: owner.number,
            identity: binding,
            capability_generation: 1,
        });
        session.update_status(gwt_agent::AgentStatus::Interrupted);
        session.save(&sessions_dir).unwrap();

        let conflict = format!(
            "{} issue #42 (active generation held by Session {session_id} (Interrupted))",
            crate::cli::execution_state::EXECUTION_GENERATION_CONFLICT_PREFIX
        );
        let loaded = LoadedIssueMonitorCandidates {
            issues: vec![issue(42)],
            source: IssueMonitorCandidateSource::Live,
            live_error: None,
        };
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        scan_loaded_issue_monitor_candidates(
            &mut monitor,
            &loaded,
            worktree.path(),
            "2026-09-05T00:00:00Z",
        );
        monitor.record_agent_issue_failed(42, conflict);

        // Still Active: the scan leaves the hold in place and reports it.
        scan_loaded_issue_monitor_candidates(
            &mut monitor,
            &loaded,
            worktree.path(),
            "2026-09-05T00:01:00Z",
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::AgentFailed)
        );
        let reported = monitor
            .agent_status_at("2026-09-05T00:01:30Z")
            .generation_reclaim
            .expect("a held generation is reported");
        assert_eq!(reported.stranded, vec![42]);
        assert_eq!(
            reported.stranded_by_holder_state,
            std::collections::BTreeMap::from([("Interrupted".to_string(), 1)])
        );

        // The reaper releases the generation; the next scan releases the row.
        let identity = gwt_agent::SessionExecutionIdentity::from_session(&session)
            .unwrap()
            .unwrap();
        let candidate =
            crate::cli::execution_state::inspect_startup_active_generation_ledgers(&[worktree
                .path()
                .to_path_buf()])
            .candidates
            .into_iter()
            .find(|candidate| candidate.owner == owner)
            .expect("active candidate");
        assert_eq!(
            crate::cli::execution_state::reap_startup_defunct_active_generation(
                &candidate,
                &sessions_dir,
                &identity,
                &[],
            )
            .unwrap(),
            crate::cli::execution_state::StartupActiveGenerationReapOutcome::Reaped
        );

        scan_loaded_issue_monitor_candidates(
            &mut monitor,
            &loaded,
            worktree.path(),
            "2026-09-05T00:02:00Z",
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "a released generation returns its Issue to the queue"
        );
        let released = monitor
            .agent_status_at("2026-09-05T00:02:30Z")
            .generation_reclaim
            .expect("the release is reported");
        assert_eq!(released.released, vec![42]);
        assert!(released.stranded.is_empty());
    }

    #[test]
    fn synchronous_claim_path_honors_autonomous_retry_backoff() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.set_autonomous_mode(true);
        monitor.record_candidate(issue(42));
        monitor.complete_active_launch(42, "tab-1::agent-42");
        assert_eq!(
            monitor.record_autonomous_failure(42, "retry later", "2026-08-26T00:00:00Z",),
            crate::issue_monitor::AutonomousFailureOutcome::Retry { attempt: 1 }
        );

        assert!(
            monitor
                .claim_next_launch_requests(&client, "host-a/session-a", "2026-08-26T00:00:30Z",)
                .is_empty(),
            "the legacy synchronous path must not bypass retry_not_before"
        );
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.attempt_count(42), 1);

        assert_eq!(
            monitor
                .claim_next_launch_requests(&client, "host-a/session-a", "2026-08-26T00:01:00Z",)
                .iter()
                .map(|request| request.issue_number)
                .collect::<Vec<_>>(),
            vec![42]
        );
    }

    #[test]
    fn durable_launch_delivery_survives_fanout_zero_and_replays_one_stable_identity() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        let offline = issue_monitor_daemon_payloads(&mut monitor, false);
        assert!(!offline
            .iter()
            .any(|payload| payload.event == "launch_request"));
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);

        let first = issue_monitor_daemon_payloads(&mut monitor, true);
        let replay = issue_monitor_daemon_payloads(&mut monitor, true);
        for payloads in [&first, &replay] {
            let launch = payloads
                .iter()
                .find(|payload| payload.event == "launch_request")
                .expect("durable launch request");
            assert_eq!(launch.payload["delivery_id"], "launch:effect-42");
        }
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);
    }

    #[test]
    fn payloads_emit_launch_request_before_launching_snapshot_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_index = payloads
            .iter()
            .position(|payload| payload.event == "launch_request")
            .expect("launch request payload");
        let first_status_index = payloads
            .iter()
            .position(|payload| payload.event == "status")
            .expect("status payload");

        assert!(
            launch_index < first_status_index,
            "the agent window launch request must reach the GUI before the monitor renders Launching"
        );
    }

    #[test]
    fn payloads_emit_all_pending_launch_requests_when_parallel_capacity_allows() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));
        client.seed(github_issue(44));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 3,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        monitor.record_candidate(issue(44));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_numbers: Vec<u64> = payloads
            .iter()
            .filter(|payload| payload.event == "launch_request")
            .filter_map(|payload| payload.payload["issue_number"].as_u64())
            .collect();

        assert_eq!(launch_numbers, vec![42, 43, 44]);
        assert_eq!(monitor.active_count(), 3);
    }

    /// A gwt-spec snapshot whose `spec` section is either inlined in the body
    /// or routed to one comment — the two shapes the storage layer produces.
    fn spec_with_storage(number: u64, spec: &str, in_comment: bool) -> IssueSnapshot {
        let comment_id = 900_000 + number;
        let wrapped = format!("<!-- artifact:spec BEGIN -->\n{spec}\n<!-- artifact:spec END -->");
        let (index, body_spec, comments) = if in_comment {
            (
                format!("spec=comment:{comment_id}"),
                String::new(),
                vec![CommentSnapshot {
                    id: CommentId(comment_id),
                    body: wrapped,
                    updated_at: UpdatedAt::new("t1"),
                }],
            )
        } else {
            (
                "spec=body".to_string(),
                format!("{wrapped}\n\n"),
                Vec::new(),
            )
        };
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("SPEC {number}"),
            body: format!(
                "<!-- gwt-spec id={number} version=1 -->\n\
                 <!-- sections:\n\
                 {index}\n\
                 plan=body\n\
                 tasks=body\n\
                 -->\n\n\
                 {body_spec}\
                 <!-- artifact:plan BEGIN -->\nPlan\n<!-- artifact:plan END -->\n\n\
                 <!-- artifact:tasks BEGIN -->\n- [ ] T-001\n<!-- artifact:tasks END -->"
            ),
            labels: vec!["gwt-spec".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments,
        }
    }

    /// Issue #3930 AC-4 / AC-7 (storage dimension): the acceptance-criteria
    /// text a candidate carries includes the `spec` section even when the
    /// storage layer routed it to a comment (#3864's shape), on both the cache
    /// loader and the live-readiness path. `AC-N:` present / absent × English
    /// / Japanese heading × body / comment storage all classify as
    /// machine-checkable with the same ids.
    #[test]
    fn spec_candidates_expose_comment_resident_acceptance_criteria() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let mut expected: Vec<(u64, Vec<&str>)> = Vec::new();
        let mut number = 100;
        for heading in ["Acceptance Criteria", "受け入れ基準", "受け入れ条件"] {
            for prefixed in [true, false] {
                for in_comment in [false, true] {
                    number += 1;
                    let items = if prefixed {
                        "- [ ] AC-7: first\n- [ ] AC-8: second"
                    } else {
                        "- [ ] first\n- [ ] second"
                    };
                    let spec = format!("# Spec\n\n## {heading}\n\n{items}");
                    cache
                        .write_snapshot(&spec_with_storage(number, &spec, in_comment))
                        .expect("write spec");
                    expected.push((
                        number,
                        if prefixed {
                            vec!["AC-7", "AC-8"]
                        } else {
                            vec!["AC-1", "AC-2"]
                        },
                    ));
                }
            }
        }
        let classify = |issue: &IssueMonitorIssue| {
            crate::issue_monitor_gate::classify_acceptance_criteria(
                issue.body.as_deref().unwrap_or(""),
            )
        };

        let candidates = load_cached_issue_monitor_candidates(dir.path()).expect("load cache");
        for (number, want) in &expected {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.number == *number)
                .expect("cached candidate");
            let criteria = classify(candidate);
            assert!(
                criteria.machine_checkable,
                "cache #{number}: {:?}",
                candidate.body
            );
            assert_eq!(criteria.ids, *want, "cache #{number}");
        }

        let live = expected
            .iter()
            .map(|(number, _)| live_issue(*number, &["gwt-spec"], Some("t1")))
            .collect();
        let (issues, errors) = issue_monitor_candidates_with_readiness(live, dir.path(), |_| {
            panic!("cache matches live; no refresh expected")
        });
        assert!(errors.is_empty(), "{errors:?}");
        for (number, want) in &expected {
            let issue = issues
                .iter()
                .find(|issue| issue.number == *number)
                .expect("live candidate");
            let criteria = classify(issue);
            assert!(
                criteria.machine_checkable,
                "live #{number}: {:?}",
                issue.body
            );
            assert_eq!(criteria.ids, *want, "live #{number}");
        }
    }

    /// Issue #3959 AC-1: #3864's own shape, as it was stored in production —
    /// `plan` and `spec` both routed to comments, so the Issue body is nothing
    /// but the section header and the `tasks` artifact, while the
    /// `- [ ] AC-N:` block lives in the spec comment. Duplicating that block
    /// into the body was the only thing that un-quarantined it, which is the
    /// proof the classifier never saw the comment.
    #[test]
    fn issue_3864_comment_resident_spec_reaches_the_acceptance_classifier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let spec_comment = 5_502_609_215_u64;
        let plan_comment = 5_494_509_117_u64;
        let acceptance = (1..=14)
            .map(|index| format!("- [ ] AC-{index}: 受け入れ条件 {index}\n"))
            .collect::<String>();
        let snapshot = IssueSnapshot {
            number: IssueNumber(3864),
            title: "SPEC 3864".to_string(),
            body: format!(
                "<!-- gwt-spec id=3864 version=1 -->\n\
                 <!-- sections:\n\
                 plan=comment:{plan_comment}\n\
                 spec=comment:{spec_comment}\n\
                 tasks=body\n\
                 -->\n\n\
                 <!-- artifact:tasks BEGIN -->\n- [x] T-001: 実装\n<!-- artifact:tasks END -->"
            ),
            labels: vec!["gwt-spec".to_string(), "auto-merge".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: vec![
                CommentSnapshot {
                    id: CommentId(plan_comment),
                    body: "<!-- artifact:plan BEGIN -->\n## 実装計画\n\nPhase 1\n\
                           <!-- artifact:plan END -->"
                        .to_string(),
                    updated_at: UpdatedAt::new("t1"),
                },
                CommentSnapshot {
                    id: CommentId(spec_comment),
                    body: format!(
                        "<!-- artifact:spec BEGIN -->\n# Spec\n\n## 受け入れ基準\n\n{acceptance}\
                         <!-- artifact:spec END -->"
                    ),
                    updated_at: UpdatedAt::new("t1"),
                },
            ],
        };
        cache.write_snapshot(&snapshot).expect("write spec");
        let want: Vec<String> = (1..=14).map(|index| format!("AC-{index}")).collect();

        let candidates = load_cached_issue_monitor_candidates(dir.path()).expect("load cache");
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.number == 3864)
            .expect("cached candidate");
        let criteria = crate::issue_monitor_gate::classify_acceptance_criteria(
            candidate.body.as_deref().unwrap_or(""),
        );
        assert!(
            criteria.machine_checkable,
            "the comment-resident block must reach the classifier: {:?}",
            candidate.body
        );
        assert_eq!(criteria.ids, want);
    }

    #[test]
    fn cached_issue_candidates_load_from_issue_cache_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let mut spec = github_issue(3165);
        spec.title = "SPEC: Missing plan and tasks".to_string();
        spec.labels = vec!["gwt-spec".to_string()];
        let ready_spec = structured_spec(3166, "t1", "Plan body", "- [ ] T-001");
        let mut closed = github_issue(3000);
        closed.title = "Closed issue".to_string();
        closed.state = IssueState::Closed;
        cache.write_snapshot(&spec).expect("write spec");
        cache.write_snapshot(&ready_spec).expect("write ready spec");
        cache.write_snapshot(&closed).expect("write closed issue");

        let candidates = load_cached_issue_monitor_candidates(dir.path()).expect("load cache");

        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].number, 3000);
        assert_eq!(candidates[0].state, IssueMonitorIssueState::Closed);
        assert_eq!(candidates[1].number, 3165);
        assert_eq!(candidates[1].title, "SPEC: Missing plan and tasks");
        assert_eq!(candidates[1].labels, vec!["gwt-spec"]);
        assert_eq!(candidates[1].state, IssueMonitorIssueState::Open);
        assert_eq!(
            candidates[1].readiness,
            crate::IssueMonitorReadiness::NotReady
        );
        assert_eq!(candidates[2].number, 3166);
        assert_eq!(
            candidates[2].readiness,
            crate::IssueMonitorReadiness::ReadyWithOpenTasks,
            "unchecked tasks are launch-ready but not completion-ready"
        );
    }

    #[test]
    fn loaded_candidate_provenance_distinguishes_live_success_from_cache_fallback() {
        let live_issue = issue(42);
        let cached_issue = issue(43);

        let live = resolve_loaded_issue_monitor_candidates(
            Ok(vec![live_issue.clone()]),
            [Ok(vec![cached_issue.clone()])],
        )
        .expect("live result");
        assert_eq!(live.source, IssueMonitorCandidateSource::Live);
        assert!(live.authorizes_remote_effects());
        assert_eq!(live.live_error, None);
        assert_eq!(live.issues, vec![live_issue]);

        let live_with_failed_spec_enrichment = LoadedIssueMonitorCandidates {
            issues: vec![cached_issue.clone()],
            source: IssueMonitorCandidateSource::Live,
            live_error: Some("issue #43 targeted refresh failed".to_string()),
        };
        assert!(
            live_with_failed_spec_enrichment.authorizes_remote_effects(),
            "a complete live list remains authoritative; the affected spec fails closed via readiness"
        );

        let empty_live = resolve_loaded_issue_monitor_candidates(
            Ok(Vec::new()),
            [Ok(vec![cached_issue.clone()])],
        )
        .expect("empty live result still authoritative");
        assert_eq!(empty_live.source, IssueMonitorCandidateSource::Live);
        assert!(empty_live.authorizes_remote_effects());
        assert!(empty_live.issues.is_empty());

        let limit_sized_live = resolve_loaded_issue_monitor_candidates(
            Ok((1..=1_000).map(issue).collect()),
            std::iter::empty::<Result<Vec<IssueMonitorIssue>, String>>(),
        )
        .expect("limit-sized live result");
        assert_eq!(
            limit_sized_live.source,
            IssueMonitorCandidateSource::LiveIncomplete,
            "a capped gh list cannot prove that an absent in-flight Issue no longer exists"
        );

        let cache = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Ok(vec![cached_issue.clone()])],
        )
        .expect("cache fallback");
        assert_eq!(cache.source, IssueMonitorCandidateSource::Cache);
        assert!(!cache.authorizes_remote_effects());
        assert_eq!(cache.live_error.as_deref(), Some("gh unavailable"));
        assert_eq!(cache.issues, vec![cached_issue]);

        let error = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Err("cache corrupt".to_string())],
        )
        .expect_err("no usable cache preserves live error");
        assert_eq!(error, "gh unavailable");
    }

    #[test]
    fn cached_candidates_preserve_live_scan_error_after_read_model_refresh() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut monitor = IssueMonitorState::new(crate::IssueMonitorConfig::default());
        let loaded = LoadedIssueMonitorCandidates {
            issues: vec![issue(42)],
            source: IssueMonitorCandidateSource::Cache,
            live_error: Some("operation deadline exceeded at issue-list stage".to_string()),
        };

        let summary = scan_loaded_issue_monitor_candidates(
            &mut monitor,
            &loaded,
            dir.path(),
            "2026-07-27T00:00:00Z",
        );

        assert_eq!(summary.scanned, 1);
        assert_eq!(
            monitor.status_view().last_error.as_deref(),
            Some(
                "issue list failed; using cache fallback: operation deadline exceeded at issue-list stage"
            )
        );
    }

    #[test]
    fn parse_github_remote_url_accepts_https_and_ssh_forms() {
        assert_eq!(
            parse_github_remote_url("https://github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("https://example.com/owner/repo"),
            None
        );
    }

    /// Issue #3933 AC-3: a per-candidate readback spends its own budget, not the
    /// scan's. Under `run_scan_stage` the operation below would be handed the
    /// whole remaining scan window and every later candidate would inherit an
    /// exhausted one — the production fan-out failure this Issue is about.
    #[test]
    fn a_readback_is_cut_at_its_own_budget_not_the_whole_scan_window() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _budget = gwt_core::test_support::ScopedEnvVar::set(
            "GWT_TEST_ISSUE_MONITOR_READBACK_BUDGET_MS",
            "50",
        );
        let scan_window = std::time::Duration::from_secs(30);
        let _scan_deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + scan_window,
        );

        let observed = run_budgeted_readback_stage(IssueMonitorScanStage::OpenPrReadback, || {
            let remaining = gwt_core::operation_deadline::current()
                .expect("the readback runs under a deadline")
                .saturating_duration_since(std::time::Instant::now());
            Ok::<_, String>(remaining)
        })
        .expect("the stage itself succeeds");

        assert!(
            observed <= std::time::Duration::from_millis(50),
            "the call must see its own budget, got {observed:?}"
        );

        // The scan window is intact afterwards, so the next candidate is not
        // paying for this one.
        let remaining = gwt_core::operation_deadline::current()
            .expect("the scan deadline is restored")
            .saturating_duration_since(std::time::Instant::now());
        assert!(
            remaining > std::time::Duration::from_secs(20),
            "the shared scan window must survive one readback, got {remaining:?}"
        );
    }

    /// Issue #3933: the fan-out yields before it eats the launch stage's budget.
    #[test]
    fn the_readback_fan_out_stops_while_the_launch_stage_still_has_budget() {
        assert!(
            readback_fan_out_has_budget(),
            "no ambient deadline imposes no fan-out limit"
        );

        let _almost_spent = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() + ISSUE_MONITOR_LAUNCH_RESERVE
                - std::time::Duration::from_millis(1),
        );

        assert!(
            !readback_fan_out_has_budget(),
            "the reserve belongs to the launch stage, not to another readback"
        );
    }

    #[test]
    fn proposal_return_deadline_expiry_is_stage_typed() {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
        );

        let error = ensure_scan_deadline(IssueMonitorScanStage::ProposalReturn)
            .expect_err("expired final deadline must reject the proposal");

        assert_eq!(error.stage, IssueMonitorScanStage::ProposalReturn);
        assert!(error.to_string().contains("proposal-return"));
    }

    #[test]
    fn github_remote_output_resolves_valid_origin() {
        assert_eq!(
            github_remote_owner_and_repo_from_get_url_output(
                true,
                Some(0),
                "https://github.com/owner/repo.git\n",
                ""
            )
            .expect("valid origin"),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn github_remote_owner_and_repo_accepts_workspace_home_with_child_bare_repo() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo_path = tmp.path().join("gwt.git");
        let status = gwt_core::process::hidden_command("git")
            .args(["init", "--bare"])
            .arg(&bare_repo_path)
            .status()
            .expect("git init --bare");
        assert!(status.success(), "git init --bare failed");
        let status = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ])
            .current_dir(&bare_repo_path)
            .status()
            .expect("git remote add origin");
        assert!(status.success(), "git remote add origin failed");

        assert_eq!(
            github_remote_owner_and_repo(tmp.path()).expect("workspace home origin"),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn github_remote_owner_and_repo_stops_hanging_program_at_operation_deadline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fake_git = temp.path().join("git");
        // Issue #3521: written by a child shell so no fork in a sibling test
        // can inherit a writable descriptor and turn the exec into ETXTBSY.
        gwt_core::test_support::write_executable_script(
            &fake_git,
            r#"#!/bin/sh
if [ "$1" = "remote" ] && [ "$2" = "get-url" ] && [ "$3" = "origin" ]; then
  sleep 2
  printf '%s\n' 'https://github.com/owner/repo.git'
  exit 0
fi
exit 1
"#,
        )
        .expect("write fake git");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo path");
        let started = std::time::Instant::now();
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            started + std::time::Duration::from_millis(150),
        );

        let error = github_remote_owner_and_repo_with_program(&repo, fake_git.as_os_str())
            .expect_err("ambient deadline must stop the hanging git remote lookup");

        assert!(error.to_string().contains("deadline"), "{error}");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1_500),
            "hanging git outlived the deadline: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn github_remote_output_classifies_missing_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(2),
            "",
            "error: No such remote 'origin'\n",
        )
        .expect_err("missing origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not configured: error: No such remote 'origin'"
        );
    }

    #[test]
    fn github_remote_output_classifies_git_failure() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(128),
            "",
            "fatal: not a git repository\n",
        )
        .expect_err("git failure");

        assert_eq!(
            error.to_string(),
            "git remote get-url origin failed with exit status 128: fatal: not a git repository"
        );
    }

    #[test]
    fn github_remote_output_classifies_non_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://example.com/owner/repo.git\n",
            "",
        )
        .expect_err("non GitHub origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not a GitHub URL: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn github_remote_output_classifies_invalid_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://github.com/owner\n",
            "",
        )
        .expect_err("invalid GitHub origin");

        assert_eq!(
            error.to_string(),
            "GitHub origin remote URL is invalid: https://github.com/owner"
        );
    }

    #[test]
    fn apply_autonomous_eligibility_is_noop_when_mode_off() {
        // SPEC #3200 FR-001: default autonomous_mode OFF ⇒ no autonomous state is
        // created and (crucially) no branch-protection network call is made. The
        // early return runs before any gh invocation, so this test exercises the
        // gate without touching the network.
        use crate::{
            IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorState,
        };
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let issues = vec![IssueMonitorIssue {
            number: 50,
            title: "t".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some("## Acceptance Criteria\n- [ ] AC-1: x\n".to_string()),
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        }];
        apply_autonomous_eligibility(
            &mut monitor,
            &issues,
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            "2026-06-29T00:00:00Z",
        );
        assert!(
            monitor.autonomous_record(50).is_none(),
            "off ⇒ no autonomous state created, no network call",
        );
    }

    #[test]
    fn autonomous_eligibility_candidate_filter_skips_non_terminal_exclusions() {
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        let mut not_ready = issue(50);
        not_ready.labels = vec!["gwt-spec".to_string(), "auto-merge".to_string()];
        not_ready.readiness = IssueMonitorReadiness::NotReady;
        let mut held = issue(51);
        held.labels = vec!["auto-merge".to_string(), "hold".to_string()];
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[not_ready.clone(), held.clone()],
            "2026-08-05T10:00:00Z",
        );

        assert!(autonomous_eligibility_candidates(
            &monitor,
            &[not_ready, held],
            "2026-08-05T10:00:01Z",
        )
        .is_empty());
        assert!(monitor.autonomous_record(50).is_none());
        assert!(monitor.autonomous_record(51).is_none());
        assert_eq!(
            monitor.inbox_item(50).map(|item| item.state),
            Some(MonitorInboxState::NotReady)
        );
        assert_eq!(
            monitor.inbox_item(51).map(|item| item.state),
            Some(MonitorInboxState::HoldExcluded)
        );
    }

    #[test]
    fn advance_autonomous_in_flight_is_noop_when_mode_off() {
        // Default OFF ⇒ no phase advancement, no network call, no merge.
        use crate::{
            AutonomousPhase, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
            IssueMonitorState,
        };
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_phase(50, AutonomousPhase::Reviewing); // would otherwise act
        let issues = vec![IssueMonitorIssue {
            number: 50,
            title: "t".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: None,
        }];
        advance_autonomous_in_flight(
            &mut monitor,
            &issues,
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            b"secret",
            "2026-06-29T00:00:00Z",
        );
        assert_eq!(
            monitor.autonomous_record(50).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing),
            "off ⇒ phase unchanged, no network/merge",
        );
        assert!(monitor.take_pending_review_dispatches().is_empty());
    }

    #[test]
    fn advance_autonomous_in_flight_is_noop_when_global_monitor_is_disabled() {
        use crate::{AutonomousPhase, IssueMonitorConfig, IssueMonitorState};
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_mode(true);
        monitor.set_autonomous_phase(50, AutonomousPhase::Reviewing);

        advance_autonomous_in_flight(
            &mut monitor,
            &[],
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            b"secret",
            "2026-06-29T00:00:00Z",
        );

        assert_eq!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::Reviewing)
        );
        assert!(monitor.pending_effects().is_empty());
    }

    #[test]
    fn parse_default_base_branch_strips_origin_prefix_and_fails_closed() {
        assert_eq!(parse_default_base_branch("origin/main\n"), "main");
        assert_eq!(parse_default_base_branch("origin/develop"), "develop");
        // A bare name with no origin/ prefix is taken as-is.
        assert_eq!(parse_default_base_branch("trunk"), "trunk");
        // Empty / unresolved ⇒ fail-closed to main.
        assert_eq!(parse_default_base_branch(""), "main");
        assert_eq!(parse_default_base_branch("origin/"), "main");
    }

    #[test]
    fn resolve_default_base_branch_uses_child_bare_repo_for_workspace_home() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo = tmp.path().join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .current_dir(tmp.path())
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let symbolic_ref = gwt_core::process::hidden_command("git")
            .args([
                "--git-dir",
                bare_repo.to_str().expect("bare repo path"),
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ])
            .current_dir(tmp.path())
            .output()
            .expect("set origin HEAD");
        assert!(
            symbolic_ref.status.success(),
            "git symbolic-ref failed: {}",
            String::from_utf8_lossy(&symbolic_ref.stderr)
        );

        assert_eq!(resolve_default_base_branch(tmp.path()), "develop");
    }

    /// Issue #3348: a repository that never configured `origin/HEAD` is a
    /// normal state. Reporting it as a scan failure aborted the whole
    /// autonomous progress loop before it could reach `gh`.
    #[test]
    fn unset_origin_head_falls_back_instead_of_failing_the_scan() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo = tmp.path().join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .current_dir(tmp.path())
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        assert_eq!(
            try_resolve_default_base_branch(tmp.path())
                .expect("unset origin/HEAD is not a failure"),
            DEFAULT_BASE_BRANCH_FALLBACK,
        );
    }

    /// Issue #3349: the scan deadline is still the one condition that aborts
    /// this stage, so a hung `git symbolic-ref` can never freeze the driver.
    #[test]
    fn expired_scan_deadline_still_fails_default_base_resolution() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
        );

        let failure = try_resolve_default_base_branch(tmp.path())
            .expect_err("an expired deadline must abort the stage");

        assert_eq!(failure.stage, IssueMonitorScanStage::DefaultBaseBranch);
    }

    /// A `gh` stand-in that refuses to answer unless it was spawned from inside
    /// a bare repository (`HEAD` + `objects/` + `refs/` sit directly in the
    /// cwd). A container root holds none of those, so this is exactly the
    /// failure Issue #3348 observed in production.
    fn write_bare_repo_bound_fake_gh(bin_dir: &Path, call_log: &Path) -> PathBuf {
        std::fs::create_dir_all(bin_dir).expect("create fake gh bin dir");
        let call_log = call_log.display().to_string();
        #[cfg(windows)]
        {
            let fake_gh = bin_dir.join("gh.cmd");
            std::fs::write(
                &fake_gh,
                format!(
                    "@echo off\r\n\
if not exist \"HEAD\" goto notbare\r\n\
if not exist \"objects\\\" goto notbare\r\n\
if not exist \"refs\\\" goto notbare\r\n\
echo %*>>\"{call_log}\"\r\n\
echo %* | findstr /C:\"mergeCommit\" >nul\r\n\
if not errorlevel 1 (\r\n\
  echo {{\"mergeCommit\":{{\"oid\":\"squash999\"}}}}\r\n\
  exit /b 0\r\n\
)\r\n\
echo %* | findstr /C:\"headRefOid\" >nul\r\n\
if not errorlevel 1 (\r\n\
  echo {{\"headRefOid\":\"abc123\"}}\r\n\
  exit /b 0\r\n\
)\r\n\
echo {{}}\r\n\
exit /b 0\r\n\
:notbare\r\n\
>&2 echo gh ran outside the child bare repository: %CD%\r\n\
exit /b 1\r\n"
                ),
            )
            .expect("write fake gh");
            fake_gh
        }
        #[cfg(not(windows))]
        {
            let fake_gh = bin_dir.join("gh");
            std::fs::write(
                &fake_gh,
                format!(
                    r#"#!/bin/sh
if [ ! -f HEAD ] || [ ! -d objects ] || [ ! -d refs ]; then
  printf '%s\n' "gh ran outside the child bare repository: $(pwd)" >&2
  exit 1
fi
printf '%s\n' "$*" >> "{call_log}"
case "$*" in
  *mergeCommit*) printf '%s\n' '{{"mergeCommit":{{"oid":"squash999"}}}}' ;;
  *headRefOid*)  printf '%s\n' '{{"headRefOid":"abc123"}}' ;;
  *)             printf '%s\n' '{{}}' ;;
esac
exit 0
"#
                ),
            )
            .expect("write fake gh");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_gh, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake gh");
            fake_gh
        }
    }

    /// Issue #3348 regression: in a container-layout project the monitored
    /// project root is NOT a git repository — it merely holds the bare
    /// `<repo>.git` plus the worktrees. Every autonomous-loop `gh` call used to
    /// be spawned with that raw root as cwd, so gh could not resolve the base
    /// repo and each fetch failed silently: the loop never advanced, the merge
    /// was never observed, and the active slot stayed held forever. The fake
    /// `gh` here fails unless it runs inside the child bare repo, so reverting
    /// the `main_worktree_root` normalization turns this test red.
    #[test]
    fn advance_autonomous_in_flight_reaches_gh_from_child_bare_repo_for_container_root() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let container_root = tmp.path().join("workspace");
        std::fs::create_dir_all(&container_root).expect("create container root");
        let bare_repo = container_root.join("repo.git");
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

        let call_log = tmp.path().join("gh-calls.log");
        let fake_gh = write_bare_repo_bound_fake_gh(&tmp.path().join("bin"), &call_log);
        let mut path_entries = vec![fake_gh.parent().expect("fake gh parent").to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            path_entries.extend(std::env::split_paths(&existing));
        }
        let _path = gwt_core::test_support::ScopedEnvVar::set(
            "PATH",
            std::env::join_paths(path_entries).expect("join PATH"),
        );
        // Issue #3675: mark the fake as installed so the unsandboxed-gh spawn
        // guard lets ProcessKind::Gh spawns through inside this scope.
        let _sandbox = gwt_core::test_support::ScopedEnvVar::set("GWT_TEST_GH_SANDBOX", "1");

        let issues = vec![IssueMonitorIssue {
            number: 42,
            title: "Issue 42".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some("## Acceptance Criteria\n- [ ] AC-1: returns 200\n".to_string()),
            url: None,
            readiness: IssueMonitorReadiness::NotApplicable,
            updated_at: Some("2026-08-15T00:00:00Z".to_string()),
        }];
        // `enabled` is required as well as `autonomous_mode`: the global kill
        // switch gates every autonomous remote call
        // (`advance_autonomous_in_flight_is_noop_when_global_monitor_is_disabled`),
        // and this test is about the gh command root, not the kill switch.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        crate::scan_issue_monitor_candidates(&mut monitor, &issues, "2026-07-28T00:00:00Z");
        // Delivering is the narrowest phase that proves the loop reaches gh: it
        // watches the merge (`--json mergeCommit`) and then re-reads the merged
        // head (`--json headRefOid`) for the layer-4 identity check.
        monitor.begin_review(42, 7, "abc123");
        monitor.record_review_verdict(42, true);
        monitor.begin_delivering(42);

        advance_autonomous_in_flight(
            &mut monitor,
            &issues,
            "owner/repo",
            &container_root,
            b"secret",
            "2026-07-28T00:10:00Z",
        );

        let calls = std::fs::read_to_string(&call_log).unwrap_or_default();
        assert!(
            calls.contains("mergeCommit") && calls.contains("headRefOid"),
            "the loop must reach gh from the child bare repo (calls={calls:?})",
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Queued),
            "a merged PR observed through the normalized gh path frees the slot without terminalizing an Open Issue",
        );
        assert_eq!(monitor.active_count(), 0);
        assert_eq!(monitor.queued_issue_numbers(), vec![42]);
    }
}

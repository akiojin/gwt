#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorPriorityPosition {
    Head,
    Index(usize),
}

/// SPEC-1942 command model for `issue.*` and `issue.spec.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueCommand {
    SpecReadAll {
        number: u64,
    },
    SpecReadSection {
        number: u64,
        section: String,
    },
    SpecEditSection {
        number: u64,
        section: String,
        file: String,
    },
    SpecEditSectionBody {
        number: u64,
        section: String,
        body: String,
    },
    SpecEditSectionJson {
        number: u64,
        section: String,
        file: Option<String>,
        replace: bool,
    },
    SpecEditSectionJsonBody {
        number: u64,
        section: String,
        body: String,
        replace: bool,
    },
    SpecList {
        phase: Option<String>,
        state: Option<String>,
    },
    SpecCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    SpecCreateBody {
        title: String,
        body: String,
        labels: Vec<String>,
    },
    SpecCreateJson {
        title: String,
        file: Option<String>,
        labels: Vec<String>,
    },
    SpecCreateJsonBody {
        title: String,
        body: String,
        labels: Vec<String>,
    },
    SpecCreateHelp,
    SpecPull {
        all: bool,
        numbers: Vec<u64>,
    },
    SpecRepair {
        number: u64,
    },
    SpecRename {
        number: u64,
        title: String,
    },
    View {
        number: u64,
        refresh: bool,
    },
    Comments {
        number: u64,
        refresh: bool,
    },
    LinkedPrs {
        number: u64,
        refresh: bool,
    },
    Create {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    CreateBody {
        title: String,
        body: String,
        labels: Vec<String>,
    },
    /// Issue #3865: update a plain Issue in place. Every field is optional and
    /// only the supplied ones are sent; `body` is a whole-body replacement and
    /// is refused for `gwt-spec` Issues, whose body is section-managed.
    Edit {
        number: u64,
        title: Option<String>,
        body: Option<String>,
        labels: Option<Vec<String>>,
    },
    Comment {
        number: u64,
        file: String,
    },
    CommentBody {
        number: u64,
        body: String,
    },
    /// SPEC #3200 Option A: an independent-review agent reports its verdict for a
    /// reviewed SHA. Published to the Issue Monitor daemon control channel, where
    /// the daemon (trusted) re-judges it against the launch-time criteria.
    MonitorReviewVerdict {
        issue_number: u64,
        reviewed_sha: String,
        verdict_raw: String,
    },
    MonitorStatus {
        project_root: Option<std::path::PathBuf>,
    },
    MonitorPriorityMove {
        project_root: Option<std::path::PathBuf>,
        number: u64,
        position: IssueMonitorPriorityPosition,
    },
    MonitorPrioritySet {
        project_root: Option<std::path::PathBuf>,
        issue_numbers: Vec<u64>,
    },
    MonitorConfigSet {
        project_root: Option<std::path::PathBuf>,
        enabled: Option<bool>,
        autonomous_mode: Option<bool>,
        max_active: Option<usize>,
        /// Issue #3917 AC-5: explicit auto-close override (`None` leaves the
        /// stored value untouched).
        auto_close_merged_issues: Option<bool>,
        /// Issue #3923 AC-5: switch the saved launch profile's agent.
        launch_agent: Option<String>,
        /// Issue #4037 AC-5: raise (`true`, recorded as a manual drain) or
        /// clear (`false`) the non-destructive update drain.
        update_drain: Option<bool>,
    },
    /// SPEC #3914 FR-011: read the launch candidate pool, provider holds and
    /// the usage threshold.
    MonitorProfiles {
        project_root: Option<std::path::PathBuf>,
    },
    /// SPEC #3914 FR-011: replace the launch candidate pool whole (idempotent)
    /// and optionally the usage threshold.
    MonitorProfilesSet {
        project_root: Option<std::path::PathBuf>,
        profiles: Vec<crate::IssueMonitorLaunchProfile>,
        usage_threshold_percent: Option<u8>,
    },
    /// SPEC-3431 FR-006: the PM's launch instruction — move the issue to the
    /// priority head and ask for one immediate scan. Never launches directly.
    MonitorLaunchNow {
        project_root: Option<std::path::PathBuf>,
        number: u64,
    },
    /// SPEC-3431 FR-033: the PM's stop instruction — revoke one launch's
    /// authority and slot without requeueing or relaunching it.
    ///
    /// The identity components are optional on the wire and required against
    /// the live state: a materializing launch is identified by its delivery, a
    /// running one by its window. Omitting a component the monitor holds is a
    /// mismatch, not a wildcard.
    MonitorStop {
        project_root: Option<std::path::PathBuf>,
        number: u64,
        reason: String,
        claim_id: Option<String>,
        delivery_id: Option<String>,
        window_id: Option<String>,
    },
    /// SPEC-3431 FR-029〜031: revoke one launch and requeue its issue at the
    /// head so the currently saved launch profile picks it up.
    ///
    /// Same identity contract as [`Self::MonitorStop`]; the difference is the
    /// outcome, not the gate.
    MonitorFailover {
        project_root: Option<std::path::PathBuf>,
        number: u64,
        reason: String,
        claim_id: Option<String>,
        delivery_id: Option<String>,
        window_id: Option<String>,
    },
    /// Issue #3645 / #3628: release the failure holding one issue out of the
    /// queue, for rows that have no live launch left to identify.
    ///
    /// Carries no identity components, and that is the point: `agent_failed`
    /// rows lost their launch, so every operation that resolves one refuses
    /// them. The state layer still fails closed on anything a launch owns.
    MonitorRequeue {
        project_root: Option<std::path::PathBuf>,
        number: u64,
        reason: String,
    },
    /// Issue #3923 AC-1: every provider-wide quota hold in force, with the
    /// evidence it was formed from.
    MonitorQuotaHoldList {
        project_root: Option<std::path::PathBuf>,
    },
    /// Issue #3883 AC-6: put the still-running agent windows back under slot
    /// accounting. Additive only — no pane is closed and no slot is taken
    /// away — so it is safe to run against a project mid-flight.
    MonitorReconcile {
        project_root: Option<std::path::PathBuf>,
    },
    /// Issue #3923 AC-1: release one provider's quota hold on the operator's
    /// authority. The release is a durable fence, so a process that still
    /// holds the hold in memory cannot re-stamp it.
    MonitorQuotaHoldClear {
        project_root: Option<std::path::PathBuf>,
        provider: String,
        reason: String,
    },
    /// Issue #3844: a launched agent declares (or clears) that it is waiting,
    /// so the Issue Monitor does not mistake the silence for a stall. `number`
    /// defaults to the launch context's owner Issue.
    MonitorWait {
        project_root: Option<std::path::PathBuf>,
        number: Option<u64>,
        reason: Option<String>,
        resume_condition: Option<String>,
        clear: bool,
    },
    /// Issue #3478 (AC-9): list the questions autonomous executions are parked
    /// on, so a human can see what is blocking the queue.
    MonitorQuestions {
        project_root: Option<std::path::PathBuf>,
    },
    /// Issue #3478 (AC-5): register a human answer for one parked question.
    MonitorQuestionAnswer {
        project_root: Option<std::path::PathBuf>,
        handoff_id: String,
        answer: String,
    },
}

/// SPEC-1942 command model for `pr.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrCommand {
    Current,
    /// `pr.list` with the optional PM thresholds from Issue #3868 (AC-5 /
    /// AC-6); `None` keeps the crate defaults.
    List {
        stale_after_hours: Option<i64>,
        escalate_after_cycles: Option<u32>,
        /// Issue #3891: bypass the TTL cache and the budget throttle.
        refresh: bool,
        /// Issue #3891 AC-2: heavy fields to hydrate; `None` keeps the crate
        /// default (checks, no body).
        include: Option<gwt_git::PrInventoryInclude>,
    },
    Create {
        base: String,
        head: Option<String>,
        title: String,
        file: String,
        labels: Vec<String>,
        draft: bool,
    },
    CreateBody {
        base: String,
        head: Option<String>,
        title: String,
        body: String,
        labels: Vec<String>,
        draft: bool,
    },
    Edit {
        number: u64,
        title: Option<String>,
        file: Option<String>,
        add_labels: Vec<String>,
    },
    EditBody {
        number: u64,
        title: Option<String>,
        body: Option<String>,
        add_labels: Vec<String>,
    },
    View {
        number: u64,
    },
    Ready {
        number: u64,
    },
    Draft {
        number: u64,
    },
    Comment {
        number: u64,
        file: String,
    },
    CommentBody {
        number: u64,
        body: String,
    },
    Reviews {
        number: u64,
    },
    ReviewThreads {
        number: u64,
    },
    ReviewThreadsReplyAndResolve {
        number: u64,
        file: String,
    },
    ReviewThreadsReplyAndResolveBody {
        number: u64,
        body: String,
    },
    Checks {
        number: u64,
    },
}

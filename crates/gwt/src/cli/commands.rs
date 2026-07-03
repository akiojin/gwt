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
}

/// SPEC-1942 command model for `pr.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrCommand {
    Current,
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

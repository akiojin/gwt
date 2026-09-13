//! Shared `pr.*` response and call shapes.
//!
//! SPEC-1942 SC-025 keeps `cli.rs` a thin family-split dispatcher, so these
//! plain data types live with the `pr` family and are re-exported from
//! `crate::cli` for the existing call sites.

/// Compact linked PR summary used by `issue.linked_prs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedPrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    #[serde(default)] // closes-the-issue flag; gates the completion probe (#3226)
    pub will_close_target: bool,
    /// GitHub merge instant used to prove that an ordinary Issue has not
    /// advanced since the closing work was delivered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<String>,
}

/// Compact PR check entry used by `pr.checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrCheckItem {
    pub name: String,
    pub state: String,
    pub conclusion: String,
    pub url: String,
    pub started_at: String,
    pub completed_at: String,
    pub workflow: String,
}

/// Render-friendly aggregate used by `pr.checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrChecksSummary {
    pub summary: String,
    pub ci_status: String,
    pub merge_status: String,
    pub review_status: String,
    pub checks: Vec<PrCheckItem>,
}

/// What GitHub did when `pr.update_branch` asked it to merge the base branch
/// into the PR head (SPEC #3835 AC-15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrUpdateBranchOutcome {
    /// The head now carries the base branch's commits, so the PR leaves
    /// `BEHIND`.
    Updated,
    /// Merging the base into the head would conflict, so GitHub refused and
    /// nothing was pushed. Resolving it is the owner's work, never the PM's
    /// automatic action (FR-007).
    Conflicted,
}

impl PrUpdateBranchOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "UPDATED",
            Self::Conflicted => "CONFLICTED",
        }
    }
}

/// Result of one `pr.update_branch` call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrUpdateBranchResult {
    pub number: u64,
    pub outcome: PrUpdateBranchOutcome,
    /// GitHub's own wording, kept verbatim so a refusal stays diagnosable.
    pub detail: String,
}

/// PR review summary used by `pr.reviews`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReview {
    pub id: String,
    pub state: String,
    pub body: String,
    pub submitted_at: String,
    pub author: String,
}

/// Single comment inside a review thread.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThreadComment {
    pub id: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub author: String,
}

/// Review thread snapshot used by `pr.review_threads`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<PrReviewThreadComment>,
}

/// Test-visible log entry for `pr.create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreateCall {
    pub base: String,
    pub head: Option<String>,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// Test-visible log entry for `pr.edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrEditCall {
    pub number: u64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub add_labels: Vec<String>,
}

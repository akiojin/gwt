//! Abstract GitHub Issue client trait and supporting types.
//!
//! The [`IssueClient`] trait encapsulates every network operation gwt-github
//! needs for SPEC-12 hybrid storage:
//!
//! - conditional `fetch` of an Issue + comments by `updatedAt` (returns
//!   `NotModified` when the server copy has not changed),
//! - `patch_body` / `patch_comment` for section-level writes,
//! - `create_comment` / `create_issue` for new SPEC creation and section
//!   promotion,
//! - `set_labels` for phase transitions,
//! - `list_spec_issues` for the Specs tab listing, implemented as a single
//!   GraphQL query in the HTTPS backend.
//!
//! This module defines the trait, its input/output types, and an in-memory
//! [`FakeIssueClient`] used by contract tests. A real HTTPS backend backed
//! by `reqwest` is implemented in [`crate::client::http`] (coming in the next
//! TDD cycle).

pub mod fake;
pub mod http;

use std::fmt;

/// Newtype for an Issue number as returned by the GitHub REST API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IssueNumber(pub u64);

impl fmt::Display for IssueNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Newtype for a comment id (GitHub `databaseId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommentId(pub u64);

impl fmt::Display for CommentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "comment:{}", self.0)
    }
}

/// Monotonic timestamp returned by the API. We store it as a string to avoid
/// pulling in a date/time dependency at this layer — higher levels treat it as
/// an opaque cache key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UpdatedAt(pub String);

impl UpdatedAt {
    /// Convenience constructor for tests.
    pub fn new<S: Into<String>>(s: S) -> Self {
        UpdatedAt(s.into())
    }
}

/// State of a GitHub Issue (open vs. closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}

/// Snapshot of a single Issue including its body and every artifact comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSnapshot {
    pub number: IssueNumber,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub state: IssueState,
    pub updated_at: UpdatedAt,
    pub comments: Vec<CommentSnapshot>,
}

/// Snapshot of a single Issue comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentSnapshot {
    pub id: CommentId,
    pub body: String,
    pub updated_at: UpdatedAt,
}

/// Result of a conditional [`IssueClient::fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchResult {
    /// The server copy has not changed since the requested `UpdatedAt`.
    NotModified,
    /// The server copy has changed — here is the fresh snapshot.
    Updated(IssueSnapshot),
}

/// Filter supplied to [`IssueClient::list_spec_issues`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpecListFilter {
    /// If `Some`, only include Issues carrying `phase/<name>` as a label.
    pub phase: Option<String>,
    /// If `Some`, only include Issues in the given state.
    pub state: Option<IssueState>,
}

/// Compact summary of a SPEC-labeled Issue used by the Specs tab listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecSummary {
    pub number: IssueNumber,
    pub title: String,
    pub state: IssueState,
    pub labels: Vec<String>,
    pub updated_at: UpdatedAt,
}

/// Errors returned by [`IssueClient`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("issue #{0} not found")]
    NotFound(IssueNumber),
    #[error("comment {0} not found")]
    CommentNotFound(CommentId),
    #[error("body exceeds GitHub 65536 byte limit")]
    BodyTooLarge,
    #[error("rate limited (retry_after = {retry_after:?})")]
    RateLimited { retry_after: Option<u64> },
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("unexpected server response: {0}")]
    Unexpected(String),
}

/// The abstract GitHub Issue client. All mutating operations return the
/// post-write server snapshot so callers can atomically update their local
/// cache.
pub trait IssueClient: Send + Sync {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError>;

    fn patch_body(&self, number: IssueNumber, new_body: &str) -> Result<IssueSnapshot, ApiError>;

    fn patch_title(&self, number: IssueNumber, new_title: &str) -> Result<IssueSnapshot, ApiError>;

    fn patch_comment(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError>;

    fn create_comment(&self, number: IssueNumber, body: &str) -> Result<CommentSnapshot, ApiError>;

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError>;

    fn set_labels(&self, number: IssueNumber, labels: &[String])
        -> Result<IssueSnapshot, ApiError>;

    fn set_state(&self, number: IssueNumber, state: IssueState) -> Result<IssueSnapshot, ApiError>;

    fn list_spec_issues(&self, filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError>;
}

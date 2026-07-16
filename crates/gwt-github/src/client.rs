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
//! [`fake::FakeIssueClient`] used by contract tests. A real HTTPS backend
//! backed by `reqwest` is implemented in [`crate::client::http`] (coming in
//! the next TDD cycle).

pub mod fake;
pub mod http;

use std::{
    fmt,
    time::{Duration, Instant},
};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("issue #{0} not found")]
    NotFound(IssueNumber),
    #[error("comment {0} not found")]
    CommentNotFound(CommentId),
    #[error("body exceeds GitHub 65536 byte limit")]
    BodyTooLarge,
    #[error("rate limited (retry_after = {retry_after:?})")]
    RateLimited { retry_after: Option<u64> },
    /// SPEC-3214 FR-011: non-rate-limit 403. Preserves the GitHub-provided
    /// reason (e.g. "Issues are disabled for this repo") so callers can show
    /// the specific cause instead of a generic failure.
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("operation timed out: {operation}")]
    Timeout { operation: String },
    #[error("failed to parse {operation}: {message}")]
    Parse { operation: String, message: String },
    #[error("partial page while reading {operation} after {completed_pages} complete page(s)")]
    PartialPage {
        operation: String,
        completed_pages: usize,
    },
    #[error("test transport override rejected: {reason}")]
    TestOverrideRejected { reason: String },
    #[error("owner repository mismatch: expected {expected}, got {actual}")]
    RepositoryMismatch { expected: String, actual: String },
    #[error("authentication required")]
    Unauthorized,
    #[error("unexpected server response: {0}")]
    Unexpected(String),
}

/// Failure classification for a remote owner mutation.
///
/// Callers must not retry [`Self::RemoteOutcomeUnknown`] until an
/// authoritative marker refresh proves whether the submitted request took
/// effect.
#[derive(Debug, thiserror::Error)]
pub enum OwnerMutationError {
    #[error("owner mutation was not submitted: {0}")]
    PreSubmit(#[source] ApiError),
    #[error("owner mutation outcome is unknown after submission: {0}")]
    RemoteOutcomeUnknown(#[source] ApiError),
}

pub type OwnerMutationResult<T> = Result<T, OwnerMutationError>;

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

/// Explicit GitHub repository identity used by owner-resolution operations.
///
/// Owner resolution never infers its mutation destination from the source
/// checkout. Every operation receives one of these values instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepositoryIdentity {
    owner: String,
    name: String,
}

impl RepositoryIdentity {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn gwt_upstream() -> Self {
        Self::new("akiojin", "gwt")
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for RepositoryIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// One absolute deadline shared by auth acquisition and all requests/pages in
/// an owner-resolution attempt.
#[derive(Debug, Clone, Copy)]
pub struct ResolutionDeadline {
    expires_at: Instant,
    connect_timeout_cap: Duration,
}

impl ResolutionDeadline {
    pub fn new(connect_timeout_cap: Duration, total_timeout: Duration) -> Self {
        Self::at(
            Instant::now()
                .checked_add(total_timeout)
                .unwrap_or_else(Instant::now),
            connect_timeout_cap,
        )
    }

    pub fn at(expires_at: Instant, connect_timeout_cap: Duration) -> Self {
        Self {
            expires_at,
            connect_timeout_cap,
        }
    }

    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }

    pub fn reserving(&self, reserve: Duration) -> Self {
        Self::at(
            self.expires_at
                .checked_sub(reserve)
                .unwrap_or_else(Instant::now),
            self.connect_timeout_cap,
        )
    }

    pub fn remaining(&self, operation: &str) -> Result<Duration, ApiError> {
        self.expires_at
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| ApiError::Timeout {
                operation: operation.to_string(),
            })
    }

    pub fn connect_timeout(&self, operation: &str) -> Result<Duration, ApiError> {
        let remaining = self.remaining(operation)?;
        let timeout = remaining.min(self.connect_timeout_cap);
        if timeout.is_zero() {
            return Err(ApiError::Timeout {
                operation: operation.to_string(),
            });
        }
        Ok(timeout)
    }
}

/// Opaque token describing the completed upstream generation used for a
/// collection read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionGeneration(String);

impl CollectionGeneration {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A collection that is returned only after all remote pages completed.
///
/// Implementors must return [`ApiError::PartialPage`] instead of constructing
/// this value when any page or cursor cannot be fetched or parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteCollection<T> {
    items: Vec<T>,
    generation: CollectionGeneration,
}

impl<T> CompleteCollection<T> {
    pub fn from_complete(items: Vec<T>, generation: CollectionGeneration) -> Self {
        Self { items, generation }
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn generation(&self) -> &CollectionGeneration {
        &self.generation
    }

    pub fn into_items(self) -> Vec<T> {
        self.items
    }

    pub fn into_parts(self) -> (Vec<T>, CollectionGeneration) {
        (self.items, self.generation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepositoryIssueKind {
    Plain,
    Spec,
}

/// Fully materialized plain Issue or SPEC from the authoritative owner corpus.
/// Pull requests are intentionally not representable in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIssue {
    pub repository: RepositoryIdentity,
    pub number: IssueNumber,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub state: IssueState,
    pub kind: RepositoryIssueKind,
    pub updated_at: UpdatedAt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryComment {
    pub id: CommentId,
    pub body: String,
    pub updated_at: UpdatedAt,
    pub author_login: Option<String>,
    pub author_type: Option<RepositoryActorType>,
    pub author_association: Option<RepositoryAuthorAssociation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RepositoryActorType {
    User,
    Bot,
    Organization,
    Mannequin,
    EnterpriseUserAccount,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RepositoryAuthorAssociation {
    Owner,
    Member,
    Collaborator,
    Contributor,
    FirstTimer,
    FirstTimeContributor,
    Mannequin,
    None,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryIssue {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPullRequest {
    pub number: IssueNumber,
    pub merge_commit_sha: String,
    pub merged_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRelease {
    pub tag_name: String,
    pub target_commitish: String,
    pub published_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitComparisonStatus {
    Ahead,
    Behind,
    Identical,
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitComparison {
    pub base: String,
    pub head: String,
    pub base_commit_sha: String,
    pub merge_base_commit_sha: String,
    pub head_commit_sha: String,
    pub status: CommitComparisonStatus,
    pub ahead_by: u64,
    pub behind_by: u64,
}

impl CommitComparison {
    pub(crate) fn validate_response(&self, operation: &str) -> Result<(), ApiError> {
        for (field, oid) in [
            ("base_commit.sha", self.base_commit_sha.as_str()),
            ("merge_base_commit.sha", self.merge_base_commit_sha.as_str()),
            ("head commit sha", self.head_commit_sha.as_str()),
        ] {
            if !is_full_commit_oid(oid) {
                return Err(ApiError::Parse {
                    operation: operation.to_string(),
                    message: format!("{field} is not a full commit OID"),
                });
            }
        }
        if is_full_commit_oid(&self.base) && self.base_commit_sha != self.base {
            return Err(ApiError::Parse {
                operation: operation.to_string(),
                message: "resolved base commit does not match requested commit".to_string(),
            });
        }
        if is_full_commit_oid(&self.head) && self.head_commit_sha != self.head {
            return Err(ApiError::Parse {
                operation: operation.to_string(),
                message: "resolved head commit does not match requested commit".to_string(),
            });
        }
        let valid_forward_shape = match self.status {
            CommitComparisonStatus::Ahead => {
                self.ahead_by > 0
                    && self.behind_by == 0
                    && self.merge_base_commit_sha == self.base_commit_sha
                    && self.head_commit_sha != self.base_commit_sha
            }
            CommitComparisonStatus::Identical => {
                self.ahead_by == 0
                    && self.behind_by == 0
                    && self.merge_base_commit_sha == self.base_commit_sha
                    && self.head_commit_sha == self.base_commit_sha
            }
            CommitComparisonStatus::Behind | CommitComparisonStatus::Diverged => true,
        };
        if !valid_forward_shape {
            return Err(ApiError::Parse {
                operation: operation.to_string(),
                message: "commit comparison ancestry shape is inconsistent".to_string(),
            });
        }
        Ok(())
    }
}

fn is_full_commit_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Deadline-aware, explicitly repository-targeted operations used by Durable
/// Owner Resolution. This is separate from [`IssueClient`] so SPEC cache
/// callers retain their existing behavior and pagination contract.
pub trait OwnerRepositoryClient: Send + Sync {
    fn list_issues(
        &self,
        repository: &RepositoryIdentity,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryIssue>, ApiError>;

    fn list_comments(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryComment>, ApiError>;

    fn fetch_issue(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<RepositoryIssue, ApiError>;

    fn create_owner_comment(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        body: &str,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryComment>;

    fn create_owner_issue(
        &self,
        repository: &RepositoryIdentity,
        input: &CreateRepositoryIssue,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue>;

    /// Close an Issue and return a readback-verified closed snapshot.
    fn close_issue_verified(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue>;

    fn fetch_merged_pull_request(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<MergedPullRequest>, ApiError>;

    fn fetch_release_by_tag(
        &self,
        repository: &RepositoryIdentity,
        tag: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<RepositoryRelease>, ApiError>;

    fn compare_commits(
        &self,
        repository: &RepositoryIdentity,
        base: &str,
        head: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<CommitComparison, ApiError>;
}

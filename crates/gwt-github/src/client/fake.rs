//! In-memory implementation of [`IssueClient`] used by contract tests.
//!
//! The [`FakeIssueClient`] stores Issues and comments in a `Mutex`-guarded
//! `HashMap` and records a per-issue call counter so tests can assert call
//! patterns. It is intentionally minimal — just enough to exercise the
//! [`IssueClient`] contract in a predictable, deterministic way.

use std::{
    cmp::Reverse,
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use crate::client::{
    ApiError, CollectionGeneration, CommentId, CommentSnapshot, CommitComparison,
    CompleteCollection, CreateRepositoryIssue, FetchResult, IssueClient, IssueNumber,
    IssueSnapshot, IssueState, MergedPullRequest, OwnerMutationError, OwnerMutationResult,
    OwnerRepositoryClient, RepositoryComment, RepositoryIdentity, RepositoryIssue,
    RepositoryIssueKind, RepositoryRelease, ResolutionDeadline, SpecListFilter, SpecSummary,
    UpdatedAt,
};

/// In-memory fake [`IssueClient`].
#[derive(Clone)]
pub struct FakeIssueClient {
    inner: Arc<Mutex<FakeState>>,
    next_issue_number: Arc<AtomicU64>,
    next_comment_id: Arc<AtomicU64>,
    next_owner_comment_id: Arc<AtomicU64>,
    clock: Arc<AtomicU64>,
}

struct FakeState {
    issues: HashMap<IssueNumber, IssueSnapshot>,
    /// Recorded call log for tests (operation + target).
    pub call_log: Vec<String>,
    owner_issues: HashMap<RepositoryIdentity, HashMap<IssueNumber, RepositoryIssue>>,
    owner_comments: HashMap<(RepositoryIdentity, IssueNumber), Vec<RepositoryComment>>,
    merged_pull_requests: HashMap<(RepositoryIdentity, IssueNumber), MergedPullRequest>,
    releases: HashMap<(RepositoryIdentity, String), RepositoryRelease>,
    commit_comparisons: HashMap<(RepositoryIdentity, String, String), CommitComparison>,
    owner_faults: HashMap<OwnerRepositoryOperation, VecDeque<OwnerRepositoryFault>>,
    owner_call_log: Vec<OwnerRepositoryCall>,
    owner_mutation_call_log: Vec<OwnerRepositoryCall>,
    owner_next_issue_number: HashMap<RepositoryIdentity, u64>,
    owner_generations: HashMap<RepositoryIdentity, u64>,
    owner_issue_generation_queue: HashMap<RepositoryIdentity, VecDeque<CollectionGeneration>>,
    owner_issue_view_queue:
        HashMap<RepositoryIdentity, VecDeque<(Vec<RepositoryIssue>, CollectionGeneration)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnerRepositoryOperation {
    ListIssues,
    ListComments,
    FetchIssue,
    CreateComment,
    CreateIssue,
    CloseIssue,
    FetchMergedPullRequest,
    FetchRelease,
    CompareCommits,
}

impl OwnerRepositoryOperation {
    fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::CreateComment | Self::CreateIssue | Self::CloseIssue
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerRepositoryFaultTiming {
    BeforeSubmit,
    AfterSubmit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRepositoryCall {
    pub operation: OwnerRepositoryOperation,
    pub repository: RepositoryIdentity,
    pub issue_number: Option<IssueNumber>,
}

struct OwnerRepositoryFault {
    timing: OwnerRepositoryFaultTiming,
    error: ApiError,
}

impl FakeIssueClient {
    pub fn new() -> Self {
        FakeIssueClient {
            inner: Arc::new(Mutex::new(FakeState {
                issues: HashMap::new(),
                call_log: Vec::new(),
                owner_issues: HashMap::new(),
                owner_comments: HashMap::new(),
                merged_pull_requests: HashMap::new(),
                releases: HashMap::new(),
                commit_comparisons: HashMap::new(),
                owner_faults: HashMap::new(),
                owner_call_log: Vec::new(),
                owner_mutation_call_log: Vec::new(),
                owner_next_issue_number: HashMap::new(),
                owner_generations: HashMap::new(),
                owner_issue_generation_queue: HashMap::new(),
                owner_issue_view_queue: HashMap::new(),
            })),
            next_issue_number: Arc::new(AtomicU64::new(1)),
            next_comment_id: Arc::new(AtomicU64::new(1)),
            next_owner_comment_id: Arc::new(AtomicU64::new(1)),
            clock: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Preload an Issue snapshot. Used by tests to set up fixtures.
    pub fn seed(&self, snapshot: IssueSnapshot) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Ensure next_issue_number stays ahead of seeded numbers.
        let current_next = self.next_issue_number.load(Ordering::SeqCst);
        if snapshot.number.0 >= current_next {
            self.next_issue_number
                .store(snapshot.number.0 + 1, Ordering::SeqCst);
        }
        for c in &snapshot.comments {
            let cid = c.id.0;
            let current_cid = self.next_comment_id.load(Ordering::SeqCst);
            if cid >= current_cid {
                self.next_comment_id.store(cid + 1, Ordering::SeqCst);
            }
        }
        state.issues.insert(snapshot.number, snapshot);
    }

    /// Snapshot of the recorded call log.
    pub fn call_log(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .call_log
            .clone()
    }

    /// Snapshot comments for one Issue. Used by higher-level tests that need
    /// to assert whether a workflow wrote GitHub comments.
    pub fn comments(&self, number: IssueNumber) -> Vec<CommentSnapshot> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .issues
            .get(&number)
            .map(|issue| issue.comments.clone())
            .unwrap_or_default()
    }

    pub fn seed_repository_issue(&self, issue: RepositoryIssue) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let repository = issue.repository.clone();
        let next_number = state
            .owner_next_issue_number
            .entry(repository.clone())
            .or_insert(1);
        *next_number = (*next_number).max(issue.number.0 + 1);
        state
            .owner_issues
            .entry(repository.clone())
            .or_default()
            .insert(issue.number, issue);
        Self::increment_owner_generation(&mut state, &repository);
    }

    pub fn seed_repository_comments(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        comments: Vec<RepositoryComment>,
    ) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for comment in &comments {
            let current_next = self.next_owner_comment_id.load(Ordering::SeqCst);
            if comment.id.0 >= current_next {
                self.next_owner_comment_id
                    .store(comment.id.0 + 1, Ordering::SeqCst);
            }
        }
        state
            .owner_comments
            .insert((repository.clone(), number), comments);
        Self::increment_owner_generation(&mut state, repository);
    }

    pub fn queue_owner_issue_generations<I, S>(
        &self,
        repository: &RepositoryIdentity,
        generations: I,
    ) where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_issue_generation_queue
            .entry(repository.clone())
            .or_default()
            .extend(
                generations
                    .into_iter()
                    .map(|generation| CollectionGeneration::new(generation.into())),
            );
    }

    pub fn queue_owner_issue_views<I, S>(&self, repository: &RepositoryIdentity, views: I)
    where
        I: IntoIterator<Item = (Vec<RepositoryIssue>, S)>,
        S: Into<String>,
    {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_issue_view_queue
            .entry(repository.clone())
            .or_default()
            .extend(views.into_iter().map(|(issues, generation)| {
                (issues, CollectionGeneration::new(generation.into()))
            }));
    }

    pub fn seed_merged_pull_request(
        &self,
        repository: &RepositoryIdentity,
        pull_request: MergedPullRequest,
    ) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .merged_pull_requests
            .insert((repository.clone(), pull_request.number), pull_request);
    }

    pub fn seed_repository_release(
        &self,
        repository: &RepositoryIdentity,
        release: RepositoryRelease,
    ) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .releases
            .insert((repository.clone(), release.tag_name.clone()), release);
    }

    pub fn seed_commit_comparison(
        &self,
        repository: &RepositoryIdentity,
        comparison: CommitComparison,
    ) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .commit_comparisons
            .insert(
                (
                    repository.clone(),
                    comparison.base.clone(),
                    comparison.head.clone(),
                ),
                comparison,
            );
    }

    pub fn fail_next_owner_operation(
        &self,
        operation: OwnerRepositoryOperation,
        timing: OwnerRepositoryFaultTiming,
        error: ApiError,
    ) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_faults
            .entry(operation)
            .or_default()
            .push_back(OwnerRepositoryFault { timing, error });
    }

    pub fn owner_call_log(&self) -> Vec<OwnerRepositoryCall> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_call_log
            .clone()
    }

    pub fn owner_mutation_call_log(&self) -> Vec<OwnerRepositoryCall> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_mutation_call_log
            .clone()
    }

    pub fn owner_mutation_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owner_mutation_call_log
            .len()
    }

    fn record(&self, state: &mut FakeState, op: &str, target: &str) {
        state.call_log.push(format!("{op}:{target}"));
    }

    fn tick(&self) -> UpdatedAt {
        let next = self.clock.fetch_add(1, Ordering::SeqCst) + 1;
        UpdatedAt(format!("t{next}"))
    }

    fn begin_owner_operation(
        state: &mut FakeState,
        operation: OwnerRepositoryOperation,
        repository: &RepositoryIdentity,
        issue_number: Option<IssueNumber>,
    ) -> Result<Option<ApiError>, ApiError> {
        state.owner_call_log.push(OwnerRepositoryCall {
            operation,
            repository: repository.clone(),
            issue_number,
        });
        let Some(fault) = state
            .owner_faults
            .get_mut(&operation)
            .and_then(VecDeque::pop_front)
        else {
            return Ok(None);
        };
        match fault.timing {
            OwnerRepositoryFaultTiming::BeforeSubmit => Err(fault.error),
            OwnerRepositoryFaultTiming::AfterSubmit => Ok(Some(fault.error)),
        }
    }

    fn record_owner_mutation(
        state: &mut FakeState,
        operation: OwnerRepositoryOperation,
        repository: &RepositoryIdentity,
        issue_number: Option<IssueNumber>,
    ) {
        debug_assert!(operation.is_mutation());
        state.owner_mutation_call_log.push(OwnerRepositoryCall {
            operation,
            repository: repository.clone(),
            issue_number,
        });
        Self::increment_owner_generation(state, repository);
    }

    fn owner_generation(
        state: &FakeState,
        repository: &RepositoryIdentity,
    ) -> CollectionGeneration {
        let generation = state
            .owner_generations
            .get(repository)
            .copied()
            .unwrap_or(1);
        CollectionGeneration::new(format!("fake:{repository}:{generation}"))
    }

    fn increment_owner_generation(state: &mut FakeState, repository: &RepositoryIdentity) {
        *state
            .owner_generations
            .entry(repository.clone())
            .or_insert(1) += 1;
    }
}

impl Default for FakeIssueClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueClient for FakeIssueClient {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "fetch", &number.to_string());
        let issue = state
            .issues
            .get(&number)
            .ok_or(ApiError::NotFound(number))?
            .clone();
        if let Some(prev) = since {
            if *prev == issue.updated_at {
                return Ok(FetchResult::NotModified);
            }
        }
        Ok(FetchResult::Updated(issue))
    }

    fn patch_body(&self, number: IssueNumber, new_body: &str) -> Result<IssueSnapshot, ApiError> {
        if new_body.len() > 65_536 {
            return Err(ApiError::BodyTooLarge);
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "patch_body", &number.to_string());
        let issue = state
            .issues
            .get_mut(&number)
            .ok_or(ApiError::NotFound(number))?;
        issue.body = new_body.to_string();
        issue.updated_at = self.tick();
        Ok(issue.clone())
    }

    fn patch_title(&self, number: IssueNumber, new_title: &str) -> Result<IssueSnapshot, ApiError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "patch_title", &number.to_string());
        let issue = state
            .issues
            .get_mut(&number)
            .ok_or(ApiError::NotFound(number))?;
        issue.title = new_title.to_string();
        issue.updated_at = self.tick();
        Ok(issue.clone())
    }

    fn patch_comment(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        if new_body.len() > 65_536 {
            return Err(ApiError::BodyTooLarge);
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "patch_comment", &comment_id.to_string());
        // Find the issue owning this comment and update it in place.
        for issue in state.issues.values_mut() {
            for comment in &mut issue.comments {
                if comment.id == comment_id {
                    comment.body = new_body.to_string();
                    comment.updated_at = self.tick();
                    issue.updated_at = comment.updated_at.clone();
                    return Ok(comment.clone());
                }
            }
        }
        Err(ApiError::CommentNotFound(comment_id))
    }

    fn create_comment(&self, number: IssueNumber, body: &str) -> Result<CommentSnapshot, ApiError> {
        if body.len() > 65_536 {
            return Err(ApiError::BodyTooLarge);
        }
        let new_id = CommentId(self.next_comment_id.fetch_add(1, Ordering::SeqCst));
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "create_comment", &number.to_string());
        let issue = state
            .issues
            .get_mut(&number)
            .ok_or(ApiError::NotFound(number))?;
        let snapshot = CommentSnapshot {
            id: new_id,
            body: body.to_string(),
            updated_at: self.tick(),
        };
        issue.comments.push(snapshot.clone());
        issue.updated_at = snapshot.updated_at.clone();
        Ok(snapshot)
    }

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        if body.len() > 65_536 {
            return Err(ApiError::BodyTooLarge);
        }
        let number = IssueNumber(self.next_issue_number.fetch_add(1, Ordering::SeqCst));
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "create_issue", &number.to_string());
        let snapshot = IssueSnapshot {
            number,
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            state: IssueState::Open,
            updated_at: self.tick(),
            comments: Vec::new(),
        };
        state.issues.insert(number, snapshot.clone());
        Ok(snapshot)
    }

    fn set_labels(
        &self,
        number: IssueNumber,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "set_labels", &number.to_string());
        let issue = state
            .issues
            .get_mut(&number)
            .ok_or(ApiError::NotFound(number))?;
        issue.labels = labels.to_vec();
        issue.updated_at = self.tick();
        Ok(issue.clone())
    }

    fn set_state(
        &self,
        number: IssueNumber,
        new_state: IssueState,
    ) -> Result<IssueSnapshot, ApiError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "set_state", &number.to_string());
        let issue = state
            .issues
            .get_mut(&number)
            .ok_or(ApiError::NotFound(number))?;
        issue.state = new_state;
        issue.updated_at = self.tick();
        Ok(issue.clone())
    }

    fn list_spec_issues(&self, filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.record(&mut state, "list_spec_issues", "*");
        let mut out: Vec<SpecSummary> = state
            .issues
            .values()
            .filter(|i| i.labels.iter().any(|l| l == "gwt-spec"))
            .filter(|i| match &filter.phase {
                Some(p) => i
                    .labels
                    .iter()
                    .any(|l| l == &format!("phase/{p}") || l == p.as_str()),
                None => true,
            })
            .filter(|i| match filter.state {
                Some(s) => i.state == s,
                None => true,
            })
            .map(|i| SpecSummary {
                number: i.number,
                title: i.title.clone(),
                state: i.state,
                labels: i.labels.clone(),
                updated_at: i.updated_at.clone(),
            })
            .collect();
        // Deterministic ordering: number desc (newest first).
        out.sort_by_key(|item| Reverse(item.number));
        Ok(out)
    }
}

impl OwnerRepositoryClient for FakeIssueClient {
    fn list_issues(
        &self,
        repository: &RepositoryIdentity,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryIssue>, ApiError> {
        deadline.remaining("list issues")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::ListIssues,
            repository,
            None,
        )?;
        let queued_view = state
            .owner_issue_view_queue
            .get_mut(repository)
            .and_then(VecDeque::pop_front);
        let (mut issues, generation) = if let Some(view) = queued_view {
            view
        } else {
            let issues = state
                .owner_issues
                .get(repository)
                .map(|issues| issues.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let generation = state
                .owner_issue_generation_queue
                .get_mut(repository)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| Self::owner_generation(&state, repository));
            (issues, generation)
        };
        issues.sort_by_key(|issue| issue.number);
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(CompleteCollection::from_complete(issues, generation))
    }

    fn list_comments(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<CompleteCollection<RepositoryComment>, ApiError> {
        deadline.remaining("list comments")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::ListComments,
            repository,
            Some(number),
        )?;
        let issue_exists = state
            .owner_issues
            .get(repository)
            .is_some_and(|issues| issues.contains_key(&number));
        if !issue_exists {
            return Err(ApiError::NotFound(number));
        }
        let mut comments = state
            .owner_comments
            .get(&(repository.clone(), number))
            .cloned()
            .unwrap_or_default();
        comments.sort_by_key(|comment| comment.id);
        let generation = Self::owner_generation(&state, repository);
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(CompleteCollection::from_complete(comments, generation))
    }

    fn fetch_issue(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<RepositoryIssue, ApiError> {
        deadline.remaining("fetch issue")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::FetchIssue,
            repository,
            Some(number),
        )?;
        let issue = state
            .owner_issues
            .get(repository)
            .and_then(|issues| issues.get(&number))
            .cloned()
            .ok_or(ApiError::NotFound(number))?;
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(issue)
    }

    fn create_owner_comment(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        body: &str,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryComment> {
        deadline
            .remaining("create comment")
            .map_err(OwnerMutationError::PreSubmit)?;
        if body.len() > 65_536 {
            return Err(OwnerMutationError::PreSubmit(ApiError::BodyTooLarge));
        }
        let submitted = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let after_error = Self::begin_owner_operation(
                &mut state,
                OwnerRepositoryOperation::CreateComment,
                repository,
                Some(number),
            )
            .map_err(OwnerMutationError::PreSubmit)?;
            let issue_exists = state
                .owner_issues
                .get(repository)
                .is_some_and(|issues| issues.contains_key(&number));
            if !issue_exists {
                return Err(OwnerMutationError::PreSubmit(ApiError::NotFound(number)));
            }
            let comment = RepositoryComment {
                id: CommentId(self.next_owner_comment_id.fetch_add(1, Ordering::SeqCst)),
                body: body.to_string(),
                updated_at: self.tick(),
            };
            state
                .owner_comments
                .entry((repository.clone(), number))
                .or_default()
                .push(comment.clone());
            if let Some(issue) = state
                .owner_issues
                .get_mut(repository)
                .and_then(|issues| issues.get_mut(&number))
            {
                issue.updated_at = comment.updated_at.clone();
            }
            Self::record_owner_mutation(
                &mut state,
                OwnerRepositoryOperation::CreateComment,
                repository,
                Some(number),
            );
            if let Some(error) = after_error {
                return Err(OwnerMutationError::RemoteOutcomeUnknown(error));
            }
            comment
        };
        self.list_comments(repository, number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?
            .items()
            .iter()
            .find(|comment| comment.id == submitted.id && comment.body == body)
            .cloned()
            .ok_or_else(|| {
                OwnerMutationError::RemoteOutcomeUnknown(ApiError::Parse {
                    operation: "read back owner comment".to_string(),
                    message: "created comment was absent or changed during readback".to_string(),
                })
            })
    }

    fn create_owner_issue(
        &self,
        repository: &RepositoryIdentity,
        input: &CreateRepositoryIssue,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue> {
        deadline
            .remaining("create issue")
            .map_err(OwnerMutationError::PreSubmit)?;
        if input.body.len() > 65_536 {
            return Err(OwnerMutationError::PreSubmit(ApiError::BodyTooLarge));
        }
        let number = {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let after_error = Self::begin_owner_operation(
                &mut state,
                OwnerRepositoryOperation::CreateIssue,
                repository,
                None,
            )
            .map_err(OwnerMutationError::PreSubmit)?;
            let next_number = state
                .owner_next_issue_number
                .entry(repository.clone())
                .or_insert(1);
            let number = IssueNumber(*next_number);
            *next_number += 1;
            let issue = RepositoryIssue {
                repository: repository.clone(),
                number,
                title: input.title.clone(),
                body: input.body.clone(),
                labels: input.labels.clone(),
                state: IssueState::Open,
                kind: RepositoryIssueKind::Plain,
                updated_at: self.tick(),
            };
            state
                .owner_issues
                .entry(repository.clone())
                .or_default()
                .insert(number, issue);
            Self::record_owner_mutation(
                &mut state,
                OwnerRepositoryOperation::CreateIssue,
                repository,
                Some(number),
            );
            if let Some(error) = after_error {
                return Err(OwnerMutationError::RemoteOutcomeUnknown(error));
            }
            number
        };
        self.fetch_issue(repository, number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)
    }

    fn close_issue_verified(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> OwnerMutationResult<RepositoryIssue> {
        deadline
            .remaining("close issue")
            .map_err(OwnerMutationError::PreSubmit)?;
        {
            let mut state = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let after_error = Self::begin_owner_operation(
                &mut state,
                OwnerRepositoryOperation::CloseIssue,
                repository,
                Some(number),
            )
            .map_err(OwnerMutationError::PreSubmit)?;
            let updated_at = self.tick();
            let issue = state
                .owner_issues
                .get_mut(repository)
                .and_then(|issues| issues.get_mut(&number))
                .ok_or(OwnerMutationError::PreSubmit(ApiError::NotFound(number)))?;
            issue.state = IssueState::Closed;
            issue.updated_at = updated_at;
            Self::record_owner_mutation(
                &mut state,
                OwnerRepositoryOperation::CloseIssue,
                repository,
                Some(number),
            );
            if let Some(error) = after_error {
                return Err(OwnerMutationError::RemoteOutcomeUnknown(error));
            }
        }
        let verified = self
            .fetch_issue(repository, number, deadline)
            .map_err(OwnerMutationError::RemoteOutcomeUnknown)?;
        if verified.state != IssueState::Closed {
            return Err(OwnerMutationError::RemoteOutcomeUnknown(ApiError::Parse {
                operation: "read back closed owner issue".to_string(),
                message: "duplicate owner remained open after close".to_string(),
            }));
        }
        Ok(verified)
    }

    fn fetch_merged_pull_request(
        &self,
        repository: &RepositoryIdentity,
        number: IssueNumber,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<MergedPullRequest>, ApiError> {
        deadline.remaining("fetch merged pull request")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::FetchMergedPullRequest,
            repository,
            Some(number),
        )?;
        let pull_request = state
            .merged_pull_requests
            .get(&(repository.clone(), number))
            .cloned();
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(pull_request)
    }

    fn fetch_release_by_tag(
        &self,
        repository: &RepositoryIdentity,
        tag: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<Option<RepositoryRelease>, ApiError> {
        deadline.remaining("fetch release")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::FetchRelease,
            repository,
            None,
        )?;
        let release = state
            .releases
            .get(&(repository.clone(), tag.to_string()))
            .cloned();
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(release)
    }

    fn compare_commits(
        &self,
        repository: &RepositoryIdentity,
        base: &str,
        head: &str,
        deadline: &ResolutionDeadline,
    ) -> Result<CommitComparison, ApiError> {
        deadline.remaining("compare commits")?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let after_error = Self::begin_owner_operation(
            &mut state,
            OwnerRepositoryOperation::CompareCommits,
            repository,
            None,
        )?;
        let comparison = state
            .commit_comparisons
            .get(&(repository.clone(), base.to_string(), head.to_string()))
            .cloned()
            .ok_or_else(|| {
                ApiError::Unexpected(format!("commit comparison not found: {base}...{head}"))
            })?;
        if let Some(error) = after_error {
            return Err(error);
        }
        Ok(comparison)
    }
}

#[cfg(test)]
mod owner_repository_tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::client::{
        CollectionGeneration, CommitComparison, CommitComparisonStatus, CompleteCollection,
        CreateRepositoryIssue, MergedPullRequest, OwnerMutationError, OwnerRepositoryClient,
        RepositoryComment, RepositoryIdentity, RepositoryIssue, RepositoryIssueKind,
        RepositoryRelease, ResolutionDeadline,
    };

    fn deadline() -> ResolutionDeadline {
        ResolutionDeadline::new(Duration::from_secs(3), Duration::from_secs(15))
    }

    fn repository_issue(
        repository: &RepositoryIdentity,
        number: u64,
        kind: RepositoryIssueKind,
        state: IssueState,
    ) -> RepositoryIssue {
        RepositoryIssue {
            repository: repository.clone(),
            number: IssueNumber(number),
            title: format!("owner {number}"),
            body: format!("body {number}"),
            labels: match kind {
                RepositoryIssueKind::Plain => Vec::new(),
                RepositoryIssueKind::Spec => vec!["gwt-spec".to_string()],
            },
            state,
            kind,
            updated_at: UpdatedAt::new(format!("u{number}")),
        }
    }

    #[test]
    fn resolution_deadline_uses_one_absolute_expiry_and_caps_connect_time() {
        let expires_at = Instant::now() + Duration::from_secs(10);
        let deadline = ResolutionDeadline::at(expires_at, Duration::from_secs(3));

        assert_eq!(deadline.expires_at(), expires_at);
        assert!(deadline.remaining("list issues").unwrap() <= Duration::from_secs(10));
        assert!(deadline.remaining("list issues").unwrap() > Duration::from_secs(9));
        assert_eq!(
            deadline.connect_timeout("list issues").unwrap(),
            Duration::from_secs(3)
        );

        let expired = ResolutionDeadline::at(
            Instant::now() - Duration::from_millis(1),
            Duration::from_secs(3),
        );
        assert!(matches!(
            expired.remaining("auth"),
            Err(ApiError::Timeout { operation }) if operation == "auth"
        ));
        assert!(matches!(
            expired.connect_timeout("auth"),
            Err(ApiError::Timeout { operation }) if operation == "auth"
        ));
    }

    #[test]
    fn complete_collection_exposes_only_items_and_generation_from_a_complete_result() {
        let collection = CompleteCollection::from_complete(
            vec![1_u8, 2, 3],
            CollectionGeneration::new("generation-7"),
        );

        assert_eq!(collection.items(), &[1, 2, 3]);
        assert_eq!(collection.generation().as_str(), "generation-7");
        let (items, generation) = collection.into_parts();
        assert_eq!(items, vec![1, 2, 3]);
        assert_eq!(generation.as_str(), "generation-7");
    }

    #[test]
    fn fake_returns_complete_repository_and_comment_collections_beyond_one_page() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        for number in 1_u64..=125 {
            let kind = if number.is_multiple_of(2) {
                RepositoryIssueKind::Spec
            } else {
                RepositoryIssueKind::Plain
            };
            let state = if number.is_multiple_of(3) {
                IssueState::Closed
            } else {
                IssueState::Open
            };
            client.seed_repository_issue(repository_issue(&repository, number, kind, state));
        }
        client.seed_repository_comments(
            &repository,
            IssueNumber(1),
            (1..=205)
                .map(|id| RepositoryComment {
                    id: CommentId(id),
                    body: format!("comment {id}"),
                    updated_at: UpdatedAt::new(format!("c{id}")),
                })
                .collect(),
        );

        let issues = client.list_issues(&repository, &deadline()).unwrap();
        assert_eq!(issues.items().len(), 125);
        assert!(issues
            .items()
            .iter()
            .any(|issue| issue.kind == RepositoryIssueKind::Plain));
        assert!(issues
            .items()
            .iter()
            .any(|issue| issue.kind == RepositoryIssueKind::Spec));
        assert!(issues
            .items()
            .iter()
            .any(|issue| issue.state == IssueState::Closed));
        assert!(!issues.generation().as_str().is_empty());

        let comments = client
            .list_comments(&repository, IssueNumber(1), &deadline())
            .unwrap();
        assert_eq!(comments.items().len(), 205);
        assert!(!comments.generation().as_str().is_empty());
    }

    #[test]
    fn repository_identity_is_explicit_and_keeps_upstream_data_isolated() {
        let client = FakeIssueClient::new();
        let upstream = RepositoryIdentity::gwt_upstream();
        let target = RepositoryIdentity::new("example", "target");
        client.seed_repository_issue(repository_issue(
            &upstream,
            10,
            RepositoryIssueKind::Plain,
            IssueState::Open,
        ));
        client.seed_repository_issue(repository_issue(
            &target,
            20,
            RepositoryIssueKind::Plain,
            IssueState::Open,
        ));

        let upstream_issues = client.list_issues(&upstream, &deadline()).unwrap();
        let target_issues = client.list_issues(&target, &deadline()).unwrap();
        assert_eq!(
            upstream_issues
                .items()
                .iter()
                .map(|issue| issue.number)
                .collect::<Vec<_>>(),
            vec![IssueNumber(10)]
        );
        assert_eq!(
            target_issues
                .items()
                .iter()
                .map(|issue| issue.number)
                .collect::<Vec<_>>(),
            vec![IssueNumber(20)]
        );
        assert_eq!(upstream.to_string(), "akiojin/gwt");
    }

    #[test]
    fn repository_issue_numbers_and_generations_are_isolated_per_repository() {
        let client = FakeIssueClient::new();
        let upstream = RepositoryIdentity::gwt_upstream();
        let target = RepositoryIdentity::new("example", "target");
        let input = CreateRepositoryIssue {
            title: "title".to_string(),
            body: "body".to_string(),
            labels: Vec::new(),
        };

        let upstream_before = client
            .list_issues(&upstream, &deadline())
            .unwrap()
            .generation()
            .clone();
        let upstream_issue = client
            .create_owner_issue(&upstream, &input, &deadline())
            .unwrap();
        let upstream_after = client
            .list_issues(&upstream, &deadline())
            .unwrap()
            .generation()
            .clone();
        let target_issue = client
            .create_owner_issue(&target, &input, &deadline())
            .unwrap();
        let upstream_after_target_mutation = client
            .list_issues(&upstream, &deadline())
            .unwrap()
            .generation()
            .clone();

        assert_eq!(upstream_issue.number, IssueNumber(1));
        assert_eq!(target_issue.number, IssueNumber(1));
        assert_ne!(upstream_before, upstream_after);
        assert_eq!(upstream_after, upstream_after_target_mutation);
    }

    #[test]
    fn fake_can_replay_distinct_issue_generations_for_stale_scan_tests() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client
            .queue_owner_issue_generations(&repository, ["generation-before", "generation-after"]);

        let before = client.list_issues(&repository, &deadline()).unwrap();
        let after = client.list_issues(&repository, &deadline()).unwrap();

        assert_eq!(before.generation().as_str(), "generation-before");
        assert_eq!(after.generation().as_str(), "generation-after");
    }

    #[test]
    fn fake_can_replay_distinct_visible_issue_snapshots_for_race_tests() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        let first = repository_issue(
            &repository,
            10,
            RepositoryIssueKind::Plain,
            IssueState::Open,
        );
        let second = repository_issue(
            &repository,
            11,
            RepositoryIssueKind::Plain,
            IssueState::Open,
        );
        client.queue_owner_issue_views(
            &repository,
            [
                (vec![first.clone()], "visible-before"),
                (vec![first.clone()], "visible-before"),
                (vec![first, second.clone()], "visible-after"),
            ],
        );

        let before = client.list_issues(&repository, &deadline()).unwrap();
        let stable_before = client.list_issues(&repository, &deadline()).unwrap();
        let after = client.list_issues(&repository, &deadline()).unwrap();

        assert_eq!(before.items().len(), 1);
        assert_eq!(stable_before.generation().as_str(), "visible-before");
        assert_eq!(
            after.items(),
            &[
                repository_issue(
                    &repository,
                    10,
                    RepositoryIssueKind::Plain,
                    IssueState::Open,
                ),
                second
            ]
        );
        assert_eq!(after.generation().as_str(), "visible-after");
    }

    #[test]
    fn partial_page_fault_never_returns_a_partial_collection() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        for number in 1..=125 {
            client.seed_repository_issue(repository_issue(
                &repository,
                number,
                RepositoryIssueKind::Plain,
                IssueState::Open,
            ));
        }
        client.fail_next_owner_operation(
            OwnerRepositoryOperation::ListIssues,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            ApiError::PartialPage {
                operation: "list issues".to_string(),
                completed_pages: 1,
            },
        );

        let result = client.list_issues(&repository, &deadline());

        assert!(matches!(
            result,
            Err(ApiError::PartialPage {
                completed_pages: 1,
                ..
            })
        ));
        assert_eq!(client.owner_mutation_count(), 0);
    }

    #[test]
    fn pull_request_records_are_not_part_of_the_issue_corpus() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.seed_merged_pull_request(
            &repository,
            MergedPullRequest {
                number: IssueNumber(5),
                merge_commit_sha: "merge-sha".to_string(),
                merged_at: "2026-07-01T12:00:00Z".to_string(),
            },
        );

        assert!(client
            .list_issues(&repository, &deadline())
            .unwrap()
            .items()
            .is_empty());
    }

    #[test]
    fn exact_fetch_and_mutations_are_repository_targeted_and_verified() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.seed_repository_issue(repository_issue(
            &repository,
            7,
            RepositoryIssueKind::Spec,
            IssueState::Open,
        ));

        let exact = client
            .fetch_issue(&repository, IssueNumber(7), &deadline())
            .unwrap();
        assert_eq!(exact.kind, RepositoryIssueKind::Spec);

        let comment = client
            .create_owner_comment(
                &repository,
                IssueNumber(7),
                "<!-- occurrence:opaque -->",
                &deadline(),
            )
            .unwrap();
        assert_eq!(comment.body, "<!-- occurrence:opaque -->");

        let created = client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title: "typed title".to_string(),
                    body: "typed body".to_string(),
                    labels: vec!["improvement".to_string()],
                },
                &deadline(),
            )
            .unwrap();
        assert_eq!(created.repository, repository);
        assert_eq!(created.kind, RepositoryIssueKind::Plain);
        assert_eq!(created.state, IssueState::Open);

        let closed = client
            .close_issue_verified(&repository, created.number, &deadline())
            .unwrap();
        assert_eq!(closed.state, IssueState::Closed);
        assert_eq!(
            client
                .fetch_issue(&repository, created.number, &deadline())
                .unwrap()
                .state,
            IssueState::Closed
        );

        let mutations = client.owner_mutation_call_log();
        assert_eq!(mutations.len(), 3);
        assert_eq!(
            mutations[0].operation,
            OwnerRepositoryOperation::CreateComment
        );
        assert_eq!(
            mutations[1].operation,
            OwnerRepositoryOperation::CreateIssue
        );
        assert_eq!(mutations[2].operation, OwnerRepositoryOperation::CloseIssue);
        assert!(mutations.iter().all(|call| call.repository == repository));
    }

    #[test]
    fn successful_fake_mutations_use_separate_authoritative_readback_operations() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.seed_repository_issue(repository_issue(
            &repository,
            7,
            RepositoryIssueKind::Plain,
            IssueState::Open,
        ));

        client
            .create_owner_comment(&repository, IssueNumber(7), "marker", &deadline())
            .expect("comment readback");
        let created = client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title: "title".to_string(),
                    body: "body".to_string(),
                    labels: Vec::new(),
                },
                &deadline(),
            )
            .expect("issue readback");
        client
            .close_issue_verified(&repository, created.number, &deadline())
            .expect("close readback");

        assert_eq!(
            client
                .owner_call_log()
                .into_iter()
                .map(|call| call.operation)
                .collect::<Vec<_>>(),
            vec![
                OwnerRepositoryOperation::CreateComment,
                OwnerRepositoryOperation::ListComments,
                OwnerRepositoryOperation::CreateIssue,
                OwnerRepositoryOperation::FetchIssue,
                OwnerRepositoryOperation::CloseIssue,
                OwnerRepositoryOperation::FetchIssue,
            ]
        );
    }

    #[test]
    fn fake_issue_readback_failure_happens_after_exactly_one_remote_mutation() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.fail_next_owner_operation(
            OwnerRepositoryOperation::FetchIssue,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            ApiError::Timeout {
                operation: "read back owner issue".to_string(),
            },
        );

        let error = client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title: "title".to_string(),
                    body: "body".to_string(),
                    labels: Vec::new(),
                },
                &deadline(),
            )
            .expect_err("readback failure");

        assert!(matches!(
            error,
            OwnerMutationError::RemoteOutcomeUnknown(ApiError::Timeout { .. })
        ));
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client
                .list_issues(&repository, &deadline())
                .expect("remote issue exists")
                .items()
                .len(),
            1
        );
    }

    #[test]
    fn injected_pre_submit_fault_returns_typed_error_and_records_zero_mutations() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::BeforeSubmit,
            ApiError::PartialPage {
                operation: "list issues".to_string(),
                completed_pages: 1,
            },
        );

        let error = client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title: "title".to_string(),
                    body: "body".to_string(),
                    labels: Vec::new(),
                },
                &deadline(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            OwnerMutationError::PreSubmit(ApiError::PartialPage {
                operation,
                completed_pages: 1
            }) if operation == "list issues"
        ));
        assert_eq!(client.owner_mutation_count(), 0);
        assert!(client
            .list_issues(&repository, &deadline())
            .unwrap()
            .items()
            .is_empty());
    }

    #[test]
    fn injected_post_submit_fault_preserves_remote_state_and_logs_one_mutation() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.fail_next_owner_operation(
            OwnerRepositoryOperation::CreateIssue,
            OwnerRepositoryFaultTiming::AfterSubmit,
            ApiError::Network("response lost".to_string()),
        );

        let error = client
            .create_owner_issue(
                &repository,
                &CreateRepositoryIssue {
                    title: "title".to_string(),
                    body: "body".to_string(),
                    labels: Vec::new(),
                },
                &deadline(),
            )
            .unwrap_err();

        assert!(matches!(
            error,
            OwnerMutationError::RemoteOutcomeUnknown(ApiError::Network(message))
                if message == "response lost"
        ));
        assert_eq!(client.owner_mutation_count(), 1);
        assert_eq!(
            client
                .list_issues(&repository, &deadline())
                .unwrap()
                .items()
                .len(),
            1
        );
    }

    #[test]
    fn merged_pr_release_and_commit_comparison_lookups_are_typed() {
        let client = FakeIssueClient::new();
        let repository = RepositoryIdentity::gwt_upstream();
        client.seed_merged_pull_request(
            &repository,
            MergedPullRequest {
                number: IssueNumber(42),
                merge_commit_sha: "merge-sha".to_string(),
                merged_at: "2026-07-01T12:00:00Z".to_string(),
            },
        );
        client.seed_repository_release(
            &repository,
            RepositoryRelease {
                tag_name: "v1.2.3".to_string(),
                target_commitish: "merge-sha".to_string(),
                published_at: "2026-07-02T12:00:00Z".to_string(),
            },
        );
        client.seed_commit_comparison(
            &repository,
            CommitComparison {
                base: "merge-sha".to_string(),
                head: "observed-sha".to_string(),
                status: CommitComparisonStatus::Ahead,
                ahead_by: 4,
                behind_by: 0,
            },
        );

        assert_eq!(
            client
                .fetch_merged_pull_request(&repository, IssueNumber(42), &deadline())
                .unwrap()
                .unwrap()
                .merge_commit_sha,
            "merge-sha"
        );
        assert_eq!(
            client
                .fetch_release_by_tag(&repository, "v1.2.3", &deadline())
                .unwrap()
                .unwrap()
                .target_commitish,
            "merge-sha"
        );
        assert_eq!(
            client
                .compare_commits(&repository, "merge-sha", "observed-sha", &deadline(),)
                .unwrap()
                .status,
            CommitComparisonStatus::Ahead
        );
    }

    #[test]
    fn new_typed_transport_failures_preserve_operation_context() {
        let errors = [
            ApiError::Timeout {
                operation: "auth".to_string(),
            },
            ApiError::Parse {
                operation: "list comments".to_string(),
                message: "missing cursor".to_string(),
            },
            ApiError::PartialPage {
                operation: "list issues".to_string(),
                completed_pages: 2,
            },
            ApiError::TestOverrideRejected {
                reason: "release build".to_string(),
            },
        ];

        assert!(errors[0].to_string().contains("auth"));
        assert!(errors[1].to_string().contains("list comments"));
        assert!(errors[2].to_string().contains("2"));
        assert!(errors[3].to_string().contains("release build"));
    }
}

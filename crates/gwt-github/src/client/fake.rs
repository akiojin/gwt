//! In-memory implementation of [`IssueClient`] used by contract tests.
//!
//! The [`FakeIssueClient`] stores Issues and comments in a `Mutex`-guarded
//! `HashMap` and records a per-issue call counter so tests can assert call
//! patterns. It is intentionally minimal — just enough to exercise the
//! [`IssueClient`] contract in a predictable, deterministic way.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use crate::client::{
    ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber, IssueSnapshot,
    IssueState, SpecListFilter, SpecSummary, UpdatedAt,
};

/// In-memory fake [`IssueClient`].
pub struct FakeIssueClient {
    inner: Mutex<FakeState>,
    next_issue_number: AtomicU64,
    next_comment_id: AtomicU64,
    clock: AtomicU64,
}

struct FakeState {
    issues: HashMap<IssueNumber, IssueSnapshot>,
    /// Recorded call log for tests (operation + target).
    pub call_log: Vec<String>,
}

impl FakeIssueClient {
    pub fn new() -> Self {
        FakeIssueClient {
            inner: Mutex::new(FakeState {
                issues: HashMap::new(),
                call_log: Vec::new(),
            }),
            next_issue_number: AtomicU64::new(1),
            next_comment_id: AtomicU64::new(1),
            clock: AtomicU64::new(1),
        }
    }

    /// Preload an Issue snapshot. Used by tests to set up fixtures.
    pub fn seed(&self, snapshot: IssueSnapshot) {
        let mut state = self.inner.lock().unwrap();
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
        self.inner.lock().unwrap().call_log.clone()
    }

    fn record(&self, state: &mut FakeState, op: &str, target: &str) {
        state.call_log.push(format!("{op}:{target}"));
    }

    fn tick(&self) -> UpdatedAt {
        let next = self.clock.fetch_add(1, Ordering::SeqCst) + 1;
        UpdatedAt(format!("t{next}"))
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
        self.record(&mut state, "patch_comment", &comment_id.to_string());
        // Find the issue owning this comment and update it in place.
        for issue in state.issues.values_mut() {
            for comment in issue.comments.iter_mut() {
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        let mut state = self.inner.lock().unwrap();
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
        out.sort_by(|a, b| b.number.cmp(&a.number));
        Ok(out)
    }
}

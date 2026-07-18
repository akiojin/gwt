//! In-memory [`TestEnv`] used by `cli` family tests (SPEC-1942 SC-027 split).
//!
//! Captures stdout / stderr buffers, file inputs, fake gh responses, and
//! call logs so tests can assert on dispatch behaviour without touching the
//! filesystem or spawning real subprocesses.

use std::{
    collections::HashMap,
    io::{self},
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

use gwt_git::PrStatus;
use gwt_github::{
    client::{fake::FakeIssueClient, ApiError, IssueClient, ResolutionDeadline},
    IssueNumber, IssueSnapshot,
};

use super::{CliEnv, InternalCommandCall, InternalCommandOutput};

use crate::cli::{
    LinkedPrSummary, PrChecksSummary, PrCreateCall, PrEditCall, PrReview, PrReviewThread,
};

/// Test-visible log entry for repository-targeted Issue creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIssueCreateCall {
    pub owner: String,
    pub repo: String,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
}

pub struct TestEnv {
    pub client: FakeIssueClient,
    pub owner_client: FakeIssueClient,
    pub cache_root: PathBuf,
    pub repo_path: PathBuf,
    pub improvement_source_scope_nonce: String,
    pub stdin: String,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub files: HashMap<String, String>,
    pub target_issue_create_call_log: Vec<TargetIssueCreateCall>,
    pub linked_prs: HashMap<u64, Vec<LinkedPrSummary>>,
    pub linked_pr_call_log: Vec<u64>,
    pub current_pr: Option<PrStatus>,
    pub prs: HashMap<u64, PrStatus>,
    pub created_pr: Option<PrStatus>,
    pub pr_comments: Vec<(u64, String)>,
    pub pr_create_call_log: Vec<PrCreateCall>,
    pub pr_edit_call_log: Vec<PrEditCall>,
    pub pr_reviews: HashMap<u64, Vec<PrReview>>,
    pub pr_review_threads: HashMap<u64, Vec<PrReviewThread>>,
    pub pr_checks: HashMap<u64, PrChecksSummary>,
    pub pr_current_call_count: usize,
    pub pr_view_call_log: Vec<u64>,
    pub pr_ready_call_log: Vec<u64>,
    pub pr_draft_call_log: Vec<u64>,
    pub pr_reviews_call_log: Vec<u64>,
    pub pr_review_threads_call_log: Vec<u64>,
    pub pr_reply_and_resolve_call_log: Vec<(u64, String)>,
    pub pr_checks_call_log: Vec<u64>,
    pub run_logs: HashMap<u64, String>,
    pub run_log_call_log: Vec<u64>,
    pub job_logs: HashMap<u64, String>,
    pub job_log_call_log: Vec<u64>,
    pub internal_command_call_log: Vec<InternalCommandCall>,
    owner_client_access_log: Mutex<Vec<OwnerClientAccessObservation>>,
}

#[derive(Debug, Clone)]
struct OwnerClientAccessObservation {
    connect_timeout: Duration,
    total_remaining: Duration,
    candidate_store_persisted: bool,
    candidate_id: Option<String>,
}

impl TestEnv {
    pub fn new(cache_root: PathBuf) -> Self {
        let repo_path = cache_root.clone();
        TestEnv {
            client: FakeIssueClient::new(),
            owner_client: FakeIssueClient::new(),
            cache_root,
            repo_path,
            improvement_source_scope_nonce: "0".repeat(64),
            stdin: String::new(),
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: HashMap::new(),
            target_issue_create_call_log: Vec::new(),
            linked_prs: HashMap::new(),
            linked_pr_call_log: Vec::new(),
            current_pr: None,
            prs: HashMap::new(),
            created_pr: None,
            pr_comments: Vec::new(),
            pr_create_call_log: Vec::new(),
            pr_edit_call_log: Vec::new(),
            pr_reviews: HashMap::new(),
            pr_review_threads: HashMap::new(),
            pr_checks: HashMap::new(),
            pr_current_call_count: 0,
            pr_view_call_log: Vec::new(),
            pr_ready_call_log: Vec::new(),
            pr_draft_call_log: Vec::new(),
            pr_reviews_call_log: Vec::new(),
            pr_review_threads_call_log: Vec::new(),
            pr_reply_and_resolve_call_log: Vec::new(),
            pr_checks_call_log: Vec::new(),
            run_logs: HashMap::new(),
            run_log_call_log: Vec::new(),
            job_logs: HashMap::new(),
            job_log_call_log: Vec::new(),
            internal_command_call_log: Vec::new(),
            owner_client_access_log: Mutex::new(Vec::new()),
        }
    }

    pub fn owner_client_access_count(&self) -> usize {
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn last_owner_client_budget(&self) -> Option<(Duration, Duration)> {
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .map(|access| (access.connect_timeout, access.total_remaining))
    }

    pub fn last_owner_client_access_saw_persisted_candidate(&self) -> Option<bool> {
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .map(|access| access.candidate_store_persisted)
    }

    pub fn last_owner_client_candidate_id(&self) -> Option<String> {
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .and_then(|access| access.candidate_id.clone())
    }

    pub fn clear_owner_client_access_log(&self) {
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub fn seed_linked_prs(&mut self, number: u64, linked_prs: Vec<LinkedPrSummary>) {
        self.linked_prs.insert(number, linked_prs);
    }

    pub fn linked_pr_calls(&self) -> Vec<u64> {
        self.linked_pr_call_log.clone()
    }

    pub fn clear_linked_pr_calls(&mut self) {
        self.linked_pr_call_log.clear();
    }

    pub fn seed_current_pr(&mut self, pr: Option<PrStatus>) {
        self.current_pr = pr;
    }

    pub fn seed_pr(&mut self, number: u64, pr: PrStatus) {
        self.prs.insert(number, pr);
    }

    pub fn seed_created_pr(&mut self, pr: PrStatus) {
        self.created_pr = Some(pr);
    }

    pub fn seed_pr_reviews(&mut self, number: u64, reviews: Vec<PrReview>) {
        self.pr_reviews.insert(number, reviews);
    }

    pub fn seed_pr_review_threads(&mut self, number: u64, threads: Vec<PrReviewThread>) {
        self.pr_review_threads.insert(number, threads);
    }

    pub fn seed_pr_checks(&mut self, number: u64, summary: PrChecksSummary) {
        self.pr_checks.insert(number, summary);
    }

    pub fn seed_run_log(&mut self, run_id: u64, log: impl Into<String>) {
        self.run_logs.insert(run_id, log.into());
    }

    pub fn seed_job_log(&mut self, job_id: u64, log: impl Into<String>) {
        self.job_logs.insert(job_id, log.into());
    }
}

impl CliEnv for TestEnv {
    type Client = FakeIssueClient;
    type OwnerClient = FakeIssueClient;
    fn client(&self) -> &Self::Client {
        &self.client
    }
    fn improvement_owner_client(
        &self,
        deadline: &ResolutionDeadline,
    ) -> Result<&Self::OwnerClient, ApiError> {
        let total_remaining = deadline.remaining("test owner client access")?;
        let connect_timeout = deadline.connect_timeout("test owner client connect")?;
        let candidate_store_path = gwt_core::paths::gwt_project_dir_for_repo_path(&self.repo_path)
            .join("improvements")
            .join("candidates.json");
        let candidate_id = std::fs::read(&candidate_store_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|store| {
                store
                    .get("candidates")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|candidates| {
                        candidates.iter().find_map(|candidate| {
                            if candidate.get("state").and_then(serde_json::Value::as_str)
                                == Some("owner-resolving")
                            {
                                candidate
                                    .get("id")
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_string)
                            } else {
                                None
                            }
                        })
                    })
            });
        let candidate_store_persisted = candidate_id.is_some();
        self.owner_client_access_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(OwnerClientAccessObservation {
                connect_timeout,
                total_remaining,
                candidate_store_persisted,
                candidate_id,
            });
        Ok(&self.owner_client)
    }
    fn improvement_source_scope_nonce(&self) -> Result<String, gwt_github::SpecOpsError> {
        Ok(self.improvement_source_scope_nonce.clone())
    }
    fn cache_root(&self) -> PathBuf {
        self.cache_root.clone()
    }
    fn repo_path(&self) -> &std::path::Path {
        &self.repo_path
    }
    fn stdout(&mut self) -> &mut dyn io::Write {
        &mut self.stdout
    }
    fn stderr(&mut self) -> &mut dyn io::Write {
        &mut self.stderr
    }
    fn read_stdin(&mut self) -> io::Result<String> {
        Ok(std::mem::take(&mut self.stdin))
    }
    fn read_file(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no such file: {path}")))
    }
    fn create_issue_in_repo(
        &mut self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> io::Result<IssueSnapshot> {
        self.target_issue_create_call_log
            .push(TargetIssueCreateCall {
                owner: owner.to_string(),
                repo: repo.to_string(),
                title: title.to_string(),
                body: body.to_string(),
                labels: labels.to_vec(),
            });
        self.client
            .create_issue(title, body, labels)
            .map_err(|err| io::Error::other(err.to_string()))
    }
    fn fetch_linked_prs(&mut self, number: IssueNumber) -> io::Result<Vec<LinkedPrSummary>> {
        self.linked_pr_call_log.push(number.0);
        Ok(self.linked_prs.get(&number.0).cloned().unwrap_or_default())
    }
    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>> {
        self.pr_current_call_count += 1;
        Ok(self.current_pr.clone())
    }
    fn create_pr(
        &mut self,
        base: &str,
        head: Option<&str>,
        title: &str,
        body: &str,
        labels: &[String],
        draft: bool,
    ) -> io::Result<PrStatus> {
        self.pr_create_call_log.push(PrCreateCall {
            base: base.to_string(),
            head: head.map(ToOwned::to_owned),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            draft,
        });
        self.created_pr
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no created pr seeded"))
    }
    fn edit_pr(
        &mut self,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
        add_labels: &[String],
    ) -> io::Result<PrStatus> {
        self.pr_edit_call_log.push(PrEditCall {
            number,
            title: title.map(ToOwned::to_owned),
            body: body.map(ToOwned::to_owned),
            add_labels: add_labels.to_vec(),
        });
        self.prs
            .get(&number)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no pr: {number}")))
    }
    fn fetch_pr(&mut self, number: u64) -> io::Result<PrStatus> {
        self.pr_view_call_log.push(number);
        self.prs
            .get(&number)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no pr: {number}")))
    }
    fn mark_pr_ready(&mut self, number: u64) -> io::Result<PrStatus> {
        self.pr_ready_call_log.push(number);
        self.prs
            .get(&number)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no pr: {number}")))
    }
    fn convert_pr_to_draft(&mut self, number: u64) -> io::Result<PrStatus> {
        self.pr_draft_call_log.push(number);
        self.prs
            .get(&number)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no pr: {number}")))
    }
    fn comment_on_pr(&mut self, number: u64, body: &str) -> io::Result<()> {
        self.pr_comments.push((number, body.to_string()));
        Ok(())
    }
    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>> {
        self.pr_reviews_call_log.push(number);
        Ok(self.pr_reviews.get(&number).cloned().unwrap_or_default())
    }
    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>> {
        self.pr_review_threads_call_log.push(number);
        Ok(self
            .pr_review_threads
            .get(&number)
            .cloned()
            .unwrap_or_default())
    }
    fn reply_and_resolve_pr_review_threads(
        &mut self,
        number: u64,
        body: &str,
    ) -> io::Result<usize> {
        self.pr_reply_and_resolve_call_log
            .push((number, body.to_string()));
        let count = self
            .pr_review_threads
            .get(&number)
            .map(|threads| threads.iter().filter(|thread| !thread.is_resolved).count())
            .unwrap_or(0);
        Ok(count)
    }
    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary> {
        self.pr_checks_call_log.push(number);
        self.pr_checks.get(&number).cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("no pr checks: {number}"))
        })
    }
    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String> {
        self.run_log_call_log.push(run_id);
        self.run_logs
            .get(&run_id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no run log: {run_id}")))
    }
    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String> {
        self.job_log_call_log.push(job_id);
        self.job_logs
            .get(&job_id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no job log: {job_id}")))
    }
    fn run_internal_command(
        &mut self,
        args: &[String],
        stdin: &str,
    ) -> io::Result<InternalCommandOutput> {
        self.internal_command_call_log.push(InternalCommandCall {
            args: args.to_vec(),
            stdin: stdin.to_string(),
        });

        let mut child = TestEnv::new(self.cache_root.clone());
        child.repo_path = self.repo_path.clone();
        child.stdin = stdin.to_string();
        child.files = self.files.clone();
        let status = super::dispatch(&mut child, args);
        Ok(InternalCommandOutput {
            status,
            stdout: child.stdout,
            stderr: child.stderr,
        })
    }
}

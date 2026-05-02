//! Production [`DefaultCliEnv`] + [`LazyIssueClient`] (SPEC-1942 SC-027 split).
//!
//! Hosts the runtime [`CliEnv`] implementation that talks to live `gh` /
//! HTTP / filesystem. Tests should use [`super::test_env::TestEnv`] instead.

use std::{
    fs,
    io::{self},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
};

use gwt_git::PrStatus;
use gwt_github::{
    client::{http::HttpIssueClient, IssueClient},
    IssueNumber, SpecListFilter,
};

use super::{CliEnv, InternalCommandOutput};

use crate::cli::{LinkedPrSummary, PrChecksSummary, PrCreateCall, PrReview, PrReviewThread};

pub(crate) type IssueClientFactory =
    dyn Fn(&str, &str) -> Result<HttpIssueClient, gwt_github::client::ApiError> + Send + Sync;

pub struct LazyIssueClient {
    owner: String,
    repo: String,
    factory: Arc<IssueClientFactory>,
    resolved: OnceLock<HttpIssueClient>,
}

impl LazyIssueClient {
    pub(super) fn new_with_factory(
        owner: &str,
        repo: &str,
        factory: Arc<IssueClientFactory>,
    ) -> Self {
        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
            factory,
            resolved: OnceLock::new(),
        }
    }

    fn resolve(&self) -> Result<&HttpIssueClient, gwt_github::client::ApiError> {
        if let Some(client) = self.resolved.get() {
            return Ok(client);
        }

        let client = (self.factory)(&self.owner, &self.repo)?;
        let _ = self.resolved.set(client);
        self.resolved.get().ok_or_else(|| {
            gwt_github::client::ApiError::Unexpected(
                "lazy issue client failed to initialize".to_string(),
            )
        })
    }
}

impl IssueClient for LazyIssueClient {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&gwt_github::client::UpdatedAt>,
    ) -> Result<gwt_github::client::FetchResult, gwt_github::client::ApiError> {
        self.resolve()?.fetch(number, since)
    }

    fn patch_body(
        &self,
        number: IssueNumber,
        new_body: &str,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.patch_body(number, new_body)
    }

    fn patch_title(
        &self,
        number: IssueNumber,
        new_title: &str,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.patch_title(number, new_title)
    }

    fn patch_comment(
        &self,
        comment_id: gwt_github::client::CommentId,
        new_body: &str,
    ) -> Result<gwt_github::client::CommentSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.patch_comment(comment_id, new_body)
    }

    fn create_comment(
        &self,
        number: IssueNumber,
        body: &str,
    ) -> Result<gwt_github::client::CommentSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.create_comment(number, body)
    }

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.create_issue(title, body, labels)
    }

    fn set_labels(
        &self,
        number: IssueNumber,
        labels: &[String],
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.set_labels(number, labels)
    }

    fn set_state(
        &self,
        number: IssueNumber,
        state: gwt_github::client::IssueState,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.resolve()?.set_state(number, state)
    }

    fn list_spec_issues(
        &self,
        filter: &SpecListFilter,
    ) -> Result<Vec<gwt_github::client::SpecSummary>, gwt_github::client::ApiError> {
        self.resolve()?.list_spec_issues(filter)
    }
}

/// Default production [`CliEnv`] that defers GitHub auth until a command
pub struct DefaultCliEnv {
    client: LazyIssueClient,
    cache_root: PathBuf,
    repo_path: PathBuf,
    owner: String,
    repo: String,
    stdout: io::Stdout,
    stderr: io::Stderr,
}

impl DefaultCliEnv {
    pub fn new(owner: &str, repo: &str, repo_path: PathBuf) -> Self {
        Self::new_with_client_factory(
            owner,
            repo,
            repo_path,
            Arc::new(HttpIssueClient::from_gh_auth),
        )
    }

    pub(super) fn new_with_client_factory(
        owner: &str,
        repo: &str,
        repo_path: PathBuf,
        factory: Arc<IssueClientFactory>,
    ) -> Self {
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path(&repo_path)
            .unwrap_or_else(|| crate::issue_cache::issue_cache_root_for_repo_slug(owner, repo));
        Self::new_with_client_factory_and_cache_root(owner, repo, repo_path, cache_root, factory)
    }

    pub(super) fn new_with_client_factory_and_cache_root(
        owner: &str,
        repo: &str,
        repo_path: PathBuf,
        cache_root: PathBuf,
        factory: Arc<IssueClientFactory>,
    ) -> Self {
        DefaultCliEnv {
            client: LazyIssueClient::new_with_factory(owner, repo, factory),
            cache_root,
            repo_path,
            owner: owner.to_string(),
            repo: repo.to_string(),
            stdout: io::stdout(),
            stderr: io::stderr(),
        }
    }

    /// Build an env for hook dispatch that deliberately skips
    /// `gh auth token` resolution. Hook handlers never touch GitHub,
    /// so forcing them to depend on the user having run `gh auth
    /// login` would break every Bash tool call on a fresh machine.
    ///
    /// The inner `HttpIssueClient` is constructed with an empty token
    /// and empty owner/repo strings; any attempt to actually call it
    /// would fail (which is fine — the hook code paths go through
    /// `run_hook`, not the SPEC issue client).
    pub fn new_for_hooks() -> Self {
        Self::new_with_client_factory_and_cache_root(
            "",
            "",
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            crate::issue_cache::detached_issue_cache_root(),
            Arc::new(|_, _| {
                let transport = gwt_github::client::http::ReqwestTransport::new()
                    .map_err(|e| gwt_github::client::ApiError::Network(e.to_string()))?;
                Ok(HttpIssueClient::with_transport(
                    transport,
                    String::new(),
                    "",
                    "",
                ))
            }),
        )
    }
}

impl CliEnv for DefaultCliEnv {
    type Client = LazyIssueClient;

    fn client(&self) -> &Self::Client {
        &self.client
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
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut io::stdin(), &mut buffer)?;
        Ok(buffer)
    }
    fn read_file(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(path)
    }
    fn fetch_linked_prs(&mut self, number: IssueNumber) -> io::Result<Vec<LinkedPrSummary>> {
        crate::cli::issue::fetch_linked_prs_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>> {
        crate::cli::pr::fetch_current_pr_via_gh(&self.repo_path)
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
        crate::cli::pr::edit_or_create_repo_guard(&self.owner, &self.repo)?;
        let request = PrCreateCall {
            base: base.to_string(),
            head: head.map(ToOwned::to_owned),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            draft,
        };
        crate::cli::pr::create_pr_via_gh(
            &format!("{}/{}", self.owner, self.repo),
            &self.repo_path,
            &request,
        )
    }
    fn edit_pr(
        &mut self,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
        add_labels: &[String],
    ) -> io::Result<PrStatus> {
        crate::cli::pr::edit_or_create_repo_guard(&self.owner, &self.repo)?;
        crate::cli::pr::edit_pr_via_gh(
            &format!("{}/{}", self.owner, self.repo),
            &self.repo_path,
            number,
            title,
            body,
            add_labels,
        )
    }
    fn fetch_pr(&mut self, number: u64) -> io::Result<PrStatus> {
        gwt_git::pr_status::fetch_pr_status(&format!("{}/{}", self.owner, self.repo), number)
            .map_err(|err| io::Error::other(err.to_string()))
    }
    fn comment_on_pr(&mut self, number: u64, body: &str) -> io::Result<()> {
        crate::cli::pr::comment_on_pr_via_gh(&self.repo_path, number, body)
    }
    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>> {
        crate::cli::pr::fetch_pr_reviews_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>> {
        crate::cli::pr::fetch_pr_review_threads_via_gh(&self.owner, &self.repo, number)
    }
    fn reply_and_resolve_pr_review_threads(
        &mut self,
        number: u64,
        body: &str,
    ) -> io::Result<usize> {
        crate::cli::pr::reply_and_resolve_pr_review_threads_via_gh(
            &self.owner,
            &self.repo,
            number,
            body,
        )
    }
    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary> {
        crate::cli::pr::fetch_pr_checks_via_gh(
            &format!("{}/{}", self.owner, self.repo),
            &self.repo_path,
            number,
        )
    }
    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String> {
        crate::cli::actions::fetch_actions_run_log_via_gh(&self.repo_path, run_id)
    }
    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String> {
        crate::cli::actions::fetch_actions_job_log_via_gh(
            &self.owner,
            &self.repo,
            &self.repo_path,
            job_id,
        )
    }
    fn run_internal_command(
        &mut self,
        args: &[String],
        stdin: &str,
    ) -> io::Result<InternalCommandOutput> {
        let current_exe = std::env::current_exe()?;
        let current_exe =
            dunce::canonicalize(&current_exe).unwrap_or_else(|_| current_exe.to_path_buf());
        let mut child = Command::new(current_exe)
            .args(args.iter().skip(1))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(&self.repo_path)
            .spawn()?;

        if let Some(mut child_stdin) = child.stdin.take() {
            io::Write::write_all(&mut child_stdin, stdin.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        Ok(InternalCommandOutput {
            status: output.status.code().unwrap_or(1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

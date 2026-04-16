use std::{
    collections::HashMap,
    fs,
    io::{self},
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use gwt_git::PrStatus;
use gwt_github::{
    client::{fake::FakeIssueClient, http::HttpIssueClient, IssueClient},
    IssueNumber, SpecListFilter,
};

use super::{
    parse_actions_args, parse_board_args, parse_hook_args, parse_issue_args, parse_pr_args, run,
    CliParseError, LinkedPrSummary, PrChecksSummary, PrCreateCall, PrEditCall, PrReview,
    PrReviewThread,
};

type IssueClientFactory =
    dyn Fn(&str, &str) -> Result<HttpIssueClient, gwt_github::client::ApiError> + Send + Sync;

/// High-level runtime environment for the CLI. Kept as a trait so tests can
/// inject a [`FakeIssueClient`] instead of spinning up real HTTP.
pub trait CliEnv {
    type Client: IssueClient;
    fn client(&self) -> &Self::Client;
    fn cache_root(&self) -> PathBuf;
    fn repo_path(&self) -> &std::path::Path;
    fn stdout(&mut self) -> &mut dyn io::Write;
    fn stderr(&mut self) -> &mut dyn io::Write;
    fn read_stdin(&mut self) -> io::Result<String>;
    fn read_file(&self, path: &str) -> io::Result<String>;
    fn fetch_linked_prs(&mut self, number: IssueNumber) -> io::Result<Vec<LinkedPrSummary>>;
    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>>;
    fn create_pr(
        &mut self,
        base: &str,
        head: Option<&str>,
        title: &str,
        body: &str,
        labels: &[String],
        draft: bool,
    ) -> io::Result<PrStatus>;
    fn edit_pr(
        &mut self,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
        add_labels: &[String],
    ) -> io::Result<PrStatus>;
    fn fetch_pr(&mut self, number: u64) -> io::Result<PrStatus>;
    fn comment_on_pr(&mut self, number: u64, body: &str) -> io::Result<()>;
    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>>;
    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>>;
    fn reply_and_resolve_pr_review_threads(&mut self, number: u64, body: &str)
        -> io::Result<usize>;
    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary>;
    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String>;
    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String>;
}

// ---------------------------------------------------------------------------
// ClientRef: a borrow wrapper that still implements IssueClient
// ---------------------------------------------------------------------------

pub(crate) struct ClientRef<'a, C: IssueClient> {
    pub(crate) inner: &'a C,
}

impl<'a, C: IssueClient> IssueClient for ClientRef<'a, C> {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&gwt_github::client::UpdatedAt>,
    ) -> Result<gwt_github::client::FetchResult, gwt_github::client::ApiError> {
        self.inner.fetch(number, since)
    }
    fn patch_body(
        &self,
        number: IssueNumber,
        new_body: &str,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.inner.patch_body(number, new_body)
    }
    fn patch_title(
        &self,
        number: IssueNumber,
        new_title: &str,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.inner.patch_title(number, new_title)
    }
    fn patch_comment(
        &self,
        comment_id: gwt_github::client::CommentId,
        new_body: &str,
    ) -> Result<gwt_github::client::CommentSnapshot, gwt_github::client::ApiError> {
        self.inner.patch_comment(comment_id, new_body)
    }
    fn create_comment(
        &self,
        number: IssueNumber,
        body: &str,
    ) -> Result<gwt_github::client::CommentSnapshot, gwt_github::client::ApiError> {
        self.inner.create_comment(number, body)
    }
    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.inner.create_issue(title, body, labels)
    }
    fn set_labels(
        &self,
        number: IssueNumber,
        labels: &[String],
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.inner.set_labels(number, labels)
    }
    fn set_state(
        &self,
        number: IssueNumber,
        state: gwt_github::client::IssueState,
    ) -> Result<gwt_github::client::IssueSnapshot, gwt_github::client::ApiError> {
        self.inner.set_state(number, state)
    }
    fn list_spec_issues(
        &self,
        filter: &SpecListFilter,
    ) -> Result<Vec<gwt_github::client::SpecSummary>, gwt_github::client::ApiError> {
        self.inner.list_spec_issues(filter)
    }
}

// ---------------------------------------------------------------------------
// DefaultCliEnv: production runtime wiring
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub struct LazyIssueClient {
    owner: String,
    repo: String,
    factory: Arc<IssueClientFactory>,
    resolved: OnceLock<HttpIssueClient>,
}

impl LazyIssueClient {
    fn new_with_factory(owner: &str, repo: &str, factory: Arc<IssueClientFactory>) -> Self {
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
/// actually needs the issue client and uses the user's home cache directory.
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

    fn new_with_client_factory(
        owner: &str,
        repo: &str,
        repo_path: PathBuf,
        factory: Arc<IssueClientFactory>,
    ) -> Self {
        let cache_root = crate::issue_cache::issue_cache_root_for_repo_path(&repo_path)
            .unwrap_or_else(|| crate::issue_cache::issue_cache_root_for_repo_slug(owner, repo));
        Self::new_with_client_factory_and_cache_root(owner, repo, repo_path, cache_root, factory)
    }

    fn new_with_client_factory_and_cache_root(
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
        super::fetch_linked_prs_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>> {
        super::fetch_current_pr_via_gh(&self.repo_path)
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
        super::edit_or_create_repo_guard(&self.owner, &self.repo)?;
        let request = PrCreateCall {
            base: base.to_string(),
            head: head.map(ToOwned::to_owned),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            draft,
        };
        super::create_pr_via_gh(
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
        super::edit_or_create_repo_guard(&self.owner, &self.repo)?;
        super::edit_pr_via_gh(
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
        super::comment_on_pr_via_gh(&self.repo_path, number, body)
    }
    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>> {
        super::fetch_pr_reviews_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>> {
        super::fetch_pr_review_threads_via_gh(&self.owner, &self.repo, number)
    }
    fn reply_and_resolve_pr_review_threads(
        &mut self,
        number: u64,
        body: &str,
    ) -> io::Result<usize> {
        super::reply_and_resolve_pr_review_threads_via_gh(&self.owner, &self.repo, number, body)
    }
    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary> {
        super::fetch_pr_checks_via_gh(
            &format!("{}/{}", self.owner, self.repo),
            &self.repo_path,
            number,
        )
    }
    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String> {
        super::fetch_actions_run_log_via_gh(&self.repo_path, run_id)
    }
    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String> {
        super::fetch_actions_job_log_via_gh(&self.owner, &self.repo, &self.repo_path, job_id)
    }
}

/// Convenience for tests and the main entry point: take a raw argv slice,
/// parse the subcommand, and run it. Returns the process exit code.
pub fn dispatch<E: CliEnv>(env: &mut E, args: &[String]) -> i32 {
    // args[0] is the program name. args[1] is the top-level verb we already
    // matched in `should_dispatch_cli`.
    let top_verb = args.get(1).map(String::as_str).unwrap_or("");
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();

    let parse_result = match top_verb {
        "issue" => parse_issue_args(&rest),
        "pr" => parse_pr_args(&rest),
        "actions" => parse_actions_args(&rest),
        "board" => parse_board_args(&rest),
        "hook" => parse_hook_args(&rest),
        "update" => Ok(super::CliCommand::Update {
            check_only: rest.iter().any(|a| a == "--check"),
        }),
        "__internal" => match rest.first().map(String::as_str) {
            Some("apply-update") => Ok(super::CliCommand::InternalApplyUpdate {
                rest: rest[1..].to_vec(),
            }),
            Some("run-installer") => Ok(super::CliCommand::InternalRunInstaller {
                rest: rest[1..].to_vec(),
            }),
            other => Err(CliParseError::UnknownSubcommand(format!(
                "__internal {}",
                other.unwrap_or("")
            ))),
        },
        _ => Err(CliParseError::UnknownSubcommand(top_verb.to_string())),
    };

    match parse_result {
        Ok(cmd) => match run(env, cmd) {
            Ok(code) => code,
            Err(e) => {
                let _ = writeln!(env.stderr(), "gwt {top_verb}: {e}");
                1
            }
        },
        Err(e) => {
            let _ = writeln!(env.stderr(), "gwt {top_verb}: {e}");
            2
        }
    }
}

// ---------------------------------------------------------------------------
// TestEnv: an in-memory CliEnv used by cli_test.rs
// ---------------------------------------------------------------------------

/// A lightweight in-memory [`CliEnv`] for unit tests. Captures stdout/stderr
/// as `Vec<u8>` and serves file contents from a `HashMap`.
pub struct TestEnv {
    pub client: FakeIssueClient,
    pub cache_root: PathBuf,
    pub repo_path: PathBuf,
    pub stdin: String,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub files: HashMap<String, String>,
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
    pub pr_reviews_call_log: Vec<u64>,
    pub pr_review_threads_call_log: Vec<u64>,
    pub pr_reply_and_resolve_call_log: Vec<(u64, String)>,
    pub pr_checks_call_log: Vec<u64>,
    pub run_logs: HashMap<u64, String>,
    pub run_log_call_log: Vec<u64>,
    pub job_logs: HashMap<u64, String>,
    pub job_log_call_log: Vec<u64>,
}

impl TestEnv {
    pub fn new(cache_root: PathBuf) -> Self {
        let repo_path = cache_root.clone();
        TestEnv {
            client: FakeIssueClient::new(),
            cache_root,
            repo_path,
            stdin: String::new(),
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: HashMap::new(),
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
            pr_reviews_call_log: Vec::new(),
            pr_review_threads_call_log: Vec::new(),
            pr_reply_and_resolve_call_log: Vec::new(),
            pr_checks_call_log: Vec::new(),
            run_logs: HashMap::new(),
            run_log_call_log: Vec::new(),
            job_logs: HashMap::new(),
            job_log_call_log: Vec::new(),
        }
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
        Ok(std::mem::take(&mut self.stdin))
    }
    fn read_file(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no such file: {path}")))
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
            .map(|threads| {
                threads
                    .iter()
                    .filter(|thread| !thread.is_resolved && !thread.is_outdated)
                    .count()
            })
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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn failing_factory(counter: Arc<AtomicUsize>) -> Arc<IssueClientFactory> {
        Arc::new(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Err(gwt_github::client::ApiError::Unauthorized)
        })
    }

    #[test]
    fn lazy_issue_client_defers_resolution_until_first_issue_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let client =
            LazyIssueClient::new_with_factory("akiojin", "gwt", failing_factory(calls.clone()));

        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let result = client.list_spec_issues(&SpecListFilter::default());
        assert!(matches!(
            result,
            Err(gwt_github::client::ApiError::Unauthorized)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn default_cli_env_construction_does_not_touch_issue_client_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let env = DefaultCliEnv::new_with_client_factory(
            "akiojin",
            "gwt",
            PathBuf::from("."),
            failing_factory(calls.clone()),
        );

        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let result = env.client().list_spec_issues(&SpecListFilter::default());
        assert!(matches!(
            result,
            Err(gwt_github::client::ApiError::Unauthorized)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn new_for_hooks_keeps_detached_cache_root() {
        let env = DefaultCliEnv::new_for_hooks();

        assert_eq!(
            env.cache_root(),
            crate::issue_cache::detached_issue_cache_root()
        );
    }
}

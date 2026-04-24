use std::{
    collections::HashMap,
    fs,
    io::{self},
    path::PathBuf,
    process::{Command, Stdio},
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
    fn run_internal_command(
        &mut self,
        args: &[String],
        stdin: &str,
    ) -> io::Result<InternalCommandOutput>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalCommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalCommandCall {
    pub args: Vec<String>,
    pub stdin: String,
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
    fn run_internal_command(
        &mut self,
        args: &[String],
        stdin: &str,
    ) -> io::Result<InternalCommandOutput> {
        let current_exe = std::env::current_exe()?;
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
        "discuss" => super::parse_discuss_args(&rest),
        "plan" => super::parse_plan_args(&rest),
        "build" => super::parse_build_args(&rest),
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
            Some("daemon-hook") => parse_hook_args(&rest[1..]).map(|cmd| match cmd {
                super::CliCommand::Hook { name, rest } => {
                    super::CliCommand::InternalDaemonHook { name, rest }
                }
                _ => unreachable!("parse_hook_args must return CliCommand::Hook"),
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
    pub internal_command_call_log: Vec<InternalCommandCall>,
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
            internal_command_call_log: Vec::new(),
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
        let status = dispatch(&mut child, args);
        Ok(InternalCommandOutput {
            status,
            stdout: child.stdout,
            stderr: child.stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    fn sample_pr_status() -> PrStatus {
        PrStatus {
            number: 128,
            title: "Enforce coverage".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://github.com/akiojin/gwt/pull/128".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: "CLEAN".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    fn sample_issue_snapshot(number: u64, labels: &[&str]) -> gwt_github::client::IssueSnapshot {
        gwt_github::client::IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: "Body".to_string(),
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
            state: gwt_github::IssueState::Open,
            updated_at: gwt_github::client::UpdatedAt::new("2026-04-20T00:00:00Z"),
            comments: vec![gwt_github::client::CommentSnapshot {
                id: gwt_github::client::CommentId(9),
                body: "comment".to_string(),
                updated_at: gwt_github::client::UpdatedAt::new("2026-04-20T00:01:00Z"),
            }],
        }
    }

    fn compile_fake_gh(bin_dir: &Path) {
        let source = r###"
use std::{env, process::ExitCode};

fn pr_json(number: &str, title: &str) -> String {
    format!(
        "{{\"number\":{number},\"title\":\"{title}\",\"state\":\"OPEN\",\"url\":\"https://github.com/akiojin/gwt/pull/{number}\",\"mergeable\":\"MERGEABLE\",\"mergeStateStatus\":\"CLEAN\",\"statusCheckRollup\":[{{\"name\":\"ci\",\"status\":\"COMPLETED\",\"conclusion\":\"SUCCESS\"}}],\"reviewDecision\":\"APPROVED\"}}"
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [pr, view, json_flag, ..] if pr == "pr" && view == "view" && json_flag == "--json" => {
            println!("{}", pr_json("12", "Current PR"));
            ExitCode::SUCCESS
        }
        [pr, view, number, repo_flag, _, json_flag, ..]
            if pr == "pr" && view == "view" && repo_flag == "--repo" && json_flag == "--json" =>
        {
            println!("{}", pr_json(number, "Fetched PR"));
            ExitCode::SUCCESS
        }
        [pr, create, ..] if pr == "pr" && create == "create" => {
            println!("https://github.com/akiojin/gwt/pull/12");
            ExitCode::SUCCESS
        }
        [pr, edit, ..] if pr == "pr" && edit == "edit" => ExitCode::SUCCESS,
        [pr, comment, ..] if pr == "pr" && comment == "comment" => ExitCode::SUCCESS,
        [pr, checks, _, json_flag, _] if pr == "pr" && checks == "checks" && json_flag == "--json" => {
            println!("[{{\"name\":\"CI\",\"state\":\"COMPLETED\",\"conclusion\":\"SUCCESS\",\"detailsUrl\":\"https://example.test/checks/12\",\"startedAt\":\"2026-04-20T00:00:00Z\",\"completedAt\":\"2026-04-20T00:01:00Z\"}}]");
            ExitCode::SUCCESS
        }
        [run, view, run_id, log_flag] if run == "run" && view == "view" && log_flag == "--log" => {
            println!("run log {run_id}");
            ExitCode::SUCCESS
        }
        [api, endpoint] if api == "api" && endpoint == "repos/akiojin/gwt/pulls/12/reviews" => {
            println!("[{{\"id\":42,\"state\":\"APPROVED\",\"body\":\"Looks good\",\"submitted_at\":\"2026-04-20T02:00:00Z\",\"user\":{{\"login\":\"reviewer\"}}}}]");
            ExitCode::SUCCESS
        }
        [api, endpoint] if api == "api" && endpoint == "/repos/akiojin/gwt/actions/jobs/91/logs" => {
            print!("job log 91");
            ExitCode::SUCCESS
        }
        [api, graphql, ..] if api == "api" && graphql == "graphql" => {
            let joined = args.join("\n");
            if joined.contains("timelineItems") {
                println!(
                    "{}",
                    r#"{"data":{"repository":{"issue":{"timelineItems":{"nodes":[
{"__typename":"CrossReferencedEvent","source":{"__typename":"PullRequest","number":12,"title":"Coverage Gate","state":"OPEN","url":"https://github.com/akiojin/gwt/pull/12"}}
]}}}}}"#
                );
            } else if joined.contains("reviewThreads") {
                println!(
                    "{}",
                    r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[
{"id":"thread-1","isResolved":false,"isOutdated":false,"path":"src/lib.rs","line":10,"comments":{"nodes":[{"id":"comment-1","body":"done","createdAt":"2026-04-20T00:00:00Z","updatedAt":"2026-04-20T00:00:00Z","author":{"login":"reviewer"}}]}}
]}}}}}"#
                );
            } else if joined.contains("addPullRequestReviewThreadReply") {
                println!("{{\"data\":{{\"addPullRequestReviewThreadReply\":{{\"comment\":{{\"id\":\"reply-1\"}}}}}}}}");
            } else if joined.contains("resolveReviewThread") {
                println!("{{\"data\":{{\"resolveReviewThread\":{{\"thread\":{{\"id\":\"thread-1\",\"isResolved\":true}}}}}}}}");
            } else {
                eprintln!("unexpected graphql args: {args:?}");
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unexpected fake gh args: {args:?}");
            ExitCode::from(1)
        }
    }
}
"###;

        let source_path = bin_dir.join("gh.rs");
        fs::write(&source_path, source).expect("write fake gh source");
        let output_path = bin_dir.join(format!("gh{}", env::consts::EXE_SUFFIX));
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&output_path)
            .status()
            .expect("compile fake gh");
        assert!(status.success(), "fake gh compilation failed");
    }

    fn with_fake_gh<T>(test: impl FnOnce(&Path) -> T) -> T {
        let _lock = crate::cli::fake_gh_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("tempdir");
        compile_fake_gh(temp.path());

        let old_path = env::var_os("PATH");
        let joined_path = env::join_paths(
            std::iter::once(PathBuf::from(temp.path()))
                .chain(old_path.iter().flat_map(env::split_paths)),
        )
        .expect("join PATH");
        env::set_var("PATH", joined_path);

        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).expect("create repo");
        let result = test(&repo_path);

        match old_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }

        result
    }

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

    #[test]
    fn test_env_records_io_and_pr_side_effects() {
        let mut env = TestEnv::new(PathBuf::from("cache-root"));
        env.stdin = "from stdin".to_string();
        env.files.insert("input.md".to_string(), "body".to_string());

        assert_eq!(env.read_stdin().expect("stdin"), "from stdin");
        assert_eq!(env.read_stdin().expect("stdin second read"), "");
        assert_eq!(env.read_file("input.md").expect("file"), "body");
        assert_eq!(
            env.read_file("missing.md").expect_err("missing").kind(),
            io::ErrorKind::NotFound
        );

        env.seed_linked_prs(
            42,
            vec![LinkedPrSummary {
                number: 128,
                title: "Coverage".to_string(),
                state: "OPEN".to_string(),
                url: "https://example.test/pr/128".to_string(),
            }],
        );
        assert_eq!(
            env.fetch_linked_prs(IssueNumber(42)).expect("linked").len(),
            1
        );
        assert_eq!(env.linked_pr_calls(), vec![42]);
        env.clear_linked_pr_calls();
        assert!(env.linked_pr_calls().is_empty());

        env.seed_current_pr(Some(sample_pr_status()));
        assert!(env.fetch_current_pr().expect("current").is_some());
        assert_eq!(env.pr_current_call_count, 1);

        env.seed_created_pr(sample_pr_status());
        let created = env
            .create_pr(
                "develop",
                Some("feature/coverage"),
                "Raise coverage",
                "Body",
                &["coverage".to_string()],
                true,
            )
            .expect("create pr");
        assert_eq!(created.number, 128);
        assert_eq!(env.pr_create_call_log.len(), 1);
        assert_eq!(
            env.pr_create_call_log[0].head.as_deref(),
            Some("feature/coverage")
        );
        assert!(env.pr_create_call_log[0].draft);

        env.seed_pr(7, sample_pr_status());
        let edited = env
            .edit_pr(7, Some("Edited"), Some("New body"), &["tested".to_string()])
            .expect("edit pr");
        assert_eq!(edited.number, 128);
        assert_eq!(env.pr_edit_call_log[0].number, 7);
        assert_eq!(env.pr_edit_call_log[0].title.as_deref(), Some("Edited"));
        assert_eq!(env.fetch_pr(7).expect("fetch pr").number, 128);
        assert_eq!(env.pr_view_call_log, vec![7]);

        env.comment_on_pr(7, "looks good").expect("comment");
        assert_eq!(env.pr_comments, vec![(7, "looks good".to_string())]);

        env.seed_pr_reviews(
            7,
            vec![PrReview {
                id: "review-1".to_string(),
                state: "APPROVED".to_string(),
                body: "Ship it".to_string(),
                submitted_at: "2026-04-20T10:00:00Z".to_string(),
                author: "codex".to_string(),
            }],
        );
        assert_eq!(env.fetch_pr_reviews(7).expect("reviews").len(), 1);
        assert_eq!(env.pr_reviews_call_log, vec![7]);

        env.seed_pr_review_threads(
            7,
            vec![
                PrReviewThread {
                    id: "thread-1".to_string(),
                    is_resolved: false,
                    is_outdated: false,
                    path: "src/main.rs".to_string(),
                    line: Some(10),
                    comments: vec![crate::cli::PrReviewThreadComment {
                        id: "comment-1".to_string(),
                        body: "Please add tests".to_string(),
                        created_at: "2026-04-20T10:00:00Z".to_string(),
                        updated_at: "2026-04-20T10:00:00Z".to_string(),
                        author: "reviewer".to_string(),
                    }],
                },
                PrReviewThread {
                    id: "thread-2".to_string(),
                    is_resolved: true,
                    is_outdated: false,
                    path: "src/lib.rs".to_string(),
                    line: Some(20),
                    comments: Vec::new(),
                },
            ],
        );
        assert_eq!(env.fetch_pr_review_threads(7).expect("threads").len(), 2);
        assert_eq!(
            env.reply_and_resolve_pr_review_threads(7, "done")
                .expect("reply"),
            1
        );
        assert_eq!(
            env.pr_reply_and_resolve_call_log,
            vec![(7, "done".to_string())]
        );

        env.seed_pr_checks(
            7,
            PrChecksSummary {
                summary: "All green".to_string(),
                ci_status: "SUCCESS".to_string(),
                merge_status: "CLEAN".to_string(),
                review_status: "APPROVED".to_string(),
                checks: Vec::new(),
            },
        );
        assert_eq!(env.fetch_pr_checks(7).expect("checks").summary, "All green");
        assert_eq!(env.pr_checks_call_log, vec![7]);

        env.seed_run_log(90, "run log");
        env.seed_job_log(91, "job log");
        assert_eq!(env.fetch_actions_run_log(90).expect("run log"), "run log");
        assert_eq!(env.fetch_actions_job_log(91).expect("job log"), "job log");
        assert_eq!(env.run_log_call_log, vec![90]);
        assert_eq!(env.job_log_call_log, vec![91]);

        let output = env
            .run_internal_command(&["gwt".to_string(), "issue".to_string()], "")
            .expect("internal command");
        assert_eq!(output.status, 2);
        assert_eq!(env.internal_command_call_log.len(), 1);
        assert_eq!(
            env.internal_command_call_log[0],
            InternalCommandCall {
                args: vec!["gwt".to_string(), "issue".to_string()],
                stdin: String::new(),
            }
        );
    }

    #[test]
    fn default_cli_env_routes_gh_backed_methods_and_internal_dispatch() {
        with_fake_gh(|repo_path| {
            let cache_root = repo_path.join(".cache");
            let mut env = DefaultCliEnv::new_with_client_factory_and_cache_root(
                "akiojin",
                "gwt",
                repo_path.to_path_buf(),
                cache_root,
                failing_factory(Arc::new(AtomicUsize::new(0))),
            );

            let linked = env.fetch_linked_prs(IssueNumber(42)).expect("linked prs");
            assert_eq!(linked.len(), 1);
            assert_eq!(linked[0].number, 12);

            let current = env
                .fetch_current_pr()
                .expect("current pr")
                .expect("current pr exists");
            assert_eq!(current.number, 12);

            let created = env
                .create_pr(
                    "develop",
                    Some("feature/coverage"),
                    "Raise coverage",
                    "Body",
                    &["coverage".to_string()],
                    true,
                )
                .expect("create pr");
            assert_eq!(created.number, 12);

            let edited = env
                .edit_pr(12, Some("Edited"), Some("Updated"), &["tested".to_string()])
                .expect("edit pr");
            assert_eq!(edited.number, 12);

            let fetched = env.fetch_pr(12).expect("fetch pr");
            assert_eq!(fetched.number, 12);

            env.comment_on_pr(12, "done").expect("comment");

            let reviews = env.fetch_pr_reviews(12).expect("reviews");
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0].author, "reviewer");

            let threads = env.fetch_pr_review_threads(12).expect("threads");
            assert_eq!(threads.len(), 1);
            assert_eq!(threads[0].path, "src/lib.rs");

            assert_eq!(
                env.reply_and_resolve_pr_review_threads(12, "done")
                    .expect("reply and resolve"),
                1
            );

            let checks = env.fetch_pr_checks(12).expect("checks");
            assert_eq!(checks.checks.len(), 1);
            assert_eq!(checks.checks[0].conclusion, "SUCCESS");

            assert_eq!(
                env.fetch_actions_run_log(90).expect("run log").trim(),
                "run log 90"
            );
            assert_eq!(
                env.fetch_actions_job_log(91).expect("job log"),
                "job log 91"
            );

            let note_path = repo_path.join("note.md");
            fs::write(&note_path, "hello").expect("write note");
            assert_eq!(
                env.read_file(note_path.to_str().expect("utf8 path"))
                    .expect("read file"),
                "hello"
            );

            // `DefaultCliEnv` spawns `current_exe()`, which is the Rust test harness in unit tests.
            // Use `--help` so the spawned binary exits successfully regardless of whether it is the
            // real CLI binary or the harness wrapper.
            let output = env
                .run_internal_command(&["gwt".to_string(), "--help".to_string()], "")
                .expect("internal dispatch");
            assert_eq!(output.status, 0);
        });
    }

    #[test]
    fn client_ref_forwards_issue_client_methods_to_the_underlying_fake_client() {
        let client = FakeIssueClient::new();
        client.seed(sample_issue_snapshot(7, &["gwt-spec", "phase/in-progress"]));
        let client_ref = ClientRef { inner: &client };

        assert!(matches!(
            client_ref.fetch(IssueNumber(7), None).expect("fetch"),
            gwt_github::client::FetchResult::Updated(_)
        ));
        client_ref
            .patch_body(IssueNumber(7), "Updated body")
            .expect("patch body");
        client_ref
            .patch_title(IssueNumber(7), "Updated title")
            .expect("patch title");
        client_ref
            .patch_comment(gwt_github::client::CommentId(9), "Updated comment")
            .expect("patch comment");
        client_ref
            .create_comment(IssueNumber(7), "Another comment")
            .expect("create comment");

        let created = client_ref
            .create_issue("New issue", "Body", &["bug".to_string()])
            .expect("create issue");
        client_ref
            .set_labels(created.number, &["chore".to_string()])
            .expect("set labels");
        client_ref
            .set_state(created.number, gwt_github::IssueState::Closed)
            .expect("set state");

        let specs = client_ref
            .list_spec_issues(&SpecListFilter::default())
            .expect("list spec issues");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].number, IssueNumber(7));

        let call_log = client.call_log();
        for expected in [
            "fetch:#7",
            "patch_body:#7",
            "patch_title:#7",
            "patch_comment:comment:9",
            "create_comment:#7",
            "create_issue:#8",
            "set_labels:#8",
            "set_state:#8",
            "list_spec_issues:*",
        ] {
            assert!(
                call_log.iter().any(|entry| entry == expected),
                "missing forwarded call {expected:?} in {call_log:?}"
            );
        }
    }
}

//! `cli::env` family of CLI runtime environments (SPEC-1942 SC-027 split).
//!
//! - `mod.rs` (this file): public [`CliEnv`] trait, helper types
//!   ([`InternalCommandOutput`] / [`InternalCommandCall`] / [`ClientRef`]),
//!   and the global [`dispatch`] entry point.
//! - `default.rs`: production [`default::DefaultCliEnv`] +
//!   [`default::LazyIssueClient`].
//! - `test_env.rs`: in-memory [`test_env::TestEnv`] used by `cli` family
//!   tests.
//! - `tests.rs`: env integration tests (`#[cfg(test)]`).

mod default;
mod test_env;
#[cfg(test)]
mod tests;

pub use default::DefaultCliEnv;
#[cfg(test)]
pub use default::{IssueClientFactory, LazyIssueClient};
pub use test_env::TestEnv;

use std::{
    io::{self},
    path::PathBuf,
};

use gwt_git::PrStatus;
use gwt_github::{client::IssueClient, IssueNumber, SpecListFilter};

use super::{
    parse_actions_args, parse_board_args, parse_hook_args, parse_issue_args, parse_pr_args, run,
    CliParseError, LinkedPrSummary, PrChecksSummary, PrReview, PrReviewThread,
};

/// High-level runtime environment for the CLI. Kept as a trait so tests can
/// inject a fake `IssueClient` (see `gwt-github`'s
/// `client::fake::FakeIssueClient`) instead of spinning up real HTTP.
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

pub struct ClientRef<'a, C: IssueClient> {
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
        "index" => super::parse_index_args(&rest),
        "hook" => parse_hook_args(&rest),
        "discuss" => super::parse_discuss_args(&rest),
        "plan" => super::parse_plan_args(&rest),
        "build" => super::parse_build_args(&rest),
        "daemon" => super::parse_daemon_args(&rest),
        "update" => {
            let mode = if rest.iter().any(|a| a == "--check") {
                super::UpdateCommand::CheckOnly
            } else {
                super::UpdateCommand::Apply
            };
            Ok(super::CliCommand::Update(mode))
        }
        "__internal" => match rest.first().map(String::as_str) {
            Some("apply-update") => Ok(super::CliCommand::Update(
                super::UpdateCommand::InternalApply {
                    rest: rest[1..].to_vec(),
                },
            )),
            Some("run-installer") => Ok(super::CliCommand::Update(
                super::UpdateCommand::InternalRunInstaller {
                    rest: rest[1..].to_vec(),
                },
            )),
            Some("daemon-hook") => parse_hook_args(&rest[1..]).map(|cmd| match cmd {
                super::CliCommand::Hook(super::HookCommand::Run { name, rest }) => {
                    super::CliCommand::Hook(super::HookCommand::InternalDaemon { name, rest })
                }
                _ => unreachable!("parse_hook_args must return CliCommand::Hook(Run {{..}})"),
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

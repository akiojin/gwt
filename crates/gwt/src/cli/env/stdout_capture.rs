use std::io;

use gwt_git::PrStatus;
use gwt_github::{IssueNumber, IssueSnapshot};

use super::{CliEnv, InternalCommandOutput};
use crate::cli::{LinkedPrSummary, PrChecksSummary, PrReview, PrReviewThread};

pub(crate) struct StdoutCaptureEnv<'a, E: CliEnv> {
    pub(crate) inner: &'a mut E,
    pub(crate) stdout: &'a mut Vec<u8>,
}

impl<E: CliEnv> CliEnv for StdoutCaptureEnv<'_, E> {
    type Client = E::Client;

    fn client(&self) -> &Self::Client {
        self.inner.client()
    }

    fn cache_root(&self) -> std::path::PathBuf {
        self.inner.cache_root()
    }

    fn repo_path(&self) -> &std::path::Path {
        self.inner.repo_path()
    }

    fn stdout(&mut self) -> &mut dyn io::Write {
        self.stdout
    }

    fn stderr(&mut self) -> &mut dyn io::Write {
        self.inner.stderr()
    }

    fn read_stdin(&mut self) -> io::Result<String> {
        self.inner.read_stdin()
    }

    fn read_file(&self, path: &str) -> io::Result<String> {
        self.inner.read_file(path)
    }

    fn create_issue_in_repo(
        &mut self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> io::Result<IssueSnapshot> {
        self.inner
            .create_issue_in_repo(owner, repo, title, body, labels)
    }

    fn fetch_linked_prs(&mut self, number: IssueNumber) -> io::Result<Vec<LinkedPrSummary>> {
        self.inner.fetch_linked_prs(number)
    }

    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>> {
        self.inner.fetch_current_pr()
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
        self.inner.create_pr(base, head, title, body, labels, draft)
    }

    fn edit_pr(
        &mut self,
        number: u64,
        title: Option<&str>,
        body: Option<&str>,
        add_labels: &[String],
    ) -> io::Result<PrStatus> {
        self.inner.edit_pr(number, title, body, add_labels)
    }

    fn fetch_pr(&mut self, number: u64) -> io::Result<PrStatus> {
        self.inner.fetch_pr(number)
    }

    fn mark_pr_ready(&mut self, number: u64) -> io::Result<PrStatus> {
        self.inner.mark_pr_ready(number)
    }

    fn convert_pr_to_draft(&mut self, number: u64) -> io::Result<PrStatus> {
        self.inner.convert_pr_to_draft(number)
    }

    fn comment_on_pr(&mut self, number: u64, body: &str) -> io::Result<()> {
        self.inner.comment_on_pr(number, body)
    }

    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>> {
        self.inner.fetch_pr_reviews(number)
    }

    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>> {
        self.inner.fetch_pr_review_threads(number)
    }

    fn reply_and_resolve_pr_review_threads(
        &mut self,
        number: u64,
        body: &str,
    ) -> io::Result<usize> {
        self.inner.reply_and_resolve_pr_review_threads(number, body)
    }

    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary> {
        self.inner.fetch_pr_checks(number)
    }

    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String> {
        self.inner.fetch_actions_run_log(run_id)
    }

    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String> {
        self.inner.fetch_actions_job_log(job_id)
    }

    fn run_internal_command(
        &mut self,
        args: &[String],
        stdin: &str,
    ) -> io::Result<InternalCommandOutput> {
        self.inner.run_internal_command(args, stdin)
    }
}

//! Supplemental Nightly proof of authenticated reads, never feedback delivery.

use std::{process::ExitCode, time::Duration};

use gwt_github::{
    client::{
        http::HttpIssueClient, OwnerRepositoryClient, RepositoryIdentity, ResolutionDeadline,
    },
    IssueNumber,
};

fn main() -> ExitCode {
    let deadline = ResolutionDeadline::new(Duration::from_secs(5), Duration::from_secs(30));
    let success = match HttpIssueClient::from_noninteractive_env("akiojin", "gwt") {
        Ok(client) => client
            // The existing fixed query uses HTTP POST /graphql. It has no mutation.
            .fetch_issue(
                &RepositoryIdentity::gwt_upstream(),
                IssueNumber(6576),
                &deadline,
            )
            .is_ok_and(|issue| issue.number == IssueNumber(6576)),
        Err(_) => false,
    };
    // Never format the token, response, or error, even on failure.
    println!("{success}");
    if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

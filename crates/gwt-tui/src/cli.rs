//! CLI dispatch for `gwt issue spec ...` subcommands.
//!
//! SPEC-12 Phase 6: when the gwt binary is invoked with arguments starting
//! with `issue`, we treat it as a CLI call rather than a TUI launch. This
//! module owns argv parsing, dispatches to the high-level SPEC operations in
//! `gwt-github`, and writes the result to stdout/stderr.
//!
//! Supported commands:
//!
//! - `gwt issue spec <n>` — print every section for an issue
//! - `gwt issue spec <n> --section <name>` — print one section only
//! - `gwt issue spec <n> --edit <name> -f <file>` — replace one section
//!   from a file (`-` means stdin)
//! - `gwt issue spec list [--phase <name>] [--state open|closed]` —
//!   list SPEC-labeled issues
//!
//! Missing (deferred to next cycle): `pull`, `create`, `repair`,
//! `migrate-specs`.

pub mod hook;

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;

use gwt_git::PrStatus;
use gwt_github::client::fake::FakeIssueClient;
use gwt_github::client::http::HttpIssueClient;
use gwt_github::client::IssueClient;
use gwt_github::{
    cache::write_atomic, ApiError, Cache, IssueNumber, IssueSnapshot, IssueState, SectionName,
    SpecListFilter, SpecOps, SpecOpsError,
};

/// Compact linked PR summary used by `gwt issue linked-prs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedPrSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
}

/// Compact PR check entry used by `gwt pr checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrCheckItem {
    pub name: String,
    pub state: String,
    pub conclusion: String,
    pub url: String,
    pub started_at: String,
    pub completed_at: String,
    pub workflow: String,
}

/// Render-friendly aggregate used by `gwt pr checks`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrChecksSummary {
    pub summary: String,
    pub ci_status: String,
    pub merge_status: String,
    pub review_status: String,
    pub checks: Vec<PrCheckItem>,
}

/// PR review summary used by `gwt pr reviews`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReview {
    pub id: String,
    pub state: String,
    pub body: String,
    pub submitted_at: String,
    pub author: String,
}

/// Single comment inside a review thread.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThreadComment {
    pub id: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub author: String,
}

/// Review thread snapshot used by `gwt pr review-threads`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub comments: Vec<PrReviewThreadComment>,
}

/// Test-visible log entry for `gwt pr create`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCreateCall {
    pub base: String,
    pub head: Option<String>,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// Test-visible log entry for `gwt pr edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrEditCall {
    pub number: u64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub add_labels: Vec<String>,
}

/// Top-level argv parse result for the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    /// `gwt issue spec <n>` — print all sections.
    SpecReadAll { number: u64 },
    /// `gwt issue spec <n> --section <name>` — print a single section.
    SpecReadSection { number: u64, section: String },
    /// `gwt issue spec <n> --edit <name> -f <file>` — replace a section.
    SpecEditSection {
        number: u64,
        section: String,
        file: String,
    },
    /// `gwt issue spec list [--phase <name>] [--state open|closed]`.
    SpecList {
        phase: Option<String>,
        state: Option<String>,
    },
    /// `gwt issue spec create --title <t> -f <body_file> [--label <l>]*`.
    SpecCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwt issue spec pull [--all | <n>...]` — refresh cache from server.
    SpecPull { all: bool, numbers: Vec<u64> },
    /// `gwt issue spec repair <n>` — clear cache and re-fetch from server.
    SpecRepair { number: u64 },
    /// `gwt issue view <n> [--refresh]` — print a plain issue from cache/live.
    IssueView { number: u64, refresh: bool },
    /// `gwt issue comments <n> [--refresh]` — print issue comments.
    IssueComments { number: u64, refresh: bool },
    /// `gwt issue linked-prs <n> [--refresh]` — print linked PR summaries.
    IssueLinkedPrs { number: u64, refresh: bool },
    /// `gwt issue create --title <t> -f <body_file> [--label <l>]*`.
    IssueCreate {
        title: String,
        file: String,
        labels: Vec<String>,
    },
    /// `gwt issue comment <n> -f <body_file>` — create a plain issue comment.
    IssueComment { number: u64, file: String },
    /// `gwt pr current` — print the PR associated with the current branch.
    PrCurrent,
    /// `gwt pr create --base <branch> [--head <branch>] --title <t> -f <body_file>`.
    PrCreate {
        base: String,
        head: Option<String>,
        title: String,
        file: String,
        labels: Vec<String>,
        draft: bool,
    },
    /// `gwt pr edit <n> [--title <t>] [-f <body_file>] [--add-label <label>]*`.
    PrEdit {
        number: u64,
        title: Option<String>,
        file: Option<String>,
        add_labels: Vec<String>,
    },
    /// `gwt pr view <n>` — print a PR by number.
    PrView { number: u64 },
    /// `gwt pr comment <n> -f <body_file>` — create a PR issue comment.
    PrComment { number: u64, file: String },
    /// `gwt pr reviews <n>` — print PR review summaries.
    PrReviews { number: u64 },
    /// `gwt pr review-threads <n>` — print review thread snapshots.
    PrReviewThreads { number: u64 },
    /// `gwt pr review-threads reply-and-resolve <n> -f <body_file>`.
    PrReviewThreadsReplyAndResolve { number: u64, file: String },
    /// `gwt pr checks <n>` — print PR checks and summary.
    PrChecks { number: u64 },
    /// `gwt actions logs --run <id>` — print raw GitHub Actions run logs.
    ActionsLogs { run_id: u64 },
    /// `gwt actions job-logs --job <id>` — print raw GitHub Actions job logs.
    ActionsJobLogs { job_id: u64 },
    /// `gwt hook <name> [args...]` — dispatch to an in-binary hook handler.
    ///
    /// See SPEC #1942 (CORE-CLI) — replaces `.claude/hooks/scripts/gwt-*.mjs`
    /// and inline shell hooks in `.claude/settings.local.json`.
    Hook { name: String, rest: Vec<String> },
}

/// Errors surfaced by argv parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliParseError {
    Usage,
    InvalidNumber(String),
    MissingFlag(&'static str),
    UnknownSubcommand(String),
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliParseError::Usage => write!(
                f,
                "usage: gwt issue spec <n> [--section <name>|--edit <name> -f <file>] | gwt issue spec list [--phase <p>] [--state open|closed] | gwt issue view|comments|linked-prs <n> [--refresh] | gwt issue create --title <t> -f <file> [--label <l>]* | gwt issue comment <n> -f <file> | gwt pr current|create --base <b> [--head <h>] --title <t> -f <file> [--label <l>]* [--draft]|edit <n> [--title <t>] [-f <file>] [--add-label <l>]*|view <n>|comment <n> -f <file>|reviews <n>|review-threads <n>|review-threads reply-and-resolve <n> -f <file>|checks <n> | gwt actions logs --run <id> | gwt actions job-logs --job <id>"
            ),
            CliParseError::InvalidNumber(s) => write!(f, "invalid issue number: {s}"),
            CliParseError::MissingFlag(flag) => write!(f, "missing required flag: {flag}"),
            CliParseError::UnknownSubcommand(s) => write!(f, "unknown subcommand: {s}"),
        }
    }
}

impl std::error::Error for CliParseError {}

/// Determine whether the given argv (starting at the program name) should be
/// handled as a CLI invocation. Returns `true` when argv[1..] begins with
/// `issue`, `pr`, `actions`, or `hook`. The TUI launcher keeps its legacy
/// behaviour (positional repo path) for any other shape.
pub fn should_dispatch_cli(args: &[String]) -> bool {
    args.get(1)
        .map(|s| matches!(s.as_str(), "issue" | "pr" | "actions" | "hook"))
        .unwrap_or(false)
}

/// Parse an argv slice into a [`CliCommand`]. The slice should start from
/// the first post-subcommand argument — i.e. if the caller received
/// `["gwt", "issue", "spec", "2001"]`, they pass `["spec", "2001"]`.
pub fn parse_issue_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("spec") => parse_spec_args(it.collect::<Vec<_>>().as_slice()),
        Some("view") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "view"),
        Some("comments") => parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "comments"),
        Some("linked-prs") => {
            parse_issue_read_args(it.collect::<Vec<_>>().as_slice(), "linked-prs")
        }
        Some("create") => parse_issue_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("comment") => parse_issue_comment_args(it.collect::<Vec<_>>().as_slice()),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

/// Parse an argv slice into a `gwt pr ...` [`CliCommand`].
pub fn parse_pr_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("current") if it.peek().is_none() => Ok(CliCommand::PrCurrent),
        Some("create") => parse_pr_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("edit") => parse_pr_edit_args(it.collect::<Vec<_>>().as_slice()),
        Some("view") => Ok(CliCommand::PrView {
            number: parse_required_number(it.next())?,
        }),
        Some("comment") => {
            let number = parse_required_number(it.next())?;
            expect_flag(it.next(), "-f")?;
            Ok(CliCommand::PrComment {
                number,
                file: it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone(),
            })
        }
        Some("reviews") => Ok(CliCommand::PrReviews {
            number: parse_required_number(it.next())?,
        }),
        Some("review-threads") => match it.next().map(String::as_str) {
            Some("reply-and-resolve") => {
                let number = parse_required_number(it.next())?;
                expect_flag(it.next(), "-f")?;
                Ok(CliCommand::PrReviewThreadsReplyAndResolve {
                    number,
                    file: it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone(),
                })
            }
            Some(number_arg) => Ok(CliCommand::PrReviewThreads {
                number: number_arg
                    .parse()
                    .map_err(|_| CliParseError::InvalidNumber(number_arg.to_string()))?,
            }),
            None => Err(CliParseError::Usage),
        },
        Some("checks") => Ok(CliCommand::PrChecks {
            number: parse_required_number(it.next())?,
        }),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

/// Parse an argv slice into a `gwt actions ...` [`CliCommand`].
pub fn parse_actions_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("logs") => {
            expect_flag(it.next(), "--run")?;
            Ok(CliCommand::ActionsLogs {
                run_id: parse_required_number(it.next())?,
            })
        }
        Some("job-logs") => {
            expect_flag(it.next(), "--job")?;
            Ok(CliCommand::ActionsJobLogs {
                job_id: parse_required_number(it.next())?,
            })
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

fn expect_flag(arg: Option<&String>, expected: &'static str) -> Result<(), CliParseError> {
    match arg.map(String::as_str) {
        Some(flag) if flag == expected => Ok(()),
        Some(flag) => Err(CliParseError::UnknownSubcommand(flag.to_string())),
        None => Err(CliParseError::MissingFlag(expected)),
    }
}

fn parse_required_number(arg: Option<&String>) -> Result<u64, CliParseError> {
    let value = arg.ok_or(CliParseError::Usage)?;
    value
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(value.clone()))
}

fn parse_pr_create_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    let mut base: Option<String> = None;
    let mut head: Option<String> = None;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut draft = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--base" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--base"));
                }
                base = Some(args[i].clone());
            }
            "--head" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--head"));
                }
                head = Some(args[i].clone());
            }
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            "--draft" => draft = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    Ok(CliCommand::PrCreate {
        base: base.ok_or(CliParseError::MissingFlag("--base"))?,
        head,
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
        draft,
    })
}

fn parse_pr_edit_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut add_labels: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--add-label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--add-label"));
                }
                add_labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    if title.is_none() && file.is_none() && add_labels.is_empty() {
        return Err(CliParseError::Usage);
    }

    Ok(CliCommand::PrEdit {
        number,
        title,
        file,
        add_labels,
    })
}

/// Parse the tail of a `gwt hook ...` argv slice into a [`CliCommand::Hook`].
///
/// SPEC #1942 (CORE-CLI): `gwt hook <name> [args...]` is the single entry
/// point for every in-binary hook handler. The known hook names are:
///
/// - `runtime-state <event>`
/// - `block-bash-policy`
/// - `forward <target>`
///
/// Unknown names still parse (we don't maintain an allowlist here) so that
/// newly added hooks don't need parser changes. Validation happens in
/// [`run_hook`].
pub fn parse_hook_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    Ok(CliCommand::Hook {
        name: head.clone(),
        rest: rest.to_vec(),
    })
}

fn parse_spec_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    if args.is_empty() {
        return Err(CliParseError::Usage);
    }
    // First argument is either a bare number (read/edit) or the literal
    // `list`.
    let head = args[0].as_str();

    if head == "list" {
        let mut phase: Option<String> = None;
        let mut state: Option<String> = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--phase" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--phase"));
                    }
                    phase = Some(args[i].clone());
                }
                "--state" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--state"));
                    }
                    state = Some(args[i].clone());
                }
                other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
            }
            i += 1;
        }
        return Ok(CliCommand::SpecList { phase, state });
    }

    if head == "create" {
        let mut title: Option<String> = None;
        let mut file: Option<String> = None;
        let mut labels: Vec<String> = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--title" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--title"));
                    }
                    title = Some(args[i].clone());
                }
                "-f" | "--file" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("-f"));
                    }
                    file = Some(args[i].clone());
                }
                "--label" => {
                    i += 1;
                    if i >= args.len() {
                        return Err(CliParseError::MissingFlag("--label"));
                    }
                    labels.push(args[i].clone());
                }
                other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
            }
            i += 1;
        }
        let title = title.ok_or(CliParseError::MissingFlag("--title"))?;
        let file = file.ok_or(CliParseError::MissingFlag("-f"))?;
        return Ok(CliCommand::SpecCreate {
            title,
            file,
            labels,
        });
    }

    if head == "pull" {
        let mut all = false;
        let mut numbers: Vec<u64> = Vec::new();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--all" => all = true,
                other => {
                    let n: u64 = other
                        .parse()
                        .map_err(|_| CliParseError::InvalidNumber(other.to_string()))?;
                    numbers.push(n);
                }
            }
            i += 1;
        }
        return Ok(CliCommand::SpecPull { all, numbers });
    }

    if head == "repair" {
        if args.len() < 2 {
            return Err(CliParseError::Usage);
        }
        let number: u64 = args[1]
            .parse()
            .map_err(|_| CliParseError::InvalidNumber(args[1].clone()))?;
        return Ok(CliCommand::SpecRepair { number });
    }

    // Read / edit path.
    let number: u64 = head
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(head.to_string()))?;

    let mut section: Option<String> = None;
    let mut edit_section: Option<String> = None;
    let mut file: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--section" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--section"));
                }
                section = Some(args[i].clone());
            }
            "--edit" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--edit"));
                }
                edit_section = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    if let Some(edit) = edit_section {
        let file = file.ok_or(CliParseError::MissingFlag("-f"))?;
        return Ok(CliCommand::SpecEditSection {
            number,
            section: edit,
            file,
        });
    }
    if let Some(s) = section {
        return Ok(CliCommand::SpecReadSection { number, section: s });
    }
    Ok(CliCommand::SpecReadAll { number })
}

fn parse_issue_read_args(args: &[&String], mode: &str) -> Result<CliCommand, CliParseError> {
    let Some(number_arg) = args.first() else {
        return Err(CliParseError::Usage);
    };
    let number = number_arg
        .parse()
        .map_err(|_| CliParseError::InvalidNumber((*number_arg).clone()))?;

    let mut refresh = false;
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--refresh" => refresh = true,
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
    }

    Ok(match mode {
        "view" => CliCommand::IssueView { number, refresh },
        "comments" => CliCommand::IssueComments { number, refresh },
        "linked-prs" => CliCommand::IssueLinkedPrs { number, refresh },
        _ => return Err(CliParseError::Usage),
    })
}

fn parse_issue_create_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    let mut title: Option<String> = None;
    let mut file: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--label"));
                }
                labels.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    Ok(CliCommand::IssueCreate {
        title: title.ok_or(CliParseError::MissingFlag("--title"))?,
        file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        labels,
    })
}

fn parse_issue_comment_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    if args.len() < 3 {
        return Err(CliParseError::Usage);
    }
    let number = args[0]
        .parse()
        .map_err(|_| CliParseError::InvalidNumber(args[0].clone()))?;
    match args[1].as_str() {
        "-f" | "--file" => Ok(CliCommand::IssueComment {
            number,
            file: args[2].clone(),
        }),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

/// High-level runtime environment for the CLI. Kept as a trait so tests can
/// inject a [`FakeIssueClient`] instead of spinning up real HTTP.
pub trait CliEnv {
    type Client: IssueClient;
    fn client(&self) -> &Self::Client;
    fn cache_root(&self) -> PathBuf;
    fn stdout(&mut self) -> &mut dyn io::Write;
    fn stderr(&mut self) -> &mut dyn io::Write;
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

/// Dispatch a parsed [`CliCommand`] against the given [`CliEnv`].
///
/// We collect output into a String buffer first so the [`SpecOps`] borrow of
/// `env.client()` does not conflict with the mutable borrow required by
/// `env.stdout()` at write time.
pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand) -> Result<i32, SpecOpsError> {
    let mut out = String::new();
    let code = match cmd {
        CliCommand::SpecReadAll { number } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let section_names = [
                "spec",
                "tasks",
                "plan",
                "research",
                "data-model",
                "quickstart",
                "tdd",
            ];
            for name in section_names {
                match ops.read_section(IssueNumber(number), &SectionName(name.to_string())) {
                    Ok(content) => {
                        out.push_str(&format!("=== {name} ===\n{content}\n"));
                    }
                    Err(SpecOpsError::SectionNotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            0
        }
        CliCommand::SpecReadSection { number, section } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let content = ops.read_section(IssueNumber(number), &SectionName(section.clone()))?;
            out.push_str(&format!("{content}\n"));
            0
        }
        CliCommand::SpecEditSection {
            number,
            section,
            file,
        } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let content = if file == "-" {
                let mut s = String::new();
                io::stdin().read_to_string(&mut s).map_err(|e| {
                    SpecOpsError::from(gwt_github::client::ApiError::Network(e.to_string()))
                })?;
                s
            } else {
                env.read_file(&file).map_err(|e| {
                    SpecOpsError::from(gwt_github::client::ApiError::Network(e.to_string()))
                })?
            };
            ops.write_section(IssueNumber(number), &SectionName(section.clone()), &content)?;
            out.push_str(&format!(
                "wrote {} bytes to section '{section}'\n",
                content.len()
            ));
            0
        }
        CliCommand::SpecList { phase, state } => {
            let filter = SpecListFilter {
                phase,
                state: state.as_deref().and_then(|s| match s {
                    "open" => Some(gwt_github::client::IssueState::Open),
                    "closed" => Some(gwt_github::client::IssueState::Closed),
                    _ => None,
                }),
            };
            let list = env.client().list_spec_issues(&filter)?;
            for s in list {
                let state_marker = match s.state {
                    gwt_github::client::IssueState::Open => "OPEN",
                    gwt_github::client::IssueState::Closed => "CLOSED",
                };
                let phase_label = s
                    .labels
                    .iter()
                    .find(|l| l.starts_with("phase/"))
                    .cloned()
                    .unwrap_or_default();
                out.push_str(&format!(
                    "#{} [{state_marker}] [{phase_label}] {}\n",
                    s.number.0, s.title
                ));
            }
            0
        }
        CliCommand::SpecCreate {
            title,
            file,
            labels,
        } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            let raw = env.read_file(&file).map_err(|e| {
                SpecOpsError::from(gwt_github::client::ApiError::Network(e.to_string()))
            })?;
            // Parse section markers from the supplied body file and map
            // them into the SpecOps section map.
            let parsed = gwt_github::extract_sections(&raw)
                .map_err(|e| SpecOpsError::from(gwt_github::body::ParseError::Section(e)))?;
            let sections: std::collections::BTreeMap<SectionName, String> =
                parsed.into_iter().map(|s| (s.name, s.content)).collect();
            let snapshot = ops.create_spec(&title, sections, &labels)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        CliCommand::SpecPull { all, numbers } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            if all {
                // Ask the server for every gwt-spec issue, then refresh
                // each one in the cache.
                let list = env.client().list_spec_issues(&SpecListFilter::default())?;
                for summary in list {
                    ops.read_section(summary.number, &SectionName("spec".to_string()))
                        .ok(); // Ignore individual errors; we just want to refresh cache.
                }
                out.push_str("pulled all gwt-spec issues\n");
            } else if numbers.is_empty() {
                return Err(SpecOpsError::SectionNotFound(
                    "pull requires --all or <n>".into(),
                ));
            } else {
                for n in numbers {
                    // Propagate fetch/auth/not-found errors instead
                    // of swallowing them with `.ok()` — a silent
                    // pull that still prints "pulled #N" on failure
                    // is worse than no pull at all.
                    ops.read_section(IssueNumber(n), &SectionName("spec".to_string()))?;
                    out.push_str(&format!("pulled #{n}\n"));
                }
            }
            0
        }
        CliCommand::SpecRepair { number } => {
            let cache = Cache::new(env.cache_root());
            let ops = SpecOps::new(
                ClientRef {
                    inner: env.client(),
                },
                cache,
            );
            // read_section runs a fresh fetch and rewrites the
            // cache — that is the repair operation for the MVP.
            // Surface any error (fetch/parse/auth/not-found) to the
            // caller so a broken cache does not hide behind a
            // misleading "repaired cache for #N" success line.
            ops.read_section(IssueNumber(number), &SectionName("spec".to_string()))?;
            out.push_str(&format!("repaired cache for #{number}\n"));
            0
        }
        CliCommand::IssueView { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue(&mut out, &entry.snapshot);
            0
        }
        CliCommand::IssueComments { number, refresh } => {
            let entry = load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            render_issue_comments(&mut out, &entry.snapshot);
            0
        }
        CliCommand::IssueLinkedPrs { number, refresh } => {
            let linked_prs = load_or_refresh_linked_prs(env, IssueNumber(number), refresh)?;
            render_linked_prs(&mut out, &linked_prs);
            0
        }
        CliCommand::IssueCreate {
            title,
            file,
            labels,
        } => {
            let body = env.read_file(&file).map_err(io_as_api_error)?;
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        CliCommand::IssueComment { number, file } => {
            let body = env.read_file(&file).map_err(io_as_api_error)?;
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            let _ = refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        CliCommand::PrCurrent => {
            match env.fetch_current_pr().map_err(io_as_api_error)? {
                Some(pr) => render_pr(&mut out, &pr),
                None => out.push_str("no current pull request\n"),
            }
            0
        }
        CliCommand::PrCreate {
            base,
            head,
            title,
            file,
            labels,
            draft,
        } => {
            let body = env.read_file(&file).map_err(io_as_api_error)?;
            let pr = env
                .create_pr(&base, head.as_deref(), &title, &body, &labels, draft)
                .map_err(io_as_api_error)?;
            out.push_str("created pull request\n");
            render_pr(&mut out, &pr);
            0
        }
        CliCommand::PrEdit {
            number,
            title,
            file,
            add_labels,
        } => {
            let body = file
                .as_deref()
                .map(|path| env.read_file(path).map_err(io_as_api_error))
                .transpose()?;
            let pr = env
                .edit_pr(number, title.as_deref(), body.as_deref(), &add_labels)
                .map_err(io_as_api_error)?;
            out.push_str("updated pull request\n");
            render_pr(&mut out, &pr);
            0
        }
        CliCommand::PrView { number } => {
            let pr = env.fetch_pr(number).map_err(io_as_api_error)?;
            render_pr(&mut out, &pr);
            0
        }
        CliCommand::PrComment { number, file } => {
            let body = env.read_file(&file).map_err(io_as_api_error)?;
            env.comment_on_pr(number, &body).map_err(io_as_api_error)?;
            out.push_str(&format!("created comment on PR #{number}\n"));
            0
        }
        CliCommand::PrReviews { number } => {
            let reviews = env.fetch_pr_reviews(number).map_err(io_as_api_error)?;
            render_pr_reviews(&mut out, &reviews);
            0
        }
        CliCommand::PrReviewThreads { number } => {
            let threads = env
                .fetch_pr_review_threads(number)
                .map_err(io_as_api_error)?;
            render_pr_review_threads(&mut out, &threads);
            0
        }
        CliCommand::PrReviewThreadsReplyAndResolve { number, file } => {
            let body = env.read_file(&file).map_err(io_as_api_error)?;
            let resolved = env
                .reply_and_resolve_pr_review_threads(number, &body)
                .map_err(io_as_api_error)?;
            out.push_str(&format!(
                "replied to and resolved {resolved} review threads on PR #{number}\n"
            ));
            0
        }
        CliCommand::PrChecks { number } => {
            let report = env.fetch_pr_checks(number).map_err(io_as_api_error)?;
            render_pr_checks(&mut out, &report);
            0
        }
        CliCommand::ActionsLogs { run_id } => {
            let log = env.fetch_actions_run_log(run_id).map_err(io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        CliCommand::ActionsJobLogs { job_id } => {
            let log = env.fetch_actions_job_log(job_id).map_err(io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        CliCommand::Hook { name, rest } => {
            return run_hook(env, &name, &rest);
        }
    };
    let _ = env.stdout().write_all(out.as_bytes());
    Ok(code)
}

fn io_as_api_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(err.to_string()))
}

fn issue_state_label(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => "OPEN",
        IssueState::Closed => "CLOSED",
    }
}

fn render_issue(out: &mut String, snapshot: &IssueSnapshot) {
    out.push_str(&format!(
        "#{} [{}] {}\n",
        snapshot.number.0,
        issue_state_label(snapshot.state),
        snapshot.title
    ));
    if !snapshot.labels.is_empty() {
        out.push_str(&format!("labels: {}\n", snapshot.labels.join(", ")));
    }
    out.push_str(&format!("updated_at: {}\n\n", snapshot.updated_at.0));
    if !snapshot.body.is_empty() {
        out.push_str(snapshot.body.trim_end_matches('\n'));
        out.push('\n');
    }
}

fn render_issue_comments(out: &mut String, snapshot: &IssueSnapshot) {
    if snapshot.comments.is_empty() {
        out.push_str("no comments\n");
        return;
    }
    for comment in &snapshot.comments {
        out.push_str(&format!(
            "=== comment:{} ({}) ===\n{}\n",
            comment.id.0, comment.updated_at.0, comment.body
        ));
    }
}

fn render_linked_prs(out: &mut String, linked_prs: &[LinkedPrSummary]) {
    if linked_prs.is_empty() {
        out.push_str("no linked pull requests\n");
        return;
    }
    for pr in linked_prs {
        out.push_str(&format!(
            "#{} [{}] {}\n{}\n",
            pr.number, pr.state, pr.title, pr.url
        ));
    }
}

fn render_pr(out: &mut String, pr: &PrStatus) {
    out.push_str(&format!("#{} [{}] {}\n", pr.number, pr.state, pr.title));
    out.push_str(&format!("url: {}\n", pr.url));
    out.push_str(&format!("ci: {}\n", pr.ci_status));
    out.push_str(&format!("mergeable: {}\n", pr.mergeable));
    out.push_str(&format!("review: {}\n", pr.review_status));
}

fn render_pr_checks(out: &mut String, summary: &PrChecksSummary) {
    out.push_str(&format!("summary: {}\n", summary.summary));
    out.push_str(&format!("ci: {}\n", summary.ci_status));
    out.push_str(&format!("merge: {}\n", summary.merge_status));
    out.push_str(&format!("review: {}\n", summary.review_status));
    if summary.checks.is_empty() {
        out.push_str("no checks\n");
        return;
    }
    for check in &summary.checks {
        out.push_str(&format!(
            "- {} [{} / {}]\n",
            check.name, check.state, check.conclusion
        ));
        if !check.workflow.is_empty() {
            out.push_str(&format!("  workflow: {}\n", check.workflow));
        }
        if !check.url.is_empty() {
            out.push_str(&format!("  url: {}\n", check.url));
        }
    }
}

fn render_pr_reviews(out: &mut String, reviews: &[PrReview]) {
    if reviews.is_empty() {
        out.push_str("no reviews\n");
        return;
    }
    for review in reviews {
        out.push_str(&format!(
            "=== review:{} [{}] by {} at {} ===\n",
            review.id, review.state, review.author, review.submitted_at
        ));
        if !review.body.is_empty() {
            out.push_str(&review.body);
            out.push('\n');
        }
    }
}

fn render_pr_review_threads(out: &mut String, threads: &[PrReviewThread]) {
    if threads.is_empty() {
        out.push_str("no review threads\n");
        return;
    }
    for thread in threads {
        out.push_str(&format!(
            "=== thread:{} resolved={} outdated={} path={} line={} ===\n",
            thread.id,
            thread.is_resolved,
            thread.is_outdated,
            if thread.path.is_empty() {
                "-"
            } else {
                thread.path.as_str()
            },
            thread
                .line
                .map(|line| line.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        for comment in &thread.comments {
            out.push_str(&format!(
                "--- comment:{} by {} ({}) ---\n{}\n",
                comment.id, comment.author, comment.updated_at, comment.body
            ));
        }
    }
}

fn load_or_refresh_issue<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    let cache = Cache::new(env.cache_root());
    if !refresh {
        if let Some(entry) = cache.load_entry(number) {
            return Ok(entry);
        }
    }
    refresh_issue_cache(env, number)
}

fn refresh_issue_cache<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
) -> Result<gwt_github::CacheEntry, SpecOpsError> {
    let snapshot = match env.client().fetch(number, None)? {
        gwt_github::FetchResult::Updated(snapshot) => snapshot,
        gwt_github::FetchResult::NotModified => {
            return Cache::new(env.cache_root())
                .load_entry(number)
                .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)));
        }
    };
    let cache = Cache::new(env.cache_root());
    cache.write_snapshot(&snapshot)?;
    cache
        .load_entry(number)
        .ok_or_else(|| SpecOpsError::SectionNotFound(format!("issue {}", number.0)))
}

fn load_or_refresh_linked_prs<E: CliEnv>(
    env: &mut E,
    number: IssueNumber,
    refresh: bool,
) -> Result<Vec<LinkedPrSummary>, SpecOpsError> {
    let cache_root = env.cache_root();
    if !refresh {
        if let Some(cached) = read_linked_prs_cache(&cache_root, number)? {
            return Ok(cached);
        }
    }
    let linked_prs = env.fetch_linked_prs(number).map_err(io_as_api_error)?;
    write_linked_prs_cache(&cache_root, number, &linked_prs)?;
    Ok(linked_prs)
}

fn linked_prs_cache_path(cache_root: &std::path::Path, number: IssueNumber) -> PathBuf {
    cache_root
        .join(number.0.to_string())
        .join("linked_prs.json")
}

fn read_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
) -> Result<Option<Vec<LinkedPrSummary>>, SpecOpsError> {
    let path = linked_prs_cache_path(cache_root, number);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = serde_json::from_str(&text)
                .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
            Ok(Some(parsed))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_as_api_error(err)),
    }
}

fn write_linked_prs_cache(
    cache_root: &std::path::Path,
    number: IssueNumber,
    linked_prs: &[LinkedPrSummary],
) -> Result<(), SpecOpsError> {
    let bytes = serde_json::to_vec_pretty(linked_prs)
        .map_err(|err| SpecOpsError::from(ApiError::Network(err.to_string())))?;
    write_atomic(&linked_prs_cache_path(cache_root, number), &bytes).map_err(io_as_api_error)
}

fn fetch_linked_prs_via_gh(
    owner: &str,
    repo: &str,
    number: IssueNumber,
) -> io::Result<Vec<LinkedPrSummary>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      timelineItems(first: 100, itemTypes: [CROSS_REFERENCED_EVENT, CONNECTED_EVENT]) {
        nodes {
          __typename
          ... on CrossReferencedEvent {
            source {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
              }
            }
          }
          ... on ConnectedEvent {
            subject {
              __typename
              ... on PullRequest {
                number
                title
                state
                url
              }
            }
          }
        }
      }
    }
  }
}
"#;

    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("repo={repo}"),
            "-F",
            &format!("number={}", number.0),
        ])
        .output()?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api graphql failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("issue"))
        .and_then(|v| v.get("timelineItems"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for node in nodes {
        let typename = node
            .get("__typename")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let pr = match typename {
            "CrossReferencedEvent" => node.get("source"),
            "ConnectedEvent" => node.get("subject"),
            _ => None,
        };
        let Some(pr) = pr else { continue };
        if pr.get("__typename").and_then(|v| v.as_str()) != Some("PullRequest") {
            continue;
        }
        let Some(pr_number) = pr.get("number").and_then(|v| v.as_u64()) else {
            continue;
        };
        if !seen.insert(pr_number) {
            continue;
        }
        out.push(LinkedPrSummary {
            number: pr_number,
            title: pr
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: pr
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: pr
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(out)
}

fn fetch_current_pr_via_gh(repo_path: &std::path::Path) -> io::Result<Option<PrStatus>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "number,title,state,url,mergeable,statusCheckRollup,reviewDecision",
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.contains("no pull requests found")
            || lowered.contains("no pull request found")
            || lowered.contains("could not resolve to a pull request")
        {
            return Ok(None);
        }
        return Err(io::Error::other(format!("gh pr view: {trimmed}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pr = gwt_git::pr_status::parse_pr_status_json(&stdout)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(Some(pr))
}

fn create_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    request: &PrCreateCall,
) -> io::Result<PrStatus> {
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--base".to_string(),
        request.base.clone(),
        "--title".to_string(),
        request.title.clone(),
        "--body".to_string(),
        request.body.clone(),
    ];
    if let Some(head) = &request.head {
        args.push("--head".to_string());
        args.push(head.clone());
    }
    for label in &request.labels {
        args.push("--label".to_string());
        args.push(label.clone());
    }
    if request.draft {
        args.push("--draft".to_string());
    }

    let output = Command::new("gh")
        .args(&args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr create: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let url = extract_pr_url(&stdout).ok_or_else(|| {
        io::Error::other(format!("gh pr create: missing PR URL in output: {stdout}"))
    })?;
    let number = parse_pr_number_from_url(&url)
        .ok_or_else(|| io::Error::other(format!("gh pr create: invalid PR URL: {url}")))?;
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

fn edit_pr_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
    title: Option<&str>,
    body: Option<&str>,
    add_labels: &[String],
) -> io::Result<PrStatus> {
    let mut args = vec!["pr".to_string(), "edit".to_string(), number.to_string()];
    if let Some(title) = title {
        args.push("--title".to_string());
        args.push(title.to_string());
    }
    if let Some(body) = body {
        args.push("--body".to_string());
        args.push(body.to_string());
    }
    for label in add_labels {
        args.push("--add-label".to_string());
        args.push(label.clone());
    }

    let output = Command::new("gh")
        .args(&args)
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr edit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))
}

fn extract_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(ToOwned::to_owned)
}

fn parse_pr_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/').rsplit('/').next()?.parse().ok()
}

fn comment_on_pr_via_gh(repo_path: &std::path::Path, number: u64, body: &str) -> io::Result<()> {
    let output = Command::new("gh")
        .args(["pr", "comment", &number.to_string(), "--body", body])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh pr comment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn fetch_pr_reviews_via_gh(owner: &str, repo: &str, number: u64) -> io::Result<Vec<PrReview>> {
    let endpoint = format!("repos/{owner}/{repo}/pulls/{number}/reviews");
    let output = Command::new("gh").args(["api", &endpoint]).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let values: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(values
        .into_iter()
        .map(|value| PrReview {
            id: value
                .get("id")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .or_else(|| {
                    value
                        .get("node_id")
                        .and_then(|v| v.as_str())
                        .map(ToOwned::to_owned)
                })
                .unwrap_or_default(),
            state: value
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            body: value
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            submitted_at: value
                .get("submitted_at")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            author: value
                .get("user")
                .and_then(|v| v.get("login"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect())
}

fn fetch_pr_review_threads_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
) -> io::Result<Vec<PrReviewThread>> {
    let query = r#"
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          comments(first: 100) {
            nodes {
              id
              body
              createdAt
              updatedAt
              author { login }
            }
          }
        }
      }
    }
  }
}
"#;
    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={query}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("repo={repo}"),
            "-F",
            &format!("number={number}"),
        ])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api graphql: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    let nodes = value
        .get("data")
        .and_then(|v| v.get("repository"))
        .and_then(|v| v.get("pullRequest"))
        .and_then(|v| v.get("reviewThreads"))
        .and_then(|v| v.get("nodes"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(nodes
        .into_iter()
        .map(|node| PrReviewThread {
            id: node
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            is_resolved: node
                .get("isResolved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_outdated: node
                .get("isOutdated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            path: node
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            line: node.get("line").and_then(|v| v.as_u64()),
            comments: node
                .get("comments")
                .and_then(|v| v.get("nodes"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|comment| PrReviewThreadComment {
                    id: comment
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    body: comment
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    created_at: comment
                        .get("createdAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    updated_at: comment
                        .get("updatedAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    author: comment
                        .get("author")
                        .and_then(|v| v.get("login"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect(),
        })
        .collect())
}

fn reply_and_resolve_pr_review_threads_via_gh(
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
) -> io::Result<usize> {
    let unresolved: Vec<PrReviewThread> = fetch_pr_review_threads_via_gh(owner, repo, number)?
        .into_iter()
        .filter(|thread| !thread.is_resolved && !thread.is_outdated)
        .collect();

    for thread in &unresolved {
        let reply_mutation = r#"
mutation($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {
    pullRequestReviewThreadId: $threadId,
    body: $body
  }) {
    comment { id }
  }
}
"#;
        let reply = Command::new("gh")
            .args([
                "api",
                "graphql",
                "-f",
                &format!("query={reply_mutation}"),
                "-f",
                &format!("threadId={}", thread.id),
                "-f",
                &format!("body={body}"),
            ])
            .output()?;
        if !reply.status.success() {
            return Err(io::Error::other(format!(
                "gh api graphql reply: {}",
                String::from_utf8_lossy(&reply.stderr).trim()
            )));
        }

        let resolve_mutation = r#"
mutation($threadId: ID!) {
  resolveReviewThread(input: { threadId: $threadId }) {
    thread { id isResolved }
  }
}
"#;
        let resolve = Command::new("gh")
            .args([
                "api",
                "graphql",
                "-f",
                &format!("query={resolve_mutation}"),
                "-f",
                &format!("threadId={}", thread.id),
            ])
            .output()?;
        if !resolve.status.success() {
            return Err(io::Error::other(format!(
                "gh api graphql resolve: {}",
                String::from_utf8_lossy(&resolve.stderr).trim()
            )));
        }
    }

    Ok(unresolved.len())
}

fn fetch_pr_checks_via_gh(
    repo_slug: &str,
    repo_path: &std::path::Path,
    number: u64,
) -> io::Result<PrChecksSummary> {
    let pr = gwt_git::pr_status::fetch_pr_status(repo_slug, number)
        .map_err(|err| io::Error::other(err.to_string()))?;

    let primary_fields = [
        "name",
        "state",
        "conclusion",
        "detailsUrl",
        "startedAt",
        "completedAt",
    ];
    let mut output = Command::new("gh")
        .args([
            "pr",
            "checks",
            &number.to_string(),
            "--json",
            &primary_fields.join(","),
        ])
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let available = parse_available_fields(&stderr);
        if !available.is_empty() {
            let fallback_fields = [
                "name",
                "state",
                "bucket",
                "link",
                "startedAt",
                "completedAt",
                "workflow",
            ];
            let selected: Vec<&str> = fallback_fields
                .iter()
                .copied()
                .filter(|field| available.iter().any(|candidate| candidate == field))
                .collect();
            if !selected.is_empty() {
                output = Command::new("gh")
                    .args([
                        "pr",
                        "checks",
                        &number.to_string(),
                        "--json",
                        &selected.join(","),
                    ])
                    .current_dir(repo_path)
                    .output()?;
            }
        }
    }

    let checks = if output.status.success() {
        parse_pr_checks_items_json(&String::from_utf8_lossy(&output.stdout))
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?
    } else {
        Vec::new()
    };

    Ok(PrChecksSummary {
        summary: format!(
            "PR #{} | CI: {} | Merge: {} | Review: {}",
            pr.number, pr.ci_status, pr.mergeable, pr.review_status
        ),
        ci_status: pr.ci_status,
        merge_status: pr.mergeable,
        review_status: pr.review_status,
        checks,
    })
}

fn parse_pr_checks_items_json(json: &str) -> Result<Vec<PrCheckItem>, serde_json::Error> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json)?;
    Ok(values
        .into_iter()
        .map(|value| PrCheckItem {
            name: value
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            state: value
                .get("state")
                .or_else(|| value.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            conclusion: value
                .get("conclusion")
                .or_else(|| value.get("bucket"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            url: value
                .get("detailsUrl")
                .or_else(|| value.get("link"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            started_at: value
                .get("startedAt")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            completed_at: value
                .get("completedAt")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            workflow: value
                .get("workflow")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect())
}

fn parse_available_fields(message: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut collecting = false;
    for line in message.lines() {
        if line.contains("Available fields:") {
            collecting = true;
            continue;
        }
        if !collecting {
            continue;
        }
        let field = line.trim();
        if field.is_empty() {
            continue;
        }
        fields.push(field.to_string());
    }
    fields
}

fn fetch_actions_run_log_via_gh(repo_path: &std::path::Path, run_id: u64) -> io::Result<String> {
    let output = Command::new("gh")
        .args(["run", "view", &run_id.to_string(), "--log"])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh run view --log: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn fetch_actions_job_log_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &std::path::Path,
    job_id: u64,
) -> io::Result<String> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/jobs/{job_id}/logs");
    let output = Command::new("gh")
        .args(["api", &endpoint])
        .current_dir(repo_path)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if output.stdout.starts_with(b"PK") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "job logs returned a zip archive; unable to parse",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Dispatch a `gwt hook <name> [args...]` invocation.
///
/// SPEC #1942 (CORE-CLI) scope: this is the single entry point for every
/// in-binary hook handler. Each handler reads stdin (usually JSON), performs
/// its judgment, and either:
///
/// - exits 0 (allow / success, stdout empty or a human-readable status)
/// - exits 2 with a `{"decision":"block",...}` JSON on stdout (block)
///
/// Dispatches to the hook handlers in [`crate::cli::hook`]. Unknown hooks
/// exit 2 with a `gwt hook: unknown hook '<name>'` message on stderr so
/// that settings_local typos surface loudly. Runtime errors from known
/// handlers exit 1 with the error chain on stderr; they are never turned
/// into `decision=block` to avoid false positives under partial outages.
pub fn run_hook<E: CliEnv>(env: &mut E, name: &str, rest: &[String]) -> Result<i32, SpecOpsError> {
    use crate::cli::hook::{block_bash_policy, forward, runtime_state, BlockDecision, HookKind};

    let Some(kind) = HookKind::from_name(name) else {
        let _ = writeln!(env.stderr(), "gwt hook: unknown hook '{name}'");
        return Ok(2);
    };

    // Every block hook returns `Result<Option<BlockDecision>, HookError>`.
    // `emit_block_decision` serializes a decision to stdout and yields
    // the block exit code (2). `emit_hook_error` reports a handler
    // error on stderr and yields 1.
    fn emit_block_decision<E: CliEnv>(env: &mut E, decision: &BlockDecision) -> i32 {
        match serde_json::to_vec(decision) {
            Ok(bytes) => {
                let _ = env.stdout().write_all(&bytes);
                let _ = env.stdout().flush();
                2
            }
            Err(err) => {
                let _ = writeln!(
                    env.stderr(),
                    "gwt hook: failed to serialize decision: {err}"
                );
                1
            }
        }
    }
    fn emit_hook_error<E: CliEnv>(env: &mut E, name: &str, err: impl std::fmt::Display) -> i32 {
        let _ = writeln!(env.stderr(), "gwt hook {name}: {err}");
        1
    }

    match kind {
        HookKind::RuntimeState => {
            let Some(event) = rest.first() else {
                let _ = writeln!(
                    env.stderr(),
                    "gwt hook runtime-state: missing <event> argument"
                );
                return Ok(2);
            };
            match runtime_state::handle(event) {
                Ok(()) => Ok(0),
                Err(err) => Ok(emit_hook_error(env, name, err)),
            }
        }
        HookKind::BlockBashPolicy => match block_bash_policy::handle() {
            Ok(None) => Ok(0),
            Ok(Some(decision)) => Ok(emit_block_decision(env, &decision)),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
        HookKind::Forward => match forward::handle() {
            Ok(()) => Ok(0),
            Err(err) => Ok(emit_hook_error(env, name, err)),
        },
    }
}

// ---------------------------------------------------------------------------
// ClientRef: a borrow wrapper that still implements IssueClient
// ---------------------------------------------------------------------------

struct ClientRef<'a, C: IssueClient> {
    inner: &'a C,
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

/// Default production [`CliEnv`] that uses an [`HttpIssueClient`] with
/// credentials from `gh auth token` and the user's home cache directory.
pub struct DefaultCliEnv {
    client: HttpIssueClient,
    cache_root: PathBuf,
    repo_path: PathBuf,
    owner: String,
    repo: String,
    stdout: io::Stdout,
    stderr: io::Stderr,
}

impl DefaultCliEnv {
    pub fn new(
        owner: &str,
        repo: &str,
        repo_path: PathBuf,
    ) -> Result<Self, gwt_github::client::ApiError> {
        let client = HttpIssueClient::from_gh_auth(owner, repo)?;
        let cache_root = Self::default_cache_root();
        Ok(DefaultCliEnv {
            client,
            cache_root,
            repo_path,
            owner: owner.to_string(),
            repo: repo.to_string(),
            stdout: io::stdout(),
            stderr: io::stderr(),
        })
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
    pub fn new_for_hooks() -> Result<Self, gwt_github::client::ApiError> {
        let transport = gwt_github::client::http::ReqwestTransport::new()
            .map_err(|e| gwt_github::client::ApiError::Network(e.to_string()))?;
        let client = HttpIssueClient::with_transport(transport, String::new(), "", "");
        Ok(DefaultCliEnv {
            client,
            cache_root: Self::default_cache_root(),
            repo_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            owner: String::new(),
            repo: String::new(),
            stdout: io::stdout(),
            stderr: io::stderr(),
        })
    }

    fn default_cache_root() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gwt")
            .join("cache")
            .join("issues")
    }
}

impl CliEnv for DefaultCliEnv {
    type Client = HttpIssueClient;

    fn client(&self) -> &Self::Client {
        &self.client
    }
    fn cache_root(&self) -> PathBuf {
        self.cache_root.clone()
    }
    fn stdout(&mut self) -> &mut dyn io::Write {
        &mut self.stdout
    }
    fn stderr(&mut self) -> &mut dyn io::Write {
        &mut self.stderr
    }
    fn read_file(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(path)
    }
    fn fetch_linked_prs(&mut self, number: IssueNumber) -> io::Result<Vec<LinkedPrSummary>> {
        fetch_linked_prs_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_current_pr(&mut self) -> io::Result<Option<PrStatus>> {
        fetch_current_pr_via_gh(&self.repo_path)
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
        edit_or_create_repo_guard(&self.owner, &self.repo)?;
        let request = PrCreateCall {
            base: base.to_string(),
            head: head.map(ToOwned::to_owned),
            title: title.to_string(),
            body: body.to_string(),
            labels: labels.to_vec(),
            draft,
        };
        create_pr_via_gh(
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
        edit_or_create_repo_guard(&self.owner, &self.repo)?;
        edit_pr_via_gh(
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
        comment_on_pr_via_gh(&self.repo_path, number, body)
    }
    fn fetch_pr_reviews(&mut self, number: u64) -> io::Result<Vec<PrReview>> {
        fetch_pr_reviews_via_gh(&self.owner, &self.repo, number)
    }
    fn fetch_pr_review_threads(&mut self, number: u64) -> io::Result<Vec<PrReviewThread>> {
        fetch_pr_review_threads_via_gh(&self.owner, &self.repo, number)
    }
    fn reply_and_resolve_pr_review_threads(
        &mut self,
        number: u64,
        body: &str,
    ) -> io::Result<usize> {
        reply_and_resolve_pr_review_threads_via_gh(&self.owner, &self.repo, number, body)
    }
    fn fetch_pr_checks(&mut self, number: u64) -> io::Result<PrChecksSummary> {
        fetch_pr_checks_via_gh(
            &format!("{}/{}", self.owner, self.repo),
            &self.repo_path,
            number,
        )
    }
    fn fetch_actions_run_log(&mut self, run_id: u64) -> io::Result<String> {
        fetch_actions_run_log_via_gh(&self.repo_path, run_id)
    }
    fn fetch_actions_job_log(&mut self, job_id: u64) -> io::Result<String> {
        fetch_actions_job_log_via_gh(&self.owner, &self.repo, &self.repo_path, job_id)
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
        "hook" => parse_hook_args(&rest),
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
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub files: std::collections::HashMap<String, String>,
    pub linked_prs: std::collections::HashMap<u64, Vec<LinkedPrSummary>>,
    pub linked_pr_call_log: Vec<u64>,
    pub current_pr: Option<PrStatus>,
    pub prs: std::collections::HashMap<u64, PrStatus>,
    pub created_pr: Option<PrStatus>,
    pub pr_comments: Vec<(u64, String)>,
    pub pr_create_call_log: Vec<PrCreateCall>,
    pub pr_edit_call_log: Vec<PrEditCall>,
    pub pr_reviews: std::collections::HashMap<u64, Vec<PrReview>>,
    pub pr_review_threads: std::collections::HashMap<u64, Vec<PrReviewThread>>,
    pub pr_checks: std::collections::HashMap<u64, PrChecksSummary>,
    pub pr_current_call_count: usize,
    pub pr_view_call_log: Vec<u64>,
    pub pr_reviews_call_log: Vec<u64>,
    pub pr_review_threads_call_log: Vec<u64>,
    pub pr_reply_and_resolve_call_log: Vec<(u64, String)>,
    pub pr_checks_call_log: Vec<u64>,
    pub run_logs: std::collections::HashMap<u64, String>,
    pub run_log_call_log: Vec<u64>,
    pub job_logs: std::collections::HashMap<u64, String>,
    pub job_log_call_log: Vec<u64>,
}

impl TestEnv {
    pub fn new(cache_root: PathBuf) -> Self {
        TestEnv {
            client: FakeIssueClient::new(),
            cache_root,
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: std::collections::HashMap::new(),
            linked_prs: std::collections::HashMap::new(),
            linked_pr_call_log: Vec::new(),
            current_pr: None,
            prs: std::collections::HashMap::new(),
            created_pr: None,
            pr_comments: Vec::new(),
            pr_create_call_log: Vec::new(),
            pr_edit_call_log: Vec::new(),
            pr_reviews: std::collections::HashMap::new(),
            pr_review_threads: std::collections::HashMap::new(),
            pr_checks: std::collections::HashMap::new(),
            pr_current_call_count: 0,
            pr_view_call_log: Vec::new(),
            pr_reviews_call_log: Vec::new(),
            pr_review_threads_call_log: Vec::new(),
            pr_reply_and_resolve_call_log: Vec::new(),
            pr_checks_call_log: Vec::new(),
            run_logs: std::collections::HashMap::new(),
            run_log_call_log: Vec::new(),
            job_logs: std::collections::HashMap::new(),
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
    fn stdout(&mut self) -> &mut dyn io::Write {
        &mut self.stdout
    }
    fn stderr(&mut self) -> &mut dyn io::Write {
        &mut self.stderr
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

fn edit_or_create_repo_guard(owner: &str, repo: &str) -> io::Result<()> {
    if owner.is_empty() || repo.is_empty() {
        return Err(io::Error::other(
            "missing repository context for PR create/edit operation",
        ));
    }
    Ok(())
}

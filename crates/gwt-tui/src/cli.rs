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

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use gwt_github::client::fake::FakeIssueClient;
use gwt_github::client::http::HttpIssueClient;
use gwt_github::client::IssueClient;
use gwt_github::{Cache, IssueNumber, SectionName, SpecListFilter, SpecOps, SpecOpsError};

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
                "usage: gwt issue spec <n> [--section <name>|--edit <name> -f <file>] | gwt issue spec list [--phase <p>] [--state open|closed]"
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
/// `issue`. The TUI launcher keeps its legacy behaviour (positional repo
/// path) for any other shape.
pub fn should_dispatch_cli(args: &[String]) -> bool {
    args.get(1).map(|s| s == "issue").unwrap_or(false)
}

/// Parse an argv slice into a [`CliCommand`]. The slice should start from
/// the first post-subcommand argument — i.e. if the caller received
/// `["gwt", "issue", "spec", "2001"]`, they pass `["spec", "2001"]`.
pub fn parse_issue_args(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("spec") => parse_spec_args(it.collect::<Vec<_>>().as_slice()),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
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

/// High-level runtime environment for the CLI. Kept as a trait so tests can
/// inject a [`FakeIssueClient`] instead of spinning up real HTTP.
pub trait CliEnv {
    type Client: IssueClient;
    fn client(&self) -> &Self::Client;
    fn cache_root(&self) -> PathBuf;
    fn stdout(&mut self) -> &mut dyn io::Write;
    fn stderr(&mut self) -> &mut dyn io::Write;
    fn read_file(&self, path: &str) -> io::Result<String>;
}

/// Dispatch a parsed [`CliCommand`] against the given [`CliEnv`].
///
/// We collect output into a String buffer first so the [`SpecOps`] borrow of
/// `env.client()` does not conflict with the mutable borrow required by
/// `env.stdout()` at write time.
pub fn run<E: CliEnv>(env: &mut E, cmd: CliCommand) -> Result<i32, SpecOpsError> {
    let mut out = String::new();
    let code = {
        let cache = Cache::new(env.cache_root());
        let ops = SpecOps::new(
            ClientRef {
                inner: env.client(),
            },
            cache,
        );
        match cmd {
            CliCommand::SpecReadAll { number } => {
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
                let content =
                    ops.read_section(IssueNumber(number), &SectionName(section.clone()))?;
                out.push_str(&format!("{content}\n"));
                0
            }
            CliCommand::SpecEditSection {
                number,
                section,
                file,
            } => {
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
        }
    };
    let _ = env.stdout().write_all(out.as_bytes());
    Ok(code)
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
    stdout: io::Stdout,
    stderr: io::Stderr,
}

impl DefaultCliEnv {
    pub fn new(owner: &str, repo: &str) -> Result<Self, gwt_github::client::ApiError> {
        let client = HttpIssueClient::from_gh_auth(owner, repo)?;
        let cache_root = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".gwt")
            .join("cache")
            .join("issues");
        Ok(DefaultCliEnv {
            client,
            cache_root,
            stdout: io::stdout(),
            stderr: io::stderr(),
        })
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
}

/// Convenience for tests and the main entry point: take a raw argv slice,
/// parse the subcommand, and run it. Returns the process exit code.
pub fn dispatch<E: CliEnv>(env: &mut E, args: &[String]) -> i32 {
    // Skip args[0] (program name) and args[1] ("issue").
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();
    match parse_issue_args(&rest) {
        Ok(cmd) => match run(env, cmd) {
            Ok(code) => code,
            Err(e) => {
                let _ = writeln!(env.stderr(), "gwt issue: {e}");
                1
            }
        },
        Err(e) => {
            let _ = writeln!(env.stderr(), "gwt issue: {e}");
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
}

impl TestEnv {
    pub fn new(cache_root: PathBuf) -> Self {
        TestEnv {
            client: FakeIssueClient::new(),
            cache_root,
            stdout: Vec::new(),
            stderr: Vec::new(),
            files: std::collections::HashMap::new(),
        }
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
}

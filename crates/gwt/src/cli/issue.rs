use std::io::{self, Read};

use gwt_github::{
    Cache, IssueClient, IssueNumber, SectionName, SpecListFilter, SpecOps, SpecOpsError,
};

use crate::cli::{CliCommand, CliEnv, CliParseError, ClientRef};

pub(super) fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
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

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: CliCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
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
                    Ok(content) => out.push_str(&format!("=== {name} ===\n{content}\n")),
                    Err(SpecOpsError::SectionNotFound(_)) => {}
                    Err(err) => return Err(err),
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
            let content = ops.read_section(IssueNumber(number), &SectionName(section))?;
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
                let mut buffer = String::new();
                io::stdin().read_to_string(&mut buffer).map_err(|err| {
                    SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
                })?;
                buffer
            } else {
                env.read_file(&file).map_err(|err| {
                    SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
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
                state: state.as_deref().and_then(|value| match value {
                    "open" => Some(gwt_github::client::IssueState::Open),
                    "closed" => Some(gwt_github::client::IssueState::Closed),
                    _ => None,
                }),
            };
            let list = env.client().list_spec_issues(&filter)?;
            for spec in list {
                let state_marker = match spec.state {
                    gwt_github::client::IssueState::Open => "OPEN",
                    gwt_github::client::IssueState::Closed => "CLOSED",
                };
                let phase_label = spec
                    .labels
                    .iter()
                    .find(|label: &&String| label.starts_with("phase/"))
                    .cloned()
                    .unwrap_or_default();
                out.push_str(&format!(
                    "#{} [{state_marker}] [{phase_label}] {}\n",
                    spec.number.0, spec.title
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
            let raw = env.read_file(&file).map_err(|err| {
                SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
            })?;
            let parsed = gwt_github::extract_sections(&raw)
                .map_err(|err| SpecOpsError::from(gwt_github::body::ParseError::Section(err)))?;
            let sections: std::collections::BTreeMap<SectionName, String> = parsed
                .into_iter()
                .map(|section| (section.name, section.content))
                .collect();
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
                let list = env.client().list_spec_issues(&SpecListFilter::default())?;
                for summary in list {
                    ops.read_section(summary.number, &SectionName("spec".to_string()))
                        .ok();
                }
                out.push_str("pulled all gwt-spec issues\n");
            } else if numbers.is_empty() {
                return Err(SpecOpsError::SectionNotFound(
                    "pull requires --all or <n>".into(),
                ));
            } else {
                for number in numbers {
                    ops.read_section(IssueNumber(number), &SectionName("spec".to_string()))?;
                    out.push_str(&format!("pulled #{number}\n"));
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
            ops.read_section(IssueNumber(number), &SectionName("spec".to_string()))?;
            out.push_str(&format!("repaired cache for #{number}\n"));
            0
        }
        CliCommand::IssueView { number, refresh } => {
            let entry = super::load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            super::render_issue(out, &entry.snapshot);
            0
        }
        CliCommand::IssueComments { number, refresh } => {
            let entry = super::load_or_refresh_issue(env, IssueNumber(number), refresh)?;
            super::render_issue_comments(out, &entry.snapshot);
            0
        }
        CliCommand::IssueLinkedPrs { number, refresh } => {
            let linked_prs = super::load_or_refresh_linked_prs(env, IssueNumber(number), refresh)?;
            super::render_linked_prs(out, &linked_prs);
            0
        }
        CliCommand::IssueCreate {
            title,
            file,
            labels,
        } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let snapshot = env.client().create_issue(&title, &body, &labels)?;
            Cache::new(env.cache_root()).write_snapshot(&snapshot)?;
            out.push_str(&format!(
                "created issue #{} with labels {:?}\n",
                snapshot.number.0, snapshot.labels
            ));
            0
        }
        CliCommand::IssueComment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let comment = env.client().create_comment(IssueNumber(number), &body)?;
            let _ = super::refresh_issue_cache(env, IssueNumber(number))?;
            out.push_str(&format!(
                "created comment {} on #{}\n",
                comment.id.0, number
            ));
            0
        }
        _ => unreachable!("issue::run called with non-issue command"),
    };
    Ok(code)
}

fn parse_spec_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    if args.is_empty() {
        return Err(CliParseError::Usage);
    }
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
        return Ok(CliCommand::SpecCreate {
            title: title.ok_or(CliParseError::MissingFlag("--title"))?,
            file: file.ok_or(CliParseError::MissingFlag("-f"))?,
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
                    let number = other
                        .parse()
                        .map_err(|_| CliParseError::InvalidNumber(other.to_string()))?;
                    numbers.push(number);
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
        let number = args[1]
            .parse()
            .map_err(|_| CliParseError::InvalidNumber(args[1].clone()))?;
        return Ok(CliCommand::SpecRepair { number });
    }

    let number = head
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
    if let Some(section) = edit_section {
        return Ok(CliCommand::SpecEditSection {
            number,
            section,
            file: file.ok_or(CliParseError::MissingFlag("-f"))?,
        });
    }
    if let Some(section) = section {
        return Ok(CliCommand::SpecReadSection { number, section });
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
    if args.len() != 3 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_github::client::{IssueSnapshot, IssueState, UpdatedAt};
    use tempfile::TempDir;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn issue_family_parse_directly_handles_view() {
        let cmd = parse(&[s("view"), s("42")]).expect("parse issue family command");
        assert_eq!(
            cmd,
            CliCommand::IssueView {
                number: 42,
                refresh: false,
            }
        );
    }

    #[test]
    fn issue_family_run_directly_renders_cached_issue() {
        let tmp = TempDir::new().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let snapshot = IssueSnapshot {
            number: IssueNumber(42),
            title: "Issue family direct run".to_string(),
            body: "body".to_string(),
            labels: vec!["bug".to_string()],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-04-12T00:00:00Z"),
            comments: vec![],
        };
        gwt_github::Cache::new(tmp.path().to_path_buf())
            .write_snapshot(&snapshot)
            .expect("write cache");

        let mut out = String::new();
        let code = run(
            &mut env,
            CliCommand::IssueView {
                number: 42,
                refresh: false,
            },
            &mut out,
        )
        .expect("run issue family");

        assert_eq!(code, 0);
        assert!(out.contains("#42 [OPEN] Issue family direct run"));
    }
}

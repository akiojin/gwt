use gwt_github::{Cache, IssueClient, IssueNumber, SpecOpsError};

use crate::cli::{CliCommand, CliEnv, CliParseError};

pub(super) fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("spec") => super::issue_spec::parse(it.collect::<Vec<_>>().as_slice()),
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
    if matches!(
        cmd,
        CliCommand::SpecReadAll { .. }
            | CliCommand::SpecReadSection { .. }
            | CliCommand::SpecEditSection { .. }
            | CliCommand::SpecEditSectionJson { .. }
            | CliCommand::SpecList { .. }
            | CliCommand::SpecCreate { .. }
            | CliCommand::SpecCreateJson { .. }
            | CliCommand::SpecCreateHelp
            | CliCommand::SpecPull { .. }
            | CliCommand::SpecRepair { .. }
            | CliCommand::SpecRename { .. }
    ) {
        return super::issue_spec::run(env, cmd, out);
    }

    let code = match cmd {
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
    fn issue_spec_submodule_parse_directly_handles_list() {
        let args = [s("list"), s("--phase"), s("phase/implementation")];
        let refs = args.iter().collect::<Vec<_>>();
        let cmd = crate::cli::issue_spec::parse(&refs).expect("parse spec family command");
        assert_eq!(
            cmd,
            CliCommand::SpecList {
                phase: Some("phase/implementation".to_string()),
                state: None,
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

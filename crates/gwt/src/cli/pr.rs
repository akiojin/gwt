use gwt_github::SpecOpsError;

use crate::cli::{CliCommand, CliEnv, CliParseError};

pub(super) fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("current") => {
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::PrCurrent)
        }
        Some("create") => parse_pr_create_args(it.collect::<Vec<_>>().as_slice()),
        Some("edit") => parse_pr_edit_args(it.collect::<Vec<_>>().as_slice()),
        Some("view") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::PrView { number })
        }
        Some("comment") => {
            let number = super::parse_required_number(it.next())?;
            super::expect_flag(it.next(), "-f")?;
            let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::PrComment { number, file })
        }
        Some("reviews") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::PrReviews { number })
        }
        Some("review-threads") => match it.next().map(String::as_str) {
            Some("reply-and-resolve") => {
                let number = super::parse_required_number(it.next())?;
                super::expect_flag(it.next(), "-f")?;
                let file = it.next().ok_or(CliParseError::MissingFlag("-f"))?.clone();
                super::ensure_no_remaining_args(it)?;
                Ok(CliCommand::PrReviewThreadsReplyAndResolve { number, file })
            }
            Some(number_arg) => {
                let number = number_arg
                    .parse()
                    .map_err(|_| CliParseError::InvalidNumber(number_arg.to_string()))?;
                super::ensure_no_remaining_args(it)?;
                Ok(CliCommand::PrReviewThreads { number })
            }
            None => Err(CliParseError::Usage),
        },
        Some("checks") => {
            let number = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::PrChecks { number })
        }
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
        CliCommand::PrCurrent => {
            match env.fetch_current_pr().map_err(super::io_as_api_error)? {
                Some(pr) => super::render_pr(out, &pr),
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
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let pr = env
                .create_pr(&base, head.as_deref(), &title, &body, &labels, draft)
                .map_err(super::io_as_api_error)?;
            out.push_str("created pull request\n");
            super::render_pr(out, &pr);
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
                .map(|path| env.read_file(path).map_err(super::io_as_api_error))
                .transpose()?;
            let pr = env
                .edit_pr(number, title.as_deref(), body.as_deref(), &add_labels)
                .map_err(super::io_as_api_error)?;
            out.push_str("updated pull request\n");
            super::render_pr(out, &pr);
            0
        }
        CliCommand::PrView { number } => {
            let pr = env.fetch_pr(number).map_err(super::io_as_api_error)?;
            super::render_pr(out, &pr);
            0
        }
        CliCommand::PrComment { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            env.comment_on_pr(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!("created comment on PR #{number}\n"));
            0
        }
        CliCommand::PrReviews { number } => {
            let reviews = env
                .fetch_pr_reviews(number)
                .map_err(super::io_as_api_error)?;
            super::render_pr_reviews(out, &reviews);
            0
        }
        CliCommand::PrReviewThreads { number } => {
            let threads = env
                .fetch_pr_review_threads(number)
                .map_err(super::io_as_api_error)?;
            super::render_pr_review_threads(out, &threads);
            0
        }
        CliCommand::PrReviewThreadsReplyAndResolve { number, file } => {
            let body = env.read_file(&file).map_err(super::io_as_api_error)?;
            let resolved = env
                .reply_and_resolve_pr_review_threads(number, &body)
                .map_err(super::io_as_api_error)?;
            out.push_str(&format!(
                "replied to and resolved {resolved} review threads on PR #{number}\n"
            ));
            0
        }
        CliCommand::PrChecks { number } => {
            let report = env
                .fetch_pr_checks(number)
                .map_err(super::io_as_api_error)?;
            super::render_pr_checks(out, &report);
            0
        }
        _ => unreachable!("pr::run called with non-pr command"),
    };
    Ok(code)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn seeded_pr() -> gwt_git::PrStatus {
        gwt_git::PrStatus {
            number: 7,
            title: "CLI family split".to_string(),
            state: gwt_git::pr_status::PrState::Open,
            url: "https://example.com/pr/7".to_string(),
            ci_status: "SUCCESS".to_string(),
            mergeable: "MERGEABLE".to_string(),
            review_status: "APPROVED".to_string(),
        }
    }

    #[test]
    fn pr_family_parse_directly_handles_current() {
        let cmd = parse(&[s("current")]).expect("parse pr family command");
        assert_eq!(cmd, CliCommand::PrCurrent);
    }

    #[test]
    fn pr_family_run_directly_renders_current_pr() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_current_pr(Some(seeded_pr()));

        let mut out = String::new();
        let code = run(&mut env, CliCommand::PrCurrent, &mut out).expect("run pr family");

        assert_eq!(code, 0);
        assert!(out.contains("#7 [OPEN] CLI family split"));
        assert_eq!(env.pr_current_call_count, 1);
    }
}

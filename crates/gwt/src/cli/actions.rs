use gwt_github::SpecOpsError;

use crate::cli::{CliCommand, CliEnv, CliParseError};

pub(super) fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("logs") => {
            super::expect_flag(it.next(), "--run")?;
            let run_id = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::ActionsLogs { run_id })
        }
        Some("job-logs") => {
            super::expect_flag(it.next(), "--job")?;
            let job_id = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(CliCommand::ActionsJobLogs { job_id })
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
        CliCommand::ActionsLogs { run_id } => {
            let log = env
                .fetch_actions_run_log(run_id)
                .map_err(super::io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        CliCommand::ActionsJobLogs { job_id } => {
            let log = env
                .fetch_actions_job_log(job_id)
                .map_err(super::io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        _ => unreachable!("actions::run called with non-actions command"),
    };
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn actions_family_parse_directly_handles_logs() {
        let cmd = parse(&[s("logs"), s("--run"), s("101")]).expect("parse actions family command");
        assert_eq!(cmd, CliCommand::ActionsLogs { run_id: 101 });
    }

    #[test]
    fn actions_family_run_directly_renders_run_log() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_run_log(101, "hello from actions log");

        let mut out = String::new();
        let code = run(&mut env, CliCommand::ActionsLogs { run_id: 101 }, &mut out)
            .expect("run actions family");

        assert_eq!(code, 0);
        assert!(out.contains("hello from actions log"));
        assert_eq!(env.run_log_call_log, vec![101]);
    }
}

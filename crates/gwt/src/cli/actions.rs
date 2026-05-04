use std::io;

use gwt_github::SpecOpsError;

use crate::cli::{ActionsCommand, CliEnv, CliParseError};

pub(super) fn parse(args: &[String]) -> Result<ActionsCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("logs") => {
            super::expect_flag(it.next(), "--run")?;
            let run_id = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(ActionsCommand::Logs { run_id })
        }
        Some("job-logs") => {
            super::expect_flag(it.next(), "--job")?;
            let job_id = super::parse_required_number(it.next())?;
            super::ensure_no_remaining_args(it)?;
            Ok(ActionsCommand::JobLogs { job_id })
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: ActionsCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match cmd {
        ActionsCommand::Logs { run_id } => {
            let log = env
                .fetch_actions_run_log(run_id)
                .map_err(super::io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        ActionsCommand::JobLogs { job_id } => {
            let log = env
                .fetch_actions_job_log(job_id)
                .map_err(super::io_as_api_error)?;
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
    };
    Ok(code)
}

pub(super) fn fetch_actions_run_log_via_gh(
    repo_path: &std::path::Path,
    run_id: u64,
) -> io::Result<String> {
    let output = gwt_core::process::hidden_command("gh")
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

pub(super) fn fetch_actions_job_log_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &std::path::Path,
    job_id: u64,
) -> io::Result<String> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/jobs/{job_id}/logs");
    let output = gwt_core::process::hidden_command("gh")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn actions_family_parse_directly_handles_logs() {
        let cmd = parse(&[s("logs"), s("--run"), s("101")]).expect("parse actions family command");
        assert_eq!(cmd, ActionsCommand::Logs { run_id: 101 });
    }

    #[test]
    fn actions_family_run_directly_renders_run_log() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_run_log(101, "hello from actions log");

        let mut out = String::new();
        let code = run(&mut env, ActionsCommand::Logs { run_id: 101 }, &mut out)
            .expect("run actions family");

        assert_eq!(code, 0);
        assert!(out.contains("hello from actions log"));
        assert_eq!(env.run_log_call_log, vec![101]);
    }
}

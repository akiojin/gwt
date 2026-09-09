use std::io;

use gwt_github::SpecOpsError;

use crate::cli::{CliEnv, CliParseError};

/// SPEC-1942 command model for `actions.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsCommand {
    /// `actions.logs`.
    Logs { run_id: u64 },
    /// `actions.job_logs`.
    JobLogs { job_id: u64 },
    /// `actions.rerun` (Issue #3515): re-run a failed run or a single failed
    /// job without pushing a throwaway commit to retrigger CI.
    Rerun { target: ActionsRerunTarget },
}

/// What an [`ActionsCommand::Rerun`] re-runs (Issue #3515).
///
/// The job form exists so a single flaky check can be retried without burning
/// every other job in the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsRerunTarget {
    /// A whole workflow run, or only its failed jobs when `failed_only`.
    Run { run_id: u64, failed_only: bool },
    /// One job inside a run.
    Job { job_id: u64 },
}

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
        Some("rerun") => {
            let target = match it.next().map(String::as_str) {
                Some("--run") => {
                    let run_id = super::parse_required_number(it.next())?;
                    let failed_only = matches!(it.peek().map(|arg| arg.as_str()), Some("--failed"));
                    if failed_only {
                        it.next();
                    }
                    ActionsRerunTarget::Run {
                        run_id,
                        failed_only,
                    }
                }
                Some("--job") => ActionsRerunTarget::Job {
                    job_id: super::parse_required_number(it.next())?,
                },
                _ => return Err(CliParseError::MissingFlag("--run")),
            };
            super::ensure_no_remaining_args(it)?;
            Ok(ActionsCommand::Rerun { target })
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
        ActionsCommand::Rerun { target } => {
            let outcome = env.rerun_actions(target).map_err(super::io_as_api_error)?;
            out.push_str(outcome.trim_end());
            out.push('\n');
            0
        }
    };
    Ok(code)
}

/// Human-readable name of a rerun target, used in every refusal message.
fn describe_rerun_target(target: &ActionsRerunTarget) -> String {
    match target {
        ActionsRerunTarget::Run { run_id, .. } => format!("run {run_id}"),
        ActionsRerunTarget::Job { job_id } => format!("job {job_id}"),
    }
}

/// Issue #3515 AC-2: the repository that GitHub attributes the rerun target to.
///
/// A run payload names it directly; a job payload only carries the API URL of
/// its run, so the slug is read back out of that path.
fn repo_slug_from_actions_payload(payload: &str, target: &ActionsRerunTarget) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    match target {
        ActionsRerunTarget::Run { .. } => value
            .get("repository")
            .and_then(|repository| repository.get("full_name"))
            .and_then(|full_name| full_name.as_str())
            .map(ToOwned::to_owned),
        ActionsRerunTarget::Job { .. } => {
            let run_url = value.get("run_url").and_then(|url| url.as_str())?;
            let after_repos = run_url.split_once("/repos/")?.1;
            let slug = after_repos.split_once("/actions/")?.0;
            (!slug.is_empty()).then(|| slug.to_string())
        }
    }
}

/// Issue #3515 AC-2: refuse to rerun anything the current repository does not
/// own. Fails closed when the payload does not attribute the target at all.
pub(super) fn ensure_actions_target_in_repo(
    expected_slug: &str,
    payload: &str,
    target: &ActionsRerunTarget,
) -> io::Result<()> {
    let described = describe_rerun_target(target);
    match repo_slug_from_actions_payload(payload, target) {
        Some(slug) if slug == expected_slug => Ok(()),
        Some(slug) => Err(io::Error::other(format!(
            "{described} belongs to {slug}, not {expected_slug}; refusing to rerun"
        ))),
        None => Err(io::Error::other(format!(
            "could not confirm which repository owns {described}; refusing to rerun"
        ))),
    }
}

/// Issue #3515 AC-2: a repo-scoped lookup that 404s means the id belongs to
/// some other repository (or does not exist). Anything else stays a transport
/// error so real outages are not misreported as a scope violation.
pub(super) fn classify_actions_target_lookup_failure(
    expected_slug: &str,
    target: &ActionsRerunTarget,
    stderr: &str,
) -> io::Error {
    let described = describe_rerun_target(target);
    if stderr.contains("404") || stderr.contains("Not Found") {
        io::Error::other(format!(
            "{described} does not belong to {expected_slug}; refusing to rerun"
        ))
    } else {
        io::Error::other(format!("gh api lookup for {described}: {}", stderr.trim()))
    }
}

fn gh_api(
    repo_path: &std::path::Path,
    args: &[&str],
    label: String,
) -> io::Result<(bool, String, String)> {
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        args,
        gwt_core::process_console::SpawnOptions::new(label).current_dir(repo_path),
    )?;
    Ok((output.success(), output.stdout, output.stderr))
}

/// Issue #3515: re-run a failed workflow run or a single failed job through the
/// repo-scoped Actions API, after proving the target belongs to this
/// repository.
pub(super) fn rerun_actions_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &std::path::Path,
    target: &ActionsRerunTarget,
) -> io::Result<String> {
    let slug = format!("{owner}/{repo}");
    let lookup_endpoint = match target {
        ActionsRerunTarget::Run { run_id, .. } => format!("/repos/{slug}/actions/runs/{run_id}"),
        ActionsRerunTarget::Job { job_id } => format!("/repos/{slug}/actions/jobs/{job_id}"),
    };
    let (ok, payload, stderr) = gh_api(
        repo_path,
        &["api", lookup_endpoint.as_str()],
        format!("gh api {lookup_endpoint}"),
    )?;
    if !ok {
        return Err(classify_actions_target_lookup_failure(
            &slug, target, &stderr,
        ));
    }
    ensure_actions_target_in_repo(&slug, &payload, target)?;

    let rerun_endpoint = match target {
        ActionsRerunTarget::Run {
            run_id,
            failed_only: true,
        } => format!("/repos/{slug}/actions/runs/{run_id}/rerun-failed-jobs"),
        ActionsRerunTarget::Run {
            run_id,
            failed_only: false,
        } => format!("/repos/{slug}/actions/runs/{run_id}/rerun"),
        ActionsRerunTarget::Job { job_id } => format!("/repos/{slug}/actions/jobs/{job_id}/rerun"),
    };
    let (ok, _, stderr) = gh_api(
        repo_path,
        &["api", "--method", "POST", rerun_endpoint.as_str()],
        format!("gh api --method POST {rerun_endpoint}"),
    )?;
    if !ok {
        return Err(io::Error::other(format!(
            "gh api --method POST {rerun_endpoint}: {}",
            stderr.trim()
        )));
    }

    let described = describe_rerun_target(target);
    let scope = match target {
        ActionsRerunTarget::Run {
            failed_only: true, ..
        } => " (failed jobs only)",
        _ => "",
    };
    Ok(format!("rerun requested for {described}{scope} in {slug}"))
}

pub(super) fn fetch_actions_run_log_via_gh(
    repo_path: &std::path::Path,
    run_id: u64,
) -> io::Result<String> {
    let hub = gwt_core::process_console::global();
    let run_str = run_id.to_string();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &["run", "view", run_str.as_str(), "--log"],
        gwt_core::process_console::SpawnOptions::new(format!("gh run view {run_id} --log"))
            .current_dir(repo_path),
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh run view --log: {}",
            output.stderr.trim()
        )));
    }
    Ok(output.stdout)
}

pub(super) fn fetch_actions_job_log_via_gh(
    owner: &str,
    repo: &str,
    repo_path: &std::path::Path,
    job_id: u64,
) -> io::Result<String> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/jobs/{job_id}/logs");
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &["api", endpoint.as_str()],
        gwt_core::process_console::SpawnOptions::new(format!("gh api {endpoint}"))
            .current_dir(repo_path),
    )?;
    if !output.success() {
        return Err(io::Error::other(format!(
            "gh api {endpoint}: {}",
            output.stderr.trim()
        )));
    }
    if output.stdout.as_bytes().starts_with(b"PK") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "job logs returned a zip archive; unable to parse",
        ));
    }
    Ok(output.stdout)
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

    // -- Issue #3515: actions.rerun -----------------------------------------

    #[test]
    fn actions_rerun_parses_run_job_and_failed_only_targets() {
        assert_eq!(
            parse(&[s("rerun"), s("--run"), s("90")]).expect("parse rerun --run"),
            ActionsCommand::Rerun {
                target: ActionsRerunTarget::Run {
                    run_id: 90,
                    failed_only: false
                }
            }
        );
        assert_eq!(
            parse(&[s("rerun"), s("--run"), s("90"), s("--failed")]).expect("parse rerun --failed"),
            ActionsCommand::Rerun {
                target: ActionsRerunTarget::Run {
                    run_id: 90,
                    failed_only: true
                }
            }
        );
        assert_eq!(
            parse(&[s("rerun"), s("--job"), s("91")]).expect("parse rerun --job"),
            ActionsCommand::Rerun {
                target: ActionsRerunTarget::Job { job_id: 91 }
            }
        );
    }

    #[test]
    fn actions_rerun_run_dispatches_through_the_env_seam() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            ActionsCommand::Rerun {
                target: ActionsRerunTarget::Run {
                    run_id: 90,
                    failed_only: true,
                },
            },
            &mut out,
        )
        .expect("run actions rerun");

        assert_eq!(code, 0);
        assert!(out.ends_with('\n'), "outcome line must end with newline");
        assert_eq!(
            env.rerun_call_log,
            vec![ActionsRerunTarget::Run {
                run_id: 90,
                failed_only: true
            }]
        );
    }

    #[test]
    fn actions_rerun_surfaces_the_env_rejection_for_a_foreign_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_rerun_rejection("run 777 does not belong to akiojin/gwt");

        let mut out = String::new();
        let err = run(
            &mut env,
            ActionsCommand::Rerun {
                target: ActionsRerunTarget::Run {
                    run_id: 777,
                    failed_only: false,
                },
            },
            &mut out,
        )
        .expect_err("foreign target must be refused");

        assert!(
            err.to_string().contains("does not belong to"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn actions_rerun_repo_guard_accepts_a_target_owned_by_the_current_repo() {
        ensure_actions_target_in_repo(
            "akiojin/gwt",
            r#"{"id":90,"repository":{"full_name":"akiojin/gwt"}}"#,
            &ActionsRerunTarget::Run {
                run_id: 90,
                failed_only: false,
            },
        )
        .expect("same-repo run must be accepted");

        ensure_actions_target_in_repo(
            "akiojin/gwt",
            r#"{"id":91,"run_url":"https://api.github.com/repos/akiojin/gwt/actions/runs/90"}"#,
            &ActionsRerunTarget::Job { job_id: 91 },
        )
        .expect("same-repo job must be accepted");
    }

    #[test]
    fn actions_rerun_repo_guard_refuses_a_target_owned_by_another_repo() {
        let err = ensure_actions_target_in_repo(
            "akiojin/gwt",
            r#"{"id":90,"repository":{"full_name":"someone/other"}}"#,
            &ActionsRerunTarget::Run {
                run_id: 90,
                failed_only: false,
            },
        )
        .expect_err("cross-repo run must be refused");
        assert!(
            err.to_string().contains("someone/other") && err.to_string().contains("run 90"),
            "unexpected error: {err}"
        );

        let err = ensure_actions_target_in_repo(
            "akiojin/gwt",
            r#"{"id":91,"run_url":"https://api.github.com/repos/someone/other/actions/runs/90"}"#,
            &ActionsRerunTarget::Job { job_id: 91 },
        )
        .expect_err("cross-repo job must be refused");
        assert!(
            err.to_string().contains("job 91"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn actions_rerun_repo_guard_fails_closed_when_the_payload_names_no_repository() {
        let err = ensure_actions_target_in_repo(
            "akiojin/gwt",
            r#"{"id":90}"#,
            &ActionsRerunTarget::Run {
                run_id: 90,
                failed_only: false,
            },
        )
        .expect_err("an unattributable payload must be refused");
        assert!(
            err.to_string().contains("could not confirm"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn actions_rerun_lookup_404_is_reported_as_a_foreign_target() {
        let err = classify_actions_target_lookup_failure(
            "akiojin/gwt",
            &ActionsRerunTarget::Run {
                run_id: 777,
                failed_only: false,
            },
            "gh: Not Found (HTTP 404)",
        );
        assert!(
            err.to_string()
                .contains("run 777 does not belong to akiojin/gwt"),
            "unexpected error: {err}"
        );

        let err = classify_actions_target_lookup_failure(
            "akiojin/gwt",
            &ActionsRerunTarget::Job { job_id: 91 },
            "gh: API rate limit exceeded (HTTP 403)",
        );
        assert!(
            !err.to_string().contains("does not belong to"),
            "a non-404 failure must stay a transport error: {err}"
        );
    }
}

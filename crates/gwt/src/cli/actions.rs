use std::io;

use gwt_github::SpecOpsError;

use crate::cli::{CliEnv, CliParseError};

/// SPEC-1942 command model for `actions.*` JSON operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionsCommand {
    /// `actions.logs`.
    Logs { run_id: u64 },
    /// `actions.job_logs`. Issue #4849: `failed_only` returns only the lines
    /// around failure markers (`FAILED`, `panicked at`, `error:`, `error[E`,
    /// `##[error]`) with `context_lines` of context on each side.
    JobLogs {
        job_id: u64,
        failed_only: bool,
        context_lines: usize,
    },
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
            let mut failed_only = false;
            let mut context_lines = DEFAULT_FAILURE_CONTEXT_LINES;
            while let Some(flag) = it.peek().map(|arg| arg.as_str()) {
                match flag {
                    "--failed-only" => {
                        it.next();
                        failed_only = true;
                    }
                    "--context" => {
                        it.next();
                        context_lines =
                            clamp_failure_context_lines(super::parse_required_number(it.next())?);
                    }
                    _ => break,
                }
            }
            super::ensure_no_remaining_args(it)?;
            Ok(ActionsCommand::JobLogs {
                job_id,
                failed_only,
                context_lines,
            })
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
            // Issue #4849: colour codes never reach the caller.
            let log = strip_ansi_escapes(&log);
            out.push_str(&log);
            if !log.ends_with('\n') {
                out.push('\n');
            }
            0
        }
        ActionsCommand::JobLogs {
            job_id,
            failed_only,
            context_lines,
        } => {
            let log = env
                .fetch_actions_job_log(job_id)
                .map_err(super::io_as_api_error)?;
            // Issue #4849 AC-1: GitHub job logs carry the runner's ANSI colour
            // codes (every cargo test job); they are stripped here, server
            // side, so no caller has to handle them.
            let log = strip_ansi_escapes(&log);
            let log = if failed_only {
                failure_view(&log, context_lines)
            } else {
                log
            };
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

/// Issue #4849 AC-2: context on each side of a failure marker by default.
pub(crate) const DEFAULT_FAILURE_CONTEXT_LINES: usize = 5;
const MAX_FAILURE_CONTEXT_LINES: usize = 50;
/// Lines of tail shown when `failed_only` finds no marker, so the caller
/// still sees how the job ended.
const FAILURE_VIEW_FALLBACK_TAIL_LINES: usize = 40;

pub(crate) fn clamp_failure_context_lines(requested: u64) -> usize {
    usize::try_from(requested)
        .unwrap_or(MAX_FAILURE_CONTEXT_LINES)
        .min(MAX_FAILURE_CONTEXT_LINES)
}

/// Issue #4849 AC-1: drop terminal escape sequences from a log.
///
/// Handles the forms GitHub runner logs actually contain: CSI (`ESC [ … <final>`,
/// colour `m`, erase `K`, cursor moves), OSC (`ESC ] … BEL` / `ESC ] … ESC \`),
/// and two-byte `ESC <char>` sequences. Text outside escapes is kept
/// byte-for-byte; a lone `ESC` without a recognised introducer is dropped.
pub(crate) fn strip_ansi_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                // Parameter / intermediate bytes 0x30–0x3F / 0x20–0x2F, then a
                // final byte 0x40–0x7E.
                for next in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if next == '\x07' || (previous == '\x1b' && next == '\\') {
                        break;
                    }
                    previous = next;
                }
            }
            // nF (`ESC ( B` charset selection): intermediates 0x20–0x2F,
            // then one final byte 0x30–0x7E.
            Some(next) if ('\x20'..='\x2f').contains(&next) => {
                for next in chars.by_ref() {
                    if !('\x20'..='\x2f').contains(&next) {
                        break;
                    }
                }
            }
            // Fp / Fs / Fe two-byte escapes (`ESC =`, `ESC 7`, `ESC M`, …).
            Some(next) if ('\x30'..='\x5f').contains(&next) => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}

/// Issue #4849 AC-2: whether a log line marks a failure worth reading.
fn is_failure_marker(line: &str) -> bool {
    line.contains("FAILED")
        || line.contains("panicked at")
        || line.contains("error:")
        || line.contains("error[E")
        || line.contains("##[error]")
}

/// Issue #4849 AC-2: only the lines around failure markers, each block
/// separated by `--`, so a red job can be read without the whole log. Without
/// any marker the last [`FAILURE_VIEW_FALLBACK_TAIL_LINES`] lines are shown
/// after a note saying no marker was found.
pub(crate) fn failure_view(log: &str, context_lines: usize) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !is_failure_marker(line) {
            continue;
        }
        let start = index.saturating_sub(context_lines);
        let end = (index + context_lines).min(lines.len().saturating_sub(1));
        match ranges.last_mut() {
            // Overlapping or adjacent windows merge into one block.
            Some((_, last_end)) if start <= *last_end + 1 => *last_end = (*last_end).max(end),
            _ => ranges.push((start, end)),
        }
    }
    if ranges.is_empty() {
        let start = lines.len().saturating_sub(FAILURE_VIEW_FALLBACK_TAIL_LINES);
        let mut out = format!(
            "no failure marker (FAILED / panicked at / error: / error[E / ##[error]) in {} lines; last {} lines follow\n",
            lines.len(),
            lines.len() - start
        );
        for line in &lines[start..] {
            out.push_str(line);
            out.push('\n');
        }
        return out;
    }
    let mut out = String::new();
    for (block, (start, end)) in ranges.iter().enumerate() {
        if block > 0 {
            out.push_str("--\n");
        }
        for (offset, line) in lines[*start..=*end].iter().enumerate() {
            out.push_str(&format!("{:>6}: {line}\n", start + offset + 1));
        }
    }
    out
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
    // Issue #4849 AC-1: current `gh` refuses to print a response that carries
    // terminal escape sequences unless told otherwise, and a job log with
    // colour codes is exactly that. The sequences are stripped by the caller
    // before anything leaves the operation.
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Gh,
        "gh",
        &["api", "--allow-escape-sequences", endpoint.as_str()],
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

    /// Issue #4849 AC-1/AC-3: a job log with ANSI colour codes and a FAILED
    /// block comes back clean, and the failures view carries the panic line.
    #[test]
    fn issue_4849_job_logs_strip_ansi_and_offer_a_failures_view() {
        let fixture = concat!(
            "2026-10-01T00:00:00Z \x1b[?25l\x1b[1mcargo test\x1b[0m\n",
            "running 3 tests\n",
            "test a::passes ... \x1b[32mok\x1b[0m\n",
            "test b::fails ... \x1b[31mFAILED\x1b[0m\n",
            "test c::passes ... \x1b[32mok\x1b[0m\n",
            "\n",
            "failures:\n",
            "\n",
            "---- b::fails stdout ----\n",
            "thread 'b::fails' panicked at crates/x/src/lib.rs:10:5:\n",
            "assertion failed: left == right\x1b[K\n",
            "\x1b]8;;https://example.invalid\x07link\x1b]8;;\x07\n",
            "line 13\nline 14\nline 15\nline 16\nline 17\nline 18\nline 19\nline 20\n",
            "##[error]Process completed with exit code 101.\n",
            "line 22\n",
        );
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        env.seed_job_log(7, fixture);

        let mut out = String::new();
        let code = run(
            &mut env,
            ActionsCommand::JobLogs {
                job_id: 7,
                failed_only: false,
                context_lines: DEFAULT_FAILURE_CONTEXT_LINES,
            },
            &mut out,
        )
        .expect("job logs with colour codes succeed");
        assert_eq!(code, 0);
        assert!(
            !out.contains('\x1b'),
            "no escape sequence leaves the operation: {out:?}"
        );
        assert!(out.contains("test b::fails ... FAILED\n"), "{out}");
        assert!(out.contains("assertion failed: left == right\n"), "{out}");
        assert!(out.contains("link\n"), "OSC hyperlink text is kept: {out}");
        assert!(out.contains("cargo test\n"), "{out}");
        assert_eq!(out.lines().count(), fixture.lines().count());

        let mut focused = String::new();
        run(
            &mut env,
            ActionsCommand::JobLogs {
                job_id: 7,
                failed_only: true,
                context_lines: 1,
            },
            &mut focused,
        )
        .expect("failures view");
        assert!(!focused.contains('\x1b'), "{focused:?}");
        assert!(focused.contains("test b::fails ... FAILED"), "{focused}");
        assert!(
            focused.contains("panicked at crates/x/src/lib.rs:10:5"),
            "the panic line is in the failures view: {focused}"
        );
        assert!(focused.contains("##[error]Process completed"), "{focused}");
        assert!(
            !focused.contains("line 16"),
            "lines far from any marker are left out: {focused}"
        );
        assert!(focused.contains("--\n"), "blocks are separated: {focused}");
        assert!(focused.lines().count() < out.lines().count());
    }

    /// Issue #4849 AC-2: a log without any marker still shows its tail, and
    /// the escape stripper handles the forms GitHub logs contain.
    #[test]
    fn issue_4849_failure_view_without_markers_shows_the_tail_and_stripper_forms() {
        let view = failure_view("one\ntwo\nthree\n", 5);
        assert!(view.starts_with("no failure marker"), "{view}");
        assert!(view.ends_with("one\ntwo\nthree\n"), "{view}");

        assert_eq!(
            strip_ansi_escapes("\x1b[31;1mred\x1b[0m plain"),
            "red plain"
        );
        assert_eq!(strip_ansi_escapes("a\x1b[2K\x1b[1Gb"), "ab");
        assert_eq!(strip_ansi_escapes("x\x1b]0;title\x1b\\y"), "xy");
        assert_eq!(strip_ansi_escapes("p\x1b(Bq\x1b=r"), "pqr");
        assert_eq!(strip_ansi_escapes("no escapes"), "no escapes");
        assert_eq!(clamp_failure_context_lines(500), MAX_FAILURE_CONTEXT_LINES);
        assert_eq!(clamp_failure_context_lines(3), 3);
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

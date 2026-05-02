//! `gwtd daemon ...` family — long-running runtime daemon (SPEC-2077).
//!
//! - `mod.rs` (this file): argv parsing + dispatch + status reporting.
//! - `server.rs`: tokio-based IPC listener (Unix domain socket today;
//!   Windows named-pipe support is a follow-up).
//!
//! The contract layer (`gwt_core::daemon::*`) defines the on-disk endpoint
//! file, handshake protocol, and `DaemonBootstrapAction`. `Start` honours
//! that contract: if a usable endpoint already exists for the cwd
//! [`RuntimeScope`], we exit 0 with a "already running" notice; otherwise
//! we generate a fresh `auth_token`, persist a new [`DaemonEndpoint`], and
//! enter the listen loop.

#[cfg(unix)]
mod server;

use std::path::PathBuf;

use gwt_core::daemon::{
    resolve_bootstrap_action, DaemonBootstrapAction, RuntimeScope, RuntimeTarget,
    DAEMON_PROTOCOL_VERSION,
};
use gwt_github::{client::ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, DaemonCommand};

pub(super) fn parse(args: &[String]) -> Result<DaemonCommand, CliParseError> {
    match args.first().map(String::as_str) {
        None | Some("start") => {
            ensure_no_extra_args(args.get(1..).unwrap_or(&[]))?;
            Ok(DaemonCommand::Start)
        }
        Some("status") => {
            ensure_no_extra_args(args.get(1..).unwrap_or(&[]))?;
            Ok(DaemonCommand::Status)
        }
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn ensure_no_extra_args(rest: &[String]) -> Result<(), CliParseError> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(CliParseError::Usage)
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: DaemonCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        DaemonCommand::Start => start_daemon(env, out),
        DaemonCommand::Status => report_status(env, out),
    }
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Unexpected(message.into()))
}

fn resolve_scope(env: &impl CliEnv) -> Result<RuntimeScope, SpecOpsError> {
    let project_root = canonical_project_root(env.repo_path().to_path_buf());
    RuntimeScope::from_project_root(&project_root, RuntimeTarget::Host)
        .map_err(|err| config_error(format!("daemon scope resolution failed: {err}")))
}

fn canonical_project_root(path: PathBuf) -> PathBuf {
    dunce::canonicalize(&path).unwrap_or(path)
}

fn report_status<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = resolve_bootstrap_action(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )
    .map_err(|err| config_error(err.to_string()))?;

    match action {
        DaemonBootstrapAction::Reuse(endpoint) => {
            out.push_str(&format!(
                "running pid={pid} bind={bind} version={version}\n",
                pid = endpoint.pid,
                bind = endpoint.bind,
                version = endpoint.daemon_version
            ));
            Ok(0)
        }
        DaemonBootstrapAction::Spawn { endpoint_path } => {
            out.push_str(&format!(
                "stopped scope={repo_hash}/{worktree_hash} endpoint={path}\n",
                repo_hash = scope.repo_hash,
                worktree_hash = scope.worktree_hash,
                path = endpoint_path.display()
            ));
            Ok(0)
        }
    }
}

#[cfg(unix)]
fn start_daemon<E: CliEnv>(env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    let scope = resolve_scope(env)?;
    let gwt_home = gwt_core::paths::gwt_home();
    let action = resolve_bootstrap_action(
        &gwt_home,
        &scope,
        DAEMON_PROTOCOL_VERSION,
        is_process_alive_pid,
    )
    .map_err(|err| config_error(err.to_string()))?;

    match action {
        DaemonBootstrapAction::Reuse(endpoint) => {
            out.push_str(&format!(
                "daemon already running pid={pid} bind={bind}\n",
                pid = endpoint.pid,
                bind = endpoint.bind
            ));
            Ok(0)
        }
        DaemonBootstrapAction::Spawn { endpoint_path } => {
            server::serve_blocking(scope, endpoint_path, out)
        }
    }
}

#[cfg(not(unix))]
fn start_daemon<E: CliEnv>(_env: &mut E, out: &mut String) -> Result<i32, SpecOpsError> {
    out.push_str(
        "gwtd daemon start: long-running daemon mode is not yet implemented on this platform; \
         use `gwt hook ...` synchronous dispatch.\n",
    );
    Ok(2)
}

fn is_process_alive_pid(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) returns 0 if the process exists, -1 with
        // ESRCH if it does not. We never deliver a real signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        // EPERM means the process exists but we lack permission to signal it.
        matches!(err.raw_os_error(), Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        // Conservative fallback — assume alive so we never racily clobber
        // an in-flight daemon endpoint on an unsupported platform.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn parse_defaults_to_start_when_no_subcommand() {
        let cmd = parse(&[]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Start);
    }

    #[test]
    fn parse_recognises_start_explicitly() {
        let cmd = parse(&[s("start")]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Start);
    }

    #[test]
    fn parse_recognises_status() {
        let cmd = parse(&[s("status")]).expect("parse");
        assert_eq!(cmd, DaemonCommand::Status);
    }

    #[test]
    fn parse_rejects_unknown_subcommand() {
        let err = parse(&[s("foo")]).unwrap_err();
        assert!(matches!(err, CliParseError::UnknownSubcommand(_)));
    }

    #[test]
    fn parse_rejects_extra_args() {
        let err = parse(&[s("start"), s("--whatever")]).unwrap_err();
        assert!(matches!(err, CliParseError::Usage));
    }
}

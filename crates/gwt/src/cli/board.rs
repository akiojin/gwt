use std::io;

use gwt_agent::{session::GWT_SESSION_ID_ENV, Session};
use gwt_core::{
    coordination::{load_snapshot, post_entry, AuthorKind, BoardEntry},
    paths::gwt_sessions_dir,
};
use gwt_github::SpecOpsError;

use crate::cli::{BoardCommand, CliEnv, CliParseError};

pub(crate) fn parse(args: &[String]) -> Result<BoardCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("show") => {
            let mut json = false;
            for arg in it {
                match arg.as_str() {
                    "--json" => json = true,
                    other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
                }
            }
            Ok(BoardCommand::Show { json })
        }
        Some("post") => parse_post_args(it.collect::<Vec<_>>().as_slice()),
        Some(other) => Err(CliParseError::UnknownSubcommand(other.to_string())),
        None => Err(CliParseError::Usage),
    }
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: BoardCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    let code = match cmd {
        BoardCommand::Show { json } => {
            let snapshot = load_snapshot(env.repo_path()).map_err(gwt_error_to_spec_ops_error)?;
            if json {
                let rendered = serde_json::to_string_pretty(&snapshot)
                    .map_err(|err| io_as_spec_ops_error(io::Error::other(err.to_string())))?;
                out.push_str(&rendered);
                out.push('\n');
            } else {
                render_snapshot(out, &snapshot);
            }
            0
        }
        BoardCommand::Post {
            kind,
            body,
            file,
            parent,
            topics,
            owners,
            targets,
        } => {
            let body = match (body, file) {
                (Some(body), None) => body,
                (None, Some(file)) => env.read_file(&file).map_err(io_as_spec_ops_error)?,
                _ => {
                    return Err(io_as_spec_ops_error(io::Error::other(
                        "board post requires exactly one of --body or -f",
                    )));
                }
            };
            let (author_kind, author) =
                current_author_from_env().unwrap_or((AuthorKind::User, "user".to_string()));
            let mut entry = BoardEntry::new(
                author_kind,
                author,
                kind.parse().map_err(gwt_error_to_spec_ops_error)?,
                body,
                None,
                parent,
                topics,
                owners,
            );
            if !targets.is_empty() {
                entry = entry.with_target_owners(targets);
            }
            let snapshot =
                post_entry(env.repo_path(), entry).map_err(gwt_error_to_spec_ops_error)?;
            publish_board_change(env.repo_path(), snapshot.board.entries.len());
            out.push_str(&format!(
                "board entries: {}\n",
                snapshot.board.entries.len()
            ));
            0
        }
    };
    Ok(code)
}

/// Best-effort daemon broadcast after a CLI `gwtd board post` succeeds
/// (SPEC-2077 Phase H1 GREEN). Mirrors the GUI handler in
/// `app_runtime/board.rs`: notify subscribers via the daemon so other
/// gwt instances on the same project see the new entry without
/// waiting for their file watcher to fire. Errors are logged at debug
/// level and ignored — local file is the source of truth.
#[cfg(unix)]
fn publish_board_change(project_root: &std::path::Path, entries_count: usize) {
    // CLI path runs in a short-lived process: `gwtd board post`
    // returns to the shell immediately, so a detached publish thread
    // would be killed before it finishes the connect/publish/ack
    // round-trip (the daemon would then never see the broadcast).
    // The publish is bounded by `daemon_publisher::publish_event`'s
    // per-stage timeout (~200 ms, ~400 ms worst case), which is an
    // acceptable amount of synchronous wall time for a CLI command.
    let result = crate::daemon_publisher::publish_event(
        project_root,
        "board",
        serde_json::json!({"entries_count": entries_count}),
    );
    if let Err(err) = result {
        tracing::debug!(
            error = %err,
            project_root = %project_root.display(),
            entries_count,
            "gwtd board post: daemon publish failed (non-fatal)"
        );
    }
}

#[cfg(not(unix))]
fn publish_board_change(_project_root: &std::path::Path, _entries_count: usize) {
    // Daemon publishing is gated on Unix; CLI continues to drive the
    // local file path on other platforms.
}

fn parse_post_args(args: &[&String]) -> Result<BoardCommand, CliParseError> {
    let mut kind: Option<String> = None;
    let mut body: Option<String> = None;
    let mut file: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut topics = Vec::new();
    let mut owners = Vec::new();
    let mut targets = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--kind" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--kind"));
                }
                kind = Some(args[i].clone());
            }
            "--body" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--body"));
                }
                body = Some(args[i].clone());
            }
            "-f" | "--file" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("-f"));
                }
                file = Some(args[i].clone());
            }
            "--parent" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--parent"));
                }
                parent = Some(args[i].clone());
            }
            "--topic" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--topic"));
                }
                topics.push(args[i].clone());
            }
            "--owner" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--owner"));
                }
                owners.push(args[i].clone());
            }
            "--target" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--target"));
                }
                targets.push(args[i].clone());
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    Ok(BoardCommand::Post {
        kind: kind.ok_or(CliParseError::MissingFlag("--kind"))?,
        body,
        file,
        parent,
        topics,
        owners,
        targets,
    })
}

fn current_author_from_env() -> Option<(AuthorKind, String)> {
    let session = current_session_from_env().ok().flatten()?;
    Some((AuthorKind::Agent, session.display_name))
}

fn current_session_from_env() -> io::Result<Option<Session>> {
    let Some(session_id) = std::env::var_os(GWT_SESSION_ID_ENV) else {
        return Ok(None);
    };
    let path = gwt_sessions_dir().join(format!("{}.toml", session_id.to_string_lossy()));
    if !path.exists() {
        return Ok(None);
    }
    Session::load_and_migrate(&path).map(Some)
}

fn render_snapshot(out: &mut String, snapshot: &gwt_core::coordination::CoordinationSnapshot) {
    out.push_str("== Chat ==\n");
    if snapshot.board.entries.is_empty() {
        out.push_str("no chat messages\n");
    } else {
        for entry in &snapshot.board.entries {
            out.push_str(&format!(
                "- [{}] {}: {} ({})\n",
                entry.kind.as_str(),
                format_author(entry),
                entry.body,
                entry.id
            ));
        }
    }
}

/// Format the author header with optional `origin_branch` /
/// `origin_session_id` suffix (SPEC-1974 FR-020). Entries without origin
/// metadata fall back to bare author, preserving legacy render output.
fn format_author(entry: &BoardEntry) -> String {
    let branch = entry
        .origin_branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let session = entry
        .origin_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (branch, session) {
        (Some(branch), Some(session)) => {
            format!("{} @ {} / {}", entry.author, branch, session)
        }
        (Some(branch), None) => format!("{} @ {}", entry.author, branch),
        (None, Some(session)) => format!("{} / {}", entry.author, session),
        (None, None) => entry.author.clone(),
    }
}

fn io_as_spec_ops_error(err: io::Error) -> SpecOpsError {
    SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
}

fn gwt_error_to_spec_ops_error(err: gwt_core::GwtError) -> SpecOpsError {
    SpecOpsError::from(gwt_github::client::ApiError::Network(err.to_string()))
}

#[cfg(test)]
mod tests {
    use gwt_core::coordination::BoardEntryKind;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn board_family_parse_show_json() {
        let cmd = parse(&[s("show"), s("--json")]).unwrap();
        assert_eq!(cmd, BoardCommand::Show { json: true });
    }

    #[test]
    fn board_family_parse_post() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("request"),
            s("--body"),
            s("hello"),
            s("--topic"),
            s("coordination"),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            BoardCommand::Post {
                kind: "request".into(),
                body: Some("hello".into()),
                file: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec![],
                targets: vec![],
            }
        );
    }

    #[test]
    fn board_family_parse_post_collects_target_flags() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("claim"),
            s("--body"),
            s("I claim feature/foo"),
            s("--target"),
            s("sess-a3f2"),
            s("--target"),
            s("feature/foo"),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            BoardCommand::Post {
                kind: "claim".into(),
                body: Some("I claim feature/foo".into()),
                file: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec!["sess-a3f2".into(), "feature/foo".into()],
            }
        );
    }

    #[test]
    fn board_family_run_post_persists_target_owners() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post {
                kind: "claim".into(),
                body: Some("taking the migration".into()),
                file: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec!["sess-a3f2".into(), "feature/x".into()],
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        assert_eq!(
            snapshot.board.entries[0].target_owners,
            vec!["sess-a3f2".to_string(), "feature/x".to_string()]
        );
    }

    #[test]
    fn board_family_rejects_card_subcommand() {
        let err = parse(&[s("card"), s("set"), s("--status"), s("running")]).unwrap_err();
        assert_eq!(err, CliParseError::UnknownSubcommand("card".into()));
    }

    #[test]
    fn board_family_run_post_updates_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post {
                kind: "request".into(),
                body: Some("Need a board".into()),
                file: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec!["1974".into()],
                targets: vec![],
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        assert_eq!(snapshot.board.entries[0].body, "Need a board");
        assert!(out.contains("board entries: 1"));
    }

    #[test]
    fn board_family_run_show_renders_origin_metadata_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        post_entry(
            tmp.path(),
            BoardEntry::new(
                AuthorKind::Agent,
                "Claude",
                BoardEntryKind::Status,
                "Investigating",
                None,
                None,
                vec![],
                vec![],
            )
            .with_origin_branch("feature/foo")
            .with_origin_session_id("sess-a3f2"),
        )
        .unwrap();

        let mut out = String::new();
        let code = run(&mut env, BoardCommand::Show { json: false }, &mut out).unwrap();

        assert_eq!(code, 0);
        assert!(
            out.contains("Claude @ feature/foo / sess-a3f2"),
            "expected origin metadata suffix, got:\n{out}"
        );
    }

    #[test]
    fn board_family_run_show_falls_back_to_author_without_origin_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        post_entry(
            tmp.path(),
            BoardEntry::new(
                AuthorKind::User,
                "user",
                BoardEntryKind::Request,
                "legacy entry",
                None,
                None,
                vec![],
                vec![],
            ),
        )
        .unwrap();

        let mut out = String::new();
        let code = run(&mut env, BoardCommand::Show { json: false }, &mut out).unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("user: legacy entry"));
        assert!(!out.contains(" @ "));
        assert!(!out.contains(" / "));
    }

    #[test]
    fn board_family_run_show_renders_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        post_entry(
            tmp.path(),
            BoardEntry::new(
                AuthorKind::User,
                "user",
                BoardEntryKind::Request,
                "Need a board",
                None,
                None,
                vec!["coordination".into()],
                vec!["1974".into()],
            ),
        )
        .unwrap();

        let mut out = String::new();
        let code = run(&mut env, BoardCommand::Show { json: false }, &mut out).unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("== Chat =="));
        assert!(out.contains("Need a board"));
        assert!(!out.contains("== Cards =="));
        assert!(!out.contains("no agent cards"));
    }
}

use std::io;

use gwt_agent::{session::GWT_SESSION_ID_ENV, Session};
use gwt_core::coordination::{load_snapshot, post_entry, AuthorKind, BoardEntry};
use gwt_core::paths::gwt_sessions_dir;
use gwt_github::SpecOpsError;

use crate::cli::{CliCommand, CliEnv, CliParseError};

pub(crate) fn parse(args: &[String]) -> Result<CliCommand, CliParseError> {
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
            Ok(CliCommand::BoardShow { json })
        }
        Some("post") => parse_post_args(it.collect::<Vec<_>>().as_slice()),
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
        CliCommand::BoardShow { json } => {
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
        CliCommand::BoardPost {
            kind,
            body,
            file,
            parent,
            topics,
            owners,
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
            let entry = BoardEntry::new(
                author_kind,
                author,
                kind.parse().map_err(gwt_error_to_spec_ops_error)?,
                body,
                None,
                parent,
                topics,
                owners,
            );
            let snapshot =
                post_entry(env.repo_path(), entry).map_err(gwt_error_to_spec_ops_error)?;
            out.push_str(&format!(
                "board entries: {}\n",
                snapshot.board.entries.len()
            ));
            0
        }
        _ => unreachable!("board::run called with non-board command"),
    };
    Ok(code)
}

fn parse_post_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    let mut kind: Option<String> = None;
    let mut body: Option<String> = None;
    let mut file: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut topics = Vec::new();
    let mut owners = Vec::new();
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
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }

    Ok(CliCommand::BoardPost {
        kind: kind.ok_or(CliParseError::MissingFlag("--kind"))?,
        body,
        file,
        parent,
        topics,
        owners,
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
    Session::load(&path).map(Some)
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
                entry.author,
                entry.body,
                entry.id
            ));
        }
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
    use super::*;
    use gwt_core::coordination::BoardEntryKind;

    fn s(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn board_family_parse_show_json() {
        let cmd = parse(&[s("show"), s("--json")]).unwrap();
        assert_eq!(cmd, CliCommand::BoardShow { json: true });
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
            CliCommand::BoardPost {
                kind: "request".into(),
                body: Some("hello".into()),
                file: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec![],
            }
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
            CliCommand::BoardPost {
                kind: "request".into(),
                body: Some("Need a board".into()),
                file: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec!["1974".into()],
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
        let code = run(&mut env, CliCommand::BoardShow { json: false }, &mut out).unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("== Chat =="));
        assert!(out.contains("Need a board"));
        assert!(!out.contains("== Cards =="));
        assert!(!out.contains("no agent cards"));
    }
}

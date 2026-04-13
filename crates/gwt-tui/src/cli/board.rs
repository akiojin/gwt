use std::io;

use gwt_agent::{session::GWT_SESSION_ID_ENV, Session};
use gwt_core::coordination::{
    apply_agent_card_patch, load_snapshot, post_entry, AgentCardContext, AgentCardPatch,
    AuthorKind, BoardEntry,
};
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
        Some("card") => parse_card_args(it.collect::<Vec<_>>().as_slice()),
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
        CliCommand::BoardCardSet {
            status,
            role,
            responsibility,
            current_focus,
            next_action,
            blocked_reason,
            topics,
            owners,
            working_scope,
            handoff_target,
            agent_id,
            session_id,
            branch,
        } => {
            let inferred = current_agent_context_from_env().map_err(io_as_spec_ops_error)?;
            let agent_id = agent_id
                .or_else(|| inferred.as_ref().map(|context| context.agent_id.clone()))
                .ok_or_else(|| {
                    io_as_spec_ops_error(io::Error::other(
                        "board card set requires --agent-id or an active gwt agent session",
                    ))
                })?;
            let branch = branch
                .or_else(|| inferred.as_ref().map(|context| context.branch.clone()))
                .unwrap_or_default();
            let session_id = session_id.or_else(|| inferred.and_then(|context| context.session_id));

            let snapshot = apply_agent_card_patch(
                env.repo_path(),
                AgentCardContext {
                    agent_id,
                    session_id,
                    branch,
                },
                AgentCardPatch {
                    role,
                    responsibility,
                    status: Some(status),
                    current_focus,
                    next_action,
                    blocked_reason,
                    related_topics: (!topics.is_empty()).then_some(topics),
                    related_owners: (!owners.is_empty()).then_some(owners),
                    working_scope,
                    handoff_target,
                },
            )
            .map_err(gwt_error_to_spec_ops_error)?;
            out.push_str(&format!("agent cards: {}\n", snapshot.cards.cards.len()));
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

fn parse_card_args(args: &[&String]) -> Result<CliCommand, CliParseError> {
    if args.first().map(|arg| arg.as_str()) != Some("set") {
        return Err(CliParseError::Usage);
    }

    let mut status: Option<String> = None;
    let mut role: Option<String> = None;
    let mut responsibility: Option<String> = None;
    let mut current_focus: Option<String> = None;
    let mut next_action: Option<String> = None;
    let mut blocked_reason: Option<String> = None;
    let mut topics = Vec::new();
    let mut owners = Vec::new();
    let mut working_scope: Option<String> = None;
    let mut handoff_target: Option<String> = None;
    let mut agent_id: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--status" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--status"));
                }
                status = Some(args[i].clone());
            }
            "--role" => assign_arg(args, &mut i, "--role", &mut role)?,
            "--responsibility" => {
                assign_arg(args, &mut i, "--responsibility", &mut responsibility)?
            }
            "--current-focus" => assign_arg(args, &mut i, "--current-focus", &mut current_focus)?,
            "--next-action" => assign_arg(args, &mut i, "--next-action", &mut next_action)?,
            "--blocked-reason" => {
                assign_arg(args, &mut i, "--blocked-reason", &mut blocked_reason)?
            }
            "--working-scope" => assign_arg(args, &mut i, "--working-scope", &mut working_scope)?,
            "--handoff-target" => {
                assign_arg(args, &mut i, "--handoff-target", &mut handoff_target)?
            }
            "--agent-id" => assign_arg(args, &mut i, "--agent-id", &mut agent_id)?,
            "--session-id" => assign_arg(args, &mut i, "--session-id", &mut session_id)?,
            "--branch" => assign_arg(args, &mut i, "--branch", &mut branch)?,
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

    Ok(CliCommand::BoardCardSet {
        status: status.ok_or(CliParseError::MissingFlag("--status"))?,
        role,
        responsibility,
        current_focus,
        next_action,
        blocked_reason,
        topics,
        owners,
        working_scope,
        handoff_target,
        agent_id,
        session_id,
        branch,
    })
}

fn assign_arg(
    args: &[&String],
    index: &mut usize,
    flag: &'static str,
    slot: &mut Option<String>,
) -> Result<(), CliParseError> {
    *index += 1;
    if *index >= args.len() {
        return Err(CliParseError::MissingFlag(flag));
    }
    *slot = Some(args[*index].clone());
    Ok(())
}

fn current_author_from_env() -> Option<(AuthorKind, String)> {
    let session = current_session_from_env().ok().flatten()?;
    Some((AuthorKind::Agent, session.display_name))
}

fn current_agent_context_from_env() -> io::Result<Option<AgentCardContext>> {
    let Some(session) = current_session_from_env()? else {
        return Ok(None);
    };
    Ok(Some(AgentCardContext {
        agent_id: session.display_name,
        session_id: Some(session.id),
        branch: session.branch,
    }))
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
    out.push_str("== Board ==\n");
    if snapshot.board.entries.is_empty() {
        out.push_str("no board entries\n");
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

    out.push_str("\n== Cards ==\n");
    if snapshot.cards.cards.is_empty() {
        out.push_str("no agent cards\n");
    } else {
        for card in &snapshot.cards.cards {
            out.push_str(&format!(
                "- {} [{}] {}\n",
                card.agent_id,
                card.status.as_deref().unwrap_or("unknown"),
                card.branch
            ));
            if let Some(focus) = card.current_focus.as_deref() {
                out.push_str(&format!("  focus: {focus}\n"));
            }
            if let Some(next) = card.next_action.as_deref() {
                out.push_str(&format!("  next: {next}\n"));
            }
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
        assert!(out.contains("== Board =="));
        assert!(out.contains("Need a board"));
    }

    #[test]
    fn board_family_run_card_set_updates_card() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            CliCommand::BoardCardSet {
                status: "running".into(),
                role: Some("implementer".into()),
                responsibility: None,
                current_focus: Some("Storage".into()),
                next_action: Some("Add watcher".into()),
                blocked_reason: None,
                topics: vec!["coordination".into()],
                owners: vec!["1974".into()],
                working_scope: None,
                handoff_target: None,
                agent_id: Some("Codex".into()),
                session_id: Some("sess-1".into()),
                branch: Some("feature/coordination".into()),
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        assert_eq!(snapshot.cards.cards.len(), 1);
        assert_eq!(snapshot.cards.cards[0].status.as_deref(), Some("running"));
        assert_eq!(snapshot.cards.cards[0].role.as_deref(), Some("implementer"));
        assert!(out.contains("agent cards: 1"));
    }
}

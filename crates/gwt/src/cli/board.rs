use std::io;

use gwt_agent::{session::GWT_SESSION_ID_ENV, Session};
use gwt_core::{
    coordination::{
        normalize_board_mentions, AuthorKind, BoardAudienceScope, BoardEntry, BoardEntryDraft,
        BoardMention, BoardOrigin,
    },
    paths::gwt_sessions_dir,
};
use gwt_github::SpecOpsError;

use crate::{
    board_audience::{
        current_session_board_scope, gui_default_board_scope, post_audience_for_session,
    },
    board_provider::{load_snapshot, load_snapshot_for_scope, post_entry},
    cli::{CliEnv, CliParseError},
};

/// SPEC-1942 family enum for `gwtd board ...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardCommand {
    /// `gwtd board show [--json] [--workspace <id>|--all]`.
    Show {
        json: bool,
        workspace: Option<String>,
        all: bool,
    },
    /// `gwtd board post --kind <kind> (--body <text> | -f <file>)
    /// [--title <text>] [--title-summary <text>] [--parent <id>] [--topic <t>]*
    /// [--owner <n>]* [--target <id>]* [--mention <kind:id>]*`.
    Post(Box<BoardPostCommand>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardPostCommand {
    pub kind: String,
    pub body: Option<String>,
    pub file: Option<String>,
    /// SPEC-2963: optional post title/subject (rendered as the Teams subject /
    /// Slack header block / board card heading). Distinct from `title_summary`,
    /// which is the short agent window-title label.
    pub title: Option<String>,
    pub title_summary: Option<String>,
    pub parent: Option<String>,
    pub topics: Vec<String>,
    pub owners: Vec<String>,
    pub targets: Vec<String>,
    pub mentions: Vec<String>,
    pub broadcast: bool,
}

pub fn parse(args: &[String]) -> Result<BoardCommand, CliParseError> {
    let mut it = args.iter().peekable();
    match it.next().map(String::as_str) {
        Some("show") => {
            let mut json = false;
            let mut workspace: Option<String> = None;
            let mut all = false;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--json" => json = true,
                    "--all" => all = true,
                    "--workspace" => {
                        let Some(value) = it.next() else {
                            return Err(CliParseError::MissingFlag("--workspace"));
                        };
                        workspace = Some(value.clone());
                    }
                    other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
                }
            }
            Ok(BoardCommand::Show {
                json,
                workspace,
                all,
            })
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
        BoardCommand::Show {
            json,
            workspace,
            all,
        } => {
            let current_session = current_session_from_env().ok().flatten();
            let scope = if all {
                BoardAudienceScope::All
            } else if let Some(workspace_id) = workspace {
                BoardAudienceScope::Workspace(workspace_id)
            } else {
                let session_scope = current_session_board_scope(
                    env.repo_path(),
                    current_session.as_ref().map(|session| session.id.as_str()),
                )
                .map_err(gwt_error_to_spec_ops_error)?;
                if current_session.is_none() && matches!(session_scope, BoardAudienceScope::All) {
                    gui_default_board_scope(env.repo_path()).map_err(gwt_error_to_spec_ops_error)?
                } else {
                    session_scope
                }
            };
            let snapshot = if matches!(scope, BoardAudienceScope::All) {
                load_snapshot(env.repo_path()).map_err(gwt_error_to_spec_ops_error)?
            } else {
                load_snapshot_for_scope(env.repo_path(), &scope)
                    .map_err(gwt_error_to_spec_ops_error)?
            };
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
        BoardCommand::Post(command) => {
            let BoardPostCommand {
                kind,
                body,
                file,
                title,
                title_summary,
                parent,
                topics,
                owners,
                targets,
                mentions,
                broadcast,
            } = *command;
            let body = match (body, file) {
                (Some(body), None) => body,
                (None, Some(file)) => env.read_file(&file).map_err(io_as_spec_ops_error)?,
                _ => {
                    return Err(io_as_spec_ops_error(io::Error::other(
                        "board post requires exactly one of --body or -f",
                    )));
                }
            };
            let current_session = current_session_from_env().ok().flatten();
            // SPEC-1974: GWT_SESSION_ID が無い CLI 呼出 (E2E テストやスクリプト)
            // を `AuthorKind::User` + name="user" にフォールバックさせると、
            // 実ユーザーの GUI 投稿 (`AuthorKind::User` + name="You") と区別が
            // つかなくなり、リーダーが Board 上で agent posts を user posts と
            // 誤認する。ここでは明確に synthetic な agent identity を割り当て
            // て impersonation を防ぐ。
            let (author_kind, author) = current_session
                .as_ref()
                .map(|session| (AuthorKind::Agent, session.display_name.clone()))
                .unwrap_or((AuthorKind::Agent, "cli".to_string()));
            let (workspace_audience, other_mention_args) = split_workspace_mentions(&mentions);
            let mentions = normalize_board_mentions(
                &parse_mentions(&other_mention_args).map_err(gwt_error_to_spec_ops_error)?,
            );
            let mut audience = Vec::new();
            if !broadcast {
                if let BoardAudienceScope::Workspace(workspace_id) = current_session_board_scope(
                    env.repo_path(),
                    current_session.as_ref().map(|session| session.id.as_str()),
                )
                .map_err(gwt_error_to_spec_ops_error)?
                {
                    audience.push(workspace_id);
                }
                audience.extend(workspace_audience);
                audience.extend(
                    post_audience_for_session(env.repo_path(), None, &mentions, false)
                        .map_err(gwt_error_to_spec_ops_error)?
                        .unwrap_or_default(),
                );
            }
            // SPEC-3046: エントリの形を決める正規化・検証は
            // BoardEntryDraft::finalize に集約されている。CLI 側は author 解決
            // (SPEC-1974) / audience 解決 / Session→origin の受け渡しだけを担う。
            let mut draft = BoardEntryDraft::new(
                author_kind,
                author,
                kind.parse().map_err(gwt_error_to_spec_ops_error)?,
                body,
            );
            draft.title = title;
            draft.title_summary = title_summary;
            draft.parent_id = parent;
            draft.related_topics = topics;
            draft.related_owners = owners;
            draft.target_owners = targets;
            draft.mentions = mentions;
            draft.audience = audience;
            if let Some(session) = current_session.as_ref() {
                draft.origin = BoardOrigin::new(
                    session.branch.clone(),
                    session.id.clone(),
                    session.display_name.clone(),
                );
            }
            let entry = draft
                .finalize()
                .map_err(|err| io_as_spec_ops_error(io::Error::other(err.to_string())))?;
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
    // per-stage timeout (~200 ms each across connect / send / ack,
    // ~600 ms worst case), which is an acceptable amount of
    // synchronous wall time for a CLI command.
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
    let mut title: Option<String> = None;
    let mut title_summary: Option<String> = None;
    let mut parent: Option<String> = None;
    let mut topics = Vec::new();
    let mut owners = Vec::new();
    let mut targets = Vec::new();
    let mut mentions = Vec::new();
    let mut broadcast = false;
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
            "--title" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title"));
                }
                title = Some(args[i].clone());
            }
            "--title-summary" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--title-summary"));
                }
                title_summary = Some(args[i].clone());
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
            "--mention" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliParseError::MissingFlag("--mention"));
                }
                mentions.push(args[i].clone());
            }
            "--broadcast" => {
                broadcast = true;
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 1;
    }
    if let Some(value) = title_summary.as_deref() {
        super::validate_title_summary_work_name("--title-summary", value)?;
    }

    Ok(BoardCommand::Post(Box::new(BoardPostCommand {
        kind: kind.ok_or(CliParseError::MissingFlag("--kind"))?,
        body,
        file,
        title,
        title_summary,
        parent,
        topics,
        owners,
        targets,
        mentions,
        broadcast,
    })))
}

fn parse_mentions(values: &[String]) -> gwt_core::Result<Vec<BoardMention>> {
    let mut mentions = Vec::new();
    for value in values {
        mentions.push(value.parse::<BoardMention>()?);
    }
    Ok(normalize_board_mentions(&mentions))
}

/// SPEC-2359 FR-096: `--mention workspace:<id>` routes to BoardEntry.audience,
/// not to BoardMention. Split the raw mention args into (workspace_audience,
/// other_mention_args) so the rest of the post path can parse other mentions
/// as `BoardMention` as before.
fn split_workspace_mentions(values: &[String]) -> (Vec<String>, Vec<String>) {
    let mut workspaces: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    for value in values {
        if let Some(rest) = value.trim().strip_prefix("workspace:") {
            let id = rest.trim();
            if !id.is_empty() && !workspaces.iter().any(|existing| existing == id) {
                workspaces.push(id.to_string());
            }
        } else {
            others.push(value.clone());
        }
    }
    (workspaces, others)
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
                "- [{}] {} ({})\n",
                entry.kind.as_str(),
                format_author(entry),
                entry.id
            ));
            append_indented_body(out, &entry.body, "  ");
        }
    }
}

fn append_indented_body(out: &mut String, body: &str, indent: &str) {
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.split('\n') {
        out.push_str(indent);
        out.push_str(line);
        out.push('\n');
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
    use gwt_agent::{AgentId, Session, GWT_SESSION_ID_ENV};
    use gwt_core::{
        coordination::BoardEntryKind,
        workspace_projection::{
            save_workspace_projection, WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary,
            WorkspaceProjection, WorkspaceStatusCategory,
        },
    };

    use crate::cli::test_support::ScopedEnvVar;

    use super::*;

    fn s(value: &str) -> String {
        value.to_string()
    }

    fn workspace_agent(
        session_id: &str,
        agent_id: &str,
        workspace_id: Option<&str>,
        affiliation_status: WorkspaceAgentAffiliationStatus,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: agent_id.to_string(),
            display_name: agent_id.to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Board audience".to_string()),
            title_summary: Some("Board audience".to_string()),
            worktree_path: None,
            branch: Some("work/board-audience".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status,
            workspace_id: workspace_id.map(str::to_string),
            updated_at: chrono::Utc::now(),
        }
    }

    fn save_projection(repo: &std::path::Path, agents: Vec<WorkspaceAgentSummary>) {
        let mut projection = WorkspaceProjection::default_for_project(repo);
        projection.id = "workspace-current".to_string();
        projection.agents = agents;
        save_workspace_projection(repo, &projection).expect("save workspace projection");
    }

    #[test]
    fn board_family_run_post_rejects_whitespace_only_body() {
        // SPEC-3046 受け入れシナリオ 1: GUI と同じ空 body 検証が CLI にも
        // 適用される（whitespace-only body は保存されずエラー）。
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        let cmd = parse(&[s("post"), s("--kind"), s("status"), s("--body"), s("   ")]).unwrap();
        let mut out = String::new();
        let err = run(&mut env, cmd, &mut out).expect_err("whitespace-only body must be rejected");
        assert!(
            err.to_string().contains("body"),
            "error should mention the body requirement: {err}"
        );
        let snapshot = gwt_core::coordination::load_snapshot(tmp.path()).unwrap();
        assert!(
            snapshot.board.entries.is_empty(),
            "rejected post must not be persisted"
        );
    }

    #[test]
    fn board_family_parse_show_json() {
        let cmd = parse(&[s("show"), s("--json")]).unwrap();
        assert_eq!(
            cmd,
            BoardCommand::Show {
                json: true,
                workspace: None,
                all: false,
            }
        );
    }

    #[test]
    fn board_family_parse_show_collects_workspace_and_all_flags() {
        let cmd = parse(&[
            s("show"),
            s("--json"),
            s("--workspace"),
            s("ws-1"),
            s("--all"),
        ])
        .unwrap();
        assert_eq!(
            cmd,
            BoardCommand::Show {
                json: true,
                workspace: Some("ws-1".into()),
                all: true,
            }
        );
    }

    #[test]
    fn board_family_run_show_workspace_filter_keeps_broadcast_and_matching_audience() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        for (body, audience) in [
            ("broadcast post", Vec::<String>::new()),
            ("scoped to ws-1", vec!["ws-1".into()]),
            ("scoped to ws-2", vec!["ws-2".into()]),
        ] {
            let mut entry = BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                gwt_core::coordination::BoardEntryKind::Status,
                body,
                None,
                None,
                vec![],
                vec![],
            );
            if !audience.is_empty() {
                entry = entry.with_audience(audience);
            }
            gwt_core::coordination::post_entry(tmp.path(), entry).unwrap();
        }

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: true,
                workspace: Some("ws-1".into()),
                all: false,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("broadcast post"), "{out}");
        assert!(out.contains("scoped to ws-1"), "{out}");
        assert!(!out.contains("scoped to ws-2"), "{out}");
    }

    #[test]
    fn board_family_run_show_all_flag_shows_full_timeline() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        for (body, audience) in [
            ("broadcast", Vec::<String>::new()),
            ("scoped to ws-1", vec!["ws-1".into()]),
            ("scoped to ws-2", vec!["ws-2".into()]),
        ] {
            let mut entry = BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                gwt_core::coordination::BoardEntryKind::Status,
                body,
                None,
                None,
                vec![],
                vec![],
            );
            if !audience.is_empty() {
                entry = entry.with_audience(audience);
            }
            gwt_core::coordination::post_entry(tmp.path(), entry).unwrap();
        }

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: true,
                workspace: Some("ws-1".into()),
                all: true,
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("broadcast"), "{out}");
        assert!(out.contains("scoped to ws-1"), "{out}");
        assert!(out.contains("scoped to ws-2"), "{out}");
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
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "request".into(),
                body: Some("hello".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            }))
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
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "claim".into(),
                body: Some("I claim feature/foo".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec!["sess-a3f2".into(), "feature/foo".into()],
                mentions: vec![],
                broadcast: false,
            }))
        );
    }

    #[test]
    fn board_family_parse_post_collects_typed_mentions() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("question"),
            s("--body"),
            s("Can you confirm this?"),
            s("--mention"),
            s("user:akiojin"),
            s("--mention"),
            s("agent:codex"),
        ])
        .unwrap();

        assert_eq!(
            cmd,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "question".into(),
                body: Some("Can you confirm this?".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec!["user:akiojin".into(), "agent:codex".into()],
                broadcast: false,
            }))
        );
    }

    #[test]
    fn board_family_parse_show_workspace_and_all() {
        let workspace = parse(&[s("show"), s("--workspace"), s("workspace-a")]).unwrap();
        assert_eq!(
            workspace,
            BoardCommand::Show {
                json: false,
                workspace: Some("workspace-a".into()),
                all: false,
            }
        );

        let all = parse(&[s("show"), s("--all"), s("--json")]).unwrap();
        assert_eq!(
            all,
            BoardCommand::Show {
                json: true,
                workspace: None,
                all: true,
            }
        );
    }

    #[test]
    fn board_family_parse_post_collects_workspace_mentions_and_broadcast() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("status"),
            s("--body"),
            s("cross-workspace update"),
            s("--mention"),
            s("workspace:workspace-a"),
            s("--mention"),
            s("workspace:workspace-b"),
            s("--broadcast"),
        ])
        .unwrap();

        assert_eq!(
            cmd,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("cross-workspace update".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![
                    "workspace:workspace-a".into(),
                    "workspace:workspace-b".into()
                ],
                broadcast: true,
            }))
        );
    }

    #[test]
    fn board_family_parse_post_accepts_title_summary() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("status"),
            s("--body"),
            s("Implementing the title-summary contract across several subsystems"),
            s("--title-summary"),
            s("Title summary contract"),
        ])
        .unwrap();

        assert_eq!(
            cmd,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some(
                    "Implementing the title-summary contract across several subsystems".into()
                ),
                file: None,
                title: None,
                title_summary: Some("Title summary contract".into()),
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            }))
        );
    }

    #[test]
    fn board_family_parse_post_accepts_title() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("status"),
            s("--body"),
            s("**bold** body"),
            s("--title"),
            s("Release notes"),
        ])
        .unwrap();

        assert_eq!(
            cmd,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("**bold** body".into()),
                file: None,
                title: Some("Release notes".into()),
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            }))
        );
    }

    #[test]
    fn board_family_parse_post_rejects_status_like_title_summary() {
        let err = parse(&[
            s("post"),
            s("--kind"),
            s("status"),
            s("--body"),
            s("Finished implementing the Agent title improvement"),
            s("--title-summary"),
            s("Agent title improvement complete"),
        ])
        .expect_err("title-summary must describe the work, not its status");

        let message = err.to_string();
        assert!(message.contains("--title-summary"), "{message}");
        assert!(message.contains("work name"), "{message}");
        assert!(message.contains("status"), "{message}");
    }

    #[test]
    fn board_family_run_post_persists_target_owners() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "claim".into(),
                body: Some("taking the migration".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec!["sess-a3f2".into(), "feature/x".into()],
                mentions: vec![],
                broadcast: false,
            })),
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
    fn board_family_run_post_persists_typed_mentions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "question".into(),
                body: Some("Can you confirm this?".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec!["user:akiojin".into(), "agent:codex".into()],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        assert_eq!(snapshot.board.entries.len(), 1);
        assert_eq!(snapshot.board.entries[0].mentions.len(), 2);
        assert_eq!(
            snapshot.board.entries[0].mentions[0].typed_key(),
            "user:akiojin"
        );
        assert_eq!(
            snapshot.board.entries[0].mentions[1].typed_key(),
            "agent:codex"
        );
    }

    #[test]
    fn board_family_parse_post_routes_workspace_mention_into_audience_and_broadcast_flag() {
        let cmd = parse(&[
            s("post"),
            s("--kind"),
            s("status"),
            s("--body"),
            s("scoped to two workspaces"),
            s("--mention"),
            s("workspace:ws-1"),
            s("--mention"),
            s("agent:codex"),
            s("--mention"),
            s("workspace:ws-2"),
            s("--broadcast"),
        ])
        .unwrap();

        assert_eq!(
            cmd,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("scoped to two workspaces".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![
                    "workspace:ws-1".into(),
                    "agent:codex".into(),
                    "workspace:ws-2".into(),
                ],
                broadcast: true,
            }))
        );
    }

    #[test]
    fn board_family_run_post_persists_audience_from_workspace_mentions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("audienced status".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![
                    "workspace:ws-1".into(),
                    "agent:codex".into(),
                    "workspace:ws-2".into(),
                ],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        let entry = &snapshot.board.entries[0];

        assert_eq!(
            entry.audience,
            vec!["ws-1".to_string(), "ws-2".to_string()],
            "workspace mentions must land on BoardEntry.audience"
        );
        assert_eq!(
            entry.mentions.len(),
            1,
            "workspace mentions must not be stored as regular BoardMentions"
        );
        assert_eq!(entry.mentions[0].typed_key(), "agent:codex");
    }

    #[test]
    fn board_family_run_post_broadcast_flag_keeps_audience_empty_without_explicit_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("broadcast post".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: true,
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        let entry = &snapshot.board.entries[0];
        assert!(
            entry.audience.is_empty(),
            "broadcast flag must keep audience empty even when current workspace exists"
        );
    }

    #[test]
    fn board_family_run_post_attaches_current_session_origin_metadata() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(tmp.path(), "work/20260506-1706", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("Implement current focus title sync".into()),
                file: None,
                title: None,
                title_summary: Some("Current focus title sync".into()),
                parent: None,
                topics: vec![],
                owners: vec!["2359".into()],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        let entry = &snapshot.board.entries[0];
        assert_eq!(entry.author_kind, AuthorKind::Agent);
        assert_eq!(entry.author, "Codex");
        assert_eq!(
            entry.title_summary.as_deref(),
            Some("Current focus title sync")
        );
        assert_eq!(
            entry.origin_session_id.as_deref(),
            Some(session.id.as_str())
        );
        assert_eq!(entry.origin_branch.as_deref(), Some("work/20260506-1706"));
        assert_eq!(entry.origin_agent_id.as_deref(), Some("Codex"));
    }

    #[test]
    fn board_family_run_post_auto_attaches_current_assigned_workspace() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/board-audience", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                Some("workspace-current"),
                WorkspaceAgentAffiliationStatus::Assigned,
            )],
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let mut out = String::new();
        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("current workspace update".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        let snapshot = load_snapshot(&repo).unwrap();
        assert_eq!(
            snapshot.board.entries[0].audience,
            vec!["workspace-current".to_string()]
        );
    }

    #[test]
    fn board_family_run_post_leaves_unassigned_and_broadcast_posts_unscoped() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/board-audience", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                None,
                WorkspaceAgentAffiliationStatus::Unassigned,
            )],
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());

        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("unassigned broadcast".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
            &mut String::new(),
        )
        .unwrap();
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                Some("workspace-current"),
                WorkspaceAgentAffiliationStatus::Assigned,
            )],
        );
        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("forced broadcast".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: true,
            })),
            &mut String::new(),
        )
        .unwrap();

        let snapshot = load_snapshot(&repo).unwrap();
        assert!(snapshot.board.entries[0].audience.is_empty());
        assert!(snapshot.board.entries[1].audience.is_empty());
    }

    #[test]
    fn board_family_run_post_keeps_unassigned_actionable_milestone_unscoped() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/workspace-materialization", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                None,
                WorkspaceAgentAffiliationStatus::Unassigned,
            )],
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());

        let mut out = String::new();
        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "claim".into(),
                body: Some("Materialize actionable Unassigned Agents before Board audience".into()),
                file: None,
                title: None,
                title_summary: Some("Workspace materialization".into()),
                parent: None,
                topics: vec!["workspace-materialization".into()],
                owners: vec!["2359".into()],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        let snapshot = load_snapshot(&repo).unwrap();
        let entry = &snapshot.board.entries[0];
        assert!(
            entry.audience.is_empty(),
            "Unassigned Board posts should not auto-materialize a Workspace or audience: {entry:?}"
        );
        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session.id)
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Unassigned
        );
        assert!(agent.workspace_id.is_none());
        assert!(
            gwt_core::workspace_projection::load_workspace_work_items(&repo)
                .expect("load workspace history")
                .is_none()
        );
    }

    #[test]
    fn board_family_run_post_broadcast_does_not_materialize_unassigned_actionable_milestone() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/workspace-materialization", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                None,
                WorkspaceAgentAffiliationStatus::Unassigned,
            )],
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());

        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "claim".into(),
                body: Some("Intentional broadcast for cross-workspace coordination".into()),
                file: None,
                title: None,
                title_summary: Some("Workspace materialization".into()),
                parent: None,
                topics: vec!["workspace-materialization".into()],
                owners: vec!["2359".into()],
                targets: vec![],
                mentions: vec![],
                broadcast: true,
            })),
            &mut String::new(),
        )
        .unwrap();

        let snapshot = load_snapshot(&repo).unwrap();
        assert!(snapshot.board.entries[0].audience.is_empty());
        let projection = gwt_core::workspace_projection::load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = projection
            .agents
            .iter()
            .find(|agent| agent.session_id == session.id)
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Unassigned
        );
        assert!(
            gwt_core::workspace_projection::load_workspace_work_items(&repo)
                .expect("load workspace history")
                .is_none()
        );
    }

    #[test]
    fn board_family_run_post_fans_out_workspace_audience_from_mentions() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/board-audience", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![
                workspace_agent(
                    &session.id,
                    "codex",
                    Some("workspace-current"),
                    WorkspaceAgentAffiliationStatus::Assigned,
                ),
                workspace_agent(
                    "session-target",
                    "reviewer",
                    Some("workspace-target"),
                    WorkspaceAgentAffiliationStatus::Assigned,
                ),
                workspace_agent(
                    "session-unassigned",
                    "observer",
                    None,
                    WorkspaceAgentAffiliationStatus::Unassigned,
                ),
            ],
        );
        let mut env = crate::cli::TestEnv::new(repo.clone());

        run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "handoff".into(),
                body: Some("handoff across workspaces".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![
                    "workspace:workspace-explicit".into(),
                    "session:session-target".into(),
                    "agent:reviewer".into(),
                    "agent:observer".into(),
                    "user:akiojin".into(),
                ],
                broadcast: false,
            })),
            &mut String::new(),
        )
        .unwrap();

        save_projection(
            &repo,
            vec![workspace_agent(
                "session-target",
                "reviewer",
                Some("workspace-later"),
                WorkspaceAgentAffiliationStatus::Assigned,
            )],
        );
        let snapshot = load_snapshot(&repo).unwrap();
        assert_eq!(
            snapshot.board.entries[0].audience,
            vec![
                "workspace-current".to_string(),
                "workspace-explicit".to_string(),
                "workspace-target".to_string(),
            ]
        );
    }

    #[test]
    fn board_family_rejects_card_subcommand() {
        let err = parse(&[s("card"), s("set"), s("--status"), s("running")]).unwrap_err();
        assert_eq!(err, CliParseError::UnknownSubcommand("card".into()));
    }

    // SPEC-1974: GWT_SESSION_ID 環境変数が設定されていない CLI 呼出
    // (E2E テスト・スクリプト経由など) は、実ユーザーの GUI 投稿
    // (`AuthorKind::User` + name="You") と区別がつくよう、明示的に synthetic
    // な agent identity (`AuthorKind::Agent` + name="cli") として記録される
    // ことを固定する。これにより `[user @ - / -]` 表示で実ユーザー投稿と
    // 誤認させる impersonation 経路を塞ぐ。
    #[test]
    fn board_family_run_post_uses_synthetic_agent_identity_when_session_env_missing() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _session_env = ScopedEnvVar::unset(GWT_SESSION_ID_ENV);
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "status".into(),
                body: Some("test post without session env".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec![],
                owners: vec![],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        let snapshot = load_snapshot(tmp.path()).unwrap();
        let entry = &snapshot.board.entries[0];
        assert_eq!(
            entry.author_kind,
            AuthorKind::Agent,
            "missing GWT_SESSION_ID must not be attributed as a real user"
        );
        assert_eq!(
            entry.author, "cli",
            "fallback identity must be a clearly synthetic agent label"
        );
        assert!(
            entry.origin_branch.is_none(),
            "no session means no origin_branch"
        );
        assert!(
            entry.origin_session_id.is_none(),
            "no session means no origin_session_id"
        );
    }

    #[test]
    fn board_family_run_post_updates_projection() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Post(Box::new(BoardPostCommand {
                kind: "request".into(),
                body: Some("Need a board".into()),
                file: None,
                title: None,
                title_summary: None,
                parent: None,
                topics: vec!["coordination".into()],
                owners: vec!["1974".into()],
                targets: vec![],
                mentions: vec![],
                broadcast: false,
            })),
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
    fn board_family_run_show_scopes_workspace_and_all_timelines() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut env = crate::cli::TestEnv::new(repo.clone());
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                "broadcast entry",
                None,
                None,
                vec![],
                vec![],
            ),
        )
        .unwrap();
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                "workspace a entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-a"]),
        )
        .unwrap();
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                "workspace b entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-b"]),
        )
        .unwrap();

        let mut workspace_out = String::new();
        run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: Some("workspace-a".into()),
                all: false,
            },
            &mut workspace_out,
        )
        .unwrap();
        assert!(workspace_out.contains("broadcast entry"), "{workspace_out}");
        assert!(
            workspace_out.contains("workspace a entry"),
            "{workspace_out}"
        );
        assert!(
            !workspace_out.contains("workspace b entry"),
            "{workspace_out}"
        );

        let mut all_out = String::new();
        run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: true,
            },
            &mut all_out,
        )
        .unwrap();
        assert!(all_out.contains("workspace b entry"), "{all_out}");
    }

    #[test]
    fn board_family_run_show_defaults_to_current_workspace_when_session_is_assigned() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _home = ScopedEnvVar::set("HOME", tmp.path());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", tmp.path());
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let sessions_dir = gwt_core::paths::gwt_sessions_dir();
        let session = Session::new(&repo, "work/board-audience", AgentId::Codex);
        session.save(&sessions_dir).unwrap();
        let _session_env = ScopedEnvVar::set(GWT_SESSION_ID_ENV, &session.id);
        save_projection(
            &repo,
            vec![workspace_agent(
                &session.id,
                "codex",
                Some("workspace-current"),
                WorkspaceAgentAffiliationStatus::Assigned,
            )],
        );
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                "current entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-current"]),
        )
        .unwrap();
        post_entry(
            &repo,
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Status,
                "other entry",
                None,
                None,
                vec![],
                vec![],
            )
            .with_audience(vec!["workspace-other"]),
        )
        .unwrap();
        let mut env = crate::cli::TestEnv::new(repo);

        let mut out = String::new();
        run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: false,
            },
            &mut out,
        )
        .unwrap();

        assert!(out.contains("current entry"), "{out}");
        assert!(!out.contains("other entry"), "{out}");
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
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: false,
            },
            &mut out,
        )
        .unwrap();

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
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: false,
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("- [request] user ("));
        assert!(out.contains("  legacy entry"));
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
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: false,
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(out.contains("== Chat =="));
        assert!(out.contains("Need a board"));
        assert!(!out.contains("== Cards =="));
        assert!(!out.contains("no agent cards"));
    }

    #[test]
    fn board_family_run_show_renders_multiline_body_as_indented_block() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = crate::cli::TestEnv::new(tmp.path().to_path_buf());
        post_entry(
            tmp.path(),
            BoardEntry::new(
                AuthorKind::Agent,
                "Codex",
                BoardEntryKind::Decision,
                "Current state: Board posts are too dense.\n\nDecision: Keep body canonical.\nNext: Update rendering.",
                None,
                None,
                vec![],
                vec![],
            )
            .with_origin_branch("work/readable-board")
            .with_origin_session_id("sess-readable"),
        )
        .unwrap();

        let mut out = String::new();
        let code = run(
            &mut env,
            BoardCommand::Show {
                json: false,
                workspace: None,
                all: false,
            },
            &mut out,
        )
        .unwrap();

        assert_eq!(code, 0);
        assert!(
            out.contains("- [decision] Codex @ work/readable-board / sess-readable ("),
            "expected metadata header without inline body, got:\n{out}"
        );
        assert!(
            out.contains("  Current state: Board posts are too dense.\n  \n  Decision: Keep body canonical.\n  Next: Update rendering."),
            "expected body lines to be indented while preserving blank lines, got:\n{out}"
        );
        assert!(
            !out.contains("Codex @ work/readable-board / sess-readable: Current state"),
            "body must not be collapsed into the header, got:\n{out}"
        );
    }
}

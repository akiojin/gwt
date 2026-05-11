use chrono::Utc;
use gwt_core::workspace_projection::{
    load_or_default_workspace_projection, load_or_synthesize_workspace_work_items,
    record_workspace_work_event, save_workspace_projection,
    update_workspace_projection_with_journal, WorkspaceAgentAffiliationStatus,
    WorkspaceExecutionContainerRef, WorkspaceProjection, WorkspaceProjectionUpdate,
    WorkspaceStatusCategory, WorkspaceWorkEvent, WorkspaceWorkEventKind, WorkspaceWorkItem,
};
use gwt_github::{ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, WorkspaceCommand};

pub fn parse(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "update" => parse_update(rest),
        "candidates" => parse_candidates(rest),
        "join" => parse_join(rest),
        "create" => parse_create(rest),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

fn parse_required_value(
    args: &[String],
    index: usize,
    flag: &'static str,
) -> Result<String, CliParseError> {
    args.get(index + 1)
        .cloned()
        .ok_or(CliParseError::MissingFlag(flag))
}

fn parse_update(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut title = None;
    let mut status = None;
    let mut status_text = None;
    let mut summary = None;
    let mut next_action = None;
    let mut owner = None;
    let mut agent_session = None;
    let mut current_focus = None;
    let mut title_summary = None;
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).ok_or(CliParseError::Usage)?.clone();
        match args[i].as_str() {
            "--title" => title = Some(value),
            "--status" => status = Some(value),
            "--status-text" => status_text = Some(value),
            "--summary" => summary = Some(value),
            "--next-action" => next_action = Some(value),
            "--owner" => owner = Some(value),
            "--agent-session" => agent_session = Some(value),
            "--current-focus" => current_focus = Some(value),
            "--title-summary" => title_summary = Some(value),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    if agent_session.is_none() && (current_focus.is_some() || title_summary.is_some()) {
        return Err(CliParseError::MissingFlag("--agent-session"));
    }
    if let Some(value) = title_summary.as_deref() {
        super::validate_title_summary_work_name("--title-summary", value)?;
    }
    Ok(WorkspaceCommand::Update {
        title,
        status,
        status_text,
        summary,
        next_action,
        owner,
        agent_session,
        current_focus,
        title_summary,
    })
}

fn parse_candidates(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    Ok(WorkspaceCommand::Candidates {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
    })
}

fn parse_join(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut workspace_id = None;
    let mut current_focus = None;
    let mut title_summary = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            "--workspace" | "--workspace-id" => {
                workspace_id = Some(parse_required_value(args, i, "--workspace")?)
            }
            "--current-focus" => {
                current_focus = Some(parse_required_value(args, i, "--current-focus")?)
            }
            "--title-summary" => {
                title_summary = Some(parse_required_value(args, i, "--title-summary")?)
            }
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    if let Some(value) = title_summary.as_deref() {
        super::validate_title_summary_work_name("--title-summary", value)?;
    }
    Ok(WorkspaceCommand::Join {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
        workspace_id: workspace_id.ok_or(CliParseError::MissingFlag("--workspace"))?,
        current_focus,
        title_summary,
    })
}

fn parse_create(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let mut agent_session = None;
    let mut title_summary = None;
    let mut current_focus = None;
    let mut spec = None;
    let mut issue = None;
    let mut split_from = None;
    let mut boundary = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-session" => {
                agent_session = Some(parse_required_value(args, i, "--agent-session")?)
            }
            "--title-summary" => {
                title_summary = Some(parse_required_value(args, i, "--title-summary")?)
            }
            "--current-focus" => {
                current_focus = Some(parse_required_value(args, i, "--current-focus")?)
            }
            "--spec" => {
                spec = Some(
                    parse_required_value(args, i, "--spec")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--issue" => {
                issue = Some(
                    parse_required_value(args, i, "--issue")?
                        .parse::<u64>()
                        .map_err(|_| CliParseError::Usage)?,
                );
            }
            "--split-from" => split_from = Some(parse_required_value(args, i, "--split-from")?),
            "--boundary" => boundary = Some(parse_required_value(args, i, "--boundary")?),
            other => return Err(CliParseError::UnknownSubcommand(other.to_string())),
        }
        i += 2;
    }
    let title_summary = title_summary.ok_or(CliParseError::MissingFlag("--title-summary"))?;
    super::validate_title_summary_work_name("--title-summary", &title_summary)?;
    Ok(WorkspaceCommand::Create {
        agent_session: agent_session.ok_or(CliParseError::MissingFlag("--agent-session"))?,
        title_summary,
        current_focus,
        spec,
        issue,
        split_from,
        boundary,
    })
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    cmd: WorkspaceCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match cmd {
        WorkspaceCommand::Update {
            title,
            status,
            status_text,
            summary,
            next_action,
            owner,
            agent_session,
            current_focus,
            title_summary,
        } => {
            let update = WorkspaceProjectionUpdate {
                title,
                status_category: status
                    .as_deref()
                    .map(parse_status_category)
                    .transpose()
                    .map_err(string_error)?,
                status_text,
                owner,
                next_action,
                summary,
                agent_session_id: agent_session,
                agent_current_focus: current_focus,
                agent_title_summary: title_summary,
            };
            let entry = update_workspace_projection_with_journal(env.repo_path(), update)
                .map_err(|error| string_error(error.to_string()))?;
            publish_workspace_change(env.repo_path());
            out.push_str(&format!("workspace updated: {}\n", entry.id));
            Ok(0)
        }
        WorkspaceCommand::Candidates { agent_session } => {
            let projection =
                load_or_synthesize_workspace_work_items(env.repo_path()).map_err(core_error)?;
            let current_intent = current_agent_intent(env.repo_path(), &agent_session)?;
            let mut candidates = projection
                .work_items
                .iter()
                .filter(|item| item.is_incomplete())
                .filter(|item| {
                    !item
                        .agents
                        .iter()
                        .any(|agent| agent.session_id == agent_session)
                })
                .map(|item| {
                    let score = workspace_similarity_score(
                        current_intent.as_deref().unwrap_or_default(),
                        &workspace_item_text(item),
                    );
                    (score, item)
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                right
                    .0
                    .cmp(&left.0)
                    .then_with(|| right.1.updated_at.cmp(&left.1.updated_at))
            });
            if candidates.is_empty() {
                out.push_str("workspace candidates: none\n");
            } else {
                for (score, item) in candidates {
                    out.push_str(&format!(
                        "{}\t{}\t{}\tscore={score}\n",
                        item.id,
                        status_category_wire(item.status_category),
                        item.title
                    ));
                }
            }
            Ok(0)
        }
        WorkspaceCommand::Join {
            agent_session,
            workspace_id,
            current_focus,
            title_summary,
        } => {
            let mut projection =
                load_or_default_workspace_projection(env.repo_path()).map_err(core_error)?;
            let Some(item) = workspace_item_by_id(env.repo_path(), &workspace_id)? else {
                return Err(string_error(format!("workspace not found: {workspace_id}")));
            };
            assign_agent_to_workspace(
                &mut projection,
                &agent_session,
                &workspace_id,
                current_focus,
                title_summary,
            )?;
            apply_workspace_item_to_projection(&mut projection, &item);
            save_workspace_projection(env.repo_path(), &projection).map_err(core_error)?;
            publish_workspace_change(env.repo_path());
            out.push_str(&format!("workspace joined: {workspace_id}\n"));
            Ok(0)
        }
        WorkspaceCommand::Create {
            agent_session,
            title_summary,
            current_focus,
            spec,
            issue,
            split_from,
            boundary,
        } => {
            let existing =
                load_or_synthesize_workspace_work_items(env.repo_path()).map_err(core_error)?;
            if split_from.is_none()
                && boundary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                let new_text = [Some(title_summary.as_str()), current_focus.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(item) = existing.work_items.iter().find(|item| {
                    item.is_incomplete()
                        && workspace_similarity_score(&new_text, &workspace_item_text(item)) >= 2
                }) {
                    return Err(string_error(format!(
                        "similar Workspace exists: {} ({})",
                        item.title, item.id
                    )));
                }
            }
            let mut projection =
                load_or_default_workspace_projection(env.repo_path()).map_err(core_error)?;
            let Some(agent) = projection
                .agents
                .iter()
                .find(|agent| agent.session_id == agent_session)
            else {
                return Err(string_error(format!(
                    "agent session not found: {agent_session}"
                )));
            };
            let workspace_id = format!("workspace-{}", Utc::now().timestamp_millis());
            let owner = spec
                .map(|number| format!("SPEC-{number}"))
                .or_else(|| issue.map(|number| format!("Issue #{number}")));
            let now = Utc::now();
            let mut event =
                WorkspaceWorkEvent::new(WorkspaceWorkEventKind::Start, workspace_id.clone(), now);
            event.title = Some(title_summary.clone());
            event.intent = current_focus.clone();
            event.summary = current_focus
                .clone()
                .or_else(|| Some(title_summary.clone()));
            event.status_category = Some(WorkspaceStatusCategory::Active);
            event.owner = owner.clone();
            event.next_action = Some("Coordinate on Board before implementation".to_string());
            event.agent_session_id = Some(agent_session.clone());
            event.agent_id = Some(agent.agent_id.clone());
            event.display_name = Some(agent.display_name.clone());
            event.execution_container = Some(WorkspaceExecutionContainerRef {
                branch: agent.branch.clone(),
                worktree_path: agent.worktree_path.clone(),
                pr_number: None,
                pr_url: None,
                pr_state: None,
            });
            if let Some(split_from) = split_from {
                event.kind = WorkspaceWorkEventKind::Split;
                event.related_work_item_id = Some(split_from);
            }
            if let Some(boundary) = boundary {
                event.next_action = Some(format!("Boundary: {boundary}"));
            }
            record_workspace_work_event(env.repo_path(), event).map_err(core_error)?;
            projection.id = workspace_id.clone();
            projection.title = title_summary.clone();
            projection.status_category = WorkspaceStatusCategory::Active;
            projection.status_text = current_focus
                .clone()
                .unwrap_or_else(|| "Workspace created".to_string());
            projection.summary = current_focus.clone();
            projection.owner = owner;
            projection.next_action = Some("Coordinate on Board before implementation".to_string());
            assign_agent_to_workspace(
                &mut projection,
                &agent_session,
                &workspace_id,
                current_focus,
                Some(title_summary),
            )?;
            projection.updated_at = now;
            save_workspace_projection(env.repo_path(), &projection).map_err(core_error)?;
            publish_workspace_change(env.repo_path());
            out.push_str(&format!("workspace created: {workspace_id}\n"));
            Ok(0)
        }
    }
}

fn core_error(error: gwt_core::error::GwtError) -> SpecOpsError {
    string_error(error.to_string())
}

fn status_category_wire(category: WorkspaceStatusCategory) -> &'static str {
    match category {
        WorkspaceStatusCategory::Active => "active",
        WorkspaceStatusCategory::Idle => "idle",
        WorkspaceStatusCategory::Blocked => "blocked",
        WorkspaceStatusCategory::Done => "done",
        WorkspaceStatusCategory::Unknown => "unknown",
    }
}

fn current_agent_intent(
    repo_path: &std::path::Path,
    agent_session: &str,
) -> Result<Option<String>, SpecOpsError> {
    let projection = load_or_default_workspace_projection(repo_path).map_err(core_error)?;
    Ok(projection
        .agents
        .iter()
        .find(|agent| agent.session_id == agent_session)
        .map(|agent| {
            [
                agent.title_summary.as_deref(),
                agent.current_focus.as_deref(),
                agent.coordination_scope.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
        }))
}

fn workspace_item_by_id(
    repo_path: &std::path::Path,
    workspace_id: &str,
) -> Result<Option<WorkspaceWorkItem>, SpecOpsError> {
    Ok(load_or_synthesize_workspace_work_items(repo_path)
        .map_err(core_error)?
        .work_items
        .into_iter()
        .find(|item| item.id == workspace_id))
}

fn apply_workspace_item_to_projection(
    projection: &mut WorkspaceProjection,
    item: &WorkspaceWorkItem,
) {
    projection.id = item.id.clone();
    projection.title = item.title.clone();
    projection.status_category = item.status_category;
    projection.status_text = item
        .summary
        .clone()
        .or_else(|| item.intent.clone())
        .unwrap_or_else(|| "Workspace selected".to_string());
    projection.summary = item.summary.clone().or_else(|| item.intent.clone());
    projection.owner = item.owner.clone();
    projection.updated_at = Utc::now();
}

fn assign_agent_to_workspace(
    projection: &mut WorkspaceProjection,
    agent_session: &str,
    workspace_id: &str,
    current_focus: Option<String>,
    title_summary: Option<String>,
) -> Result<(), SpecOpsError> {
    let Some(agent) = projection
        .agents
        .iter_mut()
        .find(|agent| agent.session_id == agent_session)
    else {
        return Err(string_error(format!(
            "agent session not found: {agent_session}"
        )));
    };
    agent.affiliation_status = WorkspaceAgentAffiliationStatus::Assigned;
    agent.workspace_id = Some(workspace_id.to_string());
    agent.status_category = WorkspaceStatusCategory::Active;
    if current_focus.is_some() {
        agent.current_focus = current_focus;
    }
    if title_summary.is_some() {
        agent.title_summary = title_summary;
    }
    agent.updated_at = Utc::now();
    Ok(())
}

fn workspace_item_text(item: &WorkspaceWorkItem) -> String {
    let mut parts = vec![item.title.as_str()];
    if let Some(intent) = item.intent.as_deref() {
        parts.push(intent);
    }
    if let Some(summary) = item.summary.as_deref() {
        parts.push(summary);
    }
    if let Some(owner) = item.owner.as_deref() {
        parts.push(owner);
    }
    parts.join("\n")
}

fn workspace_similarity_score(left: &str, right: &str) -> usize {
    let left_tokens = workspace_tokens(left);
    if left_tokens.is_empty() {
        return 0;
    }
    let right_tokens = workspace_tokens(right);
    left_tokens
        .iter()
        .filter(|token| right_tokens.contains(*token))
        .count()
}

fn workspace_tokens(value: &str) -> std::collections::BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_lowercase())
        .collect()
}

#[cfg(unix)]
fn publish_workspace_change(project_root: &std::path::Path) {
    let result = crate::daemon_publisher::publish_event(
        project_root,
        "workspace",
        serde_json::json!({"projection": "updated"}),
    );
    if let Err(err) = result {
        tracing::debug!(
            error = %err,
            project_root = %project_root.display(),
            "gwtd workspace update: daemon publish failed (non-fatal)"
        );
    }
}

#[cfg(not(unix))]
fn publish_workspace_change(_project_root: &std::path::Path) {}

fn parse_status_category(value: &str) -> Result<WorkspaceStatusCategory, String> {
    match value {
        "active" => Ok(WorkspaceStatusCategory::Active),
        "idle" => Ok(WorkspaceStatusCategory::Idle),
        "blocked" => Ok(WorkspaceStatusCategory::Blocked),
        "done" => Ok(WorkspaceStatusCategory::Done),
        "unknown" => Ok(WorkspaceStatusCategory::Unknown),
        other => Err(format!("unknown workspace status '{other}'")),
    }
}

fn string_error(error: String) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::env::TestEnv;
    use gwt_core::workspace_projection::{
        load_workspace_projection, load_workspace_work_items, record_workspace_work_event,
        save_workspace_projection, WorkspaceAgentSummary, WorkspaceProjection,
    };
    use std::ffi::OsString;

    fn s(value: &str) -> String {
        value.to_string()
    }

    struct ScopedHome {
        previous_home: Option<OsString>,
    }

    impl ScopedHome {
        fn set(path: &std::path::Path) -> Self {
            let previous_home = std::env::var_os("HOME");
            std::env::set_var("HOME", path);
            Self { previous_home }
        }
    }

    impl Drop for ScopedHome {
        fn drop(&mut self) {
            if let Some(previous_home) = self.previous_home.as_ref() {
                std::env::set_var("HOME", previous_home);
            } else {
                std::env::remove_var("HOME");
            }
        }
    }

    fn unassigned_agent(session_id: &str) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Implement Workspace history".to_string()),
            title_summary: Some("Workspace history".to_string()),
            worktree_path: None,
            branch: Some("work/20260511-0100".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Unassigned,
            workspace_id: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn parse_workspace_update_accepts_summary_fields() {
        let parsed = parse(&[
            s("update"),
            s("--title"),
            s("Fix Active Work"),
            s("--status"),
            s("active"),
            s("--summary"),
            s("Workspace state is current"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Update {
                title: Some("Fix Active Work".to_string()),
                status: Some("active".to_string()),
                status_text: None,
                summary: Some("Workspace state is current".to_string()),
                next_action: None,
                owner: None,
                agent_session: None,
                current_focus: None,
                title_summary: None,
            }
        );
    }

    #[test]
    fn parse_workspace_update_accepts_agent_title_summary() {
        let parsed = parse(&[
            s("update"),
            s("--agent-session"),
            s("session-1"),
            s("--current-focus"),
            s("Implementing the title-summary contract across Board and Workspace"),
            s("--title-summary"),
            s("Title summary contract"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Update {
                title: None,
                status: None,
                status_text: None,
                summary: None,
                next_action: None,
                owner: None,
                agent_session: Some("session-1".to_string()),
                current_focus: Some(
                    "Implementing the title-summary contract across Board and Workspace"
                        .to_string()
                ),
                title_summary: Some("Title summary contract".to_string()),
            }
        );
    }

    #[test]
    fn parse_workspace_update_requires_agent_session_for_agent_title_summary() {
        let err = parse(&[
            s("update"),
            s("--title-summary"),
            s("Title summary contract"),
        ])
        .expect_err("agent title summary requires agent session");

        assert!(matches!(err, CliParseError::MissingFlag("--agent-session")));
    }

    #[test]
    fn parse_workspace_update_rejects_status_like_agent_title_summary() {
        let err = parse(&[
            s("update"),
            s("--agent-session"),
            s("session-1"),
            s("--current-focus"),
            s("Finished implementing the Agent title improvement"),
            s("--title-summary"),
            s("エージェントタイトル改善完了"),
        ])
        .expect_err("title-summary must describe the work, not its status");

        let message = err.to_string();
        assert!(message.contains("--title-summary"), "{message}");
        assert!(message.contains("work name"), "{message}");
        assert!(message.contains("status"), "{message}");
    }

    #[test]
    fn parse_workspace_create_accepts_assignment_fields() {
        let parsed = parse(&[
            s("create"),
            s("--agent-session"),
            s("session-1"),
            s("--title-summary"),
            s("Workspace history"),
            s("--current-focus"),
            s("Implementing Workspace history"),
            s("--spec"),
            s("2359"),
            s("--split-from"),
            s("workspace-existing"),
            s("--boundary"),
            s("UI only"),
        ])
        .expect("parse");

        assert_eq!(
            parsed,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implementing Workspace history".to_string()),
                spec: Some(2359),
                issue: None,
                split_from: Some("workspace-existing".to_string()),
                boundary: Some("UI only".to_string()),
            }
        );
    }

    #[test]
    fn parse_workspace_candidates_and_join_commands() {
        let candidates = parse(&[s("candidates"), s("--agent-session"), s("session-1")])
            .expect("parse candidates");
        assert_eq!(
            candidates,
            WorkspaceCommand::Candidates {
                agent_session: "session-1".to_string()
            }
        );

        let join = parse(&[
            s("join"),
            s("--agent-session"),
            s("session-1"),
            s("--workspace"),
            s("workspace-existing"),
            s("--current-focus"),
            s("Continue Workspace history"),
            s("--title-summary"),
            s("Workspace history"),
        ])
        .expect("parse join");
        assert_eq!(
            join,
            WorkspaceCommand::Join {
                agent_session: "session-1".to_string(),
                workspace_id: "workspace-existing".to_string(),
                current_focus: Some("Continue Workspace history".to_string()),
                title_summary: Some("Workspace history".to_string()),
            }
        );
    }

    #[test]
    fn workspace_update_persists_workspace_status() {
        let _guard = crate::env_test_lock().lock().unwrap();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Update {
                title: Some("Workspace coordination".to_string()),
                status: Some("blocked".to_string()),
                status_text: Some("Waiting on Board alignment".to_string()),
                summary: Some("Align Workspace ownership before edits".to_string()),
                next_action: Some("Post Board request".to_string()),
                owner: Some("SPEC-2359".to_string()),
                agent_session: None,
                current_focus: None,
                title_summary: None,
            },
            &mut out,
        )
        .expect("update workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace updated:"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        assert_eq!(saved.title, "Workspace coordination");
        assert_eq!(saved.status_category, WorkspaceStatusCategory::Blocked);
        assert_eq!(saved.owner.as_deref(), Some("SPEC-2359"));
    }

    #[test]
    fn workspace_join_assigns_unassigned_agent_to_existing_workspace() {
        let _guard = crate::env_test_lock().lock().unwrap();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "workspace-existing",
            Utc::now(),
        );
        event.title = Some("Workspace history".to_string());
        event.summary = Some("Existing Workspace".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Join {
                agent_session: "session-1".to_string(),
                workspace_id: "workspace-existing".to_string(),
                current_focus: Some("Continue Workspace history".to_string()),
                title_summary: Some("Workspace history".to_string()),
            },
            &mut out,
        )
        .expect("join workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace joined: workspace-existing"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), Some("workspace-existing"));
        assert_eq!(saved.id, "workspace-existing");
    }

    #[test]
    fn workspace_create_records_workspace_and_assigns_agent() {
        let _guard = crate::env_test_lock().lock().unwrap();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implement Workspace history".to_string()),
                spec: Some(2359),
                issue: None,
                split_from: None,
                boundary: Some("history slice".to_string()),
            },
            &mut out,
        )
        .expect("create workspace");

        assert_eq!(code, 0);
        assert!(out.contains("workspace created: workspace-"));
        let saved = load_workspace_projection(&repo)
            .expect("load projection")
            .expect("projection");
        let workspace_id = saved.id.clone();
        let agent = saved
            .agents
            .iter()
            .find(|agent| agent.session_id == "session-1")
            .expect("agent");
        assert_eq!(
            agent.affiliation_status,
            WorkspaceAgentAffiliationStatus::Assigned
        );
        assert_eq!(agent.workspace_id.as_deref(), Some(workspace_id.as_str()));
        let items = load_workspace_work_items(&repo)
            .expect("load workspace history")
            .expect("workspace history");
        assert_eq!(items.work_items.len(), 1);
        assert_eq!(items.work_items[0].id, workspace_id);
        assert_eq!(items.work_items[0].title, "Workspace history");
    }

    #[test]
    fn workspace_candidates_lists_similar_incomplete_workspaces() {
        let _guard = crate::env_test_lock().lock().unwrap();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "workspace-existing",
            Utc::now(),
        );
        event.title = Some("Workspace history".to_string());
        event.intent = Some("Implement Workspace history with affiliation state".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let code = run(
            &mut env,
            WorkspaceCommand::Candidates {
                agent_session: "session-1".to_string(),
            },
            &mut out,
        )
        .expect("list candidates");

        assert_eq!(code, 0);
        assert!(out.contains("workspace-existing"), "{out}");
        assert!(out.contains("Workspace history"), "{out}");
        assert!(out.contains("score="), "{out}");
    }

    #[test]
    fn workspace_create_rejects_similar_workspace_without_split_boundary() {
        let _guard = crate::env_test_lock().lock().unwrap();
        let gwt_home = tempfile::tempdir().expect("gwt home");
        let _home = ScopedHome::set(gwt_home.path());
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let mut env = TestEnv::new(repo.clone());
        let mut projection = WorkspaceProjection::default_for_project(&repo);
        projection.agents.push(unassigned_agent("session-1"));
        save_workspace_projection(&repo, &projection).expect("save projection");
        let mut event = WorkspaceWorkEvent::new(
            WorkspaceWorkEventKind::Start,
            "workspace-existing",
            Utc::now(),
        );
        event.title = Some("Workspace history".to_string());
        event.intent = Some("Implement Workspace history with affiliation state".to_string());
        event.status_category = Some(WorkspaceStatusCategory::Active);
        record_workspace_work_event(&repo, event).expect("record workspace");

        let mut out = String::new();
        let err = run(
            &mut env,
            WorkspaceCommand::Create {
                agent_session: "session-1".to_string(),
                title_summary: "Workspace history".to_string(),
                current_focus: Some("Implement Workspace history affiliation".to_string()),
                spec: None,
                issue: Some(2359),
                split_from: None,
                boundary: None,
            },
            &mut out,
        )
        .expect_err("similar Workspace should be rejected");

        assert!(err.to_string().contains("similar Workspace exists"));
    }
}

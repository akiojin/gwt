use gwt_core::workspace_projection::{
    update_workspace_projection_with_journal, WorkspaceProjectionUpdate, WorkspaceStatusCategory,
};
use gwt_github::{ApiError, SpecOpsError};

use crate::cli::{CliEnv, CliParseError, WorkspaceCommand};

pub fn parse(args: &[String]) -> Result<WorkspaceCommand, CliParseError> {
    let (head, rest) = args.split_first().ok_or(CliParseError::Usage)?;
    match head.as_str() {
        "update" => parse_update(rest),
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
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
            out.push_str(&format!("workspace updated: {}\n", entry.id));
            Ok(0)
        }
    }
}

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

    fn s(value: &str) -> String {
        value.to_string()
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
}

use std::path::Path;

use gwt_core::{
    coordination::{
        normalize_board_audience, BoardAudienceScope, BoardMention, BoardMentionTargetKind,
    },
    workspace_projection::{load_workspace_projection, WorkspaceAgentSummary, WorkspaceProjection},
};

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn workspace_id_for_agent(
    projection: &WorkspaceProjection,
    agent: &WorkspaceAgentSummary,
) -> Option<String> {
    if !agent.is_assigned() {
        return None;
    }
    agent
        .workspace_id
        .as_deref()
        .and_then(non_empty_string)
        .or_else(|| non_empty_string(&projection.id))
}

fn active_workspace_id(projection: &WorkspaceProjection) -> Option<String> {
    non_empty_string(&projection.id)
}

fn gui_workspace_id(projection: &WorkspaceProjection) -> Option<String> {
    projection.assigned_agents().next()?;
    active_workspace_id(projection).or_else(|| {
        projection
            .assigned_agents()
            .find_map(|agent| workspace_id_for_agent(projection, agent))
    })
}

fn push_unique(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if value.trim().is_empty() || values.iter().any(|item| item == &value) {
        return;
    }
    values.push(value);
}

fn push_workspace_for_session(
    values: &mut Vec<String>,
    projection: &WorkspaceProjection,
    session_id: &str,
) -> bool {
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
    else {
        return false;
    };
    if let Some(workspace_id) = workspace_id_for_agent(projection, agent) {
        push_unique(values, workspace_id);
    }
    true
}

fn push_workspace_for_agent(
    values: &mut Vec<String>,
    projection: &WorkspaceProjection,
    target: &str,
) {
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    for agent in &projection.agents {
        let agent_matches = agent.agent_id.eq_ignore_ascii_case(target)
            || agent.display_name.eq_ignore_ascii_case(target);
        if !agent_matches {
            continue;
        }
        if let Some(workspace_id) = workspace_id_for_agent(projection, agent) {
            push_unique(values, workspace_id);
        }
    }
}

pub fn current_session_board_scope(
    repo_path: &Path,
    session_id: Option<&str>,
) -> gwt_core::Result<BoardAudienceScope> {
    let Some(session_id) = session_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(BoardAudienceScope::All);
    };
    let Some(projection) = load_workspace_projection(repo_path)? else {
        return Ok(BoardAudienceScope::All);
    };
    let Some(agent) = projection
        .agents
        .iter()
        .find(|agent| agent.session_id == session_id)
    else {
        return Ok(BoardAudienceScope::All);
    };
    if agent.is_unassigned() {
        return Ok(BoardAudienceScope::Broadcast);
    }
    Ok(workspace_id_for_agent(&projection, agent)
        .map(BoardAudienceScope::Workspace)
        .unwrap_or(BoardAudienceScope::All))
}

pub fn gui_default_board_scope(repo_path: &Path) -> gwt_core::Result<BoardAudienceScope> {
    let Some(projection) = load_workspace_projection(repo_path)? else {
        return Ok(BoardAudienceScope::All);
    };
    if let Some(workspace_id) = gui_workspace_id(&projection) {
        return Ok(BoardAudienceScope::Workspace(workspace_id));
    }
    if projection.unassigned_agents().next().is_some() {
        return Ok(BoardAudienceScope::Broadcast);
    }
    Ok(BoardAudienceScope::All)
}

pub fn post_audience_for_session(
    repo_path: &Path,
    session_id: Option<&str>,
    mentions: &[BoardMention],
    broadcast: bool,
) -> gwt_core::Result<Option<Vec<String>>> {
    if broadcast {
        return Ok(None);
    }
    let projection = load_workspace_projection(repo_path)?;
    let mut audience = Vec::new();
    if let (Some(projection), Some(session_id)) = (
        projection.as_ref(),
        session_id.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        push_workspace_for_session(&mut audience, projection, session_id);
    }
    collect_mention_audience(&mut audience, projection.as_ref(), mentions);
    let audience = normalize_board_audience(audience);
    Ok((!audience.is_empty()).then_some(audience))
}

pub fn post_audience_for_gui(
    repo_path: &Path,
    mentions: &[BoardMention],
    target_workspace: Option<&str>,
    broadcast: bool,
) -> gwt_core::Result<Option<Vec<String>>> {
    // SPEC-2959 FR-021: an explicit General/broadcast post carries no audience.
    if broadcast {
        return Ok(None);
    }
    let projection = load_workspace_projection(repo_path)?;
    let mut audience = Vec::new();
    match target_workspace
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // SPEC-2959 FR-020: the composer "To:" selector pins the post to a
        // specific Work, overriding the active-workspace default.
        Some(workspace_id) => push_unique(&mut audience, workspace_id.to_string()),
        None => {
            if let Some(projection) = projection.as_ref() {
                if let Some(workspace_id) = gui_workspace_id(projection) {
                    push_unique(&mut audience, workspace_id);
                }
            }
        }
    }
    collect_mention_audience(&mut audience, projection.as_ref(), mentions);
    let audience = normalize_board_audience(audience);
    Ok((!audience.is_empty()).then_some(audience))
}

fn collect_mention_audience(
    audience: &mut Vec<String>,
    projection: Option<&WorkspaceProjection>,
    mentions: &[BoardMention],
) {
    for mention in mentions {
        match mention.target_kind {
            BoardMentionTargetKind::Workspace => push_unique(audience, mention.target.clone()),
            BoardMentionTargetKind::Session => {
                if let Some(projection) = projection {
                    push_workspace_for_session(audience, projection, &mention.target);
                }
            }
            BoardMentionTargetKind::Agent => {
                if let Some(projection) = projection {
                    push_workspace_for_agent(audience, projection, &mention.target);
                }
            }
            BoardMentionTargetKind::User | BoardMentionTargetKind::Branch => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn gui_audience_broadcast_returns_none() {
        // SPEC-2959 FR-021: General/broadcast posts carry no audience.
        let dir = tempdir().unwrap();
        let audience = post_audience_for_gui(dir.path(), &[], None, true).unwrap();
        assert_eq!(audience, None);
    }

    #[test]
    fn gui_audience_explicit_target_workspace_pins_lane() {
        // SPEC-2959 FR-020: an explicit "To:" workspace overrides the default.
        let dir = tempdir().unwrap();
        let audience = post_audience_for_gui(dir.path(), &[], Some("ws-x"), false).unwrap();
        assert_eq!(audience, Some(vec!["ws-x".to_string()]));
    }

    #[test]
    fn gui_audience_blank_target_falls_back_to_default() {
        // Whitespace target with no workspace projection resolves to no audience.
        let dir = tempdir().unwrap();
        let audience = post_audience_for_gui(dir.path(), &[], Some("  "), false).unwrap();
        assert_eq!(audience, None);
    }
}

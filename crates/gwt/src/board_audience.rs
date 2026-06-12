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
    use chrono::Utc;
    use gwt_core::workspace_projection::{
        save_workspace_projection, WorkspaceAgentAffiliationStatus, WorkspaceStatusCategory,
    };
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn agent(
        session_id: &str,
        affiliation: WorkspaceAgentAffiliationStatus,
        workspace_id: Option<&str>,
    ) -> WorkspaceAgentSummary {
        WorkspaceAgentSummary {
            session_id: session_id.to_string(),
            window_id: None,
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: None,
            title_summary: None,
            worktree_path: None,
            branch: None,
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: affiliation,
            workspace_id: workspace_id.map(str::to_string),
            updated_at: Utc::now(),
        }
    }

    fn projection_with(id: &str, agents: Vec<WorkspaceAgentSummary>) -> WorkspaceProjection {
        let mut projection = WorkspaceProjection::default_for_project(PathBuf::from("/tmp/p"));
        projection.id = id.to_string();
        projection.agents = agents;
        projection
    }

    fn mention(kind: BoardMentionTargetKind, target: &str) -> BoardMention {
        BoardMention {
            target_kind: kind,
            target: target.to_string(),
            label: None,
        }
    }

    use gwt_core::test_support::ScopedEnvVar;

    fn isolate_gwt_home() -> (tempfile::TempDir, ScopedEnvVar, ScopedEnvVar) {
        let home = tempdir().unwrap();
        let home_guard = ScopedEnvVar::set("HOME", home.path());
        let userprofile_guard = ScopedEnvVar::set("USERPROFILE", home.path());
        (home, home_guard, userprofile_guard)
    }

    #[test]
    fn private_helpers_resolve_workspace_audiences() {
        let assigned = agent(
            "s-1",
            WorkspaceAgentAffiliationStatus::Assigned,
            Some("ws-a"),
        );
        let unassigned = agent("s-2", WorkspaceAgentAffiliationStatus::Unassigned, None);
        let proj = projection_with("", vec![assigned.clone(), unassigned.clone()]);

        // workspace_id_for_agent: unassigned -> None; assigned -> its own id.
        assert_eq!(workspace_id_for_agent(&proj, &unassigned), None);
        assert_eq!(
            workspace_id_for_agent(&proj, &assigned).as_deref(),
            Some("ws-a")
        );
        // Assigned with no explicit workspace_id falls back to the projection id.
        let assigned_no_ws = agent("s-3", WorkspaceAgentAffiliationStatus::Assigned, None);
        let proj_id = projection_with("ws-proj", vec![assigned_no_ws.clone()]);
        assert_eq!(
            workspace_id_for_agent(&proj_id, &assigned_no_ws).as_deref(),
            Some("ws-proj")
        );

        // gui_workspace_id: empty id falls through to the first assigned agent;
        // a non-empty id wins directly; no assigned agents -> None.
        assert_eq!(gui_workspace_id(&proj).as_deref(), Some("ws-a"));
        assert_eq!(gui_workspace_id(&proj_id).as_deref(), Some("ws-proj"));
        assert_eq!(
            gui_workspace_id(&projection_with("", vec![unassigned.clone()])),
            None
        );

        // push_workspace_for_session: known session pushes, unknown is false.
        let mut audience = Vec::new();
        assert!(push_workspace_for_session(&mut audience, &proj, "s-1"));
        assert_eq!(audience, vec!["ws-a".to_string()]);
        assert!(!push_workspace_for_session(&mut audience, &proj, "missing"));

        // push_workspace_for_agent: blank target is a no-op; a match pushes.
        let mut by_agent = Vec::new();
        push_workspace_for_agent(&mut by_agent, &proj, "   ");
        assert!(by_agent.is_empty());
        push_workspace_for_agent(&mut by_agent, &proj, "codex");
        assert_eq!(by_agent, vec!["ws-a".to_string()]);

        // collect_mention_audience exercises every target kind.
        let mut mentions = Vec::new();
        collect_mention_audience(
            &mut mentions,
            Some(&proj),
            &[
                mention(BoardMentionTargetKind::Workspace, "ws-explicit"),
                mention(BoardMentionTargetKind::Session, "s-1"),
                mention(BoardMentionTargetKind::Agent, "Codex"),
                mention(BoardMentionTargetKind::User, "akiojin"),
                mention(BoardMentionTargetKind::Branch, "work/x"),
            ],
        );
        assert!(mentions.contains(&"ws-explicit".to_string()));
        assert!(mentions.contains(&"ws-a".to_string()));
    }

    #[test]
    fn session_scope_reads_projection_from_disk() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_home, _home_guard, _userprofile_guard) = isolate_gwt_home();
        let dir = tempdir().unwrap();
        let repo = dir.path();

        save_workspace_projection(
            repo,
            &projection_with(
                "ws-main",
                vec![agent(
                    "s-assigned",
                    WorkspaceAgentAffiliationStatus::Assigned,
                    Some("ws-main"),
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            current_session_board_scope(repo, Some("s-assigned")).unwrap(),
            BoardAudienceScope::Workspace("ws-main".to_string())
        );
        // Unknown session within a present projection -> All.
        assert_eq!(
            current_session_board_scope(repo, Some("s-unknown")).unwrap(),
            BoardAudienceScope::All
        );
        // No session id short-circuits to All.
        assert_eq!(
            current_session_board_scope(repo, None).unwrap(),
            BoardAudienceScope::All
        );

        save_workspace_projection(
            repo,
            &projection_with(
                "ws-main",
                vec![agent(
                    "s-unassigned",
                    WorkspaceAgentAffiliationStatus::Unassigned,
                    None,
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            current_session_board_scope(repo, Some("s-unassigned")).unwrap(),
            BoardAudienceScope::Broadcast
        );
    }

    #[test]
    fn gui_default_scope_reads_projection_from_disk() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_home, _home_guard, _userprofile_guard) = isolate_gwt_home();
        let dir = tempdir().unwrap();
        let repo = dir.path();

        save_workspace_projection(
            repo,
            &projection_with(
                "ws-gui",
                vec![agent(
                    "s-1",
                    WorkspaceAgentAffiliationStatus::Assigned,
                    Some("ws-gui"),
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            gui_default_board_scope(repo).unwrap(),
            BoardAudienceScope::Workspace("ws-gui".to_string())
        );

        // Only unassigned agents -> Broadcast.
        save_workspace_projection(
            repo,
            &projection_with(
                "",
                vec![agent(
                    "s-2",
                    WorkspaceAgentAffiliationStatus::Unassigned,
                    None,
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            gui_default_board_scope(repo).unwrap(),
            BoardAudienceScope::Broadcast
        );

        // No agents at all -> All.
        save_workspace_projection(repo, &projection_with("", vec![])).unwrap();
        assert_eq!(
            gui_default_board_scope(repo).unwrap(),
            BoardAudienceScope::All
        );
    }

    #[test]
    fn post_audience_for_session_attaches_workspace_and_respects_broadcast() {
        let _guard = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_home, _home_guard, _userprofile_guard) = isolate_gwt_home();
        let dir = tempdir().unwrap();
        let repo = dir.path();
        // Broadcast short-circuits before any projection load.
        assert_eq!(
            post_audience_for_session(repo, None, &[], true).unwrap(),
            None
        );

        save_workspace_projection(
            repo,
            &projection_with(
                "ws-post",
                vec![agent(
                    "s-1",
                    WorkspaceAgentAffiliationStatus::Assigned,
                    Some("ws-post"),
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            post_audience_for_session(repo, Some("s-1"), &[], false).unwrap(),
            Some(vec!["ws-post".to_string()])
        );
    }

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

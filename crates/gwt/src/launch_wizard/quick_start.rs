use std::{collections::HashMap, path::Path};

use super::QuickStartEntry;

pub fn load_quick_start_entries(
    repo_path: &Path,
    sessions_dir: &Path,
    branch_name: &str,
) -> Vec<QuickStartEntry> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let sessions = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("toml")).then_some(path)
        })
        .filter_map(|path| gwt_agent::Session::load_and_migrate(&path).ok())
        .collect::<Vec<_>>();

    collect_quick_start_entries_from_sessions(repo_path, branch_name, sessions)
}

pub(super) fn collect_quick_start_entries_from_sessions(
    repo_path: &Path,
    branch_name: &str,
    sessions: Vec<gwt_agent::Session>,
) -> Vec<QuickStartEntry> {
    let mut latest_by_agent: HashMap<String, gwt_agent::Session> = HashMap::new();
    let mut latest_resumable_by_agent: HashMap<String, gwt_agent::Session> = HashMap::new();

    for session in sessions {
        if session.branch != branch_name || session.worktree_path != repo_path {
            continue;
        }

        let agent_key = session.agent_id.command().to_string();
        if agent_session_resume_id(&session).is_some() {
            let replace = latest_resumable_by_agent
                .get(&agent_key)
                .map(|current| session_is_newer(&session, current))
                .unwrap_or(true);
            if replace {
                latest_resumable_by_agent.insert(agent_key.clone(), session.clone());
            }
        }

        let replace = latest_by_agent
            .get(&agent_key)
            .map(|current| session_is_newer(&session, current))
            .unwrap_or(true);
        if replace {
            latest_by_agent.insert(agent_key, session);
        }
    }

    let mut sessions = latest_by_agent
        .into_iter()
        .map(|(agent_key, latest_session)| {
            if agent_session_resume_id(&latest_session).is_some() {
                latest_session
            } else {
                latest_resumable_by_agent
                    .get(&agent_key)
                    .cloned()
                    .unwrap_or(latest_session)
            }
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    sessions
        .into_iter()
        .map(|session| QuickStartEntry {
            session_id: session.id.clone(),
            agent_id: session.agent_id.command().to_string(),
            tool_label: session.display_name.clone(),
            model: session.model.clone(),
            reasoning: session.reasoning_level.clone(),
            version: session.tool_version.clone().or_else(|| {
                session
                    .agent_id
                    .package_name()
                    .map(|_| "installed".to_string())
            }),
            resume_session_id: agent_session_resume_id(&session),
            live_window_id: None,
            skip_permissions: session.skip_permissions,
            codex_fast_mode: session.codex_fast_mode,
            runtime_target: session.runtime_target,
            docker_service: session.docker_service.clone(),
            docker_lifecycle_intent: session.docker_lifecycle_intent,
        })
        .collect()
}

fn session_is_newer(candidate: &gwt_agent::Session, current: &gwt_agent::Session) -> bool {
    candidate.updated_at > current.updated_at
        || (candidate.updated_at == current.updated_at && candidate.created_at > current.created_at)
}

fn agent_session_resume_id(session: &gwt_agent::Session) -> Option<String> {
    session
        .agent_session_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

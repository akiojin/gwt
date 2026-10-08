//! Prepare watcher membership changes and their wire payload off the GUI loop.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use super::super::title_sync::{
    resolve_workspace_projection_title_updates, WorkspaceTitleUpdate, WorkspaceWindowTitles,
};
use super::{ActiveAgentSession, BackendEvent, ProjectContext};

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceProjectionPatchInput {
    pub(crate) context: ProjectContext,
    pub(crate) revision: u64,
    pub(crate) cached: Option<gwt::ActiveWorkProjectionView>,
    pub(crate) fresh: Box<gwt_core::workspace_projection::WorkspaceProjection>,
    pub(crate) sessions: Vec<ActiveAgentSession>,
    pub(crate) windows: WorkspaceWindowTitles,
    pub(crate) window_generations: HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceProjectionPatchPrepared {
    pub(crate) context: ProjectContext,
    pub(crate) revision: u64,
    pub(crate) projection: gwt::ActiveWorkProjectionView,
    pub(crate) title_updates: Vec<WorkspaceTitleUpdate>,
    pub(crate) title_baselines: WorkspaceWindowTitles,
    pub(crate) window_generations: HashMap<String, u64>,
    pub(crate) payload: Arc<str>,
    history_routes: Vec<(AgentSlot, AgentSlot)>,
}

#[derive(Debug, Clone, Copy)]
enum AgentSlot {
    Assigned(usize),
    Unassigned(usize),
    Root {
        work: usize,
        agent: usize,
    },
    Child {
        work: usize,
        child: usize,
        agent: usize,
    },
}

impl AgentSlot {
    fn nested(self) -> bool {
        matches!(self, Self::Root { .. } | Self::Child { .. })
    }

    fn agent(
        self,
        projection: &mut gwt::ActiveWorkProjectionView,
    ) -> &mut gwt::ActiveWorkAgentView {
        match self {
            Self::Assigned(agent) => &mut projection.agents[agent],
            Self::Unassigned(agent) => &mut projection.unassigned_agents[agent],
            Self::Root { work, agent } => &mut projection.active_works[work].agents[agent],
            Self::Child { work, child, agent } => {
                &mut projection.active_works[work].works[child].agents[agent]
            }
        }
    }

    fn same_location(
        self,
        old: &gwt::ActiveWorkProjectionView,
        next: Self,
        new: &gwt::ActiveWorkProjectionView,
    ) -> bool {
        match (self, next) {
            (Self::Assigned(_), Self::Assigned(_)) | (Self::Unassigned(_), Self::Unassigned(_)) => {
                true
            }
            (Self::Root { work: old_work, .. }, Self::Root { work: new_work, .. }) => {
                old.active_works[old_work].id == new.active_works[new_work].id
            }
            (
                Self::Child {
                    work: old_work,
                    child: old_child,
                    ..
                },
                Self::Child {
                    work: new_work,
                    child: new_child,
                    ..
                },
            ) => {
                old.active_works[old_work].id == new.active_works[new_work].id
                    && old.active_works[old_work].works[old_child].id
                        == new.active_works[new_work].works[new_child].id
            }
            _ => false,
        }
    }
}

fn agent_slots(projection: &gwt::ActiveWorkProjectionView) -> Vec<(AgentSlot, &str)> {
    let mut slots = Vec::new();
    slots.extend(
        projection
            .agents
            .iter()
            .enumerate()
            .map(|(index, agent)| (AgentSlot::Assigned(index), agent.session_id.as_str())),
    );
    slots.extend(
        projection
            .unassigned_agents
            .iter()
            .enumerate()
            .map(|(index, agent)| (AgentSlot::Unassigned(index), agent.session_id.as_str())),
    );
    for (work_index, work) in projection.active_works.iter().enumerate() {
        slots.extend(work.agents.iter().enumerate().map(|(index, agent)| {
            (
                AgentSlot::Root {
                    work: work_index,
                    agent: index,
                },
                agent.session_id.as_str(),
            )
        }));
        for (child_index, child) in work.works.iter().enumerate() {
            slots.extend(child.agents.iter().enumerate().map(|(index, agent)| {
                (
                    AgentSlot::Child {
                        work: work_index,
                        child: child_index,
                        agent: index,
                    },
                    agent.session_id.as_str(),
                )
            }));
        }
    }
    slots
}

fn history_routes(
    old: &gwt::ActiveWorkProjectionView,
    new: &gwt::ActiveWorkProjectionView,
) -> Vec<(AgentSlot, AgentSlot)> {
    let mut sources: HashMap<(bool, &str), VecDeque<AgentSlot>> = HashMap::new();
    for (slot, session_id) in agent_slots(old) {
        sources
            .entry((slot.nested(), session_id))
            .or_default()
            .push_back(slot);
    }
    agent_slots(new)
        .into_iter()
        .filter_map(|(destination, session_id)| {
            let candidates = sources.get_mut(&(destination.nested(), session_id))?;
            // Keep the existing root/child occurrence where it survives. If the
            // membership moves, take the next history from that session's pool.
            let source = if let Some(index) = candidates
                .iter()
                .position(|slot| slot.same_location(old, destination, new))
            {
                candidates.remove(index)
            } else {
                candidates.pop_front()
            }?;
            Some((source, destination))
        })
        .collect()
}

pub(crate) fn prepare_workspace_projection_patch(
    input: WorkspaceProjectionPatchInput,
) -> Result<WorkspaceProjectionPatchPrepared, String> {
    let mut seen_windows = HashSet::new();
    let mut title_updates = resolve_workspace_projection_title_updates(
        &input.context.project_root,
        &input.fresh,
        &input.sessions,
        &input.windows,
    )
    .into_iter()
    .rev()
    .filter(|(id, _, _)| seen_windows.insert(id.clone()))
    .collect::<Vec<_>>();
    // One window can resolve from several agent summaries. Compare its final
    // intended title against the original baseline exactly once at commit.
    title_updates.reverse();
    let title_baselines = title_updates
        .iter()
        .filter_map(|(id, _, _)| {
            input
                .windows
                .get(id)
                .map(|titles| (id.clone(), titles.clone()))
        })
        .collect();
    let window_generations = title_updates
        .iter()
        .filter_map(|(id, _, _)| {
            input
                .window_generations
                .get(id)
                .map(|generation| (id.clone(), *generation))
        })
        .collect();
    let mut projection = match input.cached.as_ref() {
        Some(cached) => {
            let mut projection = cached.clone();
            super::merge_workspace_projection_membership_cache_only(
                &mut projection,
                &input.context.project_root,
                &input.fresh,
            );
            projection
        }
        None => {
            let mut projection = super::active_work_projection_from_saved_with_journal(
                *input.fresh,
                Vec::new(),
                Vec::new(),
                None,
            );
            super::assign_and_merge_workspace_groups_cache_only(
                &mut projection.active_works,
                &input.context.project_root,
            );
            projection
        }
    };
    let history_routes = input
        .cached
        .as_ref()
        .map(|cached| history_routes(cached, &projection))
        .unwrap_or_default();
    // Input is a bounded snapshot. Always use the existing bounded wire mapper
    // so journal/Work/Session histories cannot enter a watcher patch.
    projection = super::bounded_active_work_projection_snapshot(&projection);
    let event = BackendEvent::ActiveWorkProjectionPatch {
        projection: Box::new(projection.clone()),
    };
    let (payload, _) = super::serialize_active_work_projection_event_with(&event, |event| {
        serde_json::to_string(event).map_err(|error| error.to_string())
    })?;
    Ok(WorkspaceProjectionPatchPrepared {
        context: input.context,
        revision: input.revision,
        projection,
        title_updates,
        title_baselines,
        window_generations,
        payload,
        history_routes,
    })
}

impl WorkspaceProjectionPatchPrepared {
    /// Call only after checking the captured project context and cache revision.
    /// All matching and routing was done by the worker; GUI work only moves Vecs.
    pub(crate) fn restore_histories(&mut self, cached: &mut gwt::ActiveWorkProjectionView) {
        self.projection.works = std::mem::take(&mut cached.works);
        self.projection.journal_entries = std::mem::take(&mut cached.journal_entries);
        for &(source, destination) in &self.history_routes {
            destination.agent(&mut self.projection).sessions =
                std::mem::take(&mut source.agent(cached).sessions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::workspace_projection::{
        WorkspaceAgentAffiliationStatus, WorkspaceAgentSummary, WorkspaceProjection,
        WorkspaceStatusCategory,
    };

    #[test]
    fn prepared_watcher_patch_moves_each_duplicate_history_slot_without_cloning() {
        let project_root = std::path::PathBuf::from("/repo");
        let mut fresh = WorkspaceProjection::default_for_project(&project_root);
        fresh.agents.push(WorkspaceAgentSummary {
            session_id: "session-live".to_string(),
            window_id: Some("tab-1::agent-1".to_string()),
            agent_id: "codex".to_string(),
            display_name: "Codex".to_string(),
            status_category: WorkspaceStatusCategory::Active,
            current_focus: Some("Worker preparation".to_string()),
            title_summary: Some("Fresh title".to_string()),
            worktree_path: Some(project_root.clone()),
            branch: Some("work/live".to_string()),
            last_board_entry_id: None,
            last_board_entry_kind: None,
            coordination_scope: None,
            affiliation_status: WorkspaceAgentAffiliationStatus::Assigned,
            workspace_id: None,
            updated_at: chrono::Utc::now(),
        });
        let mut cached = super::super::active_work_projection_from_saved_with_journal(
            fresh.clone(),
            Vec::new(),
            Vec::new(),
            None,
        );
        super::super::assign_and_merge_workspace_groups_cache_only(
            &mut cached.active_works,
            &project_root,
        );
        let mut histories = Vec::new();
        for (slot, _) in agent_slots(&cached) {
            histories.push(slot);
        }
        assert_eq!(
            histories.len(),
            3,
            "one session occurs in top/root/child slots"
        );
        for &slot in &histories {
            slot.agent(&mut cached).sessions = vec![gwt::WorkspaceHistorySessionView {
                agent_session_id: "conversation-live".to_string(),
                started_at: "2026-10-07T00:00:00Z".to_string(),
                is_active: true,
                resumable: true,
            }];
        }
        let allocations = histories
            .iter()
            .map(|slot| slot.agent(&mut cached).sessions.as_ptr())
            .collect::<Vec<_>>();
        fresh.agents[0].status_category = WorkspaceStatusCategory::Blocked;
        let context = ProjectContext {
            tab_id: "tab-1".to_string(),
            project_key: gwt_core::repo_hash::compute_repo_hash("https://example.test/repo"),
            generation: 1,
            project_root,
        };
        let mut prepared = prepare_workspace_projection_patch(WorkspaceProjectionPatchInput {
            context,
            revision: 3,
            cached: Some(super::super::bounded_active_work_projection_snapshot(
                &cached,
            )),
            fresh: Box::new(fresh),
            sessions: Vec::new(),
            windows: HashMap::new(),
            window_generations: HashMap::new(),
        })
        .expect("prepare patch");
        assert!(prepared.payload.contains("active_work_projection_patch"));
        assert!(!prepared.payload.contains("conversation-live"));
        prepared.restore_histories(&mut cached);
        for (&slot, &allocation) in histories.iter().zip(&allocations) {
            assert_eq!(
                slot.agent(&mut prepared.projection).sessions.as_ptr(),
                allocation
            );
            assert_eq!(slot.agent(&mut prepared.projection).sessions.len(), 1);
            assert!(slot.agent(&mut cached).sessions.is_empty());
        }
    }
}

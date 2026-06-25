//! Knowledge bridge / project-index search handlers split out of
//! `app_runtime/mod.rs` for SPEC-3064 Phase 1 (Pass 1).
//!
//! Owns:
//! - The frontend request payloads ([`KnowledgeSearchRequest`],
//!   [`KnowledgeLoadRequest`], [`ProjectIndexSearchRequest`]) and the
//!   off-thread task payloads ([`KnowledgeRefreshTask`],
//!   [`KnowledgeSearchTask`], [`ProjectIndexSearchTask`])
//! - [`AppRuntime::load_knowledge_bridge_events`] /
//!   [`AppRuntime::search_knowledge_bridge_events`] /
//!   [`AppRuntime::search_project_index_events`] /
//!   [`AppRuntime::update_knowledge_bridge_phase_events`] — Knowledge window
//!   loaders and the blocking-task spawns backing them
//! - [`AppRuntime::rebuild_index_cell_events`] /
//!   [`AppRuntime::refresh_index_status_events`] — Settings.Index per-cell
//!   rebuild and health refresh (SPEC-1939)
//!
//! Behavior-preserving move: the cache/search implementations stay in
//! `crate::knowledge_bridge` / `crate::index_search`.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use gwt::KnowledgeKind;

use super::{
    knowledge_kind_for_preset, load_knowledge_bridge, normalize_branch_name, work_session_index,
    workspace_resume_owner_issue_number, workspace_work_item_view_from_item, AppRuntime,
    BackendEvent, IssueBranchLinkStore, OutboundEvent, UserEvent, WindowPreset,
};

pub struct KnowledgeSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) selected_number: Option<u64>,
}

pub struct KnowledgeLoadRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) kind: KnowledgeKind,
    pub(crate) request_id: Option<u64>,
    pub(crate) selected_number: Option<u64>,
    pub(crate) refresh: bool,
}

pub struct ProjectIndexSearchRequest<'a> {
    pub(crate) id: &'a str,
    pub(crate) query: &'a str,
    pub(crate) request_id: u64,
    pub(crate) scopes: Vec<gwt::IndexSearchScope>,
    pub(crate) worktree_hash: Option<String>,
    pub(crate) match_mode: gwt::IndexSearchMatchMode,
}

pub(super) struct KnowledgeRefreshTask {
    pub(super) client_id: String,
    pub(super) id: String,
    pub(super) project_root: PathBuf,
    pub(super) kind: KnowledgeKind,
    pub(super) request_id: Option<u64>,
    pub(super) selected_number: Option<u64>,
    pub(super) force: bool,
    pub(super) sessions_dir: PathBuf,
    pub(super) issue_link_cache_dir: PathBuf,
}

struct KnowledgeSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    kind: KnowledgeKind,
    query: String,
    request_id: u64,
    selected_number: Option<u64>,
    sessions_dir: PathBuf,
    issue_link_cache_dir: PathBuf,
}

struct ProjectIndexSearchTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    query: String,
    request_id: u64,
    scopes: Vec<gwt::IndexSearchScope>,
    worktree_hash: Option<String>,
    match_mode: gwt::IndexSearchMatchMode,
}

/// SPEC-2359 US-80: a debounced Start Work duplicate-work advisory query.
struct WorkAdvisoryTask {
    client_id: String,
    id: String,
    project_root: PathBuf,
    query: String,
    request_id: u64,
}

pub(super) fn knowledge_error_event(
    id: impl Into<String>,
    kind: KnowledgeKind,
    message: impl Into<String>,
    request_id: Option<u64>,
    query: Option<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeError {
        id: id.into(),
        knowledge_kind: kind,
        request_id,
        query,
        message: message.into(),
    }
}

fn knowledge_phase_update_error_event(
    id: impl Into<String>,
    request_id: u64,
    issue_number: u64,
    message: impl Into<String>,
) -> BackendEvent {
    BackendEvent::KnowledgeBridgePhaseUpdated {
        id: id.into(),
        request_id,
        issue_number,
        result: gwt::protocol::KnowledgePhaseUpdateResult::Error {
            message: message.into(),
        },
    }
}

fn knowledge_view_events(
    client_id: String,
    id: String,
    kind: KnowledgeKind,
    request_id: Option<u64>,
    view: gwt::KnowledgeBridgeView,
) -> Vec<OutboundEvent> {
    vec![
        OutboundEvent::reply(
            client_id.clone(),
            BackendEvent::KnowledgeEntries {
                id: id.clone(),
                knowledge_kind: kind,
                request_id,
                entries: view.entries,
                selected_number: view.selected_number,
                empty_message: view.empty_message,
                refresh_enabled: view.refresh_enabled,
            },
        ),
        OutboundEvent::reply(
            client_id,
            BackendEvent::KnowledgeDetail {
                id,
                knowledge_kind: kind,
                request_id,
                detail: view.detail,
            },
        ),
    ]
}

fn augment_knowledge_bridge_related_works_from_paths(
    project_root: &Path,
    sessions_dir: &Path,
    issue_link_cache_dir: &Path,
    view: &mut gwt::KnowledgeBridgeView,
) {
    let sessions = crate::session_ledger_cache::SessionLedgerCache::new().load(sessions_dir);
    let work_items = gwt_core::workspace_projection::load_workspace_work_items(project_root)
        .ok()
        .flatten()
        .map(|projection| projection.work_items)
        .unwrap_or_default();
    let issue_by_branch = load_issue_branch_links(project_root, issue_link_cache_dir);
    augment_knowledge_bridge_related_works(
        project_root,
        view,
        &work_items,
        &sessions,
        &issue_by_branch,
    );
}

fn augment_knowledge_bridge_related_works(
    project_root: &Path,
    view: &mut gwt::KnowledgeBridgeView,
    work_items: &[gwt_core::workspace_projection::WorkItem],
    sessions: &[gwt_agent::Session],
    issue_by_branch: &HashMap<String, u64>,
) {
    if !matches!(view.kind, KnowledgeKind::Issue | KnowledgeKind::Spec) {
        return;
    }
    let relevant_numbers = view
        .entries
        .iter()
        .map(|entry| entry.number)
        .chain(view.detail.number)
        .collect::<HashSet<_>>();
    if relevant_numbers.is_empty() {
        return;
    }

    let session_index = work_session_index(sessions);
    let mut related_by_number: HashMap<u64, Vec<gwt::KnowledgeRelatedWorkView>> = HashMap::new();
    let mut represented_sessions_by_number: HashMap<u64, HashSet<String>> = HashMap::new();

    for item in work_items {
        let Some(issue_number) = issue_number_for_work_item(item, &session_index, issue_by_branch)
        else {
            continue;
        };
        if !relevant_numbers.contains(&issue_number) {
            continue;
        }
        let work_view = workspace_work_item_view_from_item(item, &session_index, project_root);
        represented_sessions_by_number
            .entry(issue_number)
            .or_default()
            .extend(
                work_view
                    .agents
                    .iter()
                    .map(|agent| agent.session_id.clone()),
            );
        related_by_number
            .entry(issue_number)
            .or_default()
            .push(knowledge_related_work_from_workspace_history(work_view));
    }

    for session in sessions {
        let Some(issue_number) = issue_number_for_session(session, issue_by_branch) else {
            continue;
        };
        if !relevant_numbers.contains(&issue_number) {
            continue;
        }
        if represented_sessions_by_number
            .get(&issue_number)
            .is_some_and(|ids| ids.contains(&session.id))
        {
            continue;
        }
        represented_sessions_by_number
            .entry(issue_number)
            .or_default()
            .insert(session.id.clone());
        related_by_number
            .entry(issue_number)
            .or_default()
            .push(knowledge_related_work_from_session(session));
    }

    for works in related_by_number.values_mut() {
        works.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        dedupe_related_work_sessions(works, &session_index);
    }

    for entry in &mut view.entries {
        if let Some(works) = related_by_number.get(&entry.number) {
            entry.related_work_count = works.len();
            entry.related_session_count = related_session_count(works);
        }
    }
    if let Some(number) = view.detail.number {
        if let Some(works) = related_by_number.get(&number) {
            view.detail.related_works = works.clone();
        }
    }
}

fn issue_number_for_work_item(
    item: &gwt_core::workspace_projection::WorkItem,
    session_index: &HashMap<&str, &gwt_agent::Session>,
    issue_by_branch: &HashMap<String, u64>,
) -> Option<u64> {
    workspace_resume_owner_issue_number(item.owner.as_deref())
        .or_else(|| issue_number_for_unambiguous_work_item_branch(item, issue_by_branch))
        .or_else(|| {
            item.agents.iter().find_map(|agent| {
                session_index
                    .get(agent.session_id.as_str())
                    .and_then(|session| issue_number_for_session(session, issue_by_branch))
            })
        })
}

fn issue_number_for_unambiguous_work_item_branch(
    item: &gwt_core::workspace_projection::WorkItem,
    issue_by_branch: &HashMap<String, u64>,
) -> Option<u64> {
    let mut branch_containers = item.execution_containers.iter().filter_map(|container| {
        let branch = container.branch.as_deref()?.trim();
        if branch.is_empty() {
            return None;
        }
        Some(branch)
    });
    let branch = branch_containers.next()?;
    if branch_containers.next().is_some() {
        return None;
    }
    issue_number_for_branch(Some(branch), issue_by_branch)
}

fn issue_number_for_session(
    session: &gwt_agent::Session,
    issue_by_branch: &HashMap<String, u64>,
) -> Option<u64> {
    session
        .linked_issue_number
        .or_else(|| issue_number_for_branch(Some(session.branch.as_str()), issue_by_branch))
}

fn issue_number_for_branch(
    branch: Option<&str>,
    issue_by_branch: &HashMap<String, u64>,
) -> Option<u64> {
    let branch = branch?.trim();
    if branch.is_empty() {
        return None;
    }
    issue_by_branch
        .get(branch)
        .copied()
        .or_else(|| issue_by_branch.get(&normalize_branch_name(branch)).copied())
}

fn load_issue_branch_links(
    project_root: &Path,
    issue_link_cache_dir: &Path,
) -> HashMap<String, u64> {
    let Some(repo_hash) = gwt::index_worker::detect_repo_hash(project_root) else {
        return HashMap::new();
    };
    let path = issue_link_cache_dir
        .join("issue-links")
        .join(format!("{}.json", repo_hash.as_str()));
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    serde_json::from_slice::<IssueBranchLinkStore>(&bytes)
        .map(|store| store.branches)
        .unwrap_or_default()
}

fn knowledge_related_work_from_workspace_history(
    work: gwt::WorkspaceHistoryView,
) -> gwt::KnowledgeRelatedWorkView {
    let branch = work
        .execution_containers
        .iter()
        .find_map(|container| container.branch.clone());
    let worktree_path = work
        .execution_containers
        .iter()
        .find_map(|container| container.worktree_path.clone());
    gwt::KnowledgeRelatedWorkView {
        id: work.id,
        title: work.title,
        status_category: work.status_category,
        branch,
        worktree_path,
        updated_at: work.updated_at,
        agents: work
            .agents
            .into_iter()
            .map(knowledge_related_agent_from_workspace_history)
            .collect(),
    }
}

fn knowledge_related_agent_from_workspace_history(
    agent: gwt::WorkspaceHistoryAgentView,
) -> gwt::KnowledgeRelatedAgentView {
    gwt::KnowledgeRelatedAgentView {
        session_id: agent.session_id,
        agent_id: agent.agent_id,
        display_name: agent.display_name,
        updated_at: agent.updated_at,
        sessions: agent
            .sessions
            .into_iter()
            .map(|session| gwt::KnowledgeRelatedSessionView {
                agent_session_id: session.agent_session_id,
                started_at: session.started_at,
                is_active: session.is_active,
                resumable: session.resumable,
            })
            .collect(),
    }
}

fn knowledge_related_work_from_session(
    session: &gwt_agent::Session,
) -> gwt::KnowledgeRelatedWorkView {
    let updated_at = session
        .updated_at
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let is_active = session_status_is_active(session.status);
    let sessions = session
        .agent_session_id
        .as_ref()
        .map(|agent_session_id| {
            vec![gwt::KnowledgeRelatedSessionView {
                agent_session_id: agent_session_id.clone(),
                started_at: updated_at.clone(),
                is_active,
                resumable: session.is_resumable_conversation(agent_session_id),
            }]
        })
        .unwrap_or_default();
    gwt::KnowledgeRelatedWorkView {
        id: format!("work-session-{}", session.id),
        title: session.branch.clone(),
        status_category: session_status_category(session.status).to_string(),
        branch: Some(session.branch.clone()),
        worktree_path: Some(session.worktree_path.display().to_string()),
        updated_at: updated_at.clone(),
        agents: vec![gwt::KnowledgeRelatedAgentView {
            session_id: session.id.clone(),
            agent_id: Some(session.agent_id.command().to_string()),
            display_name: Some(session.display_name.clone()),
            updated_at,
            sessions,
        }],
    }
}

fn session_status_category(status: gwt_agent::AgentStatus) -> &'static str {
    match status {
        gwt_agent::AgentStatus::Running | gwt_agent::AgentStatus::WaitingInput => "active",
        gwt_agent::AgentStatus::Idle | gwt_agent::AgentStatus::Stopped => "idle",
        gwt_agent::AgentStatus::Interrupted => "blocked",
        gwt_agent::AgentStatus::Unknown => "unknown",
    }
}

fn session_status_is_active(status: gwt_agent::AgentStatus) -> bool {
    matches!(
        status,
        gwt_agent::AgentStatus::Running | gwt_agent::AgentStatus::WaitingInput
    )
}

fn related_session_count(works: &[gwt::KnowledgeRelatedWorkView]) -> usize {
    works
        .iter()
        .flat_map(|work| work.agents.iter())
        .flat_map(|agent| agent.sessions.iter())
        .count()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RelatedSessionSlot {
    work_index: usize,
    agent_index: usize,
    session_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RelatedSessionRank {
    runtime_live: bool,
    recency_millis: i64,
    session_id: String,
}

fn dedupe_related_work_sessions(
    works: &mut Vec<gwt::KnowledgeRelatedWorkView>,
    session_index: &HashMap<&str, &gwt_agent::Session>,
) {
    let mut best_by_conversation: HashMap<String, (RelatedSessionRank, RelatedSessionSlot)> =
        HashMap::new();
    let mut best_by_action: HashMap<String, (RelatedSessionRank, RelatedSessionSlot)> =
        HashMap::new();

    for (work_index, work) in works.iter().enumerate() {
        for (agent_index, agent) in work.agents.iter().enumerate() {
            for (session_index_value, session) in agent.sessions.iter().enumerate() {
                let slot = RelatedSessionSlot {
                    work_index,
                    agent_index,
                    session_index: session_index_value,
                };
                let rank = related_session_rank(work, agent, session, session_index);
                if let Some(conversation_key) = related_conversation_key(session) {
                    update_best_related_session(
                        &mut best_by_conversation,
                        conversation_key,
                        rank.clone(),
                        slot.clone(),
                    );
                }
                if let Some(action_key) = related_action_key(work, agent) {
                    update_best_related_session(&mut best_by_action, action_key, rank, slot);
                }
            }
        }
    }

    let mut keep = HashSet::new();
    let mut action_keys_with_kept_sessions = HashSet::new();
    for (work_index, work) in works.iter().enumerate() {
        for (agent_index, agent) in work.agents.iter().enumerate() {
            for (session_index_value, session) in agent.sessions.iter().enumerate() {
                let slot = RelatedSessionSlot {
                    work_index,
                    agent_index,
                    session_index: session_index_value,
                };
                let is_best_conversation =
                    related_conversation_key(session).is_none_or(|conversation_key| {
                        best_by_conversation
                            .get(&conversation_key)
                            .is_some_and(|(_, best)| *best == slot)
                    });
                let is_best_action = related_action_key(work, agent).is_none_or(|action_key| {
                    best_by_action
                        .get(&action_key)
                        .is_some_and(|(_, best)| *best == slot)
                });
                if is_best_conversation && is_best_action {
                    if let Some(action_key) = related_action_key(work, agent) {
                        action_keys_with_kept_sessions.insert(action_key);
                    }
                    keep.insert(slot);
                }
            }
        }
    }

    let mut work_index = 0usize;
    works.retain_mut(|work| {
        let current_work_index = work_index;
        work_index += 1;
        let work_had_sessions = work.agents.iter().any(|agent| !agent.sessions.is_empty());
        let worktree = work
            .worktree_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string();
        let branch = work
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("")
            .to_string();
        let mut agent_index = 0usize;
        work.agents.retain_mut(|agent| {
            let current_agent_index = agent_index;
            agent_index += 1;
            let agent_had_sessions = !agent.sessions.is_empty();
            let mut session_index_value = 0usize;
            agent.sessions.retain(|_| {
                let slot = RelatedSessionSlot {
                    work_index: current_work_index,
                    agent_index: current_agent_index,
                    session_index: session_index_value,
                };
                session_index_value += 1;
                keep.contains(&slot)
            });
            if agent_had_sessions {
                return !agent.sessions.is_empty();
            }
            if !work_had_sessions {
                return true;
            }
            related_action_key_from_parts(&worktree, &branch, agent)
                .is_none_or(|action_key| !action_keys_with_kept_sessions.contains(&action_key))
        });
        !work_had_sessions || !work.agents.is_empty()
    });
}

fn update_best_related_session(
    best_by_key: &mut HashMap<String, (RelatedSessionRank, RelatedSessionSlot)>,
    key: String,
    rank: RelatedSessionRank,
    slot: RelatedSessionSlot,
) {
    if best_by_key
        .get(&key)
        .is_none_or(|(best_rank, _)| rank > *best_rank)
    {
        best_by_key.insert(key, (rank, slot));
    }
}

fn related_session_rank(
    work: &gwt::KnowledgeRelatedWorkView,
    agent: &gwt::KnowledgeRelatedAgentView,
    session: &gwt::KnowledgeRelatedSessionView,
    session_index: &HashMap<&str, &gwt_agent::Session>,
) -> RelatedSessionRank {
    let runtime_live = session_index
        .get(agent.session_id.as_str())
        .is_some_and(|ledger| session_status_is_active(ledger.status));
    let recency_millis = [
        parse_related_time_millis(&session.started_at),
        parse_related_time_millis(&agent.updated_at),
        parse_related_time_millis(&work.updated_at),
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    RelatedSessionRank {
        runtime_live,
        recency_millis,
        session_id: agent.session_id.clone(),
    }
}

fn related_conversation_key(session: &gwt::KnowledgeRelatedSessionView) -> Option<String> {
    let conversation = session.agent_session_id.trim();
    (!conversation.is_empty()).then(|| conversation.to_string())
}

fn related_action_key(
    work: &gwt::KnowledgeRelatedWorkView,
    agent: &gwt::KnowledgeRelatedAgentView,
) -> Option<String> {
    let worktree = work
        .worktree_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let branch = work
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    related_action_key_from_parts(worktree, branch, agent)
}

fn related_action_key_from_parts(
    worktree: &str,
    branch: &str,
    agent: &gwt::KnowledgeRelatedAgentView,
) -> Option<String> {
    if worktree.is_empty() && branch.is_empty() {
        return None;
    }
    let agent_label = agent
        .display_name
        .as_deref()
        .or(agent.agent_id.as_deref())
        .unwrap_or(agent.session_id.as_str())
        .trim()
        .to_ascii_lowercase();
    Some(format!("{worktree}\u{0}{branch}\u{0}{agent_label}"))
}

fn parse_related_time_millis(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .unwrap_or_default()
}

impl AppRuntime {
    fn augment_knowledge_bridge_related_works(
        &self,
        project_root: &Path,
        view: &mut gwt::KnowledgeBridgeView,
    ) {
        let sessions = self
            .session_ledger_cache
            .borrow_mut()
            .load(&self.sessions_dir);
        let work_items = self
            .work_items_cache
            .borrow_mut()
            .load_or_synthesize(project_root)
            .map(|projection| projection.work_items)
            .unwrap_or_default();
        let issue_by_branch = load_issue_branch_links(project_root, &self.issue_link_cache_dir);
        augment_knowledge_bridge_related_works(
            project_root,
            view,
            &work_items,
            &sessions,
            &issue_by_branch,
        );
    }

    pub(crate) fn load_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeLoadRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Project tab not found", request.request_id, None),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, "Window not found", request.request_id, None),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    request.request_id,
                    None,
                ),
            )];
        }

        if request.refresh {
            self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                client_id: client_id.to_string(),
                id: id.to_string(),
                project_root: tab.project_root.clone(),
                kind,
                request_id: request.request_id,
                selected_number: request.selected_number,
                force: true,
                sessions_dir: self.sessions_dir.clone(),
                issue_link_cache_dir: self.issue_link_cache_dir.clone(),
            });
            return Vec::new();
        }

        match load_knowledge_bridge(&tab.project_root, kind, request.selected_number, false) {
            Ok(mut view) => {
                self.augment_knowledge_bridge_related_works(&tab.project_root, &mut view);
                if request.request_id.is_some() && view.refresh_enabled {
                    self.spawn_knowledge_bridge_refresh(KnowledgeRefreshTask {
                        client_id: client_id.to_string(),
                        id: id.to_string(),
                        project_root: tab.project_root.clone(),
                        kind,
                        request_id: request.request_id,
                        selected_number: request.selected_number,
                        force: false,
                        sessions_dir: self.sessions_dir.clone(),
                        issue_link_cache_dir: self.issue_link_cache_dir.clone(),
                    });
                }
                knowledge_view_events(
                    client_id.to_string(),
                    id.to_string(),
                    kind,
                    request.request_id,
                    view,
                )
            }
            Err(error) => vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(id, kind, error, request.request_id, None),
            )],
        }
    }

    pub(super) fn spawn_knowledge_bridge_refresh(&self, task: KnowledgeRefreshTask) {
        let KnowledgeRefreshTask {
            client_id,
            id,
            project_root,
            kind,
            request_id,
            selected_number,
            force,
            sessions_dir,
            issue_link_cache_dir,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let refreshed = match gwt::refresh_knowledge_bridge_cache(&project_root, force) {
                Ok(refreshed) => refreshed,
                Err(error) => {
                    if force {
                        proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                            client_id,
                            knowledge_error_event(id, kind, error, request_id, None),
                        )]));
                    }
                    return;
                }
            };
            if !force && !refreshed {
                return;
            }
            let event =
                match gwt::load_knowledge_bridge(&project_root, kind, selected_number, false) {
                    Ok(mut view) => {
                        augment_knowledge_bridge_related_works_from_paths(
                            &project_root,
                            &sessions_dir,
                            &issue_link_cache_dir,
                            &mut view,
                        );
                        knowledge_view_events(client_id, id, kind, request_id, view)
                    }
                    Err(error) => vec![OutboundEvent::reply(
                        client_id,
                        knowledge_error_event(id, kind, error, request_id, None),
                    )],
                };
            proxy.send(UserEvent::Dispatch(event));
        });
    }

    pub(crate) fn search_knowledge_bridge_events(
        &self,
        client_id: &str,
        request: KnowledgeSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let kind = request.kind;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Project tab not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window not found",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset) != Some(kind) {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_error_event(
                    id,
                    kind,
                    "Window is not a knowledge bridge",
                    Some(request.request_id),
                    Some(request.query.to_string()),
                ),
            )];
        }

        self.spawn_knowledge_bridge_search(KnowledgeSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            kind,
            query: request.query.to_string(),
            request_id: request.request_id,
            selected_number: request.selected_number,
            sessions_dir: self.sessions_dir.clone(),
            issue_link_cache_dir: self.issue_link_cache_dir.clone(),
        });
        Vec::new()
    }

    fn spawn_knowledge_bridge_search(&self, task: KnowledgeSearchTask) {
        let KnowledgeSearchTask {
            client_id,
            id,
            project_root,
            kind,
            query,
            request_id,
            selected_number,
            sessions_dir,
            issue_link_cache_dir,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event =
                match gwt::search_knowledge_bridge(&project_root, kind, &query, selected_number) {
                    Ok(mut view) => {
                        augment_knowledge_bridge_related_works_from_paths(
                            &project_root,
                            &sessions_dir,
                            &issue_link_cache_dir,
                            &mut view,
                        );
                        BackendEvent::KnowledgeSearchResults {
                            id: id.clone(),
                            knowledge_kind: kind,
                            query: query.clone(),
                            request_id,
                            entries: view.entries,
                            selected_number: view.selected_number,
                            empty_message: view.empty_message,
                            refresh_enabled: view.refresh_enabled,
                        }
                    }
                    Err(error) => {
                        knowledge_error_event(id, kind, error, Some(request_id), Some(query))
                    }
                };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    pub(crate) fn search_project_index_events(
        &self,
        client_id: &str,
        request: ProjectIndexSearchRequest<'_>,
    ) -> Vec<OutboundEvent> {
        let id = request.id;
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Project tab not found".to_string(),
                },
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window not found".to_string(),
                },
            )];
        };
        if window.preset != WindowPreset::Index {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::ProjectIndexSearchError {
                    id: id.to_string(),
                    query: request.query.to_string(),
                    request_id: request.request_id,
                    message: "Window is not an Index surface".to_string(),
                },
            )];
        }

        self.spawn_project_index_search(ProjectIndexSearchTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root: tab.project_root.clone(),
            query: request.query.to_string(),
            request_id: request.request_id,
            scopes: request.scopes,
            worktree_hash: request.worktree_hash,
            match_mode: request.match_mode,
        });
        Vec::new()
    }

    /// SPEC-2359 US-80: run the Start Work duplicate-work advisory for the
    /// wizard's project. Advisory is non-blocking — on any resolution failure we
    /// reply with an empty advisory rather than an error (FR-415).
    pub(crate) fn request_work_advisory_events(
        &self,
        client_id: &str,
        id: &str,
        query: &str,
        request_id: u64,
    ) -> Vec<OutboundEvent> {
        // The Launch Wizard is a modal bound to the client's active project
        // tab (not a registered window), so resolve the project from the active
        // tab the same way wizard actions do.
        let project_root = self
            .active_tab_id
            .clone()
            .and_then(|tab_id| self.tab(&tab_id))
            .map(|tab| tab.project_root.clone());
        let Some(project_root) = project_root else {
            return vec![OutboundEvent::reply(
                client_id,
                BackendEvent::WorkAdvisoryResult {
                    id: id.to_string(),
                    query: query.to_string(),
                    request_id,
                    results: Vec::new(),
                },
            )];
        };
        self.spawn_work_advisory(WorkAdvisoryTask {
            client_id: client_id.to_string(),
            id: id.to_string(),
            project_root,
            query: query.to_string(),
            request_id,
        });
        Vec::new()
    }

    fn spawn_work_advisory(&self, task: WorkAdvisoryTask) {
        let WorkAdvisoryTask {
            client_id,
            id,
            project_root,
            query,
            request_id,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            // Advisory never blocks the launch: an error yields an empty
            // advisory (SPEC-2359 FR-415), never a surfaced failure.
            let results = gwt::work_advisory(&project_root, &query).unwrap_or_default();
            let event = BackendEvent::WorkAdvisoryResult {
                id,
                query,
                request_id,
                results,
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    fn spawn_project_index_search(&self, task: ProjectIndexSearchTask) {
        let ProjectIndexSearchTask {
            client_id,
            id,
            project_root,
            query,
            request_id,
            scopes,
            worktree_hash,
            match_mode,
        } = task;
        let proxy = self.proxy.clone();
        self.blocking_tasks.spawn(move || {
            let event = match gwt::search_project_index(
                &project_root,
                &query,
                &scopes,
                worktree_hash.as_deref(),
                match_mode,
                // GUI interactive search: the watcher owns index builds.
                false,
            ) {
                Ok(outcome) => BackendEvent::ProjectIndexSearchResults {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    results: outcome.results,
                    suggestions: outcome.suggestions,
                },
                Err(error) => BackendEvent::ProjectIndexSearchError {
                    id: id.clone(),
                    query: query.clone(),
                    request_id,
                    message: error,
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                client_id, event,
            )]));
        });
    }

    /// SPEC-2017 US-8 — Apply a Kanban phase change to the owning
    /// GitHub Issue. Validates that the target window is a knowledge
    /// bridge surface and dispatches a blocking task that calls
    /// `gwt::update_knowledge_phase`. The result is delivered as
    /// [`BackendEvent::KnowledgeBridgePhaseUpdated`] so the optimistic
    /// frontend UI can either confirm or rollback.
    pub(crate) fn update_knowledge_bridge_phase_events(
        &self,
        client_id: &str,
        id: &str,
        request_id: u64,
        issue_number: u64,
        target_phase: Option<&str>,
    ) -> Vec<OutboundEvent> {
        let Some(address) = self.window_lookup.get(id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        let Some(tab) = self.tab(&address.tab_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Project tab not found",
                ),
            )];
        };
        let Some(window) = tab.workspace.window(&address.raw_id) else {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window not found",
                ),
            )];
        };
        if knowledge_kind_for_preset(window.preset).is_none() {
            return vec![OutboundEvent::reply(
                client_id,
                knowledge_phase_update_error_event(
                    id,
                    request_id,
                    issue_number,
                    "Window is not a knowledge bridge",
                ),
            )];
        }

        let proxy = self.proxy.clone();
        let client_id = client_id.to_string();
        let id_owned = id.to_string();
        let project_root = tab.project_root.clone();
        let target_phase = target_phase.map(str::to_string);
        self.blocking_tasks.spawn(move || {
            let event = match gwt::update_knowledge_phase(
                &project_root,
                issue_number,
                target_phase.as_deref(),
            ) {
                Ok(fresh_entry) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Ok { fresh_entry },
                },
                Err(error) => BackendEvent::KnowledgeBridgePhaseUpdated {
                    id: id_owned,
                    request_id,
                    issue_number,
                    result: gwt::protocol::KnowledgePhaseUpdateResult::Error { message: error },
                },
            };
            proxy.send(UserEvent::Dispatch(vec![OutboundEvent::reply(
                &client_id, event,
            )]));
        });
        Vec::new()
    }

    /// SPEC-1939 US-5 / T-IDX-102: handle a per-cell rebuild request from the
    /// frontend. Spawns the rebuild via the global bootstrap service so the
    /// in-flight set is shared with the orchestrator and CLI.
    pub(crate) fn rebuild_index_cell_events(
        &self,
        project_root: String,
        scope: gwt::IndexRebuildScope,
        worktree_hash: Option<String>,
    ) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = crate::project_index_bootstrap::spawn_per_cell_rebuild(
            service,
            self.proxy.clone(),
            project_root,
            scope,
            worktree_hash,
        );
        Vec::new()
    }

    /// Settings.Index requests the full all-worktree health table on demand.
    /// The startup path stays current-worktree only to avoid UI-visible CPU
    /// spikes on repositories with many active worktrees.
    pub(crate) fn refresh_index_status_events(&self, project_root: String) -> Vec<OutboundEvent> {
        let project_root = std::path::PathBuf::from(project_root);
        let service =
            crate::project_index_bootstrap::ProjectIndexBootstrapService::global().clone();
        let _request = service.spawn_full_status_refresh(self.proxy.clone(), project_root);
        Vec::new()
    }
}

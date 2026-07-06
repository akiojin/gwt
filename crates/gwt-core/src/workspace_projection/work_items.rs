//! Work item / Work event data model: the event-sourced history entities
//! ([`WorkEvent`], [`WorkEventKind`]) and the hot [`WorkItemsProjection`]
//! fold that turns recorded events into current Work items, plus the legacy
//! `Workspace*`-prefixed adapter aliases.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::*;

/// SPEC-2359 Phase U-6 (FR-133): structured reference to a GitHub Issue
/// linked to a Workspace. Workspace Card preview and Detail pane render these
/// as chips (`#Issue-1234`) instead of free-text. The number is required;
/// title / url are populated when known and default to None for legacy data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceIssueLink {
    pub number: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// SPEC-2359 Phase U-6 (FR-133): structured reference to a GitHub Pull
/// Request linked to a Workspace. Carries `state` (e.g. open / merged /
/// closed) so UI can render lifecycle hints alongside `lifecycle_stage`
/// without re-querying GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePrLink {
    pub number: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
}

/// Reference from a Work item to one agent session that worked on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAgentRef {
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    pub updated_at: DateTime<Utc>,
    /// Issue #3216: kind of the event that first attached this session, kept
    /// on the ref so mis-attribution stays diagnosable from works.json alone
    /// after the work-events journal is compacted. `None` for legacy refs and
    /// synthesized (non-event) attach paths.
    #[serde(default)]
    pub attached_by: Option<WorkEventKind>,
}

/// Reference from a Work item to the branch / worktree / PR it executed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceExecutionContainerRef {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<PathBuf>,
    #[serde(default)]
    pub pr_number: Option<u64>,
    #[serde(default)]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub pr_state: Option<String>,
}

/// Lifecycle event kind in a Work item's history (start, claim, update,
/// pause, done, ...). Each kind maps to one [`WorkEvent`] record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkEventKind {
    Start,
    Claim,
    Update,
    Blocked,
    Handoff,
    Resume,
    Split,
    Merge,
    Pr,
    /// SPEC-2359 Phase W-12 Slice 5a (FR-350): the owning agent session stopped
    /// without an explicit user close. The Work is retained as Paused (not Done)
    /// so it stays on the Work surface until the user closes it.
    Pause,
    Done,
    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): the user explicitly discarded the
    /// Work from the Work surface. This is a terminal close distinct from Done:
    /// the Work leaves the active surface but its provenance is retained as
    /// discarded (not completed). Agent stop alone never yields Discard.
    Discard,
    /// SPEC-2359 Phase W-15 (FR-380): a worktree existed on disk without any
    /// matching Work record, so reconciliation materialized one. The event
    /// must not carry an explicit `status_category`: `apply_event` only
    /// preserves terminal (Done/Discarded) items against implicit-status
    /// events, and a committed Backfill event may be re-ingested on another
    /// machine after the Work was closed there (W-16 intake).
    Backfill,
}

/// One append-only event in a Work item's lifecycle. Events are folded into
/// [`WorkItem`]s by [`WorkItemsProjection`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEvent {
    pub id: String,
    pub work_item_id: String,
    pub kind: WorkEventKind,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub progress_summary: Option<String>,
    #[serde(default)]
    pub status_category: Option<WorkspaceStatusCategory>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub board_entry_id: Option<String>,
    #[serde(default)]
    pub execution_container: Option<WorkspaceExecutionContainerRef>,
    #[serde(default)]
    pub related_work_item_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl WorkEvent {
    pub fn new(
        kind: WorkEventKind,
        work_item_id: impl Into<String>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            work_item_id: work_item_id.into(),
            kind,
            title: None,
            intent: None,
            summary: None,
            progress_summary: None,
            status_category: None,
            owner: None,
            next_action: None,
            agent_session_id: None,
            agent_id: None,
            display_name: None,
            board_entry_id: None,
            execution_container: None,
            related_work_item_id: None,
            updated_at,
        }
    }
}

/// One unit of work on the Work surface: title, status, participating
/// agents, execution containers, and its event history. Built by folding
/// [`WorkEvent`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub progress_summary: Option<String>,
    pub status_category: WorkspaceStatusCategory,
    #[serde(default)]
    pub owner: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub agents: Vec<WorkAgentRef>,
    #[serde(default)]
    pub execution_containers: Vec<WorkspaceExecutionContainerRef>,
    #[serde(default)]
    pub board_refs: Vec<String>,
    #[serde(default)]
    pub related_work_item_ids: Vec<String>,
    #[serde(default)]
    pub events: Vec<WorkEvent>,
    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): terminal discarded close. A
    /// discarded Work is removed from the active Work surface but kept in the
    /// history with its provenance. Distinct from `status_category == Done`
    /// (which marks completion); `discarded` marks an explicit user discard.
    /// Back-compat default is `false` for projections written before W-12.
    #[serde(default)]
    pub discarded: bool,
}

impl WorkItem {
    /// A Work is incomplete while it is neither completed (Done) nor discarded.
    /// Both Done and Discarded are terminal closes (FR-352).
    pub fn is_incomplete(&self) -> bool {
        self.status_category != WorkspaceStatusCategory::Done && !self.discarded
    }

    /// SPEC-2359 Phase W-12 Slice 4 (FR-352): true when the Work has reached a
    /// terminal close — either completed (Done) or explicitly discarded.
    pub fn is_terminal(&self) -> bool {
        self.status_category == WorkspaceStatusCategory::Done || self.discarded
    }

    /// #3065: the most recent non-empty `next_action` across this Work's
    /// events. Used to build a per-work-item resume context instead of
    /// reading the repo-shared current projection.
    pub fn latest_next_action(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|event| {
            event
                .next_action
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    }
}

/// Materialized collection of all Work items for one project, rebuilt by
/// folding the Work event log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemsProjection {
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub work_items: Vec<WorkItem>,
}

impl WorkItemsProjection {
    pub fn empty(updated_at: DateTime<Utc>) -> Self {
        Self {
            updated_at,
            work_items: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: WorkEvent) {
        let existing_index = self
            .work_items
            .iter()
            .position(|item| item.id == event.work_item_id);
        let index = existing_index.unwrap_or_else(|| {
            self.work_items.push(WorkItem {
                id: event.work_item_id.clone(),
                title: event
                    .title
                    .clone()
                    .or_else(|| event.intent.clone())
                    .unwrap_or_else(|| event.work_item_id.clone()),
                intent: event.intent.clone(),
                summary: event.summary.clone(),
                progress_summary: event.progress_summary.clone(),
                status_category: workspace_work_event_status(&event),
                owner: event.owner.clone(),
                created_at: event.updated_at,
                updated_at: event.updated_at,
                completed_at: None,
                agents: Vec::new(),
                execution_containers: Vec::new(),
                board_refs: Vec::new(),
                related_work_item_ids: Vec::new(),
                events: Vec::new(),
                discarded: false,
            });
            self.work_items.len() - 1
        });

        // Issue #3216: FR-348 gives "1 agent session : 1 Work". When an event
        // would attach a session that another Work already owns AND the two
        // Works carry conflicting git identities, the pairing is a
        // mis-attribution (the event's work_item_id and session were assembled
        // from divergent sources, e.g. the repo-shared current projection vs
        // the live session). The attach is refused below; the event itself
        // stays recorded for diagnosis.
        let stray_session_attach = event
            .agent_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(|session_id| {
                work_session_attach_conflicts(
                    &self.work_items,
                    index,
                    session_id,
                    event.execution_container.as_ref(),
                )
            });

        let item = &mut self.work_items[index];
        // SPEC-2359 Phase W-16 (FR-403): a Backfill event is a synthetic
        // materialization marker, not activity. Applied to an existing item
        // (a duplicated / replayed backfill line), it must not advance
        // `updated_at`, overwrite the real title, or touch status — otherwise
        // every materialized row collapses onto the replay instant and the
        // recency sort degenerates. Only the execution container may merge.
        if existing_index.is_some() && event.kind == WorkEventKind::Backfill {
            if let Some(container) = event.execution_container.clone() {
                if !item
                    .execution_containers
                    .iter()
                    .any(|existing| workspace_execution_container_same(existing, &container))
                {
                    item.execution_containers.push(container);
                }
            }
            item.events.push(event);
            refresh_work_item_progress_summary(item);
            return;
        }
        if let Some(title) = non_empty_clone(event.title.as_deref()) {
            item.title = title;
        }
        if let Some(intent) = non_empty_clone(event.intent.as_deref()) {
            item.intent = Some(intent);
        }
        if let Some(summary) = non_empty_clone(event.summary.as_deref()) {
            item.summary = Some(summary);
        }
        if let Some(progress_summary) = non_empty_clone(event.progress_summary.as_deref()) {
            item.progress_summary = Some(progress_summary);
        }
        if let Some(owner) = non_empty_clone(event.owner.as_deref()) {
            item.owner = Some(owner);
        }
        // SPEC-2359 US-37: Done is a terminal state. Heartbeat update events
        // (kind=Update with status_category=None) emitted after a Done event
        // must not regress the WorkItem to Active/Idle. Only events that
        // carry an explicit `status_category` may transition out of Done.
        //
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): Discarded is likewise terminal.
        // Once a Work is discarded, subsequent events (heartbeat updates without
        // an explicit status_category) must not regress its runtime status; the
        // `discarded` flag is monotonic and never reset.
        let new_status = workspace_work_event_status(&event);
        let preserve_terminal = (item.status_category == WorkspaceStatusCategory::Done
            || item.discarded)
            && event.status_category.is_none();
        if !preserve_terminal {
            item.status_category = new_status;
        }
        if event.kind == WorkEventKind::Discard {
            item.discarded = true;
        }
        if item.status_category == WorkspaceStatusCategory::Done {
            // Preserve the first Done timestamp so idempotent Done re-applies
            // (e.g. retroactive_auto_done_scan rerun) keep the original
            // completion time.
            item.completed_at = item.completed_at.or(Some(event.updated_at));
        } else {
            item.completed_at = None;
        }
        item.updated_at = event.updated_at;
        if item.created_at > event.updated_at {
            item.created_at = event.updated_at;
        }
        if let Some(session_id) = non_empty_clone(event.agent_session_id.as_deref()) {
            if let Some(agent) = item
                .agents
                .iter_mut()
                .find(|agent| agent.session_id == session_id)
            {
                agent.agent_id = event.agent_id.clone().or(agent.agent_id.clone());
                agent.display_name = event.display_name.clone().or(agent.display_name.clone());
                agent.updated_at = event.updated_at;
            } else if stray_session_attach {
                tracing::warn!(
                    target: "gwt::workspace_projection",
                    work_item_id = %event.work_item_id,
                    session_id = %session_id,
                    event_kind = ?event.kind,
                    "refused stray session attach: session already bound to a Work \
                     with a conflicting git identity (Issue #3216)"
                );
            } else {
                item.agents.push(WorkAgentRef {
                    session_id,
                    agent_id: event.agent_id.clone(),
                    display_name: event.display_name.clone(),
                    updated_at: event.updated_at,
                    attached_by: Some(event.kind),
                });
            }
        }
        if let Some(container) = event.execution_container.clone() {
            if !item
                .execution_containers
                .iter()
                .any(|existing| workspace_execution_container_same(existing, &container))
            {
                item.execution_containers.push(container);
            }
        }
        if let Some(board_entry_id) = non_empty_clone(event.board_entry_id.as_deref()) {
            push_unique(&mut item.board_refs, board_entry_id);
        }
        if let Some(related_work_item_id) = non_empty_clone(event.related_work_item_id.as_deref()) {
            push_unique(&mut item.related_work_item_ids, related_work_item_id);
        }
        let event_updated_at = event.updated_at;
        item.events.push(event);
        item.events.sort_by_key(|event| event.updated_at);
        refresh_work_item_progress_summary(item);
        if event_updated_at > self.updated_at {
            self.updated_at = event_updated_at;
        }
        self.work_items
            .sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    }

    pub fn refresh_derived_progress_summaries(&mut self) {
        for item in &mut self.work_items {
            backfill_work_item_progress_summary(item);
        }
    }
}

fn workspace_work_event_status(event: &WorkEvent) -> WorkspaceStatusCategory {
    event.status_category.unwrap_or(match event.kind {
        WorkEventKind::Done => WorkspaceStatusCategory::Done,
        WorkEventKind::Blocked => WorkspaceStatusCategory::Blocked,
        // Pause keeps the Work incomplete (non-Done) while the agent is stopped;
        // the Idle status preserves the retained-but-not-running semantics.
        WorkEventKind::Pause => WorkspaceStatusCategory::Idle,
        // Discard does not complete the Work (status stays non-Done); the
        // terminal close is carried by the `discarded` flag (FR-352). Idle
        // mirrors the retained-but-not-running runtime status.
        WorkEventKind::Discard => WorkspaceStatusCategory::Idle,
        // Backfill materializes a Work for an existing worktree with no live
        // agent, so it surfaces as retained-but-not-running (rendered Paused).
        WorkEventKind::Backfill => WorkspaceStatusCategory::Idle,
        WorkEventKind::Start
        | WorkEventKind::Claim
        | WorkEventKind::Update
        | WorkEventKind::Handoff
        | WorkEventKind::Resume
        | WorkEventKind::Split
        | WorkEventKind::Merge
        | WorkEventKind::Pr => WorkspaceStatusCategory::Active,
    })
}

fn workspace_execution_container_same(
    left: &WorkspaceExecutionContainerRef,
    right: &WorkspaceExecutionContainerRef,
) -> bool {
    (left.branch.is_some() && left.branch == right.branch)
        || (left.worktree_path.is_some() && left.worktree_path == right.worktree_path)
        || (left.pr_number.is_some() && left.pr_number == right.pr_number)
        || (left.pr_url.is_some() && left.pr_url == right.pr_url)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

/// Issue #3216: true when `session_id` is already bound to a *different* Work
/// item whose git identity conflicts with the target item's identity (the
/// target's recorded containers plus the incoming event's container). Mirrors
/// the view-layer identity-conflict gate: a match on either dimension clears
/// the conflict, and a side without any identity never conflicts.
fn work_session_attach_conflicts(
    work_items: &[WorkItem],
    target_index: usize,
    session_id: &str,
    event_container: Option<&WorkspaceExecutionContainerRef>,
) -> bool {
    let target_containers = work_items[target_index]
        .execution_containers
        .iter()
        .chain(event_container)
        .collect::<Vec<_>>();
    work_items.iter().enumerate().any(|(index, other)| {
        index != target_index
            && other
                .agents
                .iter()
                .any(|agent| agent.session_id == session_id)
            && work_container_identities_conflict(&other.execution_containers, &target_containers)
    })
}

fn work_container_identities_conflict(
    owner_containers: &[WorkspaceExecutionContainerRef],
    target_containers: &[&WorkspaceExecutionContainerRef],
) -> bool {
    let owner_branches = owner_containers
        .iter()
        .filter_map(|container| normalized_work_branch(container.branch.as_deref()))
        .collect::<Vec<_>>();
    let target_branches = target_containers
        .iter()
        .filter_map(|container| normalized_work_branch(container.branch.as_deref()))
        .collect::<Vec<_>>();
    let owner_worktrees = owner_containers
        .iter()
        .filter_map(|container| container.worktree_path.as_deref())
        .collect::<Vec<_>>();
    let target_worktrees = target_containers
        .iter()
        .filter_map(|container| container.worktree_path.as_deref())
        .collect::<Vec<_>>();

    let branch_matches = owner_branches
        .iter()
        .any(|left| target_branches.iter().any(|right| left == right));
    let worktree_matches = owner_worktrees
        .iter()
        .any(|left| target_worktrees.iter().any(|right| left == right));
    if branch_matches || worktree_matches {
        return false;
    }
    let branch_conflicts = !owner_branches.is_empty() && !target_branches.is_empty();
    let worktree_conflicts = !owner_worktrees.is_empty() && !target_worktrees.is_empty();
    branch_conflicts || worktree_conflicts
}

fn normalized_work_branch(branch: Option<&str>) -> Option<String> {
    let value = branch?.trim();
    if value.is_empty() {
        return None;
    }
    Some(value.strip_prefix("origin/").unwrap_or(value).to_string())
}

const DERIVED_PROGRESS_SUMMARY_MAX_ITEMS: usize = 6;
const DERIVED_PROGRESS_SUMMARY_EVENT_CHAR_LIMIT: usize = 600;

fn refresh_work_item_progress_summary(item: &mut WorkItem) {
    item.progress_summary = latest_event_progress_summary(item)
        .or_else(|| synthesize_progress_summary_from_events(&item.events, &item.title));
}

fn backfill_work_item_progress_summary(item: &mut WorkItem) {
    item.progress_summary = latest_event_progress_summary(item)
        .or_else(|| non_empty_clone(item.progress_summary.as_deref()))
        .or_else(|| synthesize_progress_summary_from_events(&item.events, &item.title));
}

fn latest_event_progress_summary(item: &WorkItem) -> Option<String> {
    item.events
        .iter()
        .rev()
        .find_map(|event| non_empty_clone(event.progress_summary.as_deref()))
}

fn synthesize_progress_summary_from_events(
    events: &[WorkEvent],
    item_title: &str,
) -> Option<String> {
    let mut candidates = Vec::new();
    for event in events {
        let Some(candidate) = legacy_progress_summary_candidate(event, item_title) else {
            continue;
        };
        if candidates.last() != Some(&candidate) {
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let omitted = candidates
        .len()
        .saturating_sub(DERIVED_PROGRESS_SUMMARY_MAX_ITEMS);
    let selected = candidates
        .iter()
        .skip(omitted)
        .map(|value| format!("- {value}"));
    let mut lines = Vec::new();
    if omitted > 0 {
        lines.push(format!("- ... {omitted} earlier updates omitted"));
    }
    lines.extend(selected);
    Some(lines.join("\n"))
}

fn legacy_progress_summary_candidate(event: &WorkEvent, item_title: &str) -> Option<String> {
    for value in [
        event.summary.as_deref(),
        event.intent.as_deref(),
        event.next_action.as_deref(),
        event.title.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let Some(normalized) = normalize_legacy_progress_summary_text(value) else {
            continue;
        };
        if normalized == item_title {
            continue;
        }
        return Some(normalized);
    }
    None
}

fn normalize_legacy_progress_summary_text(value: &str) -> Option<String> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.trim();
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(
        collapsed,
        DERIVED_PROGRESS_SUMMARY_EVENT_CHAR_LIMIT,
    ))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == limit {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}

pub(super) fn non_empty_clone(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// SPEC-2359 US-66 (T-529): legacy adapter aliases for the renamed Work
/// entity family. In-repo call sites are fully migrated; these exist only
/// so external consumers keep compiling. New code must use the Work names.
pub type WorkspaceWorkItem = WorkItem;
pub type WorkspaceWorkEvent = WorkEvent;
pub type WorkspaceWorkEventKind = WorkEventKind;
pub type WorkspaceWorkItemsProjection = WorkItemsProjection;
pub type WorkspaceWorkAgentRef = WorkAgentRef;
pub type WorkspaceWorkItemsCache = WorkItemsCache;
pub type WorkspaceWorkItemsRebuildOutcome = WorkItemsRebuildOutcome;

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn workspace_work_events_build_hot_projection_with_lifecycle_refs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let work_items_path = temp.path().join("workspace/work_items.json");
        let events_path = temp.path().join("workspace/work_events.jsonl");
        let started_at = Utc.with_ymd_and_hms(2026, 5, 11, 1, 0, 0).unwrap();
        let done_at = Utc.with_ymd_and_hms(2026, 5, 11, 1, 30, 0).unwrap();

        let mut start = WorkEvent::new(
            WorkEventKind::Start,
            "workitem-workspace-history",
            started_at,
        );
        start.title = Some("Workspace WorkItem history".to_string());
        start.intent = Some("Group duplicate Workspace work under one WorkItem".to_string());
        start.summary = Some("Start the WorkItem lifecycle implementation.".to_string());
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.owner = Some("SPEC-2359".to_string());
        start.agent_session_id = Some("session-1".to_string());
        start.agent_id = Some("codex".to_string());
        start.display_name = Some("Codex".to_string());
        start.board_entry_id = Some("board-claim-1".to_string());
        start.execution_container = Some(WorkspaceExecutionContainerRef {
            branch: Some("work/20260510-2353".to_string()),
            worktree_path: Some(PathBuf::from("/repo/work/20260510-2353")),
            pr_number: Some(2638),
            pr_url: Some("https://github.com/akiojin/gwt/pull/2638".to_string()),
            pr_state: Some("open".to_string()),
        });
        record_workspace_work_event_paths(&work_items_path, &events_path, start)
            .expect("record start event");

        let mut done = WorkEvent::new(WorkEventKind::Done, "workitem-workspace-history", done_at);
        done.summary = Some("WorkItem lifecycle history is implemented.".to_string());
        done.status_category = Some(WorkspaceStatusCategory::Done);
        done.agent_session_id = Some("session-1".to_string());
        done.board_entry_id = Some("board-done-1".to_string());
        record_workspace_work_event_paths(&work_items_path, &events_path, done)
            .expect("record done event");

        let projection = load_workspace_work_items_from_path(&work_items_path)
            .expect("load work items")
            .expect("work items");
        assert_eq!(projection.work_items.len(), 1);
        let item = &projection.work_items[0];
        assert_eq!(item.id, "workitem-workspace-history");
        assert_eq!(item.title, "Work WorkItem history");
        assert_eq!(
            item.intent.as_deref(),
            Some("Group duplicate Workspace work under one WorkItem")
        );
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(item.owner.as_deref(), Some("SPEC-2359"));
        assert_eq!(item.completed_at, Some(done_at));
        assert_eq!(
            item.board_refs,
            vec!["board-claim-1".to_string(), "board-done-1".to_string()]
        );
        assert_eq!(item.agents.len(), 1);
        assert_eq!(item.agents[0].session_id, "session-1");
        assert_eq!(item.execution_containers.len(), 1);
        assert_eq!(
            item.execution_containers[0].branch.as_deref(),
            Some("work/20260510-2353")
        );
        assert_eq!(item.events.len(), 2);
        assert_eq!(item.events[0].kind, WorkEventKind::Start);
        assert_eq!(item.events[1].kind, WorkEventKind::Done);

        let event_lines = std::fs::read_to_string(&events_path).expect("event log");
        assert_eq!(event_lines.lines().count(), 2);
    }

    #[test]
    fn apply_event_synthesizes_progress_summary_from_legacy_events() {
        let work_item_id = "test-item-legacy-progress";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 16, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 16, 11, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 6, 16, 12, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);

        let mut start = WorkEvent::new(WorkEventKind::Start, work_item_id, t1);
        start.title = Some("Project Tabs UX".to_string());
        start.intent = Some("Compare browser-tab and built-in tab switching UX.".to_string());
        projection.apply_event(start);

        let mut implemented = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
        implemented.intent = Some(
            "Implemented project switcher, always-confirm close, and quiet Agent completion notifications."
                .to_string(),
        );
        projection.apply_event(implemented);

        let mut verified = WorkEvent::new(WorkEventKind::Update, work_item_id, t3);
        verified.summary =
            Some("User confirmed the Project Tabs UX. Committed and pushed 275930e5a.".to_string());
        projection.apply_event(verified);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("work item");
        let progress = item
            .progress_summary
            .as_deref()
            .expect("legacy events should synthesize progress_summary");
        assert!(progress.contains("Compare browser-tab"), "{progress}");
        assert!(
            progress.contains("Implemented project switcher"),
            "{progress}"
        );
        assert!(
            progress.contains("User confirmed the Project Tabs UX"),
            "{progress}"
        );
    }

    #[test]
    fn apply_event_preserves_done_against_subsequent_heartbeat() {
        let work_item_id = "test-item-preserve";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 14, 11, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);

        let mut done_event = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
        done_event.status_category = Some(WorkspaceStatusCategory::Done);
        done_event.title = Some("Test work item".to_string());
        projection.apply_event(done_event);

        let item_after_done = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("done event must create work item");
        assert_eq!(
            item_after_done.status_category,
            WorkspaceStatusCategory::Done
        );
        assert_eq!(item_after_done.completed_at, Some(t1));

        let update_event = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
        assert!(
            update_event.status_category.is_none(),
            "heartbeat update event has no explicit status_category"
        );
        projection.apply_event(update_event);

        let item_after_update = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item still exists");
        assert_eq!(
            item_after_update.status_category,
            WorkspaceStatusCategory::Done,
            "SPEC-2359 US-37: Done is a terminal state; heartbeat update with status_category=None must not regress it"
        );
        assert_eq!(
            item_after_update.completed_at,
            Some(t1),
            "initial Done timestamp must be preserved across subsequent update events"
        );
    }

    #[test]
    fn apply_event_discard_marks_work_terminal_discarded() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): a Discard event makes the Work
        // terminal-discarded (not Done) and removes it from the incomplete set.
        let work_item_id = "test-item-discard";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);
        let mut start = WorkEvent::new(WorkEventKind::Start, work_item_id, t1);
        start.status_category = Some(WorkspaceStatusCategory::Active);
        start.title = Some("Discardable work".to_string());
        projection.apply_event(start);

        let discard = WorkEvent::new(WorkEventKind::Discard, work_item_id, t2);
        projection.apply_event(discard);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists");
        assert!(item.discarded, "Discard event must mark the Work discarded");
        assert!(item.is_terminal(), "discarded Work is terminal");
        assert!(!item.is_incomplete(), "discarded Work is not incomplete");
        assert_ne!(
            item.status_category,
            WorkspaceStatusCategory::Done,
            "Discard is distinct from Done"
        );
        assert_eq!(
            item.completed_at, None,
            "discarded Work is not completed, so completed_at stays None"
        );
    }

    #[test]
    fn apply_event_preserves_discarded_against_subsequent_heartbeat() {
        // SPEC-2359 Phase W-12 Slice 4 (FR-352): Discarded is terminal — a later
        // heartbeat update (no explicit status_category) must not un-discard.
        let work_item_id = "test-item-discard-preserve";
        let t1 = Utc.with_ymd_and_hms(2026, 6, 4, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 4, 11, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);
        let discard = WorkEvent::new(WorkEventKind::Discard, work_item_id, t1);
        projection.apply_event(discard);
        let update = WorkEvent::new(WorkEventKind::Update, work_item_id, t2);
        projection.apply_event(update);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists");
        assert!(
            item.discarded,
            "heartbeat update must not clear the discarded terminal flag"
        );
        assert!(!item.is_incomplete());
    }

    #[test]
    fn apply_event_idempotent_done_keeps_first_timestamp() {
        let work_item_id = "test-item-idempotent";
        let t1 = Utc.with_ymd_and_hms(2026, 5, 14, 10, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap();

        let mut projection = WorkItemsProjection::empty(t1);

        let mut first_done = WorkEvent::new(WorkEventKind::Done, work_item_id, t1);
        first_done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(first_done);

        let mut second_done = WorkEvent::new(WorkEventKind::Done, work_item_id, t2);
        second_done.status_category = Some(WorkspaceStatusCategory::Done);
        projection.apply_event(second_done);

        let item = projection
            .work_items
            .iter()
            .find(|it| it.id == work_item_id)
            .expect("item exists after idempotent done");
        assert_eq!(item.status_category, WorkspaceStatusCategory::Done);
        assert_eq!(
            item.completed_at,
            Some(t1),
            "first Done timestamp must be preserved on idempotent Done re-apply"
        );
        assert_eq!(item.updated_at, t2, "updated_at should still advance");
    }

    fn container_for_test(branch: &str, worktree: &str) -> WorkspaceExecutionContainerRef {
        WorkspaceExecutionContainerRef {
            branch: Some(branch.to_string()),
            worktree_path: Some(PathBuf::from(worktree)),
            pr_number: None,
            pr_url: None,
            pr_state: None,
        }
    }

    /// Issue #3216: reproduce the works.json corruption behind Issue #3213 —
    /// a session bound to one branch's Work must not be attached to another
    /// Work whose branch identity conflicts (FR-348: 1 session : 1 Work).
    #[test]
    fn apply_event_rejects_stray_session_attach_to_conflicting_branch_item() {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 29, 7, 45, 56).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 29, 8, 24, 29).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut owner_start = WorkEvent::new(WorkEventKind::Start, "work-work-issue-3197", t0);
        owner_start.agent_session_id = Some("session-owner".to_string());
        owner_start.execution_container = Some(container_for_test(
            "work/issue-3197",
            "/repo/work/issue-3197",
        ));
        projection.apply_event(owner_start);

        let mut other_start = WorkEvent::new(WorkEventKind::Start, "work-work-issue-3184", t0);
        other_start.agent_session_id = Some("session-other".to_string());
        other_start.execution_container = Some(container_for_test(
            "work/issue-3184",
            "/repo/work/issue-3184",
        ));
        projection.apply_event(other_start);

        // The stray event: the owner's session arrives on the OTHER branch's
        // item (mis-attributed work_item_id / session pairing).
        let mut stray = WorkEvent::new(WorkEventKind::Update, "work-work-issue-3184", t1);
        stray.agent_session_id = Some("session-owner".to_string());
        projection.apply_event(stray);

        let other = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-work-issue-3184")
            .expect("other item");
        assert!(
            !other
                .agents
                .iter()
                .any(|agent| agent.session_id == "session-owner"),
            "conflicting-branch item must not gain the stray session ref"
        );
        // The suspect event itself stays recorded for diagnosis.
        assert_eq!(other.events.len(), 2);
    }

    /// Issue #3216 contract guard: the attach guard fires only on a genuine
    /// identity conflict. Same-branch duplicates and identity-less items keep
    /// the historical attach behavior.
    #[test]
    fn apply_event_allows_session_attach_without_identity_conflict() {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 29, 7, 45, 56).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut owner_start = WorkEvent::new(WorkEventKind::Start, "work-owner", t0);
        owner_start.agent_session_id = Some("session-shared".to_string());
        owner_start.execution_container = Some(container_for_test("work/same", "/repo/work/same"));
        projection.apply_event(owner_start);

        // Same-branch duplicate item (resume / backfill shape): attach allowed.
        let mut same_branch = WorkEvent::new(WorkEventKind::Resume, "work-duplicate", t0);
        same_branch.agent_session_id = Some("session-shared".to_string());
        same_branch.execution_container = Some(container_for_test("work/same", "/repo/work/same"));
        projection.apply_event(same_branch);

        // Identity-less item (synthesized live Work without git_details):
        // attach allowed.
        let mut identity_less = WorkEvent::new(WorkEventKind::Update, "work-session-abc", t0);
        identity_less.agent_session_id = Some("session-shared".to_string());
        projection.apply_event(identity_less);

        for id in ["work-duplicate", "work-session-abc"] {
            let item = projection
                .work_items
                .iter()
                .find(|item| item.id == id)
                .expect("item");
            assert!(
                item.agents
                    .iter()
                    .any(|agent| agent.session_id == "session-shared"),
                "{id} must keep the historical session attach behavior"
            );
        }
    }

    /// Issue #3216: attach provenance survives journal compaction by living on
    /// the WorkAgentRef itself.
    #[test]
    fn apply_event_records_attach_provenance_kind() {
        let t0 = Utc.with_ymd_and_hms(2026, 6, 29, 7, 45, 56).unwrap();
        let mut projection = WorkItemsProjection::empty(t0);

        let mut start = WorkEvent::new(WorkEventKind::Start, "work-provenance", t0);
        start.agent_session_id = Some("session-1".to_string());
        projection.apply_event(start);

        let item = projection
            .work_items
            .iter()
            .find(|item| item.id == "work-provenance")
            .expect("item");
        assert_eq!(item.agents[0].attached_by, Some(WorkEventKind::Start));
    }

    /// Issue #3216: existing works.json (no `attached_by`) must keep
    /// deserializing.
    #[test]
    fn work_agent_ref_deserializes_without_attached_by() {
        let json = r#"{"session_id":"s","updated_at":"2026-06-29T07:45:56Z"}"#;
        let agent: WorkAgentRef = serde_json::from_str(json).expect("deserialize legacy ref");
        assert_eq!(agent.attached_by, None);
        assert_eq!(agent.session_id, "s");
    }
}

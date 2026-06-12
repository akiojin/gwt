export function createWorkspaceKanbanSurface({
  activeWorkspace,
  agentStatusLabel,
  appendMeta,
  createWorkspacePrMeta,
  createNode,
  getActiveWorkProjection,
  openWorkspaceCleanup,
  send,
  windowMap,
  workspaceWindowById,
  openWorkspaceResumePicker,
  getResumeBounds,
  branchesSurface,
  launchPending,
}) {
  const workspaceStateMap = new Map();

  function ensureState(windowId) {
    if (!workspaceStateMap.has(windowId)) {
      workspaceStateMap.set(windowId, { selectedId: null });
    }
    return workspaceStateMap.get(windowId);
  }

  function compactText(value, fallback = "") {
    const text = String(value || "").replace(/\s+/g, " ").trim();
    return text || fallback;
  }

  function statusLabel(value) {
    const raw = String(value || "idle").toLowerCase();
    if (raw === "blocked") return "Blocked";
    if (raw === "active" || raw === "running") return "Active";
    if (raw === "done" || raw === "completed" || raw === "closed") return "Done";
    if (raw === "archived") return "Archived";
    return "Idle";
  }

  function formatLifecycleStageLabel(stage) {
    const normalized = String(stage || "").toLowerCase();
    switch (normalized) {
      case "planning":
        return "Planning";
      case "active":
        return "Active";
      case "in_review":
        return "In Review";
      case "done":
        return "Done";
      case "archived":
        return "Archived";
      default:
        return normalized
          ? normalized
              .split("_")
              .map((part) =>
                part.length > 0 ? part[0].toUpperCase() + part.slice(1) : part,
              )
              .join(" ")
          : "";
    }
  }

  // SPEC-2359 Phase W-12 (FR-349/FR-351): human-readable label for the
  // agent-session Work lifecycle state (active / paused / done / discarded)
  // rendered as a badge on each Work card.
  function formatLifecycleStateLabel(state) {
    switch (String(state || "active").toLowerCase()) {
      case "active":
        return "Active";
      case "paused":
        return "Paused";
      case "done":
        return "Done";
      case "discarded":
        return "Discarded";
      default:
        return "Active";
    }
  }

  function compactPath(value) {
    const text = compactText(value);
    if (!text) return "";
    const parts = text.split(/[\\/]+/).filter(Boolean);
    if (parts.length <= 3) return text;
    return `${parts[parts.length - 3]}/${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  }

  function workspaceTitle(entry) {
    return compactText(
      entry?.title || entry?.intent || entry?.summary || entry?.owner,
      "Work",
    );
  }

  function eventTitle(event) {
    return compactText(
      event?.title || event?.summary || event?.kind || event?.board_entry_id,
      "Work event",
    );
  }

  function containersFor(entry) {
    if (Array.isArray(entry?.execution_containers)) {
      return entry.execution_containers;
    }
    if (entry?.branch || entry?.worktree_path || entry?.pr_number) {
      return [entry];
    }
    return [];
  }

  function normalizeWorkspaceItem(item, fallback = {}) {
    const containers = containersFor(item);
    const primaryContainer = containers[0] || {};
    const id =
      item?.id ||
      item?.workspace_id ||
      fallback.id ||
      `workspace-${workspaceTitle(item).toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;
    return {
      id,
      title: workspaceTitle(item),
      intent: compactText(item?.intent),
      summary: compactText(item?.summary || item?.status_text || item?.intent),
      owner: compactText(item?.owner || fallback.owner),
      status_category: item?.status_category || fallback.status_category || "idle",
      status_text: compactText(item?.status_text || item?.summary),
      next_action: compactText(item?.next_action),
      blocked_reason: compactText(item?.blocked_reason),
      lifecycle_stage: item?.lifecycle_stage || fallback.lifecycle_stage || "",
      // SPEC-2359 Phase W-12 (FR-349/FR-351): agent-session Work lifecycle
      // state (active / paused / done / discarded). Distinct from the U-6
      // status-derived `lifecycle_stage`. Defaults to "active" so legacy
      // projections without the field keep rendering an Active Work badge.
      lifecycle_state:
        item?.lifecycle_state || fallback.lifecycle_state || "active",
      closed_at: item?.closed_at || fallback.closed_at || null,
      branch: compactText(item?.branch || primaryContainer.branch || fallback.branch),
      worktree_path: compactText(
        item?.worktree_path || primaryContainer.worktree_path || fallback.worktree_path,
      ),
      pr_number: item?.pr_number || primaryContainer.pr_number || fallback.pr_number || null,
      pr_url: item?.pr_url || primaryContainer.pr_url || fallback.pr_url || "",
      pr_state: item?.pr_state || primaryContainer.pr_state || fallback.pr_state || "",
      board_refs: Array.isArray(item?.board_refs)
        ? item.board_refs
        : Array.isArray(fallback.board_refs)
          ? fallback.board_refs
          : [],
      agents: Array.isArray(item?.agents)
        ? item.agents
        : Array.isArray(fallback.agents)
          ? fallback.agents
          : [],
      // SPEC-2359 W16-2 (FR-389): Workspace grouping key (backend merges
      // same-key rows before the wire; carried for tooling/tests).
      workspace_key: item?.workspace_key || null,
      // SPEC-2359 W16-3 (FR-390): branch known only from fetched refs — no
      // local worktree. Display-only; Launch materializes one on demand.
      remote_only: Boolean(item?.remote_only),
      // SPEC-2359 W16-4 (FR-391): derived Done classification (merged ∧ no
      // update after the merge). Display-only; clears on new activity.
      done_equivalent: Boolean(item?.done_equivalent),
      // SPEC-2359 W-15 (FR-386): merged into a base on origin (or PR merged)
      // — the "safe to delete" signal. Display-only.
      merged_into_base: Boolean(item?.merged_into_base),
      // SPEC-2359 W-16 (FR-402): uncapped agent/session count for the
      // "+N more sessions" label; 0 = not computed (legacy payloads).
      session_agent_total:
        Number(item?.session_agent_total) || Number(fallback.session_agent_total) || 0,
      events: Array.isArray(item?.events) ? item.events : [],
      cleanup_candidate: item?.cleanup_candidate || fallback.cleanup_candidate || null,
      updated_at: compactText(item?.updated_at || fallback.updated_at),
    };
  }

  function workspacesFromProjection(projection) {
    if (!projection) return [];
    const activeWorks = Array.isArray(projection.active_works)
      ? projection.active_works
      : [];
    if (activeWorks.length > 0) {
      return activeWorks.map((item) => normalizeWorkspaceItem(item, projection));
    }
    const sourceItems = Array.isArray(projection.works)
      ? projection.works
      : Array.isArray(projection.workspaces)
        ? projection.workspaces
        : Array.isArray(projection.work_items)
          ? projection.work_items
          : [];
    if (sourceItems.length > 0) {
      return sourceItems.map((item) => normalizeWorkspaceItem(item, projection));
    }

    const current = normalizeWorkspaceItem(projection, {
      id: projection.id || "__current_work__",
      title: projection.title || activeWorkspace()?.title,
    });
    const journalEntries = Array.isArray(projection.journal_entries)
      ? projection.journal_entries
      : [];
    return [
      current,
      ...journalEntries.map((entry) =>
        normalizeWorkspaceItem(entry, {
          owner: projection.owner,
          status_category: entry.status_category || "idle",
        }),
      ),
    ];
  }

  function unassignedAgentsFromProjection(projection) {
    return Array.isArray(projection?.unassigned_agents)
      ? projection.unassigned_agents
      : [];
  }

  function selectedWorkspace(state, workspaces) {
    if (workspaces.length === 0) return null;
    const selected = workspaces.find((item) => item.id === state.selectedId);
    if (selected) return selected;
    state.selectedId = workspaces[0].id;
    return workspaces[0];
  }

  function appendMetaText(container, value) {
    if (!value) return;
    appendMeta(container, value);
  }

  // Design pass (2026-06-11): branch names render as a dimmed namespace
  // prefix + strong leaf ("work/" + "20260610-0120-4") so long branch lists
  // scan by leaf. textContent stays the verbatim branch.
  function appendBranchLabel(container, value) {
    const branch = String(value || "");
    const cut = branch.lastIndexOf("/");
    if (cut > 0 && cut < branch.length - 1) {
      container.appendChild(
        createNode("span", "workspace-branch-prefix", branch.slice(0, cut + 1)),
      );
      container.appendChild(
        createNode("span", "workspace-branch-leaf", branch.slice(cut + 1)),
      );
    } else {
      container.appendChild(createNode("span", "workspace-branch-leaf", branch));
    }
  }

  // Map an agent name onto the established [data-agent-color] identity system
  // (SPEC-2133) so Work groups inherit --current-agent from existing CSS.
  function agentColorKeyword(work) {
    const name = String(
      (work && (work.display_name || work.agent_id)) || "",
    ).toLowerCase();
    if (name.includes("claude")) return "yellow";
    if (name.includes("codex")) return "cyan";
    if (name.includes("gemini")) return "magenta";
    if (name.includes("opencode")) return "green";
    if (name.includes("copilot")) return "blue";
    return "gray";
  }

  function renderWorkspaceRow(windowId, state, item) {
    const row = createNode("button", "workspace-overview-row");
    row.type = "button";
    row.dataset.workspaceId = item.id;
    row.dataset.status = String(item.status_category || "idle").toLowerCase();
    row.setAttribute("aria-selected", state.selectedId === item.id ? "true" : "false");

    const status = createNode("span", "workspace-overview-status", statusLabel(item.status_category));
    const copy = createNode("span", "workspace-overview-row-copy");
    const titleRow = createNode("span", "workspace-overview-row-title-row");
    // SPEC-2359 W-15 (user design decision 2026-06-10): the Workspace list is
    // a branch list — the row is titled by the branch (the place); the
    // record's own title (work summary) moves to the meta line below.
    const rowTitle = item.branch || item.title;
    const titleNode = createNode("span", "workspace-overview-row-title");
    appendBranchLabel(titleNode, rowTitle);
    titleRow.appendChild(titleNode);
    // SPEC-2359 Phase W-12 (FR-351): each Work card surfaces its agent-session
    // lifecycle state (Active / Paused / Done / Discarded) as a dedicated badge
    // so the Work surface is the single home for Work lifecycle.
    // SPEC-2359 W16-4 (FR-391): a merged-and-stale Workspace classifies as
    // derived Done — the badge reads Done (data-derived marks it apart from
    // an explicit close) and the row never presents as Active/Paused.
    const doneEquivalent = Boolean(item.done_equivalent);
    const lifecycleBadge = createNode(
      "span",
      "workspace-overview-lifecycle",
      doneEquivalent ? "Done" : formatLifecycleStateLabel(item.lifecycle_state),
    );
    lifecycleBadge.dataset.lifecycle = doneEquivalent
      ? "done"
      : String(item.lifecycle_state || "active").toLowerCase();
    if (doneEquivalent) {
      lifecycleBadge.dataset.derived = "true";
      lifecycleBadge.title = "Merged with no updates since — derived Done (no close recorded)";
    }
    titleRow.appendChild(lifecycleBadge);
    if (item.merged_into_base) {
      // SPEC-2359 W-15 (FR-386): branch merged into a base — safe to delete.
      titleRow.appendChild(createNode("span", "workspace-overview-merged", "Merged"));
    }
    if (item.remote_only) {
      // SPEC-2359 W16-3 (FR-390): branch exists only as a fetched remote
      // ref; Launch Agent creates the worktree on demand.
      titleRow.appendChild(createNode("span", "workspace-overview-remote", "Remote"));
    }
    const rowRelative = formatRelativeTime(item.updated_at);
    if (rowRelative) {
      const time = createNode("span", "workspace-overview-row-time", rowRelative);
      time.title = String(item.updated_at);
      titleRow.appendChild(time);
    }
    copy.appendChild(titleRow);
    const meta = createNode("span", "workspace-overview-row-meta");
    appendMetaText(meta, item.owner);
    // The record title adds information only when it differs from the branch
    // shown as the row title (backfilled rows are titled by branch already).
    if (item.title && item.title !== rowTitle) {
      appendMetaText(meta, item.title);
    }
    const prMeta = createWorkspacePrMeta?.(item);
    if (prMeta) {
      meta.appendChild(prMeta);
    }
    copy.appendChild(meta);

    row.appendChild(status);
    row.appendChild(copy);
    row.addEventListener("click", () => {
      state.selectedId = item.id;
      renderWorkspaceOverviewWindow(windowId, true);
      // The re-render replaced this row, dropping focus to <body> — restore
      // it onto the freshly rendered selected row so ArrowUp / ArrowDown
      // keyboard navigation keeps working after a mouse selection.
      focusSelectedWorkspaceRow(windowId);
    });
    return row;
  }

  function focusSelectedWorkspaceRow(windowId) {
    const host = windowMap.get(windowId);
    const row = host?.querySelector?.('.workspace-overview-row[aria-selected="true"]');
    row?.focus?.();
    row?.scrollIntoView?.({ block: "nearest" });
  }

  function renderUnassignedQueue(container, agents) {
    const section = createNode("section", "workspace-agent-queue");
    section.dataset.role = "unassigned-agents";
    section.appendChild(
      createNode("div", "workspace-overview-section-label", "Unassigned Agents"),
    );
    if (agents.length === 0) {
      section.appendChild(
        createNode("div", "workspace-overview-empty", "No Unassigned Agents"),
      );
      container.appendChild(section);
      return;
    }
    const list = createNode("div", "workspace-overview-agent-list");
    for (const agent of agents) {
      const row = createNode("article", "workspace-overview-agent-row");
      row.dataset.status = String(agent.status_category || "idle").toLowerCase();
      row.appendChild(
        createNode(
          "div",
          "workspace-overview-agent-name",
          agent.display_name || agent.agent_id || "Agent",
        ),
      );
      const meta = createNode("div", "workspace-overview-agent-meta");
      appendMetaText(meta, "No Workspace selected");
      appendMetaText(meta, agentStatusLabel?.(agent.status_category));
      appendMetaText(meta, agent.branch);
      row.appendChild(meta);
      list.appendChild(row);
    }
    section.appendChild(list);
    container.appendChild(section);
  }

  function detailSection(title, bodyBuilder) {
    const section = createNode("section", "workspace-detail-section");
    section.appendChild(createNode("h3", "workspace-detail-section-title", title));
    const body = createNode("div", "workspace-detail-section-body");
    bodyBuilder(body);
    section.appendChild(body);
    return section;
  }

  function appendTextBlock(container, text, className = "workspace-detail-text") {
    if (!text) return;
    container.appendChild(createNode("p", className, text));
  }

  function appendDefinitionList(container, rows) {
    const list = createNode("dl", "workspace-detail-meta-grid");
    for (const [label, value] of rows) {
      if (!value) continue;
      list.appendChild(createNode("dt", "", label));
      list.appendChild(createNode("dd", "", value));
    }
    if (list.childNodes.length > 0) {
      container.appendChild(list);
    }
  }

  // SPEC-2359 Workspace → Work → Session: the detail is Session-centric. Each
  // `work` (a launch, carried in `workspace.agents`) holds its conversation
  // Sessions in `work.sessions`. Sessions render as the primary rows; a Work
  // heading only appears when the Workspace has more than one Work. Persistent
  // Works always render (no live-only filtering), so Paused Workspaces are not
  // mislabelled "No assigned agents".
  // SPEC-2359 W-15 (FR-379 follow-up): Launch opens the launch wizard
  // prefilled with the Workspace's branch; the new launch becomes a new Work
  // joining this Workspace. Lives in the detail header actions as the primary
  // action — one fixed home, never after the variable-length Work list
  // (placement feedback, user verification 2026-06-11).
  function renderLaunchWorkspaceButton(workspace, windowId) {
    const branch = workspace && workspace.branch ? String(workspace.branch) : "";
    if (!branch) return null;
    // Same entry as the Branches surface "Launch Agent": opens the launch
    // wizard for this Workspace's existing branch (user wording 2026-06-11).
    const launch = createNode("button", "wizard-button primary", "Launch Agent");
    launch.type = "button";
    launch.dataset.action = "launch-workspace";
    launch.addEventListener("click", () => {
      send({
        kind: "open_launch_wizard",
        id: windowId,
        branch_name: branch,
      });
    });
    return launch;
  }

  function appendWorks(container, works, workspace) {
    const list = Array.isArray(works) ? works : [];
    if (list.length === 0) {
      // Launching lives in the detail header (one canonical home), so the
      // empty state is a plain placeholder.
      container.appendChild(
        createNode("div", "workspace-overview-empty", "No Work yet"),
      );
      return;
    }
    const wrap = createNode("div", "workspace-detail-work-list");
    for (const work of list) {
      const group = createNode("div", "workspace-detail-work-group");
      group.dataset.agentColor = agentColorKeyword(work);
      // Each Work is one Agent (a launch). The Agent header names the agent
      // (tool); the Work's Sessions (its conversation history) are listed under
      // it as sub-rows, and Resume lives on each Session row (a single list
      // element) so any conversation can be resumed directly. The header is
      // always shown so two Sessions of one Work never look like two Agents.
      const head = createNode("div", "workspace-detail-work-head");
      head.appendChild(
        createNode(
          "div",
          "workspace-detail-work-heading",
          work.display_name || work.agent_id || "Agent",
        ),
      );
      group.appendChild(head);

      const sessions = Array.isArray(work.sessions) ? work.sessions : [];
      if (sessions.length === 0) {
        // No conversation recorded yet — still expose a Resume control on the
        // placeholder row so a session-less Work stays launchable.
        const empty = createNode(
          "div",
          "workspace-overview-empty workspace-detail-session-empty",
          "No session yet",
        );
        const resumeBtn = renderWorkResumeButton(work);
        if (resumeBtn) {
          empty.appendChild(resumeBtn);
        }
        group.appendChild(empty);
      } else {
        // User decision 2026-06-12: multiple Session rows per agent read as
        // noise — render only the latest conversation (the active one, or the
        // newest by order; the backend sorts oldest-first).
        const latest =
          sessions.find((session) => session && session.is_active) ||
          sessions[sessions.length - 1];
        group.appendChild(renderSessionRow(work, latest));
        // E1: when the visible Session is history-only (not resumable) on a
        // non-running Work, no Resume appears — offer a "Start Fresh" control
        // so the Work stays launchable. Distinct label so the user knows it
        // starts a new conversation, not a resumed one.
        const startFresh = renderStartFreshButton(work, [latest]);
        if (startFresh) {
          group.appendChild(startFresh);
        }
      }
      wrap.appendChild(group);
    }
    container.appendChild(wrap);
    // SPEC-2359 W-16 (FR-402): the agents list is capped on the wire; surface
    // how many more ledger sessions exist beyond the rendered ones.
    // `session_agent_total === 0` means "not computed" (legacy payload).
    const total = Number(workspace && workspace.session_agent_total) || 0;
    if (total > list.length) {
      container.appendChild(
        createNode(
          "div",
          "workspace-detail-more-sessions workspace-overview-empty",
          `+${total - list.length} more sessions`,
        ),
      );
    }
  }

  function renderWorkResumeButton(work) {
    // A live (running) Work has nothing to resume; only Paused / historical
    // Works get a Resume control. Works without a status (history view) are
    // treated as resumable.
    const status = String(work && work.status_category ? work.status_category : "").toLowerCase();
    if (status === "active" || status === "running") {
      return null;
    }
    if (!work || !work.session_id) {
      return null;
    }
    const button = createNode("button", "wizard-button is-compact", "Resume");
    button.type = "button";
    button.dataset.action = "resume-work";
    button.dataset.sessionId = work.session_id;
    if (isWorkResumePending(work.session_id)) {
      markResumeButtonPending(button);
    }
    button.addEventListener("click", () => resumeWork(work));
    return button;
  }

  // SPEC-2359 W-17 (FR-398): pending key shared with the Resume picker and
  // the dispatcher's ack/error settle path.
  function workPendingKey(sessionId) {
    return `session:${sessionId}`;
  }

  function isWorkResumePending(sessionId) {
    return Boolean(
      sessionId
        && launchPending
        && launchPending.isPending(workPendingKey(sessionId)),
    );
  }

  function markResumeButtonPending(button) {
    button.disabled = true;
    button.textContent = "Resuming...";
    button.classList.add("is-pending");
  }

  function resumeWork(work) {
    const sessionId = work && work.session_id;
    if (!sessionId) {
      return;
    }
    const bounds = typeof getResumeBounds === "function" ? getResumeBounds() : null;
    if (!bounds) {
      return;
    }
    if (
      launchPending
      && !launchPending.begin(workPendingKey(sessionId), "Resume")
    ) {
      return;
    }
    // resume_workspace_agent resumes by the gwt session id (the Work / launch),
    // which is exactly work.session_id. Without an agent_session_id the Work's
    // latest conversation (or a fresh start) is resumed.
    send({ kind: "resume_workspace_agent", session_id: sessionId, bounds });
    renderWindows();
  }

  function renderSessionResumeButton(work, session) {
    // Resume sits on each Session row so a single conversation can be resumed
    // directly. A live (running) Work has nothing to resume; only Paused /
    // historical Works (or history view, which has no status) get the control.
    const status = String(work && work.status_category ? work.status_category : "").toLowerCase();
    if (status === "active" || status === "running") {
      return null;
    }
    if (!work || !work.session_id) {
      return null;
    }
    // A conversation gwt cannot hand the agent CLI as a `--resume` target is
    // history-only; render no Resume so a button that would silently fail is
    // never shown. (backend sets resumable; absent === assume resumable.)
    if (session && session.resumable === false) {
      return null;
    }
    const button = createNode("button", "wizard-button is-compact", "Resume");
    button.type = "button";
    button.dataset.action = "resume-session";
    button.dataset.sessionId = work.session_id;
    const agentSessionId = session && session.agent_session_id;
    if (agentSessionId) {
      button.dataset.agentSessionId = agentSessionId;
      button.setAttribute("aria-label", `Resume conversation ${agentSessionId}`);
    } else {
      button.setAttribute("aria-label", "Resume this conversation");
    }
    if (isWorkResumePending(work.session_id)) {
      markResumeButtonPending(button);
    }
    button.addEventListener("click", () => resumeSession(work, session));
    return button;
  }

  function renderStartFreshButton(work, sessions) {
    const status = String(work && work.status_category ? work.status_category : "").toLowerCase();
    if (status === "active" || status === "running") {
      return null;
    }
    if (!work || !work.session_id) {
      return null;
    }
    const list = Array.isArray(sessions) ? sessions : [];
    // If any Session is resumable, a per-Session Resume is already shown — no
    // need for a Work-level fallback.
    const anyResumable = list.some(
      (entry) => entry && entry.resumable !== false && entry.agent_session_id,
    );
    if (anyResumable) {
      return null;
    }
    const wrap = createNode("div", "workspace-detail-session-fresh");
    const button = createNode("button", "wizard-button is-compact", "Start Fresh");
    button.type = "button";
    button.dataset.action = "resume-work";
    button.dataset.sessionId = work.session_id;
    button.setAttribute("aria-label", "Start a fresh conversation for this Work");
    if (isWorkResumePending(work.session_id)) {
      markResumeButtonPending(button);
    }
    button.addEventListener("click", () => resumeWork(work));
    wrap.appendChild(button);
    return wrap;
  }

  function resumeSession(work, session) {
    const sessionId = work && work.session_id;
    if (!sessionId) {
      return;
    }
    const bounds = typeof getResumeBounds === "function" ? getResumeBounds() : null;
    if (!bounds) {
      return;
    }
    // resume_workspace_agent loads the launch config from the gwt session id
    // (the Work) and resumes the specific conversation named by
    // agent_session_id (this Session row).
    if (
      launchPending
      && !launchPending.begin(workPendingKey(sessionId), "Resume")
    ) {
      return;
    }
    const agentSessionId = session && session.agent_session_id ? session.agent_session_id : null;
    send({
      kind: "resume_workspace_agent",
      session_id: sessionId,
      agent_session_id: agentSessionId,
      bounds,
    });
    renderWindows();
  }

  function shortSessionId(value) {
    // 12 chars keeps distinct conversation UUIDs visually distinguishable
    // (8 chars collapsed near-identical prefixes); the full id is on hover.
    const text = String(value || "");
    return text.length > 12 ? text.slice(0, 12) : text;
  }

  // SPEC-2359: human-friendly relative time for Session rows. Mirrors
  // workspace-resume-picker-modal.js's formatter; kept local to avoid wiring a
  // new shared module. TODO: consolidate into one shared time util.
  function formatRelativeTime(iso) {
    if (typeof iso !== "string" || !iso) {
      return "";
    }
    const ms = Date.parse(iso);
    if (Number.isNaN(ms)) {
      return iso;
    }
    const diff = Date.now() - ms;
    if (diff < 0) {
      // Future timestamp (clock skew): fall back to an absolute rendering.
      return new Date(ms).toLocaleString();
    }
    if (diff < 1000) {
      return "just now";
    }
    const sec = Math.floor(diff / 1000);
    if (sec < 60) {
      return `${sec}s ago`;
    }
    const min = Math.floor(sec / 60);
    if (min < 60) {
      return `${min}m ago`;
    }
    const hr = Math.floor(min / 60);
    if (hr < 24) {
      return `${hr}h ago`;
    }
    const days = Math.floor(hr / 24);
    if (days < 7) {
      return `${days}d ago`;
    }
    return new Date(ms).toLocaleDateString();
  }

  function renderSessionRow(work, session) {
    const row = createNode("article", "workspace-detail-session");
    const active = Boolean(session && session.is_active);
    row.dataset.active = active ? "true" : "false";
    // The row is a Session (a conversation under the Agent), not the Agent
    // itself — the Agent (tool) is named once on the group header above. The
    // label + meta sit on the left; Resume for this conversation sits on the
    // right of the same list element.
    const main = createNode("div", "workspace-detail-session-main");

    // Name line: a clear Current/Past badge makes "latest vs past" read at a
    // glance (replacing the previous subtle "active" text); the truncated
    // conversation id carries the full id on hover.
    const nameRow = createNode("div", "workspace-detail-session-name");
    const fullId = session && session.agent_session_id ? String(session.agent_session_id) : "";
    const badge = createNode(
      "span",
      "workspace-detail-session-badge",
      active ? "Current" : "Past",
    );
    badge.dataset.sessionState = active ? "current" : "past";
    nameRow.appendChild(badge);
    const idLabel = createNode(
      "span",
      "workspace-detail-session-id",
      fullId ? `Session ${shortSessionId(fullId)}` : "Session",
    );
    if (fullId) {
      idLabel.title = fullId;
    }
    nameRow.appendChild(idLabel);
    main.appendChild(nameRow);

    const meta = createNode("div", "workspace-detail-session-meta");
    const startedAt = session ? session.started_at : work.updated_at;
    const relative = formatRelativeTime(startedAt);
    if (relative) {
      const time = createNode("span", "", relative);
      if (startedAt) {
        time.title = String(startedAt); // absolute timestamp on hover
      }
      meta.appendChild(time);
    }
    main.appendChild(meta);
    row.appendChild(main);

    const resume = renderSessionResumeButton(work, session);
    if (resume) {
      row.appendChild(resume);
    }

    // E4: expose the row's identity / state / resumability to assistive tech.
    const stateLabel = active ? "Current" : "Past";
    const resumableLabel = resume ? "resumable" : "history only";
    const ariaParts = [`Session ${fullId}`.trim(), stateLabel, resumableLabel];
    if (relative) {
      ariaParts.push(`started ${relative}`);
    }
    row.setAttribute("aria-label", ariaParts.join(", "));
    return row;
  }

  function appendEvents(container, events) {
    if (!events || events.length === 0) {
      container.appendChild(createNode("div", "workspace-overview-empty", "No lifecycle events"));
      return;
    }
    const INITIAL_LIMIT = 5;
    const list = createNode("ol", "workspace-detail-event-list");
    const visible = events.slice(0, INITIAL_LIMIT);
    const hidden = events.slice(INITIAL_LIMIT);
    for (const event of visible) {
      appendEventItem(list, event);
    }
    container.appendChild(list);
    if (hidden.length > 0) {
      const more = createNode("button", "workspace-detail-more", `${hidden.length} more events`);
      more.type = "button";
      more.addEventListener("click", () => {
        for (const event of hidden) {
          appendEventItem(list, event);
        }
        more.remove();
      });
      container.appendChild(more);
    }
  }

  function appendEventItem(list, event) {
      const item = createNode("li", "workspace-detail-event");
      const title = createNode("div", "workspace-detail-event-title", eventTitle(event));
      const meta = createNode("div", "workspace-detail-event-meta");
      appendMetaText(meta, event.kind);
      appendMetaText(meta, event.updated_at);
      appendMetaText(meta, event.board_entry_id);
      item.appendChild(title);
      if (event.summary && event.summary !== event.title) {
        appendTextBlock(item, event.summary, "workspace-detail-event-summary");
      }
      item.appendChild(meta);
      list.appendChild(item);
  }

  function resumeWorkspace(workspace) {
    const workspaceId = workspace?.id ?? null;
    if (typeof openWorkspaceResumePicker === "function") {
      openWorkspaceResumePicker(workspaceId);
    }
    send({
      kind: "list_resumable_agents",
      workspace_id: workspaceId ?? undefined,
    });
  }

  function renderWorkspaceDetail(container, workspace, windowId) {
    container.innerHTML = "";
    if (!workspace) {
      const empty = createNode("div", "workspace-overview-empty", "No Workspace selected");
      container.appendChild(empty);
      return;
    }

    const header = createNode("header", "workspace-detail-header");
    const titleWrap = createNode("div", "workspace-detail-heading");
    // SPEC-2359 W-15 (user design decision 2026-06-10): the detail heading is
    // the branch (the place); the record's title joins the subtitle line.
    const detailTitle = workspace.branch || workspace.title;
    titleWrap.classList.add("has-brackets");
    const detailTitleNode = createNode("h2", "workspace-detail-title");
    appendBranchLabel(detailTitleNode, detailTitle);
    titleWrap.appendChild(detailTitleNode);
    const subtitle = createNode("div", "workspace-detail-subtitle");
    if (workspace.title && workspace.title !== detailTitle) {
      appendMetaText(subtitle, workspace.title);
    }
    if (workspace.merged_into_base) {
      appendMetaText(subtitle, "Merged — safe to delete");
    }
    appendMetaText(subtitle, statusLabel(workspace.status_category));
    appendMetaText(subtitle, workspace.owner);
    appendMetaText(subtitle, formatLifecycleStageLabel(workspace.lifecycle_stage));
    titleWrap.appendChild(subtitle);
    header.appendChild(titleWrap);

    const actions = createNode("div", "workspace-detail-actions");
    // SPEC-2359: Resume is a per-Work (launch) operation, so the Resume control
    // lives on each Work row (see appendWorks / renderWorkResumeButton). The
    // Workspace header carries Launch Agent (the primary Workspace action —
    // a new Work joining this Workspace) plus the lifecycle closes
    // (Done / Discard / Clean Up).
    const launchAction = renderLaunchWorkspaceButton(workspace, windowId);
    if (launchAction) {
      actions.appendChild(launchAction);
    }
    // SPEC-2359 Phase W-12 (FR-351): the Work surface owns Work lifecycle
    // closing. Done / Discard are explicit user closes (FR-350 — agent stop
    // alone never closes a Work). The actual cleanup is a follow-up slice, so
    // these buttons only emit the `close_work` message for now.
    const lifecycleState = String(workspace.lifecycle_state || "active").toLowerCase();
    if (lifecycleState !== "done" && lifecycleState !== "discarded") {
      const doneButton = createNode("button", "wizard-button", "Done");
      doneButton.type = "button";
      doneButton.dataset.action = "close-work-done";
      doneButton.addEventListener("click", () =>
        send({ kind: "close_work", work_id: workspace.id, close_kind: "done" }),
      );
      actions.appendChild(doneButton);

      const discardButton = createNode("button", "wizard-button", "Discard");
      discardButton.type = "button";
      discardButton.dataset.action = "close-work-discard";
      discardButton.addEventListener("click", () =>
        send({ kind: "close_work", work_id: workspace.id, close_kind: "discarded" }),
      );
      actions.appendChild(discardButton);
    }
    // "Safe to delete" comes with the actual delete action (user
    // verification 2026-06-12): a merged row offers Clean Up for its own
    // branch; the projection-level cleanup candidate keeps the legacy
    // no-argument path.
    if (workspace.merged_into_base && workspace.branch) {
      const cleanupButton = createNode("button", "wizard-button", "Clean Up");
      cleanupButton.type = "button";
      cleanupButton.dataset.action = "cleanup-merged-workspace";
      cleanupButton.addEventListener("click", () =>
        openWorkspaceCleanup?.({
          branch: workspace.branch,
          remote_delete_available: true,
        }),
      );
      actions.appendChild(cleanupButton);
    } else if (workspace.cleanup_candidate) {
      const cleanupButton = createNode("button", "wizard-button", "Clean Up");
      cleanupButton.type = "button";
      cleanupButton.addEventListener("click", () => openWorkspaceCleanup?.());
      actions.appendChild(cleanupButton);
    }
    header.appendChild(actions);
    container.appendChild(header);

    container.appendChild(
      detailSection("Summary", (body) => {
        appendTextBlock(body, workspace.summary || workspace.status_text || workspace.intent);
        if (workspace.intent && workspace.intent !== workspace.summary) {
          appendTextBlock(body, workspace.intent);
        }
        appendTextBlock(body, workspace.next_action);
        appendTextBlock(body, workspace.blocked_reason, "workspace-detail-text is-warning");
      }),
    );
    container.appendChild(
      detailSection("Work", (body) => {
        appendWorks(body, workspace.agents, workspace);
      }),
    );
    container.appendChild(
      detailSection("Lifecycle", (body) => {
        appendDefinitionList(body, [
          ["Stage", formatLifecycleStageLabel(workspace.lifecycle_stage)],
          ["Status", statusLabel(workspace.status_category)],
          ["Updated", workspace.updated_at],
        ]);
        appendEvents(body, workspace.events);
      }),
    );
    container.appendChild(
      detailSection("Work Context", (body) => {
        appendDefinitionList(body, [
          ["Owner", workspace.owner],
          ["Branch", workspace.branch],
          ["Worktree", compactPath(workspace.worktree_path) || workspace.worktree_path],
          ["PR", workspace.pr_number ? `PR #${workspace.pr_number}` : ""],
          ["PR state", workspace.pr_state],
        ]);
      }),
    );
    container.appendChild(
      detailSection("Coordination", (body) => {
        if (workspace.board_refs.length === 0 && workspace.events.length === 0) {
          body.appendChild(createNode("div", "workspace-overview-empty", "No Board references"));
          return;
        }
        if (workspace.board_refs.length > 0) {
          const refs = createNode("div", "workspace-board-ref-list");
          for (const ref of workspace.board_refs) {
            refs.appendChild(createNode("span", "workspace-board-ref", ref));
          }
          body.appendChild(refs);
        }
        appendEvents(body, workspace.events.filter((event) => event.board_entry_id));
      }),
    );
  }

  function renderWorkspaceOverviewWindow(windowId, force) {
    const element = windowMap.get(windowId);
    if (!element) return;
    const root = element.querySelector(".workspace-overview-root");
    if (!root) return;

    const projection = getActiveWorkProjection();
    const signature = JSON.stringify(projection);
    const state = ensureState(windowId);
    if (!force && state._lastSignature !== undefined && state._lastSignature === signature) return;
    state._lastSignature = signature;

    const workspaces = workspacesFromProjection(projection);
    const unassignedAgents = unassignedAgentsFromProjection(projection);
    const selected = selectedWorkspace(state, workspaces);

    const status = root.querySelector(".workspace-overview-status-line");
    if (status) {
      status.textContent = projection
        ? `${workspaces.length} Workspaces · ${unassignedAgents.length} Unassigned Agents`
        : "No Workspace projection";
    }

    const list = root.querySelector(".workspace-overview-list");
    list.innerHTML = "";
    if (workspaces.length === 0) {
      list.appendChild(createNode("div", "workspace-overview-empty", "No Workspaces"));
    } else {
      // User verification 2026-06-12: completed local branches need a BULK
      // cleanup path — one click opens the cleanup flow with every merged
      // Workspace preselected (the modal still lets the user prune the set).
      const mergedRows = workspaces.filter(
        (workspace) => workspace.merged_into_base && workspace.branch,
      );
      if (mergedRows.length > 0) {
        const bulkRow = createNode("div", "workspace-overview-bulk-cleanup");
        const bulk = createNode(
          "button",
          "wizard-button is-compact",
          `Clean Up Merged (${mergedRows.length})`,
        );
        bulk.type = "button";
        bulk.dataset.action = "cleanup-merged-workspaces";
        bulk.addEventListener("click", () =>
          openWorkspaceCleanup?.(
            mergedRows.map((workspace) => ({
              branch: workspace.branch,
              remote_delete_available: true,
            })),
          ),
        );
        bulkRow.appendChild(bulk);
        list.appendChild(bulkRow);
      }
      for (const workspace of workspaces) {
        list.appendChild(renderWorkspaceRow(windowId, state, workspace));
      }
    }

    const queue = root.querySelector("[data-role='workspace-agent-queue-slot']");
    queue.innerHTML = "";
    renderUnassignedQueue(queue, unassignedAgents);

    renderWorkspaceDetail(root.querySelector(".workspace-overview-detail-pane"), selected, windowId);
  }

  function mountWorkSurface(parent) {
    const root = createNode("div", "workspace-overview-root");

    const toolbar = createNode("div", "workspace-toolbar is-stacked workspace-overview-toolbar");
    const toolbarMain = createNode("div", "workspace-toolbar-main");
    toolbarMain.appendChild(createNode("div", "knowledge-heading", "Workspace"));
    toolbarMain.appendChild(createNode("div", "knowledge-status workspace-overview-status-line"));
    toolbar.appendChild(toolbarMain);

    // SPEC-2359: single fused Workspace surface — the Work / Git Branches tab
    // toggle is removed. The spine lists persistent Workspaces (Active+Paused);
    // the detail pane shows the selected Workspace's sessions / branch / PR.
    const toolbarActions = createNode("div", "workspace-toolbar-actions");
    const refreshBtn = createNode("button", "icon-button", "↻");
    refreshBtn.dataset.action = "refresh-workspace-overview";
    refreshBtn.setAttribute("aria-label", "Refresh Workspaces");
    toolbarActions.appendChild(refreshBtn);
    toolbar.appendChild(toolbarActions);
    root.appendChild(toolbar);

    const workShell = createNode("div", "workspace-overview-shell");
    const listPane = createNode("aside", "workspace-overview-list-pane");
    listPane.setAttribute("aria-label", "Workspace list");
    listPane.appendChild(createNode("div", "workspace-overview-section-label", "Workspaces"));
    const listBox = createNode("div", "workspace-overview-list");
    listBox.setAttribute("role", "listbox");
    listPane.appendChild(listBox);
    const queueSlot = createNode("div");
    queueSlot.dataset.role = "workspace-agent-queue-slot";
    listPane.appendChild(queueSlot);
    workShell.appendChild(listPane);
    const detailPane = createNode("main", "workspace-overview-detail-pane");
    detailPane.setAttribute("aria-label", "Work detail");
    workShell.appendChild(detailPane);
    root.appendChild(workShell);

    parent.appendChild(root);
    return root;
  }

  function mount(parent, windowData, { focusWindowLocally, sendFocus } = {}) {
    parent.textContent = "";
    mountWorkSurface(parent);

    parent.addEventListener("mousedown", () => {
      focusWindowLocally?.(windowData.id);
      sendFocus?.(windowData.id);
    });

    const refresh = parent.querySelector("[data-action='refresh-workspace-overview']");
    refresh?.addEventListener("click", (event) => {
      event.stopPropagation();
      renderWorkspaceOverviewWindow(windowData.id, true);
    });

    // Keyboard navigation: ArrowUp / ArrowDown move the Workspace selection
    // (user request 2026-06-11). Delegated from the mount root so the
    // listener survives row re-renders; the existing row click path handles
    // selection + re-render, then focus returns to the selected row so the
    // user can keep navigating.
    parent.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const list = parent.querySelector(".workspace-overview-list");
      if (!list) return;
      const rows = Array.from(
        list.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
      );
      if (rows.length === 0) return;
      event.preventDefault?.();
      const current = rows.findIndex(
        (row) => row.getAttribute("aria-selected") === "true",
      );
      const delta = event.key === "ArrowDown" ? 1 : -1;
      const targetIndex = Math.min(
        rows.length - 1,
        Math.max(0, current === -1 ? 0 : current + delta),
      );
      const target = rows[targetIndex];
      if (!target || target.getAttribute("aria-selected") === "true") return;
      target.click();
      focusSelectedWorkspaceRow(windowData.id);
    });

    renderWorkspaceOverviewWindow(windowData.id);
  }

  function renderWindows() {
    for (const windowData of activeWorkspace()?.windows || []) {
      const preset = workspaceWindowById(windowData.id)?.preset;
      if (preset !== "work" && preset !== "workspace" && preset !== "branches") {
        continue;
      }
      renderWorkspaceOverviewWindow(windowData.id);
    }
  }

  function deleteState(windowId) {
    workspaceStateMap.delete(windowId);
  }

  return {
    mount,
    renderWindows,
    deleteState,
    _workspacesFromProjection: workspacesFromProjection,
  };
}

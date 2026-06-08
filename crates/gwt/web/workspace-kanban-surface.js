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
  branchesSurface,
  visibleBounds,
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

  function renderWorkspaceRow(windowId, state, item) {
    const row = createNode("button", "workspace-overview-row");
    row.type = "button";
    row.dataset.workspaceId = item.id;
    row.dataset.status = String(item.status_category || "idle").toLowerCase();
    const isSelected = state.selectedId === item.id;
    // aria-selected drives the CSS/test contract; aria-current is what AT
    // actually announces on a default-role button.
    row.setAttribute("aria-selected", isSelected ? "true" : "false");
    if (isSelected) row.setAttribute("aria-current", "true");
    else row.removeAttribute("aria-current");

    const status = createNode("span", "workspace-overview-status", statusLabel(item.status_category));
    const copy = createNode("span", "workspace-overview-row-copy");
    copy.appendChild(createNode("span", "workspace-overview-row-title", item.title));
    const meta = createNode("span", "workspace-overview-row-meta");
    appendMetaText(meta, item.owner);
    appendMetaText(meta, item.branch);
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
    });
    return row;
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
      appendMetaText(meta, "No Work selected");
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

  function appendAgents(container, agents) {
    if (!agents || agents.length === 0) {
      container.appendChild(createNode("div", "workspace-overview-empty", "No assigned agents"));
      return;
    }
    const liveStatuses = new Set(["active", "blocked", "idle", "running"]);
    const live = agents.filter((a) => liveStatuses.has(String(a.status_category || "").toLowerCase()));
    const terminated = agents.length - live.length;

    if (live.length === 0) {
      const msg = terminated > 0
        ? `No active agents (${terminated} completed)`
        : "No assigned agents";
      container.appendChild(createNode("div", "workspace-overview-empty", msg));
      return;
    }

    const INITIAL_LIMIT = 5;
    const list = createNode("div", "workspace-detail-agent-list");
    const visible = live.slice(0, INITIAL_LIMIT);
    const hidden = live.slice(INITIAL_LIMIT);
    for (const agent of visible) {
      list.appendChild(renderAgentRow(agent));
    }
    container.appendChild(list);
    if (hidden.length > 0) {
      const more = createNode("button", "workspace-detail-more", `${hidden.length} more agents`);
      more.type = "button";
      more.addEventListener("click", () => {
        for (const agent of hidden) {
          list.appendChild(renderAgentRow(agent));
        }
        more.remove();
      });
      container.appendChild(more);
    }
    if (terminated > 0) {
      container.appendChild(createNode("div", "workspace-overview-empty", `${terminated} completed agents`));
    }
  }

  function renderAgentRow(agent) {
    const row = createNode("article", "workspace-detail-agent");
    row.dataset.status = String(agent.status_category || "idle").toLowerCase();
    row.appendChild(
      createNode("div", "workspace-detail-agent-name", agent.display_name || agent.agent_id || "Agent"),
    );
    const meta = createNode("div", "workspace-detail-agent-meta");
    appendMetaText(meta, agentStatusLabel?.(agent.status_category));
    appendMetaText(meta, agent.title_summary || agent.current_focus);
    row.appendChild(meta);
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

  function renderWorkspaceDetail(container, workspace) {
    container.innerHTML = "";
    if (!workspace) {
      const empty = createNode("div", "workspace-overview-empty", "No Work");
      container.appendChild(empty);
      return;
    }

    const header = createNode("header", "workspace-detail-header");
    const titleWrap = createNode("div", "workspace-detail-heading");
    titleWrap.appendChild(createNode("h2", "workspace-detail-title", workspace.title));
    const subtitle = createNode("div", "workspace-detail-subtitle");
    appendMetaText(subtitle, statusLabel(workspace.status_category));
    appendMetaText(subtitle, workspace.owner);
    appendMetaText(subtitle, formatLifecycleStageLabel(workspace.lifecycle_stage));
    titleWrap.appendChild(subtitle);
    header.appendChild(titleWrap);

    const actions = createNode("div", "workspace-detail-actions");
    const resumeButton = createNode("button", "wizard-button", "Resume");
    resumeButton.type = "button";
    resumeButton.dataset.action = "resume-workspace";
    resumeButton.addEventListener("click", () => resumeWorkspace(workspace));
    actions.appendChild(resumeButton);
    if (workspace.cleanup_candidate) {
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
      detailSection("Agents", (body) => {
        appendAgents(body, workspace.agents);
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

  function normalizedBranchName(value) {
    return String(value || "").trim();
  }

  // Read-only access to the Branches surface state (SPEC-2009). The unified
  // Work surface (SPEC-2359 W-13/US-67) consumes branch entries to build a
  // branch backbone but never mutates the Branches render path.
  function branchListState(windowId) {
    if (!branchesSurface || typeof branchesSurface.ensureBranchListState !== "function") {
      return null;
    }
    try {
      return branchesSurface.ensureBranchListState(windowId);
    } catch {
      return null;
    }
  }

  function branchEntryVisibleForFilter(entry, filter) {
    if (!filter || filter === "all") return true;
    return entry.scope === filter;
  }

  // SPEC-2359 W-13/US-67 — Work-led model (Option A). Branch = Work = the place
  // work happens; a session is the work content. A Work always has >=1 agent.
  // Its LIVE sessions are shown flat (work-content / focus led), each tagged
  // with its agent type — the agent type alone does not identify the work, so
  // there is no agent-type grouping layer.
  const LIVE_SESSION_STATUSES = new Set(["active", "blocked", "idle", "running"]);

  function isLiveSession(session) {
    return LIVE_SESSION_STATUSES.has(
      String(session?.status_category || "").toLowerCase(),
    );
  }

  // Strip remote prefixes so a Work's branch and its remote-tracking
  // counterpart (origin/<branch>) are treated as the same place.
  function branchBaseName(value) {
    return normalizedBranchName(value)
      .replace(/^refs\/remotes\//, "")
      .replace(/^remotes\//, "")
      .replace(/^origin\//, "");
  }

  // Branches that back no active Work — pure git branches with no agent. A Work
  // always has an agent, so these are not Works; they live in a collapsed
  // "Other branches (idle)" section for Launch only. A work branch and its
  // remote-tracking counterpart are excluded together.
  function idleBranchEntries(workspaces, branchState) {
    const entries = Array.isArray(branchState?.entries) ? branchState.entries : [];
    if (entries.length === 0) return [];
    const workBranches = new Set(
      workspaces.map((work) => branchBaseName(work.branch)).filter(Boolean),
    );
    const filter = branchState?.filter || "local";
    return entries.filter((entry) => {
      const name = normalizedBranchName(entry.name);
      if (!name || workBranches.has(branchBaseName(name))) return false;
      return branchEntryVisibleForFilter(entry, filter);
    });
  }

  function sessionFocus(session) {
    return session.title_summary || session.current_focus || "No focus set";
  }

  function agentTypeLabel(session) {
    return session.display_name || session.agent_id || "Agent";
  }

  // One flat session row: status + work content (focus) lead; the agent type
  // is a small trailing tag.
  function renderSessionRow(session) {
    const row = createNode("article", "workspace-work-session");
    row.dataset.sessionId = session.session_id || "";
    const status = String(session.status_category || "idle").toLowerCase();
    row.dataset.status = status;
    row.appendChild(
      createNode(
        "span",
        "workspace-work-session-status",
        agentStatusLabel?.(status) || status,
      ),
    );
    row.appendChild(
      createNode("span", "workspace-work-session-focus", sessionFocus(session)),
    );
    row.appendChild(
      createNode("span", "workspace-work-session-agent", agentTypeLabel(session)),
    );
    return row;
  }

  function completedNote(count) {
    return createNode(
      "div",
      "workspace-overview-empty",
      `${count} completed session${count === 1 ? "" : "s"}`,
    );
  }

  // A Work is the spine row: branch = the place; its live sessions = the work
  // content. Terminated sessions are summarized, not listed.
  function renderWorkGroup(windowId, state, work) {
    const group = createNode("div", "workspace-work-group");
    group.dataset.workspaceId = work.id;
    if (work.branch) group.dataset.branchName = work.branch;

    // Reuse the existing selectable Work row as the group header so selection
    // → detail and the established row contract keep working.
    group.appendChild(renderWorkspaceRow(windowId, state, work));

    if (work.branch) {
      const actions = createNode("div", "workspace-work-actions");
      const resumeBtn = createNode("button", "branch-row-action", "Resume");
      resumeBtn.type = "button";
      resumeBtn.dataset.branchRowAction = "resume";
      resumeBtn.title = `Resume latest agent on ${work.branch}`;
      resumeBtn.addEventListener("click", (event) => {
        event.stopPropagation();
        send({
          kind: "resume_branch_latest_agent",
          id: windowId,
          branch_name: work.branch,
          bounds: typeof visibleBounds === "function" ? visibleBounds() : undefined,
        });
      });
      actions.appendChild(resumeBtn);
      const launchBtn = createNode("button", "branch-row-action primary", "Launch");
      launchBtn.type = "button";
      launchBtn.dataset.branchRowAction = "launch";
      launchBtn.title = `Launch Agent on ${work.branch}`;
      launchBtn.addEventListener("click", (event) => {
        event.stopPropagation();
        send({ kind: "open_launch_wizard", id: windowId, branch_name: work.branch });
      });
      actions.appendChild(launchBtn);
      group.appendChild(actions);
    }

    const allSessions = Array.isArray(work.agents) ? work.agents : [];
    const live = allSessions.filter(isLiveSession);
    const terminated = allSessions.length - live.length;
    const sessionsWrap = createNode("div", "workspace-work-sessions");
    if (live.length > 0) {
      for (const session of live) {
        sessionsWrap.appendChild(renderSessionRow(session));
      }
      if (terminated > 0) sessionsWrap.appendChild(completedNote(terminated));
    } else if (terminated > 0) {
      sessionsWrap.appendChild(completedNote(terminated));
    } else {
      sessionsWrap.appendChild(
        createNode("div", "workspace-overview-empty", "No active sessions"),
      );
    }
    group.appendChild(sessionsWrap);
    return group;
  }

  function renderIdleBranches(windowId, state, entries, branchState) {
    const section = createNode("section", "workspace-idle-branches");
    const expanded = Boolean(state.idleExpanded);
    const toggle = createNode(
      "button",
      "workspace-idle-toggle",
      `${expanded ? "▾" : "▸"} Other branches (idle) (${entries.length})`,
    );
    toggle.type = "button";
    toggle.dataset.action = "toggle-idle-branches";
    toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
    toggle.addEventListener("click", (event) => {
      event.stopPropagation();
      state.idleExpanded = !state.idleExpanded;
      renderWorkspaceOverviewWindow(windowId, true);
    });
    section.appendChild(toggle);
    if (!expanded) return section;

    // Local/Remote/All only ever scoped idle branches — keep the filter here,
    // clearly labeled to this section, not in the global toolbar.
    const activeFilter = branchState?.filter || "local";
    const filterGroup = createNode("div", "branch-filter-group");
    for (const [label, filter] of [["Local", "local"], ["Remote", "remote"], ["All", "all"]]) {
      const btn = createNode("button", "branch-filter-button", label);
      btn.type = "button";
      btn.dataset.branchFilter = filter;
      btn.classList.toggle("is-active", filter === activeFilter);
      btn.classList.toggle("active", filter === activeFilter);
      btn.addEventListener("click", (event) => {
        event.stopPropagation();
        if (branchState) branchState.filter = filter;
        renderWorkspaceOverviewWindow(windowId, true);
      });
      filterGroup.appendChild(btn);
    }
    section.appendChild(filterGroup);

    const list = createNode("div", "workspace-idle-branch-list");
    for (const entry of entries) {
      const name = normalizedBranchName(entry.name);
      const row = createNode("div", "workspace-branch-row is-idle");
      row.dataset.branchName = name;
      const head = createNode("div", "workspace-branch-head");
      const nameWrap = createNode("div", "workspace-branch-name");
      nameWrap.appendChild(createNode("span", "workspace-branch-name-text", name));
      if (entry.is_head) {
        nameWrap.appendChild(createNode("span", "workspace-branch-head-badge", "HEAD"));
      }
      head.appendChild(nameWrap);
      const meta = createNode("div", "workspace-branch-meta");
      appendMetaText(meta, entry.scope);
      appendMetaText(
        meta,
        entry.ahead || entry.behind ? `↑${entry.ahead} ↓${entry.behind}` : "synced",
      );
      head.appendChild(meta);
      const actions = createNode("div", "workspace-branch-actions");
      const launchBtn = createNode("button", "branch-row-action primary", "Launch");
      launchBtn.type = "button";
      launchBtn.dataset.branchRowAction = "launch";
      launchBtn.title = `Launch Agent on ${name}`;
      launchBtn.addEventListener("click", (event) => {
        event.stopPropagation();
        send({ kind: "open_launch_wizard", id: windowId, branch_name: name });
      });
      actions.appendChild(launchBtn);
      head.appendChild(actions);
      row.appendChild(head);
      list.appendChild(row);
    }
    section.appendChild(list);
    return section;
  }

  function aggregatePlaceStatus(sessions, fallback) {
    const statuses = (sessions || []).map((s) => String(s.status_category || "").toLowerCase());
    if (statuses.includes("blocked")) return "blocked";
    if (statuses.some((s) => s === "active" || s === "running")) return "active";
    return fallback || "idle";
  }

  // SPEC-2359 US-67 (Option A) — Work = the place (branch). Group the
  // projection's works by branch so a place is the same regardless of how the
  // backend keys a Work: branch-derived (W-8, one work per branch) or
  // session-derived (W-12, one work per session). All sessions on a branch are
  // flattened under one place. A single work on its branch keeps its own
  // identity; multiple session-works on one branch aggregate into one place.
  function placesFromProjection(projection) {
    const works = workspacesFromProjection(projection);
    // Only the active_works stream carries per-Work branches (W-8 branch-keyed
    // or W-12 session-keyed). The legacy works/journal fallback inherits the
    // projection branch, so grouping it by branch would wrongly collapse it.
    const hasActiveWorks =
      Array.isArray(projection?.active_works) && projection.active_works.length > 0;
    if (!hasActiveWorks) return works;
    const order = [];
    const byBranch = new Map();
    const branchless = [];
    for (const work of works) {
      const base = branchBaseName(work.branch);
      if (!base) {
        branchless.push(work);
        continue;
      }
      if (!byBranch.has(base)) {
        byBranch.set(base, []);
        order.push(base);
      }
      byBranch.get(base).push(work);
    }
    const places = [];
    for (const base of order) {
      const group = byBranch.get(base);
      if (group.length === 1) {
        places.push(group[0]);
        continue;
      }
      const first = group[0];
      const agents = [];
      const events = [];
      const board_refs = [];
      for (const work of group) {
        if (Array.isArray(work.agents)) agents.push(...work.agents);
        if (Array.isArray(work.events)) events.push(...work.events);
        if (Array.isArray(work.board_refs)) board_refs.push(...work.board_refs);
      }
      const withPr = group.find((work) => work.pr_number);
      places.push({
        id: `place-${base}`,
        title: first.branch || base,
        branch: first.branch,
        owner: group.find((work) => work.owner)?.owner || first.owner,
        summary: group.find((work) => work.summary)?.summary || first.summary,
        intent: first.intent,
        status_text: first.status_text,
        next_action: first.next_action,
        blocked_reason: group.find((work) => work.blocked_reason)?.blocked_reason || "",
        lifecycle_stage: first.lifecycle_stage,
        worktree_path: first.worktree_path,
        pr_number: withPr?.pr_number || null,
        pr_url: withPr?.pr_url || "",
        pr_state: withPr?.pr_state || "",
        board_refs,
        events,
        agents,
        updated_at: first.updated_at,
        status_category: aggregatePlaceStatus(agents, first.status_category),
      });
    }
    for (const work of branchless) places.push(work);
    return places;
  }

  function renderWorkspaceOverviewWindow(windowId, force) {
    const element = windowMap.get(windowId);
    if (!element) return;
    const root = element.querySelector(".workspace-overview-root");
    if (!root) return;

    const projection = getActiveWorkProjection();
    const branchState = branchListState(windowId);
    const signature = JSON.stringify({
      projection,
      entries: branchState ? branchState.entries : null,
      filter: branchState ? branchState.filter : null,
      loading: branchState ? branchState.loading : false,
      error: branchState ? branchState.error : "",
      cleanup: branchState && branchState.cleanupSelected instanceof Set
        ? Array.from(branchState.cleanupSelected)
        : null,
    });
    const state = ensureState(windowId);
    if (!force && state._lastSignature !== undefined && state._lastSignature === signature) return;
    state._lastSignature = signature;

    const places = placesFromProjection(projection);
    const unassignedAgents = unassignedAgentsFromProjection(projection);
    const selected = selectedWorkspace(state, places);
    const idleBranches = idleBranchEntries(places, branchState);

    const status = root.querySelector(".workspace-overview-status-line");
    if (status) {
      if (!projection && !branchState) {
        status.textContent = "No Work projection";
      } else {
        const parts = [`${places.length} Active Works`];
        if (branchState) parts.push(`${idleBranches.length} Idle Branches`);
        parts.push(`${unassignedAgents.length} Unassigned Agents`);
        status.textContent = parts.join(" · ");
      }
    }

    const branchNotice = root.querySelector(".branch-notice");
    if (branchNotice) {
      if (branchState?.error) {
        branchNotice.hidden = false;
        branchNotice.textContent = branchState.error;
      } else {
        branchNotice.hidden = true;
        branchNotice.textContent = "";
      }
    }

    // Work-led spine: each Work (a place = branch) lists its live sessions
    // (the work content). Idle git branches (no agent) are not Works and live
    // in a collapsed section below.
    const list = root.querySelector(".workspace-overview-list");
    list.innerHTML = "";
    if (places.length === 0) {
      list.appendChild(
        createNode(
          "div",
          "workspace-overview-empty",
          branchState?.loading
            ? "Loading Work…"
            : projection || branchState
              ? "No active Work"
              : "No Work projection",
        ),
      );
    } else {
      for (const place of places) {
        list.appendChild(renderWorkGroup(windowId, state, place));
      }
    }
    if (branchState && idleBranches.length > 0) {
      list.appendChild(renderIdleBranches(windowId, state, idleBranches, branchState));
    }

    const queue = root.querySelector("[data-role='workspace-agent-queue-slot']");
    queue.innerHTML = "";
    renderUnassignedQueue(queue, unassignedAgents);

    renderWorkspaceDetail(root.querySelector(".workspace-overview-detail-pane"), selected);
  }

  function mountWorkSurface(parent) {
    const root = createNode("div", "workspace-overview-root");

    const toolbar = createNode("div", "workspace-toolbar is-stacked workspace-overview-toolbar");
    const toolbarMain = createNode("div", "workspace-toolbar-main");
    toolbarMain.appendChild(createNode("div", "knowledge-heading", "Work"));
    toolbarMain.appendChild(createNode("div", "knowledge-status workspace-overview-status-line"));
    toolbar.appendChild(toolbarMain);

    // SPEC-2359 W-13/US-67 — Work-led unified view. The spine is Active Works.
    // Branch scope filter lives inside the collapsed "Other branches (idle)"
    // section; branch cleanup belongs to the Branches surface (SPEC-2009).
    const toolbarActions = createNode("div", "workspace-toolbar-actions");
    const branchRefreshBtn = createNode("button", "icon-button", "↻");
    branchRefreshBtn.dataset.action = "refresh-branches";
    branchRefreshBtn.setAttribute("aria-label", "Refresh branches");
    toolbarActions.appendChild(branchRefreshBtn);

    const refreshBtn = createNode("button", "icon-button", "⟳");
    refreshBtn.dataset.action = "refresh-workspace-overview";
    refreshBtn.setAttribute("aria-label", "Refresh Work");
    toolbarActions.appendChild(refreshBtn);
    toolbar.appendChild(toolbarActions);
    root.appendChild(toolbar);

    const shell = createNode("div", "workspace-overview-shell");
    const listPane = createNode("aside", "workspace-overview-list-pane");
    listPane.setAttribute("aria-label", "Active Works list");
    listPane.appendChild(
      createNode("div", "workspace-overview-section-label", "Active Works"),
    );
    const branchNotice = createNode("div", "branch-notice");
    branchNotice.hidden = true;
    listPane.appendChild(branchNotice);
    const listBox = createNode("div", "workspace-overview-list");
    listBox.setAttribute("role", "list");
    listPane.appendChild(listBox);
    const queueSlot = createNode("div");
    queueSlot.dataset.role = "workspace-agent-queue-slot";
    listPane.appendChild(queueSlot);
    shell.appendChild(listPane);

    const detailPane = createNode("main", "workspace-overview-detail-pane");
    detailPane.setAttribute("aria-label", "Work detail");
    shell.appendChild(detailPane);
    root.appendChild(shell);

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

    if (branchesSurface) {
      const branchRefresh = parent.querySelector("[data-action='refresh-branches']");
      branchRefresh?.addEventListener("click", (event) => {
        event.stopPropagation();
        const state = branchesSurface.ensureBranchListState(windowData.id);
        state.error = "";
        state.notice = "";
        branchesSurface.requestBranches(windowData.id);
        renderWorkspaceOverviewWindow(windowData.id, true);
      });
    }

    const refresh = parent.querySelector("[data-action='refresh-workspace-overview']");
    refresh?.addEventListener("click", (event) => {
      event.stopPropagation();
      renderWorkspaceOverviewWindow(windowData.id, true);
    });

    // SPEC-2359 W-13/US-67 — load branches once so the unified backbone can show
    // non-work branches alongside Active Works (the Branches surface state is
    // shared read-only; the Branches render path itself is left untouched).
    if (branchesSurface) {
      const state = branchesSurface.ensureBranchListState(windowData.id);
      if (
        (state.entries?.length || 0) === 0 &&
        !state.loading &&
        !state.error
      ) {
        branchesSurface.requestBranches(windowData.id);
      }
    }

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

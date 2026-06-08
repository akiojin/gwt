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
    row.setAttribute("aria-selected", state.selectedId === item.id ? "true" : "false");

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

  // Join Active Works (agent-session identity, FR-348) onto branch rows by
  // branch name. Works always create their branch group so active work is
  // never hidden by a branch filter; branch entries add non-work branches.
  function buildBranchBackbone(workspaces, branchState) {
    const groups = [];
    const groupByName = new Map();
    const detached = [];

    function ensureGroup(name, entry) {
      let group = groupByName.get(name);
      if (!group) {
        group = { name, entry: entry || null, works: [] };
        groupByName.set(name, group);
        groups.push(group);
      } else if (entry && !group.entry) {
        group.entry = entry;
      }
      return group;
    }

    for (const work of workspaces) {
      const branch = normalizedBranchName(work.branch);
      if (!branch) {
        detached.push(work);
        continue;
      }
      ensureGroup(branch).works.push(work);
    }

    const entries = Array.isArray(branchState?.entries) ? branchState.entries : [];
    const filter = branchState?.filter || "all";
    for (const entry of entries) {
      const name = normalizedBranchName(entry.name);
      if (!name) continue;
      if (groupByName.has(name)) {
        ensureGroup(name, entry);
        continue;
      }
      if (!branchEntryVisibleForFilter(entry, filter)) continue;
      ensureGroup(name, entry);
    }

    return { groups, detached };
  }

  function renderBranchGroup(windowId, state, group, branchState) {
    const row = createNode("div", "workspace-branch-row");
    row.dataset.branchName = group.name;
    const entry = group.entry;

    const header = createNode("div", "workspace-branch-head");

    const cleanupSet =
      branchState && branchState.cleanupSelected instanceof Set
        ? branchState.cleanupSelected
        : null;
    if (cleanupSet && entry && entry.cleanup_ready) {
      const isSelected = cleanupSet.has(group.name);
      const toggle = createNode(
        "button",
        "workspace-branch-cleanup-toggle",
        isSelected ? "☑" : "☐",
      );
      toggle.type = "button";
      toggle.dataset.branchCleanupToggle = group.name;
      toggle.setAttribute("aria-pressed", isSelected ? "true" : "false");
      toggle.title = "Select branch for cleanup";
      toggle.addEventListener("click", (event) => {
        event.stopPropagation();
        if (cleanupSet.has(group.name)) cleanupSet.delete(group.name);
        else cleanupSet.add(group.name);
        renderWorkspaceOverviewWindow(windowId, true);
      });
      header.appendChild(toggle);
    }

    const nameWrap = createNode("div", "workspace-branch-name");
    nameWrap.appendChild(createNode("span", "workspace-branch-name-text", group.name));
    if (entry?.is_head) {
      nameWrap.appendChild(createNode("span", "workspace-branch-head-badge", "HEAD"));
    }
    header.appendChild(nameWrap);

    const meta = createNode("div", "workspace-branch-meta");
    if (entry) {
      appendMetaText(meta, entry.scope);
      appendMetaText(
        meta,
        entry.ahead || entry.behind ? `↑${entry.ahead} ↓${entry.behind}` : "synced",
      );
    }
    appendMetaText(
      meta,
      `${group.works.length} Work${group.works.length === 1 ? "" : "s"}`,
    );
    header.appendChild(meta);

    const actions = createNode("div", "workspace-branch-actions");
    const resumeBtn = createNode("button", "branch-row-action", "Resume");
    resumeBtn.type = "button";
    resumeBtn.dataset.branchRowAction = "resume";
    const resumeAvailable = Boolean(entry?.resume?.available);
    resumeBtn.disabled = !resumeAvailable;
    resumeBtn.title = resumeAvailable
      ? `Resume latest agent on ${group.name}`
      : entry?.resume?.reason || "No resumable session";
    resumeBtn.addEventListener("click", (event) => {
      event.stopPropagation();
      if (resumeBtn.disabled) return;
      send({
        kind: "resume_branch_latest_agent",
        id: windowId,
        branch_name: group.name,
        bounds: typeof visibleBounds === "function" ? visibleBounds() : undefined,
      });
    });
    actions.appendChild(resumeBtn);

    const launchBtn = createNode("button", "branch-row-action primary", "Launch");
    launchBtn.type = "button";
    launchBtn.dataset.branchRowAction = "launch";
    launchBtn.title = `Launch Agent on ${group.name}`;
    launchBtn.addEventListener("click", (event) => {
      event.stopPropagation();
      send({ kind: "open_launch_wizard", id: windowId, branch_name: group.name });
    });
    actions.appendChild(launchBtn);
    header.appendChild(actions);

    row.appendChild(header);

    // US-67 #3 — a branch with no active work shows only its header (no work
    // overlay), so the backbone stays compact across many branches.
    if (group.works.length > 0) {
      const works = createNode("div", "workspace-branch-works");
      for (const work of group.works) {
        works.appendChild(renderWorkspaceRow(windowId, state, work));
      }
      row.appendChild(works);
    }
    return row;
  }

  function renderDetachedGroup(windowId, state, works) {
    const row = createNode("div", "workspace-branch-row is-detached");
    row.dataset.branchName = "";
    const header = createNode("div", "workspace-branch-head");
    header.appendChild(createNode("span", "workspace-branch-name-text", "Other Work"));
    header.appendChild(
      createNode(
        "div",
        "workspace-branch-meta",
        `${works.length} Work${works.length === 1 ? "" : "s"}`,
      ),
    );
    row.appendChild(header);
    const list = createNode("div", "workspace-branch-works");
    for (const work of works) {
      list.appendChild(renderWorkspaceRow(windowId, state, work));
    }
    row.appendChild(list);
    return row;
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

    const workspaces = workspacesFromProjection(projection);
    const unassignedAgents = unassignedAgentsFromProjection(projection);
    const selected = selectedWorkspace(state, workspaces);
    const { groups, detached } = buildBranchBackbone(workspaces, branchState);

    const status = root.querySelector(".workspace-overview-status-line");
    if (status) {
      if (!projection && !branchState) {
        status.textContent = "No Work projection";
      } else {
        const parts = [`${workspaces.length} Active Works`];
        if (branchState) parts.push(`${groups.length} Branches`);
        parts.push(`${unassignedAgents.length} Unassigned Agents`);
        status.textContent = parts.join(" · ");
      }
    }

    const activeFilter = branchState?.filter || "local";
    for (const button of root.querySelectorAll("[data-branch-filter]")) {
      button.classList.toggle("is-active", button.dataset.branchFilter === activeFilter);
      button.classList.toggle("active", button.dataset.branchFilter === activeFilter);
    }

    const cleanupBtn = root.querySelector("[data-action='open-branch-cleanup']");
    if (cleanupBtn) {
      const selectedCount =
        branchState && branchState.cleanupSelected instanceof Set
          ? branchState.cleanupSelected.size
          : 0;
      cleanupBtn.disabled = selectedCount === 0;
      cleanupBtn.textContent =
        selectedCount === 0 ? "Clean Up" : `Clean Up (${selectedCount})`;
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

    const list = root.querySelector(".workspace-overview-list");
    list.innerHTML = "";
    if (groups.length === 0 && detached.length === 0) {
      list.appendChild(
        createNode(
          "div",
          "workspace-overview-empty",
          branchState?.loading ? "Loading branches" : "No Work",
        ),
      );
    } else {
      for (const group of groups) {
        list.appendChild(renderBranchGroup(windowId, state, group, branchState));
      }
      if (detached.length > 0) {
        list.appendChild(renderDetachedGroup(windowId, state, detached));
      }
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

    // SPEC-2359 W-13/US-67 — Work and Branches are a single branch-backbone
    // view, so the branch filter / cleanup / refresh controls live directly in
    // the unified toolbar instead of behind a Work/Git Branches tab toggle.
    const toolbarActions = createNode("div", "workspace-toolbar-actions");
    const filterGroup = createNode("div", "branch-filter-group");
    for (const [label, filter] of [["Local", "local"], ["Remote", "remote"], ["All", "all"]]) {
      const btn = createNode("button", "branch-filter-button", label);
      btn.type = "button";
      btn.dataset.branchFilter = filter;
      filterGroup.appendChild(btn);
    }
    toolbarActions.appendChild(filterGroup);

    const cleanupBtn = createNode("button", "wizard-button branch-cleanup-trigger", "Clean Up");
    cleanupBtn.type = "button";
    cleanupBtn.dataset.action = "open-branch-cleanup";
    cleanupBtn.disabled = true;
    toolbarActions.appendChild(cleanupBtn);

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
    listPane.setAttribute("aria-label", "Branch and Work list");
    listPane.appendChild(
      createNode("div", "workspace-overview-section-label", "Branches & Active Works"),
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
      for (const button of parent.querySelectorAll("[data-branch-filter]")) {
        button.addEventListener("click", (event) => {
          event.stopPropagation();
          const state = branchesSurface.ensureBranchListState(windowData.id);
          state.filter = button.dataset.branchFilter;
          renderWorkspaceOverviewWindow(windowData.id, true);
        });
      }
      parent.querySelector("[data-action='open-branch-cleanup']")
        ?.addEventListener("click", (event) => {
          event.stopPropagation();
          branchesSurface.openBranchCleanupModal(windowData.id);
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

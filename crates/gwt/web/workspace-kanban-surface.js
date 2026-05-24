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
    const sourceItems = Array.isArray(projection.workspaces)
      ? projection.workspaces
      : Array.isArray(projection.work_items)
        ? projection.work_items
        : [];
    if (sourceItems.length > 0) {
      return sourceItems.map((item) => normalizeWorkspaceItem(item, projection));
    }

    const current = normalizeWorkspaceItem(projection, {
      id: projection.id || "__current_workspace__",
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
      createNode("div", "workspace-overview-section-label", "Unassigned agents"),
    );
    if (agents.length === 0) {
      section.appendChild(
        createNode("div", "workspace-overview-empty", "No unassigned agents"),
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

  function renderWorkspaceOverviewWindow(windowId, force) {
    const element = windowMap.get(windowId);
    if (!element) return;
    const root = element.querySelector(".workspace-overview-root");
    if (!root) return;

    const projection = getActiveWorkProjection();
    const signature = JSON.stringify(projection);
    const state = ensureState(windowId);
    if (!force && state._lastSignature === signature) return;
    state._lastSignature = signature;

    const workspaces = workspacesFromProjection(projection);
    const unassignedAgents = unassignedAgentsFromProjection(projection);
    const selected = selectedWorkspace(state, workspaces);

    const status = root.querySelector(".workspace-overview-status-line");
    if (status) {
      status.textContent = projection
        ? `${workspaces.length} Work · ${unassignedAgents.length} unassigned agents`
        : "No Work projection";
    }

    const list = root.querySelector(".workspace-overview-list");
    list.innerHTML = "";
    if (workspaces.length === 0) {
      list.appendChild(createNode("div", "workspace-overview-empty", "No Work"));
    } else {
      for (const workspace of workspaces) {
        list.appendChild(renderWorkspaceRow(windowId, state, workspace));
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

    const toolbarActions = createNode("div", "workspace-toolbar-actions");
    const tabGroup = createNode("div", "workspace-tab-group");
    const workTab = createNode("button", "workspace-tab is-active", "Work");
    workTab.type = "button";
    workTab.dataset.workTab = "work";
    const branchTab = createNode("button", "workspace-tab", "Git Branches");
    branchTab.type = "button";
    branchTab.dataset.workTab = "branches";
    tabGroup.appendChild(workTab);
    tabGroup.appendChild(branchTab);
    toolbarActions.appendChild(tabGroup);

    const refreshBtn = createNode("button", "icon-button", "↻");
    refreshBtn.dataset.action = "refresh-workspace-overview";
    refreshBtn.setAttribute("aria-label", "Refresh Work");
    toolbarActions.appendChild(refreshBtn);
    toolbar.appendChild(toolbarActions);
    root.appendChild(toolbar);

    const workShell = createNode("div", "workspace-overview-shell");
    workShell.dataset.workSection = "work";
    const listPane = createNode("aside", "workspace-overview-list-pane");
    listPane.setAttribute("aria-label", "Work list");
    listPane.appendChild(createNode("div", "workspace-overview-section-label", "Work"));
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

    const branchShell = createNode("div", "workspace-branches-shell");
    branchShell.dataset.workSection = "branches";
    branchShell.hidden = true;
    const branchRoot = createNode("div", "branch-list-root");
    const branchToolbar = createNode("div", "branch-toolbar workspace-toolbar is-stacked");
    const branchToolbarMain = createNode("div", "branch-toolbar-main workspace-toolbar-main");
    branchToolbarMain.appendChild(createNode("div", "branch-heading", "Git Branches"));
    const filterGroup = createNode("div", "branch-filter-group");
    for (const [label, filter] of [["Local", "local"], ["Remote", "remote"], ["All", "all"]]) {
      const btn = createNode("button", "branch-filter-button", label);
      btn.type = "button";
      btn.dataset.branchFilter = filter;
      filterGroup.appendChild(btn);
    }
    branchToolbarMain.appendChild(filterGroup);
    branchToolbar.appendChild(branchToolbarMain);
    const branchToolbarActions = createNode("div", "branch-toolbar-actions workspace-toolbar-actions");
    const selectionActions = createNode("div", "branch-selection-actions");
    const cleanupBtn = createNode("button", "wizard-button branch-cleanup-trigger", "Clean Up");
    cleanupBtn.type = "button";
    cleanupBtn.dataset.action = "open-branch-cleanup";
    selectionActions.appendChild(cleanupBtn);
    branchToolbarActions.appendChild(selectionActions);
    const branchRefreshBtn = createNode("button", "icon-button", "↻");
    branchRefreshBtn.dataset.action = "refresh-branches";
    branchRefreshBtn.setAttribute("aria-label", "Refresh branches");
    branchToolbarActions.appendChild(branchRefreshBtn);
    branchToolbar.appendChild(branchToolbarActions);
    branchRoot.appendChild(branchToolbar);
    const branchNotice = createNode("div", "branch-notice");
    branchNotice.hidden = true;
    branchRoot.appendChild(branchNotice);
    const branchScroll = createNode("div", "branch-scroll workspace-scroll");
    branchScroll.appendChild(createNode("div", "branch-list"));
    branchRoot.appendChild(branchScroll);
    branchShell.appendChild(branchRoot);
    root.appendChild(branchShell);

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

    for (const tab of parent.querySelectorAll("[data-work-tab]")) {
      tab.addEventListener("click", (event) => {
        event.stopPropagation();
        const target = tab.dataset.workTab;
        for (const t of parent.querySelectorAll("[data-work-tab]")) {
          t.classList.toggle("is-active", t.dataset.workTab === target);
        }
        for (const section of parent.querySelectorAll("[data-work-section]")) {
          section.hidden = section.dataset.workSection !== target;
        }
        if (target === "branches" && branchesSurface) {
          const state = branchesSurface.ensureBranchListState(windowData.id);
          if (state.entries.length === 0 && !state.loading && !state.error) {
            branchesSurface.requestBranches(windowData.id);
          }
          branchesSurface.renderBranches(windowData.id);
        }
      });
    }

    if (branchesSurface) {
      const branchRefresh = parent.querySelector("[data-action='refresh-branches']");
      branchRefresh?.addEventListener("click", (event) => {
        event.stopPropagation();
        const state = branchesSurface.ensureBranchListState(windowData.id);
        state.error = "";
        state.notice = "";
        branchesSurface.requestBranches(windowData.id);
        branchesSurface.renderBranches(windowData.id);
      });
      for (const button of parent.querySelectorAll("[data-branch-filter]")) {
        button.addEventListener("click", (event) => {
          event.stopPropagation();
          const state = branchesSurface.ensureBranchListState(windowData.id);
          state.filter = button.dataset.branchFilter;
          branchesSurface.renderBranches(windowData.id);
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
    renderWorkspaceOverviewWindow(windowData.id);
  }

  function renderWindows() {
    for (const windowData of activeWorkspace()?.windows || []) {
      if (workspaceWindowById(windowData.id)?.preset !== "workspace") continue;
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

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
    const titleRow = createNode("span", "workspace-overview-row-title-row");
    titleRow.appendChild(createNode("span", "workspace-overview-row-title", item.title));
    // SPEC-2359 Phase W-12 (FR-351): each Work card surfaces its agent-session
    // lifecycle state (Active / Paused / Done / Discarded) as a dedicated badge
    // so the Work surface is the single home for Work lifecycle.
    const lifecycleBadge = createNode(
      "span",
      "workspace-overview-lifecycle",
      formatLifecycleStateLabel(item.lifecycle_state),
    );
    lifecycleBadge.dataset.lifecycle = String(item.lifecycle_state || "active").toLowerCase();
    titleRow.appendChild(lifecycleBadge);
    copy.appendChild(titleRow);
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
  function appendWorks(container, works) {
    const list = Array.isArray(works) ? works : [];
    if (list.length === 0) {
      container.appendChild(createNode("div", "workspace-overview-empty", "No Work yet"));
      return;
    }
    const wrap = createNode("div", "workspace-detail-work-list");
    for (const work of list) {
      const group = createNode("div", "workspace-detail-work-group");
      // Each Work is one Agent (a launch). The Agent header names the agent
      // (tool) and carries the per-Agent Resume control; the Work's Sessions
      // (its conversation history) are listed under it as sub-rows. The header
      // is always shown so two Sessions of one Work never look like two Agents.
      const head = createNode("div", "workspace-detail-work-head");
      head.appendChild(
        createNode(
          "div",
          "workspace-detail-work-heading",
          work.display_name || work.agent_id || "Agent",
        ),
      );
      const resumeBtn = renderWorkResumeButton(work);
      if (resumeBtn) {
        head.appendChild(resumeBtn);
      }
      group.appendChild(head);

      const sessions = Array.isArray(work.sessions) ? work.sessions : [];
      if (sessions.length === 0) {
        group.appendChild(
          createNode(
            "div",
            "workspace-overview-empty workspace-detail-session-empty",
            "No session yet",
          ),
        );
      } else {
        for (const session of sessions) {
          group.appendChild(renderSessionRow(work, session));
        }
      }
      wrap.appendChild(group);
    }
    container.appendChild(wrap);
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
    button.addEventListener("click", () => resumeWork(work));
    return button;
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
    // resume_workspace_agent resumes by the gwt session id (the Work / launch),
    // which is exactly work.session_id — so this Work can be resumed directly
    // without the Workspace-scoped picker.
    send({ kind: "resume_workspace_agent", session_id: sessionId, bounds });
  }

  function shortSessionId(value) {
    const text = String(value || "");
    return text.length > 8 ? text.slice(0, 8) : text;
  }

  function renderSessionRow(work, session) {
    const row = createNode("article", "workspace-detail-session");
    const active = Boolean(session && session.is_active);
    row.dataset.active = active ? "true" : "false";
    // The row is a Session (a conversation under the Agent), not the Agent
    // itself — the Agent (tool) is named once on the group header above.
    const sessionId = session && session.agent_session_id;
    const label = sessionId ? `Session ${shortSessionId(sessionId)}` : "Session";
    row.appendChild(createNode("div", "workspace-detail-session-name", label));
    const meta = createNode("div", "workspace-detail-session-meta");
    if (active) {
      appendMetaText(meta, "active");
    }
    appendMetaText(meta, session ? session.started_at : work.updated_at);
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
      const empty = createNode("div", "workspace-overview-empty", "No Workspace selected");
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
    // SPEC-2359: Resume is a per-Work (launch) operation, so the Resume control
    // lives on each Work row (see appendWorks / renderWorkResumeButton), not on
    // the Workspace header. The Workspace header keeps only Workspace-level
    // lifecycle actions (Done / Discard / Clean Up).
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
      detailSection("Work", (body) => {
        appendWorks(body, workspace.agents);
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

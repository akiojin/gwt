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
}) {
  const workspaceKanbanStateMap = new Map();

  function ensureWorkspaceKanbanState(windowId) {
    if (!workspaceKanbanStateMap.has(windowId)) {
      workspaceKanbanStateMap.set(windowId, {
        selectedId: null,
      });
    }
    return workspaceKanbanStateMap.get(windowId);
  }

  function workspaceColumnForCurrentStatus(statusCategory) {
    const state = String(statusCategory || "").toLowerCase();
    if (state === "active" || state === "blocked") {
      return "active";
    }
    if (state === "done" || state === "completed" || state === "closed") {
      return "completed";
    }
    return "inactive";
  }

  function workspaceColumnForJournalStatus(statusCategory) {
    const state = String(statusCategory || "").toLowerCase();
    if (state === "done" || state === "completed" || state === "closed") {
      return "completed";
    }
    return "inactive";
  }

  function ownerIssueNumber(owner) {
    const match = String(owner || "").match(/(?:issue\s*)?#(\d+)|issue\s+(\d+)/i);
    if (!match) return null;
    const number = Number.parseInt(match[1] || match[2], 10);
    return Number.isFinite(number) ? number : null;
  }

  function compactWorkspaceTitle(value) {
    const title = String(value || "").replace(/\s+/g, " ").trim();
    if (!title) return "";
    return title.length > 80 ? `${title.slice(0, 77)}...` : title;
  }

  function workspaceJournalCardTitle(entry) {
    for (const value of [
      entry.title,
      entry.agent_title_summary,
      entry.summary,
      entry.agent_current_focus,
      entry.status_text,
      entry.next_action,
    ]) {
      const title = compactWorkspaceTitle(value);
      if (title) return title;
    }
    return "Workspace update";
  }

  function workspaceCardsFromProjection(projection) {
    if (!projection) return [];
    const title = projection.title || `${activeWorkspace().title || "Project"} workspace`;
    const branch = projection.branch || "";
    const worktreePath = projection.worktree_path || "";
    const cards = [
      {
        id: projection.id || "__current_workspace__",
        kind: "current",
        title,
        status_category: projection.status_category || "idle",
        status_text: projection.status_text || "No active work",
        summary: projection.summary || projection.status_text || "",
        owner: projection.owner || "",
        next_action: projection.next_action || "",
        branch,
        worktree_path: worktreePath,
        pr_number: projection.pr_number || null,
        pr_url: projection.pr_url || "",
        pr_state: projection.pr_state || "",
        board_refs: Array.isArray(projection.board_refs) ? projection.board_refs : [],
        agents: Array.isArray(projection.agents) ? projection.agents : [],
        cleanup_candidate: projection.cleanup_candidate || null,
        updated_at: projection.updated_at || "",
        resume_source: "current",
        journal_id: null,
        column: workspaceColumnForCurrentStatus(projection.status_category),
      },
    ];

    const journalEntries = Array.isArray(projection.journal_entries)
      ? projection.journal_entries
      : [];
    for (const entry of journalEntries) {
      const statusCategory = entry.status_category || "idle";
      cards.push({
        id: `journal-${entry.id || cards.length}`,
        kind: "journal",
        title: workspaceJournalCardTitle(entry),
        status_category: statusCategory,
        status_text: entry.status_text || projection.status_text || "",
        summary:
          entry.summary ||
          entry.agent_title_summary ||
          entry.agent_current_focus ||
          entry.status_text ||
          entry.next_action ||
          "Workspace update",
        owner: entry.owner || projection.owner || "",
        next_action: entry.next_action || "",
        branch: "",
        worktree_path: "",
        pr_number: projection.pr_number || null,
        pr_url: projection.pr_url || "",
        pr_state: projection.pr_state || "",
        board_refs: [],
        agents: [],
        cleanup_candidate: null,
        updated_at: entry.updated_at || "",
        resume_source: "journal",
        journal_id: entry.id || "",
        column: workspaceColumnForJournalStatus(statusCategory),
      });
    }

    return cards;
  }

  function resumeWorkspaceCard(card) {
    if (card?.resume_source === "journal" && card?.journal_id) {
      send({
        kind: "resume_workspace",
        source: "journal",
        journal_id: card.journal_id,
      });
      return;
    }
    send({ kind: "resume_workspace", source: "current" });
  }

  function renderWorkspaceKanbanCard(windowId, state, cardData) {
    const card = createNode("article", "kanban-card workspace-kanban-card");
    card.dataset.workspaceCardId = cardData.id;
    if (state.selectedId === cardData.id) {
      card.classList.add("is-selected");
    }

    const select = () => {
      state.selectedId = cardData.id;
      renderWorkspaceKanban(windowId);
    };
    const selectButton = createNode("button", "workspace-card-main");
    selectButton.type = "button";
    selectButton.addEventListener("click", select);
    if (state.selectedId === cardData.id) {
      selectButton.setAttribute("aria-current", "true");
    }

    const head = createNode("div", "kanban-card-head");
    head.appendChild(
      createNode("span", "kanban-card-number", cardData.kind === "current" ? "Current" : "Update"),
    );
    head.appendChild(
      createNode(
        "span",
        `kanban-card-chip kanban-card-chip--state-${cardData.status_category}`,
        agentStatusLabel(cardData.status_category),
      ),
    );
    selectButton.appendChild(head);
    selectButton.appendChild(createNode("div", "kanban-card-title", cardData.title));
    if (cardData.summary) {
      selectButton.appendChild(createNode("div", "workspace-card-summary", cardData.summary));
    }

    const meta = createNode("div", "kanban-card-meta");
    appendMeta(meta, cardData.owner);
    appendMeta(meta, cardData.branch);
    const cardPr = createWorkspacePrMeta?.(cardData);
    if (cardPr) {
      meta.appendChild(cardPr);
    }
    appendMeta(meta, cardData.updated_at);
    if (meta.childElementCount > 0) {
      selectButton.appendChild(meta);
    }
    card.appendChild(selectButton);

    const actions = createNode("div", "workspace-card-actions");
    const resumeButton = createNode("button", "wizard-button primary", "Resume");
    resumeButton.type = "button";
    resumeButton.addEventListener("click", (event) => {
      event.stopPropagation();
      state.selectedId = cardData.id;
      renderWorkspaceKanban(windowId);
      resumeWorkspaceCard(cardData);
    });
    actions.appendChild(resumeButton);
    card.appendChild(actions);
    return card;
  }

  function renderWorkspaceKanbanDetail(detailPane, cardData) {
    detailPane.innerHTML = "";
    if (!cardData) {
      detailPane.appendChild(createNode("div", "knowledge-detail-empty", "Select a Workspace"));
      return;
    }

    const header = createNode("div", "knowledge-detail-header");
    const head = createNode("div", "");
    const headRow = createNode("div", "knowledge-detail-head");
    headRow.appendChild(createNode("h3", "knowledge-detail-title", cardData.title));
    headRow.appendChild(
      createNode(
        "span",
        `knowledge-state-chip ${cardData.status_category}`,
        agentStatusLabel(cardData.status_category),
      ),
    );
    head.appendChild(headRow);
    const subtitle = createNode("div", "knowledge-detail-subtitle");
    appendMeta(subtitle, cardData.owner);
    appendMeta(subtitle, cardData.branch);
    const detailPr = createWorkspacePrMeta?.(cardData);
    if (detailPr) {
      subtitle.appendChild(detailPr);
    }
    if (subtitle.childElementCount > 0) {
      head.appendChild(subtitle);
    }
    header.appendChild(head);

    const actions = createNode("div", "knowledge-detail-actions");
    const resumeButton = createNode("button", "wizard-button primary", "Resume");
    resumeButton.type = "button";
    resumeButton.addEventListener("click", () => resumeWorkspaceCard(cardData));
    actions.appendChild(resumeButton);
    if (cardData.cleanup_candidate?.branch) {
      const cleanupButton = createNode("button", "wizard-button", "Review Cleanup");
      cleanupButton.type = "button";
      cleanupButton.addEventListener("click", openWorkspaceCleanup);
      actions.appendChild(cleanupButton);
    }
    header.appendChild(actions);
    detailPane.appendChild(header);

    const scroll = createNode("div", "knowledge-detail-scroll workspace-scroll");
    const summary = createNode("section", "knowledge-section");
    summary.appendChild(createNode("div", "knowledge-section-title", "Summary"));
    summary.appendChild(
      createNode(
        "pre",
        "knowledge-section-body",
        cardData.summary || cardData.status_text || "No Workspace summary yet",
      ),
    );
    scroll.appendChild(summary);

    if (cardData.next_action) {
      const next = createNode("section", "knowledge-section");
      next.appendChild(createNode("div", "knowledge-section-title", "Next Action"));
      next.appendChild(createNode("pre", "knowledge-section-body", cardData.next_action));
      scroll.appendChild(next);
    }

    if (cardData.worktree_path) {
      const context = createNode("section", "knowledge-section");
      context.appendChild(createNode("div", "knowledge-section-title", "Workspace Context"));
      context.appendChild(createNode("pre", "knowledge-section-body", cardData.worktree_path));
      scroll.appendChild(context);
    }

    if (cardData.agents.length > 0) {
      const agents = createNode("section", "knowledge-section");
      agents.appendChild(createNode("div", "knowledge-section-title", "Agents"));
      agents.appendChild(
        createNode(
          "pre",
          "knowledge-section-body",
          cardData.agents
            .map((agent) =>
              [
                agent.display_name || agent.agent_id || "Agent",
                agentStatusLabel(agent.status_category),
                agent.current_focus || agent.title_summary || "",
              ].filter(Boolean).join(" · "),
            )
            .join("\n"),
        ),
      );
      scroll.appendChild(agents);
    }

    detailPane.appendChild(scroll);
  }

  function renderWorkspaceKanban(windowId) {
    const element = windowMap.get(windowId);
    if (!element) return;
    const state = ensureWorkspaceKanbanState(windowId);
    const board = element.querySelector(".workspace-kanban-board");
    const detailPane = element.querySelector(".workspace-kanban-detail-pane");
    const status = element.querySelector(".workspace-kanban-status");
    if (!board || !detailPane || !status) return;

    const cards = workspaceCardsFromProjection(getActiveWorkProjection());
    if (!state.selectedId || !cards.some((card) => card.id === state.selectedId)) {
      state.selectedId = cards[0]?.id || null;
    }

    const columnsByStatus = new Map();
    for (const column of board.querySelectorAll(".kanban-column[data-workspace-column]")) {
      const body = column.querySelector("[data-role='body']");
      if (body) body.innerHTML = "";
      columnsByStatus.set(column.dataset.workspaceColumn, column);
    }

    const counts = new Map();
    for (const cardData of cards) {
      const column = columnsByStatus.get(cardData.column) || columnsByStatus.get("inactive");
      const body = column?.querySelector("[data-role='body']");
      if (!body) continue;
      body.appendChild(renderWorkspaceKanbanCard(windowId, state, cardData));
      counts.set(cardData.column, (counts.get(cardData.column) || 0) + 1);
    }

    for (const [columnKey, column] of columnsByStatus) {
      const countLabel = column.querySelector("[data-role='count']");
      if (countLabel) {
        countLabel.textContent = String(counts.get(columnKey) || 0);
      }
      const body = column.querySelector("[data-role='body']");
      if (body && body.childElementCount === 0) {
        body.appendChild(createNode("div", "kanban-column-empty", "No Workspaces"));
      }
    }

    status.className = "knowledge-status workspace-kanban-status";
    status.textContent = cards.length === 0 ? "No Workspace history yet" : "";
    if (cards.length === 0) {
      status.classList.add("visible", "info");
    }

    const selected = cards.find((card) => card.id === state.selectedId) || null;
    renderWorkspaceKanbanDetail(detailPane, selected);
  }

  function mountWorkspaceKanban(body, windowData, { focusWindowLocally, sendFocus }) {
    body.innerHTML = `
      <div class="workspace-kanban-root kanban-root">
        <div class="workspace-toolbar kanban-toolbar is-stacked">
          <div class="workspace-toolbar-main">
            <div class="knowledge-heading">Workspace Overview</div>
          </div>
          <div class="workspace-toolbar-actions">
            <button class="wizard-button" type="button" data-action="start-workspace">Start Work</button>
          </div>
        </div>
        <div class="knowledge-status workspace-kanban-status"></div>
        <div class="knowledge-split workspace-split kanban-shell">
          <div class="knowledge-list-pane kanban-list-pane">
            <div class="kanban-board workspace-kanban-board" role="list" aria-label="Workspace Kanban Board">
              <div class="kanban-column workspace-kanban-column" data-workspace-column="active" aria-label="Active Workspace column">
                <div class="kanban-column-header">
                  <span class="workspace-column-name">Active</span>
                  <span class="kanban-column-count" data-role="count">0</span>
                </div>
                <div class="kanban-column-body" data-role="body"></div>
              </div>
              <div class="kanban-column workspace-kanban-column" data-workspace-column="inactive" aria-label="Inactive Workspace column">
                <div class="kanban-column-header">
                  <span class="workspace-column-name">Inactive</span>
                  <span class="kanban-column-count" data-role="count">0</span>
                </div>
                <div class="kanban-column-body" data-role="body"></div>
              </div>
              <div class="kanban-column workspace-kanban-column" data-workspace-column="completed" aria-label="Completed Workspace column">
                <div class="kanban-column-header">
                  <span class="workspace-column-name">Completed</span>
                  <span class="kanban-column-count" data-role="count">0</span>
                </div>
                <div class="kanban-column-body" data-role="body"></div>
              </div>
            </div>
          </div>
          <div class="knowledge-detail-pane workspace-kanban-detail-pane"></div>
        </div>
      </div>
    `;
    body.addEventListener("mousedown", () => {
      focusWindowLocally(windowData.id);
      sendFocus(windowData.id);
    });
    body
      .querySelector("[data-action='start-workspace']")
      .addEventListener("click", (event) => {
        event.stopPropagation();
        send({ kind: "open_start_work" });
      });
    renderWorkspaceKanban(windowData.id);
  }

  function renderWorkspaceKanbanWindows() {
    for (const [windowId] of windowMap.entries()) {
      if (workspaceWindowById(windowId)?.preset === "workspace") {
        renderWorkspaceKanban(windowId);
      }
    }
  }

  return Object.freeze({
    deleteState(windowId) {
      workspaceKanbanStateMap.delete(windowId);
    },
    mount: mountWorkspaceKanban,
    renderWindow: renderWorkspaceKanban,
    renderWindows: renderWorkspaceKanbanWindows,
  });
}

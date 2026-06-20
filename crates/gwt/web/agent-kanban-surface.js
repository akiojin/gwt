const AGENT_WINDOW_PRESETS = new Set(["agent", "claude", "codex"]);
const AGENT_KANBAN_DRAG_TYPE = "application/x-gwt-agent-window-id";

export const AGENT_KANBAN_LANES = Object.freeze([
  Object.freeze({ id: "plan", title: "Plan" }),
  Object.freeze({ id: "active", title: "Active" }),
  Object.freeze({ id: "blocked", title: "Blocked" }),
  Object.freeze({ id: "done", title: "Done" }),
]);

export function isAgentKanbanPlacement(windowData) {
  return windowData?.placement?.kind === "agent_kanban";
}

function isCanvasPlacement(windowData) {
  const kind = windowData?.placement?.kind;
  return !kind || kind === "canvas";
}

function isAgentWindow(windowData) {
  return AGENT_WINDOW_PRESETS.has(windowData?.preset);
}

export function isAgentKanbanEligible(windowData) {
  return Boolean(
    windowData?.id &&
      isAgentWindow(windowData) &&
      !windowData.tab_group_id &&
      isCanvasPlacement(windowData),
  );
}

export function windowsForAgentKanbanLane(windows, boardId, laneId) {
  return (windows || [])
    .filter((windowData) => {
      const placement = windowData?.placement || {};
      return (
        isAgentWindow(windowData) &&
        placement.kind === "agent_kanban" &&
        placement.board_id === boardId &&
        placement.lane_id === laneId
      );
    })
    .sort((left, right) => {
      const leftOrder = Number(left?.placement?.order ?? 0);
      const rightOrder = Number(right?.placement?.order ?? 0);
      if (leftOrder !== rightOrder) return leftOrder - rightOrder;
      return String(left?.id || "").localeCompare(String(right?.id || ""));
    });
}

export function placeAgentWindowMessage(id, boardId, laneId, order) {
  return {
    kind: "place_agent_window_in_kanban",
    id,
    board_id: boardId,
    lane_id: laneId,
    order,
  };
}

export function moveAgentKanbanCardMessage(id, boardId, laneId, order) {
  return {
    kind: "move_agent_kanban_card",
    id,
    board_id: boardId,
    lane_id: laneId,
    order,
  };
}

export function undockAgentWindowMessage(id, geometry) {
  return {
    kind: "undock_agent_window",
    id,
    geometry,
  };
}

export function setAgentKanbanCardCollapsedMessage(id, collapsed) {
  return {
    kind: "set_agent_kanban_card_collapsed",
    id,
    collapsed,
  };
}

export function updateTerminalGridMessage(id, cols, rows) {
  return {
    kind: "update_terminal_grid",
    id,
    cols,
    rows,
  };
}

export function findAgentKanbanDropTargetAtPoint(root, clientX, clientY) {
  if (!root || !Number.isFinite(clientX) || !Number.isFinite(clientY)) {
    return null;
  }
  const lanes = Array.from(
    root.querySelectorAll(".agent-kanban-lane[data-board-id][data-lane-id]"),
  );
  for (const lane of lanes) {
    const rect = lane.getBoundingClientRect();
    if (
      clientX < rect.left ||
      clientX > rect.right ||
      clientY < rect.top ||
      clientY > rect.bottom
    ) {
      continue;
    }
    const cards = Array.from(lane.querySelectorAll(".agent-kanban-card[data-window-id]"));
    let order = cards.length;
    for (let index = 0; index < cards.length; index += 1) {
      const cardRect = cards[index].getBoundingClientRect();
      const midpoint = cardRect.top + (cardRect.bottom - cardRect.top) / 2;
      if (clientY < midpoint) {
        order = index;
        break;
      }
    }
    return {
      boardId: lane.dataset.boardId,
      laneId: lane.dataset.laneId,
      order,
    };
  }
  return null;
}

export function createAgentKanbanPendingPlacementController({
  now = () => Date.now(),
  ttlMs = 10 * 60 * 1000,
} = {}) {
  let pending = null;

  function clear() {
    pending = null;
  }

  return {
    begin({ boardId, laneId, knownAgentWindowIds = new Set() }) {
      pending = {
        boardId,
        laneId,
        knownAgentWindowIds: new Set(knownAgentWindowIds),
        startedAt: now(),
      };
    },
    clear,
    consumePlacementMessage(windows) {
      if (!pending) return null;
      if (now() - pending.startedAt > ttlMs) {
        clear();
        return null;
      }
      const candidate = (windows || []).find(
        (windowData) =>
          isAgentKanbanEligible(windowData) &&
          !pending.knownAgentWindowIds.has(windowData.id),
      );
      if (!candidate) return null;
      const order = windowsForAgentKanbanLane(
        windows,
        pending.boardId,
        pending.laneId,
      ).length;
      const message = placeAgentWindowMessage(
        candidate.id,
        pending.boardId,
        pending.laneId,
        order,
      );
      clear();
      return message;
    },
  };
}

export function createAgentKanbanSurface({
  activeWorkspace,
  createTerminalRuntime,
  send,
  onLaunchAgent,
  visibleBounds,
  windowDisplayTitle,
  windowRoleBadgeLabel,
}) {
  function mount(body, boardWindow) {
    clearChildren(body);
    const document = body.ownerDocument;
    const boardId = boardWindow.id;
    const workspace = activeWorkspace?.() || { windows: [] };
    const windows = workspace.windows || [];

    const root = document.createElement("div");
    root.className = "agent-kanban-root";
    root.dataset.boardId = boardId;

    const toolbar = document.createElement("div");
    toolbar.className = "agent-kanban-toolbar";
    const heading = document.createElement("div");
    heading.className = "agent-kanban-heading";
    heading.textContent = "Agent Kanban";
    const status = document.createElement("div");
    status.className = "agent-kanban-status";
    status.textContent = `${containedWindowCount(windows, boardId)} Agents`;
    toolbar.append(heading, status);
    root.appendChild(toolbar);

    const lanes = document.createElement("div");
    lanes.className = "agent-kanban-lanes";
    for (const lane of AGENT_KANBAN_LANES) {
      lanes.appendChild(renderLane(document, boardId, lane, windows));
    }
    root.appendChild(lanes);
    body.appendChild(root);
  }

  function renderLane(document, boardId, lane, windows) {
    const laneElement = document.createElement("section");
    laneElement.className = "agent-kanban-lane";
    laneElement.dataset.boardId = boardId;
    laneElement.dataset.laneId = lane.id;
    laneElement.setAttribute("aria-label", `${lane.title} agents`);

    const cards = windowsForAgentKanbanLane(windows, boardId, lane.id);
    const header = document.createElement("div");
    header.className = "agent-kanban-lane-header";
    const title = document.createElement("div");
    title.className = "agent-kanban-lane-title";
    title.textContent = lane.title;
    const count = document.createElement("span");
    count.className = "agent-kanban-lane-count";
    count.textContent = String(cards.length);
    const launch = document.createElement("button");
    launch.type = "button";
    launch.className = "agent-kanban-lane-add";
    launch.dataset.action = "launch-agent";
    launch.textContent = "Launch Agent";
    launch.title = "Launch Agent";
    launch.setAttribute("aria-label", `Launch Agent in ${lane.title}`);
    launch.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      onLaunchAgent?.({ boardId, laneId: lane.id });
    });
    header.append(title, count, launch);
    laneElement.appendChild(header);

    const stack = document.createElement("div");
    stack.className = "agent-kanban-card-stack";
    stack.dataset.boardId = boardId;
    stack.dataset.laneId = lane.id;
    installLaneDropHandlers(stack, boardId, lane.id);

    if (cards.length === 0) {
      const empty = document.createElement("div");
      empty.className = "agent-kanban-empty";
      empty.textContent = "Drop agents here";
      stack.appendChild(empty);
    } else {
      for (const card of cards) {
        stack.appendChild(renderCard(document, boardId, card));
      }
    }

    laneElement.appendChild(stack);
    installLaneDropHandlers(laneElement, boardId, lane.id);
    return laneElement;
  }

  function renderCard(document, boardId, windowData) {
    const placement = windowData.placement || {};
    const collapsed = Boolean(placement.collapsed);
    const card = document.createElement("article");
    card.className = "agent-kanban-card";
    card.dataset.windowId = windowData.id;
    card.dataset.collapsed = String(collapsed);
    card.draggable = true;

    card.addEventListener("dragstart", (event) => {
      card.classList.add("dragging");
      event.dataTransfer?.setData(AGENT_KANBAN_DRAG_TYPE, windowData.id);
      event.dataTransfer?.setData("text/plain", windowData.id);
      if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
    });
    card.addEventListener("dragend", () => {
      card.classList.remove("dragging");
    });

    const header = document.createElement("div");
    header.className = "agent-kanban-card-header";
    const titleWrap = document.createElement("div");
    titleWrap.className = "agent-kanban-card-title-wrap";
    const title = document.createElement("div");
    title.className = "agent-kanban-card-title";
    title.textContent =
      windowDisplayTitle?.(windowData) || windowData.title || windowData.id;
    const meta = document.createElement("div");
    meta.className = "agent-kanban-card-meta";
    meta.textContent =
      windowRoleBadgeLabel?.(windowData) || windowData.agent_id || "Agent";
    titleWrap.append(title, meta);

    const actions = document.createElement("div");
    actions.className = "agent-kanban-card-actions";
    const collapse = document.createElement("button");
    collapse.type = "button";
    collapse.dataset.action = "collapse";
    collapse.className = "agent-kanban-card-action";
    collapse.textContent = collapsed ? "+" : "-";
    collapse.title = collapsed ? "Expand" : "Collapse";
    collapse.setAttribute("aria-label", collapsed ? "Expand card" : "Collapse card");
    collapse.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      send?.(setAgentKanbanCardCollapsedMessage(windowData.id, !collapsed));
    });

    const undock = document.createElement("button");
    undock.type = "button";
    undock.dataset.action = "undock";
    undock.className = "agent-kanban-card-action";
    undock.textContent = "↗";
    undock.title = "Undock";
    undock.setAttribute("aria-label", "Undock agent window");
    undock.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      send?.(undockAgentWindowMessage(windowData.id, undockGeometry(windowData)));
    });
    actions.append(collapse, undock);
    header.append(titleWrap, actions);
    card.appendChild(header);

    if (!collapsed) {
      const terminalShell = document.createElement("div");
      terminalShell.className = "agent-kanban-card-terminal";
      const terminalRoot = document.createElement("div");
      terminalRoot.className = "terminal-root";
      terminalRoot.addEventListener("mousedown", (event) => {
        event.stopPropagation();
      });
      terminalShell.appendChild(terminalRoot);
      card.appendChild(terminalShell);
      createTerminalRuntime?.(windowData.id, terminalRoot);
    }

    return card;
  }

  function installLaneDropHandlers(element, boardId, laneId) {
    element.addEventListener("dragover", (event) => {
      const id = dragWindowId(event);
      if (!id) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    });
    element.addEventListener("drop", (event) => {
      const id = dragWindowId(event);
      if (!id) return;
      event.preventDefault();
      event.stopPropagation();
      const target = findAgentKanbanDropTargetAtPoint(
        element.ownerDocument,
        event.clientX,
        event.clientY,
      ) || { boardId, laneId, order: laneCardCount(element) };
      sendDropMessage(id, target);
    });
  }

  function sendDropMessage(id, target) {
    const windowData = (activeWorkspace?.().windows || []).find(
      (candidate) => candidate.id === id,
    );
    if (isAgentKanbanPlacement(windowData)) {
      send?.(moveAgentKanbanCardMessage(id, target.boardId, target.laneId, target.order));
    } else if (isAgentKanbanEligible(windowData)) {
      send?.(placeAgentWindowMessage(id, target.boardId, target.laneId, target.order));
    }
  }

  function undockGeometry(windowData) {
    if (windowData?.geometry) {
      return {
        x: Number(windowData.geometry.x) || 0,
        y: Number(windowData.geometry.y) || 0,
        width: Number(windowData.geometry.width) || 720,
        height: Number(windowData.geometry.height) || 420,
      };
    }
    const bounds = visibleBounds?.() || { x: 0, y: 0 };
    return {
      x: (Number(bounds.x) || 0) + 32,
      y: (Number(bounds.y) || 0) + 32,
      width: 720,
      height: 420,
    };
  }

  return { mount };
}

function containedWindowCount(windows, boardId) {
  return (windows || []).filter(
    (windowData) =>
      isAgentWindow(windowData) &&
      windowData?.placement?.kind === "agent_kanban" &&
      windowData?.placement?.board_id === boardId,
  ).length;
}

function dragWindowId(event) {
  return (
    event.dataTransfer?.getData(AGENT_KANBAN_DRAG_TYPE) ||
    event.dataTransfer?.getData("text/plain") ||
    ""
  );
}

function laneCardCount(element) {
  return element.querySelectorAll(".agent-kanban-card[data-window-id]").length;
}

function clearChildren(node) {
  while (node.firstChild) {
    node.removeChild(node.firstChild);
  }
}

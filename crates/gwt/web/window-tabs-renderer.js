function ensureChild(parent, selector, create) {
  const existing = parent.querySelector(selector);
  if (existing) {
    return existing;
  }
  const child = create(parent.ownerDocument);
  parent.appendChild(child);
  return child;
}

function tabTitle(tab) {
  return String(tab?.title || "Window");
}

// SPEC-3038 US-2: tabs project the same agent telemetry the window chrome
// shows. Both fields are optional — non-agent tabs render without telemetry.
function tabAgentState(tab) {
  const state = typeof tab?.agent_state === "string" ? tab.agent_state.trim() : "";
  return state;
}

function tabAgentColor(tab) {
  const color = typeof tab?.agent_color === "string" ? tab.agent_color.trim() : "";
  return color;
}

function tabIdFromItem(item) {
  return item?.dataset?.windowTabId || "";
}

function itemActions(item) {
  return item.__windowTabActions || {};
}

function createTabButton(document, item) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "window-tab";
  button.draggable = true;

  // SPEC-3038 US-2: state dot + label span. The dot must precede the label so
  // keyed in-place updates never reorder children; the title lives in its own
  // span so updating it cannot wipe the dot.
  const dot = document.createElement("span");
  dot.className = "window-tab-state";
  dot.setAttribute("aria-hidden", "true");
  dot.hidden = true;
  button.appendChild(dot);
  const label = document.createElement("span");
  label.className = "window-tab-label";
  button.appendChild(label);

  button.addEventListener("click", (event) => {
    event.stopPropagation();
    const id = tabIdFromItem(item);
    itemActions(item).send?.({ kind: "activate_window_tab", id });
  });
  button.addEventListener("dragstart", (event) => {
    itemActions(item).onTabDragStart?.(event, tabIdFromItem(item));
  });
  button.addEventListener("drag", (event) => {
    itemActions(item).onTabDrag?.(event, tabIdFromItem(item));
  });
  button.addEventListener("dragend", (event) => {
    itemActions(item).onTabDragEnd?.(event, tabIdFromItem(item));
  });

  return button;
}

function createCloseButton(document, item) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "window-tab-close";
  button.textContent = "×";
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    const id = tabIdFromItem(item);
    itemActions(item).send?.({ kind: "close_window", id });
  });
  return button;
}

function createTabItem(document, actions) {
  const item = document.createElement("div");
  item.className = "window-tab-item";
  item.__windowTabActions = actions;
  item.appendChild(createTabButton(document, item));
  item.appendChild(createCloseButton(document, item));
  return item;
}

function updateTabItem(item, tab, { activeWindowId, tooltipForWindow, actions }) {
  const title = tabTitle(tab);
  const active = tab.id === activeWindowId || Boolean(tab.tab_group_active);
  item.__windowTabActions = actions;
  item.dataset.windowTabId = tab.id;

  const tabButton = ensureChild(item, ".window-tab", (document) =>
    createTabButton(document, item),
  );
  tabButton.type = "button";
  tabButton.className = "window-tab";
  tabButton.draggable = true;
  tabButton.dataset.windowTabId = tab.id;
  tabButton.setAttribute("aria-label", `Activate ${title}`);
  tabButton.classList.toggle("active", active);
  if (active) {
    tabButton.setAttribute("aria-current", "page");
  } else {
    tabButton.removeAttribute("aria-current");
  }

  // SPEC-3038 US-2: project agent telemetry onto the tab. The dataset
  // attributes drive the agent-color rim + state styling in app.css.
  const agentState = tabAgentState(tab);
  if (agentState) {
    tabButton.dataset.agentState = agentState;
  } else {
    delete tabButton.dataset.agentState;
  }
  const agentColor = tabAgentColor(tab);
  if (agentColor) {
    tabButton.dataset.agentColor = agentColor;
  } else {
    delete tabButton.dataset.agentColor;
  }

  const stateDot = ensureChild(tabButton, ".window-tab-state", (document) => {
    const dot = document.createElement("span");
    dot.className = "window-tab-state";
    dot.setAttribute("aria-hidden", "true");
    return dot;
  });
  stateDot.hidden = !agentState;

  const label = ensureChild(tabButton, ".window-tab-label", (document) => {
    const span = document.createElement("span");
    span.className = "window-tab-label";
    return span;
  });
  label.textContent = title;

  tabButton.title =
    typeof tooltipForWindow === "function" ? tooltipForWindow(tab) : title;

  const closeButton = ensureChild(item, ".window-tab-close", (document) =>
    createCloseButton(document, item),
  );
  closeButton.type = "button";
  closeButton.className = "window-tab-close";
  closeButton.setAttribute("aria-label", `Close ${title}`);
  closeButton.textContent = "×";
}

export function renderWindowTabs({
  strip,
  tabs,
  activeWindowId,
  tooltipForWindow,
  send,
  onTabDragStart,
  onTabDrag,
  onTabDragEnd,
}) {
  if (!strip) {
    return;
  }
  const document = strip.ownerDocument;
  const nextTabs = Array.isArray(tabs) ? tabs : [];
  const nextIds = new Set(nextTabs.map((tab) => tab.id));
  const actions = {
    send,
    onTabDragStart,
    onTabDrag,
    onTabDragEnd,
  };

  const existingItems = new Map();
  for (let index = strip.children.length - 1; index >= 0; index -= 1) {
    const item = strip.children[index];
    if (
      !item.classList?.contains("window-tab-item") ||
      !item.dataset?.windowTabId
    ) {
      continue;
    }
    const id = item.dataset.windowTabId;
    if (!nextIds.has(id)) {
      item.remove();
      continue;
    }
    existingItems.set(id, item);
  }

  for (let index = 0; index < nextTabs.length; index += 1) {
    const tab = nextTabs[index];
    let item = existingItems.get(tab.id);
    if (!item) {
      item = createTabItem(document, actions);
    }
    updateTabItem(item, tab, {
      activeWindowId,
      tooltipForWindow,
      actions,
    });

    const current = strip.children[index] || null;
    if (current !== item) {
      strip.insertBefore(item, current);
    }
  }
}

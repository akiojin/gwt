function ensureChild(parent, selector, create) {
  const existing = parent.querySelector(selector);
  if (existing) {
    return existing;
  }
  const child = create(parent.ownerDocument);
  parent.appendChild(child);
  return child;
}

function createTabButton(document, send, requestCloseProjectTab) {
  const button = document.createElement("div");
  button.className = "project-tab";
  button.setAttribute("role", "button");
  button.tabIndex = 0;

  const dot = document.createElement("span");
  dot.className = "project-tab-dot";
  dot.dataset.role = "project-tab-dot";
  dot.dataset.state = "";
  dot.setAttribute("aria-hidden", "true");
  button.appendChild(dot);

  const label = document.createElement("span");
  label.className = "project-tab-label";
  button.appendChild(label);

  const close = document.createElement("button");
  close.className = "project-tab-close";
  close.type = "button";
  close.textContent = "×";
  button.appendChild(close);

  button.addEventListener("click", () => {
    send({ kind: "select_project_tab", tab_id: button.dataset.projectTabId });
  });
  button.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    send({ kind: "select_project_tab", tab_id: button.dataset.projectTabId });
  });
  close.addEventListener("click", (event) => {
    event.stopPropagation();
    const tabId = button.dataset.projectTabId;
    // SPEC-2013 FR-012: agent 起動中の判定と modal 表示は app.js が
    // 所有する。renderer は単に request callback を呼ぶだけにし、
    // callback が未指定のときだけ legacy direct-send にフォールバックする。
    if (typeof requestCloseProjectTab === "function") {
      requestCloseProjectTab(tabId);
      return;
    }
    send({ kind: "close_project_tab", tab_id: tabId });
  });

  return button;
}

const AGENT_WINDOW_PRESETS = new Set(["agent", "claude", "codex"]);

const LEGACY_WINDOW_RUNTIME_STATE_ALIASES = Object.freeze({
  starting: "running",
  notstarted: "not_started",
  "not-started": "not_started",
  ready: "idle",
  exited: "stopped",
});

function normalizeProjectTabRuntimeState(status) {
  const rawState = String(status || "").toLowerCase();
  return LEGACY_WINDOW_RUNTIME_STATE_ALIASES[rawState] || rawState;
}

function windowIsAgentPane(windowData) {
  const preset = String(windowData?.preset || "").toLowerCase();
  return Boolean(windowData?.agent_id) || AGENT_WINDOW_PRESETS.has(preset);
}

function projectTabWindows(tab) {
  const windows = tab?.workspace?.windows;
  return Array.isArray(windows) ? windows : [];
}

export function projectTabAgentDotState(tab, { runtimeStateForWindow } = {}) {
  for (const windowData of projectTabWindows(tab)) {
    if (!windowIsAgentPane(windowData)) {
      continue;
    }
    const runtimeState =
      typeof runtimeStateForWindow === "function"
        ? runtimeStateForWindow(windowData)
        : windowData.status;
    if (normalizeProjectTabRuntimeState(runtimeState) === "running") {
      return "running";
    }
  }
  return "";
}

export function updateProjectTabDot(
  buttonEl,
  tab,
  { runtimeStateForWindow } = {},
) {
  const dot = buttonEl.querySelector("[data-role='project-tab-dot']");
  if (!dot) {
    return;
  }
  dot.dataset.state = projectTabAgentDotState(tab, { runtimeStateForWindow });
}

export function renderProjectTabs({
  projectTabs,
  tabs,
  activeTabId,
  runtimeStateForWindow,
  send,
  requestCloseProjectTab,
}) {
  if (!projectTabs) {
    return;
  }
  const document = projectTabs.ownerDocument;
  const nextTabs = Array.isArray(tabs) ? tabs : [];
  const nextIds = new Set(nextTabs.map((tab) => tab.id));

  for (const button of projectTabs.querySelectorAll(
    ".project-tab[data-project-tab-id]",
  )) {
    if (!nextIds.has(button.dataset.projectTabId)) {
      button.remove();
    }
  }

  const existingButtons = new Map(
    Array.from(
      projectTabs.querySelectorAll(".project-tab[data-project-tab-id]"),
    ).map((button) => [button.dataset.projectTabId, button]),
  );

  nextTabs.forEach((tab, index) => {
    let button = existingButtons.get(tab.id);
    if (!button) {
      button = createTabButton(document, send, requestCloseProjectTab);
    }

    const dot = ensureChild(button, "[data-role='project-tab-dot']", (doc) => {
      const element = doc.createElement("span");
      element.className = "project-tab-dot";
      element.dataset.role = "project-tab-dot";
      element.setAttribute("aria-hidden", "true");
      return element;
    });
    const label = ensureChild(button, ".project-tab-label", (doc) => {
      const element = doc.createElement("span");
      element.className = "project-tab-label";
      return element;
    });
    const close = ensureChild(button, ".project-tab-close", (doc) => {
      const element = doc.createElement("button");
      element.className = "project-tab-close";
      element.type = "button";
      element.textContent = "×";
      element.addEventListener("click", (event) => {
        event.stopPropagation();
        const tabId = button.dataset.projectTabId;
        if (typeof requestCloseProjectTab === "function") {
          requestCloseProjectTab(tabId);
          return;
        }
        send({ kind: "close_project_tab", tab_id: tabId });
      });
      return element;
    });

    button.dataset.projectTabId = tab.id;
    button.dataset.projectRoot = tab.project_root || "";
    button.title = tab.project_root || "";
    button.setAttribute("role", "button");
    button.tabIndex = 0;
    button.classList.toggle("active", tab.id === activeTabId);
    if (tab.id === activeTabId) {
      button.setAttribute("aria-current", "page");
    } else {
      button.removeAttribute("aria-current");
    }

    label.textContent = tab.title || "";
    close.setAttribute("aria-label", `Close ${tab.title || "project"}`);
    close.title = `Close ${tab.title || "project"}`;
    dot.dataset.state = dot.dataset.state || "";
    updateProjectTabDot(button, tab, { runtimeStateForWindow });

    const current = projectTabs.children[index] || null;
    if (current !== button) {
      projectTabs.insertBefore(button, current);
    }
  });
}

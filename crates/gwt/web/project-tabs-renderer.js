// SPEC-3015 — shared legacy runtime-state alias table (was a hand-written
// duplicate of the app.js copy before the extraction).
import { LEGACY_WINDOW_RUNTIME_STATE_ALIASES } from "./window-runtime-state.js";

function ensureChild(parent, selector, create) {
  const existing = parent.querySelector(selector);
  if (existing) {
    return existing;
  }
  const child = create(parent.ownerDocument);
  parent.appendChild(child);
  return child;
}

function createTabButton(
  document,
  send,
  requestCloseProjectTab,
  onSelectProjectTab,
) {
  const button = document.createElement("div");
  button.className = "project-tab";
  button.setAttribute("role", "button");
  button.tabIndex = 0;

  const cue = document.createElement("span");
  cue.className = "project-tab-state-cue";
  cue.dataset.role = "project-tab-state-cue";
  cue.dataset.state = "";
  cue.hidden = true;
  button.appendChild(cue);

  const label = document.createElement("span");
  label.className = "project-tab-label";
  button.appendChild(label);

  const close = document.createElement("button");
  close.className = "project-tab-close";
  close.type = "button";
  close.textContent = "×";
  button.appendChild(close);

  button.addEventListener("click", () => {
    const tabId = button.dataset.projectTabId;
    onSelectProjectTab?.(tabId);
    send({ kind: "select_project_tab", tab_id: tabId });
  });
  button.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    const tabId = button.dataset.projectTabId;
    onSelectProjectTab?.(tabId);
    send({ kind: "select_project_tab", tab_id: tabId });
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

const PROJECT_TAB_STATE_PRIORITY = ["block", "start", "run"];
const PROJECT_TAB_STATE_CONFIG = Object.freeze({
  block: { label: "BLOCK", aria: "blocked" },
  start: { label: "START", aria: "starting" },
  run: { label: "RUN", aria: "running" },
});

function projectTabStateForRuntimeState(runtimeState) {
  switch (normalizeProjectTabRuntimeState(runtimeState)) {
    case "error":
      return "block";
    case "starting":
      return "start";
    case "running":
      return "run";
    default:
      return "";
  }
}

export function projectTabAgentCueState(
  tab,
  { runtimeStateForWindow } = {},
) {
  const counts = { block: 0, start: 0, run: 0 };
  for (const windowData of projectTabWindows(tab)) {
    if (!windowIsAgentPane(windowData)) {
      continue;
    }
    const runtimeState =
      typeof runtimeStateForWindow === "function"
        ? runtimeStateForWindow(windowData)
        : windowData.status;
    const state = projectTabStateForRuntimeState(runtimeState);
    if (state) {
      counts[state] += 1;
    }
  }
  for (const state of PROJECT_TAB_STATE_PRIORITY) {
    const count = counts[state];
    if (count > 0) {
      const config = PROJECT_TAB_STATE_CONFIG[state];
      const noun = count === 1 ? "agent" : "agents";
      return {
        state,
        count,
        text: count === 1 ? config.label : `${config.label} ${count}`,
        ariaLabel: `${count} ${config.aria} ${noun}`,
      };
    }
  }
  return { state: "", count: 0, text: "", ariaLabel: "" };
}

export function projectTabAgentDotState(tab, options = {}) {
  return projectTabAgentCueState(tab, options).state;
}

export const projectTabAgentPillState = projectTabAgentCueState;

export function updateProjectTabStateCue(
  buttonEl,
  tab,
  { runtimeStateForWindow } = {},
) {
  const cue = buttonEl.querySelector("[data-role='project-tab-state-cue']");
  if (!cue) {
    return;
  }
  const state = projectTabAgentCueState(tab, { runtimeStateForWindow });
  cue.dataset.state = state.state;
  cue.textContent = state.text;
  cue.hidden = !state.state;
  if (state.state) {
    buttonEl.dataset.agentState = state.state;
  } else {
    delete buttonEl.dataset.agentState;
  }
  if (state.ariaLabel) {
    cue.setAttribute("aria-label", state.ariaLabel);
    cue.title = state.ariaLabel;
  } else {
    cue.removeAttribute("aria-label");
    cue.removeAttribute("title");
  }
}

export const updateProjectTabStatePill = updateProjectTabStateCue;
export const updateProjectTabDot = updateProjectTabStateCue;

export function renderProjectTabs({
  projectTabs,
  tabs,
  activeTabId,
  runtimeStateForWindow,
  send,
  requestCloseProjectTab,
  onSelectProjectTab,
}) {
  if (!projectTabs) {
    return;
  }
  const document = projectTabs.ownerDocument;
  const nextTabs = Array.isArray(tabs) ? tabs : [];
  const nextIds = new Set();
  for (const tab of nextTabs) {
    nextIds.add(tab.id);
  }

  const existingButtons = new Map();
  for (let index = projectTabs.children.length - 1; index >= 0; index -= 1) {
    const button = projectTabs.children[index];
    if (
      !button.classList?.contains("project-tab") ||
      !button.dataset?.projectTabId
    ) {
      continue;
    }
    const tabId = button.dataset.projectTabId;
    if (!nextIds.has(tabId)) {
      button.remove();
      continue;
    }
    existingButtons.set(tabId, button);
  }

  for (let index = 0; index < nextTabs.length; index += 1) {
    const tab = nextTabs[index];
    let button = existingButtons.get(tab.id);
    if (!button) {
      button = createTabButton(
        document,
        send,
        requestCloseProjectTab,
        onSelectProjectTab,
      );
    }

    button.querySelector("[data-role='project-tab-dot']")?.remove();
    button.querySelector("[data-role='project-tab-state-pill']")?.remove();
    const cue = ensureChild(
      button,
      "[data-role='project-tab-state-cue']",
      (doc) => {
        const element = doc.createElement("span");
        element.className = "project-tab-state-cue";
        element.dataset.role = "project-tab-state-cue";
        element.dataset.state = "";
        element.hidden = true;
        return element;
      },
    );
    const label = ensureChild(button, ".project-tab-label", (doc) => {
      const element = doc.createElement("span");
      element.className = "project-tab-label";
      return element;
    });
    if (cue.nextElementSibling !== label) {
      button.insertBefore(cue, label);
    }
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
    cue.dataset.state = cue.dataset.state || "";
    updateProjectTabStateCue(button, tab, { runtimeStateForWindow });

    const current = projectTabs.children[index] || null;
    if (current !== button) {
      projectTabs.insertBefore(button, current);
    }
  }
}

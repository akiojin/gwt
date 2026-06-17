import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import {
  buildProjectSwitcherRows,
  createProjectSwitcherController,
  nextProjectTabId,
  shouldHandleProjectSwitcherShortcut,
  shouldTriggerOpenFolderHotkey,
} from "../project-switcher.js";

function keyboardEvent(document, key, overrides = {}) {
  const event = new document.defaultView.Event("keydown", {
    bubbles: true,
    cancelable: true,
  });
  Object.assign(event, {
    key,
    metaKey: true,
    ctrlKey: false,
    shiftKey: true,
    altKey: false,
    repeat: false,
    target: document.body,
    ...overrides,
  });
  return event;
}

function makeTabs() {
  return [
    {
      id: "tab-1",
      title: "Repo One",
      project_root: "/repo/one",
      workspace: {
        windows: [
          { id: "agent-1", preset: "codex", status: "running" },
          { id: "shell-1", preset: "shell", status: "running" },
        ],
      },
    },
    {
      id: "tab-2",
      title: "Repo Two",
      project_root: "/repo/two",
      workspace: {
        windows: [{ id: "agent-2", preset: "claude", status: "idle" }],
      },
    },
  ];
}

test("nextProjectTabId cycles through open project tabs with arrow direction", () => {
  const tabs = makeTabs();

  assert.equal(nextProjectTabId(tabs, "tab-1", "next"), "tab-2");
  assert.equal(nextProjectTabId(tabs, "tab-2", "next"), "tab-1");
  assert.equal(nextProjectTabId(tabs, "tab-1", "previous"), "tab-2");
  assert.equal(nextProjectTabId([{ id: "only" }], "only", "next"), null);
  assert.equal(nextProjectTabId([], null, "next"), null);
});

test("project switcher shortcut accepts Shift+Cmd+Up/Down and Shift+Cmd+P without stealing editable input", () => {
  const { document } = parseHTML("<input id='field'><main></main>");
  const input = document.getElementById("field");

  assert.equal(
    shouldHandleProjectSwitcherShortcut(
      keyboardEvent(document, "ArrowDown"),
      { projectCount: 2 },
    ),
    true,
  );
  assert.equal(
    shouldHandleProjectSwitcherShortcut(
      keyboardEvent(document, "ArrowUp"),
      { projectCount: 2 },
    ),
    true,
  );
  assert.equal(
    shouldHandleProjectSwitcherShortcut(keyboardEvent(document, "P"), {
      projectCount: 0,
    }),
    true,
  );
  assert.equal(
    shouldHandleProjectSwitcherShortcut(
      keyboardEvent(document, "ArrowRight"),
      { projectCount: 2 },
    ),
    false,
  );
  assert.equal(
    shouldHandleProjectSwitcherShortcut(
      keyboardEvent(document, "ArrowDown", { repeat: true }),
      { projectCount: 2 },
    ),
    false,
  );
  assert.equal(
    shouldHandleProjectSwitcherShortcut(
      keyboardEvent(document, "ArrowDown", { target: input }),
      { projectCount: 2 },
    ),
    false,
  );
});

test("buildProjectSwitcherRows lists open projects first with runtime and unread metadata", () => {
  const rows = buildProjectSwitcherRows({
    tabs: makeTabs(),
    recentProjects: [
      { title: "Repo Three", kind: "git", path: "/repo/three" },
      { title: "Repo One", kind: "git", path: "/repo/one" },
    ],
    activeTabId: "tab-1",
    unreadProjectIds: new Set(["tab-2"]),
    runtimeStateForWindow: (windowData) => windowData.status,
  });

  assert.deepEqual(
    rows.map((row) => `${row.section}:${row.title}`),
    ["open:Repo One", "open:Repo Two", "recent:Repo Three"],
  );
  assert.equal(rows[0].active, true);
  assert.equal(rows[0].runningCount, 1);
  assert.equal(rows[1].unread, true);
  assert.equal(rows[2].path, "/repo/three");
});

test("project switcher controller renders rows, clears unread on project select, and only requests notification permission from a click", async () => {
  const { document } = parseHTML(`
    <button id="project-switcher-button"></button>
    <div id="project-switcher-panel"></div>
  `);
  const buttonEl = document.getElementById("project-switcher-button");
  const panelEl = document.getElementById("project-switcher-panel");
  const sends = [];
  const cleared = [];
  let requests = 0;
  const unreadProjectIds = new Set(["tab-2"]);
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };

  const controller = createProjectSwitcherController({
    buttonEl,
    panelEl,
    getState: () => ({
      tabs: makeTabs(),
      active_tab_id: "tab-1",
      recent_projects: [{ title: "Repo Three", kind: "git", path: "/repo/three" }],
    }),
    send: (payload) => sends.push(payload),
    createNode,
    unreadProjectIds,
    clearUnreadProject: (projectId) => cleared.push(projectId),
    runtimeStateForWindow: (windowData) => windowData.status,
    getNotificationPermission: () => "default",
    requestNotificationPermission: async () => {
      requests += 1;
      return "granted";
    },
  });

  controller.render();
  assert.equal(requests, 0, "desktop permission must not be requested on render");

  controller.open();
  assert.equal(buttonEl.getAttribute("aria-expanded"), "true");
  assert.match(panelEl.textContent, /Open Projects/);
  assert.match(panelEl.textContent, /Repo Two/);
  assert.match(panelEl.textContent, /New/);
  assert.match(panelEl.textContent, /Recent/);
  assert.match(panelEl.textContent, /Enable desktop notifications/);

  await panelEl
    .querySelector("[data-action='enable-desktop-notifications']")
    .dispatchEvent(new document.defaultView.Event("click", { bubbles: true }));
  assert.equal(requests, 1);

  panelEl
    .querySelector("[data-project-tab-id='tab-2']")
    .dispatchEvent(new document.defaultView.Event("click", { bubbles: true }));
  assert.deepEqual(sends, [{ kind: "select_project_tab", tab_id: "tab-2" }]);
  assert.deepEqual(cleared, ["tab-2"]);
});

// SPEC-2013 Phase 8 (US-9, FR-024): the consolidated panel ends with the
// import-action footer (Open Folder… / Clone from GitHub…). Each click invokes
// the matching callback passed into the controller.
test("project switcher controller renders Open Folder / Clone import actions and invokes the matching callback", () => {
  const { document } = parseHTML(`
    <button id="project-switcher-button"></button>
    <div id="project-switcher-panel"></div>
  `);
  const buttonEl = document.getElementById("project-switcher-button");
  const panelEl = document.getElementById("project-switcher-panel");
  let openFolderCalls = 0;
  let cloneCalls = 0;
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };

  const controller = createProjectSwitcherController({
    buttonEl,
    panelEl,
    getState: () => ({
      tabs: makeTabs(),
      active_tab_id: "tab-1",
      recent_projects: [{ title: "Repo Three", kind: "git", path: "/repo/three" }],
    }),
    send: () => {},
    createNode,
    runtimeStateForWindow: (windowData) => windowData.status,
    onOpenFolder: () => {
      openFolderCalls += 1;
    },
    onCloneFromGithub: () => {
      cloneCalls += 1;
    },
  });

  controller.open();

  const openFolderAction = panelEl.querySelector("[data-action='open-folder']");
  const cloneAction = panelEl.querySelector("[data-action='clone-from-github']");
  assert.ok(openFolderAction, "expected an Open Folder action button");
  assert.ok(cloneAction, "expected a Clone from GitHub action button");
  assert.match(openFolderAction.textContent, /Open Folder/);
  assert.match(cloneAction.textContent, /Clone from GitHub/);

  openFolderAction.dispatchEvent(
    new document.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(openFolderCalls, 1);
  assert.equal(buttonEl.getAttribute("aria-expanded"), "false");

  controller.open();
  panelEl
    .querySelector("[data-action='clone-from-github']")
    .dispatchEvent(new document.defaultView.Event("click", { bubbles: true }));
  assert.equal(cloneCalls, 1);
});

// SPEC-2013 Phase 8 (US-9 AS-2): the import actions are reachable even when no
// project tab or recent project exists ("No projects" empty state).
test("project switcher import actions render even in the No projects empty state", () => {
  const { document } = parseHTML(`
    <button id="project-switcher-button"></button>
    <div id="project-switcher-panel"></div>
  `);
  const buttonEl = document.getElementById("project-switcher-button");
  const panelEl = document.getElementById("project-switcher-panel");
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };

  const controller = createProjectSwitcherController({
    buttonEl,
    panelEl,
    getState: () => ({ tabs: [], active_tab_id: null, recent_projects: [] }),
    send: () => {},
    createNode,
    runtimeStateForWindow: () => "idle",
  });

  controller.open();
  assert.match(panelEl.textContent, /No projects/);
  assert.ok(
    panelEl.querySelector("[data-action='open-folder']"),
    "expected Open Folder action in the empty state",
  );
  assert.ok(
    panelEl.querySelector("[data-action='clone-from-github']"),
    "expected Clone from GitHub action in the empty state",
  );
});

// SPEC-2013 Phase 8 (US-10, FR-025): Cmd+O / Ctrl+O launches Open Folder in the
// normal state and follows the existing hotkey guards (no editable focus, no
// open modal, no key-repeat, no extra modifiers).
test("shouldTriggerOpenFolderHotkey honors the Cmd+O / Ctrl+O guards", () => {
  const { document } = parseHTML("<input id='field'><main></main>");
  const input = document.getElementById("field");

  function openFolderEvent(overrides = {}) {
    return {
      metaKey: true,
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      repeat: false,
      code: "KeyO",
      key: "o",
      target: document.body,
      ...overrides,
    };
  }

  assert.equal(shouldTriggerOpenFolderHotkey(openFolderEvent()), true);
  assert.equal(
    shouldTriggerOpenFolderHotkey(
      openFolderEvent({ metaKey: false, ctrlKey: true }),
    ),
    true,
  );
  // key-only fallback (some layouts report only event.key)
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ code: "", key: "O" })),
    true,
  );

  assert.equal(shouldTriggerOpenFolderHotkey(null), false);
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ repeat: true })),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ shiftKey: true })),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ altKey: true })),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent(), { modalOpen: true }),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ target: input })),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ metaKey: false, ctrlKey: false })),
    false,
  );
  assert.equal(
    shouldTriggerOpenFolderHotkey(openFolderEvent({ code: "KeyP", key: "p" })),
    false,
  );
});

// SPEC-2013 US-9 (PR #3102 review) — the panel-level Enter handler must select
// only when focus is on an OPEN / RECENT row. Action buttons (Open Folder /
// Clone) must keep their native Enter -> click activation instead of being
// hijacked into selectRow.
test("handlePanelKeydown selects on Enter for rows only, never for action buttons", () => {
  const { document } = parseHTML(`
    <button id="project-switcher-button"></button>
    <div id="project-switcher-panel"></div>
  `);
  const buttonEl = document.getElementById("project-switcher-button");
  const panelEl = document.getElementById("project-switcher-panel");
  const sends = [];
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
  const controller = createProjectSwitcherController({
    buttonEl,
    panelEl,
    getState: () => ({
      tabs: makeTabs(),
      active_tab_id: "tab-1",
      recent_projects: [],
    }),
    send: (payload) => sends.push(payload),
    createNode,
    runtimeStateForWindow: (windowData) => windowData.status,
  });
  controller.open();

  // Enter on an action button: not consumed, no preventDefault, no selection.
  const action = panelEl.querySelector("[data-action='open-folder']");
  let actionPrevented = false;
  const actionHandled = controller.handlePanelKeydown({
    key: "Enter",
    target: action,
    preventDefault() {
      actionPrevented = true;
    },
  });
  assert.equal(actionHandled, false, "panel must not consume Enter on an action button");
  assert.equal(actionPrevented, false, "panel must not preventDefault the button's native activation");
  assert.equal(sends.length, 0, "Enter on an action button must not select a project");

  // Enter on an OPEN row: consumed + selects the highlighted row.
  const row = panelEl.querySelector(".project-switcher-row");
  let rowPrevented = false;
  const rowHandled = controller.handlePanelKeydown({
    key: "Enter",
    target: row,
    preventDefault() {
      rowPrevented = true;
    },
  });
  assert.equal(rowHandled, true);
  assert.equal(rowPrevented, true);
  assert.deepEqual(sends, [{ kind: "select_project_tab", tab_id: "tab-1" }]);
});

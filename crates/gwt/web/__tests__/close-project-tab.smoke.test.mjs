// SPEC-2013 FR-012 smoke test: project tab close (`×`) で確認 modal が
// 表示される / されない場合と、modal 表示中の Cancel / Close anyway 経路の
// WebSocket dispatch を guard する。

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderProjectTabs } from "../project-tabs-renderer.js";
import { renderCloseProjectTabConfirmModal } from "../close-project-tab-confirm-modal.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtmlPath = resolve(here, "..", "index.html");
const indexHtml = readFileSync(indexHtmlPath, "utf8");

function mountTabs() {
  const { document } = parseHTML('<div id="project-tabs"></div>');
  return { document, projectTabs: document.getElementById("project-tabs") };
}

function dispatchClick(target) {
  target.dispatchEvent(
    new target.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );
}

function makeTab(overrides = {}) {
  return {
    id: "tab-1",
    title: "Repo One",
    project_root: "/repo/one",
    running_agent_count: 0,
    running_agents: [],
    ...overrides,
  };
}

test("close button routes through requestCloseProjectTab even when no agents are running", () => {
  const { projectTabs } = mountTabs();
  const sends = [];
  const requestCloseCalls = [];
  renderProjectTabs({
    projectTabs,
    tabs: [makeTab()],
    activeTabId: "tab-1",
    send: (payload) => sends.push(payload),
    requestCloseProjectTab: (tabId) => requestCloseCalls.push(tabId),
  });

  const closeBtn = projectTabs.querySelector(
    ".project-tab[data-project-tab-id='tab-1'] .project-tab-close",
  );
  assert.ok(closeBtn, "close button must exist");
  dispatchClick(closeBtn);

  assert.deepEqual(requestCloseCalls, ["tab-1"]);
  assert.deepEqual(
    sends,
    [],
    "renderer must defer to requestCloseProjectTab; direct send is the responsibility of app.js",
  );
});

test("close button routes through requestCloseProjectTab when agents are running", () => {
  const { projectTabs } = mountTabs();
  const sends = [];
  const requestCloseCalls = [];
  renderProjectTabs({
    projectTabs,
    tabs: [
      makeTab({
        running_agent_count: 2,
        running_agents: [
          { display_name: "claude", branch: "feature/a" },
          { display_name: "codex", branch: "feature/b" },
        ],
      }),
    ],
    activeTabId: "tab-1",
    send: (payload) => sends.push(payload),
    requestCloseProjectTab: (tabId) => requestCloseCalls.push(tabId),
  });

  const closeBtn = projectTabs.querySelector(
    ".project-tab[data-project-tab-id='tab-1'] .project-tab-close",
  );
  dispatchClick(closeBtn);
  assert.deepEqual(requestCloseCalls, ["tab-1"]);
  assert.deepEqual(sends, [], "no direct send must happen until app.js confirms");
});

test("close button falls back to direct send when requestCloseProjectTab is not provided", () => {
  const { projectTabs } = mountTabs();
  const sends = [];
  renderProjectTabs({
    projectTabs,
    tabs: [makeTab()],
    activeTabId: "tab-1",
    send: (payload) => sends.push(payload),
  });
  const closeBtn = projectTabs.querySelector(
    ".project-tab[data-project-tab-id='tab-1'] .project-tab-close",
  );
  dispatchClick(closeBtn);
  assert.deepEqual(sends, [{ kind: "close_project_tab", tab_id: "tab-1" }]);
});

function mountModal() {
  const { document } = parseHTML(indexHtml);
  const modalEl = document.getElementById("close-project-tab-modal");
  assert.ok(modalEl, "expected #close-project-tab-modal in index.html");
  const dialogEl = modalEl.querySelector(".modal-shell");
  assert.ok(dialogEl, "expected .modal-shell inside #close-project-tab-modal");
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
  return { document, modalEl, dialogEl, createNode };
}

test("close confirm modal hides itself when state.open is false", () => {
  const { modalEl, dialogEl, createNode } = mountModal();
  modalEl.classList.add("open");
  modalEl.setAttribute("aria-hidden", "false");
  dialogEl.appendChild(createNode("div", "stale"));

  renderCloseProjectTabConfirmModal({
    modalEl,
    dialogEl,
    state: { open: false, tabId: null, tabTitle: null, runningAgents: [] },
    createNode,
    onCancel: () => {},
    onConfirm: () => {},
  });

  assert.equal(modalEl.classList.contains("open"), false);
  assert.equal(modalEl.getAttribute("aria-hidden"), "true");
  assert.equal(dialogEl.children.length, 0, "stale content must be cleared on close");
});

test("close confirm modal renders agent count + names with Cancel and Close anyway", () => {
  const { modalEl, dialogEl, createNode } = mountModal();
  let cancelCalls = 0;
  let confirmCalls = 0;

  renderCloseProjectTabConfirmModal({
    modalEl,
    dialogEl,
    state: {
      open: true,
      tabId: "tab-1",
      tabTitle: "Repo One",
      runningAgents: [
        { display_name: "claude", branch: "feature/a" },
        { display_name: "codex", branch: "feature/b" },
      ],
    },
    createNode,
    onCancel: () => {
      cancelCalls += 1;
    },
    onConfirm: () => {
      confirmCalls += 1;
    },
  });

  assert.equal(modalEl.classList.contains("open"), true);
  assert.equal(modalEl.getAttribute("aria-hidden"), null);
  const body = dialogEl.textContent;
  assert.match(body, /Close project tab\?/);
  assert.match(body, /2 running agent\(s\) will be stopped/);
  assert.match(body, /claude/);
  assert.match(body, /codex/);
  assert.match(body, /feature\/a/);
  assert.match(body, /feature\/b/);

  const cancelButton = dialogEl.querySelector(
    "[data-role='close-project-tab-cancel']",
  );
  const confirmButton = dialogEl.querySelector(
    "[data-role='close-project-tab-confirm']",
  );
  assert.ok(cancelButton, "cancel button must exist");
  assert.ok(confirmButton, "confirm button must exist");
  assert.match(cancelButton.textContent, /Cancel/i);
  assert.match(confirmButton.textContent, /Close anyway/i);
  assert.ok(
    confirmButton.classList.contains("destructive"),
    "confirm button must carry a destructive style class",
  );

  cancelButton.dispatchEvent(
    new cancelButton.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(cancelCalls, 1);
  assert.equal(confirmCalls, 0);

  confirmButton.dispatchEvent(
    new confirmButton.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(confirmCalls, 1);
});

test("close confirm modal renders normal copy when no agents are running", () => {
  const { modalEl, dialogEl, createNode } = mountModal();
  let cancelCalls = 0;
  let confirmCalls = 0;

  renderCloseProjectTabConfirmModal({
    modalEl,
    dialogEl,
    state: {
      open: true,
      tabId: "tab-1",
      tabTitle: "Repo One",
      runningAgents: [],
    },
    createNode,
    onCancel: () => {
      cancelCalls += 1;
    },
    onConfirm: () => {
      confirmCalls += 1;
    },
  });

  assert.equal(modalEl.classList.contains("open"), true);
  const body = dialogEl.textContent;
  assert.match(body, /Close project tab\?/);
  assert.match(body, /Repo One/);
  assert.match(body, /You can reopen it from Recent projects/);
  assert.equal(
    dialogEl.querySelector(".close-project-tab-modal__agent-list"),
    null,
    "no-running confirmation must not render an empty agent list",
  );

  const cancelButton = dialogEl.querySelector(
    "[data-role='close-project-tab-cancel']",
  );
  const confirmButton = dialogEl.querySelector(
    "[data-role='close-project-tab-confirm']",
  );
  assert.match(cancelButton.textContent, /Cancel/i);
  assert.match(confirmButton.textContent, /Close tab/i);
  assert.equal(
    confirmButton.classList.contains("destructive"),
    false,
    "no-running confirmation should not use destructive emphasis",
  );

  cancelButton.dispatchEvent(
    new cancelButton.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(cancelCalls, 1);
  assert.equal(confirmCalls, 0);

  confirmButton.dispatchEvent(
    new confirmButton.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(confirmCalls, 1);
});

test("close confirm modal truncates agent list to 3 with 'and N more' suffix when 4+", () => {
  const { modalEl, dialogEl, createNode } = mountModal();
  const runningAgents = [
    { display_name: "claude-1", branch: "b1" },
    { display_name: "claude-2", branch: "b2" },
    { display_name: "codex-1", branch: "b3" },
    { display_name: "codex-2", branch: "b4" },
    { display_name: "claude-3", branch: "b5" },
  ];
  renderCloseProjectTabConfirmModal({
    modalEl,
    dialogEl,
    state: {
      open: true,
      tabId: "tab-1",
      tabTitle: "Repo",
      runningAgents,
    },
    createNode,
    onCancel: () => {},
    onConfirm: () => {},
  });

  const body = dialogEl.textContent;
  assert.match(body, /claude-1/);
  assert.match(body, /claude-2/);
  assert.match(body, /codex-1/);
  assert.doesNotMatch(body, /codex-2/, "4th agent name must not appear when truncated");
  assert.doesNotMatch(body, /claude-3/);
  assert.match(body, /and 2 more/);
  assert.match(body, /5 running agent\(s\) will be stopped/);
});

test("close confirm modal cancel handler fires for Escape and overlay click", () => {
  const { modalEl, dialogEl, createNode } = mountModal();
  let cancelCalls = 0;

  renderCloseProjectTabConfirmModal({
    modalEl,
    dialogEl,
    state: {
      open: true,
      tabId: "tab-1",
      tabTitle: "Repo",
      runningAgents: [{ display_name: "claude", branch: "feature/a" }],
    },
    createNode,
    onCancel: () => {
      cancelCalls += 1;
    },
    onConfirm: () => {},
  });

  // Overlay click — clicking the backdrop element itself (not the dialog
  // shell) must trigger cancel. linkedom dispatches events through the
  // composedPath, so we click `modalEl` directly while ensuring the event
  // target identity matches the backdrop. (linkedom doesn't provide a
  // MouseEvent constructor; the renderer only inspects `event.target`.)
  const View = modalEl.ownerDocument.defaultView;
  const overlayEvent = new View.Event("click", { bubbles: true });
  modalEl.dispatchEvent(overlayEvent);
  assert.equal(cancelCalls, 1, "overlay click on backdrop must cancel");

  // Escape key on the document also cancels.
  const escapeEvent = new View.Event("keydown", { bubbles: true });
  escapeEvent.key = "Escape";
  modalEl.ownerDocument.dispatchEvent(escapeEvent);
  assert.equal(cancelCalls, 2, "Escape keydown must cancel");
});

import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import { renderProjectTabs } from "../project-tabs-renderer.js";

function setupDom() {
  const { document } = parseHTML('<div id="project-tabs"></div>');
  return {
    projectTabs: document.getElementById("project-tabs"),
  };
}

function render(deps = {}) {
  const { projectTabs = setupDom().projectTabs, sends = [] } = deps;
  renderProjectTabs({
    projectTabs,
    tabs:
      deps.tabs ?? [
        { id: "tab-1", title: "Repo One", project_root: "/repo/one" },
        { id: "tab-2", title: "Repo Two", project_root: "/repo/two" },
      ],
    activeTabId: deps.activeTabId ?? "tab-1",
    runtimeStateForWindow: deps.runtimeStateForWindow,
    send: deps.send ?? ((payload) => sends.push(payload)),
  });
  return { projectTabs, sends };
}

test("renderProjectTabs preserves existing tab buttons across workspace refreshes", () => {
  const { projectTabs } = setupDom();

  render({ projectTabs });
  const firstButton = projectTabs.querySelector('[data-project-tab-id="tab-1"]');
  const secondButton = projectTabs.querySelector('[data-project-tab-id="tab-2"]');

  render({
    projectTabs,
    activeTabId: "tab-2",
    tabs: [
      { id: "tab-1", title: "Repo One Updated", project_root: "/repo/one" },
      { id: "tab-2", title: "Repo Two", project_root: "/repo/two" },
    ],
  });

  assert.equal(
    projectTabs.querySelector('[data-project-tab-id="tab-1"]'),
    firstButton,
    "existing tab node must be updated in place instead of recreated",
  );
  assert.equal(
    projectTabs.querySelector('[data-project-tab-id="tab-2"]'),
    secondButton,
    "inactive/active tab changes must not recreate sibling buttons",
  );
  assert.equal(
    firstButton.querySelector(".project-tab-label").textContent,
    "Repo One Updated",
  );
  assert.equal(secondButton.getAttribute("aria-current"), "page");
  assert.equal(firstButton.getAttribute("aria-current"), null);
});

test("renderProjectTabs keeps one click binding per stable tab node", () => {
  const { projectTabs } = setupDom();
  const sends = [];

  render({ projectTabs, sends });
  render({ projectTabs, sends });
  render({ projectTabs, sends });

  const firstButton = projectTabs.querySelector('[data-project-tab-id="tab-1"]');
  firstButton.dispatchEvent(
    new firstButton.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );

  assert.deepEqual(sends, [{ kind: "select_project_tab", tab_id: "tab-1" }]);
});

test("renderProjectTabs updates running-agent dots without rebuilding tab buttons", () => {
  const { projectTabs } = setupDom();
  const runtimeStates = new Map([["agent-1", "running"]]);
  const tabs = [
    {
      id: "tab-1",
      title: "Repo One",
      project_root: "/repo/one",
      workspace: {
        windows: [{ id: "agent-1", preset: "codex", status: "idle" }],
      },
    },
    { id: "tab-2", title: "Repo Two", project_root: "/repo/two" },
  ];

  render({
    projectTabs,
    tabs,
    runtimeStateForWindow: (windowData) =>
      runtimeStates.get(windowData.id) || windowData.status,
  });
  const firstButton = projectTabs.querySelector('[data-project-tab-id="tab-1"]');
  assert.equal(
    firstButton.querySelector("[data-role='project-tab-dot']").dataset.state,
    "running",
  );

  runtimeStates.set("agent-1", "idle");
  render({
    projectTabs,
    tabs,
    runtimeStateForWindow: (windowData) =>
      runtimeStates.get(windowData.id) || windowData.status,
  });

  assert.equal(
    projectTabs.querySelector('[data-project-tab-id="tab-1"]'),
    firstButton,
  );
  assert.equal(
    firstButton.querySelector("[data-role='project-tab-dot']").dataset.state,
    "",
  );
});

test("renderProjectTabs marks dot running only for running agent windows", () => {
  const { projectTabs } = setupDom();
  const tabs = [
    {
      id: "tab-running",
      title: "Running Agent",
      project_root: "/repo/running",
      workspace: {
        windows: [
          { id: "agent-running", preset: "codex", status: "idle" },
          { id: "shell-running", preset: "shell", status: "running" },
        ],
      },
    },
    {
      id: "tab-idle",
      title: "Idle Agent",
      project_root: "/repo/idle",
      workspace: {
        windows: [
          { id: "agent-idle", preset: "claude", status: "idle" },
          { id: "agent-waiting", preset: "agent", status: "waiting" },
          {
            id: "custom-stopped",
            preset: "shell",
            agent_id: "custom",
            status: "stopped",
          },
        ],
      },
    },
  ];
  const runtimeStates = new Map([
    ["agent-running", "running"],
    ["shell-running", "running"],
    ["agent-idle", "idle"],
    ["agent-waiting", "waiting"],
    ["custom-stopped", "stopped"],
  ]);

  render({
    projectTabs,
    tabs,
    activeTabId: "tab-running",
    runtimeStateForWindow: (windowData) =>
      runtimeStates.get(windowData.id) || windowData.status,
  });

  assert.equal(
    projectTabs
      .querySelector(
        "[data-project-tab-id='tab-running'] [data-role='project-tab-dot']",
      )
      .dataset.state,
    "running",
  );
  assert.equal(
    projectTabs
      .querySelector(
        "[data-project-tab-id='tab-idle'] [data-role='project-tab-dot']",
      )
      .dataset.state,
    "",
  );
});

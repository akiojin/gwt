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

function stateCue(button) {
  return button.querySelector("[data-role='project-tab-state-cue']");
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

test("renderProjectTabs updates agent state cues without rebuilding tab buttons", () => {
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
  assert.equal(firstButton.dataset.agentState, "run");
  assert.equal(stateCue(firstButton).dataset.state, "run");
  assert.equal(stateCue(firstButton).textContent, "RUN");
  assert.equal(stateCue(firstButton).hidden, false);

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
  assert.equal(firstButton.dataset.agentState, undefined);
  assert.equal(stateCue(firstButton).dataset.state, "");
  assert.equal(stateCue(firstButton).textContent, "");
  assert.equal(stateCue(firstButton).hidden, true);
});

test("renderProjectTabs shows project state cues only for active agent states", () => {
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
    {
      id: "tab-non-agent",
      title: "Non Agent",
      project_root: "/repo/non-agent",
      workspace: {
        windows: [{ id: "shell-running", preset: "shell", status: "running" }],
      },
    },
  ];
  const runtimeStates = new Map([
    ["agent-running", "running"],
    ["shell-running", "running"],
    ["agent-idle", "idle"],
    ["agent-waiting", "waiting"],
    ["custom-stopped", "stopped"],
    ["shell-running", "running"],
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
        "[data-project-tab-id='tab-running'] [data-role='project-tab-state-cue']",
      )
      .dataset.state,
    "run",
  );
  assert.equal(
    projectTabs.querySelector("[data-project-tab-id='tab-running']").dataset
      .agentState,
    "run",
  );
  assert.equal(
    projectTabs
      .querySelector(
        "[data-project-tab-id='tab-running'] [data-role='project-tab-state-cue']",
      )
      .textContent,
    "RUN",
  );
  assert.equal(
    projectTabs
      .querySelector(
        "[data-project-tab-id='tab-idle'] [data-role='project-tab-state-cue']",
      )
      .dataset.state,
    "",
  );
  assert.equal(
    projectTabs
      .querySelector(
        "[data-project-tab-id='tab-non-agent'] [data-role='project-tab-state-cue']",
      )
      .hidden,
    true,
  );
});

test("renderProjectTabs prioritizes BLOCK over START over RUN and counts the selected state", () => {
  const { projectTabs } = setupDom();
  const tabs = [
    {
      id: "tab-block",
      title: "Blocked Agents",
      project_root: "/repo/block",
      workspace: {
        windows: [
          { id: "agent-error-1", preset: "codex", status: "error" },
          { id: "agent-error-2", preset: "claude", status: "error" },
          { id: "agent-starting", preset: "agent", status: "starting" },
          { id: "agent-running", preset: "codex", status: "running" },
        ],
      },
    },
    {
      id: "tab-start",
      title: "Starting Agents",
      project_root: "/repo/start",
      workspace: {
        windows: [
          { id: "agent-not-started", preset: "codex", status: "not_started" },
          { id: "agent-running-2", preset: "claude", status: "running" },
        ],
      },
    },
    {
      id: "tab-run",
      title: "Running Agents",
      project_root: "/repo/run",
      workspace: {
        windows: [
          { id: "agent-running-3", preset: "codex", status: "running" },
          { id: "agent-running-4", preset: "claude", status: "running" },
        ],
      },
    },
  ];

  render({ projectTabs, tabs, activeTabId: "tab-block" });

  const blocked = projectTabs.querySelector(
    "[data-project-tab-id='tab-block'] [data-role='project-tab-state-cue']",
  );
  assert.equal(blocked.dataset.state, "block");
  assert.equal(blocked.textContent, "BLOCK 2");
  assert.equal(blocked.getAttribute("aria-label"), "2 blocked agents");
  assert.equal(
    projectTabs.querySelector("[data-project-tab-id='tab-block']").dataset
      .agentState,
    "block",
  );

  const starting = projectTabs.querySelector(
    "[data-project-tab-id='tab-start'] [data-role='project-tab-state-cue']",
  );
  assert.equal(starting.dataset.state, "start");
  assert.equal(starting.textContent, "START");
  assert.equal(
    starting.getAttribute("aria-label"),
    "1 starting agent",
    "legacy not_started must count as START and outrank RUN",
  );

  const running = projectTabs.querySelector(
    "[data-project-tab-id='tab-run'] [data-role='project-tab-state-cue']",
  );
  assert.equal(running.dataset.state, "run");
  assert.equal(running.textContent, "RUN 2");
  assert.equal(running.getAttribute("aria-label"), "2 running agents");
});

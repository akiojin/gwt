import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import {
  AGENT_KANBAN_LANES,
  createAgentKanbanPendingPlacementController,
  createAgentKanbanSurface,
  findAgentKanbanDropTargetAtPoint,
  isAgentKanbanEligible,
  isAgentKanbanPlacement,
  moveAgentKanbanCardMessage,
  placeAgentWindowMessage,
  setAgentKanbanCardCollapsedMessage,
  undockAgentWindowMessage,
  windowsForAgentKanbanLane,
} from "../agent-kanban-surface.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("Agent Kanban groups contained agent windows by stable lane order", () => {
  const windows = sampleWindows();

  assert.deepEqual(
    AGENT_KANBAN_LANES.map((lane) => lane.id),
    ["plan", "active", "blocked", "done"],
  );
  assert.equal(isAgentKanbanPlacement(windows[1]), true);
  assert.equal(isAgentKanbanEligible(windows[1]), false);
  assert.equal(isAgentKanbanEligible(windows[4]), true);
  assert.equal(isAgentKanbanEligible({ id: "shell-1", preset: "shell" }), false);
  assert.equal(
    isAgentKanbanEligible({
      id: "grouped",
      preset: "agent",
      tab_group_id: "tabs-1",
      tab_group_active: true,
    }),
    false,
  );

  assert.deepEqual(
    windowsForAgentKanbanLane(windows, "kanban-1", "active").map((windowData) => windowData.id),
    ["agent-2", "agent-1"],
  );
});

test("Agent Kanban surface renders lanes, card terminals, and card controls", () => {
  const fixture = createFixture();
  const workspace = { windows: sampleWindows() };
  const sent = [];
  const terminalMounts = [];
  const launchRequests = [];

  const surface = createAgentKanbanSurface({
    activeWorkspace: () => workspace,
    createTerminalRuntime: (id, root) => {
      terminalMounts.push({ id, root });
      return {};
    },
    send: (message) => sent.push(message),
    onLaunchAgent: (target) => launchRequests.push(target),
    windowDisplayTitle: (windowData) => windowData.dynamic_title || windowData.title || windowData.id,
    windowRoleBadgeLabel: (windowData) => windowData.agent_id || windowData.preset,
  });

  surface.mount(fixture.body, workspace.windows[0]);

  const lanes = Array.from(fixture.body.querySelectorAll(".agent-kanban-lane"));
  assert.deepEqual(
    lanes.map((lane) => lane.dataset.laneId),
    ["plan", "active", "blocked", "done"],
  );
  assert.match(lanes[0].textContent, /Plan/);
  assert.equal(fixture.body.querySelectorAll(".agent-kanban-card").length, 3);
  assert.deepEqual(
    terminalMounts.map((entry) => entry.id),
    ["agent-3", "agent-2", "agent-1"],
    "expanded cards should remount their terminal runtime into the Kanban card",
  );

  fixture.body
    .querySelector('.agent-kanban-card[data-window-id="agent-1"] [data-action="collapse"]')
    .click();
  fixture.body
    .querySelector('.agent-kanban-card[data-window-id="agent-2"] [data-action="undock"]')
    .click();
  const launchButton = fixture.body.querySelector(
    '.agent-kanban-lane[data-lane-id="active"] [data-action="launch-agent"]',
  );
  assert.ok(launchButton, "lane header must expose a Launch Agent action");
  assert.equal(launchButton.textContent.trim(), "Launch Agent");
  assert.equal(launchButton.title, "Launch Agent");
  assert.equal(launchButton.getAttribute("aria-label"), "Launch Agent in Active");
  launchButton.click();

  assert.deepEqual(sent[0], setAgentKanbanCardCollapsedMessage("agent-1", true));
  assert.equal(sent[1].kind, "undock_agent_window");
  assert.equal(sent[1].id, "agent-2");
  assert.equal(launchRequests[0].boardId, "kanban-1");
  assert.equal(launchRequests[0].laneId, "active");
});

test("Agent Kanban card messages preserve board, lane, order, and geometry", () => {
  assert.deepEqual(placeAgentWindowMessage("agent-1", "kanban-1", "blocked", 2), {
    kind: "place_agent_window_in_kanban",
    id: "agent-1",
    board_id: "kanban-1",
    lane_id: "blocked",
    order: 2,
  });

  assert.deepEqual(moveAgentKanbanCardMessage("agent-1", "kanban-1", "done", 0), {
    kind: "move_agent_kanban_card",
    id: "agent-1",
    board_id: "kanban-1",
    lane_id: "done",
    order: 0,
  });

  assert.deepEqual(undockAgentWindowMessage("agent-1", { x: 20, y: 30, width: 720, height: 420 }), {
    kind: "undock_agent_window",
    id: "agent-1",
    geometry: { x: 20, y: 30, width: 720, height: 420 },
  });
});

test("Agent Kanban drop target hit-test computes lane insertion order", () => {
  const fixture = createFixture();
  fixture.body.innerHTML = `
    <div class="agent-kanban-lane" data-board-id="kanban-1" data-lane-id="active">
      <div class="agent-kanban-card" data-window-id="agent-1"></div>
      <div class="agent-kanban-card" data-window-id="agent-2"></div>
    </div>
  `;
  const lane = fixture.body.querySelector(".agent-kanban-lane");
  const cards = fixture.body.querySelectorAll(".agent-kanban-card");
  lane.getBoundingClientRect = () => ({ left: 10, top: 10, right: 310, bottom: 610 });
  cards[0].getBoundingClientRect = () => ({ top: 100, bottom: 180 });
  cards[1].getBoundingClientRect = () => ({ top: 220, bottom: 300 });

  assert.deepEqual(findAgentKanbanDropTargetAtPoint(fixture.body, 30, 120), {
    boardId: "kanban-1",
    laneId: "active",
    order: 0,
  });
  assert.deepEqual(findAgentKanbanDropTargetAtPoint(fixture.body, 30, 210), {
    boardId: "kanban-1",
    laneId: "active",
    order: 1,
  });
  assert.deepEqual(findAgentKanbanDropTargetAtPoint(fixture.body, 30, 500), {
    boardId: "kanban-1",
    laneId: "active",
    order: 2,
  });
  assert.equal(findAgentKanbanDropTargetAtPoint(fixture.body, 400, 500), null);
});

test("Agent Kanban pending placement consumes only newly launched eligible agents", () => {
  const controller = createAgentKanbanPendingPlacementController({ now: () => 1000 });
  controller.begin({
    boardId: "kanban-1",
    laneId: "active",
    knownAgentWindowIds: new Set(["agent-existing"]),
  });

  assert.equal(
    controller.consumePlacementMessage([
      { id: "agent-existing", preset: "agent" },
      { id: "shell-new", preset: "shell" },
    ]),
    null,
  );
  assert.deepEqual(
    controller.consumePlacementMessage([
      { id: "agent-existing", preset: "agent" },
      { id: "agent-new", preset: "agent" },
    ]),
    placeAgentWindowMessage("agent-new", "kanban-1", "active", 0),
  );
  assert.equal(controller.consumePlacementMessage([{ id: "agent-later", preset: "agent" }]), null);
});

test("Agent Kanban app wiring maps preset, hides contained windows, reparents terminals, and sends grid updates", () => {
  assert.match(
    appSource,
    /from\s+"\/agent-kanban-surface\.js"/,
    "app.js must import the Agent Kanban surface module",
  );
  assert.match(
    extractFunctionBody(appSource, "presetSurface"),
    /preset\s*===\s*"agent_kanban"[\s\S]*return\s+"agent-kanban"/,
    "agent_kanban preset must mount as the Agent Kanban surface",
  );
  assert.match(
    extractFunctionBody(appSource, "visibleWindowData"),
    /isAgentKanbanPlacement\(windowData\)[\s\S]*return\s+false/,
    "contained Agent Kanban cards must not remain visible as top-level windows",
  );
  assert.match(
    extractFunctionBody(appSource, "createTerminalRuntime"),
    /return\s+reparentTerminalRuntime\(/,
    "existing terminal runtimes must reparent into card terminal roots",
  );
  assert.match(
    extractFunctionBody(appSource, "sendGeometry"),
    /isAgentKanbanPlacement\(windowData\)[\s\S]*updateTerminalGridMessage/,
    "contained terminal fits must update cols/rows without persisting hidden geometry",
  );
  assert.match(
    appSource,
    /agentKanbanDropTargetAt\(event,\s*dragState\.id\)[\s\S]*placeAgentWindowMessage\(/,
    "titlebar pointer release must place eligible agent windows into Kanban lanes",
  );
});

function createFixture() {
  const { document } = parseHTML("<body><main id=\"root\"></main></body>");
  return {
    document,
    body: document.getElementById("root"),
  };
}

function sampleWindows() {
  return [
    { id: "kanban-1", preset: "agent_kanban", title: "Agent Kanban" },
    {
      id: "agent-1",
      preset: "agent",
      title: "Implement UI",
      agent_id: "codex",
      placement: {
        kind: "agent_kanban",
        board_id: "kanban-1",
        lane_id: "active",
        order: 1,
        collapsed: false,
      },
    },
    {
      id: "agent-2",
      preset: "agent",
      title: "Fix tests",
      agent_id: "claude-code",
      placement: {
        kind: "agent_kanban",
        board_id: "kanban-1",
        lane_id: "active",
        order: 0,
        collapsed: false,
      },
    },
    {
      id: "agent-3",
      preset: "agent",
      title: "Plan follow-up",
      placement: {
        kind: "agent_kanban",
        board_id: "kanban-1",
        lane_id: "plan",
        order: 0,
        collapsed: false,
      },
    },
    { id: "agent-free", preset: "agent", title: "Unplaced" },
  ];
}

function extractFunctionBody(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `expected function ${name} in app.js`);
  const paramsOpen = source.indexOf("(", start);
  let parenDepth = 0;
  let paramsClose = -1;
  for (let i = paramsOpen; i < source.length; i += 1) {
    const char = source[i];
    if (char === "(") parenDepth += 1;
    if (char === ")") {
      parenDepth -= 1;
      if (parenDepth === 0) {
        paramsClose = i;
        break;
      }
    }
  }
  assert.notEqual(paramsClose, -1, `expected function ${name} parameters`);
  const open = source.indexOf("{", paramsClose);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(open + 1, i);
    }
  }
  assert.fail(`expected function ${name} body`);
}

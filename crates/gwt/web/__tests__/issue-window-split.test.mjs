// SPEC #3885 Phase 2b remainder (Issue #4082 AC-1) — T-018 / T-005 / T-019 / T-020.
//
// The Issue window has two view modes (FR-014): the list (default, one read-only
// status row per running agent) and the split view, which lays the running
// Issues out as "Issue detail + interactive terminal" pairs. Switching modes
// reparents the same terminal runtime, so the PTY, its scrollback and the
// selection survive. After Windowize the pair shows the "Shown on canvas" face
// instead of a second input face (US-4 / FR-003a), and a pair can be expanded
// or shrunk in place (T-005). Stopping an agent lives only in the row's ⋯ menu
// (FR-015), and the status row's elapsed time comes from the backend's agent
// start time when the backend sends one (T-020).

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const appCss = readFileSync(resolve(here, "../styles/app.css"), "utf8");
const tokensCss = readFileSync(resolve(here, "../styles/tokens.css"), "utf8");
const typographyCss = readFileSync(resolve(here, "../styles/typography.css"), "utf8");

async function importSurfaceModule() {
  const source = readFileSync(resolve(here, "../knowledge-kanban-surface.js"), "utf8")
    .replace(
      'from "/focus-trap.js"',
      'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
    )
    .replace(
      'from "./launch-pending-controller.js"',
      'from "data:text/javascript,export function createLaunchOperationId(){return%20%22split-test%22}"',
    );
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function knowledgeEntry(number, overrides = {}) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    meta: "",
    labels: [],
    linked_branch_count: 0,
    match_score: null,
    phase: null,
    has_unknown_phase: false,
    is_spec: false,
    monitor_state: "launched",
    queue_position: null,
    exclusion_reason: null,
    related_work_refs: [],
    ...overrides,
  };
}

function createNode(document, tagName, className, text) {
  const node = document.createElement(tagName);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function previewWindow(id, issueNumber, overrides = {}) {
  return {
    id,
    preset: "agent",
    title: `Agent ${id}`,
    agent_id: "codex",
    status: "running",
    linked_issue_number: issueNumber,
    placement: { kind: "issue_preview", issue_window_id: "win-1", issue_number: issueNumber },
    ...overrides,
  };
}

async function makeFixture(t, { workspaceWindows = [], projection = null } = {}) {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const issueWindow = { id: "win-1", preset: "issue" };
  const sent = [];
  const terminalMounts = [];
  const windowized = [];
  const focused = [];
  const stateSince = new Map();
  let windows = [issueWindow, ...workspaceWindows];
  const surface = mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (...args) => createNode(document, ...args),
    createKnowledgeMarkdownBody: () => document.createElement("div"),
    windowMap: new Map([[issueWindow.id, body]]),
    workspaceWindowById: (id) => windows.find((entry) => entry.id === id) || null,
    getWorkspaceWindows: () => windows,
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => "issue",
    focusWindowLocally: (id) => focused.push(id),
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
    createTerminalRuntime: (id, root, options) => {
      terminalMounts.push({ id, root, options });
      return { id };
    },
    windowDisplayTitle: (windowData) => windowData?.title || windowData?.id,
    windowRoleBadgeLabel: (windowData) => windowData?.agent_id || "Agent",
    windowizeIssuePreviewWindow: (id) => windowized.push(id),
    windowActivityDetail: (windowData) => windowData?.dynamic_title_detail || "",
    windowRuntimeStateSince: (id) => stateSince.get(id) ?? null,
    getActiveWorkProjection: () => projection,
  });
  surface.mountKnowledgeWindow(issueWindow, body);
  // The mounted surface keeps a 60s auto-refresh interval alive; clearing the
  // bridge state releases it so the test runner can exit.
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(Boolean(load), "Issue surface requests its cache-backed rows");
  return {
    body,
    document,
    load,
    mod,
    sent,
    surface,
    terminalMounts,
    windowized,
    focused,
    stateSince,
    setWindows: (next) => {
      windows = [issueWindow, ...next];
    },
  };
}

function applyEntries(surface, load, entries, selectedNumber) {
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries,
    selected_number: selectedNumber,
    empty_message: "",
    refresh_enabled: true,
  });
}

function click(node) {
  node.dispatchEvent(new node.ownerDocument.defaultView.Event("click", { bubbles: true }));
}

function viewButton(body, mode) {
  return body.querySelector(`[data-issue-view="${mode}"]`);
}

function pairs(body) {
  return [...body.querySelectorAll(".issue-split-pair")];
}

test("FR-014: the Issue window opens in list mode and offers a list / split switch", async (t) => {
  const fx = await makeFixture(t, {
    workspaceWindows: [previewWindow("agent-1", 3671), previewWindow("agent-2", 3672)],
  });
  applyEntries(fx.surface, fx.load, [knowledgeEntry(3671), knowledgeEntry(3672)], 3671);

  const root = fx.body.querySelector(".issue-bridge-root");
  assert.equal(root.dataset.viewMode, "list", "list is the default view mode (AC-14)");
  const group = fx.body.querySelector('[role="group"][aria-label="Issue view mode"]');
  assert.ok(Boolean(group), "the toolbar carries the view mode switch");
  assert.equal(viewButton(fx.body, "list").getAttribute("aria-pressed"), "true");
  assert.equal(viewButton(fx.body, "split").getAttribute("aria-pressed"), "false");
  assert.equal(fx.body.querySelectorAll(".issue-agent-status").length, 2);
  assert.equal(pairs(fx.body).length, 0, "no pairs in list mode");
});

test("T-018: split mode lays out one detail + interactive terminal pair per running Issue", async (t) => {
  const fx = await makeFixture(t, {
    workspaceWindows: [
      previewWindow("agent-1", 3671, { dynamic_title_detail: "Running cargo test" }),
      previewWindow("agent-2", 3672),
    ],
  });
  applyEntries(
    fx.surface,
    fx.load,
    [knowledgeEntry(3671), knowledgeEntry(3672), knowledgeEntry(3673)],
    3671,
  );
  fx.terminalMounts.length = 0;

  click(viewButton(fx.body, "split"));

  const root = fx.body.querySelector(".issue-bridge-root");
  assert.equal(root.dataset.viewMode, "split");
  assert.equal(viewButton(fx.body, "split").getAttribute("aria-pressed"), "true");
  const list = pairs(fx.body);
  assert.deepEqual(
    list.map((pair) => pair.dataset.issueNumber),
    ["3671", "3672"],
    "only Issues with a running agent become pairs (US-7)",
  );
  assert.deepEqual(
    list.map((pair) => pair.dataset.windowId),
    ["agent-1", "agent-2"],
  );
  assert.equal(fx.body.querySelectorAll(".issue-agent-status").length, 0);
  assert.equal(fx.body.querySelectorAll(".issue-preview").length, 0, "no second face");

  const first = list[0];
  assert.equal(first.querySelector(".knowledge-row-badge").textContent, "Running");
  assert.equal(first.querySelector(".knowledge-row-badge").dataset.tone, "active");
  assert.equal(first.querySelector(".issue-split-title").textContent, "Issue 3671");
  assert.equal(first.querySelector(".issue-split-number").textContent, "#3671");
  assert.equal(first.querySelector(".issue-split-output").textContent, "Running cargo test");
  assert.ok(Boolean(first.querySelector(".issue-split-terminal .terminal-root")));
  assert.ok(first.classList.contains("selected"), "the selection carries over");
  assert.equal(first.getAttribute("aria-current"), "true");
  assert.ok(!list[1].classList.contains("selected"));

  const mounts = fx.terminalMounts.filter((mount) => mount.options?.readOnly !== true);
  assert.deepEqual(
    mounts.map((mount) => mount.id),
    ["agent-1", "agent-2"],
    "each pair mounts its runtime as an interactive terminal",
  );
  assert.ok(
    fx.terminalMounts.every((mount) => mount.options?.readOnly === false),
    "split mode never mounts a read-only mirror",
  );

  click(list[1].querySelector(".issue-split-header"));
  const select = fx.sent.filter((m) => m.kind === "select_knowledge_bridge_entry").at(-1);
  assert.equal(select?.number, 3672, "clicking a pair header selects its Issue");
});

test("T-018: switching back to list restores the status rows and the read-only mirror on the same runtime", async (t) => {
  const fx = await makeFixture(t, {
    workspaceWindows: [previewWindow("agent-1", 3671), previewWindow("agent-2", 3672)],
  });
  applyEntries(fx.surface, fx.load, [knowledgeEntry(3671), knowledgeEntry(3672)], 3671);
  click(viewButton(fx.body, "split"));
  fx.terminalMounts.length = 0;
  const before = fx.sent.length;

  click(viewButton(fx.body, "list"));

  assert.equal(fx.body.querySelector(".issue-bridge-root").dataset.viewMode, "list");
  assert.equal(pairs(fx.body).length, 0);
  assert.equal(fx.body.querySelectorAll(".issue-agent-status").length, 2);
  const preview = fx.body.querySelector(".issue-preview");
  assert.equal(preview?.dataset.windowId, "agent-1", "the selected Issue's mirror is back");
  assert.equal(
    fx.body.querySelector(".knowledge-row.selected")?.dataset.issueNumber,
    "3671",
    "the selection survives the round trip",
  );
  const mirror = fx.terminalMounts.find((mount) => mount.id === "agent-1");
  assert.equal(mirror?.options?.readOnly, true, "the same runtime is reparented read-only");
  const lifecycle = fx.sent
    .slice(before)
    .filter((m) => ["close_window", "stop_window", "restart_window"].includes(m.kind));
  assert.deepEqual(lifecycle, [], "switching views never touches the PTY");
});

test("US-4 / FR-003a: a Windowized agent's pair shows the canvas face, not a second terminal", async (t) => {
  const projection = {
    active_works: [
      {
        id: "work-3671",
        branch: "work/issue-3671",
        agents: [{ window_id: "agent-1", session_id: "s-1" }],
      },
    ],
  };
  const fx = await makeFixture(t, {
    workspaceWindows: [
      previewWindow("agent-1", 3671, { placement: { kind: "canvas" } }),
      previewWindow("agent-2", 3672),
    ],
    projection,
  });
  applyEntries(
    fx.surface,
    fx.load,
    [
      knowledgeEntry(3671, {
        related_work_refs: [{ id: "work-3671", branch: "work/issue-3671", updated_at: "" }],
      }),
      knowledgeEntry(3672),
    ],
    3671,
  );
  fx.terminalMounts.length = 0;

  click(viewButton(fx.body, "split"));

  const list = pairs(fx.body);
  assert.deepEqual(list.map((pair) => pair.dataset.issueNumber), ["3671", "3672"]);
  const onCanvas = list[0];
  assert.ok(onCanvas.classList.contains("is-on-canvas"));
  assert.match(onCanvas.querySelector(".issue-split-placeholder").textContent, /Shown on canvas/);
  assert.equal(Boolean(onCanvas.querySelector(".terminal-root")), false, "no second input face");
  assert.ok(Boolean(onCanvas.querySelector('[data-action="focus-canvas-window"]')));
  assert.equal(Boolean(onCanvas.querySelector('[data-action="windowize-issue-preview"]')), false);
  assert.deepEqual(
    fx.terminalMounts.map((mount) => mount.id),
    ["agent-2"],
    "only the inline agent mounts a terminal",
  );

  click(onCanvas.querySelector('[data-action="focus-canvas-window"]'));
  assert.deepEqual(fx.focused, ["agent-1"]);

  // The inline pair keeps Windowize as its hand-off to the canvas.
  click(list[1].querySelector('[data-action="windowize-issue-preview"]'));
  assert.deepEqual(fx.windowized, ["agent-2"]);
});

test("T-005: a split pair expands and shrinks in place without remounting its terminal", async (t) => {
  const fx = await makeFixture(t, {
    workspaceWindows: [previewWindow("agent-1", 3671), previewWindow("agent-2", 3672)],
  });
  const entries = [knowledgeEntry(3671), knowledgeEntry(3672)];
  applyEntries(fx.surface, fx.load, entries, 3671);
  click(viewButton(fx.body, "split"));

  let first = pairs(fx.body)[0];
  assert.equal(first.dataset.size, "normal");
  const toggle = first.querySelector('[data-action="toggle-pair-size"]');
  assert.ok(Boolean(toggle), "each pair carries an expand / shrink control");
  assert.equal(toggle.getAttribute("aria-expanded"), "false");
  fx.terminalMounts.length = 0;

  click(toggle);
  first = pairs(fx.body)[0];
  assert.equal(first.dataset.size, "expanded");
  assert.equal(
    first.querySelector('[data-action="toggle-pair-size"]').getAttribute("aria-expanded"),
    "true",
  );
  assert.equal(pairs(fx.body)[1].dataset.size, "normal", "only the toggled pair grows");
  assert.ok(Boolean(first.querySelector(".issue-split-terminal .terminal-root")), "the terminal stays");

  // A data refresh keeps the size.
  applyEntries(fx.surface, fx.load, entries, 3671);
  assert.equal(pairs(fx.body)[0].dataset.size, "expanded");

  click(pairs(fx.body)[0].querySelector('[data-action="toggle-pair-size"]'));
  assert.equal(pairs(fx.body)[0].dataset.size, "normal");
});

test("FR-015: stopping an agent is offered only in the row's ⋯ menu", async (t) => {
  const fx = await makeFixture(t, {
    workspaceWindows: [
      previewWindow("agent-1", 3671),
      previewWindow("agent-2", 3672, { status: "stopped" }),
    ],
  });
  applyEntries(fx.surface, fx.load, [knowledgeEntry(3671), knowledgeEntry(3672)], 3671);

  const row = fx.body.querySelector('.knowledge-row[data-issue-number="3671"]');
  const stop = row.querySelector('.knowledge-row-menu [data-action="stop-agent"]');
  assert.ok(Boolean(stop), "the live agent's row menu offers Stop agent");
  assert.equal(stop.getAttribute("role"), "menuitem");
  assert.equal(Boolean(row.querySelector('.knowledge-row-actions > [data-action="stop-agent"]')), false,
    "Stop never becomes a visible row button",
  );
  assert.equal(Boolean(row.querySelector('.issue-agent-status [data-action="stop-agent"]')), false);

  click(stop);
  assert.deepEqual(
    fx.sent.filter((m) => m.kind === "stop_window"),
    [{ kind: "stop_window", id: "agent-1" }],
  );

  const stoppedRow = fx.body.querySelector('.knowledge-row[data-issue-number="3672"]');
  assert.equal(Boolean(stoppedRow.querySelector('[data-action="stop-agent"]')), false,
    "a stopped agent has nothing to stop",
  );

  // The same offer exists in split mode, in the pair's menu.
  click(viewButton(fx.body, "split"));
  const pair = pairs(fx.body)[0];
  assert.ok(Boolean(pair.querySelector('.knowledge-row-menu [data-action="stop-agent"]')));
});

test("T-020: the status row's elapsed time prefers the backend agent start time", async (t) => {
  const now = Date.now();
  const fx = await makeFixture(t, {
    workspaceWindows: [
      previewWindow("agent-1", 3671, { runtime_started_at_ms: now - 125 * 60_000 }),
      previewWindow("agent-2", 3672),
    ],
  });
  fx.stateSince.set("agent-1", now - 60_000);
  fx.stateSince.set("agent-2", now - 3 * 60_000);
  applyEntries(fx.surface, fx.load, [knowledgeEntry(3671), knowledgeEntry(3672)], 3671);

  const elapsedFor = (issue) =>
    fx.body.querySelector(`[data-issue-number="${issue}"] .issue-agent-status-elapsed`);
  assert.equal(elapsedFor(3671).textContent, "2h 05m", "backend start time wins");
  assert.match(elapsedFor(3671).title, /2h 05m/);
  assert.equal(
    elapsedFor(3672).textContent,
    "3m",
    "without a backend start time the observed state change remains the fallback",
  );

  click(viewButton(fx.body, "split"));
  assert.equal(
    pairs(fx.body)[0].querySelector(".issue-split-elapsed").textContent,
    "2h 05m",
    "the pair header shows the same elapsed time",
  );
});

test("AC-6: the split view CSS is Operator-token only", () => {
  const start = appCss.indexOf(".issue-split-pair");
  assert.ok(start >= 0, "app.css styles the split pair");
  // Declarations only: an Issue reference like "#3885" inside a comment is not a
  // color literal.
  const block = appCss
    .slice(start, appCss.indexOf(".issue-window-header", start))
    .replace(/\/\*[\s\S]*?\*\//g, "");
  assert.ok(block.length > 0);
  assert.doesNotMatch(block, /#[0-9a-f]{3,8}\b/i, "no raw hex colors");
  assert.doesNotMatch(block, /rgba?\(/, "no raw rgb colors");
  for (const token of block.match(/var\(--[a-z0-9-]+\)/g) || []) {
    const name = token.slice(4, -1);
    assert.ok(
      tokensCss.includes(`${name}:`) || typographyCss.includes(`${name}:`),
      `${name} is an Operator token`,
    );
  }
});

// Issue #3884 (SPEC-3671 follow-up, SPEC #3885 Phase 1) — the Issue row's inline
// terminal.
//
// Every Issue row whose agent runs as an `issue_preview` placement mounts that
// agent's terminal inline, whether or not the row is selected (AC-6). The terminal
// is interactive — it is the same shared runtime the canvas uses, so keystrokes
// reach the PTY — and one window id is only ever mounted in one container, which
// is what rules out double input after Windowize (AC-7). Windowize stays the only
// hand-off to the canvas, and an errored / waiting agent is badged, never opened.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

async function importSurfaceModule() {
  const source = readFileSync(
    resolve(here, "../knowledge-kanban-surface.js"),
    "utf8",
  ).replace(
    'from "/focus-trap.js"',
    'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
  ).replace(
    'from "./launch-pending-controller.js"',
    'from "data:text/javascript,export function createLaunchOperationId(){return%20%22preview-test%22}"',
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function knowledgeEntry(number) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    meta: "",
    labels: [],
    linked_branch_count: 0,
    related_work_count: 0,
    related_session_count: 0,
    match_score: null,
    phase: null,
    has_unknown_phase: false,
    is_spec: false,
    monitor_state: "launched",
    queue_position: null,
    exclusion_reason: null,
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
    placement: {
      kind: "issue_preview",
      issue_window_id: "win-1",
      issue_number: issueNumber,
    },
    ...overrides,
  };
}

async function makeFixture({ workspaceWindows = [] } = {}) {
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
    focusWindowLocally() {},
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
  });
  surface.mountKnowledgeWindow(issueWindow, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return {
    body,
    document,
    load,
    mod,
    sent,
    surface,
    terminalMounts,
    window,
    windowized,
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

test("issuePreviewWindowsForIssue binds previews to their host Issue window", async () => {
  const { issuePreviewWindowsForIssue } = await importSurfaceModule();
  const mine = previewWindow("agent-1", 3671);
  const otherIssue = previewWindow("agent-2", 3672);
  const otherHost = previewWindow("agent-3", 3671, {
    placement: {
      kind: "issue_preview",
      issue_window_id: "win-2",
      issue_number: 3671,
    },
  });
  const orphan = previewWindow("agent-4", 3671, {
    placement: {
      kind: "issue_preview",
      issue_window_id: "win-gone",
      issue_number: 3671,
    },
  });
  const canvasAgent = {
    id: "agent-5",
    preset: "agent",
    placement: { kind: "canvas" },
  };
  const windows = [
    { id: "win-1", preset: "issue" },
    { id: "win-2", preset: "issue" },
    mine,
    otherIssue,
    otherHost,
    orphan,
    canvasAgent,
  ];

  assert.deepEqual(
    issuePreviewWindowsForIssue(windows, "win-1", 3671).map((entry) => entry.id),
    ["agent-1", "agent-4"],
    "the host's own previews plus orphans whose host window is gone",
  );
  assert.deepEqual(
    issuePreviewWindowsForIssue(windows, "win-2", 3671).map((entry) => entry.id),
    ["agent-3", "agent-4"],
  );
  assert.deepEqual(issuePreviewWindowsForIssue(windows, "win-1", null), []);
  assert.deepEqual(issuePreviewWindowsForIssue(windows, "win-1", 9999), []);
});

// AC-6: the inline terminal is mounted per row and does not depend on selection.
test("every launched Issue row mounts its agent terminal inline without selection", async (t) => {
  const fixture = await makeFixture({
    workspaceWindows: [previewWindow("agent-1", 3671), previewWindow("agent-2", 3672)],
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(
    fixture.surface,
    fixture.load,
    [knowledgeEntry(3671), knowledgeEntry(3672), knowledgeEntry(3673)],
    null,
  );

  const inlineTerminals = [...fixture.body.querySelectorAll(".issue-inline-terminal")];
  assert.deepEqual(
    inlineTerminals.map((section) => section.dataset.windowId),
    ["agent-1", "agent-2"],
    "one inline terminal per running agent, none for the Issue without an agent",
  );
  for (const section of inlineTerminals) {
    const row = section.closest(".knowledge-row");
    assert.ok(row, "the terminal lives inside its Issue row");
    assert.equal(row.dataset.issueNumber, section.dataset.issueNumber);
    assert.equal(
      section.closest(".knowledge-row-select"),
      null,
      "the terminal is not nested inside the row's select button",
    );
  }
  assert.equal(
    fixture.body.querySelector(".knowledge-detail-pane .issue-inline-terminal"),
    null,
    "the detail pane no longer hosts a terminal",
  );
  assert.equal(fixture.body.querySelector(".issue-preview"), null, "the old mirror is gone");

  const first = inlineTerminals[0];
  assert.equal(first.querySelector(".issue-inline-terminal-title").textContent, "Agent agent-1");
  assert.equal(first.querySelector(".issue-inline-terminal-meta").textContent, "codex");
  assert.doesNotMatch(first.textContent, /preview/i, "UI copy never says preview");

  assert.deepEqual(
    fixture.terminalMounts.map((mount) => mount.id),
    ["agent-1", "agent-2"],
  );
  for (const mount of fixture.terminalMounts) {
    assert.equal(
      mount.options?.readOnly,
      undefined,
      "AC-6: the inline terminal mounts the shared runtime interactive, never read-only",
    );
  }
  assert.equal(
    fixture.terminalMounts[0].root,
    first.querySelector(".issue-inline-terminal-body .terminal-root"),
  );
});

// AC-6: selecting a row keeps every inline terminal mounted exactly once.
test("selection changes do not stack or drop inline terminals", async (t) => {
  const fixture = await makeFixture({
    workspaceWindows: [previewWindow("agent-1", 3671), previewWindow("agent-2", 3672)],
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671), knowledgeEntry(3672)], null);

  fixture.body.querySelector('[data-issue-number="3672"] .knowledge-row-select').click();

  const ids = [...fixture.body.querySelectorAll(".issue-inline-terminal")].map(
    (section) => section.dataset.windowId,
  );
  assert.deepEqual(ids, ["agent-1", "agent-2"]);
  assert.equal(
    fixture.sent.filter((message) => message.kind === "select_knowledge_bridge_entry").length,
    1,
    "clicking the row still selects the Issue",
  );
});

// AC-6: interacting with the terminal is not a row-selection gesture.
test("clicking inside the inline terminal does not select the Issue", async (t) => {
  const fixture = await makeFixture({ workspaceWindows: [previewWindow("agent-1", 3671)] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], null);

  fixture.body
    .querySelector(".issue-inline-terminal .terminal-root")
    .dispatchEvent(new fixture.window.Event("click", { bubbles: true }));

  assert.equal(
    fixture.sent.some((message) => message.kind === "select_knowledge_bridge_entry"),
    false,
  );
});

// FR-010 / AC-7: Windowize is the hand-off, and once the placement is `canvas` the
// row stops hosting the terminal so the window id is mounted in one place only.
test("Windowize hands the inline terminal to the canvas and the row releases it", async (t) => {
  const agent = previewWindow("agent-1", 3671);
  const fixture = await makeFixture({ workspaceWindows: [agent] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], null);

  const windowize = fixture.body.querySelector(
    '.issue-inline-terminal [data-action="windowize-inline-terminal"]',
  );
  assert.ok(windowize, "the inline terminal offers a Windowize control");
  assert.equal(windowize.textContent, "Windowize");
  windowize.click();
  assert.deepEqual(fixture.windowized, ["agent-1"]);

  fixture.setWindows([{ ...agent, placement: { kind: "canvas" } }]);
  fixture.surface.renderKnowledgeBridge("win-1");
  assert.equal(fixture.body.querySelector(".issue-inline-terminal"), null);
});

// FR-011.
test("an errored or waiting agent is badged, never auto-opened on the canvas", async (t) => {
  for (const [status, label, tone] of [
    ["error", "Error", "blocked"],
    ["waiting", "Needs input", "needs-input"],
  ]) {
    const fixture = await makeFixture({
      workspaceWindows: [previewWindow("agent-1", 3671, { status })],
    });
    t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
    applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], null);

    const badge = fixture.body.querySelector(".issue-inline-terminal .knowledge-monitor-chip");
    assert.equal(badge.textContent, label);
    assert.equal(badge.dataset.tone, tone);
    assert.equal(badge.dataset.status, status);
    assert.deepEqual(fixture.windowized, []);
    assert.equal(
      fixture.sent.some((message) => message.kind === "undock_agent_window"),
      false,
    );
  }
});

test("no inline terminal is rendered when the Issue has no auto-launched agent", async (t) => {
  const fixture = await makeFixture();
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], 3671);

  assert.equal(fixture.body.querySelector(".issue-inline-terminal"), null);
  assert.equal(fixture.terminalMounts.length, 0);
});

// AC-6 / AC-7 wiring contract: the Issue surface receives the same interactive
// runtime factory the canvas uses, app.js keeps no read-only mirror path, and
// Windowize reuses the undock transition.
test("app.js wires the interactive inline terminal and the Windowize handoff", () => {
  assert.match(
    appSource,
    /createTerminalRuntime:\s*\(id,\s*terminalRoot\)\s*=>\s*\n?\s*createTerminalRuntime\(id,\s*terminalRoot\)/,
    "the Issue surface receives the shared terminal runtime factory without options",
  );
  assert.match(
    appSource,
    /windowizeIssuePreviewWindow:\s*\(id\)\s*=>\s*\{[\s\S]*undockAgentWindowMessage\(/,
    "Windowize reuses the undock_agent_window transition",
  );
  assert.doesNotMatch(
    appSource,
    /readOnly/,
    "no read-only terminal path remains: the inline terminal is interactive (Issue #3884)",
  );
  assert.doesNotMatch(appSource, /disableStdin/);
});

// SPEC-3671 P3 — the Issue window's read-only agent preview pane.
//
// The pane mirrors the agent working on the selected Issue. It attaches no input
// path (FR-008), shows exactly one terminal at a time (FR-009), offers Windowize as
// its only control (FR-010), and never opens a canvas window when the agent errors
// or waits for a human ruling (FR-011).

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

// FR-007 / FR-008.
test("selecting an Issue mirrors its agent read-only in the right pane", async (t) => {
  const fixture = await makeFixture({ workspaceWindows: [previewWindow("agent-1", 3671)] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], 3671);

  const preview = fixture.body.querySelector(".issue-preview");
  assert.ok(preview, "the selected Issue's agent is mirrored in the detail pane");
  assert.equal(preview.dataset.windowId, "agent-1");
  assert.equal(preview.querySelector(".issue-preview-title").textContent, "Agent agent-1");
  assert.equal(preview.querySelector(".issue-preview-meta").textContent, "codex");

  assert.equal(fixture.terminalMounts.length, 1);
  assert.equal(fixture.terminalMounts[0].id, "agent-1");
  assert.deepEqual(
    fixture.terminalMounts[0].options,
    { readOnly: true },
    "FR-008: the mirror mounts read-only so no input path is attached",
  );
  assert.equal(
    fixture.terminalMounts[0].root,
    preview.querySelector(".issue-preview-terminal .terminal-root"),
  );
});

// FR-009.
test("only one agent terminal is mirrored at a time", async (t) => {
  const fixture = await makeFixture({
    workspaceWindows: [
      previewWindow("agent-1", 3671),
      previewWindow("agent-2", 3672),
      previewWindow("agent-3", 3673),
    ],
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(
    fixture.surface,
    fixture.load,
    [knowledgeEntry(3671), knowledgeEntry(3672), knowledgeEntry(3673)],
    3671,
  );

  assert.equal(fixture.body.querySelectorAll(".issue-preview").length, 1);
  assert.equal(fixture.body.querySelector(".issue-preview").dataset.windowId, "agent-1");

  fixture.body.querySelector('[data-issue-number="3672"]').click();

  assert.equal(
    fixture.body.querySelectorAll(".issue-preview").length,
    1,
    "switching selection replaces the mirror instead of stacking terminals",
  );
  assert.equal(fixture.body.querySelector(".issue-preview").dataset.windowId, "agent-2");
  assert.deepEqual(
    fixture.terminalMounts.map((mount) => mount.id),
    ["agent-1", "agent-2"],
  );
});

// FR-010.
test("Windowize hands the mirrored agent back to the canvas", async (t) => {
  const fixture = await makeFixture({ workspaceWindows: [previewWindow("agent-1", 3671)] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], 3671);

  const windowize = fixture.body.querySelector('[data-action="windowize-issue-preview"]');
  assert.ok(windowize, "the mirror offers a Windowize control");
  windowize.click();

  assert.deepEqual(fixture.windowized, ["agent-1"]);
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
    applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], 3671);

    const badge = fixture.body.querySelector(".issue-preview .knowledge-monitor-chip");
    assert.equal(badge.textContent, label);
    assert.equal(badge.dataset.tone, tone);
    assert.equal(badge.dataset.status, status);
    assert.deepEqual(
      fixture.windowized,
      [],
      "an unhealthy agent must not be pushed onto the canvas on its own",
    );
    assert.equal(
      fixture.sent.some((message) => message.kind === "undock_agent_window"),
      false,
    );
  }
});

test("no preview pane is rendered when the Issue has no auto-launched agent", async (t) => {
  const fixture = await makeFixture();
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671)], 3671);

  assert.equal(fixture.body.querySelector(".issue-preview"), null);
  assert.equal(fixture.terminalMounts.length, 0);
});

// FR-008 wiring contract: the shared terminal runtime must honour readOnly on every
// input path, and app.js must pass it through from the Issue surface.
test("app.js wires the read-only mirror and the Windowize handoff", () => {
  assert.match(
    appSource,
    /createTerminalRuntime:\s*\(id,\s*terminalRoot,\s*options\)\s*=>\s*\n?\s*createTerminalRuntime\(id,\s*terminalRoot,\s*options\)/,
    "the Issue surface receives the shared terminal runtime factory",
  );
  assert.match(
    appSource,
    /windowizeIssuePreviewWindow:\s*\(id\)\s*=>\s*\{[\s\S]*undockAgentWindowMessage\(/,
    "Windowize reuses the undock_agent_window transition",
  );

  const createRuntime = extractFunctionBody(appSource, "createTerminalRuntime");
  assert.match(
    createRuntime,
    /terminalMap\.get\(windowId\)\?\.readOnly === true[\s\S]*return;/,
    "onData must drop input for a read-only mirror",
  );
  assert.match(createRuntime, /\{ readOnly: options\.readOnly === true \}/);

  const bindings = extractFunctionBody(appSource, "attachTerminalContainerBindings");
  for (const installer of [
    "installTerminalImagePasteHandlers",
    "installTerminalFileDropHandlers",
    "installTerminalContextMenuHandlers",
  ]) {
    assert.match(
      bindings,
      new RegExp(`readOnly\\s*\\n?\\s*\\?\\s*noopCleanup\\s*\\n?\\s*:\\s*${installer}`),
      `${installer} must not be attached for a read-only mirror`,
    );
  }
  assert.match(
    bindings,
    /if \(readOnly \|\| terminalMap\.get\(windowId\)\?\.isReady !== true\)/,
    "wheel-driven PTY writes must be suppressed for a read-only mirror",
  );
});

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

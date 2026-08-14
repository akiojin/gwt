// SPEC #3214 Phase 15 — the cache-backed Issue surface is the only
// Issue Monitor presenter. Rows consume KnowledgeListItem projections; raw
// IssueMonitorInboxItem payloads never enter this surface.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));

async function importSurfaceModule() {
  const source = readFileSync(
    resolve(here, "../knowledge-kanban-surface.js"),
    "utf8",
  ).replace(
    'from "/focus-trap.js"',
    'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
  ).replace(
    'from "./launch-pending-controller.js"',
    'from "data:text/javascript,export function createLaunchOperationId(){return%20%22resume-test%22}"',
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function knowledgeEntry(number, monitorState, queuePosition = null, options = {}) {
  return {
    number,
    title: `Issue ${number}`,
    state: options.state || "open",
    meta: "",
    labels: options.isSpec ? ["gwt-spec"] : ["bug"],
    linked_branch_count: 0,
    related_work_count: 0,
    related_session_count: 0,
    match_score: null,
    phase: null,
    has_unknown_phase: false,
    is_spec: Boolean(options.isSpec),
    monitor_state: monitorState,
    queue_position: queuePosition,
    exclusion_reason: options.exclusionReason || null,
  };
}

function createNode(document, tagName, className, text) {
  const node = document.createElement(tagName);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

async function makeFixture() {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const windowData = { id: "win-1", preset: "issue" };
  const sent = [];
  const surface = mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (...args) => createNode(document, ...args),
    createKnowledgeMarkdownBody: () => document.createElement("div"),
    windowMap: new Map([[windowData.id, body]]),
    workspaceWindowById: (id) => (id === windowData.id ? windowData : null),
    getWorkspaceWindows: () => [windowData],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => "issue",
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
  });
  surface.mountKnowledgeWindow(windowData, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return { body, document, mod, sent, surface, load };
}

test("monitor state renderer is exhaustive and never aliases an unknown state to Queued", async () => {
  const { monitorStateView } = await importSurfaceModule();
  const expected = new Map([
    ["queued", ["Queued", "idle"]],
    ["not_ready", ["Not ready", "needs-input"]],
    ["hold_excluded", ["On hold", "needs-input"]],
    ["launching", ["Launching", "active"]],
    ["launched", ["Launched", "active"]],
    ["merged", ["Merged", "done"]],
    ["released", ["Released", "done"]],
    ["launch_failed", ["Launch failed", "blocked"]],
    ["agent_failed", ["Agent failed", "blocked"]],
    ["blocked_by_claim", ["Blocked by claim", "needs-input"]],
    ["skipped", ["Skipped", "idle"]],
    ["needs_human", ["Needs human", "needs-input"]],
  ]);

  for (const [state, [label, tone]] of expected) {
    assert.deepEqual(monitorStateView(state), { state, label, tone });
  }
  assert.deepEqual(monitorStateView("awaiting_review"), {
    state: "awaiting_review",
    label: "Unknown (awaiting_review)",
    tone: "needs-input",
  });
  assert.equal(monitorStateView(null), null);
  assert.equal(monitorStateView(""), null);
});

test("Issue rows render monitor projections and send controls from the full canonical queue", async (t) => {
  const { body, document, sent, surface, load } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries: [
      knowledgeEntry(42, "queued", 1, { isSpec: true }),
      knowledgeEntry(43, "launching"),
      knowledgeEntry(44, "queued", 2),
      knowledgeEntry(45, "hold_excluded", null, {
        exclusionReason: "Excluded by label: hold",
      }),
      knowledgeEntry(46, "queued", 3, { state: "closed" }),
      knowledgeEntry(47, "needs_human"),
      knowledgeEntry(48, "awaiting_review"),
      knowledgeEntry(49, null),
    ],
    selected_number: 42,
    empty_message: "",
    refresh_enabled: true,
  });
  surface.applyIssueMonitorStatus({
    enabled: false,
    state: "disabled",
    queue_len: 3,
    active_count: 1,
    max_active_agents: 2,
    total_candidates: 8,
    autonomous_mode: false,
    launch_profile_source: "last_settings",
    launch_profile_summary: "codex / host",
  });

  const row42 = body.querySelector('[data-issue-number="42"]');
  const row43 = body.querySelector('[data-issue-number="43"]');
  const row44 = body.querySelector('[data-issue-number="44"]');
  const row45 = body.querySelector('[data-issue-number="45"]');
  const row48 = body.querySelector('[data-issue-number="48"]');
  const row49 = body.querySelector('[data-issue-number="49"]');
  assert.equal(row42.tagName, "DIV", "row shell is not an interactive element");
  assert.ok(row42.querySelector(":scope > .knowledge-row-select"));
  assert.ok(row42.querySelector(":scope > .knowledge-row-actions"));
  assert.equal(row42.querySelector("button button"), null, "no nested interactive controls");
  assert.equal(row42.querySelector(".knowledge-monitor-chip").textContent, "Queued");
  assert.match(row42.textContent, /Queue 1/);
  assert.equal(row45.querySelector(".knowledge-monitor-chip").textContent, "On hold");
  assert.match(row45.textContent, /Excluded by label: hold/);
  assert.equal(row48.querySelector(".knowledge-monitor-chip").textContent, "Unknown (awaiting_review)");
  assert.equal(row48.querySelector(".knowledge-monitor-chip").dataset.tone, "needs-input");
  assert.equal(row49.querySelector(".knowledge-monitor-chip"), null);

  row43.click();
  assert.deepEqual(sent.at(-1), {
    kind: "select_knowledge_bridge_entry",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: 2,
    number: 43,
  });

  row42.querySelector('[data-action="launch-now"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_launch_now",
    issue_number: 42,
    linked_issue_kind: "spec",
  });

  row44.querySelector('[data-action="move-up"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "reorder_issue_monitor_issues",
    issue_numbers: [44, 42, 46],
  });

  const maxActive = body.querySelector(".knowledge-monitor-max-active input");
  maxActive.value = "4";
  maxActive.dispatchEvent(new window.Event("change", { bubbles: true }));
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_max_active_agents",
    max_active_agents: 4,
  });

  body.querySelector('[data-action="monitor-toggle"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_enabled",
    enabled: true,
  });
  body.querySelector('[data-action="monitor-autonomous"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_autonomous_mode",
    enabled: true,
  });
  body.querySelector('[data-action="monitor-settings"]').click();
  assert.deepEqual(sent.at(-1), { kind: "issue_monitor_configure_profile" });

  const quickTitle = body.querySelector(".knowledge-monitor-quick-title");
  quickTitle.value = "Investigate flaky release gate";
  body.querySelector('[data-action="quick-register-launch"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "quick_register_issue",
    title: "Investigate flaky release gate",
    launch: true,
  });

  assert.match(
    body.querySelector(".knowledge-monitor-summary").textContent,
    /Stopped.*Queue 3.*Active 1\/2/,
  );
  assert.equal(body.querySelector('[data-action="monitor-toggle"]').textContent, "Start");
  assert.equal(
    body.querySelector('[data-action="monitor-autonomous"]').textContent,
    "Autonomous: OFF",
  );
  assert.equal(document.querySelector(".issue-monitor-card"), null);
});

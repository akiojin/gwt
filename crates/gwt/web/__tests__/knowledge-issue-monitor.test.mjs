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

async function makeFixture(options = {}) {
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
    ...options,
  });
  surface.mountKnowledgeWindow(windowData, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return { body, document, mod, sent, surface, load };
}

// SPEC #3206 FR-017 — surface errors are reported to the notification center
// and the surface shows one compact indicator line instead of a red band.
function errorSpies() {
  const reported = [];
  const resolved = [];
  return {
    reported,
    resolved,
    options: {
      reportSurfaceError: (error) => reported.push(error),
      resolveSurfaceError: (key) => resolved.push(key),
    },
  };
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

test("Issue Monitor panel presents and clears the quota-hold provider and reset", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const summary = body.querySelector(".knowledge-monitor-summary");

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "idle",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    launch_profile_source: "saved",
    launch_profile_summary: "configured",
    quota_hold: {
      provider: "codex",
      reset_at: "2026-09-04T09:30:00Z",
    },
  });

  const quotaHoldText = summary.textContent;

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "idle",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    launch_profile_source: "saved",
    launch_profile_summary: "configured",
  });

  assert.equal(summary.textContent, "Idle | Queue 3 | Active 0/2");
  assert.doesNotMatch(
    summary.textContent,
    /Quota hold|Provider codex|Reset 2026-09-04T09:30:00Z/i,
  );
  assert.match(quotaHoldText, /Quota hold/i);
  assert.match(quotaHoldText, /Provider codex/i);
  assert.match(quotaHoldText, /Reset 2026-09-04T09:30:00Z/i);
});

test("Issue Monitor panel preserves higher-priority states around quota-hold metadata", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const summary = body.querySelector(".knowledge-monitor-summary");
  const quotaHold = {
    provider: "codex",
    reset_at: "2026-09-04T09:30:00Z",
  };

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "error",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    last_error: "issue #3785: failed",
    quota_hold: quotaHold,
  });

  assert.equal(summary.textContent, "Error | Queue 3 | Active 0/2");
  // FR-017: the red monitor banner is gone and nothing replaces it in the
  // surface — the error is read in the notification center.
  assert.equal(body.querySelector(".knowledge-monitor-error"), null);
  assert.equal(body.querySelector(".surface-error-indicator"), null);
  assert.doesNotMatch(summary.textContent, /Quota hold|Provider|Reset/);

  surface.applyIssueMonitorStatus({
    enabled: false,
    state: "disabled",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    quota_hold: quotaHold,
  });

  assert.equal(summary.textContent, "Stopped | Queue 3 | Active 0/2");
  assert.doesNotMatch(summary.textContent, /Quota hold|Provider|Reset/);

  for (const state of ["active", "launching"]) {
    surface.applyIssueMonitorStatus({
      enabled: true,
      state,
      queue_len: 3,
      active_count: 1,
      max_active_agents: 2,
      quota_hold: quotaHold,
    });

    assert.equal(
      summary.textContent,
      "Quota hold | Queue 3 | Active 1/2 | Provider codex | Reset 2026-09-04T09:30:00Z",
    );
  }

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "launching",
    queue_len: 3,
    active_count: 1,
    max_active_agents: 2,
    quota_hold: {},
  });

  assert.equal(summary.textContent, "Launching | Queue 3 | Active 1/2");
  assert.doesNotMatch(summary.textContent, /Quota hold|Provider|Reset|undefined/);
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
  // SPEC #3885 T-004: the Monitor state is the row's single primary badge.
  assert.equal(row42.querySelector(".knowledge-row-badge").textContent, "Queued");
  assert.equal(row42.querySelectorAll(".knowledge-row-badge").length, 1);
  assert.match(row42.textContent, /Queue 1/);
  assert.equal(row45.querySelector(".knowledge-row-badge").textContent, "On hold");
  assert.match(row45.textContent, /Excluded by label: hold/);
  assert.equal(row48.querySelector(".knowledge-row-badge").textContent, "Unknown (awaiting_review)");
  assert.equal(row48.querySelector(".knowledge-row-badge").dataset.tone, "needs-input");
  assert.equal(row49.querySelector(".knowledge-row-badge").textContent, "Open");
  assert.equal(row49.querySelector(".knowledge-row-badge").dataset.stateKey, "issue:open");

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

  // Queue reordering lives in the row's overflow menu (SPEC #3885 AC-5).
  const moveUp = row44.querySelector('.knowledge-row-menu [data-action="move-up"]');
  assert.ok(moveUp, "Move up is reachable from the overflow menu");
  moveUp.click();
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

// --- SPEC #3206 FR-017: errors are read in ONE place (notification center) ---
// User ruling 2026-09-04: the Issue window shows no error surface of its own.
// It reports every error to the notification center and renders nothing.

test("FR-017: Issue Monitor last_error is reported to the center and nothing renders in the window", async (t) => {
  const spies = errorSpies();
  const { body, surface } = await makeFixture(spies.options);
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  assert.equal(body.querySelector(".knowledge-monitor-error"), null, "no red banner");
  assert.equal(body.querySelector(".surface-error-indicator"), null, "no compact indicator either");

  surface.applyIssueMonitorStatus({ enabled: true, state: "error", queue_len: 0, active_count: 0, max_active_agents: 1, last_error: "issue #3785: scan failed" });
  assert.deepEqual(spies.reported, [
    { key: "issue-monitor:last_error", title: "Issue Monitor", message: "issue #3785: scan failed" },
  ]);
  assert.equal(body.querySelector(".surface-error-indicator"), null, "still nothing in the surface");

  // issue_monitor_status is re-broadcast constantly — an unchanged error is
  // not a new occurrence.
  surface.applyIssueMonitorStatus({ enabled: true, state: "error", queue_len: 0, active_count: 0, max_active_agents: 1, last_error: "issue #3785: scan failed" });
  assert.equal(spies.reported.length, 1);

  surface.applyIssueMonitorStatus({ enabled: true, state: "idle", queue_len: 0, active_count: 0, max_active_agents: 1, last_error: null });
  assert.deepEqual(spies.resolved, ["issue-monitor:last_error"], "recovery resolves the center row");
});

test("FR-017: Issue window load errors report to the center without a red status band", async (t) => {
  const spies = errorSpies();
  const { body, surface, load } = await makeFixture(spies.options);
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const status = body.querySelector(".knowledge-status");

  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_error",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    message: "gh issue list: github_rate_limited (resets 09:30Z)",
  });
  assert.equal(status.classList.contains("error"), false, "no red band");
  assert.equal(status.textContent, "", "and no error text in the surface");
  assert.deepEqual(spies.reported, [
    { key: "issue-window:win-1:load", title: "Issue window", message: "gh issue list: github_rate_limited (resets 09:30Z)" },
  ]);

  // A successful reload resolves the window's error automatically.
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries: [knowledgeEntry(42, "queued", 1)],
    selected_number: 42,
    empty_message: "",
    refresh_enabled: true,
  });
  assert.ok(spies.resolved.includes("issue-window:win-1:load"));
});

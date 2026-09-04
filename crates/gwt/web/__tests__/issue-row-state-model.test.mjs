// SPEC #3885 Phase 2 (T-004 / T-005 / T-006) — the Issue row state model.
//
// Every Issue row carries exactly one primary state badge, at most two pieces of
// secondary information, and at most two visible action buttons that depend on
// that state. Anything else moves into the row's overflow menu. The row's agent
// status row carries no second badge, and once the agent is Windowized the same
// slot shows a "Shown on canvas" face instead of a second input face.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";
import {
  attentionForWorkspace,
  formatLifecycleStateLabel,
} from "../workspace-kanban-surface.js";

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
      'from "data:text/javascript,export function createLaunchOperationId(){return%20%22row-test%22}"',
    );
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function knowledgeEntry(number, overrides = {}) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    meta: "",
    labels: ["bug"],
    linked_branch_count: 0,
    related_work_count: 0,
    related_session_count: 0,
    match_score: null,
    phase: null,
    has_unknown_phase: false,
    is_spec: false,
    monitor_state: null,
    queue_position: null,
    exclusion_reason: null,
    related_work_refs: [],
    ...overrides,
  };
}

function workRow(overrides = {}) {
  return {
    id: "work-3671",
    title: "Issue window as the primary surface",
    status_category: "active",
    status_text: "Implementing P4",
    lifecycle_state: "active",
    active_agents: 1,
    blocked_agents: 0,
    branch: "work/issue-3671",
    worktree_path: "/gwt/work/issue-3671",
    pr_number: 3699,
    pr_url: "https://github.com/akiojin/gwt/pull/3699",
    pr_state: "open",
    cleanup_candidate: null,
    cleanup_blocked_reason: null,
    agents: [],
    works: [],
    ...overrides,
  };
}

function agentWindow(id, issueNumber, overrides = {}) {
  return {
    id,
    preset: "agent",
    title: `Agent ${id}`,
    agent_id: "codex",
    status: "running",
    placement: { kind: "issue_preview", issue_window_id: "win-1", issue_number: issueNumber },
    ...overrides,
  };
}

function createNode(document, tagName, className, text) {
  const node = document.createElement(tagName);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

async function makeFixture({ workspaceWindows = [], projection = null } = {}) {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const issueWindow = { id: "win-1", preset: "issue" };
  const sent = [];
  const calls = {
    continued: [],
    resumed: [],
    cleaned: [],
    windowized: [],
    focusedLocally: [],
    focusedRemotely: [],
    launchWizard: [],
    terminalMounts: [],
  };
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
    focusWindowLocally: (id) => calls.focusedLocally.push(id),
    sendWindowFocus: (id) => calls.focusedRemotely.push(id),
    focusOrSpawnPreset() {},
    openIssueLaunchWizard: (windowId, number) => calls.launchWizard.push({ windowId, number }),
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
    createTerminalRuntime: (id, root) => {
      calls.terminalMounts.push({ id, root });
      return { id, terminal: { focus() {} } };
    },
    windowDisplayTitle: (windowData) => windowData?.title || windowData?.id,
    windowRoleBadgeLabel: (windowData) => windowData?.agent_id || "Agent",
    windowizeIssuePreviewWindow: (id) => calls.windowized.push(id),
    getActiveWorkProjection: () => projection,
    workAttentionFor: attentionForWorkspace,
    formatWorkLifecycleLabel: formatLifecycleStateLabel,
    continueWork: (workId, bounds) => {
      calls.continued.push({ workId, bounds });
      return true;
    },
    openWorkspaceResumePicker: (workId) => calls.resumed.push(workId),
    openWorkspaceCleanup: (candidate, windowId) => calls.cleaned.push({ candidate, windowId }),
    getResumeBounds: () => ({ x: 0, y: 0, width: 1280, height: 800 }),
  });
  surface.mountKnowledgeWindow(issueWindow, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return {
    body,
    calls,
    document,
    load,
    mod,
    sent,
    surface,
    window,
    setWindows: (next) => {
      windows = [issueWindow, ...next];
    },
  };
}

function applyEntries(surface, load, entries, selectedNumber = null) {
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

function visibleActions(row) {
  return [...row.querySelectorAll("button[data-action]")]
    .filter((button) => !button.closest(".knowledge-row-menu-list"))
    .map((button) => button.dataset.action);
}

function overflowActions(row) {
  return [...row.querySelectorAll(".knowledge-row-menu-list button[data-action]")].map(
    (button) => button.dataset.action,
  );
}

function secondaryItems(row) {
  return [...row.querySelectorAll(".knowledge-row-secondary-item")].map((item) => ({
    kind: item.dataset.kind,
    text: item.textContent,
  }));
}

// T-004: the pure state model. One primary badge, <= 2 secondary items, <= 2
// visible actions; the rest is overflow.
test("issueRowStateModel derives one primary badge, bounded secondary info, and state-dependent actions", async () => {
  const { issueRowStateModel } = await importSurfaceModule();

  const queued = issueRowStateModel({
    entry: knowledgeEntry(42, {
      monitor_state: "queued",
      queue_position: 1,
      is_spec: true,
      labels: ["gwt-spec", "auto-merge", "bug"],
    }),
    queue: { index: 0, length: 3 },
  });
  assert.deepEqual(queued.primary, { key: "monitor:queued", label: "Queued", tone: "idle" });
  assert.deepEqual(
    queued.secondary.map((item) => item.label),
    ["Queue 1", "Spec"],
    "queue position and the SPEC attribute win over the remaining labels",
  );
  assert.deepEqual(queued.actions, ["launch-now", "configure-issue"]);
  assert.deepEqual(queued.overflow, ["move-up", "move-down"]);

  const liveInline = issueRowStateModel({
    entry: knowledgeEntry(3671, { monitor_state: "launched", labels: ["auto-merge"] }),
    work: workRow({
      status_category: "blocked",
      blocked_reason: "Waiting on review",
      cleanup_blocked_reason: "live_agent",
    }),
    attention: attentionForWorkspace(
      workRow({ status_category: "blocked", blocked_reason: "Waiting on review" }),
    ),
    inlineWindow: agentWindow("agent-1", 3671),
  });
  assert.deepEqual(liveInline.primary, {
    key: "agent:running",
    label: "Running",
    tone: "active",
  });
  assert.deepEqual(
    liveInline.secondary.map((item) => [item.kind, item.label]),
    [
      ["reason", "Waiting on review"],
      ["chip", "PR #3699 · open"],
    ],
    "the attention reason and the PR come before attribute chips",
  );
  assert.deepEqual(liveInline.actions, ["windowize-issue-preview"]);
  assert.deepEqual(liveInline.overflow, [
    "configure-issue",
    "continue-work",
    "resume-work",
    "cleanup-work",
  ]);

  const onCanvas = issueRowStateModel({
    entry: knowledgeEntry(3671, { monitor_state: "launched" }),
    work: workRow(),
    attention: attentionForWorkspace(workRow()),
    canvasWindow: agentWindow("agent-1", 3671, { placement: { kind: "canvas" } }),
  });
  assert.equal(onCanvas.primary.label, "Running");
  assert.deepEqual(onCanvas.actions, ["focus-canvas-window"]);

  for (const [status, label, tone] of [
    ["waiting", "Needs input", "needs-input"],
    ["error", "Error", "blocked"],
    ["starting", "Starting", "active"],
    ["idle", "Idle", "idle"],
  ]) {
    const model = issueRowStateModel({
      entry: knowledgeEntry(1, { monitor_state: "launched" }),
      inlineWindow: agentWindow("agent-1", 1, { status }),
    });
    assert.deepEqual(model.primary, { key: `agent:${status}`, label, tone });
  }
  const stopped = issueRowStateModel({
    entry: knowledgeEntry(1, { monitor_state: "launched" }),
    inlineWindow: agentWindow("agent-1", 1, { status: "stopped" }),
  });
  assert.equal(stopped.primary.label, "Launched", "a stopped agent falls back to the Monitor state");

  const needsHuman = issueRowStateModel({
    entry: knowledgeEntry(47, { monitor_state: "needs_human" }),
    work: workRow({ status_category: "blocked", blocked_reason: "Resolve blocker" }),
    attention: attentionForWorkspace(
      workRow({ status_category: "blocked", blocked_reason: "Resolve blocker" }),
    ),
  });
  assert.deepEqual(needsHuman.primary, {
    key: "monitor:needs_human",
    label: "Needs human",
    tone: "needs-input",
  });
  assert.deepEqual(needsHuman.actions, ["continue-work", "resume-work"]);
  assert.deepEqual(needsHuman.overflow, ["configure-issue"]);

  const merged = issueRowStateModel({
    entry: knowledgeEntry(50, { monitor_state: "merged" }),
    work: workRow({
      lifecycle_state: "done",
      pr_state: "merged",
      cleanup_candidate: { branch: "work/issue-3671" },
    }),
    attention: attentionForWorkspace(workRow({ lifecycle_state: "done" })),
  });
  assert.deepEqual(merged.primary, { key: "monitor:merged", label: "Merged", tone: "done" });
  assert.deepEqual(merged.actions, ["cleanup-work", "resume-work"]);
  assert.deepEqual(merged.overflow, ["continue-work", "configure-issue"]);

  const onHold = issueRowStateModel({
    entry: knowledgeEntry(45, {
      monitor_state: "hold_excluded",
      exclusion_reason: "Excluded by label: hold",
      is_spec: true,
      labels: ["gwt-spec", "auto-merge"],
    }),
  });
  assert.deepEqual(onHold.primary, {
    key: "monitor:hold_excluded",
    label: "On hold",
    tone: "needs-input",
  });
  assert.deepEqual(
    onHold.secondary.map((item) => [item.kind, item.label]),
    [
      ["reason", "Excluded by label: hold"],
      ["chip", "Spec"],
    ],
  );
  assert.deepEqual(onHold.actions, ["configure-issue"]);
  assert.deepEqual(onHold.overflow, []);

  const closedQueued = issueRowStateModel({
    entry: knowledgeEntry(46, { monitor_state: "queued", queue_position: 3, state: "closed" }),
    queue: { index: 2, length: 3 },
  });
  assert.equal(closedQueued.primary.label, "Queued");
  assert.deepEqual(
    closedQueued.secondary.map((item) => item.label),
    ["Closed", "Queue 3"],
  );

  const unknown = issueRowStateModel({ entry: knowledgeEntry(48, { monitor_state: "awaiting_review" }) });
  assert.deepEqual(unknown.primary, {
    key: "monitor:awaiting_review",
    label: "Unknown (awaiting_review)",
    tone: "needs-input",
  });
  assert.deepEqual(unknown.actions, ["configure-issue"]);

  const bareOpen = issueRowStateModel({ entry: knowledgeEntry(49) });
  assert.deepEqual(bareOpen.primary, { key: "issue:open", label: "Open", tone: "idle" });
  assert.deepEqual(bareOpen.secondary, []);
  assert.deepEqual(bareOpen.actions, ["launch-agent"]);
  assert.deepEqual(bareOpen.overflow, []);

  const bareClosed = issueRowStateModel({ entry: knowledgeEntry(51, { state: "closed" }) });
  assert.deepEqual(bareClosed.primary, { key: "issue:closed", label: "Closed", tone: "done" });
  assert.deepEqual(bareClosed.actions, []);

  const workRunning = issueRowStateModel({
    entry: knowledgeEntry(60),
    work: workRow(),
    attention: attentionForWorkspace(workRow()),
  });
  assert.deepEqual(workRunning.primary, { key: "work:running", label: "Active", tone: "active" });
  assert.deepEqual(workRunning.actions, ["continue-work", "resume-work"]);
  assert.deepEqual(workRunning.overflow, []);

  const workPaused = issueRowStateModel({
    entry: knowledgeEntry(61),
    work: workRow({ lifecycle_state: "paused", status_category: "idle", active_agents: 0 }),
    attention: attentionForWorkspace(
      workRow({ lifecycle_state: "paused", status_category: "idle", active_agents: 0 }),
    ),
  });
  assert.deepEqual(workPaused.primary, { key: "work:paused", label: "Paused", tone: "idle" });

  const workRemote = issueRowStateModel({
    entry: knowledgeEntry(62),
    work: workRow({ remote_only: true, active_agents: 0 }),
    attention: attentionForWorkspace(workRow({ remote_only: true, active_agents: 0 })),
  });
  assert.deepEqual(workRemote.primary, { key: "work:remote", label: "Remote", tone: "remote" });

  const workClosed = issueRowStateModel({
    entry: knowledgeEntry(63),
    work: workRow({ lifecycle_state: "done", active_agents: 0 }),
    attention: attentionForWorkspace(workRow({ lifecycle_state: "done", active_agents: 0 })),
  });
  assert.deepEqual(workClosed.primary, { key: "work:closed", label: "Done", tone: "done" });
  assert.deepEqual(workClosed.actions, ["resume-work", "continue-work"]);
});

// The canvas face of a Windowized agent is found through the Work projection
// (agent window id / session id) or through the ids this surface Windowized.
test("issueCanvasAgentWindowsForIssue links canvas agent windows back to their Issue", async () => {
  const { issueCanvasAgentWindowsForIssue } = await importSurfaceModule();
  const byWindowId = agentWindow("agent-1", 3671, { placement: { kind: "canvas" } });
  const bySessionId = agentWindow("agent-2", 3671, {
    placement: { kind: "canvas" },
    session_id: "session-2",
  });
  const remembered = agentWindow("agent-3", 3671, { placement: { kind: "canvas" } });
  const stillInline = agentWindow("agent-4", 3671);
  const unrelated = agentWindow("agent-5", 3672, { placement: { kind: "canvas" } });
  const windows = [
    { id: "win-1", preset: "issue" },
    byWindowId,
    bySessionId,
    remembered,
    stillInline,
    unrelated,
  ];
  const work = workRow({
    agents: [
      { session_id: "session-1", window_id: "agent-1" },
      { session_id: "session-2", window_id: null },
    ],
  });
  assert.deepEqual(
    issueCanvasAgentWindowsForIssue(windows, work, new Set(["agent-3"])).map((w) => w.id),
    ["agent-1", "agent-2", "agent-3"],
  );
  assert.deepEqual(issueCanvasAgentWindowsForIssue(windows, null, new Set()), []);
  assert.deepEqual(
    issueCanvasAgentWindowsForIssue(windows, null, new Set(["agent-3", "agent-4"])).map(
      (w) => w.id,
    ),
    ["agent-3"],
    "a remembered id only counts while that window is on the canvas",
  );
});

// AC-5 / T-006: the rendered row honours the model's limits in every state.
test("every rendered Issue row has one primary badge, at most two secondary items, and at most two visible actions", async (t) => {
  const fixture = await makeFixture({
    workspaceWindows: [agentWindow("agent-1", 3671)],
    projection: {
      active_works: [
        workRow({
          status_category: "blocked",
          blocked_reason: "Waiting on review",
          cleanup_blocked_reason: "live_agent",
        }),
        workRow({
          id: "work-47",
          branch: "work/issue-47",
          pr_number: null,
          status_category: "blocked",
          blocked_reason: "Resolve blocker",
        }),
        workRow({
          id: "work-50",
          branch: "work/issue-50",
          lifecycle_state: "done",
          active_agents: 0,
          pr_state: "merged",
          cleanup_candidate: { branch: "work/issue-50", worktree_path: "/gwt/work/issue-50" },
        }),
      ],
    },
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(42, {
      monitor_state: "queued",
      queue_position: 1,
      is_spec: true,
      labels: ["gwt-spec", "auto-merge", "bug", "enhancement"],
    }),
    knowledgeEntry(44, { monitor_state: "queued", queue_position: 2, labels: ["auto-merge"] }),
    knowledgeEntry(3671, {
      monitor_state: "launched",
      labels: ["auto-merge", "bug"],
      linked_branch_count: 2,
      related_work_count: 1,
      related_session_count: 3,
      match_score: 88,
      meta: "Auto-launch fixture",
      related_work_refs: [{ id: "work-3671", branch: "work/issue-3671" }],
    }),
    knowledgeEntry(47, {
      monitor_state: "needs_human",
      related_work_refs: [{ id: "work-47", branch: "work/issue-47" }],
    }),
    knowledgeEntry(50, {
      monitor_state: "merged",
      related_work_refs: [{ id: "work-50", branch: "work/issue-50" }],
    }),
    knowledgeEntry(45, {
      monitor_state: "hold_excluded",
      exclusion_reason: "Excluded by label: hold",
      labels: ["gwt-spec", "auto-merge"],
      is_spec: true,
    }),
    knowledgeEntry(49),
    knowledgeEntry(51, { state: "closed" }),
  ]);

  fixture.body.querySelector('[data-issue-filter="all"]').click();
  const rows = [...fixture.body.querySelectorAll(".knowledge-row")];
  assert.equal(rows.length, 8);
  for (const row of rows) {
    const number = row.dataset.issueNumber;
    assert.equal(
      row.querySelectorAll(".knowledge-row-badge").length,
      1,
      `Issue #${number}: exactly one primary badge`,
    );
    assert.ok(
      row.querySelectorAll(".knowledge-row-secondary-item").length <= 2,
      `Issue #${number}: at most two secondary items`,
    );
    assert.ok(
      visibleActions(row).length <= 2,
      `Issue #${number}: at most two visible actions (${visibleActions(row).join(",")})`,
    );
    assert.equal(row.querySelector("button button"), null, "no nested interactive controls");
    assert.equal(row.querySelector(".knowledge-chip"), null, "raw label chips are gone");
    assert.equal(row.querySelector(".knowledge-state-chip"), null, "the open/closed chip is gone");
    assert.equal(row.querySelector(".knowledge-monitor-chip"), null, "the monitor chip is gone");
    assert.equal(row.querySelector(".knowledge-row-work"), null, "the Work band is folded in");
    assert.equal(row.querySelector(".knowledge-meta-copy"), null, "meta copy is gone");
    const menu = row.querySelector(".knowledge-row-menu");
    if (overflowActions(row).length > 0) {
      assert.ok(menu, `Issue #${number}: overflow actions live in the row menu`);
      assert.equal(menu.tagName, "DETAILS");
      assert.match(menu.querySelector("summary").getAttribute("aria-label"), /More actions/);
      assert.equal(menu.querySelector(".knowledge-row-menu-list").getAttribute("role"), "menu");
    } else {
      assert.equal(menu, null, `Issue #${number}: no menu without overflow actions`);
    }
  }

  const row42 = fixture.body.querySelector('[data-issue-number="42"]');
  const badge42 = row42.querySelector(".knowledge-row-badge");
  assert.equal(badge42.textContent, "Queued");
  assert.equal(badge42.dataset.tone, "idle");
  assert.equal(badge42.dataset.stateKey, "monitor:queued");
  assert.deepEqual(secondaryItems(row42), [
    { kind: "chip", text: "Queue 1" },
    { kind: "chip", text: "Spec" },
  ]);
  assert.deepEqual(visibleActions(row42), ["launch-now", "configure-issue"]);
  assert.deepEqual(overflowActions(row42), ["move-up", "move-down"]);
  assert.equal(row42.querySelector('[data-action="move-up"]').disabled, true);
  assert.equal(row42.querySelector('[data-action="move-down"]').disabled, false);

  const row3671 = fixture.body.querySelector('[data-issue-number="3671"]');
  const badge3671 = row3671.querySelector(".knowledge-row-badge");
  assert.equal(badge3671.textContent, "Running");
  assert.equal(badge3671.dataset.tone, "active");
  assert.deepEqual(secondaryItems(row3671), [
    { kind: "reason", text: "Waiting on review" },
    { kind: "chip", text: "PR #3699 · open" },
  ]);
  assert.deepEqual(visibleActions(row3671), ["windowize-issue-preview"]);
  assert.deepEqual(overflowActions(row3671), [
    "configure-issue",
    "continue-work",
    "resume-work",
    "cleanup-work",
  ]);
  const cleanup3671 = row3671.querySelector('[data-action="cleanup-work"]');
  assert.equal(cleanup3671.disabled, true);
  assert.equal(cleanup3671.dataset.blockedReason, "live_agent");

  const row47 = fixture.body.querySelector('[data-issue-number="47"]');
  assert.equal(row47.querySelector(".knowledge-row-badge").textContent, "Needs human");
  assert.deepEqual(secondaryItems(row47), [{ kind: "reason", text: "Resolve blocker" }]);
  assert.deepEqual(visibleActions(row47), ["continue-work", "resume-work"]);

  const row50 = fixture.body.querySelector('[data-issue-number="50"]');
  assert.equal(row50.querySelector(".knowledge-row-badge").textContent, "Merged");
  assert.deepEqual(secondaryItems(row50), [{ kind: "chip", text: "PR #3699 · merged" }]);
  assert.deepEqual(visibleActions(row50), ["cleanup-work", "resume-work"]);

  const row45 = fixture.body.querySelector('[data-issue-number="45"]');
  assert.equal(row45.querySelector(".knowledge-row-badge").textContent, "On hold");
  assert.deepEqual(secondaryItems(row45), [
    { kind: "reason", text: "Excluded by label: hold" },
    { kind: "chip", text: "Spec" },
  ]);
  assert.deepEqual(visibleActions(row45), ["configure-issue"]);

  const row49 = fixture.body.querySelector('[data-issue-number="49"]');
  assert.equal(row49.querySelector(".knowledge-row-badge").textContent, "Open");
  assert.deepEqual(secondaryItems(row49), []);
  assert.deepEqual(visibleActions(row49), ["launch-agent"]);

  const row51 = fixture.body.querySelector('[data-issue-number="51"]');
  assert.equal(row51.querySelector(".knowledge-row-badge").textContent, "Closed");
  assert.equal(row51.querySelector(".knowledge-row-badge").dataset.tone, "done");
  assert.deepEqual(visibleActions(row51), []);
});

// The overflow menu and the visible actions dispatch the same operations as before.
test("row actions dispatch from the visible buttons and from the overflow menu", async (t) => {
  const fixture = await makeFixture({
    workspaceWindows: [agentWindow("agent-1", 3671)],
    projection: {
      active_works: [
        workRow({
          cleanup_candidate: { branch: "work/issue-3671", worktree_path: "/gwt/work/issue-3671" },
        }),
      ],
    },
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(42, { monitor_state: "queued", queue_position: 1, is_spec: true }),
    knowledgeEntry(44, { monitor_state: "queued", queue_position: 2 }),
    knowledgeEntry(3671, {
      monitor_state: "launched",
      related_work_refs: [{ id: "work-3671", branch: "work/issue-3671" }],
    }),
    knowledgeEntry(49),
  ]);

  const row42 = fixture.body.querySelector('[data-issue-number="42"]');
  row42.querySelector('[data-action="launch-now"]').click();
  assert.deepEqual(fixture.sent.at(-1), {
    kind: "issue_monitor_launch_now",
    issue_number: 42,
    linked_issue_kind: "spec",
  });
  row42.querySelector('[data-action="configure-issue"]').click();
  assert.deepEqual(fixture.sent.at(-1), {
    kind: "issue_monitor_configure_issue",
    issue_number: 42,
    linked_issue_kind: "spec",
  });
  assert.equal(
    fixture.sent.filter((message) => message.kind === "select_knowledge_bridge_entry").length,
    0,
    "row actions are not selection gestures",
  );

  const row44 = fixture.body.querySelector('[data-issue-number="44"]');
  const menu44 = row44.querySelector(".knowledge-row-menu");
  menu44.open = true;
  menu44.querySelector('[data-action="move-up"]').click();
  assert.deepEqual(fixture.sent.at(-1), {
    kind: "reorder_issue_monitor_issues",
    issue_numbers: [44, 42],
  });
  assert.equal(
    fixture.body.querySelector('[data-issue-number="44"] .knowledge-row-menu').hasAttribute("open"),
    false,
    "choosing a menu item closes the menu",
  );

  const row3671 = fixture.body.querySelector('[data-issue-number="3671"]');
  row3671.querySelector('[data-action="windowize-issue-preview"]').click();
  assert.deepEqual(fixture.calls.windowized, ["agent-1"]);
  const menu3671 = row3671.querySelector(".knowledge-row-menu");
  menu3671.open = true;
  menu3671.querySelector('[data-action="continue-work"]').click();
  assert.deepEqual(fixture.calls.continued, [
    { workId: "work-3671", bounds: { x: 0, y: 0, width: 1280, height: 800 } },
  ]);
  const reopened = fixture.body.querySelector('[data-issue-number="3671"] .knowledge-row-menu');
  reopened.open = true;
  reopened.querySelector('[data-action="resume-work"]').click();
  assert.deepEqual(fixture.calls.resumed, ["work-3671"]);
  const reopenedAgain = fixture.body.querySelector(
    '[data-issue-number="3671"] .knowledge-row-menu',
  );
  reopenedAgain.open = true;
  reopenedAgain.querySelector('[data-action="cleanup-work"]').click();
  assert.equal(fixture.calls.cleaned.length, 1);
  assert.equal(fixture.calls.cleaned[0].candidate.branch, "work/issue-3671");
  assert.equal(fixture.calls.cleaned[0].windowId, "win-1");

  fixture.body.querySelector('[data-issue-number="49"] [data-action="launch-agent"]').click();
  assert.deepEqual(fixture.calls.launchWizard, [{ windowId: "win-1", number: 49 }]);
  assert.equal(
    fixture.sent.filter((message) => message.kind === "select_knowledge_bridge_entry").length,
    0,
  );
});

// T-005 / FR-012: after Windowize the row keeps the Issue ↔ agent link as a
// "Shown on canvas" face with no second input face for the PTY.
test("after Windowize the row shows the agent as on the canvas and offers focus", async (t) => {
  const agent = agentWindow("agent-1", 3671);
  const fixture = await makeFixture({ workspaceWindows: [agent] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671, { monitor_state: "launched" })]);

  const statusRow = fixture.body.querySelector('[data-issue-number="3671"] .issue-agent-status');
  assert.ok(statusRow, "the row carries the read-only agent status row");
  assert.equal(statusRow.querySelector(".terminal-root"), null, "the list row mounts no xterm");
  assert.equal(statusRow.querySelector(".knowledge-monitor-chip"), null, "the state is the row badge");
  assert.equal(fixture.calls.terminalMounts.length, 0);
  fixture.body.querySelector('[data-action="windowize-issue-preview"]').click();
  assert.deepEqual(fixture.calls.windowized, ["agent-1"]);

  fixture.setWindows([{ ...agent, placement: { kind: "canvas" } }]);
  fixture.surface.renderKnowledgeBridge("win-1");

  const row = fixture.body.querySelector('[data-issue-number="3671"]');
  const face = row.querySelector(".issue-agent-status");
  assert.ok(face, "the row keeps an agent face for the Windowized agent");
  assert.equal(face.classList.contains("is-on-canvas"), true);
  assert.equal(face.dataset.windowId, "agent-1");
  assert.equal(face.querySelector(".terminal-root"), null, "no second input face for the PTY");
  assert.equal(fixture.calls.terminalMounts.length, 0, "nothing is mounted");
  assert.match(face.textContent, /Shown on canvas/);
  assert.equal(face.querySelector('[data-action="windowize-issue-preview"]'), null);
  assert.equal(row.querySelector(".knowledge-row-badge").textContent, "Running");
  assert.deepEqual(visibleActions(row), ["focus-canvas-window"]);

  row.querySelector('[data-action="focus-canvas-window"]').click();
  assert.deepEqual(fixture.calls.focusedLocally, ["agent-1"]);
  assert.deepEqual(fixture.calls.focusedRemotely, ["agent-1"]);
  assert.equal(
    fixture.sent.some((message) => message.kind === "select_knowledge_bridge_entry"),
    false,
  );

  // The agent window is closed: the row drops the face and falls back to the
  // Monitor state.
  fixture.setWindows([]);
  fixture.surface.renderKnowledgeBridge("win-1");
  const gone = fixture.body.querySelector('[data-issue-number="3671"]');
  assert.equal(gone.querySelector(".issue-agent-status"), null);
  assert.equal(gone.querySelector(".knowledge-row-badge").textContent, "Launched");
});

// AC-10 / T-006: the new row and terminal CSS is Operator tokens only.
test("Issue row state CSS uses Operator tokens only", () => {
  const selectors = [
    ".knowledge-row-badge",
    ".knowledge-row-secondary",
    ".knowledge-row-secondary-item",
    ".knowledge-row-menu",
    ".knowledge-row-menu-list",
    ".knowledge-row-menu-item",
    ".issue-agent-status.is-on-canvas",
    ".issue-agent-status-placeholder",
  ];
  const defined = new Set();
  for (const source of [tokensCss, typographyCss]) {
    for (const match of source.matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      defined.add(match[1]);
    }
  }
  for (const selector of selectors) {
    const blocks = blocksFor(appCss, selector);
    assert.ok(blocks.length > 0, `${selector} is styled in app.css`);
    for (const block of blocks) {
      assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b/, `${selector}: no raw hex colors`);
      assert.doesNotMatch(block, /\brgba?\(/, `${selector}: no raw rgb colors`);
      for (const match of block.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
        assert.ok(defined.has(match[1]), `${selector}: token ${match[1]} is defined`);
      }
    }
  }
  assert.match(
    blocksFor(appCss, ".knowledge-row-badge").join("\n"),
    /data-tone="remote"/,
    "the Remote lane has a badge tone",
  );
});

function blocksFor(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(^|[\\s,}])${escaped}(?=[\\s,:[{>.])`, "g");
  const blocks = [];
  for (const match of css.matchAll(pattern)) {
    const open = css.indexOf("{", match.index);
    if (open < 0) continue;
    let depth = 0;
    for (let index = open; index < css.length; index += 1) {
      if (css[index] === "{") depth += 1;
      if (css[index] === "}") {
        depth -= 1;
        if (depth === 0) {
          blocks.push(css.slice(match.index, index + 1));
          break;
        }
      }
    }
  }
  return blocks;
}

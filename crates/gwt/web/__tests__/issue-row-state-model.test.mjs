// SPEC #3885 Phase 2 (T-004 / T-005 / T-006) — the Issue row state model.
//
// Every Issue row carries exactly one primary state badge, at most two pieces of
// secondary information, and at most two visible action buttons that depend on
// that state. Anything else moves into the row's overflow menu. The row's agent
// status row carries no second badge, and once the agent is Windowized the same
// slot shows a "Shown in Agents" face instead of a second input face.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";
import {
  attentionForWorkspace,
  formatLifecycleStateLabel,
} from "../issue-other-surface.js";

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
  return import(`data:text/javascript;base64,${Buffer.from(source.replace('from "./agent-kanban-surface.js"', `from "${new URL("../agent-kanban-surface.js", import.meta.url).href}"`).replace('from "./control-explanations.js"', `from "${new URL("../control-explanations.js", import.meta.url).href}"`).replace('from "./ui-state-store.js"', `from "${new URL("../ui-state-store.js", import.meta.url).href}"`).replace('from "./ui-content.js"', `from "${new URL("../ui-content.js", import.meta.url).href}"`)).toString("base64")}`);
}

test("pending SessionStart diagnosis uses the existing row reason without changing launch state", async () => {
  const { issueRowStateModel } = await importSurfaceModule();
  const entry = knowledgeEntry(4940, { monitor_state: "launching",
    readiness_diagnosis: "SessionStart readiness pending: hooks.json missing" });
  const pending = issueRowStateModel({ entry });
  assert.equal(pending.primary.label, "Launching");
  assert.deepEqual(pending.secondary, [{ kind: "reason", key: "reason", label: entry.readiness_diagnosis }]);
  assert.deepEqual(issueRowStateModel({ entry: { ...entry, readiness_diagnosis: null } }).secondary, []);
});

test("Issue #4499 source exclusions and legacy states show triage with their actual or fallback reason", async () => {
  const { issueRowStateModel } = await importSurfaceModule();
  for (const monitor_state of ["source_excluded", "hold_excluded"]) {
    const entry = knowledgeEntry(4499, { monitor_state });
    const fallback = issueRowStateModel({ entry });
    assert.deepEqual(fallback.primary, { key: "monitor:source_excluded", label: "Needs triage", tone: "needs-input" });
    assert.deepEqual(fallback.secondary, [{ kind: "reason", key: "reason",
      label: "Source trust review is required before automatic launches." }]);
    for (const exclusion_reason of [
      "untrusted feedback; PM triage required; use a separate curated execution owner",
      "Source temporarily unavailable; hold label: HoLd",
    ]) {
      const actual = issueRowStateModel({ entry: { ...entry, exclusion_reason } });
      assert.equal(actual.secondary[0].label, exclusion_reason);
    }
    for (const exclusion_reason of [
      "hold label: hold", "Excluded by label: hold", "hold label: HoLd", "hold label: HOLD", "Excluded by label: HoLd",
    ]) {
      const stale = issueRowStateModel({ entry: { ...entry, exclusion_reason } });
      assert.equal(stale.secondary[0].label,
        "This exclusion was reported by an older monitor. Refresh to confirm queue membership.");
    }
  }
});

test("Issue #4862 a deliberate stop stays Held despite stale window activity", async () => {
  const { issueQueueColumn, issueRowStateModel } = await importSurfaceModule();
  const entry = knowledgeEntry(4862, { monitor_state: "needs_human", error_message: "stopped: PM paused for investigation",
    readiness_diagnosis: "SessionStart readiness pending: stale warning" });
  assert.equal(issueQueueColumn(entry, workRow()), "backlog");
  for (const status of ["running", "error"]) {
    const context = { entry, work: workRow(), inlineWindow: agentWindow("agent-1", 4862, { status }) };
    const model = issueRowStateModel(context);
    assert.deepEqual(model.primary, { key: "monitor:held", label: "Held", tone: "idle" });
    assert.equal(model.secondary[0].label, "PM paused for investigation");
    const prior = issueRowStateModel({ ...context, entry: { ...entry, error_message: null } });
    assert.deepEqual(model.actions, prior.actions, "presentation must not change actions");
    assert.deepEqual(model.overflow, prior.overflow);
  }
});

test("Issue #4862 launch diagnoses retain the cause and name the next operation", async () => {
  const { issueQueueColumn, issueRowStateModel } = await importSurfaceModule();
  for (const [monitor_state, error_message, recovery] of [
    ["launch_failed", "cannot launch successor of Work work-42: successor work-42-next has invalid lineage", "Work lineage · Next: workspace.close (superseded), then issue.monitor.requeue"],
    ["agent_failed", "cannot launch successor of Work work-43: owner or execution container mismatch", "Work owner mismatch · Next: workspace.candidates"],
    ["needs_human", "autonomous launch attempts exhausted (3/3) with an unchanging failure: Launch timed out: no window was created within 120s; delivery second", "Window timeout · Next: issue.monitor.requeue"],
    ["needs_human", "autonomous launch attempts exhausted (3/3) with an unchanging failure: successor work-44-next is duplicated; use workspace.close", "Launch failure · Next: issue.monitor.status"],
    ["launch_failed", "launch provider unavailable", "Launch failure · Next: issue.monitor.status"],
    ["agent_failed", "agent process exited", "Agent failure · Next: issue.monitor.status"],
  ]) {
    const model = issueRowStateModel({ entry: knowledgeEntry(4862, { monitor_state, error_message }) });
    assert.deepEqual(model.secondary.map(item => [item.key, item.label]), [["reason", error_message], ["recovery", recovery]]);
    if (monitor_state === "needs_human") {
      assert.equal(issueQueueColumn(knowledgeEntry(4862, { monitor_state, error_message }), workRow()), "backlog");
    }
  }
});

test("Issue #6141 closed issues keep live sessions in Active until cleanup", async () => {
  const { issueQueueColumn, issueRowStateModel } = await importSurfaceModule();
  const entry = knowledgeEntry(6141, { state: "closed", monitor_state: "released" });
  const live = agentWindow("live", 6141);
  assert.equal(issueQueueColumn(entry, null, live), "active");
  assert.equal(issueQueueColumn(entry, workRow()), "active");
  assert.equal(issueQueueColumn({ ...entry, monitor_state: "launched" }), "active");
  assert.equal(issueQueueColumn(entry, null, { ...live, status: "stopped" }), "done");
  assert.equal(issueQueueColumn(entry, null, { ...live, status: "error" }), "done");
  const model = issueRowStateModel({ entry, inlineWindow: live });
  assert.deepEqual(model.secondary.map(item => item.label), ["Closed", "Session cleanup pending"]);
});

test("Issue #6141 Active metric follows the displayed cards and errors explain their cause", async t => {
  const runtimeStates = {};
  const fixture = await makeFixture({ workspaceWindows: [agentWindow("live", 6141)], runtimeStates });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  fixture.surface.applyIssueMonitorStatus({ active_count: 7, max_active_agents: 8 });
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(6141, { state: "closed" })]);
  assert.equal(fixture.body.querySelector('[data-queue-column="active"] h3').textContent, "Active · 1");
  const metric = fixture.body.querySelector('[data-metric="active"]');
  assert.equal(metric.textContent, "Active 1/8");
  assert.match(metric.dataset.issueExplanation, /capacity.*7\/8/i);
  runtimeStates.live = "error";
  fixture.surface.applyIssueMonitorInbox([{ issue: { number: 6141, state: "closed" },
    state: "agent_failed", error_message: "Agent exited with code 42" }]);
  fixture.surface.renderAllKnowledgeBridgeWindows();
  assert.equal(fixture.body.querySelector('[data-queue-column="done"] h3').textContent, "Done · 1");
  assert.equal(metric.textContent, "Active 0/8");
  assert.match(fixture.body.querySelector('.knowledge-row .knowledge-row-badge').dataset.issueExplanation, /Agent exited with code 42/);
});

test("Issue #5491 judgment holds outrank stale Work activity but preserve live agents", async () => {
  const { issueQueueColumn, issueRowStateModel } = await importSurfaceModule();
  const reason = "Choose the publication scope before work can continue";
  for (const monitor_state of ["needs_human", "blocked_by_claim"]) {
    const entry = knowledgeEntry(5491, { monitor_state, queue_position: 2, error_message: reason });
    assert.equal(issueQueueColumn(entry, workRow()), "backlog");
    assert.equal(issueQueueColumn(entry, workRow(), agentWindow("live", 5491)), "active");
  }
  assert.equal(issueQueueColumn(knowledgeEntry(5491), workRow({ status_category: "blocked" })), "backlog");
  assert.equal(issueQueueColumn(knowledgeEntry(5491, { monitor_state: "launching" }), workRow({ status_category: "blocked" })), "active");
  const model = issueRowStateModel({ entry: knowledgeEntry(5491, { monitor_state: "needs_human", error_message: reason }) });
  assert.equal(model.primary.label, "Needs human");
  assert.equal(model.secondary[0].label, reason);
});

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

async function makeFixture({ workspaceWindows = [], otherProjectWindows = [], projection = null, kind = "issue", runtimeStates = {} } = {}) {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const issueWindow = { id: "win-1", preset: kind };
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
    scheduleFrame: run => { run(); return () => {}; },
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (...args) => createNode(document, ...args),
    createKnowledgeMarkdownBody: () => document.createElement("div"),
    windowMap: new Map([[issueWindow.id, body]]),
    workspaceWindowById: (id) => windows.find((entry) => entry.id === id) || null,
    getWorkspaceWindows: () => [...windows, ...otherProjectWindows],
    windowCatalogModel: {
      read: () => ({ catalog: { runtimeStates, appState: { tabs: [
        { id: "project-1", workspace: { windows } },
        { id: "project-2", workspace: { windows: otherProjectWindows } },
      ] } } }),
      subscribeFrame: () => () => {},
    },
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => kind,
    focusWindowLocally: (id) => calls.focusedLocally.push(id),
    sendWindowFocus: (id) => calls.focusedRemotely.push(id),
    focusOrSpawnPreset() {},
    openIssueLaunchWizard: (windowId, number) => calls.launchWizard.push({ windowId, number }),
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
    continueWork: (workId) => {
      calls.continued.push({ workId });
      return true;
    },
    openWorkspaceResumePicker: (workId) => calls.resumed.push(workId),
    openWorkspaceCleanup: (candidate, windowId) => calls.cleaned.push({ candidate, windowId }),
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

test("Issue #4804 queue drops honor the current project's live window without Work", async t => {
  const fixture = await makeFixture({ workspaceWindows: [agentWindow("live", 4804)] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(4804), knowledgeEntry(4805)]);
  const queued = fixture.body.querySelector('[data-queue-column="queued"]');
  const drop = number => {
    const event = new fixture.window.Event("drop", { bubbles: true, cancelable: true });
    event.dataTransfer = { getData: () => String(number) };
    queued.dispatchEvent(event);
  };
  drop(4804);
  assert.deepEqual(fixture.sent.filter(message => message.kind === "issue_monitor_queue_push"), [],
    "an active Issue cannot be queued while its Work projection is absent");
  drop(4805);
  assert.deepEqual(fixture.sent.filter(message => message.kind === "issue_monitor_queue_push")
    .map(message => message.issue_numbers), [[4805]], "an unrelated backlog Issue can still be queued");
});

test("Issue #4804 a retained canvas preserves live inline output and Focus until close", async t => {
  const live = agentWindow("live", 4804);
  const retained = agentWindow("retained", 4804, {
    linked_issue_number: 4804, status: "stopped", placement: { kind: "canvas" },
  });
  const fixture = await makeFixture({ workspaceWindows: [live, retained] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  const entries = [knowledgeEntry(4804)];
  applyEntries(fixture.surface, fixture.load, entries, 4804);
  const row = fixture.body.querySelector('.knowledge-row[data-issue-number="4804"]');
  assert.equal(row.querySelector(".knowledge-row-badge").textContent, "Running");
  assert.equal(fixture.body.querySelectorAll('[data-action="windowize-issue-preview"]').length, 0);
  row.querySelector('[data-action="focus-surface-window"]').click();
  assert.deepEqual(fixture.calls.focusedLocally, ["retained"]);
  row.querySelector('[data-action="stop-agent"]').click();
  assert.deepEqual(fixture.sent.filter(message => message.kind === "stop_window")
    .map(message => message.id), ["live"]);
  fixture.surface.requestKnowledgeDetail("win-1", "issue", 4804);
  fixture.body.querySelector('[data-issue-detail-view="output"]').click();
  assert.equal(fixture.body.querySelector(".issue-preview").dataset.windowId, "live");
  assert.equal(fixture.body.querySelectorAll('[data-action="windowize-issue-preview"]').length, 0);
  fixture.body.querySelector('[data-issue-view="split"]').click();
  const pair = fixture.body.querySelector('.issue-split-pair[data-issue-number="4804"]');
  assert.equal(pair.dataset.windowId, "live");
  pair.querySelector('[data-action="focus-surface-window"]').click();
  assert.deepEqual(fixture.calls.focusedLocally, ["retained", "retained"]);
  fixture.setWindows([live]);
  fixture.surface.renderKnowledgeBridge("win-1");
  fixture.body.querySelector('[data-action="windowize-issue-preview"]').click();
  assert.deepEqual(fixture.calls.windowized, ["live"]);
});

test("Issue #5401 identical entries retain rows and one changed key replaces only that row", async t => {
  const fixture = await makeFixture();
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  const entries = [1, 2, 3].map(number => knowledgeEntry(number));
  const row = number => fixture.body.querySelector(`.knowledge-row[data-issue-number="${number}"]`);
  applyEntries(fixture.surface, fixture.load, entries);
  const original = entries.map(entry => row(entry.number));
  applyEntries(fixture.surface, fixture.load, structuredClone(entries));
  assert.ok(entries.every((entry, index) => row(entry.number) === original[index]), "identical replay retains every row");
  applyEntries(fixture.surface, fixture.load, [entries[2], { ...entries[1], title: "Updated" }, entries[0]]);
  assert.equal(row(1), original[0]);
  assert.notEqual(row(2), original[1]);
  assert.equal(row(3), original[2]);
  assert.deepEqual([...fixture.body.querySelectorAll('[data-queue-column="backlog"] .knowledge-row')]
    .map(node => Number(node.dataset.issueNumber)), [3, 2, 1]);
});

test("Issue #5401 mounts all 200 rows but virtualizes larger lists", async t => {
  const fixture = await makeFixture();
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  const entries = Array.from({ length: 1545 }, (_, index) => knowledgeEntry(index + 1));
  applyEntries(fixture.surface, fixture.load, entries.slice(0, 200));
  assert.equal(fixture.body.querySelectorAll(".knowledge-row").length, 200);
  applyEntries(fixture.surface, fixture.load, entries);
  assert.ok(fixture.body.querySelectorAll(".knowledge-row").length < 200);
  assert.match(fixture.body.querySelector('[data-queue-column="backlog"] h3').textContent, /1545/);
  assert.ok(fixture.body.querySelector(".knowledge-virtual-spacer"));
});

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
  assert.deepEqual(queued.overflow, ["queue-remove", "move-up", "move-down"]);

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
  // SPEC #3885 FR-015: stopping the agent is offered only from the ⋯ menu.
  assert.deepEqual(liveInline.overflow, [
    "configure-issue",
    "continue-work",
    "resume-work",
    "cleanup-work",
    "stop-agent",
  ]);

  const inAgents = issueRowStateModel({
    entry: knowledgeEntry(3671, { monitor_state: "launched" }),
    work: workRow(),
    attention: attentionForWorkspace(workRow()),
    surfaceWindow: agentWindow("agent-1", 3671, { placement: { kind: "canvas" } }),
  });
  assert.equal(inAgents.primary.label, "Running");
  assert.deepEqual(inAgents.actions, ["focus-surface-window"]);

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

  const sourceExcluded = issueRowStateModel({
    entry: knowledgeEntry(45, {
      monitor_state: "source_excluded",
      exclusion_reason: "untrusted feedback; PM triage required",
      is_spec: true,
      labels: ["gwt-spec", "auto-merge"],
    }),
  });
  assert.deepEqual(sourceExcluded.primary, {
    key: "monitor:source_excluded",
    label: "Needs triage",
    tone: "needs-input",
  });
  assert.deepEqual(
    sourceExcluded.secondary.map((item) => [item.kind, item.label]),
    [
      ["reason", "untrusted feedback; PM triage required"],
      ["chip", "Spec"],
    ],
  );
  assert.deepEqual(sourceExcluded.actions, ["configure-issue"]);
  assert.deepEqual(sourceExcluded.overflow, []);

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
  // SPEC #3165 TQ-9: a Backlog Issue is exactly the row the user queues from,
  // so the push sits beside the direct launch.
  assert.deepEqual(bareOpen.actions, ["queue-push", "launch-agent"]);
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
test("issueSurfaceAgentWindowsForIssue links canvas agent windows back to their Issue", async () => {
  const { issueSurfaceAgentWindowsForIssue } = await importSurfaceModule();
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
    issueSurfaceAgentWindowsForIssue(windows, work, new Set(["agent-3"])).map((w) => w.id),
    ["agent-1", "agent-2", "agent-3"],
  );
  assert.deepEqual(issueSurfaceAgentWindowsForIssue(windows, null, new Set()), []);
  assert.deepEqual(
    issueSurfaceAgentWindowsForIssue(windows, null, new Set(["agent-3", "agent-4"])).map(
      (w) => w.id,
    ),
    ["agent-3"],
    "a remembered id only counts while that window is on the canvas",
  );
});

test("Issue #4804 recognizes a linked canvas window before its Work projection arrives", async () => {
  const { issueSurfaceAgentWindowsForIssue } = await importSurfaceModule();
  const canvas = agentWindow("agent-1", 3671, {
    linked_issue_number: 3671, placement: { kind: "canvas" },
  });
  assert.deepEqual(
    issueSurfaceAgentWindowsForIssue([canvas], null, new Set(), 3671).map(window => window.id),
    [canvas.id],
  );
});

test("Issue #4804 focuses its existing window across repeated actions and permits windowizing after close", async t => {
  const inline = agentWindow("inline", 3671, { linked_issue_number: 3671 });
  const canvas = agentWindow("canvas", 3671, {
    linked_issue_number: 3671, placement: { kind: "canvas" },
  });
  const fixture = await makeFixture({
    workspaceWindows: [inline, canvas],
    otherProjectWindows: [{ ...canvas, id: "foreign-canvas" }],
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(3671, { monitor_state: "launched" })], 3671);
  fixture.surface.requestKnowledgeDetail("win-1", "issue", 3671);
  fixture.body.querySelector('[data-issue-detail-view="output"]').click();

  assert.ok(!fixture.body.querySelector('[data-action="windowize-issue-preview"]'),
    "Kanban and the output preview offer focus while this project already has a window");
  assert.match(fixture.body.querySelector(".issue-preview-empty").textContent, /Shown in Agents/);
  const focus = fixture.body.querySelector('[data-action="focus-surface-window"]');
  focus.click();
  focus.click();
  assert.deepEqual(fixture.calls.focusedRemotely, [canvas.id, canvas.id]);
  assert.deepEqual(fixture.calls.windowized, []);

  fixture.body.querySelector('[data-issue-view="split"]').click();
  const split = fixture.body.querySelector('.issue-split-pair[data-issue-number="3671"]');
  assert.equal(split.dataset.windowId, canvas.id);
  assert.ok(!split.querySelector('[data-action="windowize-issue-preview"]'));
  assert.ok(split.querySelector('[data-action="focus-surface-window"]'));
  fixture.body.querySelector('[data-issue-view="list"]').click();

  fixture.setWindows([inline]);
  fixture.surface.renderKnowledgeBridge("win-1");
  const windowize = fixture.body.querySelector('[data-action="windowize-issue-preview"]');
  assert.ok(windowize, "closing this project's window restores Windowize despite the foreign Issue number match");
  windowize.click();
  windowize.click();
  assert.deepEqual(fixture.calls.windowized, [inline.id, inline.id],
    "consecutive requests address the same pane; backend undock is idempotent");

  fixture.setWindows([{ ...inline, placement: { kind: "canvas" } }]);
  fixture.surface.renderKnowledgeBridge("win-1");
  assert.ok(!fixture.body.querySelector('[data-action="windowize-issue-preview"]'));
  assert.ok(!fixture.body.querySelector('.issue-card-output[data-window-id="foreign-canvas"]'),
    "another project does not appear in this Issue's output");
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
      monitor_state: "source_excluded",
      exclusion_reason: "untrusted feedback; PM triage required",
      labels: ["gwt-spec", "auto-merge"],
      is_spec: true,
    }),
    knowledgeEntry(49),
    knowledgeEntry(51, { state: "closed" }),
  ]);

  // #4499: all states are visible in the queue columns.
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
  assert.deepEqual(overflowActions(row42), ["queue-remove", "move-up", "move-down"]);
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
    "stop-agent",
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
  assert.equal(row45.querySelector(".knowledge-row-badge").textContent, "Needs triage");
  assert.deepEqual(secondaryItems(row45), [
    { kind: "reason", text: "untrusted feedback; PM triage required" },
    { kind: "chip", text: "Spec" },
  ]);
  assert.deepEqual(visibleActions(row45), ["configure-issue"]);

  const row49 = fixture.body.querySelector('[data-issue-number="49"]');
  assert.equal(row49.querySelector(".knowledge-row-badge").textContent, "Open");
  assert.deepEqual(secondaryItems(row49), []);
  assert.deepEqual(visibleActions(row49), ["queue-push", "launch-agent"]);

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
  const remove42 = row42.querySelector('[data-action="queue-remove"]');
  assert.equal(remove42.textContent, "Remove from queue");
  assert.equal(remove42.getAttribute("aria-label"), "Remove from queue Issue #42");
  const beforeRemove = fixture.sent.length;
  remove42.click();
  assert.deepEqual(fixture.sent.slice(beforeRemove), [{
    kind: "issue_monitor_queue_remove",
    issue_numbers: [42],
  }]);
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
    kind: "issue_monitor_queue_move",
    issue_number: 44,
    position: 0,
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
    { workId: "work-3671" },
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

  // SPEC #3165 TQ-9: the queue push is the requested feature's main direction.
  // Clicking it must reach the backend, not fall through to the default arm.
  fixture.body.querySelector('[data-issue-number="49"] [data-action="queue-push"]').click();
  assert.deepEqual(
    fixture.sent.filter((message) => message.kind === "issue_monitor_queue_push"),
    [{ kind: "issue_monitor_queue_push", issue_numbers: [49] }],
  );

  fixture.body.querySelector('[data-issue-number="49"] [data-action="launch-agent"]').click();
  assert.deepEqual(fixture.calls.launchWizard, [{ windowId: "win-1", number: 49 }]);
  assert.equal(
    fixture.sent.filter((message) => message.kind === "select_knowledge_bridge_entry").length,
    0,
  );
});

// T-005 / FR-012: after Windowize the row keeps the Issue ↔ agent link as a
// "Shown in Agents" face with no second input face for the PTY.
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
  assert.equal(face.classList.contains("is-in-agents"), true);
  assert.equal(face.dataset.windowId, "agent-1");
  assert.equal(face.querySelector(".terminal-root"), null, "no second input face for the PTY");
  assert.equal(fixture.calls.terminalMounts.length, 0, "nothing is mounted");
  assert.match(face.textContent, /Shown in Agents/);
  assert.equal(face.querySelector('[data-action="windowize-issue-preview"]'), null);
  assert.equal(row.querySelector(".knowledge-row-badge").textContent, "Running");
  assert.deepEqual(visibleActions(row), ["focus-surface-window"]);

  row.querySelector('[data-action="focus-surface-window"]').click();
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
    ".issue-other-group",
    ".issue-other-summary",
    ".issue-lane-filter",
    ".knowledge-row-badge",
    ".knowledge-row-secondary",
    ".knowledge-row-secondary-item",
    ".knowledge-row-menu",
    ".knowledge-row-menu-list",
    ".knowledge-row-menu-item",
    ".issue-agent-status.is-in-agents",
    ".issue-agent-status-placeholder",
    ".issue-card-output",
    ".issue-card-output-text",
    ".issue-card-output-screen",
    ".issue-card-output-label",
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

test("Active cards show every launch tail without mounting terminals or replacing the card", async (t) => {
  const fixture = await makeFixture({ workspaceWindows: [
    agentWindow("agent-1", 3671), agentWindow("agent-2", 3671),
    agentWindow("agent-3", 3672),
    { id: "win-2", preset: "issue" },
    agentWindow("other-host", 3671, {
      placement: { kind: "issue_preview", issue_window_id: "win-2", issue_number: 3671 },
    }),
  ] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  fixture.surface.applyKnowledgeReceiveEvent({ kind: "terminal_preview", id: "agent-1", text: "  first\n\n<third>" });
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(3671, { monitor_state: "launched" }),
    knowledgeEntry(3672, { monitor_state: "launched" }),
    knowledgeEntry(42),
  ]);
  const previews = fixture.body.querySelectorAll(".issue-card-output");
  assert.equal(previews.length, 4, "all launches are visible, including agents hosted by the other Issues pane");
  const first = previews[0].querySelector("pre");
  assert.equal(first.textContent, "  first\n\n<third>");
  assert.equal(first.querySelector("third"), null, "terminal text is never HTML");
  assert.match(previews[0].getAttribute("aria-label"), /read.only/i);
  assert.equal(previews[1].querySelector("pre").textContent, "Waiting for output");
  const mounts = fixture.calls.terminalMounts.length;
  fixture.surface.applyKnowledgeReceiveEvent({ kind: "terminal_preview", id: "agent-1", text: "new output" });
  assert.equal(first.textContent, "new output");
  fixture.surface.applyKnowledgeReceiveEvent({ kind: "terminal_preview", id: "other-host", text: "other pane output" });
  assert.equal(fixture.body.querySelector('.issue-card-output[data-window-id="other-host"] pre').textContent, "other pane output");
  assert.ok(first.isConnected, "stream updates must preserve card focus and menus");
  assert.equal(fixture.calls.terminalMounts.length, mounts);
  assert.equal(fixture.body.querySelector('[data-queue-column="backlog"] .issue-card-output'), null);
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

// Issue #4886: a cache refresh must preserve only still-present open menus.
test("Issue menu disclosure survives row replacement without retaining removed rows", async (t) => {
  const fixture = await makeFixture({ workspaceWindows: [agentWindow("agent-41", 41), agentWindow("agent-42", 42)] });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  const entries = [41, 42].map(number => knowledgeEntry(number, { monitor_state: "launched" }));
  const refresh = rows => applyEntries(fixture.surface, fixture.load, rows);
  const menu = number => fixture.body.querySelector(`.knowledge-list [data-issue-number="${number}"] .knowledge-row-menu`);
  refresh(entries);
  const original = menu(41);
  original.setAttribute("open", "");
  refresh(entries);
  assert.equal(menu(41), original, "unchanged rows retain the native disclosure");
  assert.equal(menu(41).hasAttribute("open"), true);
  assert.equal(menu(42).hasAttribute("open"), false);
  fixture.body.querySelector('[data-issue-view="split"]').click();
  assert.equal(menu(41).hasAttribute("open"), true, "split rows retain the same Issue disclosure");
  refresh(entries);
  assert.equal(menu(41).hasAttribute("open"), true);
  assert.equal(menu(42).hasAttribute("open"), false);
  refresh([entries[1]]);
  assert.equal(menu(41), null);
  refresh(entries);
  assert.equal(menu(41).hasAttribute("open"), false, "a returning row has no stale disclosure state");
});

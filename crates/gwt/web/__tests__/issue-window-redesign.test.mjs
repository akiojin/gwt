// SPEC #3885 Phase 5 (Issue #4559, T-032..T-035): the Issue window's two-band
// toolbar, the "+ New" popover, the ⚠ monitor pill, and the detail pane order.
// These are the SPEC #3885 T-0xx tasks, not the SPEC #3200 T-032/T-033/T-035
// that live in `crates/gwt/src/issue_monitor.rs`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));

async function importSurfaceModule() {
  const source = readFileSync(resolve(here, "../knowledge-kanban-surface.js"), "utf8")
    .replace(
      'from "/focus-trap.js"',
      'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
    )
    .replace(
      'from "./launch-pending-controller.js"',
      'from "data:text/javascript,export function createLaunchOperationId(){return%20%22t%22}"',
    );
  return import(`data:text/javascript;base64,${Buffer.from(source.replace('from "./ui-state-store.js"', `from "${new URL("../ui-state-store.js", import.meta.url).href}"`)).toString("base64")}`);
}

function entry(number, monitorState, extra = {}) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    labels: ["bug"],
    is_spec: false,
    monitor_state: monitorState,
    queue_position: null,
    related_work_refs: [],
    ...extra,
  };
}

async function makeFixture(options = {}) {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const windowData = { id: "win-1", preset: "issue" };
  const sent = [];
  const reported = [];
  const resolved = [];
  const surface = mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (tag, className, text) => {
      const node = document.createElement(tag);
      if (className) node.className = className;
      if (text !== undefined && text !== null) node.textContent = String(text);
      return node;
    },
    createKnowledgeMarkdownBody: (section) => {
      const node = document.createElement("div");
      node.textContent = section?.body || "";
      return node;
    },
    windowMap: new Map([[windowData.id, body]]),
    workspaceWindowById: (id) => (id === windowData.id ? windowData : null),
    getWorkspaceWindows: () => [windowData, ...(options.windows || [])],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => "issue",
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
    reportSurfaceError: (error) => reported.push(error),
    resolveSurfaceError: (key) => resolved.push(key),
    ...options.surface,
  });
  surface.mountKnowledgeWindow(windowData, body);
  const state = surface.knowledgeBridgeStateMap.get("win-1");
  return { body, document, mod, sent, surface, state, reported, resolved };
}

const STATUS = {
  enabled: true,
  state: "active",
  queue_len: 3,
  active_count: 1,
  max_active_agents: 2,
  launch_profile_source: "saved",
  launch_profile_summary: "codex / gpt-5 / high",
};

// --- T-032 / AC-23 ---------------------------------------------------------

test("AC-23: the toolbar is two bands and no summary/settings/quick/error/status rows exist", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const root = body.querySelector(".issue-bridge-root");
  const bands = [...root.children].filter(
    (node) =>
      node.classList.contains("workspace-toolbar") ||
      node.classList.contains("knowledge-monitor-bar"),
  );
  assert.equal(bands.length, 2, "exactly the Issue band and the Monitor band");
  for (const gone of [
    ".knowledge-monitor-panel",
    ".knowledge-monitor-summary",
    ".knowledge-monitor-settings-copy",
    ".knowledge-monitor-effective-copy",
    ".knowledge-monitor-quick",
    ".knowledge-monitor-quick-title",
    ".knowledge-monitor-blackout",
    ".knowledge-monitor-error",
    ".knowledge-status",
  ]) {
    assert.equal(body.querySelector(gone), null, `${gone} is not in the DOM`);
  }
  const toolbar = root.querySelector(".workspace-toolbar");
  assert.ok(toolbar.querySelector('[data-action="issue-new"]'), "+ New sits in band 1");
  assert.ok(toolbar.querySelector('[data-action="refresh-knowledge"]'), "↻ sits in band 1");
});

test("AC-23: the Issues summary carries only status, counts and frequent controls", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const bar = body.querySelector(".knowledge-monitor-bar");
  const pill = bar.querySelector(".knowledge-monitor-pill");

  assert.equal(pill.textContent, "Stopped");
  assert.equal(bar.querySelector('[data-action="monitor-toggle"]').textContent, "Start");

  surface.applyIssueMonitorStatus(STATUS);
  assert.equal(pill.textContent, "Running");
  assert.equal(pill.dataset.tone, "active");
  assert.equal(bar.querySelector('[data-metric="active"]').textContent, "Active 1/2");
  assert.equal(bar.querySelector('[data-metric="queue"]').textContent, "Queue 3");
  assert.equal(bar.querySelector('[data-action="monitor-toggle"]').textContent, "Stop");
  assert.ok(bar.querySelector('[data-action="monitor-autonomous"][role="switch"]'));
  for (const selector of [".knowledge-monitor-max-active", '[data-action="monitor-auto-refill"]', '[data-action="monitor-auto-apply"]', ".knowledge-monitor-labels", ".knowledge-monitor-pool"]) {
    assert.equal(body.querySelector(selector), null, `${selector} belongs to Settings`);
  }

  surface.applyIssueMonitorStatus({
    ...STATUS,
    quota_hold: { provider: "codex", reset_at: "2026-09-04T09:30:00Z" },
  });
  assert.equal(pill.textContent, "Waiting · Hold");
  assert.match(pill.title, /Provider codex/);
  assert.match(pill.title, /Reset 2026-09-04T09:30:00Z/);

  surface.applyIssueMonitorStatus({ ...STATUS, enabled: false, state: "disabled" });
  assert.equal(pill.textContent, "Stopped");
  assert.equal(pill.title, "");
});

test("AC-23: ⚙ holds the settings copy and the held fallback in its tooltip", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const gear = body.querySelector('.knowledge-monitor-bar [data-action="monitor-settings"]');
  assert.equal(gear.textContent, "⚙");

  surface.applyIssueMonitorStatus({
    ...STATUS,
    effective_launch_profile: {
      agent_id: "claude",
      summary: "claude / opus / high",
      reason: "codex held until 2026-09-21T08:41:00Z",
    },
  });
  assert.match(gear.title, /^Agent settings Saved: codex \/ gpt-5 \/ high/);
  assert.match(gear.title, /Launching with claude \/ opus \/ high \(codex held until 2026-09-21T08:41:00Z\)/);

  surface.applyIssueMonitorStatus(STATUS);
  assert.equal(gear.title, "Agent settings Saved: codex / gpt-5 / high");
});

test("AC-23: ↻ carries the cached count and the refreshed time in its tooltip", async (t) => {
  const { body, sent, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries: [entry(1, "queued", { queue_position: 1 }), entry(2, null)],
    selected_number: null,
    empty_message: "",
    refresh_enabled: true,
  });
  const refresh = body.querySelector('[data-action="refresh-knowledge"]');
  assert.match(refresh.title, /2 cached/);
  assert.match(refresh.title, /Refreshed \d{2}:\d{2}/);
});

test("Issue #5321: four states distinguish current failures, waiting and stopped history", async (t) => {
  const { body, surface, reported } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const pill = body.querySelector(".knowledge-monitor-pill");
  for (const [status, label, tone] of [
    [{ enabled: false, state: "disabled", last_error: "old error", agent_blackout: "old blackout" }, "Stopped", "idle"],
    [{ state: "idle", active_count: 0, queue_len: 0, last_error: "old error" }, "Waiting · Empty", "idle"],
    [{ active_count: 2 }, "Waiting · Full", "idle"],
    [{ state: "quota_hold", quota_hold: { provider: "codex", reset_at: "2099-01-01T00:00:00Z" }, agent_blackout: "held provider" }, "Waiting · Hold", "idle"],
    [{ state: "active", last_error: "old issue failure" }, "Running", "active"],
    [{ state: "error", last_error: "scan failed" }, "⚠ Error", "blocked"],
  ]) {
    surface.applyIssueMonitorStatus({ ...STATUS, agent_blackout: null, last_error: null, quota_hold: null, ...status });
    assert.equal(pill.textContent, label);
    assert.equal(pill.dataset.tone, tone);
  }
  assert.ok(reported.some(item => item.message === "old error"), "history remains in notifications");
});

// --- T-033 / AC-24 ---------------------------------------------------------

test("AC-24: + New opens a modal-primitive popover that registers and launches", async (t) => {
  const { body, document, sent, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  body.querySelector('[data-action="issue-new"]').click();

  const backdrop = document.querySelector(".modal-backdrop.issue-new-popover");
  assert.ok(backdrop, "the popover is a .modal-backdrop");
  assert.ok(backdrop.classList.contains("open"));
  const dialog = backdrop.querySelector(".modal-shell");
  assert.equal(dialog.getAttribute("role"), "dialog");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  assert.ok(dialog.getAttribute("aria-labelledby"));
  assert.ok(dialog.querySelector(".modal-header"));
  assert.ok(dialog.querySelector(".modal-body"));
  assert.ok(dialog.querySelector(".modal-footer"));

  const title = dialog.querySelector('[data-role="issue-new-title"]');
  const autoMerge = dialog.querySelector('[data-role="issue-new-auto-merge"]');
  assert.equal(autoMerge.type, "checkbox");

  // An empty title sends nothing.
  const before = sent.length;
  dialog.querySelector('[data-action="issue-new-register"]').click();
  assert.equal(sent.length, before);

  title.value = "  Investigate flaky release gate  ";
  autoMerge.checked = true;
  dialog.querySelector('[data-action="issue-new-register-launch"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "quick_register_issue",
    title: "Investigate flaky release gate",
    launch: true,
    auto_merge: true,
  });
  assert.equal(backdrop.classList.contains("open"), false, "the popover closes");

  body.querySelector('[data-action="issue-new"]').click();
  assert.equal(title.value, "", "the popover reopens empty");
  title.value = "Plain register";
  dialog.querySelector('[data-action="issue-new-register"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "quick_register_issue",
    title: "Plain register",
    launch: false,
    auto_merge: false,
  });
});

// --- T-034 / AC-24 ---------------------------------------------------------

test("AC-24: a monitor error links the pill to its center item without adding a row", async (t) => {
  const { body, surface, reported } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const root = body.querySelector(".issue-bridge-root");
  const childCount = root.childElementCount;
  const pill = body.querySelector(".knowledge-monitor-pill");

  surface.applyIssueMonitorStatus({
    ...STATUS,
    state: "error",
    last_error: "issue #3785: scan failed\nstack line",
  });
  assert.equal(pill.textContent, "⚠ Error");
  assert.equal(pill.dataset.tone, "blocked");
  assert.equal(pill.title, "Details in notifications");
  assert.equal(pill.dataset.notificationKey, "issue-monitor:last_error");
  assert.equal(root.childElementCount, childCount, "no row is added to the toolbar");
  assert.deepEqual(reported.at(-1), {
    key: "issue-monitor:last_error",
    title: "Issue Monitor",
    message: "issue #3785: scan failed\nstack line",
  });
});

test("AC-24: a fleet outage is announced through the pill and the center, not a banner row", async (t) => {
  const { body, surface, reported, resolved } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const pill = body.querySelector(".knowledge-monitor-pill");
  const outage = "No implementation agent has been running for 1800s while 9 issue(s) were runnable";

  surface.applyIssueMonitorStatus({ ...STATUS, agent_blackout: outage });
  assert.equal(pill.textContent, "⚠ Error");
  assert.equal(pill.title, "Details in notifications");
  assert.equal(pill.dataset.notificationKey, "issue-monitor:agent_blackout");
  assert.deepEqual(reported.at(-1), {
    key: "issue-monitor:agent_blackout",
    title: "Issue Monitor",
    message: outage,
  });

  surface.applyIssueMonitorStatus(STATUS);
  assert.equal(pill.textContent, "Running");
  assert.ok(resolved.includes("issue-monitor:agent_blackout"));
});

// --- T-035 / AC-25 ---------------------------------------------------------

test("AC-25: issueAcceptanceProgress counts only `- [ ] AC-N:` lines", async () => {
  const { issueAcceptanceProgress } = await importSurfaceModule();
  const progress = issueAcceptanceProgress([
    "## 受け入れ基準",
    "- [x] AC-1: first",
    "- [ ] AC-2: second",
    "  - [X] AC-10: nested done",
    "- [ ] AC-Z: not numbered",
    "- [ ] Not an AC",
    "* [x] AC-3: star bullet",
  ].join("\n"));
  assert.equal(progress.total, 4);
  assert.equal(progress.done, 3);
  assert.deepEqual(
    progress.items.map((item) => [item.id, item.done, item.text]),
    [
      ["AC-1", true, "first"],
      ["AC-2", false, "second"],
      ["AC-10", true, "nested done"],
      ["AC-3", true, "star bullet"],
    ],
  );
  assert.deepEqual(issueAcceptanceProgress(""), { items: [], done: 0, total: 0 });
  assert.deepEqual(issueAcceptanceProgress(null), { items: [], done: 0, total: 0 });
});

test("AC-25: issueDetailActionModel gives each state its FR-023 action set", async () => {
  const { issueDetailActionModel } = await importSurfaceModule();

  const queued = issueDetailActionModel({
    entry: entry(1, "queued", { queue_position: 2 }),
    queue: { index: 1, length: 3 },
  });
  assert.equal(queued.phase, "queue");
  assert.deepEqual(queued.actions, ["launch-now", "move-to-top"]);
  assert.ok(queued.actions.length <= 3);

  const liveWindow = { id: "agent-1", status: "running", placement: { kind: "canvas" } };
  const running = issueDetailActionModel({
    entry: entry(2, "launched"),
    canvasWindow: liveWindow,
  });
  assert.equal(running.phase, "running");
  assert.deepEqual(running.actions, ["open-window"]);
  assert.ok(running.overflow.includes("stop-agent"), "Stop lives behind ⋯");
  assert.equal(running.actions.includes("stop-agent"), false);

  const failed = issueDetailActionModel({ entry: entry(3, "agent_failed") });
  assert.ok(
    [...failed.actions, ...failed.overflow].includes("requeue-issue"),
    "Requeue stays reachable",
  );

  const done = issueDetailActionModel({
    entry: entry(4, "merged"),
    work: { id: "w", pr_number: 7, pr_url: "https://github.com/o/r/pull/7" },
  });
  assert.equal(done.phase, "done");
  assert.deepEqual(done.actions, ["open-pr"]);

  const backlog = issueDetailActionModel({ entry: entry(5, null) });
  assert.ok(backlog.actions.includes("launch-agent"));
  for (const model of [queued, running, failed, done, backlog]) {
    assert.ok(model.actions.length <= 3, "at most 3 visible actions");
  }
});

test("AC-25: the detail pane renders in FR-023 order with the row's badge vocabulary", async (t) => {
  const { body, sent, surface, state } = await makeFixture({
    surface: {
      getActiveWorkProjection: () => ({
        active_works: [
          { id: "work-1", branch: "work/issue-11", pr_number: 9, pr_state: "OPEN", pr_url: "https://x/pull/9" },
        ],
      }),
    },
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const queued = entry(11, "queued", {
    queue_position: 2,
    related_work_refs: [{ id: "work-1", branch: "work/issue-11" }],
  });
  state.entries = [entry(10, "queued", { queue_position: 1 }), queued];
  state.baseEntries = state.entries.slice();
  state.selectedNumber = 11;
  state.previewHidden = false;
  state.detail = {
    number: 11,
    title: "Issue 11",
    subtitle: "#11 · Open",
    state: "open",
    labels: ["bug"],
    launch_issue_number: 11,
    sections: [
      { title: "Description", body: "## Summary\n\nbody\n\n- [x] AC-1: one\n- [ ] AC-2: two" },
      { title: "Comment 1", body: "a comment" },
    ],
    related_works: [],
  };
  surface.renderKnowledgeBridge("win-1");

  const pane = body.querySelector(".knowledge-detail-pane");
  const order = [
    ".issue-detail-id",
    ".knowledge-detail-title",
    ".issue-detail-status",
    ".issue-detail-actions",
    ".issue-detail-ac",
    ".issue-detail-body",
  ].map((selector) => {
    const node = pane.querySelector(selector);
    assert.ok(node, `${selector} renders`);
    return node;
  });
  const documentOrder = [...pane.querySelectorAll("*")];
  const positions = order.map((node) => documentOrder.indexOf(node));
  assert.deepEqual(
    positions,
    positions.slice().sort((left, right) => left - right),
    "FR-023 order is kept",
  );

  assert.match(order[0].textContent, /#11/);
  assert.match(order[0].textContent, /work\/issue-11/);
  const pill = order[2].querySelector(".knowledge-row-badge");
  const row = body.querySelector('.knowledge-row[data-issue-number="11"] .knowledge-row-badge');
  assert.equal(pill.textContent, row.textContent, "same vocabulary as the row badge");
  assert.match(order[2].textContent, /PR #9/);

  const actionIds = [...order[3].querySelectorAll(":scope > [data-action]")].map(
    (node) => node.dataset.action,
  );
  assert.deepEqual(actionIds, ["launch-now", "move-to-top"]);

  const gauge = order[4].querySelector('[role="progressbar"]');
  assert.equal(gauge.getAttribute("aria-valuenow"), "1");
  assert.equal(gauge.getAttribute("aria-valuemax"), "2");
  assert.match(order[4].textContent, /1 \/ 2/);
  assert.equal(order[4].querySelectorAll("li").length, 2);

  const sections = [...pane.querySelectorAll(".issue-detail-body details.knowledge-section")];
  assert.equal(sections.length, 2);
  assert.equal(sections[0].open, true, "the summary section is expanded");
  assert.equal(sections[1].open, false, "other sections are folded to their heading");

  order[3].querySelector('[data-action="move-to-top"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_queue_move",
    issue_number: 11,
    position: 0,
  });
});

test("Issue #5321: the default board has no open preview", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  assert.equal(body.querySelector(".issue-bridge-root").dataset.previewHidden, "true");
  assert.equal(body.querySelector(".knowledge-detail-pane").hidden, true);
  assert.equal(body.querySelector('[data-action="toggle-issue-preview"]').hidden, true);
});

// --- AC-28: tokens only ----------------------------------------------------

test("AC-28: the Phase 5 CSS uses Operator tokens only", () => {
  const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");
  const start = css.indexOf("/* SPEC #3885 Phase 5");
  const end = css.indexOf("/* /SPEC #3885 Phase 5 */");
  assert.ok(start >= 0 && end > start, "the Phase 5 block is delimited");
  const block = css.slice(start, end).replace(/\/\*[\s\S]*?\*\//g, "");
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b/, "no raw hex");
  assert.doesNotMatch(block, /\brgba?\(/, "no raw rgb");
  assert.doesNotMatch(block, /\bposition:\s*fixed/, "no private overlay shell");
});

test("Issue #5284: capacity uses a linked badge and Operator tokens without an overlay", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const badge = body.querySelector(".knowledge-monitor-capacity-badge");
  assert.equal(badge?.tagName, "BUTTON");
  assert.equal(badge.getAttribute("aria-haspopup"), "dialog");
  assert.equal(body.querySelector(".knowledge-monitor-capacity"), null);
  assert.equal(body.querySelector(".knowledge-monitor-capacity-warning"), null);
  const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");
  const block = css.split("/* Issue #5284: linked status only; diagnostics live in notifications. */")[1]?.split(".issue-new-body")[0];
  assert.ok(block, "linked capacity and center Details styles are present");
  assert.match(block, /white-space:\s*nowrap/);
  assert.match(block, /var\(--color-state-needs-input\)/);
  assert.match(block, /var\(--space-/);
  assert.match(block, /var\(--type-/);
  assert.doesNotMatch(block, /#[\da-f]{3,8}\b|\brgba?\(|\bposition:\s*(?:fixed|absolute)/i);
});

test("queue detail provenance follows authoritative terminal queue", async (t) => {
  const { body, surface, state } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  state.entries = [entry(11, "queued", { queued_by: "operator" })];
  state.selectedNumber = 11;
  state.previewHidden = false;
  state.detail = { number: 11, title: "Issue 11", labels: [], sections: [] };
  surface.applyIssueMonitorStatus({ ...STATUS, terminal_queue: [{ number: 11, queued_by: "auto-refill" }] });
  surface.renderKnowledgeBridge("win-1");
  assert.equal(body.querySelector(".issue-detail-provenance")?.textContent, "Queued by: Auto-refill");
});


test("Issue #5321: responsive preview reuses the shared nonmodal dialog primitive and tokens", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const pane = body.querySelector(".knowledge-detail-pane");
  assert.ok(pane.classList.contains("modal-shell"));
  assert.equal(pane.getAttribute("role"), "dialog");
  assert.equal(pane.getAttribute("aria-modal"), "false");
  assert.ok(pane.getAttribute("aria-label"));
  const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");
  const block = css.split("/* Issue #5321: compact summary and selection preview. */")[1]?.split("/* /Issue #5321 */")[0];
  assert.ok(block);
  assert.match(block, /@container/);
  assert.match(block, /@container[^}]+\.issue-bridge-root\[data-view-mode="list"\]/, "only Kanban uses the narrow overlay; Split keeps its two panes");
  assert.match(block, /-webkit-line-clamp:\s*2/);
  assert.doesNotMatch(block, /#[\da-f]{3,8}\b|\brgba?\(|\bposition:\s*(?:fixed|absolute)/i);
});

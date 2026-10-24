// SPEC #5016 — Issue Monitor rows combine cached KnowledgeListItem content
// with live inbox lifecycle state in the shared presentation model.

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
    `data:text/javascript;base64,${Buffer.from(source.replace('from "./agent-kanban-surface.js"', `from "${new URL("../agent-kanban-surface.js", import.meta.url).href}"`).replace('from "./control-explanations.js"', `from "${new URL("../control-explanations.js", import.meta.url).href}"`).replace('from "./ui-state-store.js"', `from "${new URL("../ui-state-store.js", import.meta.url).href}"`).replace('from "./ui-content.js"', `from "${new URL("../ui-content.js", import.meta.url).href}"`)).toString("base64")}`
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
  const windowMap = new Map([[windowData.id, body]]);
  const sent = [];
  const notices = [];
  const opened = [];
  const reports = [];
  const surface = mod.createKnowledgeKanbanSurface({
    scheduleFrame: run => { run(); return () => {}; },
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (...args) => createNode(document, ...args),
    createKnowledgeMarkdownBody: () => document.createElement("div"),
    windowMap,
    workspaceWindowById: (id) => (id === windowData.id ? windowData : null),
    getWorkspaceWindows: () => [windowData],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => "issue",
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    launchPending: {},
    reportSurfaceNotice: notice => notices.push(notice),
    reportSurfaceError: error => reports.push(error),
    openSurfaceNotice: key => { opened.push(key); return true; },
    ...options,
  });
  surface.mountKnowledgeWindow(windowData, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return { body, document, mod, sent, surface, load, windowMap, notices, opened, reports };
}

test("Issue #5287: inbox-first startup renders every column despite GUI error and enriches late cache rows", async (t) => {
  const { body, surface, load } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyIssueMonitorStatus({ enabled: true, state: "error", terminal_queue: [{ number: 2 }] });
  surface.applyIssueMonitorInbox([1, 2, 3].map(number => ({
    issue: { number, title: `Inbox issue ${number}`, state: "open", labels: ["bug"] },
    state: number === 3 ? "launched" : "queued",
    exclusion_reason: number === 1 ? "Held for acceptance review" : null,
  })));
  for (const [number, phase] of [[1, "backlog"], [2, "queued"], [3, "active"]]) {
    assert.equal(body.querySelectorAll(`[data-queue-column="${phase}"] [data-issue-number="${number}"]`).length, 1);
  }
  assert.match(body.querySelector('[data-issue-number="1"]').textContent, /Held for acceptance review/);
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, entries: [{ ...knowledgeEntry(2, "queued", 1), state: "closed", title: "Enriched cache issue" }],
    refresh_enabled: true });
  assert.equal(body.querySelectorAll(".knowledge-row").length, 3, "late cache must not remove inbox-only rows or duplicate identities");
  assert.match(body.querySelector('[data-issue-number="2"]').textContent, /Enriched cache issue/);
  assert.equal(body.querySelectorAll('[data-queue-column="queued"] [data-issue-number="2"]').length, 1,
    "the current open inbox wins over stale cached closure");
  surface.applyIssueMonitorInbox([]);
  assert.equal(body.querySelectorAll(".knowledge-row").length, 1, "removed inbox-only rows must not become sticky cache entries");
});

test("Issue #5287: an initial load failure shows its cause instead of empty columns and reports a ledger diagnostic", async (t) => {
  let missing = false;
  const opened = [];
  const { body, surface, load, sent, reports } = await makeFixture({ openSurfaceNotice(key) {
    opened.push(key);
    const found = !missing;
    missing = false;
    return found;
  } });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_error", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, message: "Issue cache could not be read" });
  const badge = body.querySelector('.knowledge-list .knowledge-notification-badge');
  assert.match(badge?.textContent || "", /Issue load failed.*Details/);
  assert.doesNotMatch(body.textContent, /Issue cache could not be read/);
  assert.equal(reports.at(-1).message, "Issue cache could not be read");
  badge.click();
  assert.equal(opened.at(-1), "issue-window:win-1:load");
  const reportCount = reports.length;
  missing = true; // The operator cleared or dismissed the existing row.
  badge.click();
  assert.equal(reports.length, reportCount + 1);
  assert.equal(reports.at(-1).message, "Issue cache could not be read");
  assert.deepEqual(opened, Array(3).fill("issue-window:win-1:load"));
  assert.ok(sent.some(event => event.kind === "report_knowledge_error" && event.id === "win-1" &&
    event.stage === "load" && event.message === "Issue cache could not be read"));
});

test("Issue #5287: a failed Issue renderer displays and reports the cause, then recovers", async (t) => {
  let failRow = false;
  const { body, surface, load, sent } = await makeFixture({ createNode(...args) {
    if (failRow && String(args[1]).split(" ").includes("knowledge-row")) {
      failRow = false;
      throw new Error("Issue card renderer failed");
    }
    return createNode(globalThis.document, ...args);
  } });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  failRow = true;
  assert.throws(() => surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, entries: [knowledgeEntry(1, "queued")], refresh_enabled: true }), /Issue card renderer failed/);
  assert.match(body.querySelector('.knowledge-list .knowledge-notification-badge')?.textContent || "", /Issue rendering failed.*Details/);
  assert.doesNotMatch(body.textContent, /Issue card renderer failed/);
  assert.ok(sent.some(event => event.kind === "report_knowledge_error" && event.stage === "render" &&
    event.message === "Issue card renderer failed"));
  surface.applyIssueMonitorStatus({ enabled: true, state: "idle" });
  assert.equal(body.querySelectorAll('.knowledge-list [role="alert"]').length, 0);
  assert.equal(body.querySelectorAll(".knowledge-row").length, 1);
});

for (const stage of ["load", "render"]) {
  test(`Issue #5287: detail-only ${stage} failure stays visible and reaches diagnostics`, async (t) => {
    const { body, surface, load, sent, reports, opened } = await makeFixture({
      createNode(...args) {
        if (args[1] === "knowledge-section") throw new Error("detail Markdown failed");
        return createNode(globalThis.document, ...args);
      },
    });
    t.after(() => surface.clearKnowledgeBridgeState("win-1"));
    surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
      request_id: load.request_id, entries: [knowledgeEntry(5287, "queued", 1)], refresh_enabled: true });
    body.querySelector('[data-issue-number="5287"] .knowledge-row-select').click();
    const request = sent.findLast(message => message.kind === "select_knowledge_bridge_entry");
    const event = { id: "win-1", knowledge_kind: "issue", request_id: request.request_id };
    if (stage === "load") {
      surface.applyKnowledgeReceiveEvent({ ...event, kind: "knowledge_error", message: "detail load failed" });
    } else {
      assert.throws(() => surface.applyKnowledgeReceiveEvent({ ...event, kind: "knowledge_detail",
        detail: { number: 5287, title: "Detail", state: "open", labels: [],
          sections: [{ title: "Description", body: "Markdown" }], related_works: [] } }), /detail Markdown failed/);
    }
    assert.match(body.querySelector('.knowledge-detail-pane .knowledge-notification-badge')?.textContent || "", /Issue .* failed.*Details/);
    assert.doesNotMatch(body.querySelector(".knowledge-detail-pane").textContent, /detail .* failed/);
    assert.ok(reports.some(report => report.key === `issue-window:win-1:${stage}` && report.message.includes("detail")));
    body.querySelector('.knowledge-detail-pane .knowledge-notification-badge').click();
    assert.equal(opened.at(-1), `issue-window:win-1:${stage}`);
    assert.ok(sent.some(message => message.kind === "report_knowledge_error" && message.stage === stage
      && message.message.includes("detail")));
  });
}

test("a failed Issue window renderer does not leave another window on the previous inbox", async (t) => {
  let failRow = false;
  const fixture = await makeFixture({ createNode(...args) {
    if (failRow && String(args[1]).split(" ").includes("knowledge-row")) {
      failRow = false;
      throw new Error("first Issue view failed");
    }
    return createNode(globalThis.document, ...args);
  } });
  const { surface, document, windowMap, sent, load } = fixture;
  const second = document.createElement("div");
  document.body.appendChild(second);
  windowMap.set("win-2", second);
  surface.mountKnowledgeWindow({ id: "win-2", preset: "issue" }, second);
  t.after(() => ["win-1", "win-2"].forEach(id => surface.clearKnowledgeBridgeState(id)));
  for (const request of [load, sent.findLast(message => message.kind === "load_knowledge_bridge")]) {
    surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: request.id, knowledge_kind: "issue",
      request_id: request.request_id, entries: [knowledgeEntry(5016, "queued", 1)], refresh_enabled: true });
  }
  surface.applyIssueMonitorStatus({ terminal_queue: [{ number: 5016 }] });
  failRow = true;
  assert.throws(() => surface.applyIssueMonitorInbox([{ issue: { number: 5016 }, state: "launched" }]), /first Issue view failed/);
  assert.equal(second.querySelectorAll('[data-queue-column="active"] .knowledge-row').length, 1);
  assert.equal(second.querySelectorAll('[data-queue-column="queued"] .knowledge-row').length, 0);
});

test("a failed Monitor notification cannot prevent the controls from reading committed status", async (t) => {
  const { body, surface } = await makeFixture({ reportSurfaceError() { throw new Error("notification failed"); } });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  assert.throws(() => surface.applyIssueMonitorStatus({ enabled: true, state: "active", queue_len: 1, active_count: 1, max_active_agents: 2, last_error: "scan failed" }), /notification failed/);
  assert.equal(monitorBand(body).pill.textContent, "Running");
});

test("a failed initial window subscription can be mounted again without leaving a partial registration", async (t) => {
  let failInitial = false;
  const { surface, document, windowMap, sent } = await makeFixture({ renderOtherWork() {
    if (failInitial) { failInitial = false; throw new Error("initial view failed"); }
  } });
  t.after(() => ["win-1", "win-2"].forEach(id => surface.clearKnowledgeBridgeState(id)));
  const second = document.createElement("div");
  document.body.appendChild(second);
  windowMap.set("win-2", second);
  failInitial = true;
  assert.throws(() => surface.mountKnowledgeWindow({ id: "win-2", preset: "issue" }, second), /initial view failed/);
  surface.mountKnowledgeWindow({ id: "win-2", preset: "issue" }, second);
  const load = sent.findLast(message => message.kind === "load_knowledge_bridge");
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-2", knowledge_kind: "issue",
    request_id: load.request_id, entries: [knowledgeEntry(5016, "queued", 1)], refresh_enabled: true });
  surface.applyIssueMonitorInbox([{ issue: { number: 5016 }, state: "launched" }]);
  assert.equal(second.querySelectorAll('[data-queue-column="active"] .knowledge-row').length, 1);
});

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
    ["source_excluded", ["Needs triage", "needs-input"]],
    ["launching", ["Launching", "active"]],
    ["launched", ["Launched", "active"]],
    ["merged", ["Merged", "done"]],
    ["released", ["Released", "done"]],
    ["launch_failed", ["Launch failed", "blocked"]],
    ["agent_failed", ["Agent failed", "blocked"]],
    ["blocked_by_claim", ["Blocked by claim", "needs-input"]],
    ["blocked_by_dependency", ["Blocked by dependency", "needs-input"]],
    ["skipped", ["Skipped", "idle"]],
    ["needs_human", ["Needs human", "needs-input"]],
  ]);

  for (const [state, [label, tone]] of expected) {
    assert.deepEqual(monitorStateView(state), { state, label, tone });
  }
  assert.deepEqual(monitorStateView("hold_excluded"), {
    state: "source_excluded", label: "Needs triage", tone: "needs-input",
  });
  assert.deepEqual(monitorStateView("awaiting_review"), {
    state: "awaiting_review",
    label: "Unknown (awaiting_review)",
    tone: "needs-input",
  });
  assert.equal(monitorStateView(null), null);
  assert.equal(monitorStateView(""), null);
});

// SPEC #3885 Phase 5 (T-032): the monitor summary line became the Monitor
// band — one state pill plus Active / Queue metrics; quota-hold detail lives in
// the pill tooltip.
function monitorBand(body) {
  const bar = body.querySelector(".knowledge-monitor-bar");
  const pill = bar.querySelector(".knowledge-monitor-pill");
  return {
    pill,
    text: () =>
      [
        pill.textContent,
        bar.querySelector('[data-metric="queue"]').textContent,
        bar.querySelector('[data-metric="active"]').textContent,
      ].join(" | "),
  };
}

test("Issue Monitor band presents and clears the quota-hold provider and reset", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const band = monitorBand(body);

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

  assert.equal(band.text(), "Waiting · Hold | Queue 3 | Active 0/2");
  assert.equal(band.pill.title, "Provider codex | Reset 2026-09-04T09:30:00Z");

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "idle",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    launch_profile_source: "saved",
    launch_profile_summary: "configured",
  });

  assert.equal(band.text(), "Running | Queue 3 | Active 0/2");
  assert.equal(band.pill.title, "");
});

test("Issue #5603: GitHub API read limits explain the reason and retry time, then clear", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const band = monitorBand(body);
  const status = { enabled: true, state: "idle", queue_len: 3, active_count: 0,
    max_active_agents: 2, last_error: null };
  const budget = {
    core: { throttled: true, reason: "budget_reserve: resource=core remaining=0 reserve=1000",
      backoff_until: "2026-10-16T01:59:53Z" },
    graphql: { throttled: false },
  };
  surface.applyIssueMonitorStatus({ ...status, github_budget: budget });
  assert.equal(band.pill.textContent, "Limited · GitHub");
  assert.equal(band.pill.dataset.tone, "idle");
  assert.match(band.pill.title, /GitHub API reads limited.*core.*budget_reserve/);
  assert.match(band.pill.title, /Retry 2026-10-16T01:59:53Z/);

  surface.applyIssueMonitorStatus({ ...status, state: "error", last_error: "unrelated scan fault", github_budget: budget });
  assert.equal(band.pill.textContent, "⚠ Error");
  assert.match(band.pill.title, /Details in notifications.*GitHub API reads limited.*core.*budget_reserve/);
  assert.match(band.pill.title, /Retry 2026-10-16T01:59:53Z/);

  surface.applyIssueMonitorStatus({ ...status, enabled: false, state: "disabled" });
  assert.equal(band.pill.textContent, "Stopped");
  assert.equal(band.pill.title, "");
  surface.applyIssueMonitorStatus({ ...status, state: "idle", last_error: null });
  assert.equal(band.pill.textContent, "Running");
  assert.equal(band.pill.title, "");
});

// Issue #4366 AC-6 / AC-6b: a hold never rewrites the saved settings line; the
// launch target and its reason are a separate line (now of the ⚙ tooltip) that
// exists only while held.
test("Issue Monitor keeps the saved agent settings and shows the held fallback separately", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const gear = body.querySelector('.knowledge-monitor-bar [data-action="monitor-settings"]');
  const saved = {
    enabled: true,
    state: "active",
    queue_len: 1,
    active_count: 1,
    max_active_agents: 1,
    launch_profile_source: "saved",
    launch_profile_summary: "codex / gpt-5 / high",
  };

  surface.applyIssueMonitorStatus({
    ...saved,
    effective_launch_profile: {
      index: 1,
      agent_id: "claude",
      summary: "claude / opus / high",
      reason: "codex held until 2026-09-21T08:41:00Z; re-verification launch at 2026-09-15T10:00:00Z",
    },
  });

  const [settingsLine, effectiveLine] = gear.title.split("\n");
  assert.equal(settingsLine, "Agent settings Saved: codex / gpt-5 / high");
  assert.match(effectiveLine, /^Launching with claude \/ opus \/ high/);
  assert.match(effectiveLine, /codex held until 2026-09-21T08:41:00Z/);
  assert.match(effectiveLine, /re-verification launch at 2026-09-15T10:00:00Z/);

  surface.applyIssueMonitorStatus(saved);

  assert.equal(gear.title, "Agent settings Saved: codex / gpt-5 / high");
});

test("Issue Monitor renders the JSON gui_status contract and follows updated limits", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const response = {
    queue: [42, 43], active_launches: [44], max_active: 4,
    gui_status: {
      enabled: true, state: "active", queue_len: 2, active_count: 1,
      max_active_agents: 4, auto_apply_updates: true, last_error: null,
    },
  };
  surface.applyIssueMonitorStatus(response.gui_status);
  surface.applyIssueMonitorInbox([{ issue: { number: 44, title: "Running issue", state: "open" }, state: "launched" }]);
  assert.match(monitorBand(body).text(),
    new RegExp(`Queue ${response.queue.length} \\| Active ${response.active_launches.length}/${response.max_active}`));
  surface.applyIssueMonitorStatus({ ...response.gui_status, max_active_agents: 5 });
  assert.match(monitorBand(body).text(), /Active 1\/5/);
});

function capacityStatus(overrides = {}) {
  return {
    enabled: true, state: "idle", active_count: 0, max_active_agents: 3,
    max_active_agents_override: null,
    agent_capacity: {
      measurement_complete: true, machine_budget: 6, recommended_worker_limit: 3,
      recommended_implementation_count: 4, recommended_total_count: 5,
      machine_live_agents: 5, own_live_agents: 3, own_pm_agents: 1, other_live_agents: 2,
      gui_cpu_millicores: 375, limiting_constraint: "cpu",
      reason: "CPU limits capacity after GUI, other projects and PM reservations.",
      constraints: [{ resource: "cpu", capacity: 6, binding: true, reason: "GUI CPU reserved" }],
    },
    ...overrides,
  };
}

test("Issue #3620: capacity details open in the center without generating passive healthy notices", async (t) => {
  const { body, surface, notices, opened } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const status = capacityStatus({ max_active_agents: 0 });
  status.agent_capacity.recommended_worker_limit = 0;
  status.agent_capacity.constraints.push({
    resource: "disk", capacity: 18446744073709551615, binding: false,
    reason: "measured target 0 bytes; no additional target storage required",
  });
  surface.applyIssueMonitorStatus(status);
  assert.equal(body.querySelector('[data-metric="active"]').textContent, "Active 0/0");
  assert.equal(notices.length, 0, "healthy capacity does not create a notification on opening a window");
  body.querySelector(".knowledge-monitor-capacity-badge").click();
  assert.deepEqual(opened, ["issue-monitor:capacity"]);
  assert.match(notices[0].details, /0 monitor workers.*4 implementation agents.*5 total including PM/);
  assert.match(notices[0].details, /Machine budget: 6.*Machine live: 5.*This project: 3.*PM: 1.*Other projects: 2/);
  assert.match(notices[0].details, /GUI CPU reserved: 0\.375 cores/);
  assert.match(notices[0].details, /CPU: 6.*limiting.*GUI CPU reserved/);
  assert.match(notices[0].details, /Disk: unlimited agent slots.*no additional target storage required/);
  assert.equal(body.querySelector(".knowledge-monitor-capacity"), null, "no inline diagnostic block");
});

test("Issue #3620: capacity badges follow confirmed manual and automatic limits", async (t) => {
  const { body, surface, notices } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyIssueMonitorStatus(capacityStatus({ max_active_agents: 7, max_active_agents_override: 7 }));
  assert.equal(body.querySelector('[data-metric="active"]').textContent, "Active 0/7");
  const badge = body.querySelector(".knowledge-monitor-capacity-badge");
  assert.equal(badge.hidden, false);
  assert.match(badge.textContent, /Over recommended.*Details/);
  assert.equal(badge.dataset.tone, "info");
  assert.match(notices.at(-1).message, /Lower Max active if slow/);
  assert.match(notices.at(-1).details, /Verification may not finish/);
  const sharedManual = capacityStatus({ max_active_agents: 3, max_active_agents_override: 5 });
  const noticeCount = notices.length;
  surface.applyIssueMonitorStatus(sharedManual);
  assert.equal(badge.textContent, "Capacity 3 · Details", "other-project consumption does not make the shared request excessive");
  assert.equal(notices.length, noticeCount, "the request fits the machine worker recommendation of five");
  surface.applyIssueMonitorStatus({ ...sharedManual,
    agent_capacity: { ...sharedManual.agent_capacity, measurement_complete: false } });
  assert.equal(badge.dataset.tone, "info", "Manual remains configured when the project receives fewer workers");
  assert.equal(notices.at(-1).level, "info");
  assert.match(notices.at(-1).message, /Capacity pending.*Manual limit 5.*no action needed/);
  surface.applyIssueMonitorStatus(capacityStatus());
  assert.equal(body.querySelector('[data-metric="active"]').textContent, "Active 0/3");
  surface.applyIssueMonitorStatus({ max_active_agents: 3, max_active_agents_override: null });
  assert.equal(badge.hidden, true, "omitted capacity cannot retain a stale badge");
});

test("Issue #5284: capacity uses one keyed notice with unique center-only diagnostics and a linked badge", async (t) => {
  const { body, surface, notices, opened } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const diskReason = "Disk measurement unavailable; check that the worktree volume is mounted and free at least 4 GiB.";
  const reason = `inventory_complete=false; own_pm=0; child_identity_missing ×37; target: /Users/example/target; ${diskReason}; ${diskReason}`;
  const status = capacityStatus({ max_active_agents: 9, max_active_agents_override: 9 });
  status.agent_capacity = { ...status.agent_capacity, measurement_complete: false, machine_budget: null,
    recommended_worker_limit: 0, reason, constraints: [
      { resource: "disk", capacity: null, binding: true, reason: diskReason },
    ] };
  surface.applyIssueMonitorStatus(status);
  assert.equal(body.querySelector('[data-metric="active"]').textContent, "Active 0/9");
  const badge = body.querySelector(".knowledge-monitor-capacity-badge");
  assert.equal(badge.textContent, "Capacity pending · Details");
  assert.equal(badge.dataset.tone, "info");
  assert.doesNotMatch(body.textContent, /inventory_complete|own_pm|child_identity|\/Users\//);
  assert.equal(body.querySelector(".knowledge-monitor-capacity-warning"), null);
  assert.equal(body.querySelector(".knowledge-monitor-capacity"), null);
  assert.equal(notices.length, 1);
  assert.equal(notices[0].key, "issue-monitor:capacity");
  assert.equal(notices[0].level, "info");
  assert.match(notices[0].message, /Capacity pending.*manual limit 9.*no action needed/i);
  assert.match(notices[0].details, /Machine budget: unknown[\s\S]*Disk: unknown/);
  assert.ok(notices[0].details.includes("/Users/example/target"));
  assert.equal(notices[0].details.split(diskReason).length - 1, 1);
  assert.equal(notices[0].details.split("child_identity_missing").length - 1, 1);
  surface.applyIssueMonitorStatus(status);
  assert.equal(notices.length, 1, "identical broadcasts do not report another occurrence");
  badge.click();
  assert.equal(opened.at(-1), "issue-monitor:capacity");
  surface.applyIssueMonitorStatus({ ...status, max_active_agents_override: null, max_active_agents: 9 });
  assert.equal(badge.dataset.tone, "info");
  assert.equal(notices.at(-1).level, "info");
  assert.match(notices.at(-1).message, /Auto limit 9 retained/);
  surface.applyIssueMonitorStatus({ ...status, max_active_agents_override: null, max_active_agents: 0 });
  assert.equal(badge.dataset.tone, "warning");
  assert.equal(notices.at(-1).key, notices[0].key);
  assert.equal(notices.at(-1).level, "warn");
  assert.match(notices.at(-1).message, /Wait or set Max active manually/);
});

test("Issue #3620: the Operator strip preserves a zero effective worker cap", async () => {
  const source = readFileSync(resolve(here, "../operator-shell.js"), "utf8")
    .replace('from "/theme-manager.js"', `from "${new URL("../theme-manager.js", import.meta.url).href}"`)
    .replace('from "/hotkey.js"', `from "${new URL("../hotkey.js", import.meta.url).href}"`)
    .replace('from "/theme-toggle.js"', `from "${new URL("../theme-toggle.js", import.meta.url).href}"`);
  const { applyIssueMonitorStatus } = await import(`data:text/javascript,${encodeURIComponent(source)}`);
  const { document } = parseHTML('<div id="op-strip-issue-monitor"><span id="op-strip-issue-monitor-value"></span></div>');
  applyIssueMonitorStatus(document, { enabled: true, state: "idle", queue_len: 2, active_count: 0, max_active_agents: 0 });
  assert.equal(document.getElementById("op-strip-issue-monitor-value").textContent, "Idle Q2 A0/0");
});

test("Issue Monitor band preserves higher-priority states around quota-hold metadata", async (t) => {
  const { body, surface } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const band = monitorBand(body);
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

  assert.equal(band.text(), "⚠ Error | Queue 3 | Active 0/2");
  assert.equal(band.pill.title, "Details in notifications");
  // FR-017: no red monitor banner; the error text is read in the
  // notification center, and the pill only says that one exists.
  assert.equal(body.querySelector(".knowledge-monitor-error"), null);
  assert.equal(body.querySelector(".surface-error-indicator"), null);

  surface.applyIssueMonitorStatus({
    enabled: false,
    state: "disabled",
    queue_len: 3,
    active_count: 0,
    max_active_agents: 2,
    quota_hold: quotaHold,
  });

  assert.equal(band.text(), "Stopped | Queue 3 | Active 0/2");
  assert.equal(band.pill.title, "");

  surface.applyIssueMonitorInbox([{ issue: { number: 44, title: "Running issue", state: "open" }, state: "launched" }]);
  for (const state of ["active", "launching"]) {
    surface.applyIssueMonitorStatus({
      enabled: true,
      state,
      queue_len: 3,
      active_count: 1,
      max_active_agents: 2,
      quota_hold: quotaHold,
    });

    assert.equal(band.text(), "Waiting · Hold | Queue 3 | Active 1/2");
    assert.equal(band.pill.title, "Provider codex | Reset 2026-09-04T09:30:00Z");
  }

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "launching",
    queue_len: 3,
    active_count: 1,
    max_active_agents: 2,
    quota_hold: {},
  });

  assert.equal(band.text(), "Running | Queue 3 | Active 1/2");
  assert.doesNotMatch(band.pill.title, /Quota hold|Provider|Reset|undefined/);
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
      knowledgeEntry(45, "source_excluded", null, {
        exclusionReason: "untrusted external Issue; PM triage required; use a separate curated execution owner",
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
  assert.equal(row45.querySelector(".knowledge-row-badge").textContent, "Needs triage");
  assert.match(row45.textContent, /untrusted external Issue; PM triage required; use a separate curated execution owner/);
  assert.match(row45.querySelector(".knowledge-row-badge").dataset.issueExplanation, /source.*trust.*triage/i);
  assert.doesNotMatch(row45.querySelector(".knowledge-row-badge").dataset.issueExplanation, /hold label/i);
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
    kind: "issue_monitor_queue_move",
    issue_number: 44,
    position: 0,
  });

  for (const selector of [
    ".knowledge-monitor-max-active", ".knowledge-monitor-labels", ".knowledge-monitor-pool",
    '[data-action="monitor-auto-apply"]',
    '[data-action="monitor-capacity-auto"]',
  ]) assert.equal(body.querySelector(selector), null, `${selector} belongs in Settings`);

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
  let settingsSection;
  document.addEventListener("settings:open", event => { settingsSection = event.detail.section; });
  const gear = body.querySelector('[data-action="monitor-settings"]');
  assert.equal(gear.getAttribute("aria-label"), "Issues settings");
  gear.click();
  assert.equal(settingsSection, "issues");

  // SPEC #3885 T-033: quick register moved into the "+ New" popover.
  body.querySelector('[data-action="issue-new"]').click();
  document.querySelector('[data-role="issue-new-title"]').value = "Investigate flaky release gate";
  document.querySelector('[data-action="issue-new-register-launch"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "quick_register_issue",
    title: "Investigate flaky release gate",
    launch: true,
    auto_merge: false,
  });

  assert.equal(monitorBand(body).text(), "Stopped | Queue 3 | Active 1/2");
  assert.equal(body.querySelector('[data-action="monitor-toggle"]').textContent, "Start");
  // Issue #3561: the click above only sent the request; the switch still shows
  // the server state (Off) until a status confirms the change.
  assert.equal(
    body.querySelector('[data-action="monitor-autonomous"]').getAttribute("aria-checked"),
    "false",
  );
  surface.applyIssueMonitorStatus({ autonomous_mode: true });
  assert.equal(body.querySelector('[data-action="monitor-autonomous"]').getAttribute("aria-checked"), "true");
  assert.equal(document.querySelector(".issue-monitor-card"), null);
  for (const [state, exclusion_reason] of [
    ["hold_excluded", "hold label: hold"],
    ["source_excluded", "Excluded by label: hold"],
  ]) {
    surface.applyIssueMonitorInbox([{ issue: { number: 45 }, state, exclusion_reason }]);
    const stale = body.querySelector('[data-issue-number="45"]');
    const message = "This exclusion was reported by an older monitor. Refresh to confirm queue membership.";
    assert.match(stale.textContent, /This exclusion was reported by an older monitor\. Refresh to confirm queue membership\./);
    assert.equal(stale.querySelector(".knowledge-row-badge").dataset.issueExplanation, message);
    assert.doesNotMatch(stale.textContent, /hold label: hold|Excluded by label: hold/);
  }
});

// --- Issue #3561: the Autonomous toggle is a state switch, never an action ---
// The label names the setting, the state word + aria-checked carry the value,
// and the server status is the only thing that ever moves the display.

test("Issue #3561: Autonomous switch exposes aria-checked and shows server state only", async (t) => {
  const spies = errorSpies();
  const { body, sent, surface } = await makeFixture(spies.options);
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const status = (autonomous, extra = {}) => ({
    enabled: false,
    state: "disabled",
    queue_len: 0,
    active_count: 0,
    max_active_agents: 1,
    total_candidates: 0,
    autonomous_mode: autonomous,
    ...extra,
  });
  const toggle = body.querySelector('[data-action="monitor-autonomous"]');
  const stateWord = () =>
    toggle.querySelector(".knowledge-monitor-switch__state").textContent;

  // AC-1 / AC-2: a WAI-ARIA switch with a fixed accessible name.
  assert.equal(toggle.tagName, "BUTTON");
  assert.equal(toggle.getAttribute("role"), "switch");
  assert.equal(toggle.getAttribute("aria-label"), "Autonomous mode");
  assert.equal(
    toggle.querySelector(".knowledge-monitor-switch__label").textContent,
    "Autonomous",
  );
  assert.ok(toggle.querySelector(".knowledge-monitor-switch__track .knowledge-monitor-switch__knob"));

  surface.applyIssueMonitorStatus(status(false));
  assert.equal(toggle.getAttribute("aria-checked"), "false");
  assert.equal(toggle.dataset.enabled, "false");
  assert.equal(stateWord(), "Off");

  // AC-3 / AC-4: a click sends the request and nothing else. No optimistic
  // value is rendered before the server confirms.
  toggle.click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_autonomous_mode",
    enabled: true,
  });
  assert.equal(toggle.getAttribute("aria-checked"), "false");
  assert.equal(stateWord(), "Off");

  // Send failure: the backend answers a rejected control with a status
  // snapshot carrying last_error and the real (unchanged) value.
  surface.applyIssueMonitorStatus(
    status(false, { state: "error", last_error: "autonomous-mode: control rejected" }),
  );
  assert.equal(toggle.getAttribute("aria-checked"), "false");
  assert.equal(stateWord(), "Off");
  assert.equal(spies.reported.length, 1);

  // Success: only the server status flips the switch.
  surface.applyIssueMonitorStatus(status(true));
  assert.equal(toggle.getAttribute("aria-checked"), "true");
  assert.equal(toggle.dataset.enabled, "true");
  assert.equal(stateWord(), "On");

  // Server truth wins over the local expectation: a click toward Off followed
  // by a status that still says On keeps On.
  toggle.click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_autonomous_mode",
    enabled: false,
  });
  surface.applyIssueMonitorStatus(status(true));
  assert.equal(toggle.getAttribute("aria-checked"), "true");
  assert.equal(stateWord(), "On");

  // AC-5: Start/Stop is an action button (label = what the press does, state
  // lives in the Monitor band pill) and it does not move on click either.
  const startStop = body.querySelector('[data-action="monitor-toggle"]');
  assert.equal(startStop.textContent, "Start");
  startStop.click();
  assert.deepEqual(sent.at(-1), { kind: "set_issue_monitor_enabled", enabled: true });
  assert.equal(startStop.textContent, "Start");
  surface.applyIssueMonitorStatus(status(true, { enabled: true, state: "idle" }));
  assert.equal(startStop.textContent, "Stop");
  assert.equal(monitorBand(body).pill.textContent, "Waiting · Empty");
});

// --- SPEC #3206 FR-017: errors are read in ONE place (notification center) ---
// User ruling 2026-09-04: the Issue window shows no error surface of its own.
// It reports every error to the notification center and renders nothing.

test("FR-017 / #5284: current Monitor error is read in the center through the state badge", async (t) => {
  const spies = errorSpies();
  const { body, surface, opened } = await makeFixture(spies.options);
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  assert.equal(body.querySelector(".knowledge-monitor-error"), null, "no red banner");
  assert.equal(body.querySelector(".surface-error-indicator"), null, "no compact indicator either");

  surface.applyIssueMonitorStatus({ enabled: true, state: "error", queue_len: 0, active_count: 0, max_active_agents: 1, last_error: "issue #3785: scan failed" });
  assert.deepEqual(spies.reported, [
    { key: "issue-monitor:last_error", title: "Issue Monitor", message: "issue #3785: scan failed" },
  ]);
  assert.equal(body.querySelector(".surface-error-indicator"), null, "still nothing in the surface");
  monitorBand(body).pill.click();
  assert.equal(opened.at(-1), "issue-monitor:last_error");

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
  // SPEC #3885 T-032: the Issue window has no status row at all.
  assert.equal(body.querySelector(".knowledge-status"), null);

  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_error",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    message: "gh issue list: github_rate_limited (resets 09:30Z)",
  });
  assert.equal(body.querySelector(".knowledge-status"), null, "no red band");
  assert.doesNotMatch(
    body.querySelector('[data-action="refresh-knowledge"]').title,
    /github_rate_limited/,
    "and no error text in the surface",
  );
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

// Issue #3628 AC-5: on 2026-08-17 nine issues sat in `agent_failed`, nothing
// ran at all, and the monitor panel still read healthy. `last_error` was
// already occupied by one of those per-issue failures, so the outage needs its
// own channel or it stays invisible exactly when it matters. SPEC #3885 T-034
// moved that channel from a banner row to the ⚠ pill + the notification center.
test("A fleet outage is shown even while a per-issue error occupies the error line", async (t) => {
  const spies = errorSpies();
  const { body, surface } = await makeFixture(spies.options);
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));

  assert.equal(body.querySelector(".knowledge-monitor-blackout"), null, "no banner row");
  const pill = monitorBand(body).pill;
  assert.equal(pill.title, "", "a healthy fleet shows nothing");

  surface.applyIssueMonitorStatus({
    enabled: true,
    state: "error",
    queue_len: 9,
    active_count: 0,
    max_active_agents: 3,
    total_candidates: 9,
    autonomous_mode: false,
    launch_profile_source: "saved",
    launch_profile_summary: "codex / host",
    last_error: "issue #2338: an execution generation already exists",
    agent_blackout:
      "No implementation agent has been running for 1800s while 9 issue(s) were runnable; the fleet has been down since 2026-08-17T00:00:00Z",
  });

  assert.equal(pill.textContent, "⚠ Error");
  assert.equal(pill.dataset.notificationKey, "issue-monitor:agent_blackout", "the outage outranks the per-issue error");
  assert.equal(pill.title, "Details in notifications");
  // FR-017 (user ruling 2026-09-04): errors are read in the notification
  // center. The outage reports under its own key, so it never competes for the
  // single `last_error` slot the per-issue failure already holds.
  assert.deepEqual(
    spies.reported.map((entry) => [entry.key, entry.message]),
    [
      [
        "issue-monitor:agent_blackout",
        "No implementation agent has been running for 1800s while 9 issue(s) were runnable; the fleet has been down since 2026-08-17T00:00:00Z",
      ],
      ["issue-monitor:last_error", "issue #2338: an execution generation already exists"],
    ],
    "the per-issue error keeps its own channel rather than being overwritten",
  );
});

// Issue #3628 AC-3: a row whose launch died had no GUI recovery at all. The
// existing Launch Now only opens the wizard, so an operator who wanted the row
// back in the queue without starting an agent had to hand-edit the state file.
test("A failed row offers a requeue that returns it to the queue without launching", async (t) => {
  const { body, sent, surface, load } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries: [
      knowledgeEntry(3628, "agent_failed"),
      knowledgeEntry(3629, "launch_failed"),
      knowledgeEntry(3630, "queued", 1),
      knowledgeEntry(3631, "launched"),
      knowledgeEntry(3632, "needs_human"),
    ],
    selected_number: 3628,
    empty_message: "",
    refresh_enabled: true,
  });

  const failedRow = body.querySelector('[data-issue-number="3628"]');
  const requeue = failedRow.querySelector('[data-action="requeue-issue"]');
  assert.ok(requeue, "an agent_failed row must offer the recovery");
  assert.equal(
    requeue.getAttribute("aria-label"),
    "Return to the queue Issue #3628",
    "the control must say it requeues rather than launches",
  );
  requeue.click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_requeue",
    issue_number: 3628,
  });

  body
    .querySelector('[data-issue-number="3629"]')
    .querySelector('[data-action="requeue-issue"]')
    .click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_requeue",
    issue_number: 3629,
  });

  // The recovery releases a failure hold. Offering it where no hold exists
  // would promise a state change that cannot happen.
  for (const number of [3630, 3631, 3632]) {
    assert.equal(
      body
        .querySelector(`[data-issue-number="${number}"]`)
        .querySelector('[data-action="requeue-issue"]'),
      null,
      `#${number} holds no failure and must not offer the recovery`,
    );
  }
});

// #4499: terminal queue membership is the Issue board's source of truth.
test("Issue board has four queue columns, labelled controls and one filter row", async (t) => {
  const { body, surface, load } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,"queued"), {...knowledgeEntry(2,"queued",1),queued_by:"operator"}, knowledgeEntry(3,"launching"),knowledgeEntry(4,null,null,{state:"closed"})],refresh_enabled:true});
  const columns = [...body.querySelectorAll("[data-queue-column]")];
  assert.deepEqual(columns.map(c=>c.dataset.queueColumn),["backlog","queued","active","done"]);
  for (const [index,column] of columns.entries()) assert.ok(column.querySelector(`[data-issue-number="${index+1}"]`));
  assert.match(columns[1].textContent,/operator/i);
  assert.equal(body.querySelector("[data-issue-filter], [data-issue-lane-filter]"),null);
  assert.equal(body.querySelector('[data-issue-view="list"]').textContent,"Kanban");
  assert.equal(body.querySelector('[data-action="monitor-settings"]').getAttribute("aria-label"), "Issues settings");
  const controls = body.querySelector('.knowledge-monitor-controls');
  assert.ok(controls.querySelector('[data-action="monitor-auto-refill"]'));
  assert.ok(controls.querySelector('[data-action="monitor-autonomous"]'));
  for (const [action, label] of [["monitor-auto-refill", "Auto-refill"], ["monitor-autonomous", "Autonomous"],
    ["monitor-toggle", "Start"], ["monitor-settings", "Issues settings"]]) {
    assert.ok(controls.querySelector(`[data-action="${action}"]`).textContent.includes(label), `${action} has a visible label`);
  }

});

test("Issue board auto-refill renders confirmed state and preserves the configured limit", async (t) => {
  const { body, surface, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const toggle = body.querySelector('[data-action="monitor-auto-refill"]');
  assert.ok(toggle);
  assert.equal(toggle.getAttribute("role"), "switch");
  assert.equal(toggle.getAttribute("aria-checked"), "false");
  surface.applyIssueMonitorStatus({terminal_queue_auto_refill:false, terminal_queue_auto_refill_limit:5});
  toggle.click();
  assert.deepEqual(sent.at(-1), {kind:"set_issue_monitor_auto_refill", enabled:true, limit:5});
  assert.equal(toggle.getAttribute("aria-checked"), "false", "request does not optimistically enable refill");
  assert.equal(toggle.querySelector('.knowledge-monitor-switch__state').textContent, "Off");
  surface.applyIssueMonitorStatus({terminal_queue_auto_refill:true, terminal_queue_auto_refill_limit:5});
  assert.equal(toggle.getAttribute("aria-checked"), "true");
  assert.equal(toggle.querySelector('.knowledge-monitor-switch__state').textContent, "On");
  toggle.click();
  assert.deepEqual(sent.at(-1), {kind:"set_issue_monitor_auto_refill", enabled:false, limit:5});
  assert.equal(toggle.getAttribute("aria-checked"), "true");
  surface.applyIssueMonitorStatus({terminal_queue_auto_refill:false, terminal_queue_auto_refill_limit:0});
  toggle.click();
  assert.deepEqual(sent.at(-1), {kind:"set_issue_monitor_auto_refill", enabled:true, limit:0});
});

test("Issue queue drops include hold-labeled cards and retain server projection", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[{...knowledgeEntry(1,null),labels:["hold"]},knowledgeEntry(2,null),
      {...knowledgeEntry(3,"queued",1),labels:["hold"]}],refresh_enabled:true});
  assert.equal(body.querySelector('[data-issue-number="1"] .knowledge-row-badge').textContent, "Open");
  assert.equal(body.querySelector('[data-issue-number="3"] .knowledge-row-badge').textContent, "Queued");
  for (const number of [1,2]) body.querySelector(`[data-issue-number="${number}"] [data-action="queue-select"]`).click();
  const drop=(column,number)=> { const event=new window.Event("drop",{bubbles:true,cancelable:true}); event.dataTransfer={getData:()=>String(number)}; body.querySelector(`[data-queue-column="${column}"]`).dispatchEvent(event); };
  const before=sent.length;
  drop("queued",1);
  assert.equal(sent.length,before+1);
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_push",issue_numbers:[1,2]});
  assert.ok(body.querySelector('[data-queue-column="backlog"] [data-issue-number="1"]'));
  drop("active",1);
  assert.equal(sent.length,before+1);
  assert.match(body.querySelector('.issue-queue-feedback').textContent,/Active.*monitor/i);
  drop("backlog",3);
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_remove",issue_numbers:[3]});
});

test("Issue #4499 queue drops explain missing cards, invalid data and unchanged columns", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind:"knowledge_entries", id:"win-1", knowledge_kind:"issue",
    request_id:load.request_id, entries:[knowledgeEntry(1,null)], refresh_enabled:true });
  const drop = (phase, raw) => {
    const event = new window.Event("drop", {bubbles:true,cancelable:true});
    event.dataTransfer = {getData:() => raw};
    body.querySelector(`[data-queue-column="${phase}"]`).dispatchEvent(event);
  };
  const before = sent.length;
  drop("queued", "99");
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Issue #99.*no longer.*Refresh/i);
  drop("queued", "1-not-an-issue");
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Drop an issue card/i);
  drop("backlog", "1");
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Issue #1.*already.*Backlog/);
  assert.equal(sent.length, before, "invalid and unchanged drops send no queue mutation");
});

test("Issue #4499 queue receipts show accepted changes and exact server refusal reasons", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind:"knowledge_entries", id:"win-1", knowledge_kind:"issue",
    request_id:load.request_id, entries:[knowledgeEntry(1,null)], refresh_enabled:true });
  const event = new window.Event("drop", {bubbles:true,cancelable:true});
  event.dataTransfer = {getData:() => "1"};
  body.querySelector('[data-queue-column="queued"]').dispatchEvent(event);
  assert.deepEqual(sent.at(-1), {kind:"issue_monitor_queue_push",issue_numbers:[1]});
  surface.applyIssueMonitorQueueResult({ kind:"issue_monitor_queue_result", operation:"push", issue_numbers:[1],
    error:"Queue change rejected: queue is unavailable" });
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Issue #1.*Queue change rejected: queue is unavailable/);
  assert.ok(body.querySelector('[data-queue-column="backlog"] [data-issue-number="1"]'));
  surface.applyIssueMonitorQueueResult({ kind:"issue_monitor_queue_result", operation:"push", issue_numbers:[1], error:null });
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Issue #1.*added to.*queue/i);
  assert.ok(body.querySelector('[data-queue-column="backlog"] [data-issue-number="1"]'), "the receipt does not invent membership");
  surface.applyIssueMonitorStatus({terminal_queue:[{number:1,queued_by:"operator"}]});
  assert.ok(body.querySelector('[data-queue-column="queued"] [data-issue-number="1"]'));
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /added to.*queue/i, "projection updates retain the result");
  surface.applyIssueMonitorQueueResult({ kind:"issue_monitor_queue_result", operation:"remove", issue_numbers:[1], error:null });
  assert.match(body.querySelector(".issue-queue-feedback").textContent, /Issue #1.*removed from.*queue/i);
});

test("terminal queue broadcasts control membership, provenance and confirmed order", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,"queued",1),knowledgeEntry(2,"queued",2)],refresh_enabled:true});
  surface.applyIssueMonitorStatus({terminal_queue:[],terminal_queue_auto_refill:false});
  assert.equal(body.querySelectorAll('[data-queue-column="queued"] .knowledge-row').length,0);
  assert.match(body.querySelector('[data-queue-column="queued"]').textContent,/Nothing will launch until an issue is queued/);
  surface.applyIssueMonitorStatus({terminal_queue:[{number:2,queued_by:"auto-refill"},{number:1,queued_by:"operator"}]});
  const queued=()=>[...body.querySelectorAll('[data-queue-column="queued"] .knowledge-row')].map(row=>Number(row.dataset.issueNumber));
  assert.deepEqual(queued(),[2,1]);
  assert.match(body.querySelector('[data-queue-column="queued"]').textContent,/auto-refill/);
  const event=new window.Event("drop",{bubbles:true,cancelable:true});
  event.dataTransfer={getData:()=>"1"};
  body.querySelector('[data-queue-column="queued"] [data-issue-number="2"]').dispatchEvent(event);
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_move",issue_number:1,position:0});
  assert.deepEqual(queued(),[2,1],"server confirmation controls order");
  surface.applyIssueMonitorStatus({terminal_queue:[{number:1,queued_by:"operator"},{number:2,queued_by:"auto-refill"}]});
  assert.deepEqual(queued(),[1,2]);
});

test("Issue #3165: dependency-blocked rows retain their queue position and explain the blocker", async (t) => {
  const { body, surface, load } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, entries: [knowledgeEntry(42, "queued", 1), knowledgeEntry(43, "queued", 2)],
    refresh_enabled: true });
  surface.applyIssueMonitorStatus({ terminal_queue: [{ number: 42 }, { number: 43 }] });
  surface.applyIssueMonitorInbox([{ issue: { number: 42 }, state: "blocked_by_dependency",
    exclusion_reason: "Blocked by #44", dependency: { blocked_by: [{ number: 44 }], cycle: [] } }]);
  const row = body.querySelector('[data-queue-column="queued"] [data-issue-number="42"]');
  assert.ok(row);
  assert.equal(row.querySelector(".knowledge-row-badge").textContent, "Blocked by dependency");
  assert.equal(row.querySelector(".knowledge-row-badge").dataset.tone, "needs-input");
  assert.match(row.textContent, /Blocked by #44/);
  assert.doesNotMatch(row.textContent, /\(hold\)/);
  assert.deepEqual([...body.querySelectorAll('[data-queue-column="queued"] .knowledge-row')]
    .map(item => Number(item.dataset.issueNumber)), [42, 43]);
});

test("latest inbox launch state moves a cached queued row to Active while retaining terminal queue operations", async (t) => {
  const { body, surface, load, sent, mod } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const cached = {kind:"knowledge_entries", id:"win-1", knowledge_kind:"issue", request_id:load.request_id,
    entries:[knowledgeEntry(5016,"queued",2), knowledgeEntry(2,"queued",3)], refresh_enabled:true};
  surface.applyKnowledgeReceiveEvent(cached);
  surface.applyIssueMonitorStatus({terminal_queue:[99,5016,2].map(number=>({number,queued_by:"operator"}))});
  const rows = phase => [...body.querySelectorAll(`[data-queue-column="${phase}"] .knowledge-row`)].map(row=>Number(row.dataset.issueNumber));
  assert.deepEqual(rows("queued"), [5016,2]);
  surface.applyIssueMonitorInbox([{issue:{number:5016},state:"launched"}]);
  const activeMember = { ...cached.entries[0], monitor_state:"launched" };
  assert.equal(mod.isIssueInTerminalQueue(activeMember), true);
  assert.equal(mod.issueQueueColumn(activeMember), 'active');
  assert.deepEqual(rows("queued"), [2]);
  assert.deepEqual(rows("active"), [5016]);
  surface.applyKnowledgeReceiveEvent(cached);
  assert.deepEqual(rows("active"), [5016], 'a late cached response cannot undo the live launch');
  body.querySelector('[data-issue-number="2"] [data-action="move-up"]').click();
  assert.deepEqual(sent.at(-1), {kind:"issue_monitor_queue_move",issue_number:2,position:1},
    'move uses the full terminal queue including the active row and uncached predecessor');
});

test("Issue #4862 failed launches leave Active despite stale Work and show the current failure reason", async (t) => {
  const failures = [[42, "launch_failed", "Launch failed"], [43, "agent_failed", "Agent failed"]];
  const { body, surface, load } = await makeFixture({
    getActiveWorkProjection: () => ({ active_works: failures.map(([number]) => ({
      id: `work-${number}`, active_agents: 1,
    })) }),
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const cached = { kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue", request_id: load.request_id,
    entries: failures.map(([number]) => ({ ...knowledgeEntry(number, "launched"), related_work_refs: [{ id: `work-${number}` }] })),
    refresh_enabled: true };
  surface.applyKnowledgeReceiveEvent(cached);
  const inbox = failures.map(([number, state]) => ({ issue: { number }, state, launched_window_id: null,
    error_message: `Issue ${number}: successor has invalid lineage` }));
  surface.applyIssueMonitorInbox(inbox);
  surface.applyKnowledgeReceiveEvent(cached);
  for (const [number, , badge] of failures) {
    const row = body.querySelector(`[data-queue-column="backlog"] [data-issue-number="${number}"]`);
    assert.ok(row, "a failed launch with no window must not inherit stale Work activity");
    assert.equal(row.querySelector(".knowledge-row-badge").textContent, badge);
    assert.match(row.querySelector('[data-kind="reason"]').textContent, /successor has invalid lineage/);
    assert.match(row.querySelector('[data-key="recovery"]')?.textContent || "", /Work lineage.*workspace.close.*issue.monitor.requeue/);
    assert.ok(row.querySelector('[data-action="requeue-issue"]'));
    assert.equal(body.querySelector(`[data-queue-column="active"] [data-issue-number="${number}"]`), null);
  }
  surface.applyIssueMonitorInbox(inbox.map(({ error_message, ...item }) => item));
  assert.equal(body.querySelector('[data-kind="reason"]'), null, "omitted failure reasons clear the previous diagnosis");
  surface.applyIssueMonitorInbox([{ issue: { number: 43 }, state: "needs_human",
    error_message: "autonomous launch attempts exhausted (3/3) with an unchanging failure: Launch timed out: no window was created within 120s" }]);
  const capped = body.querySelector('[data-queue-column="backlog"] [data-issue-number="43"]');
  assert.ok(capped, "a capped launch failure must also ignore stale Work activity");
  assert.match(capped.querySelector('[data-key="recovery"]').textContent, /Window timeout.*issue.monitor.requeue/);
  surface.applyIssueMonitorInbox([{ issue: { number: 43 }, state: "needs_human",
    error_message: "autonomous launch attempts exhausted (3/3) with an unchanging failure: successor work-43-next is duplicated; use workspace.close" }]);
  const unclassified = body.querySelector('[data-queue-column="backlog"] [data-issue-number="43"]');
  assert.ok(unclassified, "an unclassified capped launch failure must also ignore stale Work activity");
  assert.equal(unclassified.querySelector('[data-key="recovery"]').textContent, "Launch failure · Next: issue.monitor.status");
});

test("Issue #4862 a live inbox stop replaces stale cached activity outside the terminal queue", async (t) => {
  const { body, surface, load } = await makeFixture({
    getActiveWorkProjection: () => ({ active_works: [{ id: "work-42", active_agents: 1 }] }),
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const cached = { kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue", request_id: load.request_id,
    entries: [{ ...knowledgeEntry(42, "launched"), related_work_refs: [{ id: "work-42" }] }], refresh_enabled: true };
  surface.applyKnowledgeReceiveEvent(cached);
  surface.applyIssueMonitorStatus({ terminal_queue: [] });
  surface.applyIssueMonitorInbox([{ issue: { number: 42 }, state: "needs_human", error_message: "stopped: PM chose to pause" }]);
  surface.applyKnowledgeReceiveEvent(cached);
  const held = body.querySelector('[data-queue-column="backlog"] [data-issue-number="42"]');
  assert.ok(held, "a stop outside the terminal queue still overrides stale Work activity");
  assert.equal(held.querySelector(".knowledge-row-badge").textContent, "Held");
  assert.equal(held.querySelector(".knowledge-row-badge").dataset.tone, "idle");
  assert.equal(held.querySelector('[data-key="reason"]').textContent, "PM chose to pause");
  assert.equal(held.querySelector('[data-key="recovery"]'), null);
  assert.equal(body.querySelector('[data-queue-column="active"] [data-issue-number="42"]'), null);
});

test("Issue #4862 an actual same-owner running pane outranks a stale failed inbox state", async (t) => {
  const pane = { id: "agent-42", preset: "agent", agent_id: "codex", status: "running",
    placement: { kind: "issue_preview", issue_window_id: "win-1", issue_number: 42 } };
  const { body, surface, load } = await makeFixture({ getWorkspaceWindows: () => [pane] });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue", request_id: load.request_id,
    entries: [knowledgeEntry(42, "launched")], refresh_enabled: true });
  surface.applyIssueMonitorInbox([{ issue: { number: 42 }, state: "launch_failed", error_message: "old failed attempt" }]);
  const active = body.querySelector('[data-queue-column="active"] [data-issue-number="42"]');
  assert.ok(active, "a real running pane is stronger evidence than a stale failure");
  assert.equal(active.querySelector(".knowledge-row-badge").textContent, "Running");
  assert.equal(body.querySelector('[data-queue-column="backlog"] [data-issue-number="42"]'), null);
});

test("Issue #4862 a live pane wins over an earlier terminal preview", async (t) => {
  const terminal = { id: "old-agent", preset: "agent", agent_id: "codex", status: "error",
    placement: { kind: "issue_preview", issue_window_id: "win-1", issue_number: 42 } };
  const live = { ...terminal, id: "live-agent", status: "running", linked_issue_number: 42 };
  const { body, surface, load } = await makeFixture({
    getWorkspaceWindows: () => [terminal, live],
    getActiveWorkProjection: () => ({ active_works: [{ id: "work-42", agents: [{ window_id: live.id }] }] }),
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue", request_id: load.request_id,
    entries: [{ ...knowledgeEntry(42, "launched"), related_work_refs: [{ id: "work-42" }] }], refresh_enabled: true });
  for (const placement of [terminal.placement, { kind: "canvas" }]) {
    live.placement = placement;
    surface.applyIssueMonitorInbox([{ issue: { number: 42 }, state: "launch_failed", error_message: `old failed attempt: ${placement.kind}` }]);
    const active = body.querySelector('[data-queue-column="active"] [data-issue-number="42"]');
    assert.ok(active, `the live ${placement.kind} pane must outrank the earlier error preview`);
    assert.equal(active.querySelector(".knowledge-row-badge").textContent, "Running");
  }
});

test("queue drag uses terminal positions when a queued issue is not cached", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,"queued",2),knowledgeEntry(2,"queued",3)],refresh_enabled:true});
  surface.applyIssueMonitorStatus({terminal_queue:[99,1,2].map(number=>({number,queued_by:"operator"}))});
  const drop=new window.Event("drop",{bubbles:true,cancelable:true});
  drop.dataTransfer={getData:()=>"2"};
  body.querySelector('[data-queue-column="queued"] [data-issue-number="1"]').dispatchEvent(drop);
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_move",issue_number:2,position:1});
});

test("queue row move preserves uncached predecessors and waits for server order", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,"queued",2),knowledgeEntry(2,"queued",3)],refresh_enabled:true});
  surface.applyIssueMonitorStatus({terminal_queue:[99,1,2].map(number=>({number,queued_by:"operator"}))});
  body.querySelector('[data-issue-number="2"] [data-action="move-up"]').click();
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_move",issue_number:2,position:1});
  assert.equal(body.querySelector('[data-queue-column="queued"] .knowledge-row').dataset.issueNumber,"1");
  const firstVisibleUp=body.querySelector('[data-issue-number="1"] [data-action="move-up"]');
  assert.equal(firstVisibleUp.disabled,false,"uncached predecessor remains reachable");
  firstVisibleUp.click();
  assert.deepEqual(sent.at(-1),{kind:"issue_monitor_queue_move",issue_number:1,position:0});
});

test("Issue refresh preserves the Other disclosure node during user interaction", async (t) => {
  const { body, surface } = await makeFixture({
    renderOtherWork(parent) {
      if (!parent.querySelector(".issue-other-group")) {
        const other = parent.ownerDocument.createElement("details");
        other.className = "issue-other-group";
        parent.appendChild(other);
      }
    },
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const other = body.querySelector(".issue-other-group");
  other.setAttribute("open", "");
  surface.renderKnowledgeBridge("win-1");
  assert.ok(body.querySelector(".issue-other-group") === other,
    "Other disclosure identity survives refresh");
  assert.equal(other.hasAttribute("open"), true);
  assert.ok(body.querySelector(".knowledge-list").lastElementChild === other);
});


test("queue priority displays canonical reasons and observed assignment without inventing an actor", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, entries: [1, 2, 3].map(number => knowledgeEntry(number, "queued", number)), refresh_enabled: true });
  surface.applyIssueMonitorStatus({ terminal_queue: [
    { number: 1, queued_by: "urgent", priority: "urgent", priority_reason: "urgent_label", assigned_by: "alice", assigned_at: "2026-10-03T01:00:00Z" },
    { number: 2, priority: "normal", priority_reason: "urgent_limit_reached" },
    { number: 3, priority: "normal", priority_reason: "pm_demoted", assigned_by: "pm-session", assigned_at: "2026-10-03T02:00:00Z" },
  ] });
  for (const [number, label] of [[1, "Urgent"], [2, "Normal · Urgent limit reached"], [3, "Normal · PM demoted"]]) {
    const row = body.querySelector(`[data-issue-number="${number}"]`);
    assert.equal(row.querySelector('[data-key="queue-priority"]')?.textContent, label);
    assert.equal(row.querySelector('[data-key="queue"]')?.textContent, `Queue ${number}${number === 1 ? " · urgent" : ""}`);
  }
  for (const [number, actor, time] of [[1, "alice", "2026-10-03T01:00:00Z"], [2, "Unknown", "Not observed"]]) {
    body.querySelector(`[data-issue-number="${number}"] .knowledge-row-select`).click();
    const request = sent.findLast(message => message.kind === "select_knowledge_bridge_entry");
    surface.applyKnowledgeReceiveEvent({ kind: "knowledge_detail", id: "win-1", knowledge_kind: "issue", request_id: request.request_id,
      detail: { number, title: `Issue ${number}`, state: "open", labels: [], sections: [], related_works: [] } });
    if (number === 1) assert.equal(body.querySelector(".issue-detail-provenance").textContent, "Queued by: Urgent label");
    assert.equal(body.querySelector(".issue-detail-priority-assignment").textContent, `Priority assigned by: ${actor} · Assigned at: ${time}`);
  }
});

test("Issue #4783: Related Work closes only an exact backend-approved child Work", async (t) => {
  const projection = { active_works: [{ id: "workspace-1", branch: "work/one",
    manual_close_allowed: true,
    works: [
      { id: "orphan-work", lifecycle_state: "paused", manual_close_allowed: true },
      { id: "live-work", lifecycle_state: "active", manual_close_allowed: false,
        close_blocked_reason: "live_agent" },
    ],
  }] };
  const { body, surface, load, sent } = await makeFixture({
    getActiveWorkProjection: () => projection,
  });
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
    request_id: load.request_id, entries: [knowledgeEntry(1, "launched")], refresh_enabled: true });
  body.querySelector('.knowledge-row-select').click();
  const request = sent.findLast(message => message.kind === "select_knowledge_bridge_entry");
  surface.applyKnowledgeReceiveEvent({ kind: "knowledge_detail", id: "win-1", knowledge_kind: "issue",
    request_id: request.request_id, detail: { number: 1, title: "Issue 1", state: "open",
      labels: [], sections: [], related_works: ["orphan-work", "live-work", "unmatched-work"].map(id => ({
        id, title: id, branch: "work/one", status_category: "idle", agents: [],
      })) } });

  const cards = [...body.querySelectorAll(".knowledge-related-work")];
  assert.equal(cards.length, 3, "restored Work remains visible without an agent");
  const buttons = card => [...card.querySelectorAll(".workspace-detail-work-actions button")];
  assert.deepEqual(buttons(cards[0]).map(button => button.textContent), ["Done", "Discard"]);
  buttons(cards[0])[0].click();
  buttons(cards[0])[1].click();
  assert.deepEqual(sent.filter(message => message.kind === "close_work"), [
    { kind: "close_work", work_id: "orphan-work", close_kind: "done" },
    { kind: "close_work", work_id: "orphan-work", close_kind: "discarded" },
  ]);
  assert.equal(buttons(cards[1]).length, 2);
  for (const button of buttons(cards[1])) {
    assert.ok(button.disabled);
    assert.equal(button.title, "live_agent");
  }
  assert.equal(buttons(cards[2]).length, 0, "branch equality must not supply close authority");
});

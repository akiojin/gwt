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

  assert.equal(band.text(), "Quota hold | Queue 3 | Active 0/2");
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
  assert.match(monitorBand(body).text(),
    new RegExp(`Queue ${response.queue.length} \\| Active ${response.active_launches.length}/${response.max_active}`));
  assert.equal(body.querySelector(".knowledge-monitor-max-active input").value, "4");
  assert.equal(body.querySelector('[data-action="monitor-auto-apply"]').dataset.enabled, "true");
  surface.applyIssueMonitorStatus({ ...response.gui_status, max_active_agents: 5 });
  assert.equal(body.querySelector(".knowledge-monitor-max-active input").value, "5");
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
  assert.equal(band.pill.title, "issue #3785: failed");
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

  for (const state of ["active", "launching"]) {
    surface.applyIssueMonitorStatus({
      enabled: true,
      state,
      queue_len: 3,
      active_count: 1,
      max_active_agents: 2,
      quota_hold: quotaHold,
    });

    assert.equal(band.text(), "Quota hold | Queue 3 | Active 1/2");
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
    kind: "issue_monitor_queue_move",
    issue_number: 44,
    position: 0,
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
  // Issue #3906 AC-1: the auto-apply override sits next to the autonomous
  // toggle and flips the effective value the backend reported.
  body.querySelector('[data-action="monitor-auto-apply"]').click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_auto_apply_updates",
    enabled: true,
  });
  body.querySelector('[data-action="monitor-settings"]').click();
  assert.deepEqual(sent.at(-1), { kind: "issue_monitor_configure_profile" });

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
  assert.equal(body.querySelector('[data-action="monitor-toggle"]').textContent, "Start monitor");
  // Issue #3561: the click above only sent the request; the switch still shows
  // the server state (Off) until a status confirms the change.
  assert.equal(
    body.querySelector('[data-action="monitor-autonomous"]').getAttribute("aria-checked"),
    "false",
  );
  assert.equal(
    body.querySelector('[data-action="monitor-auto-apply"]').textContent,
    "Auto-apply updates: OFF",
  );
  surface.applyIssueMonitorStatus({ autonomous_mode: true, auto_apply_updates: true });
  const autoApply = body.querySelector('[data-action="monitor-auto-apply"]');
  assert.equal(autoApply.textContent, "Auto-apply updates: ON");
  assert.equal(autoApply.dataset.enabled, "true");
  autoApply.click();
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_auto_apply_updates",
    enabled: false,
  });
  assert.equal(document.querySelector(".issue-monitor-card"), null);
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
  assert.equal(startStop.textContent, "Start monitor");
  startStop.click();
  assert.deepEqual(sent.at(-1), { kind: "set_issue_monitor_enabled", enabled: true });
  assert.equal(startStop.textContent, "Start monitor");
  surface.applyIssueMonitorStatus(status(true, { enabled: true, state: "idle" }));
  assert.equal(startStop.textContent, "Stop");
  assert.equal(monitorBand(body).pill.textContent, "Running");
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
  assert.match(pill.title, /the fleet has been down since/, "the outage outranks the per-issue error");
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

// #4530: the pool editor is distinct from Agent Settings' head replacement.
test("candidate pool renders held rows and edits through profiles.set", async (t) => {
  const { body, surface, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const candidates = [
    { index: 0, agent_id: "claude", summary: "Claude / Sonnet", prefer_for: ["kind:spec"], held_until: "2026-10-01T00:00:00Z" },
    { index: 1, agent_id: "codex", summary: "Codex / gpt-6-astra", prefer_for: [], held_until: null },
  ];
  const status = { launch_profile_candidates: candidates, usage_threshold_percent: 80 };
  surface.applyIssueMonitorStatus(status);
  const pool = body.querySelector(".knowledge-monitor-pool");
  assert.ok(pool, "a collapsible candidate pool editor is present");
  assert.equal(pool.querySelectorAll(".knowledge-monitor-candidate").length, 2);
  assert.match(pool.textContent, /Auto/);
  assert.match(pool.textContent, /Held/);
  assert.match(pool.textContent, /Claude \/ Sonnet/);
  const click = (selector) => pool.querySelector(selector).click();
  const last = () => sent.at(-1);
  click('[data-pool-action="down"]');
  assert.deepEqual(last(), { kind: "issue_monitor_profiles_set", profiles: [{ agent_id: "codex" }, { agent_id: "claude" }] });
  pool.querySelectorAll('[data-pool-action="up"]')[1].click();
  assert.deepEqual(last().profiles, [{ agent_id: "codex" }, { agent_id: "claude" }]);
  click('[data-pool-action="remove"]');
  assert.deepEqual(last().profiles, [{ agent_id: "codex" }]);
  const threshold = pool.querySelector('[data-pool-field="threshold"]');
  threshold.value = "70";
  threshold.dispatchEvent(new window.Event("change", { bubbles: true }));
  assert.equal(last().usage_threshold_percent, 70);
  const tags = pool.querySelector('[data-pool-field="prefer-for"]');
  tags.value = "type:fix, kind:spec";
  tags.dispatchEvent(new window.Event("change", { bubbles: true }));
  assert.deepEqual(last().profiles, [{ agent_id: "claude", prefer_for: ["type:fix", "kind:spec"] }, { agent_id: "codex" }]);
  tags.value = "";
  tags.dispatchEvent(new window.Event("change", { bubbles: true }));
  assert.deepEqual(last().profiles[0].prefer_for, []);
  surface.applyIssueMonitorStatus(status);
  const agent = pool.querySelector('[data-pool-field="agent"]');
  agent.value = "grok";
  const addButton = pool.querySelector('[data-pool-action="add"]');
  Object.defineProperty(globalThis.document, "activeElement", { configurable: true, value: addButton });
  surface.applyIssueMonitorStatus(status);
  assert.equal(pool.querySelector('[data-pool-field="agent"]').value, "grok", "status while the Add button is focused preserves the draft");
  click('[data-pool-action="add"]');
  assert.deepEqual(last().profiles, [{ agent_id: "claude" }, { agent_id: "codex" }, { agent_id: "grok" }]);
});

test("candidate pool prevents empty pools, duplicate additions and invalid inputs", async (t) => {
  const { body, surface, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyIssueMonitorStatus({ launch_profile_candidates: [{ index: 0, agent_id: "codex", summary: "Codex", prefer_for: [] }], usage_threshold_percent: 80 });
  const pool = body.querySelector(".knowledge-monitor-pool");
  assert.ok(pool);
  assert.equal(pool.querySelector('[data-pool-action="remove"]').disabled, true);
  assert.equal(pool.querySelector('[data-pool-action="up"]').disabled, true);
  assert.equal(pool.querySelector('[data-pool-action="down"]').disabled, true);
  const before = sent.length;
  const threshold = pool.querySelector('[data-pool-field="threshold"]');
  threshold.value = "0";
  threshold.dispatchEvent(new window.Event("change", { bubbles: true }));
  const tags = pool.querySelector('[data-pool-field="prefer-for"]');
  tags.value = "not-a-tag";
  tags.dispatchEvent(new window.Event("change", { bubbles: true }));
  pool.querySelector('[data-pool-field="agent"]').value = "codex";
  pool.querySelector('[data-pool-action="add"]').click();
  assert.equal(sent.length, before, "invalid edits never reach the backend");
  assert.match(pool.querySelector('[role="status"]').textContent, /already/);
});

test("pool edits preserve newer server candidates received while an input is focused", async (t) => {
  const { body, surface, sent, document } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  const claude = { agent_id: "claude", summary: "Claude", prefer_for: [] };
  const codex = { agent_id: "codex", summary: "Codex", prefer_for: [] };
  surface.applyIssueMonitorStatus({ launch_profile_candidates: [claude, codex] });
  const tags = body.querySelector('[data-pool-field="prefer-for"]');
  Object.defineProperty(document, "activeElement", { configurable: true, value: tags });
  surface.applyIssueMonitorStatus({ launch_profile_candidates: [codex, claude, { agent_id: "grok", prefer_for: [] }] });
  tags.value = "type:fix";
  tags.dispatchEvent(new window.Event("change", { bubbles: true }));
  assert.deepEqual(sent.at(-1).profiles, [
    { agent_id: "codex" }, { agent_id: "claude", prefer_for: ["type:fix"] }, { agent_id: "grok" },
  ]);
});

// #4499: terminal queue membership is the Issue board's source of truth.
test("Issue board has four queue columns, labelled controls and one filter row", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,"queued"), {...knowledgeEntry(2,"queued",1),queued_by:"operator"}, knowledgeEntry(3,"launching"),knowledgeEntry(4,null,null,{state:"closed"})],refresh_enabled:true});
  const columns = [...body.querySelectorAll("[data-queue-column]")];
  assert.deepEqual(columns.map(c=>c.dataset.queueColumn),["backlog","queued","active","done"]);
  for (const [index,column] of columns.entries()) assert.ok(column.querySelector(`[data-issue-number="${index+1}"]`));
  assert.match(columns[1].textContent,/operator/i);
  assert.equal(body.querySelector("[data-issue-filter], [data-issue-lane-filter]"),null);
  assert.equal(body.querySelector('[data-issue-view="list"]').textContent,"Kanban");
  assert.match(body.querySelector('[data-action="monitor-settings"]').textContent,/Settings/);
  const refill=body.querySelector('[data-action="monitor-auto-refill"]');
  assert.equal(refill.getAttribute("aria-checked"),"false");
  refill.click();
  assert.deepEqual(sent.at(-1),{kind:"set_issue_monitor_auto_refill",enabled:true,limit:3});
  assert.equal(refill.getAttribute("aria-checked"),"false","wait for server status");
});

test("Issue queue drops send one bulk request and retain server projection", async (t) => {
  const { body, surface, load, sent } = await makeFixture();
  t.after(() => surface.clearKnowledgeBridgeState("win-1"));
  surface.applyKnowledgeReceiveEvent({kind:"knowledge_entries",id:"win-1",knowledge_kind:"issue",request_id:load.request_id,
    entries:[knowledgeEntry(1,null),knowledgeEntry(2,null),knowledgeEntry(3,"queued",1)],refresh_enabled:true});
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

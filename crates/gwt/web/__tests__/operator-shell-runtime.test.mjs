import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import { createUiStateStore } from "../ui-state-store.js";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");
const runtimeCpuUnitsPattern =
  /aggregate CPU.*logical-core-normalized host share.*process rows.*1 core\s*=\s*100%/i;

test("Issues strip reads shared attention summaries on model-only updates", async () => {
  const { subscribeIssueMonitorStatus } = await importOperatorShell();
  const { document } = parseHTML(html);
  const model = createUiStateStore({ status: { enabled: false }, humanAttention: {
    count: null, summary: "", byIssue: {},
  } }, { scheduleFrame: run => { run(); return () => {}; } });
  const unsubscribe = subscribeIssueMonitorStatus(document, model);
  const cell = document.getElementById("op-strip-issue-monitor");
  model.update(state => ({ ...state, humanAttention: { count: 2, summary: "Needs human 2", byIssue: {} } }));
  assert.match(cell.title, /Needs human 2/);
  model.update(state => ({ ...state, humanAttention: { count: 0, summary: "Needs human 0", byIssue: {} } }));
  assert.match(cell.title, /Needs human 0/);
  model.update(state => ({ ...state, reviewAttention: { count: 1, summary: "Review holds 1", byIssue: {} } }));
  assert.match(cell.title, /Review holds 1/);
  model.update(state => ({ ...state, reviewAttention: { count: 0, summary: "Review holds 0", byIssue: {} } }));
  assert.match(cell.title, /Review holds 0/);
  model.update(state => ({ ...state, steeringAttention: { count: 1, summary: "PM steering 1", byIssue: {} } }));
  assert.match(cell.title, /PM steering 1/);
  model.update(state => ({ ...state, steeringAttention: { count: 0, summary: "PM steering 0", byIssue: {} } }));
  assert.match(cell.title, /PM steering 0/);
  assert.equal(document.getElementById("op-strip-issue-monitor-value").textContent, "Off");
  unsubscribe();
});

test("BLOCK tooltips in the strip and Works heading read the same Work projection summary", async () => {
  const { subscribeWorkAttentionSummary, applyTelemetryCounts } = await importOperatorShell();
  const { createActiveWorkProjectionModel } = await import("../issue-other-surface.js");
  const { createWorksSurface } = await import("../works-surface.js");
  const { document } = parseHTML(html);
  const store = value => createUiStateStore(value, { scheduleFrame: run => { run(); return () => {}; } });
  const { model, applyActiveWorkProjectionReceiveEvent: receive } = createActiveWorkProjectionModel();
  const unsubscribe = subscribeWorkAttentionSummary(document, model);
  const surface = createWorksSurface({ document, workProjectionModel: model,
    windowCatalogModel: store({ catalog: { appState: { tabs: [] }, runtimeStates: {} } }),
    activityModel: store({ agents: [] }), rosterModel: store({ members: [] }),
    summaryModel: store({ running: 3, block: 7, idle: 0, error: 0 }), mountTerminal() {}, mountPreview() {} });
  applyTelemetryCounts(document, { running: 3, waiting: 7 });
  const strip = document.querySelector(".op-status-strip__cell--waiting");
  const heading = surface.element.querySelector('[data-count="block"]');
  const unchangedCounts = () => {
    assert.equal(document.getElementById("op-strip-waiting").textContent, "7");
    assert.equal(heading.textContent, "7", "Workspace counts never replace agent BLOCK counts");
  };
  for (const reason of ["Await token", "Await review"]) {
    receive({ kind: "active_work_projection", projection: { id: "project",
      active_works: [{ id: "w", title: "Ship change", blocked_reason: reason }] } });
    const expected = model.read().projection.attentionSummary.summary;
    // Initial delivery is frame-batched; awaiting its task observes the actual subscription.
    await new Promise(resolve => setTimeout(resolve, 100));
    assert.equal(strip.title, expected);
    assert.equal(heading.parentElement.title, expected);
    unchangedCounts();
  }
  receive({ kind: "active_work_projection", projection: null });
  await new Promise(resolve => setTimeout(resolve, 100));
  assert.equal(strip.title, "");
  assert.equal(heading.parentElement.title, "");
  unchangedCounts();
  unsubscribe(); surface.dispose(); model.dispose();
});

test("runtime health batches a frame into the same latest immutable cell and detail snapshot", async () => {
  const { subscribeRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof subscribeRuntimeHealth, "function");
  const { document, window } = parseHTML(html);
  const frames = new Set();
  const model = createUiStateStore(null, {
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
  });
  const unsubscribe = subscribeRuntimeHealth(document, model);
  const cell = document.getElementById("op-strip-runtime-health-value");
  const initial = cell.textContent;
  model.update(() => ({ state: "warn", cpu_percent: 40, processes: [] }));
  const payload = { state: "hot", cpu_percent: 80, memory_bytes: 1024 ** 3,
    processes: [{ pid: 7, name: "original", cpu_percent: 80, memory_bytes: 1024 ** 3 }] };
  model.update(() => payload);
  const previous = model.read();
  assert.equal(previous.cpu_percent, 80, "model reads stay synchronous");
  assert.equal(cell.textContent, initial, "updates must wait for the display frame");
  assert.equal(frames.size, 1, "a burst schedules only one display frame");
  const flush = () => {
    const callbacks = [...frames];
    frames.clear();
    callbacks.forEach(callback => callback());
  };
  payload.processes[0].name = "mutated";
  payload.cpu_percent = 5;
  flush();
  const detail = document.getElementById("op-runtime-health-detail");
  detail.querySelector('[data-sort="cpu"]')?.dispatchEvent(new window.Event("click"));
  assert.equal(cell.textContent, "HOT 80% 1.0G");
  assert.match(detail.textContent, /original/);
  assert.doesNotMatch(detail.textContent, /mutated/);
  assert.equal(previous.processes[0].name, "original");
  model.update(() => ({ state: "ok", cpu_percent: 5, processes: [] }));
  assert.equal(cell.textContent, "HOT 80% 1.0G");
  flush();
  assert.match(cell.textContent, /^OK 5%/);
  assert.doesNotMatch(detail.textContent, /original/);
  assert.equal(previous.cpu_percent, 80);
  const last = cell.textContent;
  model.update(() => ({ state: "warn", cpu_percent: 50 }));
  assert.equal(frames.size, 1);
  const cancelled = [...frames][0];
  unsubscribe();
  assert.equal(frames.size, 0, "unmount cancels the pending display frame");
  cancelled();
  model.update(() => ({ state: "hot", cpu_percent: 90 }));
  assert.equal(cell.textContent, last);
  assert.equal(frames.size, 0);
});

test("clock uses second boundaries and cancels all work while hidden", async () => {
  const { wireStatusStripClock } = await importOperatorShell();
  const { document, window } = parseHTML('<span id="op-strip-clock"></span>');
  window.matchMedia = () => ({ matches: false });
  Object.defineProperty(document, "hidden", { configurable: true, writable: true, value: false });
  let now = new Date(2026, 0, 1, 12, 34, 56, 250).getTime();
  let nextId = 0;
  const timers = new Map();
  const win = {
    Date: class extends Date { constructor() { super(now); } },
    setTimeout(callback, delay) { const id = ++nextId; timers.set(id, { callback, delay }); return id; },
    clearTimeout(id) { timers.delete(id); },
    requestAnimationFrame() { assert.fail("clock must not request animation frames"); },
  };
  const originalFrame = globalThis.requestAnimationFrame;
  globalThis.requestAnimationFrame = win.requestAnimationFrame;
  try {
    wireStatusStripClock({ doc: document, win });
  } finally {
    globalThis.requestAnimationFrame = originalFrame;
  }
  const clock = document.getElementById("op-strip-clock");
  assert.equal(clock.textContent, "12:34:56");
  assert.equal(timers.size, 1);
  assert.equal([...timers.values()][0].delay, 750);
  const fire = () => {
    const [id, timer] = [...timers.entries()][0];
    timers.delete(id);
    timer.callback();
  };
  now += 768; // A late callback must align the next tick again.
  fire();
  assert.equal(clock.textContent, "12:34:57");
  assert.equal([...timers.values()][0].delay, 982);
  document.hidden = true;
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(timers.size, 0);
  now += 6000;
  assert.equal(clock.textContent, "12:34:57");
  document.hidden = false;
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(clock.textContent, "12:35:03");
  assert.equal(timers.size, 1);
  document.dispatchEvent(new window.Event("visibilitychange"));
  assert.equal(timers.size, 1, "repeated visibility events must not duplicate timers");
  document.hidden = true;
  document.dispatchEvent(new window.Event("visibilitychange"));
  wireStatusStripClock({ doc: document, win });
  assert.equal(timers.size, 0, "initially hidden clocks must schedule no work");
});

test("Operator shell fails open when browser storage and media APIs are unavailable", async () => {
  const { initOperatorShell } = await importOperatorShell();
  const { document, window } = parseHTML(html);
  const briefing = document.getElementById("op-briefing");
  assert.ok(briefing, "fixture must include Mission Briefing");

  const brokenWindow = {
    ...window,
    setTimeout: () => 0,
    clearTimeout() {},
    localStorage: throwingStorage(),
    sessionStorage: throwingStorage(),
    matchMedia: () => {
      throw new Error("matchMedia unavailable");
    },
  };

  const originalWarn = console.warn;
  console.warn = () => {};
  try {
    assert.doesNotThrow(() => initOperatorShell({ document, window: brokenWindow }));
  } finally {
    console.warn = originalWarn;
  }
  assert.equal(briefing.hidden, true, "Mission Briefing must not block app startup");
});

test("Operator shell keeps the Command Rail always visible with no reveal state (SPEC-3038)", async () => {
  // SPEC-3038 US-1: the rail is grid-docked, so init must not install any
  // hover-reveal machinery or reveal dataset attributes. The one-shot legacy
  // localStorage migration from SPEC-2356 Phase 9 stays.
  const { initOperatorShell } = await importOperatorShell();
  const { document, window } = parseHTML(html);
  const storage = memoryStorage();
  storage.setItem("gwt:ui:sidebar-collapsed", "true");
  storage.setItem("gwt:ui:window-controls", "hidden");
  const testWindow = {
    ...window,
    setTimeout: () => 0,
    clearTimeout() {},
    localStorage: storage,
    sessionStorage: memoryStorage(),
    matchMedia: () => ({
      matches: false,
      media: "",
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      onchange: null,
      dispatchEvent: () => false,
    }),
  };

  const originalWarn = console.warn;
  const originalCustomEvent = globalThis.CustomEvent;
  console.warn = () => {};
  globalThis.CustomEvent = window.CustomEvent;
  try {
    initOperatorShell({ document, window: testWindow });

    assert.equal(
      document.getElementById("op-sidebar-edge-toggle"),
      null,
      "<< chip toggle must not exist",
    );
    assert.equal(
      document.getElementById("op-window-controls-edge-toggle"),
      null,
      "vv chip toggle must not exist",
    );

    assert.equal(
      document.querySelector(".op-sidebar-peek"),
      null,
      "the peek 帯 is retired — the rail is always visible",
    );
    assert.equal(
      document.querySelector(".op-window-controls-peek"),
      null,
      "window controls peek 帯 stays retired",
    );
    const rail = document.getElementById("op-rail");
    assert.ok(rail, "fixture must include the Command Rail");

    // FR-032: legacy keys must be removed on init.
    assert.equal(storage.getItem("gwt:ui:sidebar-collapsed"), null);
    assert.equal(storage.getItem("gwt:ui:window-controls"), null);

    // No reveal state: the rail needs no data-op-* attributes, ever.
    assert.equal(document.documentElement.dataset.opSidebar, undefined);
    assert.equal(document.documentElement.dataset.opWindowControls, undefined);

    // Hovering the rail must not flip any reveal state either.
    rail.dispatchEvent(new window.Event("pointerenter", { bubbles: true }));
    assert.equal(document.documentElement.dataset.opSidebar, undefined);

    // Storage must remain untouched (no chrome persistence).
    assert.equal(storage.getItem("gwt:ui:sidebar-collapsed"), null);
    assert.equal(storage.getItem("gwt:ui:window-controls"), null);
  } finally {
    console.warn = originalWarn;
    globalThis.CustomEvent = originalCustomEvent;
  }
});

test("Issue #3340 AS-03: runtime counts never render MISSION completion", async () => {
  const { applyTelemetryCounts } = await importOperatorShell();
  assert.equal(typeof applyTelemetryCounts, "function");
  const { document } = parseHTML(html);

  applyTelemetryCounts(document, { running: 2, idle: 1, waiting: 1, error: 1, branches: 4, done: 5, agents: 5 });
  assert.ok(!document.getElementById("op-strip-mission"), "no MISSION value");
  assert.ok(!document.querySelector(".op-status-strip__cell--mission, .op-status-strip__cell--complete"), "no completion cell");
  for (const [id, value] of Object.entries({ running: "2", idle: "1", waiting: "1", error: "1", branches: "4" })) {
    assert.equal(document.getElementById(`op-strip-${id}`).textContent, value);
  }
  assert.ok(document.getElementById("op-briefing"), "Mission Briefing remains available");
});

test("Runtime health renderer updates severity-first compact PERF values", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);

  applyRuntimeHealth(document, {
    state: "warn",
    cpu_percent: 42.4,
    memory_bytes: 768 * 1024 * 1024,
    process_count: 3,
    runner_count: 1,
    queue: {
      client_count: 2,
      queued_entries: 4,
      dirty_panes: 1,
      dropped_lossy_delta: 2,
    },
    processes: [],
  });

  const cell = document.getElementById("op-strip-runtime-health");
  const value = document.getElementById("op-strip-runtime-health-value");
  assert.equal(cell?.dataset.state, "warn");
  assert.equal(value?.textContent, "WARN 42% 768M");
  assert.match(cell?.getAttribute("title") ?? "", /processes: 3/);
  assert.match(cell?.getAttribute("title") ?? "", /dropped: \+2/);
  assert.match(cell?.getAttribute("title") ?? "", runtimeCpuUnitsPattern);
  assert.match(cell?.getAttribute("aria-label") ?? "", /WARN.*42%.*768M/i);
  assert.match(cell?.getAttribute("aria-label") ?? "", runtimeCpuUnitsPattern);

  cell?.dispatchEvent(new window.Event("focus", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  assert.equal(detail?.hidden, false, "keyboard focus must expose runtime detail");
  assert.match(detail?.textContent ?? "", runtimeCpuUnitsPattern);
});

test("Runtime health renderer shows structured diagnostic hover detail", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);

  applyRuntimeHealth(document, {
    state: "hot",
    cpu_percent: 501.2,
    memory_bytes: 2 * 1024 * 1024 * 1024,
    process_count: 2,
    runner_count: 1,
    queue: {
      client_count: 1,
      queued_entries: 7,
      dirty_panes: 2,
      dropped_lossy_delta: 3,
    },
    processes: [
      {
        pid: 101,
        parent_pid: null,
        role: "gwt",
        name: "gwt",
        cpu_percent: 82.8,
        memory_bytes: 1536 * 1024 * 1024,
      },
      {
        pid: 202,
        parent_pid: 101,
        role: "runner",
        name: "chroma_index_runner",
        cpu_percent: 118.4,
        memory_bytes: 512 * 1024 * 1024,
      },
    ],
  });

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));

  const detail = document.getElementById("op-runtime-health-detail");
  assert.ok(detail, "expected runtime health detail surface");
  assert.equal(detail?.hidden, false);

  const summary = detail?.querySelector(".op-runtime-health-detail__summary");
  const queue = detail?.querySelector(".op-runtime-health-detail__queue");
  const processRows = detail?.querySelectorAll(".op-runtime-health-detail__process") ?? [];
  assert.ok(summary, "expected summary chips");
  assert.ok(queue, "expected queue diagnostics");
  assert.equal(processRows.length, 2);
  assert.equal(
    document.getElementById("op-strip-runtime-health-value")?.textContent,
    "HOT 100% 2.0G",
    "legacy aggregate payloads must clamp to the host-share ceiling",
  );
  assert.match(summary?.textContent ?? "", /HOT/);
  assert.match(summary?.textContent ?? "", /100%/);
  assert.doesNotMatch(summary?.textContent ?? "", /501%/);
  assert.match(summary?.textContent ?? "", /2.0G/);
  assert.match(detail?.textContent ?? "", runtimeCpuUnitsPattern);
  assert.match(queue?.textContent ?? "", /queued/i);
  assert.match(queue?.textContent ?? "", /7/);

  const runnerRow = [...processRows].find((row) =>
    row.textContent?.includes("chroma_index_runner"),
  );
  assert.ok(runnerRow, "expected runner process row");
  assert.equal(
    runnerRow?.querySelector(".op-runtime-health-detail__process-role")?.textContent,
    "runner",
  );
  assert.match(
    runnerRow?.querySelector(".op-runtime-health-detail__process-name")?.textContent ?? "",
    /chroma_index_runner/,
  );
  assert.match(
    runnerRow?.querySelector(".op-runtime-health-detail__process-metric")?.textContent ?? "",
    /118%/,
    "process rows keep the 1 core = 100% convention and are not aggregate-clamped",
  );
  assert.equal(
    detail?.querySelector("button.op-runtime-health-detail__process"),
    null,
    "process rows without focus targets must stay diagnostic-only",
  );
});

test("Runtime health focusable process rows call the focus callback", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);
  const focused = [];

  applyRuntimeHealth(
    document,
    {
      state: "warn",
      cpu_percent: 64.2,
      memory_bytes: 768 * 1024 * 1024,
      process_count: 2,
      runner_count: 0,
      queue: {
        client_count: 1,
        queued_entries: 0,
        dirty_panes: 0,
        dropped_lossy_delta: 0,
      },
      processes: [
        {
          pid: 301,
          parent_pid: 300,
          role: "codex",
          name: "codex",
          cpu_percent: 52.4,
          memory_bytes: 512 * 1024 * 1024,
          focus_window_id: "agent-window-1",
        },
        {
          pid: 401,
          parent_pid: null,
          role: "gwtd",
          name: "gwtd",
          cpu_percent: 2.1,
          memory_bytes: 128 * 1024 * 1024,
        },
      ],
    },
    {
      focusWindow: (windowId) => focused.push(windowId),
    },
  );

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  const focusableRows =
    detail?.querySelectorAll(".op-runtime-health-detail__process--focusable") ?? [];
  assert.equal(focusableRows.length, 1);
  assert.equal(focusableRows[0]?.tagName, "BUTTON");
  assert.equal(focusableRows[0]?.getAttribute("type"), "button");
  assert.match(focusableRows[0]?.getAttribute("aria-label") ?? "", /Focus codex process 301/);

  const staticRows =
    detail?.querySelectorAll(".op-runtime-health-detail__process:not(button)") ?? [];
  assert.equal(staticRows.length, 1, "process rows without a focus target stay plain diagnostics");

  focusableRows[0]?.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(focused, ["agent-window-1"]);
});

test("Runtime health groups Codex launcher and native child processes", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);
  const focused = [];

  applyRuntimeHealth(
    document,
    {
      state: "hot",
      cpu_percent: 84.2,
      memory_bytes: 924 * 1024 * 1024,
      process_count: 5,
      runner_count: 0,
      queue: {
        client_count: 1,
        queued_entries: 0,
        dirty_panes: 0,
        dropped_lossy_delta: 0,
      },
      processes: [
        {
          pid: 101,
          parent_pid: null,
          role: "codex",
          name: "node",
          cpu_percent: 2.4,
          memory_bytes: 48 * 1024 * 1024,
          focus_window_id: "codex-window-1",
        },
        {
          pid: 102,
          parent_pid: 101,
          role: "codex",
          name: "codex",
          cpu_percent: 37.6,
          memory_bytes: 260 * 1024 * 1024,
          focus_window_id: "codex-window-1",
        },
        {
          pid: 201,
          parent_pid: null,
          role: "codex",
          name: "node",
          cpu_percent: 0.1,
          memory_bytes: 48 * 1024 * 1024,
        },
        {
          pid: 202,
          parent_pid: 201,
          role: "codex",
          name: "codex",
          cpu_percent: 5.5,
          memory_bytes: 220 * 1024 * 1024,
        },
        {
          pid: 301,
          parent_pid: null,
          role: "gwtd",
          name: "gwtd",
          cpu_percent: 1.0,
          memory_bytes: 12 * 1024 * 1024,
        },
      ],
    },
    {
      focusWindow: (windowId) => focused.push(windowId),
    },
  );

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  const processRows = [...(detail?.querySelectorAll(".op-runtime-health-detail__process") ?? [])];
  const codexRows = processRows.filter((row) =>
    row.querySelector(".op-runtime-health-detail__process-role")?.textContent?.includes("codex"),
  );
  const focusableRows =
    detail?.querySelectorAll(".op-runtime-health-detail__process--focusable") ?? [];

  assert.equal(processRows.length, 3, "two Codex process pairs should render as two group rows");
  assert.equal(codexRows.length, 2);
  assert.match(codexRows[0]?.textContent ?? "", /codex \(2 proc\)/);
  assert.match(codexRows[0]?.textContent ?? "", /102\+1/);
  assert.match(codexRows[0]?.textContent ?? "", /40%/);
  assert.match(codexRows[0]?.textContent ?? "", /308M/);
  assert.equal(focusableRows.length, 1);

  focusableRows[0]?.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(focused, ["codex-window-1"]);
});

test("Runtime health detail defaults to Load sort and switches to CPU and Mem", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);

  applyRuntimeHealth(document, {
    state: "hot",
    cpu_percent: 122,
    memory_bytes: 5 * 1024 * 1024 * 1024,
    process_count: 3,
    runner_count: 1,
    queue: {
      client_count: 1,
      queued_entries: 0,
      dirty_panes: 0,
      dropped_lossy_delta: 0,
    },
    processes: [
      {
        pid: 701,
        parent_pid: null,
        role: "runner",
        name: "memory-hog",
        cpu_percent: 2,
        memory_bytes: 3 * 1024 * 1024 * 1024,
      },
      {
        pid: 702,
        parent_pid: null,
        role: "gwt",
        name: "cpu-burn",
        cpu_percent: 90,
        memory_bytes: 128 * 1024 * 1024,
      },
      {
        pid: 703,
        parent_pid: null,
        role: "gwtd",
        name: "balanced",
        cpu_percent: 20,
        memory_bytes: 768 * 1024 * 1024,
      },
    ],
  });

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  const rowNames = () =>
    [...(detail?.querySelectorAll(".op-runtime-health-detail__process-name") ?? [])].map(
      (node) => node.textContent,
    );
  const sortButton = (label) =>
    [...(detail?.querySelectorAll(".op-runtime-health-detail__sort-button") ?? [])].find(
      (button) => button.textContent === label,
    );
  const sortSummary = () =>
    detail?.querySelector(".op-runtime-health-detail__process-more")?.textContent ?? "";

  assert.deepEqual(rowNames(), ["memory-hog", "cpu-burn", "balanced"]);
  assert.match(sortSummary(), /sorted by Load/);
  assert.equal(sortButton("Load")?.getAttribute("aria-pressed"), "true");

  sortButton("CPU")?.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(rowNames(), ["cpu-burn", "balanced", "memory-hog"]);
  assert.match(sortSummary(), /sorted by CPU/);
  assert.equal(sortButton("CPU")?.getAttribute("aria-pressed"), "true");

  sortButton("Mem")?.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(rowNames(), ["memory-hog", "balanced", "cpu-burn"]);
  assert.match(sortSummary(), /sorted by Mem/);
  assert.equal(sortButton("Mem")?.getAttribute("aria-pressed"), "true");
});

test("Runtime health Load sort uses grouped process totals", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);

  applyRuntimeHealth(document, {
    state: "hot",
    cpu_percent: 82,
    memory_bytes: 3 * 1024 * 1024 * 1024,
    process_count: 3,
    runner_count: 0,
    queue: {
      client_count: 1,
      queued_entries: 0,
      dirty_panes: 0,
      dropped_lossy_delta: 0,
    },
    processes: [
      {
        pid: 801,
        parent_pid: null,
        role: "codex",
        name: "node",
        cpu_percent: 1,
        memory_bytes: 512 * 1024 * 1024,
      },
      {
        pid: 802,
        parent_pid: 801,
        role: "codex",
        name: "codex",
        cpu_percent: 3,
        memory_bytes: 1536 * 1024 * 1024,
      },
      {
        pid: 803,
        parent_pid: null,
        role: "gwt",
        name: "gwt",
        cpu_percent: 70,
        memory_bytes: 128 * 1024 * 1024,
      },
    ],
  });

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  const processRows = [...(detail?.querySelectorAll(".op-runtime-health-detail__process") ?? [])];

  assert.equal(processRows.length, 2);
  assert.match(processRows[0]?.textContent ?? "", /codex \(2 proc\)/);
  assert.match(processRows[0]?.textContent ?? "", /2.0G/);
  assert.match(
    detail?.querySelector(".op-runtime-health-detail__process-more")?.textContent ?? "",
    /Showing 2 groups \/ 3 processes sorted by Load/,
  );
});

test("Runtime health detail scrolls all process rows and highlights heavy focusable rows", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);
  const focused = [];
  const processes = Array.from({ length: 24 }, (_, index) => ({
    pid: 1000 + index,
    parent_pid: index === 0 ? null : 1000,
    role: index % 2 === 0 ? "runner" : "gwt",
    name: index % 2 === 0 ? "python3" : "gwt",
    cpu_percent: 60 - index,
    memory_bytes: (256 + index) * 1024 * 1024,
  }));
  processes[0] = {
    pid: 1000,
    parent_pid: null,
    role: "codex",
    name: "codex",
    cpu_percent: 72.4,
    memory_bytes: 768 * 1024 * 1024,
    focus_window_id: "heavy-agent-window",
  };
  processes[23] = {
    pid: 2023,
    parent_pid: 1000,
    role: "docker",
    name: "docker",
    cpu_percent: 0.2,
    memory_bytes: 128 * 1024 * 1024,
    focus_window_id: "docker-agent-window",
  };

  applyRuntimeHealth(
    document,
    {
      state: "hot",
      cpu_percent: 120,
      memory_bytes: 4 * 1024 * 1024 * 1024,
      process_count: 24,
      runner_count: 0,
      queue: {
        client_count: 1,
        queued_entries: 0,
        dirty_panes: 0,
        dropped_lossy_delta: 0,
      },
      processes,
    },
    {
      focusWindow: (windowId) => focused.push(windowId),
    },
  );

  const cell = document.getElementById("op-strip-runtime-health");
  cell?.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));
  const detail = document.getElementById("op-runtime-health-detail");
  const processList = detail?.querySelector(".op-runtime-health-detail__process-list");
  const processRows = detail?.querySelectorAll(".op-runtime-health-detail__process") ?? [];
  const focusableRows =
    detail?.querySelectorAll(".op-runtime-health-detail__process--focusable") ?? [];
  assert.equal(processList?.dataset.scroll, "true");
  assert.equal(processRows.length, 24);
  assert.equal(focusableRows.length, 2);
  assert.equal(processRows[0]?.dataset.heat, "hot");
  assert.equal(processRows[0]?.dataset.agent, "true");
  assert.match(processRows[0]?.textContent ?? "", /codex/);
  assert.match(focusableRows[1]?.textContent ?? "", /docker/);
  assert.match(
    detail?.querySelector(".op-runtime-health-detail__process-more")?.textContent ?? "",
    /Showing 24 processes sorted by Load/,
  );

  focusableRows[1]?.dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(focused, ["docker-agent-window"]);
});

test("Runtime health detail scroll area clamps to the viewport", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);
  Object.defineProperty(document, "defaultView", {
    configurable: true,
    value: {
      innerWidth: 760,
      innerHeight: 520,
    },
  });

  const processes = Array.from({ length: 24 }, (_, index) => ({
    pid: 1000 + index,
    parent_pid: index === 0 ? null : 1000,
    role: index % 2 === 0 ? "runner" : "gwt",
    name: index % 2 === 0 ? "python3" : "gwt",
    cpu_percent: 16 - index,
    memory_bytes: (256 + index) * 1024 * 1024,
  }));

  applyRuntimeHealth(document, {
    state: "hot",
    cpu_percent: 42,
    memory_bytes: 3 * 1024 * 1024 * 1024,
    process_count: 25,
    runner_count: 0,
    queue: {
      client_count: 1,
      queued_entries: 0,
      dirty_panes: 0,
      dropped_lossy_delta: 0,
    },
    processes,
  });

  const cell = document.getElementById("op-strip-runtime-health");
  cell.getBoundingClientRect = () => ({
    left: 680,
    top: 488,
    right: 744,
    bottom: 520,
    width: 64,
    height: 32,
  });
  cell.dispatchEvent(new window.Event("mouseenter", { bubbles: true }));

  const detail = document.getElementById("op-runtime-health-detail");
  const processRows = detail?.querySelectorAll(".op-runtime-health-detail__process") ?? [];
  const processList = detail?.querySelector(".op-runtime-health-detail__process-list");
  assert.equal(processRows.length, 24);
  assert.equal(processList?.dataset.scroll, "true");
  assert.match(
    detail?.querySelector(".op-runtime-health-detail__process-more")?.textContent ?? "",
    /Showing 24 processes sorted by Load/,
  );
  assert.equal(detail?.style.maxHeight, "472px");
  assert.equal(detail?.style.left, "332px");
  assert.equal(detail?.style.bottom, "40px");
});

function throwingStorage() {
  return {
    getItem() {
      throw new Error("storage unavailable");
    },
    setItem() {
      throw new Error("storage unavailable");
    },
    removeItem() {
      throw new Error("storage unavailable");
    },
  };
}

function memoryStorage() {
  const values = new Map();
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    removeItem(key) {
      values.delete(key);
    },
  };
}

async function importOperatorShell() {
  const modulePath = resolve(here, "../operator-shell.js");
  const source = readFileSync(modulePath, "utf8")
    .replace('from "/theme-manager.js"', `from "${pathToFileURL(resolve(here, "../theme-manager.js")).href}"`)
    .replace('from "/hotkey.js"', `from "${pathToFileURL(resolve(here, "../hotkey.js")).href}"`)
    .replace('from "/theme-toggle.js"', `from "${pathToFileURL(resolve(here, "../theme-toggle.js")).href}"`);
  const tmpDir = resolve(here, "../../../../.tmp-tests");
  mkdirSync(tmpDir, { recursive: true });
  const tmpModule = resolve(tmpDir, "operator-shell-runtime-import.mjs");
  writeFileSync(tmpModule, source);
  return import(pathToFileURL(tmpModule).href);
}

// Issue #3884 AC-3: the RUNNING cell tells how many of the running agents live
// as inline terminals inside the Issue window (SPEC-3671 `issue_preview`
// placement) instead of on the canvas, so "RUNNING 6" with an empty canvas is
// no longer a contradiction.
test("Issue #3884 AC-3: RUNNING cell breaks out inline-terminal agents", async () => {
  const { applyTelemetryCounts } = await importOperatorShell();
  const { document } = parseHTML(html);

  const runningCell = () => document.querySelector(".op-status-strip__cell--running");
  const inline = () => document.getElementById("op-strip-running-inline");
  assert.ok(inline(), "index.html carries the inline breakdown slot inside the RUNNING cell");
  assert.ok(runningCell()?.contains(inline()));

  applyTelemetryCounts(document, { running: 6, running_inline: 5 });
  assert.equal(document.getElementById("op-strip-running")?.textContent, "6");
  assert.equal(inline()?.textContent, "5 inline");
  assert.equal(inline()?.hidden, false);
  assert.match(
    runningCell()?.title || "",
    /5 of 6 running agents are inline terminals in the Issue window/,
    "hover explains where the agents are",
  );

  applyTelemetryCounts(document, { running: 1, running_inline: 0 });
  assert.equal(inline()?.hidden, true, "no breakdown when every running agent is on the canvas");
  assert.equal(runningCell()?.title, "");

  // Counts that omit the breakdown (legacy callers) keep the slot hidden.
  applyTelemetryCounts(document, { running: 2 });
  assert.equal(inline()?.hidden, true);
});

// AC-5: the project owner lane is independent of the browser connection.
test("project health renders its immutable latest frame without changing ONLINE", async () => {
  const { subscribeProjectOwnerHealth } = await importOperatorShell();
  const { document } = parseHTML(html);
  const frames = new Set();
  const model = createUiStateStore(null, {
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
  });
  const unsubscribe = subscribeProjectOwnerHealth(document, model);
  const cell = document.querySelector("[data-project-owner-health]");
  const initial = cell.dataset.state;
  const health = { state: "degraded", detail: "Project connection lost" };
  model.update(() => health);
  health.state = "healthy";
  assert.equal(cell.dataset.state, initial, "receive only commits immutable data until the display frame");
  const flush = () => { const pending = [...frames]; frames.clear(); pending.forEach(callback => callback()); };
  flush();
  assert.equal(cell.dataset.state, "degraded");
  assert.match(cell.textContent, /unavailable/i);
  assert.equal(cell.title, "Project connection lost");
  assert.equal(document.querySelector('[data-role="connection-label"]').textContent, "ONLINE");
  model.update(() => ({ state: "healthy", detail: null }));
  flush();
  assert.equal(cell.dataset.state, "healthy");
  assert.match(cell.textContent, /ready/i);
  model.update(() => ({ state: "degraded" }));
  unsubscribe();
  assert.equal(frames.size, 0, "unmount cancels the pending frame");
});


test("Issue #6992: wedged recovery requires a current explicit confirmation", async () => {
  const { subscribeProjectOwnerHealth } = await importOperatorShell();
  const { document } = parseHTML(html);
  const frames = new Set();
  const model = createUiStateStore(null, {
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
  });
  const sent = [];
  const unsubscribe = subscribeProjectOwnerHealth(document, model, { send: message => { sent.push(message); return "sent"; } });
  const flush = () => { const pending = [...frames]; frames.clear(); pending.forEach(callback => callback()); };
  model.update(() => ({ state: "wedged", recovery_id: "wedge-test-1", detail: "Health timed out repeatedly" })); flush();
  const cell = document.querySelector("[data-project-owner-health]");
  assert.equal(cell.dataset.state, "wedged");
  assert.match(cell.textContent, /Wedged/);
  const restart = cell.querySelector("[data-owner-restart]");
  assert.ok(restart);
  restart.click();
  const modal = document.querySelector("#owner-restart-modal");
  const dialog = modal.querySelector(".modal-shell");
  assert.equal(dialog.getAttribute("role"), "dialog");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  for (const primitive of ["modal-header", "modal-body", "modal-footer"]) assert.ok(dialog.querySelector(`.${primitive}`));
  assert.match(modal.textContent, /all agents/i);
  assert.match(modal.textContent, /SIGTERM.*SIGKILL/s);
  assert.equal(sent.length, 0, "opening the dialog grants no termination authority");
  modal.querySelector("[data-owner-restart-cancel]").click();
  assert.equal(sent.length, 0);
  restart.click();
  model.update(() => ({ state: "healthy" })); flush();
  assert.equal(modal.classList.contains("open"), false, "recovery invalidates stale confirmation");
  assert.equal(restart.hidden, true);
  model.update(() => ({ state: "wedged", recovery_id: "wedge-test-1" })); flush();
  restart.click();
  model.update(() => ({ state: "wedged", recovery_id: "wedge-test-2" }));
  modal.querySelector("[data-owner-restart-confirm]").click();
  assert.equal(sent.length, 0, "receive invalidates confirmation before its display frame");
  flush();
  assert.equal(modal.classList.contains("open"), false, "a different wedged incarnation invalidates the old confirmation");
  modal.querySelector("[data-owner-restart-confirm]").click();
  assert.equal(sent.length, 0);
  restart.click();
  modal.querySelector("[data-owner-restart-confirm]").click();
  assert.deepEqual(sent, [{ kind: "restart_wedged_owner", confirmed: true, recovery_id: "wedge-test-2" }]);
  assert.equal(modal.classList.contains("open"), false);
  assert.equal(restart.disabled, true, "duplicate clicks do not replay confirmation");
  model.update(() => ({ state: "wedged", recovery_id: "wedge-test-2", detail: "Diagnostics could not be saved; retry available" })); flush();
  assert.equal(restart.disabled, false, "a fresh health observation allows retry after backend refusal");
  restart.click();
  assert.equal(modal.classList.contains("open"), true);
  assert.equal(sent.length, 1, "retry still requires a new explicit confirmation");
  modal.querySelector("[data-owner-restart-cancel]").click();
  unsubscribe();
  assert.equal(document.querySelector("#owner-restart-modal"), null);
});

test("SPEC #5016 AC-HS2: one server aggregate renders the same RUN/BLOCK/IDLE/ERROR on the title, strip and Works heading", async () => {
  const { applyTelemetryCounts } = await importOperatorShell();
  const { createProjectPageMetadata } = await import("../project-page-metadata.js");
  const { createWorksSurface } = await import("../works-surface.js");
  const { document, window: domWindow } = parseHTML(html);
  const frames = new Set();
  const scheduleFrame = fn => { frames.add(fn); return () => frames.delete(fn); };
  const flush = () => { while (frames.size) for (const fn of [...frames]) { frames.delete(fn); fn(); } };
  document.hasFocus = () => false;
  const window = { addEventListener() {}, navigator: {}, getComputedStyle: () => ({ getPropertyValue: () => "" }) };
  const metadata = createProjectPageMetadata({ document, window, scheduleFrame, send() {} });
  metadata.setProjectName("Repo");
  // The census disagrees with the server on purpose: no surface may recount it.
  const windows = [{ id: "a", session_id: "s", preset: "codex", status: "running" }];
  const activity = { count: 1, agents: [{ window_id: "a", session_id: "s", runtime_incarnation: 1 }] };
  const catalog = { version: 1, runtimeStates: {}, appState: { active_tab_id: "p", tabs: [{ id: "p", workspace: { windows } }] } };
  const store = value => createUiStateStore(value, { scheduleFrame });
  const works = createWorksSurface({ document, windowCatalogModel: store({ catalog }), activityModel: store(activity),
    rosterModel: store({ members: [] }), systemSettingsModel: store({ worksPreviewLines: 3 }), summaryModel: metadata.summary,
    windowDisplayTitle: () => "Agent", mountTerminal() {}, mountPreview: () => () => {} });
  document.body.append(works.element);
  const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
  const start = appSource.indexOf("function recomputeOperatorTelemetry() {");
  let depth = 0, end = appSource.indexOf("{", start);
  do { if (appSource[end] === "{") depth += 1; else if (appSource[end] === "}") depth -= 1; end += 1; } while (depth > 0);
  const definition = appSource.slice(start, end);
  const dependencies = { window: { __operatorShell: { applyTelemetryCounts: () => {} } },
    projectAgentSummaryModel: metadata.summary, projectAgentSummary: () => null,
    agentActivityModel: { read: () => activity }, windowCatalogModel: { read: () => ({ catalog }) },
    presetSupportsWaitingStatus: () => true, activeWorkItemsFromProjection: () => [],
    applyOperatorTelemetryCounts: counts => applyTelemetryCounts(document, counts) };
  const recomputeStrip = Function(...Object.keys(dependencies), `${definition}; return recomputeOperatorTelemetry;`)(...Object.values(dependencies));
  const surfaces = () => {
    const [, run, block] = document.title.match(/RUN (\d+) · BLOCK (\d+)/);
    const strip = id => document.getElementById(`op-strip-${id}`).textContent;
    const heading = key => works.element.querySelector(`[data-count="${key}"]`).textContent;
    return {
      title: { running: run, block },
      strip: { running: strip("running"), block: strip("waiting"), idle: strip("idle"), error: strip("error") },
      works: { running: heading("running"), block: heading("block"), idle: heading("idle"), error: heading("error") },
    };
  };
  for (const [revision, aggregate] of [[1, { running_count: 3, block_count: 2, idle_count: 1, error_count: 1 }],
    [2, { running_count: 0, block_count: 4, idle_count: 0, error_count: 0 }]]) {
    metadata.update({ ...aggregate, unread: false, revision });
    flush(); recomputeStrip();
    const { title, strip, works: heading } = surfaces();
    const expected = { running: String(aggregate.running_count), block: String(aggregate.block_count),
      idle: String(aggregate.idle_count), error: String(aggregate.error_count) };
    assert.deepEqual(strip, expected, "the status strip renders the server summary");
    assert.deepEqual(heading, expected, "the Works heading renders the server summary");
    assert.deepEqual(title, { running: expected.running, block: expected.block }, "the tab title renders the server summary");
  }
  works.dispose();
});

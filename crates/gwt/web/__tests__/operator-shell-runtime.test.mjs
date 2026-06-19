import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");

test("Operator shell fails open when browser storage and media APIs are unavailable", async () => {
  const { initOperatorShell } = await importOperatorShell();
  const { document, window } = parseHTML(html);
  const briefing = document.getElementById("op-briefing");
  assert.ok(briefing, "fixture must include Mission Briefing");

  const brokenWindow = {
    ...window,
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

test("Runtime health renderer updates severity-first compact PERF values", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document } = parseHTML(html);

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
});

test("Runtime health renderer shows structured diagnostic hover detail", async () => {
  const { applyRuntimeHealth } = await importOperatorShell();
  assert.equal(typeof applyRuntimeHealth, "function");
  const { document, window } = parseHTML(html);

  applyRuntimeHealth(document, {
    state: "hot",
    cpu_percent: 101.2,
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
        cpu_percent: 18.4,
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
  assert.match(summary?.textContent ?? "", /HOT/);
  assert.match(summary?.textContent ?? "", /101%/);
  assert.match(summary?.textContent ?? "", /2.0G/);
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
    /18%/,
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

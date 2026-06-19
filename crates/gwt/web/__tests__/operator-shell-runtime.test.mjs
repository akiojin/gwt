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
  assert.equal(detail?.querySelector("button"), null, "detail must stay diagnostic-only");
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

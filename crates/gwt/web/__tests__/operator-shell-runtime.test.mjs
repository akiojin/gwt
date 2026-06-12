import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
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
  return import(`data:text/javascript,${encodeURIComponent(source)}`);
}

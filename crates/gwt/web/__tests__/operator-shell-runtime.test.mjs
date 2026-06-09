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

test("Operator shell auto-hides chrome and exposes peek 帯 hover-reveal triggers", async () => {
  // SPEC-2356 Phase 9 (FR-021/FR-032): chrome visibility runs through the
  // hover-reveal state machine driven by the sidebar peek 帯, with no chip
  // toggles, no localStorage persistence, and a one-shot legacy migration.
  // SPEC-2356 operator chrome cleanup: window controls fold into the sidebar,
  // so the separate window-controls peek 帯 is retired.
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
      "<< chip toggle must not exist after Phase 9",
    );
    assert.equal(
      document.getElementById("op-window-controls-edge-toggle"),
      null,
      "vv chip toggle must not exist after Phase 9",
    );

    const sidebarPeek = document.querySelector(".op-sidebar-peek");
    assert.ok(sidebarPeek, "fixture must include sidebar peek 帯");
    assert.equal(
      document.querySelector(".op-window-controls-peek"),
      null,
      "window controls peek 帯 is retired — controls live in the sidebar",
    );
    assert.equal(sidebarPeek.getAttribute("aria-controls"), "op-sidebar");

    // FR-032: legacy keys must be removed on init.
    assert.equal(storage.getItem("gwt:ui:sidebar-collapsed"), null);
    assert.equal(storage.getItem("gwt:ui:window-controls"), null);

    // Default state: no data-op-* attributes (auto-hidden).
    assert.equal(document.documentElement.dataset.opSidebar, undefined);
    assert.equal(document.documentElement.dataset.opWindowControls, undefined);

    // Hover the sidebar peek 帯 → revealed (window controls reveal with it).
    sidebarPeek.dispatchEvent(new window.Event("pointerenter", { bubbles: true }));
    assert.equal(document.documentElement.dataset.opSidebar, "revealed");

    // Storage must remain untouched by hover reveal (no persistence in Phase 9).
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

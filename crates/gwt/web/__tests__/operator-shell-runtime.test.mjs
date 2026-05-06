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

test("Operator shell toggles chrome visibility through edge handles", async () => {
  const { initOperatorShell } = await importOperatorShell();
  const { document, window } = parseHTML(html);
  const storage = memoryStorage();
  const testWindow = {
    ...window,
    localStorage: storage,
    sessionStorage: memoryStorage(),
    matchMedia: () => {
      throw new Error("skip animated shell loops in runtime fixture");
    },
  };

  const originalWarn = console.warn;
  const originalCustomEvent = globalThis.CustomEvent;
  console.warn = () => {};
  globalThis.CustomEvent = window.CustomEvent;
  try {
    initOperatorShell({ document, window: testWindow });

    const sidebarHandle = document.getElementById("op-sidebar-edge-toggle");
    const windowControlsHandle = document.getElementById("op-window-controls-edge-toggle");
    assert.ok(sidebarHandle, "fixture must include sidebar edge handle");
    assert.ok(windowControlsHandle, "fixture must include window controls edge handle");

    sidebarHandle.click();
    assert.equal(document.documentElement.dataset.opSidebar, "collapsed");
    assert.equal(sidebarHandle.textContent, ">>");
    assert.equal(sidebarHandle.getAttribute("aria-expanded"), "false");
    assert.equal(sidebarHandle.getAttribute("aria-label"), "Show sidebar");
    assert.equal(storage.getItem("gwt:ui:sidebar-collapsed"), "true");

    windowControlsHandle.click();
    assert.equal(document.documentElement.dataset.opWindowControls, "hidden");
    assert.equal(windowControlsHandle.textContent, "^^");
    assert.equal(windowControlsHandle.getAttribute("aria-expanded"), "false");
    assert.equal(windowControlsHandle.getAttribute("aria-label"), "Show window controls");
    assert.equal(storage.getItem("gwt:ui:window-controls"), "hidden");
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

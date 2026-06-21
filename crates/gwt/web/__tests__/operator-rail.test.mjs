/* SPEC-3038 — Command Rail behavior.
 *
 * The 56px Command Rail is grid-docked and always visible: the SPEC-2356
 * hover-reveal state machine (peek 帯 + data-op-sidebar) is retired. This
 * suite asserts the rail-era operator-shell behavior:
 *   - init never sets reveal dataset attributes
 *   - the one-shot legacy localStorage migration stays
 *   - rail [data-cmd] items dispatch op:command
 * (The rail update badge was retired with the Update CTA returning to its
 * fixed bottom-right home — user verification 2026-06-12.)
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");

test("rail: 起動時に reveal 系 dataset 属性を一切設定しない", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    assert.equal(fixture.document.documentElement.dataset.opSidebar, undefined);
    assert.equal(fixture.document.documentElement.dataset.opWindowControls, undefined);
    assert.equal(
      fixture.document.querySelector(".op-sidebar-peek"),
      null,
      "peek 帯 must not exist in the rail era",
    );
    assert.ok(fixture.document.getElementById("op-rail"), "expected #op-rail");
  } finally {
    fixture.dispose();
  }
});

test("rail commands: [data-cmd] item click が op:command を dispatch する", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const received = [];
    fixture.document.addEventListener("op:command", (event) => {
      received.push(event.detail?.id);
    });
    const startWork = fixture.document.querySelector('.op-rail__item[data-cmd="start-work"]');
    assert.ok(startWork, "expected Start Work rail item");
    startWork.dispatchEvent(new fixture.window.Event("click", { bubbles: true }));
    assert.deepEqual(received, ["start-work"]);
  } finally {
    fixture.dispose();
  }
});

test("rail window badge: applyTelemetryCounts が windows 数を badge に反映する", async () => {
  // SPEC-3038 AS-1.4: the Windows rail item carries an open-window count
  // badge; zero windows hides the badge entirely.
  const { applyTelemetryCounts } = await importOperatorShell();
  const { document } = parseHTML(html);
  const badge = document.getElementById("op-rail-window-count");
  assert.ok(badge, "expected #op-rail-window-count badge in the rail");

  applyTelemetryCounts(document, { windows: 3 });
  assert.equal(badge.textContent, "3");
  assert.equal(badge.hidden, false);

  applyTelemetryCounts(document, { windows: 0 });
  assert.equal(badge.hidden, true);
});

test("FR-039 (安心): applyTelemetryCounts が needs_input を WAITING cell に反映しアラート点滅する", async () => {
  // The WAITING cell mirrors BLOCKED: needs_input drives both the count value
  // and the --alert pulse so an agent waiting for the operator stays loud.
  const { applyTelemetryCounts } = await importOperatorShell();
  const { document } = parseHTML(html);
  const value = document.getElementById("op-strip-waiting");
  const cell = document.querySelector(".op-status-strip__cell--waiting");
  assert.ok(value, "expected #op-strip-waiting value in the strip");
  assert.ok(cell, "expected .op-status-strip__cell--waiting cell in the strip");
  assert.equal(value.getAttribute("aria-label"), "Agents waiting for input");

  applyTelemetryCounts(document, { needs_input: 2 });
  assert.equal(value.textContent, "2");
  assert.ok(
    cell.classList.contains("op-status-strip__cell--alert"),
    "WAITING cell pulses while agents wait for input",
  );

  applyTelemetryCounts(document, { needs_input: 0 });
  assert.equal(value.textContent, "0");
  assert.ok(
    !cell.classList.contains("op-status-strip__cell--alert"),
    "WAITING cell stops pulsing once no agent is waiting",
  );
});

test("migration: 起動時に旧 localStorage キーが removeItem される", async () => {
  const fixture = await mountFixture();
  try {
    fixture.storage.setItem("gwt:ui:sidebar-collapsed", "true");
    fixture.storage.setItem("gwt:ui:window-controls", "hidden");
    fixture.init();
    assert.equal(fixture.storage.getItem("gwt:ui:sidebar-collapsed"), null);
    assert.equal(fixture.storage.getItem("gwt:ui:window-controls"), null);
  } finally {
    fixture.dispose();
  }
});

test("migration: hover-reveal 機構と旧 chip ボタンは operator-shell から消えている", async () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  assert.doesNotMatch(
    operatorShell,
    /createHoverRevealController/,
    "hover-reveal state machine must be removed in the rail era",
  );
  assert.doesNotMatch(
    operatorShell,
    /op-sidebar-peek/,
    "operator-shell.js must not reference the retired peek 帯",
  );
  assert.doesNotMatch(
    operatorShell,
    /op-sidebar-edge-toggle|op-window-controls-edge-toggle/,
    "operator-shell.js は廃止 chip ボタン id を参照してはならない",
  );
  assert.doesNotMatch(
    operatorShell,
    /hotkey\.register\("cmd\+\\\\"/,
    "Cmd+\\\\ hotkey は廃止されたまま",
  );
  assert.doesNotMatch(
    operatorShell,
    /SIDEBAR_COLLAPSED_KEY\s*=\s*"gwt:ui:sidebar-collapsed"/,
    "旧永続化キー定義は削除されたまま",
  );
  assert.match(
    operatorShell,
    /removeItem\("gwt:ui:sidebar-collapsed"\)/,
    "起動時に sidebar-collapsed migration の removeItem が呼ばれること",
  );
  assert.match(
    operatorShell,
    /removeItem\("gwt:ui:window-controls"\)/,
    "起動時に window-controls migration の removeItem が呼ばれること",
  );
});

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

async function mountFixture({ reducedMotion = false } = {}) {
  const { initOperatorShell } = await importOperatorShell();
  const { document, window } = parseHTML(html);
  const storage = memoryStorage();
  const sessionStorage = memoryStorage();
  const matchMedia = (query) => {
    if (typeof query !== "string") return staticMatch(false);
    if (query.includes("prefers-reduced-motion: reduce")) return staticMatch(reducedMotion);
    return staticMatch(false);
  };
  const testWindow = {
    ...window,
    localStorage: storage,
    sessionStorage,
    matchMedia,
  };

  const originalWarn = console.warn;
  const originalCustomEvent = globalThis.CustomEvent;
  console.warn = () => {};
  globalThis.CustomEvent = window.CustomEvent;

  return {
    document,
    window,
    storage,
    init() {
      initOperatorShell({ document, window: testWindow });
    },
    dispose() {
      console.warn = originalWarn;
      globalThis.CustomEvent = originalCustomEvent;
    },
  };
}

function staticMatch(matches) {
  return {
    matches,
    media: "",
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    onchange: null,
    dispatchEvent: () => false,
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

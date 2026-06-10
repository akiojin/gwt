/* SPEC-2356 Phase 9 — Hover-reveal chrome (FR-021/FR-031/FR-032 / US4-AS8/9/12/13).
 *
 * Sidebar (`op-sidebar`) を auto-hide にし、画面左端の peek 帯への hover/focus/tap で
 * overlay 展開、200〜400ms 遅延で収納する hover-reveal state machine を assert する。
 * クリック型 chip / Cmd+\\ は廃止。
 *
 * SPEC-2356 operator chrome cleanup: window controls / Command Palette / Update CTA を
 * sidebar に集約したため、独立した window-controls peek 帯 / hover-reveal は撤去。
 * sidebar の hover-reveal と、更新到着時の sidebar peek/badge のみを assert する。
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");

test("hover-reveal: 起動時は idle (data-op-sidebar / data-op-window-controls 属性なし)", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    assert.equal(fixture.document.documentElement.dataset.opSidebar, undefined);
    assert.equal(fixture.document.documentElement.dataset.opWindowControls, undefined);
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: peek 帯への pointerenter で sidebar が即時 reveal", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    assert.ok(peek, "expected .op-sidebar-peek element");
    peek.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: window controls peek 帯 は撤去され、sidebar peek のみが残る", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    assert.equal(
      fixture.document.querySelector(".op-window-controls-peek"),
      null,
      "window controls peek 帯 must be removed — controls live in the sidebar",
    );
    assert.ok(
      fixture.document.querySelector(".op-sidebar-peek"),
      "the sidebar peek 帯 remains the single hover-reveal affordance",
    );
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: pointerleave 後 250ms 経過で sidebar が idle に戻る", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    peek.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");

    peek.dispatchEvent(new fixture.window.Event("pointerleave", { bubbles: true }));
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      "revealed",
      "panel must remain revealed while close timer is pending",
    );

    fixture.advanceTime(249);
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");
    fixture.advanceTime(2);
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      undefined,
      "panel must collapse 250ms after pointerleave + focusout",
    );
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: keyboard focusin で peek 帯から panel を reveal、focusout で 250ms 後 close", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    assert.ok(
      peek.hasAttribute("tabindex") && peek.getAttribute("tabindex") !== "-1",
      "peek 帯 must be keyboard-focusable",
    );
    peek.dispatchEvent(new fixture.window.Event("focusin", { bubbles: true }));
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");
    peek.dispatchEvent(new fixture.window.Event("focusout", { bubbles: true }));
    fixture.advanceTime(260);
    assert.equal(fixture.document.documentElement.dataset.opSidebar, undefined);
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: touch (pointerType=touch) でも sidebar peek 帯から reveal", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    const event = new fixture.window.Event("pointerdown", { bubbles: true });
    Object.defineProperty(event, "pointerType", { value: "touch" });
    peek.dispatchEvent(event);
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: prefers-reduced-motion で close delay 0ms に縮退する", async () => {
  const fixture = await mountFixture({ reducedMotion: true });
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    peek.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    assert.equal(fixture.document.documentElement.dataset.opSidebar, "revealed");
    peek.dispatchEvent(new fixture.window.Event("pointerleave", { bubbles: true }));
    fixture.advanceTime(1);
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      undefined,
      "reduced-motion must collapse the panel without any close delay",
    );
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: panel 自体への pointerenter は close timer を中断する", async () => {
  const fixture = await mountFixture();
  try {
    fixture.init();
    const peek = fixture.document.querySelector(".op-sidebar-peek");
    const sidebar = fixture.document.getElementById("op-sidebar");
    assert.ok(sidebar, "expected op-sidebar element");

    peek.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    peek.dispatchEvent(new fixture.window.Event("pointerleave", { bubbles: true }));
    fixture.advanceTime(100);
    sidebar.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    fixture.advanceTime(200);
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      "revealed",
      "moving the pointer onto the panel must keep it revealed",
    );

    sidebar.dispatchEvent(new fixture.window.Event("pointerleave", { bubbles: true }));
    fixture.advanceTime(260);
    assert.equal(fixture.document.documentElement.dataset.opSidebar, undefined);
  } finally {
    fixture.dispose();
  }
});

test("hover-reveal: window controls (Tile/Stack/Align/Windows/Add) は sidebar 内に常駐する", async () => {
  // SPEC-2356 operator chrome cleanup: window operations move into the sidebar
  // Windows section, so revealing the sidebar reveals them — there is no
  // separate window-controls reveal state to manage anymore.
  const fixture = await mountFixture();
  try {
    fixture.init();
    const sidebar = fixture.document.getElementById("op-sidebar");
    for (const id of ["tile-button", "stack-button", "align-button", "window-list-button", "add-button"]) {
      const control = fixture.document.getElementById(id);
      assert.ok(control, `expected ${id}`);
      assert.ok(sidebar.contains(control), `${id} must live inside the sidebar`);
    }

    const peek = fixture.document.querySelector(".op-sidebar-peek");
    peek.dispatchEvent(new fixture.window.Event("pointerenter", { bubbles: true }));
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      "revealed",
      "revealing the sidebar reveals the window controls along with it",
    );
  } finally {
    fixture.dispose();
  }
});

test("update peek: op:update-available で sidebar が peek し data-op-sidebar-update が付く", async () => {
  // SPEC-2356 operator chrome cleanup: the Update CTA lives in the auto-hidden
  // sidebar. When update-cta.js dispatches op:update-available the shell peeks
  // the sidebar (briefly reveal + close) and badges the peek 帯 so the user
  // notices without hovering.
  const fixture = await mountFixture();
  try {
    fixture.init();
    fixture.document.dispatchEvent(new fixture.window.CustomEvent("op:update-available"));
    assert.equal(
      fixture.document.documentElement.dataset.opSidebarUpdate,
      "available",
      "update availability must badge the sidebar peek 帯",
    );
    assert.equal(
      fixture.document.documentElement.dataset.opSidebar,
      "revealed",
      "the sidebar must peek open so the Update CTA is briefly visible",
    );

    fixture.document.dispatchEvent(new fixture.window.CustomEvent("op:update-dismissed"));
    assert.equal(
      fixture.document.documentElement.dataset.opSidebarUpdate,
      undefined,
      "dismissing the update clears the peek badge",
    );
  } finally {
    fixture.dispose();
  }
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

test("migration: 旧 chip ボタンと Cmd+\\\\ hotkey は登録されない", async () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  assert.doesNotMatch(
    operatorShell,
    /op-sidebar-edge-toggle/,
    "operator-shell.js は廃止 chip ボタン id を参照してはならない",
  );
  assert.doesNotMatch(
    operatorShell,
    /op-window-controls-edge-toggle/,
    "operator-shell.js は廃止 chip ボタン id を参照してはならない",
  );
  assert.doesNotMatch(
    operatorShell,
    /hotkey\.register\("cmd\+\\\\"/,
    "Cmd+\\\\ hotkey は廃止されるべき",
  );
  assert.doesNotMatch(
    operatorShell,
    /SIDEBAR_COLLAPSED_KEY\s*=\s*"gwt:ui:sidebar-collapsed"/,
    "旧永続化キー定義は削除されるべき",
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
  const timers = createFakeTimers();
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
    setTimeout: timers.setTimeout,
    clearTimeout: timers.clearTimeout,
  };

  const originalWarn = console.warn;
  const originalCustomEvent = globalThis.CustomEvent;
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  console.warn = () => {};
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.setTimeout = timers.setTimeout;
  globalThis.clearTimeout = timers.clearTimeout;

  return {
    document,
    window,
    storage,
    init() {
      initOperatorShell({ document, window: testWindow });
    },
    advanceTime(ms) {
      timers.advance(ms);
    },
    dispose() {
      console.warn = originalWarn;
      globalThis.CustomEvent = originalCustomEvent;
      globalThis.setTimeout = originalSetTimeout;
      globalThis.clearTimeout = originalClearTimeout;
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

function createFakeTimers() {
  let nextId = 1;
  const queue = new Map();
  let now = 0;

  const setTimeoutFn = (handler, delay = 0) => {
    const id = nextId++;
    queue.set(id, { handler, dueAt: now + Math.max(0, delay) });
    return id;
  };
  const clearTimeoutFn = (id) => {
    queue.delete(id);
  };
  const advance = (ms) => {
    const target = now + ms;
    const ordered = () =>
      Array.from(queue.entries())
        .filter(([, entry]) => entry.dueAt <= target)
        .sort((a, b) => a[1].dueAt - b[1].dueAt);
    for (let pair = ordered()[0]; pair; pair = ordered()[0]) {
      const [id, entry] = pair;
      queue.delete(id);
      now = entry.dueAt;
      try {
        entry.handler();
      } catch (error) {
        console.error("fake timer threw", error);
      }
    }
    now = target;
  };

  return { setTimeout: setTimeoutFn, clearTimeout: clearTimeoutFn, advance };
}

async function importOperatorShell() {
  const modulePath = resolve(here, "../operator-shell.js");
  const source = readFileSync(modulePath, "utf8")
    .replace('from "/theme-manager.js"', `from "${pathToFileURL(resolve(here, "../theme-manager.js")).href}"`)
    .replace('from "/hotkey.js"', `from "${pathToFileURL(resolve(here, "../hotkey.js")).href}"`)
    .replace('from "/theme-toggle.js"', `from "${pathToFileURL(resolve(here, "../theme-toggle.js")).href}"`);
  return import(`data:text/javascript,${encodeURIComponent(source)}`);
}

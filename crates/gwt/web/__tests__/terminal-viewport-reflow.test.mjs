// SPEC-2008 Phase 24 / T-184..T-186 — terminal viewport reflow on host
// resize and tab visibility transitions. Behaviour tests drive the
// extracted controller (terminal-viewport-reflow.js) so the operation
// shape is exercised end-to-end (`tasks/memory.md` 2026-05-07 memory —
// window interaction features need behavior tests, not only source-string
// contract). app.js still imports the same primitives, and a thin
// source-string assertion at the bottom makes sure the wiring stays in
// place.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import {
  applyVisibilityTransition,
  attachContainerResizeReflow,
  attachHostResizeReflow,
  classifyProjectWindowVisibility,
  createTerminalFitScheduler,
  elementHasLayoutBox,
  gateTerminalInputForReadiness,
  rearmRefreshOnVisible,
  runTerminalActivationSequence,
  viewportEligibleForRefresh,
} from "../terminal-viewport-reflow.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const appCssSource = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function fixtureWindow() {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  return document.defaultView;
}

test("attachHostResizeReflow fans fitTerminal(persist=true) across visible terminals (T-184/T-187)", () => {
  const window = fixtureWindow();
  const terminals = ["wtA", "wtB", "wtC"];
  const fitCalls = [];
  const beforeFanCalls = [];

  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => terminals,
    canRefreshViewport: (id) => id !== "wtB", // wtB is hidden / minimised.
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    beforeFan: () => beforeFanCalls.push("flushed"),
  });

  window.dispatchEvent(new window.Event("resize"));

  assert.deepEqual(beforeFanCalls, ["flushed"]);
  assert.deepEqual(fitCalls, [
    ["wtA", true],
    ["wtC", true],
  ]);

  // dispose detaches the listener so subsequent resize events do not
  // double-fire fan-out (regression against repeated wiring).
  dispose();
  window.dispatchEvent(new window.Event("resize"));
  assert.deepEqual(fitCalls.length, 2, "dispose() must remove the listener");
});

test("applyVisibilityTransition fires onReveal only on hidden -> visible with terminal (T-185/T-188)", () => {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  const make = (hidden) => {
    const el = document.createElement("section");
    el.hidden = hidden;
    return el;
  };

  // hidden -> visible with terminal: must call onReveal and clear .hidden.
  let revealed = 0;
  const hiddenWithTerminal = make(true);
  const fired = applyVisibilityTransition({
    element: hiddenWithTerminal,
    shouldHide: false,
    hasTerminal: true,
    onReveal: () => {
      revealed += 1;
    },
  });
  assert.equal(fired, true);
  assert.equal(revealed, 1);
  assert.equal(hiddenWithTerminal.hidden, false);

  // hidden -> visible but no terminal runtime yet: do NOT fire (avoids
  // scheduling fit on a window that has not mounted xterm).
  let revealedNoTerm = 0;
  const hiddenNoTerminal = make(true);
  const firedNoTerm = applyVisibilityTransition({
    element: hiddenNoTerminal,
    shouldHide: false,
    hasTerminal: false,
    onReveal: () => {
      revealedNoTerm += 1;
    },
  });
  assert.equal(firedNoTerm, false);
  assert.equal(revealedNoTerm, 0);
  assert.equal(hiddenNoTerminal.hidden, false);

  // visible -> visible: no transition, do NOT fire.
  let revealedVisible = 0;
  const visibleEl = make(false);
  applyVisibilityTransition({
    element: visibleEl,
    shouldHide: false,
    hasTerminal: true,
    onReveal: () => {
      revealedVisible += 1;
    },
  });
  assert.equal(revealedVisible, 0);

  // visible -> hidden: do NOT fire and apply the new hidden state.
  let revealedHide = 0;
  const becomingHidden = make(false);
  applyVisibilityTransition({
    element: becomingHidden,
    shouldHide: true,
    hasTerminal: true,
    onReveal: () => {
      revealedHide += 1;
    },
  });
  assert.equal(revealedHide, 0);
  assert.equal(becomingHidden.hidden, true);
});

test("viewportEligibleForRefresh skips display:none and minimised windows (T-186)", () => {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  const visibleEl = document.createElement("section");
  visibleEl.hidden = false;
  document.body.appendChild(visibleEl);
  const hiddenEl = document.createElement("section");
  hiddenEl.hidden = true;
  document.body.appendChild(hiddenEl);

  // Hidden element short-circuits before the workspace state is consulted.
  assert.equal(
    viewportEligibleForRefresh({ element: hiddenEl, workspaceWindow: { minimized: false } }),
    false,
    ".hidden element must skip refresh",
  );

  // Visible + minimised: the existing minimised short-circuit still wins.
  assert.equal(
    viewportEligibleForRefresh({ element: visibleEl, workspaceWindow: { minimized: true } }),
    false,
    "minimised workspace state must skip refresh",
  );

  // Visible + not minimised: refresh allowed.
  assert.equal(
    viewportEligibleForRefresh({ element: visibleEl, workspaceWindow: { minimized: false } }),
    true,
  );

  const disconnectedEl = document.createElement("section");
  disconnectedEl.hidden = false;
  assert.equal(
    viewportEligibleForRefresh({
      element: disconnectedEl,
      workspaceWindow: { minimized: false },
    }),
    false,
    "detached elements must skip refresh",
  );

  // Defensive: missing element / workspaceWindow falls back to allow.
  assert.equal(
    viewportEligibleForRefresh({ element: null, workspaceWindow: null }),
    true,
  );
});

test("rearmRefreshOnVisible reruns a pending hidden viewport refresh once visible (T-199)", () => {
  const calls = [];
  let pending = true;
  let visible = false;

  const attemptHidden = rearmRefreshOnVisible({
    hasPendingRefresh: () => pending,
    canRefresh: () => visible,
    clearPendingRefresh: () => {
      pending = false;
      calls.push("clear");
    },
    scheduleRefresh: () => calls.push("refresh"),
  });

  assert.equal(attemptHidden, false, "hidden windows keep the pending refresh armed");
  assert.equal(pending, true, "pending flag must survive while hidden");
  assert.deepEqual(calls, []);

  visible = true;
  const attemptVisible = rearmRefreshOnVisible({
    hasPendingRefresh: () => pending,
    canRefresh: () => visible,
    clearPendingRefresh: () => {
      pending = false;
      calls.push("clear");
    },
    scheduleRefresh: () => calls.push("refresh"),
  });

  assert.equal(attemptVisible, true);
  assert.equal(pending, false, "pending flag must clear before the refresh is scheduled");
  assert.deepEqual(calls, ["clear", "refresh"]);
});

test("rearmRefreshOnVisible is a no-op when no hidden refresh is pending (T-199)", () => {
  const calls = [];
  const didRearm = rearmRefreshOnVisible({
    hasPendingRefresh: () => false,
    canRefresh: () => true,
    clearPendingRefresh: () => calls.push("clear"),
    scheduleRefresh: () => calls.push("refresh"),
  });
  assert.equal(didRearm, false);
  assert.deepEqual(calls, []);
});

test("runTerminalActivationSequence renders before fit and emits geometry (T-199 / FR-056)", () => {
  // SPEC-2008 Phase 26.B / FR-056 regression: a hidden -> visible
  // transition must call terminal.refresh() before fitAddon.fit() so
  // xterm has populated cell metrics by the time proposeDimensions runs.
  // The previous Phase 24 ordering (fit-then-refresh) became a silent
  // no-op because proposeDimensions returns undefined whenever
  // _renderService.dimensions.css.cell.width === 0, which is exactly
  // the state of a freshly-revealed display:none element.
  const callOrder = [];
  let layoutFlushed = 0;
  const parent = {
    clientWidth: 800,
    clientHeight: 480,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      layoutFlushed += 1;
      return { width: 800, height: 480 };
    },
  };
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: parent },
      refresh: (start, end) => {
        callOrder.push(`refresh:${start}-${end}`);
      },
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
    },
  };
  let geometry = null;
  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-A",
    sendGeometry: (id, cols, rows) => {
      callOrder.push("sendGeometry");
      geometry = { id, cols, rows };
    },
  });
  assert.deepEqual(
    callOrder,
    ["refresh:0-23", "flush-layout", "fit", "sendGeometry", "focus"],
    "refresh must precede layout flush, fit, sendGeometry, and focus",
  );
  assert.equal(layoutFlushed, 1, "parent.getBoundingClientRect must be called exactly once");
  assert.deepEqual(geometry, { id: "win-A", cols: 80, rows: 24 });
  assert.equal(result.ran, true);
  assert.equal(result.cols, 80);
  assert.equal(result.rows, 24);
});

test("runTerminalActivationSequence honours shouldFocus / shouldPersistGeometry flags (T-199)", () => {
  const callOrder = [];
  const parent = {
    clientWidth: 800,
    clientHeight: 480,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      return { width: 800, height: 480 };
    },
  };
  const runtime = {
    terminal: {
      cols: 100,
      rows: 30,
      element: { parentElement: parent },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
    },
  };
  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-B",
    shouldFocus: false,
    shouldPersistGeometry: false,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });
  // sendGeometry / focus are suppressed by the flags.
  assert.deepEqual(callOrder, ["refresh", "flush-layout", "fit"]);
  assert.equal(result.ran, true);
});

test("runTerminalActivationSequence syncs geometry when focus reflow changes the xterm grid (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 112, rows: 28 }),
      fit: () => {
        callOrder.push("fit");
        runtime.terminal.cols = 112;
        runtime.terminal.rows = 28;
      },
    },
  };
  let geometry = null;

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-focus-grid-changed",
    shouldFocus: false,
    shouldPersistGeometry: false,
    syncGeometryOnGridChange: true,
    sendGeometry: (id, cols, rows) => {
      callOrder.push("sendGeometry");
      geometry = { id, cols, rows };
    },
  });

  assert.deepEqual(
    callOrder,
    ["refresh", "fit", "sendGeometry"],
    "focus reflow must sync backend geometry exactly when fit changes cols/rows",
  );
  assert.deepEqual(geometry, { id: "win-focus-grid-changed", cols: 112, rows: 28 });
  assert.deepEqual(result, { ran: true, cols: 112, rows: 28 });
});

test("runTerminalActivationSequence does not sync unchanged focus grids (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 100,
      rows: 30,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 100, rows: 30 }),
      fit: () => callOrder.push("fit"),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-focus-grid-unchanged",
    shouldFocus: false,
    shouldPersistGeometry: false,
    syncGeometryOnGridChange: true,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["refresh", "fit"]);
  assert.deepEqual(result, { ran: true, cols: 100, rows: 30 });
});

test("runTerminalActivationSequence keeps viewport-only fits off grid-change sync (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 120, rows: 32 }),
      fit: () => {
        callOrder.push("fit");
        runtime.terminal.cols = 120;
        runtime.terminal.rows = 32;
      },
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-viewport-only",
    shouldFocus: false,
    shouldPersistGeometry: false,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["refresh", "fit"]);
  assert.deepEqual(result, { ran: true, cols: 120, rows: 32 });
});

test("runTerminalActivationSequence waits for the terminal host layout box before fitting (#2839)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: {
        parentElement: {
          clientWidth: 0,
          clientHeight: 360,
          getBoundingClientRect: () => {
            callOrder.push("flush-layout");
            return { width: 0, height: 360 };
          },
        },
      },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => ({ cols: 100, rows: 28 }),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-layout-pending",
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, [], "0-size terminal host must not fit, send geometry, or focus");
  assert.deepEqual(result, { ran: false, cols: 80, rows: 24 });
});

test("runTerminalActivationSequence waits when xterm fit dimensions are unavailable (#2839)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: {
        parentElement: {
          clientWidth: 800,
          clientHeight: 420,
          getBoundingClientRect: () => {
            callOrder.push("flush-layout");
            return { width: 800, height: 420 };
          },
        },
      },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => undefined,
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-cell-pending",
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(
    callOrder,
    ["refresh", "flush-layout"],
    "unresolved xterm cell metrics must not fit, send geometry, or focus",
  );
  assert.deepEqual(result, { ran: false, cols: 80, rows: 24 });
});

test("runTerminalActivationSequence is a no-op when runtime is missing pieces (T-199)", () => {
  assert.deepEqual(
    runTerminalActivationSequence({ runtime: null, windowId: "x" }),
    { ran: false, cols: 0, rows: 0 },
  );
  assert.deepEqual(
    runTerminalActivationSequence({
      runtime: { terminal: null, fitAddon: { fit() {} } },
      windowId: "x",
    }),
    { ran: false, cols: 0, rows: 0 },
  );
  assert.deepEqual(
    runTerminalActivationSequence({
      runtime: { terminal: { rows: 24, refresh() {}, focus() {} }, fitAddon: null },
      windowId: "x",
    }),
    { ran: false, cols: 0, rows: 0 },
  );
});

test("classifyProjectWindowVisibility keeps inactive project terminals hidden, not removed", () => {
  const result = classifyProjectWindowVisibility({
    activeWindowIds: ["tab-a::agent-1", "tab-a::board-1"],
    allProjectWindowIds: [
      "tab-a::agent-1",
      "tab-a::board-1",
      "tab-b::agent-1",
    ],
    mountedWindowIds: [
      "tab-a::agent-1",
      "tab-b::agent-1",
      "orphan::agent-1",
    ],
  });

  assert.deepEqual(result.visible, ["tab-a::agent-1", "tab-a::board-1"]);
  assert.deepEqual(result.hidden, ["tab-b::agent-1"]);
  assert.deepEqual(result.removed, ["orphan::agent-1"]);
});

test("attachHostResizeReflow throws when given a non-DOM window", () => {
  assert.throws(
    () =>
      attachHostResizeReflow({
        window: null,
        terminalIds: () => [],
        canRefreshViewport: () => true,
        fitTerminal: () => {},
      }),
    /requires a DOM window/,
  );
});

test("elementHasLayoutBox blocks 0-size containers (Issue #2832 / SPEC-2008 Phase 26.A regression)", () => {
  // SPEC-2008 Phase 26.A only checked `.hidden` and `.minimized`, so a
  // structurally-visible window whose flex/grid layout had not propagated
  // could pass the visibility predicate while the parent container was
  // still 0x0. fitAddon then resolved against the broken box, isReady
  // flipped true, and the deferredWrites flushed into xterm's default
  // 80x24 grid — the Claude Code post-launch corruption symptom.
  assert.equal(elementHasLayoutBox({ clientWidth: 800, clientHeight: 480 }), true);
  assert.equal(elementHasLayoutBox({ clientWidth: 0, clientHeight: 480 }), false);
  assert.equal(elementHasLayoutBox({ clientWidth: 800, clientHeight: 0 }), false);
  assert.equal(elementHasLayoutBox({ clientWidth: 0, clientHeight: 0 }), false);

  // Falls back to getBoundingClientRect when client* are unavailable
  // (e.g. linkedom fixtures used elsewhere in this suite).
  assert.equal(
    elementHasLayoutBox({
      getBoundingClientRect: () => ({ width: 600, height: 320 }),
    }),
    true,
  );
  assert.equal(
    elementHasLayoutBox({
      getBoundingClientRect: () => ({ width: 0, height: 320 }),
    }),
    false,
  );

  // Defensive default: missing element falls through (don't pin the
  // handshake retry loop on inputs the predicate can not measure).
  assert.equal(elementHasLayoutBox(null), true);
  assert.equal(elementHasLayoutBox(undefined), true);
  assert.equal(elementHasLayoutBox({}), true);
});

test("attachHostResizeReflow coalesces rapid resize events via requestAnimationFrame (Issue #2903)", () => {
  const window = fixtureWindow();
  let rafCallback = null;
  let rafIdCounter = 1;
  let cancelledIds = [];
  window.requestAnimationFrame = (cb) => {
    rafCallback = cb;
    return rafIdCounter++;
  };
  window.cancelAnimationFrame = (id) => {
    cancelledIds.push(id);
    rafCallback = null;
  };

  const fitCalls = [];
  const beforeFanCalls = [];

  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => ["wtA", "wtB"],
    canRefreshViewport: (id) => id !== "wtB",
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    beforeFan: () => beforeFanCalls.push("flushed"),
  });

  // Fire 5 rapid resize events (simulates Chrome maximize animation).
  for (let i = 0; i < 5; i++) {
    window.dispatchEvent(new window.Event("resize"));
  }

  // Nothing should have executed synchronously — all deferred to rAF.
  assert.equal(fitCalls.length, 0, "fitTerminal must not fire synchronously when rAF is available");
  assert.equal(beforeFanCalls.length, 0, "beforeFan must not fire synchronously when rAF is available");

  // 4 intermediate rAFs should have been cancelled (5 events, only last survives).
  assert.equal(cancelledIds.length, 4, "previous rAF frames must be cancelled on rapid resize");

  // Flush the single surviving rAF callback.
  assert.ok(rafCallback, "a rAF must be scheduled after the last resize");
  rafCallback();

  // Only one fan-out should have run.
  assert.deepEqual(beforeFanCalls, ["flushed"], "beforeFan must fire exactly once");
  assert.deepEqual(fitCalls, [["wtA", true]], "fitTerminal must fire once per visible terminal");

  dispose();
});

test("attachHostResizeReflow dispose cancels pending rAF (Issue #2903)", () => {
  const window = fixtureWindow();
  let rafCallback = null;
  let cancelCount = 0;
  window.requestAnimationFrame = (cb) => { rafCallback = cb; return 99; };
  window.cancelAnimationFrame = () => { cancelCount++; rafCallback = null; };

  const fitCalls = [];
  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => ["wtA"],
    canRefreshViewport: () => true,
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
  });

  window.dispatchEvent(new window.Event("resize"));
  assert.ok(rafCallback, "rAF must be scheduled");

  // Dispose before rAF fires.
  dispose();
  assert.equal(cancelCount, 1, "dispose must cancel pending rAF");

  // Even if someone flushes an old callback ref, listener is removed.
  window.dispatchEvent(new window.Event("resize"));
  assert.equal(fitCalls.length, 0, "no fits after dispose");
});

test("createTerminalFitScheduler budgets multi-terminal fits across frames", () => {
  const callbacks = [];
  const fitCalls = [];
  const scheduler = createTerminalFitScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    maxFitsPerFrame: 4,
  });

  for (let i = 0; i < 12; i += 1) {
    scheduler.enqueue(`terminal-${i + 1}`, { persist: false });
  }

  assert.equal(callbacks.length, 1, "fit burst must schedule one shared frame");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    ["terminal-1", "terminal-2", "terminal-3", "terminal-4"],
    "first frame must only run the configured fit budget",
  );
  assert.equal(callbacks.length, 1, "remaining fits must share one follow-up frame");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    [
      "terminal-1",
      "terminal-2",
      "terminal-3",
      "terminal-4",
      "terminal-5",
      "terminal-6",
      "terminal-7",
      "terminal-8",
    ],
    "second frame must continue in insertion order",
  );
  assert.equal(callbacks.length, 1, "third frame must be scheduled for the tail");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    Array.from({ length: 12 }, (_, index) => `terminal-${index + 1}`),
    "all queued terminals must eventually fit exactly once",
  );
  assert.equal(callbacks.length, 0, "no extra frames after completion");
  assert.equal(scheduler.pendingCount(), 0);
});

test("createTerminalFitScheduler coalesces same-window fits and preserves persist=true", () => {
  const callbacks = [];
  const fitCalls = [];
  const scheduler = createTerminalFitScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    maxFitsPerFrame: 4,
  });

  assert.equal(scheduler.enqueue("agent-1", { persist: false }), true);
  assert.equal(scheduler.enqueue("agent-1", { persist: true }), true);
  assert.equal(scheduler.enqueue("agent-2", { persist: false }), true);
  assert.equal(callbacks.length, 1, "coalesced requests still share one frame");
  assert.equal(scheduler.pendingCount(), 2);

  callbacks.shift()();

  assert.deepEqual(fitCalls, [
    ["agent-1", true],
    ["agent-2", false],
  ]);
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(callbacks.length, 0);
});

test("app.js wires the reflow controller for resize, transition, and predicate", () => {
  // Source-string contract retained per the memory — limited to wiring
  // detection so a future refactor that drops the import / call surfaces
  // immediately, without claiming behaviour coverage.
  assert.match(
    appSource,
    /from "\/terminal-viewport-reflow\.js"/,
    "app.js must import terminal-viewport-reflow primitives",
  );
  assert.match(
    appSource,
    /createTerminalFitScheduler\(\{\s*fitTerminal\s*\}\)/,
    "app.js must construct the shared terminal fit scheduler from fitTerminal",
  );
  assert.match(
    appSource,
    /function scheduleTerminalFit\(windowId,\s*persist = false\)[\s\S]*?terminalFitScheduler\.enqueue\(windowId,\s*\{\s*persist\s*\}\)/,
    "app.js must expose a scheduleTerminalFit wrapper over the shared fit scheduler",
  );
  assert.match(
    appSource,
    /attachHostResizeReflow\(\{[\s\S]*?fitTerminal:\s*scheduleTerminalFit[\s\S]*?\}\)/,
    "host resize fan-out must route fit requests through the shared scheduler",
  );
  assert.match(
    appSource,
    /syncMaximizedWindowsToViewport[\s\S]*?scheduleTerminalFit\(windowData\.id,\s*false\)/,
    "maximized viewport sync must route visual terminal fits through the shared scheduler",
  );
  assert.match(
    appSource,
    /scheduleTerminalFit\(windowData\.id,\s*shouldPersistTerminalGeometry\)/,
    "workspace render geometry changes must route terminal fits through the shared scheduler",
  );
  assert.match(
    appSource,
    /applyVisibilityTransition\(\{/,
    "render path must apply visibility transition through the helper",
  );
  assert.match(
    appSource,
    /viewportEligibleForRefresh\(\{/,
    "canRefreshTerminalViewport must consult the shared predicate",
  );
  assert.match(
    appSource,
    /classifyProjectWindowVisibility\(\{/,
    "project tab switches must classify inactive project windows as hidden instead of disposing their terminal runtimes",
  );
  assert.match(
    appSource,
    /rearmRefreshOnVisible/,
    "hidden refresh requests must re-arm through the shared visibility helper",
  );
  assert.match(
    appSource,
    /document\.addEventListener\("visibilitychange"[\s\S]*?rearmVisibleTerminalViewportRefreshes\(\);/,
    "document visibility restore must re-arm visible terminal viewport refreshes",
  );
  assert.match(
    appSource,
    /function forceTerminalViewportRefresh\(windowId,[\s\S]*?viewportRefreshPending = true[\s\S]*?runTerminalActivationSequence\(\{/,
    "forceTerminalViewportRefresh must mark hidden/unresolved terminals pending and run the activation sequence when visible",
  );
  assert.match(
    appSource,
    /replaceTerminalSnapshot\([\s\S]*?forceTerminalViewportRefresh\(windowId,\s*\{\s*shouldPersistGeometry:\s*true\s*\}\);/,
    "snapshot replay must use the force refresh path so terminal.reset() cannot strand scrollback",
  );
  // SPEC-2008 Phase 26.B / FR-056 wiring: activation path must delegate
  // to runTerminalActivationSequence so refresh-before-fit ordering stays
  // testable. A future refactor that drops the helper or reverts to the
  // legacy fit-then-refresh ordering will fail this assertion and
  // surface the regression in CI immediately.
  assert.match(
    appSource,
    /runTerminalActivationSequence\(\{/,
    "scheduleTerminalFocusActivation must delegate to runTerminalActivationSequence",
  );
  assert.match(
    appSource,
    /scheduleTerminalFocusActivation\(topmostId,\s*\{\s*shouldPersistGeometry:\s*false,?\s*\}\)/,
    "topmost focus activation must not persist geometry unconditionally on every workspace render",
  );
  assert.match(
    appSource,
    /function scheduleTerminalFocusActivation\([\s\S]*?runTerminalActivationSequence\(\{[\s\S]*?shouldPersistGeometry,[\s\S]*?syncGeometryOnGridChange:\s*true,[\s\S]*?sendGeometry,[\s\S]*?\}\);/,
    "focus activation must opt into grid-change geometry sync while keeping caller-owned persistence",
  );

  // Issue #2937 — the focus-change reflow path must not give up after one
  // frame. When runTerminalActivationSequence can't resolve a real grid yet
  // (a revealed tab-group member whose container is still 0-size),
  // scheduleTerminalFocusActivation must re-arm a bounded retry
  // (activationAttempts <= HANDSHAKE_RETRY_LIMIT), mirroring the
  // initial-fit handshake. Wiring detection only; the {ran:false} contract
  // itself is covered by the unit tests above.
  assert.match(
    appSource,
    /const activation = runTerminalActivationSequence\(\{/,
    "focus activation must capture the activation result to detect !ran",
  );
  assert.match(
    appSource,
    /if \(!activation\.ran\) \{[\s\S]*?activationAttempts[\s\S]*?HANDSHAKE_RETRY_LIMIT[\s\S]*?scheduleTerminalFocusActivation\(windowId,[\s\S]*?return;/,
    "focus activation must re-arm a bounded retry when the activation did not run (#2937)",
  );
  assert.match(
    appSource,
    /activationAttempts: 0,/,
    "createTerminalRuntime must initialize activationAttempts for the focus-path retry counter (#2937)",
  );

  // Issue #2832 — SPEC-2008 Phase 26.A regression fix: completeInitialFitHandshake
  // must defer (and retry via rAF) while the container has no layout box,
  // so deferredWrites do not flush into xterm's default 80x24 grid before
  // fit can resolve real cols/rows. Wiring detection only — behavior
  // coverage lives in the elementHasLayoutBox unit test above.
  assert.match(
    appSource,
    /elementHasLayoutBox/,
    "app.js must import elementHasLayoutBox so the initial-fit handshake can gate on container layout",
  );
  assert.match(
    appSource,
    /terminalContainerHasLayoutBox\(windowId\)/,
    "completeInitialFitHandshake must consult terminalContainerHasLayoutBox",
  );
  assert.match(
    appSource,
    /function terminalContainerHasLayoutBox\(windowId\) \{[\s\S]*?terminalMap\.get\(windowId\)[\s\S]*?parentElement[\s\S]*?elementHasLayoutBox/,
    "terminalContainerHasLayoutBox must measure the actual xterm host, not only the outer workspace window",
  );
  assert.match(
    appSource,
    /handshakeAttempts/,
    "completeInitialFitHandshake must bound its retry loop with a handshakeAttempts counter",
  );
  assert.match(
    appSource,
    /HANDSHAKE_RETRY_LIMIT/,
    "handshake retry must be capped by HANDSHAKE_RETRY_LIMIT",
  );

  // Issue #2903 — browser lineHeight parity: app.js must detect Blink
  // browsers and adjust xterm lineHeight so the agent terminal line spacing
  // matches the native WebView rendering.
  assert.match(
    appSource,
    /isBlinkBrowser\b/,
    "app.js must define isBlinkBrowser helper for engine-specific lineHeight",
  );
  assert.match(
    appSource,
    /lineHeight:\s*isBlinkBrowser/,
    "createTerminalRuntime must use isBlinkBrowser to select lineHeight",
  );

  // Issue #2924 — stray "C" byte appears in Claude Code prompt buffer on
  // launch. xterm.js can emit onData firings before the initial-fit
  // handshake has completed (e.g. application-response sequences echoed
  // before the deferredWrites flush has even started). The terminal.onData
  // callback must consult gateTerminalInputForReadiness so pre-ready
  // input is dropped instead of contaminating Claude Code's stdin.
  assert.match(
    appSource,
    /gateTerminalInputForReadiness/,
    "terminal.onData must consult gateTerminalInputForReadiness so pre-ready input cannot reach PTY",
  );
});

test("app.css recovers terminal cell columns at the gwt default 720x420 window (Issue #2923 follow-up)", () => {
  // The Claude Code footer (`bypass permissions on (shift+tab to cycle)` +
  // `◯ <effort> · /effort`) lands at ~77 cells. With the original
  // `inset: 8px 10px 10px;` and xterm's vendor `overflow-y: scroll`
  // reserving a scrollbar gutter, the gwt-default 720×420 agent window
  // shrank the cell grid to ~76 cols and Claude Code's footer wrapped
  // `/effort` to `/eff` + `ort`. Pin the tighter inset and the
  // `overflow-y: auto` override so the gutter only steals cells when
  // scrollback is actually present.
  assert.match(
    appCssSource,
    /\.terminal-root\s*\{[^}]*inset:\s*8px\s+4px\s+4px;/,
    ".terminal-root must use the tightened 8px/4px/4px inset so the cell grid keeps ~+1 column at 720x420 windows",
  );
  assert.match(
    appCssSource,
    /\.surface-terminal\s+\.terminal-root\s+\.xterm-viewport\s*\{[^}]*overflow-y:\s*auto;/,
    "xterm-viewport overflow-y must override the vendor `scroll` so the scrollbar gutter is reclaimed when scrollback is empty",
  );
});

test("gateTerminalInputForReadiness drops onData firings before the initial-fit handshake (Issue #2924)", () => {
  // Pre-ready firings exist because xterm.js emits responses to
  // application queries (Primary DA, cursor reports, focus tracking)
  // synchronously inside `terminal.write`, and the deferredWrites flush
  // is itself called from inside the runtime once handshake completes.
  // The user did not press a key — these bytes are xterm.js internal
  // noise that must not reach Claude Code's stdin.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: false }, data: "C" }),
    { forward: false, reason: "runtime-not-ready" },
  );
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: false }, data: "\x1b[C" }),
    { forward: false, reason: "runtime-not-ready" },
  );
});

test("gateTerminalInputForReadiness forwards onData firings once the runtime is ready", () => {
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: true }, data: "hello" }),
    { forward: true },
  );
});

test("gateTerminalInputForReadiness forwards when no runtime is registered (defensive)", () => {
  // A missing runtime means the firing was not produced by a gated xterm
  // instance — preserve the legacy behaviour and forward, so non-PTY
  // surfaces (e.g. board / static terminals) keep working if they ever
  // route through the same helper.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: null, data: "C" }),
    { forward: true },
  );
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: undefined, data: "C" }),
    { forward: true },
  );
});

test("gateTerminalInputForReadiness forwards when isReady is missing (legacy runtime)", () => {
  // An older runtime that never set `isReady` should still forward input,
  // because the gate only takes effect when the SPEC-2008 Phase 26.A
  // handshake explicitly enrolled the runtime by setting isReady=false.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: {}, data: "C" }),
    { forward: true },
  );
});

// --- attachContainerResizeReflow: re-fit when the terminal CONTAINER size
// actually changes (maximize/restore/tile/server-geometry/no-op-fit gaps that
// the per-lifecycle-event wiring misses, leaving a black band below the grid).
function makeContainerResizeHarness(initial = { clientWidth: 800, clientHeight: 400 }) {
  const element = { ...initial };
  const observerInstances = [];
  class FakeResizeObserver {
    constructor(callback) {
      this.callback = callback;
      this.observed = [];
      this.disconnected = false;
      observerInstances.push(this);
    }
    observe(target) {
      this.observed.push(target);
    }
    disconnect() {
      this.disconnected = true;
    }
  }
  const fitCalls = [];
  let pendingFrame = null;
  const dispose = attachContainerResizeReflow({
    element,
    windowId: "win-1",
    fitTerminal: (id, persist) => fitCalls.push({ id, persist }),
    ResizeObserverImpl: FakeResizeObserver,
    requestFrame: (cb) => {
      pendingFrame = cb;
      return 7;
    },
    cancelFrame: () => {
      pendingFrame = null;
    },
  });
  const observer = observerInstances[0];
  return {
    element,
    observer,
    fitCalls,
    dispose,
    fire: () => observer.callback(),
    runFrame: () => {
      const cb = pendingFrame;
      pendingFrame = null;
      if (cb) cb();
    },
    pending: () => pendingFrame,
  };
}

test("attachContainerResizeReflow refits with persisted geometry once per coalesced size change", () => {
  const h = makeContainerResizeHarness();
  assert.ok(h.observer, "a ResizeObserver is constructed and observes the container");
  assert.deepEqual(h.observer.observed, [h.element], "observes the terminal container element");

  // Initial observation with unchanged size must NOT schedule a redundant fit
  // (createTerminalRuntime already runs the initial-fit handshake).
  h.fire();
  assert.equal(h.pending(), null, "no fit scheduled when the container size is unchanged");

  // Container grows (e.g. maximize): two rapid notifications coalesce to one fit.
  h.element.clientHeight = 900;
  h.fire();
  h.fire();
  assert.ok(h.pending(), "a frame is scheduled once the container size changes");
  h.runFrame();
  assert.deepEqual(
    h.fitCalls,
    [{ id: "win-1", persist: true }],
    "coalesced into a single fit that persists geometry to the PTY",
  );
});

test("attachContainerResizeReflow defers to the manual drag-resize path via shouldSkip", () => {
  const element = { clientWidth: 800, clientHeight: 400 };
  let cb;
  class FakeResizeObserver {
    constructor(callback) {
      cb = callback;
    }
    observe() {}
    disconnect() {}
  }
  const fitCalls = [];
  let pendingFrame = null;
  let skip = true;
  attachContainerResizeReflow({
    element,
    windowId: "win-1",
    fitTerminal: (id, persist) => fitCalls.push({ id, persist }),
    shouldSkip: () => skip,
    ResizeObserverImpl: FakeResizeObserver,
    requestFrame: (fn) => {
      pendingFrame = fn;
      return 1;
    },
    cancelFrame: () => {
      pendingFrame = null;
    },
  });
  element.clientHeight = 600;
  cb();
  assert.equal(pendingFrame, null, "no fit scheduled while a manual resize owns reflow");
  assert.equal(fitCalls.length, 0);
  // Once the manual resize ends, a later container change refits normally.
  skip = false;
  element.clientHeight = 650;
  cb();
  assert.ok(pendingFrame, "fit scheduled after the manual resize releases");
  pendingFrame();
  assert.deepEqual(fitCalls, [{ id: "win-1", persist: true }]);
});

test("attachContainerResizeReflow dispose disconnects the observer and cancels pending frame", () => {
  const h = makeContainerResizeHarness();
  h.element.clientWidth = 1200;
  h.fire();
  assert.ok(h.pending(), "frame pending before dispose");
  h.dispose();
  assert.equal(h.observer.disconnected, true, "observer disconnected on dispose");
  assert.equal(h.pending(), null, "pending frame cancelled on dispose");
});

test("attachContainerResizeReflow is a no-op when ResizeObserver is unavailable", () => {
  const dispose = attachContainerResizeReflow({
    element: { clientWidth: 10, clientHeight: 10 },
    windowId: "win-1",
    fitTerminal: () => {
      throw new Error("must not fit without a ResizeObserver");
    },
    ResizeObserverImpl: null,
  });
  assert.equal(typeof dispose, "function");
  dispose();
});

test("app.js wires attachContainerResizeReflow on the terminal container", () => {
  // Source-string contract: the container reflow controller must be imported
  // and attached in createTerminalRuntime, and disposed in cleanup.
  assert.match(
    appSource,
    /attachContainerResizeReflow/,
    "app.js must import + use attachContainerResizeReflow",
  );
});

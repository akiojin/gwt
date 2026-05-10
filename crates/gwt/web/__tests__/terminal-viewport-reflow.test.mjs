// SPEC-2008 Phase 24 / T-184..T-186 — terminal viewport reflow on host
// resize and tab visibility transitions. Behaviour tests drive the
// extracted controller (terminal-viewport-reflow.js) so the operation
// shape is exercised end-to-end (`tasks/lessons.md` 2026-05-07 lesson —
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
  attachHostResizeReflow,
  viewportEligibleForRefresh,
} from "../terminal-viewport-reflow.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

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
  const hiddenEl = document.createElement("section");
  hiddenEl.hidden = true;

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

  // Defensive: missing element / workspaceWindow falls back to allow.
  assert.equal(
    viewportEligibleForRefresh({ element: null, workspaceWindow: null }),
    true,
  );
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

test("app.js wires the reflow controller for resize, transition, and predicate", () => {
  // Source-string contract retained per the lesson — limited to wiring
  // detection so a future refactor that drops the import / call surfaces
  // immediately, without claiming behaviour coverage.
  assert.match(
    appSource,
    /from "\/terminal-viewport-reflow\.js"/,
    "app.js must import terminal-viewport-reflow primitives",
  );
  assert.match(
    appSource,
    /attachHostResizeReflow\(\{[\s\S]*?fitTerminal,\s*\n[\s\S]*?\}\)/,
    "host resize fan-out must dispatch through the reflow controller",
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
});

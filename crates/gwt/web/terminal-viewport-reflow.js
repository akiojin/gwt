// SPEC-2008 Phase 24 — operation-shape primitives for host resize and tab
// visibility transitions. Pure module so __tests__ can drive the
// behavior with linkedom + stubs (`tasks/memory.md` 2026-05-07 memory —
// window interaction features must be covered by behavior tests, not just
// source-string contracts).

/**
 * Attach a host `window.resize` listener that refreshes every visible
 * terminal viewport. The caller supplies the terminal id iterator,
 * predicate, and per-id fit hook so the controller stays decoupled from
 * the IIFE-scoped state in `app.js`.
 *
 * Returns a `dispose` function that detaches the listener — useful for
 * tests, harmless in production (bound to the lifetime of the page).
 */
export function attachHostResizeReflow({
  window,
  terminalIds,
  canRefreshViewport,
  fitTerminal,
  beforeFan,
}) {
  if (!window || typeof window.addEventListener !== "function") {
    throw new TypeError("attachHostResizeReflow requires a DOM window");
  }
  const hasRaf = typeof window.requestAnimationFrame === "function";
  let rafId = null;
  const run = () => {
    rafId = null;
    if (typeof beforeFan === "function") beforeFan();
    for (const windowId of terminalIds()) {
      if (typeof canRefreshViewport === "function" && !canRefreshViewport(windowId)) {
        continue;
      }
      fitTerminal(windowId, true);
    }
  };
  const handler = () => {
    if (hasRaf) {
      if (rafId !== null) window.cancelAnimationFrame(rafId);
      rafId = window.requestAnimationFrame(run);
    } else {
      run();
    }
  };
  window.addEventListener("resize", handler);
  return () => {
    if (rafId !== null && hasRaf) window.cancelAnimationFrame(rafId);
    window.removeEventListener("resize", handler);
  };
}

/**
 * Apply the `.hidden` mutation for a single tab and notify the caller when
 * a hidden -> visible transition occurs against a terminal-bearing window.
 * Returns `true` if the activation hook was fired.
 */
export function applyVisibilityTransition({
  element,
  shouldHide,
  hasTerminal,
  onReveal,
}) {
  if (!element) return false;
  const wasHidden = element.hidden === true;
  element.hidden = !!shouldHide;
  const becameVisible = wasHidden && !shouldHide;
  if (becameVisible && hasTerminal && typeof onReveal === "function") {
    onReveal();
    return true;
  }
  return false;
}

function idSet(values) {
  const set = new Set();
  for (const value of values || []) {
    if (value === null || value === undefined) continue;
    set.add(String(value));
  }
  return set;
}

/**
 * Classify mounted workspace windows during a project-tab render.
 *
 * Active-project windows are visible, windows that still belong to another
 * project tab stay mounted but hidden, and only windows missing from every
 * project tab are safe to dispose. This keeps inactive terminal xterm
 * runtimes alive so returning to a project can reflow instead of recreating
 * the terminal surface from scratch.
 */
export function classifyProjectWindowVisibility({
  activeWindowIds,
  allProjectWindowIds,
  mountedWindowIds,
}) {
  const active = idSet(activeWindowIds);
  const all = idSet(allProjectWindowIds);
  const visible = Array.from(active);
  const hidden = [];
  const removed = [];

  for (const windowId of mountedWindowIds || []) {
    const id = String(windowId);
    if (active.has(id)) continue;
    if (all.has(id)) {
      hidden.push(id);
    } else {
      removed.push(id);
    }
  }

  return { visible, hidden, removed };
}

/**
 * Predicate used by both the host resize fan-out and the existing
 * `fitTerminal` short-circuit. Pulled out so a unit test can pin the
 * `.hidden` short-circuit ahead of the workspace `minimized` check.
 */
export function viewportEligibleForRefresh({ element, workspaceWindow }) {
  if (element && element.hidden) return false;
  if (workspaceWindow && workspaceWindow.minimized) return false;
  return true;
}

/**
 * SPEC-2008 Phase 26.A regression fix (Issue #2832): a window can be
 * structurally visible (`.hidden = false`, not minimized) yet still have
 * no layout box at the moment the initial-fit `requestAnimationFrame`
 * fires — flex/grid layout has not propagated to the descendants, custom
 * fonts have not loaded, or the workspace is mid-render. In that state
 * `fitAddon.fit()` resolves against a 0-sized parent and silently leaves
 * xterm at its default 80×24 grid; flipping `isReady = true` then flushes
 * `deferredWrites` into that broken grid, producing the Claude-Code
 * startup corruption that ships until the next resize/move.
 *
 * The handshake should defer until the container has a real layout box.
 * `clientWidth` / `clientHeight` is preferred over `getBoundingClientRect`
 * because it ignores transforms and is cheap to read in a rAF callback.
 * `null` / missing elements fall through to `true` so non-DOM callers
 * (e.g. test fixtures without a workspace window registered) do not
 * regress.
 */
export function elementHasLayoutBox(element) {
  if (!element) return true;
  if (typeof element.clientWidth === "number" && typeof element.clientHeight === "number") {
    return element.clientWidth > 0 && element.clientHeight > 0;
  }
  if (typeof element.getBoundingClientRect === "function") {
    const rect = element.getBoundingClientRect();
    return !!rect && rect.width > 0 && rect.height > 0;
  }
  return true;
}

function currentTerminalGrid(terminal) {
  return {
    cols: typeof terminal?.cols === "number" ? terminal.cols : 0,
    rows: typeof terminal?.rows === "number" ? terminal.rows : 0,
  };
}

function fitAddonCanResolveDimensions(fitAddon) {
  if (typeof fitAddon?.proposeDimensions !== "function") return true;
  const dimensions = fitAddon.proposeDimensions();
  return (
    !!dimensions &&
    Number.isFinite(dimensions.cols) &&
    Number.isFinite(dimensions.rows) &&
    dimensions.cols > 0 &&
    dimensions.rows > 0
  );
}

/**
 * SPEC-2008 Phase 26.B / FR-056 — terminal activation must render BEFORE
 * fit. Phase 24 activation called fitAddon.fit() first, then scheduled a
 * later viewport refresh. But fitAddon.proposeDimensions() returns
 * `undefined` whenever `_renderService.dimensions.css.cell.width === 0`,
 * which is the state right after a hidden → visible transition (xterm
 * cell metrics are only populated by an actual render). The previous
 * activation therefore became a silent no-op and the viewport stayed
 * stuck on the pre-hidden cols/rows until the next OS resize.
 *
 * This helper centralises the post-activation sequence so the operation
 * shape is testable (`__tests__/terminal-viewport-reflow.test.mjs`) and
 * any future caller (snapshot replay, project switch, manual rehydrate)
 * goes through the same render-before-fit ordering.
 *
 * Steps, in order:
 *   1. `terminal.refresh(0, rows-1)` — force xterm to paint a frame so
 *      `_renderService.dimensions.css.cell.{width,height}` are non-zero.
 *   2. `parentElement.getBoundingClientRect()` — force a synchronous
 *      layout flush before fit reads the container size, otherwise the
 *      previous render's pending style changes can leave the parent
 *      `getComputedStyle` width/height at the pre-visibility value.
 *   3. `fitAddon.fit()` — proposeDimensions now sees non-zero cell width
 *      and returns the correct cols/rows.
 *   4. `sendGeometry(windowId, cols, rows)` — sync backend PTY size.
 *   5. `terminal.focus()` — restore keyboard focus.
 *
 * Returns `{ ran, cols, rows }` so tests can pin which path executed.
 */
export function runTerminalActivationSequence({
  runtime,
  windowId,
  shouldFocus = true,
  shouldPersistGeometry = true,
  sendGeometry,
}) {
  if (!runtime || !runtime.terminal || !runtime.fitAddon) {
    return { ran: false, cols: 0, rows: 0 };
  }
  const { terminal, fitAddon } = runtime;
  const currentGrid = currentTerminalGrid(terminal);
  const parent = terminal.element && terminal.element.parentElement;
  // Issue #2923: the previous gate (`parent && !elementHasLayoutBox(parent)`)
  // silently skipped when `parent` was null and let fit resolve against an
  // unknown layout reference. A null parent at activation time means the
  // terminal host is not attached, so `fitAddon.fit()` would fall back to
  // xterm's default 80×24 grid and `isReady` would flip true on that
  // wrong grid — the exact regression signature ("Welcome back Akio!"
  // splash fragmented, recovers on resize). Require a real parent with a
  // measurable layout box before continuing.
  if (!parent || !elementHasLayoutBox(parent)) {
    return { ran: false, cols: currentGrid.cols, rows: currentGrid.rows };
  }
  const rowsForRefresh = Math.max((terminal.rows || 1) - 1, 0);
  terminal.refresh(0, rowsForRefresh);
  if (typeof parent.getBoundingClientRect === "function") {
    parent.getBoundingClientRect();
  }
  if (!fitAddonCanResolveDimensions(fitAddon)) {
    return { ran: false, cols: currentGrid.cols, rows: currentGrid.rows };
  }
  fitAddon.fit();
  if (shouldPersistGeometry && typeof sendGeometry === "function") {
    sendGeometry(windowId, terminal.cols, terminal.rows);
  }
  if (shouldFocus && typeof terminal.focus === "function") {
    terminal.focus();
  }
  return { ran: true, cols: terminal.cols, rows: terminal.rows };
}

/**
 * Issue #2924: stray `C` byte appears in Claude Code's prompt buffer on
 * launch even though the user pressed nothing. xterm.js can emit `onData`
 * firings before the SPEC-2008 Phase 26.A initial-fit handshake has
 * completed — for example application-response sequences echoed during
 * the deferredWrites flush itself, or focus side-effects raised while the
 * runtime is still resolving its cell metrics. The user reports the
 * symptom is Claude-Code-only, deterministic, single-byte, and a real
 * stdin byte (Backspace removes it, Enter sends it). Mirror the
 * `deferredWrites` readiness contract: while `runtime.isReady === false`,
 * drop the input rather than forwarding it to PTY. Pre-ready bytes are
 * never produced by a real user keystroke (the terminal grid is not even
 * fitted yet) so dropping is safer than deferring (which would still
 * deliver the contaminant once the handshake completed).
 *
 * Returns `{ forward: true }` when the byte stream should be sent to PTY,
 * `{ forward: false, reason: <slug> }` when it should be dropped. The
 * reason is included so callers can emit a structured trace entry that
 * survives `gwt_input_trace` log filtering.
 *
 * The gate is intentionally lenient when no runtime is registered or the
 * runtime predates SPEC-2008 Phase 26.A (no `isReady` field), so non-PTY
 * terminal surfaces and legacy callers keep working unchanged.
 */
export function gateTerminalInputForReadiness({ runtime, data: _data }) {
  if (!runtime) return { forward: true };
  if (runtime.isReady === false) {
    return { forward: false, reason: "runtime-not-ready" };
  }
  return { forward: true };
}

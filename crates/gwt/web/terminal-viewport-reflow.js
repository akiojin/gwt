// SPEC-2008 Phase 24 — operation-shape primitives for host resize and tab
// visibility transitions. Pure module so __tests__ can drive the
// behavior with linkedom + stubs (`tasks/lessons.md` 2026-05-07 lesson —
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
  const handler = () => {
    if (typeof beforeFan === "function") beforeFan();
    for (const windowId of terminalIds()) {
      if (typeof canRefreshViewport === "function" && !canRefreshViewport(windowId)) {
        continue;
      }
      fitTerminal(windowId, true);
    }
  };
  window.addEventListener("resize", handler);
  return () => window.removeEventListener("resize", handler);
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

// SPEC-1939 Phase 13 — project-bar Index badge withdrawn (concept separation).
// `formatIndexStatusLabel` removed. The remaining helpers drive the per-tab
// `.project-tab-dot` and the Settings.Index navigation event consumed by
// callers other than the deleted badge.

export const INDEX_STATUS_OPEN_SETTINGS_EVENT = "settings:open";
export const INDEX_STATUS_OPEN_SETTINGS_TARGET = "index";

// SPEC-1939 T-IDX-107 — aggregate per-worktree files / files-docs health
// into a single dot state for the project tab indicator.
//
// Aggregation rule (plan.md):
//   any files / files-docs `error`     -> "error"  (red)
//   else status.state === "repairing"  -> "repairing" (yellow)
//   else every contributing scope ready -> "ready" (green)
//   otherwise (empty / skipped)        -> ""
//
// `issues` and `specs` are repo-shared and intentionally do not contribute.
export function aggregateProjectTabDotState(status) {
  if (!status) return "";
  const filesByWorktree = (status.scopes && status.scopes.files) || {};
  const docsByWorktree = (status.scopes && status.scopes["files-docs"]) || {};
  const contributing = [
    ...Object.values(filesByWorktree),
    ...Object.values(docsByWorktree),
  ];
  if (contributing.length === 0) return "";
  let sawError = false;
  let sawReady = false;
  for (const cell of contributing) {
    if (!cell) continue;
    if (cell.healthy === true && cell.repair_required === false) {
      sawReady = true;
      continue;
    }
    if (cell.healthy === false) {
      sawError = true;
    }
  }
  if (sawError) return "error";
  if (status.state === "repairing") return "repairing";
  if (sawReady) return "ready";
  return "";
}

// Dispatch the canonical settings:open event consumed by the Settings.Index
// tab handler (PR3). Callers (Settings button, command palette, future
// affordances) reuse this helper so the navigation event shape stays canonical.
export function dispatchOpenIndexSettings(target) {
  const dispatchTarget = target || (typeof document !== "undefined" ? document : null);
  if (!dispatchTarget) return;
  const ownerView = dispatchTarget.ownerDocument && dispatchTarget.ownerDocument.defaultView;
  const Ctor = (ownerView && ownerView.CustomEvent) || globalThis.CustomEvent;
  dispatchTarget.dispatchEvent(
    new Ctor(INDEX_STATUS_OPEN_SETTINGS_EVENT, {
      detail: { target: INDEX_STATUS_OPEN_SETTINGS_TARGET },
      bubbles: true,
    }),
  );
}

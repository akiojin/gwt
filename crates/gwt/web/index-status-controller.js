// SPEC-1939 Phase 12 / T-IDX-103..T-IDX-105 — pure formatter for the
// project bar Index status badge. The summary badge is intentionally short:
// scope/worktree details belong to the Settings.Index tab, which is wired
// in PR3.
//
// State machine:
//   ""        | "skipped"        -> hidden (no badge)
//   "ready"                       -> "Index: ready" (active color)
//   "checking"                    -> "Index: checking" (default neutral)
//   "repair_required"             -> "Index: repair" (red, "auto-rebuild not started")
//   "repairing"                   -> "Index: repairing" (yellow, spinner)
//   "error"                       -> "Index: error" (red, "auto-rebuild failed")

const READY_TITLE = "Project index is ready";
const CHECKING_TITLE = "Checking project index health";
const REPAIRING_TITLE = "Auto-rebuild in progress";
const REPAIR_REQUIRED_TITLE = "Auto-rebuild not started";
const ERROR_TITLE = "Auto-rebuild failed";

export function formatIndexStatusLabel(state) {
  if (!state || state === "skipped") {
    return {
      hidden: true,
      label: "",
      className: "index-status",
      title: "",
    };
  }
  switch (state) {
    case "ready":
      return {
        hidden: false,
        label: "Index: ready",
        className: "index-status ready",
        title: READY_TITLE,
      };
    case "repairing":
      return {
        hidden: false,
        label: "Index: repairing",
        className: "index-status repairing",
        title: REPAIRING_TITLE,
      };
    case "repair_required":
      return {
        hidden: false,
        label: "Index: repair",
        className: "index-status repair_required",
        title: REPAIR_REQUIRED_TITLE,
      };
    case "error":
      return {
        hidden: false,
        label: "Index: error",
        className: "index-status error",
        title: ERROR_TITLE,
      };
    default:
      return {
        hidden: false,
        label: "Index: checking",
        className: "index-status checking",
        title: CHECKING_TITLE,
      };
  }
}

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
// tab handler (PR3). Kept in this module so `renderIndexStatus` and any
// other badge interaction share the same payload shape.
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

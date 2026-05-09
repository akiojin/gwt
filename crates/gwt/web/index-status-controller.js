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

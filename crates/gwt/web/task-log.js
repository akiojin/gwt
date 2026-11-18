import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// SPEC #7007 T-013: the Log surface lists the notification center's #5016
// history groups exactly as the history model grouped them — one row per
// group with its occurrence count. The mapping below only translates a
// history row into the surface model (a display category and title);
// grouping identity stays with the history model, so this surface never
// re-groups by message text, and because the feed is this project page's
// single notification center, rows from another project or owner can
// never be merged in here.
export function logGroupsFromHistoryRows(rows) {
  return (rows || []).map(row => ({
    id: row.id,
    key: row.key || null,
    category: row.resolved ? "completed"
      : row.level === "error" || row.level === "warn" ? "action"
        : row.level === "success" || row.level === "done" ? "completed"
          : "information",
    title: row.issueNumber ? `${row.title} #${row.issueNumber}` : row.title,
    message: row.message ?? null,
    details: row.details ?? null,
    count: row.count ?? 1,
    latestAt: row.latestAt ?? null,
    resolved: Boolean(row.resolved),
    activatable: Boolean(row.activatable),
  }));
}

// Task-centered Log host (SPEC #7007 T-013). Rendered by the React surface
// `TaskLog` (crates/gwt/web-react). The caller feeds the mapped history
// groups through `receive`; opening the original record stays with the
// caller, which owns the notification center.
export function createTaskLog({ document, onViewNotification, scheduleFrame }) {
  const model = createUiStateStore({ groups: [] }, { scheduleFrame });
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("task-log", container, { store: model, onViewNotification });
  function receive(groups) {
    model.update(() => ({ groups }));
  }
  function dispose() {
    island.unmount();
  }
  return { element: container, receive, dispose };
}

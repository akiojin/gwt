import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Task-centered Progress host (SPEC #7007 T-012). Rendered by the React
// surface `TaskProgress` (crates/gwt/web-react). The caller derives the
// progress state from the PM window snapshot (request-task overview plus
// the live-facts active windows) and feeds it through `receive`;
// navigation callbacks stay with the caller, which owns the PM view
// switching and the existing task detail read.
export function createTaskProgress({ document, onCreateTask, onReviewDecision, onViewTaskDetails, onViewAllTasks, scheduleFrame }) {
  const model = createUiStateStore(null, { scheduleFrame });
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("task-progress", container, { store: model, onCreateTask, onReviewDecision, onViewTaskDetails, onViewAllTasks });
  function receive(state) {
    model.update(() => state);
  }
  function dispose() {
    island.unmount();
  }
  return { element: container, receive, dispose };
}

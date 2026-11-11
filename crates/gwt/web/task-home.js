import { createUiStateStore } from "./ui-state-store.js";
import { mountIsland } from "./react/islands.js";

// Task-centered Home host (SPEC #7007). Rendered by the React surface
// `TaskHome` (crates/gwt/web-react). The caller derives the home state from
// the PM window model (request-task overview, pending decision count) and
// feeds it through `receive`; navigation callbacks stay with the caller,
// which owns the PM view switching and the chat composer.
export function createTaskHome({ document, onCreateTask, onReviewDecisions, onViewProgress, scheduleFrame }) {
  const model = createUiStateStore(null, { scheduleFrame });
  const container = document.createElement("div");
  container.className = "react-island";
  const island = mountIsland("task-home", container, { store: model, onCreateTask, onReviewDecisions, onViewProgress });
  function receive(state) {
    model.update(() => state);
  }
  function dispose() {
    island.unmount();
  }
  return { element: container, receive, dispose };
}

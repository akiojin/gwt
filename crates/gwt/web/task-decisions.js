import { createUiStateStore } from "./ui-state-store.js";
import { mountIsland } from "./react/islands.js";

// Task-centered Decide host (SPEC #7007). Rendered by the React surface
// `TaskDecisions` (crates/gwt/web-react). The caller derives the decision
// state from the PM reports snapshot and feeds it through `receive`; answer
// and reply callbacks stay with the caller, which owns the PM chat send path.
export function createTaskDecisions({ document, onAnswerDecision, onReplyInChat, scheduleFrame }) {
  const model = createUiStateStore(null, { scheduleFrame });
  const container = document.createElement("div");
  container.className = "react-island";
  const island = mountIsland("task-decisions", container, { store: model, onAnswerDecision, onReplyInChat });
  function receive(state) {
    model.update(() => state);
  }
  function dispose() {
    island.unmount();
  }
  return { element: container, receive, dispose };
}

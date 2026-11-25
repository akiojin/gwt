import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Task-centered Decide host (SPEC #7007). Rendered by the React surface
// `TaskDecisions` (crates/gwt/web-react). The caller derives the decision
// state from the gwtd decision queue (Issue #7742) and feeds it through
// `receive`; answer, reply and defer callbacks stay with the caller, which
// owns the queue requests.
export function createTaskDecisions({ document, onAnswerDecision, onReplyInChat, onDeferDecision, scheduleFrame }) {
  const model = createUiStateStore(null, { scheduleFrame });
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("task-decisions", container, { store: model, onAnswerDecision, onReplyInChat, onDeferDecision });
  function receive(state) {
    model.update(() => state);
  }
  function dispose() {
    island.unmount();
  }
  return { element: container, receive, dispose };
}

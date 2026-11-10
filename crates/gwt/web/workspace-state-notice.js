import { createUiStateStore } from "./ui-state-store.js";
import { mountIsland } from "./react/islands.js";

// Persistent project-state load diagnostics, cleared only after a successful retry.
// Rendered by the React surface `WorkspaceStateNotice` (crates/gwt/web-react).
export function createWorkspaceStateNotice({ document, send, scheduleFrame }) {
  const model = createUiStateStore(null, { scheduleFrame });
  const container = document.createElement("div");
  container.className = "react-island";
  document.getElementById("app").appendChild(container);
  mountIsland("workspace-state-notice", container, { store: model, send });
  function receive(notice) {
    model.update(() => notice);
  }
  return { receive };
}

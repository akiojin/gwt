import { createUiStateStore } from "./ui-state-store.js";
import { mountIsland } from "./react/islands.js";

// Issue #7741 (SPEC #7007 item 82): host of a project's "Available execution
// environments" list inside project Settings. The list is drawn by the React
// surface `ProjectExecutionEnvironments` (crates/gwt/web-react); this host
// only hands it the latest view and forwards its saves.
const mounts = new WeakMap();

export function renderProjectExecutionEnvironments(panel, { view, error = null, send }) {
  const mounted = mounts.get(panel);
  if (mounted) {
    mounted.send = send;
    mounted.store.update(() => ({ view, error }));
    return;
  }
  const document = panel.ownerDocument;
  const mount = { send, store: createUiStateStore({ view, error }) };
  mounts.set(panel, mount);
  mountIsland("project-execution-environments", panel, {
    store: mount.store,
    onSave: ids => mount.send({ kind: "set_project_execution_environments", environment_ids: ids }),
    onManage: () => document.dispatchEvent(
      new document.defaultView.CustomEvent("environments:open", { detail: { view: "environments" } })),
  });
}

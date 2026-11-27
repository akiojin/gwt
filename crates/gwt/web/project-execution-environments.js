import { createUiStateStore } from "./ui-state-store.js";
import { mountIsland } from "./react/islands.js";

// Issue #7741 (SPEC #7007 item 82): host of a project's "Available execution
// environments" list inside project Settings. The list is drawn by the React
// surface `ProjectExecutionEnvironments` (crates/gwt/web-react); this host
// only hands it the latest view and forwards its saves.
const mounts = new WeakMap();

function render(island, kind, panel, { view, error = null, send }) {
  const mounted = mounts.get(panel);
  if (mounted) {
    mounted.send = send;
    mounted.store.update(() => ({ view, error }));
    return;
  }
  const document = panel.ownerDocument;
  const mount = { send, store: createUiStateStore({ view, error }) };
  mounts.set(panel, mount);
  mountIsland(island, panel, {
    store: mount.store,
    onSave: ids => mount.send({ kind, environment_ids: ids }),
    onManage: () => document.dispatchEvent(
      new document.defaultView.CustomEvent("environments:open", { detail: { view: "environments" } })),
  });
}

export function renderProjectExecutionEnvironments(panel, model) {
  render("project-execution-environments", "set_project_execution_environments", panel, model);
}

// Issue #7732 (SPEC #7007 items 194 and 233): the Project Manager's own list
// in the application settings. It is saved with this machine, not a project.
export function renderPmExecutionEnvironments(panel, model) {
  render("pm-execution-environments", "set_pm_execution_environments", panel, model);
}

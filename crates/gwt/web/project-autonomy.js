import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Issue #7728: host of the one project stop/start. The status, the Start /
// Stop action and the stop confirmation are drawn by the React surface
// `ProjectAutonomy` (crates/gwt/web-react); the Hub project list and Settings
// mount it through this host so both send the same `set_project_autonomy`.
export function createProjectAutonomyControl({ document, projectTitle = "", send }) {
  const element = createDetachedIslandContainer(document);
  const title = () => (typeof projectTitle === "function" ? projectTitle() : projectTitle);
  const props = autonomy => ({ autonomy, projectTitle: title(), onSetRunning: running => send(running) });
  const island = mountIsland("project-autonomy", element, props(null));
  return {
    element,
    update(autonomy) { island.update(props(autonomy)); },
    unmount() { island.unmount(); },
  };
}

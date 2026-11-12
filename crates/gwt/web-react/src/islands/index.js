// React islands: mount points that let unmigrated imperative surfaces host a
// React surface during the staged migration (SPEC #6943 FR-004).
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { WorkspaceStateNotice } from "../surfaces/WorkspaceStateNotice.jsx";
import { TaskHome } from "../surfaces/TaskHome.jsx";

const islands = Object.freeze({
  "workspace-state-notice": WorkspaceStateNotice,
  "task-home": TaskHome,
});

export function mountIsland(name, container, props) {
  const Component = islands[name];
  if (!Component) throw new Error(`Unknown React island: ${name}`);
  const root = createRoot(container);
  let mounted = true;
  const render = next => flushSync(() => root.render(createElement(Component, next)));
  render(props);
  return {
    update(next) {
      if (!mounted) throw new Error(`React island ${name} is unmounted`);
      render(next);
    },
    unmount() {
      if (!mounted) return;
      mounted = false;
      root.unmount();
    },
  };
}

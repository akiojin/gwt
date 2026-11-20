// React islands: mount points that let unmigrated imperative surfaces host a
// React surface during the staged migration (SPEC #6943 FR-004).
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
export { createIslandContainer, createDetachedIslandContainer } from "../hosts/island-container.js";
import { WorkspaceStateNotice } from "../surfaces/WorkspaceStateNotice.jsx";
import { TaskHome } from "../surfaces/TaskHome.jsx";
import { TaskDecisions } from "../surfaces/TaskDecisions.jsx";
import { TaskProgress } from "../surfaces/TaskProgress.jsx";
import { TaskLog } from "../surfaces/TaskLog.jsx";
import { ExecutionEnvironments } from "../surfaces/ExecutionEnvironments.jsx";
import { AppFooter, AppFooterPm, AppHeader, AppHeaderNav } from "../surfaces/AppChrome.jsx";
import { ProjectExecutionEnvironments } from "../surfaces/ProjectExecutionEnvironments.jsx";
import { HomeConversation } from "../surfaces/HomeConversation.jsx";
import { ProjectThemes } from "../surfaces/ProjectThemes.jsx";
import { AgentHistoryPlans } from "../surfaces/AgentHistoryPlans.jsx";

const islands = Object.freeze({
  "workspace-state-notice": WorkspaceStateNotice,
  "task-home": TaskHome,
  "task-decisions": TaskDecisions,
  "task-progress": TaskProgress,
  "task-log": TaskLog,
  "execution-environments": ExecutionEnvironments,
  "app-header": AppHeader,
  "app-header-nav": AppHeaderNav,
  "app-footer": AppFooter,
  "app-footer-pm": AppFooterPm,
  "project-execution-environments": ProjectExecutionEnvironments,
  "home-conversation": HomeConversation,
  "project-themes": ProjectThemes,
  "agent-history-plans": AgentHistoryPlans,
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

import { createUiStateStore } from "./ui-state-store.js";

const unknownActivity = () => ({ snapshot: null, agents: [], byIssue: {}, count: null, issueCount: 0, pmCount: 0 });

// Display-only membership. Window status, Work history and Monitor reservations
// are deliberately absent: only the host's observed child census proves live use.
export function createAgentActivityModel(projectKey) {
  const model = createUiStateStore(unknownActivity());
  function clearAgentActivity() { model.update(unknownActivity); }
  function applyAgentActivitySnapshot(snapshot) {
    if (!snapshot) return;
    if (snapshot.project_key !== projectKey || !snapshot.host_id ||
        !Number.isFinite(Date.parse(snapshot.observed_at)) ||
        (snapshot.agents !== null && !Array.isArray(snapshot.agents))) return;
    model.update(state => {
      if (state.snapshot && Date.parse(snapshot.observed_at) < Date.parse(state.snapshot.observed_at)) return state;
      if (snapshot.agents === null) return { ...unknownActivity(), snapshot };
      const seen = {};
      const agents = snapshot.agents.filter(agent => {
        if (!agent.window_id || !agent.session_id || !Number.isSafeInteger(agent.runtime_incarnation) ||
            agent.runtime_incarnation < 1 || Object.hasOwn(seen, agent.window_id)) return false;
        seen[agent.window_id] = true;
        return true;
      });
      const byIssue = {};
      for (const agent of agents) {
        if (Number.isSafeInteger(agent.issue_number) && agent.issue_number > 0) {
          (byIssue[agent.issue_number] ??= []).push(agent);
        }
      }
      return { snapshot, agents, byIssue, count: agents.length,
        issueCount: Object.keys(byIssue).length, pmCount: agents.filter(agent => agent.is_pm).length };
    });
  }
  return { model, applyAgentActivitySnapshot, clearAgentActivity };
}

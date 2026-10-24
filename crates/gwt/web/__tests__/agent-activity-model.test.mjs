import assert from "node:assert/strict";
import { test } from "node:test";
import { createAgentActivityModel } from "../agent-activity-model.js";

const agent = (id, issue_number = null, is_pm = false) => ({
  window_id: `project::${id}`, session_id: `session-${id}`,
  runtime_incarnation: 1, issue_number, is_pm,
});
const snapshot = (agents, overrides = {}) => ({ project_key: "project", host_id: "host",
  observed_at: "2026-10-24T14:00:00Z", agents, ...overrides });

test("AC34 one shared census counts agents, distinct issues and PM independently", () => {
  const { model, applyAgentActivitySnapshot, clearAgentActivity } = createAgentActivityModel("project");
  assert.equal(model.read().count, null, "missing evidence is unknown, not a measured zero");
  const pm = agent("pm", null, true);
  const agents = [pm, agent("one", 4777), agent("two", 4777), agent("three", 5016), agent("four", 3431)];
  applyAgentActivitySnapshot(snapshot([...agents, pm]));
  assert.equal(model.read().count, 5);
  assert.equal(model.read().issueCount, 3);
  assert.equal(model.read().pmCount, 1);
  assert.equal(model.read().byIssue[4777].length, 2);
  applyAgentActivitySnapshot(snapshot(agents.slice(0, 2), { observed_at: "2026-10-24T14:00:01Z" }));
  assert.equal(model.read().count, 2, "a child exit removes membership without a Work mutation");
  applyAgentActivitySnapshot(snapshot(agents));
  assert.equal(model.read().count, 2, "an older projection cannot resurrect exited children");
  applyAgentActivitySnapshot(snapshot(agents, { project_key: "other", observed_at: "2026-10-24T14:00:02Z" }));
  assert.equal(model.read().count, 2, "another project cannot replace this census");
  clearAgentActivity();
  assert.equal(model.read().count, null, "disconnection invalidates the census");
  applyAgentActivitySnapshot(snapshot([]));
  assert.equal(model.read().count, 0, "an observed empty census is a measured zero");
});

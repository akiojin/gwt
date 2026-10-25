import assert from "node:assert/strict";
import { test } from "node:test";
import { createAgentActivityModel } from "../agent-activity-model.js";

test("roster receipts are project-scoped and stale revisions cannot undo a member change", async () => {
  const { createAgentRosterModel } = await import("../agent-activity-model.js");
  const { model, applyAgentRosterSnapshot, applyAgentRosterError } = createAgentRosterModel("project");
  const member = { agent_member_id: "member-1", name: "Ada", enabled: true };
  applyAgentRosterSnapshot({ project_key: "other", revision: 4, members: [member] });
  assert.equal(model.read().members.length, 0);
  applyAgentRosterSnapshot({ project_key: "project", revision: 3, members: [member], capacity: 1 });
  assert.equal(model.read().members[0].member_id, "member-1");
  applyAgentRosterSnapshot({ project_key: "project", revision: 2, members: [] });
  assert.equal(model.read().members.length, 1);
  applyAgentRosterSnapshot({ project_key: "project", revision: 3, members: [member], error: "stale revision; refresh and retry" });
  assert.match(model.read().error, /refresh/);
  applyAgentRosterError({ project_key: "project", message: "Read unavailable" });
  assert.equal(model.read().error, "Read unavailable");
});

test("Works telemetry sums measured project conversations once, preserves missing and excludes account totals", async () => {
  const { selectWorksTelemetry } = await import("../agent-activity-model.js");
  const usage = { session_id: "s1", provider: "codex", provider_session_id: "conversation", project_path: "/repo", active: true,
    state: { kind: "ok" }, input_tokens: 20, output_tokens: 0, cache_read_tokens: 10, cache_creation_tokens: null,
    context_left_pct: 80, reported_context_left_pct: null,
    telemetry: { today_input_tokens: 20, today_output_tokens: 0, today_cache_read_tokens: 10,
      today_cache_creation_tokens: null, minute_buckets: Array.from({ length: 30 }, (_, i) => ({ started_at: String(i), tokens: 0 })) } };
  const params = { projectRoot: "/repo", sessions: [usage, { ...usage, project_path: "/other", input_tokens: 999 }],
    agents: [{ window_id: "w1", session_id: "s1" }], works: [{ agents: [{ session_id: "s1" }], works: [
      { agents: [{ session_id: "old" }], execution_diagnosis: { verification_state: "failed" } },
      { agents: [{ session_id: "s1" }], execution_diagnosis: { verification_state: "passed" } },
    ] }] };
  const result = selectWorksTelemetry(params);
  assert.equal(result.today.input_tokens, 20);
  assert.equal(result.today.cache_tokens, 10);
  assert.equal(result.by_window.w1.cache_tokens, 10);
  const claude = { ...usage, provider: "claude_code", cache_creation_tokens: 3,
    telemetry: { ...usage.telemetry, today_cache_creation_tokens: 3 } };
  const claudeResult = selectWorksTelemetry({ ...params, sessions: [claude] });
  assert.equal(claudeResult.by_window.w1.cache_tokens, 13);
  assert.equal(claudeResult.today.cache_tokens, 13);
  const incompleteClaude = selectWorksTelemetry({ ...params, sessions: [{ ...usage, provider: "claude_code" }] });
  assert.equal(incompleteClaude.by_window.w1.cache_tokens, null);
  assert.equal(incompleteClaude.today.cache_tokens, null);
  assert.equal(result.today.output_tokens, 0);
  assert.equal(result.today.cost_usd, null);
  assert.equal(result.by_window.w1.verification, "Passed");
  assert.equal(result.by_window.w1.context_remaining_percent, null, "estimated context is never presented as reported");
  assert.equal(result.fleet_tokens_per_minute[29], 0);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [{ ...usage, telemetry: null }] }).today.input_tokens, null);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [usage, usage] }).today.input_tokens, 20);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [{ ...usage, state: { kind: "no_data" }, telemetry: null }] }).by_window.w1.input_tokens, null);
});

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

test("AC34 unmeasured projections preserve the census and observed unknown keeps its watermark", () => {
  const { model, applyAgentActivitySnapshot } = createAgentActivityModel("project");
  const agents = [agent("one", 4777)];
  applyAgentActivitySnapshot(snapshot(agents));
  applyAgentActivitySnapshot(undefined);
  assert.equal(model.read().count, 1, "a cleanup producer without a census cannot erase measured activity");
  applyAgentActivitySnapshot(snapshot(null, { observed_at: "2026-10-24T14:00:02Z" }));
  assert.equal(model.read().count, null, "an unavailable observation is unknown, not zero");
  applyAgentActivitySnapshot(snapshot(agents, { observed_at: "2026-10-24T14:00:01Z" }));
  assert.equal(model.read().count, null, "an older positive cannot replace an unavailable observation");
  applyAgentActivitySnapshot(snapshot([], { observed_at: "2026-10-24T14:00:03Z" }));
  assert.equal(model.read().count, 0, "a fresh empty observation is a measured zero");
});


test("member tokens use measured conversations and never count Codex cache twice", async () => {
  const { selectWorksTelemetry } = await import("../agent-activity-model.js");
  const row = { project_path: "/repo", state: { kind: "ok" }, output_tokens: 2, active: true };
  const result = selectWorksTelemetry({ projectRoot: "/repo", sessions: [
    { ...row, provider: "codex", provider_session_id: "c1", agent_member_id: "m1", input_tokens: 20, cache_read_tokens: 10, cache_creation_tokens: 0 },
    { ...row, provider: "claude_code", provider_session_id: "c2", agent_member_id: "m2", input_tokens: 5, cache_read_tokens: 7, cache_creation_tokens: 3 },
    { ...row, provider: "claude_code", provider_session_id: "c3", agent_member_id: "m3", input_tokens: 5, cache_read_tokens: null, cache_creation_tokens: 3 },
  ] });
  assert.equal(result.by_member.m1, 22);
  assert.equal(result.by_member.m2, 17);
  assert.equal(result.by_member.m3, null);
});

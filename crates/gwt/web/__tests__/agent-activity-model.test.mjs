import assert from "node:assert/strict";
import { test } from "node:test";
import { createAgentActivityModel, createAgentRosterModel, createAgentScheduleModel } from "../agent-activity-model.js";

test("plans accept only canonical project revisions and keep the current plan until readback", () => {
  const { model, applyAgentScheduleSnapshot, applyAgentScheduleError,
    beginAgentScheduleRequest, clearAgentSchedule } = createAgentScheduleModel("project");
  const snapshot = { project_key: "project", roster_revision: 3, queue_revision: "queue-3",
    schedule_revision: 2, members: [{ agent_member_id: "m1", issues: [{ issue_number: 42, reason: "queue_order" }], empty_reason: null }] };
  applyAgentScheduleSnapshot({ ...snapshot, project_key: "other" });
  assert.equal(model.read().snapshot, null);
  applyAgentScheduleSnapshot(snapshot);
  const action = { kind: "reassign", issue_number: 42, source_member_id: "m1", target_member_id: "m2",
    expected_roster_revision: 3, expected_queue_revision: "queue-3", expected_schedule_revision: 2 };
  assert.equal(beginAgentScheduleRequest(action), true);
  assert.equal(beginAgentScheduleRequest(action), false, "only one move can be pending");
  assert.equal(model.read().pending, true);
  assert.deepEqual(model.read().snapshot, snapshot, "sending is not a successful move");
  applyAgentScheduleSnapshot({ ...snapshot });
  assert.equal(model.read().pending, true, "a delayed pre-move read is not the move's acknowledgement");
  applyAgentScheduleError({ project_key: "other", message: "unrelated" });
  assert.equal(model.read().pending, true);
  applyAgentScheduleError({ project_key: "project", message: "Schedule changed; refresh and retry" });
  assert.deepEqual(model.read().snapshot, snapshot);
  assert.equal(model.read().pending, false);
  assert.match(model.read().error, /refresh/);
  assert.equal(beginAgentScheduleRequest(action), false, "an error requires a canonical refresh");
  assert.equal(beginAgentScheduleRequest({ kind: "read" }), true);
  assert.equal(beginAgentScheduleRequest({ kind: "read" }), true, "Refresh recovers a lost response");
  applyAgentScheduleSnapshot({ ...snapshot, schedule_revision: 1 });
  assert.equal(model.read().pending, true, "a stale reply cannot ACK the pending read");
  const saved = { ...snapshot, schedule_revision: 3, members: [{ agent_member_id: "m2", issues: [{ issue_number: 42, reason: "assigned_by_you" }] }] };
  applyAgentScheduleSnapshot(saved);
  assert.deepEqual(model.read().snapshot, saved);
  assert.equal(model.read().error, null);
  assert.equal(model.read().pending, false);
  assert.equal(beginAgentScheduleRequest(action), false, "a captured stale action cannot be submitted");
  applyAgentScheduleSnapshot({ ...snapshot, roster_revision: 2, schedule_revision: 4 });
  assert.deepEqual(model.read().snapshot, saved, "an older roster cannot resurrect members");
  clearAgentSchedule();
  assert.equal(model.read().snapshot, null, "disconnect invalidates mutation authority");
  assert.equal(beginAgentScheduleRequest(action), false);
});

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

const namedRoster = (revision, name = "Ada", overrides = {}) => ({ project_key: "project", revision,
  members: [{ agent_member_id: "m1", name, enabled: true, assignment: { session_id: "s1" }, agent_id: "codex", history: [{ name: "Ada" }] },
    { agent_member_id: "m2", name: "Resting", enabled: false }], ...overrides });
function nameController(sent) {
  const frames = new Set(), controller = createAgentRosterModel("project", { onRename: request => sent.push(request),
    scheduleFrame: callback => { frames.add(callback); return () => frames.delete(callback); } });
  return { ...controller, flush() { for (const callback of [...frames]) { frames.delete(callback); callback(); } } };
}

test("member name autosave serializes latest drafts and accepts only correlated receipts", () => {
  const sent = [], { model, editAgentMemberName, applyAgentRosterSnapshot, applyAgentRosterError, flush } = nameController(sent);
  applyAgentRosterSnapshot(namedRoster(3));
  editAgentMemberName("m1", "  Maple  ");
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].action, { kind: "rename", member_id: "m1", name: "Maple" });
  assert.equal(sent[0].expected_revision, 3); assert.ok(sent[0].request_id);
  assert.equal(model.read().members[0].name, "Ada", "sending never changes canonical identity");
  editAgentMemberName("m1", "Cedar"); editAgentMemberName("m1", "Birch");
  applyAgentRosterSnapshot(namedRoster(3));
  applyAgentRosterError({ project_key: "project", request_id: "older", message: "Old error" });
  assert.equal(sent.length, 1, "uncorrelated traffic cannot release the serialized writer");
  applyAgentRosterSnapshot(namedRoster(4, "Maple"), sent[0].request_id);
  assert.equal(sent.length, 1, "a receive handler only commits state; queued transport waits for the model effect frame");
  flush();
  assert.equal(sent.length, 2);
  assert.equal(sent[1].action.name, "Birch"); assert.equal(sent[1].expected_revision, 4);
  assert.notEqual(sent[1].request_id, sent[0].request_id);
  applyAgentRosterSnapshot(namedRoster(5, "Birch"), sent[1].request_id);
  assert.equal(model.read().name_edits.m1.status, "saved");
  assert.equal(model.read().name_edits.m1.value, "Birch");
  assert.equal(model.read().members[0].member_id, "m1");
  assert.deepEqual(model.read().members[0].assignment, { session_id: "s1" });
  assert.deepEqual(model.read().members[0].history, [{ name: "Ada" }]);
  applyAgentRosterSnapshot(namedRoster(6, "Current canonical name"));
  assert.equal(model.read().name_edits.m1.value, "Current canonical name", "a completed draft follows a later canonical rename");
});

test("member name autosave preserves a word separator through ACK, no-op and readback", () => {
  const sent = [], { model, editAgentMemberName, applyAgentRosterSnapshot, flush } = nameController(sent);
  applyAgentRosterSnapshot(namedRoster(3));
  editAgentMemberName("m1", "Maple ");
  assert.equal(sent[0].action.name, "Maple", "only the saved name is trimmed");
  applyAgentRosterSnapshot(namedRoster(4, "Maple"), sent[0].request_id);
  flush();
  assert.equal(model.read().name_edits.m1.value, "Maple ", "ACK preserves the next word's separator");
  editAgentMemberName("m1", "Maple  ");
  applyAgentRosterSnapshot(namedRoster(4, "Maple"));
  flush();
  assert.equal(sent.length, 1, "whitespace-only edits do not send another write");
  assert.equal(model.read().name_edits.m1.value, "Maple  ", "no-op and readback preserve the raw draft");
  editAgentMemberName("m1", `${model.read().name_edits.m1.value}renamed`);
  assert.equal(sent[1].action.name, "Maple  renamed");
  applyAgentRosterSnapshot(namedRoster(5, "Maple  renamed"), sent[1].request_id);
  applyAgentRosterSnapshot(namedRoster(6, "Changed elsewhere"));
  assert.equal(model.read().name_edits.m1.value, "Changed elsewhere", "a different canonical name still updates the completed draft");
});

test("member names preserve invalid and composing drafts without submitting them", () => {
  const sent = [], { model, editAgentMemberName, applyAgentRosterSnapshot } = nameController(sent);
  applyAgentRosterSnapshot(namedRoster(3));
  editAgentMemberName("m1", "   ");
  assert.equal(model.read().name_edits.m1.value, "   "); assert.match(model.read().name_edits.m1.error, /display name/i);
  editAgentMemberName("m1", "  rEsTiNg  ");
  assert.match(model.read().name_edits.m1.error, /already registered/);
  editAgentMemberName("m1", "ま", { composing: true });
  assert.equal(sent.length, 0);
  editAgentMemberName("m1", "楓");
  assert.equal(sent.length, 1);
  editAgentMemberName("m1", "", { composing: true });
  applyAgentRosterSnapshot(namedRoster(4, "楓"), sent[0].request_id);
  assert.equal(model.read().name_edits.m1.value, "", "an earlier success never erases unfinished IME input");
  editAgentMemberName("m1", "");
  assert.equal(sent.length, 1); assert.match(model.read().name_edits.m1.error, /display name/i);
});

test("stale or failed name receipts retain drafts until explicit retry at the current revision", () => {
  const sent = [], controller = nameController(sent);
  const { model, editAgentMemberName, retryAgentMemberName, applyAgentRosterSnapshot, applyAgentRosterError } = controller;
  applyAgentRosterSnapshot(namedRoster(3)); editAgentMemberName("m1", "Maple");
  applyAgentRosterSnapshot(namedRoster(5, "Another writer"));
  applyAgentRosterSnapshot(namedRoster(4, "Maple"), sent[0].request_id);
  assert.equal(model.read().revision, 5); assert.equal(model.read().members[0].name, "Another writer");
  assert.equal(model.read().name_edits.m1.value, "Maple"); assert.equal(model.read().name_edits.m1.status, "error");
  applyAgentRosterSnapshot(namedRoster(6, "Another writer")); editAgentMemberName("m1", "Cedar");
  assert.equal(sent.length, 1, "conflicts require an explicit retry, even after further editing or refresh");
  retryAgentMemberName("m1"); assert.equal(sent.length, 2); assert.equal(sent[1].expected_revision, 6);
  applyAgentRosterError({ project_key: "other", request_id: sent[1].request_id, message: "Unrelated" });
  assert.equal(model.read().name_edits.m1.status, "saving");
  applyAgentRosterError({ project_key: "project", request_id: sent[1].request_id, message: "Connection lost; retry" });
  assert.equal(model.read().name_edits.m1.status, "error"); assert.equal(model.read().name_edits.m1.value, "Cedar");
  retryAgentMemberName("m1");
  applyAgentRosterSnapshot(namedRoster(7, "Cedar"), sent[2].request_id);
  assert.equal(model.read().name_edits.m1.status, "saved");
});

test("the name writer preserves other member drafts and revalidates coalesced edits after readback", () => {
  const sent = [], { model, editAgentMemberName, applyAgentRosterSnapshot, clearAgentRosterRename, flush } = nameController(sent);
  applyAgentRosterSnapshot(namedRoster(3));
  editAgentMemberName("m1", "Maple"); editAgentMemberName("m1", "Cedar"); editAgentMemberName("m2", "Birch");
  assert.equal(sent.length, 1);
  applyAgentRosterSnapshot(namedRoster(4, "Maple", { members: [
    { agent_member_id: "m1", name: "Maple", enabled: true }, { agent_member_id: "m2", name: "Cedar", enabled: false },
  ] }), sent[0].request_id);
  assert.equal(model.read().name_edits.m1.status, "invalid");
  assert.match(model.read().name_edits.m1.error, /already registered/);
  flush();
  assert.equal(sent.length, 2); assert.equal(sent[1].action.member_id, "m2");
  assert.equal(sent[1].action.name, "Birch"); assert.equal(sent[1].expected_revision, 4);
  clearAgentRosterRename();
  assert.equal(model.read().pending_name, null); assert.equal(model.read().name_edits.m2.status, "error");
  assert.match(model.read().name_edits.m2.error, /Connection lost/);
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
    model: "claude-sonnet-4-5",
    telemetry: { ...usage.telemetry, today_cache_creation_tokens: 3 } };
  const claudeResult = selectWorksTelemetry({ ...params, sessions: [claude], bySession: { s1: { model: null, branch: "pm/resident" } } });
  assert.equal(claudeResult.by_window.w1.cache_tokens, 13);
  assert.equal(claudeResult.by_window.w1.model, "claude-sonnet-4-5", "missing Session model cannot erase measured provider model");
  assert.equal(claudeResult.by_window.w1.branch, "pm/resident");
  assert.equal(claudeResult.today.cache_tokens, 13);
  const incompleteClaude = selectWorksTelemetry({ ...params, sessions: [{ ...usage, provider: "claude_code" }] });
  assert.equal(incompleteClaude.by_window.w1.cache_tokens, null);
  assert.equal(incompleteClaude.today.cache_tokens, null);
  assert.equal(result.today.output_tokens, 0);
  assert.equal(result.today.cost_usd, null);
  assert.equal(result.today.landed_pr_count, null);
  assert.ok(result.availability, "Works must explain the availability of Today and Fleet");
  assert.equal(result.availability.today.state, "measured");
  assert.equal(result.availability.fleet.state, "measured");
  assert.equal(result.by_window.w1.verification, "Passed");
  assert.equal(result.by_window.w1.context_remaining_percent, null, "estimated context is never presented as reported");
  assert.equal(result.fleet_tokens_per_minute[29], 0);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [{ ...usage, telemetry: null }] }).today.input_tokens, null);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [usage, usage] }).today.input_tokens, 20);
  assert.equal(selectWorksTelemetry({ ...params, sessions: [{ ...usage, state: { kind: "no_data" }, telemetry: null }] }).by_window.w1.input_tokens, null);
  const noData = { ...usage, state: { kind: "no_data" }, telemetry: null };
  const absent = selectWorksTelemetry({ ...params, sessions: [noData] });
  assert.equal(absent.availability.today.state, "no_data");
  assert.equal(absent.availability.fleet.state, "no_data");
  const partial = selectWorksTelemetry({ ...params, sessions: [usage, { ...noData, provider_session_id: "history" }] });
  assert.equal(partial.today.input_tokens, null, "a partial subtotal must not become a Today total");
  assert.equal(partial.availability.today.state, "partial");
  const failed = selectWorksTelemetry({ ...params, sessions: [{ ...noData, state: { kind: "unavailable", reason: "Transcript cannot be read" } }] });
  assert.equal(failed.availability.today.state, "unavailable");
  assert.match(failed.availability.today.detail, /Transcript cannot be read/);
  assert.equal(failed.availability.fleet.state, "unavailable");
  const incomplete = selectWorksTelemetry({ ...params, sessions: [{ ...usage, telemetry: { ...usage.telemetry,
    quality: "partial", today_input_tokens: null, today_output_tokens: null,
    minute_buckets: usage.telemetry.minute_buckets.map(bucket => ({ ...bucket, tokens: null })) } }] });
  assert.equal(incomplete.availability.today.state, "partial");
  assert.equal(incomplete.availability.fleet.state, "partial");
  assert.ok(incomplete.fleet_tokens_per_minute.every(value => value === null));
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


test("switch requests retain their identity until matching project receipts and reject duplicates", async () => {
  const { createAgentSwitchModel } = await import("../agent-activity-model.js");
  assert.equal(typeof createAgentSwitchModel, "function");
  const { model, beginAgentSwitchRequest, applyAgentSwitchSnapshot, applyAgentSwitchError } = createAgentSwitchModel("project");
  const action = { kind: "request", operation_id: "operation-one", member_id: "m1", expected_roster_revision: 3, target_issue_number: 5002 };
  assert.equal(beginAgentSwitchRequest(action), true);
  assert.deepEqual(model.read().pending, action);
  assert.equal(beginAgentSwitchRequest(action), false);
  applyAgentSwitchError({ project_key: "project", operation_id: "older-operation", message: "Earlier request failed" });
  applyAgentSwitchError({ project_key: "project", message: "An earlier read failed" });
  assert.deepEqual(model.read().pending, action, "uncorrelated errors cannot acknowledge a switch");
  assert.equal(model.read().error, null);
  const intent = { operation_id: action.operation_id, member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "requested", reason: "Waiting for a safe checkpoint" };
  applyAgentSwitchSnapshot({ project_key: "other", intents: [intent] });
  applyAgentSwitchError({ project_key: "other", message: "No" });
  applyAgentSwitchSnapshot({ project_key: "project", intents: [{ ...intent, operation_id: "older-operation", stage: "completed" }] });
  assert.deepEqual(model.read().pending, action);
  assert.equal(model.read().intents.length, 0, "a delayed unrelated receipt cannot become this switch");
  applyAgentSwitchSnapshot({ project_key: "project", intents: [intent] });
  assert.equal(model.read().pending, null);
  assert.deepEqual(model.read().intents, [intent]);
  assert.equal(beginAgentSwitchRequest({ ...action, operation_id: "another" }), false, "an active switch owns the member");
});

test("switch cancellation and disconnect preserve unknown outcomes without stale progress or success", async () => {
  const { createAgentSwitchModel } = await import("../agent-activity-model.js");
  assert.equal(typeof createAgentSwitchModel, "function");
  const { model, beginAgentSwitchRequest, applyAgentSwitchSnapshot, clearAgentSwitch } = createAgentSwitchModel("project");
  const intent = { operation_id: "operation-one", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "checkpoint_verified" };
  const receipt = value => applyAgentSwitchSnapshot({ project_key: "project", intents: [value] });
  receipt(intent);
  assert.equal(beginAgentSwitchRequest({ kind: "cancel", operation_id: intent.operation_id }), true);
  assert.equal(beginAgentSwitchRequest({ kind: "cancel", operation_id: intent.operation_id }), false);
  receipt({ ...intent, stage: "checkpointing" });
  assert.equal(model.read().intents[0].stage, "checkpoint_verified");
  assert.equal(model.read().pending.kind, "cancel", "an old progress event does not acknowledge cancellation");
  receipt({ ...intent, stage: "cancelled" });
  assert.equal(model.read().pending, null);
  const action = { kind: "request", operation_id: "operation-two", member_id: "m1", expected_roster_revision: 3, target_issue_number: 5003 };
  assert.equal(beginAgentSwitchRequest(action), true);
  clearAgentSwitch();
  assert.equal(model.read().disconnected, true);
  assert.deepEqual(model.read().pending, action, "response loss retains the request identity");
  receipt({ ...intent, stage: "completed" });
  assert.deepEqual(model.read().pending, action, "an older operation cannot confirm a new switch");
  receipt({ ...intent, operation_id: action.operation_id, target_issue_number: 5003, stage: "stopping_current" });
  assert.equal(model.read().disconnected, false);
  assert.equal(model.read().pending, null);
  assert.equal(beginAgentSwitchRequest({ kind: "cancel", operation_id: action.operation_id }), false, "stopping is past the cancel boundary");
});


test("switch refresh selects the latest timestamped operation without terminal regression", async () => {
  const { createAgentSwitchModel } = await import("../agent-activity-model.js");
  const { model, applyAgentSwitchSnapshot } = createAgentSwitchModel("project");
  const old = { operation_id: "old", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "completed", created_at: "2026-10-25T00:00:00Z" };
  const latest = { ...old, operation_id: "new", target_issue_number: 5003, stage: "failed", created_at: "2026-10-25T00:01:00Z" };
  applyAgentSwitchSnapshot({ project_key: "project", intents: [old] });
  applyAgentSwitchSnapshot({ project_key: "project", intents: [latest, old] });
  assert.deepEqual(model.read().intents, [latest], "refresh observes a later switch by the same member");
  applyAgentSwitchSnapshot({ project_key: "project", intents: [old, { ...latest, stage: "stopping_current" }] });
  assert.deepEqual(model.read().intents, [latest], "an old progress receipt cannot undo the final result");
});


test("only a correlated Unknown read may release an absent local switch for a new explicit request", async t => {
  const crypto = globalThis.crypto, descriptor = Object.getOwnPropertyDescriptor(crypto, "randomUUID");
  t.after(() => {
    if (descriptor) Object.defineProperty(crypto, "randomUUID", descriptor);
    else delete crypto.randomUUID;
  });
  Object.defineProperty(crypto, "randomUUID", { configurable: true, value: undefined });
  const { createAgentSwitchModel } = await import("../agent-activity-model.js");
  const { model, beginAgentSwitchRequest, applyAgentSwitchSnapshot, clearAgentSwitch } = createAgentSwitchModel("project");
  const action = { kind: "request", operation_id: "operation-one", member_id: "m1", expected_roster_revision: 3, target_issue_number: 5002 };
  beginAgentSwitchRequest(action);
  const earlierRead = { kind: "read" }; beginAgentSwitchRequest(earlierRead);
  assert.equal(typeof earlierRead.request_id, "string");
  assert.ok(earlierRead.request_id.length > 0, "read correlation does not require the secure-context UUID API");
  clearAgentSwitch();
  applyAgentSwitchSnapshot({ project_key: "project", request_id: earlierRead.request_id, intents: [] });
  assert.deepEqual(model.read().pending, action, "a read issued before Unknown is not recovery evidence");
  const recoveryRead = { kind: "read" }; beginAgentSwitchRequest(recoveryRead);
  assert.notEqual(recoveryRead.request_id, earlierRead.request_id);
  applyAgentSwitchSnapshot({ project_key: "project", intents: [] });
  applyAgentSwitchSnapshot({ project_key: "other", request_id: recoveryRead.request_id, intents: [] });
  assert.deepEqual(model.read().pending, action, "tick and other-project snapshots cannot release the request");
  applyAgentSwitchSnapshot({ project_key: "project", request_id: recoveryRead.request_id, intents: [] });
  assert.equal(model.read().pending, null);
  assert.equal(model.read().disconnected, false);
  assert.equal(model.read().notice.message, "No saved request was observed. Request again to recheck the current member.");
  assert.equal(model.read().intents.length, 0, "absence never fabricates a terminal result");
  const next = { ...action, operation_id: "operation-two" };
  assert.equal(beginAgentSwitchRequest(next), true, "only a fresh explicit click starts another operation");
  applyAgentSwitchSnapshot({ project_key: "project", request_id: recoveryRead.request_id, intents: [] });
  assert.deepEqual(model.read().pending, next, "the old correlated read cannot release a newer request");
});


test("server Unknown keeps saved-source recovery against old progress and accepts an explicit recovery receipt", async () => {
  const { createAgentSwitchModel } = await import("../agent-activity-model.js");
  const { model, applyAgentSwitchSnapshot } = createAgentSwitchModel("project");
  const unknown = { operation_id: "operation", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002,
    stage: "unknown", saved_work_id: "saved-source", can_resume_saved: true };
  const receipt = intent => applyAgentSwitchSnapshot({ project_key: "project", intents: [intent] });
  receipt(unknown);
  receipt({ ...unknown, stage: "stopping_current", saved_work_id: null, can_resume_saved: false });
  assert.deepEqual(model.read().intents, [unknown], "a delayed read cannot hide canonical Unknown recovery");
  receipt({ ...unknown, stage: "cancelled", can_resume_saved: false });
  assert.equal(model.read().intents[0].stage, "cancelled", "the explicit saved-source recovery receipt remains observable");
});

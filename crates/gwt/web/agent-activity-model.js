import { createUiStateStore } from "./ui-state-store.js";
import { createLaunchOperationId } from "./launch-pending-controller.js";

const unknownActivity = () => ({ snapshot: null, agents: [], byIssue: {}, count: null, issueCount: 0, pmCount: 0 });

export function createAgentScheduleModel(projectKey) {
  const empty = () => ({ snapshot: null, error: null, pending: false });
  const model = createUiStateStore(empty());
  let pendingMutationRevision = null;
  function applyAgentScheduleSnapshot(snapshot) {
    if (!snapshot || snapshot.project_key !== projectKey || !Array.isArray(snapshot.members) ||
        !Number.isSafeInteger(snapshot.roster_revision) || !Number.isSafeInteger(snapshot.schedule_revision) ||
        typeof snapshot.queue_revision !== "string") return;
    model.update(state => {
      if (state.snapshot && (snapshot.roster_revision < state.snapshot.roster_revision ||
          snapshot.schedule_revision < state.snapshot.schedule_revision)) return state;
      if (pendingMutationRevision !== null && snapshot.schedule_revision <= pendingMutationRevision) return state;
      pendingMutationRevision = null;
      if (state.snapshot && JSON.stringify(snapshot) === JSON.stringify(state.snapshot)) snapshot = state.snapshot;
      return { snapshot, error: null, pending: false };
    });
  }
  function applyAgentScheduleError(event) {
    if (event?.project_key === projectKey) {
      pendingMutationRevision = null;
      model.update(state => ({ ...state, error: event.message, pending: false }));
    }
  }
  function beginAgentScheduleRequest(action) {
    const state = model.read(), snapshot = state.snapshot;
    if (action.kind !== "read" && (state.pending || state.error || !snapshot ||
        action.expected_roster_revision !== snapshot.roster_revision ||
        action.expected_queue_revision !== snapshot.queue_revision ||
        action.expected_schedule_revision !== snapshot.schedule_revision)) return false;
    pendingMutationRevision = action.kind === "read" ? null : snapshot.schedule_revision;
    model.update(current => ({ ...current, pending: true }));
    return true;
  }
  return { model, applyAgentScheduleSnapshot, applyAgentScheduleError, beginAgentScheduleRequest,
    clearAgentSchedule: () => { pendingMutationRevision = null; model.update(empty); } };
}

const switchStages = ["requested", "checkpointing", "checkpoint_verified", "stopping_current", "launching_next", "completed", "cancelled", "failed", "unknown"];
const switchTerminal = stage => ["completed", "cancelled", "failed"].includes(stage);

export function createAgentSwitchModel(projectKey) {
  const model = createUiStateStore({ snapshot: null, intents: [], pending: null, error: null, disconnected: false, notice: null });
  const currentOperations = new Map();
  let pendingRead = null;
  function beginAgentSwitchRequest(action) {
    const state = model.read();
    if (action?.kind === "read") {
      action.request_id = createLaunchOperationId("switch-read");
      pendingRead = { request_id: action.request_id, operation_id: state.pending?.operation_id, disconnected: state.disconnected };
      return true;
    }
    if (state.pending || state.disconnected) return false;
    if (action?.kind === "request") {
      if (!action.operation_id || !action.member_id || !Number.isSafeInteger(action.expected_roster_revision) ||
          !Number.isSafeInteger(action.target_issue_number) || action.target_issue_number < 1 ||
          state.intents.some(intent => intent.member_id === action.member_id && !switchTerminal(intent.stage))) return false;
      currentOperations.set(action.member_id, action.operation_id);
    } else if (action?.kind === "cancel") {
      const intent = state.intents.find(intent => intent.operation_id === action.operation_id);
      if (!intent || !["requested", "checkpointing", "checkpoint_verified"].includes(intent.stage)) return false;
    } else return false;
    pendingRead = null;
    model.update(current => ({ ...current, pending: action, error: null, notice: null }));
    return true;
  }
  function applyAgentSwitchSnapshot(snapshot) {
    if (!snapshot || snapshot.project_key !== projectKey || !Array.isArray(snapshot.intents)) return;
    model.update(state => {
      // Only a read issued for this Unknown operation can establish absence.
      const matchingRead = pendingRead?.request_id === snapshot.request_id && !!snapshot.request_id;
      const absent = matchingRead && pendingRead.disconnected && state.disconnected && state.pending &&
        pendingRead.operation_id === state.pending.operation_id && !snapshot.intents.some(intent => intent.operation_id === state.pending.operation_id);
      if (matchingRead) pendingRead = null;
      const absentMember = absent ? state.pending.member_id ?? state.intents.find(intent => intent.operation_id === state.pending.operation_id)?.member_id : null;
      if (absentMember && currentOperations.get(absentMember) === state.pending.operation_id) currentOperations.delete(absentMember);
      // A request is acknowledged only by its own operation, never an older read.
      const byMember = {};
      for (const intent of snapshot.intents) {
        if (!intent.operation_id || !intent.member_id || !switchStages.includes(intent.stage)) continue;
        const expected = currentOperations.get(intent.member_id);
        if (expected && expected !== intent.operation_id) {
          const previous = state.intents.find(value => value.member_id === intent.member_id);
          if (state.pending?.operation_id === expected || !previous || !switchTerminal(previous.stage) ||
              !(Date.parse(intent.created_at) > Date.parse(previous.created_at))) continue;
        }
        const candidate = byMember[intent.member_id];
        if (!candidate || Date.parse(intent.created_at) > Date.parse(candidate.created_at) ||
            (!candidate.created_at && switchTerminal(candidate.stage) && !switchTerminal(intent.stage))) byMember[intent.member_id] = intent;
      }
      const intents = state.intents.filter(intent => !absent || intent.operation_id !== state.pending.operation_id);
      for (const intent of Object.values(byMember)) {
        const index = intents.findIndex(current => current.member_id === intent.member_id), previous = intents[index];
        if (previous?.operation_id === intent.operation_id && (switchTerminal(previous.stage) ||
            previous.stage === "unknown" && intent.stage !== "unknown" && !switchTerminal(intent.stage) ||
            previous.stage !== "unknown" && intent.stage !== "unknown" && switchStages.indexOf(intent.stage) < switchStages.indexOf(previous.stage))) continue;
        currentOperations.set(intent.member_id, intent.operation_id);
        if (index < 0) intents.push(intent); else intents[index] = intent;
      }
      const receipt = intents.find(intent => intent.operation_id === state.pending?.operation_id);
      const acknowledged = receipt && (state.pending.kind === "request" || switchTerminal(receipt.stage) || ["stopping_current", "launching_next"].includes(receipt.stage));
      return { snapshot, intents, pending: acknowledged || absent ? null : state.pending, error: null,
        disconnected: !absent && state.pending && !acknowledged ? state.disconnected : false,
        notice: absent ? { member_id: absentMember, message: "No saved request was observed. Request again to recheck the current member." } : state.notice };
    });
  }
  function applyAgentSwitchError(event) {
    if (event?.project_key !== projectKey) return;
    model.update(state => {
      if (state.pending && event.operation_id !== state.pending.operation_id ||
          !state.pending && event.operation_id && !state.intents.some(intent => intent.operation_id === event.operation_id)) return state;
      return { ...state, pending: null, error: event.message };
    });
  }
  function clearAgentSwitch() {
    model.update(state => ({ ...state, disconnected: true, error: null }));
  }
  return { model, beginAgentSwitchRequest, applyAgentSwitchSnapshot, applyAgentSwitchError, clearAgentSwitch };
}

export function createAgentRosterModel(projectKey) {
  let latestSnapshot = null, memberUsage = {};
  const model = createUiStateStore({ members: [], revision: null, capacity: null,
    recommended_capacity: null, past_contributors: [], error: null });
  function applyAgentRosterSnapshot(snapshot) {
    if (!snapshot || snapshot.project_key !== projectKey || !Array.isArray(snapshot.members) ||
        !Number.isSafeInteger(snapshot.revision)) return;
    model.update(state => {
      if (state.revision !== null && snapshot.revision < state.revision) return state;
      latestSnapshot = snapshot;
      return { ...snapshot, members: snapshot.members.map(member => ({ ...member,
        member_id: member.agent_member_id, tokens: member.tokens ?? memberUsage[member.agent_member_id] ?? null })) };
    });
  }
  function applyAgentRosterError(event) {
    if (event?.project_key === projectKey) {
      if (latestSnapshot) latestSnapshot = { ...latestSnapshot, error: event.message };
      model.update(state => ({ ...state, error: event.message }));
    }
  }
  function applyAgentRosterUsage(sessions, projectRoot) {
    memberUsage = selectWorksTelemetry({ sessions, projectRoot }).by_member;
    if (latestSnapshot) applyAgentRosterSnapshot(latestSnapshot);
  }
  return { model, applyAgentRosterSnapshot, applyAgentRosterError, applyAgentRosterUsage };
}

// Inputs are session-attributed measurements, never provider account totals.
export function selectWorksTelemetry({ sessions = [], projectRoot, agents = [], works = [], bySession = {} }) {
  const seen = new Set();
  const projectSessions = sessions.filter(session => {
    if (!projectRoot || session.project_path !== projectRoot) return false;
    const key = `${session.provider}:${session.provider_session_id ?? session.session_id}`;
    if (seen.has(key)) return false;
    seen.add(key); return true;
  });
  const sumKnown = values => values.length && values.every(value => typeof value === "number" && Number.isFinite(value))
    ? values.reduce((sum, value) => sum + value, 0) : null;
  const today = key => sumKnown(projectSessions.map(session => session.telemetry?.[key]));
  const cache = (provider, read, creation) => provider === "codex" ? read ?? null : sumKnown([read, creation]);
  const by_member = {};
  for (const session of projectSessions) {
    const id = session.agent_member_id;
    if (!id) continue;
    const input = session.provider === "codex" ? session.input_tokens
      : sumKnown([session.input_tokens, session.cache_read_tokens, session.cache_creation_tokens]);
    const tokens = session.state?.kind === "ok" ? sumKnown([input, session.output_tokens]) : null;
    by_member[id] = Object.hasOwn(by_member, id) ? sumKnown([by_member[id], tokens]) : tokens;
  }
  const by_window = {};
  for (const agent of agents) {
    const usage = projectSessions.find(session => session.session_id === agent.session_id && session.active !== false && session.state?.kind === "ok");
    const ownsSession = work => work.agents?.some(member => member.session_id === agent.session_id);
    const work = works.find(work => ownsSession(work) || work.works?.some(ownsSession));
    const current = work?.works?.find(ownsSession) ?? work;
    const recorded = current?.agents?.find(member => member.session_id === agent.session_id);
    const diagnosis = current?.execution_diagnosis;
    const verification = { passed: "Passed", failed: "Failed", missing: "Not run", running: "Running",
      waiting: "Waiting", admission_waiting: "Waiting", stale_fingerprint: "Stale evidence", wrong_session: "Unavailable" }[diagnosis?.verification_state] ?? null;
    by_window[agent.window_id] = {
      input_tokens: usage?.input_tokens ?? null,
      cache_tokens: usage ? cache(usage.provider, usage.cache_read_tokens, usage.cache_creation_tokens) : null,
      output_tokens: usage?.output_tokens ?? null, context_remaining_percent: usage?.reported_context_left_pct ?? null,
      tokens_per_minute: usage?.telemetry?.minute_buckets?.map(bucket => bucket.tokens) ?? null,
      branch: recorded?.branch ?? work?.branch ?? null, ahead_of_develop: null,
      verification,
      last_action: recorded?.last_board_entry_kind && recorded?.updated_at
        ? `${recorded.last_board_entry_kind} · ${recorded.updated_at}` : null,
      elapsed_seconds: null, warning_count: null,
      ...bySession[agent.session_id],
      model: usage?.model ?? bySession[agent.session_id]?.model ?? null,
    };
  }
  // Each bucket needs evidence for every live participant at the same timestamp.
  const series = agents.map(agent => projectSessions.find(session => session.session_id === agent.session_id && session.active !== false)?.telemetry?.minute_buckets);
  const fleet = Array.from({ length: 30 }, (_, index) => {
    const starts = series.map(buckets => buckets?.[index]?.started_at);
    return starts.length && starts[0] && starts.every(start => start === starts[0])
      ? sumKnown(series.map(buckets => buckets?.[index]?.tokens)) : null;
  });
  return { today: { input_tokens: today("today_input_tokens"), output_tokens: today("today_output_tokens"),
    cache_tokens: sumKnown(projectSessions.map(session => cache(session.provider,
      session.telemetry?.today_cache_read_tokens, session.telemetry?.today_cache_creation_tokens))),
    cost_usd: null, landed_pr_count: null }, fleet_tokens_per_minute: fleet, by_window, by_member };
}

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

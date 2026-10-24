import { createUiStateStore } from "./ui-state-store.js";

const unknownActivity = () => ({ snapshot: null, agents: [], byIssue: {}, count: null, issueCount: 0, pmCount: 0 });

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
      model: usage?.model ?? null, elapsed_seconds: null, warning_count: null,
      ...bySession[agent.session_id],
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

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

function rosterRevision(snapshot) {
  if (snapshot?.revision_decimal !== undefined) {
    const value = snapshot.revision_decimal;
    if (typeof value !== "string" || !/^\d+$/.test(value) || BigInt(value) > 18446744073709551615n) return null;
    const exact = BigInt(value);
    return exact <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(exact) : exact.toString();
  }
  return Number.isSafeInteger(snapshot?.revision) && snapshot.revision >= 0 ? snapshot.revision : null;
}

export function createAgentRosterModel(projectKey, { onRename = () => {}, scheduleFrame } = {}) {
  let latestSnapshot = null, memberUsage = {}, latestRead = null;
  const model = createUiStateStore({ members: [], revision: null, capacity: null,
    recommended_capacity: null, past_contributors: [], error: null, name_edits: {}, pending_name: null, pending_roster: null }, { scheduleFrame });
  function nameError(state, memberId, value) {
    const name = value.trim();
    if (!name) return "Enter a display name.";
    if (state.members.some(member => member.member_id !== memberId && member.name.trim().toLowerCase() === name.toLowerCase())) {
      return "This name is already registered, including resting members.";
    }
    return "";
  }
  function updateNameEdit(memberId, reduce) {
    model.update(state => ({ ...state, name_edits: { ...state.name_edits, [memberId]: reduce(state.name_edits[memberId]) } }));
  }
  function sendNextName() {
    const state = model.read();
    if (state.pending_name || state.revision === null) return;
    const next = Object.entries(state.name_edits).find(([, edit]) => edit.status === "queued" && !edit.composing && !edit.blocked);
    if (!next) return;
    const [memberId, edit] = next, member = state.members.find(member => member.member_id === memberId);
    const error = member ? nameError(state, memberId, edit.value) : "This member is no longer available.";
    if (error) {
      updateNameEdit(memberId, current => ({ ...current, error, status: "invalid" })); sendNextName(); return;
    }
    // Normalize the saved name without changing the draft or its typing position.
    const name = edit.value.trim();
    if (name === member.name) {
      updateNameEdit(memberId, current => ({ ...current, error: "", status: "saved" })); sendNextName(); return;
    }
    const request = { kind: "agent_roster", project_key: projectKey, action: { kind: "rename", member_id: memberId, name },
      expected_revision: state.revision, request_id: createLaunchOperationId("member-name") };
    model.update(current => ({ ...current, pending_name: request,
      name_edits: { ...current.name_edits, [memberId]: { ...current.name_edits[memberId], status: "saving", error: "" } } }));
    try { onRename(request); } catch (_error) {
      applyAgentRosterError({ project_key: projectKey, request_id: request.request_id, message: "Name could not be sent. Retry when connected." });
    }
  }
  function editAgentMemberName(memberId, value, { composing = false } = {}) {
    const state = model.read();
    const member = state.members.find(member => member.member_id === memberId);
    if (typeof value !== "string" || !member) return;
    const error = composing ? "" : nameError(state, memberId, value);
    if (!composing && !error && value.trim() === member.name && state.pending_name?.action.member_id !== memberId) {
      updateNameEdit(memberId, () => ({ value, composing: false, blocked: false, error: "", status: "saved" }));
      sendNextName(); return;
    }
    updateNameEdit(memberId, previous => ({ value, composing, blocked: previous?.blocked || false,
      error: error || (previous?.blocked ? previous.error : ""),
      status: composing ? "editing" : error ? "invalid" : previous?.blocked ? "error" : "queued" }));
    sendNextName();
  }
  function retryAgentMemberName(memberId) {
    const state = model.read(), edit = state.name_edits[memberId];
    if (!edit || edit.composing || !edit.blocked || nameError(state, memberId, edit.value)) return;
    updateNameEdit(memberId, current => ({ ...current, blocked: false, error: "", status: "queued" }));
    sendNextName();
  }
  function settleName(requestId, error = "") {
    const state = model.read(), pending = state.pending_name;
    if (!requestId || pending?.request_id !== requestId) return;
    const memberId = pending.action.member_id, edit = state.name_edits[memberId];
    const validationError = edit.composing ? "" : nameError(state, memberId, edit.value);
    model.update(current => ({ ...current, pending_name: null,
      name_edits: { ...current.name_edits, [memberId]: error
        ? { ...edit, blocked: true, error, status: "error" }
        : !edit.composing && edit.value.trim() === pending.action.name
          ? { ...edit, blocked: false, error: "", status: "saved" }
          : { ...edit, error: validationError, status: edit.composing ? "editing" : validationError ? "invalid" : "queued" } } }));
  }
  function beginAgentRosterRequest(request) {
    if (request?.project_key !== projectKey || !request.request_id) return false;
    if (request.action?.kind === "read") {
      latestRead = { request_id: request.request_id, error: model.read().error }; return true;
    }
    if (!["add", "rest", "resize", "set_pm_eligibility", "select_pm"].includes(request.action?.kind)) return false;
    latestRead = null;
    if (latestSnapshot) latestSnapshot = { ...latestSnapshot, error: null };
    model.update(state => ({ ...state, error: null, pending_roster: { request_id: request.request_id, action: request.action } }));
    return true;
  }

  function applyAgentRosterSnapshot(snapshot, requestId) {
    const revision = rosterRevision(snapshot);
    if (!snapshot || snapshot.project_key !== projectKey || !Array.isArray(snapshot.members) ||
        revision === null) return false;
    if (model.read().revision !== null && BigInt(revision) < BigInt(model.read().revision)) {
      settleName(requestId, snapshot.error || "This name changed elsewhere. Review your draft and retry.");
      if (requestId && model.read().pending_roster?.request_id === requestId) applyAgentRosterError({
        project_key: projectKey, request_id: requestId, message: snapshot.error || "Members changed while this request was pending. Reload the current roster and retry." });
      if (latestRead?.request_id === requestId) latestRead = null;
      return false;
    }
    const readReceipt = requestId && latestRead?.request_id === requestId;
    const recoverRead = readReceipt && latestRead.error === model.read().error;
    if (readReceipt) latestRead = null;
    model.update(state => {
      if (state.revision !== null && BigInt(revision) < BigInt(state.revision)) return state;
      // Correlated rename errors belong to their field. Replaying an old
      // receipt must not replace another operation's banner or revive one.
      const ordinaryReceipt = requestId && state.pending_roster?.request_id === requestId;
      latestSnapshot = { ...snapshot, revision, error: !requestId || ordinaryReceipt || recoverRead ? snapshot.error ?? null : state.error };
      const nameEdits = { ...state.name_edits };
      for (const member of snapshot.members) {
        const edit = nameEdits[member.agent_member_id];
        if (edit?.status === "saved" && edit.value.trim() !== member.name) nameEdits[member.agent_member_id] = { ...edit, value: member.name };
        if (edit?.blocked && !edit.composing && edit.value.trim() === member.name && !snapshot.error) {
          nameEdits[member.agent_member_id] = { ...edit, blocked: false, error: "", status: "saved" };
        }
      }
      return { ...latestSnapshot, name_edits: nameEdits, pending_name: state.pending_name, pending_roster: ordinaryReceipt ? null : state.pending_roster, members: snapshot.members.map(member => ({ ...member,
        member_id: member.agent_member_id, tokens: member.tokens ?? memberUsage[member.agent_member_id] ?? null })) };
    });
    const state = model.read(), pending = state.pending_name;
    if (requestId && pending?.request_id === requestId) {
      const member = state.members.find(member => member.member_id === pending.action.member_id);
      settleName(requestId, member?.name === pending.action.name ? ""
        : snapshot.error || "This name changed elsewhere. Review your draft and retry.");
    }
    return true;
  }
  function applyAgentRosterError(event) {
    if (event?.project_key === projectKey) {
      if (event.request_id) {
        if (model.read().pending_name?.request_id === event.request_id) { settleName(event.request_id, event.message || "Name could not be saved. Retry."); return; }
        if (model.read().pending_roster?.request_id === event.request_id) model.update(state => ({ ...state, pending_roster: null }));
        else if (latestRead?.request_id === event.request_id) {
          const current = latestRead.error === model.read().error; latestRead = null;
          if (!current) return;
        } else return;
      }
      if (latestSnapshot) latestSnapshot = { ...latestSnapshot, error: event.message };
      model.update(state => ({ ...state, error: event.message }));
    }
  }
  function clearAgentRosterRename() {
    latestRead = null;
    const pending = model.read().pending_name;
    if (pending) settleName(pending.request_id, "Connection lost. Review your draft and retry.");
    const ordinary = model.read().pending_roster;
    if (ordinary) applyAgentRosterError({ project_key: projectKey, request_id: ordinary.request_id,
      message: "Connection lost. Reload members, review the current state and retry." });
  }
  function applyAgentRosterUsage(sessions, projectRoot) {
    memberUsage = selectWorksTelemetry({ sessions, projectRoot }).by_member;
    if (latestSnapshot) applyAgentRosterSnapshot(latestSnapshot);
  }
  // Receipts only commit data. Continue queued transport on the store's
  // existing frame boundary, independently of whichever view is mounted.
  model.subscribeFrame(state => state.pending_name ? null
    : Object.values(state.name_edits).find(edit => edit.status === "queued"), sendNextName);
  return { model, applyAgentRosterSnapshot, applyAgentRosterError, applyAgentRosterUsage,
    editAgentMemberName, retryAgentMemberName, clearAgentRosterRename, beginAgentRosterRequest };
}

// A target keeps its own draft and in-flight CAS. Both roles use this state;
// their adapters select the existing canonical writer and revision token.
export function createMemberTuningModel(projectKey, { onSave = () => {}, scheduleFrame } = {}) {
  const model = createUiStateStore({ targets: {} }, { scheduleFrame });
  const keyFor = (role, memberId) => `${role}:${memberId}`;
  let pmMemberId;
  const values = (profile, role = profile.role) => {
    const agent_id = profile.agent_id?.trim() || "";
    let reasoning = profile.reasoning?.trim() || null;
    if (role === "pm" && agent_id === "grok" && /^[aA][uU][tT][oO]$/.test(reasoning || "")) reasoning = null;
    return { agent_id, model: profile.model?.trim() || null, reasoning };
  };
  const equal = (left, right, role) => JSON.stringify(values(left, role)) === JSON.stringify(values(right, role));
  function setTarget(key, next) { model.update(state => ({ targets: { ...state.targets, [key]: next } })); }

  function sendNext() {
    for (const [key, target] of Object.entries(model.read().targets)) {
      if (target.available === false || target.status !== "queued" || target.pending || target.composing || target.blocked) continue;
      const canonical = target.canonical, profile = values(target.draft, canonical.role);
      if (canonical.role === "pm" && pmMemberId !== undefined && pmMemberId !== canonical.member_id) continue;
      if (!canonical?.revision || !profile.agent_id) continue;
      const requestId = createLaunchOperationId("member-tuning");
      const request = canonical.role === "pm"
        ? { kind: "set_pm_launch_profile", project_key: projectKey, member_id: canonical.member_id,
          request_id: requestId, expected_profile_revision: canonical.revision, ...profile }
        : { kind: "agent_roster", project_key: projectKey, request_id: requestId,
          action: { kind: "set_tuning", member_id: canonical.member_id, model: profile.model,
            reasoning: profile.reasoning, expected_core_revision: canonical.revision } };
      setTarget(key, { ...target, status: "saving", error: "", field: null,
        pending: { request_id: requestId, profile, baseRevision: canonical.revision } });
      try { onSave(request); } catch (_error) {
        applyTuningError({ project_key: projectKey, role: canonical.role, member_id: canonical.member_id,
          request_id: requestId, message: "Profile could not be sent. Retry when connected." });
      }
    }
  }

  function applyTuningSnapshot(snapshot, requestId) {
    if (snapshot?.project_key !== projectKey || !["implementation", "pm"].includes(snapshot.role)
      || snapshot.role === "pm" && pmMemberId !== undefined && pmMemberId !== snapshot.member_id
      || !snapshot.member_id || typeof snapshot.revision !== "string" || !snapshot.revision || !snapshot.agent_id) return;
    const key = keyFor(snapshot.role, snapshot.member_id);
    const previous = model.read().targets[key];
    // Correlated old receipts cannot update a target after its request ended.
    if (requestId && previous?.pending?.request_id !== requestId) return;
    function nextSnapshot(current) {
      if (!current) return { canonical: snapshot, available: true, draft: values(snapshot), status: "saved", error: "",
        field: null, blocked: false, composing: false, pending: null };
      if (requestId && current.pending && current.canonical.revision !== current.pending.baseRevision && current.canonical.revision !== snapshot.revision) {
        return { ...current, pending: null, status: "error", blocked: true, error: "This profile changed elsewhere. Reload the current profile, review your draft and retry." };
      }
      const next = { ...current, available: snapshot.role === "pm" ? current.available : true, canonical: { ...current.canonical, ...snapshot } };
      if (requestId && current.pending) {
        next.pending = null;
        if (!equal(snapshot, current.pending.profile, snapshot.role)) {
          return { ...next, status: "error", blocked: true,
            error: "This profile changed elsewhere. Review your draft and retry." };
        }
        if (current.blocked) return { ...next, status: "error", blocked: true, error: current.error };
        next.status = current.composing ? "editing" : equal(current.draft, current.pending.profile, snapshot.role) ? "saved" : "queued";
        next.blocked = false; next.error = ""; next.field = null;
      } else if (current.status === "saved" && !equal(snapshot, current.canonical, snapshot.role)) next.draft = values(snapshot);
      return next;
    }
    setTarget(key, nextSnapshot(previous));
  }

  function editTuning(role, memberId, patch, { composing = false } = {}) {
    const key = keyFor(role, memberId), target = model.read().targets[key];
    if (!target || target.available === false || !patch || Object.keys(patch).some(field => !["agent_id", "model", "reasoning"].includes(field)
      || patch[field] !== null && typeof patch[field] !== "string")) return;
    if (role === "implementation" && patch.agent_id !== undefined && patch.agent_id !== target.canonical.agent_id) return;
    setTarget(key, { ...target, draft: { ...target.draft, ...patch }, composing,
      status: composing ? "editing" : target.blocked ? "error" : target.pending ? "saving" : "queued" });
    sendNext();
  }

  function applyTuningError(event) {
    if (event?.project_key !== projectKey) return;
    const key = keyFor(event.role, event.member_id), target = model.read().targets[key];
    if (!event.request_id || target?.pending?.request_id !== event.request_id) return;
    setTarget(key, { ...target, pending: null, status: "error", blocked: true,
      error: event.message || "Profile could not be saved. Review your draft and retry.", field: event.field || null });
  }

  function retryTuning(role, memberId) {
    const key = keyFor(role, memberId), target = model.read().targets[key];
    if (!target?.blocked || target.available === false || target.pending || target.composing) return;
    setTarget(key, { ...target, status: "queued", blocked: false, error: "", field: null });
    sendNext();
  }

  function setTuningAvailable(role, memberId, available) {
    const key = keyFor(role, memberId);
    const target = model.read().targets[key];
    if (target) setTarget(key, { ...target, available, ...(role === "pm" && !available
      ? { blocked: true, status: target.pending ? target.status : "error", error: target.error || "PM profile unavailable. Reload the current profile, review your draft and retry." } : {}) });
  }

  function setPmProfileAvailable(available) {
    if (pmMemberId) setTuningAvailable("pm", pmMemberId, available);
  }

  function applyPmTuningStatus(status) {
    if (typeof status?.project_key === "string" && status.project_key && status.project_key !== projectKey) return;
    if (status?.project_key !== projectKey || status.available !== true
      || typeof status.configured_member_id !== "string" || !status.configured_member_id.trim()
      || typeof status.profile_revision !== "string" || !status.profile_revision.trim()
      || typeof status.configured_agent_id !== "string" || !status.configured_agent_id.trim()) {
      for (const target of Object.values(model.read().targets)) {
        if (target.canonical.role === "pm") setTuningAvailable("pm", target.canonical.member_id, false);
      }
      return;
    }
    setPmMember(status.configured_member_id);
    applyTuningSnapshot({ project_key: projectKey, role: "pm", member_id: status.configured_member_id,
      revision: status.profile_revision, agent_options: status.agent_options, agent_id: status.configured_agent_id,
      model: status.configured_model, reasoning: status.configured_reasoning });
    setTuningAvailable("pm", status.configured_member_id, true);
  }

  function applyRosterTuningSnapshot(roster, requestId, { accepted = null } = {}) {
    if (roster?.project_key !== projectKey) return;
    const pending = requestId && Object.values(model.read().targets).find(target => target.canonical.role === "implementation" && target.pending?.request_id === requestId);
    if (accepted === false) {
      if (pending) applyTuningError({ project_key: projectKey, role: "implementation", member_id: pending.canonical.member_id,
        request_id: requestId, message: "This profile changed elsewhere. Reload the current profile, review your draft and retry." });
      return;
    }
    if (requestId && !pending && accepted !== true) return;
    for (const member of roster.members || []) {
      const memberId = member.agent_member_id || member.member_id;
      const core = member.saved_implementation_core, revision = member.saved_implementation_core_revision;
      const acknowledgement = pending?.canonical.member_id === memberId ? requestId : undefined;
      if (core && revision) applyTuningSnapshot({ project_key: projectKey, role: "implementation", member_id: memberId,
        agent_id: core.agent_id, model: core.model, reasoning: core.reasoning, revision }, roster.error ? undefined : acknowledgement);
      if (!core || !revision) {
        setTuningAvailable("implementation", memberId, false);
        if (acknowledgement) applyTuningError({ project_key: projectKey, role: "implementation", member_id: memberId,
        request_id: requestId, message: roster.error || "Saved implementation settings are unavailable. Reload registered members and retry." });
      }
      if (acknowledgement && roster.error) applyTuningError({ project_key: projectKey, role: "implementation", member_id: memberId, request_id: requestId, message: roster.error });
    }
  }

  function applyRosterTuningError(event) {
    if (event?.project_key !== projectKey || !event.request_id) return;
    const pending = Object.values(model.read().targets).find(target => target.canonical.role === "implementation" && target.pending?.request_id === event.request_id);
    if (pending) applyTuningError({ ...event, role: "implementation", member_id: pending.canonical.member_id });
  }

  function applyPmTuningResult(event) {
    if (event?.project_key !== projectKey || !event.member_id || !event.request_id) return;
    const target = model.read().targets[keyFor("pm", event.member_id)];
    if (target?.pending?.request_id !== event.request_id || pmMemberId !== undefined && event.member_id !== pmMemberId) return;
    const snapshot = event.profile && typeof event.profile_revision === "string" && event.profile_revision
      ? { project_key: projectKey, role: "pm", member_id: event.member_id, revision: event.profile_revision,
        agent_id: event.profile.agent_id, model: event.profile.model, reasoning: event.profile.reasoning } : null;
    if (event.success && snapshot) applyTuningSnapshot(snapshot, event.request_id);
    else {
      if (snapshot && (target.canonical.revision === target.pending.baseRevision || target.canonical.revision === snapshot.revision)) applyTuningSnapshot(snapshot);
      applyTuningError({ ...event, role: "pm", message: event.message || "Profile result is unavailable. Reload and retry." });
    }
  }

  function setPmMember(memberId) {
    pmMemberId = memberId || null;
    for (const target of Object.values(model.read().targets)) {
      if (target.canonical.role === "pm" && target.canonical.member_id !== pmMemberId) {
        setTuningAvailable("pm", target.canonical.member_id, false);
        if (target.pending) applyTuningError({ project_key: projectKey, role: "pm", member_id: target.canonical.member_id,
          request_id: target.pending.request_id, message: "The configured Project Manager changed. Reload and review this draft." });
      }
    }
  }

  function disconnectTuning() {
    for (const target of Object.values(model.read().targets)) {
      if (target.pending) applyTuningError({ project_key: projectKey, role: target.canonical.role,
        member_id: target.canonical.member_id, request_id: target.pending.request_id,
        message: "Connection lost. Reload the current profile, then review your draft and retry." });
    }
  }

  model.subscribeFrame(state => Object.values(state.targets).some(target => target.status === "queued"), sendNext);
  return { model, applyTuningSnapshot, editTuning, applyTuningError, retryTuning, disconnectTuning, setPmMember, setTuningAvailable, setPmProfileAvailable, applyPmTuningStatus, applyRosterTuningSnapshot, applyRosterTuningError, applyPmTuningResult };
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
  const availability = (rows, complete, scope) => {
    if (complete) return { state: "measured", detail: `Measured: ${scope} measurements are complete.` };
    const failed = rows.find(row => ["unavailable", "stale", "disabled"].includes(row?.state?.kind));
    if (failed) return { state: "unavailable", detail: `Unavailable: ${failed.state.reason ||
      (failed.state.kind === "stale" ? "session measurements are stale" : failed.state.kind === "disabled"
        ? "session usage collection is disabled" : "session usage could not be read")}.` };
    return rows.some(row => row?.telemetry)
      ? { state: "partial", detail: `Partial: ${scope} measurements are incomplete.` }
      : { state: "no_data", detail: `No data: no ${scope} measurements.` };
  };
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
  const liveSessions = agents.map(agent => projectSessions.find(session => session.session_id === agent.session_id && session.active !== false));
  const series = liveSessions.map(session => session?.telemetry?.minute_buckets);
  const fleet = Array.from({ length: 30 }, (_, index) => {
    const starts = series.map(buckets => buckets?.[index]?.started_at);
    return starts.length && starts[0] && starts.every(start => start === starts[0])
      ? sumKnown(series.map(buckets => buckets?.[index]?.tokens)) : null;
  });
  const todayTotals = { input_tokens: today("today_input_tokens"), output_tokens: today("today_output_tokens"),
    cache_tokens: sumKnown(projectSessions.map(session => cache(session.provider,
      session.telemetry?.today_cache_read_tokens, session.telemetry?.today_cache_creation_tokens))),
    cost_usd: null, landed_pr_count: null };
  return { today: todayTotals, fleet_tokens_per_minute: fleet, by_window, by_member,
    availability: {
      today: availability(projectSessions, [todayTotals.input_tokens, todayTotals.cache_tokens, todayTotals.output_tokens]
        .every(value => typeof value === "number" && Number.isFinite(value)), "project session token"),
      fleet: availability(liveSessions, fleet.every(value => typeof value === "number" && Number.isFinite(value)), "live agent minute"),
    } };
}

// Display-only membership. Window status, Work history and Issues reservations
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

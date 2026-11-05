import { createUiStateStore } from './ui-state-store.js';

// LAN HTTP origins expose getRandomValues even when randomUUID is unavailable.
export function createProjectOperationId() {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID();
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
  const hex = Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function createProjectExecutionPolicyModel(projectKey) {
  const model = createUiStateStore({ snapshot: null, lastKnown: null, disconnected: false });
  let generationFence = null, reconnectGeneration = null;
  function applyStatus(snapshot) {
    if (snapshot?.project_key !== projectKey || !Number.isSafeInteger(snapshot.generation)
      || !Number.isSafeInteger(snapshot.revision)) return;
    model.update(state => {
      const previous = state.snapshot;
      if (generationFence !== null && snapshot.generation !== generationFence) return state;
      const reconnect = snapshot.generation === reconnectGeneration;
      if (previous && !reconnect && snapshot.generation < previous.generation) return state;
      const sameGeneration = [previous, state.lastKnown].filter(value => value?.generation === snapshot.generation);
      if (!reconnect && sameGeneration.some(value => Date.parse(snapshot.observed_at) < Date.parse(value.observed_at) ||
        !snapshot.degraded && (snapshot.revision < value.revision ||
          Number.isSafeInteger(snapshot.view_set_revision) && Number.isSafeInteger(value.view_set_revision)
            && snapshot.view_set_revision < value.view_set_revision))) return state;
      reconnectGeneration = null;
      return { snapshot, lastKnown: !snapshot.degraded && snapshot.effective_state !== 'unknown' ? snapshot : state.lastKnown,
        disconnected: false };
    });
  }
  return { model, applyStatus,
    // Only the active authenticated view challenge (or first fresh Hub
    // catalog) can reset an incarnation. Disconnection grants no reset.
    bindGeneration(generation, { strict = true } = {}) {
      if (!Number.isSafeInteger(generation)) return;
      generationFence = strict ? generation : null;
      reconnectGeneration = model.read().snapshot?.generation !== generation ? generation : null;
    },
    connectionLost() { model.update(state => ({ ...state, disconnected: true })); } };
}

const labels = { running: 'Running', waiting: 'Waiting', stopped: 'Stopped', stopping: 'Stopping',
  selection_pending: 'Selection pending', unknown: 'Unknown' };

// Hub, Agents and the shared PM editor consume this same project-owned snapshot.
export function mountProjectExecutionStatus(container, model, { projectTitle = '', onSettings } = {}) {
  const doc = container.ownerDocument;
  const line = doc.createElement('p'); line.className = 'settings-status';
  line.dataset.projectExecutionStatus = ''; line.setAttribute('role', 'status');
  line.setAttribute('aria-live', 'polite'); container.append(line);
  const unsubscribe = model.subscribe(state => state, state => {
    const snapshot = state.snapshot;
    const title = typeof projectTitle === 'function' ? projectTitle() : projectTitle;
    const stateLabel = labels[snapshot?.effective_state] || 'Unknown';
    const unknown = state.disconnected || !snapshot || snapshot.degraded || snapshot.effective_state === 'unknown';
    const known = state.lastKnown;
    const current = unknown ? `Unknown${known ? ` · Last known: ${labels[known.effective_state] || 'Unknown'} (${known.observed_at || 'Unobserved'})` : ''}` : stateLabel;
    const choice = snapshot?.choice === 'continue' ? 'Continue' : snapshot?.choice === 'stop' ? 'Stop' : !snapshot || snapshot.degraded ? 'Unknown' : 'None';
    const reason = snapshot?.start_reason?.replaceAll('_', ' ') || 'Observation unavailable';
    const time = snapshot?.observed_at || 'Unobserved';
    line.textContent = `Project${title ? ` · ${title}` : ''}: ${current} · Saved choice: ${choice} · ${reason} · ${time}${snapshot?.degraded ? ' · Degraded' : ''}`;
    line.dataset.state = unknown ? 'unknown' : snapshot.effective_state;
  });
  if (onSettings) {
    const button = doc.createElement('button'); button.type = 'button'; button.className = 'text-button';
    button.textContent = 'Project Manager settings'; button.addEventListener('click', onSettings); container.append(button);
  }
  return unsubscribe;
}

export function createProjectViewReadiness({ document, projectKey, send, viewId = createProjectOperationId(),
  scheduleFrame = callback => typeof requestAnimationFrame === 'function' ? requestAnimationFrame(callback) : callback() }) {
  let challenge = null, rendered = false, acknowledged = null, scheduled = false, connection = 0;
  function ready() {
    if (!rendered || !challenge || acknowledged === challenge.nonce || document.visibilityState === 'hidden' || scheduled) return;
    scheduled = true;
    const generation = connection;
    scheduleFrame(() => {
      if (generation !== connection) return;
      scheduled = false;
      if (!rendered || !challenge || acknowledged === challenge.nonce || document.visibilityState === 'hidden') return;
      acknowledged = challenge.nonce;
      send({ kind: 'project_view_ready', generation: challenge.generation, nonce: challenge.nonce, view_id: viewId });
    });
  }
  return { challenge(value) { if (value?.project_key === projectKey) { challenge = value; ready(); } },
    rendered() { rendered = true; ready(); }, visible: ready,
    connectionLost() { connection += 1; challenge = null; rendered = false; acknowledged = null; scheduled = false; } };
}

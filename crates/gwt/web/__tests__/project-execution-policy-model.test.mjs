import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';

test('Project readiness works on HTTP origins without randomUUID', () => {
  const crypto = globalThis.crypto;
  const descriptor = Object.getOwnPropertyDescriptor(crypto, 'randomUUID');
  try {
    Object.defineProperty(crypto, 'randomUUID', { value: undefined, configurable: true });
    const sent = [];
    const readiness = createProjectViewReadiness({ document: { visibilityState: 'visible' }, projectKey: 'http-project',
      send: event => sent.push(event), scheduleFrame: callback => callback() });
    readiness.challenge({ project_key: 'http-project', generation: 1, nonce: 'host-nonce' });
    readiness.rendered();
    assert.match(sent[0].view_id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  } finally {
    if (descriptor) Object.defineProperty(crypto, 'randomUUID', descriptor); else delete crypto.randomUUID;
  }
});
import { createProjectExecutionPolicyModel, createProjectViewReadiness, mountProjectExecutionStatus } from '../project-execution-policy-model.js';
const key = '0123456789abcdef';
const status = (overrides = {}) => ({ project_key: key, generation: 2, revision: 4, choice: 'continue',
  effective_state: 'running', start_reason: 'user_continue', observed_at: '2026-11-03T10:00:00Z', view_set_revision: 6, degraded: false, ...overrides });
test('policy snapshots reject foreign projects and stale generation or revision', () => {
  const policy = createProjectExecutionPolicyModel(key);
  policy.applyStatus(status());
  policy.applyStatus(status({ project_key: 'foreign', revision: 8 }));
  policy.applyStatus(status({ generation: 1, revision: 9 }));
  policy.applyStatus(status({ revision: 3 }));
  assert.equal(policy.model.read().snapshot.revision, 4);
  policy.applyStatus(status({ view_set_revision: 5, effective_state: 'selection_pending' }));
  assert.equal(policy.model.read().snapshot.view_set_revision, 6, 'a delayed view-set observation cannot undo a later actual view');
  policy.bindGeneration(2); policy.applyStatus(status({ revision: 3 }));
  assert.equal(policy.model.read().snapshot.revision, 4, 'a repeated challenge never permits an older policy revision');
  policy.connectionLost();
  assert.equal(policy.model.read().snapshot.effective_state, 'running');
  assert.equal(policy.model.read().disconnected, true);
});
test('shared projection shows missing observations as unknown and preserves saved choice', () => {
  const { document } = parseHTML('<html><body><div id="hub"></div><div id="pm"></div></body></html>');
  const policy = createProjectExecutionPolicyModel(key);
  const hub = document.getElementById('hub'), pm = document.getElementById('pm');
  mountProjectExecutionStatus(hub, policy.model, { projectTitle: 'Alpha' });
  mountProjectExecutionStatus(pm, policy.model, { projectTitle: 'Alpha' });
  assert.match(hub.textContent, /unknown/i);
  policy.applyStatus(status());
  assert.equal(hub.textContent, pm.textContent);
  assert.match(hub.textContent, /Alpha.*Running.*Continue.*user continue.*2026-11-03/i);
  policy.connectionLost();
  assert.match(hub.textContent, /unknown.*last known.*Running/i);
  assert.match(hub.textContent, /Continue/);
  policy.applyStatus(status({ effective_state: 'unknown', degraded: true, start_reason: 'observation_unavailable', view_set_revision: 0 }));
  assert.match(hub.textContent, /Unknown.*Last known: Running/);
  assert.match(hub.textContent, /Continue/);
});
test('actual view readiness waits for visible render and binds the received challenge once', () => {
  const sent = [], document = { visibilityState: 'hidden' };
  const readiness = createProjectViewReadiness({ document, projectKey: key, viewId: 'view-a', send: event => sent.push(event) });
  readiness.challenge({ project_key: key, generation: 2, nonce: 'server-nonce' });
  readiness.rendered(); assert.equal(sent.length, 0);
  document.visibilityState = 'visible'; readiness.visible();
  assert.deepEqual(sent, [{ kind: 'project_view_ready', generation: 2, nonce: 'server-nonce', view_id: 'view-a' }]);
  readiness.visible(); readiness.rendered(); assert.equal(sent.length, 1);
  readiness.connectionLost(); readiness.challenge({ project_key: key, generation: 3, nonce: 'next' });
  readiness.visible(); assert.equal(sent.length, 1);
  readiness.rendered(); assert.equal(sent.at(-1).nonce, 'next');
});
test('a fresh connection accepts a restarted backend incarnation without losing the last-known display', () => {
  const policy = createProjectExecutionPolicyModel(key); policy.applyStatus(status({ generation: 8 }));
  policy.connectionLost(); policy.applyStatus(status({ generation: 1, revision: 5, effective_state: 'waiting' }));
  assert.equal(policy.model.read().snapshot.generation, 8, 'a disconnect alone cannot accept a new generation');
  policy.bindGeneration(1);
  policy.applyStatus(status({ generation: 1, revision: 5, effective_state: 'waiting' }));
  policy.applyStatus(status({ generation: 8, revision: 9 }));
  assert.equal(policy.model.read().snapshot.generation, 1);
  assert.equal(policy.model.read().disconnected, false);
});

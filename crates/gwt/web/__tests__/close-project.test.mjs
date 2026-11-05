import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { createCloseProjectController } from '../close-project-confirm-modal.js';

const token = { project_key: '0123456789abcdef', generation: 1, nonce: 'nonce' };
function fixture() {
  const { document } = parseHTML('<html><body><button id="origin">Close Project</button></body></html>');
  const sent = [], closed = [], storage = new Map();
  const sessionStorage = { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  const controller = createCloseProjectController({ document, send: message => { sent.push(message); return 'sent'; }, onClosed: key => closed.push(key), sessionStorage });
  document.body.append(controller.modal);
  return { document, controller, sent, closed, sessionStorage };
}
function preview(current, overrides = {}) {
  current.controller.request(token.project_key);
  current.controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [],
    choice_required: true, policy_revision: 3, view_set_revision: 4, view_count: 1, project_wide: false, ...overrides });
}
test('last view always offers Continue Stop Cancel, including an idle project', () => {
  const current = fixture(); preview(current);
  assert.equal(current.sent.length, 1);
  const dialog = current.controller.modal.querySelector('[role="dialog"]');
  assert.equal(dialog.getAttribute('aria-modal'), 'true');
  for (const primitive of ['modal-header', 'modal-body', 'modal-footer']) assert.ok(dialog.querySelector(`.${primitive}`));
  for (const role of ['continue', 'stop', 'cancel']) assert.ok(dialog.querySelector(`[data-role="close-project-${role}"]`));
  current.document.querySelector('[data-role="close-project-continue"]').click();
  const intent = current.sent.at(-1);
  assert.equal(intent.kind, 'confirm_close_project');
  assert.equal(intent.choice, 'continue');
  assert.equal(intent.expected_policy_revision, 3);
  assert.equal(intent.expected_view_set_revision, 4);
  assert.match(intent.operation_id, /^[0-9a-f-]{36}$/);
  assert.deepEqual(intent.token, token);
});
test('ordinary multi-view close detaches only this view without fabricating a choice', () => {
  const current = fixture(); preview(current, { choice_required: false, view_count: 2 });
  assert.equal(current.controller.modal.classList.contains('open'), false);
  assert.equal(current.sent.at(-1).choice, null);
  assert.equal(current.sent.at(-1).kind, 'confirm_close_project');
  current.controller.receive({ kind: 'project_view_closed', project_key: token.project_key });
  assert.deepEqual(current.closed, [token.project_key]);
});
test('Cancel changes no policy and Hub requests an explicit project-wide close', () => {
  const current = fixture();
  current.controller.request(token.project_key, { projectWide: true });
  assert.deepEqual(current.sent.at(-1), { kind: 'preview_close_project', project_key: token.project_key, project_wide: true });
  current.controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [],
    choice_required: true, policy_revision: 3, view_set_revision: 4, view_count: 2, project_wide: true });
  assert.match(current.controller.modal.textContent, /2.*view/i);
  current.document.querySelector('[data-role="close-project-cancel"]').click();
  assert.deepEqual(current.sent.at(-1), { kind: 'cancel_close_project', token });
  assert.equal(current.controller.modal.classList.contains('open'), false);
});
test('ACK loss preserves the immutable exact intent and reconciles status without resending', () => {
  const current = fixture(), payload = { ...token };
  preview(current, { token: payload }); payload.nonce = 'changed';
  current.document.querySelector('[data-role="close-project-stop"]').click();
  const intent = current.sent.at(-1);
  assert.equal(intent.token.nonce, 'nonce');
  current.controller.connectionLost();
  current.controller.reconnect();
  assert.deepEqual(current.sent.at(-1), { kind: 'get_project_execution_status', project_key: token.project_key, operation_id: intent.operation_id });
  current.controller.request(token.project_key);
  assert.equal(current.sent.filter(event => event.kind === 'confirm_close_project').length, 1);
  assert.match(current.controller.modal.textContent, /unconfirmed|unknown|check/i);
  current.controller.receive({ kind: 'project_execution_status', status: { project_key: token.project_key, generation: 1,
    operation_outcome: { operation_id: 'foreign', effect_state: 'confirmed' } } });
  assert.deepEqual(current.closed, []);
  current.controller.receive({ kind: 'project_execution_status', status: { project_key: token.project_key, generation: 1,
    operation_outcome: { operation_id: intent.operation_id, generation: 1, choice: 'stop', effect_state: 'confirmed' } } });
  assert.deepEqual(current.closed, [token.project_key]);
});
test('reload restores only an unanswered exact intent for status reconciliation', () => {
  const current = fixture(); preview(current);
  current.document.querySelector('[data-role="close-project-continue"]').click();
  const intent = current.sent.at(-1), sent = [];
  const restored = createCloseProjectController({ document: current.document, sessionStorage: current.sessionStorage, navigationType: 'reload', send: event => sent.push(event) });
  restored.reconnect();
  assert.deepEqual(sent, [{ kind: 'get_project_execution_status', project_key: token.project_key, operation_id: intent.operation_id }]);
});
test('repeated requests and disconnected previews cannot retain stale authority', () => {
  const current = fixture(); current.controller.request(token.project_key); current.controller.request(token.project_key);
  assert.equal(current.sent.length, 1);
  current.controller.connectionLost(); current.controller.request(token.project_key);
  assert.equal(current.sent.length, 2);
  current.controller.receive({ kind: 'close_project_preview', token: { ...token, project_key: 'foreign' }, title: 'Foreign', choice_required: true });
  assert.equal(current.controller.modal.classList.contains('open'), false);
});
test('a missing or degraded receipt never reallocates the unanswered choice', () => {
  const current = fixture(); preview(current);
  current.document.querySelector('[data-role="close-project-stop"]').click();
  const intent = current.sent.at(-1);
  current.controller.receive({ kind: 'project_execution_status', status: { project_key: token.project_key, degraded: true, operation_outcome: null } });
  const buttons = [...current.controller.modal.querySelectorAll('button')];
  assert.equal(buttons.some(button => /review choice|retry/i.test(button.textContent)), false);
  buttons.find(button => button.textContent === 'Check status').click();
  assert.equal(current.sent.filter(event => event.kind === 'confirm_close_project').length, 1);
  assert.equal(current.sent.at(-1).operation_id, intent.operation_id);
});
test('only a correlated known-before-save refusal permits the exact same intent in this document', () => {
  const current = fixture(); preview(current);
  current.document.querySelector('[data-role="close-project-stop"]').click();
  const intent = current.sent.at(-1);
  current.controller.receive({ kind: 'close_project_error', project_key: token.project_key, code: 'ChoiceNotStarted',
    operation_id: 'foreign', retry_same_intent: true, message: 'Not started' });
  assert.equal([...current.controller.modal.querySelectorAll('button')].some(button => /retry/i.test(button.textContent)), false);
  current.controller.receive({ kind: 'close_project_error', project_key: token.project_key, code: 'ChoiceNotStarted',
    operation_id: intent.operation_id, retry_same_intent: true, message: 'Not started' });
  const retry = [...current.controller.modal.querySelectorAll('button')].find(button => button.textContent === 'Retry same choice');
  assert.ok(retry); retry.click();
  assert.deepEqual(current.sent.at(-1), intent);
  current.controller.connectionLost();
  current.controller.receive({ kind: 'close_project_error', project_key: token.project_key, operation_id: intent.operation_id,
    retry_same_intent: true, message: 'Old connection' });
  assert.equal([...current.controller.modal.querySelectorAll('button')].some(button => /retry/i.test(button.textContent)), false);
});
test('lost ordinary-view detach ACK never leaves a durable choice or blocks a newly rendered view', () => {
  const current = fixture(); preview(current, { choice_required: false, view_count: 2 });
  assert.equal(current.sent.at(-1).choice, null);
  assert.equal(current.sessionStorage.getItem('gwt:project-close-intent'), undefined);
  const sent = [];
  const reloaded = createCloseProjectController({ document: current.document, sessionStorage: current.sessionStorage, navigationType: 'reload',
    send: event => { sent.push(event); return 'sent'; } });
  reloaded.reconnect(); assert.deepEqual(sent, []);
  reloaded.request(token.project_key);
  assert.equal(sent.at(-1).kind, 'preview_close_project');
  current.controller.connectionLost(); current.controller.request(token.project_key);
  assert.equal(current.sent.at(-1).kind, 'preview_close_project');
});

test('a confirmed receipt from an earlier generation cannot close the fresh incarnation', () => {
  const current = fixture(); preview(current);
  current.document.querySelector('[data-role="close-project-continue"]').click();
  const intent = current.sent.at(-1);
  current.controller.receive({ kind: 'project_execution_status', status: { project_key: token.project_key, generation: 2,
    operation_outcome: { operation_id: intent.operation_id, generation: 1, choice: 'continue', effect_state: 'confirmed' } } });
  assert.deepEqual(current.closed, []);
  assert.equal(current.sessionStorage.getItem('gwt:project-close-intent'), undefined);
  current.controller.request(token.project_key);
  assert.equal(current.sent.at(-1).kind, 'preview_close_project');
});
test('cloned tab storage never restores close authority on a fresh navigation', () => {
  const current = fixture(); preview(current);
  current.document.querySelector('[data-role="close-project-continue"]').click();
  const intent = current.sent.at(-1), values = new Map([['gwt:project-close-intent', current.sessionStorage.getItem('gwt:project-close-intent')]]);
  const clone = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
  const sent = [], closed = [];
  const fresh = createCloseProjectController({ document: current.document, sessionStorage: clone, navigationType: 'navigate',
    send: event => { sent.push(event); return 'sent'; }, onClosed: key => closed.push(key) });
  fresh.reconnect(); assert.deepEqual(sent, []);
  fresh.receive({ kind: 'project_execution_status', status: { project_key: token.project_key, generation: 1,
    operation_outcome: { operation_id: intent.operation_id, generation: 1, choice: 'continue', effect_state: 'confirmed' } } });
  assert.deepEqual(closed, []);
  fresh.request(token.project_key); assert.equal(sent.at(-1).kind, 'preview_close_project');
  assert.equal(current.sessionStorage.getItem('gwt:project-close-intent'), JSON.stringify(intent), 'the original tab retains its own pending receipt');
});

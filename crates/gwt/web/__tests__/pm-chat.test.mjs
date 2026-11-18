import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseHTML } from 'linkedom';
import { createPmChat, createPmWindowModel } from '../pm-chat.js';

const report = (id = 'one') => ({ id, kind: 'progress', created_at: '2026-10-11T00:00:00Z',
  body: '# Update\n\n**Ready** and `code`\n\n- checked\n\n```sh\ncargo test\n```',
  body_html: '<h1>Update</h1><p><strong>Ready</strong> and <code>code</code></p><ul><li>checked</li></ul><pre><code>cargo test</code></pre>' });

const structuredReport = (id = 'typed') => ({ ...report(id),
  body: 'Ready <literal>\nNext: Check delivery', body_html: '<p>Ready Next: Check delivery</p>',
  structured: { summary: { result: 'Ready', next_action: 'Check delivery' },
    details: '**All checks passed.**', refs: ['https://example.com/issues/3431'] },
  details_html: '<p><strong>All checks passed.</strong></p><ul><li><a href="https://example.com/issues/3431">https://example.com/issues/3431</a></li></ul>' });

function fixture({ tasksDefault = false, onOpenTerminal } = {}) {
  const { document } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const visibility = [], copied = [], sent = [], waiting = [], reviews = [];
  let model;
  const view = createPmChat({ document, root, sessionId: 'session', onOpenTerminal,
    onTaskCancel: reason => { const request = model.requestPmTaskCancel('pm', reason); if (request) sent.push(request); },
    onTaskReopen: reason => { const request = model.requestPmTaskReopen('pm', reason); if (request) sent.push(request); },
    onDecisionsWaiting: decisions => waiting.push(decisions),
    onReviewAttention: items => reviews.push(items),
    onTaskDetail: key => { const request = model.requestPmTaskDetail('pm', key); if (request) sent.push(request); },
    onLogVisibility: visible => visibility.push(visible), copyText: async text => copied.push(text),
    onSend: text => { const request = model.requestPmChatSend('pm', text); if (request) sent.push(request); } });
  if (!tasksDefault) view.showReports();
  model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  model.subscribePmWindowState(state => state.windows.pm, state => view.update(state));
  let revision = 0;
  const update = (reports = [], error = null, current = null, current_error = undefined) => {
    const request = model.requestPmReports('pm');
    const event = { kind: 'pm_reports', id: 'pm', request_id: request.request_id,
      version: { epoch: 'fixture', revision: ++revision }, snapshot: request.version === null,
      session_id: model.readPmWindowState().windows.pm.sessionId, reports, error, current,
      ...(current_error === undefined ? {} : { current_error }) };
    model.applyPmWindowReceiveEvent(event);
    return event;
  };
  return { root, model, view, visibility, copied, sent, waiting, reviews, update };
}

test('Chat opens its bound PM terminal in Works without losing the draft', () => {
  const opened = [];
  let available = false;
  const { root, model, view } = fixture({ onOpenTerminal: session => { opened.push(session); return available; } });
  view.showChat();
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'Keep this unfinished request';
  const button = [...root.querySelectorAll('.pm-chat__chat button')]
    .find(node => node.textContent === 'Open terminal in Works');
  assert.ok(button, 'Chat offers a direct Works terminal action');
  button.click();
  assert.deepEqual(opened, ['session']);
  assert.match(root.querySelector('.pm-chat__terminal-status').textContent, /unavailable.*current PM session/i);
  assert.equal(draft.value, 'Keep this unfinished request');
  assert.equal(view.isChatVisible(), true);
  available = true;
  button.click();
  assert.equal(root.querySelector('.pm-chat__terminal-status').hidden, true);
  assert.equal(draft.value, 'Keep this unfinished request');
  model.bindPmWindowState('pm', 'replacement');
  view.showChat();
  button.click();
  assert.deepEqual(opened, ['session', 'session', 'replacement']);
  view.dispose();
  button.click();
  assert.equal(opened.length, 3);
});

const taskOverview = (tasks = [], pending_intake = 2) => ({ state: 'ready', revision: 4, total: tasks.length,
  pending_intake, tasks });
const trackedTask = (key, extra = {}) => ({ key, parent_key: null, title: `Task ${key}`,
  status: 'pending', assignee: 'PM', wait_reason: null, next_action: 'Check the original outcome',
  review_required: false, ...extra });

const bounded = text => ({ text, total_chars: [...text].length });
const selectedDetail = () => ({ status: 'pending', original: bounded('Exact <original>\nrequest'), purpose: bounded('Original purpose'),
  completion_conditions: [bounded('Verify <criterion>')], completion_condition_total: 2,
  evidence: [bounded('proof:<literal>')], evidence_total: 2, history_total: 2,
  history: [{ revision: 4, action: 'complete', actor_kind: 'pm', actor_id: bounded('PM reviewer'),
    at: '2026-11-13T00:00:00Z', reason: bounded('Verified <outcome>') }] });
const detailReply = (request, extra = {}) => ({ kind: 'pm_request_task', id: 'pm', request_id: request.request_id,
  result: { state: 'detail', key: request.task_key, revision: 4, detail: selectedDetail(), ...extra } });

test('Task Reopen requires fresh completed details and matches its own action receipt', () => {
  const { model, update } = fixture();
  assert.equal(typeof model.requestPmTaskReopen, 'function');
  update([], null, { request_tasks: taskOverview([trackedTask('one', { status: 'completed' })]) });
  const read = model.requestPmTaskDetail('pm', 'one');
  assert.equal(model.requestPmTaskReopen('pm', 'Review again'), null);
  model.applyPmWindowReceiveEvent(detailReply(read));
  assert.equal(model.requestPmTaskReopen('pm', 'Review again'), null, 'saved pending detail wins over completed overview');
  const completed = { ...selectedDetail(), status: 'completed' };
  model.applyPmWindowReceiveEvent(detailReply(model.requestPmTaskDetail('pm', 'one'), { detail: completed }));
  assert.equal(model.requestPmTaskReopen('pm', '  '), null);
  const request = model.requestPmTaskReopen('pm', 'Review the original outcome again');
  assert.equal(request.kind, 'reopen_pm_request_task');
  assert.equal(request.task_key, 'one');
  assert.deepEqual(request.request, { operation_id: request.request.operation_id, expected_revision: 4,
    task_id: '', reason: 'Review the original outcome again' });
  assert.equal(model.requestPmTaskCancel('pm', 'Cancel instead'), null, 'one shared pending mutation');
  assert.equal(model.requestPmTaskReopen('pm', 'Duplicate'), null);
  const reply = { kind: 'pm_request_task', id: 'pm', request_id: request.request_id,
    result: { state: 'ready', receipt: { action: 'reopen', operation_id: request.request.operation_id, revision: 5 } } };
  model.applyPmWindowReceiveEvent({ ...reply, result: { ...reply.result, receipt: { ...reply.result.receipt, action: 'cancel' } } });
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'pending');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.data.status, 'completed', 'never optimistic');
  model.applyPmWindowReceiveEvent(reply);
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'recorded');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.stale, true);
  assert.equal(model.requestPmTaskReopen('pm', 'Old detail'), null);
  model.applyPmWindowReceiveEvent(detailReply(model.requestPmTaskDetail('pm', 'one'), { revision: 5, detail: completed }));
  const retry = model.requestPmTaskReopen('pm', 'Reread before retrying');
  model.markPmTaskMutationUnavailable('pm', retry.request_id);
  model.applyPmWindowReceiveEvent({ ...reply, request_id: retry.request_id,
    result: { ...reply.result, receipt: { ...reply.result.receipt, operation_id: retry.request.operation_id, revision: 6 } } });
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'error', 'late receipt cannot confirm a timed-out action');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(model.readPmWindowState().windows.pm.taskMutation, null);
});

test('completed Task Reopen exposes a labelled reason and retains drafts on conflict', () => {
  const { model, update, root, sent } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one', { status: 'completed' })]) });
  const read = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(read, { detail: { ...selectedDetail(), status: 'completed' } }));
  const reopen = [...root.querySelectorAll('button')].find(node => node.textContent === 'Reopen task');
  assert.ok(reopen);
  assert.equal(reopen.hidden, false);
  reopen.click();
  assert.equal(sent.length, 0);
  const reason = root.querySelector('[aria-label="Reason for this task change"]');
  reason.value = 'The original outcome is not met';
  reason.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
  root.querySelector('.pm-chat__draft').value = 'Keep my conversation draft';
  reopen.click();
  reopen.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, 'reopen_pm_request_task');
  model.applyPmWindowReceiveEvent({ kind: 'pm_request_task', id: 'pm', request_id: sent[0].request_id,
    result: { state: 'error', code: 'conflict', message: 'The task changed. Reload details.' } });
  assert.equal(reason.value, 'The original outcome is not met');
  assert.equal(root.querySelector('.pm-chat__draft').value, 'Keep my conversation draft');
  assert.equal(reopen.getAttribute('aria-disabled'), 'true');
  assert.match(root.querySelector('.pm-chat__task-cancel-status').textContent, /task changed/);
});

test('Task cancellation requires a fresh selected read and correlates only its own receipt', () => {
  const { model, update } = fixture();
  assert.equal(typeof model.requestPmTaskCancel, 'function');
  update([], null, { request_tasks: taskOverview([trackedTask('one')]) });
  const read = model.requestPmTaskDetail('pm', 'one');
  assert.equal(model.requestPmTaskCancel('pm', 'Withdraw'), null);
  model.applyPmWindowReceiveEvent(detailReply(read));
  assert.equal(model.requestPmTaskCancel('pm', '  '), null);
  const request = model.requestPmTaskCancel('pm', 'Withdraw <literally>');
  assert.equal(request.kind, 'cancel_pm_request_task');
  assert.equal(request.task_key, 'one');
  assert.equal(request.request.task_id, '');
  assert.equal(request.request.expected_revision, 4);
  assert.equal(request.request.reason, 'Withdraw <literally>');
  assert.match(request.request.operation_id, /^task-cancel-/);
  assert.equal(model.requestPmTaskCancel('pm', 'Duplicate'), null);
  const reply = { kind: 'pm_request_task', id: 'pm', request_id: request.request_id,
    result: { state: 'ready', revision: 5, task: { status: 'cancelled' },
      receipt: { action: 'cancel', operation_id: request.request.operation_id, revision: 5 } } };
  model.applyPmWindowReceiveEvent({ ...reply, request_id: request.request_id + 1 });
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'pending');
  model.applyPmWindowReceiveEvent({ ...reply, result: { ...reply.result, receipt: { ...reply.result.receipt, operation_id: 'other' } } });
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'pending');
  model.markPmTaskMutationUnavailable('pm', request.request_id);
  model.applyPmWindowReceiveEvent(reply);
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'error');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.stale, true);
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.data.status, 'pending', 'never optimistic');
  assert.equal(model.requestPmTaskCancel('pm', 'Retry'), null);
  update([], null, { request_tasks: { state: 'error', message: 'The list read failed' } });
  const reload = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(reload, { revision: 5 }));
  const retry = model.requestPmTaskCancel('pm', 'Withdraw after rereading');
  assert.ok(retry, 'a fresh selected read recovers cancellation without pretending the list recovered');
  assert.equal(model.readPmWindowState().windows.pm.requestTasksError, 'The list read failed');
  model.applyPmWindowReceiveEvent({ ...reply, request_id: retry.request_id,
    result: { ...reply.result, revision: 6, receipt: { ...reply.result.receipt, operation_id: retry.request.operation_id, revision: 6 } } });
  assert.equal(model.readPmWindowState().windows.pm.taskMutation.status, 'recorded');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.stale, true);
  model.bindPmWindowState('pm', 'replacement');
  model.applyPmWindowReceiveEvent(reply);
  assert.equal(model.readPmWindowState().windows.pm.taskMutation, null);
});

test('Task detail cancellation keeps its reason and chat draft while stale and exposes labelled controls', async () => {
  const { model, update, root, sent } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one')]) });
  const read = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(read));
  const reason = root.querySelector('[aria-label="Reason for this task change"]');
  assert.ok(reason, 'the selected detail exposes a labelled reason field');
  const cancel = [...root.querySelectorAll('button')].find(node => node.textContent === 'Cancel task');
  cancel.click();
  assert.equal(sent.length, 0, 'empty reason cannot submit');
  reason.value = 'Withdraw this request';
  reason.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
  root.querySelector('.pm-chat__draft').value = 'Keep my chat draft';
  cancel.click();
  cancel.click();
  assert.equal(sent.length, 1);
  assert.equal(cancel.getAttribute('aria-disabled'), 'true');
  model.applyPmWindowReceiveEvent({ kind: 'pm_request_task', id: 'pm', request_id: sent[0].request_id,
    result: { state: 'error', code: 'conflict', message: 'The task changed. Reload details.' } });
  assert.equal(reason.value, 'Withdraw this request');
  assert.match(root.querySelector('.pm-chat__task-cancel-status').textContent, /task changed/);
  assert.equal(root.querySelector('.pm-chat__draft').value, 'Keep my chat draft');
  update([], null, { request_tasks: { state: 'error', message: 'The list read failed' } });
  const reload = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(reload, { revision: 5 }));
  assert.equal(cancel.getAttribute('aria-disabled'), 'false', 'fresh details are actionable while the list warning remains');
  update([], null, { request_tasks: { state: 'error', message: 'A new read failed' } });
  assert.equal(cancel.getAttribute('aria-disabled'), 'true', 'a new error still fences stale details');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(reason.value, '');
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  assert.match(css, /\.pm-chat__task-cancel\s*\{[^}]*var\(--space-/);
});

test('selected Task reads correlate key, request and Session and retain confirmed stale data', () => {
  const { model, update } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one'), trackedTask('two')]) });
  assert.equal(typeof model.requestPmTaskDetail, 'function');
  const first = model.requestPmTaskDetail('pm', 'one');
  assert.deepEqual(first, { kind: 'load_pm_request_task', id: 'pm', request_id: first.request_id, task_key: 'one' });
  const second = model.requestPmTaskDetail('pm', 'two');
  model.applyPmWindowReceiveEvent(detailReply(first));
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.status, 'loading');
  model.applyPmWindowReceiveEvent(detailReply(second, { key: 'one' }));
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.status, 'loading');
  model.applyPmWindowReceiveEvent(detailReply(second));
  const confirmed = model.readPmWindowState().windows.pm.taskDetail.data;
  assert.equal(confirmed.original.text, 'Exact <original>\nrequest');
  update([], null, { request_tasks: { ...taskOverview([]), revision: 5 } });
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.stale, true);
  const reload = model.requestPmTaskDetail('pm', 'two');
  model.applyPmWindowReceiveEvent(detailReply(reload));
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.status, 'error', 'older revision cannot refresh detail');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.data, confirmed);
  const retry = model.requestPmTaskDetail('pm', 'two');
  model.applyPmWindowReceiveEvent({ ...detailReply(retry), result: { state: 'error', message: 'Restore saved data.' } });
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.data, confirmed);
  const pending = model.requestPmTaskDetail('pm', 'two');
  model.bindPmWindowState('pm', 'replacement');
  model.applyPmWindowReceiveEvent(detailReply(pending));
  assert.equal(model.readPmWindowState().windows.pm.taskDetail, null);
});

test('Tasks and Chat expose literal bounded selected detail with reload, Close and retained draft', () => {
  const { root, view, model, sent, update } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one')]) });
  view.showChat();
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'Keep my draft';
  const button = [...root.querySelectorAll('.pm-chat__task button')].find(node => node.textContent === 'View task details');
  assert.ok(button, 'selected details are reachable from the shared row');
  button.click();
  const request = sent.at(-1);
  model.applyPmWindowReceiveEvent(detailReply(request));
  const detail = root.querySelector('.pm-chat__task-detail');
  assert.equal(detail.getAttribute('aria-label'), 'Selected task details');
  assert.match(detail.textContent, /Exact <original>\nrequest/);
  assert.match(detail.textContent, /Showing 1 of 2 completion criteria/);
  assert.match(detail.textContent, /Showing 1 of 2 evidence references/);
  assert.match(detail.textContent, /Showing 1 of 2 recorded changes/);
  assert.match(detail.textContent, /Verified <outcome>/);
  assert.equal(detail.querySelectorAll('original,criterion,literal,outcome').length, 0);
  assert.equal(draft.value, 'Keep my draft');
  const tasks = [...root.querySelectorAll('.pm-chat__toolbar button')].find(node => node.textContent === 'Tasks');
  tasks.click();
  assert.equal(detail.hidden, false);
  const reload = [...detail.querySelectorAll('button')].find(node => node.textContent === 'Reload details');
  reload.click();
  model.markPmTaskDetailUnavailable('pm');
  assert.match(detail.textContent, /last confirmed|stale/i);
  assert.match(detail.textContent, /Exact <original>/);
  [...detail.querySelectorAll('button')].find(node => node.textContent === 'Close details').click();
  assert.equal(detail.hidden, true);
  assert.equal(draft.value, 'Keep my draft');
});

test('selected detail reload cannot roll back a confirmed revision ahead of the overview', () => {
  const { model, update } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one')]) });
  const first = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(first, { revision: 6 }));
  const confirmed = model.readPmWindowState().windows.pm.taskDetail.data;
  const reload = model.requestPmTaskDetail('pm', 'one');
  model.applyPmWindowReceiveEvent(detailReply(reload, { revision: 5 }));
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.status, 'error');
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.revision, 6);
  assert.equal(model.readPmWindowState().windows.pm.taskDetail.data, confirmed);
});

test('selected Task detail styles use Operator tokens and preserve literal whitespace', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  assert.match(css, /\.pm-chat__task-detail\s*\{[^}]*var\(--color-border\)/);
  assert.match(css, /\.pm-chat__task-detail-text\s*\{[^}]*white-space:\s*pre-wrap/);
});

test('Home opens Chat in one action and rebinds without showing the previous conversation (AC-3431-CHAT-1)', () => {
  const { root, model, view } = fixture({ tasksDefault: true });
  assert.equal(view.isChatVisible(), false, 'Home remains the default PM view');
  const home = root.querySelector('.task-home');
  assert.ok(home && !home.hidden, 'Home is visible on entry');
  const chatButton = [...root.querySelectorAll('.pm-chat__toolbar button')]
    .find(button => button.textContent === 'Chat');
  chatButton.click();
  assert.equal(view.isChatVisible(), true, 'one toolbar action opens Chat');
  model.applyPmWindowReceiveEvent(conversation([{ id: 'old', role: 'assistant', text: 'Previous session' }]));
  assert.match(root.querySelector('.pm-chat__chat').textContent, /Previous session/);
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(view.isChatVisible(), false, 'a new session returns to Home');
  assert.equal(home.hidden, false, 'Home is visible after rebinding');
  chatButton.click();
  assert.equal(view.isChatVisible(), true, 'the rebound session keeps one-action Chat access');
  assert.equal(root.querySelectorAll('.pm-chat__bubble').length, 0, 'previous-session bubbles are cleared');
  model.applyPmWindowReceiveEvent(conversation([{ id: 'stale', role: 'assistant', text: 'Stale reply' }]));
  assert.equal(root.querySelectorAll('.pm-chat__bubble').length, 0, 'a delayed old-session reply stays hidden');
  model.applyPmWindowReceiveEvent({ ...conversation([{ id: 'new', role: 'assistant', text: 'New session' }]),
    session_id: 'replacement' });
  assert.deepEqual([...root.querySelectorAll('.pm-chat__bubble')].map(node => node.textContent), ['New session']);
});

test('Home is the default PM view; Tasks preserves the unfinished filter and stable rows', () => {
  const { root, update, view, model } = fixture({ tasksDefault: true });
  update([report()], null, { request_tasks: taskOverview([
    trackedTask('parent'), trackedTask('child', { parent_key: 'parent', status: 'waiting',
      wait_reason: 'Waiting for CI', review_required: true }),
    trackedTask('done', { status: 'completed' }), trackedTask('cancelled', { status: 'cancelled' }),
  ]) });
  assert.ok(root.querySelector('.task-home'), 'a Home surface exists');
  assert.equal(root.querySelector('.pm-chat__tasks').hidden, true, 'Home is the entry view');
  view.showTasks();
  const tasks = root.querySelector('.pm-chat__tasks');
  assert.ok(tasks, 'a separate Tasks surface exists');
  assert.equal(tasks.hidden, false);
  assert.equal(root.querySelector('.pm-chat__body').hidden, true);
  assert.equal(root.querySelectorAll('.pm-chat__task').length, 2);
  assert.match(tasks.textContent, /Waiting for CI/);
  assert.match(tasks.textContent, /PM reassessment required/);
  assert.match(tasks.textContent, /Part of: Task parent/);
  assert.match(tasks.textContent, /2 requests await classification/);
  const row = root.querySelector('.pm-chat__task');
  update([report('new')], null, { request_tasks: taskOverview([
    trackedTask('parent'), trackedTask('child', { parent_key: 'parent', status: 'waiting',
      wait_reason: 'Waiting for CI', review_required: true }),
    trackedTask('done', { status: 'completed' }), trackedTask('cancelled', { status: 'cancelled' }),
  ], 0) });
  assert.match(tasks.textContent, /0 requests await classification/);
  assert.equal(root.querySelector('.pm-chat__task'), row, 'report polls do not replace Task rows');
  const noticeNode = tasks.querySelector('[role="status"]').firstChild;
  update([report('poll')], null, model.readPmWindowState().windows.pm.current);
  assert.ok(tasks.querySelector('[role="status"]').firstChild === noticeNode,
    'unchanged polls do not replace the live status text');
  const filter = [...tasks.querySelectorAll('button')].find(node => node.textContent === 'Include completed');
  filter.click();
  assert.equal(root.querySelectorAll('.pm-chat__task').length, 4);
  view.showReports();
  assert.equal(tasks.hidden, true);
  assert.equal(root.querySelector('.pm-chat__body').hidden, false);
  view.showTasks();
  assert.equal(filter.getAttribute('aria-pressed'), 'true');
  assert.equal(root.querySelectorAll('.pm-chat__task').length, 4);
});

test('Tasks and Chat show direct-child progress from the full projection independently of filters', () => {
  const { root, update, view } = fixture();
  const parent = trackedTask('parent', { child_progress: { total: 55, completed: 2, cancelled: 1 } });
  const publish = task => update([], null, { request_tasks: taskOverview([
    task, trackedTask('child', { parent_key: 'parent' }), trackedTask('legacy'),
  ]) });
  publish(parent);
  view.showTasks();
  const row = root.querySelector('[data-task-key="parent"]');
  const label = 'Direct subtasks: 2 completed · 1 cancelled · 52 unfinished (55 total)';
  assert.match(row.textContent, /Pending/);
  assert.ok(row.textContent.includes(label), 'counts include children outside the preview');
  assert.equal(root.querySelector('[data-task-key="legacy"]').textContent.includes('Direct subtasks'), false);
  [...root.querySelectorAll('button')].find(node => node.textContent === 'Include completed').click();
  assert.ok(row.textContent.includes(label), 'filtering does not recalculate from visible rows');
  view.showChat();
  assert.ok(row.textContent.includes(label), 'Chat uses the same progress projection');
  publish(parent);
  assert.equal(root.querySelector('[data-task-key="parent"]'), row, 'unchanged polls retain the row');
  publish({ ...parent, child_progress: { total: 55, completed: 55, cancelled: 0 } });
  assert.ok(row.textContent.includes('Direct subtasks: 55 completed · 0 cancelled · 0 unfinished (55 total)'));
  assert.match(row.textContent, /Pending/, 'finishing children does not complete their parent');
});

test('Task criteria disclose bounded literal previews and retain the same Task disclosure', () => {
  const { root, model, update, view } = fixture();
  const criteria = { purpose: 'Keep <img src=x> as literal purpose',
    completion_conditions: ['Verify <script>literal</script>', 'Confirm the original outcome'], completion_condition_total: 7 };
  let task = trackedTask('known', { source_key: 'source', criteria_preview: criteria });
  const publish = () => update([report()], null, { request_tasks: taskOverview([task, trackedTask('legacy')]) });
  publish();
  view.showChat();
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'Keep my unfinished request';
  const disclosure = () => root.querySelector('[data-task-key="known"] details.pm-chat__task-criteria');
  const initial = disclosure();
  assert.ok(initial, 'saved criteria use a native disclosure');
  assert.equal(initial.querySelector('summary').textContent, 'Purpose and completion criteria');
  assert.equal(initial.hasAttribute('open'), false);
  assert.ok(initial.textContent.includes(criteria.purpose));
  assert.deepEqual([...initial.querySelectorAll('ol li')].map(node => node.textContent), criteria.completion_conditions);
  assert.match(initial.textContent, /Showing 2 of 7 completion criteria\. Text is limited to 240 characters per item\./);
  assert.equal(initial.querySelector('img, script'), null, 'preview content stays literal');
  const legacy = root.querySelector('[data-task-key="legacy"]');
  assert.equal(legacy.querySelector('.pm-chat__task-criteria'), null);
  assert.ok(legacy.querySelector('button'), 'an older summary remains usable');
  initial.setAttribute('open', '');
  publish();
  assert.equal(disclosure(), initial, 'report-only polls retain the disclosure node');
  task = { ...task, title: 'Updated title', criteria_preview: { ...criteria, purpose: 'Updated purpose' } };
  publish();
  assert.equal(disclosure().hasAttribute('open'), true, 'same-key Task updates retain the disclosure');
  assert.ok(disclosure().textContent.includes('Updated purpose'));
  model.applyPmWindowReceiveEvent(conversation([{ id: 'original', role: 'user', text: 'Original request', source_key: 'source' }]));
  assert.equal(disclosure().hasAttribute('open'), true, 'source-link arrival retains the disclosure');
  update([], null, { request_tasks: { state: 'error', message: 'Unreadable task store' } });
  assert.equal(disclosure().hasAttribute('open'), true, 'a stale confirmed preview stays expanded');
  assert.match(root.querySelector('.pm-chat__tasks').textContent, /last confirmed list/);
  assert.equal(draft.value, 'Keep my unfinished request');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(root.querySelectorAll('.pm-chat__task-criteria').length, 0, 'a new PM Session clears old criteria');
});

test('Task criteria disclosure uses Operator tokens and a visible keyboard focus ring', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  for (const selector of ['\\.pm-chat__task-criteria', '\\.pm-chat__task-criteria > summary']) {
    const rule = css.match(new RegExp(`${selector}\\s*\\{([^}]+)\\}`));
    assert.ok(rule, `${selector} has an Operator style rule`);
    assert.match(rule[1], /var\(--/);
    assert.doesNotMatch(rule[1], /#[0-9a-f]|rgba?\(/i);
  }
  assert.match(css, /\.pm-chat__task-criteria > summary:focus-visible\s*\{[^}]*outline:[^}]*var\(--color-focus-ring\)/);
});

test('A current-card failure keeps the last explanation but never hides newly saved reports (#6950)', () => {
  const { root, model, update } = fixture();
  const confirmed = { status: { items: [{ kind: 'waiting', text: 'Watching CI', started_at: '2026-10-13T09:00:00Z' }], next_trigger: 'CI result' }, request_tasks: taskOverview() };
  update([report('one')], null, confirmed);
  const before = model.readPmWindowState().windows.pm.current;
  update([report('one'), report('two')], null, { request_tasks: taskOverview() }, 'PM current status could not be loaded');
  const state = model.readPmWindowState().windows.pm;
  assert.deepEqual(state.reports.map(item => item.id), ['one', 'two'], 'saved reports are displayed');
  assert.equal(state.current, before, 'the last confirmed explanation is kept');
  assert.equal(root.querySelectorAll('.pm-chat__body [data-report-id]').length, 2);
  assert.match(root.querySelector('.pm-chat__body').textContent, /PM current status could not be loaded/);
  update([report('three')], 'Unreadable reports', { request_tasks: taskOverview() });
  assert.deepEqual(model.readPmWindowState().windows.pm.reports.map(item => item.id), ['one', 'two'],
    'only a reports failure keeps the last confirmed list');
});

test('Task read errors preserve confirmed data in the shared immutable model and recover explicitly', () => {
  const { root, model, update } = fixture({ tasksDefault: true });
  update();
  assert.match(root.querySelector('.pm-chat__tasks').textContent, /unavailable/i);
  update([], null, { request_tasks: { ...taskOverview([trackedTask('known')], 7), total: 83 } });
  const before = model.readPmWindowState().windows.pm.requestTasks;
  assert.equal(before?.state, 'ready');
  assert.ok(Object.isFrozen(before));
  update();
  assert.equal(model.readPmWindowState().windows.pm.requestTasks, before);
  assert.match(root.querySelector('.pm-chat__tasks').textContent, /last confirmed list.*unavailable/is);
  assert.match(root.querySelector('.pm-chat__tasks').textContent,
    /Last confirmed summary \(stale\): 1 of 83 recorded tasks shown \(preview limited to 50; unfinished first\)\. 7 requests await classification\./);
  update([], null, { request_tasks: { state: 'error', message: 'Unreadable task store' } });
  assert.equal(model.readPmWindowState().windows.pm.requestTasks, before);
  assert.equal(root.querySelectorAll('.pm-chat__task').length, 1);
  assert.match(root.querySelector('.pm-chat__tasks').textContent, /last confirmed list.*Unreadable task store/s);
  update([], 'Unreadable reports', { request_tasks: taskOverview([trackedTask('fresh')]) });
  assert.equal(model.readPmWindowState().windows.pm.requestTasks.tasks[0].key, 'fresh',
    'Reports failure cannot hide independently confirmed Task data');
  update([], null, { request_tasks: { state: 'absent' } });
  assert.equal(root.querySelectorAll('.pm-chat__task').length, 0);
  assert.match(root.querySelector('.pm-chat__tasks').textContent, /No request tasks have been recorded/);
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(model.readPmWindowState().windows.pm.requestTasks, null, 'a new session clears the old projection');
});

test('Reports revisions recover gaps and fence stale requests without starving slow replies', () => {
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  const request = model.requestPmReports('pm');
  assert.equal(request.version, null);
  const event = (request, revision, extra = {}) => ({ kind: 'pm_reports', id: 'pm', session_id: 'session',
    request_id: request.request_id, version: { epoch: 'server', revision }, snapshot: request.version === null,
    reports: [report(String(revision))], ...extra });
  model.applyPmWindowReceiveEvent(event(request, 1));
  const accepted = model.readPmWindowState().windows.pm;
  const slow = model.requestPmReports('pm');
  model.requestPmReports('pm'); // A later poll must not starve a slow valid reply.
  model.applyPmWindowReceiveEvent(event(slow, 2));
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, '2');
  model.applyPmWindowReceiveEvent(event(request, 1));
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, '2');
  const ordinary = model.requestPmReports('pm');
  assert.equal(model.applyPmWindowReceiveEvent(event(ordinary, 4)), true, 'gap requests a full reload');
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, '2');
  const reset = model.requestPmReports('pm');
  assert.equal(reset.version, null);
  assert.deepEqual(model.requestPmReports('pm'), reset, 'retry the same force token after a lost reply');
  model.applyPmWindowReceiveEvent(event(reset, 4));
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, '4');
  assert.notEqual(model.readPmWindowState().windows.pm, accepted);
  const oldEpoch = model.requestPmReports('pm', true);
  const newEpoch = model.requestPmReports('pm', true);
  model.applyPmWindowReceiveEvent(event(newEpoch, 1, { version: { epoch: 'restart', revision: 1 } }));
  model.applyPmWindowReceiveEvent(event(oldEpoch, 99));
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, '1');
  assert.equal(model.readPmWindowState().windows.pm.reportsVersion.epoch, 'restart');
  const current = model.readPmWindowState().windows.pm;
  const duplicate = model.requestPmReports('pm');
  model.applyPmWindowReceiveEvent(event(duplicate, 1, { version: current.reportsVersion }));
  assert.equal(model.readPmWindowState().windows.pm, current);
  const resetAgain = model.requestPmReports('pm', true);
  model.applyPmWindowReceiveEvent(event(resetAgain, 0, { version: { epoch: 'restart', revision: 0 } }));
  assert.equal(model.readPmWindowState().windows.pm, current, 'force cannot roll back within an epoch');
  model.bindPmWindowState('pm', 'replacement');
  model.applyPmWindowReceiveEvent(event(resetAgain, 2));
  assert.equal(model.readPmWindowState().windows.pm.loaded, false);
});

test('Reports commits the content type before any view renders it', () => {
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  const incoming = report();
  const request = model.requestPmReports('pm');
  model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm', session_id: 'session', reports: [incoming],
    request_id: request.request_id, version: { epoch: 'fixture', revision: 1 }, snapshot: true });
  const committed = model.readPmWindowState().windows.pm.reports[0];
  assert.deepEqual(committed.content, { type: 'markdown', body: incoming.body, body_html: incoming.body_html });
  incoming.body_html = '<p>changed outside the model</p>';
  assert.match(committed.content.body_html, /<strong>Ready<\/strong>/);
  assert.ok(Object.isFrozen(committed.content));
});

test('current summary defaults to collapsed and keeps disclosure across polling per window', () => {
  const { root, update } = fixture();
  const current = { status: { items: [
    { kind: 'working', text: '確認しています', started_at: '2026-10-14T00:00:00Z' },
    { kind: 'waiting', text: 'Waiting for CI', started_at: '2026-10-14T00:00:00Z' },
  ], next_trigger: 'CI passes', user_action: null, updated_at: '2026-10-14T00:00:00Z' }, stale_minutes: 15 };
  update([report()], null, current);
  const details = root.querySelector('details');
  assert.ok(details, 'current card uses native disclosure');
  assert.equal(details.hasAttribute('open'), false);
  assert.match(details.querySelector('summary').textContent, /1 working.*1 waiting.*Next: CI passes.*User action: None/s);
  assert.match(details.querySelector('summary').textContent, /Updated:.*PM has not updated for 15 minutes/s);
  details.setAttribute('open', '');
  update([report()], null, { ...current, stale_minutes: 16 });
  assert.equal(root.querySelector('details'), details, 'polling keeps the disclosure node');
  assert.equal(details.hasAttribute('open'), true);
  assert.match(root.textContent, /確認しています/, 'authored content keeps its language');
  assert.equal(fixture().root.querySelector('details').hasAttribute('open'), false, 'another window starts collapsed');
  assert.ok(root.querySelector('.pm-chat__history-header select'), 'history heading and filter share a row');
});

test('Reports has a single scroll body and compact token-based history', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const rule = name => css.match(new RegExp(`\\.${name}\\s*\\{([^}]+)\\}`))[1];
  assert.match(rule('pm-chat__body'), /overflow-y:\s*auto/);
  assert.doesNotMatch(rule('pm-chat__current'), /max-height|overflow:\s*auto/);
  assert.doesNotMatch(rule('pm-chat__transcript'), /overflow:\s*auto/);
  assert.match(rule('pm-chat__message'), /padding:\s*var\(--space-2\)/);
  assert.match(css, /\.pm-chat__current-summary > span\s*\{[^}]*white-space:\s*nowrap[^}]*text-overflow:\s*ellipsis/);
});

test('current card stays above history and distinguishes PM explanation from automatic facts', () => {
  const { root, model, view, update } = fixture();
  const card = root.querySelector('[aria-label="PM status"]');
  assert.ok(card, 'current card exists before any PM post');
  assert.match(card.textContent, /No current status reported yet/);
  const current = {
    observed_at: '2026-10-13T09:20:00.100Z', stale_minutes: 15,
    status: { items: [{ kind: 'waiting', text: 'v9.117.2 の公開を待っています', started_at: '2026-10-13T09:00:00Z' }],
      next_trigger: 'v9.117.2 の公開', user_action: null, updated_at: '2026-10-13T09:05:00Z' },
    facts: { source: 'daemon', active_windows: [{ issue_number: 5349, title: 'PM current state', last_activity_at: '2026-10-13T09:19:00Z' }],
      queue_head: { issue_number: 5342, title: 'Instruction memory' }, needs_human_count: 2,
      release: { number: 5352, title: 'Release v9.117.2', url: 'https://github.com/akiojin/gwt/pull/5352' },
      release_observed_at: '2026-10-13T09:18:00Z', pending_update: '9.117.2', error: null },
  };
  const older = update([report()], null, current);
  view.receiveHumanAttention(2);
  assert.equal(root.querySelector('.pm-chat__body').firstElementChild, card);
  assert.match(card.querySelector('[aria-label="PM notes"]').textContent, /Waiting.*v9.117.2/s);
  assert.equal(card.querySelector('[aria-label="PM notes"] time').getAttribute('datetime'), current.status.items[0].started_at);
  assert.match(card.textContent, /User action: None/);
  assert.match(card.textContent, /PM has not updated for 15 minutes/);
  const facts = card.querySelector('[aria-label="Live facts"]');
  assert.match(facts.textContent, /#5349.*PM current state/s);
  assert.match(facts.textContent, /#5342.*Instruction memory/s);
  assert.match(facts.textContent, /needs_human: 2/);
  assert.match(facts.textContent, /Release v9.117.2/);
  assert.match(facts.textContent, /Pending update: v9.117.2/);
  assert.equal(root.querySelectorAll('article').length, 1, 'current state is outside chronological reports');
  update([report()], null, { ...current, observed_at: '2026-10-13T09:20:00.100001Z', stale_minutes: null, status: { ...current.status,
    items: [{ ...current.status.items[0], text: '公開結果を確認しています', kind: 'working' }], user_action: '配信設定を確認' } });
  assert.doesNotMatch(card.textContent, /公開を待っています|has not updated for 15 minutes/);
  assert.match(card.textContent, /公開結果を確認しています|配信設定を確認/);
  model.applyPmWindowReceiveEvent(older);
  assert.match(card.textContent, /公開結果を確認しています/, 'an older concurrent snapshot cannot restore the earlier status');
  assert.doesNotMatch(card.textContent, /公開を待っています/);
  update([], 'Reports could not be loaded');
  assert.match(card.textContent, /公開結果を確認しています/, 'failed reads retain latest measured state');
});

test('current card marks unknown automatic data instead of claiming an empty fleet', () => {
  const { root, update } = fixture();
  update([], null, { status: null, facts: { source: 'unavailable', active_windows: [], queue_head: null,
    needs_human_count: null, release: null, release_observed_at: null, pending_update: null, error: 'Issues unavailable' } });
  const facts = root.querySelector('[aria-label="Live facts"]');
  assert.match(facts.textContent, /Issues unavailable/);
  assert.match(facts.textContent, /needs_human: Unknown/);
  assert.match(facts.textContent, /Release: Unknown/);
});

// SPEC #5016 HSF (AC-HS1/HS2): Live facts shows the shared needs_human
// value the status strip reads, never its own count of the raw facts list.
test('Live facts needs_human reads the shared attention count, not the facts field', () => {
  const { root, view, update } = fixture();
  update([], null, { status: null, facts: { source: 'daemon', active_windows: [], queue_head: null,
    needs_human_count: 5, release: null, release_observed_at: null, pending_update: null, error: null } });
  const facts = () => root.querySelector('[aria-label="Live facts"]').textContent;
  assert.match(facts(), /needs_human: Unknown/, 'the raw facts count is not a second definition');
  view.receiveHumanAttention(1);
  assert.match(facts(), /needs_human: 1/);
  view.receiveHumanAttention(null);
  assert.match(facts(), /needs_human: Unknown/);
});

test('Reports renders shared Markdown, metadata and copy, with no input or submit surface', async () => {
  const { root, update, copied } = fixture();
  update([report()]);
  assert.equal(root.querySelector('[role="log"]').getAttribute('aria-label'), 'PM reports');
  assert.equal(root.querySelector('.pm-chat__transcript h1').textContent, 'Update');
  assert.equal(root.querySelector('article strong').textContent, 'Ready');
  assert.equal(root.querySelector('li').textContent, 'checked');
  assert.equal(root.querySelector('pre code').textContent, 'cargo test');
  assert.equal(root.querySelector('article time').getAttribute('datetime'), report().created_at);
  assert.match(root.textContent, /progress/);
  // Reports remain read-only; Chat and the selected Task cancellation accept input.
  const editable = 'input, textarea, form, [contenteditable], button[type="submit"]';
  assert.equal(root.querySelectorAll('.pm-chat__body ' + editable.split(', ').join(', .pm-chat__body ')).length, 0);
  assert.equal([...root.querySelectorAll(editable)].filter(node => !node.closest('.pm-chat__composer, .pm-chat__task-cancel')).length, 0);
  assert.equal(root.querySelectorAll('.pm-chat__composer').length, 1);
  root.querySelector('[aria-label="Copy report"]').click();
  await Promise.resolve();
  assert.deepEqual(copied, [report().body]);
});

test('Q&A separates safe question text from the Markdown answer and copies both', async () => {
  const { root, update, copied } = fixture();
  const qa = { ...report(), kind: 'qa', question: 'Why <img src=x onerror=alert(1)>?\nWhat next?' };
  update([qa]);
  const question = root.querySelector('section[aria-label="Question"]');
  const answer = root.querySelector('section[aria-label="Answer"]');
  assert.ok(question, 'Q&A must have a distinct question section');
  assert.equal(question.querySelector('h2').textContent, 'Q');
  assert.equal(question.querySelector('.pm-chat__question').textContent, qa.question);
  assert.equal(question.querySelector('img'), null);
  assert.equal(answer.querySelector('h2').textContent, 'A');
  assert.equal(answer.querySelector('.knowledge-markdown-body strong').textContent, 'Ready');
  root.querySelector('[aria-label="Copy report"]').click();
  await Promise.resolve();
  assert.deepEqual(copied, [`Q: ${qa.question}\n\nA:\n${qa.body}`]);
});

test('decision Reports display and copy numbered choices without changing the stored body', async () => {
  const { root, update, copied, model } = fixture();
  const incoming = structuredReport('decision');
  incoming.kind = 'decision';
  incoming.structured.summary.decision = { question: 'Which recovery?', recommendation: 'Retry',
    reason: 'The work is saved', options: ['Retry — run again', 'Wait — keep state',
      'Inspect <logs> — gather evidence', 'Cancel — stop'] };
  const expected = 'Ready\nNext: Check delivery\nDecision: Which recovery?\nRecommended: Retry (The work is saved)\n1. Retry — run again\n2. Wait — keep state\n3. Inspect <logs> — gather evidence\n4. Cancel — stop\nReply with the option number, or Other: your own answer.';
  update([incoming]);
  const article = root.querySelector('article');
  assert.equal(article.querySelector('.pm-chat__text').textContent, expected);
  assert.equal(article.querySelector('logs'), null, 'choices remain literal text');
  article.querySelector('[aria-label="Copy report"]').click();
  await Promise.resolve();
  assert.deepEqual(copied, [`${expected}\n\n**All checks passed.**\n\n- https://example.com/issues/3431`]);
  assert.equal(model.readPmWindowState().windows.pm.reports[0].body, incoming.body);
});

test('structured Reports show a compact overview and copy all expandable details and references', async () => {
  const { root, update, copied } = fixture();
  const incoming = structuredReport();
  const complete = `${'詳細'.repeat(9000)}\n\nTAIL: item 48 remains available`;
  incoming.structured.details = complete;
  incoming.details_html = `<p>${complete}</p><p><strong>Complete evidence</strong></p><ul><li><a href="${incoming.structured.refs[0]}">Issue evidence</a></li></ul>`;
  update([incoming]);
  const article = root.querySelector('article');
  const details = article.querySelector('details');
  assert.ok(details, 'structured details use native disclosure');
  assert.equal(details.hasAttribute('open'), false);
  assert.equal(details.querySelector('summary').textContent, 'Details');
  const overview = article.querySelector(':scope > .pm-chat__text');
  assert.equal(overview.textContent, incoming.body, 'typed overview preserves literal text and logical lines');
  assert.ok(overview.classList.contains('is-plaintext'));
  assert.equal(overview.querySelector('literal'), null);
  assert.ok(details.querySelector('.knowledge-markdown-body'), 'details use the common Markdown renderer');
  assert.ok(details.textContent.includes(complete), 'all 18k characters and the final item survive');
  assert.equal(details.querySelector('strong').textContent, 'Complete evidence');
  assert.equal(details.querySelector('a').getAttribute('href'), incoming.structured.refs[0]);
  article.querySelector('[aria-label="Copy report"]').click();
  await Promise.resolve();
  assert.deepEqual(copied, [`${incoming.body}\n\n${complete}\n\n- ${incoming.structured.refs[0]}`]);
});

test('structured details have a safe text fallback while legacy, peer and Q&A retain full content', () => {
  const { root, update } = fixture();
  const incoming = structuredReport();
  delete incoming.details_html;
  incoming.structured.details = '<script>literal detail</script>\nLast detail';
  const legacy = report('legacy');
  const peer = { ...report('pm-peer-one'), body: 'Complete peer text', body_html: '<p>Complete peer text</p>' };
  const qa = { ...report('qa'), kind: 'qa', question: 'Explain every result.' };
  update([incoming, legacy, peer, qa]);
  const articles = Array.from(root.querySelectorAll('article'));
  const details = articles[0].querySelector('details');
  assert.ok(details, 'typed report retains its disclosure without HTML');
  assert.equal(details.querySelector('script'), null);
  assert.equal(details.querySelector('.is-plaintext').textContent,
    `${incoming.structured.details}\n\n- ${incoming.structured.refs[0]}`);
  assert.equal(root.querySelectorAll('article details').length, 1, 'legacy sources do not gain a disclosure');
  assert.match(articles[1].textContent, /Update.*Ready.*checked.*cargo test/s);
  assert.match(articles[2].textContent, /Complete peer text/);
  assert.match(articles[3].querySelector('[aria-label="Answer"]').textContent, /Update.*Ready.*checked.*cargo test/s);
});

test('structured summary-only reports omit empty Details while refs-only reports retain disclosure and raw Copy', async () => {
  const { root, update, copied } = fixture();
  const empty = { ...structuredReport('empty'), structured: { summary: { result: 'Ready' }, details: '', refs: [] }, details_html: '' };
  const refsOnly = { ...structuredReport('refs-only'), structured: { summary: { result: 'Ready' }, details: '',
    refs: ['https://example.com/issues/3431'] }, details_html: '<ul><li><a href="https://example.com/issues/3431">https://example.com/issues/3431</a></li></ul>' };
  update([empty, refsOnly]);
  assert.equal(root.querySelector('article[data-report-id="empty"] details') === null, true);
  empty.structured.details = ' \n ';
  update([empty, refsOnly]);
  const emptyRow = root.querySelector('article[data-report-id="empty"]');
  const refsRow = root.querySelector('article[data-report-id="refs-only"]');
  assert.equal(emptyRow.querySelector('details') === null, true, 'whitespace has no detail content to expand');
  assert.equal(emptyRow.querySelector('.pm-chat__text').textContent, empty.body);
  assert.ok(refsRow.querySelector('details'), 'references alone remain expandable');
  assert.equal(refsRow.querySelector('a').getAttribute('href'), refsOnly.structured.refs[0]);
  refsRow.querySelector('[aria-label="Copy report"]').click();
  await Promise.resolve();
  assert.deepEqual(copied, [`${refsOnly.body}\n\n- ${refsOnly.structured.refs[0]}`]);
});

test('structured disclosure follows report identity across polling and filters without moving a history reader', () => {
  const { root, model, update } = fixture();
  const body = root.querySelector('.pm-chat__body');
  Object.defineProperties(body, { clientHeight: { value: 100 }, scrollHeight: { value: 2000 } });
  const first = structuredReport('first');
  const second = { ...structuredReport('second'), kind: 'decision' };
  update([first]);
  const opened = root.querySelector('article details');
  assert.ok(opened, 'typed report has a disclosure to open');
  opened.setAttribute('open', '');
  body.scrollTop = 120;
  update([first, second]);
  let details = root.querySelectorAll('article details');
  assert.equal(details[0].hasAttribute('open'), true, 'a new report does not close an expanded report');
  assert.equal(details[1].hasAttribute('open'), false, 'a different report starts collapsed');
  assert.equal(body.scrollTop, 120);
  const select = root.querySelector('select');
  const choose = kind => {
    select.querySelector(`option[value="${kind}"]`).selected = true;
    select.dispatchEvent(new root.ownerDocument.defaultView.Event('change'));
  };
  choose('decision');
  choose('all');
  details = root.querySelectorAll('article details');
  assert.equal(details[0].hasAttribute('open'), true, 'filtering retains the state by report id');
  details[0].removeAttribute('open');
  update([second, first]);
  assert.equal(Array.from(root.querySelectorAll('article details')).some(node => node.hasAttribute('open')), false,
    'closing a report survives reordering and polling');
  root.querySelector('article details').setAttribute('open', '');
  model.bindPmWindowState('pm', 'replacement');
  update([second]);
  assert.equal(root.querySelector('article details').hasAttribute('open'), false, 'a new session resets local disclosure state');
});

test('report kind filter retains its selection on polling and hides with the execution log', () => {
  const { root, update } = fixture();
  const history = [report(), { ...report('qa'), kind: 'qa', question: 'Why?' }, { ...report('decision'), kind: 'decision' }];
  update(history);
  const select = root.querySelector('select');
  assert.ok(select, 'Reports must offer a report kind selector');
  assert.match(select.closest('label').textContent, /^Report kind/);
  assert.deepEqual(Array.from(select.options, option => [option.value, option.textContent]), [
    ['all', 'All reports'], ['qa', 'Q&A'], ['progress', 'Progress'], ['decision', 'Decision'], ['blocker', 'Blocker'],
  ]);
  const choose = kind => {
    select.querySelector(`option[value="${kind}"]`).selected = true;
    select.dispatchEvent(new root.ownerDocument.defaultView.Event('change'));
  };
  choose('qa');
  assert.equal(root.querySelectorAll('article').length, 1);
  assert.ok(root.querySelector('.pm-chat__question'));
  update([...history, { ...report('follow-up'), kind: 'qa', question: 'What next?' }]);
  assert.equal(select.value, 'qa');
  assert.equal(root.querySelectorAll('article').length, 2);
  for (const kind of ['progress', 'decision']) {
    choose(kind);
    assert.equal(root.querySelectorAll('article').length, 1);
    assert.equal(root.querySelector('.pm-chat__author span').textContent, kind);
  }
  choose('blocker');
  assert.equal(root.querySelectorAll('article').length, 0);
  assert.match(root.querySelector('.pm-chat__body [role="status"]').textContent, /No Blocker reports yet/);
  [...root.querySelectorAll('.pm-chat__toolbar button')].find(button => button.textContent === 'Execution log').click();
  assert.equal(select.closest('.pm-chat__body').hidden, true);
  [...root.querySelectorAll('.pm-chat__toolbar button')].find(button => button.textContent === 'Reports').click();
  assert.equal(select.value, 'blocker');
  assert.equal(select.closest('.pm-chat__body').hidden, false);
  choose('all');
  assert.equal(root.querySelectorAll('article').length, 4);
});

test('Reports starts at the tail, follows new reports and preserves history across hidden updates (Issue #6235)', () => {
  const { root, update } = fixture();
  const body = root.querySelector('.pm-chat__body');
  const height = () => 500 + 100 * root.querySelectorAll('article').length;
  let top = 0;
  Object.defineProperties(body, {
    clientHeight: { get: () => body.hidden ? 0 : 100 },
    scrollHeight: { get: () => body.hidden ? 0 : height() },
    scrollTop: {
      get: () => body.hidden ? 0 : top,
      set: value => { top = Math.max(0, Math.min(value, body.hidden ? 0 : height() - 100)); },
    },
  });
  const history = [report()];
  update(history);
  assert.equal(body.scrollTop, 500, 'initial reports open at the latest content');
  update([...history, report('two')]);
  assert.equal(body.scrollTop, 600, 'new reports follow the tail');
  body.scrollTop = 100;
  body.dispatchEvent(new root.ownerDocument.defaultView.Event('scroll'));
  update([...history, report('two'), report('three')]);
  assert.equal(body.scrollTop, 100, 'new reports do not move a reader in history');
  [...root.querySelectorAll('.pm-chat__toolbar button')].find(button => button.textContent === 'Execution log').click();
  update([...history, report('four')]);
  [...root.querySelectorAll('.pm-chat__toolbar button')].find(button => button.textContent === 'Reports').click();
  assert.equal(body.scrollTop, 100, 'hidden zero-size updates must not overwrite the saved position');
});

test('report updates preserve nodes and scroll, append history, and retain history on read failure', () => {
  const { root, model, update } = fixture();
  update([report()]);
  const first = root.querySelector('article');
  const transcript = root.querySelector('.pm-chat__body');
  transcript.scrollTop = 20;
  update([report()]);
  assert.ok(root.querySelector('article') === first);
  assert.equal(transcript.scrollTop, 20);
  update([report(), report('two')]);
  assert.equal(root.querySelectorAll('article').length, 2);
  // A newer authoritative snapshot can remove history; ordering uses revision, not length.
  update([report()]);
  assert.equal(root.querySelectorAll('article').length, 1);
  update([], 'Reports could not be loaded');
  assert.equal(root.querySelectorAll('article').length, 1);
  assert.match(root.querySelector('.pm-chat__body [role="status"]').textContent, /could not be loaded/);
  update([report(), report('two'), report('pm-peer-a'), report('pm-peer-b')], null,
    { observed_at: '2026-10-15T00:00:00Z' });
  update([report(), report('two'), { ...report('qa'), kind: 'qa', question: 'Can we continue?' }], null,
    { observed_at: '2026-10-15T00:01:00Z' });
  assert.deepEqual(model.readPmWindowState().windows.pm.reports.map(row => row.id), ['one', 'two', 'qa']);
  assert.equal(model.readPmWindowState().windows.pm.current.observed_at, '2026-10-15T00:01:00Z');
  assert.equal(root.querySelectorAll('article').length, 3);
});

test('Reports and raw terminal share one immutable version without replaying terminal packets', () => {
  const { model, update } = fixture();
  const versions = [], packets = [];
  let previousPacket;
  model.subscribePmWindowState(state => state.windows.pm, state => {
    versions.push(state.revision);
    if (state.terminal && state.terminal !== previousPacket) packets.push(state.terminal.dataBase64);
    previousPacket = state.terminal;
  });
  model.applyPmWindowReceiveEvent({ kind: 'terminal_snapshot', id: 'pm', data_base64: 'snapshot' });
  update([report()]);
  model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'pm', data_base64: 'output' });
  assert.deepEqual(versions, [1, 2, 3, 4]);
  assert.deepEqual(packets, ['snapshot', 'output']);
  assert.ok(Object.isFrozen(model.readPmWindowState().windows.pm.reports));
});

test('Execution log toggle retains Reports; stale sessions and native transcript cannot replace reports', () => {
  const { root, model, view, update, visibility } = fixture();
  update([report()]);
  const buttons = root.querySelectorAll('.pm-chat__toolbar button');
  [...buttons].find(button => button.textContent === 'Execution log').click();
  assert.equal(visibility.at(-1), true);
  view.showReports();
  assert.equal(visibility.at(-1), false);
  assert.equal(root.querySelectorAll('article').length, 1);
  const previous = model.readPmWindowState().windows.pm;
  model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm', session_id: 'stale', reports: [] });
  model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm', session_id: 'session', reports: [] });
  assert.ok(model.readPmWindowState().windows.pm === previous);
  model.applyPmWindowReceiveEvent({ kind: 'pm_conversation', id: 'pm', session_id: 'session',
    snapshot: { availability: 'ready', messages: [{ id: 'm', role: 'assistant', text: 'Native transcript' }] } });
  assert.equal(model.readPmWindowState().windows.pm.reports, previous.reports);
  assert.doesNotMatch(root.querySelector('.pm-chat__transcript').textContent, /Native transcript/);
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(root.querySelectorAll('article').length, 0);
  model.removePmWindowState('pm');
  assert.equal(model.readPmWindowState().windows.pm, undefined);
});

test('Reports reuses the content renderer and Operator tokens without an overlay shell', async () => {
  const renderer = await readFile(new URL('../pm-chat.js', import.meta.url), 'utf8');
  assert.match(renderer, /renderUiContent\(document, report\.content/);
  assert.doesNotMatch(renderer, /Claude|Codex|OpenCode|agent_id|⏺|⎿|\\x1b/i);
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const section = css.slice(css.indexOf('/* PM conversation */'));
  assert.match(section, /var\(--color-/);
  assert.match(section, /\.pm-chat__filter\s*\{[^}]*var\(--space-/);
  assert.match(section, /\.pm-chat__filter select:focus-visible\s*\{[^}]*var\(--color-focus-ring\)/);
  assert.match(section, /\.pm-chat__question\s*\{[^}]*white-space:\s*pre-wrap/);
  assert.match(section, /\.pm-chat__text\.is-plaintext\s*\{[^}]*white-space:\s*pre-wrap/);
  assert.match(section, /\.pm-chat__section-label\s*\{[^}]*var\(--type-/);
  assert.doesNotMatch(section, /#[\da-f]{3,8}\b|rgba?\(|position:\s*(fixed|absolute)/i);
});


test('Reports rejects file-drop input while Execution log keeps the attachment bridge', async () => {
  let source = await readFile(new URL('../terminal-attachments.js', import.meta.url), 'utf8');
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu', 'ui-state-store']) {
    source = source.replace(`"/${name}.js"`, JSON.stringify(new URL(`../${name}.js`, import.meta.url).href));
  }
  const { createTerminalAttachments } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document, window } = parseHTML('<html><body><div class="workspace-window" data-id="pm"><div class="pm-chat">Report</div></div></body></html>');
  const previous = { window: globalThis.window, document: globalThis.document };
  const sent = [], uploaded = [];
  globalThis.window = window;
  globalThis.document = document;
  try {
    const pane = document.querySelector('.workspace-window');
    window.__gwtAttachmentUploader = async ({ file }) => { uploaded.push(file.name); return { upload_id: 'fixture' }; };
    const bridge = createTerminalAttachments({ send: event => { sent.push(event); return "sent"; },
      captureAttachmentTarget: () => ({ expected_target: { project_key: "project", session_id: "session", host_id: "host", runtime_incarnation: 1 }, connection_epoch: 1 }),
      terminalMap: new Map([['pm', { terminal: { focus() {} } }]]), windowMap: new Map([['pm', pane]]),
      workspaceWindowById: () => ({ preset: 'agent' }), isAgentWindowPreset: preset => preset === 'agent',
      workspaceWindowElement: () => pane });
    bridge.installBrowserFileDropBridge();
    const drop = async () => {
      const event = new window.Event('drop', { bubbles: true, cancelable: true });
      event.dataTransfer = { types: ['Files'], files: [{ name: 'input.txt', size: 4 }] };
      document.querySelector('.pm-chat').dispatchEvent(event);
      await new Promise(resolve => setImmediate(resolve));
    };
    await drop();
    assert.deepEqual(uploaded, []);
    assert.deepEqual(sent, []);
    document.querySelector('.pm-chat').classList.add('pm-chat--logs');
    await drop();
    assert.deepEqual(uploaded, ['input.txt']);
    assert.equal(sent[0]?.kind, 'attach_files');
  } finally {
    globalThis.window = previous.window;
    globalThis.document = previous.document;
  }
});

// Exercise the real token/XHR path: the uploader override bypasses both.
async function clipboardFixture(windowId, run) {
  let source = await readFile(new URL('../terminal-attachments.js', import.meta.url), 'utf8');
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu', 'ui-state-store']) {
    source = source.replace(`"/${name}.js"`, JSON.stringify(new URL(`../${name}.js`, import.meta.url).href));
  }
  const { createTerminalAttachments } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document, window } = parseHTML('<html><body><div class="workspace-window"><div class="terminal-root"></div></div></body></html>');
  const previous = Object.fromEntries(['window', 'document', 'fetch', 'XMLHttpRequest', '__gwtAttachmentUploader'].map(key => [key, globalThis[key]]));
  const sent = [], alerts = [];
  let tokenRequests = 0;
  globalThis.window = window;
  globalThis.document = document;
  window.__gwtAttachmentUploader = undefined;
  window.alert = message => alerts.push(message);
  globalThis.fetch = async () => {
    tokenRequests += 1;
    if (tokenRequests === 1) throw new Error('token connection interrupted');
    return { ok: true, json: async () => ({ token: 'fresh-token' }) };
  };
  globalThis.XMLHttpRequest = class {
    upload = {};
    status = 200;
    response = { upload_id: 'image-upload', mime_type: 'image/png', filename: 'clipboard-image.png', size: 3 };
    open() {}
    setRequestHeader() {}
    send() { this.onload(); }
  };
  try {
    const pane = document.querySelector('.workspace-window');
    pane.dataset.id = windowId;
    const bridge = createTerminalAttachments({ send: event => { sent.push(event); return "sent"; },
      captureAttachmentTarget: () => ({ expected_target: { project_key: "project", session_id: "session", host_id: "host", runtime_incarnation: 1 }, connection_epoch: 1 }),
      terminalMap: new Map(), windowMap: new Map(), workspaceWindowById: () => ({ preset: 'agent' }),
      isAgentWindowPreset: preset => preset === 'agent', workspaceWindowElement: () => pane });
    const terminalRoot = pane.querySelector('.terminal-root');
    bridge.installTerminalImagePasteHandlers(windowId, terminalRoot, { focus() {} });
    const paste = async () => {
      const event = new window.Event('paste', { bubbles: true, cancelable: true });
      const file = new File(['png'], 'clipboard-image.png', { type: 'image/png' });
      event.clipboardData = { items: [{ kind: 'file', type: file.type, getAsFile: () => file }] };
      terminalRoot.dispatchEvent(event);
      await new Promise(resolve => setImmediate(resolve));
    };
    await run({ paste, sent, alerts, pane, tokenRequests: () => tokenRequests });
  } finally {
    Object.assign(globalThis, previous);
  }
}

test('PM and Agent clipboard paste retry a failed token request without reloading', async () => {
  for (const windowId of ['pm', 'agent']) {
    await clipboardFixture(windowId, async ({ paste, sent, tokenRequests }) => {
      await paste();
      assert.equal(sent.filter(event => event.kind === 'paste_image_uploaded').length, 0);
      await paste();
      assert.equal(tokenRequests(), 2, `${windowId}: failed token must be fetched again`);
      assert.equal(sent.filter(event => event.kind === 'paste_image_uploaded').length, 1);
      assert.equal(sent.at(-1).id, windowId);
    });
  }
});

test('clipboard failure shows and reports the concrete upload cause', async () => {
  await clipboardFixture('pm', async ({ paste, sent, alerts, pane }) => {
    await paste();
    assert.match(alerts[0], /Could not upload pasted image.*token connection interrupted/);
    assert.match(pane.querySelector('.attachment-progress').textContent, /token connection interrupted/);
    assert.equal(sent[0].kind, 'attachment_upload_failed');
    assert.equal(sent[0].id, 'pm');
    assert.equal(sent[0].message, alerts[0]);
    assert.match(sent[0].operation_id, /^attachment-/);
  });
});


test('Works full terminals own browser and native file drops and their progress', async () => {
  let source = await readFile(new URL('../terminal-attachments.js', import.meta.url), 'utf8');
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu', 'ui-state-store']) {
    source = source.replace(`"/${name}.js"`, JSON.stringify(new URL(`../${name}.js`, import.meta.url).href));
  }
  const { createTerminalAttachments } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document, window } = parseHTML('<html><body><section hidden><div class="workspace-window" data-id="agent"></div></section><section class="works-surface"><section class="works-full"><header class="works-full-header">Agent</header><div class="works-terminal-host"><div class="works-terminal terminal-root" data-window-id="agent"></div></div></section></section></body></html>');
  const previous = { window: globalThis.window, document: globalThis.document };
  globalThis.window = window;
  globalThis.document = document;
  const sent = [], alerts = [];
  window.alert = message => alerts.push(message);
  window.__gwtAttachmentUploader = async () => ({ upload_id: 'tile-upload' });
  try {
    const cached = document.querySelector('.workspace-window');
    const tile = document.querySelector('.works-full');
    const terminalRoot = tile.querySelector('.terminal-root');
    document.elementFromPoint = () => terminalRoot;
    const bridge = createTerminalAttachments({ send: message => { sent.push(message); return "sent"; },
      captureAttachmentTarget: () => ({ expected_target: { project_key: "project", session_id: "session", host_id: "host", runtime_incarnation: 1 }, connection_epoch: 1 }),
      terminalMap: new Map([['agent', { terminalContainer: terminalRoot, terminal: { focus() {} } }]]),
      windowMap: new Map([['agent', cached]]), workspaceWindowById: () => ({ preset: 'agent' }),
      isAgentWindowPreset: preset => preset === 'agent', workspaceWindowElement: () => cached });
    bridge.installBrowserFileDropBridge();
    bridge.installNativeFileDropBridge();
    const drop = new window.Event('drop', { bubbles: true, cancelable: true });
    drop.dataTransfer = { types: ['Files'], files: [{ name: 'input.txt', size: 4 }] };
    tile.querySelector('header').dispatchEvent(drop);
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(sent.map(message => message.kind), ['attach_files']);
    assert.equal(sent[0].id, 'agent');
    assert.equal(tile.querySelector('.attachment-progress').hidden, false);
    assert.equal(cached.querySelector('.attachment-progress'), null);
    window.dispatchEvent(new window.CustomEvent('gwt:native-file-drop', {
      detail: { paths: ['C:/input.txt'], x: 10, y: 20 },
    }));
    assert.equal(sent[1].id, 'agent');
    assert.equal(sent[1].files[0].source, 'native_path');
    assert.deepEqual(alerts, []);
  } finally {
    globalThis.window = previous.window;
    globalThis.document = previous.document;
  }
});

async function attachmentLifecycleFixture(run) {
  let source = await readFile(new URL('../terminal-attachments.js', import.meta.url), 'utf8');
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu', 'ui-state-store']) {
    source = source.replace(`"/${name}.js"`, JSON.stringify(new URL(`../${name}.js`, import.meta.url).href));
  }
  const { createTerminalAttachments } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document, window } = parseHTML('<html><body><div class="workspace-window" data-id="agent"><div class="terminal-root" data-window-id="agent"></div></div></body></html>');
  const previous = { window: globalThis.window, document: globalThis.document, setTimeout: globalThis.setTimeout };
  const navigatorDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const clipboardCleanups = [];
  Object.assign(globalThis, { window, document });
  const pane = document.querySelector('.workspace-window'), root = pane.firstElementChild;
  const sent = [], notices = [], timers = [], uploads = [], dismissed = [];
  let context = { expected_target: { project_key: 'project', session_id: 'session', host_id: 'host', runtime_incarnation: 1 }, connection_epoch: 1 };
  let sendResult = 'sent', finishUpload = null;
  globalThis.setTimeout = (callback, delay, ...args) => {
    if (delay !== 700) return previous.setTimeout(callback, delay, ...args);
    timers.push(callback); return timers.length;
  };
  window.alert = () => {};
  window.__gwtAttachmentUploader = ({ file }) => {
    uploads.push(file.name);
    return new Promise(resolve => { finishUpload = () => resolve({ upload_id: 'upload', filename: file.name, size: file.size }); });
  };
  const terminalMap = new Map([['agent', { terminalContainer: root, terminal: { focus() {} } }]]);
  const bridge = createTerminalAttachments({ send: message => { sent.push(message); return sendResult; }, terminalMap,
    windowMap: new Map([['agent', pane]]), workspaceWindowById: () => ({ preset: 'agent' }),
    isAgentWindowPreset: preset => preset === 'agent', workspaceWindowElement: () => pane,
    captureAttachmentTarget: () => context, reportAttachmentNotice: notice => notices.push(notice), dismissAttachmentNotice: (...args) => dismissed.push(args) });
  bridge.installBrowserFileDropBridge();
  bridge.installTerminalFileDropHandlers('agent', root, { focus() {} });
  bridge.installTerminalImagePasteHandlers('agent', root, { focus() {} });
  const flush = () => new Promise(resolve => setImmediate(resolve));
  const drop = async () => {
    const event = new window.Event('drop', { bubbles: true, cancelable: true });
    event.dataTransfer = { types: ['Files'], files: [{ name: 'input.txt', size: 4 }] };
    root.dispatchEvent(event);
    await flush();
  };
  const paste = async () => {
    const event = new window.Event('paste', { bubbles: true, cancelable: true });
    const file = new File(['png'], 'clipboard-image.png', { type: 'image/png' });
    event.clipboardData = { items: [{ kind: 'file', type: file.type, getAsFile: () => file }] };
    root.dispatchEvent(event);
    await flush();
  };
  const beginContextPaste = async (stage = 'read') => {
    let release;
    const gate = new Promise(resolve => { release = resolve; });
    const image = new Blob(['png'], { type: 'image/png' });
    const item = { types: ['image/png'], getType: () => stage === 'getType' ? gate : Promise.resolve(image) };
    Object.defineProperty(globalThis, 'navigator', { configurable: true, value: { clipboard: {
      read: () => stage === 'read' ? gate : Promise.resolve([item]), readText: async () => '',
    } } });
    const cleanup = bridge.installTerminalContextMenuHandlers('agent', root, { hasSelection: () => false, focus() {}, paste() {} });
    clipboardCleanups.push(cleanup);
    [...document.querySelectorAll('.terminal-context-menu button')].find(button => button.textContent === 'Paste')
      .dispatchEvent(new window.Event('click', { bubbles: true, cancelable: true }));
    await flush();
    return { cleanup, release: async () => { release(stage === 'read' ? [item] : image); await flush(); await flush(); } };
  };
  const finish = async () => { finishUpload(); await flush(); };
  const receipt = (phase, extra = {}) => bridge.handleAttachmentProgress({ id: 'agent', operation_id: sent.at(-1)?.operation_id,
    expected_target: sent.at(-1)?.expected_target, phase, ...extra });
  try { await run({ bridge, root, pane, terminalMap, sent, notices, timers, uploads, dismissed, drop, paste, beginContextPaste, finish, receipt,
    target: () => context, replace: value => { context = value; }, sendResult: value => { sendResult = value; } }); }
  finally {
    for (const cleanup of clipboardCleanups) cleanup();
    if (navigatorDescriptor) Object.defineProperty(globalThis, 'navigator', navigatorDescriptor); else delete globalThis.navigator;
    Object.assign(globalThis, previous);
  }
}

test('attachment captures target before upload and refuses a replacement without rebinding', async () => {
  await attachmentLifecycleFixture(async f => {
    await f.drop();
    f.replace({ ...f.target(), expected_target: { ...f.target().expected_target, session_id: 'replacement' } });
    const replacement = document.createElement('div'); replacement.dataset.windowId = 'agent'; document.body.appendChild(replacement);
    f.terminalMap.get('agent').terminalContainer = replacement;
    await f.finish();
    assert.equal(replacement.querySelector('.attachment-progress') === null, true, 'retirement must not write into the replacement host');
    assert.equal(f.sent.length, 0);
    assert.match(f.notices.at(-1).message, /target.*changed/i);
    assert.equal(f.root.querySelector(".attachment-progress").hidden, true);
    assert.equal(f.uploads.length, 1);
  });
});

test('attachment send outcomes distinguish unavailable, ambiguous, and submitted disconnect without retry', async () => {
  for (const [result, label] of [['unavailable', /not sent/i], ['outcome_unknown', /Outcome unknown/], ['sent', /Queued/]]) {
    await attachmentLifecycleFixture(async f => {
      f.sendResult(result); await f.drop(); await f.finish();
      assert.match(f.root.textContent, label);
      assert.deepEqual(f.sent[0].expected_target, f.target().expected_target);
      if (result === 'sent') {
        f.bridge.handleAttachmentTransportChange(false);
        assert.match(f.root.textContent, /Outcome unknown/);
        f.bridge.handleAttachmentTransportChange(true);
        f.receipt('attached');
        assert.match(f.root.textContent, /Outcome unknown/);
      }
      assert.equal(f.sent.length, 1);
    });
  }
});

test('attachment accepts only known exact receipts and keeps terminal outcomes monotonic', async () => {
  await attachmentLifecycleFixture(async f => {
    f.bridge.handleAttachmentProgress({ id: 'missing', operation_id: 'foreign', phase: 'attached' });
    assert.equal(f.pane.querySelector('.attachment-progress') === null, true);
    assert.equal(document.body.querySelector('.attachment-progress') === null, true);
    await f.drop(); await f.finish();
    f.receipt('attached', { expected_target: { ...f.target().expected_target, runtime_incarnation: 2 } });
    assert.match(f.root.textContent, /Queued/);
    f.receipt('attached'); assert.match(f.root.textContent, /Attached/);
    f.receipt('queued'); assert.match(f.root.textContent, /Attached/);
    f.timers.at(-1)();
    assert.equal(f.root.querySelector('.attachment-progress').hidden, true);
    f.receipt('queued');
    assert.equal(f.root.querySelector('.attachment-progress').hidden, true);
    assert.equal(f.sent.length, 1);
  });
});

test('old attachment timers and receipts cannot overwrite a new operation or its moved host', async () => {
  await attachmentLifecycleFixture(async f => {
    await f.drop(); await f.finish();
    const old = f.sent[0]; f.receipt('attached'); const timer = f.timers.at(-1);
    await f.drop(); await f.finish();
    f.receipt('attached');
    const secondTimer = f.timers.at(-1);
    timer(); assert.equal(f.root.querySelector('.attachment-progress').hidden, false);
    const moved = document.createElement('div'); moved.dataset.windowId = 'agent'; document.body.appendChild(moved);
    f.terminalMap.get('agent').terminalContainer = moved;
    f.bridge.reconcileAttachmentProgress();
    assert.match(moved.textContent, /Attached/);
    f.bridge.handleAttachmentProgress({ ...old, phase: 'failed', message: 'old failure' });
    assert.doesNotMatch(moved.textContent, /old failure/);
    const movedTimer = f.timers.at(-1);
    secondTimer(); assert.equal(moved.querySelector('.attachment-progress').hidden, false, 'old host timer cannot hide moved host');
    movedTimer(); assert.equal(moved.querySelector('.attachment-progress').hidden, true);
  });
});

test('missing old-backend metadata and target loss show persistent honest Unknown', async () => {
  await attachmentLifecycleFixture(async f => {
    await f.drop(); await f.finish();
    f.receipt('attached', { expected_target: undefined });
    assert.match(f.root.textContent, /Outcome unknown/);
    f.receipt('queued'); assert.match(f.root.textContent, /Outcome unknown/);
    f.replace(null); f.bridge.reconcileAttachmentProgress();
    assert.equal(f.root.querySelector('.attachment-progress')?.hidden ?? true, true);
    assert.match(f.notices.at(-1).message, /Outcome unknown/);
    assert.equal(f.sent.length, 1);
  });
});

test('app attachment socket adapter never queues or replays a refused or ambiguous send', async () => {
  const source = await readFile(new URL('../app.js', import.meta.url), 'utf8');
  const definition = source.slice(source.indexOf('      function sendOnSocket('), source.indexOf('      function sendKnowledgeSemanticSearchNow('));
  const pending = [], hubPending = [], writes = [];
  const socket = { readyState: 1, send: raw => writes.push(JSON.parse(raw)) };
  const send = new Function('activeProjectKey', 'socketProjectKey', 'WebSocket', 'isHubNavigationMessage',
    'hubSocket', 'pendingHubMessages', 'pendingMessages', 'connectionOverlay', `${definition}; return sendOnSocket;`)(
    () => 'project', 'project', { OPEN: 1 }, () => false, null, hubPending, pending, { reportInputDropped() {} });
  for (const kind of ['attach_files', 'paste_image_uploaded']) {
    const message = { kind, id: 'agent', operation_id: 'operation', expected_target: { project_key: 'project' } };
    assert.equal(send(message, null), 'unavailable');
    assert.equal(send({ ...message, expected_target: { project_key: 'foreign' } }, socket), 'unavailable');
    assert.equal(send(message, socket), 'sent');
    assert.equal(send(message, { readyState: 1, send() { throw new Error('close during send'); } }), 'outcome_unknown');
  }
  assert.equal(writes.length, 2);
  assert.deepEqual(pending, []);
  assert.deepEqual(hubPending, []);
});

test('unknown census prevents upload and an old epoch cannot settle a submitted attachment', async () => {
  await attachmentLifecycleFixture(async f => {
    f.replace(null); await f.drop();
    assert.equal(f.uploads.length, 0);
    assert.equal(f.sent.length, 0);
    assert.match(f.root.textContent, /not sent/i);
  });
  await attachmentLifecycleFixture(async f => {
    await f.drop(); await f.finish();
    f.replace({ ...f.target(), connection_epoch: 2 });
    f.receipt('attached');
    assert.equal(f.root.querySelector('.attachment-progress').hidden, true);
    assert.match(f.notices.at(-1).message, /Outcome unknown/);
    assert.equal(f.sent.length, 1);
  });
});

test('same-target temporary unmount retains terminal reason and Failed never auto-hides', async () => {
  await attachmentLifecycleFixture(async f => {
    await f.drop(); await f.finish();
    f.receipt('failed', { message: 'No input was attempted.' });
    f.receipt('injecting'); assert.match(f.root.textContent, /No input was attempted/);
    assert.equal(f.timers.length, 0);
    f.root.remove(); f.bridge.reconcileAttachmentProgress();
    assert.equal(f.pane.querySelector('.attachment-progress') === null, true, 'detached native host must not fall back to cached chrome');
    f.pane.appendChild(f.root); f.bridge.reconcileAttachmentProgress();
    f.pane.remove(); f.bridge.reconcileAttachmentProgress();
    document.body.appendChild(f.pane); f.bridge.reconcileAttachmentProgress();
    assert.match(f.root.textContent, /No input was attempted/);
    assert.equal(f.root.querySelector('.attachment-progress').hidden, false);
    assert.equal(f.notices.length, 0);
  });
});

test('app captures attachment authority from known census and matching catalog Session only', async () => {
  const source = await readFile(new URL('../app.js', import.meta.url), 'utf8');
  const start = source.indexOf('      function captureAttachmentTarget(');
  const definition = source.slice(start, source.indexOf('      const {', start));
  const row = { window_id: 'agent', session_id: 'session', runtime_incarnation: 3 };
  let activity = { count: 1, snapshot: { project_key: 'project', host_id: 'host' }, agents: [row] };
  let data = { preset: 'agent', session_id: 'session' };
  const capture = new Function('agentActivityModel', 'currentNativeWindow', 'activeProjectKey', 'routeProjectKey',
    'isAgentWindowPreset', 'socketReceiveDispatcherGeneration', `${definition}; return captureAttachmentTarget;`)(
    { read: () => activity }, () => data, () => 'project', 'project', preset => preset === 'agent', 7);
  assert.deepEqual(capture('agent'), { expected_target: { project_key: 'project', session_id: 'session', host_id: 'host', runtime_incarnation: 3 }, connection_epoch: 7 });
  activity = { ...activity, count: null }; assert.equal(capture('agent'), null);
  activity = { ...activity, count: 1 }; data = { ...data, session_id: 'replacement' }; assert.equal(capture('agent'), null);
  data = { ...data, session_id: 'session' }; activity = { ...activity, snapshot: { ...activity.snapshot, host_id: '' } };
  assert.equal(capture('agent'), null);
});


test('explicit paste with unavailable target and removed host shows one Failed notice without upload or send', async () => {
  await attachmentLifecycleFixture(async f => {
    f.replace(null);
    f.pane.remove();
    await f.paste();
    assert.equal(f.notices.length, 1);
    assert.match(f.notices[0].message, /target unavailable.*not sent/i);
    assert.equal(f.uploads.length, 0);
    assert.equal(f.sent.length, 0);
    assert.equal(document.body.querySelector('.attachment-progress') === null, true);
    f.bridge.reconcileAttachmentProgress();
    f.bridge.handleAttachmentProgress({ id: 'agent', operation_id: 'unknown', phase: 'attached' });
    assert.equal(f.notices.length, 1);
    assert.equal(document.body.querySelector('.attachment-progress') === null, true);
  });
});


test('clipboard intent cannot adopt a replacement or retire its existing operation after an await', async () => {
  for (const initialMissing of [false, true]) {
    await attachmentLifecycleFixture(async f => {
      const original = f.target();
      if (initialMissing) f.replace(null);
      const paste = await f.beginContextPaste();
      f.replace({ ...original, expected_target: { ...original.expected_target, session_id: 'replacement' } });
      await f.drop(); await f.finish();
      const replacement = f.sent[0], dismissed = f.dismissed.length;
      assert.match(f.root.textContent, /Queued/);
      await paste.release();
      assert.equal(f.uploads.length, 1, 'old intent must not upload against replacement');
      assert.equal(f.sent.length, 1);
      assert.equal(f.dismissed.length, dismissed, 'old intent must not dismiss replacement notice');
      assert.equal(f.notices.length, 1);
      assert.match(f.notices[0].message, /not sent/i);
      assert.match(f.root.textContent, /Queued/);
      f.receipt('attached', { operation_id: replacement.operation_id });
      assert.match(f.root.textContent, /Attached/, 'replacement controller must still accept its own receipt');
    });
  }
});

test('clipboard getType intent rejects changed incarnation or connection epoch before upload', async () => {
  for (const change of ['incarnation', 'epoch']) {
    await attachmentLifecycleFixture(async f => {
      const original = f.target(), paste = await f.beginContextPaste('getType');
      f.replace(change === 'epoch' ? { ...original, connection_epoch: 2 }
        : { ...original, expected_target: { ...original.expected_target, runtime_incarnation: 2 } });
      await paste.release();
      assert.equal(f.uploads.length, 0);
      assert.equal(f.sent.length, 0);
      assert.equal(f.notices.length, 1);
      assert.match(f.notices[0].message, /not sent/i);
      assert.equal(f.root.querySelector('.attachment-progress') === null, true);
    });
  }
});

test('unchanged clipboard intent survives valid host move and old binding disposal', async () => {
  await attachmentLifecycleFixture(async f => {
    const original = structuredClone(f.target()), paste = await f.beginContextPaste();
    paste.cleanup();
    const moved = document.createElement('div'); moved.dataset.windowId = 'agent'; f.pane.appendChild(moved);
    f.terminalMap.get('agent').terminalContainer = moved;
    await paste.release(); await f.finish();
    assert.equal(f.uploads.length, 1);
    assert.equal(f.sent.length, 1);
    assert.deepEqual(f.sent[0].expected_target, original.expected_target);
    assert.equal(f.notices.length, 0);
    assert.match(moved.textContent, /Queued/);
  });
});

test('context-menu optional beforePaste runs once before clipboard awaits and preserves no-hook callback arity', async () => {
  const { createTerminalContextMenuController } = await import('../terminal-context-menu.js');
  for (const withHook of [false, true]) {
    const { document, window } = parseHTML('<html><body><div id="terminal"></div></body></html>');
    const sequence = [], context = { captured: null };
    let args;
    const controller = createTerminalContextMenuController({ document, window, terminalRoot: document.getElementById('terminal'),
      ...(withHook ? { beforePaste: () => { sequence.push('capture'); return context; } } : {}),
      readClipboardItems: async () => { sequence.push('read'); return [{ types: ['image/png'], getType: async () => { sequence.push('getType'); return new Blob(['png']); } }]; },
      pasteImage: (...values) => { sequence.push('paste'); args = values; } });
    try {
      await controller.pasteFromClipboard();
      assert.deepEqual(sequence, withHook ? ['capture', 'read', 'getType', 'paste'] : ['read', 'getType', 'paste']);
      assert.equal(args.length, withHook ? 2 : 1);
      if (withHook) assert.equal(args[1], context);
    } finally { controller.dispose(); }
  }
});

const conversation = (messages, extra = {}) => ({ kind: 'pm_conversation', id: 'pm', session_id: 'session',
  snapshot: { conversation_id: 'c', availability: 'ready', detail: null, messages, ...extra } });

test('Chat shares parent/child Tasks and returns only to their exact human source', () => {
  const { root, model, view, update } = fixture();
  const tasks = [trackedTask('parent', { source_key: 'source' }),
    trackedTask('child', { parent_key: 'parent', source_key: 'source' })];
  update([], null, { request_tasks: taskOverview(tasks, 0) });
  view.showChat();
  const preview = root.querySelector('.pm-chat__tasks');
  const links = () => [...preview.querySelectorAll('button')].filter(node => node.textContent === 'Show in conversation');
  assert.equal(preview.hidden, false);
  assert.equal(links().length, 2);
  assert.ok(links().every(button => button.disabled), 'a missing conversation never supplies a guessed target');
  assert.match(preview.textContent, /original message.*not.*current conversation preview/i);
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'Keep this unfinished request';
  model.applyPmWindowReceiveEvent(conversation([
    { id: 'same-text', role: 'user', text: 'Same request', source_key: 'foreign' },
    { id: 'original', role: 'user', text: 'Same request', source_key: 'source' },
    { id: 'answer', role: 'assistant', text: 'Same request', source_key: 'source' },
  ]));
  assert.ok(links().every(button => !button.disabled), 'late conversation arrival updates both parent and child');
  const original = root.querySelector('[data-message-id="original"]');
  const navigation = [];
  original.scrollIntoView = () => navigation.push('scroll');
  original.focus = () => navigation.push('focus');
  view.showTasks();
  links()[1].click();
  assert.equal(view.isChatVisible(), true);
  assert.deepEqual(navigation, ['scroll', 'focus']);
  assert.equal(original.getAttribute('tabindex'), '-1');
  assert.equal(draft.value, 'Keep this unfinished request');
  assert.equal(preview.querySelectorAll('.pm-chat__task').length, 2);
  model.applyPmWindowReceiveEvent(conversation([
    { id: 'answer', role: 'assistant', text: 'Same request', source_key: 'source' },
  ], { conversation_id: 'replacement' }));
  assert.ok(links().every(button => button.disabled), 'a rotated conversation and assistant source cannot match');
  links()[0].click();
  assert.deepEqual(navigation, ['scroll', 'focus'], 'a stale button cannot navigate to the old bubble');
  model.bindPmWindowState('pm', 'replacement-session');
  assert.equal(preview.querySelectorAll('.pm-chat__task').length, 0);
});

test('Chat shows the PM conversation as user-right and PM-left bubbles (AC-3431-CHAT-1)', () => {
  const { root, model, view, update } = fixture({ tasksDefault: true });
  update([report()]);
  assert.equal(root.querySelector('.pm-chat__chat').hidden, true, 'Home remains the default view');
  assert.deepEqual(model.requestPmConversation('pm'), { kind: 'load_pm_conversation', id: 'pm' });
  assert.equal(model.requestPmConversation('missing'), null);
  model.applyPmWindowReceiveEvent(conversation([
    { id: 'u1', role: 'user', text: 'Ship <b>it</b>?' },
    { id: 'a1', role: 'assistant', text: '# Plan', text_html: '<h1>Plan</h1><table><tr><td>1</td></tr></table><pre><code>cargo test</code></pre>' },
    { id: 'u2', role: 'user', text: 'Thanks' },
  ]));
  [...root.querySelectorAll('.pm-chat__toolbar button')].find(button => button.textContent === 'Chat').click();
  const chat = root.querySelector('.pm-chat__chat');
  assert.equal(chat.hidden, false);
  assert.equal(root.querySelector('.pm-chat__tasks').hidden, false, 'Chat shares the existing Tasks preview');
  const bubbles = [...chat.querySelectorAll('.pm-chat__bubble')];
  assert.deepEqual(bubbles.map(bubble => [bubble.dataset.messageId, bubble.dataset.role]),
    [['u1', 'user'], ['a1', 'assistant'], ['u2', 'user']]);
  assert.ok(bubbles[0].classList.contains('pm-chat__bubble--user'));
  assert.ok(bubbles[1].classList.contains('pm-chat__bubble--assistant'));
  assert.equal(bubbles[0].querySelector('b'), null, 'user text is never parsed as HTML');
  assert.match(bubbles[0].textContent, /Ship <b>it<\/b>\?/);
  assert.ok(bubbles[1].querySelector('h1') && bubbles[1].querySelector('table') && bubbles[1].querySelector('pre'));
  assert.equal(root.querySelectorAll('.pm-chat__transcript article').length, 1, 'Reports stay report-only');
  view.showReports();
  assert.equal(chat.hidden, true);
});

test('Chat collapses earlier PM messages of one turn behind an N messages disclosure (T-CHAT-COLLAPSE)', () => {
  const { root, model, view } = fixture();
  view.showChat();
  const chat = root.querySelector('.pm-chat__chat');
  const turn = [
    { id: 'u1', role: 'user', text: 'Status?' },
    { id: 'a1', role: 'assistant', text: 'Checking the queue.' },
    { id: 'a2', role: 'assistant', text: 'Reading CI.' },
    { id: 'a3', role: 'assistant', text: 'All green.' },
    { id: 'u2', role: 'user', text: 'Thanks' },
    { id: 'a4', role: 'assistant', text: 'You are welcome.' },
  ];
  model.applyPmWindowReceiveEvent(conversation(turn));
  const log = chat.querySelector('.pm-chat__conversation');
  const order = () => [...log.children].map(node => node.tagName === 'DETAILS'
    ? `group:${[...node.querySelectorAll('.pm-chat__bubble')].map(bubble => bubble.dataset.messageId).join(',')}`
    : node.dataset.messageId);
  assert.deepEqual(order(), ['u1', 'group:a1,a2', 'a3', 'u2', 'a4'], 'only the latest PM message of a turn stays open');
  const group = log.querySelector('details.pm-chat__collapsed');
  assert.equal(group.open, false);
  assert.equal(group.querySelector('summary').textContent, '2 earlier messages');
  group.open = true;
  model.applyPmWindowReceiveEvent(conversation([...turn.slice(0, 4), { id: 'a5', role: 'assistant', text: 'Merged.' }, ...turn.slice(4)]));
  assert.deepEqual(order(), ['u1', 'group:a1,a2,a3', 'a5', 'u2', 'a4']);
  const regrown = log.querySelector('details.pm-chat__collapsed');
  assert.equal(regrown.open, true, 'an expanded group stays expanded across snapshots');
  assert.equal(regrown.querySelector('summary').textContent, '3 earlier messages');
  model.applyPmWindowReceiveEvent(conversation([turn[0], turn[1], turn[4], { id: 'a6', role: 'assistant', text: 'Done.' }, { id: 'a7', role: 'assistant', text: 'Bye.' }]));
  assert.deepEqual(order(), ['u1', 'a1', 'u2', 'group:a6', 'a7']);
  assert.equal(log.querySelector('details.pm-chat__collapsed summary').textContent, '1 earlier message');
});

test('Chat rejects stale sessions and explains unavailable conversations', () => {
  const { root, model, view } = fixture();
  view.showChat();
  const notice = () => root.querySelector('.pm-chat__chat [role="status"]').textContent;
  assert.match(notice(), /Loading PM conversation/);
  model.applyPmWindowReceiveEvent(conversation([], { availability: 'waiting', detail: 'Waiting for the PM session.' }));
  assert.equal(notice(), 'Waiting for the PM session.');
  model.applyPmWindowReceiveEvent(conversation([]));
  assert.match(notice(), /No conversation yet/);
  model.applyPmWindowReceiveEvent(conversation([{ id: 'a1', role: 'assistant', text: 'Hello' }]));
  assert.equal(root.querySelectorAll('.pm-chat__bubble').length, 1);
  const before = model.readPmWindowState().windows.pm;
  model.applyPmWindowReceiveEvent({ ...conversation([{ id: 'x', role: 'user', text: 'old' }]), session_id: 'stale' });
  model.applyPmWindowReceiveEvent({ ...conversation([]), id: 'other' });
  assert.equal(model.readPmWindowState().windows.pm, before);
  assert.equal(model.readPmWindowState().windows.other, undefined);
  model.applyPmWindowReceiveEvent(conversation([{ id: 'a1', role: 'assistant', text: 'Hello' }]));
  assert.equal(model.readPmWindowState().windows.pm, before, 'identical snapshots do not re-render');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(root.querySelectorAll('.pm-chat__bubble').length, 0);
});

test('Chat composer sends text to the PM pane and keeps it until delivery is confirmed (AC-3431-CHAT-3)', () => {
  const { root, model, view, sent } = fixture();
  view.showChat();
  const chat = root.querySelector('.pm-chat__chat');
  const form = chat.querySelector('form.pm-chat__composer');
  const draft = form.querySelector('textarea');
  const button = form.querySelector('button[type="submit"]');
  const status = form.querySelector('.pm-chat__composer-status');
  assert.equal(draft.getAttribute('aria-label'), 'Message to PM');
  assert.equal(button.disabled, true, 'an empty draft cannot be sent');
  const key = (extra = {}) => {
    const event = new root.ownerDocument.defaultView.Event('keydown', { bubbles: true, cancelable: true });
    Object.assign(event, { key: 'Enter', shiftKey: false, isComposing: false }, extra);
    draft.dispatchEvent(event);
    return event;
  };
  draft.value = '   ';
  draft.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
  assert.equal(button.disabled, true, 'whitespace is not a message');
  draft.value = 'Ship #3431?\nThen report.';
  draft.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
  assert.equal(button.disabled, false);
  assert.equal(key({ shiftKey: true }).defaultPrevented, false, 'Shift+Enter inserts a new line');
  assert.equal(key({ isComposing: true }).defaultPrevented, false, 'Enter during IME composition is not a send');
  assert.equal(sent.length, 0);
  assert.equal(key().defaultPrevented, true);
  assert.deepEqual(sent, [{ kind: 'pane_send_input', session_id: 'session', text: 'Ship #3431?\nThen report.\r', request_id: sent[0].request_id }]);
  assert.equal(draft.value, 'Ship #3431?\nThen report.', 'the draft is kept while delivery is pending');
  assert.equal(button.disabled, true);
  assert.equal(draft.readOnly, true, 'the pending draft cannot be edited and then lost on success');
  assert.match(status.textContent, /Sending/);
  key();
  assert.equal(sent.length, 1, 'a pending message is not sent twice');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: true, window_id: 'other', error: null, request_id: sent[0].request_id });
  assert.match(status.textContent, /Sending/, 'another window result is ignored');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: false, window_id: 'pm', error: 'no live runtime for remote pane', request_id: sent[0].request_id });
  assert.equal(draft.value, 'Ship #3431?\nThen report.', 'a refused message stays in the draft');
  assert.equal(status.textContent, 'Not sent: no live runtime for remote pane. Your message is kept.');
  assert.equal(button.disabled, false);
  assert.equal(draft.readOnly, false);
  form.dispatchEvent(new root.ownerDocument.defaultView.Event('submit', { cancelable: true }));
  assert.equal(sent.length, 2);
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: true, window_id: 'pm', error: null, request_id: sent[1].request_id });
  assert.equal(draft.value, '', 'a delivered message clears the draft');
  assert.equal(status.textContent, '');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: false, window_id: 'pm', error: 'late' });
  assert.equal(status.textContent, '', 'an unsolicited result does not change the composer');
  draft.value = 'Next';
  draft.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
  key();
  model.expirePmChatSend('pm');
  assert.match(status.textContent, /not confirmed/i);
  assert.equal(draft.value, 'Next');
  model.markPmChatSendUnavailable('pm');
  assert.match(status.textContent, /not confirmed/i, 'only a pending send can fail');
  key();
  model.markPmChatSendUnavailable('pm');
  assert.match(status.textContent, /Not sent.*not connected/i);
  key();
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: false, window_id: null, error: 'no pane bound to this project session', request_id: sent.at(-1).request_id });
  assert.match(status.textContent, /Not sent.*no pane bound/, 'a session refusal reaches the only pending send');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(model.requestPmChatSend('pm', 'x').session_id, 'replacement');
});

test('Chat ignores unrelated and expired send results while preserving the current draft', () => {
  const { root, model, view, sent } = fixture();
  view.showChat();
  const draft = root.querySelector('.pm-chat__draft');
  const submit = () => {
    draft.value = 'Keep this message';
    draft.dispatchEvent(new root.ownerDocument.defaultView.Event('input'));
    root.querySelector('.pm-chat__composer').dispatchEvent(new root.ownerDocument.defaultView.Event('submit', { cancelable: true }));
    return sent.at(-1);
  };
  const first = submit();
  const reply = (request_id, window_id = 'pm') => model.applyPmWindowReceiveEvent({
    kind: 'pane_send_result', request_id, window_id, ok: true, error: null });
  reply(undefined);
  assert.equal(draft.value, 'Keep this message', 'a legacy Command Palette result cannot clear the Chat draft');
  reply(first.request_id + 1);
  assert.equal(model.readPmWindowState().windows.pm.chatSend.status, 'pending');
  model.expirePmChatSend('pm');
  const retry = submit();
  assert.notEqual(retry.request_id, first.request_id);
  reply(first.request_id);
  assert.equal(model.readPmWindowState().windows.pm.chatSend.status, 'pending', 'an expired attempt cannot settle its retry');
  reply(retry.request_id, 'other');
  assert.equal(draft.value, 'Keep this message', 'the correlated reply still must match its window');
  reply(retry.request_id);
  assert.equal(draft.value, '', 'only the matching result clears the draft');
  const oldSession = submit();
  model.bindPmWindowState('pm', 'replacement');
  const rebound = submit();
  reply(oldSession.request_id);
  assert.equal(model.readPmWindowState().windows.pm.chatSend.status, 'pending', 'a former Session result cannot settle a new send');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', request_id: rebound.request_id,
    window_id: null, ok: false, error: 'no pane bound to this project session' });
  assert.equal(model.readPmWindowState().windows.pm.chatSend.status, 'failed');
  assert.equal(draft.value, 'Keep this message', 'a matching early refusal preserves the draft');
});

test('Chat composer is Operator-styled and stays below the scrolling conversation', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const rule = name => css.match(new RegExp(`\\.${name}\\s*\\{([^}]*)\\}`))?.[1] || '';
  assert.match(rule('pm-chat__chat'), /display:\s*flex/);
  assert.match(rule('pm-chat__conversation'), /overflow-y:\s*auto/);
  assert.match(rule('pm-chat__composer'), /border-top:\s*1px solid var\(--color-border\)/);
  assert.match(rule('pm-chat__workspace'), /display:\s*flex/);
  assert.match(css, /@container\s*\(max-width:\s*760px\)[\s\S]*?\.pm-chat--chat\s+\.pm-chat__workspace\s*\{[^}]*flex-direction:\s*column/);
  for (const name of ['pm-chat__composer', 'pm-chat__draft', 'pm-chat__composer-status', 'pm-chat__collapsed', 'pm-chat__collapsed-summary']) {
    assert.doesNotMatch(rule(name), /#[0-9a-f]{3,8}\b|rgba?\(/i, name);
  }
});

const decisionReport = (id = 'decision-1') => ({ ...structuredReport(id), kind: 'decision',
  structured: { summary: { result: 'The launch error task is waiting for a wording choice.',
      impact: 'The task stays stopped until you choose.', owner: 'Noah',
      decision: { question: 'Which recovery message should the agent use?',
        recommendation: 'Use the shorter message', reason: 'It names the next step without internal terms.',
        options: ['Use the shorter message', 'Keep the detailed message'] } },
    details: '', refs: [] } });

test('Chat and Reports share Decide answer pending, failure, retry and confirmed state', () => {
  const { root, model, view, update, sent } = fixture();
  const incoming = decisionReport();
  incoming.structured.summary.decision.options = ['Use <short> wording', 'Keep the detailed message',
    'Inspect the saved evidence', 'Wait for another report'];
  update([incoming]);
  view.showChat();
  const region = root.querySelector('section[aria-label="Decision reports"]');
  assert.ok(region, 'Chat labels its report-derived decisions separately from native messages');
  assert.ok(region.closest('.pm-chat__conversation'), 'decisions stay in the Chat scrolling area');
  const chatControls = () => region.querySelector('[data-decision-id="decision-1"] .pm-chat__decision-controls');
  const reportControls = () => root.querySelector('[data-report-id="decision-1"] .pm-chat__decision-controls');
  assert.ok(chatControls(), 'Chat exposes the saved decision controls');
  assert.ok(reportControls(), 'Reports exposes the same decision controls');
  for (const controls of [chatControls(), reportControls()]) {
    assert.deepEqual([...controls.querySelectorAll('.pm-chat__decision-option')].map(button => button.textContent),
      incoming.structured.summary.decision.options.map((option, index) => `${index + 1}. ${option}`));
    assert.equal(controls.querySelector('short'), null, 'option labels are literal text');
    assert.match(controls.textContent, /Recommended: Use the shorter message/);
  }
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'Keep my unfinished request';
  const option = chatControls().querySelectorAll('.pm-chat__decision-option')[1];
  update([incoming]);
  assert.equal(chatControls().querySelectorAll('.pm-chat__decision-option')[1], option,
    'unchanged polling keeps the focused choice node');
  option.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].text,
    'Answer to "Which recovery message should the agent use?": 2. Keep the detailed message\r');
  for (const controls of [chatControls(), reportControls()]) {
    assert.ok([...controls.querySelectorAll('button')].every(button => button.disabled));
    assert.match(controls.querySelector('[role="status"]').textContent, /Sending/);
  }
  assert.ok(chatControls().querySelectorAll('.pm-chat__decision-option')[1] === option,
    'pending state keeps the keyboard-selected option node');
  root.querySelector('.task-decisions__option').click();
  assert.equal(sent.length, 1, 'switching surfaces cannot duplicate an in-flight answer');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', request_id: sent[0].request_id,
    ok: false, error: 'The PM pane refused the message.' });
  for (const controls of [chatControls(), reportControls()]) {
    assert.match(controls.querySelector('[role="alert"]').textContent, /The PM pane refused the message/);
    assert.equal(controls.querySelector('.pm-chat__decision-option').disabled, false);
  }
  assert.ok(chatControls().querySelectorAll('.pm-chat__decision-option')[1] === option,
    'a failed answer leaves the original option available for keyboard retry');
  assert.match(root.querySelector('.task-decisions__card').textContent, /The PM pane refused the message/);
  const retry = reportControls().querySelector('.pm-chat__decision-option');
  let focused = retry;
  Object.defineProperty(root.ownerDocument, 'activeElement', { configurable: true, get: () => focused });
  retry.click();
  assert.equal(sent.length, 2, 'an explicit choice retries the failed answer');
  assert.doesNotMatch(chatControls().textContent, /Answer sent/);
  const confirmedStatus = reportControls().querySelector('[role="status"]');
  confirmedStatus.focus = () => { focused = confirmedStatus; };
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', request_id: sent[1].request_id,
    ok: true });
  for (const controls of [chatControls(), reportControls()]) {
    assert.match(controls.querySelector('[role="status"]').textContent, /Answer sent: Use <short> wording/);
    assert.equal(controls.querySelectorAll('button').length, 0);
  }
  assert.match(root.querySelector('.task-decisions__card').textContent, /Answer sent: Use <short> wording/);
  assert.ok(focused === confirmedStatus, 'confirmation moves focus from the removed option to its status');
  assert.equal(draft.value, 'Keep my unfinished request', 'a choice send never clears an unrelated composer draft');
  assert.equal(model.readPmWindowState().windows.pm.reports[0].body, incoming.body);
});

test('Other opens Chat without sending or overwriting the draft', () => {
  const { root, view, update, sent } = fixture();
  update([decisionReport(), { ...report('untyped'), kind: 'decision', body: 'Approve the release note wording?' }]);
  const draft = root.querySelector('.pm-chat__draft');
  draft.value = 'My own unfinished answer';
  let focused = false;
  draft.focus = () => { focused = true; };
  const controls = root.querySelector('[data-report-id="decision-1"] .pm-chat__decision-controls');
  assert.ok(controls, 'a structured Report offers a free reply');
  const reply = [...controls.querySelectorAll('button')].find(button => button.textContent === 'Other: reply in chat');
  assert.ok(reply);
  reply.click();
  assert.equal(view.isChatVisible(), true);
  assert.equal(focused, true, 'the existing composer receives focus');
  assert.equal(draft.value, 'My own unfinished answer');
  assert.equal(sent.length, 0);
  const untypedControls = root.querySelectorAll('[data-decision-id="untyped"] .pm-chat__decision-controls, [data-report-id="untyped"] .pm-chat__decision-controls');
  assert.equal(untypedControls.length, 2);
  for (const controls of untypedControls) {
    assert.equal(controls.querySelectorAll('.pm-chat__decision-option').length, 0,
      'a report without a typed packet never invents options');
  }
  root.querySelector('.pm-chat__composer').dispatchEvent(new root.ownerDocument.defaultView.Event('submit'));
  assert.equal(sent[0].text, 'My own unfinished answer\r');
  assert.equal(draft.value, 'My own unfinished answer', 'the free reply stays in its existing draft until confirmed');
});

test('Decision controls follow same-report updates and reset on PM rebind while native messages keep their order', () => {
  const { root, model, update } = fixture();
  const incoming = decisionReport();
  update([incoming]);
  const region = root.querySelector('section[aria-label="Decision reports"]');
  assert.ok(region, 'the report-derived Chat region exists');
  model.applyPmWindowReceiveEvent(conversation([{ id: 'a1', role: 'assistant', text: 'Native PM message' },
    { id: 'u1', role: 'user', text: 'Native user message' }]));
  assert.equal(root.querySelector('section[aria-label="Decision reports"]'), region,
    'native conversation updates keep the decision section attached');
  assert.deepEqual([...root.querySelectorAll('.pm-chat__bubble')].map(node => node.dataset.messageId), ['a1', 'u1']);
  assert.equal(region.closest('.pm-chat__bubble'), null);
  const changed = decisionReport();
  changed.structured.summary.decision.options.push('Ask the reviewer');
  update([changed]);
  for (const controls of root.querySelectorAll('.pm-chat__decision-controls')) {
    assert.equal(controls.querySelectorAll('.pm-chat__decision-option').length, 3);
  }
  root.querySelector('[data-report-id="decision-1"] .pm-chat__decision-option').click();
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: true });
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(region.querySelector('[data-decision-id]'), null, 'a rebind removes the previous report decisions');
  update([incoming]);
  assert.doesNotMatch(region.textContent, /Answer sent/);
  assert.equal(region.querySelectorAll('.pm-chat__decision-option').length, 2,
    'the same report is actionable in its new Session');
  assert.equal(root.querySelectorAll('.pm-chat__bubble').length, 0);
});

test('Report decision controls use Operator layout tokens and wrap long option buttons', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const rule = name => css.match(new RegExp(`\\.${name}\\s*\\{([^}]*)\\}`))?.[1] || '';
  assert.match(rule('pm-chat__decision-controls'), /gap:\s*var\(--space-/);
  assert.match(rule('pm-chat__decision-options'), /flex-wrap:\s*wrap/);
  assert.match(rule('pm-chat__decision-option'), /white-space:\s*normal/);
  assert.match(rule('pm-chat__decision-option'), /overflow-wrap:\s*anywhere/);
  for (const name of ['pm-chat__decision-reports', 'pm-chat__decision-controls', 'pm-chat__decision-options', 'pm-chat__decision-option']) {
    assert.doesNotMatch(rule(name), /#[0-9a-f]{3,8}\b|rgba?\(|position:\s*(fixed|absolute)/i, name);
  }
});

test('Decide lists only decision reports and answers through the chat reply path', () => {
  const { root, model, view, update, sent } = fixture({ tasksDefault: true });
  update([report(), decisionReport(), { ...report('blocker-1'), kind: 'blocker' }]);
  view.showDecisions();
  const decide = root.querySelector('.task-decisions');
  assert.ok(decide, 'a Decide surface exists');
  assert.equal(root.querySelector('.pm-chat__tasks').hidden, true);
  const cards = root.querySelectorAll('.task-decisions__card');
  assert.equal(cards.length, 1, 'progress and blocker reports are not promoted to decisions');
  assert.match(decide.textContent, /1 decision/);
  const options = [...cards[0].querySelectorAll('.task-decisions__option')];
  assert.equal(options.length, 2);
  options[1].click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, 'pane_send_input');
  assert.match(sent[0].text,
    /^Answer to "Which recovery message should the agent use\?": 2\. Keep the detailed message/);
  options[0].click();
  assert.equal(sent.length, 1, 'no second answer is sent while the first is in flight');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: true });
  const card = root.querySelector('.task-decisions__card');
  assert.match(card.textContent, /Answer sent: Keep the detailed message/);
  assert.equal(card.querySelectorAll('.task-decisions__option').length, 0,
    'a confirmed answer cannot be answered again');
});

test('Decide keeps a decision actionable and shows the error when the answer send fails', () => {
  const { root, model, view, update, sent } = fixture({ tasksDefault: true });
  update([decisionReport()]);
  view.showDecisions();
  root.querySelector('.task-decisions__option').click();
  assert.equal(sent.length, 1);
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: false,
    error: 'The PM pane refused the message.' });
  const card = root.querySelector('.task-decisions__card');
  assert.match(card.textContent, /The PM pane refused the message\./);
  const options = card.querySelectorAll('.task-decisions__option');
  assert.equal(options.length, 2, 'the decision stays actionable after a failed send');
  assert.equal(options[0].disabled, false);
});

test('Decide offers the chat reply path for a decision report without a typed packet', () => {
  const { root, view, update, sent } = fixture({ tasksDefault: true });
  update([{ ...report('plain-decision'), kind: 'decision', body: 'Approve the release note wording?' }]);
  view.showDecisions();
  const card = root.querySelector('.task-decisions__card');
  assert.ok(card, 'an untyped decision report is still listed');
  assert.equal(card.querySelectorAll('.task-decisions__option').length, 0,
    'no options are invented without a typed packet');
  const reply = [...card.querySelectorAll('button')].find(button => button.textContent === 'Reply in chat');
  assert.ok(reply);
  reply.click();
  assert.equal(sent.length, 0, 'the reply path never sends by itself');
  assert.equal(root.querySelector('.pm-chat__chat').hidden, false, 'the chat view opens for the reply');
});

test('Home Review decisions opens the Decide surface', async () => {
  const { root, update } = fixture({ tasksDefault: true });
  update([decisionReport()], null, { request_tasks: taskOverview([]) });
  // Home renders through the store's frame batching; wait for the frame to
  // land instead of assuming a synchronous render.
  let review = null;
  for (let attempt = 0; attempt < 80 && !review; attempt++) {
    review = [...root.querySelectorAll('.task-home button')]
      .find(button => /Review 1 decision/.test(button.textContent));
    if (!review) await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.ok(review, 'Home offers Review decisions while a decision waits');
  review.click();
  assert.equal(root.querySelector('.task-decisions').parentElement.hidden, false);
  assert.equal(root.querySelector('.task-home').parentElement.hidden, true);
});

// SPEC #5016 AC-HS1 / AC-HS2: the waiting-decision count is derived once,
// so answering a decision in Decide cannot leave Home still asking for it.
test('Home and Decide read one waiting-decision count after an answer', async () => {
  const { root, model, view, update } = fixture({ tasksDefault: true });
  update([decisionReport('decision-1'), decisionReport('decision-2')], null, { request_tasks: taskOverview([]) });
  view.showDecisions();
  root.querySelector('.task-decisions__option').click();
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: true });
  const home = () => root.querySelector('.task-home')?.textContent ?? '';
  for (let attempt = 0; attempt < 80 && !/1 decision needs/.test(home()); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.match(home(), /1 decision needs your attention/, 'Home does not count the answered decision');
  const actionable = [...root.querySelectorAll('.task-decisions__card')]
    .filter(card => card.querySelector('.task-decisions__option'));
  assert.equal(actionable.length, 1, 'Decide leaves the same one decision actionable');
});

// SPEC #5016 HSK: the shared attention model receives the same waiting
// decisions that Home counts, never a recount of the raw reports.
test('PM decisions waiting are published from the one derivation Home reads', async () => {
  const { root, model, view, waiting, update } = fixture({ tasksDefault: true });
  update([decisionReport('decision-1'), decisionReport('decision-2')], null, { request_tasks: taskOverview([]) });
  assert.deepEqual(waiting.at(-1), { waiting: 2, ids: ['decision-2', 'decision-1'] });
  view.showDecisions();
  root.querySelector('.task-decisions__option').click();
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: true });
  const home = () => root.querySelector('.task-home')?.textContent ?? '';
  for (let attempt = 0; attempt < 80 && !/1 decision needs/.test(home()); attempt++) {
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.equal(waiting.at(-1).waiting, 1);
  assert.equal(waiting.at(-1).ids.length, 1);
  assert.match(home(), /1 decision needs your attention/, 'Home shows the published count');
  const published = waiting.length;
  view.update(model.readPmWindowState().windows.pm);
  assert.equal(waiting.length, published, 'an unchanged waiting set is not republished');
});

// SPEC #5016 HSV: unresolved reviews observed by the PM current facts are
// published once per change; an absent observation is unknown, not zero.
test('PM current review attention is published from the facts the PM window received', () => {
  const { reviews, update } = fixture({ tasksDefault: true });
  update([], null, { facts: { source: 'daemon', active_windows: [] } });
  assert.equal(reviews.at(-1), null, 'facts without the observation publish unknown');
  const items = [{ pr_number: 71, issue_numbers: [5016], reason: 'Changes requested' }];
  update([], null, { facts: { source: 'daemon', active_windows: [], review_attention: items } });
  assert.deepEqual(reviews.at(-1), items);
  const published = reviews.length;
  update([], null, { facts: { source: 'daemon', active_windows: [], review_attention: items } });
  assert.equal(reviews.length, published, 'an unchanged observation is not republished');
  update([], null, { facts: { source: 'daemon', active_windows: [], review_attention: [] } });
  assert.deepEqual(reviews.at(-1), [], 'an observed empty list clears the count');
});

test('Decide forgets session answers when the PM session changes', () => {
  const { root, model, view, update } = fixture({ tasksDefault: true });
  update([decisionReport()]);
  view.showDecisions();
  root.querySelector('.task-decisions__option').click();
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', window_id: 'pm', ok: true });
  assert.match(root.querySelector('.task-decisions__card').textContent, /Answer sent/);
  model.bindPmWindowState('pm', 'other-session');
  update([decisionReport()]);
  const card = root.querySelector('.task-decisions__card');
  assert.doesNotMatch(card.textContent, /Answer sent/);
  assert.equal(card.querySelectorAll('.task-decisions__option').length, 2,
    'the same decision is actionable again in the new session');
});

// --- SPEC #7007 T-012/T-013: Progress and Log views in the PM pane ---

async function waitForPm(find, label) {
  for (let attempt = 0; attempt < 80; attempt++) {
    const found = find();
    if (found) return found;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.fail(`timed out waiting for ${label}`);
}

test('Progress view lists request tasks from the snapshot with snapshot totals', async () => {
  const { root, view, update } = fixture();
  update([], null, { request_tasks: { state: 'ready', revision: 4, total: 57, pending_intake: 3,
    tasks: [trackedTask('one', { status: 'in_progress', assignee: 'Maya' })] } });
  view.showProgress();
  const card = await waitForPm(() => root.querySelector('.task-card'), 'a Progress task card');
  assert.match(card.textContent, /Task one/);
  assert.match(card.textContent, /Maya/);
  const progress = root.querySelector('.task-progress');
  assert.equal(progress.parentElement.hidden, false, 'the Progress surface is visible');
  assert.match(progress.textContent, /57 tasks/);
  const progressButton = [...root.querySelectorAll('.pm-chat__toolbar button')]
    .find(button => button.textContent === 'Progress');
  assert.ok(progressButton, 'the toolbar carries a Progress view button');
  assert.equal(progressButton.getAttribute('aria-pressed'), 'true');
  view.dispose();
});

test('Home View progress opens the Progress surface', async () => {
  const { root, view, update } = fixture({ tasksDefault: true });
  update([], null, { request_tasks: taskOverview([trackedTask('one', { status: 'in_progress' })]) });
  const button = await waitForPm(() => [...root.querySelectorAll('.task-home button')]
    .find(node => node.textContent === 'View progress →'), 'the Home View progress action');
  button.click();
  assert.equal(root.querySelector('.task-progress').parentElement.hidden, false);
  assert.equal(root.querySelector('.task-home').parentElement.hidden, true);
  view.dispose();
});

test('Progress View task details switches to the Tasks view and requests the detail', async () => {
  const { root, view, update, sent } = fixture();
  update([], null, { request_tasks: taskOverview([trackedTask('one', { status: 'in_progress' })]) });
  view.showProgress();
  const details = await waitForPm(() => [...root.querySelectorAll('.task-card button')]
    .find(node => node.textContent === 'View task details'), 'the Progress detail action');
  details.click();
  assert.equal(root.querySelector('.task-progress').parentElement.hidden, true);
  assert.equal(sent.filter(request => request.kind === 'load_pm_request_task').length, 1,
    'the detail read travels the existing task detail path');
  view.dispose();
});

test('Log view renders fed notification groups and routes the drawer action', async () => {
  const { document } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const viewed = [];
  const view = createPmChat({ document, root, sessionId: 'session',
    onLogVisibility: () => {}, onViewNotification: entry => viewed.push(entry) });
  view.receiveLog([{ id: 7, key: 'window-command:agent-1:input', category: 'action',
    title: 'Input failed', message: 'Your input was not delivered to the window.',
    details: 'owner identity mismatch', count: 2, latestAt: new Date().toISOString(),
    resolved: false, activatable: false }]);
  view.showTaskLog();
  const card = await waitForPm(() => root.querySelector('.task-log__card'), 'a Log group card');
  assert.match(card.textContent, /Input failed/);
  assert.match(card.textContent, /Action required/);
  assert.match(card.textContent, /Repeated 2 times/);
  const logButton = [...root.querySelectorAll('.pm-chat__toolbar button')]
    .find(button => button.textContent === 'Log');
  assert.ok(logButton, 'the toolbar carries a Log view button distinct from Execution log');
  assert.equal(logButton.getAttribute('aria-pressed'), 'true');
  const viewButton = [...card.querySelectorAll('button')]
    .find(node => node.textContent === 'View in Notifications');
  viewButton.click();
  assert.equal(viewed.length, 1);
  assert.equal(viewed[0].key, 'window-command:agent-1:input');
  view.dispose();
});

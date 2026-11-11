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

function fixture({ tasksDefault = false } = {}) {
  const { document } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const visibility = [], copied = [], sent = [];
  let model;
  const view = createPmChat({ document, root, sessionId: 'session',
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
  return { root, model, view, visibility, copied, sent, update };
}

const taskOverview = (tasks = [], pending_intake = 2) => ({ state: 'ready', revision: 4, total: tasks.length,
  pending_intake, tasks });
const trackedTask = (key, extra = {}) => ({ key, parent_key: null, title: `Task ${key}`,
  status: 'pending', assignee: 'PM', wait_reason: null, next_action: 'Check the original outcome',
  review_required: false, ...extra });

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
  const { root, model, update } = fixture();
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
  // Only the Chat composer (FR-CHAT-3) accepts input; Reports and Tasks stay read-only.
  const editable = 'input, textarea, form, [contenteditable], button[type="submit"]';
  assert.deepEqual([...root.querySelectorAll(editable)].filter(node => !node.closest('.pm-chat__composer')), []);
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
    const bridge = createTerminalAttachments({ send: event => sent.push(event),
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
    const bridge = createTerminalAttachments({ send: event => sent.push(event),
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
    const bridge = createTerminalAttachments({ send: message => sent.push(message),
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

const conversation = (messages, extra = {}) => ({ kind: 'pm_conversation', id: 'pm', session_id: 'session',
  snapshot: { conversation_id: 'c', availability: 'ready', detail: null, messages, ...extra } });

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
  assert.equal(root.querySelector('.pm-chat__tasks').hidden, true);
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
  assert.deepEqual(sent, [{ kind: 'pane_send_input', session_id: 'session', text: 'Ship #3431?\nThen report.\r' }]);
  assert.equal(draft.value, 'Ship #3431?\nThen report.', 'the draft is kept while delivery is pending');
  assert.equal(button.disabled, true);
  assert.equal(draft.readOnly, true, 'the pending draft cannot be edited and then lost on success');
  assert.match(status.textContent, /Sending/);
  key();
  assert.equal(sent.length, 1, 'a pending message is not sent twice');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: true, window_id: 'other', error: null });
  assert.match(status.textContent, /Sending/, 'another window result is ignored');
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: false, window_id: 'pm', error: 'no live runtime for remote pane' });
  assert.equal(draft.value, 'Ship #3431?\nThen report.', 'a refused message stays in the draft');
  assert.equal(status.textContent, 'Not sent: no live runtime for remote pane. Your message is kept.');
  assert.equal(button.disabled, false);
  assert.equal(draft.readOnly, false);
  form.dispatchEvent(new root.ownerDocument.defaultView.Event('submit', { cancelable: true }));
  assert.equal(sent.length, 2);
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: true, window_id: 'pm', error: null });
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
  model.applyPmWindowReceiveEvent({ kind: 'pane_send_result', ok: false, window_id: null, error: 'no pane bound to this project session' });
  assert.match(status.textContent, /Not sent.*no pane bound/, 'a session refusal reaches the only pending send');
  model.bindPmWindowState('pm', 'replacement');
  assert.equal(model.requestPmChatSend('pm', 'x').session_id, 'replacement');
});

test('Chat composer is Operator-styled and stays below the scrolling conversation', async () => {
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const rule = name => css.match(new RegExp(`\\.${name}\\s*\\{([^}]*)\\}`))?.[1] || '';
  assert.match(rule('pm-chat__chat'), /display:\s*flex/);
  assert.match(rule('pm-chat__conversation'), /overflow-y:\s*auto/);
  assert.match(rule('pm-chat__composer'), /border-top:\s*1px solid var\(--color-border\)/);
  for (const name of ['pm-chat__composer', 'pm-chat__draft', 'pm-chat__composer-status', 'pm-chat__collapsed', 'pm-chat__collapsed-summary']) {
    assert.doesNotMatch(rule(name), /#[0-9a-f]{3,8}\b|rgba?\(/i, name);
  }
});

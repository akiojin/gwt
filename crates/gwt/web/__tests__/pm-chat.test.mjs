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

function fixture() {
  const { document } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const visibility = [], copied = [];
  const view = createPmChat({ document, root, sessionId: 'session',
    onLogVisibility: visible => visibility.push(visible), copyText: async text => copied.push(text) });
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  model.subscribePmWindowState(state => state.windows.pm, state => view.update(state));
  let revision = 0;
  const update = (reports = [], error = null, current = null) => {
    const request = model.requestPmReports('pm');
    const event = { kind: 'pm_reports', id: 'pm', request_id: request.request_id,
      version: { epoch: 'fixture', revision: ++revision }, snapshot: request.version === null,
      session_id: model.readPmWindowState().windows.pm.sessionId, reports, error, current };
    model.applyPmWindowReceiveEvent(event);
    return event;
  };
  return { root, model, view, visibility, copied, update };
}

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
    needs_human_count: null, release: null, release_observed_at: null, pending_update: null, error: 'Monitor unavailable' } });
  const facts = root.querySelector('[aria-label="Live facts"]');
  assert.match(facts.textContent, /Monitor unavailable/);
  assert.match(facts.textContent, /needs_human: Unknown/);
  assert.match(facts.textContent, /Release: Unknown/);
});

test('Reports renders shared Markdown, metadata and copy, with no input or submit surface', async () => {
  const { root, update, copied } = fixture();
  update([report()]);
  assert.equal(root.querySelector('[role="log"]').getAttribute('aria-label'), 'PM reports');
  assert.equal(root.querySelector('h1').textContent, 'Update');
  assert.equal(root.querySelector('article strong').textContent, 'Ready');
  assert.equal(root.querySelector('li').textContent, 'checked');
  assert.equal(root.querySelector('pre code').textContent, 'cargo test');
  assert.equal(root.querySelector('article time').getAttribute('datetime'), report().created_at);
  assert.match(root.textContent, /progress/);
  assert.equal(root.querySelectorAll('input, textarea, form, [contenteditable], button[type="submit"]').length, 0);
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
  assert.match(root.querySelector('[role="status"]').textContent, /No Blocker reports yet/);
  root.querySelectorAll('.pm-chat__toolbar button')[1].click();
  assert.equal(select.closest('.pm-chat__body').hidden, true);
  root.querySelectorAll('.pm-chat__toolbar button')[0].click();
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
  root.querySelectorAll('.pm-chat__toolbar button')[1].click();
  update([...history, report('four')]);
  root.querySelectorAll('.pm-chat__toolbar button')[0].click();
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
  assert.match(root.querySelector('[role="status"]').textContent, /could not be loaded/);
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
  buttons[1].click();
  assert.equal(visibility.at(-1), true);
  view.showReports();
  assert.equal(visibility.at(-1), false);
  assert.equal(root.querySelectorAll('article').length, 1);
  const previous = model.readPmWindowState().windows.pm;
  model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm', session_id: 'stale', reports: [] });
  model.applyPmWindowReceiveEvent({ kind: 'pm_conversation', id: 'pm', session_id: 'session', snapshot: { messages: [] } });
  assert.ok(model.readPmWindowState().windows.pm === previous);
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

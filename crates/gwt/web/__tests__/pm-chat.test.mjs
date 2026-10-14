import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseHTML } from 'linkedom';
import { createPmChat, createPmWindowModel } from '../pm-chat.js';

const report = (id = 'one') => ({ id, kind: 'progress', created_at: '2026-10-11T00:00:00Z',
  body: '# Update\n\n**Ready** and `code`\n\n- checked\n\n```sh\ncargo test\n```',
  body_html: '<h1>Update</h1><p><strong>Ready</strong> and <code>code</code></p><ul><li>checked</li></ul><pre><code>cargo test</code></pre>' });

test('slice revisions fence Reports gaps and RAW interleave while snapshots recover bounded history', () => {
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  const receive = (revision, base, mode, reports, extra = {}) => model.applyPmWindowReceiveEvent({
    kind: 'pm_reports', id: 'pm', session_id: 'session', reports, reports_mode: mode,
    slice: { epoch: 'server', base_revision: base, revision }, ...extra });
  receive(1, 0, 'snapshot', [report()]);
  assert.equal(model.readPmWindowState().windows.pm.reportsCursor.revision, 1);
  receive(2, 1, 'delta', [report('two')], { ordered_ids: ['one', 'two'], removed_ids: [] });
  assert.deepEqual(model.readPmWindowState().windows.pm.reports.map(item => item.id), ['one', 'two']);
  receive(1, 0, 'snapshot', [report('old')]);
  assert.equal(model.readPmWindowState().windows.pm.reports[1].id, 'two');
  assert.equal(receive(4, 3, 'delta', [report('gap')]), 'reports');
  assert.equal(model.readPmWindowState().windows.pm.reportsCursor.revision, 2);
  receive(4, 0, 'snapshot', [report('tail')]);
  assert.deepEqual(model.readPmWindowState().windows.pm.reports.map(item => item.id), ['tail']);
  const history = model.readPmWindowState().windows.pm.reports;
  receive(4, 4, 'unchanged', [], { current: { observed_at: '2026-10-14T00:00:00Z', stale_minutes: 12 } });
  assert.equal(model.readPmWindowState().windows.pm.reports, history);
  assert.equal(model.readPmWindowState().windows.pm.current.stale_minutes, 12);
  const consumed = [];
  for (const id of ['a', 'b', 'c', 'd', 'e']) model.subscribePmWindowState(state => state.windows[id]?.terminal,
    packet => { if (packet) consumed.push(packet.dataBase64); });
  for (const id of ['a', 'b', 'c', 'd', 'e']) for (let revision = 1; revision <= 2; revision++) {
    assert.equal(model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id, data_base64: `${id}${revision}`,
      slice: { epoch: 'connection', base_revision: revision - 1, revision } }), null);
  }
  assert.equal(consumed.length, 10);
  assert.equal(model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'a', data_base64: 'gap',
    slice: { epoch: 'connection', base_revision: 3, revision: 4 } }), 'terminal');
  assert.equal(consumed.length, 10);
  assert.equal(model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'a', data_base64: 'pending',
    slice: { epoch: 'connection', base_revision: 4, revision: 5 } }), null, 'one snapshot request covers the pending suffix');
  model.applyPmWindowReceiveEvent({ kind: 'terminal_snapshot', id: 'a', data_base64: 'recovered',
    slice: { epoch: 'new-connection', base_revision: 0, revision: 1 } });
  assert.equal(consumed.at(-1), 'recovered');
  model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'a', data_base64: 'retired',
    slice: { epoch: 'connection', base_revision: 2, revision: 3 } });
  assert.equal(consumed.at(-1), 'recovered');
});

function fixture() {
  const { document } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const visibility = [], copied = [], requested = [];
  const view = createPmChat({ document, root, sessionId: 'session',
    onLogVisibility: visible => visibility.push(visible), copyText: async text => copied.push(text),
    onRequestReports: request => requested.push(request) });
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  model.subscribePmWindowState(state => state.windows.pm, state => view.update(state));
  let reportRevision = 0;
  const update = (reports = [], error = null, current = null, revision = ++reportRevision) => model.applyPmWindowReceiveEvent({
    kind: 'pm_reports', id: 'pm', session_id: model.readPmWindowState().windows.pm.sessionId, reports, error, current,
    reports_mode: 'snapshot', slice: { epoch: 'reports', base_revision: 0, revision } });
  return { root, model, view, visibility, copied, requested, update };
}

test('Reports pages replace bounded visible history and can return to current reports', () => {
  const { root, model, requested } = fixture();
  const before = { created_at: '2026-10-11T00:00:00Z', source: 'legacy', ordinal: 20, id: 'new' };
  const receive = (mode, reports, cursor) => model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm',
    session_id: 'session', slice: { epoch: 'server', base_revision: 0, revision: 1 },
    reports_mode: mode, reports, before: cursor });
  receive('snapshot', [report('new')], before);
  root.querySelector('[aria-label="Older PM reports"]').click();
  assert.deepEqual(requested, [{ before }]);
  receive('history', [report('old')], null);
  assert.equal(root.querySelectorAll('article').length, 1);
  assert.equal(root.querySelector('article').dataset.reportId, 'old');
  assert.equal(root.querySelector('[aria-label="Older PM reports"]').hidden, true);
  receive('unchanged', [], before);
  assert.equal(model.readPmWindowState().windows.pm.reportsPage, true, 'delayed polling cannot leave an older page marked as latest');
  root.querySelector('[aria-label="Latest PM reports"]').click();
  assert.deepEqual(requested.at(-1), { reset: true });
  receive('snapshot', [report('new')], before);
  assert.equal(root.querySelector('article').dataset.reportId, 'new');
  assert.equal(root.querySelector('[aria-label="Latest PM reports"]').hidden, true);
  receive('history', [report('old')], null);
  model.resetPmWindowSlices();
  assert.equal(model.readPmWindowState().windows.pm.reportsCursor, null, 'reconnect requires a fresh Reports snapshot');
});

test('Reports request generations preserve the latest paging intent across out-of-order workers', () => {
  const model = createPmWindowModel();
  model.bindPmWindowState('pm', 'session');
  const poll = model.preparePmReportsRequest('pm');
  const history = model.preparePmReportsRequest('pm', { before: { id: 'anchor' } });
  const response = (request, reports, mode) => model.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'pm',
    session_id: 'session', request_id: request.request_id, reports_mode: mode, reports,
    slice: { epoch: 'server', base_revision: 0, revision: 1 } });
  response(history, [report('old')], 'history');
  response(poll, [report('latest')], 'snapshot');
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, 'old');
  const latest = model.preparePmReportsRequest('pm', { reset: true });
  assert.equal(model.preparePmReportsRequest('pm'), null, 'routine polling cannot supersede a pending explicit Latest request');
  response(latest, [report('latest')], 'snapshot');
  response(history, [report('old')], 'history');
  assert.equal(model.readPmWindowState().windows.pm.reports[0].id, 'latest');
  const slowPoll = model.preparePmReportsRequest('pm');
  assert.equal(model.preparePmReportsRequest('pm'), null, 'slow successful polls must not starve behind newer polls');
  response(slowPoll, [], 'unchanged');
  assert.ok(model.preparePmReportsRequest('pm'), 'accepted replies release the pending poll');
});

test('current card stays above history and distinguishes PM explanation from automatic facts', () => {
  const { root, update } = fixture();
  const card = root.querySelector('[aria-label="PM の現在"]');
  assert.ok(card, 'current card exists before any PM post');
  assert.match(card.textContent, /現在状態はまだ報告されていません/);
  const current = {
    observed_at: '2026-10-13T09:20:00.100Z', stale_minutes: 15,
    status: { items: [{ kind: 'waiting', text: 'v9.117.2 の公開を待っています', started_at: '2026-10-13T09:00:00Z' }],
      next_trigger: 'v9.117.2 の公開', user_action: null, updated_at: '2026-10-13T09:05:00Z' },
    facts: { source: 'daemon', active_windows: [{ issue_number: 5349, title: 'PM current state', last_activity_at: '2026-10-13T09:19:00Z' }],
      queue_head: { issue_number: 5342, title: 'Instruction memory' }, needs_human_count: 2,
      release: { number: 5352, title: 'Release v9.117.2', url: 'https://github.com/akiojin/gwt/pull/5352' },
      release_observed_at: '2026-10-13T09:18:00Z', pending_update: '9.117.2', error: null },
  };
  update([report()], null, current);
  assert.equal(root.querySelector('.pm-chat__body').firstElementChild, card);
  assert.match(card.querySelector('[aria-label="PM の記述"]').textContent, /待ち.*v9.117.2/s);
  assert.equal(card.querySelector('time').getAttribute('datetime'), current.status.items[0].started_at);
  assert.match(card.textContent, /ユーザーへの依頼: なし/);
  assert.match(card.textContent, /PM の更新が 15 分止まっています/);
  const facts = card.querySelector('[aria-label="自動の事実"]');
  assert.match(facts.textContent, /#5349.*PM current state/s);
  assert.match(facts.textContent, /#5342.*Instruction memory/s);
  assert.match(facts.textContent, /needs_human: 2/);
  assert.match(facts.textContent, /Release v9.117.2/);
  assert.match(facts.textContent, /未適用の更新: v9.117.2/);
  assert.equal(root.querySelectorAll('article').length, 1, 'current state is outside chronological reports');
  update([report()], null, { ...current, observed_at: '2026-10-13T09:20:00.100001Z', stale_minutes: null, status: { ...current.status,
    items: [{ ...current.status.items[0], text: '公開結果を確認しています', kind: 'working' }], user_action: '配信設定を確認' } });
  assert.doesNotMatch(card.textContent, /公開を待っています|15 分止まっています/);
  assert.match(card.textContent, /公開結果を確認しています|配信設定を確認/);
  update([report()], null, current);
  assert.match(card.textContent, /公開結果を確認しています/, 'an older concurrent snapshot cannot restore the earlier status');
  assert.doesNotMatch(card.textContent, /公開を待っています/);
  update([], 'Reports could not be loaded');
  assert.match(card.textContent, /公開結果を確認しています/, 'failed reads retain latest measured state');
});

test('current card marks unknown automatic data instead of claiming an empty fleet', () => {
  const { root, update } = fixture();
  update([], null, { status: null, facts: { source: 'unavailable', active_windows: [], queue_head: null,
    needs_human_count: null, release: null, release_observed_at: null, pending_update: null, error: 'Monitor unavailable' } });
  const facts = root.querySelector('[aria-label="自動の事実"]');
  assert.match(facts.textContent, /Monitor unavailable/);
  assert.match(facts.textContent, /needs_human: 不明/);
  assert.match(facts.textContent, /リリース: 不明/);
});

test('Reports renders shared Markdown, metadata and copy, with no input or submit surface', async () => {
  const { root, update, copied } = fixture();
  update([report()]);
  assert.equal(root.querySelector('[role="log"]').getAttribute('aria-label'), 'PM reports');
  assert.equal(root.querySelector('h1').textContent, 'Update');
  assert.equal(root.querySelector('strong').textContent, 'Ready');
  assert.equal(root.querySelector('li').textContent, 'checked');
  assert.equal(root.querySelector('pre code').textContent, 'cargo test');
  assert.equal(root.querySelector('time').getAttribute('datetime'), report().created_at);
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

test('report updates preserve nodes and scroll, append history, and retain history on read failure', () => {
  const { root, update } = fixture();
  update([report()]);
  const first = root.querySelector('article');
  const transcript = root.querySelector('[role="log"]');
  transcript.scrollTop = 20;
  update([report()]);
  assert.ok(root.querySelector('article') === first);
  assert.equal(transcript.scrollTop, 20);
  update([report(), report('two')]);
  assert.equal(root.querySelectorAll('article').length, 2);
  // Concurrent reads may complete out of order; revision fences older reads.
  update([report()], null, null, 1);
  assert.equal(root.querySelectorAll('article').length, 2);
  update([], 'Reports could not be loaded');
  assert.equal(root.querySelectorAll('article').length, 2);
  assert.match(root.querySelector('[role="status"]').textContent, /could not be loaded/);
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
  model.applyPmWindowReceiveEvent({ kind: 'terminal_snapshot', id: 'pm', data_base64: 'snapshot',
    slice: { epoch: 'terminal', base_revision: 0, revision: 1 } });
  update([report()]);
  model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'pm', data_base64: 'output',
    slice: { epoch: 'terminal', base_revision: 1, revision: 2 } });
  assert.deepEqual(versions, [1, 2, 3, 4]);
  assert.deepEqual(packets, ['snapshot', 'output']);
  assert.ok(Object.isFrozen(model.readPmWindowState().windows.pm.reports));
});

test('RAW updates advance the view version without serializing unchanged filtered Reports', () => {
  const { root, model, update } = fixture();
  update([report(), { ...report('qa'), kind: 'qa', question: 'Why?' }]);
  const select = root.querySelector('select');
  select.querySelector('option[value="qa"]').selected = true;
  select.dispatchEvent(new root.ownerDocument.defaultView.Event('change'));
  const first = root.querySelector('article');
  const stringify = JSON.stringify;
  let serialized = 0;
  try {
    JSON.stringify = (...args) => { serialized++; return stringify(...args); };
    for (const [index, packet] of ['one', 'two', 'three'].entries()) {
      model.applyPmWindowReceiveEvent({ kind: 'terminal_output', id: 'pm',
        data_base64: Buffer.from(packet).toString('base64'),
        slice: { epoch: 'terminal', base_revision: index, revision: index + 1 } });
    }
  } finally { JSON.stringify = stringify; }
  assert.equal(serialized, 0, 'RAW never scans the full Reports history');
  assert.equal(root.querySelector('article'), first);
  assert.equal(root.dataset.stateVersion, String(model.readPmWindowState().windows.pm.revision));
});

test('Execution log toggle retains Reports; stale sessions and native transcript cannot replace reports', () => {
  const { root, model, update, visibility } = fixture();
  update([report()]);
  const buttons = root.querySelectorAll('.pm-chat__toolbar button');
  buttons[1].click();
  assert.equal(visibility.at(-1), true);
  buttons[0].click();
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
  assert.match(renderer, /markdownContent/);
  assert.doesNotMatch(renderer, /Claude|Codex|OpenCode|agent_id|⏺|⎿|\\x1b/i);
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const section = css.slice(css.indexOf('/* PM conversation */'));
  assert.match(section, /var\(--color-/);
  assert.match(section, /\.pm-chat__filter\s*\{[^}]*var\(--space-/);
  assert.match(section, /\.pm-chat__filter select:focus-visible\s*\{[^}]*var\(--color-focus-ring\)/);
  assert.match(section, /\.pm-chat__question\s*\{[^}]*white-space:\s*pre-wrap/);
  assert.match(section, /\.pm-chat__section-label\s*\{[^}]*var\(--type-/);
  assert.doesNotMatch(section, /#[\da-f]{3,8}\b|rgba?\(|position:\s*(fixed|absolute)/i);
});


test('Reports rejects file-drop input while Execution log keeps the attachment bridge', async () => {
  let source = await readFile(new URL('../terminal-attachments.js', import.meta.url), 'utf8');
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu']) {
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
  for (const name of ['terminal-copy-shortcut', 'terminal-context-menu']) {
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

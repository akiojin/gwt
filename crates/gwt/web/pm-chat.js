import { createUiStateStore } from './ui-state-store.js';
import { markdownContent, renderUiContent } from './ui-content.js';

// Explicit PM reports and raw terminal packets share one immutable window version.
// The terminal runtime retains responsibility for buffering and snapshot ordering.
export function createPmWindowModel() {
  const model = createUiStateStore({ windows: {}, changedWindowId: null });
  // chrono emits UTC timestamps with variable fractional precision. Keep the
  // nanoseconds when comparing; JavaScript Date truncates them to milliseconds.
  const snapshotTime = value => value.replace(/(?:\.(\d+))?Z$/, (_, digits = '') => `.${digits.padEnd(9, '0')}Z`);
  function emptyWindow(windowId, sessionId = null) {
    return { windowId, sessionId, revision: 0, terminal: null,
      reports: [], current: null, error: null, loaded: false };
  }
  function bindPmWindowState(windowId, sessionId) {
    model.update(state => {
      const previous = state.windows[windowId];
      if (previous && previous.sessionId === sessionId) return state;
      const next = previous?.sessionId == null ? { ...emptyWindow(windowId, sessionId), terminal: previous?.terminal || null }
        : emptyWindow(windowId, sessionId);
      return { windows: { ...state.windows, [windowId]: { ...next, revision: (previous?.revision || 0) + 1 } }, changedWindowId: windowId };
    });
  }
  function applyPmWindowReceiveEvent(event) {
    model.update(state => {
      const previous = state.windows[event.id];
      let next = previous || emptyWindow(event.id);
      if (event.kind === 'pm_reports') {
        if (!previous || event.session_id !== previous.sessionId) return state;
        // The durable ledger only appends. Ignore older concurrent reads.
        if (!event.error && (event.reports || []).length < previous.reports.length) return state;
        if (!event.error && event.current?.observed_at && previous.current?.observed_at
          && snapshotTime(event.current.observed_at) < snapshotTime(previous.current.observed_at)) return state;
        next = { ...next, error: event.error || null, loaded: true,
          current: event.error ? previous.current : (event.current || null),
          reports: event.error ? previous.reports : (event.reports || []) };
      } else if (event.kind === 'terminal_output' || event.kind === 'terminal_snapshot') {
        next = { ...next, terminal: { kind: event.kind, dataBase64: event.data_base64 } };
      } else return state;
      return { windows: { ...state.windows, [event.id]: { ...next, revision: next.revision + 1 } }, changedWindowId: event.id };
    });
  }
  function removePmWindowState(windowId) {
    model.update(state => {
      const { [windowId]: removed, ...windows } = state.windows;
      return removed ? { windows, changedWindowId: windowId } : state;
    });
  }
  function readPmWindowState() { return model.read(); }
  function subscribePmWindowState(select, render) { return model.subscribe(select, render); }
  return { bindPmWindowState, applyPmWindowReceiveEvent, removePmWindowState, readPmWindowState, subscribePmWindowState };
}

// Read-only report view. The caller owns polling and the unchanged terminal.
export function createPmChat({ document, root, sessionId, onLogVisibility,
  copyText = text => document.defaultView.navigator.clipboard.writeText(text) }) {
  let session = sessionId;
  let disposed = false;
  let logs = false;
  let digest = '';
  let currentDigest;
  let latestState;
  const element = (tag, className, text) => {
    const node = document.createElement(tag);
    node.className = className;
    if (text) node.textContent = text;
    return node;
  };
  root.classList.add('pm-chat');
  const toolbar = element('div', 'pm-chat__toolbar');
  toolbar.setAttribute('role', 'group');
  toolbar.setAttribute('aria-label', 'PM view');
  const reportsButton = element('button', '', 'Reports');
  const logButton = element('button', '', 'Execution log');
  reportsButton.type = logButton.type = 'button';
  toolbar.append(reportsButton, logButton);
  const body = element('div', 'pm-chat__body');
  const currentCard = element('section', 'pm-chat__current');
  currentCard.setAttribute('aria-label', 'PM の現在');
  const filter = element('label', 'pm-chat__filter', 'Report kind');
  const kindSelect = element('select', '');
  kindSelect.setAttribute('aria-label', 'Report kind');
  const kindLabels = { all: 'All reports', qa: 'Q&A', progress: 'Progress', decision: 'Decision', blocker: 'Blocker' };
  for (const [kind, label] of Object.entries(kindLabels)) {
    const option = element('option', '', label);
    option.value = kind;
    option.selected = kind === 'all';
    kindSelect.append(option);
  }
  filter.append(kindSelect);
  const notice = element('p', 'pm-chat__notice');
  notice.setAttribute('role', 'status');
  const transcript = element('div', 'pm-chat__transcript');
  transcript.setAttribute('role', 'log');
  transcript.setAttribute('aria-label', 'PM reports');
  body.append(currentCard, filter, notice, transcript);
  root.append(toolbar, body);

  function showLogs(value) {
    logs = value;
    body.hidden = logs;
    root.classList.toggle('pm-chat--logs', logs);
    reportsButton.setAttribute('aria-pressed', String(!logs));
    logButton.setAttribute('aria-pressed', String(logs));
    onLogVisibility(logs);
  }
  const showReports = () => showLogs(false);
  const showLog = () => showLogs(true);
  const filterReports = () => update(latestState);
  function renderCurrent(current) {
    const nextDigest = JSON.stringify(current ? [current.status, current.facts, current.stale_minutes] : null);
    if (currentDigest === nextDigest) return;
    currentDigest = nextDigest;
    const authored = element('section', 'pm-chat__current-authored');
    authored.setAttribute('aria-label', 'PM の記述');
    authored.append(element('h3', '', 'PM の記述'));
    const time = value => {
      const node = element('time', '', value ? new Date(value).toLocaleString() : '不明');
      if (value) node.setAttribute('datetime', value);
      return node;
    };
    const status = current?.status;
    if (status) {
      const items = element('ul', '');
      for (const item of status.items) {
        const row = element('li', '', `${item.kind === 'waiting' ? '待ち' : '作業'}: ${item.text} — 開始 `);
        row.append(time(item.started_at));
        items.append(row);
      }
      authored.append(items, element('p', '', `次に動く条件: ${status.next_trigger}`),
        element('p', '', `ユーザーへの依頼: ${status.user_action || 'なし'}`));
      const updated = element('p', 'pm-chat__current-meta', '更新: ');
      updated.append(time(status.updated_at));
      authored.append(updated);
      if (current.stale_minutes != null) {
        const stale = element('p', 'pm-chat__current-stale', `PM の更新が ${current.stale_minutes} 分止まっています`);
        stale.setAttribute('role', 'status');
        authored.append(stale);
      }
    } else authored.append(element('p', '', '現在状態はまだ報告されていません。'));
    const automatic = element('section', 'pm-chat__current-facts');
    automatic.setAttribute('aria-label', '自動の事実');
    automatic.append(element('h3', '', '自動の事実'));
    const facts = current?.facts;
    if (!facts) automatic.append(element('p', '', 'Monitor の事実を読み込み中…'));
    else {
      const issueLabel = issue => `${issue.issue_number ? `#${issue.issue_number}` : 'Issue 未割当'} ${issue.title || '題名不明'}`;
      automatic.append(element('p', 'pm-chat__current-meta', `Monitor: ${facts.source === 'daemon' ? '接続中' : facts.source === 'degraded_cache' ? 'キャッシュ（Monitor 未接続）' : '取得不可'}`));
      if (facts.error) automatic.append(element('p', 'pm-chat__current-stale', facts.error));
      const windows = element('ul', '');
      for (const item of facts.active_windows || []) {
        const row = element('li', '', `${issueLabel(item)} — 最終活動 `);
        row.append(time(item.last_activity_at));
        windows.append(row);
      }
      automatic.append(element('p', '', `稼働中の窓: ${(facts.active_windows || []).length}${facts.inventory_incomplete ? '（観測は不完全）' : ''}`), windows,
        element('p', '', `queue の先頭: ${facts.queue_head ? issueLabel(facts.queue_head) : facts.source === 'unavailable' ? '不明' : 'なし'}`),
        element('p', '', `needs_human: ${facts.needs_human_count ?? '不明'}`),
        element('p', '', `リリース: ${facts.release ? `#${facts.release.number} ${facts.release.title}` : facts.release_observed_at ? '進行中の PR なし' : '不明'}`),
        element('p', '', `未適用の更新: ${facts.pending_update ? `v${facts.pending_update}` : facts.update_known ? 'なし' : '不明'}`));
      if (facts.release_observed_at) {
        const observed = element('p', 'pm-chat__current-meta', 'リリース PR 観測: ');
        observed.append(time(facts.release_observed_at));
        automatic.append(observed);
      }
    }
    currentCard.replaceChildren(element('h2', '', 'PM の現在'), authored, automatic);
  }
  function update(state) {
    if (disposed || !state) return;
    setSession(state.sessionId);
    latestState = state;
    root.dataset.stateVersion = String(state.revision);
    renderCurrent(state.current);
    const reports = state.reports;
    const visibleReports = kindSelect.value === 'all' ? reports : reports.filter(report => report.kind === kindSelect.value);
    const nextDigest = JSON.stringify([kindSelect.value, visibleReports]);
    if (digest !== nextDigest) {
      const atBottom = transcript.scrollHeight - transcript.clientHeight - transcript.scrollTop <= 32;
      const previousScroll = transcript.scrollTop;
      const nodes = visibleReports.map(report => {
        const row = element('article', 'pm-chat__message');
        const qa = report.kind === 'qa';
        const header = element('div', 'pm-chat__author');
        const time = element('time', '', new Date(report.created_at).toLocaleString());
        time.setAttribute('datetime', report.created_at);
        const copy = element('button', '', 'Copy');
        copy.type = 'button';
        copy.setAttribute('aria-label', 'Copy report');
        copy.addEventListener('click', async () => {
          try {
            await copyText(qa ? `Q: ${report.question || ''}\n\nA:\n${report.body}` : report.body);
            if (!disposed) copy.textContent = 'Copied';
          } catch {
            if (!disposed) {
              notice.textContent = 'Could not copy. Select the report text and copy.';
              notice.hidden = false;
            }
          }
        });
        header.append(element('span', '', qa ? 'Q&A' : report.kind), time, copy);
        row.append(header);
        const content = renderUiContent(document, markdownContent(report), 'pm-chat__text');
        if (qa) {
          const question = element('section', '');
          question.setAttribute('aria-label', 'Question');
          question.append(element('h2', 'pm-chat__section-label', 'Q'), element('p', 'pm-chat__question', report.question || ''));
          const answer = element('section', 'pm-chat__answer');
          answer.setAttribute('aria-label', 'Answer');
          answer.append(element('h2', 'pm-chat__section-label', 'A'), content);
          row.append(question, answer);
        } else row.append(content);
        return row;
      });
      transcript.replaceChildren(...nodes);
      transcript.scrollTop = atBottom ? transcript.scrollHeight : previousScroll;
      digest = nextDigest;
    }
    notice.textContent = state.error || (!state.loaded ? 'Loading PM reports…'
      : !reports.length ? 'No PM reports yet. Use the execution log to interact with PM.'
        : visibleReports.length ? '' : `No ${kindLabels[kindSelect.value]} reports yet.`);
    notice.hidden = !notice.textContent;
  }
  function setSession(nextSession) {
    if (disposed || session === nextSession) return;
    session = nextSession;
    transcript.replaceChildren();
    transcript.scrollTop = 0;
    digest = '';
    currentDigest = undefined;
    showLogs(false);
  }
  function dispose() {
    disposed = true;
    reportsButton.removeEventListener('click', showReports);
    logButton.removeEventListener('click', showLog);
    kindSelect.removeEventListener('change', filterReports);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs');
  }
  reportsButton.addEventListener('click', showReports);
  logButton.addEventListener('click', showLog);
  kindSelect.addEventListener('change', filterReports);
  update({ sessionId, revision: 0, reports: [], loaded: false });
  showLogs(false);
  return { update, setSession, dispose };
}

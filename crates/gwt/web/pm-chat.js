import { createUiStateStore } from './ui-state-store.js';
import { markdownContent, renderUiContent } from './ui-content.js';

// Explicit PM reports and raw terminal packets share one immutable window version.
// The terminal runtime retains responsibility for buffering and snapshot ordering.
export function createPmWindowModel() {
  const model = createUiStateStore({ windows: {}, changedWindowId: null });
  function emptyWindow(windowId, sessionId = null) {
    return { windowId, sessionId, revision: 0, terminal: null,
      reports: [], error: null, loaded: false };
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
        next = { ...next, error: event.error || null, loaded: true,
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
  const notice = element('p', 'pm-chat__notice');
  notice.setAttribute('role', 'status');
  const transcript = element('div', 'pm-chat__transcript');
  transcript.setAttribute('role', 'log');
  transcript.setAttribute('aria-label', 'PM reports');
  body.append(notice, transcript);
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
  function update(state) {
    if (disposed || !state) return;
    setSession(state.sessionId);
    root.dataset.stateVersion = String(state.revision);
    const reports = state.reports;
    const nextDigest = JSON.stringify(reports);
    if (digest !== nextDigest) {
      const atBottom = transcript.scrollHeight - transcript.clientHeight - transcript.scrollTop <= 32;
      const previousScroll = transcript.scrollTop;
      const nodes = reports.map(report => {
        const row = element('article', 'pm-chat__message');
        const header = element('div', 'pm-chat__author');
        const time = element('time', '', new Date(report.created_at).toLocaleString());
        time.setAttribute('datetime', report.created_at);
        const copy = element('button', '', 'Copy');
        copy.type = 'button';
        copy.setAttribute('aria-label', 'Copy report');
        copy.addEventListener('click', async () => {
          try {
            await copyText(report.body);
            if (!disposed) copy.textContent = 'Copied';
          } catch {
            if (!disposed) {
              notice.textContent = 'Could not copy. Select the report text and copy.';
              notice.hidden = false;
            }
          }
        });
        header.append(element('span', '', report.kind), time, copy);
        row.append(header, renderUiContent(document, markdownContent(report), 'pm-chat__text'));
        return row;
      });
      transcript.replaceChildren(...nodes);
      transcript.scrollTop = atBottom ? transcript.scrollHeight : previousScroll;
      digest = nextDigest;
    }
    notice.textContent = state.error || (!state.loaded ? 'Loading PM reports…'
      : reports.length ? '' : 'No PM reports yet. Use the execution log to interact with PM.');
    notice.hidden = !notice.textContent;
  }
  function setSession(nextSession) {
    if (disposed || session === nextSession) return;
    session = nextSession;
    transcript.replaceChildren();
    transcript.scrollTop = 0;
    digest = '';
    showLogs(false);
  }
  function dispose() {
    disposed = true;
    reportsButton.removeEventListener('click', showReports);
    logButton.removeEventListener('click', showLog);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs');
  }
  reportsButton.addEventListener('click', showReports);
  logButton.addEventListener('click', showLog);
  update({ sessionId, revision: 0, reports: [], loaded: false });
  showLogs(false);
  return { update, setSession, dispose };
}

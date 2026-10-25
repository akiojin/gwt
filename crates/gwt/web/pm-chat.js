import { createUiStateStore } from './ui-state-store.js';
import { renderUiContent } from './ui-content.js';

// Explicit PM reports and raw terminal packets share one immutable window version.
// The terminal runtime retains responsibility for buffering and snapshot ordering.
export function createPmWindowModel() {
  const model = createUiStateStore({ windows: {}, changedWindowId: null, resyncRequest: null });
  let requestSequence = 0;
  const requests = new Map();
  const validVersion = version => version && typeof version.epoch === 'string' && version.epoch.length > 0
    && Number.isSafeInteger(version.revision) && version.revision > 0;
  function requestPmReports(windowId, resync = false) {
    const state = model.read().windows[windowId];
    if (!state?.sessionId) return null;
    let request = requests.get(windowId);
    if (!request) {
      request = { applied: 0, issued: 0, force: null, needsSnapshot: true };
      requests.set(windowId, request);
    }
    if (resync) { request.needsSnapshot = true; request.force = null; }
    if (request.needsSnapshot && request.force !== null) {
      return { kind: 'load_pm_reports', id: windowId, request_id: request.force, version: null };
    }
    const requestId = ++requestSequence;
    request.issued = requestId;
    if (request.needsSnapshot) request.force = requestId;
    return { kind: 'load_pm_reports', id: windowId, request_id: requestId,
      version: request.needsSnapshot ? null : state.reportsVersion };
  }
  function emptyWindow(windowId, sessionId = null) {
    return { windowId, sessionId, revision: 0, terminal: null,
      reportsVersion: null, reports: [], current: null, error: null, loaded: false };
  }
  function bindPmWindowState(windowId, sessionId) {
    model.update(state => {
      const previous = state.windows[windowId];
      if (previous && previous.sessionId === sessionId) return state;
      requests.delete(windowId);
      const next = previous?.sessionId == null ? { ...emptyWindow(windowId, sessionId), terminal: previous?.terminal || null }
        : emptyWindow(windowId, sessionId);
      return { ...state, windows: { ...state.windows, [windowId]: { ...next, revision: (previous?.revision || 0) + 1 } }, changedWindowId: windowId };
    });
  }
  function applyPmWindowReceiveEvent(event) {
    let needsResync = false;
    model.update(state => {
      const previous = state.windows[event.id];
      let next = previous || emptyWindow(event.id);
      if (event.kind === 'pm_reports') {
        if (!previous || event.session_id !== previous.sessionId) return state;
        const request = requests.get(event.id);
        if (!request || !Number.isSafeInteger(event.request_id) || event.request_id <= request.applied
          || event.request_id > request.issued || !validVersion(event.version)) return state;
        const version = previous.reportsVersion;
        if (request.force !== null) {
          if (event.request_id !== request.force || event.snapshot !== true) return state;
          if (version?.epoch === event.version.epoch && event.version.revision < version.revision) return state;
        } else {
          if (event.snapshot !== false) return state;
          if (version?.epoch === event.version.epoch && event.version.revision <= version.revision) return state;
          if (!version || version.epoch !== event.version.epoch || event.version.revision !== version.revision + 1) {
            request.needsSnapshot = true;
            needsResync = true;
            return { ...state, resyncRequest: { windowId: event.id,
              expected: version, received: event.version } };
          }
        }
        request.applied = event.request_id;
        request.force = null;
        request.needsSnapshot = false;
        if (version?.epoch === event.version.epoch && version.revision === event.version.revision) return state;
        next = { ...next, reportsVersion: event.version, error: event.error || null, loaded: true,
          current: event.error ? previous.current : (event.current || null),
          reports: event.error ? previous.reports : (event.reports || []).map(report => ({ ...report,
            content: { type: 'markdown', body: report.body || '', body_html: report.body_html || '' } })) };
      } else if (event.kind === 'terminal_output' || event.kind === 'terminal_snapshot') {
        next = { ...next, terminal: { kind: event.kind, dataBase64: event.data_base64 } };
      } else return state;
      return { ...state, windows: { ...state.windows, [event.id]: { ...next, revision: next.revision + 1 } }, changedWindowId: event.id };
    });
    return needsResync;
  }
  function removePmWindowState(windowId) {
    requests.delete(windowId);
    model.update(state => {
      const { [windowId]: removed, ...windows } = state.windows;
      return removed ? { ...state, windows, changedWindowId: windowId } : state;
    });
  }
  function readPmWindowState() { return model.read(); }
  function subscribePmWindowState(select, render) { return model.subscribe(select, render); }
  return { requestPmReports, bindPmWindowState, applyPmWindowReceiveEvent, removePmWindowState, readPmWindowState, subscribePmWindowState };
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
  let scrollTop = 0;
  let followTail = true;
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
  currentCard.setAttribute('aria-label', 'PM status');
  const currentDetails = element('details', '');
  const currentSummary = element('summary', 'pm-chat__current-summary');
  const currentContent = element('div', 'pm-chat__current-content');
  currentDetails.append(currentSummary, currentContent);
  currentCard.append(currentDetails);
  const historyHeader = element('div', 'pm-chat__history-header');
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
  historyHeader.append(element('h2', '', 'Reports'), filter);
  body.append(currentCard, historyHeader, notice, transcript);
  root.append(toolbar, body);

  function rememberScroll() {
    // Hidden tabs/windows have no layout; their zero offsets are not user intent.
    if (!body.clientHeight) return;
    scrollTop = body.scrollTop;
    followTail = body.scrollHeight - body.clientHeight - scrollTop <= 32;
  }
  function restoreScroll() {
    if (disposed || !body.clientHeight) return;
    body.scrollTop = followTail ? body.scrollHeight : scrollTop;
  }
  body.addEventListener('scroll', rememberScroll);
  const resizeObserver = typeof document.defaultView.ResizeObserver === 'function'
    ? new document.defaultView.ResizeObserver(restoreScroll) : null;
  resizeObserver?.observe(body);

  function showLogs(value) {
    if (value && !logs) rememberScroll();
    logs = value;
    body.hidden = logs;
    root.classList.toggle('pm-chat--logs', logs);
    reportsButton.setAttribute('aria-pressed', String(!logs));
    logButton.setAttribute('aria-pressed', String(logs));
    onLogVisibility(logs);
    if (!logs) restoreScroll();
  }
  const showReports = () => showLogs(false);
  const showLog = () => showLogs(true);
  const filterReports = () => update(latestState);
  function renderCurrent(current) {
    const nextDigest = JSON.stringify(current ? [current.status, current.facts, current.stale_minutes] : null);
    if (currentDigest === nextDigest) return;
    currentDigest = nextDigest;
    const authored = element('section', 'pm-chat__current-authored');
    authored.setAttribute('aria-label', 'PM notes');
    authored.append(element('h3', '', 'PM notes'));
    const time = value => {
      const node = element('time', '', value ? new Date(value).toLocaleString() : 'Unknown');
      node.title = node.textContent;
      if (value) node.setAttribute('datetime', value);
      return node;
    };
    const status = current?.status;
    const working = status?.items.filter(item => item.kind !== 'waiting').length || 0;
    const waiting = status?.items.filter(item => item.kind === 'waiting').length || 0;
    const heading = element('span', 'pm-chat__current-heading');
    heading.append(element('strong', '', 'PM status'), element('span', '', `${working} working · ${waiting} waiting`));
    const next = element('span', 'pm-chat__current-next');
    const trigger = element('span', '', status ? `Next: ${status.next_trigger}` : 'No current status reported yet.');
    trigger.title = trigger.textContent;
    next.append(trigger);
    if (status) next.append(element('span', '', ` · User action: ${status.user_action ? 'Requested' : 'None'}`));
    const meta = element('span', 'pm-chat__current-meta', 'Updated: ');
    meta.append(time(status?.updated_at));
    if (current?.stale_minutes != null) {
      meta.append(element('span', 'pm-chat__current-stale', ` · PM has not updated for ${current.stale_minutes} minutes`));
    }
    currentSummary.replaceChildren(heading, next, meta);
    if (status) {
      const items = element('ul', '');
      for (const item of status.items) {
        const row = element('li', '', `${item.kind === 'waiting' ? 'Waiting' : 'Working'}: ${item.text} — Started `);
        row.append(time(item.started_at));
        items.append(row);
      }
      authored.append(items, element('p', '', `Next: ${status.next_trigger}`),
        element('p', '', `User action: ${status.user_action || 'None'}`));
      const updated = element('p', 'pm-chat__current-meta', 'Updated: ');
      updated.append(time(status.updated_at));
      authored.append(updated);
      if (current.stale_minutes != null) {
        const stale = element('p', 'pm-chat__current-stale', `PM has not updated for ${current.stale_minutes} minutes`);
        stale.setAttribute('role', 'status');
        authored.append(stale);
      }
    } else authored.append(element('p', '', 'No current status reported yet.'));
    const automatic = element('section', 'pm-chat__current-facts');
    automatic.setAttribute('aria-label', 'Live facts');
    automatic.append(element('h3', '', 'Live facts'));
    const facts = current?.facts;
    if (!facts) automatic.append(element('p', '', 'Loading Monitor facts…'));
    else {
      const issueLabel = issue => `${issue.issue_number ? `#${issue.issue_number}` : 'Unassigned issue'} ${issue.title || 'Unknown title'}`;
      automatic.append(element('p', 'pm-chat__current-meta', `Monitor: ${facts.source === 'daemon' ? 'Connected' : facts.source === 'degraded_cache' ? 'Cached (Monitor disconnected)' : 'Unavailable'}`));
      if (facts.error) automatic.append(element('p', 'pm-chat__current-stale', facts.error));
      const windows = element('ul', '');
      for (const item of facts.active_windows || []) {
        const row = element('li', '', `${issueLabel(item)} — Last activity `);
        row.append(time(item.last_activity_at));
        windows.append(row);
      }
      automatic.append(element('p', '', `Active windows: ${(facts.active_windows || []).length}${facts.inventory_incomplete ? ' (incomplete observation)' : ''}`), windows,
        element('p', '', `Queue head: ${facts.queue_head ? issueLabel(facts.queue_head) : facts.source === 'unavailable' ? 'Unknown' : 'None'}`),
        element('p', '', `needs_human: ${facts.needs_human_count ?? 'Unknown'}`),
        element('p', '', `Release: ${facts.release ? `#${facts.release.number} ${facts.release.title}` : facts.release_observed_at ? 'No active PR' : 'Unknown'}`),
        element('p', '', `Pending update: ${facts.pending_update ? `v${facts.pending_update}` : facts.update_known ? 'None' : 'Unknown'}`));
      if (facts.release_observed_at) {
        const observed = element('p', 'pm-chat__current-meta', 'Release PR observed: ');
        observed.append(time(facts.release_observed_at));
        automatic.append(observed);
      }
    }
    currentContent.replaceChildren(authored, automatic);
  }
  function update(state) {
    if (disposed || !state) return;
    setSession(state.sessionId);
    latestState = state;
    root.dataset.stateVersion = String(state.revision);
    if (transcript.childElementCount) rememberScroll();
    renderCurrent(state.current);
    const reports = state.reports;
    const visibleReports = kindSelect.value === 'all' ? reports : reports.filter(report => report.kind === kindSelect.value);
    const nextDigest = JSON.stringify([kindSelect.value, visibleReports]);
    if (digest !== nextDigest) {
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
        const content = renderUiContent(document, report.content, 'pm-chat__text');
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
      digest = nextDigest;
    }
    notice.textContent = state.error || (!state.loaded ? 'Loading PM reports…'
      : !reports.length ? 'No PM reports yet. Use the execution log to interact with PM.'
        : visibleReports.length ? '' : `No ${kindLabels[kindSelect.value]} reports yet.`);
    notice.hidden = !notice.textContent;
    restoreScroll();
  }
  function setSession(nextSession) {
    if (disposed || session === nextSession) return;
    session = nextSession;
    transcript.replaceChildren();
    body.scrollTop = 0;
    scrollTop = 0;
    followTail = true;
    digest = '';
    currentDigest = undefined;
    showLogs(false);
  }
  function dispose() {
    disposed = true;
    body.removeEventListener('scroll', rememberScroll);
    resizeObserver?.disconnect();
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
  return { update, setSession, showReports, dispose };
}

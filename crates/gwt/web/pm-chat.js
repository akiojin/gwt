import { createUiStateStore } from './ui-state-store.js';
import { renderUiContent } from './ui-content.js';

function reportDetailsMarkdown(structured) {
  return [structured.details, (structured.refs || []).map(ref => `- ${ref}`).join('\n')]
    .filter(Boolean).join('\n\n');
}

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
      reportsVersion: null, reports: [], current: null, error: null, loaded: false,
      requestTasks: null, requestTasksError: null };
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
        const taskOverview = event.current?.request_tasks;
        const taskError = taskOverview?.state === 'error' ? taskOverview.message
          : (!taskOverview ? event.error || 'Request tasks are unavailable. Reload this page to try again.' : null);
        next = { ...next, reportsVersion: event.version, error: event.error || null, loaded: true,
          requestTasks: taskError ? previous.requestTasks : (taskOverview || null),
          requestTasksError: taskError,
          current: event.error ? previous.current : (event.current || null),
          reports: event.error ? previous.reports : (event.reports || []).map(report => ({ ...report,
            content: report.structured ? { type: 'text', body: report.body || '' }
              : { type: 'markdown', body: report.body || '', body_html: report.body_html || '' },
            detailsContent: report.structured ? { type: 'markdown', body: reportDetailsMarkdown(report.structured),
              body_html: report.details_html || '' } : null })) };
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
  let activeView = 'tasks';
  let includeCompleted = false;
  let tasksDigest = '';
  const taskRows = new Map();
  let digest = '';
  let currentDigest;
  let latestState;
  let scrollTop = 0;
  let followTail = true;
  const expandedReports = new Set();
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
  const tasksButton = element('button', '', 'Tasks');
  const reportsButton = element('button', '', 'Reports');
  const logButton = element('button', '', 'Execution log');
  tasksButton.type = reportsButton.type = logButton.type = 'button';
  toolbar.append(tasksButton, reportsButton, logButton);
  const tasksBody = element('section', 'pm-chat__tasks');
  tasksBody.setAttribute('aria-label', 'Request tasks');
  const tasksHeader = element('div', 'pm-chat__history-header');
  const completedButton = element('button', '', 'Include completed');
  completedButton.type = 'button';
  completedButton.setAttribute('aria-pressed', 'false');
  tasksHeader.append(element('h2', '', 'Tasks'), completedButton);
  const tasksNotice = element('p', 'pm-chat__notice');
  tasksNotice.setAttribute('role', 'status');
  const taskList = element('div', 'pm-chat__task-list');
  tasksBody.append(tasksHeader, tasksNotice, taskList);
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
  root.append(toolbar, tasksBody, body);

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

  function showView(value) {
    if (activeView === 'reports') rememberScroll();
    activeView = value;
    logs = value === 'log';
    body.hidden = value !== 'reports';
    tasksBody.hidden = value !== 'tasks';
    root.classList.toggle('pm-chat--logs', logs);
    tasksButton.setAttribute('aria-pressed', String(value === 'tasks'));
    reportsButton.setAttribute('aria-pressed', String(value === 'reports'));
    logButton.setAttribute('aria-pressed', String(logs));
    onLogVisibility(logs);
    if (value === 'reports') restoreScroll();
  }
  const showTasks = () => showView('tasks');
  const showReports = () => showView('reports');
  const showLog = () => showView('log');
  const filterReports = () => update(latestState);
  const filterTasks = () => {
    includeCompleted = !includeCompleted;
    completedButton.setAttribute('aria-pressed', String(includeCompleted));
    renderTasks(latestState);
  };
  function renderTasks(state) {
    const overview = state?.requestTasks;
    const error = state?.requestTasksError;
    const rows = overview?.state === 'ready' ? overview.tasks : [];
    const visible = rows.filter(task => includeCompleted || !['completed', 'cancelled'].includes(task.status));
    const nextDigest = JSON.stringify([includeCompleted, rows]);
    if (nextDigest !== tasksDigest) {
      tasksDigest = nextDigest;
      const parents = new Map(rows.map(task => [task.key, task.title]));
      const labels = { pending: 'Pending', in_progress: 'In progress', waiting: 'Waiting', completed: 'Completed', cancelled: 'Cancelled' };
      const nodes = visible.map(task => {
        let row = taskRows.get(task.key);
        if (!row) {
          row = element('article', 'pm-chat__task');
          row.dataset.taskKey = task.key;
          taskRows.set(task.key, row);
        }
        const digest = JSON.stringify([task, parents.get(task.parent_key)]);
        if (row.dataset.digest !== digest) {
          row.dataset.digest = digest;
          row.classList.toggle('pm-chat__task--child', !!task.parent_key);
          const title = element('h3', '', task.title);
          const meta = element('p', 'pm-chat__task-meta', `${labels[task.status] || 'Unknown'} · ${task.assignee || 'Unassigned'}`);
          const fields = [title, meta];
          if (task.parent_key) fields.push(element('p', 'pm-chat__task-meta', `Part of: ${parents.get(task.parent_key) || 'Parent outside this preview'}`));
          if (task.review_required) fields.push(element('p', 'pm-chat__task-review', 'PM reassessment required'));
          if (task.wait_reason) fields.push(element('p', '', `Waiting: ${task.wait_reason}`));
          if (task.next_action) fields.push(element('p', '', `Next: ${task.next_action}`));
          row.replaceChildren(...fields);
        }
        return row;
      });
      // Keep stable rows attached: report-only polls cannot disturb reading
      // position or selection inside an unchanged Task summary.
      if (nodes.length !== taskList.children.length || nodes.some((node, index) => taskList.children[index] !== node)) {
        taskList.replaceChildren(...nodes);
      }
      const keys = new Set(rows.map(task => task.key));
      for (const key of taskRows.keys()) if (!keys.has(key)) taskRows.delete(key);
    }
    const pending = overview?.pending_intake || 0;
    const summary = overview?.state === 'ready'
      ? `${visible.length} of ${overview.total} recorded tasks shown${overview.total > rows.length ? ' (preview limited to 50; unfinished first)' : ''}. ${pending} request${pending === 1 ? '' : 's'} await classification.${!visible.length ? ' No unfinished tasks in this preview.' : ''}`
      : overview?.state === 'absent' ? 'No request tasks have been recorded yet.' : 'Loading request tasks…';
    const noticeText = error
      ? `Task data could not be refreshed.${overview ? ' Showing the last confirmed list.' : ''} ${error} Reopen Tasks after the store is recovered.`
      : summary;
    if (tasksNotice.textContent !== noticeText) tasksNotice.textContent = noticeText;
  }
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
    renderTasks(state);
    if (transcript.childElementCount) rememberScroll();
    renderCurrent(state.current);
    const reports = state.reports;
    const visibleReports = kindSelect.value === 'all' ? reports : reports.filter(report => report.kind === kindSelect.value);
    const nextDigest = JSON.stringify([kindSelect.value, visibleReports]);
    if (digest !== nextDigest) {
      for (const row of transcript.children) {
        const details = row.querySelector('details');
        if (details?.hasAttribute('open')) expandedReports.add(row.dataset.reportId);
        else expandedReports.delete(row.dataset.reportId);
      }
      const nodes = visibleReports.map(report => {
        const row = element('article', 'pm-chat__message');
        row.dataset.reportId = report.id;
        const qa = report.kind === 'qa';
        const header = element('div', 'pm-chat__author');
        const time = element('time', '', new Date(report.created_at).toLocaleString());
        time.setAttribute('datetime', report.created_at);
        const copy = element('button', '', 'Copy');
        copy.type = 'button';
        copy.setAttribute('aria-label', 'Copy report');
        copy.addEventListener('click', async () => {
          try {
            await copyText(qa ? `Q: ${report.question || ''}\n\nA:\n${report.body}`
              : [report.body, report.structured ? reportDetailsMarkdown(report.structured) : ''].filter(Boolean).join('\n\n'));
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
        } else {
          row.append(content);
          if (report.structured && (report.structured.details.trim() || report.structured.refs.length)) {
            const details = element('details', '');
            if (expandedReports.has(report.id)) details.setAttribute('open', '');
            details.append(element('summary', '', 'Details'),
              renderUiContent(document, report.detailsContent, 'pm-chat__text'));
            row.append(details);
          }
        }
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
    expandedReports.clear();
    taskRows.clear();
    taskList.replaceChildren();
    tasksDigest = '';
    includeCompleted = false;
    completedButton.setAttribute('aria-pressed', 'false');
    transcript.replaceChildren();
    body.scrollTop = 0;
    scrollTop = 0;
    followTail = true;
    digest = '';
    currentDigest = undefined;
    showTasks();
  }
  function dispose() {
    disposed = true;
    expandedReports.clear();
    body.removeEventListener('scroll', rememberScroll);
    resizeObserver?.disconnect();
    tasksButton.removeEventListener('click', showTasks);
    completedButton.removeEventListener('click', filterTasks);
    reportsButton.removeEventListener('click', showReports);
    logButton.removeEventListener('click', showLog);
    kindSelect.removeEventListener('change', filterReports);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs');
  }
  reportsButton.addEventListener('click', showReports);
  tasksButton.addEventListener('click', showTasks);
  completedButton.addEventListener('click', filterTasks);
  logButton.addEventListener('click', showLog);
  kindSelect.addEventListener('change', filterReports);
  update({ sessionId, revision: 0, reports: [], loaded: false });
  showTasks();
  return { update, setSession, showTasks, showReports, dispose };
}

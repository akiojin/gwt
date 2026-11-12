import { createUiStateStore } from './ui-state-store.js';
import { renderUiContent } from './ui-content.js';
import { createTaskHome } from './task-home.js';

function reportDetailsMarkdown(structured) {
  return [structured.details, (structured.refs || []).map(ref => `- ${ref}`).join('\n')]
    .filter(Boolean).join('\n\n');
}

// Stored summary text participates in journal replay validation. Format the
// choices for display without rewriting that durable body or its receipt.
function reportDisplayText(report) {
  const summary = report.structured?.summary;
  const decision = report.kind === 'decision' && summary?.decision;
  if (!decision) return report.body || '';
  const lines = [summary.result];
  for (const [label, key] of [['Impact', 'impact'], ['Owner', 'owner'], ['Next', 'next_action']]) {
    if (summary[key]) lines.push(`${label}: ${summary[key]}`);
  }
  lines.push(`Decision: ${decision.question}`, `Recommended: ${decision.recommendation} (${decision.reason})`,
    ...decision.options.map((option, index) => `${index + 1}. ${option}`),
    'Reply with the option number, or Other: your own answer.');
  return lines.join('\n');
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
      requestTasks: null, requestTasksError: null, conversation: null, chatSend: null };
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
        // A Current-card failure fences only that card; saved reports still apply (#6950).
        const currentFailed = Boolean(event.error || event.current_error);
        next = { ...next, reportsVersion: event.version, error: event.error || event.current_error || null, loaded: true,
          requestTasks: taskError ? previous.requestTasks : (taskOverview || null),
          requestTasksError: taskError,
          current: currentFailed ? previous.current : (event.current || null),
          reports: event.error ? previous.reports : (event.reports || []).map(report => ({ ...report,
            content: report.structured ? { type: 'text', body: reportDisplayText(report) }
              : { type: 'markdown', body: report.body || '', body_html: report.body_html || '' },
            detailsContent: report.structured ? { type: 'markdown', body: reportDetailsMarkdown(report.structured),
              body_html: report.details_html || '' } : null })) };
      } else if (event.kind === 'pm_conversation') {
        // The host already resolved the session's native record and drops reads
        // of a replaced PM; this guard only keeps a late reply off a rebound window.
        if (!previous || event.session_id !== previous.sessionId || !event.snapshot) return state;
        const snapshot = event.snapshot;
        const conversation = { availability: snapshot.availability || 'unavailable', detail: snapshot.detail || null,
          messages: (snapshot.messages || []).map(message => ({ id: message.id, role: message.role,
            content: message.role === 'assistant'
              ? { type: 'markdown', body: message.text || '', body_html: message.text_html || '' }
              : { type: 'text', body: message.text || '' } })) };
        if (JSON.stringify(conversation) === JSON.stringify(previous.conversation)) return state;
        next = { ...next, conversation };
      } else if (event.kind === 'pane_send_result') {
        // Palette/CLI replies and expired attempts cannot settle a Chat send.
        // Session refusals omit the window, but still carry the exact request ID.
        if (!Number.isSafeInteger(event.request_id)) return state;
        const windowId = Object.keys(state.windows).find(id => state.windows[id].chatSend?.status === 'pending'
          && state.windows[id].chatSend.sequence === event.request_id
          && (event.window_id == null || event.window_id === id));
        const pending = state.windows[windowId];
        if (pending?.chatSend?.status !== 'pending') return state;
        const chatSend = { sequence: pending.chatSend.sequence, status: event.ok ? 'sent' : 'failed',
          error: event.ok ? null : event.error || 'The PM pane refused the message.' };
        return { ...state, windows: { ...state.windows, [windowId]: { ...pending, chatSend,
          revision: pending.revision + 1 } }, changedWindowId: windowId };
      } else if (event.kind === 'terminal_output' || event.kind === 'terminal_snapshot') {
        next = { ...next, terminal: { kind: event.kind, dataBase64: event.data_base64 } };
      } else return state;
      return { ...state, windows: { ...state.windows, [event.id]: { ...next, revision: next.revision + 1 } }, changedWindowId: event.id };
    });
    return needsResync;
  }
  let chatSendSequence = 0;
  function setChatSend(windowId, accept, chatSend) {
    let changed = false;
    model.update(state => {
      const previous = state.windows[windowId];
      if (!previous?.sessionId || !accept(previous.chatSend)) return state;
      changed = true;
      return { ...state, windows: { ...state.windows, [windowId]: { ...previous, chatSend,
        revision: previous.revision + 1 } }, changedWindowId: windowId };
    });
    return changed;
  }
  // The existing pane input path pastes the body and submits it on its own.
  function requestPmChatSend(windowId, text) {
    const sessionId = model.read().windows[windowId]?.sessionId;
    if (!sessionId || !String(text ?? '').trim()) return null;
    const sequence = ++chatSendSequence;
    if (!setChatSend(windowId, current => current?.status !== 'pending', { sequence, status: 'pending', error: null })) return null;
    return { kind: 'pane_send_input', session_id: sessionId, text: `${text}\r`, request_id: sequence };
  }
  function failPendingChatSend(windowId, error, sequence = null) {
    const current = model.read().windows[windowId]?.chatSend;
    return setChatSend(windowId, send => send?.status === 'pending' && (sequence === null || send.sequence === sequence),
      { sequence: current?.sequence, status: 'failed', error });
  }
  const markPmChatSendUnavailable = windowId => failPendingChatSend(windowId,
    'The PM connection is not connected. Reconnect, then send again.');
  const expirePmChatSend = (windowId, sequence = null) => failPendingChatSend(windowId,
    'Delivery was not confirmed. Check the execution log before sending again.', sequence);
  function requestPmConversation(windowId) {
    return model.read().windows[windowId]?.sessionId ? { kind: 'load_pm_conversation', id: windowId } : null;
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
  return { requestPmReports, requestPmConversation, requestPmChatSend, markPmChatSendUnavailable, expirePmChatSend, bindPmWindowState, applyPmWindowReceiveEvent, removePmWindowState, readPmWindowState, subscribePmWindowState };
}

// Read-only report view. The caller owns polling and the unchanged terminal.
export function createPmChat({ document, root, sessionId, onLogVisibility, onChatVisible = () => {}, onSend = () => {},
  copyText = text => document.defaultView.navigator.clipboard.writeText(text) }) {
  let session = sessionId;
  let disposed = false;
  let logs = false;
  let activeView = 'home';
  let includeCompleted = false;
  let tasksDigest = '';
  let chatDigest;
  let chatSend = null;
  const taskRows = new Map();
  let digest = '';
  let currentDigest;
  let latestState;
  let scrollTop = 0;
  let followTail = true;
  const expandedReports = new Set();
  const expandedChatGroups = new Set();
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
  const homeButton = element('button', '', 'Home');
  const chatButton = element('button', '', 'Chat');
  const tasksButton = element('button', '', 'Tasks');
  const reportsButton = element('button', '', 'Reports');
  const logButton = element('button', '', 'Execution log');
  homeButton.type = chatButton.type = tasksButton.type = reportsButton.type = logButton.type = 'button';
  toolbar.append(homeButton, chatButton, tasksButton, reportsButton, logButton);
  // SPEC #7007: the task-centered Home island is the entry view. Its actions
  // reuse the existing surfaces: creating a task goes through the PM chat
  // composer (the request-task intake path), decisions open the Reports
  // decision filter, and progress is the Tasks view.
  const taskHome = createTaskHome({ document,
    onCreateTask: () => { showView('chat'); draft.focus(); },
    onReviewDecisions: () => {
      kindSelect.value = 'decision';
      showView('reports');
      update(latestState);
    },
    onViewProgress: () => showView('tasks'),
  });
  const chatBody = element('section', 'pm-chat__chat');
  chatBody.setAttribute('aria-label', 'PM conversation');
  const chatNotice = element('p', 'pm-chat__notice');
  chatNotice.setAttribute('role', 'status');
  const chatLog = element('div', 'pm-chat__conversation');
  chatLog.setAttribute('role', 'log');
  chatLog.setAttribute('aria-label', 'PM conversation messages');
  const composer = element('form', 'pm-chat__composer');
  composer.setAttribute('aria-label', 'Message PM');
  const draft = element('textarea', 'pm-chat__draft');
  draft.setAttribute('aria-label', 'Message to PM');
  draft.rows = 2;
  draft.placeholder = 'Message PM (Enter to send, Shift+Enter for a new line)';
  const sendButton = element('button', '', 'Send');
  sendButton.type = 'submit';
  const composerStatus = element('p', 'pm-chat__composer-status');
  composerStatus.setAttribute('aria-live', 'polite');
  composer.append(draft, sendButton, composerStatus);
  chatBody.append(chatNotice, chatLog, composer);
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
  root.append(toolbar, taskHome.element, tasksBody, body, chatBody);

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
    chatBody.hidden = value !== 'chat';
    taskHome.element.hidden = value !== 'home';
    root.classList.toggle('pm-chat--logs', logs);
    homeButton.setAttribute('aria-pressed', String(value === 'home'));
    chatButton.setAttribute('aria-pressed', String(value === 'chat'));
    tasksButton.setAttribute('aria-pressed', String(value === 'tasks'));
    reportsButton.setAttribute('aria-pressed', String(value === 'reports'));
    logButton.setAttribute('aria-pressed', String(logs));
    onLogVisibility(logs);
    if (value === 'reports') restoreScroll();
    if (value === 'chat') {
      chatLog.scrollTop = chatLog.scrollHeight;
      onChatVisible();
    }
  }
  const showHome = () => showView('home');
  const showChat = () => showView('chat');
  const showTasks = () => showView('tasks');
  const showReports = () => showView('reports');
  const showLog = () => showView('log');
  const filterReports = () => update(latestState);
  const filterTasks = () => {
    includeCompleted = !includeCompleted;
    completedButton.setAttribute('aria-pressed', String(includeCompleted));
    renderTasks(latestState);
  };
  // Derive the Home island state from the same window snapshot the other
  // views read, so every surface reports the same numbers (SPEC #7007 AC-6).
  // Unknown stays unknown: decisions are counted only once reports have
  // loaded, and no completed-today source exists yet, so it stays null.
  function renderHome(state) {
    const overview = state?.requestTasks ?? null;
    const decisionsWaiting = state?.loaded
      ? (state.reports || []).filter(report => report.kind === 'decision').length : null;
    taskHome.receive({ overview, decisionsWaiting, completedToday: null,
      stale: Boolean(state?.requestTasksError) && overview !== null });
  }
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
      ? `Task data could not be refreshed.${overview ? ` Showing the last confirmed list. Last confirmed summary (stale): ${summary}` : ''} ${error} Reopen Tasks after the store is recovered.`
      : summary;
    if (tasksNotice.textContent !== noticeText) tasksNotice.textContent = noticeText;
  }
  function renderChat(conversation) {
    const nextDigest = JSON.stringify(conversation);
    if (chatDigest === nextDigest) return;
    chatDigest = nextDigest;
    const following = chatLog.scrollHeight - chatLog.clientHeight - chatLog.scrollTop <= 32;
    const messages = conversation?.availability === 'ready' ? conversation.messages : [];
    const bubbleFor = message => {
      const user = message.role === 'user';
      const bubble = element('article', `pm-chat__bubble pm-chat__bubble--${user ? 'user' : 'assistant'}`);
      bubble.dataset.messageId = message.id;
      bubble.dataset.role = user ? 'user' : 'assistant';
      bubble.setAttribute('aria-label', user ? 'You' : 'PM');
      bubble.append(renderUiContent(document, message.content, 'pm-chat__text'));
      return bubble;
    };
    // Within one PM turn only the latest PM message stays open; the earlier ones fold
    // behind a disclosure keyed by the turn's first message. The open state is read from
    // the DOM at render time because the toggle event fires asynchronously.
    for (const group of chatLog.querySelectorAll('details.pm-chat__collapsed')) {
      if (group.open) expandedChatGroups.add(group.dataset.groupKey); else expandedChatGroups.delete(group.dataset.groupKey);
    }
    const nodes = [];
    for (let start = 0; start < messages.length;) {
      let end = start + 1;
      if (messages[start].role !== 'user') while (end < messages.length && messages[end].role !== 'user') end += 1;
      const earlier = messages.slice(start, end - 1);
      if (earlier.length) {
        const key = earlier[0].id;
        const group = element('details', 'pm-chat__collapsed');
        group.dataset.groupKey = key;
        group.open = expandedChatGroups.has(key);
        group.append(element('summary', 'pm-chat__collapsed-summary',
          `${earlier.length} earlier message${earlier.length === 1 ? '' : 's'}`), ...earlier.map(bubbleFor));
        nodes.push(group);
      }
      nodes.push(bubbleFor(messages[end - 1]));
      start = end;
    }
    chatLog.replaceChildren(...nodes);
    chatNotice.textContent = !conversation ? 'Loading PM conversation…'
      : conversation.availability !== 'ready' ? conversation.detail || 'The PM conversation is unavailable.'
        : messages.length ? '' : 'No conversation yet. Send a message to PM below.';
    chatNotice.hidden = !chatNotice.textContent;
    if (following) chatLog.scrollTop = chatLog.scrollHeight;
  }
  function syncComposer() {
    const pending = chatSend?.status === 'pending';
    // The sent text stays visible but frozen, so a confirmation clears only what was sent.
    draft.readOnly = pending;
    sendButton.disabled = pending || !draft.value.trim();
    const text = pending ? 'Sending…'
      : chatSend?.status === 'failed' ? `Not sent: ${String(chatSend.error).replace(/[.\s]+$/, '')}. Your message is kept.` : '';
    if (composerStatus.textContent !== text) composerStatus.textContent = text;
  }
  function renderComposer(next) {
    // A confirmed delivery clears only the draft that was sent.
    if (next?.status === 'sent' && chatSend?.status === 'pending' && chatSend.sequence === next.sequence) draft.value = '';
    chatSend = next || null;
    syncComposer();
  }
  function submitDraft(event) {
    event?.preventDefault();
    if (disposed || chatSend?.status === 'pending' || !draft.value.trim()) return;
    onSend(draft.value);
  }
  function draftKeydown(event) {
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
    submitDraft(event);
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
    if (!facts) automatic.append(element('p', '', 'Loading Issues facts…'));
    else {
      const issueLabel = issue => `${issue.issue_number ? `#${issue.issue_number}` : 'Unassigned issue'} ${issue.title || 'Unknown title'}`;
      automatic.append(element('p', 'pm-chat__current-meta', `Issues: ${facts.source === 'daemon' ? 'Connected' : facts.source === 'degraded_cache' ? 'Cached (Issues disconnected)' : 'Unavailable'}`));
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
    renderHome(state);
    renderTasks(state);
    renderChat(state.conversation);
    renderComposer(state.chatSend);
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
              : [report.content.body, report.structured ? reportDetailsMarkdown(report.structured) : ''].filter(Boolean).join('\n\n'));
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
    chatDigest = undefined;
    expandedChatGroups.clear();
    chatLog.replaceChildren();
    draft.value = '';
    renderComposer(null);
    includeCompleted = false;
    completedButton.setAttribute('aria-pressed', 'false');
    transcript.replaceChildren();
    body.scrollTop = 0;
    scrollTop = 0;
    followTail = true;
    digest = '';
    currentDigest = undefined;
    showHome();
  }
  function dispose() {
    disposed = true;
    expandedReports.clear();
    body.removeEventListener('scroll', rememberScroll);
    resizeObserver?.disconnect();
    taskHome.dispose();
    homeButton.removeEventListener('click', showHome);
    chatButton.removeEventListener('click', showChat);
    composer.removeEventListener('submit', submitDraft);
    draft.removeEventListener('keydown', draftKeydown);
    draft.removeEventListener('input', syncComposer);
    tasksButton.removeEventListener('click', showTasks);
    completedButton.removeEventListener('click', filterTasks);
    reportsButton.removeEventListener('click', showReports);
    logButton.removeEventListener('click', showLog);
    kindSelect.removeEventListener('change', filterReports);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs');
  }
  homeButton.addEventListener('click', showHome);
  chatButton.addEventListener('click', showChat);
  composer.addEventListener('submit', submitDraft);
  draft.addEventListener('keydown', draftKeydown);
  draft.addEventListener('input', syncComposer);
  reportsButton.addEventListener('click', showReports);
  tasksButton.addEventListener('click', showTasks);
  completedButton.addEventListener('click', filterTasks);
  logButton.addEventListener('click', showLog);
  kindSelect.addEventListener('change', filterReports);
  update({ sessionId, revision: 0, reports: [], loaded: false });
  showHome();
  return { update, setSession, showHome, showChat, showTasks, showReports, dispose,
    isChatVisible: () => !disposed && activeView === 'chat' };
}

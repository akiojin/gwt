import { createUiStateStore } from './ui-state-store.js';
import { renderUiContent } from './ui-content.js';
import { createTaskHome } from './task-home.js';
import { createTaskDecisions } from './task-decisions.js';

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
  let paneResultSequence = 0;
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
      requestTasks: null, requestTasksError: null, taskDetail: null, taskCancel: null, conversation: null, chatSend: null, paneResult: null };
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
        if (next.taskDetail?.data && (taskError || (taskOverview?.revision > next.taskDetail.revision))) {
          next.taskDetail = { ...next.taskDetail, stale: true };
        }
      } else if (event.kind === 'pm_request_task') {
        const cancellation = previous?.taskCancel;
        const selected = previous?.taskDetail;
        if (cancellation?.status === 'pending' && event.request_id === cancellation.requestId) {
          const result = event.result;
          const recorded = result?.state === 'ready' && result.receipt?.action === 'cancel'
            && result.receipt.operation_id === cancellation.operationId
            && Number.isSafeInteger(result.receipt.revision) && result.receipt.revision > cancellation.revision;
          if (!recorded && result?.state !== 'error') return state;
          next = { ...next, taskCancel: { ...cancellation, status: recorded ? 'recorded' : 'error',
            error: recorded ? null : result.message || 'Cancellation was not confirmed. Reload details before trying again.' },
            taskDetail: selected?.key === cancellation.key ? { ...selected, stale: true } : selected };
          return { ...state, windows: { ...state.windows, [event.id]: { ...next, revision: next.revision + 1 } }, changedWindowId: event.id };
        }
        if (!selected || selected.status !== 'loading' || event.request_id !== selected.requestId) return state;
        const result = event.result;
        if (result?.state === 'detail') {
          if (result.key !== selected.key) return state;
          if (!Number.isSafeInteger(result.revision)
            || result.revision < Math.max(previous.requestTasks?.revision || 0, selected.revision || 0)) {
            next = { ...next, taskDetail: { ...selected, status: 'error', stale: true,
              error: 'The task changed during this read. Reload details to get the current snapshot.' } };
          } else {
            next = { ...next, taskDetail: { ...selected, status: 'ready', stale: false,
              revision: result.revision, data: result.detail, error: null } };
          }
        } else if (result?.state === 'error') {
          next = { ...next, taskDetail: { ...selected, status: 'error', stale: Boolean(selected.data),
            error: result.message || 'Task details could not be read. Reload details to try again.' } };
        } else return state;
      } else if (event.kind === 'pm_conversation') {
        // The host already resolved the session's native record and drops reads
        // of a replaced PM; this guard only keeps a late reply off a rebound window.
        if (!previous || event.session_id !== previous.sessionId || !event.snapshot) return state;
        const snapshot = event.snapshot;
        const conversation = { availability: snapshot.availability || 'unavailable', detail: snapshot.detail || null,
          messages: (snapshot.messages || []).map(message => ({ id: message.id, role: message.role,
            sourceKey: message.role === 'user' && typeof message.source_key === 'string' ? message.source_key : null,
            content: message.role === 'assistant'
              ? { type: 'markdown', body: message.text || '', body_html: message.text_html || '' }
              : { type: 'text', body: message.text || '' } })) };
        if (JSON.stringify(conversation) === JSON.stringify(previous.conversation)) return state;
        next = { ...next, conversation };
      } else if (event.kind === 'pane_send_result') {
        // Every result is receipted on its window: results do not always
        // carry a correlation ID (palette / CLI sends), and the Decide
        // surface settles the answer it sent from the window receipt. Only a
        // result carrying the exact request ID may settle a Chat send, so
        // palette replies and expired attempts cannot clear the composer.
        // Session refusals omit the window, but still carry the request ID.
        const correlatedId = Number.isSafeInteger(event.request_id)
          ? Object.keys(state.windows).find(id => state.windows[id].chatSend?.status === 'pending'
            && state.windows[id].chatSend.sequence === event.request_id
            && (event.window_id == null || event.window_id === id))
          : undefined;
        const pendingIds = Object.keys(state.windows)
          .filter(id => state.windows[id].chatSend?.status === 'pending');
        const receiptId = correlatedId ?? event.window_id
          ?? (pendingIds.length === 1 ? pendingIds[0] : undefined);
        let next = state;
        const receiptWindow = receiptId != null ? state.windows[receiptId] : undefined;
        if (receiptWindow) {
          next = { ...state, windows: { ...state.windows, [receiptId]: { ...receiptWindow,
            paneResult: { receipt: ++paneResultSequence, ok: Boolean(event.ok),
              error: event.ok ? null : event.error || 'The PM pane refused the message.' },
            revision: receiptWindow.revision + 1 } }, changedWindowId: receiptId };
        }
        const pending = correlatedId != null ? next.windows[correlatedId] : undefined;
        if (pending?.chatSend?.status !== 'pending') return next;
        const chatSend = { sequence: pending.chatSend.sequence, status: event.ok ? 'sent' : 'failed',
          error: event.ok ? null : event.error || 'The PM pane refused the message.' };
        return { ...next, windows: { ...next.windows, [correlatedId]: { ...pending, chatSend,
          revision: pending.revision + 1 } }, changedWindowId: correlatedId };
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
  function requestPmTaskDetail(windowId, key) {
    const previous = model.read().windows[windowId];
    if (!previous?.sessionId) return null;
    const task = previous.requestTasks?.tasks?.find(task => task.key === key);
    if (key !== null && !task && previous.taskDetail?.key !== key) return null;
    const requestId = ++requestSequence;
    const retained = previous.taskDetail?.key === key ? previous.taskDetail : null;
    model.update(state => ({ ...state, windows: { ...state.windows, [windowId]: { ...previous,
      taskDetail: key === null ? null : { key, title: task?.title || retained?.title, requestId, status: 'loading', data: retained?.data || null,
        revision: retained?.revision ?? null, stale: Boolean(retained?.stale), error: null },
      revision: previous.revision + 1 } }, changedWindowId: windowId }));
    return key === null ? null : { kind: 'load_pm_request_task', id: windowId, request_id: requestId, task_key: key };
  }
  function markPmTaskDetailUnavailable(windowId, requestId = null) {
    const previous = model.read().windows[windowId], selected = previous?.taskDetail;
    if (selected?.status !== 'loading' || (requestId !== null && requestId !== selected.requestId)) return;
    model.update(state => ({ ...state, windows: { ...state.windows, [windowId]: { ...previous,
      taskDetail: { ...selected, status: 'error', stale: Boolean(selected.data),
        error: 'The read was not confirmed. Reconnect, then reload details.' },
      revision: previous.revision + 1 } }, changedWindowId: windowId }));
  }
  function requestPmTaskCancel(windowId, reason) {
    const previous = model.read().windows[windowId], selected = previous?.taskDetail;
    if (!previous?.sessionId || previous.taskCancel?.status === 'pending' || selected?.status !== 'ready'
      || selected.stale || !['pending', 'in_progress', 'waiting', 'completed'].includes(selected.data?.status)
      || selected.revision < (previous.requestTasks?.revision || 0) || !String(reason ?? '').trim()) return null;
    const requestId = ++requestSequence, operationId = `task-cancel-${globalThis.crypto.randomUUID()}`;
    model.update(state => ({ ...state, windows: { ...state.windows, [windowId]: { ...previous,
      taskCancel: { key: selected.key, requestId, operationId, revision: selected.revision, status: 'pending', error: null },
      revision: previous.revision + 1 } }, changedWindowId: windowId }));
    return { kind: 'cancel_pm_request_task', id: windowId, request_id: requestId, task_key: selected.key,
      request: { operation_id: operationId, expected_revision: selected.revision, reason, task_id: '' } };
  }
  function markPmTaskCancelUnavailable(windowId, requestId) {
    const previous = model.read().windows[windowId], cancellation = previous?.taskCancel;
    if (cancellation?.status !== 'pending' || cancellation.requestId !== requestId) return;
    model.update(state => ({ ...state, windows: { ...state.windows, [windowId]: { ...previous,
      taskCancel: { ...cancellation, status: 'error', error: 'Cancellation was not confirmed. Reconnect, then reload details before trying again.' },
      taskDetail: previous.taskDetail?.key === cancellation.key ? { ...previous.taskDetail, stale: true } : previous.taskDetail,
      revision: previous.revision + 1 } }, changedWindowId: windowId }));
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
  return { requestPmReports, requestPmConversation, requestPmTaskDetail, markPmTaskDetailUnavailable, requestPmTaskCancel, markPmTaskCancelUnavailable, requestPmChatSend, markPmChatSendUnavailable, expirePmChatSend, bindPmWindowState, applyPmWindowReceiveEvent, removePmWindowState, readPmWindowState, subscribePmWindowState };
}

// Read-only report view. The caller owns polling and the unchanged terminal.
export function createPmChat({ document, root, sessionId, onLogVisibility, onChatVisible = () => {}, onTaskDetail = () => {}, onTaskCancel = () => {}, onSend = () => {}, onOpenTerminal = () => false,
  copyText = text => document.defaultView.navigator.clipboard.writeText(text) }) {
  let session = sessionId;
  let disposed = false;
  let logs = false;
  let activeView = 'home';
  let includeCompleted = false;
  let tasksDigest = '';
  let chatDigest;
  let chatSend = null;
  let draftSend = null;
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
  const decideButton = element('button', '', 'Decide');
  const chatButton = element('button', '', 'Chat');
  const tasksButton = element('button', '', 'Tasks');
  const reportsButton = element('button', '', 'Reports');
  const logButton = element('button', '', 'Execution log');
  homeButton.type = decideButton.type = chatButton.type = tasksButton.type
    = reportsButton.type = logButton.type = 'button';
  toolbar.append(homeButton, decideButton, chatButton, tasksButton, reportsButton, logButton);
  // SPEC #7007: the task-centered Home island is the entry view. Its actions
  // reuse the existing surfaces: creating a task goes through the PM chat
  // composer (the request-task intake path), reviewing decisions opens the
  // Decide surface, and progress is the Tasks view.
  const taskHome = createTaskHome({ document,
    onCreateTask: () => { showView('chat'); draft.focus(); },
    onReviewDecisions: () => showView('decide'),
    onViewProgress: () => showView('tasks'),
  });
  // SPEC #7007 T-010: the Decide island aggregates only decision reports.
  // Answers travel the existing PM reply path (the chat send); the view
  // tracks the in-flight answer and the answers this pane confirmed during
  // the session, but never marks a decision resolved — only a later reports
  // snapshot can retire it.
  const taskDecisions = createTaskDecisions({ document,
    onAnswerDecision: (item, optionIndex) => answerDecision(item, optionIndex),
    onReplyInChat: () => { showView('chat'); draft.focus(); },
  });
  const chatBody = element('section', 'pm-chat__chat');
  chatBody.setAttribute('aria-label', 'PM conversation');
  const chatActions = element('div', 'pm-chat__toolbar');
  const terminalButton = element('button', '', 'Open terminal in Works');
  terminalButton.type = 'button';
  chatActions.append(terminalButton);
  const terminalStatus = element('p', 'pm-chat__notice pm-chat__terminal-status');
  terminalStatus.setAttribute('role', 'status');
  terminalStatus.hidden = true;
  const chatNotice = element('p', 'pm-chat__notice');
  chatNotice.setAttribute('role', 'status');
  const chatLog = element('div', 'pm-chat__conversation');
  chatLog.setAttribute('role', 'log');
  chatLog.setAttribute('aria-label', 'PM conversation messages');
  const chatDecisionReports = element('section', 'pm-chat__decision-reports');
  chatDecisionReports.setAttribute('aria-label', 'Decision reports');
  const chatDecisionList = element('div', 'pm-chat__decision-list');
  chatDecisionReports.append(element('h2', '', 'Decision reports'),
    element('p', '', 'Saved PM reports, separate from the native conversation.'), chatDecisionList);
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
  chatBody.append(chatActions, chatNotice, chatLog, composer);
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
  const taskDetail = element('section', 'pm-chat__task-detail');
  taskDetail.setAttribute('aria-label', 'Selected task details');
  taskDetail.hidden = true;
  const detailHeading = element('h3', '', 'Selected task details');
  detailHeading.tabIndex = -1;
  const detailActions = element('div', 'pm-chat__toolbar');
  const reloadDetail = element('button', '', 'Reload details');
  const closeDetail = element('button', '', 'Close details');
  reloadDetail.type = closeDetail.type = 'button';
  detailActions.append(reloadDetail, closeDetail);
  const detailNotice = element('p', 'pm-chat__notice');
  detailNotice.setAttribute('aria-live', 'polite');
  const detailContent = element('div', '');
  const cancelControls = element('div', 'pm-chat__task-cancel');
  const cancelLabel = element('label', '', 'Reason for cancelling this task');
  const cancelReason = element('textarea', 'pm-chat__task-cancel-reason');
  cancelReason.setAttribute('aria-label', 'Reason for cancelling this task');
  cancelReason.maxLength = 1000;
  cancelReason.rows = 2;
  cancelLabel.append(cancelReason);
  const cancelTask = element('button', '', 'Cancel task');
  cancelTask.type = 'button';
  const cancelStatus = element('p', 'pm-chat__task-cancel-status');
  cancelStatus.setAttribute('aria-live', 'polite');
  cancelControls.append(element('p', 'pm-chat__task-meta', 'Cancel tracking only. This does not stop agents or change Issues or pull requests.'), cancelLabel, cancelTask);
  taskDetail.append(detailHeading, detailActions, detailNotice, cancelStatus, cancelControls, detailContent);
  cancelReason.addEventListener('input', () => renderTaskCancellation(latestState));
  cancelTask.addEventListener('click', () => {
    if (!disposed && cancelTask.getAttribute('aria-disabled') === 'false') onTaskCancel(cancelReason.value);
  });
  reloadDetail.addEventListener('click', () => {
    if (!disposed && latestState?.taskDetail && latestState.taskDetail.status !== 'loading') onTaskDetail(latestState.taskDetail.key);
  });
  closeDetail.addEventListener('click', () => {
    if (disposed) return;
    const key = latestState?.taskDetail?.key;
    onTaskDetail(null);
    const trigger = taskRows.get(key)?.querySelector('.pm-chat__task-detail-button');
    (trigger && root.contains(trigger) ? trigger : completedButton).focus();
  });
  tasksBody.append(tasksHeader, tasksNotice, taskDetail, taskList);
  let selectedDetailKey = null;
  let detailData = null;
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
  const taskWorkspace = element('div', 'pm-chat__workspace');
  taskWorkspace.append(chatBody, tasksBody);
  root.append(toolbar, terminalStatus, taskHome.element, taskDecisions.element, body, taskWorkspace);

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
    taskWorkspace.hidden = value !== 'tasks' && value !== 'chat';
    tasksBody.hidden = value !== 'tasks' && value !== 'chat';
    chatBody.hidden = value !== 'chat';
    taskHome.element.hidden = value !== 'home';
    taskDecisions.element.hidden = value !== 'decide';
    root.classList.toggle('pm-chat--logs', logs);
    root.classList.toggle('pm-chat--chat', value === 'chat');
    homeButton.setAttribute('aria-pressed', String(value === 'home'));
    decideButton.setAttribute('aria-pressed', String(value === 'decide'));
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
  const showDecisions = () => showView('decide');
  const showChat = () => showView('chat');
  const showTasks = () => showView('tasks');
  const showReports = () => showView('reports');
  const showLog = () => showView('log');
  const openTerminal = () => {
    terminalStatus.hidden = onOpenTerminal(session) !== false;
    terminalStatus.textContent = terminalStatus.hidden ? ''
      : 'Terminal unavailable in Works. Wait for the current PM session to connect.';
  };
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
  function renderHome(state, decisions) {
    const overview = state?.requestTasks ?? null;
    taskHome.receive({ overview, decisionsWaiting: decisions.waiting, completedToday: null,
      stale: Boolean(state?.requestTasksError) && overview !== null });
  }
  // Decide aggregates the same decision reports Home counts (AC-6), newest
  // first. Typed packets expose their options; untyped decision reports keep
  // the plain reply path and carry their display text instead.
  const answeredDecisions = new Map();
  let pendingDecision = null;
  let decisionSendError = null;
  let observedPaneReceipt = 0;
  const chatDecisionRows = new Map();
  const decisionControls = new WeakMap();
  function decisionItems(state) {
    return (state?.reports || []).filter(report => report.kind === 'decision').map(report => {
      const summary = report.structured?.summary ?? null;
      const packet = summary?.decision ?? null;
      const displayText = reportDisplayText(report);
      return {
        id: String(report.id),
        question: packet?.question || summary?.result || displayText.split('\n')[0] || 'Decision needed',
        result: summary?.result ?? null,
        impact: summary?.impact ?? null,
        owner: summary?.owner ?? null,
        recommendation: packet?.recommendation ?? null,
        reason: packet?.reason ?? null,
        options: packet ? [...packet.options] : null,
        detailText: packet ? null : displayText,
        createdAt: report.created_at ?? null,
      };
    }).reverse();
  }
  function observeDecisionSend(send) {
    if (!pendingDecision || !send || send.sequence <= pendingDecision.afterSequence) return;
    if (send.status === 'sent') {
      answeredDecisions.set(pendingDecision.id, pendingDecision.optionText);
      pendingDecision = null;
      decisionSendError = null;
    } else if (send.status === 'failed') {
      decisionSendError = { id: pendingDecision.id,
        message: send.error || 'The answer could not be sent.' };
      pendingDecision = null;
    }
  }
  // A result without a correlation ID never settles the composer send, but
  // its window receipt still settles a Decide answer waiting on it: the pane
  // serializes sends, so the first result receipted after the answer was
  // sent is that answer's outcome.
  function observePaneResult(result) {
    if (!result || result.receipt <= observedPaneReceipt) return;
    observedPaneReceipt = result.receipt;
    if (!pendingDecision || result.receipt <= pendingDecision.afterReceipt) return;
    if (result.ok) {
      answeredDecisions.set(pendingDecision.id, pendingDecision.optionText);
      pendingDecision = null;
      decisionSendError = null;
    } else {
      decisionSendError = { id: pendingDecision.id,
        message: result.error || 'The answer could not be sent.' };
      pendingDecision = null;
    }
  }
  // SPEC #5016 AC-HS1: the one derivation of the waiting-decision count.
  // A decision answered in this session no longer waits: Home reads this
  // count, and Decide lists the same items with the same answered map.
  function deriveDecisions(state) {
    observeDecisionSend(state?.chatSend ?? null);
    observePaneResult(state?.paneResult ?? null);
    const items = decisionItems(state);
    const answered = {};
    for (const [id, optionText] of answeredDecisions) answered[id] = optionText;
    const waiting = state?.loaded ? items.filter(item => !answered[item.id]).length : null;
    return { items, answered, waiting };
  }
  function renderDecisionSurfaces(state) {
    const decisions = deriveDecisions(state);
    renderHome(state, decisions);
    renderDecisions(state, decisions);
    renderReportDecisions(decisions);
    return decisions;
  }
  function renderDecisions(state, { items, answered }) {
    taskDecisions.receive({
      loaded: Boolean(state?.loaded),
      error: state?.error ?? null,
      decisions: items,
      // The island's in-flight flag follows the answer's own lifecycle; the
      // shared composer send slot is guarded separately in answerDecision.
      sending: pendingDecision != null,
      pendingId: pendingDecision?.id ?? null,
      sendError: decisionSendError,
      answered,
    });
  }
  function answerDecision(item, optionIndex) {
    if (disposed || answeredDecisions.has(item.id) || chatSend?.status === 'pending' || pendingDecision) return;
    const optionText = item.options?.[optionIndex];
    if (optionText == null) return;
    pendingDecision = { id: item.id, optionText, afterSequence: chatSend?.sequence ?? 0,
      afterReceipt: latestState?.paneResult?.receipt ?? 0 };
    decisionSendError = null;
    onSend(`Answer to "${item.question}": ${optionIndex + 1}. ${optionText}`);
    const send = latestState?.chatSend ?? null;
    if (pendingDecision && (!send || send.sequence <= pendingDecision.afterSequence
      || send.status !== 'pending')) {
      // The reply path refused the send (no session, or another send already
      // in flight): keep the decision actionable and say why on its card.
      decisionSendError = { id: item.id,
        message: 'The answer could not be sent. Reconnect, then try again.' };
      pendingDecision = null;
    }
    renderDecisionSurfaces(latestState);
  }
  function replyInChat() {
    if (disposed || latestState?.chatSend?.status === 'pending' || pendingDecision) return;
    showChat();
    draft.focus();
  }
  function renderDecisionControls(controls, item, answered) {
    const busy = pendingDecision != null || latestState?.chatSend?.status === 'pending';
    const error = decisionSendError?.id === item.id ? decisionSendError.message : null;
    const itemDigest = JSON.stringify(item);
    let view = decisionControls.get(controls);
    if (!view || view.itemDigest !== itemDigest) {
      const nodes = [];
      if (item.recommendation) nodes.push(element('p', '',
        `Recommended: ${item.recommendation}${item.reason ? ` — ${item.reason}` : ''}`));
      const status = element('p', '');
      status.setAttribute('role', 'status');
      status.setAttribute('tabindex', '-1');
      const alert = element('p', 'pm-chat__decision-error');
      alert.setAttribute('role', 'alert');
      const retry = element('p', '', 'Choose an option to try again.');
      const options = element('div', 'pm-chat__decision-options');
      for (const [index, option] of (item.options || []).entries()) {
        const button = element('button', 'pm-chat__decision-option', `${index + 1}. ${option}`);
        button.type = 'button';
        button.addEventListener('click', () => answerDecision(item, index));
        options.append(button);
      }
      const reply = element('button', '', 'Other: reply in chat');
      reply.type = 'button';
      reply.addEventListener('click', replyInChat);
      options.append(reply);
      controls.replaceChildren(...nodes, status, alert, retry, options);
      view = { itemDigest, status, alert, retry, options, busy: false, focusedControl: null };
      decisionControls.set(controls, view);
    }
    const active = document.activeElement;
    const ownsFocus = view.options.contains(active);
    if (busy && !view.busy) view.focusedControl = ownsFocus ? active : null;
    // Native disabled buttons may blur. Restore only the interaction that
    // started here, while leaving a deliberate move to another control alone.
    const focusStayed = ownsFocus || (view.focusedControl && (!active || active === document.body));
    const statusText = answered ? `Answer sent: ${answered}`
      : pendingDecision?.id === item.id ? 'Sending answer…' : '';
    if (view.status.textContent !== statusText) view.status.textContent = statusText;
    if (view.alert.textContent !== (error || '')) view.alert.textContent = error || '';
    view.alert.hidden = view.retry.hidden = !error;
    if (answered) {
      if (view.options.parentNode === controls) {
        view.options.remove();
        if (focusStayed) view.status.focus({ preventScroll: true });
      }
      view.focusedControl = null;
    } else {
      if (view.options.parentNode !== controls) controls.append(view.options);
      for (const button of view.options.children) if (button.disabled !== busy) button.disabled = busy;
      if (!busy && view.busy) {
        if (focusStayed) view.focusedControl?.focus({ preventScroll: true });
        view.focusedControl = null;
      }
    }
    view.busy = busy;
  }
  function renderReportDecisions({ items, answered }) {
    chatDecisionReports.hidden = !items.length;
    if (items.length && chatDecisionReports.parentNode !== chatLog) chatLog.append(chatDecisionReports);
    else if (!items.length) chatDecisionReports.remove();
    const nodes = items.map(item => {
      let row = chatDecisionRows.get(item.id);
      if (!row) {
        row = element('div', 'pm-chat__decision-report');
        row.dataset.decisionId = item.id;
        row.append(element('h3', ''), element('p', ''), element('div', 'pm-chat__decision-controls'));
        chatDecisionRows.set(item.id, row);
      }
      if (row.firstChild.textContent !== item.question) row.firstChild.textContent = item.question;
      const summary = row.children[1];
      if (summary.textContent !== (item.result || '')) summary.textContent = item.result || '';
      summary.hidden = !item.result;
      renderDecisionControls(row.lastChild, item, answered[item.id]);
      return row;
    });
    if (nodes.length !== chatDecisionList.children.length
      || nodes.some((node, index) => chatDecisionList.children[index] !== node)) chatDecisionList.replaceChildren(...nodes);
    const ids = new Set(items.map(item => item.id));
    for (const id of chatDecisionRows.keys()) if (!ids.has(id)) chatDecisionRows.delete(id);
    // Answer-only updates do not change report content. Refresh its controls
    // independently so Copy, disclosure and reading position stay intact.
    for (const row of transcript.children) {
      const item = items.find(item => item.id === row.dataset.reportId);
      const controls = row.querySelector('.pm-chat__decision-controls');
      if (item && controls) renderDecisionControls(controls, item, answered[item.id]);
    }
  }
  function renderTasks(state) {
    renderTaskDetail(state?.taskDetail);
    renderTaskCancellation(state);
    const overview = state?.requestTasks;
    const error = state?.requestTasksError;
    const rows = overview?.state === 'ready' ? overview.tasks : [];
    const visible = rows.filter(task => includeCompleted || !['completed', 'cancelled'].includes(task.status));
    const sources = state?.conversation?.availability === 'ready'
      ? state.conversation.messages.filter(message => message.role === 'user' && message.sourceKey)
      : [];
    const nextDigest = JSON.stringify([includeCompleted, rows, sources.map(message => [message.sourceKey, message.id])]);
    if (nextDigest !== tasksDigest) {
      const focusedTaskControl = document.activeElement?.matches('.pm-chat__task-detail-button')
        ? '.pm-chat__task-detail-button' : '.pm-chat__task-criteria > summary';
      const focusedTaskKey = taskList.contains(document.activeElement)
        && document.activeElement?.matches(focusedTaskControl)
        ? document.activeElement.closest('.pm-chat__task')?.dataset.taskKey : null;
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
        const source = sources.find(message => message.sourceKey === task.source_key);
        const digest = JSON.stringify([task, parents.get(task.parent_key), source?.id]);
        if (row.dataset.digest !== digest) {
          const previousCriteria = row.querySelector('.pm-chat__task-criteria');
          const criteriaOpen = previousCriteria?.hasAttribute('open');
          row.dataset.digest = digest;
          row.classList.toggle('pm-chat__task--child', !!task.parent_key);
          const title = element('h3', '', task.title);
          const meta = element('p', 'pm-chat__task-meta', `${labels[task.status] || 'Unknown'} · ${task.assignee || 'Unassigned'}`);
          const fields = [title, meta];
          const detail = element('button', 'pm-chat__task-detail-button', 'View task details');
          detail.type = 'button';
          detail.addEventListener('click', () => { if (!disposed) onTaskDetail(task.key); });
          fields.push(detail);
          if (task.child_progress) {
            const { total, completed, cancelled } = task.child_progress;
            fields.push(element('p', 'pm-chat__task-meta',
              `Direct subtasks: ${completed} completed · ${cancelled} cancelled · ${total - completed - cancelled} unfinished (${total} total)`));
          }
          if (task.parent_key) fields.push(element('p', 'pm-chat__task-meta', `Part of: ${parents.get(task.parent_key) || 'Parent outside this preview'}`));
          if (task.review_required) fields.push(element('p', 'pm-chat__task-review', 'PM reassessment required'));
          if (task.wait_reason) fields.push(element('p', '', `Waiting: ${task.wait_reason}`));
          if (task.next_action) fields.push(element('p', '', `Next: ${task.next_action}`));
          if (task.criteria_preview) {
            const preview = task.criteria_preview;
            const criteria = element('details', 'pm-chat__task-criteria');
            if (criteriaOpen) criteria.setAttribute('open', '');
            const conditions = element('ol', '');
            for (const condition of preview.completion_conditions) conditions.append(element('li', '', condition));
            criteria.append(element('summary', '', 'Purpose and completion criteria'),
              element('p', '', `Purpose: ${preview.purpose}`),
              element('p', 'pm-chat__task-meta', `Preview. Showing ${preview.completion_conditions.length} of ${preview.completion_condition_total} completion criteria. Text is limited to 240 characters per item.`),
              conditions);
            fields.push(criteria);
          }
          const link = element('button', '', 'Show in conversation');
          link.type = 'button';
          link.disabled = !source;
          link.addEventListener('click', () => {
            if (disposed || latestState?.conversation?.availability !== 'ready') return;
            const message = latestState.conversation.messages.find(message => message.role === 'user'
              && message.sourceKey && message.sourceKey === task.source_key);
            if (!message) return;
            showChat();
            const bubble = [...chatLog.querySelectorAll('.pm-chat__bubble--user')]
              .find(node => node.dataset.messageId === message.id);
            bubble?.scrollIntoView({ block: 'center' });
            bubble?.focus({ preventScroll: true });
          });
          fields.push(link);
          if (!source) fields.push(element('p', 'pm-chat__task-meta', 'The original message is not in the current conversation preview.'));
          row.replaceChildren(...fields);
        }
        return row;
      });
      // Keep stable rows attached: report-only polls cannot disturb reading
      // position or selection inside an unchanged Task summary.
      if (nodes.length !== taskList.children.length || nodes.some((node, index) => taskList.children[index] !== node)) {
        taskList.replaceChildren(...nodes);
      }
      if (focusedTaskKey) nodes.find(node => node.dataset.taskKey === focusedTaskKey)
        ?.querySelector(focusedTaskControl)?.focus();
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
  function renderTaskDetail(selected) {
    taskDetail.hidden = !selected;
    if (!selected) { selectedDetailKey = null; detailData = null; cancelReason.value = ''; detailContent.replaceChildren(); return; }
    const changedKey = selected.key !== selectedDetailKey;
    if (changedKey) cancelReason.value = '';
    selectedDetailKey = selected.key;
    detailHeading.textContent = selected.title || 'Selected task details';
    // Retain keyboard focus while the read is pending.
    reloadDetail.setAttribute('aria-disabled', String(selected.status === 'loading'));
    detailNotice.textContent = [selected.status === 'loading' ? 'Loading task details…' : '',
      selected.stale ? 'Showing the last confirmed detail (stale). Reload details after recovering the task store.' : '',
      selected.error || '', selected.data ? `Selected snapshot revision ${selected.revision}. Text below is a bounded preview.` : ''].filter(Boolean).join(' ');
    if (detailData !== selected.data) {
      detailData = selected.data;
      const nodes = [];
      const text = value => value ? value.text + ([...value.text].length < value.total_chars
        ? `\n[Showing ${[...value.text].length} of ${value.total_chars} characters]` : '') : '';
      const paragraph = value => element('p', 'pm-chat__task-detail-text', text(value));
      const data = selected.data;
      if (data) {
        nodes.push(element('h4', '', 'Original request'), data.original ? paragraph(data.original)
          : element('p', '', 'The original request is missing from saved task data.'),
        element('h4', '', 'Purpose'), paragraph(data.purpose));
        const list = (title, values, total, label) => {
          const items = element('ol', '');
          for (const value of values) { const item = element('li', ''); item.append(paragraph(value)); items.append(item); }
          nodes.push(element('h4', '', title), element('p', 'pm-chat__task-meta', `Showing ${values.length} of ${total} ${label}.`), items);
        };
        list('Completion criteria', data.completion_conditions, data.completion_condition_total, 'completion criteria');
        list('Completion evidence', data.evidence, data.evidence_total, 'evidence references');
        nodes.push(element('h4', '', 'Recent recorded changes'),
          element('p', 'pm-chat__task-meta', `Showing ${data.history.length} of ${data.history_total} recorded changes, newest first. Only changes with a saved task delta are included.`));
        for (const change of data.history) {
          nodes.push(element('p', 'pm-chat__task-meta', `Revision ${change.revision} · ${change.action} · ${change.actor_kind} · ${text(change.actor_id)} · ${change.at}`), paragraph(change.reason));
        }
      }
      detailContent.replaceChildren(...nodes);
    }
    if (changedKey) { taskDetail.scrollTop = 0; detailHeading.focus(); }
  }
  function renderTaskCancellation(state) {
    const selected = state?.taskDetail, cancellation = state?.taskCancel;
    const current = cancellation?.key === selected?.key ? cancellation : null;
    const actionable = ['pending', 'in_progress', 'waiting', 'completed'].includes(selected?.data?.status);
    if (!actionable && cancelControls.contains(document.activeElement)) reloadDetail.focus();
    cancelControls.hidden = !actionable;
    cancelTask.setAttribute('aria-disabled', String(!actionable || selected?.status !== 'ready' || selected.stale
      || cancellation?.status === 'pending' || !cancelReason.value.trim()));
    cancelReason.readOnly = cancellation?.status === 'pending';
    cancelStatus.textContent = cancellation?.status === 'pending'
      ? (current ? 'Saving cancellation…' : 'Another task cancellation is awaiting confirmation. Reconnect and reload its details if confirmation does not arrive.')
      : current?.status === 'recorded' ? `Cancellation recorded.${selected?.data?.status === 'cancelled' && !selected.stale ? ' This task is cancelled.' : ' Reload details to see the current state.'}`
      : current?.error || (selected?.data?.status === 'cancelled' ? 'This task is cancelled.' : '');
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
      if (user) bubble.setAttribute('tabindex', '-1');
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
    // Keep the report-derived section attached while replacing native messages;
    // native order stays intact and a focused report control keeps its focus.
    for (const node of [...chatLog.children]) if (node !== chatDecisionReports) node.remove();
    for (const node of nodes) chatLog.insertBefore(node, chatDecisionReports.parentNode === chatLog ? chatDecisionReports : null);
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
    if (next?.status === 'pending' && draftSend?.sequence === null) draftSend.sequence = next.sequence;
    // A confirmed delivery clears only the draft that was sent.
    if (next?.status === 'sent' && chatSend?.status === 'pending' && draftSend?.sequence === next.sequence) {
      draft.value = '';
    }
    if (next?.status !== 'pending') draftSend = null;
    chatSend = next || null;
    syncComposer();
  }
  function submitDraft(event) {
    event?.preventDefault();
    if (disposed || chatSend?.status === 'pending' || !draft.value.trim()) return;
    draftSend = { sequence: null };
    onSend(draft.value);
    if (draftSend?.sequence === null) draftSend = null;
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
    const decisions = renderDecisionSurfaces(state);
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
        const decision = decisions.items.find(item => item.id === String(report.id));
        if (decision) {
          const controls = element('div', 'pm-chat__decision-controls');
          renderDecisionControls(controls, decision, decisions.answered[decision.id]);
          row.append(controls);
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
    terminalStatus.hidden = true;
    terminalStatus.textContent = '';
    expandedReports.clear();
    answeredDecisions.clear();
    pendingDecision = null;
    decisionSendError = null;
    observedPaneReceipt = 0;
    chatDecisionRows.clear();
    chatDecisionList.replaceChildren();
    chatDecisionReports.hidden = true;
    taskRows.clear();
    taskList.replaceChildren();
    tasksDigest = '';
    chatDigest = undefined;
    expandedChatGroups.clear();
    chatLog.replaceChildren();
    draft.value = '';
    draftSend = null;
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
    taskDecisions.dispose();
    homeButton.removeEventListener('click', showHome);
    decideButton.removeEventListener('click', showDecisions);
    chatButton.removeEventListener('click', showChat);
    composer.removeEventListener('submit', submitDraft);
    draft.removeEventListener('keydown', draftKeydown);
    draft.removeEventListener('input', syncComposer);
    tasksButton.removeEventListener('click', showTasks);
    completedButton.removeEventListener('click', filterTasks);
    reportsButton.removeEventListener('click', showReports);
    logButton.removeEventListener('click', showLog);
    terminalButton.removeEventListener('click', openTerminal);
    kindSelect.removeEventListener('change', filterReports);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs', 'pm-chat--chat');
  }
  homeButton.addEventListener('click', showHome);
  decideButton.addEventListener('click', showDecisions);
  chatButton.addEventListener('click', showChat);
  composer.addEventListener('submit', submitDraft);
  draft.addEventListener('keydown', draftKeydown);
  draft.addEventListener('input', syncComposer);
  reportsButton.addEventListener('click', showReports);
  tasksButton.addEventListener('click', showTasks);
  completedButton.addEventListener('click', filterTasks);
  logButton.addEventListener('click', showLog);
  terminalButton.addEventListener('click', openTerminal);
  kindSelect.addEventListener('change', filterReports);
  update({ sessionId, revision: 0, reports: [], loaded: false });
  showHome();
  return { update, setSession, showHome, showDecisions, showChat, showTasks, showReports, dispose,
    isChatVisible: () => !disposed && activeView === 'chat' };
}

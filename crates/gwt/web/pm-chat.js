import { createUiStateStore } from './ui-state-store.js';
import { renderUiContent } from './ui-content.js';

// Parsed conversation and raw terminal packets share one immutable window version.
// The terminal runtime retains responsibility for buffering and snapshot ordering.
export function createPmWindowModel() {
  const model = createUiStateStore({ windows: {}, changedWindowId: null });
  function emptyWindow(windowId, sessionId = null) {
    return { windowId, sessionId, revision: 0, terminal: null,
      conversation: { availability: 'waiting', conversation_id: null, messages: [] } };
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
      if (event.kind === 'pm_conversation') {
        if (!previous || event.session_id !== previous.sessionId) return state;
        const snapshot = event.snapshot;
        const retainHistory = ['waiting', 'unavailable'].includes(snapshot.availability)
          && (!snapshot.conversation_id || snapshot.conversation_id === previous.conversation.conversation_id);
        const messages = retainHistory ? previous.conversation.messages : (snapshot.messages || [])
          .filter(message => (message.role === 'user' || message.role === 'assistant') && typeof message.text === 'string' && message.text.trim())
          .map(message => ({ ...message, content: { type: 'text', body: message.text } }));
        next = { ...next, conversation: { ...snapshot, messages,
          conversation_id: retainHistory ? previous.conversation.conversation_id : snapshot.conversation_id } };
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

// Provider-neutral conversation view. The caller owns polling and the terminal.
export function createPmChat({ document, root, windowId, sessionId, send, onLogVisibility }) {
  let session = sessionId;
  let pending = false;
  let disposed = false;
  let logs = false;
  let availability = 'waiting';
  let digest = '';
  let conversation = null;
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
  const chatButton = element('button', '', 'Chat');
  const logButton = element('button', '', 'Execution log');
  chatButton.type = logButton.type = 'button';
  toolbar.append(chatButton, logButton);
  const body = element('div', 'pm-chat__body');
  const notice = element('p', 'pm-chat__notice');
  notice.setAttribute('role', 'status');
  const transcript = element('div', 'pm-chat__transcript');
  transcript.setAttribute('role', 'log');
  transcript.setAttribute('aria-label', 'PM conversation');
  const form = element('form', 'pm-chat__input');
  const input = element('textarea', '');
  input.setAttribute('aria-label', 'Message PM');
  input.placeholder = 'Message PM';
  input.rows = 3;
  const submit = element('button', '', 'Send');
  submit.type = 'submit';
  const error = element('p', 'pm-chat__error');
  error.setAttribute('role', 'alert');
  error.hidden = true;
  form.append(input, submit, error);
  body.append(notice, transcript, form);
  root.append(toolbar, body);

  function controls() {
    input.disabled = pending || !session || availability !== 'ready';
    submit.disabled = input.disabled;
  }
  function showLogs(value) {
    logs = value;
    root.hidden = availability === 'unsupported';
    body.hidden = logs;
    root.classList.toggle('pm-chat--logs', logs);
    chatButton.setAttribute('aria-pressed', String(!logs));
    logButton.setAttribute('aria-pressed', String(logs));
    onLogVisibility(logs || availability === 'unsupported');
  }
  const showChat = () => showLogs(false);
  const showLog = () => showLogs(true);
  function showError(message) {
    error.textContent = message;
    error.hidden = !message;
  }
  function handleSendResult(event) {
    if (disposed || !pending || (event.window_id && event.window_id !== windowId)) return;
    pending = false;
    if (event.ok) {
      input.value = '';
      showError('');
    } else {
      showError(event.error || 'Message could not be sent. Try again.');
    }
    controls();
  }
  function sendInput(event) {
    event.preventDefault();
    if (disposed || input.disabled || !input.value.trim()) return;
    pending = true;
    showError('');
    controls();
    try {
      const result = send({ kind: 'pane_send_input', session_id: session, text: input.value });
      if (result === 'unavailable' || result === false) {
        handleSendResult({ ok: false, error: 'Connection unavailable. Your message has not been sent.' });
      }
    } catch (cause) {
      handleSendResult({ ok: false, error: cause.message || 'Message could not be sent.' });
    }
  }
  function update(state) {
    if (disposed || !state) return;
    setSession(state.sessionId);
    root.dataset.stateVersion = String(state.revision);
    const snapshot = state.conversation;
    const previousAvailability = availability;
    availability = snapshot.availability;
    const messages = snapshot.messages;
    const nextDigest = JSON.stringify(messages.map(({ id, role, text }) => [id, role, text]));
    const changedConversation = conversation !== snapshot.conversation_id;
    if (changedConversation || digest !== nextDigest) {
      const atBottom = transcript.scrollHeight - transcript.clientHeight - transcript.scrollTop <= 32;
      const previousScroll = transcript.scrollTop;
      const nodes = messages.map(message => {
        const row = element('article', `pm-chat__message pm-chat__message--${message.role}`);
        row.append(element('div', 'pm-chat__author', message.role === 'user' ? 'You' : 'PM'), renderUiContent(document, message.content, 'pm-chat__text'));
        return row;
      });
      transcript.replaceChildren(...nodes);
      transcript.scrollTop = changedConversation || atBottom ? transcript.scrollHeight : previousScroll;
      digest = nextDigest;
      conversation = snapshot.conversation_id;
    }
    notice.textContent = availability === 'waiting' ? 'Waiting for PM conversation…'
      : availability === 'unavailable' ? (snapshot.detail || 'PM conversation unavailable. Open the execution log for details.')
      : snapshot.detail || (messages.length ? '' : 'No conversation messages yet.');
    notice.hidden = !notice.textContent;
    if (previousAvailability !== availability) showLogs(logs);
    controls();
  }
  function setSession(nextSession) {
    if (disposed || session === nextSession) return;
    session = nextSession;
    transcript.replaceChildren();
    transcript.scrollTop = 0;
    digest = '';
    conversation = null;
    pending = false;
    input.value = '';
    showError('');
    availability = 'waiting';
    showLogs(false);
  }
  function dispose() {
    disposed = true;
    chatButton.removeEventListener('click', showChat);
    logButton.removeEventListener('click', showLog);
    form.removeEventListener('submit', sendInput);
    root.replaceChildren();
    root.classList.remove('pm-chat', 'pm-chat--logs');
  }
  chatButton.addEventListener('click', showChat);
  logButton.addEventListener('click', showLog);
  form.addEventListener('submit', sendInput);
  update({ sessionId, revision: 0, conversation: { availability: 'waiting', conversation_id: null, messages: [] } });
  showLogs(false);
  return { update, setSession, handleSendResult, dispose };
}

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
  function update(snapshot) {
    if (disposed) return;
    const previousAvailability = availability;
    availability = snapshot.availability;
    const messages = (snapshot.messages || []).filter(message =>
      (message.role === 'user' || message.role === 'assistant') && typeof message.text === 'string' && message.text.trim());
    const nextDigest = JSON.stringify(messages.map(({ id, role, text }) => [id, role, text]));
    const changedConversation = conversation !== snapshot.conversation_id;
    // A failed or pending read is not evidence that existing history vanished.
    const retainHistory = (availability === 'waiting' || availability === 'unavailable')
      && (!snapshot.conversation_id || !changedConversation);
    if (!retainHistory && (changedConversation || digest !== nextDigest)) {
      const atBottom = transcript.scrollHeight - transcript.clientHeight - transcript.scrollTop <= 32;
      const previousScroll = transcript.scrollTop;
      const nodes = messages.map(message => {
        const row = element('article', `pm-chat__message pm-chat__message--${message.role}`);
        row.append(element('div', 'pm-chat__author', message.role === 'user' ? 'You' : 'PM'), element('p', 'pm-chat__text', message.text));
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
    update({ availability: 'waiting', conversation_id: null, messages: [] });
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
  update({ availability: 'waiting', conversation_id: null, messages: [] });
  showLogs(false);
  return { update, setSession, handleSendResult, dispose };
}

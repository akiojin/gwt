import { createFocusTrap } from './focus-trap.js';

// Shared by the Hub and the Project page. Authorization always comes from
// the runtime preview; UI state never manufactures a close token.
export function createCloseProjectController({ document: doc, send, onClosed = () => {}, onError = () => {} }) {
  const node = (tag, className, text) => {
    const element = doc.createElement(tag);
    element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const modal = node('div', 'modal-backdrop');
  modal.id = 'close-project-modal';
  modal.setAttribute('aria-hidden', 'true');
  const dialog = node('div', 'modal-shell');
  dialog.setAttribute('role', 'dialog');
  dialog.setAttribute('aria-modal', 'true');
  dialog.setAttribute('aria-labelledby', 'close-project-title');
  dialog.tabIndex = -1;
  modal.append(dialog);
  let token = null;
  let previewPending = false;
  let releaseTrap = null;
  let returnFocus = null;
  function hide() {
    modal.classList.remove('open');
    modal.setAttribute('aria-hidden', 'true');
    releaseTrap?.();
    releaseTrap = null;
    returnFocus?.focus?.();
    returnFocus = null;
    token = null;
  }
  function finish(kind) {
    const current = token;
    hide();
    if (current) send({ kind, token: current });
  }
  modal.addEventListener('click', (event) => {
    if (event.target === modal) finish('cancel_close_project');
  });
  doc.addEventListener('keydown', (event) => {
    if (token && event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      finish('cancel_close_project');
    }
  });
  function receive(event) {
    if (event.kind === 'project_closed') {
      if (token?.project_key === event.project_key) hide();
      onClosed(event.project_key);
      return true;
    }
    if (event.kind === 'close_project_error') {
      previewPending = false;
      hide();
      onError(event.message);
      return true;
    }
    if (event.kind !== 'close_project_preview') return false;
    previewPending = false;
    hide();
    if (!(event.running_agents || []).length) {
      send({ kind: 'confirm_close_project', token: event.token });
      return true;
    }
    token = event.token;
    returnFocus = doc.activeElement;
    const header = node('header', 'modal-header');
    const title = node('h2', '', 'Close Project?');
    title.id = 'close-project-title';
    header.append(title);
    const body = node('div', 'modal-body');
    body.append(node('p', '', `${event.title}: running agents will be stopped.`));
    const list = node('ul', '');
    for (const agent of event.running_agents) list.append(node('li', '', `${agent.display_name}${agent.branch ? ` (${agent.branch})` : ''}`));
    body.append(list);
    const footer = node('footer', 'modal-footer');
    const cancel = node('button', 'text-button', 'Cancel');
    cancel.type = 'button';
    cancel.dataset.role = 'close-project-cancel';
    cancel.addEventListener('click', () => finish('cancel_close_project'));
    const confirm = node('button', 'wizard-button primary destructive', 'Close Project');
    confirm.type = 'button';
    confirm.dataset.role = 'close-project-confirm';
    confirm.addEventListener('click', () => finish('confirm_close_project'));
    footer.append(cancel, confirm);
    dialog.replaceChildren(header, body, footer);
    modal.classList.add('open');
    modal.removeAttribute('aria-hidden');
    releaseTrap = createFocusTrap(dialog, { document: doc });
    cancel.focus();
    return true;
  }
  function connectionLost() {
    previewPending = false;
    hide();
  }
  return { modal, receive, connectionLost, request: (projectKey) => {
    if (previewPending) return;
    if (token) finish('cancel_close_project');
    previewPending = true;
    send({ kind: 'preview_close_project', project_key: projectKey });
  } };
}

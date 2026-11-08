import { createFocusTrap } from './focus-trap.js';
import { createUiStateStore } from './ui-state-store.js';
import { createProjectOperationId } from './project-execution-policy-model.js';

// Runtime previews own close authority. Unanswered choices survive reload only
// for read-only reconciliation; they are never replayed from browser storage.
export function createCloseProjectController({ document: doc, send, onClosed = () => {}, onError = () => {},
  projectKey = null, sessionStorage = doc.defaultView?.sessionStorage,
  navigationType = doc.defaultView?.performance?.getEntriesByType?.('navigation')?.[0]?.type }) {
  const storageKey = 'gwt:project-close-intent';
  const persist = intent => { try { if (intent?.choice === 'continue' || intent?.choice === 'stop') sessionStorage?.setItem(storageKey, JSON.stringify(intent)); else sessionStorage?.removeItem(storageKey); } catch { /* status still works in this document */ } };
  let restored = null;
  try {
    const saved = JSON.parse(sessionStorage?.getItem(storageKey) || 'null');
    if (navigationType === 'reload' && saved?.operation_id && saved?.token?.project_key && ['continue', 'stop'].includes(saved.choice)
      && (!projectKey || saved.token.project_key === projectKey)) restored = saved;
  } catch { /* an invalid saved receipt grants no authority */ }
  const node = (tag, className, text) => {
    const element = doc.createElement(tag); element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const modal = node('div', 'modal-backdrop'); modal.id = 'close-project-modal'; modal.setAttribute('aria-hidden', 'true');
  const dialog = node('div', 'modal-shell'); dialog.setAttribute('role', 'dialog'); dialog.setAttribute('aria-modal', 'true');
  dialog.setAttribute('aria-labelledby', 'close-project-title'); dialog.tabIndex = -1; modal.append(dialog);
  const model = createUiStateStore({ preview: null, intent: restored, unknown: Boolean(restored), absent: false, retrySameIntent: false, error: null, closed: null });
  let previewPending = null, releaseTrap = null, returnFocus = null, freshIntentId = null;
  function hide() {
    modal.classList.remove('open'); modal.setAttribute('aria-hidden', 'true'); releaseTrap?.(); releaseTrap = null;
    returnFocus?.focus?.(); returnFocus = null;
  }
  function cancel() {
    const current = model.read().preview?.token;
    model.update(state => ({ ...state, preview: null }));
    if (current) send({ kind: 'cancel_close_project', token: current });
  }
  function reconcile() {
    const intent = model.read().intent;
    if (intent) send({ kind: 'get_project_execution_status', project_key: intent.token.project_key, operation_id: intent.operation_id });
  }
  function confirm(choice) {
    const preview = model.read().preview;
    if (!preview || model.read().intent) return;
    const intent = { kind: 'confirm_close_project', token: preview.token, choice,
      operation_id: createProjectOperationId(), expected_policy_revision: preview.policy_revision,
      expected_view_set_revision: preview.view_set_revision };
    freshIntentId = intent.operation_id;
    model.update(state => ({ ...state, preview: null, intent, unknown: false, absent: false, retrySameIntent: false }));
    if (send(intent) !== 'sent') {
      freshIntentId = null;
      model.update(state => ({ ...state, intent: null, unknown: false, absent: false, retrySameIntent: false,
        error: { message: 'Project choice was not sent. Reconnect and try Close Project again.' } }));
    }
  }
  function closed(key) {
    const matches = model.read().intent?.token.project_key === key;
    model.update(state => ({ ...state, preview: state.preview?.token.project_key === key ? null : state.preview,
      intent: matches ? null : state.intent, unknown: matches ? false : state.unknown, closed: { projectKey: key } }));
  }
  modal.addEventListener('click', event => { if (event.target === modal && model.read().preview) cancel(); });
  doc.addEventListener('keydown', event => {
    if (model.read().preview && event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); cancel(); }
  });
  function receive(event) {
    if (event.kind === 'project_closed') { closed(event.project_key); return true; }
    if (event.kind === 'project_view_closed') { closed(event.project_key); return true; }
    if (event.kind === 'project_execution_status') {
      const intent = model.read().intent, status = event.status;
      if (!intent || status?.project_key !== intent.token.project_key) return false;
      if (status.generation != null && status.generation !== intent.token.generation) {
        freshIntentId = null;
        model.update(state => ({ ...state, intent: null, unknown: false, absent: false, retrySameIntent: false }));
        return false;
      }
      const outcome = status.operation_outcome;
      if (status.generation === intent.token.generation && outcome?.generation === intent.token.generation
        && outcome.operation_id === intent.operation_id && outcome.choice === intent.choice && outcome.effect_state === 'confirmed') closed(status.project_key);
      else model.update(state => ({ ...state, unknown: true, absent: !outcome, retrySameIntent: false }));
      return false; // the shared project projection consumes the same snapshot
    }
    if (event.kind === 'close_project_error') {
      previewPending = null;
      const intent = model.read().intent;
      const retainedIntent = intent?.choice === null ? null : intent;
      const retrySameIntent = Boolean(retainedIntent && freshIntentId === intent.operation_id && event.project_key === intent.token.project_key
        && event.operation_id === intent.operation_id && event.retry_same_intent === true);
      model.update(state => ({ ...state, preview: null, intent: retainedIntent, unknown: Boolean(retainedIntent), retrySameIntent, error: { message: event.message } }));
      return true;
    }
    if (event.kind !== 'close_project_preview') return false;
    if (!previewPending || event.token?.project_key !== previewPending.projectKey || model.read().intent) return true;
    previewPending = null;
    model.update(state => ({ ...state, preview: { token: event.token, title: event.title, running_agents: event.running_agents || [],
      choice_required: event.choice_required !== false, policy_revision: event.policy_revision, view_set_revision: event.view_set_revision,
      view_count: event.view_count, project_wide: event.project_wide } }));
    return true;
  }
  model.subscribe(state => state.intent, persist);
  model.subscribe(state => state.preview, preview => {
    hide();
    if (!preview) return;
    if (!preview.choice_required) { confirm(null); return; }
    returnFocus = doc.activeElement;
    const header = node('header', 'modal-header'), title = node('h2', '', 'Close Project?'); title.id = 'close-project-title'; header.append(title);
    const body = node('div', 'modal-body');
    body.append(node('p', '', `${preview.title}: ${preview.project_wide ? `close all ${preview.view_count ?? 'observed'} project views` : 'close the last project view'}.`));
    body.append(node('p', '', 'Continue keeps requested work running. Stop saves and drains this project. Cancel keeps this view open.'));
    const list = node('ul', '');
    for (const agent of preview.running_agents) list.append(node('li', '', `${agent.display_name}${agent.branch ? ` (${agent.branch})` : ''}`));
    body.append(list);
    const footer = node('footer', 'modal-footer');
    const button = (label, role, action, className = 'wizard-button') => {
      const element = node('button', className, label); element.type = 'button'; element.dataset.role = `close-project-${role}`;
      element.addEventListener('click', action); footer.append(element); return element;
    };
    const cancelButton = button('Cancel', 'cancel', cancel, 'text-button');
    button('Continue', 'continue', () => confirm('continue'), 'wizard-button primary');
    button('Stop', 'stop', () => confirm('stop'), 'wizard-button destructive');
    dialog.replaceChildren(header, body, footer); modal.classList.add('open'); modal.removeAttribute('aria-hidden');
    releaseTrap = createFocusTrap(dialog, { document: doc }); cancelButton.focus();
  });
  model.subscribe(state => [state.intent, state.unknown, state.absent, state.retrySameIntent], ([intent, unknown, absent, retrySameIntent]) => {
    if (!intent || !unknown) { if (!model.read().preview) hide(); return; }
    hide(); returnFocus = doc.activeElement;
    const header = node('header', 'modal-header'), title = node('h2', '', 'Check Project Choice'); title.id = 'close-project-title'; header.append(title);
    const body = node('div', 'modal-body');
    body.append(node('p', '', absent ? 'No saved receipt was observed. The request may still be in flight. Check status before continuing.' : 'The result is unconfirmed. Check the saved choice before retrying.'));
    const footer = node('footer', 'modal-footer'), check = node('button', 'wizard-button', 'Check status'); check.type = 'button'; check.addEventListener('click', reconcile); footer.append(check);
    if (retrySameIntent) {
      const retry = node('button', 'wizard-button', 'Retry same choice'); retry.type = 'button';
      retry.addEventListener('click', () => {
        const state = model.read();
        if (!state.retrySameIntent || freshIntentId !== intent.operation_id || state.intent !== intent) return;
        model.update(current => ({ ...current, unknown: false, retrySameIntent: false }));
        if (send(intent) !== 'sent') model.update(current => ({ ...current, unknown: true }));
      }); footer.append(retry);
    }
    dialog.replaceChildren(header, body, footer); modal.classList.add('open'); modal.removeAttribute('aria-hidden');
    releaseTrap = createFocusTrap(dialog, { document: doc }); check.focus();
  });
  model.subscribe(state => state.error, error => { if (error) onError(error.message); });
  model.subscribe(state => state.closed, value => { if (value) { if (!model.read().intent && !model.read().preview) hide(); onClosed(value.projectKey); } });
  function request(key, { projectWide = false } = {}) {
    if (model.read().intent) { model.update(state => ({ ...state, unknown: true })); reconcile(); return; }
    if (previewPending) return;
    if (model.read().preview) cancel();
    previewPending = { projectKey: key, projectWide };
    if (send({ kind: 'preview_close_project', project_key: key, project_wide: projectWide }) !== 'sent') previewPending = null;
  }
  return { modal, receive, request, reconnect: reconcile, connectionLost() {
    previewPending = null; freshIntentId = null;
    model.update(state => {
      const intent = state.intent?.choice === null ? null : state.intent;
      return { ...state, preview: null, intent, unknown: Boolean(intent), retrySameIntent: false };
    });
  } };
}

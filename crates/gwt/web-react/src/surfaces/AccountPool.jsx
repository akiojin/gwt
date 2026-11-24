import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { createFocusTrap } from "../../../web/focus-trap.js";
import { focusElement } from "../hosts/focus.js";

function AccountDialog({ title, initialFocus, onDismiss, terminal = false, children }) {
  const titleId = useId();
  const root = useRef(null);
  const dismiss = useRef(onDismiss);
  dismiss.current = onDismiss;
  useEffect(() => {
    const dialog = root.current;
    const document = dialog.ownerDocument;
    const previous = document.activeElement;
    const release = createFocusTrap(dialog, { document });
    focusElement(dialog.querySelector(initialFocus));
    const onKeyDown = event => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      dismiss.current();
    };
    dialog.addEventListener("keydown", onKeyDown);
    return () => {
      dialog.removeEventListener("keydown", onKeyDown);
      release();
      if (previous?.isConnected) focusElement(previous);
    };
  }, [initialFocus]);
  return createPortal(
    <div className="modal-backdrop open">
      <div className={terminal ? "modal-shell exec-env__login-dialog" : "modal-shell"}
        ref={root} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className="modal-header"><h2 id={titleId}>{title}</h2></div>
        {children}
      </div>
    </div>,
    document.body,
  );
}

export function AddPoolAccount({ options, saving, error, onAdd, onDismiss }) {
  const [chosen, choose] = useState(null);
  const allowed = options.filter(option => !option.blocked_reason);
  const blocked = options.filter(option => option.blocked_reason);
  const tool = allowed.find(option => option.tool === chosen) || allowed[0];
  return (
    <AccountDialog title="Add an account" initialFocus={tool ? "select" : "[data-account-cancel]"} onDismiss={onDismiss}>
      <form onSubmit={event => { event.preventDefault(); if (tool && !saving) onAdd(tool.tool); }}>
        <div className="modal-body">
          <label className="exec-env__field">
            <span>Provider</span>
            <select value={tool?.tool || ""} disabled={!tool || saving} onChange={event => choose(event.target.value)}>
              {options.map(option => (
                <option key={option.tool} value={option.tool} disabled={Boolean(option.blocked_reason)}>
                  {option.tool_name}
                </option>
              ))}
            </select>
          </label>
          {blocked.length ? (
            <ul className="exec-env__reasons" aria-label="Providers that cannot be added">
              {blocked.map(option => (
                <li key={option.tool}><strong>{option.tool_name}</strong>{" · "}{option.blocked_reason}</li>
              ))}
            </ul>
          ) : null}
          {error ? <p className="exec-env__error" role="alert">{error}</p> : null}
        </div>
        <div className="modal-footer">
          <button type="button" className="text-button" data-account-cancel onClick={onDismiss}>Cancel</button>
          <button type="submit" className="wizard-button primary" disabled={!tool || saving}>
            {saving ? "Opening terminal…" : "Open login terminal"}
          </button>
        </div>
      </form>
    </AccountDialog>
  );
}

export function DeletionImpact({ impact }) {
  if (!impact) return null;
  const { sessions = [], agents = [], unknown_sessions = [] } = impact;
  return (
    <dl className="exec-env__facts">
      <div>
        <dt>Sessions to resume after their turn</dt>
        <dd>{sessions.length ? <ul>{sessions.map((name, index) => <li key={index}>{name}</li>)}</ul> : "None"}</dd>
      </div>
      {unknown_sessions.length ? <div>
        <dt>Impact unknown</dt>
        <dd><ul>{unknown_sessions.map((name, index) => <li key={index}>{name}</li>)}</ul></dd>
      </div> : null}
      <div>
        <dt>Agents whose priorities will change</dt>
        <dd>{agents.length ? <ul>{agents.map((name, index) => <li key={index}>{name}</li>)}</ul> : "None"}</dd>
      </div>
    </dl>
  );
}

export function DeletePoolAccount({ account, onDelete, onDismiss }) {
  return (
    <AccountDialog title={`Delete ${account.name}?`} initialFocus="[data-account-cancel]" onDismiss={onDismiss}>
      <div className="modal-body"><DeletionImpact impact={account.deleteImpact} /></div>
      <div className="modal-footer">
        <button type="button" className="text-button" data-account-cancel onClick={onDismiss}>Cancel</button>
        <button type="button" className="wizard-button destructive" onClick={onDelete}>Delete account</button>
      </div>
    </AccountDialog>
  );
}

export function AccountLogin({ login, account, error, onMount, onClose }) {
  const terminal = useRef(null);
  useEffect(() => onMount(terminal.current, login.id), [login.id, onMount]);
  let status = "Complete sign-in in the terminal.";
  if (login.finished) {
    status = error ? "The terminal has closed."
      : login.checking ? "Checking sign-in…"
        : account?.signedIn ? "Signed in" : "Sign-in is still pending. Sign in again to retry.";
  }
  return (
    <AccountDialog title={`Login terminal${account ? ` for ${account.name}` : ""}`} terminal
      initialFocus="[data-account-close-login]" onDismiss={onClose}>
      <div className="modal-body">
        <div className="exec-env__login-terminal" data-account-login-terminal ref={terminal} />
        <p role="status" className="exec-env__hint">{status}</p>
        {error ? <p className="exec-env__error" role="alert">{error}</p> : null}
      </div>
      <div className="modal-footer">
        <button type="button" className="text-button" data-account-close-login onClick={onClose}>{login.finished ? "Done" : "Close terminal"}</button>
      </div>
    </AccountDialog>
  );
}

export function AccountSessions({ sessions, accounts, saving, onResume }) {
  const usingAccounts = sessions.filter(session => session.execution_account?.account_id);
  if (!usingAccounts.length) return null;
  const name = id => accounts.find(account => account.id === id)?.name || id;
  return (
    <section className="exec-env__manage" aria-label="Active sessions">
      <h2>Active sessions</h2>
      <ul className="exec-env__list">
        {usingAccounts.map(session => (
          <li className="exec-env__card" key={session.id} data-account-session={session.id}>
            <div className="exec-env__card-head"><h2>{session.title}</h2></div>
            <p className="exec-env__meta">{`Using ${name(session.execution_account.account_id)}`}</p>
            {session.account_switches?.length ? (
              <ul className="exec-env__reasons" aria-label="Account switches">
                {session.account_switches.map(change => (
                  <li key={`${change.at}:${change.from}:${change.to}`}>
                    {`${name(change.from)} → ${name(change.to)} (${change.reason === "rate_limit" ? "rate limit" : "manual"})`}
                  </li>
                ))}
              </ul>
            ) : null}
            <div className="exec-env__actions">
              <button type="button" disabled={saving || session.account_switch_pending || Boolean(session.resume_blocked_reason)}
                onClick={() => onResume(session.id)}>
                {session.is_pm ? "Resume PM with the next account" : "Resume with the next account"}
              </button>
            </div>
            {session.resume_blocked_reason ? <p className="exec-env__reason">{session.resume_blocked_reason}</p> : null}
            {session.account_switch_pending ? <p className="exec-env__hint" role="status">Waiting to resume with the next account</p> : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

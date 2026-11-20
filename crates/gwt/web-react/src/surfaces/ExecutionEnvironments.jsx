import { useEffect, useRef } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement } from "../hosts/focus.js";
import { fillLegacySlot } from "../hosts/legacy-slot.js";
import { localTimeLabel } from "./time-labels.js";

// Execution environments (Issue #7648, SPEC #7007 T-030). The landing view
// lists the environments agents run on; Accounts is the one place usage,
// reset times and limit state are shown, once per account; every account
// row opens its detail. The store carries rows the host derived from the
// provider usage snapshot and the tool catalog — nothing is invented here.
const selectModel = state => state;

const AVAILABILITY_LABELS = {
  available: "Available",
  limit: "Limit reached",
  unknown: "Usage unknown",
};

function UsageLine({ account }) {
  const reset = localTimeLabel(account.resetsAt);
  return (
    <p className="exec-env__usage" data-severity={account.severity || undefined}>
      {account.usage || account.status.label}
      {account.usage && reset ? (
        <>
          {" · resets "}
          <time dateTime={account.resetsAt}>{reset}</time>
        </>
      ) : null}
      {account.usage && account.status.kind === "stale" ? ` · ${account.status.label}` : ""}
    </p>
  );
}

function LimitChip({ account }) {
  if (account.limitReached) return <span className="exec-env__chip exec-env__chip--limit">Limit reached</span>;
  if (account.usingCredits) return <span className="exec-env__chip">Using credits</span>;
  return null;
}

function Environments({ model, headingRef, onNavigate, onOpenToolManagement }) {
  const environments = model.environments || [];
  return (
    <>
      <div className="exec-env__header">
        <p className="exec-env__eyebrow">What your agents run on</p>
        <h1 ref={headingRef} tabIndex={-1}>Execution environments</h1>
        <p className="exec-env__lead">Each environment pairs a provider account with the tool agents reach it through.</p>
      </div>
      {environments.length === 0 ? (
        <div className="exec-env__empty">
          <h2>{model.loaded ? "No execution environments yet" : "Loading execution environments…"}</h2>
          <p>An environment appears once its tool is installed and its account reports usage.</p>
        </div>
      ) : (
        <ul className="exec-env__list" aria-label="Execution environments">
          {environments.map(environment => (
            <li className="exec-env__card" key={environment.id} data-environment={environment.id} data-availability={environment.availability}>
              <div className="exec-env__card-head">
                <h2>{environment.name}</h2>
                <span className={`exec-env__chip exec-env__chip--${environment.availability}`}>
                  {AVAILABILITY_LABELS[environment.availability]}
                </span>
              </div>
              <p className="exec-env__meta">
                {environment.vendor ? `Provider ${environment.vendor} · ` : ""}
                {`Via ${environment.via}`}
                {environment.toolVersion ? ` ${environment.toolVersion}` : ""}
                {` · ${environment.members} account`}
              </p>
              <UsageLine account={environment.account} />
              <div className="exec-env__actions">
                <button type="button" aria-label={`Open the ${environment.name} account`} onClick={() => onNavigate("account", environment.id)}>
                  Open account
                </button>
                {environment.updateAvailable ? (
                  <button type="button" aria-label={`Update ${environment.via} in Tool management`} onClick={onOpenToolManagement}>
                    Tool update available
                  </button>
                ) : null}
              </div>
            </li>
          ))}
        </ul>
      )}
      <section className="exec-env__manage" aria-label="Manage">
        <h2>Manage</h2>
        <div className="exec-env__manage-row">
          <button type="button" onClick={() => onNavigate("accounts")}>Accounts</button>
          <p>Usage, reset times and limits are shown per account.</p>
        </div>
        <div className="exec-env__manage-row">
          <button type="button" onClick={onOpenToolManagement}>Tool management</button>
          <p>Install and update the tools (CLIs) themselves in Settings.</p>
        </div>
      </section>
    </>
  );
}

function Accounts({ model, headingRef, onNavigate, onOpenToolManagement }) {
  const accounts = model.accounts || [];
  return (
    <>
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("environments")}>← Execution environments</button>
        <h1 ref={headingRef} tabIndex={-1}>Accounts</h1>
        <p className="exec-env__lead">Usage, reset times and limits are shown here, once per account.</p>
      </div>
      {accounts.length === 0 ? (
        <div className="exec-env__empty">
          <h2>{model.loaded ? "No accounts report usage yet" : "Loading accounts…"}</h2>
          <p>An account appears once its tool has been used on this device.</p>
        </div>
      ) : (
        <ul className="exec-env__list" aria-label="Accounts">
          {accounts.map(account => (
            <li className="exec-env__card" key={account.provider} data-account={account.provider}>
              <div className="exec-env__card-head">
                <h2>{account.name}</h2>
                <LimitChip account={account} />
              </div>
              <p className="exec-env__meta">
                {[account.accountLabel, account.plan, account.usage ? account.status.label : null].filter(Boolean).join(" · ") || "Account"}
              </p>
              <UsageLine account={account} />
              <div className="exec-env__actions">
                <button type="button" aria-label={`Details for the ${account.name} account`} onClick={() => onNavigate("account", account.provider)}>
                  Details
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      <section className="exec-env__manage" aria-label="Manage">
        <div className="exec-env__manage-row">
          <button type="button" onClick={onOpenToolManagement}>Tool management</button>
          <p>Installing or updating a tool is separate from its account.</p>
        </div>
      </section>
    </>
  );
}

function AccountDetail({ model, account, headingRef, onNavigate, renderAccountUsage, onSetUsageCollection }) {
  const slotRef = useRef(null);
  // The windows, credits and consumption card is still drawn by the
  // unmigrated usage renderer; refresh it whenever the rows change.
  useEffect(() => {
    if (account) fillLegacySlot(slotRef.current, element => renderAccountUsage(element, account.provider));
  });
  if (!account) {
    return (
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("accounts")}>← Accounts</button>
        <h1 ref={headingRef} tabIndex={-1}>Account</h1>
        <p className="exec-env__lead">{model.loaded ? "This account no longer reports usage." : "Loading account…"}</p>
      </div>
    );
  }
  return (
    <>
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("accounts")}>← Accounts</button>
        <h1 ref={headingRef} tabIndex={-1}>{`${account.name} account`}</h1>
        <p className="exec-env__lead">{account.status.label}</p>
      </div>
      <section className="exec-env__detail" aria-label="Usage and limits">
        <h2>Usage and limits</h2>
        <div className="exec-env__usage-slot" ref={slotRef} data-usage-slot={account.provider} />
      </section>
      {account.provider === "claude_code" ? (
        <section className="exec-env__detail" aria-label="Usage collection">
          <h2>Usage collection</h2>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={account.collectionEnabled}
              onChange={() => onSetUsageCollection(account.provider, !account.collectionEnabled)}
            />
            <span>Show Claude Code account usage (5-hour / weekly)</span>
          </label>
          <p className="exec-env__hint">
            Off by default (opt-in). When enabled, Claude account usage reads your OAuth token from the Keychain /
            credentials file and requests usage from the Anthropic API (polled at most once every 3 minutes). While
            disabled, no Keychain read or network request happens. Per-session token usage is read locally and is not
            affected by this setting.
          </p>
        </section>
      ) : (
        <p className="exec-env__hint">Codex usage is read from local session files automatically.</p>
      )}
      <div className="exec-env__actions">
        <button type="button" onClick={() => onNavigate("environments")}>View execution environments</button>
      </div>
    </>
  );
}

export function ExecutionEnvironments({
  store,
  renderAccountUsage = () => {},
  onNavigate = () => {},
  onOpenToolManagement = () => {},
  onSetUsageCollection = () => {},
}) {
  const model = useStoreSelector(store, selectModel, { frame: true }) || {};
  const view = model.view || { name: "environments", provider: null };
  const headingRef = useRef(null);
  const shownRef = useRef(null);
  const viewKey = `${view.name}:${view.provider || ""}`;
  // Moving between views replaces the whole body, so keyboard focus follows
  // to the new heading. The first render leaves focus where the user had it.
  useEffect(() => {
    if (shownRef.current !== null && shownRef.current !== viewKey) focusElement(headingRef.current);
    shownRef.current = viewKey;
  }, [viewKey]);
  const account = (model.accounts || []).find(item => item.provider === view.provider) || null;
  return (
    <section className="exec-env" aria-label="Execution environments" data-view={view.name}>
      {view.name === "accounts" ? (
        <Accounts model={model} headingRef={headingRef} onNavigate={onNavigate} onOpenToolManagement={onOpenToolManagement} />
      ) : view.name === "account" ? (
        <AccountDetail
          model={model}
          account={account}
          headingRef={headingRef}
          onNavigate={onNavigate}
          renderAccountUsage={renderAccountUsage}
          onSetUsageCollection={onSetUsageCollection}
        />
      ) : (
        <Environments model={model} headingRef={headingRef} onNavigate={onNavigate} onOpenToolManagement={onOpenToolManagement} />
      )}
    </section>
  );
}

import { useEffect, useRef, useState } from "react";
import { useStoreSelector } from "../store/useStoreSelector.js";
import { focusElement } from "../hosts/focus.js";
import { fillLegacySlot } from "../hosts/legacy-slot.js";
import { localTimeLabel } from "./time-labels.js";
import { AddEnvironment, DeleteEnvironment, EditEnvironment } from "./ExecutionEnvironmentForms.jsx";
import { ProjectPriorityList } from "./ProjectExecutionEnvironments.jsx";
import { AccountLogin, AccountSessions, AddPoolAccount, CopilotSignIn, DeletePoolAccount } from "./AccountPool.jsx";

// Execution environments (Issue #7648, SPEC #7007 T-030; Issue #7741). The
// landing view lists the environments agents run on, as the backend defines
// them; Accounts is the one place usage, reset times and limit state are
// shown, once per account. The store carries rows the host derived from the
// definitions, the provider usage snapshot and the tool catalog — nothing is
// invented here.
const selectModel = state => state;

const AVAILABILITY_LABELS = {
  available: "Available",
  limit: "Limit reached",
  unknown: "Usage unknown",
  not_installed: "Tool not installed",
};

function UsageLine({ account }) {
  if (!account) return <p className="exec-env__usage">No usage reported for this account yet</p>;
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

// Issue #7841 AC-2: the latest limit gwt saw the account hit, shown even when
// the provider reports no usage.
function LastLimit({ account }) {
  const limit = account?.lastLimit;
  if (!limit) return null;
  const detected = localTimeLabel(limit.detectedAt);
  const reset = localTimeLabel(limit.resetsAt);
  return (
    <p className="exec-env__meta" data-last-limit="">
      {"Last limit hit "}
      <time dateTime={limit.detectedAt}>{detected}</time>
      {reset ? (
        <>
          {" · resets "}
          <time dateTime={limit.resetsAt}>{reset}</time>
        </>
      ) : " · reset time not given"}
    </p>
  );
}

function LimitChip({ account }) {
  if (account?.limitReached) return <span className="exec-env__chip exec-env__chip--limit">Limit reached</span>;
  if (account?.usingCredits) return <span className="exec-env__chip">Using credits</span>;
  return null;
}

function poolLabel(environment) {
  const count = environment.poolSize;
  return `Using ${environment.accountName || "unknown"} · ${count} account${count === 1 ? "" : "s"} in pool`;
}

function Environments({ model, headingRef, onNavigate, onOpenToolManagement, onSetProjectEnvironments }) {
  const environments = model.environments || [];
  return (
    <>
      <div className="exec-env__header">
        <p className="exec-env__eyebrow">What your agents run on</p>
        <h1 ref={headingRef} tabIndex={-1}>Execution environments</h1>
        <p className="exec-env__lead">
          Each environment pairs an account with the tool agents reach it through. Each project chooses which ones it uses,
          and in what order.
        </p>
      </div>
      <div className="exec-env__actions">
        <button type="button" className="exec-env__primary" onClick={() => onNavigate("add")}>Add an execution environment</button>
      </div>
      {model.error ? <p className="exec-env__error" role="alert">{model.error}</p> : null}
      {environments.length === 0 ? (
        <div className="exec-env__empty">
          <h2>{model.loaded ? "No execution environments yet" : "Loading execution environments…"}</h2>
          <p>An environment is created when a tool that signs in to its own account is installed on this device.</p>
        </div>
      ) : (
        <ul className="exec-env__list" aria-label="Execution environments">
          {environments.map(environment => (
            <li className="exec-env__card" key={environment.id} data-environment={environment.id} data-kind={environment.kind} data-availability={environment.availability}>
              <div className="exec-env__card-head">
                <h2>{environment.name}</h2>
                <span className={`exec-env__chip exec-env__chip--${environment.availability}`}>
                  {AVAILABILITY_LABELS[environment.availability]}
                </span>
              </div>
              <p className="exec-env__meta">
                {environment.poolSize ? poolLabel(environment) : `Account ${environment.accountName || "unknown"}`}
                {` · Via ${environment.via}`}
                {environment.toolVersion ? ` ${environment.toolVersion}` : ""}
                {environment.projects.length === 0
                  ? " · Not on any project's list"
                  : ` · Used by ${environment.projects.map(project => project.title).join(", ")}`}
              </p>
              {environment.account ? <UsageLine account={environment.account} /> : (
                <p className="exec-env__usage">
                  {environment.kind === "set" ? "No usage data yet" : `${environment.via} uses the provider in its own settings; usage is not reported here.`}
                </p>
              )}
              <div className="exec-env__actions">
                <button type="button" aria-label={`${environment.readOnlyReason ? "Details for" : "Edit"} ${environment.name}`} onClick={() => onNavigate("edit", environment.id)}>
                  {environment.readOnlyReason ? "Details" : "Edit"}
                </button>
                {environment.inUseAccountId ? (
                  <button type="button" aria-label={`Open the ${environment.accountName} account`} onClick={() => onNavigate("account", environment.inUseAccountId)}>
                    Open account
                  </button>
                ) : null}
                {environment.updateAvailable ? (
                  <button type="button" aria-label={`Update ${environment.via} in Tool management`} onClick={onOpenToolManagement}>
                    Tool update available
                  </button>
                ) : null}
                <button
                  type="button"
                  className="exec-env__danger"
                  aria-label={`Delete ${environment.name}`}
                  disabled={Boolean(environment.deleteBlockedReason)}
                  aria-describedby={environment.deleteBlockedReason ? `exec-env-delete-${environment.id}` : undefined}
                  onClick={() => onNavigate("delete", environment.id)}
                >
                  Delete
                </button>
              </div>
              {environment.deleteBlockedReason ? (
                <p className="exec-env__reason" id={`exec-env-delete-${environment.id}`}>{environment.deleteBlockedReason}</p>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {/* Issue #7648 AC-6: the open project's priority list. A page without a project (the Hub) has none. */}
      {model.definitions?.project ? (
        <section className="exec-env__priority" aria-label="This project's priority">
          <ProjectPriorityList view={model.definitions} onSave={onSetProjectEnvironments} />
        </section>
      ) : null}
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

function Accounts({
  model, headingRef, onNavigate, onOpenToolManagement, onAddPoolAccount, onSignInPoolAccount,
  onUsePoolAccount, onMovePoolAccount, onDeletePoolAccount, onResumeNextPoolAccount, onMountLoginTerminal, onCloseLogin, onSaveCopilotAccount,
}) {
  const pools = model.pools || [];
  const accounts = pools.flatMap(pool => pool.accounts);
  const [dialog, setDialog] = useState(null);
  const deleting = accounts.find(account => account.id === dialog?.accountId);
  const login = model.login;
  const closeDialog = () => setDialog(null);
  const saveCopilot = (id, login, token) => {
    closeDialog();
    onSaveCopilotAccount(id, login, token);
  };
  useEffect(() => { if (login) setDialog(null); }, [login]);
  return (
    <>
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("environments")}>← Execution environments</button>
        <h1 ref={headingRef} tabIndex={-1}>Accounts</h1>
        <p className="exec-env__lead">
          {model.poolProviders ? "New sessions try signed-in accounts in order." : "Usage, reset times and limits are shown here, once per account."}
        </p>
      </div>
      {model.poolProviders ? (
        <div className="exec-env__actions">
          <button type="button" className="exec-env__primary" disabled={model.saving} onClick={() => setDialog({ kind: "add" })}>Add an account</button>
        </div>
      ) : null}
      {model.error && !dialog && !login ? <p className="exec-env__error" role="alert">{model.error}</p> : null}
      {pools.length === 0 ? (
        <div className="exec-env__empty">
          <h2>{model.loaded ? model.poolProviders ? "No accounts yet" : "No accounts report usage yet" : "Loading accounts…"}</h2>
          {model.poolProviders ? null : <p>An account appears once its tool has been used on this device.</p>}
        </div>
      ) : pools.map(pool => (
        <section className="exec-env__pool" key={pool.tool} data-pool={pool.tool} aria-label={`${pool.toolName} accounts`}>
          <h2>{pool.toolName}</h2>
          <p className="exec-env__meta">{`${pool.accounts.length} account${pool.accounts.length === 1 ? "" : "s"} in pool, in priority order`}</p>
          <ul className="exec-env__list">
          {pool.accounts.map(account => (
            <li className="exec-env__card" key={account.id} data-account={account.id} data-sign-in={account.signIn.kind}>
              <div className="exec-env__card-head">
                <h3>{account.name}</h3>
                <span className="exec-env__chips">
                  {account.inUse ? <span className="exec-env__chip">{account.registered ? "First choice" : "In use"}</span> : null}
                  <LimitChip account={account.usage} />
                </span>
              </div>
              <p className="exec-env__meta">
                {[
                  pool.toolName, account.signInIdentity || account.usage?.accountLabel, account.usage?.plan,
                  account.registered && account.managed && !account.signedIn ? "Waiting for sign-in" : account.signIn.label,
                  account.usage?.usage ? account.usage.status.label : null,
                ].filter(Boolean).join(" · ") || "Account"}
              </p>
              <UsageLine account={account.usage} />
              <LastLimit account={account.usage} />
              <div className="exec-env__actions">
                  <button type="button" aria-label={`Details for the ${account.name} account`} onClick={() => onNavigate("account", account.id)}>
                    Details
                  </button>
                {account.registered ? <>
                  <button type="button" aria-label={`Use ${account.name}`} disabled={!account.canUse || model.saving}
                    onClick={() => onUsePoolAccount(account.id)}>Use this account</button>
                  {account.canSignIn ? (
                    <button type="button" aria-label={`Sign in to ${account.name}`} disabled={model.saving}
                      onClick={() => account.tool === "gh"
                        ? setDialog({ kind: "copilot", accountId: account.id })
                        : onSignInPoolAccount(account.id)}>{account.signedIn ? "Sign in again" : "Sign in"}</button>
                  ) : null}
                  <button type="button" aria-label={`Move ${account.name} up`} disabled={!account.canMoveUp || model.saving}
                    onClick={() => onMovePoolAccount(account.id, "up")}>Up</button>
                  <button type="button" aria-label={`Move ${account.name} down`} disabled={!account.canMoveDown || model.saving}
                    onClick={() => onMovePoolAccount(account.id, "down")}>Down</button>
                  <button type="button" className="exec-env__danger" aria-label={`Delete ${account.name}`}
                    disabled={Boolean(account.deleteBlockedReason) || model.saving}
                    onClick={() => setDialog({ kind: "delete", accountId: account.id })}>Delete</button>
                </> : null}
              </div>
              {account.deleteBlockedReason ? <p className="exec-env__reason">{account.deleteBlockedReason}</p> : null}
            </li>
          ))}
          </ul>
        </section>
      ))}
      <AccountSessions sessions={model.activeSessions || []} accounts={accounts} saving={model.saving} onResume={onResumeNextPoolAccount} />
      <section className="exec-env__manage" aria-label="Manage">
        <div className="exec-env__manage-row">
          <button type="button" onClick={onOpenToolManagement}>Tool management</button>
          <p>Installing or updating a tool is separate from its account.</p>
        </div>
      </section>
      {login ? (
        <AccountLogin login={login} account={accounts.find(account => account.id === login.accountId)}
          error={model.error} onMount={onMountLoginTerminal} onClose={onCloseLogin} />
      ) : dialog?.kind === "add" ? (
        <AddPoolAccount options={model.poolProviders} saving={model.saving} error={model.error} onAdd={onAddPoolAccount} onSaveCopilot={saveCopilot} onDismiss={closeDialog} />
      ) : deleting && dialog.kind === "copilot" ? (
        <CopilotSignIn account={deleting} saving={model.saving} onSave={saveCopilot} onDismiss={closeDialog} />
      ) : deleting ? (
        <DeletePoolAccount account={deleting} onDismiss={closeDialog} onDelete={() => { closeDialog(); onDeletePoolAccount(deleting.id); }} />
      ) : null}
    </>
  );
}

function AccountDetail({ model, account, headingRef, onNavigate, renderAccountUsage, onSetUsageCollection }) {
  const slotRef = useRef(null);
  const usage = account?.usage || null;
  const readable = usage && usage.status.kind !== "unknown";
  // The windows, credits and consumption card is still drawn by the
  // unmigrated usage renderer; refresh it whenever the rows change.
  useEffect(() => {
    if (readable) fillLegacySlot(slotRef.current, element => renderAccountUsage(element, usage.provider, usage.poolAccountId));
  });
  if (!account) {
    return (
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("accounts")}>← Accounts</button>
        <h1 ref={headingRef} tabIndex={-1}>Account</h1>
        <p className="exec-env__lead">{model.loaded ? "This account is no longer registered." : "Loading account…"}</p>
      </div>
    );
  }
  return (
    <>
      <div className="exec-env__header">
        <button type="button" className="exec-env__back" onClick={() => onNavigate("accounts")}>← Accounts</button>
        <h1 ref={headingRef} tabIndex={-1}>{`${account.name} account`}</h1>
        <p className="exec-env__lead">{[account.signIn.label, usage?.status.label].filter(Boolean).join(" · ")}</p>
      </div>
      <section className="exec-env__detail" aria-label="Usage and limits">
        <h2>Usage and limits</h2>
        {readable ? (
          <div className="exec-env__usage-slot" ref={slotRef} data-usage-slot={`${usage.provider}:${usage.poolAccountId}`} />
        ) : (
          <UsageLine account={usage} />
        )}
        <LastLimit account={usage} />
      </section>
      {usage?.provider === "claude_code" ? (
        <section className="exec-env__detail" aria-label="Usage collection">
          <h2>Usage collection</h2>
          <label className="settings-toggle">
            <input
              type="checkbox"
              checked={usage.collectionEnabled}
              onChange={() => onSetUsageCollection(usage.provider, !usage.collectionEnabled)}
            />
            <span>Show Claude Code account usage (5-hour / weekly)</span>
          </label>
          <p className="exec-env__hint">
            Off by default (opt-in). When enabled, Claude account usage reads each account's OAuth token from the
            Keychain / credentials file and requests usage from the Anthropic API (polled at most once every 3 minutes
            per account). While disabled, no Keychain read or network request happens. Per-session token usage is read
            locally and is not affected by this setting.
          </p>
        </section>
      ) : usage?.provider === "codex" ? (
        <p className="exec-env__hint">Codex usage is read from the local session files of this account's conversations.</p>
      ) : null}
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
  onAdd = () => {},
  onUpdate = () => {},
  onDelete = () => {},
  onSetProjectEnvironments = () => {},
  onAddPoolAccount = () => {},
  onSignInPoolAccount = () => {},
  onSaveCopilotAccount = () => {},
  onMovePoolAccount = () => {},
  onUsePoolAccount = () => {},
  onDeletePoolAccount = () => {},
  onResumeNextPoolAccount = () => {},
  onMountLoginTerminal = () => {},
  onCloseLogin = () => {},
}) {
  const model = useStoreSelector(store, selectModel, { frame: true }) || {};
  const view = model.view || { name: "environments", key: null };
  const headingRef = useRef(null);
  const shownRef = useRef(null);
  const viewKey = `${view.name}:${view.key || ""}`;
  // Moving between views replaces the whole body, so keyboard focus follows
  // to the new heading. The first render leaves focus where the user had it.
  useEffect(() => {
    if (shownRef.current !== null && shownRef.current !== viewKey) focusElement(headingRef.current);
    shownRef.current = viewKey;
  }, [viewKey]);
  const account = (model.pools || []).flatMap(pool => pool.accounts).find(item => item.id === view.key) || null;
  const environment = (model.environments || []).find(item => item.id === view.key) || null;
  return (
    <section className="exec-env" aria-label="Execution environments" data-view={view.name}>
      {view.name === "accounts" ? (
        <Accounts
          model={model} headingRef={headingRef} onNavigate={onNavigate} onOpenToolManagement={onOpenToolManagement}
          onAddPoolAccount={onAddPoolAccount} onSignInPoolAccount={onSignInPoolAccount} onSaveCopilotAccount={onSaveCopilotAccount}
          onMovePoolAccount={onMovePoolAccount} onUsePoolAccount={onUsePoolAccount}
          onDeletePoolAccount={onDeletePoolAccount} onResumeNextPoolAccount={onResumeNextPoolAccount}
          onMountLoginTerminal={onMountLoginTerminal} onCloseLogin={onCloseLogin}
        />
      ) : view.name === "account" ? (
        <AccountDetail
          model={model}
          account={account}
          headingRef={headingRef}
          onNavigate={onNavigate}
          renderAccountUsage={renderAccountUsage}
          onSetUsageCollection={onSetUsageCollection}
        />
      ) : view.name === "add" ? (
        <AddEnvironment model={model} headingRef={headingRef} onNavigate={onNavigate} onAdd={onAdd} />
      ) : view.name === "edit" ? (
        <EditEnvironment model={model} environment={environment} headingRef={headingRef} onNavigate={onNavigate} onUpdate={onUpdate} />
      ) : view.name === "delete" ? (
        <DeleteEnvironment model={model} environment={environment} headingRef={headingRef} onNavigate={onNavigate} onDelete={onDelete} />
      ) : (
        <Environments
          model={model} headingRef={headingRef} onNavigate={onNavigate} onOpenToolManagement={onOpenToolManagement}
          onSetProjectEnvironments={onSetProjectEnvironments}
        />
      )}
    </section>
  );
}

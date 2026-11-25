import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Issue #7648 (SPEC #7007 T-030): the Execution environments screen is the
// one place provider usage, reset times and limit state are shown. This
// module derives its rows from the provider usage snapshot (SPEC-2970) and
// the supported agent catalog (SPEC #5017); it never invents an account, a
// sign-in state or a pool member the backend did not report.

export const USAGE_PROVIDER_NAME = Object.freeze({
  codex: "Codex", claude_code: "Claude Code", grok_build: "Grok Build", copilot: "GitHub Copilot", antigravity: "Antigravity",
});

// The supported-agent catalog id of the tool each provider account is
// reached through, and the vendor the account belongs to. The tool id is
// also the id of the provider's default (adopted) account (Issue #7841).
const PROVIDER_TOOL = Object.freeze({
  codex: Object.freeze({ agentId: "codex", vendor: "OpenAI" }),
  claude_code: Object.freeze({ agentId: "claude", vendor: "Anthropic" }),
  grok_build: Object.freeze({ agentId: "grok", vendor: "xAI" }),
  copilot: Object.freeze({ agentId: "gh", vendor: "GitHub" }),
  antigravity: Object.freeze({ agentId: "agy", vendor: "Google" }),
});

// The pool account a usage row belongs to (Issue #7841). A row from before
// accounts were pooled describes the provider's default account.
export function usagePoolAccountId(account) {
  return account?.pool_account_id || PROVIDER_TOOL[account?.provider]?.agentId || null;
}

function toolProvider(tool) {
  return Object.keys(PROVIDER_TOOL).find(provider => PROVIDER_TOOL[provider].agentId === tool) || null;
}

const SIGN_IN = Object.freeze({
  signed_in: Object.freeze({ kind: "signed_in", label: "Signed in" }),
  signed_out: Object.freeze({ kind: "signed_out", label: "Not signed in" }),
  unknown: Object.freeze({ kind: "unknown", label: "Sign-in unknown" }),
});

function usageProviderName(provider) {
  return USAGE_PROVIDER_NAME[provider] || String(provider || "Unknown");
}

function usageWindowPercent(window) {
  const value = window?.used_percent;
  if (typeof value !== "number" &&
      !(typeof value === "string" && value.trim() !== "")) return null;
  const percent = Number(value);
  return Number.isFinite(percent) ? Math.max(0, Math.min(100, percent)) : null;
}

const readable = account => ["ok", "stale"].includes(account?.state?.kind);

// The window nearest its limit. Claude's model-specific weekly windows are
// sub-limits of the shared quota, so only the shared windows govern.
function usageGoverningWindow(account) {
  if (account.provider === "claude_code" && !readable(account)) return null;
  let governing = null;
  for (const window of account.windows || []) {
    if (account.provider === "claude_code" &&
        window.kind !== "five_hour" && window.kind !== "weekly") continue;
    const percent = usageWindowPercent(window);
    if (percent == null) continue;
    if (!governing || percent > governing.percent) governing = { window, percent };
  }
  return governing;
}

function usageSummaryPercent(account) {
  return usageGoverningWindow(account)?.percent ?? null;
}

function codexCreditStatus(account) {
  if (account.provider !== "codex" || !account.credits || !readable(account)) return null;
  const full = (account.windows || []).some(window =>
    ["five_hour", "weekly"].includes(window.kind) && usageWindowPercent(window) === 100);
  return {
    active: full && account.credits.has_credits === true,
    exhausted: full && account.credits.has_credits === false,
  };
}

function usageDisplayLimitReached(account) {
  const credits = codexCreditStatus(account);
  return Boolean(!credits?.active && (account.limit_reached || credits?.exhausted));
}

function usageSeverity(account) {
  if (usageDisplayLimitReached(account)) return "danger";
  if (codexCreditStatus(account)?.active) return "normal";
  const percent = usageSummaryPercent(account);
  if (percent == null) return "";
  if (percent >= 95) return "danger";
  if (percent >= 80) return "warning";
  return "normal";
}

function creditBalance(credits) {
  if (credits.unlimited) return "unlimited";
  return Number.isFinite(credits.balance) && credits.balance >= 0
    ? `${credits.balance.toLocaleString("en-US", { maximumFractionDigits: 2 })} credits` : "— credits";
}

// What is known about the account's usage data. A degraded state is named
// instead of being shown as zero usage.
function accountStatus(account) {
  const state = account.state || {};
  switch (state.kind) {
    case "disabled":
      return { kind: "disabled", label: "Usage collection is off" };
    case "no_data":
      return { kind: "no_data", label: "No usage data yet" };
    case "unavailable":
      return { kind: "unavailable", label: state.reason ? `Unavailable — ${state.reason}` : "Unavailable" };
    case "stale":
      return { kind: "stale", label: `Last read ${Math.round((state.age_secs || 0) / 60)} min ago` };
    case "unknown":
      return { kind: "unknown", label: state.reason ? `Usage unknown (${state.reason})` : "Usage unknown" };
    default:
      return { kind: "ok", label: "Up to date" };
  }
}

// A limit gwt itself saw this account hit, kept even when the provider
// reports no usage (Issue #7841 AC-2).
function lastLimit(account) {
  const limit = account.last_limit;
  if (!limit?.detected_at) return null;
  return { detectedAt: limit.detected_at, resetsAt: limit.resets_at || null };
}

// One row per account the backend reported: its usage, reset and limit
// state, each read from the same snapshot. Rows are keyed by provider and
// pool account; a row from before accounts were pooled is the default one.
export function accountRows(snapshot, now = Date.now()) {
  return (snapshot?.accounts || []).map(account => {
    const status = accountStatus(account);
    const credits = codexCreditStatus(account);
    const governing = readable(account) ? usageGoverningWindow(account) : null;
    const limit = lastLimit(account);
    // A provider that reports usage decides its own limit state; without
    // usage, a limit gwt saw counts until the reset it announced.
    const limitPending = !readable(account) && limit?.resetsAt != null && Date.parse(limit.resetsAt) > now;
    const limitReached = usageDisplayLimitReached(account) || limitPending;
    let usage = null;
    if (credits?.active) usage = `Using credits · ${creditBalance(account.credits)}`;
    else if (governing) usage = `${Math.round(governing.percent)}% used`;
    return {
      provider: account.provider,
      poolAccountId: usagePoolAccountId(account),
      name: usageProviderName(account.provider),
      accountLabel: account.account_label || null,
      plan: account.plan || null,
      status,
      usage,
      usedPercent: credits?.active ? null : governing?.percent ?? null,
      resetsAt: governing?.window.resets_at || null,
      limitReached,
      lastLimit: limit,
      usingCredits: Boolean(credits?.active),
      severity: limitPending ? "danger" : usageSeverity(account),
      collectionEnabled: status.kind !== "disabled",
    };
  });
}

// Issue #7841: the registered accounts, one pool per tool in priority order,
// each joined with its own usage row. The first account is the one in use.
export function accountPools(view, usage) {
  const pools = [];
  const registered = Array.isArray(view?.pool_providers);
  const availableTools = new Set((view?.pool_providers || []).filter(option => !option.blocked_reason).map(option => option.tool));
  for (const account of view?.accounts || []) {
    let pool = pools.find(candidate => candidate.tool === account.tool);
    if (!pool) {
      pool = { tool: account.tool, toolName: account.tool_name || account.tool, accounts: [] };
      pools.push(pool);
    }
    const provider = toolProvider(account.tool);
    const signIn = SIGN_IN[account.sign_in] || SIGN_IN.unknown;
    const signedIn = signIn.kind === "signed_in";
    pool.accounts.push({
      id: account.id,
      name: account.name,
      tool: account.tool,
      provider,
      signIn,
      signInIdentity: account.sign_in_identity,
      registered,
      signedIn,
      managed: account.id !== account.tool,
      canSignIn: !account.sign_in_blocked_reason && availableTools.has(account.tool),
      deleteBlockedReason: account.delete_blocked_reason,
      deleteImpact: account.delete_impact,
      usage: provider ? usage.find(row => row.provider === provider && row.poolAccountId === account.id) || null : null,
    });
  }
  for (const pool of pools) {
    const first = registered && pool.accounts.some(account => account.signIn.kind !== "unknown")
      ? pool.accounts.find(account => account.signedIn)
      : pool.accounts[0];
    pool.accounts.forEach((account, index) => {
      account.inUse = account === first;
      account.canUse = account.signedIn && index > 0;
      account.canMoveUp = index > 0;
      account.canMoveDown = index < pool.accounts.length - 1;
    });
  }
  return pools;
}

// One row per execution environment the backend has defined (Issue #7741).
// Usage is the account's, so it is shown on the environment that runs the
// account's own tool; a tool that reads its own settings reports none. An
// account's own environment runs its pool: it shows the pool's size and the
// account in use (Issue #7841 AC-3).
export function environmentRows(view, snapshot, agents, now = Date.now()) {
  const usage = accountRows(snapshot, now);
  const pools = accountPools(view, usage);
  const accounts = view?.accounts || [];
  return (view?.environments || []).map(environment => {
    const registered = accounts.find(account => account.id === environment.account_id) || null;
    const pool = environment.kind === "set" ? pools.find(candidate => candidate.tool === registered?.tool) || null : null;
    const inUse = pool?.accounts.find(account => account.inUse) || null;
    const account = inUse?.usage || null;
    const agent = Array.isArray(agents)
      ? agents.find(candidate => candidate.id === environment.via_tool) || null : null;
    const availability = !environment.tool_installed ? "not_installed"
      : account?.limitReached ? "limit"
        : account && (account.status.kind === "ok" || account.status.kind === "stale") ? "available" : "unknown";
    return {
      id: environment.id,
      name: environment.name,
      kind: environment.kind,
      accountName: inUse?.name || environment.account_name,
      poolSize: pool ? pool.accounts.length : null,
      inUseAccountId: inUse?.id || null,
      usageProvider: account?.provider || null,
      via: environment.via_tool_name,
      viaTool: environment.via_tool,
      toolVersion: agent?.installed_version?.trim() || null,
      updateAvailable: Boolean(agent?.update_available),
      availability,
      account,
      runtime: environment.runtime || {},
      deleteBlockedReason: environment.delete_blocked_reason || null,
      readOnlyReason: environment.runtime_read_only_reason || null,
      toolSettings: environment.tool_settings || null,
      projects: environment.projects || [],
      deleteImpact: environment.delete_impact,
    };
  });
}

const VIEWS = new Set(["environments", "accounts", "account", "add", "edit", "delete"]);

// Execution environments host. Rendered by the React surface
// `ExecutionEnvironments` (crates/gwt/web-react). The definitions come from
// the backend (`execution_environments`); the usage snapshot and the tool
// catalog stay owned by their models. This host only derives rows.
export function createExecutionEnvironments({
  document, usageModel, supportedAgentsModel, renderAccountUsage,
  onOpenToolManagement, onSetUsageCollection, openLoginTerminal, loginEventModel, definitionsModel = null, send = () => {}, scheduleFrame,
}) {
  let definitions = null;
  let login = null;
  // Whether this screen is waiting for an answer to its own request.
  let pending = false;
  const derive = () => {
    const snapshot = usageModel.read();
    const agents = supportedAgentsModel?.read()?.agents ?? null;
    const accounts = accountRows(snapshot);
    return {
      loaded: definitions != null,
      definitions,
      accounts,
      pools: accountPools(definitions, accounts),
      poolProviders: definitions?.pool_providers,
      activeSessions: definitions?.active_sessions || [],
      environments: environmentRows(definitions, snapshot, agents),
      registeredAccounts: definitions?.accounts || [],
      viaOptions: definitions?.via_options || [],
    };
  };
  const model = createUiStateStore({
    ...derive(), view: { name: "environments", key: null }, error: null, saving: false, saved: false,
  }, { scheduleFrame });
  const refresh = () => model.update(state => ({ ...state, ...derive() }));
  const releases = [usageModel.subscribe(state => state, refresh)];
  if (supportedAgentsModel) releases.push(supportedAgentsModel.subscribe(state => state.agents, refresh));
  // What to show once the pending request is answered.
  let afterSave = null;
  function open(name = "environments", key = null) {
    if (!VIEWS.has(name)) return;
    if (name !== "accounts") closeLogin(false);
    afterSave = null;
    model.update(state => ({
      ...state, error: null, saved: false,
      view: { name, key: ["account", "edit", "delete"].includes(name) ? key : null },
    }));
  }
  function request(message, next = null) {
    afterSave = next;
    pending = true;
    model.update(state => ({ ...state, saving: true, saved: false, error: null }));
    if (send(message) === "unavailable") applyError("The connection is unavailable. Try again when it reconnects.");
  }
  // `execution_environments`: the definitions after a read or a change.
  function applyView(view) {
    definitions = view;
    pending = false;
    const next = afterSave;
    afterSave = null;
    model.update(state => ({
      ...state, ...derive(), error: null, saved: state.saving, saving: false,
      login: state.login?.checking ? { ...state.login, checking: false } : state.login,
      view: next && state.saving ? next : state.view,
    }));
  }
  // `execution_environment_error`: the request was refused; say why in place.
  // A refusal of a request this screen did not send belongs elsewhere.
  function applyError(message) {
    if (!pending) return;
    pending = false;
    afterSave = null;
    model.update(state => ({ ...state, saving: false, saved: false, error: message }));
  }
  function applyLoginEvent(event) {
    if (event.kind === "pool_account_login_started") {
      if (container.hidden) {
        send({ kind: "close_pool_account_login", login_id: event.login_id });
        return;
      }
      login = { id: event.login_id, chunks: [], terminal: null };
      model.update(state => ({
        ...state, error: null, view: { name: "accounts", key: null },
        login: { id: event.login_id, accountId: event.account_id, finished: false },
      }));
      return;
    }
    if (event.login_id !== login?.id) return;
    if (event.kind === "pool_account_login_output") {
      if (login.terminal) login.terminal.appendOutput(event.data_base64);
      else login.chunks.push(event.data_base64);
    } else if (event.kind === "pool_account_login_finished") {
      model.update(state => ({ ...state, login: { ...state.login, finished: true, checking: true } }));
      reload();
    }
  }
  function mountLoginTerminal(element, id) {
    if (!openLoginTerminal || login?.id !== id) return undefined;
    const current = login;
    const terminal = openLoginTerminal(element, data => {
      if (login === current && !model.read().login?.finished) send({ kind: "pool_account_login_input", login_id: id, data });
    });
    current.terminal = terminal;
    for (const chunk of current.chunks) terminal.appendOutput(chunk);
    current.chunks.length = 0;
    return () => {
      terminal.dispose();
      current.terminal = null;
    };
  }
  function closeLogin(refresh = true) {
    if (!login) return;
    const { id } = login;
    login = null;
    const finished = model.read().login?.finished;
    model.update(state => ({ ...state, login: null }));
    if (!finished) {
      send({ kind: "close_pool_account_login", login_id: id });
      if (refresh) reload();
    }
  }
  if (definitionsModel) {
    releases.push(definitionsModel.subscribe(state => state.view, view => { if (view) applyView(view); }));
    releases.push(definitionsModel.subscribe(state => state.refusal, refusal => { if (refusal) applyError(refusal.message); }));
  }
  const container = createDetachedIslandContainer(document);
  if (loginEventModel) releases.push(loginEventModel.subscribe(state => state, event => { if (event) applyLoginEvent(event); }));
  const island = mountIsland("execution-environments", container, {
    store: model, renderAccountUsage, onNavigate: open, onOpenToolManagement, onSetUsageCollection,
    onAdd: input => request({ kind: "add_execution_environment", ...input }, { name: "environments", key: null }),
    onUpdate: (id, change) => request({ kind: "update_execution_environment", environment_id: id, ...change }),
    onDelete: id => request({ kind: "delete_execution_environment", environment_id: id }, { name: "environments", key: null }),
    onSetProjectEnvironments: ids => request({ kind: "set_project_execution_environments", environment_ids: ids }),
    onAddPoolAccount: tool => request({ kind: "add_pool_account", tool }),
    onSignInPoolAccount: account_id => request({ kind: "sign_in_pool_account", account_id }),
    onSaveCopilotAccount: (account_id, login, token) => request({ kind: "save_copilot_account", account_id, login, token }),
    onMovePoolAccount: (account_id, direction) => request({ kind: "move_pool_account", account_id, direction }),
    onUsePoolAccount: account_id => request({ kind: "use_pool_account", account_id }),
    onDeletePoolAccount: account_id => request({ kind: "delete_pool_account", account_id }),
    onResumeNextPoolAccount: id => request({ kind: "resume_next_pool_account", id }),
    onMountLoginTerminal: mountLoginTerminal, onCloseLogin: () => closeLogin(),
  });
  function dispose() {
    closeLogin(false);
    for (const release of releases) release?.();
    island.unmount();
  }
  function reload() {
    pending = true;
    send({ kind: "get_execution_environments" });
  }
  function refreshVisibleAccounts() {
    if (!container.hidden && !pending && model.read().view.name === "accounts") reload();
  }
  function connectionLost() {
    loginEventModel?.update(() => null);
    if (login || pending) {
      const message = login
        ? "The login terminal closed when the connection was lost."
        : "The connection was lost before the request was confirmed.";
      login = null;
      pending = false;
      afterSave = null;
      model.update(state => ({ ...state, login: null, saving: false, error: message }));
    }
  }
  return { element: container, open, reload, refreshVisibleAccounts, closeLogin, connectionLost, dispose };
}

import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Issue #7648 (SPEC #7007 T-030): the Execution environments screen is the
// one place provider usage, reset times and limit state are shown. This
// module derives its rows from the provider usage snapshot (SPEC-2970) and
// the supported agent catalog (SPEC #5017); it never invents an account, a
// sign-in state or a pool member the backend did not report.

export const USAGE_PROVIDER_NAME = Object.freeze({ codex: "Codex", claude_code: "Claude Code" });

// The supported-agent catalog id of the tool each provider account is
// reached through, and the vendor the account belongs to.
const PROVIDER_TOOL = Object.freeze({
  codex: Object.freeze({ agentId: "codex", vendor: "OpenAI" }),
  claude_code: Object.freeze({ agentId: "claude", vendor: "Anthropic" }),
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
    default:
      return { kind: "ok", label: "Up to date" };
  }
}

// One row per account the backend reported: its usage, reset and limit
// state, each read from the same snapshot.
export function accountRows(snapshot) {
  return (snapshot?.accounts || []).map(account => {
    const status = accountStatus(account);
    const credits = codexCreditStatus(account);
    const governing = readable(account) ? usageGoverningWindow(account) : null;
    const limitReached = usageDisplayLimitReached(account);
    let usage = null;
    if (credits?.active) usage = `Using credits · ${creditBalance(account.credits)}`;
    else if (governing) usage = `${Math.round(governing.percent)}% used`;
    return {
      provider: account.provider,
      name: usageProviderName(account.provider),
      accountLabel: account.account_label || null,
      plan: account.plan || null,
      status,
      usage,
      usedPercent: credits?.active ? null : governing?.percent ?? null,
      resetsAt: governing?.window.resets_at || null,
      limitReached,
      usingCredits: Boolean(credits?.active),
      severity: usageSeverity(account),
      collectionEnabled: status.kind !== "disabled",
    };
  });
}

// One row per execution environment the backend has defined (Issue #7741).
// Usage is the account's, so it is shown on the environment that runs the
// account's own tool; a tool that reads its own settings reports none.
export function environmentRows(view, snapshot, agents) {
  const usage = accountRows(snapshot);
  const accounts = view?.accounts || [];
  return (view?.environments || []).map(environment => {
    const registered = accounts.find(account => account.id === environment.account_id) || null;
    const account = environment.kind === "set"
      ? usage.find(row => PROVIDER_TOOL[row.provider]?.agentId === registered?.tool) || null : null;
    const agent = Array.isArray(agents)
      ? agents.find(candidate => candidate.id === environment.via_tool) || null : null;
    const availability = !environment.tool_installed ? "not_installed"
      : account?.limitReached ? "limit"
        : account && (account.status.kind === "ok" || account.status.kind === "stale") ? "available" : "unknown";
    return {
      id: environment.id,
      name: environment.name,
      kind: environment.kind,
      accountName: environment.account_name,
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
  onOpenToolManagement, onSetUsageCollection, definitionsModel = null, send = () => {}, scheduleFrame,
}) {
  let definitions = null;
  // Whether this screen is waiting for an answer to its own request.
  let pending = false;
  const derive = () => {
    const snapshot = usageModel.read();
    const agents = supportedAgentsModel?.read()?.agents ?? null;
    return {
      loaded: definitions != null,
      accounts: accountRows(snapshot),
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
    send(message);
  }
  // `execution_environments`: the definitions after a read or a change.
  function applyView(view) {
    definitions = view;
    pending = false;
    const next = afterSave;
    afterSave = null;
    model.update(state => ({
      ...state, ...derive(), error: null, saved: state.saving, saving: false,
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
  if (definitionsModel) {
    releases.push(definitionsModel.subscribe(state => state.view, view => { if (view) applyView(view); }));
    releases.push(definitionsModel.subscribe(state => state.refusal, refusal => { if (refusal) applyError(refusal.message); }));
  }
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("execution-environments", container, {
    store: model, renderAccountUsage, onNavigate: open, onOpenToolManagement, onSetUsageCollection,
    onAdd: input => request({ kind: "add_execution_environment", ...input }, { name: "environments", key: null }),
    onUpdate: (id, change) => request({ kind: "update_execution_environment", environment_id: id, ...change }),
    onDelete: id => request({ kind: "delete_execution_environment", environment_id: id }, { name: "environments", key: null }),
  });
  function dispose() {
    for (const release of releases) release?.();
    island.unmount();
  }
  function reload() {
    pending = true;
    send({ kind: "get_execution_environments" });
  }
  return { element: container, open, reload, dispose };
}

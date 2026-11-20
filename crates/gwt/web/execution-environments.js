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

// One row per provider account that agents can actually be run through: the
// tool has to be installed. Before the tool catalog has arrived the row is
// kept, because an unknown install state is not a missing tool.
export function environmentRows(snapshot, agents) {
  const rows = [];
  for (const account of accountRows(snapshot)) {
    const tool = PROVIDER_TOOL[account.provider];
    const agent = tool && Array.isArray(agents)
      ? agents.find(candidate => candidate.id === tool.agentId) || null : null;
    if (Array.isArray(agents) && tool && (!agent || !agent.installed)) continue;
    const availability = account.limitReached ? "limit"
      : account.status.kind === "ok" || account.status.kind === "stale" ? "available" : "unknown";
    rows.push({
      id: account.provider,
      name: account.name,
      vendor: tool?.vendor || null,
      via: agent?.name || account.name,
      toolVersion: agent?.installed_version?.trim() || null,
      updateAvailable: Boolean(agent?.update_available),
      members: 1,
      availability,
      account,
    });
  }
  return rows;
}

const VIEWS = new Set(["environments", "accounts", "account"]);

// Execution environments host. Rendered by the React surface
// `ExecutionEnvironments` (crates/gwt/web-react). The usage snapshot and the
// tool catalog stay owned by their models; this host only derives rows.
export function createExecutionEnvironments({
  document, usageModel, supportedAgentsModel, renderAccountUsage,
  onOpenToolManagement, onSetUsageCollection, scheduleFrame,
}) {
  const derive = () => {
    const snapshot = usageModel.read();
    const agents = supportedAgentsModel?.read()?.agents ?? null;
    return { loaded: snapshot != null, accounts: accountRows(snapshot), environments: environmentRows(snapshot, agents) };
  };
  const model = createUiStateStore({ ...derive(), view: { name: "environments", provider: null } }, { scheduleFrame });
  const refresh = () => model.update(state => ({ ...state, ...derive() }));
  const releases = [usageModel.subscribe(state => state, refresh)];
  if (supportedAgentsModel) releases.push(supportedAgentsModel.subscribe(state => state.agents, refresh));
  function open(name = "environments", provider = null) {
    if (!VIEWS.has(name)) return;
    model.update(state => ({ ...state, view: { name, provider: name === "account" ? provider : null } }));
  }
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("execution-environments", container, {
    store: model, renderAccountUsage, onNavigate: open, onOpenToolManagement, onSetUsageCollection,
  });
  function dispose() {
    for (const release of releases) release?.();
    island.unmount();
  }
  return { element: container, open, dispose };
}

import test from "node:test";
import assert from "node:assert/strict";
import { accountPools, accountRows, environmentRows, providerRows, usagePoolAccountId } from "../execution-environments.js";

// Issue #7648 (SPEC #7007 T-030): rows of the Execution environments screen
// are derived from the provider usage snapshot and the tool catalog.
const RESET = "2026-11-21T06:00:00Z";
const claude = (extra = {}) => ({
  provider: "claude_code",
  account_label: "work@example.test",
  plan: "Max",
  state: { kind: "ok" },
  limit_reached: false,
  windows: [
    { kind: "five_hour", used_percent: 12, resets_at: "2026-11-20T09:00:00Z" },
    { kind: "weekly", used_percent: 58, resets_at: RESET },
    { kind: "opus_weekly", used_percent: 99, resets_at: RESET },
  ],
  ...extra,
});
const codex = (extra = {}) => ({
  provider: "codex",
  state: { kind: "ok" },
  limit_reached: false,
  windows: [{ kind: "weekly", used_percent: 90, resets_at: RESET }],
  ...extra,
});
// SPEC #7347: the backend's provider list, in display order.
const descriptor = (provider, display_name, short_label, acquisition, default_account_id) =>
  ({ provider, display_name, short_label, acquisition, default_account_id });
const PROVIDERS = [
  descriptor("claude_code", "Claude Code", "CC", "api", "claude"),
  descriptor("codex", "Codex", "CX", "cli_output", "codex"),
  descriptor("grok_build", "Grok Build", "GB", "none", "grok"),
  descriptor("antigravity", "Antigravity CLI", "AG", "none", "agy"),
  descriptor("opencode", "OpenCode", "OC", "none", "opencode"),
  descriptor("openclaw", "OpenClaw", "OW", "none", "openclaw"),
  descriptor("hermes", "Hermes Agent", "HA", "none", "hermes"),
  descriptor("copilot", "GitHub Copilot", "GH", "none", "gh"),
];
const agents = [
  { id: "claude", name: "Claude Code", installed: true, installed_version: "2.1.3", update_available: false },
  { id: "codex", name: "Codex", installed: true, installed_version: "0.164.0", update_available: true },
];

// AC-2: usage, reset and limit state are read from the snapshot as reported.
test("account rows report the governing window's usage, reset and limit state", () => {
  const [row] = accountRows({ accounts: [claude()] });
  assert.equal(row.usage, "58% used", "model sub-limits do not govern the shared quota");
  assert.equal(row.resetsAt, RESET);
  assert.equal(row.limitReached, false);
  assert.equal(row.severity, "normal");

  const [limited] = accountRows({ accounts: [claude({ limit_reached: true })] });
  assert.equal(limited.limitReached, true);
  assert.equal(limited.severity, "danger");

  const [warn, danger] = accountRows({ accounts: [
    codex({ windows: [{ kind: "weekly", used_percent: 80 }] }),
    codex({ windows: [{ kind: "weekly", used_percent: 95 }] }),
  ] });
  assert.deepEqual([warn.severity, danger.severity], ["warning", "danger"]);
});

test("a degraded account names its state instead of showing zero usage", () => {
  const rows = accountRows({ accounts: [
    claude({ state: { kind: "disabled" }, windows: [] }),
    codex({ state: { kind: "no_data" }, windows: [] }),
    codex({ state: { kind: "unavailable", reason: "token expired" }, windows: [] }),
    codex({ state: { kind: "stale", age_secs: 600 } }),
    codex({ state: { kind: "unknown", reason: "No official usage source" }, windows: [] }),
  ] });
  assert.deepEqual(rows.map(row => row.usage), [null, null, null, "90% used", null]);
  assert.deepEqual(rows.map(row => row.status.label), [
    "Usage collection is off", "No usage data yet", "Unavailable — token expired", "Last read 10 min ago",
    "Usage unknown (No official usage source)",
  ]);
  assert.equal(rows[0].collectionEnabled, false);
});

test("a full Codex subscription distinguishes available credits from an exhausted limit", () => {
  const full = { windows: [{ kind: "weekly", used_percent: 100, resets_at: RESET }] };
  const [paying] = accountRows({ accounts: [codex({ ...full, credits: { has_credits: true, balance: 12.5 } })] });
  assert.equal(paying.usage, "Using credits · 12.5 credits");
  assert.equal(paying.usingCredits, true);
  assert.equal(paying.limitReached, false);
  const [out] = accountRows({ accounts: [codex({ ...full, credits: { has_credits: false, balance: 0 } })] });
  assert.equal(out.limitReached, true);
  assert.equal(out.usage, "100% used");
});

// Issue #7741: environments are the backend's definitions; usage belongs to
// the account and is shown on the environment that runs the account's own tool.
const definition = (id, extra = {}) => ({
  id, name: `${id} name`, kind: "set", account_id: id, account_name: `${id} account`,
  via_tool: id, via_tool_name: id === "claude" ? "Claude Code" : "Codex",
  runtime: {}, tool_installed: true, projects: [], ...extra,
});
const view = (...environments) => ({
  accounts: [{ id: "claude", tool: "claude", name: "Claude Code 1" }, { id: "codex", tool: "codex", name: "Codex 1" }],
  environments,
});
const NOW = Date.parse("2026-11-24T03:00:00Z");

test("an environment row joins its definition with the account's usage and the tool's version", () => {
  const snapshot = { providers: PROVIDERS, accounts: [claude({ limit_reached: true }), codex()] };
  const rows = environmentRows(view(definition("claude"), definition("codex")), snapshot, agents);
  assert.deepEqual(rows.map(row => [row.id, row.via, row.toolVersion, row.availability, row.usageProvider]), [
    ["claude", "Claude Code", "2.1.3", "limit", "claude_code"],
    ["codex", "Codex", "0.164.0", "available", "codex"],
  ]);
  assert.equal(rows[1].updateAvailable, true);
  assert.equal(rows[0].account.usage, "58% used");
  assert.deepEqual(environmentRows(null, snapshot, agents), [], "nothing is shown before the definitions arrive");
});

test("an environment keeps its row when its tool is missing or reports no usage", () => {
  const through = definition("claude:hermes", {
    kind: "via", account_id: "claude", via_tool: "hermes", via_tool_name: "Hermes Agent",
    runtime_read_only_reason: "Hermes Agent reads these from its own settings file.",
    tool_settings: { path: "/home/.hermes/config.yaml", values: [{ label: "Model", value: "glm-4.6" }] },
    projects: [{ project_root: "/p/alpha", title: "alpha", priority: 1 }],
  });
  const rows = environmentRows(
    view(definition("claude", { tool_installed: false, delete_blocked_reason: "Created with the account." }), through, definition("codex")),
    { accounts: [claude(), codex({ state: { kind: "no_data" }, windows: [] })] }, agents);
  assert.deepEqual(rows.map(row => row.availability), ["not_installed", "unknown", "unknown"]);
  assert.equal(rows[0].deleteBlockedReason, "Created with the account.");
  assert.equal(rows[1].account, null, "the account's usage is not repeated on another tool");
  assert.equal(rows[1].readOnlyReason, "Hermes Agent reads these from its own settings file.");
  assert.equal(rows[1].toolSettings.values[0].value, "glm-4.6");
  assert.deepEqual(rows[1].projects.map(project => project.title), ["alpha"]);
});

// Issue #7841 AC-1 / AC-4: rows are keyed by provider and pool account; a
// row from before accounts were pooled is the provider's default account,
// as the provider list names it (SPEC #7347 FR-004).
test("usage rows are keyed by pool account and old rows read as the default account", () => {
  assert.equal(usagePoolAccountId(claude(), PROVIDERS), "claude");
  assert.equal(usagePoolAccountId(codex(), PROVIDERS), "codex");
  assert.equal(usagePoolAccountId(claude({ pool_account_id: "claude-2" }), PROVIDERS), "claude-2");
  assert.equal(usagePoolAccountId({ provider: "copilot" }, PROVIDERS), "gh");
  assert.equal(usagePoolAccountId({ provider: "copilot" }), null, "no list, no guess");
});

// Issue #7841 AC-2: a tool without an official usage source says so, and a
// limit gwt saw stays visible until the reset it announced.
test("an account without an official usage source names the reason and keeps the last limit", () => {
  const grok = (last_limit) => ({
    provider: "grok_build", pool_account_id: "grok", state: { kind: "unknown", reason: "no official usage source" },
    windows: [], limit_reached: false, last_limit,
  });
  const [pending] = accountRows({ accounts: [grok({ detected_at: "2026-11-24T02:00:00Z", resets_at: "2026-11-24T06:00:00Z" })] }, NOW);
  assert.equal(pending.usage, null);
  assert.equal(pending.status.label, "Usage unknown (no official usage source)");
  assert.deepEqual(pending.lastLimit, { detectedAt: "2026-11-24T02:00:00Z", resetsAt: "2026-11-24T06:00:00Z" });
  assert.equal(pending.limitReached, true, "the announced reset has not come yet");
  const [reset] = accountRows({ accounts: [grok({ detected_at: "2026-11-24T00:00:00Z", resets_at: "2026-11-24T01:00:00Z" })] }, NOW);
  assert.equal(reset.limitReached, false);
  assert.equal(reset.lastLimit.resetsAt, "2026-11-24T01:00:00Z", "the last limit is still shown after its reset");
  const [readableLimit] = accountRows({ accounts: [codex({ last_limit: { detected_at: "2026-11-24T02:00:00Z", resets_at: "2026-11-24T06:00:00Z" } })] }, NOW);
  assert.equal(readableLimit.limitReached, false, "a provider that reports usage decides its own limit state");
});

// Issue #7841 AC-1: accounts are grouped into one pool per tool, in priority
// order, each joined with its own usage and sign-in state.
test("accounts form one pool per tool with their own usage and sign-in state", () => {
  const definitions = {
    accounts: [
      { id: "claude", tool: "claude", tool_name: "Claude Code", name: "Claude Code 1", sign_in: "signed_in" },
      { id: "codex", tool: "codex", tool_name: "Codex", name: "Codex 1", sign_in: "signed_in" },
      { id: "claude-2", tool: "claude", tool_name: "Claude Code", name: "Claude Code 2", sign_in: "signed_out" },
      { id: "agy", tool: "agy", tool_name: "Antigravity", name: "Antigravity 1", sign_in: "unknown" },
    ],
  };
  const usage = accountRows({ providers: PROVIDERS, accounts: [
    codex(), claude(), claude({ pool_account_id: "claude-2", account_label: "second@example.test", plan: "Pro" }),
  ] }, NOW);
  const pools = accountPools(definitions, usage);
  assert.deepEqual(pools.map(pool => [pool.tool, pool.toolName, pool.accounts.map(account => account.id)]), [
    ["claude", "Claude Code", ["claude", "claude-2"]],
    ["codex", "Codex", ["codex"]],
    ["agy", "Antigravity", ["agy"]],
  ]);
  const [first, second] = pools[0].accounts;
  assert.equal(first.inUse, true);
  assert.equal(second.inUse, false);
  assert.equal(first.usage.accountLabel, "work@example.test");
  assert.equal(second.usage.accountLabel, "second@example.test");
  assert.equal(second.usage.plan, "Pro");
  assert.deepEqual([first.signIn.label, second.signIn.label, pools[2].accounts[0].signIn.label],
    ["Signed in", "Not signed in", "Sign-in unknown"]);
  assert.equal(pools[2].accounts[0].usage, null, "no row is invented for an account the poller did not report");
});

// Issue #7841 AC-3: an account's own environment shows its pool's size and
// the account in use, with that account's usage.
test("an account's own environment shows the pool size and the account in use", () => {
  const definitions = {
    accounts: [
      { id: "claude-2", tool: "claude", name: "Claude Code 2" },
      { id: "claude", tool: "claude", name: "Claude Code 1" },
      { id: "codex", tool: "codex", name: "Codex 1" },
    ],
    environments: [definition("claude"), definition("codex")],
  };
  const snapshot = { providers: PROVIDERS, accounts: [claude({ limit_reached: true }), codex(), claude({ pool_account_id: "claude-2" })] };
  const [own, single] = environmentRows(definitions, snapshot, agents, NOW);
  assert.deepEqual([own.poolSize, own.accountName, own.inUseAccountId, own.availability], [2, "Claude Code 2", "claude-2", "available"]);
  assert.equal(own.account.poolAccountId, "claude-2", "usage is the account in use, not the provider's default");
  assert.deepEqual([single.poolSize, single.accountName], [1, "Codex 1"]);
});

// SPEC #7347 AC-1 / AC-2: every listed provider has a row, named and
// labelled as the backend lists it; one whose usage cannot be read says n/a
// and why, instead of being hidden.
test("provider rows list every provider and mark unreadable usage as n/a", () => {
  const unknown = { kind: "unknown", reason: "no official usage source" };
  const rows = providerRows({
    providers: PROVIDERS,
    accounts: [
      claude({ pool_account_id: "claude" }),
      claude({ pool_account_id: "claude-2", windows: [{ kind: "weekly", used_percent: 3 }] }),
      codex({ pool_account_id: "codex" }),
      { provider: "grok_build", pool_account_id: "grok", state: unknown, windows: [], limit_reached: false },
      { provider: "copilot", pool_account_id: "gh", state: { kind: "unavailable", reason: "timeout" }, windows: [], limit_reached: false },
    ],
  });
  assert.deepEqual(rows.map(row => [row.provider, row.name, row.shortLabel]),
    PROVIDERS.map(p => [p.provider, p.display_name, p.short_label]));
  const by = provider => rows.find(row => row.provider === provider);
  assert.equal(by("claude_code").usage, "58% used", "the default account's row");
  assert.equal(by("claude_code").na, false);
  assert.equal(by("codex").usage, "90% used");
  assert.equal(by("codex").severity, "warning");
  for (const provider of ["grok_build", "antigravity", "opencode", "openclaw", "hermes"]) {
    assert.equal(by(provider).na, true, provider);
    assert.equal(by(provider).usage, "n/a", provider);
    assert.match(by(provider).naReason, /no way to read .* usage/i, provider);
    assert.match(by(provider).naReason, /limits are unknown/i, provider);
  }
  assert.equal(by("copilot").na, true);
  assert.match(by("copilot").naReason, /could not be read \(timeout\)/);
  assert.match(by("copilot").naReason, /limits are unknown/i);
});

// SPEC #7347 AC-3: a provider the display has never heard of is listed and
// drawn from the backend's list alone.
test("a new provider is listed without display changes", () => {
  const rows = providerRows({
    providers: [...PROVIDERS, descriptor("test_dummy", "Test Dummy", "TD", "api", "dummy")],
    accounts: [{ provider: "test_dummy", pool_account_id: "dummy", state: { kind: "ok" }, limit_reached: false,
      windows: [{ kind: "five_hour", used_percent: 97, resets_at: RESET }] }],
  });
  const dummy = rows.at(-1);
  assert.deepEqual([dummy.name, dummy.shortLabel, dummy.usage, dummy.severity, dummy.na],
    ["Test Dummy", "TD", "97% used", "danger", false]);
  const [row] = accountRows({ providers: [descriptor("test_dummy", "Test Dummy", "TD", "api", "dummy")],
    accounts: [{ provider: "test_dummy", state: { kind: "ok" }, limit_reached: false, windows: [] }] });
  assert.equal(row.name, "Test Dummy");
  assert.equal(row.poolAccountId, "dummy");
});

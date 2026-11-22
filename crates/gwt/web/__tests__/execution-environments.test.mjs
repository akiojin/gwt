import test from "node:test";
import assert from "node:assert/strict";
import { accountRows, environmentRows } from "../execution-environments.js";

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
  ] });
  assert.deepEqual(rows.map(row => row.usage), [null, null, null, "90% used"]);
  assert.deepEqual(rows.map(row => row.status.label), [
    "Usage collection is off", "No usage data yet", "Unavailable — token expired", "Last read 10 min ago",
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

test("an environment row joins its definition with the account's usage and the tool's version", () => {
  const snapshot = { accounts: [claude({ limit_reached: true }), codex()] };
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

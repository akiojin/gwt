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

test("an environment needs an installed tool and takes its availability from the account", () => {
  const snapshot = { accounts: [claude({ limit_reached: true }), codex()] };
  const rows = environmentRows(snapshot, agents);
  assert.deepEqual(rows.map(row => [row.id, row.via, row.toolVersion, row.availability, row.members]), [
    ["claude_code", "Claude Code", "2.1.3", "limit", 1],
    ["codex", "Codex", "0.164.0", "available", 1],
  ]);
  assert.equal(rows[1].updateAvailable, true);
  const missing = environmentRows(snapshot, [{ ...agents[0], installed: false }, agents[1]]);
  assert.deepEqual(missing.map(row => row.id), ["codex"]);
  assert.equal(environmentRows(snapshot, null).length, 2, "an unknown install state is not a missing tool");
  assert.equal(environmentRows({ accounts: [codex({ state: { kind: "no_data" }, windows: [] })] }, agents)[0].availability, "unknown");
});

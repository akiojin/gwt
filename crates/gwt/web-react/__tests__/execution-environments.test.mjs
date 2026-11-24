// @vitest-environment linkedom
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { ExecutionEnvironments } from "../src/surfaces/ExecutionEnvironments.jsx";
import { accountRows, environmentRows } from "../../web/execution-environments.js";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

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

// Issue #7741: the environments are the backend's definitions.
const definition = (id, extra = {}) => ({
  id, name: id === "claude" ? "Claude Code 1" : "Codex 1", kind: "set", account_id: id,
  account_name: id === "claude" ? "Claude Code 1" : "Codex 1",
  via_tool: id, via_tool_name: id === "claude" ? "Claude Code" : "Codex",
  runtime: {}, tool_installed: true, projects: [],
  delete_blocked_reason: "Created with the account. It is deleted when the account is deleted.", ...extra,
});
const hermes = (extra = {}) => definition("claude:hermes", {
  name: "Claude Code 1 (via Hermes Agent)", kind: "via", account_id: "claude", account_name: "Claude Code 1",
  via_tool: "hermes", via_tool_name: "Hermes Agent", delete_blocked_reason: undefined,
  runtime_read_only_reason: "Hermes Agent reads these from its own settings file. Change them there.",
  tool_settings: { path: "/home/.hermes/config.yaml", values: [{ label: "Provider", value: "zai" }, { label: "Model", value: "glm-4.6" }] },
  projects: [{ project_root: "/p/alpha", title: "alpha", priority: 2 }], ...extra,
});
const definitionsOf = (...environments) => ({
  accounts: [
    { id: "claude", tool: "claude", tool_name: "Claude Code", name: "Claude Code 1" },
    { id: "codex", tool: "codex", tool_name: "Codex", name: "Codex 1" },
  ],
  environments,
  via_options: [
    { account_id: "claude", via_tool: "hermes", via_tool_name: "Hermes Agent", blocked_reason: 'Hermes Agent is already registered as "Claude Code 1 (via Hermes Agent)".' },
    { account_id: "claude", via_tool: "openclaw", via_tool_name: "OpenClaw" },
    { account_id: "claude", via_tool: "opencode", via_tool_name: "OpenCode", blocked_reason: "OpenCode is not installed. Install it in Tool management." },
    { account_id: "codex", via_tool: "openclaw", via_tool_name: "OpenClaw" },
  ],
});
const DEFAULT_DEFINITIONS = definitionsOf(definition("claude"), definition("codex"));

async function mount(snapshot, {
  view = { name: "environments", key: null }, loaded = true, definitions = DEFAULT_DEFINITIONS, extra = {},
} = {}) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  // linkedom has no focus; record the target the surface chose instead.
  const focused = [];
  window.HTMLElement.prototype.focus = function focus() { focused.push(this); };
  const state = () => ({
    loaded, accounts: accountRows(snapshot), environments: environmentRows(loaded ? definitions : null, snapshot, agents),
    registeredAccounts: definitions.accounts, viaOptions: definitions.via_options, error: null, saving: false, saved: false, ...extra,
  });
  const model = createUiStateStore({ ...state(), view }, { scheduleFrame: callback => { callback(); return () => {}; } });
  const calls = { tools: 0, collection: [], rendered: [], added: [], updated: [], deleted: [], pool: [] };
  const navigate = (name, key = null) => model.update(current => ({ ...current, view: { name, key } }));
  const root = createRoot(document.body);
  await act(async () => root.render(React.createElement(ExecutionEnvironments, {
    store: model,
    renderAccountUsage: (element, provider) => {
      calls.rendered.push(provider);
      element.textContent = `usage card for ${provider}`;
    },
    onNavigate: navigate,
    onOpenToolManagement: () => { calls.tools += 1; },
    onSetUsageCollection: (provider, enabled) => calls.collection.push([provider, enabled]),
    onAdd: input => calls.added.push(input),
    onUpdate: (id, change) => calls.updated.push([id, change]),
    onDelete: id => calls.deleted.push(id),
    onAddPoolAccount: tool => calls.pool.push(["add", tool]),
    onSignInPoolAccount: id => calls.pool.push(["sign-in", id]),
    onMovePoolAccount: (id, direction) => calls.pool.push(["move", id, direction]),
    onUsePoolAccount: id => calls.pool.push(["use", id]),
    onDeletePoolAccount: id => calls.pool.push(["delete", id]),
    onResumeNextPoolAccount: id => calls.pool.push(["resume", id]),
  })));
  const click = async element => { await act(async () => { element.click(); }); };
  const button = label => [...document.querySelectorAll("button")].find(item =>
    item.textContent.trim() === label || item.getAttribute("aria-label") === label);
  return { document, model, calls, click, button, focused };
}

test("account management shows pending sign-in, pool order and affected sessions before deletion", async () => {
  const view = {
    pool_providers: [{ tool: "codex", tool_name: "Codex" }, {
      tool: "agy", tool_name: "Antigravity", blocked_reason: "Antigravity supports one account.",
    }],
    accounts: [
      { id: "codex", tool: "codex", tool_name: "Codex", name: "Codex 1", signed_in: true },
      { id: "codex-2", tool: "codex", tool_name: "Codex", name: "Codex 2", signed_in: false,
        delete_impact: { sessions: ["Issue agent"], agents: ["Reviewer"] } },
    ],
    active_sessions: [{
      id: "pm", title: "Project Manager", is_pm: true, execution_account: { account_id: "codex" },
      account_switches: [{ from: "codex-2", to: "codex", reason: "rate_limit", at: RESET }],
    }],
  };
  const usage = { accounts: [codex()] };
  const { document, click, button, calls, focused } = await mount(usage, {
    view: { name: "accounts", key: null },
    extra: { accounts: accountRows(usage, view), poolProviders: view.pool_providers, activeSessions: view.active_sessions },
  });
  const rows = [...document.querySelectorAll("[data-account]")];
  assert.deepEqual(rows.map(row => row.dataset.account), ["codex", "codex-2"]);
  assert.match(rows[0].textContent, /First choice/);
  assert.match(rows[1].textContent, /Waiting for sign-in/);
  assert.doesNotMatch(rows[1].textContent, /90% used/, "the default account's usage is not given to an added account");
  assert.equal(accountRows({
    accounts: [codex(), codex({ pool_account_id: "codex-2", windows: [{ kind: "weekly", used_percent: 42 }] })],
  }, view)[1].usage, "42% used", "a measured pool account displays its own usage");
  assert.equal(button("Use Codex 2").disabled, true);
  await click(button("Sign in to Codex 2"));
  await click(button("Move Codex 2 up"));
  assert.deepEqual(calls.pool, [["sign-in", "codex-2"], ["move", "codex-2", "up"]]);

  await click(button("Delete Codex 2"));
  const dialog = document.querySelector('[role="dialog"].modal-shell');
  assert.equal(dialog.parentElement.classList.contains("modal-backdrop"), true);
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  assert.deepEqual(["header", "body", "footer"].map(part => Boolean(dialog.querySelector(`.modal-${part}`))), [true, true, true]);
  assert.match(dialog.textContent, /Issue agent/);
  assert.match(dialog.textContent, /Reviewer/);
  assert.equal(focused.at(-1).textContent, "Cancel");
  await click(dialog.querySelector(".modal-footer button:last-child"));
  assert.deepEqual(calls.pool.at(-1), ["delete", "codex-2"]);

  const active = document.querySelector('[data-account-session="pm"]');
  assert.match(active.textContent, /Codex 2 → Codex 1 \(rate limit\)/);
  await click(active.querySelector("button"));
  assert.deepEqual(calls.pool.at(-1), ["resume", "pm"]);

  await click(button("Add an account"));
  const add = document.querySelector('[role="dialog"].modal-shell');
  assert.equal(add.querySelector('option[value="agy"]').hasAttribute("disabled"), true);
  assert.match(add.textContent, /Antigravity supports one account/);
  await act(async () => add.querySelector("form").dispatchEvent(new document.defaultView.Event("submit", { bubbles: true, cancelable: true })));
  assert.deepEqual(calls.pool.at(-1), ["add", "codex"]);
});

// AC-1: every environment's state is visible on the landing view.
test("the landing view lists every environment with its tool, usage and limit state", async () => {
  const { document } = await mount({ accounts: [claude({ limit_reached: true }), codex()] });
  assert.equal(document.querySelector("h1").textContent, "Execution environments");
  const cards = [...document.querySelectorAll("[data-environment]")];
  assert.deepEqual(cards.map(card => card.dataset.environment), ["claude", "codex"]);
  assert.match(cards[0].textContent, /Limit reached/);
  assert.match(cards[0].textContent, /Account Claude Code 1 · Via Claude Code 2\.1\.3 · Not on any project's list/);
  assert.match(cards[0].textContent, /58% used · resets /);
  const time = cards[0].querySelector("time");
  // linkedom keeps React's property spelling; browsers serialize `datetime`.
  assert.equal(time.getAttribute("datetime") ?? time.getAttribute("dateTime"), RESET);
  assert.match(cards[1].textContent, /Available/);
  assert.match(cards[1].textContent, /90% used/);
});

test("empty and loading states are told apart", async () => {
  const empty = await mount({ accounts: [] }, { definitions: definitionsOf() });
  assert.match(empty.document.body.textContent, /No execution environments yet/);
  const loading = await mount(null, { loaded: false });
  assert.match(loading.document.body.textContent, /Loading execution environments…/);
});

test("every account row opens its detail and the detail hosts the usage card", async () => {
  const { document, click, button, calls } = await mount({ accounts: [claude(), codex()] });
  await click(button("Accounts"));
  assert.equal(document.querySelector("h1").textContent, "Accounts");
  const rows = [...document.querySelectorAll("[data-account]")];
  assert.equal(rows.length, 2);
  for (const row of rows) assert.ok(row.querySelector("button"), "each account row has a way to its detail");
  assert.match(rows[0].textContent, /work@example\.test · Max · Up to date/);

  await click(button("Details for the Codex account"));
  assert.equal(document.querySelector("h1").textContent, "Codex account");
  assert.equal(document.querySelector("[data-usage-slot]").textContent, "usage card for codex");
  assert.ok(calls.rendered.includes("codex"));
  assert.equal(document.querySelector("input[type=checkbox]"), null, "Codex usage has no opt-in");

  await click(button("← Accounts"));
  await click(button("Details for the Claude Code account"));
  assert.equal(document.querySelector("[data-usage-slot]").textContent, "usage card for claude_code");
  await click(button("← Accounts"));
  await click(button("← Execution environments"));
  assert.equal(document.querySelector("h1").textContent, "Execution environments");
});

test("Claude usage collection is switched from its account detail", async () => {
  const off = claude({ state: { kind: "disabled" }, windows: [] });
  const { document, click, calls } = await mount({ accounts: [off] }, { view: { name: "account", key: "claude_code" } });
  const checkbox = document.querySelector("input[type=checkbox]");
  assert.equal(checkbox.checked, false);
  await click(checkbox);
  assert.deepEqual(calls.collection, [["claude_code", true]]);
});

test("tool management stays a link to Settings from both list views", async () => {
  const { click, button, calls } = await mount({ accounts: [codex()] });
  await click(button("Tool management"));
  await click(button("Update Codex in Tool management"));
  await click(button("Accounts"));
  await click(button("Tool management"));
  assert.equal(calls.tools, 3);
});

test("moving between views puts focus on the new heading", async () => {
  const { document, click, button, focused } = await mount({ accounts: [codex()] });
  assert.deepEqual(focused, [], "the first render leaves focus alone");
  await click(button("Accounts"));
  assert.equal(focused.at(-1), document.querySelector("h1"));
});

// Issue #7741 AC-1: an account's own environment cannot be deleted here, and
// the row says why; an environment registered through a tool can.
test("a row offers what can be done to it and shows why Delete is unavailable", async () => {
  const { document } = await mount({ accounts: [claude()] }, { definitions: definitionsOf(definition("claude"), hermes()) });
  const [own, through] = [...document.querySelectorAll("[data-environment]")];
  const ownDelete = own.querySelector('button[aria-label="Delete Claude Code 1"]');
  assert.equal(ownDelete.disabled, true);
  assert.match(document.getElementById(ownDelete.getAttribute("aria-describedby")).textContent, /deleted when the account is deleted/);
  assert.ok(own.querySelector('button[aria-label="Edit Claude Code 1"]'));

  assert.equal(through.dataset.kind, "via");
  assert.equal(through.querySelector('button[aria-label="Delete Claude Code 1 (via Hermes Agent)"]').disabled, false);
  assert.ok(through.querySelector('button[aria-label="Details for Claude Code 1 (via Hermes Agent)"]'));
  assert.match(through.textContent, /Used by alpha/);
  assert.match(through.textContent, /usage is not reported here/);
  assert.equal(through.querySelector('button[aria-label^="Open the"]'), null, "usage is not repeated on another tool");
});

test("registering through a tool offers every tool and explains the ones that cannot be chosen", async () => {
  const { document, click, button, calls } = await mount({ accounts: [claude()] });
  await click(button("Add an execution environment"));
  assert.equal(document.querySelector("h1").textContent, "Add an execution environment");
  const [, tool] = [...document.querySelectorAll("select")];
  assert.deepEqual([...tool.querySelectorAll("option")].map(option => [option.value, option.hasAttribute("disabled")]), [
    ["hermes", true], ["openclaw", false], ["opencode", true],
  ]);
  const reasons = [...document.querySelectorAll("[data-blocked-tool]")].map(item => item.textContent);
  assert.equal(reasons.length, 2);
  assert.match(reasons[0], /Hermes Agent — Hermes Agent is already registered/);
  assert.match(reasons[1], /OpenCode — OpenCode is not installed/);
  assert.equal(document.querySelector('input[type="text"]').value, "Claude Code 1 (via OpenClaw)");
  assert.match(document.body.textContent, /Runtime options are not set here/);

  await act(async () => { document.querySelector("form").dispatchEvent(new document.defaultView.Event("submit", { bubbles: true, cancelable: true })); });
  assert.deepEqual(calls.added, [{ account_id: "claude", via_tool: "openclaw", name: "Claude Code 1 (via OpenClaw)" }]);
});

// AC-3: the account's own tool has editable runtime options.
test("runtime options of an account's own environment are edited and saved as they change", async () => {
  const saved = definition("claude", { runtime: { model: "opus", skip_permissions: false, fast_mode: false } });
  const { document, click, calls } = await mount({ accounts: [claude()] }, {
    definitions: definitionsOf(saved), view: { name: "edit", key: "claude" },
  });
  assert.equal(document.querySelector("h1").textContent, "Edit Claude Code 1");
  assert.match(document.body.textContent, /take effect from the next session/);
  const [skip, fast] = [...document.querySelectorAll("input[type=checkbox]")];
  await click(fast);
  await click(skip);
  assert.deepEqual(calls.updated, [
    ["claude", { runtime: { model: "opus", skip_permissions: false, fast_mode: true } }],
    ["claude", { runtime: { model: "opus", skip_permissions: true, fast_mode: false } }],
  ]);
  const model = document.querySelector('input[placeholder="Tool default"]');
  assert.equal(model.value, "opus");
  const remove = [...document.querySelectorAll("button")].find(item => item.textContent === "Delete");
  assert.equal(remove.disabled, true);
});

// AC-3: a tool that reads its own settings is shown read-only.
test("an environment through a tool that reads its own settings shows them read-only", async () => {
  const { document } = await mount({ accounts: [claude()] }, {
    definitions: definitionsOf(definition("claude"), hermes()), view: { name: "edit", key: "claude:hermes" },
  });
  assert.equal(document.querySelector("h1").textContent, "Claude Code 1 (via Hermes Agent)");
  assert.equal(document.querySelectorAll("input[type=checkbox]").length, 0, "no runtime option can be changed here");
  assert.equal(document.querySelector('input[placeholder="Tool default"]'), null);
  const facts = [...document.querySelectorAll('[aria-label="Current settings"] dl > div')].map(row => row.textContent);
  assert.deepEqual(facts, ["Providerzai", "Modelglm-4.6"]);
  assert.match(document.body.textContent, /reads these from its own settings file/);
  assert.match(document.body.textContent, /\/home\/\.hermes\/config\.yaml/);
  assert.match(document.querySelector("[data-used-by]").textContent, /Used by alpha \(priority 2\)/);
});

// AC-4: the confirmation names the projects that use the environment.
test("deleting an environment names the projects that use it before it is deleted", async () => {
  const { document, click, button, calls } = await mount({ accounts: [claude()] }, {
    definitions: definitionsOf(definition("claude"), { ...hermes(), delete_impact: { unknown_sessions: ["Hermes review"] } }),
  });
  await click(button("Delete Claude Code 1 (via Hermes Agent)"));
  assert.equal(document.querySelector("h1").textContent, "Delete this execution environment?");
  const projects = [...document.querySelectorAll("[data-project]")].map(item => item.textContent);
  assert.deepEqual(projects, ["alpha — priority 2"]);
  assert.match(document.body.textContent, /taken off the list of each of these projects/);
  assert.match(document.body.textContent, /The Claude Code 1 account and its other environments stay/);
  assert.match(document.body.textContent, /Impact unknown.*Hermes review/s);
  assert.deepEqual(calls.deleted, []);
  await click(button("Delete"));
  assert.deepEqual(calls.deleted, ["claude:hermes"]);
});

test("a refused request is shown where it was made", async () => {
  const { document } = await mount({ accounts: [claude()] }, {
    view: { name: "edit", key: "claude" }, extra: { error: "This execution environment no longer exists." },
  });
  assert.equal(document.querySelector('[role="alert"]').textContent, "This execution environment no longer exists.");
});

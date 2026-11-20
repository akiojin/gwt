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

async function mount(snapshot, { view = { name: "environments", provider: null }, loaded = true } = {}) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  // linkedom has no focus; record the target the surface chose instead.
  const focused = [];
  window.HTMLElement.prototype.focus = function focus() { focused.push(this); };
  const state = () => ({ loaded, accounts: accountRows(snapshot), environments: environmentRows(snapshot, agents) });
  const model = createUiStateStore({ ...state(), view }, { scheduleFrame: callback => { callback(); return () => {}; } });
  const calls = { tools: 0, collection: [], rendered: [] };
  const navigate = (name, provider = null) => model.update(current => ({ ...current, view: { name, provider } }));
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
  })));
  const click = async element => { await act(async () => { element.click(); }); };
  const button = label => [...document.querySelectorAll("button")].find(item =>
    item.textContent.trim() === label || item.getAttribute("aria-label") === label);
  return { document, model, calls, click, button, focused };
}

// AC-1: every environment's state is visible on the landing view.
test("the landing view lists every environment with its tool, usage and limit state", async () => {
  const { document } = await mount({ accounts: [claude({ limit_reached: true }), codex()] });
  assert.equal(document.querySelector("h1").textContent, "Execution environments");
  const cards = [...document.querySelectorAll("[data-environment]")];
  assert.deepEqual(cards.map(card => card.dataset.environment), ["claude_code", "codex"]);
  assert.match(cards[0].textContent, /Limit reached/);
  assert.match(cards[0].textContent, /Provider Anthropic · Via Claude Code 2\.1\.3 · 1 account/);
  assert.match(cards[0].textContent, /58% used · resets /);
  const time = cards[0].querySelector("time");
  // linkedom keeps React's property spelling; browsers serialize `datetime`.
  assert.equal(time.getAttribute("datetime") ?? time.getAttribute("dateTime"), RESET);
  assert.match(cards[1].textContent, /Available/);
  assert.match(cards[1].textContent, /90% used/);
});

test("empty and loading states are told apart", async () => {
  const empty = await mount({ accounts: [] });
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
  const { document, click, calls } = await mount({ accounts: [off] }, { view: { name: "account", provider: "claude_code" } });
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

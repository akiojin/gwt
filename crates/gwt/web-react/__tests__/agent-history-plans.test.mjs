// Issue #7649 (SPEC #7007 T-031): the project's agent history and planned
// assignments.
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { AgentHistoryPlans } from "../src/surfaces/AgentHistoryPlans.jsx";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const members = [
  { member_id: "pm", name: "Hikari", history: [
    { session_id: "pm-1", role: "pm", provider: "claude", model: null, reasoning: null, environment: "Claude pool",
      environment_model: "claude-opus", issue_number: null, title: null, result: "Running", started_at: "2026-11-20T09:00:00Z", ended_at: null },
  ] },
  { member_id: "m1", name: "Oda", history: [
    { session_id: "s1", role: "implementation", provider: "codex", model: "gpt-6.2", reasoning: "high", environment: "Codex pool",
      environment_model: null, issue_number: 7048, title: "Shared state model", result: "WaitingInput", started_at: "2026-11-20T10:00:00Z", ended_at: null },
    { session_id: "s0", role: "implementation", provider: "codex", model: null, reasoning: null, environment: null,
      environment_model: null, issue_number: 7047, title: null, result: "Stopped", started_at: "2026-11-19T10:00:00Z", ended_at: "2026-11-19T12:00:00Z" },
  ] },
  { member_id: "m2", name: "Himeno", history: [] },
];
const snapshot = { roster_revision: 3, members: [
  { agent_member_id: "pm", issues: [{ issue_number: 1, reason: "queue_order" }] },
  { agent_member_id: "m1", issues: [{ issue_number: 5561, reason: "queue_order" }, { issue_number: 5562, reason: "assigned_by_you" }] },
  { agent_member_id: "m2", issues: [{ issue_number: 7016, reason: "same_spec_continuation" }] },
] };

async function mount(state) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const store = createUiStateStore(state);
  const update = change => act(async () => store.update(change));
  const root = createRoot(document.body);
  await act(async () => root.render(React.createElement(AgentHistoryPlans, { store })));
  const cells = () => [...document.querySelectorAll('table[aria-label="Agent history"] tbody tr')]
    .map(row => [...row.children].map(cell => cell.textContent));
  const plans = () => [...document.querySelectorAll('[aria-label="Planned assignments"] li')].map(item => item.textContent);
  return { document, update, cells, plans };
}

test("AC-1: every agent's sessions are listed newest first with issue, environment, model, reasoning and result", async () => {
  const { document, cells } = await mount({ members, pmMemberIds: ["pm"], rosterRevision: 3, schedule: { snapshot } });
  assert.deepEqual([...document.querySelectorAll('table[aria-label="Agent history"] thead th')].map(cell => cell.textContent),
    ["Agent", "Issue", "Execution environment", "Model", "Reasoning", "Result", "Started (UTC)"]);
  assert.deepEqual(cells(), [
    ["Oda", "#7048 Shared state model", "Codex pool", "gpt-6.2", "high", "Waiting for input", "2026-11-20 10:00"],
    // The Project Manager's row is part of the history, and its model is the
    // environment's own when the session recorded none.
    ["HikariPM", "Project Manager", "Claude pool", "claude-opus", "Tool default", "Running", "2026-11-20 09:00"],
    // Missing facts are named, never left blank.
    ["Oda", "#7047", "codex (not registered as an execution environment)", "Tool default", "Tool default", "Stopped", "2026-11-19 10:00"],
  ]);
  assert.equal(document.querySelector("tbody tr:nth-child(2)").dataset.role, "pm");
});

test("AC-1: a project with no recorded session says so", async () => {
  const { document } = await mount({ members: [{ member_id: "m2", name: "Himeno", history: [] }], pmMemberIds: [], rosterRevision: 3, schedule: { snapshot: null } });
  assert.equal(document.querySelector('table[aria-label="Agent history"]'), null);
  assert.match(document.querySelector(".agent-history").textContent, /No sessions are recorded for this project yet\./);
});

test("AC-2: planned assignments follow the assignment plan's members, order and reasons, without the Project Manager", async () => {
  const { plans, update, document } = await mount({ members, pmMemberIds: ["pm"], rosterRevision: 3, schedule: { snapshot } });
  assert.deepEqual(plans(), [
    "Oda → #5561Queue orderPlanned",
    "Oda → #5562Assigned by youPlanned",
    "Himeno → #7016Same SPEC continuationPlanned",
  ]);
  // A changed plan replaces the list; nothing is kept from the earlier one.
  await update(state => ({ ...state, schedule: { snapshot: { roster_revision: 3, members: [
    { agent_member_id: "m2", issues: [{ issue_number: 5561, reason: "assigned_by_you" }] },
  ] } } }));
  assert.deepEqual(plans(), ["Himeno → #5561Assigned by youPlanned"]);
  await update(state => ({ ...state, schedule: { snapshot: { roster_revision: 3, members: [] } } }));
  assert.deepEqual(plans(), []);
  assert.match(document.querySelector(".agent-plans").textContent, /No planned assignments\./);
});

test("AC-2: an unread, failed or outdated plan is never shown as the current one", async () => {
  const { document, update, plans } = await mount({ members, pmMemberIds: ["pm"], rosterRevision: 3, schedule: { snapshot: null } });
  const status = () => document.querySelector(".agent-plans [role=status]").textContent;
  assert.equal(status(), "Plans unavailable. Refresh plans to try again.");
  await update(state => ({ ...state, schedule: { snapshot, error: "Schedule could not be read." } }));
  assert.equal(status(), "Schedule could not be read.");
  assert.deepEqual(plans(), []);
  await update(state => ({ ...state, rosterRevision: 4, schedule: { snapshot } }));
  assert.equal(status(), "Members changed. Refresh plans to see the current plan.");
  assert.deepEqual(plans(), []);
});

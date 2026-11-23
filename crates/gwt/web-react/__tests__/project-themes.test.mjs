// @vitest-environment linkedom
/* Issue #7643 (SPEC #7007 T-025) — the Themes screen of a project.
 *
 * AC-1: theme list → theme detail → the issues under it.
 * AC-2: "Issues completed today" is the number the backend counted.
 * AC-3: a project with no themes says what a theme is and how one appears.
 * A-23: deleting a theme asks first and says what happens: its GitHub Issue
 *       is closed, the issues under it stay, and it can be reopened.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import "../__tests__/jsx-loader.mjs";
import { ProjectThemes } from "../src/surfaces/ProjectThemes.jsx";
import { createUiStateStore } from "../../web/ui-state-store.js";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

// Rows as the host derives them (crates/gwt/web/project-themes.js).
const issue = (number, completed, extra = {}) => ({
  number, title: `Issue ${number}`, completed, url: `https://github.com/o/r/issues/${number}`, inRepository: true, ...extra,
});
const rows = [
  {
    number: 7, title: "Full refresh", summary: "Rebuild every screen.", url: "https://github.com/o/r/issues/7",
    total: 3, completed: 1, percent: 33, status: "in_progress",
    issues: [issue(8, false), issue(9, true), issue(3, false, { inRepository: false, url: "https://github.com/o/other/issues/3" })],
  },
  {
    number: 20, title: "Crash fix", summary: "", url: "https://github.com/o/r/issues/20",
    total: 1, completed: 1, percent: 100, status: "almost_done", issues: [issue(21, true)],
  },
];

function mount(state = {}) {
  const { document, window } = parseHTML("<!doctype html><html><body><div id=\"root\"></div></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const store = createUiStateStore({
    loaded: true, loading: false, error: null, themes: rows, completedToday: 3, truncated: false,
    selected: null, dialog: null, deleting: false, deleteError: null, notice: null, ...state,
  }, { scheduleFrame: callback => { callback(); return () => {}; } });
  const calls = [];
  const set = change => act(() => store.update(current => ({ ...current, ...change })));
  const actions = {
    reload: () => calls.push(["reload"]),
    openTheme: number => set({ selected: number }),
    closeTheme: () => set({ selected: null }),
    openIssue: number => calls.push(["openIssue", number]),
    openConversation: source => calls.push(["openConversation", source]),
    askDelete: number => set({ dialog: { number } }),
    cancelDelete: () => set({ dialog: null }),
    confirmDelete: () => calls.push(["confirmDelete", store.read().dialog.number]),
  };
  const root = createRoot(document.getElementById("root"));
  act(() => root.render(React.createElement(ProjectThemes, { store, actions })));
  const click = node => act(() => node.click());
  return { document, store, calls, click, set, unmount: () => act(() => root.unmount()) };
}

test("AC-1: a theme row opens the theme, and an issue under it opens that issue", async () => {
  const page = mount();
  const rows = [...page.document.querySelectorAll("[data-themes-list] [data-theme-number]")];
  assert.deepEqual(rows.map(row => row.getAttribute("data-theme-number")), ["7", "20"]);
  assert.match(rows[0].textContent, /Full refresh \(#7\)/);
  assert.match(rows[0].textContent, /3 issues under it · 1 completed/);
  assert.match(rows[0].textContent, /In progress/);
  assert.match(rows[1].textContent, /1 issue under it · 1 completed/);
  assert.match(rows[1].textContent, /Almost done/);
  assert.equal(rows[0].querySelector("[role=progressbar]").getAttribute("aria-valuenow"), "1");

  await page.click(rows[0]);
  const root = page.document.querySelector(".project-themes");
  assert.equal(root.getAttribute("data-themes-view"), "theme");
  assert.match(root.querySelector("h1").textContent, /Full refresh \(#7\)/);
  assert.match(root.textContent, /This theme is a GitHub Issue: #7/);
  assert.equal(root.querySelector("a[href='https://github.com/o/r/issues/7']").textContent, "Open on GitHub");
  assert.match(root.textContent, /Rebuild every screen\./);
  const issues = [...root.querySelectorAll("[data-theme-issues] [data-theme-issue]")];
  assert.deepEqual(issues.map(row => row.getAttribute("data-theme-issue")), ["8", "9", "3"]);
  assert.match(issues[0].textContent, /Open/);
  assert.match(issues[1].textContent, /Completed/);

  await page.click(issues[1]);
  assert.deepEqual(page.calls, [["openIssue", 9]], "a completed issue opens the same way");
  // An issue of another repository is a link to GitHub, not an in-app open.
  assert.equal(issues[2].tagName, "A");
  assert.equal(issues[2].getAttribute("href"), "https://github.com/o/other/issues/3");

  await page.click(root.querySelector("[data-themes-back]"));
  assert.equal(page.document.querySelector(".project-themes").getAttribute("data-themes-view"), "themes");
  page.unmount();
});

test("AC-2: the screen shows the completed-today number it was given, and counts themes by status", () => {
  const page = mount();
  const stat = name => page.document.querySelector(`[data-themes-stat="${name}"]`).textContent;
  assert.equal(stat("completed_today"), "3");
  assert.equal(stat("in_progress"), "1");
  assert.equal(stat("almost_done"), "1");
  assert.doesNotMatch(page.document.body.textContent, /Unknown/);
  page.unmount();
});

test("AC-3: no themes shows what a theme is and how one appears", async () => {
  const page = mount({ themes: [], completedToday: 0 });
  const empty = page.document.querySelector("[data-themes-empty]");
  assert.match(empty.textContent, /No themes yet/);
  assert.match(empty.textContent, /conversation on Home/);
  assert.match(empty.textContent, /GitHub Issue that has sub-issues/);
  assert.equal(page.document.querySelector("[data-themes-list]"), null);
  assert.equal(page.document.querySelector('[data-themes-stat="completed_today"]').textContent, "0");
  await page.click(page.document.querySelector("[data-themes-refresh]"));
  assert.deepEqual(page.calls, [["reload"]]);
  page.unmount();
});

test("before the first answer the screen says it is loading, and a refusal is shown with a retry", async () => {
  const page = mount({ loaded: false, loading: true, themes: [] });
  assert.match(page.document.querySelector(".project-themes__status").textContent, /Loading themes/);
  assert.equal(page.document.querySelector("[data-themes-empty]"), null, "not loaded is not empty");
  assert.equal(page.document.querySelector("[data-themes-stat]"), null);
  await page.set({ loading: false, error: "GitHub sign-in is required." });
  assert.match(page.document.querySelector("[role=alert]").textContent, /sign-in is required/);
  assert.equal(page.document.querySelector("[data-themes-refresh]").textContent, "Try again");
  page.unmount();
});

test("A-23: deleting a theme asks first and says what happens to the issues under it", async () => {
  const page = mount({ selected: 7 });
  assert.equal(page.document.querySelector("[data-theme-delete-dialog]"), null);
  await page.click(page.document.querySelector("[data-theme-delete]"));
  const dialog = page.document.querySelector("[data-theme-delete-dialog] [role=dialog]");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  assert.ok(dialog.querySelector(".modal-header") && dialog.querySelector(".modal-body") && dialog.querySelector(".modal-footer"));
  assert.match(dialog.textContent, /"Full refresh" leaves this list: GitHub Issue #7 is closed on GitHub as not planned\./);
  assert.match(dialog.querySelector("[data-theme-delete-impact]").textContent, /The 3 issues under it stay as they are\./);
  assert.match(dialog.querySelector("[data-theme-delete-undo]").textContent, /Nothing is erased\. Reopen the Issue on GitHub to bring the theme back\./);
  assert.deepEqual(page.calls, [], "nothing is deleted before the confirmation");

  await page.click(dialog.querySelector("[data-theme-delete-cancel]"));
  assert.equal(page.document.querySelector("[data-theme-delete-dialog]"), null);
  assert.deepEqual(page.calls, []);

  await page.click(page.document.querySelector("[data-theme-delete]"));
  await page.click(page.document.querySelector("[data-theme-delete-confirm]"));
  assert.deepEqual(page.calls, [["confirmDelete", 7]]);

  // While GitHub answers, neither button acts; a refusal stays in the dialog.
  await page.set({ deleting: true });
  assert.ok(page.document.querySelector("[data-theme-delete-confirm]").disabled);
  assert.ok(page.document.querySelector("[data-theme-delete-cancel]").disabled);
  await page.set({ deleting: false, deleteError: "Closing an Issue needs write access to the repository." });
  assert.match(page.document.querySelector("[data-theme-delete-dialog] [role=alert]").textContent, /write access/);
  page.unmount();
});

test("a theme with one issue is worded in the singular", async () => {
  const page = mount({ selected: 20, dialog: { number: 20 } });
  assert.match(page.document.querySelector("[data-theme-delete-impact]").textContent, /The 1 issue under it stays as it is\./);
  page.unmount();
});

// Issue #7644 AC-2: a theme recorded from the conversation says so and opens
// the message it came from.
test("a recorded theme shows where it came from and opens that message", () => {
  const source = { sessionId: "pm-session", messageId: "native-turn", recordedAt: "2026-11-23T10:00:00Z" };
  const page = mount({ selected: 7, themes: rows.map(row => row.number === 7 ? { ...row, recordedFrom: source } : row) });
  const origin = page.document.querySelector("[data-theme-origin]");
  assert.match(origin.textContent, /Recorded automatically from the conversation on Home/);
  page.click(page.document.querySelector("[data-theme-conversation]"));
  assert.deepEqual(page.calls, [["openConversation", source]]);
  page.unmount();
  const other = mount({ selected: 20 });
  assert.equal(other.document.querySelector("[data-theme-conversation]"), null);
  assert.equal(other.document.querySelector("[data-theme-origin]"), null);
  other.unmount();
});

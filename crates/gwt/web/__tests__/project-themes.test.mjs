/* Issue #7643 (SPEC #7007 T-025) — the Themes host: what it asks the backend
 * and how an answer, a refusal and a stale reply change the screen.
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import { createProjectThemes } from "../project-themes.js";
import { createUiStateStore } from "../ui-state-store.js";

const view = (themes, completedToday = 0) => ({ themes, completed_today: completedToday, truncated: false });
const theme = (number, total = 2, completed = 1) => ({
  number, title: `Theme ${number}`, summary: "", url: `https://github.com/o/r/issues/${number}`,
  issues_total: total, issues_completed: completed,
  issues: [{ number: number + 1, title: "Child", state: "open", url: `https://github.com/o/r/issues/${number + 1}`, in_repository: true }],
});

// React commits a click's state change on a later task in this DOM.
const settle = async () => {
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
};

function fixture({ sendResult = "sent" } = {}) {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const sent = [];
  const opened = [];
  const repliesModel = createUiStateStore({ reply: null });
  const themes = createProjectThemes({
    document, repliesModel,
    send: message => { sent.push(message); return sendResult; },
    onOpenIssue: number => opened.push(number),
    scheduleFrame: callback => { callback(); return () => {}; },
  });
  document.body.appendChild(themes.element);
  const reply = async event => { repliesModel.update(() => ({ reply: event })); await settle(); };
  const click = async selector => { document.querySelector(selector).click(); await settle(); };
  const text = () => themes.element.textContent;
  return { document, themes, sent, opened, reply, click, text };
}

test("opening the screen asks for the themes and shows the answer", async () => {
  const page = fixture();
  page.themes.reload();
  await settle();
  assert.deepEqual(page.sent, [{ kind: "load_themes", request_id: 1 }]);
  assert.match(page.text(), /Loading themes/);

  await page.reply({ kind: "themes", request_id: 1, view: view([theme(7), theme(20, 3, 3)], 4) });
  assert.match(page.text(), /Theme 7 \(#7\)/);
  assert.equal(page.document.querySelector('[data-themes-stat="completed_today"]').textContent, "4");
  // Progress is GitHub's sub-issue total; a theme whose every issue is
  // completed reads as almost done.
  const done = page.document.querySelector('[data-theme-number="20"]');
  assert.match(done.textContent, /3 issues under it · 3 completed/);
  assert.match(done.textContent, /Almost done/);
  const bar = page.document.querySelector('[data-theme-number="7"] [role=progressbar]');
  assert.deepEqual([bar.getAttribute("aria-valuenow"), bar.getAttribute("aria-valuemax")], ["1", "2"]);
  assert.match(page.document.querySelector('[data-theme-number="7"]').textContent, /In progress/);

  await page.click('[data-theme-number="7"]');
  await page.click('[data-theme-issue="8"]');
  assert.deepEqual(page.opened, [8]);
  page.themes.dispose();
});

test("an answer to an older request does not replace the newer one", async () => {
  const page = fixture();
  page.themes.reload();
  page.themes.reload();
  await page.reply({ kind: "themes", request_id: 1, view: view([theme(7)]) });
  assert.doesNotMatch(page.text(), /Theme 7/, "the first request was superseded");
  await page.reply({ kind: "themes", request_id: 2, view: view([theme(20)]) });
  assert.match(page.text(), /Theme 20/);
  page.themes.dispose();
});

test("a refusal says why, and the themes already shown stay", async () => {
  const page = fixture();
  page.themes.reload();
  await page.reply({ kind: "themes", request_id: 1, view: view([theme(7)]) });
  await page.click("[data-themes-refresh]");
  await page.reply({ kind: "theme_error", request_id: 2, message: "GitHub could not be reached: timeout" });
  assert.match(page.document.querySelector("[role=alert]").textContent, /could not be reached/);
  assert.match(page.text(), /Theme 7/);
  page.themes.dispose();
});

test("a confirmed delete asks the backend, then returns to the list without the theme", async () => {
  const page = fixture();
  page.themes.reload();
  await page.reply({ kind: "themes", request_id: 1, view: view([theme(7), theme(20)]) });
  await page.click('[data-theme-number="7"]');
  await page.click("[data-theme-delete]");
  assert.equal(page.sent.length, 1, "asking is not deleting");
  await page.click("[data-theme-delete-confirm]");
  assert.deepEqual(page.sent[1], { kind: "delete_theme", number: 7, request_id: 2 });
  // A second press while GitHub answers sends nothing more.
  await page.click("[data-theme-delete-confirm]");
  assert.equal(page.sent.length, 2);

  await page.reply({ kind: "themes", request_id: 2, view: view([theme(20)]) });
  assert.equal(page.document.querySelector("[data-theme-delete-dialog]"), null);
  assert.equal(page.document.querySelector(".project-themes").getAttribute("data-themes-view"), "themes");
  assert.match(page.text(), /Theme #7 was deleted\./);
  assert.doesNotMatch(page.text(), /Theme 7 \(#7\)/);
  page.themes.dispose();
});

test("a refused delete keeps the dialog open with the reason", async () => {
  const page = fixture();
  page.themes.reload();
  await page.reply({ kind: "themes", request_id: 1, view: view([theme(7)]) });
  await page.click('[data-theme-number="7"]');
  await page.click("[data-theme-delete]");
  await page.click("[data-theme-delete-confirm]");
  await page.reply({ kind: "theme_error", request_id: 2, message: "Closing an Issue needs write access to the repository." });
  assert.match(page.document.querySelector("[data-theme-delete-dialog] [role=alert]").textContent, /write access/);
  assert.match(page.text(), /Theme 7 \(#7\)/, "the theme is still there");
  page.themes.dispose();
});

test("without a connection the screen says so instead of waiting", async () => {
  const page = fixture({ sendResult: "unavailable" });
  page.themes.reload();
  await settle();
  assert.match(page.document.querySelector("[role=alert]").textContent, /Not connected to gwt/);
  assert.doesNotMatch(page.text(), /Loading themes/);
  page.themes.dispose();
});

test("the Themes styles use Operator tokens only", () => {
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  const rules = css.split("\n").filter(line => line.includes(".project-themes"));
  assert.ok(rules.length > 10, "the Themes rules exist");
  for (const rule of rules) {
    assert.doesNotMatch(rule, /#[0-9a-fA-F]{3,8}\b|rgba?\(/, rule);
  }
});

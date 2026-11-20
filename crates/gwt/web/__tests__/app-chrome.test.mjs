/* Issue #7740 (SPEC #7007 slice F-1) — the header and footer shared by every
 * page. One definition lists the destinations; a destination without a
 * screen stays visible, disabled, and says why (design item 80).
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import { createAppChrome } from "../app-chrome.js";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const componentsCss = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");

// React commits a click's state change on a later task in this DOM.
const click = async node => {
  node.click();
  await new Promise(resolve => setImmediate(resolve));
  await new Promise(resolve => setImmediate(resolve));
};

const DESTINATIONS = ["pm", "notifications", "projects", "agents", "environments", "menu"];

// The project page ships the header skeleton in index.html and mounts the nav.
function projectPage() {
  const { document, window } = parseHTML(indexHtml);
  globalThis.document = document;
  globalThis.window = window;
  const calls = [];
  const handlers = Object.fromEntries(["pm", "notifications", "agents", "environments", "settings"]
    .map(id => [id, () => calls.push(id)]));
  const chrome = createAppChrome({ document, handlers });
  chrome.mountNav(document.getElementById("app-head-nav"));
  chrome.mountFooterPm(document.getElementById("app-foot-pm"));
  const item = id => document.querySelector(`.app-head [data-destination="${id}"]`);
  return { document, chrome, calls, item };
}

test("AC-1: the header lists its destinations in one fixed order and opens each screen", async () => {
  const { document, calls, item } = projectPage();
  assert.equal(document.querySelector(".app-head .app-head__brand").textContent, "gwt");
  assert.deepEqual(
    [...document.querySelectorAll(".app-head__row .app-head__nav > [data-destination]")].map(node => node.dataset.destination),
    DESTINATIONS,
  );
  assert.equal(item("settings"), null, "Settings is not an entry of the tab row (design item 155)");
  for (const id of ["pm", "notifications", "agents", "environments"]) item(id).click();
  assert.deepEqual(calls, ["pm", "notifications", "agents", "environments"]);

  const menu = item("menu");
  assert.equal(menu.getAttribute("aria-haspopup"), "menu");
  assert.equal(menu.getAttribute("aria-expanded"), "false");
  await click(menu);
  assert.equal(menu.getAttribute("aria-expanded"), "true");
  const settings = document.querySelector('body > .app-head-popover[role="menu"] [role="menuitem"][data-destination="settings"]');
  assert.equal(settings.textContent, "Settings");
  await click(settings);
  assert.deepEqual(calls.slice(-1), ["settings"]);
  assert.equal(menu.getAttribute("aria-expanded"), "false", "choosing an item closes the menu");
  assert.equal(document.querySelector(".app-head-popover"), null);
});

test("AC-2: a destination without a screen stays visible, disabled, and explains why", async () => {
  const { document, calls, item } = projectPage();
  const projects = item("projects");
  assert.equal(projects.getAttribute("aria-disabled"), "true");
  assert.equal(projects.disabled, false, "the entry stays focusable so its reason can be reached");
  assert.match(projects.getAttribute("title"), /not available yet/);
  assert.match(projects.getAttribute("aria-label"), /^Projects\. Unavailable: /);
  await click(projects);
  assert.deepEqual(calls, []);
  const popover = document.querySelector('body > .app-head-popover[role="status"]');
  assert.match(popover.textContent, /not available yet/, "the reason is shown where the entry was chosen");
  await click(projects);
  assert.equal(document.querySelector(".app-head-popover"), null, "choosing it again dismisses the reason");
  assert.equal(item("agents").hasAttribute("aria-disabled"), false);
});

test("AC-1/AC-2: pages without a project get the same header and footer and say a project is needed", async () => {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const chrome = createAppChrome({ document });
  const page = chrome.mountPage();
  document.body.append(page.header, page.footer);
  const item = id => document.querySelector(`header.app-head [data-destination="${id}"]`);
  assert.deepEqual(
    [...document.querySelectorAll(".app-head__nav > [data-destination]")].map(node => node.dataset.destination),
    DESTINATIONS,
  );
  for (const id of ["pm", "notifications", "agents", "environments"]) {
    assert.equal(item(id).getAttribute("aria-disabled"), "true", id);
    assert.match(item(id).getAttribute("title"), /Open a project/, id);
  }
  await click(item("menu"));
  const settings = document.querySelector('.app-head-popover [data-destination="settings"]');
  assert.equal(settings.getAttribute("aria-disabled"), "true");
  assert.match(settings.querySelector(".app-head-popover__reason").textContent, /Open a project/);

  // The footer carries the version and the PM name only (design item 155).
  const footer = document.querySelector("footer.app-foot");
  assert.deepEqual([...footer.children].map(node => node.textContent), ["PM: —"]);
  chrome.setVersion("v9.9.9");
  assert.deepEqual([...footer.children].map(node => node.textContent), ["v9.9.9", "PM: —"]);
});

test("the PM entry and the footer name the PM and count the waiting decisions", () => {
  const { document, chrome, item } = projectPage();
  assert.equal(item("pm").querySelector(".app-head__count").hidden, true);
  assert.equal(document.querySelector("#app-foot-pm .app-foot__pm").textContent, "PM: —");
  chrome.setPm({ name: "Hikari", decisions: 3 });
  const pm = item("pm");
  assert.equal(pm.querySelector(".app-head__avatar").textContent, "H");
  assert.equal(pm.querySelector(".app-head__label").textContent, "Hikari");
  assert.equal(pm.querySelector(".app-head__count").textContent, "3");
  assert.equal(pm.querySelector(".app-head__count").hidden, false);
  assert.equal(pm.getAttribute("aria-label"), "Project Manager Hikari: 3 decisions waiting");
  assert.equal(document.querySelector("#app-foot-pm .app-foot__pm").textContent, "PM: Hikari");
  chrome.setPm({ name: "Hikari", decisions: null });
  assert.equal(item("pm").querySelector(".app-head__count").hidden, true);

  chrome.setActiveSurface("agents");
  assert.equal(item("agents").getAttribute("aria-current"), "page");
  chrome.setActiveSurface("issues");
  assert.equal(item("agents").hasAttribute("aria-current"), false);
});

// SPEC #3206 FR-009: the bell mirrors the unread model. It moved from the rail.
test("the Notifications entry is the bell: unread count, error emphasis and drawer state", () => {
  const { chrome, item } = projectPage();
  const bell = item("notifications");
  assert.equal(bell.id, "op-notifications-button");
  assert.equal(bell.getAttribute("aria-controls"), "notification-center");
  assert.equal(bell.getAttribute("aria-expanded"), "false");
  assert.equal(bell.querySelector(".app-head__count").hidden, true, "0 unread hides the count");
  assert.equal(bell.dataset.unread, "0");

  chrome.setNotifications({ count: 3, hasError: false, open: false });
  assert.equal(bell.querySelector(".app-head__count").textContent, "3");
  assert.equal(bell.querySelector(".app-head__count").hidden, false);
  assert.equal(bell.dataset.unread, "3");
  assert.equal(bell.getAttribute("aria-label"), "Notifications (3 unread)");

  chrome.setNotifications({ count: 120, hasError: true, open: true });
  assert.equal(bell.querySelector(".app-head__count").textContent, "99+", "the count is clamped");
  assert.equal(bell.dataset.hasError, "true", "state token emphasis hook");
  assert.equal(bell.getAttribute("aria-label"), "Notifications (120 unread, includes errors)");
  assert.equal(bell.getAttribute("aria-expanded"), "true");
});

test("AC-4: the project page moves the old top-bar entries into the shared header", async () => {
  const { document } = parseHTML(indexHtml);
  const header = document.querySelector("header.app-head.project-bar");
  assert.ok(header, "the project page uses the shared header");
  assert.ok(header.querySelector(".app-head__brand"));
  assert.deepEqual([...header.querySelector(".app-head__row").children].map(node => node.id || node.className),
    ["app-head-nav", "project-actions"], "the nav and the page's own actions share the one row");
  assert.equal(document.getElementById("open-agents-button"), null, "the Ledger entry moved to the header nav");
  assert.equal(document.getElementById("open-environments-button"), null, "the Environments entry moved to the header nav");
  assert.equal(document.querySelector('.op-rail [data-cmd="toggle-notifications"]'), null, "the rail no longer carries a second bell");

  const footer = document.querySelector("footer.app-foot");
  assert.deepEqual([...footer.children].map(node => node.id), ["app-version", "update-check", "app-foot-pm"]);
  assert.equal(header.querySelector("#app-version"), null, "the version lives in the footer only");
});

test("AC-3: the header is a brand row plus one sideways-scrolling row", async () => {
  const row = componentsCss.match(/\.app-head__row\s*\{[^}]*\}/)?.[0] || "";
  assert.match(row, /flex-wrap:\s*nowrap/, "entries never wrap onto another row");
  assert.match(row, /overflow-x:\s*auto/, "entries that do not fit scroll sideways");
  const phone = componentsCss.match(/@media \(max-width: 600px\)\s*\{\s*\.app-head\s*\{[^}]*\}\s*\.app-head__row\s*\{[^}]*\}/)?.[0] || "";
  assert.match(phone, /\.app-head\s*\{[^}]*flex-wrap:\s*wrap/, "the brand takes its own row on a phone");
  assert.match(phone, /\.app-head__row\s*\{[^}]*flex:\s*1 0 100%/, "the nav row spans the full width below it");
});

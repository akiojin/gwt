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
  const handlers = Object.fromEntries(["pm", "notifications", "projects", "agents", "environments", "settings"]
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
  for (const id of ["pm", "notifications", "projects", "agents", "environments"]) item(id).click();
  assert.deepEqual(calls, ["pm", "notifications", "projects", "agents", "environments"]);

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

test("AC-2: a destination the page cannot open stays visible, disabled, and explains why", async () => {
  const { document, window } = parseHTML("<!doctype html><html><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  const calls = [];
  // The page at `/` owns Home and Projects only (Issue #7639).
  const chrome = createAppChrome({ document, handlers: { pm: () => calls.push("pm"), projects: () => calls.push("projects") } });
  const page = chrome.mountPage();
  document.body.append(page.header, page.footer);
  const item = id => document.querySelector(`.app-head [data-destination="${id}"]`);
  const agents = item("agents");
  assert.equal(agents.getAttribute("aria-disabled"), "true");
  assert.equal(agents.disabled, false, "the entry stays focusable so its reason can be reached");
  assert.match(agents.getAttribute("title"), /Open a project/);
  assert.match(agents.getAttribute("aria-label"), /^Agents\. Unavailable: /);
  await click(agents);
  assert.deepEqual(calls, []);
  const popover = document.querySelector('body > .app-head-popover[role="status"]');
  assert.match(popover.textContent, /Open a project/, "the reason is shown where the entry was chosen");
  await click(agents);
  assert.equal(document.querySelector(".app-head-popover"), null, "choosing it again dismisses the reason");
  for (const id of ["pm", "projects"]) {
    assert.equal(item(id).hasAttribute("aria-disabled"), false, id);
    item(id).click();
  }
  assert.deepEqual(calls, ["pm", "projects"]);
  chrome.setActiveSurface("projects");
  assert.equal(item("projects").getAttribute("aria-current"), "page");
  assert.equal(item("pm").hasAttribute("aria-current"), false);
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
  for (const id of ["pm", "notifications", "projects", "agents", "environments"]) {
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

// Issue #7635 AC-1 / AC-2: the PM entry draws the PM agent's avatar through
// the shared avatar part, and falls back to the initial when none is set.
test("the PM entry draws the PM's avatar and follows a preset change", () => {
  const { chrome, item } = projectPage();
  chrome.setPm({ name: "Hikari", decisions: 0, avatar: { kind: "preset", preset_id: "fox" } });
  const avatar = () => item("pm").querySelector(".app-head__avatar");
  assert.equal(avatar().textContent, "🦊");
  chrome.setPm({ name: "Hikari", decisions: 0, avatar: { kind: "preset", preset_id: "owl" } });
  assert.equal(avatar().textContent, "🦉");
  chrome.setPm({ name: "Hikari", decisions: 0, avatar: { kind: "preset", preset_id: "retired" } });
  assert.equal(avatar().textContent, "H", "an unknown preset falls back to the initial");
  chrome.setPm({ name: "Hikari", decisions: 0 });
  assert.equal(avatar().textContent, "H");
});

// Issue #7636 AC-1: an uploaded avatar is drawn as an image on the PM entry.
test("the PM entry draws an uploaded avatar image", () => {
  const { chrome, item } = projectPage();
  const image = "data:image/png;base64,iVBORw0KGgo=";
  chrome.setPm({ name: "Hikari", decisions: 0, avatar: { kind: "upload", image } });
  assert.equal(item("pm").querySelector(".app-head__avatar img.agent-avatar-image")?.getAttribute("src"), image);
  chrome.setPm({ name: "Hikari", decisions: 0, avatar: { kind: "preset", preset_id: "fox" } });
  assert.equal(item("pm").querySelector(".app-head__avatar img"), null);
  assert.equal(item("pm").querySelector(".app-head__avatar").textContent, "🦊");
});

// Issue #7646 AC-6 (A-26, items 44 and 100): the footer and the project
// settings name the PM the ledger designates for this project, not a stale
// window, and follow a reassignment.
test("the project's PM is the ledger's designation, ahead of the running window", async () => {
  const { projectPm } = await import("../app-chrome.js");
  const roster = { project_key: "p1", revision: "1", members: [
    { member_id: "a", name: "Aoi", avatar: { kind: "preset", preset: "owl" } },
    { member_id: "h", name: "Hikari", pm_project_key: "p1" },
    { member_id: "o", name: "Other", pm_project_key: "p2" },
  ] };
  assert.deepEqual(projectPm(roster, { agent_name: "Aoi" }), { name: "Hikari", avatar: undefined });
  const reassigned = { ...roster, members: roster.members.map(member => ({ ...member, pm_project_key: member.member_id === "a" ? "p1" : member.pm_project_key === "p1" ? undefined : member.pm_project_key })) };
  assert.deepEqual(projectPm(reassigned, { agent_name: "Hikari" }), { name: "Aoi", avatar: { kind: "preset", preset: "owl" } });
  // A loaded ledger without a designation falls back to the running PM window.
  assert.deepEqual(projectPm({ project_key: "p1", revision: "1", members: [] }, { agent_name: "Aoi" }), { name: "Aoi", avatar: null });
  assert.deepEqual(projectPm({ project_key: "p1", revision: "1", members: [] }, null), { name: "", avatar: null });
});

// Issue #8012: on reload the PM window's catalog name painted first and the
// ledger's PM replaced it later. Until the ledger loads, the PM stays unnamed.
test("the PM stays unnamed until the ledger loads", async () => {
  const { projectPm } = await import("../app-chrome.js");
  assert.deepEqual(projectPm({ project_key: "p1", revision: null, members: [] }, { agent_name: "Hikari" }), { name: "", avatar: null });
  assert.deepEqual(projectPm(null, { agent_name: "Hikari" }), { name: "", avatar: null });
});

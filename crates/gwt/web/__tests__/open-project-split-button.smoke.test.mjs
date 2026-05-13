// Issue #2684 — DOM smoke test for the top-toolbar Open Project split-button.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "..", "index.html"), "utf8");

function dom() {
  return parseHTML(indexHtml).document;
}

test("split-button group hosts the primary Open Project button and a caret trigger", () => {
  const document = dom();
  const group = document.querySelector(".split-button-group#open-project-group");
  assert.ok(group, "expected .split-button-group#open-project-group");

  const primary = group.querySelector("#open-project-button");
  assert.ok(primary, "expected #open-project-button inside the group");
  assert.match(primary.textContent, /Open Project/);

  const caret = group.querySelector("#open-project-menu-button");
  assert.ok(caret, "expected #open-project-menu-button caret inside the group");
  assert.equal(caret.getAttribute("aria-haspopup"), "menu");
  assert.equal(caret.getAttribute("aria-expanded"), "false");
  assert.equal(caret.getAttribute("aria-controls"), "open-project-menu");
});

test("split-button dropdown menu exposes Open / Clone / Recent items", () => {
  const document = dom();
  const menu = document.getElementById("open-project-menu");
  assert.ok(menu, "expected #open-project-menu");
  assert.equal(menu.getAttribute("role"), "menu");
  assert.equal(menu.getAttribute("aria-hidden"), "true");
  assert.equal(menu.getAttribute("aria-labelledby"), "open-project-menu-button");

  const openItem = menu.querySelector("#open-project-menu-open");
  assert.ok(openItem, "expected Open Project... menu item");
  assert.equal(openItem.getAttribute("role"), "menuitem");
  assert.match(openItem.textContent, /Open Project\.\.\./);

  const cloneItem = menu.querySelector("#open-project-menu-clone");
  assert.ok(cloneItem, "expected Clone from GitHub... menu item");
  assert.equal(cloneItem.getAttribute("role"), "menuitem");
  assert.match(cloneItem.textContent, /Clone from GitHub\.\.\./);

  const recentLabel = menu.querySelector(".split-button-menu-section-label");
  assert.ok(recentLabel, "expected Recent section label");
  assert.match(recentLabel.textContent, /Recent/);

  const recentList = menu.querySelector("#open-project-menu-recent");
  assert.ok(recentList, "expected #open-project-menu-recent container");
});

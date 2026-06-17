// SPEC-2013 Phase 8 — DOM smoke test for the consolidated `Projects ▾` chrome.
// The standalone Open Project split-button (Issue #2684) was retired: Open
// Folder / Clone from GitHub now live inside the Projects switcher, and the
// 0-tab picker uses the same labels.

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

test("the Open Project split-button is fully retired", () => {
  const document = dom();
  for (const id of [
    "open-project-group",
    "open-project-button",
    "open-project-menu",
    "open-project-menu-button",
    "open-project-menu-open",
    "open-project-menu-clone",
    "open-project-menu-recent",
  ]) {
    assert.equal(
      document.getElementById(id),
      null,
      `#${id} must be removed — Projects ▾ owns open/clone now`,
    );
  }
  assert.equal(
    document.querySelector(".split-button-group"),
    null,
    "no split-button-group may remain in the top bar",
  );
});

test("the top bar keeps a single Projects switcher control", () => {
  const document = dom();
  const button = document.getElementById("project-switcher-button");
  assert.ok(button, "expected #project-switcher-button");
  assert.match(button.textContent, /Projects/);
  assert.equal(button.getAttribute("aria-haspopup"), "listbox");
  assert.ok(
    document.getElementById("project-switcher-panel"),
    "expected #project-switcher-panel",
  );
});

test("the 0-tab picker uses the consolidated Open Folder / Clone labels", () => {
  const document = dom();
  const open = document.getElementById("picker-open-project");
  assert.ok(open, "expected #picker-open-project");
  assert.match(open.textContent, /Open Folder/);
  const clone = document.getElementById("picker-clone-project");
  assert.ok(clone, "expected #picker-clone-project");
  assert.match(clone.textContent, /Clone from GitHub/);
});

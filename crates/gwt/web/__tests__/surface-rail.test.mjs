/* Issue #4777 T-1 — the rail picks one of four surfaces.
 *
 * Issues / Agents / Board / Settings are the only places the user can be.
 * Each rail entry carries an icon and a visible label, reports the selected
 * surface with aria-pressed, and asks the host to open the surface on click.
 * Below 860px the rail turns into a horizontal strip across the top.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");

async function importSurfaceRail() {
  return import(pathToFileURL(resolve(here, "../surface-rail.js")).href);
}

test("surfaceForPreset folds every window preset into one of the four surfaces", async () => {
  const { surfaceForPreset } = await importSurfaceRail();
  const cases = {
    issue: "issues",
    issue_monitor: "issues",
    agent_kanban: "issues",
    spec: "issues",
    index: "issues",
    work: "issues",
    workspace: "issues",
    agent: "agents",
    claude: "agents",
    codex: "agents",
    board: "board",
    settings: "settings",
    branches: "settings",
    profile: "settings",
  };
  for (const [preset, surface] of Object.entries(cases)) {
    assert.equal(surfaceForPreset(preset), surface, `preset ${preset}`);
  }
  for (const preset of ["shell", "file_tree", "logs", "pr", "console", "memo", "", null, undefined]) {
    assert.equal(surfaceForPreset(preset), null, `preset ${preset} belongs to no surface`);
  }
});

test("surfaceForWindow puts the PM apart from the agents by its role marker", async () => {
  const { surfaceForWindow } = await importSurfaceRail();
  assert.equal(surfaceForWindow({ preset: "claude", is_pm: true }), "pm");
  assert.equal(surfaceForWindow({ preset: "claude", is_pm: false }), "agents");
  assert.equal(surfaceForWindow({ preset: "board" }), "board");
  assert.equal(surfaceForWindow(null), null);
});

test("the PM sits alone above the rule, then the Surfaces group of Issues / Agents / Board / Settings", () => {
  const { document } = parseHTML(html);
  const children = Array.from(document.querySelector(".op-rail").children).filter(
    (child) => child.tagName !== "#comment",
  );
  const [pmGroup, rule, surfacesGroup] = children;
  assert.deepEqual(
    Array.from(pmGroup.querySelectorAll("[data-surface]")).map((item) => item.id),
    ["op-pm-entry"],
  );
  assert.ok(rule.classList.contains("op-rail__divider"));
  assert.equal(surfacesGroup?.getAttribute("aria-label"), "Surfaces");
  const pm = document.getElementById("op-pm-entry");
  assert.equal(pm.dataset.surface, "pm");
  assert.equal(pm.getAttribute("aria-pressed"), "false");
  assert.ok(pm.querySelector("svg"), "the PM carries an SVG icon");
  assert.equal(pm.querySelector(".op-rail__surface-label")?.textContent.trim(), "PM");
  assert.equal(pm.classList.contains("op-rail__surface"), false, "the PM is not one of the surfaces");
  const items = Array.from(surfacesGroup.querySelectorAll("[data-surface]"));
  assert.deepEqual(
    items.map((item) => item.dataset.surface),
    ["issues", "agents", "board", "settings"],
  );
  assert.deepEqual(
    items.map((item) => item.querySelector(".op-rail__surface-label")?.textContent.trim()),
    ["Issues", "Agents", "Board", "Settings"],
  );
  for (const item of items) {
    assert.equal(item.tagName, "BUTTON");
    assert.equal(item.getAttribute("type"), "button");
    assert.ok(item.querySelector("svg"), `${item.dataset.surface} carries an SVG icon`);
    assert.equal(item.getAttribute("aria-pressed"), "false");
    assert.ok(item.getAttribute("title"), `${item.dataset.surface} explains itself on hover`);
  }
  // The existing ⌘G Issues entry keeps its id so its wiring stays intact.
  assert.equal(items[0].id, "op-workspace-overview-entry");
});

test("applySurfaceSelection presses exactly the selected surface", async () => {
  const { applySurfaceSelection } = await importSurfaceRail();
  const { document } = parseHTML(html);
  const pressed = () =>
    Array.from(document.querySelectorAll("[data-surface]"))
      .filter((item) => item.getAttribute("aria-pressed") === "true")
      .map((item) => item.dataset.surface);

  applySurfaceSelection(document, "board");
  assert.deepEqual(pressed(), ["board"]);
  applySurfaceSelection(document, "agents");
  assert.deepEqual(pressed(), ["agents"]);
  applySurfaceSelection(document, "pm");
  assert.deepEqual(pressed(), ["pm"]);
  applySurfaceSelection(document, null);
  assert.deepEqual(pressed(), []);
});

test("installSurfaceRail asks the host to open the clicked surface", async () => {
  const { installSurfaceRail } = await importSurfaceRail();
  const { document, window } = parseHTML(html);
  const opened = [];
  installSurfaceRail(document, { openSurface: (surface) => opened.push(surface) });
  for (const surface of ["settings", "agents", "board", "issues"]) {
    document
      .querySelector(`[data-surface="${surface}"]`)
      .dispatchEvent(new window.Event("click", { bubbles: true }));
  }
  assert.deepEqual(opened, ["settings", "agents", "board", "issues"]);
  // The PM keeps its own launcher wiring; the surface router never sees it.
  document.getElementById("op-pm-entry").dispatchEvent(new window.Event("click", { bubbles: true }));
  assert.deepEqual(opened, ["settings", "agents", "board", "issues"]);
});

test("the rail is 64px wide and turns into a top strip at 860px and below", () => {
  assert.match(componentsCss, /grid-template-columns:\s*64px 1fr;/);
  const narrow = componentsCss.match(/@media \(max-width: 860px\) \{([\s\S]*?)\n\}/);
  assert.ok(narrow, "expected a max-width: 860px block in components.css");
  assert.match(narrow[1], /grid-template-areas:[\s\S]*"rail"/);
  assert.match(narrow[1], /\.op-rail \{[^}]*flex-direction: row;/);
});

test("the selected surface is marked by a 2px accent edge, not only by color", () => {
  assert.match(
    componentsCss,
    /\.op-rail__surface\[aria-pressed="true"\] \{[^}]*border-left-color: var\(--color-state-active\);/,
  );
});

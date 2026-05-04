import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexPath = resolve(here, "../index.html");
const html = readFileSync(indexPath, "utf8");
const { document } = parseHTML(html);
const operatorShellSource = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("index.html declares Operator chrome scaffold", () => {
  for (const sel of [
    "#op-theme-toggle",
    ".op-sidebar",
    ".op-status-strip",
    "#op-strip-clock",
    "#op-strip-active",
    "#op-strip-idle",
    "#op-strip-blocked",
    "#op-briefing",
    "#op-palette-backdrop",
    "#op-palette-input",
    "#op-palette-list",
    "#op-hotkey-overlay",
    "#op-palette-button",
  ]) {
    assert.ok(document.querySelector(sel), `missing chrome element: ${sel}`);
  }
});

test("project bar exposes a Layers column with three layers", () => {
  const layers = document.querySelectorAll(".op-sidebar__section .op-layer[data-layer]");
  assert.equal(layers.length, 3, "expected three sidebar layers");
  const labels = Array.from(layers).map((el) => el.dataset.layer);
  assert.deepEqual(labels.sort(), ["agents", "git", "hooks"]);
});

test("project bar and command palette expose Start Work outside the Branches surface", () => {
  const projectBarAction = document.querySelector('.op-sidebar .op-layer[data-cmd="start-work"]');
  assert.ok(projectBarAction, "expected Project Bar to expose a Start Work action");
  assert.match(projectBarAction.textContent, /Start Work/);
  assert.match(
    operatorShellSource,
    /id:\s*"start-work"[\s\S]+label:\s*"Start Work"/,
    "expected Command Palette registry to include Start Work",
  );
  assert.match(
    appSource,
    /case\s+"start-work":[\s\S]+kind:\s*"open_start_work"/,
    "expected Start Work command to send the global open_start_work event",
  );
});

test("frontend handles active work projection as status-strip telemetry", () => {
  assert.match(
    appSource,
    /case\s+"active_work_projection":[\s\S]+activeWorkProjection\s*=\s*event\.projection/,
    "expected frontend to store active_work_projection separately from canvas workspace_state",
  );
  assert.match(
    appSource,
    /activeWorkProjection[\s\S]+applyTelemetryCounts/,
    "expected active work projection to feed Operator telemetry",
  );
});

test("Workspace sidebar exposes active work and per-agent overview", () => {
  assert.ok(
    document.querySelector("#op-active-work"),
    "expected Workspace shell to expose an active work overview region",
  );
  assert.ok(
    document.querySelector("#op-active-work-agents"),
    "expected Workspace shell to expose a per-agent list region",
  );
  assert.match(
    appSource,
    /function\s+renderActiveWorkOverview\(\)[\s\S]+activeWorkProjection\.agents/,
    "expected frontend to render per-agent projection data, not only aggregate counters",
  );
  assert.match(
    appSource,
    /op-agent-card[\s\S]+last_board_entry_id/,
    "expected agent cards to preserve board linkage for handoff/debugging",
  );
});

test("Workspace active work overview behaves like a command center", () => {
  assert.match(
    appSource,
    /Add Agent to This Work/,
    "expected Workspace-origin launch copy to make same-work agent addition explicit",
  );
  assert.match(
    appSource,
    /last_board_entry_kind/,
    "expected agent cards to expose the latest coordination milestone kind",
  );
  assert.match(
    appSource,
    /coordination_scope/,
    "expected agent cards to expose owner/topic scope for each agent",
  );
  assert.match(
    appSource,
    /function\s+focusBoardEntry\(/,
    "expected Board actions to deep-link to the referenced coordination entry",
  );
  assert.match(
    appSource,
    /data-board-entry-id/,
    "expected Board timeline entries to be addressable from Workspace links",
  );
});

test("Branches remains a branch browser, not a planning workspace", () => {
  const branchPreset = document.querySelector('.preset-button[data-preset="branches"]');
  assert.ok(branchPreset, "expected Branches preset to remain available");
  assert.match(branchPreset.textContent, /Browse repository branches and launch agents/);
  assert.doesNotMatch(
    `${html}\n${appSource}`,
    /Planning Session|Workspace card/i,
    "Branches should not render Planning Session or Workspace-card concepts",
  );
});

test("hotkey overlay lists ⌘P/⌘B/⌘G/⌘L/⌘?/Esc", () => {
  const overlay = document.getElementById("op-hotkey-overlay");
  assert.ok(overlay, "hotkey overlay missing");
  const text = overlay.textContent.replace(/\s+/g, " ");
  for (const phrase of ["⌘ P", "⌘ B", "⌘ G", "⌘ L", "⌘ K", "⌘ ?", "Esc"]) {
    assert.ok(text.includes(phrase), `expected ${phrase} in hotkey overlay`);
  }
});

test("head loads tokens, typography, components, and Operator modules", () => {
  const css = Array.from(document.querySelectorAll("link[rel=stylesheet]")).map((l) => l.href);
  for (const required of [
    "/styles/tokens.css",
    "/styles/typography.css",
    "/styles/components.css",
    "/assets/xterm/xterm.css",
  ]) {
    assert.ok(css.some((href) => href.endsWith(required)), `expected stylesheet: ${required}`);
  }
  const inlineScripts = Array.from(document.querySelectorAll("head script:not([src])")).map((s) => s.textContent);
  assert.ok(
    inlineScripts.some((src) => src.includes("data-theme") && src.includes("prefers-color-scheme")),
    "expected FOUC-prevention bootstrap script in <head>",
  );
});

test("body markup wires Mission Briefing reveal lines (US-1 AS-1)", () => {
  const lines = document.querySelectorAll(".op-briefing__line");
  assert.ok(lines.length >= 4, `expected >=4 briefing lines, got ${lines.length}`);
  const online = document.querySelector(".op-briefing__online");
  assert.ok(online, "expected OPERATOR ONLINE marker");
  assert.match(online.textContent, /OPERATOR ONLINE/);
});

test("font preload hints exist for Mona/Hubot/JetBrains", () => {
  const preloads = Array.from(document.querySelectorAll("link[rel=preload][as=font]")).map((l) => l.href);
  for (const expected of ["MonaSans.woff2", "HubotSans-Bold.woff2", "JetBrainsMono.woff2"]) {
    assert.ok(preloads.some((h) => h.endsWith(`/assets/fonts/${expected}`)), `missing preload: ${expected}`);
  }
});

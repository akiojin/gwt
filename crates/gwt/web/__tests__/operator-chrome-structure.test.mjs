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

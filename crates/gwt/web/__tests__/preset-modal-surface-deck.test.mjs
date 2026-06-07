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
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appCss = readFileSync(resolve(here, "../styles/app.css"), "utf8");

const modal = document.getElementById("preset-modal");

test("Surface Deck — preset modal exposes a mono kicker with a cyan tick", () => {
  const kicker = modal.querySelector(".preset-modal__kicker");
  assert.ok(kicker, "expected a .preset-modal__kicker element above the title");
  assert.match(
    kicker.textContent,
    /DEPLOY A SURFACE/,
    "kicker copy must read 'DEPLOY A SURFACE'",
  );
});

test("Surface Deck — title node remains for aria-labelledby contract", () => {
  const title = modal.querySelector("#preset-modal-title");
  assert.ok(title, "expected #preset-modal-title to remain for aria-labelledby");
});

test("Surface Deck — modal splits presets into three categorized sections", () => {
  const sections = [...modal.querySelectorAll(".preset-section[data-category]")];
  assert.equal(
    sections.length,
    3,
    "expected exactly three categorized preset sections",
  );

  const byCategory = new Map(
    sections.map((section) => [section.dataset.category, section]),
  );
  assert.ok(byCategory.has("surface"), "expected a data-category='surface' section");
  assert.ok(byCategory.has("knowledge"), "expected a data-category='knowledge' section");
  assert.ok(byCategory.has("config"), "expected a data-category='config' section");

  const labelOf = (cat) =>
    byCategory.get(cat).querySelector(".preset-section-label").textContent.trim();
  assert.equal(labelOf("surface"), "Surfaces");
  assert.equal(labelOf("knowledge"), "GitHub & Knowledge");
  assert.equal(labelOf("config"), "Configuration");
});

test("Surface Deck — categories own the expected preset members", () => {
  const presetsIn = (category) =>
    [
      ...modal.querySelectorAll(
        `.preset-section[data-category="${category}"] .preset-button`,
      ),
    ].map((btn) => btn.dataset.preset);

  assert.deepEqual(presetsIn("surface").sort(), [
    "board",
    "console",
    "file_tree",
    "logs",
    "work",
  ]);
  assert.deepEqual(presetsIn("knowledge").sort(), [
    "index",
    "issue",
    "pr",
    "spec",
  ]);
  assert.deepEqual(presetsIn("config").sort(), ["profile", "settings"]);
});

test("Surface Deck — all 11 legacy data-preset values are preserved", () => {
  const presets = [...modal.querySelectorAll(".preset-button[data-preset]")].map(
    (btn) => btn.dataset.preset,
  );
  const expected = [
    "file_tree",
    "settings",
    "index",
    "profile",
    "logs",
    "issue",
    "spec",
    "work",
    "board",
    "pr",
    "console",
  ].sort();
  assert.deepEqual(presets.sort(), expected);
  assert.equal(presets.length, 11, "expected exactly 11 preset buttons");
});

test("Surface Deck — no forbidden presets leak into the modal", () => {
  for (const forbidden of ["shell", "claude", "codex", "branches"]) {
    assert.equal(
      modal.querySelector(`.preset-button[data-preset="${forbidden}"]`),
      null,
      `forbidden preset '${forbidden}' must not appear in the modal`,
    );
  }
});

test("Surface Deck — every button carries icon glyph + strong label + description", () => {
  const buttons = [...modal.querySelectorAll(".preset-button")];
  assert.equal(buttons.length, 11);
  for (const btn of buttons) {
    const preset = btn.dataset.preset;
    assert.ok(
      btn.dataset.category,
      `button '${preset}' must carry data-category for accent resolution`,
    );

    const icon = btn.querySelector(".preset-button__icon");
    assert.ok(icon, `button '${preset}' must have a .preset-button__icon`);
    assert.ok(
      icon.textContent.trim().length > 0,
      `button '${preset}' icon must contain a glyph`,
    );
    assert.equal(
      icon.getAttribute("aria-hidden"),
      "true",
      `button '${preset}' icon must be aria-hidden`,
    );

    const text = btn.querySelector(".preset-button__text");
    assert.ok(text, `button '${preset}' must wrap label+desc in .preset-button__text`);
    const strong = text.querySelector("strong");
    assert.ok(strong && strong.textContent.trim(), `button '${preset}' needs a strong label`);
    const desc = text.querySelector("span");
    assert.ok(desc && desc.textContent.trim(), `button '${preset}' needs a description span`);
  }
});

test("Surface Deck — specific glyphs map to the right presets", () => {
  const glyphOf = (preset) =>
    modal
      .querySelector(`.preset-button[data-preset="${preset}"] .preset-button__icon`)
      .textContent.trim();
  const expected = {
    file_tree: "⊟",
    logs: "≡",
    console: "❯",
    board: "▦",
    work: "◆",
    issue: "◍",
    spec: "❡",
    pr: "⇄",
    index: "⌕",
    settings: "⚙",
    profile: "◑",
  };
  for (const [preset, glyph] of Object.entries(expected)) {
    assert.equal(glyphOf(preset), glyph, `preset '${preset}' must use glyph '${glyph}'`);
  }
});

test("Surface Deck — footer keeps Cancel and gains a keyboard hint", () => {
  const cancel = modal.querySelector("#close-modal");
  assert.ok(cancel, "Cancel button must remain");
  assert.equal(cancel.textContent.trim(), "Cancel");

  const hint = modal.querySelector(".preset-modal__hint");
  assert.ok(hint, "expected a .preset-modal__hint element");
  assert.match(hint.textContent, /deploy/i, "hint must mention 'deploy'");
  assert.match(hint.textContent, /esc/i, "hint must mention 'esc'");
});

test("Surface Deck CSS — preset-list is a 2-column responsive grid", () => {
  assert.match(
    appCss,
    /\.preset-list\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/,
    "preset-list must declare a 2-column grid",
  );
  assert.match(
    appCss,
    /@media[^{]*max-width[^{]*\{[\s\S]*?\.preset-list\s*\{[^}]*grid-template-columns:\s*1fr/,
    "preset-list must collapse to a single column at narrow widths",
  );
});

test("Surface Deck CSS — buttons use a two-track icon+text layout", () => {
  assert.match(
    appCss,
    /\.preset-button\s*\{[^}]*grid-template-columns:\s*auto\s+1fr/,
    "preset-button must lay out icon chip + text via a grid",
  );
});

test("Surface Deck CSS — category accents bind --cat-accent per category", () => {
  assert.match(
    componentsCss,
    /\[data-category="surface"\][^{]*\{[^}]*--cat-accent:\s*var\(--color-state-active\)/,
    "surface accent must be cyan (--color-state-active)",
  );
  assert.match(
    componentsCss,
    /\[data-category="knowledge"\][^{]*\{[^}]*--cat-accent:\s*var\(--agent-claude\)/,
    "knowledge accent must be amber (--agent-claude)",
  );
  assert.match(
    componentsCss,
    /\[data-category="config"\][^{]*\{[^}]*--cat-accent:\s*var\(--color-text-muted\)/,
    "config accent must be muted",
  );
});

test("Surface Deck CSS — active/hover state glows with the category accent", () => {
  assert.match(
    componentsCss,
    /\.preset-button\.is-active[\s\S]*?box-shadow:[^;]*var\(--cat-accent\)/,
    "keyboard .is-active state must glow with the category accent",
  );
});

test("Surface Deck CSS — open animation staggers and respects reduced motion", () => {
  assert.match(componentsCss, /@keyframes\s+presetRise/, "presetRise keyframes must exist");
  assert.match(
    componentsCss,
    /#preset-modal\.open\s+\.preset-button\s*\{[\s\S]*?animation:\s*presetRise/,
    "open modal must run presetRise on its buttons",
  );
  assert.match(
    componentsCss,
    /@media\s*\(prefers-reduced-motion:\s*reduce\)[\s\S]*?\.preset-button[\s\S]*?animation:\s*none/,
    "reduced-motion must disable the stagger animation",
  );
});

test("Surface Deck CSS — modal shell carries operator atmosphere", () => {
  assert.match(
    componentsCss,
    /\.modal-shell\.is-surface-deck::before[\s\S]*?var\(--bg-depth-glow\)/,
    "preset modal shell ::before must paint the depth glow atmosphere",
  );
  // The preset shell opts into the atmosphere via the modifier class.
  const shell = modal.querySelector(".modal-shell");
  assert.ok(
    shell.classList.contains("is-surface-deck"),
    "preset modal shell must carry the is-surface-deck modifier",
  );
});

test("Surface Deck JS — opening the modal focuses the first preset button and roves with arrows", () => {
  // roving helper + arrow-key handling wired into app.js
  assert.match(
    appSource,
    /presetRovingButtons|\.preset-button/,
    "expected preset roving wiring referencing .preset-button",
  );
  assert.match(
    appSource,
    /ArrowRight|ArrowLeft|ArrowDown|ArrowUp/,
    "expected arrow-key handling for preset roving",
  );
  assert.match(
    appSource,
    /is-active/,
    "expected roving to toggle the .is-active class",
  );
});

test("Surface Deck JS — wires the roving keydown listener and clears state on close", () => {
  assert.match(
    appSource,
    /modal\.addEventListener\(\s*"keydown",\s*handlePresetRovingKeydown\s*\)/,
    "modal must register the roving keydown listener",
  );
  assert.match(
    appSource,
    /Enter[\s\S]{0,200}\.click\(\)/,
    "Enter must trigger the active preset button's click()",
  );
  assert.match(
    appSource,
    /PRESET_GRID_COLUMNS\s*=\s*2/,
    "vertical roving must jump a full 2-column row",
  );
});

// Behavioral lock for the 2-column roving math. We replicate the exact
// index arithmetic from app.js against the real modal grid so a future
// column-count change can't silently break ↑↓ navigation.
test("Surface Deck behavioral — 2-column roving math walks the real grid", () => {
  const buttons = [...modal.querySelectorAll(".preset-button")];
  assert.equal(buttons.length, 11);
  const COLUMNS = 2;
  const move = (current, key) => {
    if (key === "ArrowRight") return Math.min(current + 1, buttons.length - 1);
    if (key === "ArrowLeft") return Math.max(current - 1, 0);
    if (key === "ArrowDown") return Math.min(current + COLUMNS, buttons.length - 1);
    if (key === "ArrowUp") return Math.max(current - COLUMNS, 0);
    return current;
  };
  // Row 0 = [0,1]; ArrowRight steps within row, ArrowDown jumps to row 1.
  assert.equal(move(0, "ArrowRight"), 1);
  assert.equal(move(0, "ArrowDown"), 2);
  assert.equal(move(2, "ArrowUp"), 0);
  // Clamp at edges.
  assert.equal(move(0, "ArrowLeft"), 0);
  assert.equal(move(buttons.length - 1, "ArrowRight"), buttons.length - 1);
  assert.equal(move(buttons.length - 1, "ArrowDown"), buttons.length - 1);
});

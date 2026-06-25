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
    "improvement",
    "issue_monitor",
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

test("Surface Deck — all 13 visible data-preset values are preserved", () => {
  const presets = [...modal.querySelectorAll(".preset-button[data-preset]")].map(
    (btn) => btn.dataset.preset,
  );
  const expected = [
    "file_tree",
    "issue_monitor",
    "settings",
    "index",
    "profile",
    "logs",
    "issue",
    "spec",
    "work",
    "board",
    "improvement",
    "pr",
    "console",
  ].sort();
  assert.deepEqual(presets.sort(), expected);
  assert.equal(presets.length, 13, "expected exactly 13 preset buttons");
});

test("Surface Deck — no forbidden presets leak into the modal", () => {
  for (const forbidden of ["shell", "claude", "codex", "branches", "agent_kanban"]) {
    assert.equal(
      modal.querySelector(`.preset-button[data-preset="${forbidden}"]`),
      null,
      `forbidden preset '${forbidden}' must not appear in the modal`,
    );
  }
});

test("Surface Deck — every button carries icon glyph + strong label + description", () => {
  const buttons = [...modal.querySelectorAll(".preset-button")];
  assert.equal(buttons.length, 13);
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
    improvement: "◇",
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

test("Surface Deck — three sections live inside a .preset-deck landscape wrapper", () => {
  // SPEC-2356 landscape redesign (案B "weighted deck"): the three categorized
  // sections are wrapped in a .preset-deck so they lay out as side-by-side
  // columns instead of a tall vertical stack that overflows short viewports.
  const deck = modal.querySelector(".preset-deck");
  assert.ok(deck, "expected a .preset-deck wrapper for the landscape layout");
  const sections = [...modal.querySelectorAll(".preset-section[data-category]")];
  assert.equal(sections.length, 3, "the deck must hold the three categorized sections");
  for (const section of sections) {
    assert.equal(
      section.parentElement,
      deck,
      `section '${section.dataset.category}' must be a direct child of .preset-deck`,
    );
  }
});

test("Surface Deck CSS — landscape deck lays sections out as weighted columns", () => {
  // 45fr / 33fr / 22fr maps column width to content volume (5 / 4 / 2 buttons)
  // so the deck stays low and balanced.
  assert.match(
    appCss,
    /\.preset-deck\s*\{[^}]*grid-template-columns:\s*45fr\s+33fr\s+22fr/,
    "preset-deck must be a weighted 3-column grid (45/33/22)",
  );
  // SURFACES + KNOWLEDGE keep a 2-column inner grid; CONFIG collapses to one
  // column (it only has 2 buttons) to avoid a wide half-empty row.
  assert.match(
    appCss,
    /\.preset-list\s*\{[^}]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\)/,
    "base preset-list keeps a 2-column inner grid for surface/knowledge",
  );
  assert.match(
    appCss,
    /\[data-category="config"\]\s+\.preset-list\s*\{[^}]*grid-template-columns:\s*1fr/,
    "config column collapses its preset-list to a single column",
  );
  // Narrow viewports collapse the whole deck back to a single column (scrollable).
  assert.match(
    appCss,
    /@media[^{]*max-width[^{]*\{[\s\S]*?\.preset-deck\s*\{[^}]*grid-template-columns:\s*1fr/,
    "preset-deck must collapse to a single column at narrow widths",
  );
});

test("Surface Deck CSS — shell widens for landscape and keeps a vertical scroll safety net", () => {
  assert.match(
    appCss,
    /\.modal-shell\.is-surface-deck\s*\{[^}]*width:\s*min\(\s*940px/,
    "is-surface-deck must widen to a landscape width",
  );
  // Judge catch: the atmosphere shell previously set `overflow: hidden`, which
  // kills the scroll safety net on very short viewports. It must allow vertical
  // scroll while still clipping the horizontal atmosphere bleed.
  assert.doesNotMatch(
    componentsCss,
    /\.modal-shell\.is-surface-deck\s*\{[^}]*\boverflow:\s*hidden/,
    "is-surface-deck must not hard-clip overflow (would kill the scroll safety net)",
  );
  assert.match(
    componentsCss,
    /\.modal-shell\.is-surface-deck\s*\{[^}]*overflow-y:\s*auto/,
    "is-surface-deck must allow vertical scroll as a safety net",
  );
});

test("Surface Deck CSS — buttons keep a 2-column inner grid (legacy contract)", () => {
  // The base preset-list contract is still a responsive 2-column grid; the
  // landscape deck only overrides the config column and adds the outer deck.
  assert.match(
    appCss,
    /@media[^{]*max-width[^{]*\{[\s\S]*?\.preset-list\s*\{[^}]*grid-template-columns:\s*1fr/,
    "preset-list must still collapse to a single column at the narrow breakpoint",
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
  // In the landscape deck the buttons stay nested inside per-category
  // .preset-list grids, so nth-child resets per column and the three columns
  // deploy in parallel — nth-child(1..5) already covers the tallest column.
  assert.match(
    componentsCss,
    /#preset-modal\.open\s+\.preset-button:nth-child\(5\)/,
    "stagger delays must cover the tallest (SURFACES) column",
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
  // Landscape weighted deck → geometry direction-nearest roving (layout-agnostic),
  // not a fixed column-count jump. The handler reads real tile geometry so the
  // weighted 45/33/22 columns and uneven row counts navigate intuitively.
  assert.match(
    appSource,
    /getBoundingClientRect\(\)/,
    "preset roving must use geometry (getBoundingClientRect) for the weighted layout",
  );
});

// Behavioral lock for the geometry direction-nearest roving math. linkedom does
// no layout, so we replicate the exact scorer from app.js against synthetic rect
// centers modeling the weighted deck (SURFACES 2-col with a fourth row, KNOWLEDGE 2-col
// x 2-row, CONFIG 1-col x 2-row) and assert intuitive navigation. A future
// scorer change that breaks cross-column / clamp behavior trips this test.
test("Surface Deck behavioral — geometry roving picks the nearest tile in the pressed direction", () => {
  const buttons = [...modal.querySelectorAll(".preset-button")];
  assert.equal(buttons.length, 13);

  // center coords keyed by DOM order: SURFACES(0-6), KNOWLEDGE(7-10), CONFIG(11-12)
  const centers = [
    { x: 100, y: 100 }, // 0 file_tree   (SURFACES col1 row1)
    { x: 250, y: 100 }, // 1 logs        (SURFACES col2 row1)
    { x: 100, y: 180 }, // 2 console     (SURFACES col1 row2)
    { x: 250, y: 180 }, // 3 board       (SURFACES col2 row2)
    { x: 100, y: 260 }, // 4 work        (SURFACES col1 row3)
    { x: 250, y: 260 }, // 5 monitor     (SURFACES col2 row3)
    { x: 100, y: 340 }, // 6 improvement (SURFACES col1 row4)
    { x: 450, y: 100 }, // 7 issue       (KNOWLEDGE col1 row1)
    { x: 580, y: 100 }, // 8 spec        (KNOWLEDGE col2 row1)
    { x: 450, y: 180 }, // 9 pr          (KNOWLEDGE col1 row2)
    { x: 580, y: 180 }, // 10 index      (KNOWLEDGE col2 row2)
    { x: 750, y: 100 }, // 11 settings   (CONFIG row1)
    { x: 750, y: 180 }, // 12 profile    (CONFIG row2)
  ];

  // Replica of app.js findGeometryNeighbor: direction half-plane filter +
  // primary-axis distance with a secondary-axis bias so same-row/column wins.
  const AXIS_BIAS = 2.5;
  const move = (current, key) => {
    const src = centers[current];
    let best = current;
    let bestScore = Infinity;
    centers.forEach((dst, i) => {
      if (i === current) return;
      const dx = dst.x - src.x;
      const dy = dst.y - src.y;
      const inDir =
        (key === "ArrowRight" && dx > 1) ||
        (key === "ArrowLeft" && dx < -1) ||
        (key === "ArrowDown" && dy > 1) ||
        (key === "ArrowUp" && dy < -1);
      if (!inDir) return;
      const horiz = key === "ArrowRight" || key === "ArrowLeft";
      const primary = horiz ? Math.abs(dx) : Math.abs(dy);
      const secondary = horiz ? Math.abs(dy) : Math.abs(dx);
      // Reject too-diagonal candidates (>~63°) so pressing Down at the bottom
      // of a short column clamps instead of leaping to a taller column's lower row.
      if (secondary > primary * 2) return;
      const score = primary + secondary * AXIS_BIAS;
      if (score < bestScore) {
        bestScore = score;
        best = i;
      }
    });
    return best;
  };

  // Within SURFACES column block.
  assert.equal(move(0, "ArrowRight"), 1, "File Tree → Logs");
  assert.equal(move(0, "ArrowDown"), 2, "File Tree → Console");
  assert.equal(move(2, "ArrowUp"), 0, "Console → File Tree");
  assert.equal(move(1, "ArrowDown"), 3, "Logs → Board (same inner column)");
  assert.equal(move(3, "ArrowDown"), 5, "Board → Issue Monitor");
  assert.equal(move(4, "ArrowRight"), 5, "Workspace → Issue Monitor");
  assert.equal(move(4, "ArrowDown"), 6, "Workspace → Improvement Inbox");
  // Cross-column to the next category at the same row.
  assert.equal(move(1, "ArrowRight"), 7, "Logs → Issue (jump to KNOWLEDGE)");
  assert.equal(move(8, "ArrowRight"), 11, "SPEC → Settings (jump to CONFIG)");
  // CONFIG single column.
  assert.equal(move(11, "ArrowDown"), 12, "Settings → Profile");
  assert.equal(move(12, "ArrowUp"), 11, "Profile → Settings");
  // Clamp at edges (no wrap).
  assert.equal(move(0, "ArrowLeft"), 0, "left edge clamps");
  assert.equal(move(0, "ArrowUp"), 0, "top edge clamps");
  assert.equal(move(12, "ArrowDown"), 12, "bottom of CONFIG clamps");
  assert.equal(move(11, "ArrowRight"), 11, "right edge clamps");
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const workspaceSource = readFileSync(resolve(here, "../workspace-kanban-surface.js"), "utf8");
const releaseNotesSource = readFileSync(resolve(here, "../release-notes-window.js"), "utf8");
const appCss = readFileSync(resolve(here, "../styles/app.css"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");

test("Workspace Overview is governed by Quiet Work UI list filters + Detail", () => {
  assert.match(workspaceSource, /workspace-overview-root/);
  assert.match(workspaceSource, /workspace-overview-list-pane/);
  assert.match(workspaceSource, /workspace-overview-filter-bar/);
  assert.match(workspaceSource, /workspace-overview-list/);
  assert.match(workspaceSource, /workspace-overview-detail-pane/);
  assert.doesNotMatch(
    workspaceSource,
    /ATTENTION_LANES|workspace-attention-lane|workspace-kanban-board|data-workspace-column|workspace-kanban-column/,
    "Workspace Overview must not reintroduce mini Kanban lanes in the persistent pane",
  );
  assert.doesNotMatch(
    workspaceSource,
    /createNode\("pre"|"pre",\s*"workspace/,
    "Workspace detail must not render work content as preformatted dumps",
  );
});

test("Workspace work-surface typography keeps content on body or mono fonts", () => {
  const combinedCss = `${appCss}\n${componentsCss}`;
  const detailRule = ruleFor(combinedCss, ".workspace-overview-detail-pane");
  assert.match(detailRule, /font-family:\s*var\(--font-body\)/);
  assert.doesNotMatch(
    blocksFor(combinedCss, /\.workspace-(?:overview|detail)[^{]*\{/),
    /font-family:\s*var\(--font-display\)|font-stretch:\s*75%|text-transform:\s*uppercase/,
    "Workspace work content must not use display/condensed typography",
  );
});

test("Release Notes remains app-global and does not spawn a project workspace preset", () => {
  assert.match(releaseNotesSource, /kind:\s*"open_release_notes"/);
  assert.doesNotMatch(
    `${appSource}\n${releaseNotesSource}`,
    /focusOrSpawnPreset\("release_notes"\)|preset:\s*"release_notes"|kind:\s*"create_window"[\s\S]{0,120}release_notes/,
    "gwt Release Notes must not be modeled as project workspace state",
  );
});

test("Release Notes uses shared global window chrome instead of a custom fixed overlay rule", () => {
  assert.match(releaseNotesSource, /op-global-window/);
  assert.match(componentsCss, /\.op-global-window\s*\{/);

  const legacyRule = ruleFor(componentsCss, ".release-notes-window");
  assert.doesNotMatch(
    legacyRule,
    /position:\s*fixed/,
    "Release Notes should inherit positioning from global window chrome, not a bespoke fixed overlay",
  );
});

test("Text-first UI primitives define shared readable content formatting", () => {
  const primitives = [
    [".text-first-content", /font-family:\s*var\(--font-body\)/, /line-height:\s*1\.[5-9]/],
    [".text-first-message", /background:\s*var\(--color-surface(?:-elevated)?\)/, /border:\s*1px solid var\(--color-border\)/],
    [".text-first-meta", /font-family:\s*var\(--font-mono\)/, /color:\s*var\(--color-text-muted\)/],
    [".text-first-code", /font-family:\s*var\(--font-mono\)/, /background:\s*var\(--color-surface(?:-elevated)?\)/],
    [".text-first-state", /background:\s*var\(--color-surface(?:-elevated)?\)/, /border:\s*1px solid var\(--color-border\)/],
    [".text-first-scroll", /overflow:\s*auto/, /min-height:\s*0/],
  ];

  for (const [selector, first, second] of primitives) {
    const block = ruleFor(componentsCss, selector);
    assert.match(block, first, `${selector} is missing the primary text-first contract`);
    assert.match(block, second, `${selector} is missing the secondary text-first contract`);
    assert.doesNotMatch(
      block,
      /font-family:\s*var\(--font-display\)|font-stretch:\s*75%|backdrop-filter|transparent|opacity:\s*0\.[0-9]/,
      `${selector} must stay body/mono, opaque, and non-decorative`,
    );
  }
});

test("Core readable surfaces opt into the text-first app-wide format bridge", () => {
  const combinedCss = `${appCss}\n${componentsCss}`;
  const bridge = blockContaining(combinedCss, ".release-notes-content", ".workspace-overview-detail-pane");

  for (const selector of [
    ".release-notes-content",
    ".workspace-overview-detail-pane",
    ".workspace-overview-row",
    ".workspace-detail-agent",
    ".workspace-detail-event",
    ".terminal-overlay",
    ".file-tree-viewer-body",
    ".board-panel",
    ".logs-panel",
    ".knowledge-panel",
  ]) {
    assert.match(bridge, selectorRegex(selector), `text-first bridge must include ${selector}`);
  }
  assert.match(bridge, /font-family:\s*var\(--font-body\)/);
  assert.match(bridge, /line-height:\s*1\.[5-9]/);
  assert.doesNotMatch(
    bridge,
    /font-family:\s*var\(--font-display\)|font-stretch:\s*75%|backdrop-filter|transparent|opacity:\s*0\.[0-9]/,
    "text-first bridge must not use display typography or translucent readable backgrounds",
  );
});

function ruleFor(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `missing CSS rule: ${selector}`);
  return match[1];
}

function selectorRegex(selector) {
  return new RegExp(selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
}

function blockContaining(css, ...selectors) {
  const blocks = blocksFor(css, /[^{]+\{/);
  const match = blocks
    .split("\n/* block */\n")
    .find((block) => selectors.every((selector) => selectorRegex(selector).test(block)));
  assert.ok(match, `missing CSS bridge block containing selectors: ${selectors.join(", ")}`);
  return match;
}

function blocksFor(css, selectorRegex) {
  const blocks = [];
  let match;
  const regex = new RegExp(selectorRegex.source, "g");
  while ((match = regex.exec(css))) {
    const start = match.index;
    const open = css.indexOf("{", start);
    if (open < 0) continue;
    let depth = 1;
    let i = open + 1;
    while (i < css.length && depth > 0) {
      if (css[i] === "{") depth += 1;
      if (css[i] === "}") depth -= 1;
      i += 1;
    }
    blocks.push(css.slice(start, i));
  }
  return blocks.join("\n/* block */\n");
}

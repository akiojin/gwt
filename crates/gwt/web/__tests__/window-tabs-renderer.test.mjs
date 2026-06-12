import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { parseHTML } from "linkedom";

import { renderWindowTabs } from "../window-tabs-renderer.js";

const here = dirname(fileURLToPath(import.meta.url));

const DEFAULT_TABS = [
  { id: "win-1", title: "Agent One", preset: "codex" },
  { id: "win-2", title: "Agent Two", preset: "claude" },
];

function setupDom() {
  const { document } = parseHTML('<div class="window-tab-strip"></div>');
  return {
    strip: document.querySelector(".window-tab-strip"),
  };
}

function render(deps = {}) {
  const { strip = setupDom().strip, sends = [], drags = [], closes = [] } = deps;
  renderWindowTabs({
    strip,
    tabs: deps.tabs ?? DEFAULT_TABS,
    activeWindowId: deps.activeWindowId ?? "win-1",
    tooltipForWindow:
      deps.tooltipForWindow ?? ((tab) => `Tooltip for ${tab.title}`),
    send: deps.send ?? ((payload) => sends.push(payload)),
    requestClose: deps.requestClose ?? ((id) => closes.push(id)),
    onTabDragStart:
      deps.onTabDragStart ?? ((event, id) => drags.push(["start", id])),
    onTabDrag: deps.onTabDrag ?? ((event, id) => drags.push(["drag", id])),
    onTabDragEnd:
      deps.onTabDragEnd ?? ((event, id) => drags.push(["end", id])),
  });
  return { strip, sends, drags, closes };
}

test("renderWindowTabs preserves tab DOM across active-tab changes", () => {
  const { strip } = setupDom();

  render({ strip });
  const firstItem = strip.querySelector('[data-window-tab-id="win-1"]');
  const secondItem = strip.querySelector('[data-window-tab-id="win-2"]');
  const firstButton = firstItem.querySelector(".window-tab");
  const secondButton = secondItem.querySelector(".window-tab");
  const firstClose = firstItem.querySelector(".window-tab-close");

  render({
    strip,
    activeWindowId: "win-2",
    tabs: [
      { id: "win-1", title: "Agent One Updated", preset: "codex" },
      { id: "win-2", title: "Agent Two", preset: "claude" },
    ],
  });

  assert.equal(
    strip.querySelector('[data-window-tab-id="win-1"]'),
    firstItem,
    "window tab item must be updated in place instead of recreated",
  );
  assert.equal(
    strip.querySelector('[data-window-tab-id="win-2"]'),
    secondItem,
    "active tab changes must not recreate sibling tab items",
  );
  assert.equal(firstItem.querySelector(".window-tab"), firstButton);
  assert.equal(secondItem.querySelector(".window-tab"), secondButton);
  assert.equal(firstItem.querySelector(".window-tab-close"), firstClose);
  assert.equal(firstButton.textContent, "Agent One Updated");
  assert.equal(firstButton.title, "Tooltip for Agent One Updated");
  assert.equal(
    firstButton.getAttribute("aria-label"),
    "Activate Agent One Updated",
  );
  assert.equal(
    firstClose.getAttribute("aria-label"),
    "Close Agent One Updated",
  );
  assert.equal(firstButton.getAttribute("aria-current"), null);
  assert.equal(secondButton.getAttribute("aria-current"), "page");
  assert.equal(firstButton.classList.contains("active"), false);
  assert.equal(secondButton.classList.contains("active"), true);
  assert.equal(secondButton.draggable, true);
});

test("renderWindowTabs keeps one activate/close binding per stable tab node", () => {
  const { strip } = setupDom();
  const sends = [];
  const closes = [];

  render({ strip, sends, closes });
  render({ strip, sends, closes, activeWindowId: "win-2" });
  render({ strip, sends, closes, activeWindowId: "win-1" });

  const firstButton = strip.querySelector(
    '[data-window-tab-id="win-1"] .window-tab',
  );
  firstButton.dispatchEvent(
    new firstButton.ownerDocument.defaultView.Event("click", {
      bubbles: true,
    }),
  );

  const secondClose = strip.querySelector(
    '[data-window-tab-id="win-2"] .window-tab-close',
  );
  secondClose.dispatchEvent(
    new secondClose.ownerDocument.defaultView.Event("click", {
      bubbles: true,
    }),
  );

  assert.deepEqual(sends, [{ kind: "activate_window_tab", id: "win-1" }]);
  assert.deepEqual(
    closes,
    ["win-2"],
    "tab × must route through the close-confirm callback exactly once",
  );
});

test("tab close button never sends close_window directly (SPEC-3038 US-3)", () => {
  // The Close Guard owns the actual close: the renderer only reports intent.
  const { strip } = setupDom();
  const sends = [];
  const closes = [];

  render({ strip, sends, closes });
  const close = strip.querySelector(
    '[data-window-tab-id="win-1"] .window-tab-close',
  );
  close.dispatchEvent(
    new close.ownerDocument.defaultView.Event("click", { bubbles: true }),
  );

  assert.deepEqual(closes, ["win-1"]);
  assert.equal(
    sends.some((message) => message?.kind === "close_window"),
    false,
    "no direct close_window message may leave the renderer",
  );
});

test("renderWindowTabs reorders and removes tabs without rebuilding kept nodes", () => {
  const { strip } = setupDom();

  render({ strip });
  const firstItem = strip.querySelector('[data-window-tab-id="win-1"]');

  render({
    strip,
    activeWindowId: "win-3",
    tabs: [
      { id: "win-3", title: "Agent Three", preset: "agent" },
      { id: "win-1", title: "Agent One", preset: "codex" },
    ],
  });

  assert.equal(strip.children.length, 2);
  assert.equal(strip.children[0].dataset.windowTabId, "win-3");
  assert.equal(strip.children[1], firstItem);
  assert.equal(strip.querySelector('[data-window-tab-id="win-2"]'), null);
});

test("renderWindowTabs projects agent telemetry onto tabs (SPEC-3038 US-2)", () => {
  const { strip } = setupDom();

  render({
    strip,
    tabs: [
      {
        id: "win-1",
        title: "Agent One",
        preset: "codex",
        agent_state: "active",
        agent_color: "cyan",
      },
      {
        id: "win-2",
        title: "Agent Two",
        preset: "claude",
        agent_state: "blocked",
        agent_color: "yellow",
      },
    ],
  });

  const first = strip.querySelector('[data-window-tab-id="win-1"] .window-tab');
  assert.equal(first.dataset.agentState, "active");
  assert.equal(
    first.dataset.agentColor,
    "cyan",
    "tab must carry the agent color so --current-agent resolves on the rim",
  );
  const dot = first.querySelector(".window-tab-state");
  assert.ok(dot, "expected a state dot inside the tab");
  assert.equal(dot.hidden, false);
  assert.equal(
    dot.getAttribute("aria-hidden"),
    "true",
    "state dot is decorative — state is announced via the window chrome",
  );
  assert.equal(
    first.querySelector(".window-tab-label")?.textContent,
    "Agent One",
    "title must live in a label span so the dot survives re-renders",
  );

  const second = strip.querySelector('[data-window-tab-id="win-2"] .window-tab');
  assert.equal(second.dataset.agentState, "blocked");
  assert.equal(second.dataset.agentColor, "yellow");
});

test("renderWindowTabs omits telemetry attributes for non-agent tabs (SPEC-3038 US-2)", () => {
  const { strip } = setupDom();

  render({
    strip,
    tabs: [{ id: "win-1", title: "Board", preset: "board" }],
  });

  const tab = strip.querySelector('[data-window-tab-id="win-1"] .window-tab');
  assert.equal(tab.dataset.agentState, undefined);
  assert.equal(tab.dataset.agentColor, undefined);
  const dot = tab.querySelector(".window-tab-state");
  assert.ok(dot, "the dot node stays keyed in place even without telemetry");
  assert.equal(dot.hidden, true, "non-agent tabs must hide the state dot");
});

test("renderWindowTabs updates telemetry in place on state change (SPEC-3038 AS-2.2)", () => {
  const { strip } = setupDom();

  render({
    strip,
    tabs: [
      {
        id: "win-1",
        title: "Agent One",
        preset: "codex",
        agent_state: "active",
        agent_color: "cyan",
      },
    ],
  });
  const button = strip.querySelector('[data-window-tab-id="win-1"] .window-tab');

  render({
    strip,
    tabs: [
      {
        id: "win-1",
        title: "Agent One",
        preset: "codex",
        agent_state: "blocked",
        agent_color: "cyan",
      },
    ],
  });

  assert.equal(
    strip.querySelector('[data-window-tab-id="win-1"] .window-tab'),
    button,
    "state changes must update the keyed tab node in place",
  );
  assert.equal(button.dataset.agentState, "blocked");

  render({
    strip,
    tabs: [{ id: "win-1", title: "Agent One", preset: "codex" }],
  });
  assert.equal(
    button.dataset.agentState,
    undefined,
    "clearing telemetry must remove the attribute",
  );
});

test("app.css styles telemetry tabs: agent rim, state dot, pulse, hover-only close (SPEC-3038)", () => {
  const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");

  // The window-level [data-agent-state] glow rules must not leak onto tabs.
  assert.match(
    css,
    /\.window-tab\[data-agent-state\]\s*\{[^}]*box-shadow:\s*none/,
    "tabs must neutralize the window rim glow",
  );
  assert.match(
    css,
    /\.window-tab\[data-agent-state\]\s*\{[^}]*border-left[^}]*--current-agent/,
    "tabs must carry an agent-colored left rim",
  );
  assert.match(css, /\.window-tab-state\s*\{/, "expected a state dot rule");
  assert.match(
    css,
    /\.window-tab\[data-agent-state="active"\]\s+\.window-tab-state\s*\{[^}]*var\(--color-state-active\)/,
    "active dot uses the active state token",
  );
  assert.match(
    css,
    /\.window-tab\[data-agent-state="blocked"\]\s+\.window-tab-state\s*\{[^}]*var\(--color-state-blocked\)/,
    "blocked dot uses the blocked state token",
  );
  assert.match(
    css,
    /@keyframes\s+window-tab-state-pulse/,
    "expected a named tab state pulse animation",
  );
  assert.match(
    css,
    /prefers-reduced-motion[\s\S]{0,2000}?\.window-tab\[data-agent-state="active"\]\s+\.window-tab-state[\s\S]{0,200}?animation:\s*none/,
    "reduced-motion must stop tab dot pulses",
  );
  // AS-2.4: the close button only appears on hover / keyboard focus.
  assert.match(
    css,
    /\.window-tab-close\s*\{[^}]*opacity:\s*0/,
    "tab close button must be hidden until hover",
  );
  assert.match(
    css,
    /\.window-tab-item:hover\s+\.window-tab-close|\.window-tab-item:focus-within\s+\.window-tab-close/,
    "hover / focus-within must reveal the tab close button",
  );
  assert.match(
    css,
    /@media\s*\(hover:\s*none\)[\s\S]{0,400}?\.window-tab-close\s*\{[^}]*opacity:\s*1/,
    "touch environments keep the close button visible",
  );
});

test("renderWindowTabs drag callbacks read the current tab id after rerender", () => {
  const { strip } = setupDom();
  const drags = [];

  render({ strip, drags });
  render({
    strip,
    drags,
    tabs: [
      { id: "win-1", title: "Agent One Updated", preset: "codex" },
      { id: "win-2", title: "Agent Two", preset: "claude" },
    ],
  });

  const firstButton = strip.querySelector(
    '[data-window-tab-id="win-1"] .window-tab',
  );
  const event = (type) =>
    new firstButton.ownerDocument.defaultView.Event(type, { bubbles: true });
  firstButton.dispatchEvent(event("dragstart"));
  firstButton.dispatchEvent(event("drag"));
  firstButton.dispatchEvent(event("dragend"));

  assert.deepEqual(drags, [
    ["start", "win-1"],
    ["drag", "win-1"],
    ["end", "win-1"],
  ]);
});

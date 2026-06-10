import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import { renderWindowTabs } from "../window-tabs-renderer.js";

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
  const { strip = setupDom().strip, sends = [], drags = [] } = deps;
  renderWindowTabs({
    strip,
    tabs: deps.tabs ?? DEFAULT_TABS,
    activeWindowId: deps.activeWindowId ?? "win-1",
    tooltipForWindow:
      deps.tooltipForWindow ?? ((tab) => `Tooltip for ${tab.title}`),
    send: deps.send ?? ((payload) => sends.push(payload)),
    onTabDragStart:
      deps.onTabDragStart ?? ((event, id) => drags.push(["start", id])),
    onTabDrag: deps.onTabDrag ?? ((event, id) => drags.push(["drag", id])),
    onTabDragEnd:
      deps.onTabDragEnd ?? ((event, id) => drags.push(["end", id])),
  });
  return { strip, sends, drags };
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

  render({ strip, sends });
  render({ strip, sends, activeWindowId: "win-2" });
  render({ strip, sends, activeWindowId: "win-1" });

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

  assert.deepEqual(sends, [
    { kind: "activate_window_tab", id: "win-1" },
    { kind: "close_window", id: "win-2" },
  ]);
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

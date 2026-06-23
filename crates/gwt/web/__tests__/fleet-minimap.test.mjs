// SPEC-2008 camera-focus / FR-094 — Fleet Minimap.
//
// The minimap is a permanent overlay docked in the canvas corner. It keeps a
// one-glance view of every canvas window in WORLD position (one `.cell` per
// window) and overlays the current camera viewport as a `.camera` frame.
// Clicking a cell flies the camera to that window via the `frameWindow`
// callback. These linkedom tests exercise the real renderCells /
// updateCameraFrame DOM output and the cell-click → frameWindow contract.

import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

import { createFleetMinimap } from "../fleet-minimap.js";

// createFleetMinimap reaches for the global `document` (document.createElement)
// for the camera-frame and cell nodes, so each test installs a fresh linkedom
// document as the global before building a container.
function setupDom() {
  const { document, window } = parseHTML(
    "<!doctype html><html><body></body></html>",
  );
  globalThis.document = document;
  globalThis.window = window;
  const container = document.createElement("div");
  // linkedom has no layout engine; clientWidth/clientHeight are 0 by default.
  // Pin them so buildTransform has a non-degenerate inner rect to map into.
  Object.defineProperty(container, "clientWidth", { value: 200, configurable: true });
  Object.defineProperty(container, "clientHeight", { value: 120, configurable: true });
  document.body.appendChild(container);
  return { document, container };
}

function windowAt(id, x, y, width = 100, height = 80, extra = {}) {
  return { id, geometry: { x, y, width, height }, ...extra };
}

function makeMinimap(container, windows, overrides = {}) {
  const calls = { framed: [] };
  const minimap = createFleetMinimap({
    container,
    getWindows: () => windows,
    getVisibleBounds: () => ({ x: 0, y: 0, width: 200, height: 120 }),
    getFocusedId: () => null,
    frameWindow: (id) => calls.framed.push(id),
    windowDisplayTitle: (w) => `Title ${w.id}`,
    cellAgentColor: (w) => w.agent_color || "",
    cellTelemetryState: (w) => w.telemetry || "",
    ...overrides,
  });
  return { minimap, calls };
}

test("createFleetMinimap returns a no-op surface when there is no container", () => {
  const minimap = createFleetMinimap({ container: null });
  assert.equal(typeof minimap.renderCells, "function");
  assert.equal(typeof minimap.update, "function");
  assert.equal(typeof minimap.updateCameraFrame, "function");
  assert.equal(typeof minimap.setZoom, "function");
  // Must not throw when driven without a DOM target.
  minimap.renderCells();
  minimap.update();
  minimap.setZoom(1.25);
});

test("renderCells creates one cell per window keyed by window id", () => {
  const { container } = setupDom();
  const windows = [
    windowAt("w-1", 0, 0),
    windowAt("w-2", 300, 0),
    windowAt("w-3", 0, 200),
  ];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();

  const cells = container.querySelectorAll(".fleet-minimap__cell");
  assert.equal(cells.length, 3, "one cell per window");
  const ids = [...cells].map((cell) => cell.dataset.windowId).sort();
  assert.deepEqual(ids, ["w-1", "w-2", "w-3"]);
  // Each cell exposes its window title for a11y + tooltip.
  for (const cell of cells) {
    assert.equal(cell.getAttribute("aria-label"), `Title ${cell.dataset.windowId}`);
    assert.equal(cell.title, `Title ${cell.dataset.windowId}`);
  }
});

test("FR-045 (anshin): cellTooltip overrides the cell title/aria-label", () => {
  // The minimap cell tooltip surfaces the agent's live activity (title ·
  // detail) when an app-provided cellTooltip factory is wired, instead of
  // the plain display title.
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0), windowAt("w-2", 300, 0)];
  const { minimap } = makeMinimap(container, windows, {
    cellTooltip: (w) => `Title ${w.id} · doing thing ${w.id}`,
  });

  minimap.renderCells();

  const cell = container.querySelector('[data-window-id="w-1"]');
  assert.equal(cell.title, "Title w-1 · doing thing w-1");
  assert.equal(cell.getAttribute("aria-label"), "Title w-1 · doing thing w-1");
});

test("FR-045 (anshin): cellTooltip falls back to windowDisplayTitle when absent", () => {
  // Back-compat: callers that do not pass cellTooltip keep the display title
  // on both the tooltip and aria-label.
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0)];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();

  const cell = container.querySelector('[data-window-id="w-1"]');
  assert.equal(cell.title, "Title w-1");
  assert.equal(cell.getAttribute("aria-label"), "Title w-1");
});

test("data-empty flips with the window count", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0)];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();
  assert.equal(container.dataset.empty, "false", "non-empty fleet marks data-empty=false");

  windows.length = 0;
  minimap.renderCells();
  assert.equal(container.dataset.empty, "true", "emptying the fleet marks data-empty=true");
  assert.equal(
    container.querySelectorAll(".fleet-minimap__cell").length,
    0,
    "stale cells are removed when the window set empties",
  );
});

test("clicking a cell flies the camera to that window via frameWindow", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0), windowAt("w-2", 300, 0)];
  const { minimap, calls } = makeMinimap(container, windows);

  minimap.renderCells();
  const target = container.querySelector('.fleet-minimap__cell[data-window-id="w-2"]');
  assert.ok(target, "the w-2 cell must exist");
  target.dispatchEvent(new container.ownerDocument.defaultView.Event("click"));

  assert.deepEqual(calls.framed, ["w-2"], "cell click frames its own window id");
});

test("cells carry agent color and telemetry datasets only when present", () => {
  const { container } = setupDom();
  const windows = [
    windowAt("w-agent", 0, 0, 100, 80, { agent_color: "violet", telemetry: "active" }),
    windowAt("w-plain", 300, 0),
  ];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();

  const agentCell = container.querySelector('[data-window-id="w-agent"]');
  assert.equal(agentCell.dataset.agentColor, "violet");
  assert.equal(agentCell.dataset.telemetry, "active");

  const plainCell = container.querySelector('[data-window-id="w-plain"]');
  assert.equal(plainCell.dataset.agentColor, undefined, "non-agent cell omits agent color");
  assert.equal(plainCell.dataset.telemetry, undefined, "non-agent cell omits telemetry");
});

test("FR-039 (安心): a needs_input telemetry surfaces as its own minimap dataset", () => {
  // The minimap dot color/pulse keys off data-telemetry, so the loud
  // needs_input state must round-trip onto the cell rather than collapse.
  const { container } = setupDom();
  const windows = [windowAt("w-wait", 0, 0, 100, 80, { agent_color: "amber", telemetry: "needs_input" })];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();

  const cell = container.querySelector('[data-window-id="w-wait"]');
  assert.equal(cell.dataset.telemetry, "needs_input");
});

test("the focused window's cell gets the is-focused class", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0), windowAt("w-2", 300, 0)];
  const { minimap } = makeMinimap(container, windows, {
    getFocusedId: () => "w-2",
  });

  minimap.renderCells();

  const focused = container.querySelector('[data-window-id="w-2"]');
  const other = container.querySelector('[data-window-id="w-1"]');
  assert.ok(focused.classList.contains("is-focused"), "focused cell is marked");
  assert.ok(!other.classList.contains("is-focused"), "unfocused cell is not marked");
});

test("centered-radar: the camera frame stays centred and panning translates the world layer", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0), windowAt("w-2", 400, 300)];
  // A mutable visible-bounds so we can repan and re-drive the same instance.
  let visibleBounds = { x: 0, y: 0, width: 200, height: 120 };
  const { minimap } = makeMinimap(container, windows, {
    getVisibleBounds: () => visibleBounds,
  });

  minimap.renderCells();

  // Exactly one world layer (cells live inside it) + one camera frame.
  const worldLayers = container.querySelectorAll(".fleet-minimap__world");
  assert.equal(worldLayers.length, 1, "exactly one world layer");
  assert.equal(
    container.querySelectorAll(".fleet-minimap__world .fleet-minimap__cell").length,
    2,
    "cells live inside the world layer (so panning translates them as one)",
  );
  const frames = container.querySelectorAll(".fleet-minimap__camera");
  assert.equal(frames.length, 1, "exactly one camera frame element");
  const frame = frames[0];
  assert.equal(frame.hidden, false, "camera frame is visible while windows exist");

  // The camera frame is CENTERED: left + width/2 ≈ container centre x (100),
  // top + height/2 ≈ centre y (60). It represents "your current view" fixed at
  // the middle of the radar.
  const center = { x: 100, y: 60 };
  const frameCenterX = parseFloat(frame.style.left) + parseFloat(frame.style.width) / 2;
  const frameCenterY = parseFloat(frame.style.top) + parseFloat(frame.style.height) / 2;
  assert.ok(Math.abs(frameCenterX - center.x) < 0.5, "camera frame is horizontally centred");
  assert.ok(Math.abs(frameCenterY - center.y) < 0.5, "camera frame is vertically centred");

  const worldLayer = worldLayers[0];
  const transformBefore = worldLayer.style.transform;
  const frameBefore = { left: frame.style.left, top: frame.style.top };

  // Panning the camera (same zoom, new x/y) translates the WORLD LAYER (windows
  // move) while the camera frame stays centred (viewport fixed).
  visibleBounds = { x: 200, y: 150, width: 200, height: 120 };
  minimap.update();
  assert.equal(
    container.querySelectorAll(".fleet-minimap__camera").length,
    1,
    "camera frame is repositioned in place, never duplicated",
  );
  assert.notEqual(
    worldLayer.style.transform,
    transformBefore,
    "the world layer translates when the camera pans (windows move)",
  );
  assert.deepEqual(
    { left: frame.style.left, top: frame.style.top },
    frameBefore,
    "the camera frame stays put (centred) on a same-zoom pan",
  );
});

test("centered-radar: setZoom rescales the cells (radar zoom)", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0, 100, 80), windowAt("w-2", 400, 0, 100, 80)];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();
  const cell = container.querySelector('[data-window-id="w-2"]');
  const widthBefore = parseFloat(cell.style.width);
  const leftBefore = parseFloat(cell.style.left);

  // Zooming the radar in enlarges the cells and pushes their world-positions out.
  minimap.setZoom(2);
  const widthAfter = parseFloat(cell.style.width);
  const leftAfter = parseFloat(cell.style.left);
  assert.ok(widthAfter > widthBefore, "zooming in enlarges the cell");
  assert.ok(Math.abs(leftAfter) > Math.abs(leftBefore), "zooming in spreads world positions out");
});

test("the camera frame hides when there are no framable windows", () => {
  const { container } = setupDom();
  const windows = [];
  const { minimap } = makeMinimap(container, windows);

  minimap.renderCells();

  const frame = container.querySelector(".fleet-minimap__camera");
  assert.ok(frame, "the camera frame element persists across empties");
  assert.equal(frame.hidden, true, "camera frame hides when nothing is framable");
});

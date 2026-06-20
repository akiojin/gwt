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
  assert.equal(typeof minimap.updateCameraFrame, "function");
  // Must not throw when driven without a DOM target.
  minimap.renderCells();
  minimap.updateCameraFrame();
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

test("updateCameraFrame positions a single visible .camera element in place", () => {
  const { container } = setupDom();
  const windows = [windowAt("w-1", 0, 0), windowAt("w-2", 400, 300)];
  // A mutable visible-bounds so we can repan and re-drive the same instance.
  let visibleBounds = { x: 0, y: 0, width: 200, height: 120 };
  const { minimap } = makeMinimap(container, windows, {
    getVisibleBounds: () => visibleBounds,
  });

  minimap.renderCells();

  const frames = container.querySelectorAll(".fleet-minimap__camera");
  assert.equal(frames.length, 1, "exactly one camera frame element");
  const frame = frames[0];
  assert.equal(frame.hidden, false, "camera frame is visible while windows exist");
  // Positioned with inline px geometry.
  for (const prop of ["left", "top", "width", "height"]) {
    assert.match(
      frame.style[prop] || "",
      /px$/,
      `camera frame ${prop} must be set in px`,
    );
  }
  const before = { left: frame.style.left, top: frame.style.top };

  // Panning the camera (new visible bounds) and re-driving updateCameraFrame
  // repositions the SAME node in place — never appends a second frame.
  visibleBounds = { x: 200, y: 150, width: 200, height: 120 };
  minimap.updateCameraFrame();
  assert.equal(
    container.querySelectorAll(".fleet-minimap__camera").length,
    1,
    "camera frame is repositioned in place, never duplicated",
  );
  assert.notDeepEqual(
    { left: frame.style.left, top: frame.style.top },
    before,
    "camera frame moves after the camera pans",
  );
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

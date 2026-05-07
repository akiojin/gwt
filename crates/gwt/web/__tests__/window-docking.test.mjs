import { test } from "node:test";
import assert from "node:assert/strict";

import {
  detachGeometryFromClientPoint,
  TITLEBAR_DOCK_HIT_HEIGHT,
  findTitlebarDockTarget,
  resolveDragReleasePoint,
} from "../window-docking.js";

test("titlebar docking only targets another visible window titlebar", () => {
  const windows = [
    {
      id: "source",
      geometry: { x: 40, y: 40, width: 200, height: 120 },
      z_index: 3,
    },
    {
      id: "target-low",
      geometry: { x: 100, y: 100, width: 220, height: 140 },
      z_index: 1,
    },
    {
      id: "target-high",
      geometry: { x: 110, y: 100, width: 220, height: 140 },
      z_index: 9,
    },
    {
      id: "hidden-tab",
      tab_group_id: "group-a",
      tab_group_active: false,
      geometry: { x: 115, y: 100, width: 220, height: 140 },
      z_index: 20,
    },
  ];

  assert.equal(
    findTitlebarDockTarget(windows, { x: 130, y: 112 }, "source"),
    "target-high",
    "topmost visible target titlebar should win",
  );

  assert.equal(
    findTitlebarDockTarget(windows, { x: 130, y: 100 + TITLEBAR_DOCK_HIT_HEIGHT + 1 }, "source"),
    null,
    "dropping on the body must not dock",
  );

  assert.equal(
    findTitlebarDockTarget(windows, { x: 42, y: 42 }, "source"),
    null,
    "the dragged source window must not dock into itself",
  );
});

test("titlebar docking is only an entry point for ungrouped source windows", () => {
  const windows = [
    {
      id: "source",
      tab_group_id: "group-a",
      tab_group_active: true,
      geometry: { x: 40, y: 40, width: 200, height: 120 },
      z_index: 5,
    },
    {
      id: "target",
      geometry: { x: 100, y: 100, width: 220, height: 140 },
      z_index: 9,
    },
  ];

  assert.equal(
    findTitlebarDockTarget(windows, { x: 130, y: 112 }, "source"),
    null,
    "dragging a grouped window titlebar must keep moving the group, not redock the tab",
  );
});

test("titlebar docking ignores invalid pointer points", () => {
  const windows = [
    {
      id: "source",
      geometry: { x: 40, y: 40, width: 200, height: 120 },
      z_index: 1,
    },
    {
      id: "target",
      geometry: { x: 100, y: 100, width: 220, height: 140 },
      z_index: 9,
    },
  ];

  assert.equal(findTitlebarDockTarget(windows, null, "source"), null);
  assert.equal(findTitlebarDockTarget(windows, { x: Number.NaN, y: 112 }, "source"), null);
  assert.equal(findTitlebarDockTarget(windows, { x: 130, y: Number.POSITIVE_INFINITY }, "source"), null);
});

test("tab detach geometry uses the latest valid drag point in canvas world coordinates", () => {
  const canvasRect = { left: 20, top: 10, right: 820, bottom: 610, width: 800, height: 600 };
  const viewport = { x: -240, y: -120, zoom: 2 };
  const draggedWindow = {
    id: "agent-1",
    geometry: { x: 40, y: 40, width: 720, height: 420 },
  };

  const releasePoint = resolveDragReleasePoint(
    { clientX: 0, clientY: 0 },
    { x: 220, y: 110 },
    canvasRect,
  );
  const geometry = detachGeometryFromClientPoint(
    releasePoint,
    draggedWindow,
    canvasRect,
    viewport,
  );

  assert.deepEqual(releasePoint, { x: 220, y: 110 });
  assert.deepEqual(geometry, {
    x: 188,
    y: 91,
    width: 720,
    height: 420,
  });
});

test("tab detach release point accepts a valid dragend coordinate before fallback", () => {
  const canvasRect = { left: 20, top: 10, right: 820, bottom: 610, width: 800, height: 600 };

  assert.deepEqual(
    resolveDragReleasePoint(
      { clientX: 320, clientY: 210 },
      { x: 220, y: 110 },
      canvasRect,
    ),
    { x: 320, y: 210 },
  );
});

test("tab detach release point treats dragend zero coordinates as missing when a fallback exists", () => {
  const canvasRect = { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600 };

  assert.deepEqual(
    resolveDragReleasePoint(
      { clientX: 0, clientY: 0 },
      { x: 220, y: 110 },
      canvasRect,
    ),
    { x: 220, y: 110 },
  );
});

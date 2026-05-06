import { test } from "node:test";
import assert from "node:assert/strict";

import {
  TITLEBAR_DOCK_HIT_HEIGHT,
  findTitlebarDockTarget,
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

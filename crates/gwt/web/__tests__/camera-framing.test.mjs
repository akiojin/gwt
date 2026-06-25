import assert from "node:assert/strict";
import test from "node:test";

import {
  computeCameraFrameArea,
  computeViewportForWorldRect,
  expandWorldRectForLayoutSize,
} from "../camera-framing.js";

function projectedRect(rect, viewport) {
  return {
    left: rect.x * viewport.zoom + viewport.x,
    top: rect.y * viewport.zoom + viewport.y,
    right: (rect.x + rect.width) * viewport.zoom + viewport.x,
    bottom: (rect.y + rect.height) * viewport.zoom + viewport.y,
  };
}

test("camera framing keeps a mobile surface inside the command rail safe area", () => {
  const frameArea = computeCameraFrameArea({
    canvasRect: { left: 0, top: 44, right: 390, bottom: 816, width: 390, height: 772 },
    obstructionRects: [
      { left: 0, top: 44, right: 56, bottom: 816, width: 56, height: 772 },
    ],
  });
  assert.deepEqual(frameArea, {
    left: 56,
    top: 0,
    width: 334,
    height: 772,
  });

  const worldRect = { x: 0, y: 0, width: 420, height: 260 };
  const viewport = computeViewportForWorldRect(worldRect, {
    frameArea,
    fillRatio: 0.92,
    minZoom: 0.15,
    maxZoom: 1,
  });
  const projected = projectedRect(worldRect, viewport);

  assert.ok(projected.left >= frameArea.left);
  assert.ok(projected.right <= frameArea.left + frameArea.width);
  assert.ok(projected.top >= frameArea.top);
  assert.ok(projected.bottom <= frameArea.top + frameArea.height);
});

test("camera framing uses the full canvas when no chrome overlaps it", () => {
  const frameArea = computeCameraFrameArea({
    canvasRect: { left: 56, top: 44, right: 1200, bottom: 820, width: 1144, height: 776 },
    obstructionRects: [
      { left: 0, top: 44, right: 56, bottom: 820, width: 56, height: 776 },
    ],
    margin: 16,
  });

  assert.deepEqual(frameArea, {
    left: 16,
    top: 16,
    width: 1112,
    height: 744,
  });
});

test("camera framing expands geometry to the actual layout size before fitting", () => {
  const rect = expandWorldRectForLayoutSize(
    { x: 1176, y: 592, width: 360, height: 260 },
    { width: 420, height: 260 },
  );

  assert.deepEqual(rect, {
    x: 1176,
    y: 592,
    width: 420,
    height: 260,
  });

  const viewport = computeViewportForWorldRect(rect, {
    frameArea: { left: 0, top: 0, width: 334, height: 772 },
    fillRatio: 0.92,
    minZoom: 0.15,
    maxZoom: 1,
  });
  const projected = projectedRect(rect, viewport);

  assert.ok(projected.right <= 334);
});

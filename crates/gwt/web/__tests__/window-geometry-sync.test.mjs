import assert from "node:assert/strict";
import test from "node:test";

import * as geometrySync from "../window-geometry-sync.js";
import {
  beginLocalGeometryEdit,
  clearLocalGeometryEdit,
  commitLocalGeometryEdit,
  createGeometrySyncState,
  localGeometryBaseRevision,
  shouldApplyWorkspaceGeometry,
  workspaceGeometryRevision,
} from "../window-geometry-sync.js";

test("workspaceGeometryRevision treats missing legacy revisions as zero", () => {
  assert.equal(workspaceGeometryRevision({ id: "w-1" }), 0);
  assert.equal(workspaceGeometryRevision({ id: "w-1", geometry_revision: 7 }), 7);
});

test("active local resize suppresses stale workspace geometry until a newer revision arrives", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 3);

  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 3 }),
    false,
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 2 }),
    false,
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 4 }),
    true,
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 4 }),
    true,
    "newer workspace geometry should clear the local edit guard"
  );
});

test("pending local resize commit keeps suppressing stale workspace geometry", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 8);
  commitLocalGeometryEdit(state, "w-1", 8);

  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 8 }),
    false,
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 9 }),
    true,
  );
});

test("pending local resize commit advances the next local base revision", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 0);
  commitLocalGeometryEdit(state, "w-1", 0);

  assert.equal(
    localGeometryBaseRevision(state, "w-1", { id: "w-1", geometry_revision: 0 }),
    1,
  );

  beginLocalGeometryEdit(
    state,
    "w-1",
    localGeometryBaseRevision(state, "w-1", { id: "w-1", geometry_revision: 0 }),
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 1 }),
    false,
    "the first resize ack must not overwrite a second in-flight resize",
  );

  commitLocalGeometryEdit(state, "w-1", 1);
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 1 }),
    false,
  );
  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 2 }),
    true,
  );
});

test("clearLocalGeometryEdit removes the stale workspace geometry guard", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 2);
  clearLocalGeometryEdit(state, "w-1");

  assert.equal(
    shouldApplyWorkspaceGeometry(state, { id: "w-1", geometryRevision: 2 }),
    true,
  );
});

test("resize release geometry uses the pointer-end event coordinates", () => {
  assert.equal(typeof geometrySync.syncResizeStatePointerEvent, "function");
  assert.equal(typeof geometrySync.resizeGeometryFromPointerState, "function");

  const resizeState = {
    startX: 100,
    startY: 50,
    latestClientX: 126,
    latestClientY: 66,
    width: 500,
    height: 300,
  };

  const synced = geometrySync.syncResizeStatePointerEvent(resizeState, {
    clientX: 190,
    clientY: 130,
  });
  const geometry = geometrySync.resizeGeometryFromPointerState(resizeState, {
    zoom: 2,
    minWidth: 420,
    minHeight: 260,
  });

  assert.equal(synced, true);
  assert.equal(resizeState.latestClientX, 190);
  assert.equal(resizeState.latestClientY, 130);
  assert.deepEqual(geometry, {
    clientX: 190,
    clientY: 130,
    width: 545,
    height: 340,
  });
});

// maximizedGeometry must keep a CONSTANT 24px SCREEN inset at every zoom.
// bounds are world-space (viewport divided by zoom); the window lives inside
// #canvas-stage which applies scale(zoom). A raw +24 world-unit inset rendered
// as 24*zoom screen px and drifted the maximized window off the viewport at
// zoom != 1. Screen inset = geometry.x * zoom + viewport.x; with
// geometry.x = bounds.x + 24/zoom and bounds.x = -viewport.x/zoom this is a
// constant 24px regardless of zoom.
test("maximizedGeometry keeps a constant 24px screen inset at zoom = 1", () => {
  // viewport.x = 0, zoom = 1 → visibleBounds.x = 0
  const g = geometrySync.maximizedGeometry({ x: 0, y: 0, width: 1000, height: 800 }, 1);
  assert.deepEqual(g, { x: 24, y: 24, width: 952, height: 752 });
});

test("maximizedGeometry divides the screen inset by zoom so it does not drift when zoomed in", () => {
  // zoom = 2, viewport.x = 0 → visibleBounds = { x: 0, width: clientWidth/zoom }
  // For a 1000px-wide canvas at zoom 2: visibleBounds.width = 500.
  const z = 2;
  const bounds = { x: 0, y: 0, width: 1000 / z, height: 800 / z };
  const g = geometrySync.maximizedGeometry(bounds, z);
  // world inset must be 24/zoom = 12 so that *zoom screen = 24px constant.
  assert.equal(g.x, 12);
  assert.equal(g.y, 12);
  // screen-space left = g.x * zoom + viewport.x(0) = 24 (constant)
  assert.equal(g.x * z, 24);
  // screen-space width = g.width * zoom = clientWidth - 48 (24 each side)
  assert.equal(g.width * z, 1000 - 48);
});

test("maximizedGeometry divides the screen inset by zoom when zoomed out", () => {
  const z = 0.5;
  const bounds = { x: 0, y: 0, width: 1000 / z, height: 800 / z };
  const g = geometrySync.maximizedGeometry(bounds, z);
  assert.equal(g.x * z, 24);
  assert.equal(g.width * z, 1000 - 48);
});

test("maximizedGeometry defaults zoom to 1 and never returns negative size", () => {
  const g = geometrySync.maximizedGeometry({ x: 5, y: 5, width: 10, height: 10 });
  assert.equal(g.x, 29);
  assert.equal(g.width, 0); // 10 - 48 clamped to 0
  assert.equal(g.height, 0);
});

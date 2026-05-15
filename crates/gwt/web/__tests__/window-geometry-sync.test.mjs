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

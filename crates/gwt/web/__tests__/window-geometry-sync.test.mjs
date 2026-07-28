import assert from "node:assert/strict";
import test from "node:test";

import * as geometrySync from "../window-geometry-sync.js";
import {
  ACTIVE_EDIT_EXPIRY_MS,
  PENDING_EDIT_EXPIRY_MS,
  beginLocalGeometryEdit,
  clearLocalGeometryEdit,
  commitLocalGeometryEdit,
  createGeometrySyncState,
  localGeometryBaseRevision,
  resolveIncomingGeometry,
  workspaceGeometryRevision,
} from "../window-geometry-sync.js";

const G0 = { x: 120, y: 100, width: 520, height: 300 };
const G1 = { x: 320, y: 250, width: 520, height: 300 };

test("workspaceGeometryRevision treats missing legacy revisions as zero", () => {
  assert.equal(workspaceGeometryRevision({ id: "w-1" }), 0);
  assert.equal(workspaceGeometryRevision({ id: "w-1", geometry_revision: 7 }), 7);
});

// Issue #3364 — the guard must be decidable at processing time WITHOUT knowing
// the echo's revision: while a gesture is active or a commit is pending, a
// backlogged stale `workspace_state` (ANY revision) must not clobber the local
// geometry. Only the commit's own echo (matched by CONTENT) or expiry releases
// the guard.
test("resolveIncomingGeometry applies incoming geometry when no local edit exists", () => {
  const state = createGeometrySyncState();
  const decision = resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 1_000 });
  assert.deepEqual(decision, { apply: true, patchGeometry: null });
});

test("an active gesture suppresses ALL incoming geometry regardless of revision", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 3, 1_000);

  // Stale echo of the current state.
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 1_100 }).apply,
    false,
  );
  // Even a state that would have carried a NEWER revision must not fight the
  // pointer mid-gesture — the commit at gesture end resolves it.
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G1, now: 1_200 }).apply,
    false,
  );
  // The active suppression asks the caller to keep its own (DOM) truth.
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G1, now: 1_300 }).patchGeometry,
    null,
  );
});

test("a leaked active gesture expires after ACTIVE_EDIT_EXPIRY_MS", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 3, 1_000);

  assert.equal(
    resolveIncomingGeometry(state, {
      id: "w-1",
      geometry: G0,
      now: 1_000 + ACTIVE_EDIT_EXPIRY_MS - 1,
    }).apply,
    false,
  );
  assert.equal(
    resolveIncomingGeometry(state, {
      id: "w-1",
      geometry: G0,
      now: 1_001 + ACTIVE_EDIT_EXPIRY_MS,
    }).apply,
    true,
    "expired active edits must stop suppressing server truth",
  );
  // Expiry clears the entry: the next state applies normally.
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 2_000 + ACTIVE_EDIT_EXPIRY_MS })
      .apply,
    true,
  );
});

test("a pending commit suppresses stale geometry and patches the model with the committed truth", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 8, 1_000);
  commitLocalGeometryEdit(state, "w-1", 8, G1, 2_000);

  // Backlogged pre-commit states (old geometry, any revision) are suppressed…
  const stale = resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 2_100 });
  assert.equal(stale.apply, false);
  // …and the caller is told to patch the incoming model to the committed
  // geometry so minimap / model / DOM stay consistent while the echo is in
  // flight.
  assert.deepEqual(stale.patchGeometry, G1);

  // Repeated stale states keep being suppressed (the queue can hold many).
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 2_200 }).apply,
    false,
  );
});

test("a pending commit is released by its own echo, matched by content", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 8, 1_000);
  commitLocalGeometryEdit(state, "w-1", 8, G1, 2_000);

  const echo = resolveIncomingGeometry(state, { id: "w-1", geometry: { ...G1 }, now: 2_500 });
  assert.deepEqual(echo, { apply: true, patchGeometry: null });

  // The echo cleared the guard: later server states apply normally again.
  assert.equal(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 2_600 }).apply,
    true,
  );
});

test("echo matching tolerates sub-pixel float drift", () => {
  const state = createGeometrySyncState();
  commitLocalGeometryEdit(state, "w-1", 0, G1, 1_000);

  const echo = resolveIncomingGeometry(state, {
    id: "w-1",
    geometry: { x: G1.x + 0.4, y: G1.y - 0.4, width: G1.width + 0.2, height: G1.height },
    now: 1_100,
  });
  assert.equal(echo.apply, true, "sub-pixel drift must still count as the commit echo");
});

test("a pending commit expires after PENDING_EDIT_EXPIRY_MS (lost echo safety valve)", () => {
  const state = createGeometrySyncState();
  commitLocalGeometryEdit(state, "w-1", 8, G1, 1_000);

  assert.equal(
    resolveIncomingGeometry(state, {
      id: "w-1",
      geometry: G0,
      now: 1_000 + PENDING_EDIT_EXPIRY_MS - 1,
    }).apply,
    false,
  );
  assert.equal(
    resolveIncomingGeometry(state, {
      id: "w-1",
      geometry: G0,
      now: 1_001 + PENDING_EDIT_EXPIRY_MS,
    }).apply,
    true,
    "server truth must win once the echo is considered lost",
  );
});

test("pending local commit advances the next local base revision for automated senders", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 0, 1_000);
  commitLocalGeometryEdit(state, "w-1", 0, G1, 1_100);

  assert.equal(
    localGeometryBaseRevision(state, "w-1", { id: "w-1", geometry_revision: 0 }),
    1,
  );
});

test("clearLocalGeometryEdit removes the guard (drag cancel / no-move click)", () => {
  const state = createGeometrySyncState();
  beginLocalGeometryEdit(state, "w-1", 2, 1_000);
  clearLocalGeometryEdit(state, "w-1");

  assert.deepEqual(
    resolveIncomingGeometry(state, { id: "w-1", geometry: G0, now: 1_100 }),
    { apply: true, patchGeometry: null },
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

// SPEC-2008 2026-06-20 Camera Focus Rework: maximizedGeometry was removed.
// Issue #3364: `shouldApplyWorkspaceGeometry` (revision-arithmetic guard) was
// replaced by `resolveIncomingGeometry` (content-matched echo ack) because a
// backlogged receive queue makes ANY revision comparison undecidable at
// processing time. The geometry-sync module owns gesture conflict suppression
// and pointer-resize helpers.

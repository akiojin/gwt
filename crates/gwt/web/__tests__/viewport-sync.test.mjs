import assert from "node:assert/strict";
import test from "node:test";

import { createViewportSyncState } from "../viewport-sync.js";

test("pending local viewport ignores stale server viewport until the ack arrives", () => {
  const sync = createViewportSyncState({
    initialViewport: { x: 0, y: 0, zoom: 1 },
  });
  const localViewport = { x: -120, y: -40, zoom: 1 };

  assert.deepEqual(sync.applyLocalViewport(localViewport), localViewport);
  assert.equal(sync.hasPendingLocalViewport(), true);
  assert.deepEqual(
    sync.applyServerViewport({ x: 0, y: 0, zoom: 1 }),
    localViewport,
    "stale workspace_state must not overwrite an in-flight local pan",
  );
  assert.equal(sync.hasPendingLocalViewport(), true);

  assert.deepEqual(sync.applyServerViewport(localViewport), localViewport);
  assert.equal(
    sync.hasPendingLocalViewport(),
    false,
    "matching server ack should release the local guard",
  );
});

test("server viewport applies immediately when there is no pending local edit", () => {
  const sync = createViewportSyncState({
    initialViewport: { x: 0, y: 0, zoom: 1 },
  });

  assert.deepEqual(
    sync.applyServerViewport({ x: 24, y: 36, zoom: 1.25 }),
    { x: 24, y: 36, zoom: 1.25 },
  );
  assert.equal(sync.hasPendingLocalViewport(), false);
});

test("a newer local viewport remains authoritative until its own ack arrives", () => {
  const sync = createViewportSyncState({
    initialViewport: { x: 0, y: 0, zoom: 1 },
  });

  sync.applyLocalViewport({ x: -100, y: 0, zoom: 1 });
  const latest = sync.applyLocalViewport({ x: -180, y: -30, zoom: 1.1 });

  assert.deepEqual(
    sync.applyServerViewport({ x: -100, y: 0, zoom: 1 }),
    latest,
    "ack for a superseded local viewport must not release the latest edit",
  );
  assert.equal(sync.hasPendingLocalViewport(), true);
  assert.deepEqual(sync.applyServerViewport(latest), latest);
  assert.equal(sync.hasPendingLocalViewport(), false);
});

test("server viewport for a different scope bypasses pending local edits", () => {
  const sync = createViewportSyncState({
    initialViewport: { x: 0, y: 0, zoom: 1 },
  });

  sync.applyLocalViewport({ x: -90, y: -10, zoom: 1 }, { scopeKey: "tab-1" });

  assert.deepEqual(
    sync.applyServerViewport({ x: 400, y: 240, zoom: 0.8 }, { scopeKey: "tab-2" }),
    { x: 400, y: 240, zoom: 0.8 },
    "project switch must not be blocked by stale viewport acks from the previous tab",
  );
  assert.equal(sync.hasPendingLocalViewport(), false);
});

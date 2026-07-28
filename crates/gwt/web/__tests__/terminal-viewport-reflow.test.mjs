// SPEC-2008 Phase 24 / T-184..T-186 — terminal viewport reflow on host
// resize and tab visibility transitions. Behaviour tests drive the
// extracted controller (terminal-viewport-reflow.js) so the operation
// shape is exercised end-to-end (`.gwt/work/memory.md` 2026-05-07 memory —
// window interaction features need behavior tests, not only source-string
// contract). app.js still imports the same primitives, and a thin
// source-string assertion at the bottom makes sure the wiring stays in
// place.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import * as terminalViewportReflow from "../terminal-viewport-reflow.js";

import {
  applyVisibilityTransition,
  attachContainerResizeReflow,
  attachHostResizeReflow,
  classifyProjectWindowVisibility,
  clearTerminalOutputBeforeSnapshot,
  createTerminalFitScheduler,
  createTerminalSnapshotWriteCoordinator,
  createTerminalViewportRefreshScheduler,
  decodeTerminalSnapshotBoundary,
  elementHasLayoutBox,
  gateTerminalInputForReadiness,
  rearmRefreshOnVisible,
  runTerminalActivationSequence,
  viewportEligibleForRefresh,
} from "../terminal-viewport-reflow.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E7): syncMaximizedWindowsToViewport moved from app.js
// to project-shell-surface.js; the maximized-sync wiring assertion reads
// the surface module while the rest of the wiring stays pinned to app.js.
const projectShellSurfaceSource = readFileSync(
  resolve(here, "../project-shell-surface.js"),
  "utf8",
);
const appCssSource = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function fixtureWindow() {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  return document.defaultView;
}

function asyncTerminalStub() {
  const writes = [];
  const resets = [];
  const callbacks = [];
  let nextResetFailure = null;
  let nextWriteFailure = null;
  return {
    writes,
    resets,
    write(text, callback) {
      if (nextWriteFailure) {
        const { beforeThrow, error } = nextWriteFailure;
        nextWriteFailure = null;
        beforeThrow?.();
        throw error;
      }
      writes.push(text);
      if (typeof callback === "function") {
        callbacks.push(callback);
      }
    },
    reset() {
      if (nextResetFailure) {
        const { beforeThrow, error } = nextResetFailure;
        nextResetFailure = null;
        beforeThrow?.();
        throw error;
      }
      resets.push(writes.length);
    },
    failNextReset(error, beforeThrow) {
      nextResetFailure = { beforeThrow, error };
    },
    failNextWrite(error, beforeThrow) {
      nextWriteFailure = { beforeThrow, error };
    },
    completeNextWrite() {
      const callback = callbacks.shift();
      assert.equal(typeof callback, "function", "expected a pending xterm write callback");
      callback();
    },
  };
}

function snapshotWriteHarness({
  completionRefreshError = null,
  failureSettlementError = null,
} = {}) {
  const windowId = "wt-async-snapshot";
  const terminal = asyncTerminalStub();
  const pendingSnapshotMap = new Map();
  const pendingOutputMap = new Map();
  const runtime = { deferredWrites: [] };
  const installedDecoders = [];
  const decodedSnapshots = [];
  const errors = [];
  const failureSettlements = [];
  const refreshes = [];
  let nextDecodeFailure = null;
  let nextTakeError = null;
  let runtimeCurrent = true;

  const coordinator = createTerminalSnapshotWriteCoordinator({
    terminal,
    hasPendingSnapshot: () => pendingSnapshotMap.has(windowId),
    takePendingSnapshot: () => {
      if (nextTakeError) {
        const error = nextTakeError;
        nextTakeError = null;
        throw error;
      }
      if (!pendingSnapshotMap.has(windowId)) {
        return { present: false };
      }
      const snapshot = pendingSnapshotMap.get(windowId);
      pendingSnapshotMap.delete(windowId);
      return { present: true, snapshot };
    },
    discardPendingSnapshot: () => pendingSnapshotMap.delete(windowId),
    decodeSnapshot: (snapshot) => {
      if (nextDecodeFailure) {
        const { beforeThrow, error } = nextDecodeFailure;
        nextDecodeFailure = null;
        beforeThrow?.();
        throw error;
      }
      decodedSnapshots.push(snapshot);
      return {
        snapshotText: snapshot ? `decoded:${snapshot}` : "",
        nextLiveDecoder: `decoder:${snapshot}`,
      };
    },
    isRuntimeCurrent: () => runtimeCurrent,
    installLiveDecoder: (decoder) => installedDecoders.push(decoder),
    onSnapshotFailureSettled: () => {
      failureSettlements.push("settled");
      if (failureSettlementError) {
        throw failureSettlementError;
      }
      installedDecoders.push("decoder:failure");
      flushDeferredWrites();
    },
    onLatestSnapshotWritten: () => {
      try {
        refreshes.push("refresh");
        if (completionRefreshError) {
          throw completionRefreshError;
        }
      } finally {
        flushDeferredWrites();
      }
    },
    onError: (error, stage) => errors.push({ error, stage }),
  });

  function receiveSnapshot(snapshot) {
    clearTerminalOutputBeforeSnapshot({
      windowId,
      runtime,
      pendingOutputMap,
      clearBatchedOutput: () => {},
    });
    pendingSnapshotMap.set(windowId, snapshot);
    return coordinator.start();
  }

  function receiveLive(text) {
    if (coordinator.shouldDeferOutput()) {
      runtime.deferredWrites.push(text);
    } else {
      terminal.write(text);
    }
  }

  function flushDeferredWrites() {
    const deferred = runtime.deferredWrites;
    runtime.deferredWrites = [];
    for (const text of deferred) {
      terminal.write(text);
    }
  }

  return {
    coordinator,
    decodedSnapshots,
    errors,
    failureSettlements,
    failNextDecode(error, beforeThrow) {
      nextDecodeFailure = { beforeThrow, error };
    },
    failNextTake(error) {
      nextTakeError = error;
    },
    installedDecoders,
    receiveLive,
    receiveSnapshot,
    refreshes,
    runtime,
    setRuntimeCurrent(value) {
      runtimeCurrent = value;
    },
    terminal,
  };
}

test("decodeTerminalSnapshotBoundary isolates snapshot and subsequent live UTF-8 state (T-159/T-162)", () => {
  const windowId = "wt-decoder-boundary";
  const decoderMap = new Map([[windowId, new TextDecoder()]]);
  const previousLiveDecoder = decoderMap.get(windowId);

  assert.equal(
    previousLiveDecoder.decode(Uint8Array.of(0xe3), { stream: true }),
    "",
    "the live decoder must retain the incomplete Japanese prefix before snapshot replacement",
  );

  const expectedSnapshot = "snapshot 日本語";
  const { snapshotText, nextLiveDecoder } = decodeTerminalSnapshotBoundary(
    new TextEncoder().encode(expectedSnapshot),
  );
  decoderMap.set(windowId, nextLiveDecoder);

  assert.equal(snapshotText, expectedSnapshot);
  assert.equal(snapshotText.includes("\uFFFD"), false);
  assert.notEqual(nextLiveDecoder, previousLiveDecoder);
  assert.equal(
    decoderMap.get(windowId).decode(new TextEncoder().encode("後続ライブ"), { stream: true }),
    "後続ライブ",
    "post-snapshot live output must decode through a fresh streaming decoder",
  );
});

test("clearTerminalOutputBeforeSnapshot drops only output before the latest snapshot boundary", () => {
  const windowId = "wt-snapshot-boundary";
  const pendingOutputMap = new Map([[windowId, ["pending-before-first"]]]);
  const runtime = { deferredWrites: ["deferred-before-first"] };
  const batchedOutputMap = new Map([[windowId, ["batched-before-first"]]]);
  const clearedBatches = [];
  const clearBoundary = () =>
    clearTerminalOutputBeforeSnapshot({
      windowId,
      runtime,
      pendingOutputMap,
      clearBatchedOutput: (id) => {
        clearedBatches.push(id);
        batchedOutputMap.delete(id);
      },
    });

  clearBoundary();

  assert.deepEqual(clearedBatches, [windowId]);
  assert.equal(pendingOutputMap.has(windowId), false);
  assert.deepEqual(runtime.deferredWrites, []);
  assert.equal(batchedOutputMap.has(windowId), false);

  pendingOutputMap.set(windowId, ["pending-between-snapshots"]);
  runtime.deferredWrites.push("deferred-between-snapshots");
  batchedOutputMap.set(windowId, ["batched-between-snapshots"]);
  clearBoundary();

  assert.deepEqual(clearedBatches, [windowId, windowId]);
  assert.equal(pendingOutputMap.has(windowId), false);
  assert.deepEqual(runtime.deferredWrites, []);
  assert.equal(batchedOutputMap.has(windowId), false);

  pendingOutputMap.set(windowId, ["pending-after-latest"]);
  runtime.deferredWrites.push("deferred-after-latest");
  batchedOutputMap.set(windowId, ["batched-after-latest"]);

  assert.deepEqual(pendingOutputMap.get(windowId), ["pending-after-latest"]);
  assert.deepEqual(runtime.deferredWrites, ["deferred-after-latest"]);
  assert.deepEqual(batchedOutputMap.get(windowId), ["batched-after-latest"]);
});

test("snapshot coordinator drains issued live writes and applies only the latest empty snapshot", () => {
  const harness = snapshotWriteHarness();
  harness.terminal.write("live-before-snapshot", () => {});

  harness.receiveSnapshot("A");
  harness.receiveSnapshot("B");
  harness.receiveSnapshot("");

  assert.deepEqual(harness.terminal.writes, ["live-before-snapshot", ""]);
  assert.deepEqual(harness.terminal.resets, []);

  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.terminal.resets, [], "the barrier must wait behind issued live output");

  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.decodedSnapshots, [""]);
  assert.equal(harness.terminal.resets.length, 1);
  assert.deepEqual(harness.terminal.writes, ["live-before-snapshot", "", ""]);

  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.installedDecoders, ["decoder:"]);
  assert.deepEqual(harness.refreshes, ["refresh"]);
});

test("snapshot coordinator serializes a newer snapshot and its live output after an in-flight snapshot", () => {
  const harness = snapshotWriteHarness();

  harness.receiveSnapshot("A");
  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.decodedSnapshots, ["A"]);
  assert.deepEqual(harness.terminal.writes, ["", "decoded:A"]);

  harness.receiveSnapshot("B");
  harness.receiveLive("live-after-B");
  assert.deepEqual(harness.runtime.deferredWrites, ["live-after-B"]);

  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.decodedSnapshots, ["A", "B"]);
  assert.equal(harness.terminal.resets.length, 2);
  assert.deepEqual(harness.installedDecoders, []);
  assert.deepEqual(harness.refreshes, []);
  assert.deepEqual(harness.terminal.writes, ["", "decoded:A", "decoded:B"]);

  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.installedDecoders, ["decoder:B"]);
  assert.deepEqual(harness.refreshes, ["refresh"]);
  assert.deepEqual(harness.runtime.deferredWrites, []);
  assert.deepEqual(harness.terminal.writes, ["", "decoded:A", "decoded:B", "live-after-B"]);
});

test("snapshot coordinator callbacks are no-ops after terminal runtime teardown", () => {
  const harness = snapshotWriteHarness();

  harness.receiveSnapshot("A");
  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.terminal.writes, ["", "decoded:A"]);

  harness.receiveLive("must-not-flush");
  harness.setRuntimeCurrent(false);
  harness.terminal.completeNextWrite();

  assert.deepEqual(harness.installedDecoders, []);
  assert.deepEqual(harness.refreshes, []);
  assert.deepEqual(harness.runtime.deferredWrites, ["must-not-flush"]);
  assert.deepEqual(harness.terminal.writes, ["", "decoded:A"]);
});

for (const [stage, armFailure] of [
  ["take", (harness, error) => harness.failNextTake(error)],
  ["decode", (harness, error) => harness.failNextDecode(error)],
  ["reset", (harness, error) => harness.terminal.failNextReset(error)],
  ["snapshot-write", (harness, error) => harness.terminal.failNextWrite(error)],
]) {
  test(`snapshot coordinator releases the latch after a synchronous ${stage} failure`, () => {
    const harness = snapshotWriteHarness();
    const failure = new Error(`${stage} failed`);

    assert.equal(harness.receiveSnapshot("broken"), true);
    harness.receiveLive("live-after-snapshot");
    armFailure(harness, failure);
    assert.doesNotThrow(() => harness.terminal.completeNextWrite());

    assert.equal(harness.coordinator.shouldDeferOutput(), false);
    assert.deepEqual(harness.errors, [{ error: failure, stage }]);
    assert.deepEqual(harness.failureSettlements, ["settled"]);
    assert.deepEqual(harness.runtime.deferredWrites, []);
    assert.equal(harness.terminal.writes.at(-1), "live-after-snapshot");
    assert.deepEqual(harness.installedDecoders, ["decoder:failure"]);
    harness.receiveLive("live-after-failure");
    assert.equal(harness.terminal.writes.at(-1), "live-after-failure");

    assert.equal(harness.receiveSnapshot("valid"), true);
    harness.terminal.completeNextWrite();
    harness.terminal.completeNextWrite();
    assert.deepEqual(harness.installedDecoders, ["decoder:failure", "decoder:valid"]);
  });
}

test("snapshot coordinator discards a snapshot when the barrier write throws synchronously", () => {
  const harness = snapshotWriteHarness();
  const failure = new Error("barrier write failed");
  harness.terminal.failNextWrite(failure);

  assert.doesNotThrow(() => assert.equal(harness.receiveSnapshot("broken"), false));
  assert.equal(harness.coordinator.shouldDeferOutput(), false);
  assert.deepEqual(harness.errors, [{ error: failure, stage: "barrier-write" }]);
  assert.deepEqual(harness.failureSettlements, ["settled"]);
  assert.deepEqual(harness.installedDecoders, ["decoder:failure"]);

  harness.receiveLive("live-after-barrier-failure");
  assert.equal(harness.terminal.writes.at(-1), "live-after-barrier-failure");
  assert.equal(harness.receiveSnapshot("valid"), true);
  harness.terminal.completeNextWrite();
  harness.terminal.completeNextWrite();
  assert.deepEqual(harness.installedDecoders, ["decoder:failure", "decoder:valid"]);
});

for (const [stage, armFailure] of [
  ["decode", (harness, error, beforeThrow) => harness.failNextDecode(error, beforeThrow)],
  [
    "reset",
    (harness, error, beforeThrow) => harness.terminal.failNextReset(error, beforeThrow),
  ],
  [
    "snapshot-write",
    (harness, error, beforeThrow) => harness.terminal.failNextWrite(error, beforeThrow),
  ],
]) {
  test(`snapshot coordinator automatically starts pending B after A ${stage} failure`, () => {
    const harness = snapshotWriteHarness();
    const failure = new Error(`A ${stage} failed`);

    harness.receiveSnapshot("A");
    harness.receiveLive("live-after-A");
    armFailure(harness, failure, () => {
      harness.receiveSnapshot("B");
      harness.receiveLive("live-after-B");
    });
    harness.terminal.completeNextWrite();

    assert.equal(harness.coordinator.shouldDeferOutput(), true);
    assert.deepEqual(harness.errors, [{ error: failure, stage }]);
    assert.deepEqual(harness.failureSettlements, []);
    assert.deepEqual(harness.runtime.deferredWrites, ["live-after-B"]);
    assert.equal(harness.terminal.writes.includes("live-after-A"), false);
    assert.equal(harness.terminal.writes.at(-1), "", "B must receive a fresh barrier");

    harness.terminal.completeNextWrite();
    harness.terminal.completeNextWrite();
    assert.deepEqual(harness.installedDecoders, ["decoder:B"]);
    assert.deepEqual(harness.runtime.deferredWrites, []);
    assert.equal(harness.terminal.writes.at(-1), "live-after-B");
    assert.equal(harness.coordinator.shouldDeferOutput(), false);
  });
}

test("snapshot failure settlement replaces an incomplete live decoder before flushing", () => {
  const windowId = "wt-failed-snapshot-decoder";
  const terminal = asyncTerminalStub();
  const pendingSnapshotMap = new Map([[windowId, "A"]]);
  const deferredLive = [new TextEncoder().encode("live-after-A")];
  const decodedLive = [];
  const errors = [];
  let liveDecoder = new TextDecoder();
  const incompleteLiveDecoder = liveDecoder;
  incompleteLiveDecoder.decode(Uint8Array.of(0xe3), { stream: true });

  const coordinator = createTerminalSnapshotWriteCoordinator({
    terminal,
    hasPendingSnapshot: () => pendingSnapshotMap.has(windowId),
    takePendingSnapshot: () => {
      const snapshot = pendingSnapshotMap.get(windowId);
      pendingSnapshotMap.delete(windowId);
      return { present: true, snapshot };
    },
    discardPendingSnapshot: () => pendingSnapshotMap.delete(windowId),
    decodeSnapshot: () => {
      throw new Error("decode failed");
    },
    isRuntimeCurrent: () => true,
    installLiveDecoder: (decoder) => {
      liveDecoder = decoder;
    },
    onSnapshotFailureSettled: () => {
      liveDecoder = new TextDecoder();
      for (const bytes of deferredLive.splice(0)) {
        decodedLive.push(liveDecoder.decode(bytes, { stream: true }));
      }
    },
    onLatestSnapshotWritten: () => {},
    onError: (error, stage) => errors.push({ error, stage }),
  });

  coordinator.start();
  terminal.completeNextWrite();

  assert.notEqual(liveDecoder, incompleteLiveDecoder);
  assert.deepEqual(decodedLive, ["live-after-A"]);
  assert.equal(decodedLive[0].includes("\uFFFD"), false);
  assert.equal(
    liveDecoder.decode(new TextEncoder().encode("後続ライブ"), { stream: true }),
    "後続ライブ",
  );
  assert.equal(coordinator.shouldDeferOutput(), false);
  assert.equal(errors[0]?.stage, "decode");
});

test("snapshot failure settlement callback errors are isolated after latch release", () => {
  const settlementFailure = new Error("settlement failed");
  const harness = snapshotWriteHarness({ failureSettlementError: settlementFailure });
  const decodeFailure = new Error("decode failed");

  harness.receiveSnapshot("A");
  harness.failNextDecode(decodeFailure);
  assert.doesNotThrow(() => harness.terminal.completeNextWrite());

  assert.equal(harness.coordinator.shouldDeferOutput(), false);
  assert.deepEqual(harness.errors, [
    { error: decodeFailure, stage: "decode" },
    { error: settlementFailure, stage: "failure-settlement" },
  ]);
});

test("snapshot completion flushes deferred live output when its refresh effect throws", () => {
  const refreshFailure = new Error("refresh failed");
  const harness = snapshotWriteHarness({ completionRefreshError: refreshFailure });

  harness.receiveSnapshot("A");
  harness.receiveLive("live-after-A");
  harness.terminal.completeNextWrite();
  assert.doesNotThrow(() => harness.terminal.completeNextWrite());

  assert.deepEqual(harness.runtime.deferredWrites, []);
  assert.equal(harness.terminal.writes.at(-1), "live-after-A");
  assert.equal(harness.coordinator.shouldDeferOutput(), false);
  assert.deepEqual(harness.errors, [{ error: refreshFailure, stage: "completion" }]);
});

test("app routes snapshot receipts and live output through the async snapshot coordinator", () => {
  assert.match(
    appSource,
    /createTerminalSnapshotWriteCoordinator,/,
    "app.js must import the async snapshot write coordinator",
  );
  assert.match(
    appSource,
    /runtime\.snapshotWriteCoordinator\s*=\s*createTerminalSnapshotWriteCoordinator\(\{[\s\S]*?terminal,[\s\S]*?hasPendingSnapshot:[\s\S]*?takePendingSnapshot:[\s\S]*?discardPendingSnapshot:[\s\S]*?decodeSnapshot:[\s\S]*?isRuntimeCurrent:[\s\S]*?installLiveDecoder:[\s\S]*?onSnapshotFailureSettled:[\s\S]*?onLatestSnapshotWritten:[\s\S]*?onError:/,
    "each terminal runtime must settle failed snapshots, guard stale callbacks, and report isolated errors",
  );
  assert.match(
    appSource,
    /onSnapshotFailureSettled:\s*\(\)\s*=>\s*\{[\s\S]*?decoderMap\.set\(windowId,\s*new TextDecoder\(\)\);[\s\S]*?flushDeferredTerminalWrites\(windowId,\s*runtime\);/,
    "failed snapshot settlement must publish a fresh live decoder before flushing deferred output",
  );
  assert.match(
    appSource,
    /onFlush:\s*\(windowId\)\s*=>\s*\{[\s\S]*?snapshotWriteCoordinator\?\.shouldDeferOutput\(\)\s*===\s*true[\s\S]*?return;[\s\S]*?scheduleTerminalViewportRefresh\(windowId\);/,
    "callbacks from writes issued before the barrier must not refresh an in-flight snapshot",
  );
  assert.match(
    appSource,
    /runtime\.isReady\s*===\s*false\s*\|\|\s*runtime\.snapshotWriteCoordinator\?\.shouldDeferOutput\(\)\s*===\s*true[\s\S]*?runtime\.deferredWrites\.push\(base64\);/,
    "live output must stay encoded and deferred while a snapshot write is pending or in flight",
  );
  assert.match(
    appSource,
    /function replaceTerminalSnapshot\(windowId,\s*base64\)\s*\{[\s\S]*?clearTerminalOutputBeforeSnapshot\(\{[\s\S]*?pendingSnapshotMap\.set\(windowId,\s*base64\);[\s\S]*?runtime\.snapshotWriteCoordinator\.start\(\);/,
    "every snapshot receipt must clear older queues, replace the pending snapshot, and start or join the coordinator",
  );
  assert.match(
    appSource,
    /if\s*\(pendingSnapshotMap\.has\(windowId\)\)\s*\{[\s\S]*?runtime\.snapshotWriteCoordinator\.start\(\);[\s\S]*?\}/,
    "the initial-fit handshake must start queued empty snapshots without consuming them outside the coordinator",
  );
  assert.match(
    appSource,
    /onLatestSnapshotWritten:\s*\(\)\s*=>\s*\{\s*try\s*\{[\s\S]*?forceTerminalViewportRefresh\(windowId,[\s\S]*?\}\s*finally\s*\{[\s\S]*?flushDeferredTerminalWrites\(windowId,\s*runtime\);[\s\S]*?\}/,
    "final snapshot completion must flush post-boundary live output even when viewport refresh throws",
  );
});

test("attachHostResizeReflow fans fitTerminal(persist=true) across visible terminals (T-184/T-187)", () => {
  const window = fixtureWindow();
  const terminals = ["wtA", "wtB", "wtC"];
  const fitCalls = [];
  const beforeFanCalls = [];

  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => terminals,
    canRefreshViewport: (id) => id !== "wtB", // wtB is hidden / minimised.
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    beforeFan: () => beforeFanCalls.push("flushed"),
  });

  window.dispatchEvent(new window.Event("resize"));

  assert.deepEqual(beforeFanCalls, ["flushed"]);
  assert.deepEqual(fitCalls, [
    ["wtA", true],
    ["wtC", true],
  ]);

  // dispose detaches the listener so subsequent resize events do not
  // double-fire fan-out (regression against repeated wiring).
  dispose();
  window.dispatchEvent(new window.Event("resize"));
  assert.deepEqual(fitCalls.length, 2, "dispose() must remove the listener");
});

test("applyVisibilityTransition fires onReveal only on hidden -> visible with terminal (T-185/T-188)", () => {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  const make = (hidden) => {
    const el = document.createElement("section");
    el.hidden = hidden;
    return el;
  };

  // hidden -> visible with terminal: must call onReveal and clear .hidden.
  let revealed = 0;
  const hiddenWithTerminal = make(true);
  const fired = applyVisibilityTransition({
    element: hiddenWithTerminal,
    shouldHide: false,
    hasTerminal: true,
    onReveal: () => {
      revealed += 1;
    },
  });
  assert.equal(fired, true);
  assert.equal(revealed, 1);
  assert.equal(hiddenWithTerminal.hidden, false);

  // hidden -> visible but no terminal runtime yet: do NOT fire (avoids
  // scheduling fit on a window that has not mounted xterm).
  let revealedNoTerm = 0;
  const hiddenNoTerminal = make(true);
  const firedNoTerm = applyVisibilityTransition({
    element: hiddenNoTerminal,
    shouldHide: false,
    hasTerminal: false,
    onReveal: () => {
      revealedNoTerm += 1;
    },
  });
  assert.equal(firedNoTerm, false);
  assert.equal(revealedNoTerm, 0);
  assert.equal(hiddenNoTerminal.hidden, false);

  // visible -> visible: no transition, do NOT fire.
  let revealedVisible = 0;
  const visibleEl = make(false);
  applyVisibilityTransition({
    element: visibleEl,
    shouldHide: false,
    hasTerminal: true,
    onReveal: () => {
      revealedVisible += 1;
    },
  });
  assert.equal(revealedVisible, 0);

  // visible -> hidden: do NOT fire and apply the new hidden state.
  let revealedHide = 0;
  const becomingHidden = make(false);
  applyVisibilityTransition({
    element: becomingHidden,
    shouldHide: true,
    hasTerminal: true,
    onReveal: () => {
      revealedHide += 1;
    },
  });
  assert.equal(revealedHide, 0);
  assert.equal(becomingHidden.hidden, true);
});

test("viewportEligibleForRefresh skips display:none and minimised windows (T-186)", () => {
  const { document } = parseHTML(`<!doctype html><body></body>`);
  const visibleEl = document.createElement("section");
  visibleEl.hidden = false;
  document.body.appendChild(visibleEl);
  const hiddenEl = document.createElement("section");
  hiddenEl.hidden = true;
  document.body.appendChild(hiddenEl);

  // Hidden element short-circuits before the workspace state is consulted.
  assert.equal(
    viewportEligibleForRefresh({ element: hiddenEl, workspaceWindow: { minimized: false } }),
    false,
    ".hidden element must skip refresh",
  );

  // Visible + minimised: the existing minimised short-circuit still wins.
  assert.equal(
    viewportEligibleForRefresh({ element: visibleEl, workspaceWindow: { minimized: true } }),
    false,
    "minimised workspace state must skip refresh",
  );

  // Visible + not minimised: refresh allowed.
  assert.equal(
    viewportEligibleForRefresh({ element: visibleEl, workspaceWindow: { minimized: false } }),
    true,
  );

  const disconnectedEl = document.createElement("section");
  disconnectedEl.hidden = false;
  assert.equal(
    viewportEligibleForRefresh({
      element: disconnectedEl,
      workspaceWindow: { minimized: false },
    }),
    false,
    "detached elements must skip refresh",
  );

  // Defensive: missing element / workspaceWindow falls back to allow.
  assert.equal(
    viewportEligibleForRefresh({ element: null, workspaceWindow: null }),
    true,
  );
});

test("rearmRefreshOnVisible reruns a pending hidden viewport refresh once visible (T-199)", () => {
  const calls = [];
  let pending = true;
  let visible = false;

  const attemptHidden = rearmRefreshOnVisible({
    hasPendingRefresh: () => pending,
    canRefresh: () => visible,
    clearPendingRefresh: () => {
      pending = false;
      calls.push("clear");
    },
    scheduleRefresh: () => calls.push("refresh"),
  });

  assert.equal(attemptHidden, false, "hidden windows keep the pending refresh armed");
  assert.equal(pending, true, "pending flag must survive while hidden");
  assert.deepEqual(calls, []);

  visible = true;
  const attemptVisible = rearmRefreshOnVisible({
    hasPendingRefresh: () => pending,
    canRefresh: () => visible,
    clearPendingRefresh: () => {
      pending = false;
      calls.push("clear");
    },
    scheduleRefresh: () => calls.push("refresh"),
  });

  assert.equal(attemptVisible, true);
  assert.equal(pending, false, "pending flag must clear before the refresh is scheduled");
  assert.deepEqual(calls, ["clear", "refresh"]);
});

test("rearmRefreshOnVisible is a no-op when no hidden refresh is pending (T-199)", () => {
  const calls = [];
  const didRearm = rearmRefreshOnVisible({
    hasPendingRefresh: () => false,
    canRefresh: () => true,
    clearPendingRefresh: () => calls.push("clear"),
    scheduleRefresh: () => calls.push("refresh"),
  });
  assert.equal(didRearm, false);
  assert.deepEqual(calls, []);
});

test("runTerminalFitRequest marks hidden persisted fits pending without stale activation", () => {
  const calls = [];
  const result = terminalViewportReflow.runTerminalFitRequest({
    persist: true,
    canFit: () => {
      calls.push("can-fit");
      return false;
    },
    activate: () => {
      calls.push("activate");
      return { ran: true };
    },
    markPending: () => calls.push("mark-pending"),
  });

  assert.equal(result, null);
  assert.deepEqual(calls, ["can-fit", "mark-pending"]);
});

test("runTerminalFitRequest keeps unresolved persisted activation pending", () => {
  const calls = [];
  const activation = { ran: false, reason: "layout-pending" };
  const result = terminalViewportReflow.runTerminalFitRequest({
    persist: true,
    canFit: () => {
      calls.push("can-fit");
      return true;
    },
    activate: () => {
      calls.push("activate");
      return activation;
    },
    markPending: () => calls.push("mark-pending"),
  });

  assert.equal(result, activation);
  assert.deepEqual(calls, ["can-fit", "activate", "mark-pending"]);
});

test("runTerminalRevealActivation schedules one persisted activation after consuming pending refresh", () => {
  const calls = [];
  const result = terminalViewportReflow.runTerminalRevealActivation({
    schedulePendingOutput: () => {
      calls.push("schedule-output");
      return true;
    },
    consumePendingRefresh: () => {
      calls.push("consume-pending-refresh");
      return true;
    },
    scheduleActivation: (options) => calls.push(["activation", options]),
  });

  assert.deepEqual(calls, [
    "schedule-output",
    "consume-pending-refresh",
    ["activation", { shouldPersistGeometry: true }],
  ]);
  assert.deepEqual(result, {
    pendingOutputScheduled: true,
    pendingRefreshConsumed: true,
    activationScheduled: true,
  });
});

test("runTerminalRevealActivation schedules exactly one persisted activation without pending refresh", () => {
  const calls = [];
  const result = terminalViewportReflow.runTerminalRevealActivation({
    schedulePendingOutput: () => {
      calls.push("schedule-output");
      return false;
    },
    consumePendingRefresh: () => {
      calls.push("consume-pending-refresh");
      return false;
    },
    scheduleActivation: (options) => calls.push(["activation", options]),
  });

  assert.deepEqual(calls, [
    "schedule-output",
    "consume-pending-refresh",
    ["activation", { shouldPersistGeometry: true }],
  ]);
  assert.deepEqual(result, {
    pendingOutputScheduled: false,
    pendingRefreshConsumed: false,
    activationScheduled: true,
  });
});

test("activation intent coalescing keeps authoritative persistence through retries", () => {
  let pendingIntent = terminalViewportReflow.mergeTerminalActivationIntent(null, {
    shouldPersistGeometry: false,
    reason: "topmost_focus",
  });
  pendingIntent = terminalViewportReflow.mergeTerminalActivationIntent(pendingIntent, {
    shouldPersistGeometry: true,
    reason: "visibility_reveal",
  });
  pendingIntent = terminalViewportReflow.mergeTerminalActivationIntent(pendingIntent, {
    shouldPersistGeometry: false,
    reason: "topmost_focus",
  });

  const consumed = terminalViewportReflow.takeTerminalActivationIntent(pendingIntent);
  assert.deepEqual(consumed, {
    intent: {
      shouldPersistGeometry: true,
      reason: "visibility_reveal",
    },
    pendingIntent: null,
  });
  let retryIntent = terminalViewportReflow.mergeTerminalActivationIntent(
    null,
    consumed.intent,
  );
  retryIntent = terminalViewportReflow.mergeTerminalActivationIntent(retryIntent, {
    shouldPersistGeometry: false,
    reason: "topmost_focus",
  });
  assert.deepEqual(terminalViewportReflow.takeTerminalActivationIntent(retryIntent), {
    intent: {
      shouldPersistGeometry: true,
      reason: "visibility_reveal",
    },
    pendingIntent: null,
  });
});

test("activation settlement rearms only failed authoritative viewport refreshes", () => {
  const settle = terminalViewportReflow.resolveTerminalViewportRefreshSettlement;
  const cases = [
    {
      name: "persisted activation failure",
      input: {
        activationRan: false,
        shouldPersistGeometry: true,
        hasPendingRefresh: false,
      },
      expected: { shouldUpdate: true, pending: true },
    },
    {
      name: "consumed pending refresh failure",
      input: {
        activationRan: false,
        shouldPersistGeometry: false,
        hasPendingRefresh: true,
      },
      expected: { shouldUpdate: true, pending: true },
    },
    {
      name: "focus-only failure",
      input: {
        activationRan: false,
        shouldPersistGeometry: false,
        hasPendingRefresh: false,
      },
      expected: { shouldUpdate: false, pending: false },
    },
  ];

  for (const { name, input, expected } of cases) {
    assert.deepEqual(settle?.(input), expected, name);
  }
});

test("successful authoritative activation clears the viewport refresh settlement", () => {
  assert.deepEqual(
    terminalViewportReflow.resolveTerminalViewportRefreshSettlement?.({
      activationRan: true,
      shouldPersistGeometry: true,
      hasPendingRefresh: true,
    }),
    { shouldUpdate: true, pending: false },
  );
});

test("retry exhaustion preserves refresh intent for a later visibility restore", () => {
  const settle = terminalViewportReflow.resolveTerminalViewportRefreshSettlement;
  const host = {
    clientWidth: 0,
    clientHeight: 0,
    getBoundingClientRect: () => ({
      width: host.clientWidth,
      height: host.clientHeight,
    }),
  };
  const geometry = [];
  const runtime = {
    viewportRefreshPending: false,
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: host },
      refresh: () => {},
      focus: () => {},
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 120, rows: 36 }),
      fit: () => {
        runtime.terminal.cols = 120;
        runtime.terminal.rows = 36;
      },
    },
  };
  const applySettlement = (activation, shouldPersistGeometry, hasPendingRefresh) => {
    const settlement = settle?.({
      activationRan: activation.ran,
      shouldPersistGeometry,
      hasPendingRefresh,
    });
    if (settlement?.shouldUpdate) {
      runtime.viewportRefreshPending = settlement.pending;
    }
  };

  // One initial frame plus HANDSHAKE_RETRY_LIMIT (60) bounded retries.
  for (let attempt = 0; attempt <= 60; attempt += 1) {
    const hasPendingRefresh = runtime.viewportRefreshPending;
    const activation = runTerminalActivationSequence({
      runtime,
      windowId: "retry-exhaustion",
      shouldPersistGeometry: true,
      hasPendingRefresh,
      sendGeometry: (windowId, cols, rows) =>
        geometry.push({ windowId, cols, rows }),
    });
    assert.equal(activation.ran, false);
    applySettlement(activation, true, hasPendingRefresh);
  }

  assert.equal(runtime.viewportRefreshPending, true);
  assert.deepEqual(geometry, []);

  // A later document visibility restore consumes the flag, then succeeds
  // once layout has settled and clears the authoritative refresh state.
  const pendingConsumed = rearmRefreshOnVisible({
    hasPendingRefresh: () => runtime.viewportRefreshPending,
    canRefresh: () => true,
    clearPendingRefresh: () => {
      runtime.viewportRefreshPending = false;
    },
  });
  assert.equal(pendingConsumed, true);
  host.clientWidth = 960;
  host.clientHeight = 540;
  const restored = runTerminalActivationSequence({
    runtime,
    windowId: "retry-exhaustion",
    shouldPersistGeometry: true,
    hasPendingRefresh: true,
    sendGeometry: (windowId, cols, rows) =>
      geometry.push({ windowId, cols, rows }),
  });
  applySettlement(restored, true, true);

  assert.equal(restored.ran, true);
  assert.equal(runtime.viewportRefreshPending, false);
  assert.deepEqual(geometry, [
    { windowId: "retry-exhaustion", cols: 120, rows: 36 },
  ]);
});

test("pending reveal and topmost focus share one authoritative activation frame", () => {
  const frameQueue = [];
  let scheduledFrameCount = 0;
  const callOrder = [];
  const parent = {
    clientWidth: 960,
    clientHeight: 540,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      return { width: 960, height: 540 };
    },
  };
  const runtime = {
    viewportRefreshPending: true,
    activationFrame: null,
    // A persisted request already ran while hidden: its frame was consumed,
    // but the authoritative intent must remain queued for the next reveal.
    pendingActivationIntent: {
      shouldPersistGeometry: true,
      reason: "hidden_persisted_request",
    },
    isReady: true,
    lastSuccessfulActivationSnapshot: {
      width: 960,
      height: 540,
      cols: 132,
      rows: 38,
    },
    terminal: {
      cols: 132,
      rows: 38,
      element: { parentElement: parent },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 132, rows: 38 }),
      fit: () => callOrder.push("fit"),
    },
  };
  const activations = [];

  const scheduleFocusActivation = ({ shouldPersistGeometry, reason }) => {
    runtime.pendingActivationIntent =
      terminalViewportReflow.mergeTerminalActivationIntent(
        runtime.pendingActivationIntent,
        { shouldPersistGeometry, reason },
      );
    if (runtime.activationFrame !== null) return;
    scheduledFrameCount += 1;
    runtime.activationFrame = scheduledFrameCount;
    frameQueue.push(() => {
      runtime.activationFrame = null;
      const consumed = terminalViewportReflow.takeTerminalActivationIntent(
        runtime.pendingActivationIntent,
      );
      runtime.pendingActivationIntent = consumed.pendingIntent;
      const activation = runTerminalActivationSequence({
        runtime,
        windowId: "pending-reveal",
        shouldFocus: true,
        shouldPersistGeometry: consumed.intent.shouldPersistGeometry,
        syncGeometryOnGridChange: true,
        allowFastPath: true,
        sendGeometry: () => callOrder.push("sendGeometry"),
      });
      activations.push({ intent: consumed.intent, activation });
    });
  };

  const reveal = terminalViewportReflow.runTerminalRevealActivation({
    schedulePendingOutput: () => false,
    consumePendingRefresh: () =>
      rearmRefreshOnVisible({
        hasPendingRefresh: () => runtime.viewportRefreshPending,
        canRefresh: () => true,
        clearPendingRefresh: () => {
          runtime.viewportRefreshPending = false;
        },
      }),
    scheduleActivation: ({ shouldPersistGeometry }) =>
      scheduleFocusActivation({
        shouldPersistGeometry,
        reason: "visibility_reveal",
      }),
  });
  scheduleFocusActivation({
    shouldPersistGeometry: false,
    reason: "topmost_focus",
  });

  assert.deepEqual(reveal, {
    pendingOutputScheduled: false,
    pendingRefreshConsumed: true,
    activationScheduled: true,
  });
  assert.equal(runtime.viewportRefreshPending, false);
  assert.equal(scheduledFrameCount, 1);
  assert.equal(frameQueue.length, 1);

  frameQueue.shift()();

  assert.deepEqual(callOrder, ["refresh", "flush-layout", "fit", "sendGeometry", "focus"]);
  assert.equal(callOrder.filter((call) => call === "sendGeometry").length, 1);
  assert.equal(callOrder.filter((call) => call === "focus").length, 1);
  assert.equal(runtime.viewportRefreshPending, false);
  assert.equal(runtime.pendingActivationIntent, null);
  assert.equal(runtime.activationFrame, null);
  assert.equal(activations.length, 1);
  assert.deepEqual(activations[0].intent, {
    shouldPersistGeometry: true,
    reason: "hidden_persisted_request",
  });
  assert.equal(activations[0].activation.fastPath, false);
  assert.equal(activations[0].activation.geometrySent, true);
});

test("observeTerminalFontMetricsReady routes only current runtimes by visibility", async () => {
  let resolveFontsReady;
  const fontsReady = new Promise((resolve) => {
    resolveFontsReady = resolve;
  });
  const currentRuntimes = new Map([
    ["visible", {}],
    ["hidden", {}],
    ["removed-before-ready", {}],
  ]);
  const calls = [];

  assert.equal(
    terminalViewportReflow.observeTerminalFontMetricsReady({
      fontsReady,
      terminalIds: () => currentRuntimes.keys(),
      canRefresh: (windowId) => {
        calls.push(`can-refresh:${windowId}`);
        return windowId !== "hidden";
      },
      scheduleFit: (windowId, persist) =>
        calls.push(`fit:${windowId}:${String(persist)}`),
      markPending: (windowId) => calls.push(`pending:${windowId}`),
    }),
    true,
  );

  currentRuntimes.delete("removed-before-ready");
  currentRuntimes.set("created-before-ready", {});
  resolveFontsReady();
  await fontsReady;
  await Promise.resolve();

  assert.deepEqual(calls, [
    "can-refresh:visible",
    "fit:visible:true",
    "can-refresh:hidden",
    "pending:hidden",
    "can-refresh:created-before-ready",
    "fit:created-before-ready:true",
  ]);
  assert.equal(calls.some((call) => call.includes("removed-before-ready")), false);
});

test("runTerminalActivationSequence renders before fit and emits geometry (T-199 / FR-056)", () => {
  // SPEC-2008 Phase 26.B / FR-056 regression: a hidden -> visible
  // transition must call terminal.refresh() before fitAddon.fit() so
  // xterm has populated cell metrics by the time proposeDimensions runs.
  // The previous Phase 24 ordering (fit-then-refresh) became a silent
  // no-op because proposeDimensions returns undefined whenever
  // _renderService.dimensions.css.cell.width === 0, which is exactly
  // the state of a freshly-revealed display:none element.
  const callOrder = [];
  let layoutFlushed = 0;
  const parent = {
    clientWidth: 800,
    clientHeight: 480,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      layoutFlushed += 1;
      return { width: 800, height: 480 };
    },
  };
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: parent },
      refresh: (start, end) => {
        callOrder.push(`refresh:${start}-${end}`);
      },
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
    },
  };
  let geometry = null;
  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-A",
    sendGeometry: (id, cols, rows) => {
      callOrder.push("sendGeometry");
      geometry = { id, cols, rows };
    },
  });
  assert.deepEqual(
    callOrder,
    ["refresh:0-23", "flush-layout", "fit", "sendGeometry", "focus"],
    "refresh must precede layout flush, fit, sendGeometry, and focus",
  );
  assert.equal(layoutFlushed, 1, "parent.getBoundingClientRect must be called exactly once");
  assert.deepEqual(geometry, { id: "win-A", cols: 80, rows: 24 });
  assert.equal(result.ran, true);
  assert.equal(result.cols, 80);
  assert.equal(result.rows, 24);
});

test("runTerminalActivationSequence honours shouldFocus / shouldPersistGeometry flags (T-199)", () => {
  const callOrder = [];
  const parent = {
    clientWidth: 800,
    clientHeight: 480,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      return { width: 800, height: 480 };
    },
  };
  const runtime = {
    terminal: {
      cols: 100,
      rows: 30,
      element: { parentElement: parent },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
    },
  };
  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-B",
    shouldFocus: false,
    shouldPersistGeometry: false,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });
  // sendGeometry / focus are suppressed by the flags.
  assert.deepEqual(callOrder, ["refresh", "flush-layout", "fit"]);
  assert.equal(result.ran, true);
});

test("runTerminalActivationSequence syncs geometry when focus reflow changes the xterm grid (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 112, rows: 28 }),
      fit: () => {
        callOrder.push("fit");
        runtime.terminal.cols = 112;
        runtime.terminal.rows = 28;
      },
    },
  };
  let geometry = null;

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-focus-grid-changed",
    shouldFocus: false,
    shouldPersistGeometry: false,
    syncGeometryOnGridChange: true,
    sendGeometry: (id, cols, rows) => {
      callOrder.push("sendGeometry");
      geometry = { id, cols, rows };
    },
  });

  assert.deepEqual(
    callOrder,
    ["refresh", "fit", "sendGeometry"],
    "focus reflow must sync backend geometry exactly when fit changes cols/rows",
  );
  assert.deepEqual(geometry, { id: "win-focus-grid-changed", cols: 112, rows: 28 });
  assert.deepEqual(result, {
    ran: true,
    cols: 112,
    rows: 28,
    fastPath: false,
    gridChanged: true,
    geometrySent: true,
    reason: "activated",
  });
});

test("runTerminalActivationSequence does not sync unchanged focus grids (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 100,
      rows: 30,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 100, rows: 30 }),
      fit: () => callOrder.push("fit"),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-focus-grid-unchanged",
    shouldFocus: false,
    shouldPersistGeometry: false,
    syncGeometryOnGridChange: true,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["refresh", "fit"]);
  assert.deepEqual(result, {
    ran: true,
    cols: 100,
    rows: 30,
    fastPath: false,
    gridChanged: false,
    geometrySent: false,
    reason: "activated",
  });
});

test("runTerminalActivationSequence uses a focus-only fast path for same-grid ready activations (T-307)", () => {
  const callOrder = [];
  const parent = {
    clientWidth: 960,
    clientHeight: 540,
  };
  const runtime = {
    isReady: true,
    lastSuccessfulActivationSnapshot: {
      width: 960,
      height: 540,
      cols: 132,
      rows: 38,
    },
    terminal: {
      cols: 132,
      rows: 38,
      element: { parentElement: parent },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => ({ cols: 132, rows: 38 }),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-fast-path",
    shouldPersistGeometry: false,
    syncGeometryOnGridChange: true,
    allowFastPath: true,
    pendingOutputCount: 0,
    hasPendingRefresh: false,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["focus"]);
  assert.deepEqual(result, {
    ran: true,
    cols: 132,
    rows: 38,
    fastPath: true,
    gridChanged: false,
    geometrySent: false,
    reason: "same-grid",
  });
});

test("runTerminalActivationSequence prioritizes persisted geometry over the same-grid fast path", () => {
  const callOrder = [];
  const parent = {
    clientWidth: 960,
    clientHeight: 540,
    getBoundingClientRect: () => {
      callOrder.push("flush-layout");
      return { width: 960, height: 540 };
    },
  };
  const runtime = {
    isReady: true,
    lastSuccessfulActivationSnapshot: {
      width: 960,
      height: 540,
      cols: 132,
      rows: 38,
    },
    terminal: {
      cols: 132,
      rows: 38,
      element: { parentElement: parent },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => ({ cols: 132, rows: 38 }),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-authoritative-same-grid",
    shouldFocus: false,
    shouldPersistGeometry: true,
    syncGeometryOnGridChange: true,
    allowFastPath: true,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["refresh", "flush-layout", "fit", "sendGeometry"]);
  assert.equal(result.fastPath, false);
  assert.equal(result.gridChanged, false);
  assert.equal(result.geometrySent, true);
});

test("runTerminalActivationSequence keeps full activation for pending output, refresh, unready, and changed layout (T-308)", () => {
  function makeRuntime(overrides = {}) {
    const callOrder = [];
    const parent = {
      clientWidth: overrides.width ?? 960,
      clientHeight: overrides.height ?? 540,
    };
    const runtime = {
      isReady: overrides.isReady ?? true,
      lastSuccessfulActivationSnapshot: {
        width: 960,
        height: 540,
        cols: 132,
        rows: 38,
      },
      terminal: {
        cols: 132,
        rows: 38,
        element: { parentElement: parent },
        refresh: () => callOrder.push("refresh"),
        focus: () => callOrder.push("focus"),
      },
      fitAddon: {
        fit: () => callOrder.push("fit"),
        proposeDimensions: () => ({ cols: 132, rows: 38 }),
      },
    };
    return { runtime, callOrder };
  }

  for (const [name, options] of [
    ["pending output", { pendingOutputCount: 1 }],
    ["pending viewport refresh", { hasPendingRefresh: true }],
    ["unready runtime", { isReady: false }],
    ["changed layout box", { width: 980 }],
  ]) {
    const { runtime, callOrder } = makeRuntime(options);
    const result = runTerminalActivationSequence({
      runtime,
      windowId: `win-${name}`,
      shouldFocus: false,
      shouldPersistGeometry: false,
      syncGeometryOnGridChange: true,
      allowFastPath: true,
      pendingOutputCount: options.pendingOutputCount ?? 0,
      hasPendingRefresh: options.hasPendingRefresh ?? false,
      sendGeometry: () => callOrder.push("sendGeometry"),
    });

    assert.deepEqual(callOrder, ["refresh", "fit"], `${name} must keep full activation`);
    assert.equal(result.fastPath, false, `${name} must not use the fast path`);
  }

  const zeroLayoutCalls = [];
  const zeroLayoutRuntime = {
    isReady: true,
    lastSuccessfulActivationSnapshot: { width: 960, height: 540, cols: 132, rows: 38 },
    terminal: {
      cols: 132,
      rows: 38,
      element: { parentElement: { clientWidth: 0, clientHeight: 540 } },
      refresh: () => zeroLayoutCalls.push("refresh"),
      focus: () => zeroLayoutCalls.push("focus"),
    },
    fitAddon: {
      fit: () => zeroLayoutCalls.push("fit"),
      proposeDimensions: () => ({ cols: 132, rows: 38 }),
    },
  };
  const zeroLayoutResult = runTerminalActivationSequence({
    runtime: zeroLayoutRuntime,
    windowId: "win-zero-layout",
    allowFastPath: true,
    sendGeometry: () => zeroLayoutCalls.push("sendGeometry"),
  });

  assert.deepEqual(zeroLayoutCalls, []);
  assert.equal(zeroLayoutResult.ran, false);
  assert.equal(zeroLayoutResult.fastPath, false);
});

test("runTerminalActivationSequence keeps viewport-only fits off grid-change sync (T-266)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: { parentElement: null },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      proposeDimensions: () => ({ cols: 120, rows: 32 }),
      fit: () => {
        callOrder.push("fit");
        runtime.terminal.cols = 120;
        runtime.terminal.rows = 32;
      },
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-viewport-only",
    shouldFocus: false,
    shouldPersistGeometry: false,
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, ["refresh", "fit"]);
  assert.deepEqual(result, {
    ran: true,
    cols: 120,
    rows: 32,
    fastPath: false,
    gridChanged: true,
    geometrySent: false,
    reason: "activated",
  });
});

test("runTerminalActivationSequence waits for the terminal host layout box before fitting (#2839)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: {
        parentElement: {
          clientWidth: 0,
          clientHeight: 360,
          getBoundingClientRect: () => {
            callOrder.push("flush-layout");
            return { width: 0, height: 360 };
          },
        },
      },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => ({ cols: 100, rows: 28 }),
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-layout-pending",
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(callOrder, [], "0-size terminal host must not fit, send geometry, or focus");
  assert.deepEqual(result, {
    ran: false,
    cols: 80,
    rows: 24,
    fastPath: false,
    gridChanged: false,
    geometrySent: false,
    reason: "layout-pending",
  });
});

test("runTerminalActivationSequence waits when xterm fit dimensions are unavailable (#2839)", () => {
  const callOrder = [];
  const runtime = {
    terminal: {
      cols: 80,
      rows: 24,
      element: {
        parentElement: {
          clientWidth: 800,
          clientHeight: 420,
          getBoundingClientRect: () => {
            callOrder.push("flush-layout");
            return { width: 800, height: 420 };
          },
        },
      },
      refresh: () => callOrder.push("refresh"),
      focus: () => callOrder.push("focus"),
    },
    fitAddon: {
      fit: () => callOrder.push("fit"),
      proposeDimensions: () => undefined,
    },
  };

  const result = runTerminalActivationSequence({
    runtime,
    windowId: "win-cell-pending",
    sendGeometry: () => callOrder.push("sendGeometry"),
  });

  assert.deepEqual(
    callOrder,
    ["refresh", "flush-layout"],
    "unresolved xterm cell metrics must not fit, send geometry, or focus",
  );
  assert.deepEqual(result, {
    ran: false,
    cols: 80,
    rows: 24,
    fastPath: false,
    gridChanged: false,
    geometrySent: false,
    reason: "fit-dimensions-unavailable",
  });
});

test("runTerminalActivationSequence is a no-op when runtime is missing pieces (T-199)", () => {
  const missingResult = {
    ran: false,
    cols: 0,
    rows: 0,
    fastPath: false,
    gridChanged: false,
    geometrySent: false,
    reason: "runtime-missing",
  };
  assert.deepEqual(
    runTerminalActivationSequence({ runtime: null, windowId: "x" }),
    missingResult,
  );
  assert.deepEqual(
    runTerminalActivationSequence({
      runtime: { terminal: null, fitAddon: { fit() {} } },
      windowId: "x",
    }),
    missingResult,
  );
  assert.deepEqual(
    runTerminalActivationSequence({
      runtime: { terminal: { rows: 24, refresh() {}, focus() {} }, fitAddon: null },
      windowId: "x",
    }),
    missingResult,
  );
});

test("classifyProjectWindowVisibility keeps inactive project terminals hidden, not removed", () => {
  const result = classifyProjectWindowVisibility({
    activeWindowIds: ["tab-a::agent-1", "tab-a::board-1"],
    allProjectWindowIds: [
      "tab-a::agent-1",
      "tab-a::board-1",
      "tab-b::agent-1",
    ],
    mountedWindowIds: [
      "tab-a::agent-1",
      "tab-b::agent-1",
      "orphan::agent-1",
    ],
  });

  assert.deepEqual(result.visible, ["tab-a::agent-1", "tab-a::board-1"]);
  assert.deepEqual(result.hidden, ["tab-b::agent-1"]);
  assert.deepEqual(result.removed, ["orphan::agent-1"]);
});

test("classifyProjectWindowVisibility accepts prebuilt id sets", () => {
  const legacy = classifyProjectWindowVisibility({
    activeWindowIds: ["tab-a::agent-1", "tab-a::board-1"],
    allProjectWindowIds: [
      "tab-a::agent-1",
      "tab-a::board-1",
      "tab-b::agent-1",
    ],
    mountedWindowIds: [
      "tab-a::agent-1",
      "tab-b::agent-1",
      "orphan::agent-1",
    ],
  });
  const fromSets = classifyProjectWindowVisibility({
    activeWindowIdSet: new Set(["tab-a::agent-1", "tab-a::board-1"]),
    allProjectWindowIdSet: new Set([
      "tab-a::agent-1",
      "tab-a::board-1",
      "tab-b::agent-1",
    ]),
    mountedWindowIds: [
      "tab-a::agent-1",
      "tab-b::agent-1",
      "orphan::agent-1",
    ],
  });

  assert.deepEqual(fromSets, legacy);
});

test("attachHostResizeReflow throws when given a non-DOM window", () => {
  assert.throws(
    () =>
      attachHostResizeReflow({
        window: null,
        terminalIds: () => [],
        canRefreshViewport: () => true,
        fitTerminal: () => {},
      }),
    /requires a DOM window/,
  );
});

test("elementHasLayoutBox blocks 0-size containers (Issue #2832 / SPEC-2008 Phase 26.A regression)", () => {
  // SPEC-2008 Phase 26.A only checked `.hidden` and `.minimized`, so a
  // structurally-visible window whose flex/grid layout had not propagated
  // could pass the visibility predicate while the parent container was
  // still 0x0. fitAddon then resolved against the broken box, isReady
  // flipped true, and the deferredWrites flushed into xterm's default
  // 80x24 grid — the Claude Code post-launch corruption symptom.
  assert.equal(elementHasLayoutBox({ clientWidth: 800, clientHeight: 480 }), true);
  assert.equal(elementHasLayoutBox({ clientWidth: 0, clientHeight: 480 }), false);
  assert.equal(elementHasLayoutBox({ clientWidth: 800, clientHeight: 0 }), false);
  assert.equal(elementHasLayoutBox({ clientWidth: 0, clientHeight: 0 }), false);

  // Falls back to getBoundingClientRect when client* are unavailable
  // (e.g. linkedom fixtures used elsewhere in this suite).
  assert.equal(
    elementHasLayoutBox({
      getBoundingClientRect: () => ({ width: 600, height: 320 }),
    }),
    true,
  );
  assert.equal(
    elementHasLayoutBox({
      getBoundingClientRect: () => ({ width: 0, height: 320 }),
    }),
    false,
  );

  // Defensive default: missing element falls through (don't pin the
  // handshake retry loop on inputs the predicate can not measure).
  assert.equal(elementHasLayoutBox(null), true);
  assert.equal(elementHasLayoutBox(undefined), true);
  assert.equal(elementHasLayoutBox({}), true);
});

test("attachHostResizeReflow coalesces rapid resize events via requestAnimationFrame (Issue #2903)", () => {
  const window = fixtureWindow();
  let rafCallback = null;
  let rafIdCounter = 1;
  let cancelledIds = [];
  window.requestAnimationFrame = (cb) => {
    rafCallback = cb;
    return rafIdCounter++;
  };
  window.cancelAnimationFrame = (id) => {
    cancelledIds.push(id);
    rafCallback = null;
  };

  const fitCalls = [];
  const beforeFanCalls = [];

  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => ["wtA", "wtB"],
    canRefreshViewport: (id) => id !== "wtB",
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    beforeFan: () => beforeFanCalls.push("flushed"),
  });

  // Fire 5 rapid resize events (simulates Chrome maximize animation).
  for (let i = 0; i < 5; i++) {
    window.dispatchEvent(new window.Event("resize"));
  }

  // Nothing should have executed synchronously — all deferred to rAF.
  assert.equal(fitCalls.length, 0, "fitTerminal must not fire synchronously when rAF is available");
  assert.equal(beforeFanCalls.length, 0, "beforeFan must not fire synchronously when rAF is available");

  // 4 intermediate rAFs should have been cancelled (5 events, only last survives).
  assert.equal(cancelledIds.length, 4, "previous rAF frames must be cancelled on rapid resize");

  // Flush the single surviving rAF callback.
  assert.ok(rafCallback, "a rAF must be scheduled after the last resize");
  rafCallback();

  // Only one fan-out should have run.
  assert.deepEqual(beforeFanCalls, ["flushed"], "beforeFan must fire exactly once");
  assert.deepEqual(fitCalls, [["wtA", true]], "fitTerminal must fire once per visible terminal");

  dispose();
});

test("attachHostResizeReflow dispose cancels pending rAF (Issue #2903)", () => {
  const window = fixtureWindow();
  let rafCallback = null;
  let cancelCount = 0;
  window.requestAnimationFrame = (cb) => { rafCallback = cb; return 99; };
  window.cancelAnimationFrame = () => { cancelCount++; rafCallback = null; };

  const fitCalls = [];
  const dispose = attachHostResizeReflow({
    window,
    terminalIds: () => ["wtA"],
    canRefreshViewport: () => true,
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
  });

  window.dispatchEvent(new window.Event("resize"));
  assert.ok(rafCallback, "rAF must be scheduled");

  // Dispose before rAF fires.
  dispose();
  assert.equal(cancelCount, 1, "dispose must cancel pending rAF");

  // Even if someone flushes an old callback ref, listener is removed.
  window.dispatchEvent(new window.Event("resize"));
  assert.equal(fitCalls.length, 0, "no fits after dispose");
});

test("createTerminalFitScheduler budgets multi-terminal fits across frames", () => {
  const callbacks = [];
  const fitCalls = [];
  const scheduler = createTerminalFitScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    maxFitsPerFrame: 4,
  });

  for (let i = 0; i < 12; i += 1) {
    scheduler.enqueue(`terminal-${i + 1}`, { persist: false });
  }

  assert.equal(callbacks.length, 1, "fit burst must schedule one shared frame");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    ["terminal-1", "terminal-2", "terminal-3", "terminal-4"],
    "first frame must only run the configured fit budget",
  );
  assert.equal(callbacks.length, 1, "remaining fits must share one follow-up frame");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    [
      "terminal-1",
      "terminal-2",
      "terminal-3",
      "terminal-4",
      "terminal-5",
      "terminal-6",
      "terminal-7",
      "terminal-8",
    ],
    "second frame must continue in insertion order",
  );
  assert.equal(callbacks.length, 1, "third frame must be scheduled for the tail");

  callbacks.shift()();
  assert.deepEqual(
    fitCalls.map(([id]) => id),
    Array.from({ length: 12 }, (_, index) => `terminal-${index + 1}`),
    "all queued terminals must eventually fit exactly once",
  );
  assert.equal(callbacks.length, 0, "no extra frames after completion");
  assert.equal(scheduler.pendingCount(), 0);
});

test("createTerminalFitScheduler coalesces same-window fits and preserves persist=true", () => {
  const callbacks = [];
  const fitCalls = [];
  const scheduler = createTerminalFitScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    fitTerminal: (id, persist) => fitCalls.push([id, persist]),
    maxFitsPerFrame: 4,
  });

  assert.equal(scheduler.enqueue("agent-1", { persist: false }), true);
  assert.equal(scheduler.enqueue("agent-1", { persist: true }), true);
  assert.equal(scheduler.enqueue("agent-2", { persist: false }), true);
  assert.equal(callbacks.length, 1, "coalesced requests still share one frame");
  assert.equal(scheduler.pendingCount(), 2);

  callbacks.shift()();

  assert.deepEqual(fitCalls, [
    ["agent-1", true],
    ["agent-2", false],
  ]);
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(callbacks.length, 0);
});

test("createTerminalViewportRefreshScheduler budgets multi-terminal refreshes across frames", () => {
  const callbacks = [];
  const refreshCalls = [];
  const scheduler = createTerminalViewportRefreshScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    canRefresh: () => true,
    refresh: (id) => refreshCalls.push(id),
    maxRefreshesPerFrame: 4,
  });

  for (let i = 0; i < 12; i += 1) {
    scheduler.enqueue(`terminal-${i + 1}`);
  }

  assert.equal(callbacks.length, 1, "refresh burst must schedule one shared frame");
  assert.equal(scheduler.hasPending("terminal-1"), true);
  assert.equal(scheduler.hasPending("terminal-12"), true);
  assert.equal(scheduler.hasPending("terminal-missing"), false);

  callbacks.shift()();
  assert.deepEqual(
    refreshCalls,
    ["terminal-1", "terminal-2", "terminal-3", "terminal-4"],
    "first frame must only run the configured refresh budget",
  );
  assert.equal(callbacks.length, 1, "remaining refreshes must share one follow-up frame");
  assert.equal(scheduler.hasPending("terminal-1"), false);
  assert.equal(scheduler.hasPending("terminal-12"), true);

  callbacks.shift()();
  assert.deepEqual(
    refreshCalls,
    [
      "terminal-1",
      "terminal-2",
      "terminal-3",
      "terminal-4",
      "terminal-5",
      "terminal-6",
      "terminal-7",
      "terminal-8",
    ],
    "second frame must continue in insertion order",
  );
  assert.equal(callbacks.length, 1, "third frame must be scheduled for the tail");

  callbacks.shift()();
  assert.deepEqual(
    refreshCalls,
    Array.from({ length: 12 }, (_, index) => `terminal-${index + 1}`),
    "all queued terminals must eventually refresh exactly once",
  );
  assert.equal(callbacks.length, 0, "no extra frames after completion");
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(scheduler.hasPending("terminal-12"), false);
});

test("createTerminalViewportRefreshScheduler coalesces and marks ineligible windows pending", () => {
  const callbacks = [];
  const refreshCalls = [];
  const pendingMarks = [];
  const eligibility = new Map([
    ["agent-1", true],
    ["agent-2", false],
  ]);
  const scheduler = createTerminalViewportRefreshScheduler({
    schedule: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    canRefresh: (id) => eligibility.get(id) !== false,
    refresh: (id) => refreshCalls.push(id),
    markPending: (id) => pendingMarks.push(id),
    maxRefreshesPerFrame: 4,
  });

  assert.equal(scheduler.enqueue("agent-1"), true);
  assert.equal(scheduler.enqueue("agent-1"), true);
  assert.equal(scheduler.enqueue("agent-2"), true);
  assert.equal(callbacks.length, 1, "coalesced refreshes still share one frame");
  assert.equal(scheduler.pendingCount(), 2);

  callbacks.shift()();

  assert.deepEqual(refreshCalls, ["agent-1"]);
  assert.deepEqual(pendingMarks, ["agent-2"]);
  assert.equal(scheduler.pendingCount(), 0);
  assert.equal(callbacks.length, 0);
});

test("app.js wires the reflow controller for resize, transition, and predicate", () => {
  // Source-string contract retained per the memory — limited to wiring
  // detection so a future refactor that drops the import / call surfaces
  // immediately, without claiming behaviour coverage.
  assert.match(
    appSource,
    /from "\/terminal-viewport-reflow\.js"/,
    "app.js must import terminal-viewport-reflow primitives",
  );
  assert.match(
    appSource,
    /createTerminalFitScheduler\(\{\s*fitTerminal\s*\}\)/,
    "app.js must construct the shared terminal fit scheduler from fitTerminal",
  );
  const fitTerminalSource = appSource.slice(
    appSource.indexOf("function fitTerminal("),
    appSource.indexOf("function scheduleTerminalFit("),
  );
  assert.match(
    fitTerminalSource,
    /runTerminalFitRequest\(\{[\s\S]*?persist,[\s\S]*?canFit:[\s\S]*?markPending:[\s\S]*?activate:[\s\S]*?runTerminalActivationSequence\(\{/,
    "fitTerminal must route persisted hidden and unresolved fits through the pending-aware helper",
  );
  assert.doesNotMatch(
    fitTerminalSource,
    /sendGeometry\(windowId,\s*runtime\.terminal\.cols,\s*runtime\.terminal\.rows\)/,
    "hidden persisted fits must never send stale runtime cols/rows",
  );
  assert.match(
    appSource,
    /createTerminalViewportRefreshScheduler\(\{[\s\S]*?canRefresh:\s*canRefreshTerminalViewport[\s\S]*?refresh:\s*\(windowId\)\s*=>[\s\S]*?refreshTerminalViewport\(windowId\)[\s\S]*?markPending:\s*markTerminalViewportRefreshPending[\s\S]*?\}\)/,
    "app.js must construct the shared terminal viewport refresh scheduler",
  );
  assert.match(
    appSource,
    /function scheduleTerminalFit\(windowId,\s*persist = false\)[\s\S]*?terminalFitScheduler\.enqueue\(windowId,\s*\{\s*persist\s*\}\)/,
    "app.js must expose a scheduleTerminalFit wrapper over the shared fit scheduler",
  );
  assert.match(
    appSource,
    /function scheduleTerminalViewportRefresh\(windowId\)[\s\S]*?terminalViewportRefreshScheduler\.enqueue\(windowId\)/,
    "app.js must route routine terminal viewport refreshes through the shared scheduler",
  );
  assert.match(
    appSource,
    /terminalViewportRefreshScheduler\?\.clear\(windowId\)/,
    "removed terminal windows must be cleared from the shared viewport refresh scheduler",
  );
  assert.match(
    appSource,
    /attachHostResizeReflow\(\{[\s\S]*?fitTerminal:\s*scheduleTerminalFit[\s\S]*?\}\)/,
    "host resize fan-out must route fit requests through the shared scheduler",
  );
  // SPEC-2008 2026-06-20 Camera Focus Rework: syncMaximizedWindowsToViewport
  // was removed (maximize-to-fill is superseded by the per-viewer camera), so
  // the only surviving visual terminal-fit routing is the workspace render
  // geometry-change path below.
  assert.doesNotMatch(
    projectShellSurfaceSource,
    /syncMaximizedWindowsToViewport\s*\(/,
    "the removed maximized viewport sync must not be called",
  );
  assert.match(
    appSource,
    /scheduleTerminalFit\(windowData\.id,\s*shouldPersistTerminalGeometry\)/,
    "workspace render geometry changes must route terminal fits through the shared scheduler",
  );
  assert.match(
    appSource,
    /applyVisibilityTransition\(\{/,
    "render path must apply visibility transition through the helper",
  );
  assert.match(
    appSource,
    /viewportEligibleForRefresh\(\{/,
    "canRefreshTerminalViewport must consult the shared predicate",
  );
  assert.match(
    appSource,
    /classifyProjectWindowVisibility\(\{/,
    "project tab switches must classify inactive project windows as hidden instead of disposing their terminal runtimes",
  );
  assert.match(
    appSource,
    /rearmRefreshOnVisible/,
    "hidden refresh requests must re-arm through the shared visibility helper",
  );
  const pendingRearmStart = appSource.indexOf(
    "function consumePendingTerminalViewportRefresh(",
  );
  const revealActivationStart = appSource.indexOf(
    "function activateTerminalOnReveal(",
  );
  assert.notEqual(pendingRearmStart, -1);
  assert.notEqual(revealActivationStart, -1);
  const pendingRearmSource = appSource.slice(
    pendingRearmStart,
    revealActivationStart,
  );
  assert.doesNotMatch(
    pendingRearmSource,
    /forceTerminalViewportRefresh/,
    "pending viewport rearm must not synchronously compete with the activation scheduler",
  );
  assert.doesNotMatch(
    pendingRearmSource,
    /scheduleRefresh:/,
    "pending viewport rearm must only consume the flag; reveal/restore owns scheduling",
  );
  assert.match(
    appSource,
    /function activateTerminalOnReveal\(windowId\)[\s\S]*?runTerminalRevealActivation\(\{[\s\S]*?shouldPersistGeometry[\s\S]*?visibility_reveal/,
    "all reveal paths must share one persisted geometry router",
  );
  assert.equal(
    (appSource.match(/onReveal:\s*\(\)\s*=>\s*activateTerminalOnReveal\(/g) || []).length,
    2,
    "both workspace reveal surfaces must delegate exactly once to the common router",
  );
  assert.match(
    appSource,
    /observeTerminalFontMetricsReady\(\{[\s\S]*?fontsReady:\s*document\.fonts\?\.ready,[\s\S]*?terminalIds:\s*\(\)\s*=>\s*terminalMap\.keys\(\),[\s\S]*?scheduleFit:\s*scheduleTerminalFit,[\s\S]*?markPending:\s*markTerminalViewportRefreshPending/,
    "font readiness must fan out over the current terminal map using existing fit/pending paths",
  );
  assert.match(
    appSource,
    /document\.addEventListener\("visibilitychange"[\s\S]*?rearmVisibleTerminalViewportRefreshes\(\);/,
    "document visibility restore must re-arm visible terminal viewport refreshes",
  );
  assert.match(
    appSource,
    /function rearmVisibleTerminalViewportRefreshes\(\)[\s\S]*?if\s*\(consumePendingTerminalViewportRefresh\(windowId\)\)\s*\{[\s\S]*?scheduleTerminalFocusActivation\(windowId,\s*\{[\s\S]*?shouldPersistGeometry:\s*true,[\s\S]*?reason:\s*"visibility_restore"[\s\S]*?continue;[\s\S]*?scheduleTerminalViewportRefresh\(windowId\)/,
    "visibility restore must route pending authoritative geometry through the activation scheduler",
  );
  assert.match(
    appSource,
    /function forceTerminalViewportRefresh\(windowId,[\s\S]*?viewportRefreshPending = true[\s\S]*?runTerminalActivationSequence\(\{/,
    "forceTerminalViewportRefresh must mark hidden/unresolved terminals pending and run the activation sequence when visible",
  );
  assert.match(
    appSource,
    /onLatestSnapshotWritten:[\s\S]*?forceTerminalViewportRefresh\(windowId,\s*\{\s*shouldPersistGeometry:\s*true\s*\}\);/,
    "snapshot replay must use the force refresh path so terminal.reset() cannot strand scrollback",
  );
  // SPEC-2008 Phase 26.B / FR-056 wiring: activation path must delegate
  // to runTerminalActivationSequence so refresh-before-fit ordering stays
  // testable. A future refactor that drops the helper or reverts to the
  // legacy fit-then-refresh ordering will fail this assertion and
  // surface the regression in CI immediately.
  assert.match(
    appSource,
    /runTerminalActivationSequence\(\{/,
    "scheduleTerminalFocusActivation must delegate to runTerminalActivationSequence",
  );
  assert.match(
    appSource,
    /scheduleTerminalFocusActivation\(topmostId,\s*\{[\s\S]*?shouldPersistGeometry:\s*false[\s\S]*?reason:\s*"topmost_focus"[\s\S]*?\}\)/,
    "topmost focus activation must not persist geometry unconditionally on every workspace render",
  );
  assert.match(
    appSource,
    /function scheduleTerminalFocusActivation\([\s\S]*?runTerminalActivationSequence\(\{[\s\S]*?shouldPersistGeometry,[\s\S]*?syncGeometryOnGridChange:\s*true,[\s\S]*?sendGeometry,[\s\S]*?\}\);/,
    "focus activation must opt into grid-change geometry sync while keeping caller-owned persistence",
  );
  assert.match(
    appSource,
    /pendingActivationIntent\s*=\s*mergeTerminalActivationIntent\([\s\S]*?if\s*\(runtime\.activationFrame\s*!==\s*null\)\s*\{\s*return;/,
    "focus activation requests must merge persistence intent before an existing-frame early return",
  );
  assert.match(
    appSource,
    /takeTerminalActivationIntent\(\s*activeRuntime\.pendingActivationIntent,?\s*\)[\s\S]*?activeRuntime\.pendingActivationIntent\s*=\s*pendingIntent;[\s\S]*?\{\s*shouldPersistGeometry,\s*reason\s*\}\s*=\s*intent;/,
    "the scheduled frame must consume and clear the effective coalesced activation intent",
  );
  assert.match(
    appSource,
    /runTerminalActivationSequence\(\{[\s\S]*?shouldPersistGeometry,[\s\S]*?traceTerminalActivation\(windowId,\s*activation,\s*\{[\s\S]*?activation_reason:\s*reason,[\s\S]*?should_persist_geometry:\s*shouldPersistGeometry/,
    "activation and trace must use the consumed effective intent",
  );
  assert.match(
    appSource,
    /if\s*\(!activation\.ran\)[\s\S]*?scheduleTerminalFocusActivation\(windowId,\s*\{\s*shouldPersistGeometry,\s*reason,/,
    "a bounded retry must inherit the consumed effective intent",
  );
  assert.match(
    appSource,
    /const refreshSettlement = resolveTerminalViewportRefreshSettlement\(\{[\s\S]*?activationRan:\s*activation\.ran,[\s\S]*?shouldPersistGeometry,[\s\S]*?hasPendingRefresh,[\s\S]*?\}\);[\s\S]*?if \(refreshSettlement\.shouldUpdate\) \{[\s\S]*?activeRuntime\.viewportRefreshPending = refreshSettlement\.pending;[\s\S]*?\}[\s\S]*?if \(!activation\.ran\)[\s\S]*?if \(activation\.fastPath\)/,
    "activation settlement must rearm failed authoritative refreshes before retry exhaustion and clear successful ones before the fast path returns",
  );
  assert.match(
    appSource,
    /pendingActivationIntent:\s*null,/,
    "terminal runtimes must initialize their coalesced activation intent",
  );

  // Issue #2937 — the focus-change reflow path must not give up after one
  // frame. When runTerminalActivationSequence can't resolve a real grid yet
  // (a revealed tab-group member whose container is still 0-size),
  // scheduleTerminalFocusActivation must re-arm a bounded retry
  // (activationAttempts <= HANDSHAKE_RETRY_LIMIT), mirroring the
  // initial-fit handshake. Wiring detection only; the {ran:false} contract
  // itself is covered by the unit tests above.
  assert.match(
    appSource,
    /const activation = runTerminalActivationSequence\(\{/,
    "focus activation must capture the activation result to detect !ran",
  );
  assert.match(
    appSource,
    /if \(!activation\.ran\) \{[\s\S]*?activationAttempts[\s\S]*?HANDSHAKE_RETRY_LIMIT[\s\S]*?scheduleTerminalFocusActivation\(windowId,[\s\S]*?return;/,
    "focus activation must re-arm a bounded retry when the activation did not run (#2937)",
  );
  assert.match(
    appSource,
    /activationAttempts: 0,/,
    "createTerminalRuntime must initialize activationAttempts for the focus-path retry counter (#2937)",
  );

  // Issue #2832 — SPEC-2008 Phase 26.A regression fix: completeInitialFitHandshake
  // must defer (and retry via rAF) while the container has no layout box,
  // so deferredWrites do not flush into xterm's default 80x24 grid before
  // fit can resolve real cols/rows. Wiring detection only — behavior
  // coverage lives in the elementHasLayoutBox unit test above.
  assert.match(
    appSource,
    /elementHasLayoutBox/,
    "app.js must import elementHasLayoutBox so the initial-fit handshake can gate on container layout",
  );
  assert.match(
    appSource,
    /terminalContainerHasLayoutBox\(windowId\)/,
    "completeInitialFitHandshake must consult terminalContainerHasLayoutBox",
  );
  assert.match(
    appSource,
    /function terminalContainerHasLayoutBox\(windowId\) \{[\s\S]*?terminalMap\.get\(windowId\)[\s\S]*?parentElement[\s\S]*?elementHasLayoutBox/,
    "terminalContainerHasLayoutBox must measure the actual xterm host, not only the outer workspace window",
  );
  assert.match(
    appSource,
    /handshakeAttempts/,
    "completeInitialFitHandshake must bound its retry loop with a handshakeAttempts counter",
  );
  assert.match(
    appSource,
    /HANDSHAKE_RETRY_LIMIT/,
    "handshake retry must be capped by HANDSHAKE_RETRY_LIMIT",
  );

  // Issue #2903 — browser lineHeight parity: app.js must detect Blink
  // browsers and adjust xterm lineHeight so the agent terminal line spacing
  // matches the native WebView rendering.
  assert.match(
    appSource,
    /isBlinkBrowser\b/,
    "app.js must define isBlinkBrowser helper for engine-specific lineHeight",
  );
  assert.match(
    appSource,
    /lineHeight:\s*isBlinkBrowser/,
    "createTerminalRuntime must use isBlinkBrowser to select lineHeight",
  );

  // Issue #2924 — stray "C" byte appears in Claude Code prompt buffer on
  // launch. xterm.js can emit onData firings before the initial-fit
  // handshake has completed (e.g. application-response sequences echoed
  // before the deferredWrites flush has even started). The terminal.onData
  // callback must consult gateTerminalInputForReadiness so pre-ready
  // input is dropped instead of contaminating Claude Code's stdin.
  assert.match(
    appSource,
    /gateTerminalInputForReadiness/,
    "terminal.onData must consult gateTerminalInputForReadiness so pre-ready input cannot reach PTY",
  );
});

test("app.css recovers terminal cell columns at the gwt default 720x420 window (Issue #2923 follow-up)", () => {
  // The Claude Code footer (`bypass permissions on (shift+tab to cycle)` +
  // `◯ <effort> · /effort`) lands at ~77 cells. With the original
  // `inset: 8px 10px 10px;` and xterm's vendor `overflow-y: scroll`
  // reserving a scrollbar gutter, the gwt-default 720×420 agent window
  // shrank the cell grid to ~76 cols and Claude Code's footer wrapped
  // `/effort` to `/eff` + `ort`. Pin the tighter inset and the
  // `overflow-y: auto` override so the gutter only steals cells when
  // scrollback is actually present.
  assert.match(
    appCssSource,
    /\.terminal-root\s*\{[^}]*inset:\s*8px\s+4px\s+4px;/,
    ".terminal-root must use the tightened 8px/4px/4px inset so the cell grid keeps ~+1 column at 720x420 windows",
  );
  assert.match(
    appCssSource,
    /\.surface-terminal\s+\.terminal-root\s+\.xterm-viewport\s*\{[^}]*overflow-y:\s*auto;/,
    "xterm-viewport overflow-y must override the vendor `scroll` so the scrollbar gutter is reclaimed when scrollback is empty",
  );
});

test("gateTerminalInputForReadiness drops onData firings before the initial-fit handshake (Issue #2924)", () => {
  // Pre-ready firings exist because xterm.js emits responses to
  // application queries (Primary DA, cursor reports, focus tracking)
  // synchronously inside `terminal.write`, and the deferredWrites flush
  // is itself called from inside the runtime once handshake completes.
  // The user did not press a key — these bytes are xterm.js internal
  // noise that must not reach Claude Code's stdin.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: false }, data: "C" }),
    { forward: false, reason: "runtime-not-ready" },
  );
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: false }, data: "\x1b[C" }),
    { forward: false, reason: "runtime-not-ready" },
  );
});

test("gateTerminalInputForReadiness forwards onData firings once the runtime is ready", () => {
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: { isReady: true }, data: "hello" }),
    { forward: true },
  );
});

test("gateTerminalInputForReadiness forwards when no runtime is registered (defensive)", () => {
  // A missing runtime means the firing was not produced by a gated xterm
  // instance — preserve the legacy behaviour and forward, so non-PTY
  // surfaces (e.g. board / static terminals) keep working if they ever
  // route through the same helper.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: null, data: "C" }),
    { forward: true },
  );
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: undefined, data: "C" }),
    { forward: true },
  );
});

test("gateTerminalInputForReadiness forwards when isReady is missing (legacy runtime)", () => {
  // An older runtime that never set `isReady` should still forward input,
  // because the gate only takes effect when the SPEC-2008 Phase 26.A
  // handshake explicitly enrolled the runtime by setting isReady=false.
  assert.deepEqual(
    gateTerminalInputForReadiness({ runtime: {}, data: "C" }),
    { forward: true },
  );
});

// --- attachContainerResizeReflow: re-fit when the terminal CONTAINER size
// actually changes (maximize/restore/tile/server-geometry/no-op-fit gaps that
// the per-lifecycle-event wiring misses, leaving a black band below the grid).
function makeContainerResizeHarness(initial = { clientWidth: 800, clientHeight: 400 }) {
  const element = { ...initial };
  const observerInstances = [];
  class FakeResizeObserver {
    constructor(callback) {
      this.callback = callback;
      this.observed = [];
      this.disconnected = false;
      observerInstances.push(this);
    }
    observe(target) {
      this.observed.push(target);
    }
    disconnect() {
      this.disconnected = true;
    }
  }
  const fitCalls = [];
  let pendingFrame = null;
  const dispose = attachContainerResizeReflow({
    element,
    windowId: "win-1",
    fitTerminal: (id, persist) => fitCalls.push({ id, persist }),
    ResizeObserverImpl: FakeResizeObserver,
    requestFrame: (cb) => {
      pendingFrame = cb;
      return 7;
    },
    cancelFrame: () => {
      pendingFrame = null;
    },
  });
  const observer = observerInstances[0];
  return {
    element,
    observer,
    fitCalls,
    dispose,
    fire: () => observer.callback(),
    runFrame: () => {
      const cb = pendingFrame;
      pendingFrame = null;
      if (cb) cb();
    },
    pending: () => pendingFrame,
  };
}

test("attachContainerResizeReflow refits with persisted geometry once per coalesced size change", () => {
  const h = makeContainerResizeHarness();
  assert.ok(h.observer, "a ResizeObserver is constructed and observes the container");
  assert.deepEqual(h.observer.observed, [h.element], "observes the terminal container element");

  // Initial observation with unchanged size must NOT schedule a redundant fit
  // (createTerminalRuntime already runs the initial-fit handshake).
  h.fire();
  assert.equal(h.pending(), null, "no fit scheduled when the container size is unchanged");

  // Container grows (e.g. maximize): two rapid notifications coalesce to one fit.
  h.element.clientHeight = 900;
  h.fire();
  h.fire();
  assert.ok(h.pending(), "a frame is scheduled once the container size changes");
  h.runFrame();
  assert.deepEqual(
    h.fitCalls,
    [{ id: "win-1", persist: true }],
    "coalesced into a single fit that persists geometry to the PTY",
  );
});

test("attachContainerResizeReflow defers to the manual drag-resize path via shouldSkip", () => {
  const element = { clientWidth: 800, clientHeight: 400 };
  let cb;
  class FakeResizeObserver {
    constructor(callback) {
      cb = callback;
    }
    observe() {}
    disconnect() {}
  }
  const fitCalls = [];
  let pendingFrame = null;
  let skip = true;
  attachContainerResizeReflow({
    element,
    windowId: "win-1",
    fitTerminal: (id, persist) => fitCalls.push({ id, persist }),
    shouldSkip: () => skip,
    ResizeObserverImpl: FakeResizeObserver,
    requestFrame: (fn) => {
      pendingFrame = fn;
      return 1;
    },
    cancelFrame: () => {
      pendingFrame = null;
    },
  });
  element.clientHeight = 600;
  cb();
  assert.equal(pendingFrame, null, "no fit scheduled while a manual resize owns reflow");
  assert.equal(fitCalls.length, 0);
  // Once the manual resize ends, a later container change refits normally.
  skip = false;
  element.clientHeight = 650;
  cb();
  assert.ok(pendingFrame, "fit scheduled after the manual resize releases");
  pendingFrame();
  assert.deepEqual(fitCalls, [{ id: "win-1", persist: true }]);
});

test("attachContainerResizeReflow dispose disconnects the observer and cancels pending frame", () => {
  const h = makeContainerResizeHarness();
  h.element.clientWidth = 1200;
  h.fire();
  assert.ok(h.pending(), "frame pending before dispose");
  h.dispose();
  assert.equal(h.observer.disconnected, true, "observer disconnected on dispose");
  assert.equal(h.pending(), null, "pending frame cancelled on dispose");
});

test("attachContainerResizeReflow is a no-op when ResizeObserver is unavailable", () => {
  const dispose = attachContainerResizeReflow({
    element: { clientWidth: 10, clientHeight: 10 },
    windowId: "win-1",
    fitTerminal: () => {
      throw new Error("must not fit without a ResizeObserver");
    },
    ResizeObserverImpl: null,
  });
  assert.equal(typeof dispose, "function");
  dispose();
});

test("app.js wires attachContainerResizeReflow on the terminal container", () => {
  // Source-string contract: the container reflow controller must be imported
  // and attached in createTerminalRuntime, and disposed in cleanup.
  assert.match(
    appSource,
    /attachContainerResizeReflow/,
    "app.js must import + use attachContainerResizeReflow",
  );
});

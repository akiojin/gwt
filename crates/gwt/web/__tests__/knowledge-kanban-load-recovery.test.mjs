// Issue #3297 — cache-backed knowledge load recovery contract.
//
// The 5s load-recovery timer must not escalate its retry into a forced
// remote refresh (a forced refresh runs a full GitHub sync that takes
// minutes and guarantees the "Timed out loading cache-backed data" error
// on slow machines), and a late knowledge_entries response must still be
// applied when the window has no data yet instead of being discarded by
// request_id mismatch.
//
// The surface module imports "/focus-trap.js" (a browser absolute path),
// so the tests load the module source with that import stubbed and drive
// the real factory functions.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));

async function importSurfaceModule() {
  const source = readFileSync(
    resolve(here, "../knowledge-kanban-surface.js"),
    "utf8",
  ).replace(
    'from "/focus-trap.js"',
    'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function createSurface(mod, sent) {
  const windowData = { id: "win-1", preset: "issue" };
  return mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    createNode: () => ({ appendChild() {}, classList: { add() {} } }),
    createKnowledgeMarkdownBody: () => ({ appendChild() {} }),
    windowMap: new Map(),
    workspaceWindowById: (id) => (id === "win-1" ? windowData : null),
    getWorkspaceWindows: () => [windowData],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: (preset) => (preset === "issue" ? "issue" : null),
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
  });
}

function withPatchedTimers(run) {
  const timers = [];
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  globalThis.setTimeout = (callback, delay) => {
    timers.push({ callback, delay, cleared: false, fired: false });
    return timers.length - 1;
  };
  globalThis.clearTimeout = (id) => {
    if (typeof id === "number" && timers[id]) {
      timers[id].cleared = true;
    }
  };
  const fire = (predicate) => {
    const timer = timers.find(
      (entry) => !entry.cleared && !entry.fired && predicate(entry),
    );
    assert.ok(timer, "expected a pending timer to fire");
    timer.fired = true;
    timer.callback();
  };
  try {
    return run({ timers, fire });
  } finally {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  }
}

test("load recovery retry must not escalate to a forced refresh", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);

    surface.requestKnowledgeBridge("win-1", "issue", false);
    assert.equal(sent.length, 1);
    assert.equal(sent[0].kind, "load_knowledge_bridge");
    assert.equal(sent[0].refresh, false);

    fire((timer) => timer.delay === 5000);

    assert.equal(sent.length, 2, "recovery timer must retry the load once");
    assert.equal(sent[1].kind, "load_knowledge_bridge");
    assert.equal(
      sent[1].refresh,
      false,
      "the recovery retry must stay a cache read; a forced refresh runs a full remote sync and always outlives the next 5s timer",
    );
  });
});

test("a late knowledge_entries response is applied when the window has no data", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);

    surface.requestKnowledgeBridge("win-1", "issue", false);
    const staleRequestId = sent[0].request_id;
    fire((timer) => timer.delay === 5000);
    const retryRequestId = sent[1].request_id;
    assert.notEqual(staleRequestId, retryRequestId);

    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_entries",
      id: "win-1",
      knowledge_kind: "issue",
      request_id: staleRequestId,
      entries: [{ number: 42, title: "Issue bridge", phase: "backlog" }],
      selected_number: 42,
      empty_message: "",
      refresh_enabled: true,
    });

    const state = surface.knowledgeBridgeStateMap.get("win-1");
    assert.equal(
      state.baseEntries.length,
      1,
      "a late response must still populate an empty window instead of being discarded by request_id",
    );
    assert.equal(state.loading, false, "the applied response must finish the load");
    assert.equal(state.error, "");
  });
});

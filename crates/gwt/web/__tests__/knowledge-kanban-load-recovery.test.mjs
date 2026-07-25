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

test("a stale load detail cannot overwrite a newer Issue row selection", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(() => {
      const sent = [];
      const surface = createSurface(mod, sent);

      surface.requestKnowledgeBridge("win-1", "issue", false);
      const loadRequestId = sent.at(-1).request_id;
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_entries",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: loadRequestId,
        entries: [
          { number: 7, title: "Initial Issue", phase: "backlog" },
          { number: 42, title: "Selected Issue", phase: "backlog" },
        ],
        selected_number: 7,
        empty_message: "",
        refresh_enabled: true,
      });

      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const selectionRequestId = sent.at(-1).request_id;
      assert.notEqual(selectionRequestId, loadRequestId);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: loadRequestId,
        detail: { number: 7, title: "Stale initial detail", sections: [] },
      });

      let state = surface.knowledgeBridgeStateMap.get("win-1");
      assert.equal(
        state.selectedNumber,
        42,
        "the explicit row selection must survive a late initial-load detail",
      );
      assert.equal(state.detailLoading, true);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: selectionRequestId,
        detail: { number: 42, title: "Selected detail", sections: [] },
      });
      state = surface.knowledgeBridgeStateMap.get("win-1");
      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detail?.number, 42);
      assert.equal(state.detailLoading, false);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        detail: { number: 7, title: "Legacy stale detail", sections: [] },
      });
      assert.equal(
        state.selectedNumber,
        42,
        "an uncorrelated legacy detail for another row must not replace the active selection",
      );
      assert.equal(state.detail?.number, 42);
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("rapid Issue A to B to A selection applies only the newest detail generation", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(() => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Issue A", phase: "backlog" },
        { number: 42, title: "Issue B", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;
      state.detail = { number: 7, title: "Issue A", sections: [] };

      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const requestB = sent.at(-1).request_id;
      surface.requestKnowledgeDetail("win-1", "issue", 7);
      const requestA = sent.at(-1).request_id;

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: requestB,
        detail: { number: 42, title: "Late Issue B", sections: [] },
      });
      assert.equal(state.selectedNumber, 7);
      assert.equal(state.detail?.number, 7);
      assert.equal(state.detailLoading, true);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: requestA,
        detail: { number: 7, title: "Latest Issue A", sections: [] },
      });
      assert.equal(state.selectedNumber, 7);
      assert.equal(state.detail?.title, "Latest Issue A");
      assert.equal(state.detailLoading, false);
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("refresh detail and error cannot overwrite a row selected after refresh began", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(() => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Initial", phase: "backlog" },
        { number: 42, title: "Chosen", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;

      surface.requestKnowledgeBridge("win-1", "issue", true);
      const refreshRequestId = sent.at(-1).request_id;
      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const detailRequestId = sent.at(-1).request_id;

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_entries",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: refreshRequestId,
        entries: state.baseEntries.slice(),
        selected_number: 7,
        empty_message: "",
        refresh_enabled: true,
      });
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: refreshRequestId,
        detail: { number: 7, title: "Stale refresh detail", sections: [] },
      });
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_error",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: refreshRequestId,
        message: "stale refresh error",
      });

      assert.equal(state.selectedNumber, 42);
      assert.notEqual(state.detail?.number, 7);
      assert.equal(state.detailLoading, true);
      assert.equal(state.error, "");

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: detailRequestId,
        detail: { number: 42, title: "Chosen detail", sections: [] },
      });
      assert.equal(state.detail?.number, 42);
      assert.equal(state.detailLoading, false);
      assert.equal(state.error, "");
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("late semantic search results cannot replace a row selected after search began", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(({ fire }) => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Search default", phase: "backlog" },
        { number: 42, title: "Explicit selection", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;
      state.detail = { number: 7, title: "Search default", sections: [] };
      state.query = "semantic query";

      surface.scheduleKnowledgeSearch("win-1", "issue");
      fire((timer) => timer.delay === 250);
      const searchRequestId = sent.at(-1).request_id;
      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const selectionRequestId = sent.at(-1).request_id;

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_search_results",
        id: "win-1",
        knowledge_kind: "issue",
        query: "semantic query",
        request_id: searchRequestId,
        entries: [{ number: 7, title: "Late search result", phase: "backlog" }],
        selected_number: 7,
        empty_message: "",
        refresh_enabled: true,
      });

      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detail?.number, 7);
      assert.equal(state.detailLoading, true);
      assert.equal(state.error, "");
      assert.equal(
        sent.filter((message) => message.kind === "select_knowledge_bridge_entry").length,
        1,
        "a late search response must not dispatch a replacement detail request",
      );

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: selectionRequestId,
        detail: { number: 42, title: "Explicit selection", sections: [] },
      });
      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detail?.number, 42);
      assert.equal(state.detailLoading, false);
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("late semantic search and initial-load errors preserve a newer row request", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(({ fire }) => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Initial", phase: "backlog" },
        { number: 42, title: "Chosen", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;
      state.query = "semantic query";

      surface.scheduleKnowledgeSearch("win-1", "issue");
      fire((timer) => timer.delay === 250);
      const searchRequestId = sent.at(-1).request_id;
      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const selectionRequestId = sent.at(-1).request_id;

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_error",
        id: "win-1",
        knowledge_kind: "issue",
        query: "semantic query",
        request_id: searchRequestId,
        message: "late semantic failure",
      });
      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detailLoading, true);
      assert.equal(state.error, "");

      state.query = "";
      surface.requestKnowledgeBridge("win-1", "issue", false);
      const loadRequestId = sent.at(-1).request_id;
      surface.requestKnowledgeDetail("win-1", "issue", 42);
      const latestSelectionRequestId = sent.at(-1).request_id;
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_error",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: loadRequestId,
        message: "late initial load failure",
      });
      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detailLoading, true);
      assert.equal(state.error, "");
      assert.notEqual(latestSelectionRequestId, selectionRequestId);
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("clearing and retyping keeps one semantic search process in flight", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { querySelector: () => null, getElementById: () => null };
  try {
    withPatchedTimers(({ fire }) => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Initial", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;

      state.query = "query A";
      surface.scheduleKnowledgeSearch("win-1", "issue");
      fire((timer) => timer.delay === 250);
      const first = sent.find(
        (message) => message.kind === "search_knowledge_bridge",
      );
      assert.ok(first);

      state.query = "";
      surface.scheduleKnowledgeSearch("win-1", "issue");
      assert.equal(
        state.inFlightSearchRequestId,
        first.request_id,
        "clear must retain ownership of the running search process",
      );

      state.query = "query B";
      surface.scheduleKnowledgeSearch("win-1", "issue");
      assert.equal(
        sent.filter((message) => message.kind === "search_knowledge_bridge").length,
        1,
        "retyping while A is running must queue B instead of starting a second process",
      );

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_search_results",
        id: "win-1",
        knowledge_kind: "issue",
        query: "query A",
        request_id: first.request_id,
        entries: [],
        selected_number: null,
        empty_message: "",
        refresh_enabled: true,
      });
      const searches = sent.filter(
        (message) => message.kind === "search_knowledge_bridge",
      );
      assert.equal(searches.length, 2);
      assert.equal(searches[1].query, "query B");
      assert.equal(state.inFlightSearchRequestId, searches[1].request_id);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_error",
        id: "win-1",
        knowledge_kind: "issue",
        query: "query A",
        request_id: first.request_id,
        message: "late A error",
      });
      assert.equal(
        state.inFlightSearchRequestId,
        searches[1].request_id,
        "a duplicate late A event must not settle B",
      );
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("a load that finishes before its superseded search replays the query once", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { querySelector: () => null, getElementById: () => null };
  try {
    withPatchedTimers(({ fire }) => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.baseEntries = [
        { number: 7, title: "Before refresh", phase: "backlog" },
      ];
      state.entries = state.baseEntries.slice();
      state.selectedNumber = 7;
      state.query = "query A";

      surface.scheduleKnowledgeSearch("win-1", "issue");
      fire((timer) => timer.delay === 250);
      const firstSearch = sent.find(
        (message) => message.kind === "search_knowledge_bridge",
      );
      assert.ok(firstSearch);

      surface.requestKnowledgeBridge("win-1", "issue", true);
      const load = sent.findLast(
        (message) => message.kind === "load_knowledge_bridge",
      );
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_entries",
        id: "win-1",
        knowledge_kind: "issue",
        request_id: load.request_id,
        entries: [{ number: 7, title: "After refresh", phase: "backlog" }],
        selected_number: 7,
        empty_message: "",
        refresh_enabled: true,
      });

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_search_results",
        id: "win-1",
        knowledge_kind: "issue",
        query: "query A",
        request_id: firstSearch.request_id,
        entries: [],
        selected_number: null,
        empty_message: "",
        refresh_enabled: true,
      });

      const searches = sent.filter(
        (message) => message.kind === "search_knowledge_bridge",
      );
      assert.equal(searches.length, 2);
      assert.equal(searches[1].query, "query A");
      assert.equal(state.inFlightSearchRequestId, searches[1].request_id);
      assert.equal(state.searching, true);
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("legacy request-id-less detail is accepted only for the selected Issue", async () => {
  const mod = await importSurfaceModule();
  const originalDocument = globalThis.document;
  globalThis.document = { getElementById: () => null };
  try {
    withPatchedTimers(() => {
      const surface = createSurface(mod, []);
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.selectedNumber = 42;
      state.detail = { number: 42, title: "Current", sections: [] };

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        detail: { number: 7, title: "Legacy stale", sections: [] },
      });
      assert.equal(state.detail?.number, 42);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_detail",
        id: "win-1",
        knowledge_kind: "issue",
        detail: { number: 42, title: "Legacy current", sections: [] },
      });
      assert.equal(state.selectedNumber, 42);
      assert.equal(state.detail?.title, "Legacy current");
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

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

// The knowledge_detail receive path consults the Kanban Drawer via
// document.getElementById; the functional harness runs without a page, so
// provide the minimal document surface when none exists.
if (typeof globalThis.document === "undefined") {
  globalThis.document = { getElementById: () => null };
}

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

function createSurface(mod, sent, preset = "issue") {
  const windowData = { id: "win-1", preset };
  return mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    createNode: () => ({ appendChild() {}, classList: { add() {} } }),
    createKnowledgeMarkdownBody: () => ({ appendChild() {} }),
    windowMap: new Map(),
    workspaceWindowById: (id) => (id === "win-1" ? windowData : null),
    getWorkspaceWindows: () => [windowData],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: (windowPreset) =>
      windowPreset === "issue" || windowPreset === "spec" ? windowPreset : null,
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
  });
}

// SPEC #3170 T-948/T-949 shared fixtures: two cached rows plus an
// authoritative detail for row A, driven through the public surface API
// exactly like the production WebSocket receive path.
const ROW_A = {
  number: 1,
  title: "Alpha issue",
  state: "open",
  labels: ["bug"],
  phase: "backlog",
};
const ROW_B = {
  number: 2,
  title: "Beta issue",
  state: "open",
  labels: ["gwt-spec"],
  phase: "draft",
};

function detailFor(row, body) {
  return {
    number: row.number,
    title: row.title,
    subtitle: "",
    state: row.state,
    labels: row.labels,
    sections: [{ title: "Description", body }],
    launch_issue_number: row.number,
    related_works: [],
  };
}

function seedListAndAuthoritativeA(surface, kind) {
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: kind,
    request_id: 0,
    entries: [ROW_A, ROW_B],
    selected_number: ROW_A.number,
    empty_message: "",
    refresh_enabled: true,
  });
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: kind,
    request_id: 0,
    detail: detailFor(ROW_A, "Alpha body"),
  });
  return surface.knowledgeBridgeStateMap.get("win-1");
}

for (const kind of ["issue", "spec"]) {
  test(`selecting row B renders the local preview synchronously (${kind} preset, AS-17.4)`, async () => {
    const mod = await importSurfaceModule();
    const sent = [];
    const surface = createSurface(mod, sent, kind);
    const state = seedListAndAuthoritativeA(surface, kind);
    assert.equal(state.detail.number, ROW_A.number);

    surface.requestKnowledgeDetail("win-1", kind, ROW_B.number);

    // Same click turn — before any backend response arrives.
    assert.equal(state.selectedNumber, ROW_B.number);
    assert.equal(state.detailLoading, true, "body shows Loading detail");
    assert.ok(state.detail, "a local selection preview must be materialized");
    assert.equal(
      state.detail.number,
      ROW_B.number,
      "preview identity must be row B within the click turn (FR-101)",
    );
    assert.equal(state.detail.title, ROW_B.title);
    assert.equal(state.detail.state, ROW_B.state);
    assert.deepEqual(state.detail.labels, ROW_B.labels);
    assert.equal(
      state.detail.launch_issue_number,
      ROW_B.number,
      "the Launch Agent action must target row B immediately",
    );
    assert.ok(
      !(state.detail.sections || []).some((section) =>
        String(section.body).includes("Alpha body"),
      ),
      "row A's body must disappear when B is selected (no mismatched frame)",
    );
    assert.equal(
      typeof state.selectionGeneration,
      "number",
      "an independent monotonically increasing selection generation is required (FR-101)",
    );
  });

  test(`re-selecting the authoritative row preserves its body (${kind} preset, AS-17.4)`, async () => {
    const mod = await importSurfaceModule();
    const sent = [];
    const surface = createSurface(mod, sent, kind);
    const state = seedListAndAuthoritativeA(surface, kind);

    surface.requestKnowledgeDetail("win-1", kind, ROW_A.number);

    assert.equal(state.selectedNumber, ROW_A.number);
    assert.ok(
      (state.detail.sections || []).some((section) =>
        String(section.body).includes("Alpha body"),
      ),
      "the already-authoritative body must be preserved while refreshing",
    );
  });
}

test("a late initial load cannot move a newer explicit selection (AS-17.5)", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeBridge("win-1", "issue", false);
  const staleLoadRequestId = sent.at(-1).request_id;
  surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);

  // The initial load completes late, still claiming row A as selected and
  // no longer containing row B.
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: staleLoadRequestId,
    entries: [ROW_A],
    selected_number: ROW_A.number,
    empty_message: "",
    refresh_enabled: true,
  });

  assert.equal(
    state.selectedNumber,
    ROW_B.number,
    "an initial-load completion may refresh rows but must not move the newer explicit selection (FR-101)",
  );
  assert.equal(
    state.detail?.number,
    ROW_B.number,
    "the selection preview identity must survive the late list response",
  );
});

test("a search completion cannot move a newer explicit selection (AS-17.5)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");

    state.query = "alpha";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    fire((timer) => timer.delay === 250);
    const searchRequestId = sent.at(-1).request_id;
    assert.equal(sent.at(-1).kind, "search_knowledge_bridge");

    // The user explicitly selects B while the search is in flight.
    surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);

    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_search_results",
      id: "win-1",
      knowledge_kind: "issue",
      query: "alpha",
      request_id: searchRequestId,
      entries: [ROW_A, ROW_B],
      selected_number: ROW_A.number,
      empty_message: "",
      refresh_enabled: true,
    });

    assert.equal(
      state.selectedNumber,
      ROW_B.number,
      "a search completion may refresh its rows but must not move the newer explicit selection (FR-101)",
    );
    assert.equal(state.detail?.number, ROW_B.number);
  });
});

test("A→B→A reverse-order detail completions leave the latest selection authoritative (AS-17.5)", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);
  const requestForB = sent.at(-1).request_id;
  surface.requestKnowledgeDetail("win-1", "issue", ROW_A.number);
  const requestForA = sent.at(-1).request_id;

  // Responses arrive in reverse order: A (latest) first, then stale B.
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: requestForA,
    detail: detailFor(ROW_A, "Fresh alpha body"),
  });
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: requestForB,
    detail: detailFor(ROW_B, "Stale beta body"),
  });

  assert.equal(state.selectedNumber, ROW_A.number);
  assert.equal(
    state.detail?.number,
    ROW_A.number,
    "a stale earlier-selection completion must not overwrite the latest one (FR-101)",
  );
  assert.equal(state.detailLoading, false);
});

test("a request-ID-less legacy detail is fenced while a newer selection is pending (AS-17.5)", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);

  // A legacy backend replays row A's detail without a request id while the
  // newer explicit selection of B is still pending.
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: "issue",
    detail: detailFor(ROW_A, "Legacy alpha body"),
  });

  assert.equal(
    state.selectedNumber,
    ROW_B.number,
    "a request-ID-less legacy detail is accepted only for the current number when no newer selection is pending",
  );
  assert.equal(state.detail?.number, ROW_B.number);
});

function withPatchedTimersAsync(run) {
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
  const finish = () => {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
  };
  return Promise.resolve(run({ timers, fire })).finally(finish);
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

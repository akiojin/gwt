// Issues #3297 / #5413 — cache-backed knowledge load recovery contract.
// Slow reads must not become duplicate requests or premature load errors;
// successful late responses must recover the window.
//
// The surface module imports "/focus-trap.js" (a browser absolute path),
// so the tests load the module source with that import stubbed and drive
// the real factory functions.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { parseHTML } from "linkedom";

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
  ).replace(
    'from "./launch-pending-controller.js"',
    'from "data:text/javascript,export function createLaunchOperationId(){return%20%22resume-test%22}"',
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source.replace('from "./control-explanations.js"', `from "${new URL("../control-explanations.js", import.meta.url).href}"`).replace('from "./ui-state-store.js"', `from "${new URL("../ui-state-store.js", import.meta.url).href}"`)).toString("base64")}`
  );
}

function createSurface(mod, sent, preset = "issue", options = {}) {
  const windowData = options.windowData || { id: "win-1", preset };
  const directSend = options.sendKnowledgeSemanticSearchNow
    || ((message) => {
      sent.push(message);
      return true;
    });
  return mod.createKnowledgeKanbanSurface({
    send: options.send || ((message) => sent.push(message)),
    sendKnowledgeSemanticSearchNow: directSend,
    createNode: options.createNode || (() => ({ appendChild() {}, classList: { add() {} } })),
    createKnowledgeMarkdownBody:
      options.createKnowledgeMarkdownBody || (() => ({ appendChild() {} })),
    windowMap: options.windowMap || new Map(),
    workspaceWindowById: options.workspaceWindowById || ((id) => (id === "win-1" ? windowData : null)),
    getWorkspaceWindows: () => [windowData],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset:
      options.knowledgeKindForPreset ||
      ((windowPreset) =>
        windowPreset === "issue" || windowPreset === "spec" ? "issue" : null),
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
  surface.ensureKnowledgeBridgeState("win-1", kind);
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

test("Issue #5396: semantic list completion does not request an implicit preview", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    const before = sent.length;
    surface.applyKnowledgeReceiveEvent(searchResultsEvent(requestId));
    assert.equal(sent.slice(before).filter(message => message.kind === "select_knowledge_bridge_entry").length, 0);
  });
});

test("Issue #5543: related-work enrichment refreshes only the explicitly selected detail", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async () => {
    const sent = [];
    const surface = createSurface(mod, sent);
    surface.requestKnowledgeBridge("win-1", "issue", false);
    const listEvent = {
      kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
      request_id: sent.at(-1).request_id, entries: [ROW_A, ROW_B],
      selected_number: null, refresh_enabled: true,
    };
    const detailRequests = () => sent.filter(message => message.kind === "select_knowledge_bridge_entry");
    surface.applyKnowledgeReceiveEvent(listEvent);
    assert.equal(detailRequests().length, 0, "initial rows must not open a detail");

    surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);
    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_detail", id: "win-1", knowledge_kind: "issue",
      request_id: detailRequests().at(-1).request_id,
      detail: detailFor(ROW_B, "Beta body"),
    });
    const state = surface.knowledgeBridgeStateMap.get("win-1");
    const selectionGeneration = state.selectionGeneration;
    const enriched = {
      ...listEvent,
      entries: [ROW_A, { ...ROW_B, related_work_refs: [{ id: "work-beta" }],
        related_work_count: 1, related_session_count: 1 }],
    };
    surface.applyKnowledgeReceiveEvent(enriched);
    assert.equal(detailRequests().length, 2, "enrichment must requery the selected detail once");
    assert.equal(detailRequests().at(-1).number, ROW_B.number);
    assert.equal(state.selectedNumber, ROW_B.number);
    assert.equal(state.selectionGeneration, selectionGeneration, "refresh is not an explicit selection");
    surface.applyKnowledgeReceiveEvent(enriched);
    assert.equal(detailRequests().length, 2, "the same related snapshot must not requery again");

    const relatedWorks = [{ id: "work-beta", title: "Beta work" }];
    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_detail", id: "win-1", knowledge_kind: "issue",
      request_id: detailRequests().at(-1).request_id,
      detail: { ...detailFor(ROW_B, "Beta body"), related_works: relatedWorks },
    });
    assert.deepEqual(state.detail.related_works, relatedWorks);
  });
});

for (const kind of ["issue", "spec"]) {
  test(`selecting row B renders the local preview synchronously (${kind} preset, AS-17.4)`, async () => {
    const mod = await importSurfaceModule();
    const sent = [];
    const surface = createSurface(mod, sent, kind);
    const state = seedListAndAuthoritativeA(surface, "issue");
    assert.equal(state.detail.number, ROW_A.number);

    surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);

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
    assert.equal(state.detail.subtitle, `#${ROW_B.number}`);
    assert.equal(state.detail.state, ROW_B.state);
    assert.equal(state.detail.phase, ROW_B.phase);
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
    const state = seedListAndAuthoritativeA(surface, "issue");

    surface.requestKnowledgeDetail("win-1", "issue", ROW_A.number);

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

test("a request-ID-less detail is rejected after any explicit selection, even for the current number", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeDetail("win-1", "issue", ROW_A.number);
  const preserved = state.detail;
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: "issue",
    detail: detailFor(ROW_A, "Uncorrelated replacement"),
  });

  assert.equal(state.detail, preserved);
  assert.ok(
    state.detail.sections.some((section) => section.body === "Alpha body"),
    "selectionGeneration > 0 must reject every ID-less detail",
  );
});

test("a refresh does not invalidate the current explicit detail request", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);
  const detailRequestId = sent.at(-1).request_id;
  surface.requestKnowledgeBridge("win-1", "issue", true);
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_detail",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: detailRequestId,
    detail: detailFor(ROW_B, "Fresh beta body"),
  });

  assert.equal(state.detail?.number, ROW_B.number);
  assert.ok(state.detail.sections.some((section) => section.body === "Fresh beta body"));
});

test("a late load error cannot surface after a newer explicit selection", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeBridge("win-1", "issue", true);
  const loadRequestId = sent.at(-1).request_id;
  surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_error",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: loadRequestId,
    message: "stale refresh error",
  });

  assert.equal(state.selectedNumber, ROW_B.number);
  assert.equal(state.detail?.number, ROW_B.number);
  assert.equal(state.error, "", "a prior selection generation cannot surface its error");
});

// ---------------------------------------------------------------------------
// SPEC #3170 T-951/T-952 — silent indefinite semantic retry + lifecycle +
// local fallback. The retry window is owned by the frontend: fixed delays
// 5s, 10s, 20s, 30s, then 30s indefinitely (FR-099); every degradation is
// invisible (FR-100); offline retries never enter the generic pending queue.
// ---------------------------------------------------------------------------

function searchResultsEvent(requestId, overrides = {}) {
  return {
    kind: "knowledge_search_results",
    id: "win-1",
    knowledge_kind: "issue",
    query: "alpha",
    request_id: requestId,
    entries: [ROW_A],
    selected_number: null,
    empty_message: "",
    refresh_enabled: true,
    ...overrides,
  };
}

const TRANSIENT_RETRY = {
  error_code: "INDEX_NOT_READY",
  retryable: true,
  retry_after_ms: 5000,
};

function startSearch(surface, sent, { fire }) {
  const state = surface.knowledgeBridgeStateMap.get("win-1");
  state.query = "alpha";
  surface.scheduleKnowledgeSearch("win-1", "issue");
  fire((timer) => timer.delay === 250);
  const message = sent.at(-1);
  assert.equal(message.kind, "search_knowledge_bridge");
  return message.request_id;
}

test("typed transient failure schedules the silent 5/10/20/30/30 retry ladder (T-951)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    let requestId = startSearch(surface, sent, { fire });

    const expectedDelays = [5000, 10000, 20000, 30000, 30000, 30000];
    for (const delay of expectedDelays) {
      surface.applyKnowledgeReceiveEvent(
        searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
      );
      assert.equal(
        state.error,
        "",
        "typed transient semantic failure must stay invisible (FR-100)",
      );
      assert.equal(
        state.entries.length,
        1,
        "cache-backed rows from the completion stay usable",
      );
      const pending = timers.filter(
        (timer) => !timer.cleared && !timer.fired && timer.delay === delay,
      );
      assert.equal(
        pending.length,
        1,
        `exactly one silent retry must be scheduled at ${delay}ms (FR-099)`,
      );
      const sentBefore = sent.length;
      fire((timer) => timer.delay === delay);
      assert.equal(
        sent.length,
        sentBefore + 1,
        "a retry fires exactly one new attempt (one in-flight attempt)",
      );
      const retryMessage = sent.at(-1);
      assert.equal(retryMessage.kind, "search_knowledge_bridge");
      assert.equal(retryMessage.query, "alpha");
      requestId = retryMessage.request_id;
    }
  });
});

test("a retry success resets the ladder back to 5 seconds (T-951)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    seedListAndAuthoritativeA(surface, "issue");
    let requestId = startSearch(surface, sent, { fire });

    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );
    fire((timer) => timer.delay === 5000);
    requestId = sent.at(-1).request_id;
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );
    fire((timer) => timer.delay === 10000);
    requestId = sent.at(-1).request_id;

    // Success: no directive. The ladder resets and no retry stays pending.
    surface.applyKnowledgeReceiveEvent(searchResultsEvent(requestId));
    assert.equal(
      timers.filter(
        (timer) =>
          !timer.cleared &&
          !timer.fired &&
          [5000, 10000, 20000, 30000].includes(timer.delay),
      ).length,
      0,
      "success must cancel the retry window (FR-099)",
    );

    // The next transient failure starts again at 5 seconds.
    const nextSearchId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(nextSearchId, { semantic_retry: TRANSIENT_RETRY }),
    );
    assert.equal(
      timers.filter(
        (timer) => !timer.cleared && !timer.fired && timer.delay === 5000,
      ).length,
      1,
      "after a success the ladder must restart at 5 seconds",
    );
  });
});

test("a search-correlated legacy error is silent degradation without retry (T-951, AS-17.3)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    const detailBefore = state.detail;

    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_error",
      id: "win-1",
      knowledge_kind: "issue",
      request_id: requestId,
      query: "alpha",
      message: "semantic search runner exited with 1: raw diagnostic",
    });

    assert.equal(
      state.error,
      "",
      "a search-correlated legacy error is semantic degradation and must be hidden (FR-100)",
    );
    assert.equal(state.searchInFlight, false, "in-flight ownership released");
    assert.deepEqual(
      state.baseEntries.map((entry) => entry.number),
      [ROW_A.number, ROW_B.number],
      "cache rows survive silent degradation",
    );
    assert.equal(state.detail, detailBefore, "current detail is preserved");
    assert.equal(
      timers.filter(
        (timer) =>
          !timer.cleared &&
          !timer.fired &&
          [5000, 10000, 20000, 30000].includes(timer.delay),
      ).length,
      0,
      "an untyped legacy failure must not start the indefinite retry (FR-100)",
    );
  });
});

test("a typed non-semantic search error remains visible (T-951 guard)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });

    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_error",
      id: "win-1",
      knowledge_kind: "issue",
      request_id: requestId,
      query: "alpha",
      error_domain: "non_semantic",
      message: "failed to read issue cache",
    });

    assert.equal(
      state.error,
      "failed to read issue cache",
      "cache and window failures must keep the visible error channel",
    );
    assert.equal(state.searchInFlight, false, "in-flight ownership is released");
    assert.equal(
      timers.filter(
        (timer) =>
          !timer.cleared &&
          !timer.fired &&
          [5000, 10000, 20000, 30000].includes(timer.delay),
      ).length,
      0,
      "non-semantic failures must not start the semantic retry ladder",
    );
  });
});

test("non-semantic knowledge errors remain visible (T-951 guard)", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  const state = seedListAndAuthoritativeA(surface, "issue");

  surface.requestKnowledgeBridge("win-1", "issue", false);
  const loadRequestId = sent.at(-1).request_id;
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_error",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: loadRequestId,
    message: "failed to read issue cache",
  });

  assert.equal(
    state.error,
    "failed to read issue cache",
    "cache/load failures keep their existing visible error channel",
  );
});

test("row selection does not cancel or reset the retry window (T-951, AS-17.2)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );
    const retryOrdinal = state.semanticRetryIndex;

    surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);

    assert.equal(
      state.semanticRetryIndex,
      retryOrdinal,
      "selecting a row must preserve the typed retry ordinal",
    );
    assert.equal(state.semanticRetryActive, true);
    assert.equal(
      timers.filter(
        (timer) => !timer.cleared && !timer.fired && timer.delay === 5000,
      ).length,
      1,
      "selecting a row must leave the semantic retry window untouched",
    );
  });
});

test("query change and clear invalidate the pending retry (T-952, AS-17.2)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );

    state.query = "beta";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    const staleRetry = timers.find(
      (timer) => timer.delay === 5000 && !timer.fired,
    );
    assert.ok(
      staleRetry.cleared,
      "changing the query must cancel the previous retry timer",
    );

    fire((timer) => timer.delay === 250);
    const secondRequestId = sent.at(-1).request_id;
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(secondRequestId, {
        query: "beta",
        semantic_retry: TRANSIENT_RETRY,
      }),
    );
    state.query = "";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    assert.equal(
      timers.filter(
        (timer) =>
          !timer.cleared &&
          !timer.fired &&
          [5000, 10000, 20000, 30000].includes(timer.delay),
      ).length,
      0,
      "clearing the query must cancel the retry window",
    );
  });
});

test("SEARCH_UNAVAILABLE is the second exact typed transient code", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, {
        semantic_retry: {
          error_code: "SEARCH_UNAVAILABLE",
          retryable: true,
          retry_after_ms: 5000,
        },
      }),
    );
    assert.equal(
      timers.filter((timer) => !timer.cleared && !timer.fired && timer.delay === 5000).length,
      1,
    );
  });
});

test("auto refresh gates revision checks on visibility and semantic owners (#5394, T-951)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ intervals }) => {
    const originalDocument = globalThis.document;
    const { document: domDocument } = parseHTML("<html><body></body></html>");
    globalThis.document = domDocument;
    try {
      const sent = [];
      const body = domDocument.createElement("div");
      domDocument.body.appendChild(body);
      const windowMap = new Map([["win-1", body]]);
      const windowData = { id: "win-1", preset: "issue" };
      let activeTab = true;
      const createNode = (tag, className, textContent = "") => {
        const node = domDocument.createElement(tag);
        node.className = className || "";
        node.textContent = textContent;
        return node;
      };
      const surface = createSurface(mod, sent, "issue", {
        windowMap,
        windowData,
        workspaceWindowById: () => activeTab ? windowData : null,
        createNode,
        createKnowledgeMarkdownBody: (value) =>
          createNode("div", "knowledge-markdown", String(value || "")),
      });
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      state.entries = [ROW_A, ROW_B];
      state.baseEntries = [ROW_A, ROW_B];
      state.selectedNumber = ROW_A.number;
      state.detail = detailFor(ROW_A, "Alpha body");
      state.refreshEnabled = true;
      state.cacheRevision = "revision-1";

      surface.mountKnowledgeWindow({ id: "win-1", preset: "issue" }, body);
      assert.equal(intervals.length, 1, "mount must install one auto-refresh owner");
      assert.equal(
        sent.filter((message) => message.kind === "load_knowledge_bridge").length,
        0,
        "the seeded surface must not need an initial knowledge load",
      );
      assert.equal(
        sent.filter((message) => message.kind === "list_issue_monitor").length,
        1,
        "the unified Issue surface must hydrate its monitor controls",
      );

      const ownerCases = [
        ["semanticRetryTimer", 991],
        ["pendingSearchTimer", 992],
        ["inFlightSearchIntent", { generation: 1, kind: "issue", query: "alpha" }],
        ["queuedSearchIntent", { generation: 1, kind: "issue", query: "alpha" }],
        ["semanticRetryActive", true],
      ];
      for (const [field, value] of ownerCases) {
        state[field] = value;
        intervals[0].callback();
        assert.equal(
          sent.filter((message) => message.kind === "load_knowledge_bridge").length,
          0,
          `auto refresh must not interrupt ${field}`,
        );
        state[field] = field === "semanticRetryActive" ? false : null;
      }

      intervals[0].callback();
      assert.equal(sent.filter(message => message.kind === "check_knowledge_revision").length, 1);
      intervals[0].callback();
      intervals[0].callback();
      assert.equal(sent.filter(message => message.kind === "check_knowledge_revision").length, 1,
        "a slow revision probe must not be duplicated by subsequent ticks");
      surface.applyKnowledgeReceiveEvent({ kind: "knowledge_revision", id: "win-1",
        knowledge_kind: "issue", revision: "revision-1" });
      assert.equal(sent.filter(message => message.kind === "load_knowledge_bridge").length, 0,
        "unchanged cache must not load or rebuild the list");

      // Repeated hidden ticks coalesce to one revision check on activation.
      const hiddenCases = [
        [() => { body.hidden = true; }, () => { body.hidden = false; }],
        [() => { windowData.minimized = true; }, () => { windowData.minimized = false; }],
        [() => { windowData.tab_group_id = "group"; windowData.tab_group_active = false; },
          () => { windowData.tab_group_active = true; }],
        [() => { activeTab = false; }, () => { activeTab = true; }],
        [() => { domDocument.hidden = true; }, () => { domDocument.hidden = false; }],
      ];
      for (const [hide, reveal] of hiddenCases) {
        const before = sent.length;
        hide();
        intervals[0].callback();
        intervals[0].callback();
        assert.equal(sent.length, before, "hidden/minimized/inactive surfaces do no reads");
        reveal();
        surface.refreshVisibleKnowledgeWindows();
        surface.refreshVisibleKnowledgeWindows();
        assert.equal(sent.length, before + 1, "activation checks only once");
        surface.applyKnowledgeReceiveEvent({ kind: "knowledge_revision", id: "win-1",
          knowledge_kind: "issue", revision: "revision-1" });
      }
      intervals[0].callback();
      // A probe that completes after hiding must defer its changed revision.
      body.hidden = true;
      surface.applyKnowledgeReceiveEvent({ kind: "knowledge_revision", id: "win-1",
        knowledge_kind: "issue", revision: "revision-2" });
      assert.equal(sent.filter(message => message.kind === "load_knowledge_bridge").length, 0);
      body.hidden = false;
      surface.refreshVisibleKnowledgeWindows();
      surface.applyKnowledgeReceiveEvent({ kind: "knowledge_revision", id: "win-1",
        knowledge_kind: "issue", revision: "revision-2" });
      assert.equal(
        sent.filter((message) => message.kind === "load_knowledge_bridge").length,
        1,
        "a changed revision loads once after visibility and semantic owners release",
      );
      // A concurrent list response may already contain the probed change.
      state.loading = false;
      state.refreshing = false;
      intervals[0].callback();
      state.loading = true;
      surface.applyKnowledgeCacheRevision({ kind: "knowledge_revision", id: "win-1",
        knowledge_kind: "issue", revision: "revision-3" });
      surface.applyKnowledgeReceiveEvent({ kind: "knowledge_entries", id: "win-1",
        knowledge_kind: "issue", entries: [ROW_A, ROW_B], cache_revision: "revision-3",
        refresh_enabled: true });
      surface.refreshVisibleKnowledgeWindows();
      assert.equal(sent.filter(message => message.kind === "load_knowledge_bridge").length, 1,
        "a matching concurrent list response consumes the pending revision change");
      surface.clearKnowledgeBridgeState("win-1");
    } finally {
      globalThis.document = originalDocument;
    }
  });
});

test("A→B→A ignores the first A completion and dispatches only the latest intent", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");

    state.query = "alpha";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    fire((timer) => timer.delay === 250);
    const firstA = sent.at(-1);

    state.query = "beta";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    state.query = "alpha";
    surface.scheduleKnowledgeSearch("win-1", "issue");

    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(firstA.request_id, {
        entries: [{ ...ROW_A, title: "Stale alpha" }],
      }),
    );

    assert.notEqual(
      state.entries[0]?.title,
      "Stale alpha",
      "matching request id and query are insufficient after A→B→A",
    );
    assert.equal(sent.length > 0, true);
    const latestA = sent.at(-1);
    assert.notEqual(latestA.request_id, firstA.request_id);
    assert.equal(latestA.query, "alpha");
    assert.equal(state.searchInFlight, true);
  });
});

test("a forced callback from a cancelled retry generation is inert", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire, forceFire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );
    const staleTimer = timers.find((timer) => timer.delay === 5000);

    state.query = "beta";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    const sentBefore = sent.length;
    forceFire(staleTimer);

    assert.equal(sent.length, sentBefore, "a cancelled callback must fail its generation fence");
  });
});

test("changing knowledge kind invalidates an Issue retry owner", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire, forceFire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );
    const staleTimer = timers.find((timer) => timer.delay === 5000);

    surface.ensureKnowledgeBridgeState("win-1", "spec");
    const sentBefore = sent.length;
    forceFire(staleTimer);

    assert.equal(sent.length, sentBefore);
    assert.equal(state.kind, "spec");
  });
});

test("window destroy invalidates the retry window (T-952, AS-17.2)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );

    surface.clearKnowledgeBridgeState("win-1");

    assert.equal(
      timers.filter(
        (timer) => !timer.cleared && !timer.fired && timer.delay === 5000,
      ).length,
      0,
      "destroying the window must cancel its retry timer",
    );
  });
});

test("a late receive event cannot resurrect a destroyed Knowledge window", async () => {
  const mod = await importSurfaceModule();
  const sent = [];
  const surface = createSurface(mod, sent);
  seedListAndAuthoritativeA(surface, "issue");
  surface.clearKnowledgeBridgeState("win-1");

  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    entries: [ROW_B],
    selected_number: ROW_B.number,
    refresh_enabled: true,
  });

  assert.equal(surface.knowledgeBridgeStateMap.has("win-1"), false);
});

test("a prior live window response cannot contaminate a remounted window with the same id", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async () => {
    const sent = [];
    const surface = createSurface(mod, sent);
    surface.requestKnowledgeBridge("win-1", "issue", false);
    const oldRequestId = sent.at(-1).request_id;
    surface.clearKnowledgeBridgeState("win-1");
    surface.requestKnowledgeBridge("win-1", "issue", false);
    const remounted = surface.knowledgeBridgeStateMap.get("win-1");

    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_entries",
      id: "win-1",
      knowledge_kind: "issue",
      request_id: oldRequestId,
      entries: [ROW_A],
      selected_number: ROW_A.number,
      refresh_enabled: true,
    });

    assert.deepEqual(remounted.baseEntries, []);
    assert.equal(remounted.loading, true);
  });
});

test("an OPEN-to-closed send race never falls back to generic pending send", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ fire }) => {
    const pending = [];
    const directAttempts = [];
    const surface = createSurface(mod, pending, "issue", {
      send: (message) => pending.push(message),
      sendKnowledgeSemanticSearchNow: (message) => {
        directAttempts.push(message);
        return false;
      },
    });
    const state = seedListAndAuthoritativeA(surface, "issue");
    pending.length = 0;

    state.query = "alpha";
    surface.scheduleKnowledgeSearch("win-1", "issue");
    fire((timer) => timer.delay === 250);

    assert.equal(directAttempts.length, 1, "the semantic path attempted one atomic direct send");
    assert.equal(pending.length, 0, "failed direct send must not enter the generic pending queue");
    assert.equal(state.searchInFlight, false);
    assert.equal(state.semanticRetryActive, true, "reconnect owns the unsent live intent");
  });
});

test("offline retries never enter the generic pending queue; reconnect restarts at 5s (T-952)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const online = { value: true };
    const windowData = { id: "win-1", preset: "issue" };
    const surface = mod.createKnowledgeKanbanSurface({
      send: (message) => sent.push(message),
      sendKnowledgeSemanticSearchNow: (message) => {
        if (!online.value) return false;
        sent.push(message);
        return true;
      },
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
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, { semantic_retry: TRANSIENT_RETRY }),
    );

    // Transport drops before the retry fires: the attempt must be skipped
    // entirely instead of accumulating in the offline pending queue.
    online.value = false;
    assert.equal(
      typeof surface.handleKnowledgeTransportChange,
      "function",
      "the surface must expose a transport-change hook (FR-099)",
    );
    surface.handleKnowledgeTransportChange(false);
    const sentWhileOffline = sent.length;
    const retryTimer = timers.find(
      (timer) => timer.delay === 5000 && !timer.fired,
    );
    if (retryTimer && !retryTimer.cleared) {
      retryTimer.fired = true;
      retryTimer.callback();
    }
    assert.equal(
      sent.length,
      sentWhileOffline,
      "an offline retry must not send (zero pending-message accumulation)",
    );

    // Reconnect with the same open window/query: fresh ladder from 5s.
    online.value = true;
    surface.handleKnowledgeTransportChange(true);
    assert.equal(
      timers.filter(
        (timer) => !timer.cleared && !timer.fired && timer.delay === 5000,
      ).length,
      1,
      "reconnect must restart the retry sequence at 5 seconds (AS-17.2)",
    );
    assert.equal(state.error, "", "reconnect handling stays silent");
  });
});

test("disconnect while a request is in flight restarts the same live query at 5s", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    startSearch(surface, sent, { fire });
    assert.equal(state.searchInFlight, true);

    surface.handleKnowledgeTransportChange(false);
    surface.handleKnowledgeTransportChange(true);

    assert.equal(state.searchInFlight, false);
    assert.equal(
      timers.filter((timer) => !timer.cleared && !timer.fired && timer.delay === 5000).length,
      1,
    );
  });
});

test("disconnect after a completed search does not create a semantic retry window", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");
    const requestId = startSearch(surface, sent, { fire });
    surface.applyKnowledgeReceiveEvent(searchResultsEvent(requestId));
    assert.equal(state.searchInFlight, false);
    assert.equal(state.semanticRetryActive, false);

    surface.handleKnowledgeTransportChange(false);
    surface.handleKnowledgeTransportChange(true);

    assert.equal(
      timers.filter((timer) => !timer.cleared && !timer.fired && timer.delay === 5000).length,
      0,
      "a successful idle query must not become a reconnect retry owner",
    );
    assert.equal(sent.length, 1, "reconnect must not replay a completed query");
  });
});

for (const semanticRetry of [
  { error_code: "SEARCH_FAILED", retryable: false },
  { error_code: "SEARCH_FAILED", retryable: true },
  { error_code: "INDEX_NOT_READY", retryable: false },
  { error_code: "FUTURE_CODE", retryable: true },
  { error_code: "INDEX_NOT_READY", retryable: true },
  { error_code: "INDEX_NOT_READY", retryable: true, retry_after_ms: 0 },
  { error_code: "INDEX_NOT_READY", retryable: true, retry_after_ms: 30000 },
  { error_code: "INDEX_NOT_READY", retryable: true, retry_after_ms: "5000" },
  {
    error_code: "INDEX_NOT_READY",
    retryable: true,
    retry_after_ms: 5000,
    reason: "must stay backend-only",
  },
  { retryable: true },
  "INDEX_NOT_READY",
]) {
  test(`non-transient or malformed retry directive stays silent without retry: ${JSON.stringify(semanticRetry)}`, async () => {
    const mod = await importSurfaceModule();
    await withPatchedTimersAsync(async ({ timers, fire }) => {
      const sent = [];
      const surface = createSurface(mod, sent);
      const state = seedListAndAuthoritativeA(surface, "issue");
      const requestId = startSearch(surface, sent, { fire });
      surface.applyKnowledgeReceiveEvent(
        searchResultsEvent(requestId, { semantic_retry: semanticRetry }),
      );

      assert.equal(state.error, "");
      assert.equal(
        timers.filter((timer) => !timer.cleared && !timer.fired && timer.delay >= 5000).length,
        0,
      );
    });
  });
}

test("Issue #4174: retired PR knowledge kind does not mount or request data", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async () => {
    const { document: domDocument } = parseHTML("<html><body></body></html>");
    const body = domDocument.createElement("div");
    const sent = [];
    const surface = createSurface(mod, sent, "pr", {
      knowledgeKindForPreset: () => "pr",
    });
    try {
      surface.mountKnowledgeWindow({ id: "win-1", preset: "pr" }, body);
      assert.equal(body.childElementCount, 0, "retired PR GUI must not render a list or detail");
      assert.deepEqual(sent, [], "retired PR GUI must not load, search or select");
      assert.equal(surface.knowledgeBridgeStateMap.has("win-1"), false);
    } finally {
      surface.clearKnowledgeBridgeState("win-1");
    }
  });
});

test("a new query filters baseEntries locally before semantic results arrive (T-952, AS-17.7)", async () => {
  const mod = await importSurfaceModule();
  await withPatchedTimersAsync(async ({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    const state = seedListAndAuthoritativeA(surface, "issue");

    state.query = "beta";
    surface.scheduleKnowledgeSearch("win-1", "issue");

    // Before the debounce fires and before any semantic response, the
    // visible rows must already be the local filter of baseEntries.
    assert.deepEqual(
      state.entries.map((entry) => entry.number),
      [ROW_B.number],
      "local number/title/label filtering must be immediate (AS-17.7)",
    );

    // Semantic completion then provides the authoritative rows.
    fire((timer) => timer.delay === 250);
    const requestId = sent.at(-1).request_id;
    surface.applyKnowledgeReceiveEvent(
      searchResultsEvent(requestId, {
        query: "beta",
        entries: [ROW_B, ROW_A],
      }),
    );
    assert.deepEqual(
      state.entries.map((entry) => entry.number),
      [ROW_B.number, ROW_A.number],
      "authoritative semantic rows must not be substring-filtered again",
    );
  });
});

test("unchanged entry revisions retain rows while completing loads and preserving selection/search", async () => {
  const mod = await importSurfaceModule();
  const { document } = parseHTML(`
    <section id="knowledge-window"><div class="issue-bridge-root">
      <button data-action="refresh-knowledge"></button>
      <input class="knowledge-search" />
      <div class="knowledge-status"></div><div class="knowledge-list"></div>
      <div class="knowledge-detail-pane"></div>
    </div></section>
  `);
  const originalDocument = globalThis.document;
  globalThis.document = document;
  try {
    await withPatchedTimersAsync(async ({ fire }) => {
      const element = document.getElementById("knowledge-window");
      const sent = [];
      const createNode = (tag, className = "", text = "") => {
        const node = document.createElement(tag);
        node.className = className;
        node.textContent = text;
        return node;
      };
      const surface = createSurface(mod, sent, "issue", {
        windowMap: new Map([["win-1", element]]), createNode,
        createKnowledgeMarkdownBody: () => document.createElement("div"),
      });
      const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
      const receive = (revision, entries = [ROW_A, ROW_B]) => surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
        request_id: state.loadRequestId, revision, entries,
        selected_number: ROW_A.number, refresh_enabled: true,
      });
      surface.requestKnowledgeBridge("win-1", "issue");
      receive("rows-v1");
      surface.requestKnowledgeDetail("win-1", "issue", ROW_B.number);
      const originalRow = element.querySelector('[data-issue-number="2"]');
      const refresh = element.querySelector('[data-action="refresh-knowledge"]');
      refresh.disabled = true;
      surface.requestKnowledgeBridge("win-1", "issue");
      receive("rows-v1");
      assert.ok(element.querySelector('[data-issue-number="2"]') === originalRow,
        "same revision must preserve list DOM nodes");
      assert.equal(state.loading, false);
      assert.equal(state.refreshing, false);
      assert.equal(state.loadRecoveryTimer, null);
      assert.equal(refresh.disabled, false, "load completion must re-enable refresh");
      assert.equal(state.selectedNumber, ROW_B.number);
      assert.equal(element.querySelector(".knowledge-detail-title").textContent, ROW_B.title);
      assert.equal(originalRow.classList.contains("selected"), true);

      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_error", id: "win-1", knowledge_kind: "issue",
        request_id: state.loadRequestId, message: "Temporary load error",
      });
      surface.requestKnowledgeBridge("win-1", "issue");
      receive("rows-v1");
      assert.ok(!element.querySelector(".knowledge-list").textContent.includes("Issue load failed"),
        "a later successful response must clear the rendered load error");

      receive("rows-v2", [ROW_A, { ...ROW_B, title: "Updated beta" }]);
      assert.ok(element.querySelector('[data-issue-number="2"]') !== originalRow);
      assert.ok(element.querySelector(".knowledge-list").textContent.includes("Updated beta"));
      const updatedRow = element.querySelector('[data-issue-number="2"]');
      receive(undefined, [ROW_A, { ...ROW_B, title: "Legacy beta" }]);
      assert.ok(element.querySelector('[data-issue-number="2"]') !== updatedRow,
        "changed legacy entries still update their row");

      receive("rows-v2");
      state.query = "Alpha";
      surface.scheduleKnowledgeSearch("win-1", "issue");
      const filteredRow = element.querySelector('[data-issue-number="1"]');
      assert.ok(filteredRow);
      surface.requestKnowledgeBridge("win-1", "issue");
      receive("rows-v2");
      assert.ok(element.querySelector('[data-issue-number="1"]') === filteredRow,
        "same revision must retain filtered list nodes while resuming a cancelled search timer");
      assert.equal(state.query, "Alpha");
      assert.deepEqual(state.entries.map(entry => entry.number), [ROW_A.number]);
      fire(timer => timer.delay === 250);
      assert.equal(sent.at(-1).kind, "search_knowledge_bridge",
        "a list completion must still dispatch the queued query");
      assert.equal(sent.at(-1).query, "Alpha");
      assert.ok(element.querySelector('[data-issue-number="1"]') === filteredRow,
        "resuming and dispatching the query must preserve the filtered list nodes");
      assert.equal(state.selectedNumber, ROW_B.number);
      surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_search_results", id: "win-1", knowledge_kind: "issue",
        query: "Alpha", request_id: sent.at(-1).request_id,
        entries: [ROW_B, ROW_A], refresh_enabled: true,
      });
      const semanticRow = element.querySelector('[data-issue-number="2"]');
      assert.ok(semanticRow);
      surface.requestKnowledgeBridge("win-1", "issue");
      receive("rows-v2");
      assert.deepEqual(state.entries.map(entry => entry.number), [ROW_B.number, ROW_A.number],
        "same base revision must preserve authoritative semantic results");
      assert.ok(element.querySelector('[data-issue-number="2"]') === semanticRow,
        "same revision must retain semantic result nodes");
      receive("rows-v2");
      assert.ok(element.querySelector('[data-issue-number="2"]') === semanticRow,
        "an already reserved search must not redraw the list");
      fire(timer => timer.delay === 250);
      receive("rows-v2");
      assert.ok(element.querySelector('[data-issue-number="2"]') === semanticRow,
        "an in-flight search must not redraw the list");
      assert.equal(state.queuedSearchIntent, null, "the in-flight search must not be duplicated");
      surface.clearKnowledgeBridgeState("win-1");
    });
  } finally {
    globalThis.document = originalDocument;
  }
});

test("virtualized 500-row selection keeps mounted row identity and updates the right pane within p95 50ms", async () => {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML(`
    <section id="knowledge-window">
      <div class="issue-bridge-root">
      <button data-action="refresh-knowledge"></button>
      <input class="knowledge-search" />
      <button data-issue-filter="open"></button>
      <button data-issue-filter="closed"></button>
      <button data-issue-filter="all"></button>
      <div class="knowledge-status"></div>
      <div class="knowledge-list"></div>
      <div class="knowledge-detail-pane"></div>
      </div>
    </section>
  `);
  const originalDocument = globalThis.document;
  globalThis.document = document;
  try {
    const element = document.getElementById("knowledge-window");
    const windowMap = new Map([["win-1", element]]);
    const sent = [];
    const createNode = (tagName, className = "", text = "") => {
      const node = document.createElement(tagName);
      if (className) node.className = className;
      if (text !== undefined && text !== null) node.textContent = String(text);
      return node;
    };
    const surface = createSurface(mod, sent, "issue", {
      windowMap,
      createNode,
      createKnowledgeMarkdownBody: () => document.createElement("div"),
    });
    const state = surface.ensureKnowledgeBridgeState("win-1", "issue");
    state.entries = Array.from({ length: 500 }, (_, index) => ({
      number: index + 1,
      title: `Issue ${index + 1}`,
      state: "open",
      labels: index % 2 === 0 ? ["bug"] : ["gwt-spec"],
      phase: "backlog",
    }));
    state.baseEntries = state.entries.slice();
    state.refreshEnabled = true;
    surface.renderKnowledgeBridge("win-1");

    state.searching = true;
    surface.renderKnowledgeBridge("win-1");
    assert.equal(
      element.querySelector(".knowledge-status")?.textContent,
      "",
      "Issue/SPEC semantic progress must not create a visible status",
    );
    assert.equal(
      element.querySelector(".knowledge-status")?.classList.contains("visible"),
      false,
    );
    state.searching = false;
    surface.renderKnowledgeBridge("win-1");

    const list = element.querySelector(".knowledge-list");
    const originalRows = [...list.querySelectorAll(".knowledge-row")];
    assert.ok(originalRows.length > 0 && originalRows.length < 200);
    const samples = [];
    for (let index = 0; index < 40; index += 1) {
      const target = originalRows[(index * 13) % originalRows.length];
      const started = performance.now();
      target.dispatchEvent(new window.Event("click", { bubbles: true }));
      samples.push(performance.now() - started);
      assert.equal(
        element.querySelector(".knowledge-detail-title")?.textContent,
        `Issue ${((index * 13) % originalRows.length) + 1}`,
        "the clicked identity must be visible in the same synchronous turn",
      );
      assert.equal(
        element.querySelector(".knowledge-detail-subtitle")?.textContent,
        `#${((index * 13) % originalRows.length) + 1}`,
      );
      assert.ok(element.querySelector(".knowledge-detail-empty")?.textContent.includes("Loading detail"));
    }

    assert.deepEqual(
      [...list.querySelectorAll(".knowledge-row")],
      originalRows,
      "selection must not replace or reorder any list row node",
    );
    samples.sort((left, right) => left - right);
    const p95 = samples[Math.ceil(samples.length * 0.95) - 1];
    assert.ok(p95 < 50, `selection p95 ${p95.toFixed(2)}ms must stay below 50ms`);
  } finally {
    globalThis.document = originalDocument;
  }
});

function withPatchedTimersAsync(run) {
  const timers = [];
  const intervals = [];
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const originalSetInterval = globalThis.setInterval;
  const originalClearInterval = globalThis.clearInterval;
  globalThis.setTimeout = (callback, delay) => {
    timers.push({ callback, delay, cleared: false, fired: false });
    return timers.length;
  };
  globalThis.clearTimeout = (id) => {
    if (typeof id === "number" && timers[id - 1]) {
      timers[id - 1].cleared = true;
    }
  };
  globalThis.setInterval = (callback, delay) => {
    intervals.push({ callback, delay, cleared: false });
    return intervals.length;
  };
  globalThis.clearInterval = (id) => {
    if (typeof id === "number" && intervals[id - 1]) {
      intervals[id - 1].cleared = true;
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
  const forceFire = (timer) => {
    assert.ok(timer, "expected a timer callback to force-fire");
    timer.fired = true;
    timer.callback();
  };
  const finish = () => {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
    globalThis.setInterval = originalSetInterval;
    globalThis.clearInterval = originalClearInterval;
  };
  return Promise.resolve(run({ timers, intervals, fire, forceFire })).finally(finish);
}

function withPatchedTimers(run) {
  const timers = [];
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  globalThis.setTimeout = (callback, delay) => {
    timers.push({ callback, delay, cleared: false, fired: false });
    return timers.length;
  };
  globalThis.clearTimeout = (id) => {
    if (typeof id === "number" && timers[id - 1]) {
      timers[id - 1].cleared = true;
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

test("slow cache loads show progress without retrying or overlapping refresh (#5413 AC-1/3)", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);

    surface.requestKnowledgeBridge("win-1", "issue", false);
    fire((timer) => timer.delay === 5000);
    const state = surface.knowledgeBridgeStateMap.get("win-1");
    assert.equal(state.loading, true);
    assert.equal(state.error, "");
    assert.equal(state.loadSlow, true);
    surface.requestKnowledgeBridge("win-1", "issue", false);
    surface.requestKnowledgeBridge("win-1", "issue", true);
    surface.requestKnowledgeBridge("win-1", "issue", true);
    assert.equal(sent.filter(message => message.kind === "load_knowledge_bridge").length, 1);
    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_entries", id: "win-1", knowledge_kind: "issue",
      request_id: sent[0].request_id, entries: [ROW_A], refresh_enabled: true,
    });
    const loads = sent.filter(message => message.kind === "load_knowledge_bridge");
    assert.equal(loads.length, 2, "one queued explicit refresh starts only after the read completes");
    assert.equal(loads[1].refresh, true);
    assert.equal(state.loadSlow, false);
    surface.clearKnowledgeBridgeState("win-1");
  });
});

test("cache load deadline errors recover on the original late response (#5413 AC-1/2)", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);

    surface.requestKnowledgeBridge("win-1", "issue", false);
    const requestId = sent[0].request_id;
    fire((timer) => timer.delay === 5000);
    fire((timer) => timer.delay === 55000);
    const state = surface.knowledgeBridgeStateMap.get("win-1");
    assert.equal(state.error, "Timed out loading cache-backed data");
    assert.equal(state.loading, false);
    assert.equal(sent.filter(message => message.kind === "load_knowledge_bridge").length, 1);
    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_entries",
      id: "win-1",
      knowledge_kind: "issue",
      request_id: requestId,
      entries: [{ number: 42, title: "Issue bridge", phase: "backlog" }],
      selected_number: 42,
      empty_message: "",
      refresh_enabled: true,
    });

    assert.equal(
      state.baseEntries.length,
      1,
      "a late response must still populate an empty window instead of being discarded by request_id",
    );
    assert.equal(state.loading, false, "the applied response must finish the load");
    assert.equal(state.error, "");
    assert.equal(state.loadSlow, false);
  });
});

test("backend cache load failure ends waiting immediately (#5413 AC-1)", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ timers, fire }) => {
    const sent = [];
    const surface = createSurface(mod, sent);
    surface.requestKnowledgeBridge("win-1", "issue", false);
    fire(timer => timer.delay === 5000);
    surface.applyKnowledgeReceiveEvent({
      kind: "knowledge_error", id: "win-1", knowledge_kind: "issue",
      request_id: sent[0].request_id, message: "Cache read failed",
    });
    const state = surface.knowledgeBridgeStateMap.get("win-1");
    assert.equal(state.loading, false);
    assert.equal(state.error, "Cache read failed");
    assert.equal(state.loadSlow, false);
    assert.ok(timers.filter(timer => !timer.fired).every(timer => timer.cleared));
  });
});

test("closing a slow cache load cancels its deadline (#5413)", async () => {
  const mod = await importSurfaceModule();
  withPatchedTimers(({ timers, fire }) => {
    const surface = createSurface(mod, []);
    surface.requestKnowledgeBridge("win-1", "issue", false);
    fire(timer => timer.delay === 5000);
    surface.clearKnowledgeBridgeState("win-1");
    assert.ok(timers.filter(timer => !timer.fired).every(timer => timer.cleared));
    assert.equal(surface.knowledgeBridgeStateMap.has("win-1"), false);
  });
});

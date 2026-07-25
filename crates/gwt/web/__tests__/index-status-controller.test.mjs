// SPEC-1939 Phase 15 — project-bar Index badge withdrawn. The remaining
// coverage keeps the dedicated Index window and project-tab separation
// contract stable.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E3): the Index window search/health surface moved out
// of app.js into a dedicated module; assertions about the moved code read
// the module source while receive()/Settings wiring stays in app.js.
const indexSurfaceSource = readFileSync(
  resolve(here, "../project-index-search-surface.js"),
  "utf8",
);
const projectTabsRendererSource = readFileSync(
  resolve(here, "../project-tabs-renderer.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E4): the Settings window renderer, the settings:open
// dispatch listener, and requestFullIndexStatusRefresh moved into the
// extracted settings surface module.
const settingsSurfaceSource = readFileSync(
  resolve(here, "../settings-surface.js"),
  "utf8",
);

async function importIndexSearchSurface() {
  const indexSettingsStub = Buffer.from(`
    export function buildIndexHealthSummary() {
      return {
        available: false,
        readyCount: 0,
        totalCount: 0,
        degradedCount: 0,
        degradedScopes: [],
      };
    }
    export function renderIndexSettingsPanel() {}
  `).toString("base64");
  const source = indexSurfaceSource.replace(
    'from "/index-settings-panel.js"',
    `from "data:text/javascript;base64,${indexSettingsStub}"`,
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function makeElement(document, tag, options = {}) {
  const element = document.createElement(tag);
  if (options.className) {
    element.className = options.className;
  }
  for (const [name, value] of Object.entries(options.attrs || {})) {
    element.setAttribute(name, String(value));
  }
  for (const [name, value] of Object.entries(options.dataset || {})) {
    element.dataset[name] = String(value);
  }
  if (options.text !== undefined) {
    element.textContent = String(options.text);
  }
  return element;
}

function installDeterministicTimers() {
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const pending = new Map();
  let nextId = 1;
  globalThis.setTimeout = (callback, delay) => {
    const id = nextId;
    nextId += 1;
    pending.set(id, { callback, delay });
    return id;
  };
  globalThis.clearTimeout = (id) => {
    pending.delete(id);
  };
  return {
    fire(delay) {
      const entry = [...pending.entries()].find(([, timer]) => timer.delay === delay);
      assert.ok(entry, `expected a pending ${delay}ms timer`);
      const [id, timer] = entry;
      pending.delete(id);
      timer.callback();
    },
    restore() {
      globalThis.setTimeout = originalSetTimeout;
      globalThis.clearTimeout = originalClearTimeout;
    },
  };
}

async function mountIndexSearchSurface({ indexStatus = null } = {}) {
  const mod = await importIndexSearchSurface();
  const { document, window } = parseHTML(
    "<html><body><section data-id='index-1'><div id='mount'></div></section></body></html>",
  );
  Object.defineProperty(window.HTMLSelectElement.prototype, "value", {
    configurable: true,
    get() {
      return this.getAttribute("value") || "";
    },
    set(value) {
      this.setAttribute("value", String(value));
    },
  });
  const previousDocument = globalThis.document;
  const previousCss = globalThis.CSS;
  globalThis.document = document;
  globalThis.CSS = { escape: (value) => String(value) };
  const sent = [];
  const surface = mod.createProjectIndexSearchSurface({
    send: (message) => sent.push(message),
    sendWindowFocus() {},
    focusWindowLocally() {},
    activeProjectTab: () => ({ project_root: "/project" }),
    makeEl: (tag, options) => makeElement(document, tag, options),
    clearChildren: (node) => node.replaceChildren(),
    focusOrSpawnPreset() {},
    knowledgeKindForPreset() {},
    requestKnowledgeDetail() {},
    renderKnowledgeBridge() {},
    renderIndexPanelInAllSettingsWindows() {},
    refreshProjectTabStateCues() {},
    requestFullIndexStatusRefresh() {},
  });
  surface.mountProjectIndexSurface(document.getElementById("mount"), {
    id: "index-1",
  });
  if (indexStatus) {
    surface.setIndexStatus("/project", indexStatus);
  }
  return {
    document,
    window,
    sent,
    surface,
    restore() {
      globalThis.document = previousDocument;
      globalThis.CSS = previousCss;
    },
  };
}

test("project-bar Index badge has been withdrawn (SPEC-1939 Phase 13)", () => {
  const { document } = parseHTML(indexHtml);
  assert.equal(
    document.getElementById("index-status"),
    null,
    "#index-status badge must not exist in the embedded HTML",
  );
  assert.ok(
    !indexHtml.includes(".index-status "),
    "embedded inline <style> must not declare .index-status rules",
  );
  assert.ok(
    !componentsCss.includes(".index-status"),
    "components.css must not declare .index-status rules",
  );
  assert.ok(
    !indexHtml.includes("index-status-toast"),
    "embedded HTML must not declare the index-status progress toast",
  );
});

test("project tab state cues no longer wire Project Index health", () => {
  assert.ok(
    !projectTabsRendererSource.includes("aggregateProjectTabDotState"),
    "project tab state cues should be driven by agent runtime state, not Project Index health",
  );
  assert.ok(
    !appSource.includes("aggregateProjectTabDotState"),
    "app.js must not import or call the removed project-tab Index health helper",
  );
  assert.ok(
    !appSource.includes("formatIndexStatusLabel"),
    "app.js must not import or call the removed formatIndexStatusLabel helper",
  );
  assert.ok(
    !appSource.includes("showRepairingProgressToast"),
    "app.js must not retain the badge progress toast helper",
  );
  assert.ok(
    !appSource.includes("indexStatusLabel"),
    "app.js must not retain references to the removed badge element",
  );
});

test("settings target=index opens the dedicated Index window", () => {
  assert.ok(
    !settingsSurfaceSource.includes('buildSettingsTab("index"'),
    "Settings must not expose an Index tab",
  );
  assert.ok(
    !settingsSurfaceSource.includes('dataset.settingsPanel = "index"'),
    "Settings must not mount an Index panel",
  );
  assert.ok(
    settingsSurfaceSource.includes('if (target === "index")') &&
      settingsSurfaceSource.includes('focusOrSpawnPreset("index");'),
    "settings:open target=index must spawn the dedicated Index window",
  );
});

test("Index window exposes semantic search and health refresh contract", () => {
  assert.ok(
    settingsSurfaceSource.includes("function requestFullIndexStatusRefresh()"),
    "expected a dedicated full index status refresh helper",
  );
  assert.ok(
    settingsSurfaceSource.includes('send({ kind: "refresh_index_status", project_root: activeProjectRoot })'),
    "Index window Health tab must request the expensive all-worktree status on demand",
  );
  assert.ok(
    indexSurfaceSource.includes('kind: "search_project_index"') &&
      appSource.includes('case "project_index_search_results"') &&
    appSource.includes('case "project_index_search_error"'),
    "Index window must wire search request, result, and error events",
  );
});

test("Index search UI exposes explicit search controls and readable result scoring", () => {
  assert.ok(
    indexSurfaceSource.includes("index-run-button") &&
      indexSurfaceSource.includes("formatIndexSearchMatch") &&
      indexSurfaceSource.includes("% match"),
    "Index search should have an explicit search action and user-facing match scores",
  );
  assert.ok(
    indexSurfaceSource.includes("indexFileScopesSelected(state)") &&
      indexSurfaceSource.includes("File worktree"),
    "worktree selection should be scoped to Files / Docs search instead of looking globally required",
  );
  assert.ok(
    indexSurfaceSource.includes("moveIndexResultSelection") &&
      indexSurfaceSource.includes('event.key === "ArrowDown"') &&
      indexSurfaceSource.includes('event.key === "ArrowUp"'),
    "result lists should support keyboard movement",
  );
  assert.ok(
    indexSurfaceSource.includes("inFlightSignature") &&
      indexSurfaceSource.includes("state.inFlightSignature === searchSignature"),
    "explicit search clicks should not duplicate an identical debounced search already in flight",
  );
  assert.ok(
    indexSurfaceSource.includes("state.query = input.value;") &&
      indexSurfaceSource.includes("renderProjectIndexSearch(windowData.id);\n            scheduleProjectIndexSearch(windowData.id);"),
    "typing in the search field should immediately enable the explicit Search button before debounce fires",
  );
});

test("Index search UI exposes semantic and all-terms match modes", () => {
  assert.ok(
    indexSurfaceSource.includes("index-match-mode-list") &&
      indexSurfaceSource.includes('data-match-mode="semantic"') &&
      indexSurfaceSource.includes('data-match-mode="all_terms"'),
    "Index search should expose a Semantic / All terms segmented control",
  );
  assert.ok(
    indexSurfaceSource.includes("match_mode: intent.matchMode") &&
      indexSurfaceSource.includes("matchMode") &&
      indexSurfaceSource.includes("searchSignature = JSON.stringify({ query, scopes, worktreeHash, matchMode"),
    "match mode should be sent to the backend and included in the request signature",
  );
  assert.ok(
    indexSurfaceSource.includes("state.suggestions") &&
      indexSurfaceSource.includes("Semantic suggestions") &&
      indexSurfaceSource.includes("Matched:") &&
      indexSurfaceSource.includes("Searching all terms"),
    "All terms mode should render suggestions separately, show concise matched-term evidence, and use matching loading copy",
  );
  assert.ok(
    indexSurfaceSource.includes("function indexSearchPlaceholder(state)") &&
      indexSurfaceSource.includes("Search by meaning, e.g. work lifecycle") &&
      indexSurfaceSource.includes("All terms required, e.g. Work discussion") &&
      indexSurfaceSource.includes("input.placeholder = indexSearchPlaceholder(state);"),
    "Index search placeholder should explain the active Semantic / All terms mode",
  );
});

test("Index search clear preserves the active process fence", () => {
  assert.ok(
    indexSurfaceSource.includes("function clearProjectIndexSearchState(state)") &&
      indexSurfaceSource.includes("if (state.inFlightRequestId)") &&
      indexSurfaceSource.includes('state.queuedSearchIntent = { status: "empty" };'),
    "clearing must queue an empty latest intent without forgetting the active process",
  );
  assert.ok(
    indexSurfaceSource.includes("if (!state.query.trim()) {\n              clearProjectIndexSearchState(state);"),
    "the input handler must clear visible results immediately instead of waiting for debounce",
  );
});

test("Index search coalesces held A to latest C and applies only matching responses", async () => {
  const timers = installDeterministicTimers();
  const mounted = await mountIndexSearchSurface();
  try {
    const input = mounted.document.querySelector(".index-search-input");
    const typeQuery = (query) => {
      input.value = query;
      input.dispatchEvent(new mounted.window.Event("input"));
    };

    typeQuery("A");
    timers.fire(250);
    assert.deepEqual(
      mounted.sent.map((message) => message.query),
      ["A"],
      "the first search must start immediately after debounce",
    );
    const requestA = mounted.sent[0];
    assert.deepEqual(
      Object.keys(requestA).sort(),
      [
        "id",
        "kind",
        "match_mode",
        "query",
        "request_id",
        "scopes",
        "worktree_hash",
      ],
      "latest-only scheduling must preserve the legacy wire shape",
    );

    typeQuery("B");
    typeQuery("C");
    timers.fire(250);
    assert.deepEqual(
      mounted.sent.map((message) => message.query),
      ["A"],
      "a held request permits no parallel backend search",
    );
    const state = mounted.surface.indexSearchStateMap.get("index-1");
    assert.equal(
      state.queuedSearchIntent?.query,
      "C",
      "the surface must retain only the latest queued search intent",
    );

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: requestA.request_id,
      results: [{ title: "result A" }],
      suggestions: [],
    });

    assert.deepEqual(
      mounted.sent.map((message) => message.query),
      ["A", "C"],
      "A completion must start only the latest queued query C",
    );
    const requestC = mounted.sent[1];
    assert.deepEqual(
      state.results,
      [],
      "superseded A results must never become visible while C is current",
    );
    assert.equal(state.searching, true);

    mounted.surface.handleProjectIndexSearchError({
      id: "index-1",
      request_id: requestA.request_id,
      message: "late A failure",
    });
    assert.equal(state.error, "", "late A errors must not affect C lifecycle");
    assert.equal(state.inFlightRequestId, requestC.request_id);

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: requestC.request_id,
      results: [{ title: "result C" }],
      suggestions: [],
    });
    assert.deepEqual(state.results, [{ title: "result C" }]);
    assert.equal(state.searching, false);
    assert.equal(state.error, "");
  } finally {
    mounted.restore();
    timers.restore();
  }
});

test("Index search clear and retype keep the held process single-flight", async () => {
  const timers = installDeterministicTimers();
  const mounted = await mountIndexSearchSurface();
  try {
    const input = mounted.document.querySelector(".index-search-input");
    input.value = "held query";
    input.dispatchEvent(new mounted.window.Event("input"));
    timers.fire(250);

    const request = mounted.sent[0];
    const state = mounted.surface.indexSearchStateMap.get("index-1");
    assert.equal(state.searching, true, "the fixture must hold a real request");
    assert.equal(state.inFlightRequestId, request.request_id);

    input.value = "";
    input.dispatchEvent(new mounted.window.Event("input"));

    assert.equal(mounted.sent.length, 1, "clear must not send replacement work");
    assert.equal(state.searching, false);
    assert.equal(
      state.inFlightRequestId,
      request.request_id,
      "clear must retain the active process fence until its response arrives",
    );
    assert.notEqual(state.inFlightSignature, "");
    assert.equal(state.queuedSearchIntent?.status, "empty");
    assert.equal(state.searchIntentPending, false);
    assert.deepEqual(state.results, []);
    assert.deepEqual(state.suggestions, []);
    assert.equal(state.error, "");

    input.value = "replacement query";
    input.dispatchEvent(new mounted.window.Event("input"));
    timers.fire(250);
    assert.equal(
      mounted.sent.length,
      1,
      "retyping after clear must not run beside the still-held process",
    );
    assert.equal(state.queuedSearchIntent?.query, "replacement query");

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: request.request_id,
      results: [{ title: "stale held result" }],
      suggestions: [{ title: "stale held suggestion" }],
    });
    assert.equal(
      mounted.sent.length,
      2,
      "held completion must start exactly one replacement",
    );
    const replacement = mounted.sent[1];
    assert.equal(replacement.query, "replacement query");
    mounted.surface.handleProjectIndexSearchError({
      id: "index-1",
      request_id: request.request_id,
      message: "stale held failure",
    });

    assert.deepEqual(state.results, [], "late held results must stay invalidated");
    assert.deepEqual(state.suggestions, [], "late held suggestions must stay invalidated");
    assert.equal(state.error, "", "late held errors must stay invalidated");
    assert.equal(state.inFlightRequestId, replacement.request_id);

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: replacement.request_id,
      results: [{ title: "replacement result" }],
      suggestions: [],
    });
    assert.deepEqual(state.results, [{ title: "replacement result" }]);
    assert.equal(state.searching, false);
    const visibleText =
      mounted.document.querySelector(".index-result-list").textContent;
    assert.match(visibleText, /replacement result/);
    assert.doesNotMatch(
      visibleText,
      /stale held/,
      "only the replacement response may populate the mounted surface",
    );
  } finally {
    mounted.restore();
    timers.restore();
  }
});

test("Index search holds scope, match mode, and worktree changes to one latest request", async () => {
  const timers = installDeterministicTimers();
  const mounted = await mountIndexSearchSurface({
    indexStatus: {
      state: "ready",
      worktrees: {
        "wt-current": { branch: "main", path: "/project" },
        "wt-other": { branch: "feature", path: "/other" },
      },
    },
  });
  try {
    const input = mounted.document.querySelector(".index-search-input");
    input.value = "held query";
    input.dispatchEvent(new mounted.window.Event("input"));
    timers.fire(250);

    const request = mounted.sent[0];
    const state = mounted.surface.indexSearchStateMap.get("index-1");
    assert.equal(state.searching, true, "the fixture must hold a real request");
    assert.equal(state.inFlightRequestId, request.request_id);

    mounted.document
      .querySelector("[data-scope='files']")
      .dispatchEvent(new mounted.window.Event("click", { bubbles: true }));
    timers.fire(250);

    assert.equal(mounted.sent.length, 1, "scope changes must not run in parallel");
    assert.equal(state.queuedSearchIntent?.scopes.includes("files"), true);
    assert.equal(state.queuedSearchIntent?.matchMode, "semantic");
    assert.equal(state.queuedSearchIntent?.worktreeHash, "wt-current");

    mounted.document
      .querySelector("[data-match-mode='all_terms']")
      .dispatchEvent(new mounted.window.Event("click", { bubbles: true }));
    timers.fire(250);

    assert.equal(mounted.sent.length, 1, "match-mode changes must not run in parallel");
    assert.equal(state.queuedSearchIntent?.scopes.includes("files"), true);
    assert.equal(state.queuedSearchIntent?.matchMode, "all_terms");
    assert.equal(state.queuedSearchIntent?.worktreeHash, "wt-current");

    const worktreeSelect = mounted.document.querySelector(".index-worktree-select");
    worktreeSelect.value = "wt-other";
    worktreeSelect.dispatchEvent(new mounted.window.Event("change"));
    timers.fire(250);

    assert.equal(mounted.sent.length, 1, "worktree changes must not run in parallel");
    assert.equal(state.queuedSearchIntent?.scopes.includes("files"), true);
    assert.equal(state.queuedSearchIntent?.matchMode, "all_terms");
    assert.equal(state.queuedSearchIntent?.worktreeHash, "wt-other");

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: request.request_id,
      results: [{ title: "stale held result" }],
      suggestions: [],
    });

    assert.equal(mounted.sent.length, 2, "held completion must start one latest request");
    assert.deepEqual(
      {
        query: mounted.sent[1].query,
        matchMode: mounted.sent[1].match_mode,
        worktreeHash: mounted.sent[1].worktree_hash,
        filesSelected: mounted.sent[1].scopes.includes("files"),
      },
      {
        query: "held query",
        matchMode: "all_terms",
        worktreeHash: "wt-other",
        filesSelected: true,
      },
      "the replacement must preserve the latest state from every held control change",
    );
    assert.deepEqual(state.results, [], "the superseded held response must stay hidden");
    assert.equal(state.inFlightRequestId, mounted.sent[1].request_id);
    assert.equal(state.queuedSearchIntent, null);

    mounted.surface.handleProjectIndexSearchResults({
      id: "index-1",
      request_id: mounted.sent[1].request_id,
      results: [{ title: "latest result" }],
      suggestions: [],
    });

    assert.deepEqual(state.results, [{ title: "latest result" }]);
    assert.equal(state.searching, false);
  } finally {
    mounted.restore();
    timers.restore();
  }
});

test("Index result Open uses target numbers for Issue and SPEC hits", () => {
  assert.ok(
    indexSurfaceSource.includes("function openKnowledgeIndexResultTarget(preset, target)") &&
      indexSurfaceSource.includes("requestKnowledgeDetail(windowId, knowledgeKind, number)") &&
      indexSurfaceSource.includes("pendingIndexOpenTargetsByPreset.set(preset"),
    "Issue/SPEC result Open should select the indexed target number, including newly created windows",
  );
  assert.ok(
    indexSurfaceSource.includes('openKnowledgeIndexResultTarget("issue", target)') &&
      indexSurfaceSource.includes('openKnowledgeIndexResultTarget("spec", target)'),
    "Issue and SPEC index results must use target-aware navigation",
  );
});

test("Index search tab refreshes missing health once per project root", () => {
  assert.ok(
    indexSurfaceSource.includes("function ensureIndexStatusRefresh(state, status)") &&
      indexSurfaceSource.includes("state.statusRefreshProjectRoot !== activeProjectRoot") &&
      indexSurfaceSource.includes("state.statusRefreshProjectRoot = activeProjectRoot;") &&
      indexSurfaceSource.includes("requestFullIndexStatusRefresh();"),
    "Search mount should request a full status refresh when the active project has no health payload yet",
  );
  assert.ok(
    indexSurfaceSource.includes("state.statusRefreshProjectRoot = \"\";"),
    "Search refresh guard should reset when project context disappears or a status payload arrives",
  );
});

test("Index search tab renders abnormal-first health summary without repair controls", () => {
  assert.ok(
    indexSurfaceSource.includes("function renderIndexSearchHealthSummary(") &&
      indexSurfaceSource.includes('data-role="index-search-health-summary"'),
    "Search tab should render a dedicated health summary near the search controls",
  );
  assert.ok(
    indexSurfaceSource.includes("index-search-health-inline") &&
      indexSurfaceSource.includes("open-index-health") &&
      indexSurfaceSource.includes("requestFullIndexStatusRefresh();"),
    "degraded Search health should stay inline and provide a Health tab action",
  );
  assert.ok(
    indexSurfaceSource.includes("buildIndexHealthSummary(") &&
      indexSurfaceSource.includes("readyCount") &&
      indexSurfaceSource.includes("degradedScopes"),
    "Search health summary should be built from ready/degraded scope counts",
  );
  assert.ok(
    !indexSurfaceSource.includes('data-action="open-project"') &&
      !indexSurfaceSource.includes("Open Project"),
    "Index window empty states must not offer an Open Project action",
  );
});

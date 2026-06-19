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
      indexSurfaceSource.includes("state.searching && state.inFlightSignature === searchSignature"),
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
    indexSurfaceSource.includes("match_mode: state.matchMode") &&
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

test("Index search clears invalidate any in-flight request immediately", () => {
  assert.ok(
    indexSurfaceSource.includes("function clearProjectIndexSearchState(state)") &&
      indexSurfaceSource.includes("invalidateProjectIndexSearchRequest(state);"),
    "clearing the query should invalidate stale backend responses through a shared helper",
  );
  assert.ok(
    indexSurfaceSource.includes("if (!state.query.trim()) {\n              clearProjectIndexSearchState(state);"),
    "the input handler must clear and invalidate immediately instead of waiting for debounce",
  );
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

test("Index search tab does not trigger full health refresh on mount", () => {
  const refreshCallCount = (
    indexSurfaceSource.match(/requestFullIndexStatusRefresh\(\);/g) || []
  ).length;
  assert.equal(
    refreshCallCount,
    2,
    "full index status refresh must stay limited to Health tab activation and manual refresh",
  );
});

// SPEC-1939 Phase 15 — project-bar Index badge withdrawn. The remaining
// coverage exercises the per-tab dot aggregator and the Index window
// navigation event helper that other entry points still consume.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import {
  aggregateProjectTabDotState,
  dispatchOpenIndexSettings,
  INDEX_STATUS_OPEN_SETTINGS_EVENT,
  INDEX_STATUS_OPEN_SETTINGS_TARGET,
} from "../index-status-controller.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const projectTabsRendererSource = readFileSync(
  resolve(here, "../project-tabs-renderer.js"),
  "utf8",
);
const projectTabSource = `${appSource}\n${projectTabsRendererSource}`;

test("dispatchOpenIndexSettings emits settings:open with target=index", () => {
  const { document } = parseHTML(`<!doctype html><body><button id="badge"></button></body>`);
  const badge = document.getElementById("badge");

  let captured = null;
  document.addEventListener(INDEX_STATUS_OPEN_SETTINGS_EVENT, (event) => {
    captured = event;
  });

  dispatchOpenIndexSettings(badge);

  assert.ok(captured, "settings:open should bubble up to document");
  assert.equal(captured.detail.target, INDEX_STATUS_OPEN_SETTINGS_TARGET);
});

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

test("aggregateProjectTabDotState ignores repo-shared scopes", () => {
  // Repo-shared scopes are intentionally excluded from per-worktree dots.
  assert.equal(
    aggregateProjectTabDotState({
      scopes: {
        issues: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
        specs: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
        memory: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
        board: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
      },
    }),
    "",
  );
});

test("aggregateProjectTabDotState returns 'error' when any worktree files cell is unhealthy", () => {
  assert.equal(
    aggregateProjectTabDotState({
      state: "repair_required",
      scopes: {
        files: {
          wtA: { healthy: true, repair_required: false, document_count: 1 },
          wtB: { healthy: false, repair_required: true, document_count: 0 },
        },
      },
    }),
    "error",
  );
});

test("aggregateProjectTabDotState returns 'repairing' when state is repairing and no error", () => {
  assert.equal(
    aggregateProjectTabDotState({
      state: "repairing",
      scopes: {
        files: {
          wtA: { healthy: true, repair_required: false, document_count: 1 },
        },
        "files-docs": {
          wtA: { healthy: true, repair_required: false, document_count: 1 },
        },
      },
    }),
    "repairing",
  );
});

test("aggregateProjectTabDotState returns 'ready' when every files / files-docs cell is healthy", () => {
  assert.equal(
    aggregateProjectTabDotState({
      state: "ready",
      scopes: {
        files: {
          wtA: { healthy: true, repair_required: false, document_count: 310 },
        },
        "files-docs": {
          wtA: { healthy: true, repair_required: false, document_count: 16 },
        },
      },
    }),
    "ready",
  );
});

test("aggregateProjectTabDotState returns '' when no worktree health is reported", () => {
  assert.equal(aggregateProjectTabDotState({ state: "ready", scopes: {} }), "");
  assert.equal(aggregateProjectTabDotState(null), "");
});

test("app.js still wires the per-tab dot aggregator", () => {
  assert.ok(
    projectTabSource.includes("aggregateProjectTabDotState(status)"),
    "renderProjectTabs should consume the shared aggregator",
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
    !appSource.includes('buildSettingsTab("index"'),
    "Settings must not expose an Index tab",
  );
  assert.ok(
    !appSource.includes('dataset.settingsPanel = "index"'),
    "Settings must not mount an Index panel",
  );
  assert.ok(
    appSource.includes('if (target === "index")') &&
      appSource.includes('focusOrSpawnPreset("index");'),
    "settings:open target=index must spawn the dedicated Index window",
  );
});

test("Index window exposes semantic search and health refresh contract", () => {
  assert.ok(
    appSource.includes("function requestFullIndexStatusRefresh()"),
    "expected a dedicated full index status refresh helper",
  );
  assert.ok(
    appSource.includes('send({ kind: "refresh_index_status", project_root: activeProjectRoot })'),
    "Index window Health tab must request the expensive all-worktree status on demand",
  );
  assert.ok(
    appSource.includes('kind: "search_project_index"') &&
      appSource.includes('case "project_index_search_results"') &&
    appSource.includes('case "project_index_search_error"'),
    "Index window must wire search request, result, and error events",
  );
});

test("Index search UI exposes explicit search controls and readable result scoring", () => {
  assert.ok(
    appSource.includes("index-run-button") &&
      appSource.includes("formatIndexSearchMatch") &&
      appSource.includes("% match"),
    "Index search should have an explicit search action and user-facing match scores",
  );
  assert.ok(
    appSource.includes("indexFileScopesSelected(state)") &&
      appSource.includes("File worktree"),
    "worktree selection should be scoped to Files / Docs search instead of looking globally required",
  );
  assert.ok(
    appSource.includes("moveIndexResultSelection") &&
      appSource.includes('event.key === "ArrowDown"') &&
      appSource.includes('event.key === "ArrowUp"'),
    "result lists should support keyboard movement",
  );
  assert.ok(
    appSource.includes("inFlightSignature") &&
      appSource.includes("state.searching && state.inFlightSignature === searchSignature"),
    "explicit search clicks should not duplicate an identical debounced search already in flight",
  );
  assert.ok(
    appSource.includes("state.query = input.value;") &&
      appSource.includes("renderProjectIndexSearch(windowData.id);\n            scheduleProjectIndexSearch(windowData.id);"),
    "typing in the search field should immediately enable the explicit Search button before debounce fires",
  );
});

test("Index search tab does not trigger full health refresh on mount", () => {
  const refreshCallCount = (
    appSource.match(/requestFullIndexStatusRefresh\(\);/g) || []
  ).length;
  assert.equal(
    refreshCallCount,
    2,
    "full index status refresh must stay limited to Health tab activation and manual refresh",
  );
});

// SPEC-1939 Phase 12 / T-IDX-103..T-IDX-105 — index status badge formatter
// + click → settings:open dispatch coverage.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import {
  aggregateProjectTabDotState,
  dispatchOpenIndexSettings,
  formatIndexStatusLabel,
  INDEX_STATUS_OPEN_SETTINGS_EVENT,
  INDEX_STATUS_OPEN_SETTINGS_TARGET,
} from "../index-status-controller.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("formatIndexStatusLabel hides badge for empty / skipped", () => {
  for (const state of ["", null, undefined, "skipped"]) {
    const formatted = formatIndexStatusLabel(state);
    assert.equal(formatted.hidden, true, `state=${String(state)} should hide`);
    assert.equal(formatted.label, "");
    assert.equal(formatted.className, "index-status");
    assert.equal(formatted.title, "");
  }
});

test("formatIndexStatusLabel labels each visible state distinctly", () => {
  assert.deepEqual(formatIndexStatusLabel("ready"), {
    hidden: false,
    label: "Index: ready",
    className: "index-status ready",
    title: "Project index is ready",
  });

  assert.deepEqual(formatIndexStatusLabel("repairing"), {
    hidden: false,
    label: "Index: repairing",
    className: "index-status repairing",
    title: "Auto-rebuild in progress",
  });

  assert.deepEqual(formatIndexStatusLabel("repair_required"), {
    hidden: false,
    label: "Index: repair",
    className: "index-status repair_required",
    title: "Auto-rebuild not started",
  });

  assert.deepEqual(formatIndexStatusLabel("error"), {
    hidden: false,
    label: "Index: error",
    className: "index-status error",
    title: "Auto-rebuild failed",
  });

  // Unknown / "checking" defaults to neutral checking copy.
  assert.deepEqual(formatIndexStatusLabel("checking"), {
    hidden: false,
    label: "Index: checking",
    className: "index-status checking",
    title: "Checking project index health",
  });
  assert.equal(
    formatIndexStatusLabel("anything-else").label,
    "Index: checking",
    "unknown state falls back to checking",
  );
});

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

test("index-status badge in index.html is a button with accessible label", () => {
  const { document } = parseHTML(indexHtml);
  const badge = document.getElementById("index-status");

  assert.ok(badge, "#index-status element exists");
  assert.equal(
    badge.tagName.toUpperCase(),
    "BUTTON",
    "badge must be a clickable button (T-IDX-105)",
  );
  assert.equal(badge.getAttribute("type"), "button");
  assert.match(
    badge.getAttribute("aria-label") || "",
    /index/i,
    "badge should have an accessible label",
  );
  // Hidden by default until backend reports a state.
  assert.ok(badge.hasAttribute("hidden"));
});

test("index.html declares spinner / yellow CSS for repairing", () => {
  // Either the inline <style> in index.html or components.css must define
  // the `.index-status.repairing` rule with explicit yellow tokens.
  const repairingInline = /\.index-status\.repairing\b/.test(indexHtml);
  const repairingComponents = /\.index-status\.repairing\b/.test(componentsCss);
  assert.ok(
    repairingInline && repairingComponents,
    "repairing must be styled in both index.html and components.css",
  );
  assert.match(
    indexHtml,
    /\.index-status\.repairing::before[\s\S]*animation: index-status-spin/,
    "repairing should render a spinner pseudo-element",
  );
});

test("renderIndexStatus in app.js consumes formatIndexStatusLabel", () => {
  assert.ok(
    appSource.includes('from "/index-status-controller.js"'),
    "app.js should import index-status-controller",
  );
  assert.ok(
    appSource.includes("formatIndexStatusLabel(state)"),
    "renderIndexStatus should delegate to formatIndexStatusLabel",
  );
  assert.ok(
    appSource.includes("dispatchOpenIndexSettings(indexStatusLabel)"),
    "click handler should dispatch settings:open via the shared helper",
  );
});

test("aggregateProjectTabDotState ignores repo-shared scopes", () => {
  // issues / specs are repo-shared and intentionally do not contribute.
  assert.equal(
    aggregateProjectTabDotState({
      scopes: {
        issues: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
        specs: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
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

test("app.js wires the shared aggregator and progress toast helpers", () => {
  assert.ok(
    appSource.includes("aggregateProjectTabDotState(status)"),
    "renderProjectTabs should consume the shared aggregator",
  );
  assert.ok(
    appSource.includes("showRepairingProgressToast(status)"),
    "indexStatusLabel click should call the progress toast helper",
  );
});

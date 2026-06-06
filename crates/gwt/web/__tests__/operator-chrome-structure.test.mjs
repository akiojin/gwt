import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexPath = resolve(here, "../index.html");
const html = readFileSync(indexPath, "utf8");
const { document } = parseHTML(html);
const operatorShellSource = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const projectTabsRendererSource = readFileSync(
  resolve(here, "../project-tabs-renderer.js"),
  "utf8",
);
const branchCleanupSource = readFileSync(resolve(here, "../branch-cleanup-modal.js"), "utf8");
const windowDockingSource = readFileSync(resolve(here, "../window-docking.js"), "utf8");
const workspaceOverviewPath = resolve(here, "../workspace-kanban-surface.js");
const workspaceOverviewSource = existsSync(workspaceOverviewPath)
  ? readFileSync(workspaceOverviewPath, "utf8")
  : "";
const appAndProjectTabsSource = `${appSource}\n${projectTabsRendererSource}`;
const typographySource = readFileSync(resolve(here, "../styles/typography.css"), "utf8");
// Issue #2694 Phase D: the formerly-inline <style> block now lives at
// /styles/app.css and is loaded via `<link rel="stylesheet">`. The grep
// surface used by the CSS contract tests below remains stable.
const inlineStyle = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function cssRemVar(source, name) {
  const match = source.match(new RegExp(`${name}:\\s*([0-9.]+)rem\\s*;`));
  assert.ok(match, `missing typography token: ${name}`);
  return Number(match[1]);
}

function terminalOptionNumber(name) {
  const runtime = appSource.match(/new\s+Terminal\(\{\s*([\s\S]*?)\s*\}\);/);
  assert.ok(runtime, "expected xterm Terminal constructor options");
  const match = runtime[1].match(new RegExp(`${name}:\\s*([0-9.]+)`));
  assert.ok(match, `missing terminal option: ${name}`);
  return Number(match[1]);
}

function extractFunctionBody(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `expected function ${name} in source`);
  const open = source.indexOf("{", start);
  assert.notEqual(open, -1, `expected function ${name} body`);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, i);
      }
    }
  }
  assert.fail(`unterminated function ${name}`);
}

// SPEC-2356 — extract every @media block matching the supplied condition,
// using brace-depth tracking so nested rules don't truncate the body.
function extractMediaBlocks(css, condition) {
  const out = [];
  const marker = `@media`;
  let cursor = 0;
  while (true) {
    const at = css.indexOf(marker, cursor);
    if (at < 0) break;
    const headerEnd = css.indexOf("{", at);
    if (headerEnd < 0) break;
    const header = css.slice(at, headerEnd);
    if (!header.includes(condition)) {
      cursor = headerEnd + 1;
      continue;
    }
    // Walk forward from headerEnd, tracking brace depth.
    let depth = 1;
    let i = headerEnd + 1;
    while (i < css.length && depth > 0) {
      const ch = css[i];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      i += 1;
    }
    out.push(css.slice(headerEnd + 1, i - 1));
    cursor = i;
  }
  return out.join("\n");
}

function extractContainerBlocks(css, condition) {
  const out = [];
  const marker = `@container`;
  let cursor = 0;
  while (true) {
    const at = css.indexOf(marker, cursor);
    if (at < 0) break;
    const headerEnd = css.indexOf("{", at);
    if (headerEnd < 0) break;
    const header = css.slice(at, headerEnd);
    if (!header.includes(condition)) {
      cursor = headerEnd + 1;
      continue;
    }
    let depth = 1;
    let i = headerEnd + 1;
    while (i < css.length && depth > 0) {
      const ch = css[i];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      i += 1;
    }
    out.push(css.slice(headerEnd + 1, i - 1));
    cursor = i;
  }
  return out.join("\n");
}

test("index.html declares Operator chrome scaffold", () => {
  for (const sel of [
    "#op-theme-toggle",
    ".op-sidebar",
    ".op-status-strip",
    "#op-strip-clock",
    "#op-strip-active",
    "#op-strip-idle",
    "#op-strip-blocked",
    "#op-briefing",
    "#op-palette-backdrop",
    "#op-palette-input",
    "#op-palette-list",
    "#op-hotkey-overlay",
    "#op-palette-button",
  ]) {
    assert.ok(document.querySelector(sel), `missing chrome element: ${sel}`);
  }
});

test("project bar exposes a Layers column with three layers", () => {
  const layers = document.querySelectorAll(".op-sidebar__section .op-layer[data-layer]");
  assert.equal(layers.length, 3, "expected three sidebar layers");
  const labels = Array.from(layers).map((el) => el.dataset.layer);
  assert.deepEqual(labels.sort(), ["agents", "git", "hooks"]);
});

test("project bar and command palette expose Start Work outside the Branches surface", () => {
  const projectBarAction = document.querySelector('.op-sidebar .op-layer[data-cmd="start-work"]');
  assert.ok(projectBarAction, "expected Project Bar to expose a Start Work action");
  assert.match(projectBarAction.textContent, /Start Work/);
  assert.match(
    operatorShellSource,
    /id:\s*"start-work"[\s\S]+label:\s*"Start Work"/,
    "expected Command Palette registry to include Start Work",
  );
  assert.match(
    appSource,
    /case\s+"start-work":[\s\S]+kind:\s*"open_start_work"/,
    "expected Start Work command to send the global open_start_work event",
  );
});

test("frontend handles active work projection as status-strip telemetry", () => {
  assert.match(
    appSource,
    /case\s+"active_work_projection":[\s\S]+activeWorkProjection\s*=\s*event\.projection/,
    "expected frontend to store active_work_projection separately from canvas workspace_state",
  );
  assert.match(
    appSource,
    /activeWorkProjection[\s\S]+applyTelemetryCounts/,
    "expected active work projection to feed Operator telemetry",
  );
  assert.match(
    appSource,
    /counts\.branches[\s\S]+activeWorks\.length/,
    "expected Status Strip Work telemetry to count active Works, not only branch-list rows",
  );
});

test("workspace_state hot path leaves Active Work overview redraw to active_work_projection", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  assert.doesNotMatch(
    renderAppStateBody,
    /renderActiveWorkOverview\s*\(/,
    "workspace_state renderAppState must not rebuild Active Work overview DOM",
  );

  const receiveBody = extractFunctionBody(appSource, "receive");
  const activeProjectionCase = receiveBody.match(
    /case\s+"active_work_projection":[\s\S]*?break;/,
  );
  assert.ok(activeProjectionCase, "expected active_work_projection receive case");
  assert.match(
    activeProjectionCase[0],
    /activeWorkProjection\s*=\s*event\.projection/,
    "active_work_projection must still own the projection state",
  );
  assert.match(
    activeProjectionCase[0],
    /renderActiveWorkOverview\(\);/,
    "active_work_projection must redraw Active Work overview immediately",
  );
  assert.match(
    activeProjectionCase[0],
    /workspaceOverviewSurface\.renderWindows\(\);/,
    "active_work_projection must still refresh Workspace Overview windows",
  );
});

test("Board workspace id sync avoids stringify and reuses active Work id cache", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  assert.match(
    appSource,
    /let\s+currentProjectWorkspaceKey\s*=/,
    "app.js must track a stable key for the current Board workspace id set",
  );
  assert.match(
    appSource,
    /let\s+activeWorkProjectionWorkspaceIds\s*=/,
    "app.js must cache active Work projection workspace ids",
  );
  assert.match(
    appSource,
    /function\s+syncCurrentProjectWorkspaceIds\s*\(/,
    "app.js must centralize Board workspace id synchronization",
  );
  assert.doesNotMatch(
    renderAppStateBody,
    /JSON\.stringify\s*\(/,
    "workspace_state renderAppState must not serialize workspace id arrays for Board sync",
  );
  assert.match(
    renderAppStateBody,
    /syncCurrentProjectWorkspaceIds\s*\(\s*deriveCurrentProjectWorkspaceIds\s*\(\s*tab\?\.workspace\s*\|\|\s*\{\}\s*\)\s*,?\s*\)/,
    "renderAppState must route Board workspace id updates through the shared sync helper",
  );

  const deriveBody = extractFunctionBody(appSource, "deriveCurrentProjectWorkspaceIds");
  const cachedProjectionIndex = deriveBody.indexOf("activeWorkProjectionWorkspaceIds");
  const workspaceAgentsIndex = deriveBody.indexOf("workspaceState?.workspace?.agents");
  assert.ok(
    cachedProjectionIndex >= 0,
    "deriveCurrentProjectWorkspaceIds must read cached active Work projection ids",
  );
  assert.ok(
    workspaceAgentsIndex >= 0,
    "deriveCurrentProjectWorkspaceIds must keep the workspace-agent fallback",
  );
  assert.ok(
    cachedProjectionIndex < workspaceAgentsIndex,
    "cached active Work projection ids must be used before walking workspace agents",
  );
  assert.doesNotMatch(
    deriveBody,
    /activeWorkProjection\.active_works\.map/,
    "workspace_state id derivation must not remap active_work_projection on every render",
  );

  const receiveBody = extractFunctionBody(appSource, "receive");
  const activeProjectionCase = receiveBody.match(
    /case\s+"active_work_projection":[\s\S]*?break;/,
  );
  assert.ok(activeProjectionCase, "expected active_work_projection receive case");
  const cacheIndex = activeProjectionCase[0].indexOf(
    "cacheActiveWorkProjectionWorkspaceIds(activeWorkProjection)",
  );
  const syncIndex = activeProjectionCase[0].indexOf("syncCurrentProjectWorkspaceIds(");
  assert.ok(
    cacheIndex >= 0,
    "active_work_projection must update the cached active Work id set",
  );
  assert.ok(
    syncIndex > cacheIndex,
    "active_work_projection must sync Board ids after refreshing the cache",
  );
  assert.doesNotMatch(
    activeProjectionCase[0],
    /currentProjectWorkspaceId\s*=\s*deriveCurrentProjectWorkspaceIds/,
    "active_work_projection must not bypass the shared sync helper",
  );

  const syncBody = extractFunctionBody(appSource, "syncCurrentProjectWorkspaceIds");
  assert.match(
    syncBody,
    /workIdsKey\s*\(/,
    "sync helper must compare stable Board workspace id keys",
  );
  assert.match(
    syncBody,
    /refreshBoardCurrentWorkspaceId\s*\(\s*\)/,
    "sync helper must still refresh Board states when the id set changes",
  );
});

test("workspace_state hot path gates Project Tabs redraw by tab shell key", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  assert.match(
    appSource,
    /let\s+renderedProjectTabsKey\s*=/,
    "app.js must track the last rendered Project Tabs shell key",
  );
  assert.match(
    appSource,
    /function\s+projectTabsRenderKey\s*\(/,
    "app.js must define a Project Tabs shell key helper",
  );
  assert.match(
    renderAppStateBody,
    /projectTabsRenderKey\s*\(\s*appState\s*\)/,
    "renderAppState must derive the next Project Tabs shell key from appState",
  );
  assert.match(
    renderAppStateBody,
    /renderedProjectTabsKey[\s\S]*!==[\s\S]*nextProjectTabsKey[\s\S]*renderProjectTabs\(\)/,
    "renderAppState must redraw Project Tabs only when the shell key changes",
  );
});

test("Project Tabs shell key ignores workspace geometry but includes tab identity", () => {
  const keyBody = extractFunctionBody(appSource, "projectTabsRenderKey");
  assert.match(
    keyBody,
    /active_tab_id/,
    "Project Tabs shell key must include active_tab_id",
  );
  for (const field of ["id", "title", "project_root"]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Project Tabs shell key must include tab ${field}`,
    );
  }
  for (const workspaceField of ["workspace", "windows", "geometry", "viewport"]) {
    assert.doesNotMatch(
      keyBody,
      new RegExp(`\\b${workspaceField}\\b`),
      `Project Tabs shell key must ignore ${workspaceField}`,
    );
  }
});

test("Project Tabs shell key avoids JSON stringify allocation", () => {
  const keyBody = extractFunctionBody(appSource, "projectTabsRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Project Tabs shell key must append primitive fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Project Tabs shell key must not serialize an object graph on every workspace_state",
  );
  assert.doesNotMatch(
    keyBody,
    /\.map\s*\(/,
    "Project Tabs shell key must not allocate a mapped tab array",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+tab\s+of\s+tabs\s*\)/,
    "Project Tabs shell key must iterate tabs directly",
  );
});

test("hidden project picker does not rebuild Recent Projects on workspace_state", () => {
  const renderProjectPickerBody = extractFunctionBody(appSource, "renderProjectPicker");
  assert.match(
    appSource,
    /let\s+renderedRecentProjectsKey\s*=/,
    "app.js must track the visible picker Recent Projects render key",
  );
  assert.match(
    appSource,
    /let\s+renderedRecentProjectsMenuKey\s*=/,
    "app.js must track the split-menu Recent Projects render key separately",
  );
  assert.match(
    appSource,
    /function\s+recentProjectsRenderKey\s*\(/,
    "app.js must define a Recent Projects render key helper",
  );
  assert.match(
    renderProjectPickerBody,
    /if\s*\(\s*shouldShow\s*\)[\s\S]*renderRecentProjects\(\)/,
    "renderProjectPicker must render Recent Projects only when the picker is visible",
  );
  assert.doesNotMatch(
    renderProjectPickerBody.replace(/if\s*\(\s*shouldShow\s*\)[\s\S]*?renderRecentProjects\(\)\s*;?/, ""),
    /renderRecentProjects\(\)/,
    "renderProjectPicker must not also call renderRecentProjects unconditionally",
  );
});

test("Recent Projects render key ignores workspace state and split menu refreshes on open", () => {
  const keyBody = extractFunctionBody(appSource, "recentProjectsRenderKey");
  for (const field of ["title", "kind", "path"]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Recent Projects key must include ${field}`,
    );
  }
  for (const workspaceField of ["workspace", "windows", "geometry", "viewport"]) {
    assert.doesNotMatch(
      keyBody,
      new RegExp(`\\b${workspaceField}\\b`),
      `Recent Projects key must ignore ${workspaceField}`,
    );
  }
  const openMenuBody = extractFunctionBody(appSource, "openOpenProjectMenu");
  assert.match(
    openMenuBody,
    /renderRecentProjectsIntoMenu\s*\(\s*\{\s*force:\s*true\s*\}\s*\)/,
    "Open Project split menu must force-refresh Recent Projects from current appState when opened",
  );
});

test("viewport-only workspace_state skips unchanged window reconciliation", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    appSource,
    /let\s+renderedWorkspaceWindowsKey\s*=/,
    "app.js must track the last reconciled Workspace Windows shell key",
  );
  assert.match(
    appSource,
    /function\s+workspaceWindowsRenderKey\s*\(/,
    "app.js must define a Workspace Windows render key helper",
  );
  assert.match(
    appSource,
    /let\s+viewportDomApplied\s*=\s*false\s*;/,
    "app.js must track whether the viewport DOM has been initialized",
  );

  const nextViewportIndex = renderWorkspaceBody.indexOf(
    "const nextViewport = viewportSyncState.applyServerViewport",
  );
  const viewportChangedIndex = renderWorkspaceBody.indexOf(
    "const viewportChanged = !sameViewportValues(viewport, nextViewport);",
  );
  const assignViewportIndex = renderWorkspaceBody.indexOf("viewport = nextViewport;");
  const applyViewportIndex = renderWorkspaceBody.indexOf("applyViewport();");
  const keyIndex = renderWorkspaceBody.indexOf(
    "const nextWorkspaceWindowsKey = workspaceWindowsRenderKey(workspace);",
  );
  const guardIndex = renderWorkspaceBody.indexOf(
    "if (renderedWorkspaceWindowsKey === nextWorkspaceWindowsKey)",
  );
  const classifyIndex = renderWorkspaceBody.indexOf(
    "classifyProjectWindowVisibility",
  );
  const ensureIndex = renderWorkspaceBody.indexOf("ensureWindow(windowData)");
  const focusIndex = renderWorkspaceBody.indexOf("focusWindowLocally(topmostId)");
  const applyCalls = [...renderWorkspaceBody.matchAll(/applyViewport\(\);/g)];

  assert.notEqual(
    nextViewportIndex,
    -1,
    "renderWorkspace must store the applied server viewport before DOM writes",
  );
  assert.ok(
    viewportChangedIndex > nextViewportIndex,
    "renderWorkspace must compare the current viewport with the applied server viewport",
  );
  assert.ok(
    assignViewportIndex > viewportChangedIndex,
    "renderWorkspace must compare before replacing the current viewport reference",
  );
  assert.equal(
    applyCalls.length,
    1,
    "renderWorkspace must keep server viewport DOM writes behind one guarded apply call",
  );
  assert.ok(
    applyViewportIndex > assignViewportIndex,
    "changed server viewport must assign the new viewport before applying DOM writes",
  );
  assert.match(
    renderWorkspaceBody.slice(assignViewportIndex, keyIndex),
    /if\s*\(\s*!viewportDomApplied\s*\|\|\s*viewportChanged\s*\)\s*\{\s*applyViewport\(\);\s*\}/,
    "unchanged server viewport must skip applyViewport only after the first DOM apply",
  );
  assert.ok(keyIndex > applyViewportIndex, "window key guard must run after viewport");
  assert.ok(guardIndex > keyIndex, "renderWorkspace must guard on the window key");
  assert.ok(
    guardIndex < classifyIndex && guardIndex < ensureIndex && guardIndex < focusIndex,
    "unchanged window key must return before reconciliation and focus activation",
  );
  assert.match(
    renderWorkspaceBody.slice(guardIndex, classifyIndex),
    /return\s*;/,
    "unchanged window key guard must return before reconciliation",
  );
});

test("maximized viewport sync is coalesced across unchanged workspace_state events", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  const schedulerBody = extractFunctionBody(
    appSource,
    "scheduleMaximizedWindowsToViewportSync",
  );

  assert.match(
    appSource,
    /let\s+maximizedViewportSyncFrame\s*=\s*null\s*;/,
    "app.js must track a pending maximized viewport sync frame",
  );
  assert.match(
    schedulerBody,
    /if\s*\(\s*maximizedViewportSyncFrame\s*!==\s*null\s*\)[\s\S]*?return\s*;/,
    "maximized viewport sync scheduler must coalesce duplicate pending requests",
  );
  assert.match(
    schedulerBody,
    /maximizedViewportSyncFrame\s*=\s*requestAnimationFrame\s*\(\s*\(\s*\)\s*=>\s*\{/,
    "maximized viewport sync scheduler must own the animation-frame reservation",
  );
  assert.match(
    schedulerBody,
    /maximizedViewportSyncFrame\s*=\s*null\s*;[\s\S]*?syncMaximizedWindowsToViewport\s*\(\s*\)/,
    "maximized viewport sync scheduler must clear the pending handle before running the existing sync body",
  );
  assert.match(
    renderWorkspaceBody,
    /scheduleMaximizedWindowsToViewportSync\s*\(\s*\)/,
    "renderWorkspace must route maximized sync through the coalesced scheduler",
  );
  assert.doesNotMatch(
    renderWorkspaceBody,
    /requestAnimationFrame\s*\(\s*syncMaximizedWindowsToViewport\s*\)/,
    "renderWorkspace must not schedule raw maximized sync frames per workspace_state",
  );
});

test("clamped no-op zoom returns before local viewport apply and persist work", () => {
  const zoomBody = extractFunctionBody(appSource, "zoomCanvasAt");
  assert.match(
    appSource,
    /function\s+sameViewportValues\s*\(/,
    "app.js must define a semantic viewport equality helper",
  );
  const nextViewportIndex = zoomBody.indexOf("const nextViewport = {");
  const guardIndex = zoomBody.indexOf("if (sameViewportValues(viewport, nextViewport))");
  const assignIndex = zoomBody.indexOf("viewport = nextViewport;");
  const recordIndex = zoomBody.indexOf("recordLocalViewportEdit();");
  const applyIndex = zoomBody.indexOf("applyViewport();");
  const persistIndex = zoomBody.indexOf("persistViewport();");

  assert.notEqual(nextViewportIndex, -1, "zoomCanvasAt must compute next viewport first");
  assert.ok(guardIndex > nextViewportIndex, "no-op guard must inspect the computed viewport");
  assert.ok(
    guardIndex < assignIndex
      && guardIndex < recordIndex
      && guardIndex < applyIndex
      && guardIndex < persistIndex,
    "clamped no-op zoom must return before assignment, local edit tracking, viewport writes, and persist scheduling",
  );
  assert.match(
    zoomBody.slice(guardIndex, assignIndex),
    /return\s*;/,
    "same viewport guard must return before changed-zoom apply/persist sequence",
  );
  assert.ok(
    assignIndex < recordIndex && recordIndex < applyIndex && applyIndex < persistIndex,
    "changed zoom must still assign next viewport before the existing apply/persist sequence",
  );
});

test("Workspace Windows render key ignores viewport and includes window shell fields", () => {
  const keyBody = extractFunctionBody(appSource, "workspaceWindowsRenderKey");
  assert.match(
    keyBody,
    /active_tab_id/,
    "Workspace Windows key must include active tab identity",
  );
  assert.match(
    keyBody,
    /allProjectWindowIds\s*\(\s*\)/,
    "Workspace Windows key must include all project window ids for stale mounted window cleanup",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "minimized",
    "maximized",
    "z_index",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Workspace Windows key must include ${field}`,
    );
  }
  assert.doesNotMatch(
    keyBody,
    /\bviewport\b/,
    "Workspace Windows key must ignore viewport-only state",
  );
});

test("Workspace Windows render key avoids JSON stringify allocation", () => {
  const keyBody = extractFunctionBody(appSource, "workspaceWindowsRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must provide a primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Workspace Windows key must append primitive key parts directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Workspace Windows key must not serialize a nested object graph",
  );
  assert.doesNotMatch(
    keyBody,
    /\bwindows\.map\s*\(/,
    "Workspace Windows key must not allocate per-window map results",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+windowData\s+of\s+windows\s*\)/,
    "Workspace Windows key must walk windows with a direct loop",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+windowId\s+of\s+allProjectWindowIds\s*\(\s*\)\s*\)/,
    "Workspace Windows key must include mounted project window ids with a direct loop",
  );
});

test("Window List skips unchanged row rebuilds after updating open state", () => {
  const renderWindowListBody = extractFunctionBody(appSource, "renderWindowList");
  assert.match(
    appSource,
    /let\s+renderedWindowListKey\s*=/,
    "app.js must track the last rendered Window List row key",
  );
  assert.match(
    appSource,
    /function\s+windowListRenderKey\s*\(/,
    "app.js must define a Window List render key helper",
  );

  const hiddenIndex = renderWindowListBody.indexOf("windowListPanel.hidden");
  const ariaIndex = renderWindowListBody.indexOf("aria-expanded");
  const keyIndex = renderWindowListBody.indexOf(
    "const nextWindowListKey = windowListRenderKey();",
  );
  const closedGuardIndex = renderWindowListBody.indexOf("if (!windowListOpen)");
  const closedSentinelIndex = renderWindowListBody.indexOf(
    'renderedWindowListKey = "__closed__";',
  );
  const guardIndex = renderWindowListBody.indexOf(
    "if (renderedWindowListKey === nextWindowListKey)",
  );
  const clearIndex = renderWindowListBody.indexOf("windowListPanel.innerHTML = \"\";");

  assert.notEqual(hiddenIndex, -1, "renderWindowList must update panel hidden state");
  assert.notEqual(ariaIndex, -1, "renderWindowList must update trigger aria-expanded");
  assert.notEqual(closedGuardIndex, -1, "renderWindowList must have a closed fast path");
  assert.notEqual(
    closedSentinelIndex,
    -1,
    "closed Window List fast path must store a stable sentinel key",
  );
  assert.ok(
    closedGuardIndex > hiddenIndex && closedGuardIndex > ariaIndex,
    "closed Window List fast path must run after chrome hidden/expanded updates",
  );
  assert.ok(
    closedGuardIndex < keyIndex,
    "closed Window List fast path must return before key generation",
  );
  assert.ok(
    closedSentinelIndex > closedGuardIndex && closedSentinelIndex < keyIndex,
    "closed Window List fast path must store the sentinel before any key generation",
  );
  assert.match(
    renderWindowListBody.slice(closedGuardIndex, keyIndex),
    /return\s*;/,
    "closed Window List fast path must return before windowListRenderKey()",
  );
  assert.ok(
    keyIndex > hiddenIndex && keyIndex > ariaIndex,
    "open Window List key must run after open state updates",
  );
  assert.ok(guardIndex > keyIndex, "renderWindowList must guard on the Window List key");
  assert.ok(
    guardIndex < clearIndex,
    "unchanged Window List key must return before row clearing/rebuild",
  );
  assert.match(
    renderWindowListBody.slice(guardIndex, clearIndex),
    /return\s*;/,
    "unchanged Window List key guard must return before clearing rows",
  );

  const toggleWindowListBody = extractFunctionBody(appSource, "toggleWindowList");
  const invalidateIndex = toggleWindowListBody.indexOf("renderedWindowListKey = \"\";");
  const renderIndex = toggleWindowListBody.indexOf("renderWindowList();");
  const requestIndex = toggleWindowListBody.indexOf("requestWindowList();");
  assert.notEqual(invalidateIndex, -1, "toggleWindowList must invalidate the Window List key");
  assert.ok(
    invalidateIndex < renderIndex && renderIndex < requestIndex,
    "opening Window List must render current rows before requesting backend entries",
  );
});

test("Window List render key ignores viewport and includes row shell fields", () => {
  const keyBody = extractFunctionBody(appSource, "windowListRenderKey");
  assert.match(
    keyBody,
    /active_tab_id/,
    "Window List key must include active tab identity",
  );
  assert.match(
    keyBody,
    /windowListEntries/,
    "Window List key must include server-provided window_list entries",
  );
  assert.match(
    keyBody,
    /activeWorkspace\s*\(\s*\)/,
    "Window List key must include active workspace window identity/order",
  );
  assert.match(
    keyBody,
    /runtimeStateForWindow\s*\(/,
    "Window List key must include runtime state used by status chips",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "minimized",
    "maximized",
    "z_index",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Window List key must include ${field}`,
    );
  }
  assert.doesNotMatch(
    keyBody,
    /\bviewport\b/,
    "Window List key must ignore viewport-only state",
  );
});

test("Static project chrome renderers guard unchanged DOM writes", () => {
  assert.match(
    appSource,
    /let\s+renderedAppVersionLabel\s*=/,
    "app.js must track the last rendered app version label",
  );
  assert.match(
    appSource,
    /let\s+renderedProjectPickerKey\s*=/,
    "app.js must track the last rendered Project Picker key",
  );
  assert.match(
    appSource,
    /let\s+renderedProjectOnboardingKey\s*=/,
    "app.js must track the last rendered Project Onboarding key",
  );
  assert.match(
    appSource,
    /let\s+renderedActionAvailabilityKey\s*=/,
    "app.js must track the last rendered action availability key",
  );

  const renderAppVersionBody = extractFunctionBody(appSource, "renderAppVersion");
  const versionGuardIndex = renderAppVersionBody.indexOf(
    "if (renderedAppVersionLabel === label)",
  );
  const versionHiddenIndex = renderAppVersionBody.indexOf("appVersionLabel.hidden");
  assert.ok(
    versionGuardIndex !== -1 && versionGuardIndex < versionHiddenIndex,
    "renderAppVersion must return before unchanged label DOM writes",
  );

  const renderProjectPickerBody = extractFunctionBody(appSource, "renderProjectPicker");
  const pickerKeyIndex = renderProjectPickerBody.indexOf(
    "const nextProjectPickerKey = projectPickerRenderKey(activeTab);",
  );
  const pickerGuardIndex = renderProjectPickerBody.indexOf(
    "if (renderedProjectPickerKey === nextProjectPickerKey)",
  );
  const pickerClassIndex = renderProjectPickerBody.indexOf(
    "projectPicker.classList.toggle",
  );
  assert.ok(
    pickerKeyIndex !== -1 &&
      pickerGuardIndex > pickerKeyIndex &&
      pickerGuardIndex < pickerClassIndex,
    "renderProjectPicker must return before unchanged picker DOM writes",
  );

  const renderProjectOnboardingBody = extractFunctionBody(appSource, "renderProjectOnboarding");
  const onboardingKeyIndex = renderProjectOnboardingBody.indexOf(
    "const nextProjectOnboardingKey = projectOnboardingRenderKey(tab);",
  );
  const onboardingGuardIndex = renderProjectOnboardingBody.indexOf(
    "if (renderedProjectOnboardingKey === nextProjectOnboardingKey)",
  );
  const onboardingClassIndex = renderProjectOnboardingBody.indexOf(
    "projectOnboarding.classList",
  );
  assert.ok(
    onboardingKeyIndex !== -1 &&
      onboardingGuardIndex > onboardingKeyIndex &&
      onboardingGuardIndex < onboardingClassIndex,
    "renderProjectOnboarding must return before unchanged onboarding DOM writes",
  );

  const updateActionAvailabilityBody = extractFunctionBody(appSource, "updateActionAvailability");
  const actionKeyIndex = updateActionAvailabilityBody.indexOf(
    "const nextActionAvailabilityKey = actionAvailabilityRenderKey(activeTab);",
  );
  const actionGuardIndex = updateActionAvailabilityBody.indexOf(
    "if (renderedActionAvailabilityKey === nextActionAvailabilityKey)",
  );
  const disabledIndex = updateActionAvailabilityBody.indexOf("addButton.disabled");
  assert.ok(
    actionKeyIndex !== -1 &&
      actionGuardIndex > actionKeyIndex &&
      actionGuardIndex < disabledIndex,
    "updateActionAvailability must return before unchanged disabled-state DOM writes",
  );
});

test("Static project chrome keys ignore workspace geometry and include visible transition state", () => {
  const pickerKeyBody = extractFunctionBody(appSource, "projectPickerRenderKey");
  assert.match(
    pickerKeyBody,
    /activeTab/,
    "Project Picker key must include visible active-tab state",
  );
  assert.match(
    pickerKeyBody,
    /projectError/,
    "Project Picker key must include picker error text",
  );
  assert.match(
    pickerKeyBody,
    /recentProjectsRenderKey\s*\(/,
    "Project Picker key must include Recent Projects only when visible",
  );

  const onboardingKeyBody = extractFunctionBody(appSource, "projectOnboardingRenderKey");
  for (const field of ["kind", "project_root"]) {
    assert.match(
      onboardingKeyBody,
      new RegExp(`\\b${field}\\b`),
      `Project Onboarding key must include ${field}`,
    );
  }

  const actionKeyBody = extractFunctionBody(appSource, "actionAvailabilityRenderKey");
  assert.match(
    actionKeyBody,
    /activeTab/,
    "Action Availability key must include active-tab availability",
  );

  for (const keyBody of [pickerKeyBody, onboardingKeyBody, actionKeyBody]) {
    for (const workspaceField of ["viewport", "windows", "geometry"]) {
      assert.doesNotMatch(
        keyBody,
        new RegExp(`\\b${workspaceField}\\b`),
        `Static project chrome keys must ignore ${workspaceField}`,
      );
    }
  }
});

test("Static project chrome keys avoid JSON stringify allocation", () => {
  for (const name of ["projectPickerRenderKey", "projectOnboardingRenderKey"]) {
    const keyBody = extractFunctionBody(appSource, name);
    assert.match(
      keyBody,
      /appendRenderKeyPart\s*\(/,
      `${name} must append primitive fields directly`,
    );
    assert.doesNotMatch(
      keyBody,
      /JSON\.stringify\s*\(/,
      `${name} must not serialize an object graph on every workspace_state`,
    );
  }
});

test("Static project chrome reuses the renderAppState active tab lookup", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  const tabLookupIndex = renderAppStateBody.indexOf("const tab = activeProjectTab();");
  const pickerCallIndex = renderAppStateBody.indexOf("renderProjectPicker(tab);");
  const actionCallIndex = renderAppStateBody.indexOf("updateActionAvailability(tab);");
  const onboardingCallIndex = renderAppStateBody.indexOf("renderProjectOnboarding(tab);");
  const workspaceCallIndex = renderAppStateBody.indexOf(
    "renderWorkspace(tab?.workspace || emptyWorkspace());",
  );
  assert.ok(
    tabLookupIndex >= 0 &&
      pickerCallIndex > tabLookupIndex &&
      actionCallIndex > tabLookupIndex &&
      onboardingCallIndex > tabLookupIndex &&
      workspaceCallIndex > tabLookupIndex,
    "renderAppState must resolve the active tab once and pass it through static chrome and workspace renderers",
  );

  const pickerKeyBody = extractFunctionBody(appSource, "projectPickerRenderKey");
  const actionKeyBody = extractFunctionBody(appSource, "actionAvailabilityRenderKey");
  for (const keyBody of [pickerKeyBody, actionKeyBody]) {
    assert.doesNotMatch(
      keyBody,
      /activeProjectTab\s*\(\s*\)/,
      "static chrome key helpers must not rescan activeProjectTab on the workspace_state hot path",
    );
  }
});

test("Per-window renderer guards unchanged DOM writes after mount and preset sync", () => {
  assert.match(
    appSource,
    /const\s+renderedWindowElementKeys\s*=\s*new\s+Map\s*\(/,
    "app.js must track per-window element render keys",
  );
  assert.match(
    appSource,
    /function\s+windowElementRenderKey\s*\(/,
    "app.js must define a per-window element render key helper",
  );

  const ensureWindowBody = extractFunctionBody(appSource, "ensureWindow");
  const mountIndex = ensureWindowBody.indexOf("mountWindowBody(windowData, element);");
  const keyIndex = ensureWindowBody.indexOf(
    "const nextWindowElementKey = windowElementRenderKey(windowData);",
  );
  const guardIndex = ensureWindowBody.indexOf(
    "if (renderedWindowElementKeys.get(windowData.id) === nextWindowElementKey)",
  );
  const storeIndex = ensureWindowBody.indexOf(
    "renderedWindowElementKeys.set(windowData.id, nextWindowElementKey);",
  );
  const renderKeyCalls = [
    ...ensureWindowBody.matchAll(/windowElementRenderKey\(windowData\)/g),
  ];
  const titleIndex = ensureWindowBody.indexOf(".title-text");
  const roleBadgeIndex = ensureWindowBody.indexOf("setWindowRoleBadge");
  const tabsIndex = ensureWindowBody.indexOf("renderWindowTabs");
  const agentColorIndex = ensureWindowBody.indexOf("agentColor");
  const classIndex = ensureWindowBody.indexOf('classList.toggle("minimized"');
  const styleIndex = ensureWindowBody.indexOf("element.style.zIndex");
  const statusIndex = ensureWindowBody.indexOf("applyStatus");
  const fitIndex = ensureWindowBody.indexOf("scheduleTerminalFit");

  assert.notEqual(mountIndex, -1, "ensureWindow must keep preset/body mount logic");
  assert.ok(keyIndex > mountIndex, "per-window key must run after preset/body mounting");
  assert.ok(guardIndex > keyIndex, "ensureWindow must guard on the per-window key");
  assert.equal(
    renderKeyCalls.length,
    1,
    "ensureWindow must reuse the computed per-window key instead of recomputing it",
  );
  for (const [label, index] of [
    ["title", titleIndex],
    ["role badge", roleBadgeIndex],
    ["tab strip", tabsIndex],
    ["agent color", agentColorIndex],
    ["class", classIndex],
    ["style", styleIndex],
    ["status", statusIndex],
    ["terminal fit", fitIndex],
  ]) {
    assert.notEqual(index, -1, `ensureWindow must still contain ${label} writes`);
    assert.ok(
      guardIndex < index,
      `unchanged per-window key must return before ${label} writes`,
    );
  }
  assert.notEqual(
    storeIndex,
    -1,
    "ensureWindow must store the previously computed per-window key",
  );
  assert.ok(
    statusIndex < storeIndex,
    "ensureWindow must store changed keys after status/detail normalization",
  );
  assert.match(
    ensureWindowBody.slice(guardIndex, titleIndex),
    /return\s*;/,
    "unchanged per-window key guard must return before window DOM writes",
  );
});

test("Per-window render key covers DOM shell fields and removal cleanup", () => {
  const keyBody = extractFunctionBody(appSource, "windowElementRenderKey");
  assert.match(
    keyBody,
    /windowDisplayTitle\s*\(/,
    "per-window key must include displayed title",
  );
  assert.match(
    keyBody,
    /windowTitleTooltip\s*\(/,
    "per-window key must include title tooltip",
  );
  assert.match(
    keyBody,
    /windowGroupId\s*\(\s*windowData\s*\)/,
    "per-window key must derive the tab group for tab strip inputs",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "per-window key must append primitive key fields directly",
  );
  assert.match(
    keyBody,
    /detailMap\.get\s*\(\s*windowData\.id\s*\)/,
    "per-window key must include status detail text",
  );
  assert.match(
    keyBody,
    /maximizedGeometry\s*\(\s*visibleBounds\s*\(\s*\)\s*,\s*viewport\.zoom\s*\)/,
    "per-window key must include viewport-relative maximized fill",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "minimized",
    "maximized",
    "z_index",
    "tab_group_id",
    "tab_group_active",
    "maximized_fill",
    "tabs",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `per-window key must include ${field}`,
    );
  }

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  const cleanupIndex = renderWorkspaceBody.indexOf("renderedWindowElementKeys.delete(windowId);");
  const removeIndex = renderWorkspaceBody.indexOf("element.remove();");
  assert.notEqual(cleanupIndex, -1, "window removal must clear per-window render key");
  assert.ok(
    cleanupIndex < removeIndex,
    "per-window render key cleanup must happen before element removal",
  );
});

test("Per-window render key avoids JSON stringify and mapped tab allocation", () => {
  const keyBody = extractFunctionBody(appSource, "windowElementRenderKey");
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "per-window key must not serialize an object graph for each changed window",
  );
  assert.doesNotMatch(
    keyBody,
    /windowTabsFor\s*\(\s*windowData\s*\)\.map\s*\(/,
    "per-window key must not allocate a mapped tab shell array",
  );
  assert.doesNotMatch(
    keyBody,
    /windowTabsFor\s*\(\s*windowData\s*\)/,
    "per-window key must avoid allocating a filtered tab array",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+tab\s+of\s+activeWorkspace\s*\(\s*\)\.windows\s*\|\|\s*\[\]\s*\)/,
    "per-window key must iterate workspace tab candidates directly",
  );
  assert.match(
    keyBody,
    /windowGroupId\s*\(\s*tab\s*\)\s*!==\s*tabGroupId[\s\S]+continue\s*;/,
    "per-window key must keep only tabs in the same group",
  );
  for (const field of [
    "runtime_state",
    "detail",
    "display_title",
    "title_tooltip",
    "role_badge",
    "geometry_revision",
    "maximized_fill",
    "tabs",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `per-window primitive key must keep ${field}`,
    );
  }
});

test("Focus class updates skip unchanged focus and avoid all-window scans", () => {
  const focusBody = extractFunctionBody(appSource, "focusWindowLocally");
  assert.match(
    focusBody,
    /const\s+targetElement\s*=\s*windowMap\.get\s*\(\s*windowId\s*\)/,
    "focusWindowLocally must resolve the requested window directly",
  );
  assert.match(
    focusBody,
    /focusedId\s*===\s*windowId[\s\S]+targetElement\?\.classList\.contains\s*\(\s*"focused"\s*\)[\s\S]+return\s*;/,
    "focusWindowLocally must return when focus is unchanged and the class is already applied",
  );
  assert.doesNotMatch(
    focusBody,
    /windowMap\.entries\s*\(/,
    "focusWindowLocally must not scan every mounted window",
  );
});

test("Focus class updates touch previous/current elements and keep no-topmost reset", () => {
  const focusBody = extractFunctionBody(appSource, "focusWindowLocally");
  assert.match(
    focusBody,
    /const\s+previousFocusedId\s*=\s*focusedId/,
    "focusWindowLocally must remember the previous focused window",
  );
  assert.match(
    focusBody,
    /focusedId\s*=\s*windowId/,
    "focusWindowLocally must update focusedId to the requested window",
  );
  assert.match(
    focusBody,
    /previousFocusedId\s*!==\s*windowId[\s\S]+previousElement\?\.classList\.remove\s*\(\s*"focused"\s*\)/,
    "focusWindowLocally must remove focus from the previous element only",
  );
  assert.match(
    focusBody,
    /targetElement\.classList\.add\s*\(\s*"focused"\s*\)/,
    "focusWindowLocally must add focus to the requested element",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    renderWorkspaceBody,
    /focusedId\s*=\s*null\s*;/,
    "renderWorkspace must still clear focusedId when no topmost active window exists",
  );
});

test("Workspace visibility classification reuses direct id sets", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    appSource,
    /function\s+workspaceWindowIdSet\s*\(/,
    "app.js must provide a direct-loop active window id set helper",
  );
  assert.match(
    appSource,
    /function\s+allProjectWindowIdSet\s*\(/,
    "app.js must provide a direct-loop all-project window id set helper",
  );
  assert.doesNotMatch(
    renderWorkspaceBody,
    /workspace\.windows\.map\s*\(\s*\(?\s*windowData\s*\)?\s*=>\s*windowData\.id\s*\)/,
    "renderWorkspace must not allocate an active window id array before classification",
  );
  assert.match(
    renderWorkspaceBody,
    /const\s+activeWindowIdSet\s*=\s*workspaceWindowIdSet\s*\(\s*workspace\s*\)/,
    "renderWorkspace must derive active window ids as a Set once",
  );
  assert.match(
    renderWorkspaceBody,
    /activeWindowIdSet\s*,[\s\S]*allProjectWindowIdSet:\s*allProjectWindowIdSet\s*\(\s*\)/,
    "renderWorkspace must pass id sets to classifyProjectWindowVisibility",
  );
  assert.match(
    renderWorkspaceBody,
    /topmostId\s*&&\s*activeWindowIdSet\.has\s*\(\s*topmostId\s*\)/,
    "renderWorkspace must reuse the active id set for topmost focus membership",
  );
});

test("Runtime status updates skip unchanged DOM and dependent surface writes", () => {
  assert.match(
    appSource,
    /const\s+renderedRuntimeStatusKeys\s*=\s*new\s+Map\s*\(/,
    "app.js must track per-window runtime status render keys",
  );
  assert.match(
    appSource,
    /function\s+windowRuntimeStatusRenderKey\s*\(/,
    "app.js must define a runtime status render key helper",
  );

  const statusBody = extractFunctionBody(appSource, "applyStatus");
  const keyIndex = statusBody.indexOf(
    "const nextRuntimeStatusKey = windowRuntimeStatusRenderKey(",
  );
  const guardIndex = statusBody.indexOf(
    "renderedRuntimeStatusKeys.get(windowId) === nextRuntimeStatusKey",
  );
  const chipIndex = statusBody.indexOf("chip.classList.remove(");
  const overlayIndex = statusBody.indexOf("const overlay = element.querySelector");
  const telemetryIndex = statusBody.indexOf("recomputeOperatorTelemetry");
  const windowListIndex = statusBody.lastIndexOf("renderWindowList()");
  const dotsIndex = statusBody.indexOf("refreshProjectTabDots()");

  assert.notEqual(keyIndex, -1, "applyStatus must compute a runtime status key");
  assert.notEqual(guardIndex, -1, "applyStatus must guard unchanged runtime status");
  for (const [label, index] of [
    ["status chip class writes", chipIndex],
    ["overlay lookup/writes", overlayIndex],
    ["telemetry recompute", telemetryIndex],
    ["Window List refresh", windowListIndex],
    ["project tab dot refresh", dotsIndex],
  ]) {
    assert.notEqual(index, -1, `applyStatus must still contain ${label}`);
    assert.ok(guardIndex < index, `unchanged runtime status must return before ${label}`);
  }
  assert.match(
    statusBody.slice(guardIndex, chipIndex),
    /return\s*;/,
    "unchanged runtime status guard must return before DOM/dependent-surface writes",
  );
});

test("Runtime status key covers state detail preset visibility and cleanup", () => {
  const keyBody = extractFunctionBody(appSource, "windowRuntimeStatusRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Runtime status key must append primitive fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Runtime status key must not serialize an object on every window_status event",
  );
  for (const pattern of [
    /windowId/,
    /windowMap\.has\s*\(\s*windowId\s*\)/,
    /runtimeState/,
    /effectiveDetail/,
    /windowData\?\.preset/,
    /shouldShowRuntimeStatus\s*\(/,
    /mapAgentTelemetryState\s*\(/,
  ]) {
    assert.match(keyBody, pattern, `runtime status key must include ${pattern}`);
  }

  const statusBody = extractFunctionBody(appSource, "applyStatus");
  const stateMapIndex = statusBody.indexOf("windowRuntimeStateMap.set(windowId, runtimeState);");
  const effectiveDetailIndex = statusBody.indexOf("const effectiveDetail = detailMap.get(windowId)");
  const keyIndex = statusBody.indexOf(
    "const nextRuntimeStatusKey = windowRuntimeStatusRenderKey(",
  );
  assert.ok(
    stateMapIndex !== -1 && stateMapIndex < keyIndex,
    "applyStatus must update runtime state before computing the status key",
  );
  assert.ok(
    effectiveDetailIndex !== -1 && effectiveDetailIndex < keyIndex,
    "applyStatus must compute effective detail before the status key",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  const cleanupIndex = renderWorkspaceBody.indexOf("renderedRuntimeStatusKeys.delete(windowId);");
  const removeIndex = renderWorkspaceBody.indexOf("element.remove();");
  assert.notEqual(cleanupIndex, -1, "window removal must clear runtime status render key");
  assert.ok(
    cleanupIndex < removeIndex,
    "runtime status render key cleanup must happen before element removal",
  );
});

test("Terminal output decode is deferred out of the receive path", () => {
  const batcherConfig = appSource.match(
    /const\s+terminalOutputBatcher\s*=\s*createTerminalOutputBatcher\(\{([\s\S]*?)\n\s{6}\}\);/,
  );
  assert.ok(batcherConfig, "app.js must configure the terminal output batcher");
  assert.match(
    batcherConfig[1],
    /mergeChunks:\s*\(\s*chunks\s*,\s*windowId\s*\)/,
    "terminal output batcher must merge encoded chunks during scheduled flush",
  );
  assert.match(
    batcherConfig[1],
    /decoderMap\.get\s*\(\s*windowId\s*\)/,
    "flush merge must use the per-window TextDecoder",
  );
  assert.match(
    batcherConfig[1],
    /chunks\s*\.\s*map[\s\S]*decodeBase64\s*\(\s*chunk\s*\)/,
    "flush merge must decode each encoded chunk in order",
  );

  const writeOutputBody = extractFunctionBody(appSource, "writeOutput");
  assert.match(
    writeOutputBody,
    /terminalOutputBatcher\.enqueue\(\s*windowId,\s*base64\s*\)/,
    "ready terminal output must enqueue encoded chunks without eager decode",
  );
  const readyEnqueueIndex = writeOutputBody.indexOf(
    "terminalOutputBatcher.enqueue(",
  );
  const readyPath = writeOutputBody.slice(readyEnqueueIndex);
  assert.doesNotMatch(
    readyPath,
    /decoder\.decode\s*\(|decodeBase64\s*\(/,
    "writeOutput ready path must not decode before the scheduled flush",
  );
});

test("Terminal output writes are gated while windows are hidden", () => {
  const batcherConfig = appSource.match(
    /const\s+terminalOutputBatcher\s*=\s*createTerminalOutputBatcher\(\{([\s\S]*?)\n\s{6}\}\);/,
  );
  assert.ok(batcherConfig, "app.js must configure the terminal output batcher");
  assert.match(
    batcherConfig[1],
    /canWrite:\s*canRefreshTerminalViewport/,
    "terminal output batcher must reuse the terminal visibility predicate before decode/write",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    renderWorkspaceBody,
    /onReveal:\s*\(\)\s*=>\s*\{[\s\S]*?terminalOutputBatcher\.schedulePending\(windowId\)[\s\S]*?rearmPendingTerminalViewportRefresh\(windowId\)[\s\S]*?scheduleTerminalFocusActivation\(windowId\)/,
    "hidden project-tab reveal must re-arm pending output before viewport/focus activation",
  );
  assert.match(
    renderWorkspaceBody,
    /onReveal:\s*\(\)\s*=>\s*\{[\s\S]*?terminalOutputBatcher\.schedulePending\(windowData\.id\)[\s\S]*?rearmPendingTerminalViewportRefresh\(windowData\.id\)[\s\S]*?scheduleTerminalFocusActivation\(windowData\.id\)/,
    "hidden window-tab reveal must re-arm pending output before viewport/focus activation",
  );
});

test("Operator telemetry skips unchanged counts before DOM writes", () => {
  assert.match(
    appSource,
    /let\s+renderedOperatorTelemetryKey\s*=\s*""/,
    "app.js must track the last rendered telemetry counts key",
  );
  assert.match(
    appSource,
    /function\s+operatorTelemetryRenderKey\s*\(/,
    "app.js must define a telemetry counts key helper",
  );
  assert.match(
    appSource,
    /function\s+applyOperatorTelemetryCounts\s*\(/,
    "app.js must route telemetry DOM writes through a guarded helper",
  );

  const helperBody = extractFunctionBody(appSource, "applyOperatorTelemetryCounts");
  const keyIndex = helperBody.indexOf("const nextOperatorTelemetryKey = operatorTelemetryRenderKey(counts);");
  const guardIndex = helperBody.indexOf(
    "renderedOperatorTelemetryKey === nextOperatorTelemetryKey",
  );
  const applyIndex = helperBody.indexOf("window.__operatorShell.applyTelemetryCounts(counts)");
  const storeIndex = helperBody.indexOf("renderedOperatorTelemetryKey = nextOperatorTelemetryKey");

  assert.notEqual(keyIndex, -1, "telemetry helper must compute a stable key");
  assert.ok(guardIndex > keyIndex, "telemetry helper must guard after key computation");
  assert.ok(
    guardIndex < applyIndex,
    "unchanged telemetry counts must return before applyTelemetryCounts DOM writes",
  );
  assert.ok(
    applyIndex < storeIndex,
    "telemetry key should be stored after applyTelemetryCounts succeeds",
  );
  assert.match(
    helperBody.slice(guardIndex, applyIndex),
    /return\s*;/,
    "unchanged telemetry guard must return before DOM writes",
  );
});

test("Operator telemetry key covers status strip fields and branch telemetry uses guard", () => {
  const keyBody = extractFunctionBody(appSource, "operatorTelemetryRenderKey");
  for (const field of [
    "active",
    "idle",
    "blocked",
    "done",
    "agents",
    "branches",
    "git",
    "hooks",
    "layers",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `telemetry key must include ${field}`,
    );
  }

  const telemetryBody = extractFunctionBody(appSource, "recomputeOperatorTelemetry");
  assert.match(
    telemetryBody,
    /applyOperatorTelemetryCounts\s*\(\s*counts\s*\)/,
    "runtime telemetry must use the guarded telemetry helper",
  );
  assert.doesNotMatch(
    telemetryBody,
    /applyTelemetryCounts\s*\(\s*counts\s*\)/,
    "runtime telemetry must not call applyTelemetryCounts directly",
  );
  assert.match(
    appSource,
    /case\s+"branch_entries"[\s\S]+applyOperatorTelemetryCounts\s*\(\s*\{\s*branches:\s*branchesCount,\s*git:\s*branchesCount,\s*\}\s*\)/,
    "branch telemetry must use the guarded telemetry helper",
  );
});

test("Sidebar Layers Agents counter filters non-agent preset windows", () => {
  // SPEC-2356 follow-up: recomputeOperatorTelemetry walked windowMap.values()
  // without checking preset, so every workspace window with data-agent-state
  // (Board / Workspace / Logs / Branches / etc.) inflated counts.agents and
  // the Sidebar Layers "Agents" row showed e.g. 4 when only 2 agent panes
  // were live. The DOM walk must scope to presets that represent agent panes.
  const fnMatch = appSource.match(
    /function\s+recomputeOperatorTelemetry[\s\S]*?(?=\n\s+function\s+\w)/,
  );
  assert.ok(
    fnMatch,
    "expected recomputeOperatorTelemetry definition in app.js",
  );
  const body = fnMatch[0];
  assert.match(
    body,
    /windowMap\.entries\(\)/,
    "recomputeOperatorTelemetry must iterate windowMap.entries() so it can resolve each window's preset before counting it as an agent",
  );
  assert.match(
    body,
    /presetSupportsWaitingStatus/,
    "recomputeOperatorTelemetry must filter via presetSupportsWaitingStatus(preset) so non-agent windows do not inflate the Sidebar Agents counter",
  );
});

test("Workspace sidebar exposes active work and per-agent overview", () => {
  assert.ok(
    document.querySelector("#op-active-work"),
    "expected Workspace shell to expose an Active Works overview region",
  );
  assert.ok(
    document.querySelector("#op-active-work-agents"),
    "expected Workspace shell to expose a per-Work list region",
  );
  assert.match(
    appSource,
    /function\s+activeWorkItemsFromProjection\(\)[\s\S]+active_works/,
    "expected frontend to render plural active_works data, not only a single aggregate projection",
  );
  assert.match(
    appSource,
    /op-work-card[\s\S]+renderActiveWorkAgentCard\(agent\)/,
    "expected Active Works rows to render their live Agent cards",
  );
  assert.match(
    appSource,
    /function\s+renderActiveWorkAgentCard\(agent\)[\s\S]+op-agent-card[\s\S]+last_board_entry_id/,
    "expected Active Works rows to preserve agent board linkage for handoff/debugging",
  );
});

test("Workspace sidebar keeps Quick before the expanding active work list", () => {
  const sections = Array.from(document.querySelectorAll(".op-sidebar > .op-sidebar__section"));
  const headings = sections.map((section) =>
    section.querySelector(".op-sidebar__heading span")?.textContent?.trim(),
  );
  assert.deepEqual(
    headings.slice(0, 3),
    ["Layers", "Quick", "Active Works"],
    "Quick must stay above Active Works so Work cards do not push it off-screen",
  );
});

test("workspace windows expose role badges and hide panel runtime chips", () => {
  assert.match(
    appSource,
    /function\s+presetRoleLabel\(preset\)/,
    "expected a shared preset role label helper",
  );
  assert.match(
    appSource,
    /class="window-role-badge"/,
    "expected titlebar template to include a role badge surface",
  );
  assert.match(
    appSource,
    /function\s+shouldShowRuntimeStatus\(windowData\)/,
    "expected runtime chip visibility to be centralized",
  );
  assert.match(
    appSource,
    /runtimeChip\.hidden\s*=\s*!shouldShowRuntimeStatus\(windowData\)/,
    "expected non-terminal panels to hide the runtime status chip",
  );
  assert.match(
    inlineStyle,
    /\.status-chip\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden runtime chips must stay hidden even though .status-chip uses inline-flex",
  );
  assert.match(
    appSource,
    /window-list-role/,
    "expected window list rows to include a role badge",
  );
});

test("Agent title role badges resolve runtime identity instead of generic presets", () => {
  assert.match(
    appSource,
    /const\s+AGENT_ROLE_LABELS\s*=\s*Object\.freeze\(\{[\s\S]*claude:\s*"Claude Code"[\s\S]*codex:\s*"Codex"/,
    "expected Agent role badges to map runtime ids to display names",
  );
  assert.match(
    appSource,
    /function\s+agentRoleLabel\(windowData\)[\s\S]+windowData\?\.agent_id[\s\S]+AGENT_ROLE_LABELS/,
    "expected Agent role badge labels to resolve from window.agent_id",
  );
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+agentRoleLabel\(windowData\)/,
    "expected titlebar and list role badges to share an Agent-aware label helper",
  );
  assert.doesNotMatch(
    appSource,
    /window-role-badge"\)\.textContent\s*=\s*presetRoleLabel\(windowData\.preset\)/,
    "titlebar role badge must not show the generic Agent preset label",
  );
  assert.doesNotMatch(
    appSource,
    /window-list-role">\$\{presetRoleLabel\(entry\.preset\)\}/,
    "window list role badge must not show the generic Agent preset label",
  );
});

test("Non-Agent duplicate title role badges are omitted", () => {
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+windowDisplayTitle\(windowData\)[\s\S]+return\s+""/,
    "expected duplicate role badges such as Branches / Branches to be suppressed",
  );
  assert.match(
    appSource,
    /function\s+setWindowRoleBadge\(badgeElement,\s*windowData\)[\s\S]+badgeElement\.hidden\s*=\s*!label/,
    "expected titlebar role badge updates to hide empty labels",
  );
  assert.match(
    inlineStyle,
    /\.window-role-badge\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden role badges must stay hidden even though .window-role-badge uses inline-flex",
  );
});

test("Agent fallback runtime titles still show Agent role badges", () => {
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+const\s+isAgentWindow\s*=\s*isAgentWindowPreset\(windowData\?\.preset\)/,
    "expected duplicate suppression to distinguish Agent windows from static panels",
  );
  assert.match(
    appSource,
    /if\s*\(!isAgentWindow\s*&&\s*label\s*===\s*displayTitle\)\s*return\s+""/,
    "Agent role badges must remain visible when the fallback title is the same runtime name",
  );
  assert.doesNotMatch(
    appSource,
    /if\s*\(!label\s*\|\|\s*label\s*===\s*displayTitle\)\s*return\s+""/,
    "generic duplicate suppression must not hide Agent role badges",
  );
});

test("Memo is not exposed as a workspace window feature", () => {
  assert.equal(
    document.querySelector('.preset-button[data-preset="memo"]'),
    null,
    "Add window must not expose the unusable Memo surface",
  );
  assert.doesNotMatch(
    appSource,
    /memoSurface|memo-root|load_memo|create_memo_note|update_memo_note|delete_memo_note/,
    "Memo frontend state, renderer, and protocol events should be removed",
  );
  assert.doesNotMatch(
    inlineStyle,
    /surface-memo|memo-layout|memo-note|memo-editor|memo-status/,
    "Memo-specific CSS should be removed with the surface",
  );
});

test("Agent window titles are truncated with long focus detail kept as tooltip", () => {
  assert.match(
    appSource,
    /function\s+windowTitleTooltip\(windowData\)/,
    "expected a shared helper for long title detail tooltip",
  );
  assert.match(
    appSource,
    /titleText\.title\s*=\s*windowTitleTooltip\(windowData\)/,
    "expected titlebar to keep long focus detail in a tooltip",
  );
  assert.match(
    inlineStyle,
    /\.title\s*\{[\s\S]*flex:\s*1[\s\S]*min-width:\s*0[\s\S]*overflow:\s*hidden/,
    "title row must shrink instead of pushing window controls",
  );
  assert.match(
    inlineStyle,
    /\.title-text\s*\{[\s\S]*overflow:\s*hidden[\s\S]*text-overflow:\s*ellipsis[\s\S]*white-space:\s*nowrap/,
    "title text must use one-line ellipsis",
  );
  assert.match(
    inlineStyle,
    /\.window-actions\s*\{[\s\S]*flex-shrink:\s*0/,
    "window controls must not shrink or overlap with long titles",
  );
});

test("canvas world grid is synchronized from viewport state", () => {
  assert.ok(
    document.querySelector("#canvas-world-grid.canvas-world-grid"),
    "expected canvas to expose a dedicated world-space grid layer",
  );
  assert.match(
    appSource,
    /const\s+worldGrid\s*=\s*document\.getElementById\("canvas-world-grid"\)/,
    "expected app.js to bind the canvas world grid element",
  );
  assert.match(
    appSource,
    /function\s+applyWorldGridViewport\(\)[\s\S]+viewport\.x[\s\S]+viewport\.y[\s\S]+viewport\.zoom/,
    "expected world grid sync to derive position and size from the viewport",
  );
  assert.match(
    appSource,
    /function\s+applyViewport\(\)[\s\S]+applyWorldGridViewport\(\)/,
    "expected applyViewport to update the grid together with the stage transform",
  );
});

test("Workspace active work overview behaves like a command center", () => {
  assert.match(
    appSource,
    /Add Agent to This Work/,
    "expected Workspace-origin launch copy to make same-work agent addition explicit",
  );
  assert.match(
    appSource,
    /last_board_entry_kind/,
    "expected agent cards to expose the latest coordination milestone kind",
  );
  assert.match(
    appSource,
    /coordination_scope/,
    "expected agent cards to expose owner/topic scope for each agent",
  );
  assert.match(
    appSource,
    /function\s+focusBoardEntry\(/,
    "expected Board actions to deep-link to the referenced coordination entry",
  );
  assert.match(
    appSource,
    /data-board-entry-id/,
    "expected Board timeline entries to be addressable from Workspace links",
  );
});

test("Active Work title prefers concrete work context over Start Work workflow label", () => {
  assert.match(
    appSource,
    /function\s+activeWorkDisplayTitle\(projection,\s*agents\)[\s\S]+agent\.title_summary[\s\S]+projection\?\.summary[\s\S]+projection\?\.owner[\s\S]+projection\?\.title[\s\S]+Start Work[\s\S]+Active Work/,
    "expected Active Work title resolution to prefer Agent/Workspace work context and treat Start Work as a fallback-only workflow label",
  );
  assert.match(
    appSource,
    /createNode\("div",\s*"op-work-title",\s*activeWorkDisplayTitle\(work,\s*work\.agents\)\)/,
    "expected Active Work summary title to use the display-title helper",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("div",\s*"op-work-title",\s*activeWorkProjection\.title\s*\|\|\s*"Active Work"\)/,
    "Active Work must not render the saved Start Work workflow title directly",
  );
});

test("Active Work sidebar only renders while live Agent windows are focusable", () => {
  assert.match(
    appSource,
    /function\s+activeWorkFocusableAgents\(work\)[\s\S]+workspaceWindowById\(agent\.window_id\)/,
    "expected Active Work cards to be filtered against live workspace windows",
  );
  assert.match(
    appSource,
    /const\s+activeWorkSection\s*=\s*document\.getElementById\("op-active-work"\)/,
    "expected Active Work visibility to be controlled at the section level",
  );
  assert.match(
    appSource,
    /function\s+setActiveWorkSectionVisible\(visible\)[\s\S]+activeWorkSection\.hidden\s*=\s*!visible/,
    "expected no-Agent Active Work state to hide the entire section instead of leaving stale work UI",
  );
  assert.match(
    appSource,
    /if\s*\(workCount\s*===\s*0\)\s*\{[\s\S]+setActiveWorkSectionVisible\(false\)[\s\S]+return;/,
    "expected no focusable Agent windows to remove the Active Work sidebar section",
  );
  assert.match(
    appSource,
    /function\s+focusActiveWorkAgentWindow\(agent\)[\s\S]+restore_window[\s\S]+focusWindowRemotely\(agent\.window_id,\s*\{\s*center:\s*true\s*\}\)/,
    "expected Focus to restore minimized Agent windows before focusing them",
  );
  assert.match(
    appSource,
    /function\s+agentStatusLabel\(state\)[\s\S]+Running[\s\S]+Blocked[\s\S]+Idle[\s\S]+Done/,
    "expected raw active/blocked/idle/done status values to be mapped to user-facing labels",
  );
  assert.match(
    appSource,
    /function\s+agentRuntimeStatusLabel\(agent\)[\s\S]+runtimeStateForWindow\(windowData\)[\s\S]+windowRuntimeLabel\(runtimeState\)[\s\S]+agentStatusLabel\(agent\.status_category\)/,
    "expected Active Work agent cards to derive their visible runtime label from the live window state before falling back to workspace category",
  );
  assert.match(
    appSource,
    /createNode\("div",\s*"op-agent-state",\s*agentRuntimeStatusLabel\(agent\)\)/,
    "Active Work must render Waiting from WindowState even when workspace status_category remains active",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("div",\s*"op-agent-state",\s*state\)/,
    "Active Work must not render raw status wire values such as ACTIVE",
  );
  assert.match(
    appSource,
    /function\s+renderAppState\(nextState\)[\s\S]+renderActiveWorkOverview\(\)/,
    "expected workspace changes to re-evaluate whether Active Work agents are still focusable",
  );
});

test("Launch Wizard focus start method renders backend-provided running session detail", () => {
  assert.match(
    appSource,
    /for\s*\(\s*const method of launchWizard\.start_methods \|\| \[\]\s*\)/,
    "expected Launch Wizard to render backend-provided start methods",
  );
  assert.match(
    appSource,
    /const detail = method\.enabled === false[\s\S]*?method\.disabled_reason[\s\S]*?: method\.detail;[\s\S]*?createNode\("div", "start-method-detail", detail\)/,
    "expected running-session Focus details to come from the backend start method payload",
  );
});

test("Workspace Overview is separate from live-only Active Work", () => {
  assert.ok(
    document.querySelector("#op-workspace-overview-entry"),
    "expected Sidebar to expose a Workspace Overview entry even when Active Work is hidden",
  );
  assert.ok(
    !document.querySelector("#project-workspace-overview-button"),
    "Workspace Overview is per-project content and must live in the Sidebar, not the global Project Bar",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceOverview\(/,
    "expected a shared opener for the Sidebar entry",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceOverview\(\)\s*\{[\s\S]{0,300}?focusOrSpawnPreset\("work"\)/,
    "expected Workspace Overview to open the Work window instead of a drawer",
  );
  assert.match(
    appSource,
    /op-workspace-overview-entry[\s\S]+openWorkspaceOverview/,
    "expected Sidebar Workspace Overview entry to open the overview",
  );
});

test("Workspace Overview uses the Quiet Work full-window List + Detail layout", () => {
  assert.ok(
    workspaceOverviewSource.length > 0,
    "expected Workspace Overview renderer to live in workspace-kanban-surface.js",
  );
  assert.match(
    appSource,
    /from\s+"\/workspace-kanban-surface\.js"/,
    "expected app.js to import the Workspace Overview surface module",
  );
  assert.match(
    appSource,
    /presetSurface\(preset\)[\s\S]+preset\s*===\s*"work"[\s\S]+return\s+"work"/,
    "expected Work to be a first-class window surface",
  );
  assert.match(
    workspaceOverviewSource,
    /workspace-overview-root[\s\S]+workspace-overview-list-pane[\s\S]+workspace-overview-detail-pane/,
    "expected Workspace Overview to use a quiet List + Detail shell",
  );
  assert.match(
    workspaceOverviewSource,
    /workspace-agent-queue/,
    "expected unassigned agents to stay in a dedicated queue outside Workspace rows",
  );
  assert.match(
    workspaceOverviewSource,
    /function\s+workspacesFromProjection\([^)]*\)[\s\S]+projection\.works[\s\S]+projection\.workspaces[\s\S]+projection\.work_items/,
    "expected Work Overview to render Work entries from active_work_projection",
  );
  // SPEC-2359 US-42 — Resume action now opens the Workspace Resume
  // Picker via `list_resumable_agents` instead of the legacy
  // `resume_workspace` event. The picker drives the actual restart with
  // `resume_workspace_agent` after the user selects a candidate agent.
  assert.match(
    workspaceOverviewSource,
    /function\s+resumeWorkspace\([^)]*\)[\s\S]+list_resumable_agents/,
    "expected Workspace Resume action to ask the backend to list resumable agents",
  );
  assert.doesNotMatch(
    workspaceOverviewSource,
    /workspace-kanban-board|data-workspace-column|workspace-kanban-column/,
    "Workspace Overview must not reintroduce Workspace-specific Kanban columns",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+(workspaceCardsFromProjection|renderWorkspaceKanbanCard|renderWorkspaceKanbanDetail)\(/,
    "Workspace Overview rendering internals should not remain in app.js",
  );
});

test("Workspace Overview root participates in full-window layout", () => {
  const fullWindowRootRule = inlineStyle.match(
    /\.file-tree-root,[\s\S]*?\.mock-root\s*\{[^}]+\}/,
  );
  assert.ok(fullWindowRootRule, "expected shared full-window root layout rule");
  assert.match(
    fullWindowRootRule[0],
    /\.workspace-overview-root/,
    "Workspace Overview root must fill the window body so list/detail panes can scroll",
  );
  assert.match(fullWindowRootRule[0], /position:\s*absolute/);
  assert.match(fullWindowRootRule[0], /display:\s*flex/);
  assert.match(fullWindowRootRule[0], /flex-direction:\s*column/);
});

test("Workspace Overview legacy drawer scaffold is retired", () => {
  assert.doesNotMatch(
    html,
    /workspace-overview-drawer/,
    "Workspace Overview should no longer ship an unused drawer scaffold",
  );
  assert.doesNotMatch(
    appSource,
    /closeWorkspaceOverview|renderWorkspaceOverview|workspaceOverviewFocusTrapRelease/,
    "Workspace Overview drawer lifecycle code should not remain in app.js",
  );
});

test("no surface presets auto-maximize — uniform 720×420 floating windows", () => {
  assert.match(
    appSource,
    /function\s+isAutoMaximizedSurfacePreset\([^)]*\)\s*\{[^}]*return\s+false/,
    "expected isAutoMaximizedSurfacePreset to always return false (auto-maximize abolished)",
  );
});

test("Active Work and Workspace Overview render PR metadata as links", () => {
  assert.match(
    appSource,
    /function\s+createWorkspacePrMeta\(/,
    "expected a shared PR metadata renderer instead of duplicating string-only PR labels",
  );
  assert.match(
    workspaceOverviewSource,
    /createWorkspacePrMeta\?\.\(item\)/,
    "expected Workspace Overview to render the saved PR link/state from the projection",
  );
  assert.match(
    appSource,
    /createWorkspacePrMeta\(activeWorkProjection\)/,
    "expected Active Work sidebar to render the live PR link/state from the projection",
  );
  assert.match(
    appSource,
    /pr_url[\s\S]+href[\s\S]+PR #/,
    "expected PR metadata to use the backend-provided URL for the link target",
  );
  assert.match(
    appSource,
    /pr_state[\s\S]+appendMeta/,
    "expected PR state to be displayed next to the PR link",
  );
});

test("Workspace Overview exposes user-confirmed cleanup for completed workspaces", () => {
  assert.match(
    appSource,
    /cleanup_candidate/,
    "expected active work projection cleanup_candidate to drive Workspace cleanup",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceCleanup\(/,
    "expected Workspace Overview to open a cleanup confirmation instead of deleting automatically",
  );
  assert.match(
    appSource,
    /default_delete_remote[\s\S]+deleteRemote\s*:\s*false|deleteRemote\s*:\s*false[\s\S]+default_delete_remote/,
    "expected Workspace cleanup to default remote deletion off",
  );
  assert.match(
    `${appSource}\n${branchCleanupSource}`,
    /Also delete matching remote branches/,
    "expected remote deletion to remain an explicit opt-in in the confirmation UI",
  );
});

test("Agent window chrome resolves dynamic purpose titles before legacy titles", () => {
  assert.match(
    appSource,
    /function\s+windowDisplayTitle\(windowData\)[\s\S]+dynamic_title[\s\S]+purpose_title[\s\S]+title/,
    "expected a shared display-title helper with dynamic > purpose > legacy title precedence",
  );
  assert.match(
    appSource,
    /window-list-title">\$\{escapeHtml\(windowDisplayTitle\(entry\)\)\}/,
    "expected the Windows dropdown to escape the shared display-title helper output",
  );
  assert.match(
    appSource,
    /title-text"\)\.textContent\s*=\s*windowDisplayTitle\(windowData\)/,
    "expected window titlebars to use the shared display-title helper",
  );
});

test("Branches preset is removed — branch browsing is part of the Work surface", () => {
  const branchPreset = document.querySelector('.preset-button[data-preset="branches"]');
  assert.equal(branchPreset, null, "Branches preset button must not exist — use Work surface");
  assert.doesNotMatch(
    `${html}\n${appSource}`,
    /Planning Session|Workspace[- ]card/i,
    "Work surface should not render Planning Session or Workspace-card concepts",
  );
});

test("Branches loading state becomes recoverable when the WebSocket disconnects", () => {
  assert.match(
    appSource,
    /function\s+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)/,
    "expected a dedicated Branches loading connection-loss helper",
  );
  assert.match(
    appSource,
    /failLoadingBranchesOnConnectionLoss[\s\S]+state\.loading\s*=\s*false[\s\S]+state\.receivedFreshEntries\s*=\s*false/,
    "expected connection loss to clear stale Branches loading flags",
  );
  assert.match(
    appSource,
    /Connection lost while loading branches/,
    "expected initial branch inventory loss to surface a retryable error",
  );
  assert.match(
    appSource,
    /function\s+setConnectionState\(connected\)[\s\S]+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)[\s\S]+renderBranches\(windowId\)/,
    "expected socket disconnect to re-render Branches after clearing stale loading",
  );
});

test("Branches detail-check state explains checking and interrupted cleanup safety", () => {
  assert.match(
    appSource,
    /function\s+branchLoadStatusSummary\(state\)/,
    "expected Branches to derive a status summary from branch load state",
  );
  for (const copy of [
    "Checking branch details",
    "Branch detail check interrupted",
    "Safety unknown",
    "Refresh to verify cleanup safety",
  ]) {
    assert.ok(appSource.includes(copy), `expected Branches clarity copy: ${copy}`);
  }
  assert.doesNotMatch(
    appSource,
    /Cleanup status unavailable/,
    "Branches rows should explain unknown safety instead of showing an unavailable cleanup placeholder",
  );
  assert.doesNotMatch(
    appSource,
    /state\.loading\s*\?\s*"loading"\s*:\s*"unknown"/,
    "Branches cleanup badges should not collapse interrupted hydration into an ambiguous unknown label",
  );
  for (const selector of [
    ".branch-notice-title",
    ".branch-notice-detail",
    ".branch-notice-hint",
  ]) {
    assert.ok(inlineStyle.includes(selector), `missing Branches status summary CSS: ${selector}`);
  }
  assert.match(
    inlineStyle,
    /\.branch-notice\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden Branches notices must not leave an empty status band behind",
  );
});

test("Branches detail-check checking state uses motion without animating static states", () => {
  assert.match(
    inlineStyle,
    /@keyframes\s+branch-detail-check-sweep/,
    "expected Branches checking summary to define a named sweep animation",
  );
  assert.match(
    inlineStyle,
    /@keyframes\s+branch-cleanup-checking-pulse/,
    "expected Branches checking badge to define a named pulse animation",
  );
  assert.match(
    inlineStyle,
    /\.branch-notice\[data-branch-status="checking"\]::before[\s\S]+animation:\s*branch-detail-check-sweep/,
    "expected only the checking summary notice to run the progress sweep",
  );
  assert.match(
    inlineStyle,
    /\.branch-cleanup-badge\.loading[\s\S]+animation:\s*branch-cleanup-checking-pulse/,
    "expected only checking cleanup badges to pulse",
  );

  const reducedMotion = extractMediaBlocks(inlineStyle, "prefers-reduced-motion: reduce");
  assert.match(
    reducedMotion,
    /branch-notice\[data-branch-status="checking"\]::before[\s\S]+animation:\s*none/,
    "reduced-motion users must not get the checking summary sweep",
  );
  assert.match(
    reducedMotion,
    /branch-cleanup-badge\.loading[\s\S]+animation:\s*none/,
    "reduced-motion users must not get the checking badge pulse",
  );
});

test("hotkey overlay lists ⌘P/⌘B/⌘G/⌘L/⌘?/Esc (sidebar toggle hotkey is removed in Phase 9)", () => {
  const overlay = document.getElementById("op-hotkey-overlay");
  assert.ok(overlay, "hotkey overlay missing");
  const text = overlay.textContent.replace(/\s+/g, " ");
  for (const phrase of ["⌘ P", "⌘ B", "⌘ G", "⌘ L", "⌘ K", "⌘ ?", "Esc"]) {
    assert.ok(text.includes(phrase), `expected ${phrase} in hotkey overlay`);
  }
  // SPEC-2356 Phase 9 (FR-021/FR-032): the Layout group + "Toggle sidebar / ⌘\\"
  // row are removed in favor of the hover-reveal peek 帯.
  assert.ok(!text.includes("⌘ \\"), "Cmd+\\\\ must not appear in the hotkey overlay");
  assert.ok(
    !text.toLowerCase().includes("toggle sidebar"),
    "Toggle sidebar row must not appear in the hotkey overlay",
  );
  const groups = Array.from(overlay.querySelectorAll(".op-hotkey-card__group-title")).map((el) =>
    el.textContent?.trim().toLowerCase(),
  );
  for (const expected of ["navigation", "help"]) {
    assert.ok(groups.includes(expected), `expected hotkey overlay group "${expected}", got ${groups.join("/")}`);
  }
  assert.ok(!groups.includes("layout"), "Layout group is removed in Phase 9");
});

test("head loads tokens, typography, components, and Operator modules", () => {
  const css = Array.from(document.querySelectorAll("link[rel=stylesheet]")).map((l) => l.href);
  for (const required of [
    "/styles/tokens.css",
    "/styles/typography.css",
    "/styles/components.css",
    "/assets/xterm/xterm.css",
  ]) {
    assert.ok(css.some((href) => href.endsWith(required)), `expected stylesheet: ${required}`);
  }
  const inlineScripts = Array.from(document.querySelectorAll("head script:not([src])")).map((s) => s.textContent);
  assert.ok(
    inlineScripts.some((src) => src.includes("data-theme") && src.includes("prefers-color-scheme")),
    "expected FOUC-prevention bootstrap script in <head>",
  );
});

test("body markup wires Mission Briefing reveal lines (US-1 AS-1)", () => {
  const lines = document.querySelectorAll(".op-briefing__line");
  assert.ok(lines.length >= 4, `expected >=4 briefing lines, got ${lines.length}`);
  const online = document.querySelector(".op-briefing__online");
  assert.ok(online, "expected OPERATOR ONLINE marker");
  assert.match(online.textContent, /OPERATOR ONLINE/);
});

test("font preload hints exist for Mona/Hubot/JetBrains", () => {
  const preloads = Array.from(document.querySelectorAll("link[rel=preload][as=font]")).map((l) => l.href);
  for (const expected of ["MonaSans.woff2", "HubotSans-Bold.woff2", "JetBrainsMono.woff2"]) {
    assert.ok(preloads.some((h) => h.endsWith(`/assets/fonts/${expected}`)), `missing preload: ${expected}`);
  }
});

test("developer readability typography keeps working text above minimum sizes", () => {
  assert.ok(cssRemVar(typographySource, "--type-xs") >= 0.75, "--type-xs must be at least 12px");
  assert.ok(cssRemVar(typographySource, "--type-sm") >= 0.875, "--type-sm must be at least 14px");
  assert.match(
    typographySource,
    /\.t-mono\s*\{[\s\S]*?line-height:\s*1\.(?:4|[5-9])[\s\S]*?\}/,
    "expected mono utility line-height to stay readable",
  );
  assert.doesNotMatch(
    typographySource,
    /\.(?:t-body|t-mono)\s*\{[\s\S]*?font-stretch:\s*75%/,
    "body and mono working text must not use condensed display typography",
  );
});

test("Mission Briefing has accessible role and live region", () => {
  const briefing = document.getElementById("op-briefing");
  assert.ok(briefing);
  assert.equal(briefing.getAttribute("role"), "status");
  assert.equal(briefing.getAttribute("aria-live"), "polite");
  assert.match(briefing.getAttribute("aria-label") ?? "", /boot|operator/i);
});

test("Status Strip is exposed as a live region with semantic value labels", () => {
  const strip = document.getElementById("op-status-strip");
  assert.ok(strip);
  assert.equal(strip.getAttribute("role"), "status");
  assert.equal(strip.getAttribute("aria-live"), "polite");
  for (const id of ["op-strip-active", "op-strip-idle", "op-strip-blocked", "op-strip-branches"]) {
    const el = document.getElementById(id);
    assert.ok(el, `expected element ${id}`);
    assert.ok(el.getAttribute("aria-label"), `${id} must have an aria-label`);
  }
  // clock cell is intentionally hidden from screen readers (per-second updates)
  const clockCell = document.getElementById("op-strip-clock")?.parentElement;
  assert.ok(clockCell, "clock cell exists");
  assert.equal(clockCell.getAttribute("aria-hidden"), "true");
});

test("Status Strip omits the retired server URL cell", () => {
  assert.equal(document.getElementById("op-strip-server-url"), null);
  assert.equal(document.getElementById("op-strip-server-url-copy"), null);
  assert.equal(document.querySelector(".op-status-strip__cell--server-url"), null);
  assert.doesNotMatch(appSource, /op-strip-server-url/);
  assert.doesNotMatch(appSource, /kind:\s*"open_server_url"/);
});

test("Status Strip labels the WebSocket connection state as ONLINE/OFFLINE", () => {
  const strip = document.getElementById("op-status-strip");
  const connectionLabel = strip?.querySelector("[data-role='connection-label']");
  assert.ok(connectionLabel, "expected a connection label in the status strip");
  assert.equal(connectionLabel.textContent?.trim(), "ONLINE");
  assert.doesNotMatch(html, />\s*LIVE\s*</);
  assert.match(appSource, /connectionStatusLabel\.textContent\s*=\s*connected\s*\?\s*"ONLINE"\s*:\s*"OFFLINE"/);
});

test("Sidebar Quick rows expose aria-keyshortcuts and kbd badges", () => {
  for (const [cmd, key] of [
    ["open-board", "B"],
    ["open-logs", "L"],
  ]) {
    const button = document.querySelector(`.op-layer[data-cmd="${cmd}"]`);
    assert.ok(button, `expected Quick row for ${cmd}`);
    const shortcut = button.getAttribute("aria-keyshortcuts");
    assert.ok(shortcut, `${cmd} must declare aria-keyshortcuts`);
    assert.match(shortcut, new RegExp(`Meta\\+${key}`));
    const kbd = button.querySelector("kbd.op-layer__kbd");
    assert.ok(kbd, `${cmd} must show a kbd badge`);
  }
});

test("Command Palette trigger button declares aria-keyshortcuts", () => {
  const trigger = document.getElementById("op-palette-button");
  assert.ok(trigger, "palette trigger exists");
  const shortcut = trigger.getAttribute("aria-keyshortcuts") ?? "";
  assert.ok(shortcut.includes("Meta+K"), "trigger must declare Meta+K");
  assert.ok(shortcut.includes("Meta+P"), "trigger must declare Meta+P");
});

test("chrome visibility uses peek 帯 hover-reveal instead of click chips", () => {
  // SPEC-2356 Phase 9 (FR-021/FR-022): the chip-style toggles and Project Bar
  // text toggles are removed; auto-hide chrome is summoned via the peek 帯
  // (`.op-sidebar-peek` / `.op-window-controls-peek`).
  assert.equal(
    document.getElementById("op-sidebar-toggle"),
    null,
    "Project Bar must not expose a SIDEBAR text toggle",
  );
  assert.equal(
    document.getElementById("op-window-controls-toggle"),
    null,
    "Project Bar must not expose a WINDOWS text toggle",
  );
  assert.equal(
    document.getElementById("op-sidebar-edge-toggle"),
    null,
    "<< chip toggle is removed in Phase 9",
  );
  assert.equal(
    document.getElementById("op-window-controls-edge-toggle"),
    null,
    "vv chip toggle is removed in Phase 9",
  );

  const sidebarPeek = document.querySelector(".op-sidebar-peek");
  const windowControlsPeek = document.querySelector(".op-window-controls-peek");
  assert.ok(sidebarPeek, "expected .op-sidebar-peek hover trigger");
  assert.ok(windowControlsPeek, "expected .op-window-controls-peek hover trigger");
  assert.equal(sidebarPeek.getAttribute("aria-controls"), "op-sidebar");
  assert.equal(
    windowControlsPeek.getAttribute("aria-controls"),
    "floating-window-controls-actions",
  );
  assert.match(sidebarPeek.getAttribute("aria-label") ?? "", /show sidebar/i);
  assert.match(windowControlsPeek.getAttribute("aria-label") ?? "", /show window controls/i);
  assert.equal(sidebarPeek.getAttribute("tabindex"), "0", "sidebar peek 帯 must be keyboard-focusable");
  assert.equal(
    windowControlsPeek.getAttribute("tabindex"),
    "0",
    "window controls peek 帯 must be keyboard-focusable",
  );
  assert.equal(sidebarPeek.getAttribute("role"), "button");
  assert.equal(windowControlsPeek.getAttribute("role"), "button");
});

test("window controls peek 帯 targets only collapsible control groups", () => {
  const windowControlsPeek = document.querySelector(".op-window-controls-peek");
  assert.ok(windowControlsPeek, "expected window controls peek 帯");

  const controlledIds = (windowControlsPeek.getAttribute("aria-controls") ?? "")
    .split(/\s+/)
    .filter(Boolean);
  assert.deepEqual(controlledIds, ["floating-window-controls-actions"]);
  assert.ok(!controlledIds.includes("floating-window-controls"));

  const actionsGroup = document.getElementById("floating-window-controls-actions");
  const primaryGroup = document.getElementById("floating-window-controls-primary");
  const addGroup = document.getElementById("floating-window-controls-add");
  assert.ok(actionsGroup, "expected continuous window controls actions group");
  assert.ok(primaryGroup, "expected primary window controls group");
  assert.ok(addGroup, "expected add-window control group");

  const floatingControls = document.getElementById("floating-window-controls");
  assert.ok(floatingControls, "expected floating window controls root");
  const toolbarChildren = Array.from(floatingControls.children);
  assert.ok(
    toolbarChildren.indexOf(windowControlsPeek) < toolbarChildren.indexOf(actionsGroup),
    "peek must precede actions in DOM order so forward Tab enters the revealed controls",
  );

  assert.ok(actionsGroup.contains(primaryGroup), "primary controls must stay inside the continuous actions group");
  assert.ok(actionsGroup.contains(addGroup), "add controls must stay inside the continuous actions group");

  for (const id of ["tile-button", "stack-button", "align-button", "window-list-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.ok(actionsGroup.contains(control), `${id} must be inside the continuous actions group`);
  }

  const addButton = document.getElementById("add-button");
  assert.ok(addButton, "expected add-button");
  assert.ok(addGroup.contains(addButton), "add-button must be inside the add controlled group");
  assert.ok(actionsGroup.contains(addButton), "add-button must be reachable through the continuous actions group");

  for (const id of ["op-palette-button", "zoom-out-button", "zoom-reset-button", "zoom-in-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.equal(
      actionsGroup.contains(control),
      false,
      `${id} must remain outside collapsible window controls groups`,
    );
  }
});

test("floating window controls mark only window operations as hideable", () => {
  for (const id of ["tile-button", "stack-button", "align-button", "window-list-button", "add-button"]) {
    const button = document.getElementById(id);
    assert.ok(button, `expected ${id}`);
    assert.equal(button.dataset.windowControl, "true", `${id} should be hidden by the window controls toggle`);
  }
  for (const id of ["op-palette-button", "zoom-out-button", "zoom-reset-button", "zoom-in-button"]) {
    const button = document.getElementById(id);
    assert.ok(button, `expected ${id}`);
    assert.notEqual(button.dataset.windowControl, "true", `${id} should remain visible when window controls are hidden`);
  }
});

test("workspace windows expose draggable tab docking affordances", () => {
  assert.match(
    appSource,
    /class="window-tab-strip"|className\s*=\s*"window-tab-strip"/,
    "expected grouped windows to render a tab strip",
  );
  assert.match(
    appSource,
    /function\s+titlebarDockTargetAt\(/,
    "expected ungrouped windows to find dock targets from titlebar drag",
  );
  assert.match(
    windowDockingSource,
    /export\s+const\s+TITLEBAR_DOCK_HIT_HEIGHT\s*=\s*38/,
    "expected dock hit-testing to be constrained to the titlebar height",
  );
  assert.match(
    windowDockingSource,
    /point\.y\s*<=\s*geometry\.y\s*\+\s*titlebarHeight/,
    "expected body/canvas drops to avoid the dock path",
  );
  assert.match(
    appSource,
    /function\s+updateTitlebarDockPreview\(/,
    "expected titlebar drag to update dock target hover preview",
  );
  assert.match(
    appSource,
    /function\s+clearTitlebarDockPreview\(/,
    "expected dock target preview to be cleared after drop or cancel",
  );
  assert.match(
    appSource,
    /dragState\.dockTargetId[\s\S]+kind:\s*"dock_window_tab"[\s\S]+target_id:\s*dragState\.dockTargetId/,
    "expected titlebar drag pointerup to dock only when a titlebar target was hit",
  );
  assert.match(
    appSource,
    /kind:\s*"dock_window_tab"/,
    "expected tab drop to send dock_window_tab",
  );
  assert.match(
    appSource,
    /kind:\s*"detach_window_tab"/,
    "expected tab drag outside a group to send detach_window_tab",
  );
  assert.match(
    appSource,
    /kind:\s*"activate_window_tab"/,
    "expected tab click to activate a grouped window tab",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s+\.titlebar/,
    "expected dockable titlebar targets to have a visible preview state",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s*\{/,
    "expected dockable targets to outline the whole window, not just the titlebar",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s+\.window-tab-strip::before/,
    "expected dockable targets to expose a tab insertion indicator",
  );
});

test("Project Bar brand prefix wraps GWT OPERATOR with bracket flank", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The pseudo-element content lives only in CSS, not in the DOM, so we
  // assert the rule itself contains the bracket-flanked brand string.
  const projectBarBefore = css.match(/\.project-bar::before\s*\{[\s\S]*?\}/);
  assert.ok(projectBarBefore, "expected .project-bar::before rule");
  assert.match(projectBarBefore[0], /content:\s*"⌜ GWT · OPERATOR ⌟"/);
});

test("Mission Briefing splash has dismissible affordance (pointer-events + cursor)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const briefing = css.match(/\.op-briefing\s*\{[\s\S]*?\}/);
  assert.ok(briefing, "expected .op-briefing rule");
  assert.match(briefing[0], /pointer-events:\s*auto/);
  assert.match(briefing[0], /cursor:\s*pointer/);
});

test("theme-toggle wires aria-label updates on every render", () => {
  // SPEC-2356 FR-024 — segmented toggle exposes the live preference + effective
  // theme via the radiogroup container's aria-label so screen readers always
  // announce the current state.
  const themeToggle = readFileSync(resolve(here, "../theme-toggle.js"), "utf8");
  assert.match(
    themeToggle,
    /root\.setAttribute\(\s*"aria-label"/,
    "expected segmented theme toggle to update aria-label on every render",
  );
  assert.match(
    themeToggle,
    /Theme: \$\{pref === "auto" \? `auto/,
    "expected aria-label format to disclose preference + effective theme",
  );
});

test("xterm content stays on the dark Operator palette across app theme changes", () => {
  assert.match(
    appSource,
    /theme:\s*XTERM_THEME_DARK/,
    "expected Terminal initialization to use the dark xterm palette directly",
  );
  assert.doesNotMatch(
    appSource,
    /XTERM_THEME_LIGHT/,
    "xterm content must not define a light palette; only the terminal chrome follows app theme",
  );
  assert.doesNotMatch(
    appSource,
    /registerXtermThemeAdapter/,
    "theme toggles must not swap xterm content away from the dark palette",
  );
});

test("xterm developer readability defaults use larger font metrics", () => {
  assert.ok(terminalOptionNumber("fontSize") >= 14, "xterm fontSize must be at least 14px");
  assert.match(
    appSource,
    /lineHeight:\s*isBlinkBrowser\(\)\s*\?\s*1\.3[0-9]*\s*:\s*1\.3/,
    "xterm lineHeight must be at least 1.30 for both Blink and WebKit paths (Issue #2903)",
  );
});

test("xterm fontFamily is resolved without CSS var() so OffscreenCanvas char measurement matches the rendered font", () => {
  // xterm 6.0.0 measures the char cell via an OffscreenCanvas strategy
  // (ctx.font = `${fontSize}px ${fontFamily}`). Canvas 2D `ctx.font` CANNOT
  // resolve CSS custom properties, so a `var(--font-mono)` family token is
  // dropped and the SHORTER system fallback (SF Mono / Menlo) is measured,
  // while the rendered rows resolve var() to the TALLER JetBrains Mono — the
  // permanent measure-vs-render mismatch that clips glyphs vertically.
  const runtime = appSource.match(/new\s+Terminal\(\{\s*([\s\S]*?)\s*\}\);/);
  assert.ok(runtime, "expected xterm Terminal constructor options");
  assert.doesNotMatch(
    runtime[1],
    /var\(/,
    "xterm Terminal options must not pass a CSS var() font family; OffscreenCanvas ctx.font cannot resolve it",
  );
  assert.match(
    appSource,
    /function\s+resolveTerminalFontFamily\s*\(/,
    "expected a resolveTerminalFontFamily() helper that resolves --font-mono to a var()-free stack",
  );
  // The resolver's hardcoded fallback must still prefer JetBrains Mono.
  assert.match(
    appSource,
    /resolveTerminalFontFamily[\s\S]{0,400}?JetBrains Mono/,
    "resolveTerminalFontFamily fallback must reference JetBrains Mono",
  );
});

test("operator-shell wires hover-reveal chrome and Mission Briefing early dismiss", () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  // SPEC-2356 Phase 9 (FR-021/FR-022/FR-031): hover-reveal state machine sets
  // root.dataset.opSidebar = "revealed" / opWindowControls = "revealed" rather
  // than "collapsed", and Cmd+\\ hotkey is removed.
  assert.ok(
    !operatorShell.includes('hotkey.register("cmd+\\\\"'),
    "Cmd+backslash hotkey must be removed in Phase 9",
  );
  assert.doesNotMatch(
    operatorShell,
    /toggleSidebar\s*:/,
    "Phase 9 hover-reveal controller must not expose a toggleSidebar entrypoint",
  );
  assert.match(
    operatorShell,
    /root\.dataset\[datasetKey\]\s*=\s*"revealed"/,
    "expected hover-reveal state machine to write data-op-* = \"revealed\"",
  );
  assert.match(
    operatorShell,
    /earlyDismiss/,
    "expected Mission Briefing earlyDismiss helper",
  );
});

test("app state rendering dismisses Mission Briefing so startup cannot stay on splash", () => {
  assert.match(
    appSource,
    /function\s+dismissOperatorBriefing\(\)[\s\S]+op-briefing[\s\S]+hidden\s*=\s*true/,
    "expected app.js to expose a fail-open Mission Briefing dismiss helper",
  );
  assert.match(
    appSource,
    /function\s+renderAppState\(nextState\)\s*\{\s*dismissOperatorBriefing\(\)/,
    "expected first app state render to unblock Welcome/Workspace surfaces",
  );
});

test("components.css hover-reveals only the marked floating window control groups", () => {
  // SPEC-2356 Phase 9 (FR-022): the continuous actions group auto-hides by default and
  // are revealed only when [data-op-window-controls="revealed"] is set.
  // Palette and Zoom controls remain in the toolbar regardless.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /#floating-window-controls-actions[\s\S]*?display:\s*none/);
  assert.match(css, /#floating-window-controls-actions\s*\{[^}]*order:\s*1/);
  assert.match(css, /\.op-window-controls-peek\s*\{[^}]*order:\s*2/);
  assert.match(
    css,
    /\[data-op-window-controls="revealed"\][\s\S]+?#floating-window-controls-actions[\s\S]*?display:\s*flex/,
  );
  assert.doesNotMatch(css, /\[data-op-window-controls="revealed"\][\s\S]+#op-palette-button/);
  assert.doesNotMatch(css, /\[data-op-window-controls="revealed"\][\s\S]+#zoom-reset-button/);
  assert.doesNotMatch(css, /\[data-op-window-controls="hidden"\]/);
  assert.match(css, /\.op-window-controls-peek\s*\{/);
  assert.match(css, /\.op-sidebar-peek\s*\{/);
});

test("floating actions expose Align without resizing windows", () => {
  const button = document.getElementById("align-button");
  assert.ok(button, "expected Align button");
  assert.equal(button.textContent.trim(), "Align");
  assert.match(
    appSource,
    /alignButton\.addEventListener\("click",\s*\(\)\s*=>\s*arrangeWindows\("align"\)\)/,
    "expected Align to reuse arrange_windows with the align mode",
  );
});

test("operator-shell migrates legacy chrome keys and wires hover-reveal independently", () => {
  // SPEC-2356 Phase 9 (FR-032): legacy localStorage keys are removed on boot
  // and the chip-style toggles (and their Project Bar text counterparts) must
  // not be referenced anywhere in operator-shell.
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  assert.doesNotMatch(operatorShell, /SIDEBAR_COLLAPSED_KEY\s*=\s*"gwt:ui:sidebar-collapsed"/);
  assert.doesNotMatch(operatorShell, /WINDOW_CONTROLS_KEY\s*=\s*"gwt:ui:window-controls"/);
  assert.doesNotMatch(operatorShell, /op-sidebar-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-sidebar-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-toggle/);
  assert.match(operatorShell, /removeItem\("gwt:ui:sidebar-collapsed"\)/);
  assert.match(operatorShell, /removeItem\("gwt:ui:window-controls"\)/);
  assert.match(operatorShell, /op:chrome-visibility-changed/);
  assert.match(operatorShell, /op:window-controls-changed/);
  assert.match(operatorShell, /\.op-sidebar-peek/);
  assert.match(operatorShell, /\.op-window-controls-peek/);
});

test("components.css declares Status Strip BLOCKED pulse + live indicator", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // PR #2414 introduced the pulse animation; PR #2404 the layer live dot.
  assert.match(css, /op-status-strip-blocked-pulse/);
  assert.match(css, /\.op-layer\[data-live="true"\] \.op-layer__label::before/);
});

test("components.css declares Operator scrollbar + tinted text selection", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /::-webkit-scrollbar-thumb/);
  assert.match(css, /scrollbar-color:\s*var\(--color-border-strong\)/);
  assert.match(css, /::selection/);
});

test("components.css uses op-divider utility class", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /\.op-divider\s*\{/);
  assert.match(css, /\.op-divider--vertical/);
  assert.match(css, /\.op-divider--strong/);
});

test("Drawer + preset modals have role/aria-modal/aria-hidden wiring", () => {
  // Every modal-backdrop must declare aria-hidden initially and contain a
  // modal-shell with role="dialog" + aria-modal="true". Without this, screen
  // readers don't recognize the modal as a dialog and the inert state isn't
  // signaled when closed.
  for (const id of ["branch-cleanup-modal", "migration-modal", "preset-modal", "wizard-modal"]) {
    const backdrop = document.getElementById(id);
    assert.ok(backdrop, `expected modal backdrop #${id}`);
    assert.equal(
      backdrop.getAttribute("aria-hidden"),
      "true",
      `${id} backdrop must start aria-hidden="true"`,
    );
    const shell = backdrop.querySelector(".modal-shell");
    assert.ok(shell, `${id} must contain a .modal-shell`);
    assert.equal(shell.getAttribute("role"), "dialog", `${id} shell needs role="dialog"`);
    assert.equal(shell.getAttribute("aria-modal"), "true", `${id} shell needs aria-modal="true"`);
    assert.equal(shell.getAttribute("tabindex"), "-1", `${id} shell needs tabindex="-1" for focus management`);
    // Either aria-label or aria-labelledby must point at a real label
    const label = shell.getAttribute("aria-label");
    const labelledby = shell.getAttribute("aria-labelledby");
    assert.ok(label || labelledby, `${id} shell needs aria-label or aria-labelledby`);
    if (labelledby) {
      assert.ok(
        document.getElementById(labelledby),
        `${id}: aria-labelledby="${labelledby}" must point at an existing element`,
      );
    }
  }
});

test("WebView modal text uses native selection and terminal overlays use explicit copy", () => {
  const modalShellRule = inlineStyle.match(/\.modal-shell\s*\{[\s\S]*?\}/);
  assert.ok(modalShellRule, "expected shared modal shell CSS rule");
  assert.doesNotMatch(
    modalShellRule[0],
    /user-select\s*:\s*none/,
    "modal shells must not disable browser-native text selection",
  );

  const visibleOverlayRule = inlineStyle.match(
    /\.terminal-overlay\.visible\s*\{[\s\S]*?\}/,
  );
  assert.ok(visibleOverlayRule, "expected visible terminal overlay CSS rule");
  assert.match(visibleOverlayRule[0], /pointer-events:\s*auto/);
  assert.match(visibleOverlayRule[0], /user-select:\s*text/);

  const overlayMessageRule = inlineStyle.match(/\.overlay-message\s*\{[\s\S]*?\}/);
  assert.ok(overlayMessageRule, "expected terminal overlay message CSS rule");
  assert.match(overlayMessageRule[0], /user-select:\s*text/);
  assert.match(appSource, /copyButton\.className\s*=\s*"overlay-copy-button"/);
  assert.match(
    appSource,
    /copyButton\.addEventListener\("click"[\s\S]*copyTerminalOverlayMessage\(windowData\.id\)/,
    "terminal overlays must use an explicit copy button instead of modal-shell auto-copy",
  );
});

test("terminal overlay never covers raw terminal status details", () => {
  const visibleToggle = appSource.match(
    /overlay\.classList\.toggle\(\s*"visible",\s*([\s\S]*?)\s*\);/,
  );
  assert.ok(visibleToggle, "expected terminal overlay visibility toggle");
  assert.match(
    visibleToggle[1],
    /shouldShowOverlay/,
    "expected terminal overlay visibility to be driven by the named guard",
  );
  const overlayVisibilitySource = appSource.match(
    /const\s+shouldShowOverlay\s*=\s*([\s\S]*?);\s*const\s+shouldSpin/,
  );
  assert.ok(overlayVisibilitySource, "expected shouldShowOverlay guard");
  assert.equal(
    overlayVisibilitySource[1].trim(),
    "false",
    "terminal status details must never add a foreground overlay over the raw TTY",
  );
  assert.doesNotMatch(
    overlayVisibilitySource[1],
    /runtimeState\s*===\s*"error"/,
    "error status details must stay in terminal status surfaces instead of an overlay",
  );
  assert.doesNotMatch(
    overlayVisibilitySource[1],
    /runtimeState\s*===\s*"running"/,
    "running launch details must not cover the raw terminal TTY with an overlay",
  );
  const spinnerSource = appSource.match(
    /const\s+shouldSpin\s*=\s*([\s\S]*?);\s*const\s+spinner/,
  );
  assert.ok(spinnerSource, "expected shouldSpin guard");
  assert.equal(
    spinnerSource[1].trim(),
    "false",
    "hidden terminal overlays must never run a foreground startup spinner",
  );
  assert.doesNotMatch(
    spinnerSource[1],
    /runtimeState\s*===\s*"error"/,
    "error status details must not start a foreground terminal overlay spinner",
  );
  assert.doesNotMatch(
    spinnerSource[1],
    /runtimeState\s*===\s*"running"/,
    "running launch details must not start a foreground terminal overlay spinner",
  );
});

test("Every keyframes-driven animation has a prefers-reduced-motion override", () => {
  // Catch the gap where someone adds a new @keyframes + animation without
  // pairing it with a reduced-motion override. Approach: for each
  // keyframes name, find its `animation: <name>` use site, walk back to
  // the enclosing selector, and verify that selector (or a parent
  // matching it) appears in any prefers-reduced-motion block.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const keyframes = [...new Set([...css.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)].map((m) => m[1]))];
  assert.ok(keyframes.length >= 6, `expected >= 6 keyframes, got ${keyframes.length}`);

  // CSS @media blocks have nested rules with their own closing braces, so
  // a naive regex with `[^}]*` or `[\s\S]*?\n\}` undercaptures. Walk the
  // string and use depth tracking to extract every prefers-reduced-motion
  // block in full.
  const reducedMotionUnion = extractMediaBlocks(css, "prefers-reduced-motion: reduce");
  assert.ok(reducedMotionUnion.length > 0, "expected at least one prefers-reduced-motion block");

  for (const name of keyframes) {
    // Locate `animation: <name>` (skip the @keyframes definition itself).
    const useIndex = css.search(new RegExp(`animation:[^;]*\\b${name}\\b[^;]*;`));
    assert.ok(useIndex > 0, `@keyframes ${name} has no animation: use site`);
    // Walk back from the use site to the most recent `{` to find the
    // enclosing selector — that's the line that opens the rule.
    const before = css.slice(0, useIndex);
    const lastOpenBrace = before.lastIndexOf("{");
    assert.ok(lastOpenBrace > 0, `cannot locate opening brace for ${name}`);
    // Selector text is the line(s) immediately before the `{`. Walk
    // backward to the previous `}` (or BOF) for the rule start.
    const ruleStart = before.lastIndexOf("}", lastOpenBrace - 1);
    const selectorText = before.slice(ruleStart + 1, lastOpenBrace).trim();
    // Grab every class, attribute selector, or id used in the selector.
    const tokens = selectorText.match(/\.[\w-]+|\[[\w-]+(="[^"]*")?\]|#[\w-]+/g) || [];
    const hasOverride = tokens.some((tok) => reducedMotionUnion.includes(tok));
    assert.ok(
      hasOverride,
      `@keyframes ${name} (selector "${selectorText}") has no prefers-reduced-motion override`,
    );
  }
});

test("Branch rows are keyboard-navigable with click + dblclick parity", () => {
  // Branch rows are <div>s with both click (select) and dblclick (open
  // launch wizard) handlers. Keyboard parity:
  //   Enter / Space → select (same as click)
  //   Cmd/Ctrl + Enter → activate (same as dblclick — open wizard)
  // Without keyboard support, keyboard-only users could Tab to a row
  // but couldn't pick branches or launch agents from the Branches
  // surface at all.
  assert.match(
    appSource,
    /row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)[\s\S]*createBranchRow|createBranchRow[\s\S]*row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)/,
    "expected createBranchRow to set tabindex and role=button",
  );
  // The keydown handler must distinguish plain Enter/Space (select)
  // from modified Enter (activate / open wizard).
  assert.match(
    appSource,
    /event\.metaKey\s*\|\|\s*event\.ctrlKey[\s\S]*activate\(\)/,
    "expected modified Enter to invoke activate (open launch wizard)",
  );
});

test("Branches separates row actions from cleanup toolbar action", () => {
  const branchesBlock = appSource
    .split('if (surface === "branches")')
    .at(1)
    ?.split('if (surface === "profile")')
    .at(0);

  assert.ok(branchesBlock, "expected Branches surface render block");
  assert.match(branchesBlock, /<div class="branch-heading">Repository branches<\/div>/);
  assert.doesNotMatch(
    branchesBlock,
    /double-click to launch/i,
    "Branches heading should not explain the double-click shortcut",
  );
  assert.match(
    branchesBlock,
    /data-action="open-branch-cleanup"/,
    "cleanup remains a toolbar-level selection action",
  );
  assert.doesNotMatch(
    branchesBlock,
    /data-action="open-branch-resume"|data-action="open-branch-launch"/,
    "Resume and Launch must not remain toolbar-level selected-branch actions",
  );

  assert.match(appSource, /branch-row-actions/, "branch rows must render action buttons");
  assert.match(
    appSource,
    /setAttribute\("data-branch-row-action",\s*"resume"\)/,
    "branch rows must expose a row-level Resume action",
  );
  assert.match(
    appSource,
    /setAttribute\("data-branch-row-action",\s*"launch"\)/,
    "branch rows must expose a row-level Launch action",
  );
  assert.match(
    appSource,
    /entry\.resume\.available[\s\S]{0,500}?resumeButton\.disabled/,
    "Resume row action must be disabled when the branch is not resumable",
  );
  assert.match(
    appSource,
    /branchName[\s\S]{0,260}?kind:\s*"resume_branch_latest_agent"|kind:\s*"resume_branch_latest_agent"[\s\S]{0,260}?branchName/,
    "row Resume action must send its own branch name",
  );
  assert.match(
    appSource,
    /branchName[\s\S]{0,220}?kind:\s*"open_launch_wizard"|kind:\s*"open_launch_wizard"[\s\S]{0,220}?branchName/,
    "row Launch action must send its own branch name",
  );
  assert.match(
    appSource,
    /row\.addEventListener\("dblclick",\s*activate\)/,
    "branch row double-click remains a Launch Agent shortcut",
  );
});

test("Branches row layout responds to minimized window width", () => {
  assert.match(
    inlineStyle,
    /\.branch-list-root\s*{[\s\S]*container-type:\s*inline-size/,
    "Branches list must own an inline-size container so minimized app windows drive row layout",
  );

  const minimizedBranchBlock = extractContainerBlocks(inlineStyle, "max-width: 900px");
  assert.match(
    minimizedBranchBlock,
    /\.branch-row\s*{[\s\S]*grid-template-columns:\s*auto\s+minmax\(0,\s*1fr\)\s+auto/,
    "minimized Branches rows should keep branch text wide and reserve one actions column",
  );
  assert.match(
    minimizedBranchBlock,
    /\.branch-meta\s*{[\s\S]*grid-column:\s*2\s*\/\s*4[\s\S]*grid-row:\s*2/,
    "minimized Branches rows should move metadata below the branch text instead of squeezing it",
  );
  assert.match(
    minimizedBranchBlock,
    /\.branch-row-actions\s*{[\s\S]*grid-column:\s*3[\s\S]*grid-row:\s*1/,
    "minimized Branches rows should keep Resume/Launch aligned on the top row",
  );
  assert.match(
    inlineStyle,
    /\.branch-upstream,\s*\n\.branch-date\s*{[\s\S]*white-space:\s*nowrap[\s\S]*text-overflow:\s*ellipsis/,
    "branch upstream/date text should truncate instead of wrapping into vertical columns",
  );
});

test("Keyboard-navigable rows have :focus-visible outlines", () => {
  // After PRs #2464/#2465 made file-tree-row and branch-row keyboard-
  // navigable, the rows could receive focus but had no visible outline
  // — keyboard users couldn't see where focus was. Project tabs use
  // the same div+role+tabindex pattern and need the same focus-visible
  // treatment. Source-of-truth audit: every selector in app.js that
  // sets `tabIndex = 0` and `role="button"` must be in the CSS group.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  for (const selector of [".project-tab", ".file-tree-row", ".branch-row"]) {
    const re = new RegExp(`:root\\[data-theme\\] \\${selector}:focus-visible`);
    assert.match(
      css,
      re,
      `expected ${selector} to be in the unified focus-visible group`,
    );
  }
});

test("Launch wizard open errors render in wizard modal and close locally", () => {
  assert.match(
    appSource,
    /let\s+launchWizardOpenError\s*=\s*null/,
    "expected frontend to track launch wizard open errors separately from project_open_error",
  );
  // Issue #2698 PR 1 (B7) — the case body now defers via
  // `wizardInteractionGuard.defer(...)` before mutating the error
  // state, so the regex window between the case label and the
  // assignment must permit the guard preamble. The intent is
  // unchanged: this case populates `launchWizardOpenError`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,600}?launchWizardOpenError\s*=/,
    "expected launch_wizard_open_error events to populate wizard error state",
  );
  assert.match(
    appSource,
    /function\s+closeLaunchWizardLocal\(\)[\s\S]{0,300}?launchWizardOpenError\s*=\s*null/,
    "expected a local close path for error-only wizard state",
  );
  assert.match(
    appSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]{0,500}?closeLaunchWizardLocal\(\)[\s\S]{0,500}?sendAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc/close to locally dismiss error-only wizard state before sending backend cancel",
  );
});

test("Launch wizard tombstone does not dismiss open-error modal state", () => {
  assert.match(
    appSource,
    /if\s*\(deferred\.kind\s*===\s*"launch_wizard_state"\)[\s\S]{0,500}?if\s*\(deferred\.wizard\)\s*\{[\s\S]{0,160}?launchWizardOpenError\s*=\s*null[\s\S]{0,160}?\}[\s\S]{0,240}?launchWizard\s*=\s*deferred\.wizard/,
    "expected deferred launch_wizard_state tombstones to preserve launchWizardOpenError",
  );
  assert.match(
    appSource,
    /case\s+"launch_wizard_state":[\s\S]{0,900}?if\s*\(event\.wizard\)\s*\{[\s\S]{0,160}?launchWizardOpenError\s*=\s*null[\s\S]{0,160}?\}[\s\S]{0,240}?launchWizard\s*=\s*event\.wizard/,
    "expected launch_wizard_state tombstones to clear stale wizard state without clearing launchWizardOpenError",
  );
});

test("Launch wizard removes duplicate header close button", () => {
  assert.equal(
    document.getElementById("wizard-close-button"),
    null,
    "Launch wizard header must not render a second Close button",
  );
  assert.ok(
    document.getElementById("wizard-cancel-button"),
    "Launch wizard footer dismiss button must remain available",
  );
  assert.equal(
    appSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\)/,
    "expected the footer dismiss button to own the close helper",
  );
});

test("Launch wizard uses one visible dismiss control in the footer", () => {
  assert.equal(
    document.getElementById("wizard-close-button"),
    null,
    "Launch wizard header must not render a second Close button",
  );
  assert.ok(
    document.getElementById("wizard-cancel-button"),
    "Launch wizard footer dismiss button must remain available",
  );
  assert.equal(
    appSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\)/,
    "expected the footer dismiss button to own the close helper",
  );
});

test("Launch wizard renders a backend-gated Back control in the footer", () => {
  assert.ok(
    document.getElementById("wizard-back-button"),
    "Launch wizard footer must expose a Back button for returning to Start methods",
  );
  assert.match(
    appSource,
    /const wizardBackButton = document\.getElementById\("wizard-back-button"\)/,
    "expected app.js to bind the footer Back button",
  );
  assert.match(
    appSource,
    /wizardBackButton\.hidden\s*=\s*!launchWizard\.show_back_button/,
    "expected Back visibility to be controlled by backend view state",
  );
  assert.match(
    appSource,
    /wizardBackButton\.addEventListener\("click",\s*\(\)\s*=>\s*\{[\s\S]*?kind:\s*"back"/,
    "expected Back to dispatch the canonical backend action",
  );
});

test("File tree rows are keyboard-navigable (tabindex + role + keydown)", () => {
  // <div>-based rows can't be Tab'd to or activated with the keyboard
  // unless explicitly opted in. Add tabindex, role="button", and a
  // keydown handler that mirrors the click handler for Enter/Space.
  // Without all three, keyboard-only users couldn't browse the file
  // tree — only mouse users could.
  assert.match(appSource, /row\.tabIndex\s*=\s*0/, "expected file tree row tabindex=0");
  assert.match(
    appSource,
    /row\.setAttribute\("role",\s*"button"\)/,
    "expected file tree row role='button'",
  );
  assert.match(
    appSource,
    /row\.addEventListener\("keydown",[\s\S]*event\.key === "Enter" \|\| event\.key === " "[\s\S]*activate\(\)/,
    "expected file tree row keydown handler invoking the click activate path on Enter/Space",
  );
});

test("File tree directory rows expose collapse state via aria-expanded", () => {
  // The file tree shows directories with ▾/▸ caret glyphs to indicate
  // expand/collapse state visually. Without aria-expanded, screen
  // readers can't tell whether a directory is open or closed, so the
  // user has to guess via context. File (non-directory) rows must
  // explicitly removeAttribute so a row that was a directory and
  // became a file (via state change) doesn't keep the marker.
  assert.match(
    appSource,
    /isDirectory[\s\S]*row\.setAttribute\("aria-expanded",\s*expanded\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected directory rows to set aria-expanded based on expanded state",
  );
  assert.match(
    appSource,
    /row\.removeAttribute\("aria-expanded"\)/,
    "expected non-directory rows to remove aria-expanded",
  );
});

test("Launch wizard choice buttons expose toggle state via aria-pressed", () => {
  // The wizard's agent / preset picker uses createChoiceButton for
  // mutually-exclusive options. Without aria-pressed, screen readers
  // can't announce which option is currently chosen — they just hear
  // a button list with no selection state.
  assert.match(
    appSource,
    /button\.setAttribute\("aria-pressed",\s*selected\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected createChoiceButton to set aria-pressed based on selected",
  );
});

test("Launch wizard separates launch settings from runtime controls", () => {
  assert.equal(
    appSource.includes("wizardAdvancedOpen"),
    false,
    "Launch wizard should not keep an Advanced disclosure state",
  );
  for (const retiredCopy of ["Advanced", "Show advanced", "Hide advanced"]) {
    assert.equal(
      appSource.includes(`"${retiredCopy}"`),
      false,
      `Launch wizard should not render ${retiredCopy}`,
    );
  }

  const launchSettingsStart = appSource.indexOf(
    'createLaunchSection(\n            "Launch settings"',
  );
  const linkedIssueStart = appSource.indexOf(
    'createLaunchSection(\n            "Linked issue"',
  );
  const runtimeStart = appSource.indexOf(
    'createLaunchSection(\n            "Runtime"',
  );

  assert.notEqual(launchSettingsStart, -1, "expected Launch settings section");
  assert.notEqual(linkedIssueStart, -1, "expected Linked issue section");
  assert.notEqual(runtimeStart, -1, "expected Runtime section");
  assert.ok(
    launchSettingsStart < linkedIssueStart && linkedIssueStart < runtimeStart,
    "expected Launch settings before Linked issue and Runtime after Linked issue",
  );

  const launchSettingsBlock = appSource.slice(
    launchSettingsStart,
    linkedIssueStart,
  );
  for (const copy of ["Version", "Skip permission prompts", "Fast mode"]) {
    assert.ok(
      launchSettingsBlock.includes(`"${copy}"`),
      `expected Launch settings to include ${copy}`,
    );
  }
  assert.equal(
    launchSettingsBlock.includes('"Runtime target"'),
    false,
    "Launch settings should not contain runtime target controls",
  );

  const runtimeBlock = appSource.slice(
    runtimeStart,
    appSource.indexOf("wizardBody.appendChild(panel);"),
  );
  for (const copy of ["Runtime target", "Docker service", "Docker lifecycle"]) {
    assert.ok(
      runtimeBlock.includes(`"${copy}"`),
      `expected Runtime to include ${copy}`,
    );
  }
  for (const copy of ["Version", "Skip permission prompts", "Fast mode"]) {
    assert.equal(
      runtimeBlock.includes(`"${copy}"`),
      false,
      `Runtime should not contain ${copy}`,
    );
  }
});

test("Launch wizard runtime confirmation shows summary without setup forms", () => {
  assert.match(
    appSource,
    /const showManualSetup = launchWizard\.show_manual_setup !== false;[\s\S]*?const isRuntimeConfirmation = Boolean\([\s\S]*?const showStartMethods = Boolean\([\s\S]*?launchWizard\.show_start_methods[\s\S]*?const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected showManualSetup to be initialized before Runtime confirmation setup gating",
  );
  assert.match(
    appSource,
    /const isRuntimeConfirmation = Boolean\(\s*launchWizard\.runtime_context_resolved\s*&&\s*launchWizard\.show_runtime_confirmation\s*\);/,
    "expected renderer to derive a dedicated Runtime confirmation state",
  );
  assert.match(
    appSource,
    /const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected manual setup forms to be suppressed during Runtime confirmation",
  );
  assert.match(
    appSource,
    /renderWizardSummary\(\);[\s\S]*?const isRuntimeConfirmation = Boolean/,
    "expected read-only launch summary to remain visible before Runtime-only body rendering",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showStartMethods\s*\)/,
    "expected Start methods rows to be gated by backend start-method state",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_branch_controls !== false\s*\)/,
    "expected Branch controls to be part of setup forms only",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch"/,
    "expected Launch controls to be part of setup forms only",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&[\s\S]*?launchWizard\.show_fast_mode[\s\S]*?\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch settings"/,
    "expected Launch settings controls to be part of setup forms only",
  );
  // SPEC-2014 2026-05-29 amendment (FR-109): Fast mode is now a toggle switch
  // (appendToggleField) instead of a checkbox, but stays provider-neutral —
  // wired to launchWizard.fast_mode + set_fast_mode, not Codex-only state.
  assert.match(
    appSource,
    /appendToggleField\(\s*grid,\s*"Fast mode"[\s\S]*?launchWizard\.fast_mode[\s\S]*?kind:\s*"set_fast_mode"/,
    "expected Launch settings to wire provider-neutral Fast mode controls",
  );
  assert.doesNotMatch(
    appSource,
    /Codex fast mode/,
    "expected Launch settings copy to avoid Codex-only Fast mode wording",
  );
  // SPEC-2014 Amendment 2026-05-20 (FR-057): Linked issue is gated by its
  // dedicated `show_linked_issue` flag so it only appears when the wizard
  // was opened through the Knowledge Issue Bridge.
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_linked_issue\s*\)/,
    "expected Linked issue controls to be gated by show_linked_issue and setup forms",
  );
});

test("Launch wizard submit button uses Continue before runtime context is resolved", () => {
  assert.match(
    appSource,
    /launchWizard\.primary_action_label\s*\|\|[\s\S]{0,260}?launchWizard\.runtime_context_resolved === false\s*\?\s*"Continue"\s*:/,
    "expected unresolved Launch Agent runtime context to use Continue instead of Launch",
  );
});

test("Launch wizard keeps cancel available during runtime resolution", () => {
  const closeHelper = appSource.match(
    /function closeLaunchWizardFromChrome\(\) \{([\s\S]*?)\n      \}/,
  );
  assert.ok(closeHelper, "expected launch wizard close helper");
  assert.equal(
    closeHelper[1].includes("runtime_resolution_pending"),
    false,
    "runtime resolution pending must not block the footer Cancel button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.disabled\s*=\s*false/,
    "Cancel button must stay enabled while runtime resolution is pending",
  );
  const escapeHandler = appSource.match(
    /if \(wizardModal\.classList\.contains\("open"\)\) \{([\s\S]*?)event\.preventDefault\(\);\n          return;/,
  );
  assert.ok(escapeHandler, "expected launch wizard Escape handler");
  assert.equal(
    escapeHandler[1].includes("runtime_resolution_pending"),
    false,
    "Escape must keep using the cancel path while runtime resolution is pending",
  );
});

test("Launch wizard disables panel controls while runtime resolution is pending", () => {
  assert.match(
    appSource,
    /const selector =\n\s+"input, textarea, select, button, \[role='button'\], \[contenteditable='true'\]";[\s\S]*?querySelectorAll\(selector\)/,
    "expected a pending-state helper to disable wizard panel controls",
  );
  assert.match(
    appSource,
    /panel\.classList\.toggle\(\s*"wizard-disabled"[\s\S]{0,180}?isRuntimeResolutionPending\s*\|\|\s*isLaunchActionPending/,
    "expected the launch panel to expose a disabled visual state while pending",
  );
  assert.match(
    inlineStyle,
    /\.launch-panel\.wizard-disabled[\s\S]*?pointer-events:\s*none;/,
    "expected pending wizard controls to ignore pointer events",
  );
});

test("Launch wizard renders centered split flow without backdrop dismissal", () => {
  assert.equal(
    appSource.includes("isStartWorkMode"),
    false,
    "Start Work should share the centered wizard modal instead of toggling drawer mode",
  );
  assert.equal(
    appSource.includes('wizardModal.classList.toggle("is-drawer"'),
    false,
    "Launch Wizard should not toggle drawer placement",
  );
  assert.ok(
    appSource.includes("wizard-progress-rail")
      && appSource.includes("wizard-main")
      && appSource.includes("wizard-content-pane"),
    "expected the wizard body to be split into progress rail and content pane",
  );
  assert.equal(
    /if\s*\(\s*event\.target === wizardModal\s*\)\s*\{\s*closeLaunchWizardFromChrome\(\);\s*\}/.test(appSource),
    false,
    "wizard backdrop clicks must not dismiss the wizard",
  );
});

test("Launch wizard start methods keep disabled and hover states distinct", () => {
  assert.match(
    inlineStyle,
    /\.start-method-button:hover:not\(:disabled\)/,
    "enabled start method rows should expose a hover state",
  );
  assert.match(
    inlineStyle,
    /\.start-method-button:disabled/,
    "disabled start method rows should expose a disabled state",
  );
});

test("Launch wizard pending launch feedback has stable styling hooks", () => {
  assert.match(
    inlineStyle,
    /\.modal-shell\.is-wizard\.is-launch-pending[\s\S]*?cursor:\s*progress;/,
    "expected Launch Wizard modal to expose progress cursor while a launch action is pending",
  );
  assert.match(
    inlineStyle,
    /\.start-method-button\.is-pending[\s\S]*?cursor:\s*progress;/,
    "expected pending Start method row to expose a progress cursor",
  );
  assert.match(
    inlineStyle,
    /\.launch-pending-note[\s\S]*?color:\s*var\(--color-text-strong\)/,
    "expected pending launch copy to be styled as visible modal feedback",
  );
});

test("Launch wizard renders start methods as direct actions", () => {
  assert.ok(
    appSource.includes('"Start methods"'),
    "expected Launch Wizard to label the first section as Start methods",
  );
  assert.ok(
    appSource.includes("start_methods")
      && appSource.includes("show_start_methods")
      && appSource.includes('kind: "use_start_method"'),
    "expected start method rows to dispatch direct backend actions",
  );
  assert.equal(
    appSource.includes('"Quick start"'),
    false,
    "Launch Wizard must not expose the old Quick start heading",
  );
  assert.equal(
    appSource.includes('kind: "select_quick_start"'),
    false,
    "start methods should not use the old selection-before-footer-submit model",
  );
  assert.equal(
    appSource.includes("quick-start-actions"),
    false,
    "start methods should not render multiple inline action buttons",
  );
});

test("Selected list rows mark active item with aria-current", () => {
  // Same pattern as project tabs (PR #2455): list-style buttons with a
  // selected state need aria-current to announce which row is active.
  // Coverage: knowledge-row, profile-row, logs-entry.
  // The set/remove pair is required so a previously-selected row
  // doesn't retain the marker after the user picks a different row.
  for (const desc of [
    "knowledge",
    "profile",
    "logs entry",
  ]) {
    // Each list iterates over its rows and conditionally sets
    // aria-current="true" on the selected one. We assert both the
    // setAttribute and removeAttribute calls exist somewhere in app.js.
    // The descriptive label is just for the failure message.
  }
  // Count occurrences: knowledge, profile, logs, file-tree, plus the
  // project-tabs case from PR #2455.
  const setMatches =
    appAndProjectTabsSource.match(/setAttribute\("aria-current",\s*"(true|page)"\)/g) || [];
  const removeMatches =
    appAndProjectTabsSource.match(/removeAttribute\("aria-current"\)/g) || [];
  assert.ok(
    setMatches.length >= 5,
    `expected >= 5 aria-current set calls (project tab + 4 row types), got ${setMatches.length}`,
  );
  assert.ok(
    removeMatches.length >= 5,
    `expected >= 5 aria-current remove calls (one per set call), got ${removeMatches.length}`,
  );
});

test("Project tabs mark the active project with aria-current=\"page\"", () => {
  // Project tabs use role="button" but represent a navigation choice. The
  // appropriate ARIA pattern is aria-current="page" on the active tab so
  // screen readers announce it as the current location. Inactive tabs
  // must explicitly clear the attribute so the previously-active tab
  // doesn't retain the marker after a switch.
  assert.match(
    projectTabsRendererSource,
    /button\.setAttribute\("aria-current",\s*"page"\)/,
    "expected the active project tab to set aria-current=\"page\"",
  );
  assert.match(
    projectTabsRendererSource,
    /button\.removeAttribute\("aria-current"\)/,
    "expected inactive project tabs to remove aria-current",
  );
});

test("Error regions declare role=\"alert\" so screen readers announce them", () => {
  // Without role="alert" (or aria-live="assertive"), errors that appear
  // in wizard-error and project-picker-error are silent for screen reader
  // users — they only see them after manually navigating to the error
  // element. role="alert" implies aria-live="assertive" + aria-atomic so
  // the full message is announced immediately when the element becomes
  // non-hidden.
  for (const id of ["wizard-error", "project-picker-error"]) {
    const el = document.getElementById(id);
    assert.ok(el, `expected error region #${id}`);
    assert.equal(el.getAttribute("role"), "alert", `${id} must have role="alert"`);
  }
});

test("Wizard and preset modals activate focus-trap on open and release on close (app.js inline)", () => {
  // The wizard and preset modals are rendered inline in app.js (not in
  // separate renderer modules), so they need closure-scoped trap-release
  // variables. Pin both pairs.
  assert.match(appSource, /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\/focus-trap\.js"/);
  // Wizard
  assert.match(appSource, /let\s+wizardFocusTrapRelease\s*=\s*null/);
  assert.match(
    appSource,
    /wizardFocusTrapRelease\s*=\s*createFocusTrap\(wizardDialog,\s*\{\s*document\s*\}\)/,
    "wizard must activate focus trap on the dialog",
  );
  assert.match(
    appSource,
    /typeof\s+wizardFocusTrapRelease\s*===\s*"function"\s*\)\s*\{\s*wizardFocusTrapRelease\(\)/,
    "wizard must release focus trap on close",
  );
  // Preset
  assert.match(appSource, /let\s+presetModalFocusTrapRelease\s*=\s*null/);
  assert.match(
    appSource,
    /presetModalFocusTrapRelease\s*=\s*createFocusTrap\(presetShell,\s*\{\s*document\s*\}\)/,
    "preset modal must activate focus trap on the shell",
  );
  assert.match(
    appSource,
    /typeof\s+presetModalFocusTrapRelease\s*===\s*"function"\s*\)\s*\{\s*presetModalFocusTrapRelease\(\)/,
    "preset modal must release focus trap on close",
  );
});

test("Drawer modal renderers activate focus-trap on open and release on close", () => {
  // Focus trap prevents Tab from escaping the modal into background
  // content. Both renderers must:
  // - Import createFocusTrap
  // - Maintain a focusTrapMap WeakMap to track release functions
  // - Call createFocusTrap on fresh open and store the release
  // - Invoke the release before restoring focus on close
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\.\/focus-trap\.js"/,
      `${name} must import createFocusTrap from ./focus-trap.js (relative path so Node tests resolve)`,
    );
    assert.match(src, /const\s+focusTrapMap\s*=\s*new\s+WeakMap\(\)/, `${name} must define focusTrapMap`);
    assert.match(
      src,
      /const\s+release\s*=\s*createFocusTrap\(dialogEl,\s*\{\s*document:\s*ownerDoc\s*\}\)/,
      `${name} must activate focus trap on open with the dialog as container`,
    );
    assert.match(
      src,
      /focusTrapMap\.set\(modalEl,\s*release\)/,
      `${name} must store the trap release in focusTrapMap`,
    );
    assert.match(
      src,
      /releaseTrap\s*=\s*focusTrapMap\.get\(modalEl\)[\s\S]*if\s*\(typeof\s+releaseTrap\s*===\s*"function"\)\s*releaseTrap\(\)/,
      `${name} must release the trap before restoring focus on close`,
    );
  }
});

test("Dynamically-created form fields without surrounding <label> have aria-label", () => {
  // Form fields that aren't wrapped in a <label> element need aria-label
  // so screen readers announce their purpose instead of just "edit text"
  // (input/textarea) or the first option (select). This audit covers
  // the launch wizard and profile editor surfaces.
  // SPEC-2014 Amendment 2026-05-20 (FR-058): the wizard "Issue number"
  // field is now rendered as static read-only text (no <input>), so its
  // accessibility name is conveyed by the surrounding launch-field label
  // instead of a per-input aria-label. The audit entry is removed
  // accordingly.
  const expected = [
    { selector: 'input\\.setAttribute\\("aria-label", "Branch name"\\)', desc: "wizard branch name" },
    { selector: 'keyInput\\.setAttribute\\("aria-label", `Environment variable key, row ', desc: "env var key" },
    { selector: 'modeSelect\\.setAttribute\\("aria-label", `Environment variable mode, row ', desc: "env var mode" },
    { selector: 'valueInput\\.setAttribute\\("aria-label", `Profile value, row ', desc: "profile env value" },
    { selector: 'select\\.setAttribute\\("aria-label", label\\)', desc: "launch-field select reuses label" },
  ];
  for (const { selector, desc } of expected) {
    assert.match(
      appSource,
      new RegExp(selector),
      `expected ${desc} field to have aria-label set`,
    );
  }
});

test("Every role=\"dialog\" element has a programmatic accessible name", () => {
  // Catch the case where a new dialog is added without aria-label or
  // aria-labelledby — without an accessible name, screen readers
  // announce just "dialog" with no context. Iterates the live DOM
  // tree so future additions are covered automatically.
  const dialogs = document.querySelectorAll('[role="dialog"]');
  assert.ok(dialogs.length >= 5, `expected >= 5 role="dialog" elements, got ${dialogs.length}`);
  for (const dialog of dialogs) {
    const label = dialog.getAttribute("aria-label");
    const labelledby = dialog.getAttribute("aria-labelledby");
    const id = dialog.getAttribute("id") || dialog.parentElement?.getAttribute("id") || "(no id)";
    assert.ok(
      (label && label.length > 0) || (labelledby && labelledby.length > 0),
      `role="dialog" at #${id} must have aria-label or aria-labelledby`,
    );
    if (labelledby) {
      const target = document.getElementById(labelledby);
      assert.ok(
        target,
        `role="dialog" at #${id} aria-labelledby="${labelledby}" must point at an existing element`,
      );
      assert.ok(
        target.textContent && target.textContent.trim().length > 0,
        `role="dialog" at #${id} aria-labelledby target must have non-empty text`,
      );
    }
  }
});

test("Migration progress bar references the phase label via aria-labelledby", () => {
  // The native <progress> element gets an implicit role="progressbar"
  // but no programmatic name unless aria-label or aria-labelledby is
  // provided. Without it, screen readers announce just "progressbar 50%"
  // and the user doesn't know which phase is running.
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  assert.match(
    migrationSrc,
    /phaseLabel\.id\s*=\s*"migration-modal-phase-label"/,
    "expected migration phase label to have a stable id",
  );
  assert.match(
    migrationSrc,
    /progress\.setAttribute\("aria-labelledby",\s*"migration-modal-phase-label"\)/,
    "expected migration <progress> to reference the phase label via aria-labelledby",
  );
});

test("Drawer modal renderers signal busy state via aria-busy during async stages", () => {
  // The branch-cleanup running stage and migration running stage are
  // synchronous async operations. Without aria-busy, screen readers don't
  // signal that the dialog is in a loading state — users may try to
  // interact with controls that are about to disappear.
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /dialogEl\.setAttribute\("aria-busy",\s*"true"\)/,
      `${name} must set aria-busy="true" during the running stage`,
    );
    assert.match(
      src,
      /dialogEl\.setAttribute\("aria-busy",\s*"false"\)/,
      `${name} must clear aria-busy in non-running stages`,
    );
  }
});

test("Drawer modal renderers restore focus on close (WeakMap pattern)", () => {
  // Companion to fresh-open focus management: when the modal closes the
  // trigger element captured at open time must regain focus, otherwise
  // keyboard users land on document.body. Both renderers must implement
  // the WeakMap-keyed return-focus pattern.
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(src, /const\s+focusReturnMap\s*=\s*new\s+WeakMap\(\)/, `${name} must define focusReturnMap WeakMap`);
    assert.match(src, /focusReturnMap\.set\(modalEl,\s*ownerDoc\.activeElement\)/, `${name} must save activeElement on open`);
    assert.match(src, /focusReturnMap\.get\(modalEl\)/, `${name} must read trigger on close`);
    assert.match(src, /focusReturnMap\.delete\(modalEl\)/, `${name} must clear the map after close`);
    assert.match(src, /returnTo\.focus\(\{\s*preventScroll:\s*true\s*\}\)/, `${name} must restore focus with preventScroll`);
  }
});

test("Drawer modal renderers move focus into dialog on fresh open", () => {
  // The Hotkey overlay (PR #2440) already does this; the drawer modals
  // need parity so screen readers announce them and keyboard users land
  // inside. Re-renders during running/result stages must NOT re-move focus
  // (users keep their place navigating buttons).
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    // Each renderer must capture wasOpen BEFORE adding the .open class
    // and gate the focus move on !wasOpen.
    assert.match(
      src,
      /const\s+wasOpen\s*=\s*modalEl\.classList\.contains\("open"\)/,
      `${name} must capture wasOpen before adding the .open class`,
    );
    assert.match(
      src,
      /!wasOpen[\s\S]*dialogEl\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
      `${name} must focus dialogEl with preventScroll only on fresh open`,
    );
  }
});

test("Preset (Add Window) modal manages focus on open and restores on close", () => {
  // The "+" Add Window modal opens via openModal()/closeModal() in app.js.
  // Verify the same focus-management pattern is wired: capture trigger,
  // move focus to modal-shell on open, restore on close.
  assert.match(appSource, /let\s+presetModalFocusReturn\s*=\s*null/);
  assert.match(appSource, /presetModalFocusReturn\s*=\s*document\.activeElement/);
  assert.match(
    appSource,
    /presetShell\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected preset modal shell focus on open with preventScroll",
  );
  assert.match(
    appSource,
    /presetModalFocusReturn\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected preset modal restore focus on close with preventScroll",
  );
});

test("Wizard modal manages focus on open and restores on close", () => {
  // The wizard modal's renderer lives in app.js (not a separate module),
  // so focus management is wired inline. Verify the same pattern as the
  // drawer modals: wizardFocusReturn captures activeElement on open,
  // wizardDialog.focus({ preventScroll: true }) moves focus into the
  // dialog, and wizardFocusReturn.focus() restores on close.
  assert.match(appSource, /let\s+wizardFocusReturn\s*=\s*null/);
  assert.match(appSource, /wizardFocusReturn\s*=\s*document\.activeElement/);
  assert.match(
    appSource,
    /wizardDialog\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected wizardDialog focus on open with preventScroll",
  );
  assert.match(
    appSource,
    /wizardFocusReturn\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected wizardFocusReturn focus on close with preventScroll",
  );
});

test("Drawer modals close on Escape — keyboard parity with backdrop click", () => {
  // The Hotkey overlay and Command Palette already had Esc-close (PR #2440).
  // branch-cleanup and migration modals previously closed only on backdrop
  // click — keyboard users were trapped. app.js must wire a global keydown
  // that maps Escape to the same close path for both modals.
  assert.match(
    appSource,
    /event\.key\s*!==?\s*"Escape"/,
    "expected app.js to gate keydown on Escape",
  );
  assert.match(
    appSource,
    /branchCleanupModal\.classList\.contains\("open"\)[\s\S]*closeBranchCleanupModal/,
    "expected Esc to call closeBranchCleanupModal when that modal is open",
  );
  // SPEC-1934 US-7 / FR-032: the confirmation modal is Accept-only. Esc no
  // longer routes to skip_migration; at the confirm stage it is swallowed
  // (no backend send), at the error stage it dismisses the UI without
  // flipping `migration_pending`.
  assert.doesNotMatch(
    appSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]{0,400}skip_migration/,
    "Esc must not send skip_migration when the Accept-only migration modal is open",
  );
  assert.match(
    appSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]*migrationModalState\.stage\s*===\s*"error"[\s\S]*migrationModalState\.open\s*=\s*false/,
    "expected Esc on the migration modal error stage to dismiss the dialog without backend skip",
  );
  assert.match(
    appSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]*launchWizardSurface\.sendAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc to send cancel when wizard modal is open",
  );
  assert.match(
    appSource,
    /if\s*\(windowListOpen\)\s*\{[\s\S]*windowListOpen\s*=\s*false[\s\S]*windowListButton\.focus/,
    "expected Esc to close window list dropdown and restore focus to trigger",
  );
  // SPEC-2356 — preset modal Esc-close: closes via closeModal() which
  // handles both the .open class flip and focus restore.
  assert.match(
    appSource,
    /if\s*\(modal\.classList\.contains\("open"\)\)\s*\{[\s\S]*closeModal\(\)/,
    "expected Esc to call closeModal when preset modal is open",
  );
});

test("Drawer modal renderers toggle aria-hidden alongside .open class", () => {
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  // Both renderers must flip aria-hidden in lockstep with the .open class.
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /modalEl\.setAttribute\("aria-hidden",\s*"true"\)/,
      `${name} must set aria-hidden="true" on close`,
    );
    assert.match(
      src,
      /modalEl\.removeAttribute\("aria-hidden"\)/,
      `${name} must remove aria-hidden on open`,
    );
  }
});

test("Hotkey overlay manages focus on open / close (modal dialog a11y)", () => {
  // The dialog must be focusable so we can move focus inside on open and
  // return it to the trigger on close — without this, screen readers don't
  // announce the dialog and keyboard users get stranded after Esc.
  const card = document.querySelector("#op-hotkey-overlay .op-hotkey-card");
  assert.ok(card, "expected hotkey overlay card");
  assert.equal(card.getAttribute("tabindex"), "-1");
  assert.equal(card.getAttribute("role"), "dialog");
  assert.equal(card.getAttribute("aria-modal"), "true");
  // operator-shell.js wires the dynamic focus pieces.
  assert.match(operatorShellSource, /returnFocusTo\s*=\s*doc\.activeElement/);
  assert.match(operatorShellSource, /card\.focus\(\{\s*preventScroll:\s*true\s*\}\)/);
  assert.match(operatorShellSource, /returnFocusTo\.focus/);
});

test("Command Palette implements the WAI-ARIA combobox/listbox pattern", () => {
  // The input must be a combobox that controls the list and announces the
  // active option via aria-activedescendant. Without these the listbox role
  // already on the <ul> is meaningless to screen readers.
  const paletteInput = document.querySelector("#op-palette-input");
  assert.ok(paletteInput, "expected palette input");
  assert.equal(paletteInput.getAttribute("role"), "combobox");
  assert.equal(paletteInput.getAttribute("aria-controls"), "op-palette-list");
  assert.equal(paletteInput.getAttribute("aria-autocomplete"), "list");
  // aria-expanded starts false; operator-shell flips it on open/close.
  assert.equal(paletteInput.getAttribute("aria-expanded"), "false");
  assert.match(paletteInput.getAttribute("aria-label") ?? "", /command/i);
  // operator-shell.js must wire the dynamic pieces.
  assert.match(operatorShellSource, /input\.setAttribute\("aria-expanded",\s*"true"\)/);
  assert.match(operatorShellSource, /input\.setAttribute\("aria-activedescendant"/);
  assert.match(operatorShellSource, /input\.removeAttribute\("aria-activedescendant"\)/);
  assert.match(operatorShellSource, /li\.setAttribute\("role",\s*"option"\)/);
  assert.match(operatorShellSource, /li\.setAttribute\("aria-selected"/);
  assert.match(operatorShellSource, /li\.id\s*=\s*`op-palette-row-\$\{idx\}`/);
});

test("op-drawer scaffold honors prefers-reduced-motion (parity with legacy is-drawer modal)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The legacy `.modal-backdrop.is-drawer .modal-shell.is-drawer-shell` already
  // suppresses its slide transform under reduced-motion. The forward-looking
  // op-drawer scaffold needed parity — without this guard, future direct
  // adoption would slide in even for users who opted out of motion.
  const reducedMotionBlocks = css.match(
    /@media\s*\(\s*prefers-reduced-motion:\s*reduce\s*\)\s*\{[\s\S]*?\n\}/g,
  );
  assert.ok(reducedMotionBlocks, "expected at least one prefers-reduced-motion block");
  const opDrawerCovered = reducedMotionBlocks.some(
    (block) => /\.op-drawer\b/.test(block) && /transition:\s*none/.test(block),
  );
  assert.ok(opDrawerCovered, ".op-drawer scaffold must drop its transition under reduced-motion");
});

test("mapAgentTelemetryState emits only Living Telemetry states CSS handles", () => {
  // app.js has a closure-scoped runtime→Living Telemetry mapper. CSS only
  // styles `[data-agent-state]` for declared telemetry states — any drift
  // (e.g. emitting "warn" or "exited") would silently render no rim. Pin
  // the contract so refactors can't introduce undeclared states.
  const mapperBlock = appSource.match(
    /function\s+mapAgentTelemetryState\s*\([^)]*\)\s*\{[\s\S]*?\n\s{6,8}\}/,
  );
  assert.ok(mapperBlock, "expected mapAgentTelemetryState to be defined in app.js");
  const returnedStates = new Set();
  for (const m of mapperBlock[0].matchAll(/return\s+"([^"]+)"/g)) {
    returnedStates.add(m[1]);
  }
  const allowed = new Set(["active", "not_started", "idle", "blocked", "done"]);
  for (const state of returnedStates) {
    assert.ok(allowed.has(state), `mapAgentTelemetryState returned undeclared state: ${state}`);
  }
  // And the four design states must all be reachable, not just allowed.
  for (const required of allowed) {
    assert.ok(returnedStates.has(required), `Living Telemetry state never emitted: ${required}`);
  }
});

test("Status Strip ACTIVE / IDLE / BLOCKED cells all tint with their state color", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The ACTIVE / IDLE cells previously had no tonal hint — only BLOCKED did.
  // Add parallel symmetry so the three count cells render with matching state
  // colors (cyan / gray / red) for at-a-glance scanning.
  assert.match(css, /\.op-status-strip__cell--active\s+\.op-status-strip__value\s*\{[^}]*--color-state-active/);
  assert.match(css, /\.op-status-strip__cell--idle\s+\.op-status-strip__value\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-status-strip__cell--blocked\s+\.op-status-strip__value\s*\{[^}]*--color-state-blocked/);
  // Markup also needs the modifiers wired so the CSS selectors actually match.
  const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--active/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--idle/);
});

test("agent cards style all four Living Telemetry states (active / blocked / idle / done)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // Each of the four states must have a distinct visual treatment so operators
  // can scan card boundaries at a glance — not just the high-priority pair.
  assert.match(css, /\.op-agent-card\[data-state="active"\]\s*\{[^}]*--color-state-active/);
  assert.match(css, /\.op-agent-card\[data-state="blocked"\]\s*\{[^}]*--color-state-blocked/);
  assert.match(css, /\.op-agent-card\[data-state="idle"\]\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-agent-card\[data-state="done"\]\s*\{[^}]*--color-state-done/);
  // The chip labels also need the matching foreground tint per state.
  assert.match(css, /\.op-agent-card\[data-state="idle"\]\s*\.op-agent-state\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-agent-card\[data-state="done"\]\s*\.op-agent-state\s*\{[^}]*--color-state-done/);
});

test("terminal surface body stays on the dark Operator canvas across themes (FR-013)", () => {
  // SPEC-2356 FR-013 改定: terminal window の body 領域 (.window-body と
  // terminal-root padding) は xterm の Dark Operator background と連続させ
  // るため、Light テーマでも Dark Operator background に固定する。Light で
  // --color-canvas (#e9edf0) が見えると xterm 周囲に「外枠」が現れて二重
  // 枠になるので、ここで結合してしまうのが正解。
  const surfaceTerminalRule =
    /\.surface-terminal\s+\.window-body\s*\{[^}]*background:\s*([^;]+);/;
  const match = inlineStyle.match(surfaceTerminalRule);
  assert.ok(match, "expected .surface-terminal .window-body rule with explicit background");
  const value = match[1].trim();
  assert.doesNotMatch(
    value,
    /var\(\s*--color-canvas\s*\)/,
    `.surface-terminal .window-body must not follow --color-canvas (got "${value}")`,
  );
  assert.doesNotMatch(
    value,
    /var\(\s*--color-surface(?:-elevated)?\s*\)/,
    `.surface-terminal .window-body must not follow surface tokens (got "${value}")`,
  );
  // Accept either an explicit dark hex (Dark Operator canvas) or a dedicated
  // dark canvas token.  We codify the value contract so the regression cannot
  // silently flip back to a light token.
  const usesDarkOperatorBackground =
    /^#0a0d12$/i.test(value) || /var\(\s*--color-canvas-dark\s*\)/.test(value);
  assert.ok(
    usesDarkOperatorBackground,
    `.surface-terminal .window-body must use Dark Operator canvas (#0a0d12 or --color-canvas-dark); got "${value}"`,
  );
});

test("terminal root spacing stays inside the window body (SPEC-2008 FR-060)", () => {
  const terminalRootRule = inlineStyle.match(/\.terminal-root\s*\{([^}]*)\}/);
  assert.ok(terminalRootRule, "expected .terminal-root CSS rule");
  const body = terminalRootRule[1];

  assert.match(
    body,
    /position:\s*absolute/,
    ".terminal-root must remain absolutely positioned inside .window-body",
  );
  assert.match(
    body,
    /inset:\s*8px\s+4px\s+4px\s*;/,
    ".terminal-root must express terminal chrome spacing as inset so its outer box stays inside .window-body (Issue #2923 follow-up: side / bottom insets reduced to 4px so the cell grid recovers ~1 column at the gwt-default 720×420 window)",
  );
  assert.match(
    body,
    /overflow:\s*hidden/,
    ".terminal-root must clip xterm internals within the in-bounds terminal host box",
  );
});

test("terminal root does not combine full inset with padding overflow (SPEC-2008 SC-039)", () => {
  const terminalRootRule = inlineStyle.match(/\.terminal-root\s*\{([^}]*)\}/);
  assert.ok(terminalRootRule, "expected .terminal-root CSS rule");
  const body = terminalRootRule[1];

  assert.doesNotMatch(
    body,
    /inset:\s*0\s*;[\s\S]*padding:\s*(?!0\b)[^;]+;/,
    ".terminal-root must not use inset:0 plus nonzero padding; content-box padding extends past .window-body and clips the bottom prompt",
  );
  assert.match(
    body,
    /padding:\s*0\s*;/,
    ".terminal-root padding must stay zero so FitAddon measures the same box xterm can paint into",
  );
});

test("terminal transcript host paints every xterm layer as an opaque dark body (SPEC-2356 FR-035)", () => {
  const expectedSelectors = [
    ".surface-terminal .terminal-root",
    ".surface-terminal .terminal-root .xterm",
    ".surface-terminal .terminal-root .xterm-viewport",
    ".surface-terminal .terminal-root .xterm-screen",
    ".surface-terminal .terminal-root .xterm-helper-textarea",
  ];

  for (const selector of expectedSelectors) {
    const block = cssBlockContaining(inlineStyle, selector);
    assert.match(
      block,
      /background:\s*(?:#0a0d12|var\(\s*--color-canvas-dark\s*\))/i,
      `${selector} must explicitly paint the Dark Operator terminal background`,
    );
    assert.doesNotMatch(
      block,
      /transparent|backdrop-filter|opacity:\s*0\.[0-9]/i,
      `${selector} must not let the Canvas or sibling windows show through text`,
    );
  }
});

test("terminal overlay surfaces are opaque and copyable, not translucent glass (SPEC-2356 FR-033)", () => {
  for (const selector of [".terminal-overlay", ".terminal-overlay.visible"]) {
    const block = cssBlockContaining(inlineStyle, selector);
    assert.doesNotMatch(
      block,
      /transparent|backdrop-filter|opacity:\s*0\.[0-9]/i,
      `${selector} must not use translucent readable backgrounds`,
    );
    assert.match(
      block,
      /background:\s*(?:#0a0d12|var\(\s*--color-canvas-dark\s*\)|var\(\s*--color-surface(?:-elevated)?\s*\)|color-mix\([^;]+var\(\s*--color-canvas-dark\s*\)[^;]+\))/i,
      `${selector} must use an opaque terminal/text surface background`,
    );
  }
});

test("agent-state telemetry never makes readable workspace windows translucent (SPEC-2356 FR-033)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  for (const state of ["idle", "not_started"]) {
    const selector = `.workspace-window[data-agent-state="${state}"]`;
    const block = cssBlockContaining(css, selector);
    assert.match(
      block,
      /opacity:\s*1\s*;/,
      `${selector} must keep the entire window fully opaque; state dimming belongs on non-window telemetry affordances only`,
    );
  }

  for (const state of ["idle", "not_started"]) {
    const block = cssBlockContaining(css, `[data-agent-state="${state}"]`);
    assert.doesNotMatch(
      block,
      /opacity:\s*0\.[0-9]/,
      `[data-agent-state="${state}"] must not dim every descendant surface by applying opacity to the parent window`,
    );
  }
});

test("non-terminal surface bodies still follow the overall theme (FR-013 boundary)", () => {
  // The Dark fix is scoped to .surface-terminal.  Other surfaces (Board /
  // Logs / File Tree / Branches / Knowledge / Workspace / Console / Mock / Profile) must
  // keep tracking the active theme via --color-surface so tabbed windows
  // still flip body color when a non-terminal tab is selected.
  const otherSurfaceRule =
    /(?:\.surface-(?:file-tree|branches|board|logs|knowledge|index|workspace|console|mock|profile)\s+\.window-body,?\s*)+\{[^}]*background:\s*var\(\s*--color-surface\s*\)/;
  assert.match(
    inlineStyle,
    otherSurfaceRule,
    "non-terminal surface bodies must continue to use var(--color-surface)",
  );
});

test("mountWindowBody clears every known surface class before applying the active surface", () => {
  const mountBody = appSource.match(
    /function\s+mountWindowBody\(windowData,\s*element\)\s*\{[\s\S]*?if\s*\(surface\s*===\s*"terminal"\)/,
  );
  assert.ok(mountBody, "expected mountWindowBody implementation");
  for (const surfaceClass of [
    "surface-terminal",
    "surface-file-tree",
    "surface-branches",
    "surface-board",
    "surface-logs",
    "surface-knowledge",
    "surface-index",
    "surface-work",
    "surface-profile",
    "surface-console",
    "surface-mock",
  ]) {
    assert.match(
      mountBody[0],
      new RegExp(`"${surfaceClass}"`),
      `mountWindowBody must remove stale ${surfaceClass} classes before adding the active surface`,
    );
  }
});

test("every readable non-terminal surface participates in the opaque window chrome grammar", () => {
  for (const surface of [
    "file-tree",
    "branches",
    "board",
    "logs",
    "knowledge",
    "index",
    "work",
    "profile",
    "console",
    "mock",
  ]) {
    assert.match(
      inlineStyle,
      new RegExp(`\\.workspace-window\\.surface-${surface}\\b`),
      `${surface} window shell must opt into the shared opaque surface background`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.titlebar\\b`),
      `${surface} titlebar must use the shared opaque titlebar rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.window-body\\b`),
      `${surface} body must use the shared opaque body rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.status-chip\\b`),
      `${surface} status chip text must use the shared readable chrome rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.icon-button\\b`),
      `${surface} icon button text must use the shared readable chrome rule`,
    );
  }
});

test("Sidebar Layer buttons reset UA chrome so Windows WebView2 stops drawing default border (FR-030)", () => {
  // SPEC-2356 FR-030 / US-4 AS-11: WebView2 / Chromium の `<button>` UA
  // default は border + grey background を出す。`.op-layer` は indicator
  // dot + label color + token-driven focus ring のみで状態を表現するため、
  // base rule で UA chrome を解除する必要がある。
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const layerRule = css.match(/\.op-layer\s*\{([^}]*)\}/);
  assert.ok(layerRule, "expected base .op-layer rule in components.css");
  const body = layerRule[1];
  assert.match(
    body,
    /appearance:\s*none/,
    ".op-layer must declare appearance:none to disable UA <button> chrome",
  );
  assert.match(
    body,
    /border:\s*0/,
    ".op-layer must zero out border so WebView2 stops drawing the default frame",
  );
  assert.match(
    body,
    /background:\s*transparent/,
    ".op-layer must clear background so the sidebar surface shows through",
  );
});

// --- SPEC-2359 Work Unification (US-49): Workspace → Work/Works labels ---

test("SC-207: user-facing UI labels must not contain 'Workspace'", () => {
  const sidebarLabel = document.querySelector("#op-workspace-overview-entry .op-layer__label");
  assert.ok(sidebarLabel, "expected sidebar overview entry to exist");
  assert.equal(sidebarLabel.textContent.trim(), "Work");
  assert.doesNotMatch(sidebarLabel.textContent, /Workspace/i);

  const sidebarAria = document.querySelector("#op-workspace-overview-entry");
  assert.doesNotMatch(sidebarAria.getAttribute("aria-label") ?? "", /workspace/i);

  const presetSections = document.querySelectorAll(".preset-section-label");
  for (const presetSection of presetSections) {
    if (/workspace/i.test(presetSection.textContent)) {
      assert.fail("preset section label must not contain 'Workspace'");
    }
  }

  const presetButtons = document.querySelectorAll(".preset-button strong");
  for (const btn of presetButtons) {
    assert.doesNotMatch(btn.textContent, /^Workspace$/i,
      `preset button "${btn.textContent}" must not say 'Workspace'`);
  }

  const allAriaLabels = Array.from(document.querySelectorAll("[aria-label]"))
    .map((el) => el.getAttribute("aria-label") ?? "");
  for (const label of allAriaLabels) {
    assert.doesNotMatch(label, /Workspace/i,
      `aria-label "${label}" must not contain 'Workspace'`);
  }
});

function cssBlockContaining(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(`(?:^|\\n)([^{}]*${escaped}[^{}]*)\\{([^}]*)\\}`, "m");
  const match = css.match(regex);
  assert.ok(match, `missing CSS rule containing selector: ${selector}`);
  return match[2];
}

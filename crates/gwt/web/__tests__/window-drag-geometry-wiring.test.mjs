// Issue #3364 — verify app.js wires the drag path into the same
// begin/commit local-geometry-edit lifecycle as the resize path, commits
// gestures through one shared helper (optimistic model sync + immediate
// Fleet Minimap render + revision-unconditional send), and resolves
// incoming workspace geometry through the content-matched guard BEFORE
// render keys / minimap / telemetry read the incoming windows.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("app.js imports resolveIncomingGeometry (content-matched guard) instead of the revision-arithmetic guard", () => {
  assert.match(
    appSource,
    /resolveIncomingGeometry\s*,?/,
    "expected app.js to import resolveIncomingGeometry from window-geometry-sync.js",
  );
  assert.doesNotMatch(
    appSource,
    /shouldApplyWorkspaceGeometry/,
    "the revision-arithmetic guard must be fully replaced (Issue #3364)",
  );
});

test("titlebar drag pointerdown begins a local geometry edit and records the base revision", () => {
  assert.match(
    appSource,
    /titlebar\.addEventListener\(\s*["']pointerdown["'][\s\S]{0,1600}?beginLocalGeometryEdit\(\s*geometrySyncState\s*,[\s\S]{0,300}?dragState\s*=\s*\{/,
    "expected the titlebar pointerdown handler to call beginLocalGeometryEdit before creating dragState",
  );
  assert.match(
    appSource,
    /dragState\s*=\s*\{[\s\S]{0,800}?baseGeometryRevision\s*:/,
    "expected dragState to capture baseGeometryRevision for the drop commit",
  );
});

test("a shared commitWindowGeometryGesture helper commits, syncs the model, renders the minimap, and sends unconditionally", () => {
  const helper = appSource.match(
    /function\s+commitWindowGeometryGesture\(([\s\S]*?)\n\s{6}\}/,
  );
  assert.ok(helper, "expected a commitWindowGeometryGesture helper function");
  const body = helper[0];
  assert.match(body, /commitLocalGeometryEdit\(/, "helper must commit the local edit guard");
  assert.match(
    body,
    /fleetMinimap\?\.renderCells\(\)/,
    "helper must re-render the Fleet Minimap immediately (no echo wait)",
  );
  assert.match(
    body,
    /sendGeometry\([\s\S]{0,200}?null\s*,?\s*\)/,
    "helper must send with a null base revision (explicit user placement, applied unconditionally)",
  );
});

test("the drag drop branch commits through the shared gesture helper", () => {
  assert.match(
    appSource,
    /if\s*\(\s*dragState\s*&&\s*dragState\.pointerId\s*===\s*event\.pointerId\s*\)\s*\{[\s\S]{0,2600}?commitWindowGeometryGesture\(\s*dragState\.id/,
    "expected the drag pointerup moved-branch to call commitWindowGeometryGesture",
  );
});

test("kanban / dock drops and no-move clicks clear the drag guard instead of committing", () => {
  assert.match(
    appSource,
    /agentKanbanTarget\s*\)\s*\{[\s\S]{0,220}?clearLocalGeometryEdit\(\s*geometrySyncState\s*,\s*dragState\.id\s*\)/,
    "expected the agent-kanban drop branch to clear the drag guard",
  );
  assert.match(
    appSource,
    /dragState\.dockTargetId\s*\)\s*\{[\s\S]{0,220}?clearLocalGeometryEdit\(\s*geometrySyncState\s*,\s*dragState\.id\s*\)/,
    "expected the dock drop branch to clear the drag guard",
  );
  assert.match(
    appSource,
    /clearLocalGeometryEdit\(\s*geometrySyncState\s*,\s*dragState\.id\s*\)[\s\S]{0,220}?handleTitlebarClick\(dragState\.id\)/,
    "expected the no-move click branch to clear the drag guard",
  );
});

test("drag pointercancel abandons the gesture by clearing the guard (server truth wins)", () => {
  assert.match(
    appSource,
    /pointerDragCancel[\s\S]{0,400}?clearLocalGeometryEdit\(\s*geometrySyncState\s*,\s*dragState\.id\s*\)[\s\S]{0,200}?dragState\s*=\s*null/,
    "expected the drag pointercancel branch to clear the local edit guard",
  );
});

test("finishWindowResize commits through the shared gesture helper", () => {
  assert.match(
    appSource,
    /function\s+finishWindowResize\([\s\S]{0,1400}?commitWindowGeometryGesture\(\s*resizeState\.id/,
    "expected finishWindowResize to route through commitWindowGeometryGesture",
  );
});

test("forceResetResizeState finalizes the abandoned gesture instead of discarding it", () => {
  const helper = appSource.match(
    /function\s+forceResetResizeState\([\s\S]*?\n\s{6}\}/,
  );
  assert.ok(helper, "expected forceResetResizeState to exist");
  assert.match(
    helper[0],
    /commitWindowGeometryGesture\(/,
    "expected forceResetResizeState to commit the latest DOM geometry (Issue #3364: a clear-only reset let the next workspace_state snap the window back)",
  );
});

test("renderWorkspace resolves incoming geometry (and patches the model) before render keys and the minimap read it", () => {
  const renderWorkspaceSource = appSource.match(
    /function\s+renderWorkspace\(workspace\)\s*\{[\s\S]*?workspaceWindowsRenderKey\(workspace\)/,
  );
  assert.ok(renderWorkspaceSource, "expected renderWorkspace to compute the windows render key");
  assert.match(
    renderWorkspaceSource[0],
    /resolveIncomingGeometry\(\s*geometrySyncState\s*,/,
    "expected a pre-pass that resolves incoming geometry BEFORE workspaceWindowsRenderKey",
  );
  assert.match(
    renderWorkspaceSource[0],
    /\.geometry\s*=\s*\{\s*\.\.\./,
    "expected the pre-pass to patch suppressed windows' model geometry to the local truth",
  );
});

test("sendGeometry omits base_geometry_revision for explicit gesture commits", () => {
  const sendGeometrySource = appSource.match(
    /function\s+sendGeometry\([\s\S]*?\n\s{6}\}/,
  );
  assert.ok(sendGeometrySource, "expected sendGeometry to exist");
  assert.match(
    sendGeometrySource[0],
    /base_geometry_revision\s*:[\s\S]{0,120}?===\s*null\s*\?\s*undefined/,
    "expected sendGeometry to translate a null base into an omitted field (unconditional apply)",
  );
});

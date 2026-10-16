import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { createUiStateStore } from "../ui-state-store.js";

const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");

test("all window navigation uses the rail layout without a canvas fallback", () => {
  assert.doesNotMatch(app, /getElementById\("canvas(?:-stage|-world-grid)?"\)/);
  assert.doesNotMatch(app, /createFleetMinimap|createCanvasWheelGestureClassifier|zoomCanvasAt/);
  assert.doesNotMatch(app, /kind: "(?:update_viewport|dock_window_tab|detach_window_tab)"/);
  assert.doesNotMatch(app, /data-action="ungroup"|class="resize-handle"/);
  assert.match(app, /splitSurfaces\.showWindow\(windowId\)/);
});


test("pending window navigation wins over initial catalog restoration", () => {
  const start = app.indexOf("      let initialSurfaceTabId = null;");
  const end = app.indexOf("      // SPEC-2356 — keyboard equivalent", start);
  assert.ok(start >= 0 && end > start, "expected the initial catalog subscription");
  const model = createUiStateStore({
    catalog: { appState: { active_tab_id: null, tabs: [] }, focusedId: null, version: 0 },
    pendingSurface: null,
  });
  const routed = [];
  model.subscribe(state => state.navigation, navigation => {
    if (navigation) routed.push(navigation.windowId);
  });
  const frame = windowId => model.update(state => ({ ...state,
    navigation: { windowId, serial: (state.navigation?.serial || 0) + 1 },
  }));
  const activeWindows = catalog => catalog.appState.tabs
    .find(tab => tab.id === catalog.appState.active_tab_id)?.workspace.windows || [];
  new Function("windowCatalogModel", "catalogActiveWindows", "windowMap", "requestWindowFrame", "frameWindow",
    app.slice(start, end))(model, activeWindows,
    new Map([["requested-log", {}], ["restored-pm", {}]]), frame, frame);

  model.update(state => ({ ...state,
    pendingSurface: { tabId: "A", preset: "logs", surfaceOwner: null },
    catalog: { appState: { active_tab_id: "A", tabs: [{ id: "A", workspace: { windows: [
      { id: "requested-log", preset: "logs", z_index: 1 },
      { id: "restored-pm", preset: "claude", is_pm: true, z_index: 10 },
    ] } }] }, focusedId: null, version: 1 },
  }));
  assert.deepEqual(routed, ["requested-log"], "the requested target must survive synchronous restore notifications");
  model.update(state => ({ ...state, catalog: { ...state.catalog, version: 2 } }));
  assert.deepEqual(routed, ["requested-log"], "later metadata must not replay initial restoration");
});

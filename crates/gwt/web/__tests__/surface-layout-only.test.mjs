import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { createUiStateStore } from "../ui-state-store.js";

const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const components = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");

test("the permanent surface host leaves project picker and onboarding visible", () => {
  assert.doesNotMatch(components, /\.surface-area\.has-surface-layout\s*>\s*:not\(\.split-surfaces\)/);
});

test("all window navigation uses the rail layout without a canvas fallback", () => {
  assert.doesNotMatch(app, /getElementById\("canvas(?:-stage|-world-grid)?"\)/);
  assert.doesNotMatch(app, /createFleetMinimap|createCanvasWheelGestureClassifier|zoomCanvasAt/);
  assert.doesNotMatch(app, /kind: "(?:update_viewport|dock_window_tab|detach_window_tab)"/);
  assert.doesNotMatch(app, /data-action="ungroup"|class="resize-handle"/);
  assert.match(app, /splitSurfaces\.showWindow\(windowId\)/);
});

test("surface navigation and launches carry no world coordinates or canvas visibility rules", () => {
  const modules = readdirSync(new URL("..", import.meta.url))
    .filter(name => name.endsWith(".js"));
  for (const name of modules) {
    const source = readFileSync(new URL(`../${name}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /\b(?:isOffCanvasPlacement|isCanvasPlacement|visibleBounds|getResumeBounds)\b/,
      `${name} must use surface ownership and containment without world bounds`);
  }
  assert.doesNotMatch(app, /viewport:\s*\{\s*x:/);
  assert.doesNotMatch(app, /visibleWindowData|raiseWindowElementLocally|numericZIndex|style\.zIndex/);
});


test("initial navigation honors pending choices and containment owners", () => {
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
    new Map([["requested-log", {}], ["restored-pm", {}], ["issues-2", {}], ["preview-2", {}]]), frame, frame);

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
  model.update(state => ({ ...state, catalog: {
    appState: { active_tab_id: "B", tabs: [{ id: "B", workspace: { windows: [
      { id: "issues-2", preset: "settings", z_index: 10 },
      { id: "preview-2", preset: "agent", z_index: 200,
        placement: { kind: "issue_preview", issue_window_id: "issues-2", issue_number: 4777 } },
    ] } }] }, focusedId: null, version: 3,
  } }));
  assert.deepEqual(routed, ["requested-log", "issues-2"],
    "restoring an inline agent must select its owning surface without opening an Agent tab");
});

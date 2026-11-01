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
  const flags = app.match(/      let initialSurfaceTabId = null;\n      let initialSurfaceSelected = false;\n/)?.[0];
  const start = app.indexOf("      windowCatalogModel.subscribe(model => model.catalog, catalog => {");
  const end = app.indexOf("      // SPEC-2356 — keyboard equivalent", start);
  const openStart = app.indexOf("      function openSurface(surface) {");
  const openEnd = app.indexOf("      // Every window entry point", openStart);
  const renderStart = app.indexOf("      function mergeProjectCatalog(state) {");
  const renderEnd = app.indexOf("      let initialSurfaceTabId = null;", renderStart);
  assert.ok(flags && start >= 0 && end > start && openStart >= 0 && openEnd > openStart
    && renderStart >= 0 && renderEnd > renderStart,
    "expected surface selection, real workspace rendering and the initial catalog subscription");
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
  let selected = null;
  let closes = 0;
  const { openSurface, renderAppState, receiveHubState } = new Function("environment", `
    const { windowCatalogModel, catalogActiveWindows, windowMap, requestWindowFrame, frameWindow,
      splitSurfaces } = environment;
    let appState = { active_tab_id: null, tabs: [] };
    let hubCatalog = null;
    const routeProjectKey = "project-A";
    let pendingFrameWindowId = null, pendingPmFrame = false, reconcilingWindowCatalog = false;
    const dismissOperatorBriefing = () => {}, connectSocket = () => {}, applyStatus = () => {};
    const traceMeasure = (_event, _detail, render) => render();
    const UI_TRACE_EVENT = {}, versionState = {}, detailMap = new Map();
    const setVersionState = () => {}, renderProjectPicker = () => {}, renderProjectOnboarding = () => {};
    const document = { getElementById: () => ({}) };
    const activeProjectTab = () => appState.tabs.find(tab => tab.id === appState.active_tab_id);
    const emptyWorkspace = () => ({ windows: [] }), renderWorkspace = () => {};
    const deriveCurrentProjectWorkspaceIds = () => [], syncCurrentProjectWorkspaceIds = () => {};
    const publishWindowCatalog = () => windowCatalogModel.update(state => ({ ...state, catalog: {
      appState, focusedId: null, version: state.catalog.version + 1,
    } }));
    ${flags}${app.slice(openStart, openEnd)}${app.slice(renderStart, renderEnd)}${app.slice(start, end)}
    return { openSurface, renderAppState, receiveHubState };
  `)({ windowCatalogModel: model, catalogActiveWindows: activeWindows,
    windowMap: new Map([["restored-board", {}], ["requested-log", {}], ["restored-pm", {}], ["issues-2", {}], ["preview-2", {}]]),
    requestWindowFrame: frame, frameWindow: frame,
    splitSurfaces: {
      show: surface => { selected = surface; return true; },
      close: () => { closes += 1; selected = null; },
    } });
  const firstProject = { id: "A", project_key: "project-A", workspace: { windows: [
    { id: "restored-board", preset: "board", z_index: 10 },
  ] } };

  openSurface("agents");
  renderAppState({ active_tab_id: "A", tabs: [firstProject] });
  assert.equal(selected, "agents", "the real first workspace render must retain the explicit rail choice");
  assert.equal(closes, 0, "first hydration must not close the already selected surface");
  assert.deepEqual(routed, [], "the explicit rail choice must survive the first hydrated project");
  renderAppState({ active_tab_id: "A", tabs: [firstProject], app_version: "metadata" });
  assert.equal(selected, "agents");
  assert.deepEqual(routed, [], "later metadata must preserve the explicit first-project rail choice");

  renderAppState({ active_tab_id: null, tabs: [] });
  assert.equal(closes, 1, "leaving a hydrated project closes its surface");
  renderAppState({ active_tab_id: "A", tabs: [firstProject] });
  assert.equal(closes, 1, "hydration without an explicit choice needs no close");
  assert.deepEqual(routed, ["restored-board"], "without a rail choice, first hydration restores its saved surface");

  model.update(state => ({ ...state, pendingSurface: { tabId: "A", preset: "logs", surfaceOwner: null } }));
  const requestedProject = { ...firstProject, workspace: { windows: [
    { id: "requested-log", preset: "logs", z_index: 1 },
    { id: "restored-pm", preset: "claude", is_pm: true, z_index: 10 },
  ] } };
  renderAppState({ active_tab_id: "A", tabs: [requestedProject] });
  assert.deepEqual(routed, ["restored-board", "requested-log"], "the requested target must survive synchronous restore notifications");
  renderAppState({ active_tab_id: "A", tabs: [requestedProject], app_version: "later" });
  assert.deepEqual(routed, ["restored-board", "requested-log"], "later metadata must not replay initial restoration");
  const secondProject = { id: "B", project_key: "project-B", workspace: { windows: [
    { id: "issues-2", preset: "settings", z_index: 10 },
    { id: "preview-2", preset: "agent", z_index: 200,
      placement: { kind: "issue_preview", issue_window_id: "issues-2", issue_number: 4777 } },
  ] } };
  renderAppState({ active_tab_id: "B", tabs: [firstProject, secondProject] });
  assert.equal(closes, 2, "a real project switch must close the previous surface");
  assert.deepEqual(routed, ["restored-board", "requested-log", "issues-2"],
    "restoring an inline agent must select its owning surface without opening an Agent tab");
  receiveHubState({ projects: [firstProject], recent_projects: [] });
  assert.equal(closes, 3, "a project switch produced by the merged Hub catalog must also close the previous surface");
  assert.deepEqual(routed, ["restored-board", "requested-log", "issues-2", "restored-board"]);
});

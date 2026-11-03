import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { projectUrlPath, routeWebSocketUrl } from "../frontend-route.js";
import { createPmWindowModel } from "../pm-chat.js";
import { createUiStateStore } from "../ui-state-store.js";
import { createAgentSwitchModel } from "../agent-activity-model.js";

const source = readFileSync(new URL("../app.js", import.meta.url), "utf8");
function functionSource(name) {
  const start = source.indexOf(`      function ${name}(`);
  assert.notEqual(start, -1);
  const end = source.indexOf("\n      }", start) + 8;
  return source.slice(start, end);
}
function fixture({ routeProjectKey = null } = {}) {
  const sockets = [];
  const opened = [];
  class Socket {
    static OPEN = 1;
    constructor(url) { this.url = url; this.readyState = 0; this.handlers = {}; this.sent = []; sockets.push(this); }
    addEventListener(kind, handler) { this.handlers[kind] = handler; }
    send(raw) { this.sent.push(JSON.parse(raw)); }
    close() { this.readyState = 3; this.handlers.close?.(); }
    open() { this.readyState = 1; this.handlers.open(); }
  }
  const context = vm.createContext({
    URL, WebSocket: Socket, socket: null, socketProjectKey: null, hubSocket: null, hubReconnectTimer: null, pendingHubMessages: [],
    pendingMessages: [], reconnectTimer: null, socketReceiveDispatcherGeneration: 0, socketInputHandoff: null,
    startupAutoResumeReadySent: false, startupFrames: [], startupTimers: [],
    requestAnimationFrame(callback) { context.startupFrames.push(callback); return context.startupFrames.length; },
    cancelAnimationFrame() {},
    setTimeout(callback) { context.startupTimers.push(callback); return context.startupTimers.length; },
    reconnectCallbacks: [], connectionStates: [],
    socketReceiveDispatcher: null, recoveryCenterController: null,
    closeResetCount: 0, attentionReportCount: 0,
    agentCompletionNotifier: { reset() {} },
    closeProjectController: { connectionLost() { context.closeResetCount += 1; } },
    projectPageMetadata: { resetConnection() {}, reportAttention() { context.attentionReportCount += 1; } },
    requestVisiblePmReports() {}, requestSupportedAgentRefresh() {}, pmChatViews: new Map(),
    activeWorkspace: () => ({ windows: [{ id: "pane-a" }] }),
    resyncCalls: [],
    resyncBoardLogs: (ids) => context.resyncCalls.push(["board-logs", ...ids]),
    resyncBranches: (ids) => context.resyncCalls.push(["branches", ...ids]),
    resyncKnowledge: (ids) => context.resyncCalls.push(["knowledge", ...ids]),
    resyncProfile: (ids) => context.resyncCalls.push(["profile", ...ids]),
    resyncProjectIndexSearch: (ids) => context.resyncCalls.push(["index", ...ids]),
    resyncSettings: () => context.resyncCalls.push(["settings"]),
    applyPmWindowReceiveEvent: createPmWindowModel().applyPmWindowReceiveEvent,
    appState: { tabs: [], active_tab_id: null }, hubCatalog: null,
    routeProjectKey, routeProjectMissing: false, routeWebSocketUrl, projectUrlPath,
    createUiStateStore,
    opened, renderRouteNotFound() {
      assert.equal(context.routeProjectMissing, true);
      assert.ok(sockets.every(socket => socket.readyState === 3));
      assert.equal(context.socketInputHandoff, null);
      assert.equal(context.overlayDisposed, true);
      context.notFoundRendered = (context.notFoundRendered || 0) + 1;
    },
    renderAppState(state) { context.appState = context.mergeProjectCatalog(state); context.connectSocket(); },
    window: {
      location: { href: "http://localhost:1234/?token=discard#fragment" },
      setTimeout: (callback) => { context.reconnectCallbacks.push(callback); return context.reconnectCallbacks.length; },
      open: (...args) => { opened.push(args); return null; },
    },
    document: {},
    droppedInputs: 0, connectionOverlay: {
      reportInputDropped() { context.droppedInputs += 1; },
      dispose() { context.overlayDisposed = true; },
    },
    clearTimeout() {}, clearPickerPending() {},
    setConnectionState(connected) { context.connectionStates.push(connected); }, syncRunningBranchCleanups() {},
    dispatcherOptions: [],
    disposedDispatchers: 0,
    createSocketReceiveDispatcher: (options) => {
      context.dispatcherOptions.push(options);
      return {
        handle(event) { options.receive(JSON.parse(event.data)); },
        dispose() { context.disposedDispatchers++; },
      };
    },
    received: [], receive(event) { context.received.push(event); },
    uiTraceWiring: { isTracing: () => false }, traceUi() {},
  });
  for (const name of ["activeProjectTab", "websocketUrl", "send", "handleSocketOpen", "handleSocketMessage", "handleSocketClose", "installSocketEventHandlers", "connectSocket", "connectHubSocket", "isHubNavigationMessage", "isHubNavigationResult", "sendStartupAutoResumeReady"]) {
    vm.runInContext(functionSource(name), context);
  }
  for (const name of ["mergeProjectCatalog", "receiveHubState", "showProjectRouteNotFound"]) {
    if (source.includes(`function ${name}(`)) vm.runInContext(functionSource(name), context);
  }
  vm.runInContext(source.slice(source.indexOf('      const projectRouteNotFoundModel ='),
    source.indexOf('      function showProjectRouteNotFound(')), context);
  // Allow the pre-change implementation to reach behavioral assertions.
  if (source.includes("function activeProjectKey(")) vm.runInContext(functionSource("activeProjectKey"), context);
  if (source.includes("function requestSocketSnapshots(")) vm.runInContext(functionSource("requestSocketSnapshots"), context);
  if (source.includes("function resyncSocketSurfaces(")) vm.runInContext(functionSource("resyncSocketSurfaces"), context);
  if (source.includes("function closeSocketInputHandoff(")) vm.runInContext(functionSource("closeSocketInputHandoff"), context);
  if (source.includes("function sendOnSocket(")) vm.runInContext(functionSource("sendOnSocket"), context);
  const select = (key) => { context.appState = { active_tab_id: key, tabs: [{ id: key, project_key: key }] }; context.connectSocket(); };
  return { context, sockets, select, opened };
}

test("Hub bootstrap retains its catalog socket while selecting a project", () => {
  const { context, sockets, select } = fixture();
  context.connectSocket();
  assert.equal(new URL(sockets[0].url).search, "");
  sockets[0].open();
  select("0123456789abcdef");
  assert.equal(sockets.length, 2);
  assert.equal(sockets[0].readyState, 1, "Hub catalog must remain connected");
  assert.equal(new URL(sockets[1].url).search, "?repo_hash=0123456789abcdef");
  select("0123456789abcdef");
  assert.equal(sockets.length, 2, "same scope must not reconnect in a loop");
  sockets[0].handlers.close();
  assert.equal(context.reconnectTimer, null, "stale close must not reset the new connection");
});

test("offline input is discarded while queued control messages retain project scope", () => {
  const { context, sockets, select } = fixture();
  context.connectSocket();
  sockets[0].open();
  select("0123456789abcdef");
  context.send({ kind: "terminal_input", id: "pane-a", data: "a" });
  context.send({ kind: "pane_send_input", session_id: "pane-a", text: "a" });
  context.send({ kind: "update_terminal_grid", id: "pane-a", cols: 80, rows: 24 });
  select("fedcba9876543210");
  context.send({ kind: "terminal_input", id: "pane-b", data: "b" });
  context.send({ kind: "update_terminal_grid", id: "pane-b", cols: 100, rows: 28 });
  assert.equal(context.droppedInputs, 3, "every discarded input is surfaced immediately");
  assert.equal(sockets.length, 3);
  sockets[2].open();
  assert.deepEqual(sockets[2].sent.filter((m) => m.kind !== "frontend_ready"), [{ kind: "update_terminal_grid", id: "pane-b", cols: 100, rows: 28 }, { kind: "get_system_settings" }]);
  select("0123456789abcdef");
  sockets[3].open();
  assert.deepEqual(sockets[3].sent.filter((m) => m.kind !== "frontend_ready"), [{ kind: "update_terminal_grid", id: "pane-a", cols: 80, rows: 24 }, { kind: "get_system_settings" }]);
});

test("Hub socket refuses pane input before project scope is known", () => {
  const { context, sockets } = fixture();
  context.connectSocket();
  sockets[0].open();
  assert.equal(context.send({ kind: "terminal_input", id: "pane", data: "x" }), "unavailable");
  assert.equal(context.send({ kind: "pane_send_input", session_id: "pane", text: "x" }), "unavailable");
  assert.equal(sockets[0].sent.filter((m) => m.kind !== "frontend_ready").length, 0);
});

const projectA = { id: "tab-a", project_key: "0123456789abcdef", title: "A", kind: "git" };
const projectB = { id: "tab-b", project_key: "fedcba9876543210", title: "B", kind: "git" };




test("retained Hub processes catalog and navigation without duplicating global events", () => {
  const { context, sockets, select } = fixture();
  context.connectSocket(); sockets[0].open();
  select(projectA.project_key); sockets[1].open();
  const catalog = { kind: "hub_state", state: { projects: [projectA, projectB] } };
  sockets[0].handlers.message({ data: JSON.stringify(catalog) });
  sockets[0].handlers.message({ data: JSON.stringify({ kind: "update_state" }) });
  sockets[1].handlers.message({ data: JSON.stringify({ kind: "update_state" }) });
  assert.deepEqual(context.received.map((event) => event.kind), ["hub_state", "update_state"]);
  context.send({ kind: "reopen_recent_project", path: "/project-b" });
  assert.equal(sockets[0].sent.at(-1).path, "/project-b");
  assert.equal(sockets[1].sent.length, 2);
  select(projectB.project_key);
  assert.equal(sockets[0].readyState, 1);
  assert.equal(sockets[1].readyState, 3);
  sockets[0].close();
  assert.equal(context.send({ kind: "reopen_recent_project", path: "/new" }), "queued");
  context.connectSocket(); sockets.at(-1).open();
  assert.equal(sockets.at(-1).sent.at(-1).path, "/new");
  assert.equal(new URL(sockets.at(-1).url).search, "");
  const before = context.received.length;
  sockets[0].handlers.message({ data: JSON.stringify(catalog) });
  assert.equal(context.received.length, before, "replaced Hub cannot deliver stale events");
  const count = sockets.length;
  select(null);
  assert.equal(sockets.length, count, "returning to Hub reuses its open connection");
  assert.equal(sockets[2].readyState, 3, "returning to Hub closes the project connection");
  sockets.at(-1).handlers.message({ data: JSON.stringify({ kind: "update_state" }) });
  assert.equal(context.received.length, before + 1, "Hub handles global events when it is the active channel");
});

// Issue #4538 AC-2: a `/p/<repo-hash>` tab derives its Project scope from
// the URL, keeps it across catalog changes and reconnects, and opens other
// Projects in their own browser tabs.
test("route-bound Project tab connects its scope from the URL before any catalog", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectB.project_key });
  context.connectSocket();
  assert.equal(sockets.length, 2, "Hub catalog plus the route's Project socket");
  assert.equal(new URL(sockets[0].url).search, "");
  assert.equal(new URL(sockets[1].url).searchParams.get("repo_hash"), projectB.project_key);
  sockets[1].open();
  assert.deepEqual(sockets[1].sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }], "full sync is requested on the scoped socket");
  context.receiveHubState({ app_version: "1", projects: [projectA, projectB], recent_projects: [] });
  assert.equal(context.activeProjectTab().id, projectB.id, "the catalog entry for the route becomes active");
  assert.equal(sockets.length, 2, "catalog updates never rebind the scope");
});

test("route-bound Project tab reconnects to the same Project only", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  sockets[1].open();
  sockets[1].close();
  assert.equal(context.closeResetCount, 1, "disconnected Project discards its close preview authority");
  context.connectSocket();
  assert.equal(context.attentionReportCount, 1, "fresh attention is reported for each opened Project connection");
  assert.equal(new URL(sockets.at(-1).url).searchParams.get("repo_hash"), projectA.project_key);
  sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }], "reconnect re-requests only this client's full sync");
  assert.equal(context.attentionReportCount, 2);
});

test("receive recovery requests pane and open surface snapshots only on the owning socket", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  sockets[1].open();
  const old = context.dispatcherOptions.at(-1);
  let recoveryLoads = 0;
  context.recoveryCenterController = { reconnect() { recoveryLoads++; } };
  assert.equal(typeof old.onResync, "function", "overflow must have a snapshot recovery callback");
  old.onResync("overflow");
  assert.equal(sockets.length, 3, "overflow also fences the native receive backlog");
  assert.deepEqual(sockets[1].sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }]);
  sockets.at(-1).open();
  assert.equal(sockets[1].readyState, 3, "the input handoff ends when recovery opens");
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }]);
  assert.deepEqual(context.resyncCalls, [
    ["board-logs", "pane-a"], ["branches", "pane-a"], ["knowledge", "pane-a"],
    ["profile", "pane-a"],
    ["index", "pane-a"], ["settings"],
  ]);
  assert.equal(recoveryLoads, 1, "a lost Recovery Center read is reissued");
  sockets.at(-1).close();
  context.connectSocket();
  sockets.at(-1).open();
  old.onResync("visible");
  assert.equal(sockets.at(-1).sent.length, 2, "a stale dispatcher cannot resync a replacement socket");
});

test("native foreground recovery does not require an observed hidden event", () => {
  const document = { hidden: false };
  let resumes = 0;
  let refreshes = 0;
  const reconnects = [];
  const reports = [];
  const refresh = () => { refreshes++; };
  const handlerSource = source.split('document.addEventListener("visibilitychange", ')[1].split("\n      });")[0] + "\n}";
  const context = {
    document,
    socketReceiveDispatcher: { resume() { resumes++; } },
    socketProjectKey: projectA.project_key,
    connectSocket: force => reconnects.push(force),
    requestVisiblePmReports: force => reports.push(force),
    rearmVisibleTerminalViewportRefreshes: refresh,
    resumeKnowledgeEventRefresh: refresh, refreshVisibleKnowledgeWindows: refresh,
  };
  const handler = vm.runInNewContext(`(${handlerSource})`, context);
  handler({ isTrusted: true });
  assert.equal(resumes, 1, "native restore can arrive without a preceding hidden event");
  handler({ isTrusted: false });
  assert.equal(resumes, 1, "pane visibility notifications must not reset socket recovery");
  assert.equal(refreshes, 6, "both visible notifications retain the existing display refreshes");
  context.socketReceiveDispatcher = null;
  handler({ isTrusted: true });
  assert.deepEqual(reconnects, [true], "native restore also fences a replacement that has not opened yet");
  handler({ isTrusted: false });
  assert.deepEqual(reconnects, [true], "synthetic visibility cannot replace the pending connection");
  document.hidden = true;
  handler({ isTrusted: true });
  assert.equal(resumes, 1, "backgrounding never requests snapshots");
  assert.equal(refreshes, 12);
  assert.deepEqual(reports, [true, true, true, true], "visible restores force Reports; hidden notification does not");
  assert.deepEqual(reconnects, [true], "backgrounding never replaces the pending connection");
});

test("window reveal forces only its visible Reports snapshot and preserves terminal activation", () => {
  const requests = [], sent = [], activated = [];
  const pm = { id: "pm", is_pm: true };
  const context = vm.createContext({
    document: { hidden: false }, socket: { readyState: 1 }, WebSocket: { OPEN: 1 },
    socketProjectKey: "project", activeProjectKey: () => "project",
    activeWorkspace: () => ({ windows: [pm, { id: "other", is_pm: true }] }),
    pmChatViews: new Map([["pm", { logHost: { hidden: true } }], ["other", { logHost: { hidden: true } }]]),
    visibleWindowData: () => true,
    requestPmReports: (id, force) => { requests.push([id, force]); return { kind: "load_pm_reports", id }; },
    send: request => sent.push(request.id),
    runTerminalRevealActivation: () => { activated.push("terminal"); return {}; },
    traceUi() {}, UI_TRACE_EVENT: {},
  });
  vm.runInContext(functionSource("requestVisiblePmReports") + "\n" + functionSource("activateTerminalOnReveal"), context);
  context.activateTerminalOnReveal("pm");
  assert.deepEqual(requests, [["pm", true]]);
  assert.deepEqual(sent, ["pm"]);
  assert.deepEqual(activated, ["terminal"]);
  context.pmChatViews.get("pm").logHost.hidden = false;
  context.activateTerminalOnReveal("pm");
  assert.deepEqual(sent, ["pm"], "Execution log keeps its existing hydration path");
});

test("foreground recovery replaces the native backlog without replaying mutations", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  const oldSocket = sockets[1];
  oldSocket.open();
  const oldDispatcher = context.dispatcherOptions.at(-1);
  context.send({ kind: "post_board_entry", id: "pane-a", body: "draft" });
  const sentBeforeResume = oldSocket.sent.length;
  oldDispatcher.onResync("visible");
  assert.equal(sockets.length, 3, "fresh snapshots need a transport without native buffered frames");
  assert.equal(oldSocket.readyState, 1, "keep the live input path until replacement OPEN");
  assert.equal(oldSocket.sent.length, sentBeforeResume, "do not append snapshots behind the native backlog");
  assert.equal(context.disposedDispatchers, 1);
  assert.equal(context.closeResetCount, 1, "the lost connection invalidates close preview authority");
  assert.equal(sockets[0].readyState, 0, "the retained catalog connection is untouched");
  const failedReplacement = sockets.at(-1);
  assert.equal(new URL(failedReplacement.url).searchParams.get("repo_hash"), projectA.project_key);
  const input = { kind: "terminal_input", id: "pane-a", data: "x" };
  const paneInput = { kind: "pane_send_input", session_id: "pane-a", text: "y" };
  assert.equal(context.send(input), "sent");
  assert.equal(context.send(paneInput), "sent");
  assert.deepEqual(oldSocket.sent.slice(-2), [input, paneInput], "input is delivered live, never queued for another prompt");
  assert.equal(context.pendingMessages.length, 0);
  oldSocket.handlers.message({ data: JSON.stringify({ kind: "terminal_output", id: "pane-a" }) });
  oldSocket.handlers.close();
  assert.deepEqual(context.received, [], "native messages from the old socket stay fenced");
  assert.equal(context.reconnectTimer, null, "an old close cannot delay the replacement");
  failedReplacement.close();
  context.reconnectCallbacks.at(-1)();
  const replacement = sockets.at(-1);
  assert.notEqual(replacement, failedReplacement, "retry replaces a connection that failed before OPEN");
  assert.equal(context.send(input), "sent", "a failed replacement keeps the same live input channel during retry");
  replacement.open();
  assert.equal(oldSocket.readyState, 3, "release the old transport after the replacement opens");
  assert.equal(context.droppedInputs, 0);
  assert.deepEqual(replacement.sent, [{ kind: "frontend_ready" }], "one full sync and no mutation replay");
  assert.deepEqual(context.resyncCalls, [
    ["board-logs", "pane-a"], ["branches", "pane-a"], ["knowledge", "pane-a"],
    ["profile", "pane-a"], ["index", "pane-a"], ["settings"],
  ]);
  oldDispatcher.onResync("visible");
  assert.equal(sockets.length, 4, "a stale dispatcher cannot replace the new socket");
});

test("recovery input handoff closes on a project switch and never crosses scopes", () => {
  const { context, sockets, select } = fixture();
  select(projectA.project_key);
  const old = sockets.at(-1);
  old.open();
  context.dispatcherOptions.at(-1).onResync("visible");
  const pending = sockets.at(-1);
  select("fedcba9876543210");
  assert.equal(old.readyState, 3);
  assert.equal(pending.readyState, 3);
  const before = old.sent.length;
  assert.equal(context.send({ kind: "terminal_input", id: "pane-b", data: "x" }), "unavailable");
  assert.equal(old.sent.length, before, "the former project must never receive the new project's input");
  sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }]);
});


test("Reports history survives disconnect while the Project reconnects", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  const reportsView = { sessionId: "pm-session", controller: { update() {} } };
  context.pmChatViews.set("pm-window", reportsView);
  context.connectSocket();
  sockets[1].open();
  sockets[1].close();
  assert.equal(context.closeResetCount, 1, "disconnect must reach the Project lifecycle");
  assert.equal(context.reconnectTimer, 1, "disconnect must schedule reconnect");
  assert.equal(context.pmChatViews.get("pm-window"), reportsView, "durable Reports view is retained");
  context.connectSocket();
  assert.equal(new URL(sockets.at(-1).url).searchParams.get("repo_hash"), projectA.project_key);
  sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }]);
});

test("project not found stops every reconnect for the route", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  sockets[1].open();
  context.dispatcherOptions.at(-1).onResync("visible");
  assert.equal(sockets[1].readyState, 1, "exercise not-found cleanup during the live input handoff");
  const reconnectCount = context.reconnectCallbacks.length;
  const connectionStates = [...context.connectionStates];
  assert.equal(context.notFoundRendered, undefined, "the initial false state must not close the route");
  vm.runInContext('projectRouteNotFoundModel.update(() => true)', context);
  assert.equal(context.notFoundRendered, 1);
  assert.ok(sockets.every((socket) => socket.readyState === 3));
  assert.equal(context.reconnectCallbacks.length, reconnectCount, "terminal close must not reserve reconnects");
  assert.deepEqual(context.connectionStates, connectionStates, "terminal close must not start an outage overlay");
  assert.equal(context.reconnectTimer, null);
  assert.equal(context.hubReconnectTimer, null);
  vm.runInContext('projectRouteNotFoundModel.update(() => true)', context);
  assert.equal(context.notFoundRendered, 1, "the same missing state must not rebuild the view");
  const count = sockets.length;
  context.connectSocket();
  assert.equal(sockets.length, count);
});

test("a queued startup notification ignores a route that became missing", () => {
  for (const missing of [false, true]) {
    const queued = [], sent = [];
    const context = vm.createContext({
      startupAutoResumeReadySent: false, routeProjectMissing: false, socketReceiveDispatcherGeneration: 1,
      setTimeout() {}, clearTimeout() {}, cancelAnimationFrame() {},
      requestAnimationFrame(callback) { queued.push(callback); },
      send(message) { sent.push(message); },
    });
    vm.runInContext(functionSource('sendStartupAutoResumeReady'), context);
    context.sendStartupAutoResumeReady();
    context.routeProjectMissing = missing;
    queued[0]();
    assert.equal(sent.length, missing ? 0 : 1);
    if (!missing) assert.equal(sent[0].kind, 'startup_auto_resume_ready');
  }
});

test("canvas readiness is sent once on each connection after backend restarts", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket(); sockets[0].open(); sockets[1].open();
  for (let restart = 0; restart <= 3; restart++) {
    const connection = sockets.at(-1);
    const framesBefore = context.startupFrames.length;
    context.sendStartupAutoResumeReady();
    context.sendStartupAutoResumeReady();
    assert.equal(context.startupFrames.length, framesBefore + 1, "each connection needs a fresh readiness callback");
    context.startupFrames.at(-1)();
    assert.equal(connection.sent.filter(message => message.kind === "startup_auto_resume_ready").length, 1,
      `connection ${restart} must release its backend's startup gate exactly once`);
    if (restart < 3) {
      connection.close(); context.connectSocket(); sockets.at(-1).open();
    }
  }
});

test("canvas readiness survives suspended frames and discards old connection callbacks", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket(); sockets[0].open(); sockets[1].open();
  context.sendStartupAutoResumeReady();
  assert.equal(context.startupTimers.length, 1, "a background tab needs a timer fallback");
  const oldFrame = context.startupFrames.at(-1), oldTimer = context.startupTimers.at(-1);
  sockets[1].close(); context.connectSocket(); sockets.at(-1).open();
  context.sendStartupAutoResumeReady();
  const connection = sockets.at(-1);
  oldTimer(); oldFrame();
  assert.deepEqual(connection.sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }], "old callbacks cannot release a new backend");
  context.startupTimers.at(-1)(); // No animation frame runs in the background.
  context.startupFrames.at(-1)(); // Foregrounding later must not send twice.
  assert.equal(connection.sent.filter(message => message.kind === "startup_auto_resume_ready").length, 1);
});

test("a missing route ignores late receive work", () => {
  const context = vm.createContext({
    routeProjectMissing: true,
    closeProjectController: { receive() { assert.fail('a missing route must ignore received work'); } },
  });
  vm.runInContext(functionSource('receive'), context);
  context.receive({ kind: 'workspace_state', workspace: {} });
});


test("aggregate control events bypass held rendering frames and stale sockets stay fenced", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  const held = [];
  context.createSocketReceiveDispatcher = ({ receive }) => ({ handle(event) { held.push(() => receive(JSON.parse(event.data))); } });
  context.connectSocket();
  sockets[1].open();
  const old = sockets[1];
  const aggregate = { kind: "project_agent_aggregate", aggregate: { running_count: 2, unread: true, revision: 3 } };
  old.handlers.message({ data: JSON.stringify(aggregate) });
  assert.equal(context.received.at(-1)?.kind, "project_agent_aggregate");
  assert.equal(held.length, 0, "aggregate must never wait for animation frames");
  old.handlers.message({ data: JSON.stringify({ kind: "project_notification", notification: { transition_id: "one" } }) });
  assert.equal(context.received.at(-1)?.kind, "project_notification");
  assert.equal(held.length, 0, "desktop intent must not wait for animation frames in a hidden tab");
  old.handlers.message({ data: JSON.stringify({ kind: "workspace_state" }) });
  assert.equal(held.length, 1, "normal render scheduling must remain unchanged");
  old.close();
  context.connectSocket();
  sockets.at(-1).open();
  const before = context.received.length;
  old.handlers.message({ data: JSON.stringify(aggregate) });
  held[0]();
  assert.equal(context.received.length, before, "old control and queued render events remain fenced");
});

test("offline acknowledgement is ephemeral and never replays against a fresh connection", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  sockets[1].open();
  sockets[1].close();
  assert.equal(context.send({ kind: "project_aggregate_ack", revision: 99, visible: true, focused: true }), "unavailable");
  assert.equal(context.send({ kind: "project_client_attention", visible: true, focused: true, notification_permission: "granted" }), "unavailable");
  assert.equal(context.pendingMessages.length, 0, "stale attention must never replay after reconnect");
  context.connectSocket();
  sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }]);
});


test("switch mutations never queue or replay, and a closed-socket race retains an unknown operation", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  const switching = createAgentSwitchModel(projectA.project_key);
  Object.assign(context, switching);
  vm.runInContext(functionSource("requestAgentSwitch"), context);
  context.connectSocket(); sockets[1].open(); sockets[1].close();
  const action = { kind: "request", operation_id: "operation-one", member_id: "member", expected_roster_revision: 3, target_issue_number: 42 };
  context.requestAgentSwitch(action);
  assert.equal(context.pendingMessages.length, 0, "an offline switch must not execute on the next Host");
  assert.deepEqual(switching.model.read().pending, action);
  assert.equal(switching.model.read().disconnected, true);
  assert.equal(context.send({ kind: "agent_switch", action: { kind: "cancel", operation_id: action.operation_id } }), "unavailable");
  assert.equal(context.pendingMessages.length, 0);
  context.connectSocket(); sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }, { kind: "get_system_settings" }]);
  sockets.at(-1).send = () => { throw new Error("connection closed during send"); };
  assert.doesNotThrow(() => assert.equal(context.send({ kind: "agent_switch", action }), "unavailable"));
  assert.equal(context.pendingMessages.length, 0);
});


test("first project connection reads saved Works settings without a Settings host", () => {
  const { sockets, select } = fixture();
  select("0123456789abcdef"); sockets.at(-1).open();
  assert.equal(sockets.at(-1).sent.filter(message => message.kind === "get_system_settings").length, 1);
});

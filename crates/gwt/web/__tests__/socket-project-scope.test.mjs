import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { projectUrlPath, routeWebSocketUrl } from "../frontend-route.js";

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
    pendingMessages: [], reconnectTimer: null, socketReceiveDispatcherGeneration: 0,
    socketReceiveDispatcher: null, recoveryCenterController: null,
    closeResetCount: 0, notificationResetCount: 0,
    agentCompletionNotifier: { reset() { context.notificationResetCount += 1; } },
    closeProjectController: { connectionLost() { context.closeResetCount += 1; } },
    appState: { tabs: [], active_tab_id: null }, hubCatalog: null,
    routeProjectKey, routeProjectMissing: false, routeWebSocketUrl, projectUrlPath,
    opened, renderRouteNotFound() { context.notFoundRendered = true; },
    renderAppState(state) { context.appState = context.mergeProjectCatalog(state); context.connectSocket(); },
    window: {
      location: { href: "http://localhost:1234/?token=discard#fragment" },
      setTimeout: () => 1,
      open: (...args) => { opened.push(args); return null; },
    },
    document: {},
    clearTimeout() {}, setConnectionState() {}, syncRunningBranchCleanups() {},
    createSocketReceiveDispatcher: ({ receive }) => ({ handle(event) { receive(JSON.parse(event.data)); } }),
    received: [], receive(event) { context.received.push(event); },
    uiTraceWiring: { isTracing: () => false }, traceUi() {},
  });
  for (const name of ["activeProjectTab", "websocketUrl", "send", "handleSocketOpen", "handleSocketMessage", "handleSocketClose", "installSocketEventHandlers", "connectSocket", "connectHubSocket", "isHubNavigationMessage", "isHubNavigationResult"]) {
    vm.runInContext(functionSource(name), context);
  }
  for (const name of ["mergeProjectCatalog", "receiveHubState", "showProjectRouteNotFound"]) {
    if (source.includes(`function ${name}(`)) vm.runInContext(functionSource(name), context);
  }
  // Allow the pre-change implementation to reach behavioral assertions.
  if (source.includes("function activeProjectKey(")) vm.runInContext(functionSource("activeProjectKey"), context);
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

test("queued input waits for a matching socket and never crosses project switches", () => {
  const { context, sockets, select } = fixture();
  context.connectSocket();
  sockets[0].open();
  select("0123456789abcdef");
  context.send({ kind: "terminal_input", id: "pane-a", data: "a" });
  select("fedcba9876543210");
  context.send({ kind: "terminal_input", id: "pane-b", data: "b" });
  assert.equal(sockets.length, 3);
  sockets[2].open();
  assert.deepEqual(sockets[2].sent.filter((m) => m.kind === "terminal_input"), [{ kind: "terminal_input", id: "pane-b", data: "b" }]);
  select("0123456789abcdef");
  sockets[3].open();
  assert.deepEqual(sockets[3].sent.filter((m) => m.kind === "terminal_input"), [{ kind: "terminal_input", id: "pane-a", data: "a" }]);
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
  assert.equal(sockets[1].sent.length, 1);
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
  assert.deepEqual(sockets[1].sent, [{ kind: "frontend_ready" }], "full sync is requested on the scoped socket");
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
  assert.equal(context.notificationResetCount, 2, "reconnect discards observed Running duration");
  assert.equal(new URL(sockets.at(-1).url).searchParams.get("repo_hash"), projectA.project_key);
  sockets.at(-1).open();
  assert.deepEqual(sockets.at(-1).sent, [{ kind: "frontend_ready" }], "reconnect re-requests only this client's full sync");
});


test("project not found stops every reconnect for the route", () => {
  const { context, sockets } = fixture({ routeProjectKey: projectA.project_key });
  context.connectSocket();
  context.showProjectRouteNotFound();
  assert.equal(context.notFoundRendered, true);
  assert.ok(sockets.every((socket) => socket.readyState === 3));
  const count = sockets.length;
  context.connectSocket();
  assert.equal(sockets.length, count);
});

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const source = readFileSync(new URL("../app.js", import.meta.url), "utf8");
function functionSource(name) {
  const start = source.indexOf(`      function ${name}(`);
  assert.notEqual(start, -1);
  const end = source.indexOf("\n      }", start) + 8;
  return source.slice(start, end);
}
function fixture() {
  const sockets = [];
  class Socket {
    static OPEN = 1;
    constructor(url) { this.url = url; this.readyState = 0; this.handlers = {}; this.sent = []; sockets.push(this); }
    addEventListener(kind, handler) { this.handlers[kind] = handler; }
    send(raw) { this.sent.push(JSON.parse(raw)); }
    close() { this.readyState = 3; this.handlers.close?.(); }
    open() { this.readyState = 1; this.handlers.open(); }
  }
  const context = vm.createContext({
    URL, WebSocket: Socket, socket: null, socketProjectKey: null,
    pendingMessages: [], reconnectTimer: null, socketReceiveDispatcherGeneration: 0,
    socketReceiveDispatcher: null, recoveryCenterController: null,
    appState: { tabs: [], active_tab_id: null },
    window: { location: { href: "http://localhost:1234/?token=discard#fragment" }, setTimeout: () => 1 },
    clearTimeout() {}, setConnectionState() {}, syncRunningBranchCleanups() {},
    createSocketReceiveDispatcher: () => ({ handle() {} }),
    uiTraceWiring: { isTracing: () => false }, traceUi() {},
  });
  for (const name of ["activeProjectTab", "websocketUrl", "send", "handleSocketOpen", "handleSocketMessage", "handleSocketClose", "installSocketEventHandlers", "connectSocket"]) {
    vm.runInContext(functionSource(name), context);
  }
  // Allow the pre-change implementation to reach behavioral assertions.
  if (source.includes("function activeProjectKey(")) vm.runInContext(functionSource("activeProjectKey"), context);
  const select = (key) => { context.appState = { active_tab_id: key, tabs: [{ id: key, project_key: key }] }; context.connectSocket(); };
  return { context, sockets, select };
}

test("Hub bootstrap reconnects once for the active project and ignores the old socket", () => {
  const { context, sockets, select } = fixture();
  context.connectSocket();
  assert.equal(new URL(sockets[0].url).search, "");
  sockets[0].open();
  select("0123456789abcdef");
  assert.equal(sockets.length, 2);
  assert.equal(new URL(sockets[1].url).search, "?repo_hash=0123456789abcdef");
  select("0123456789abcdef");
  assert.equal(sockets.length, 2, "same scope must not reconnect in a loop");
  sockets[0].handlers.close();
  assert.equal(context.reconnectTimer, null, "stale close must not reset the new connection");
});

test("queued input waits for a matching socket and never crosses project switches", () => {
  const { context, sockets, select } = fixture();
  select("0123456789abcdef");
  context.send({ kind: "terminal_input", id: "pane-a", data: "a" });
  select("fedcba9876543210");
  context.send({ kind: "terminal_input", id: "pane-b", data: "b" });
  assert.equal(sockets.length, 2);
  sockets[1].open();
  assert.deepEqual(sockets[1].sent.filter((m) => m.kind === "terminal_input"), [{ kind: "terminal_input", id: "pane-b", data: "b" }]);
  select("0123456789abcdef");
  sockets[2].open();
  assert.deepEqual(sockets[2].sent.filter((m) => m.kind === "terminal_input"), [{ kind: "terminal_input", id: "pane-a", data: "a" }]);
});

test("Hub socket refuses pane input before project scope is known", () => {
  const { context, sockets } = fixture();
  context.connectSocket();
  sockets[0].open();
  assert.equal(context.send({ kind: "terminal_input", id: "pane", data: "x" }), "unavailable");
  assert.equal(context.send({ kind: "pane_send_input", session_id: "pane", text: "x" }), "unavailable");
  assert.equal(sockets[0].sent.filter((m) => m.kind !== "frontend_ready").length, 0);
});

// Issue #4538 AC-3: the Hub at `/` is a picker only — Open Folder, Clone,
// Recent, and open Projects — with path-free, new-tab Project links.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import { mountHubApp } from "../hub-app.js";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const projectA = { id: "tab-a", project_key: "0123456789abcdef", title: "Alpha", kind: "git" };
const projectB = { id: "tab-b", project_key: "fedcba9876543210", title: "Beta", kind: "git" };

function fixture() {
  const { document, window } = parseHTML(indexHtml);
  const sockets = [];
  class FakeSocket {
    static OPEN = 1;
    constructor(url) {
      this.url = url;
      this.readyState = 0;
      this.handlers = {};
      this.sent = [];
      sockets.push(this);
    }
    addEventListener(kind, handler) { this.handlers[kind] = handler; }
    send(raw) { this.sent.push(JSON.parse(raw)); }
    open() { this.readyState = 1; this.handlers.open(); }
    deliver(event) { this.handlers.message({ data: JSON.stringify(event) }); }
  }
  const timers = [];
  const win = Object.assign(window, {
    location: { href: "http://127.0.0.1:4545/", pathname: "/" },
    WebSocket: FakeSocket,
    setTimeout: (callback) => { timers.push(callback); return timers.length; },
    clearTimeout() {},
  });
  const app = mountHubApp({ window: win, document });
  return { app, document, sockets, timers };
}

function hubState(projects, recent = []) {
  return { kind: "hub_state", hub: { app_version: "9.9.9", projects, recent_projects: recent } };
}

test("Hub replaces the Project workspace with a picker only", () => {
  const { document, sockets } = fixture();
  assert.equal(document.title, "gwt — Hub");
  for (const selector of ["#app", "#project-tabs", ".project-tab", "#workspace", "#op-rail", "#project-switcher-button"]) {
    assert.equal(document.querySelector(selector), null, `${selector} must not exist on the Hub`);
  }
  assert.ok(document.querySelector('[data-hub-action="open-folder"]'));
  assert.ok(document.querySelector('[data-hub-action="clone"]'));
  assert.ok(document.querySelector('[data-hub-list="open"]'));
  assert.ok(document.querySelector('[data-hub-list="recent"]'));
  assert.equal(sockets.length, 1);
  assert.equal(new URL(sockets[0].url).search, "", "the Hub connects unscoped");
  sockets[0].open();
  assert.deepEqual(sockets[0].sent, [{ kind: "frontend_ready" }]);
});

test("open and recent Projects are path-free links that open a new tab", () => {
  const { document, sockets } = fixture();
  sockets[0].open();
  sockets[0].deliver(hubState([projectA], [
    { path: "/Users/me/work/beta", title: "Beta", kind: "git", project_key: projectB.project_key },
    { path: "/Users/me/work/gamma", title: "Gamma", kind: "git", project_key: null },
  ]));
  const links = [...document.querySelectorAll(".gwt-hub a")];
  assert.deepEqual(links.map((link) => link.getAttribute("href")), [
    "/p/0123456789abcdef",
    "/p/fedcba9876543210",
  ]);
  for (const link of links) {
    assert.equal(link.getAttribute("target"), "_blank");
    assert.equal(link.getAttribute("rel"), "noopener");
  }
  assert.doesNotMatch(document.body.innerHTML, /\/Users\/me/, "no filesystem path reaches the Hub DOM");
  const pending = document.querySelector('[data-hub-list="recent"] .is-pending');
  assert.equal(pending.textContent, "Gamma");
  assert.equal(pending.getAttribute("href"), null, "an unresolved Recent entry is not a link");
});

test("empty catalog states are explicit", () => {
  const { document, sockets } = fixture();
  sockets[0].open();
  sockets[0].deliver(hubState([], []));
  assert.equal(document.querySelector('[data-hub-list="open"]').textContent, "No open projects");
  assert.equal(document.querySelector('[data-hub-list="recent"]').textContent, "No recent projects");
});

test("Open Folder asks the runtime and then offers the new Project as a link", () => {
  const { document, sockets } = fixture();
  sockets[0].open();
  sockets[0].deliver(hubState([projectA]));
  document.querySelector('[data-hub-action="open-folder"]').click();
  assert.deepEqual(sockets[0].sent.at(-1), { kind: "open_project_dialog" });
  sockets[0].deliver(hubState([projectA, projectB]));
  const notice = document.querySelector(".gwt-hub__notice");
  assert.equal(notice.hidden, false);
  const link = notice.querySelector("a");
  assert.equal(link.getAttribute("href"), "/p/fedcba9876543210");
  assert.equal(link.getAttribute("target"), "_blank");
});

test("a failed open is reported without a link", () => {
  const { document, sockets } = fixture();
  sockets[0].open();
  document.querySelector('[data-hub-action="open-folder"]').click();
  sockets[0].deliver({ kind: "project_open_error", message: "not a git repository" });
  const alert = document.querySelector(".gwt-hub__error");
  assert.equal(alert.hidden, false);
  assert.equal(alert.getAttribute("role"), "alert");
  assert.equal(alert.textContent, "not a git repository");
  sockets[0].deliver(hubState([projectA]));
  assert.equal(document.querySelector(".gwt-hub__notice").hidden, true);
});

test("Clone uses the shared modal dialog and closes when the clone lands", () => {
  const { document, sockets } = fixture();
  sockets[0].open();
  document.querySelector('[data-hub-action="clone"]').click();
  const modal = document.getElementById("clone-project-modal");
  assert.ok(modal.classList.contains("modal-backdrop"));
  assert.ok(modal.classList.contains("open"));
  const dialog = modal.querySelector(".modal-shell");
  assert.equal(dialog.getAttribute("role"), "dialog");
  assert.equal(dialog.getAttribute("aria-modal"), "true");
  sockets[0].deliver({ kind: "clone_project_parent_selected", path: "/tmp/parent" });
  document.getElementById("clone-project-url-input").value = "https://github.com/o/r.git";
  document.getElementById("clone-project-url-input").dispatchEvent(new document.defaultView.Event("input"));
  document.getElementById("clone-project-start").click();
  assert.deepEqual(sockets[0].sent.at(-1), {
    kind: "clone_project_start",
    url: "https://github.com/o/r.git",
    parent_path: "/tmp/parent",
  });
  sockets[0].deliver({ kind: "clone_project_done" });
  assert.equal(modal.classList.contains("open"), false);
});

test("the Hub reconnects after its socket closes", () => {
  const { sockets, timers } = fixture();
  sockets[0].open();
  sockets[0].handlers.close();
  assert.equal(timers.length, 1);
  timers[0]();
  assert.equal(sockets.length, 2);
  assert.equal(new URL(sockets[1].url).search, "");
});

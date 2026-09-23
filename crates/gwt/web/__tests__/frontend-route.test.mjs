// Issue #4538 AC-1 / AC-2: route parsing, path-free URLs, route-derived
// WebSocket scope, and the bootstrap's single-application choice.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

import {
  parseFrontendRoute,
  projectUrlPath,
  renderRouteNotFound,
  routeWebSocketUrl,
} from "../frontend-route.js";
import { bootFrontendRoute } from "../frontend-bootstrap.js";

const indexHtml = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const notFoundHtml = readFileSync(new URL("../project-not-found.html", import.meta.url), "utf8");

test("root is the Hub and a canonical hash is exactly one Project", () => {
  assert.deepEqual(parseFrontendRoute("/"), { kind: "hub", projectKey: null });
  assert.deepEqual(parseFrontendRoute("/p/0123456789abcdef"), {
    kind: "project",
    projectKey: "0123456789abcdef",
  });
});

test("invalid or extra route segments are not found", () => {
  for (const pathname of [
    "/p/",
    "/p/zzz",
    "/p/0123456789ABCDEF",
    "/p/0123456789abcdef0",
    "/p/0123456789abcdef/",
    "/p/0123456789abcdef/extra",
    "/p/..%2F..%2Fetc",
    "/projects",
    "",
  ]) {
    assert.equal(parseFrontendRoute(pathname).kind, "not_found", pathname);
  }
});

test("project URLs are built only from canonical keys", () => {
  assert.equal(projectUrlPath("fedcba9876543210"), "/p/fedcba9876543210");
  assert.throws(() => projectUrlPath("/Users/me/repo"));
  assert.throws(() => projectUrlPath(""));
});

test("the WebSocket scope comes from the route, never from query or fragment", () => {
  const href = "http://127.0.0.1:4545/p/0123456789abcdef?token=x#frag";
  assert.equal(routeWebSocketUrl(href, "0123456789abcdef"), "ws://127.0.0.1:4545/ws?repo_hash=0123456789abcdef");
  assert.equal(routeWebSocketUrl("https://host/", null), "wss://host/ws");
});

test("not-found view is path-free and links to the Hub", () => {
  const { document } = parseHTML(indexHtml);
  renderRouteNotFound(document);
  assert.equal(document.title, "gwt — Project not found");
  assert.equal(document.getElementById("app"), null, "no workspace remains");
  assert.equal(document.querySelector("[data-route-not-found] a").getAttribute("href"), "/");
  for (const html of [document.body.innerHTML, notFoundHtml]) {
    assert.match(html, /Project not found/);
    assert.doesNotMatch(html, /\/Users\/|\/home\/|[A-Z]:\\/);
  }
});

test("index.html boots only the route bootstrap and the Project links the Hub in a new tab", () => {
  const scripts = [...indexHtml.matchAll(/<script type="module" src="([^"]+)"/g)].map((match) => match[1]);
  assert.deepEqual(scripts, ["/frontend-bootstrap.js"]);
  const { document } = parseHTML(indexHtml);
  const home = document.getElementById("project-home-link");
  assert.equal(home.getAttribute("href"), "/");
  assert.equal(home.getAttribute("target"), "_blank");
  assert.equal(home.getAttribute("rel"), "noopener");
});

function bootIn(pathname) {
  const { document, window } = parseHTML(indexHtml);
  const sockets = [];
  const win = Object.assign(window, {
    location: { href: `http://127.0.0.1:4545${pathname}`, pathname },
    WebSocket: class {
      static OPEN = 1;
      constructor(url) { this.url = url; sockets.push(this); }
      addEventListener() {}
      send() {}
    },
    setTimeout: () => 0,
    clearTimeout() {},
  });
  const route = bootFrontendRoute({ window: win, document });
  return { route, document, sockets };
}

test("bootstrap loads the Project app only for a Project route", () => {
  const { route, document, sockets } = bootIn("/p/0123456789abcdef");
  assert.equal(route.kind, "project");
  assert.equal(document.documentElement.dataset.route, "project");
  const scripts = [...document.head.querySelectorAll('script[type="module"]')].map((script) => script.getAttribute("src"));
  assert.deepEqual(scripts, ["/app.js"]);
  assert.ok(document.getElementById("app"), "the Project workspace shell stays");
  assert.equal(sockets.length, 0, "the Project app owns its own connections");
});

test("bootstrap mounts the Hub for root without the Project app", () => {
  const { route, document, sockets } = bootIn("/");
  assert.equal(route.kind, "hub");
  assert.equal(document.head.querySelector('script[src="/app.js"]'), null);
  assert.equal(document.getElementById("app"), null);
  assert.ok(document.querySelector("[data-hub]"));
  assert.equal(sockets.length, 1);
});

test("bootstrap renders not-found for any other route", () => {
  const { route, document, sockets } = bootIn("/p/not-a-key");
  assert.equal(route.kind, "not_found");
  assert.ok(document.querySelector("[data-route-not-found]"));
  assert.equal(document.head.querySelector('script[src="/app.js"]'), null);
  assert.equal(sockets.length, 0);
});

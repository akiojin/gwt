// Issue #4538 AC-1 / AC-2: one browser tab is one route. `/` is the Hub,
// `/p/<repo-hash>` is exactly one Project, and anything else is not found.
// The hash is the canonical 16-hex ProjectKey; a route never carries a path.

export const PROJECT_KEY_PATTERN = /^[0-9a-f]{16}$/;

export function parseFrontendRoute(pathname) {
  if (pathname === "/" || pathname === "/index.html") {
    return { kind: "hub", projectKey: null };
  }
  const match = /^\/p\/([^/]+)$/.exec(String(pathname || ""));
  if (match && PROJECT_KEY_PATTERN.test(match[1])) {
    return { kind: "project", projectKey: match[1] };
  }
  return { kind: "not_found", projectKey: null };
}

export function projectUrlPath(projectKey) {
  if (!PROJECT_KEY_PATTERN.test(String(projectKey || ""))) {
    throw new Error("invalid project key");
  }
  return `/p/${projectKey}`;
}

export const HUB_URL_PATH = "/";

// The WebSocket scope is derived from the route, never from page state:
// a Project tab always reconnects to the same Project.
export function routeWebSocketUrl(locationHref, projectKey) {
  const url = new URL(locationHref);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = "/ws";
  url.search = "";
  url.hash = "";
  if (projectKey) url.searchParams.set("repo_hash", projectKey);
  return url.toString();
}

// Replace the page with the path-free not-found view shared with the
// server's `/p/<invalid>` response.
export function renderRouteNotFound(doc) {
  doc.title = "gwt — Project not found";
  const main = doc.createElement("main");
  main.className = "route-not-found";
  main.dataset.routeNotFound = "";
  const title = doc.createElement("h1");
  title.className = "route-not-found__title";
  title.textContent = "Project not found";
  const copy = doc.createElement("p");
  copy.className = "route-not-found__copy";
  copy.textContent = "This project URL does not match an open or recent gwt project.";
  const home = doc.createElement("a");
  home.className = "wizard-button primary route-not-found__home";
  home.href = HUB_URL_PATH;
  home.textContent = "Go to Hub";
  main.append(title, copy, home);
  doc.body.className = "route-body";
  doc.body.replaceChildren(main);
  return main;
}

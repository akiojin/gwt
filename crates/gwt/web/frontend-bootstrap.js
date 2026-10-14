// Issue #4538: the single HTML entrypoint for `/` and `/p/<repo-hash>`.
// The route decides which application loads: the Hub picker never loads the
// Project workspace (no tabs, no canvas), and a Project tab loads only it.
import { parseFrontendRoute, renderRouteNotFound } from "./frontend-route.js";
import { mountHubApp } from "./hub-app.js";

// The Project app is added as a module script element rather than a dynamic
// import(): an inserted script delays the document `load` event until it has
// run, so "page loaded" keeps meaning "application wired" (dynamic imports do
// not delay `load`).
export const PROJECT_APP_MODULE = "/app.js";

export function bootFrontendRoute({ window: win, document: doc }) {
  const route = parseFrontendRoute(win.location.pathname);
  doc.documentElement.dataset.route = route.kind;
  if (route.kind === "project") {
    const script = doc.createElement("script");
    script.type = "module";
    script.src = PROJECT_APP_MODULE;
    doc.head.append(script);
  } else if (route.kind === "hub") {
    mountHubApp({ window: win, document: doc });
  } else {
    renderRouteNotFound(doc);
  }
  return route;
}

if (typeof window !== "undefined" && typeof document !== "undefined") {
  bootFrontendRoute({ window, document });
}

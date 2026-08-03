// Issue #3365 — degradation visibility for swallowed render/receive failures.
//
// The WebSocket dispatcher keeps the event loop alive when `receive()` throws,
// and the workspace render guard isolates per-window sync failures. Both used
// to be console-only, so the user saw a silently stale UI. The banner is the
// user-visible counterpart (same spirit as the connection overlay): a
// persistent, dismissible alert with a failure count and a Reload action.

import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";

import { createRenderDegradationBanner } from "../render-degradation-banner.js";

function mount() {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  return document;
}

function bannerIn(document) {
  return document.querySelector(".render-degradation-banner");
}

test("report shows a persistent alert banner", () => {
  const document = mount();
  const banner = createRenderDegradationBanner({ document });

  banner.report({ source: "receive:workspace_state", error: new Error("boom") });

  const el = bannerIn(document);
  assert.ok(el, "banner element must mount into document.body");
  assert.equal(el.getAttribute("role"), "alert");
  assert.match(el.textContent, /1 update failed to render/);
});

test("repeat reports keep one banner and bump the count", () => {
  const document = mount();
  const banner = createRenderDegradationBanner({ document });

  banner.report({ source: "render_workspace", error: new Error("first") });
  banner.report({ source: "render_workspace", error: new Error("second") });

  const banners = document.querySelectorAll(".render-degradation-banner");
  assert.equal(banners.length, 1, "reports must aggregate into a single banner");
  assert.match(banners[0].textContent, /2 updates failed to render/);
});

test("dismiss removes the banner and a new report re-shows it with the cumulative count", () => {
  const document = mount();
  const banner = createRenderDegradationBanner({ document });

  banner.report({ source: "render_workspace", error: new Error("first") });
  const dismiss = document.querySelector(".render-degradation-banner__dismiss");
  assert.ok(dismiss, "banner must expose a dismiss button");
  dismiss.dispatchEvent(new document.defaultView.Event("click"));
  assert.equal(bannerIn(document), null, "dismiss must remove the banner");

  banner.report({ source: "render_workspace", error: new Error("second") });
  const el = bannerIn(document);
  assert.ok(el, "a new failure after dismiss must re-show the banner");
  assert.match(el.textContent, /2 updates failed to render/);
});

test("reload button invokes the injected reload callback", () => {
  const document = mount();
  let reloads = 0;
  const banner = createRenderDegradationBanner({
    document,
    reload: () => {
      reloads += 1;
    },
  });

  banner.report({ source: "render_workspace", error: new Error("boom") });
  const reload = document.querySelector(".render-degradation-banner__reload");
  assert.ok(reload, "banner must expose a reload button");
  reload.dispatchEvent(new document.defaultView.Event("click"));
  assert.equal(reloads, 1);
});

test("report is a no-op without a usable document", () => {
  const banner = createRenderDegradationBanner({ document: null });
  assert.doesNotThrow(() =>
    banner.report({ source: "render_workspace", error: new Error("boom") }),
  );
  const bodyless = createRenderDegradationBanner({ document: {} });
  assert.doesNotThrow(() =>
    bodyless.report({ source: "render_workspace", error: new Error("boom") }),
  );
});

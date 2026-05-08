// SPEC-2041 Phase 14 — unified GUI update CTA.
//
// The update notification must be one actionable bottom-right CTA, not a
// transient toast plus a separate persistent button. These tests exercise the
// CTA controller with DOM-like elements so regressions are caught by behavior,
// not source-string proximity.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import { createUpdateCtaController } from "../update-cta.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");

test("update_state renders one reusable update CTA", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });
  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  const cta = fixture.document.getElementById("update-cta");
  const action = cta.querySelector("[data-update-cta-action]");
  const dismiss = cta.querySelector("[data-update-cta-dismiss]");
  assert.equal(action.textContent, "Update available: v9.23.0 - Click to update");
  assert.equal(action.title, "Update available: v9.23.0 - Click to update");
  assert.equal(action.getAttribute("aria-label"), "Update available: v9.23.0 - Click to update");
  assert.equal(dismiss.textContent, "\u00d7");
  assert.equal(dismiss.getAttribute("aria-label"), "Dismiss update notification");
  assert.equal(cta.title, "Update available: v9.23.0 - Click to update");
  assert.equal(cta.getAttribute("aria-label"), "Update available: v9.23.0 - Click to update");
  assert.equal(cta.dataset.status, "available");
  assert.equal(fixture.versionUpdates.length, 2);
});

test("update CTA click cancel leaves it available and does not send apply_update", () => {
  const fixture = createFixture({ confirmResult: false });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.querySelector("[data-update-cta-action]").click();

  assert.equal(fixture.confirmCalls.length, 1);
  assert.deepEqual(fixture.sent, []);
  assert.equal(fixture.document.getElementById("update-cta").dataset.status, "available");
});

test("update CTA click approve sends apply_update and shows applying state", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.querySelector("[data-update-cta-action]").click();

  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }]);
  const cta = fixture.document.getElementById("update-cta");
  const action = cta.querySelector("[data-update-cta-action]");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(action.disabled, true);
  assert.equal(action.textContent, "Applying update...");
  assert.equal(cta.querySelector("[data-update-cta-dismiss]"), null);
});

test("duplicate update_state does not reset an applying CTA", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });
  fixture.document.querySelector("[data-update-cta-action]").click();

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  const cta = fixture.document.getElementById("update-cta");
  const action = cta.querySelector("[data-update-cta-action]");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(action.disabled, true);
  assert.equal(action.textContent, "Applying update...");
});

test("update_apply_error reuses the same CTA and allows retry", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.querySelector("[data-update-cta-action]").click();

  controller.showError("Failed to start the update.");
  const cta = fixture.document.getElementById("update-cta");
  const action = cta.querySelector("[data-update-cta-action]");

  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  assert.equal(cta.dataset.status, "error");
  assert.equal(action.disabled, false);
  assert.match(action.textContent, /Update failed/);
  assert.match(action.textContent, /Failed to start the update/);
  assert.ok(cta.querySelector("[data-update-cta-dismiss]"));

  action.click();
  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }, { kind: "apply_update" }]);
});

test("update CTA dismiss hides available state without applying", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.querySelector("[data-update-cta-dismiss]").click();

  assert.equal(fixture.document.getElementById("update-cta"), null);
  assert.equal(fixture.confirmCalls.length, 0);
  assert.deepEqual(fixture.sent, []);
});

test("dismissed update CTA reappears on the next update_state", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.querySelector("[data-update-cta-dismiss]").click();

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "available");
  assert.equal(
    cta.querySelector("[data-update-cta-action]").textContent,
    "Update available: v9.23.0 - Click to update",
  );
});

test("update CTA dismiss hides error state without retrying", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.querySelector("[data-update-cta-action]").click();
  controller.showError("Failed to start the update.");

  fixture.document.querySelector("[data-update-cta-dismiss]").click();

  assert.equal(fixture.document.getElementById("update-cta"), null);
  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }]);
});

test("app.js delegates update handling to the unified update CTA controller", () => {
  assert.match(appSource, /createUpdateCtaController/);
  assert.match(appSource, /updateCtaController\.handleUpdateState\(event\)/);
  assert.match(appSource, /updateCtaController\.showError\(/);
});

test("legacy split update toast and button surfaces are removed", () => {
  assert.doesNotMatch(appSource, /showUpdateToast/);
  assert.doesNotMatch(appSource, /showUpdateButton/);
  assert.doesNotMatch(indexHtml, /\.update-toast\b/);
  assert.doesNotMatch(indexHtml, /\.update-button\b/);
});

test("index.html declares a fixed bottom-right unified update CTA style", () => {
  const styleMatch = componentsCss.match(/\.update-cta\s*\{[^}]+\}/);
  assert.ok(styleMatch, "expected .update-cta rule inside components.css");
  assert.match(styleMatch[0], /position:\s*fixed/);
  assert.match(styleMatch[0], /bottom:\s*\d+px/);
  assert.match(styleMatch[0], /right:\s*\d+px/);
  assert.match(componentsCss, /\.update-cta\.is-applying\s*\{/);
  assert.match(componentsCss, /\.update-cta\.is-error\s*\{/);
  assert.match(componentsCss, /\.update-cta__action\s*\{/);
  assert.match(componentsCss, /\.update-cta__dismiss\s*\{/);
  assert.doesNotMatch(indexHtml, /\.update-cta\s*\{/);
});

function createFixture({ confirmResult = true } = {}) {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  const sent = [];
  const confirmCalls = [];
  const versionUpdates = [];
  return {
    document,
    sent,
    confirmCalls,
    versionUpdates,
    options: {
      document,
      send(message) {
        sent.push(message);
      },
      confirmUpdate(version) {
        confirmCalls.push(version);
        return confirmResult;
      },
      setVersionState(current, latest) {
        versionUpdates.push({ current, latest });
      },
    },
  };
}

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
  const dismiss = fixture.document.querySelector("[data-update-cta-dismiss]");
  assert.equal(cta.tagName, "BUTTON");
  assert.equal(cta.textContent, "Update available: v9.23.0 - Click to update");
  assert.equal(cta.title, "Update available: v9.23.0 - Click to update");
  assert.equal(cta.getAttribute("aria-label"), "Update available: v9.23.0 - Click to update");
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

  fixture.document.getElementById("update-cta").click();

  assert.equal(fixture.confirmCalls.length, 1);
  assert.deepEqual(fixture.sent, []);
  assert.equal(fixture.document.getElementById("update-cta").dataset.status, "available");
});

test("update CTA click approve sends apply_update and shows applying state", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.getElementById("update-cta").click();

  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }]);
  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(cta.disabled, true);
  assert.equal(cta.textContent, "Applying update...");
  assert.equal(fixture.document.querySelector("[data-update-cta-dismiss]"), null);
});

test("duplicate update_state does not reset an applying CTA", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(cta.disabled, true);
  assert.equal(cta.textContent, "Applying update...");
});

test("update_apply_error reuses the same CTA and allows retry", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.getElementById("update-cta").click();

  controller.showError("Failed to start the update.");
  const cta = fixture.document.getElementById("update-cta");

  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  assert.equal(cta.dataset.status, "error");
  assert.equal(cta.disabled, false);
  assert.match(cta.textContent, /Update failed/);
  assert.match(cta.textContent, /Failed to start the update/);
  assert.ok(fixture.document.querySelector("[data-update-cta-dismiss]"));

  cta.click();
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
    cta.textContent,
    "Update available: v9.23.0 - Click to update",
  );
});

test("update CTA dismiss hides error state without retrying", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.getElementById("update-cta").click();
  controller.showError("Failed to start the update.");

  fixture.document.querySelector("[data-update-cta-dismiss]").click();

  assert.equal(fixture.document.getElementById("update-cta"), null);
  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }]);
});

test("update_state removes stale split toast and button DOM before showing CTA", () => {
  const fixture = createFixture();
  const staleToast = fixture.document.createElement("button");
  staleToast.className = "update-toast";
  staleToast.textContent = "stale toast";
  const staleButton = fixture.document.createElement("button");
  staleButton.className = "update-button";
  staleButton.textContent = "stale button";
  const staleCta = fixture.document.createElement("div");
  staleCta.id = "update-cta";
  staleCta.textContent = "stale wrapper";
  fixture.document.body.appendChild(staleToast);
  fixture.document.body.appendChild(staleButton);
  fixture.document.body.appendChild(staleCta);
  const controller = createUpdateCtaController(fixture.options);

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  assert.equal(Boolean(fixture.document.querySelector(".update-toast")), false);
  assert.equal(Boolean(fixture.document.querySelector(".update-button")), false);
  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  assert.equal(fixture.document.getElementById("update-cta").tagName, "BUTTON");
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
  const shellMatch = componentsCss.match(/\.update-cta-shell\s*\{[^}]+\}/);
  assert.ok(shellMatch, "expected .update-cta-shell rule inside components.css");
  assert.match(shellMatch[0], /position:\s*fixed/);
  assert.match(shellMatch[0], /bottom:\s*\d+px/);
  assert.match(shellMatch[0], /right:\s*\d+px/);
  const styleMatch = componentsCss.match(/\.update-cta\s*\{[^}]+\}/);
  assert.ok(styleMatch, "expected .update-cta rule inside components.css");
  assert.match(componentsCss, /\.update-cta\.is-applying\s*\{/);
  assert.match(componentsCss, /\.update-cta\.is-error\s*\{/);
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

// SPEC-2041 Phase 19 (FR-052..066) — Post-click modal & restart UX.
//
// These tests pin the modal-driven state machine before implementation.
// They will fail against the current `update-cta.js` (which uses
// window.confirm and immediate apply_update_state_and_exit). The implementation
// in T-122 must satisfy these invariants without regressing earlier tests.

test("phase19: click no longer uses window.confirm and instead opens #update-modal in downloading state", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");

  fixture.document.getElementById("update-cta").click();

  assert.equal(
    fixture.confirmCalls.length,
    0,
    "window.confirm must not be called; the modal replaces it",
  );
  const modal = fixture.document.getElementById("update-modal");
  assert.ok(modal, "expected #update-modal to be rendered after click");
  assert.equal(modal.dataset.state, "downloading");
  assert.deepEqual(fixture.sent, [{ kind: "apply_update_start" }]);
  assert.equal(
    fixture.document.getElementById("update-cta").dataset.status,
    "applying",
  );
});

test("phase19: update_progress events update modal progress bar and byte counter", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateProgress({
    downloaded: 1024 * 1024,
    total: 4 * 1024 * 1024,
    asset: "gwt-macos-arm64.tar.gz",
    version: "9.26.0",
  });
  controller.handleUpdateProgress({
    downloaded: 3 * 1024 * 1024,
    total: 4 * 1024 * 1024,
    asset: "gwt-macos-arm64.tar.gz",
    version: "9.26.0",
  });

  const modal = fixture.document.getElementById("update-modal");
  const progress = modal.querySelector("[data-update-modal-progress]");
  const counter = modal.querySelector("[data-update-modal-byte-counter]");
  assert.ok(progress, "expected progress bar element");
  assert.ok(counter, "expected byte counter element");
  assert.equal(progress.getAttribute("aria-valuenow"), "75");
  assert.match(counter.textContent, /3.0\s*MB\s*\/\s*4.0\s*MB/);
});

test("phase19: cancel during download sends cancel_update_download and returns CTA to available", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  fixture.document
    .querySelector("[data-update-modal-cancel]")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "cancel_update_download" },
  ]);
  assert.equal(fixture.document.getElementById("update-modal"), null);
  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "available");
});

test("phase19: update_ready transitions modal to ready state with [Later] [Restart now]", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateReady({
    version: "9.26.0",
    asset_path: "/Users/x/.gwt/pending-update/9.26.0/gwt-macos-arm64.tar.gz",
  });

  const modal = fixture.document.getElementById("update-modal");
  assert.equal(modal.dataset.state, "ready");
  assert.ok(modal.querySelector("[data-update-modal-restart-now]"));
  assert.ok(modal.querySelector("[data-update-modal-later]"));
});

test("phase19: [Restart now] sends apply_update_restart_now without confirmation", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });

  fixture.document
    .querySelector("[data-update-modal-restart-now]")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_restart_now" },
  ]);
  assert.equal(
    fixture.confirmCalls.length,
    0,
    "Restart now must not gate behind window.confirm",
  );
});

test("phase19: [Later] closes modal, sends apply_update_later, and morphs CTA to ready state", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });

  fixture.document
    .querySelector("[data-update-modal-later]")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_later" },
  ]);
  assert.equal(fixture.document.getElementById("update-modal"), null);
  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "ready");
  assert.match(cta.textContent, /Update v9.26.0 ready\s*[—-]\s*Restart now/);
  assert.ok(fixture.document.querySelector("[data-update-cta-dismiss]"));
});

test("phase19: re-clicking CTA in ready state opens modal at ready (no re-download)", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });
  fixture.document.querySelector("[data-update-modal-later]").click();

  fixture.document.getElementById("update-cta").click();

  const modal = fixture.document.getElementById("update-modal");
  assert.ok(modal, "modal should reopen on re-click");
  assert.equal(modal.dataset.state, "ready");
  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_later" },
  ]);
});

test("phase19: dismiss in ready state hides CTA without losing pending binary", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });
  fixture.document.querySelector("[data-update-modal-later]").click();

  fixture.document
    .querySelector("[data-update-cta-dismiss]")
    .click();

  assert.equal(fixture.document.getElementById("update-cta"), null);
  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_later" },
  ]);
});

test("phase19: update_apply_error transitions modal to failed state with stage/reason/log/buttons", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateApplyError({
    stage: "Download asset",
    reason: "HTTP 503",
    log_path: "/Users/x/.gwt/logs/update-2026-05-10.log",
  });

  const modal = fixture.document.getElementById("update-modal");
  assert.equal(modal.dataset.state, "failed");
  const stage = modal.querySelector("[data-update-modal-stage]");
  const reason = modal.querySelector("[data-update-modal-reason]");
  const log = modal.querySelector("[data-update-modal-log]");
  assert.match(stage.textContent, /Download asset/);
  assert.match(reason.textContent, /HTTP 503/);
  assert.match(log.textContent, /update-2026-05-10/);
  assert.ok(modal.querySelector("[data-update-modal-open-log]"));
  assert.ok(modal.querySelector("[data-update-modal-retry]"));
  assert.ok(modal.querySelector("[data-update-modal-close]"));
});

test("phase19: failed-state [Retry] resends apply_update_start and switches modal back to downloading", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateApplyError({
    stage: "Download asset",
    reason: "HTTP 503",
    log_path: "/x.log",
  });

  fixture.document
    .querySelector("[data-update-modal-retry]")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_start" },
  ]);
  assert.equal(
    fixture.document.getElementById("update-modal").dataset.state,
    "downloading",
  );
});

test("phase19: failed-state [Open log] sends open_update_log with the log_path", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateApplyError({
    stage: "Replace binary",
    reason: "EPERM",
    log_path: "/Users/x/.gwt/logs/update-2026-05-10.log",
  });

  fixture.document
    .querySelector("[data-update-modal-open-log]")
    .click();

  assert.deepEqual(fixture.sent.slice(-1), [
    {
      kind: "open_update_log",
      log_path: "/Users/x/.gwt/logs/update-2026-05-10.log",
    },
  ]);
});

test("phase19: failed-state [Close] closes modal and returns CTA to available", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateApplyError({
    stage: "Download asset",
    reason: "HTTP 503",
    log_path: "/x.log",
  });

  fixture.document
    .querySelector("[data-update-modal-close]")
    .click();

  assert.equal(fixture.document.getElementById("update-modal"), null);
  assert.equal(
    fixture.document.getElementById("update-cta").dataset.status,
    "available",
  );
});

test("phase19: update_apply_pending_persisted morphs CTA directly to ready state (next-launch path)", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  controller.handleUpdateApplyPendingPersisted({ version: "9.26.0" });

  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "ready");
  assert.match(cta.textContent, /Update v9.26.0 ready\s*[—-]\s*Restart now/);
  assert.ok(fixture.document.querySelector("[data-update-cta-dismiss]"));
});

test("phase19: controller exposes the Phase 19 event handlers required by app.js wiring", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  assert.equal(typeof controller.handleUpdateProgress, "function");
  assert.equal(typeof controller.handleUpdateReady, "function");
  assert.equal(typeof controller.handleUpdateApplyError, "function");
  assert.equal(typeof controller.handleUpdateApplyPendingPersisted, "function");
});

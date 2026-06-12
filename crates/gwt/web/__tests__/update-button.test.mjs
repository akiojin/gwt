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

// Phase 19 supersedes the Phase 14 window.confirm gate. The "click cancel"
// test is dropped because the new modal flow always sends apply_update_start;
// see the dedicated phase19 tests below for the modal-driven cancel path.
test("update CTA click sends apply_update_start and shows applying state with modal", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.getElementById("update-cta").click();

  assert.deepEqual(fixture.sent, [{ kind: "apply_update_start" }]);
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
  assert.deepEqual(fixture.sent, [
    { kind: "apply_update_start" },
    { kind: "apply_update_start" },
  ]);
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
  assert.deepEqual(fixture.sent, [{ kind: "apply_update_start" }]);
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
  // Phase 19 replaces showError() with handleUpdateApplyError().
  assert.match(appSource, /updateCtaController\.handleUpdateApplyError\(/);
  assert.match(appSource, /updateCtaController\.handleUpdateProgress\(/);
  assert.match(appSource, /updateCtaController\.handleUpdateReady\(/);
  assert.match(
    appSource,
    /updateCtaController\.handleUpdateApplyPendingPersisted\(/,
  );
});

test("legacy split update toast and button surfaces are removed", () => {
  assert.doesNotMatch(appSource, /showUpdateToast/);
  assert.doesNotMatch(appSource, /showUpdateButton/);
  assert.doesNotMatch(indexHtml, /\.update-toast\b/);
  assert.doesNotMatch(indexHtml, /\.update-button\b/);
});

test("update CTA floats fixed bottom-right again (user verification 2026-06-12)", () => {
  // SPEC-2356 moved the CTA into a sidebar Update section, but the user found
  // it undiscoverable there — the shell returns to its previous fixed
  // bottom-right home and the sidebar anchor is gone from index.html.
  const shellMatch = componentsCss.match(/\.update-cta-shell\s*\{[^}]+\}/);
  assert.ok(shellMatch, "expected .update-cta-shell rule inside components.css");
  assert.match(shellMatch[0], /position:\s*fixed/);
  assert.match(shellMatch[0], /bottom:/);
  assert.match(shellMatch[0], /right:/);
  assert.doesNotMatch(indexHtml, /id="update-cta-anchor"/);
  const styleMatch = componentsCss.match(/\.update-cta\s*\{[^}]+\}/);
  assert.ok(styleMatch, "expected .update-cta rule inside components.css");
  assert.match(componentsCss, /\.update-cta\.is-applying\s*\{/);
  assert.match(componentsCss, /\.update-cta\.is-error\s*\{/);
  assert.match(componentsCss, /\.update-cta__dismiss\s*\{/);
  assert.doesNotMatch(indexHtml, /\.update-cta\s*\{/);
});

test("update CTA shell mounts on document.body (user verification 2026-06-12)", () => {
  const { document } = parseHTML(
    '<!doctype html><html><body><aside class="op-sidebar"></aside></body></html>',
  );
  const sent = [];
  const controller = createUpdateCtaController({
    document,
    send: (m) => sent.push(m),
    confirmUpdate: () => true,
    setVersionState: () => {},
  });
  controller.showAvailable("9.50.0");
  const shell = document.getElementById("update-cta-shell");
  assert.ok(shell, "expected the update CTA shell to be created");
  assert.equal(shell.parentElement, document.body, "shell mounts on <body>");
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

function cssRule(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = componentsCss.match(new RegExp(`${escaped}\\s*\\{([^}]+)\\}`));
  assert.ok(match, `expected ${selector} rule inside components.css`);
  return match[1];
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

test("phase19: repeated update_ready for the same version is idempotent (Playwright update-modal.spec.ts harness)", () => {
  // Live backend re-polls update_state / re-emits update_ready while the
  // Playwright spec is driving the post-click flow. The old implementation
  // recreated the entire modal panel on every call, detaching the
  // [Later] / [Restart now] buttons mid-click and exhausting Playwright's
  // detach-retry. Pinning idempotency here keeps that harness path green
  // and protects the production runtime from button churn on transient
  // duplicate emissions from the update poller.
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });
  const laterFirst = fixture.document.querySelector("[data-update-modal-later]");
  assert.ok(laterFirst, "first ready render must produce a Later button");

  controller.handleUpdateReady({ version: "9.26.0", asset_path: "/x" });
  const laterSecond = fixture.document.querySelector("[data-update-modal-later]");
  assert.strictEqual(
    laterSecond,
    laterFirst,
    "duplicate update_ready with same version must reuse the existing Later button (no detach)",
  );

  // A new version, in contrast, MUST re-render the panel.
  controller.handleUpdateReady({ version: "9.27.0", asset_path: "/x" });
  const laterThird = fixture.document.querySelector("[data-update-modal-later]");
  assert.notStrictEqual(
    laterThird,
    laterFirst,
    "update_ready with new version must replace the Later button",
  );
});

test("phase19: repeated downloading render for same version preserves [Cancel] button (Playwright update-modal.spec.ts harness)", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();

  const cancelFirst = fixture.document.querySelector("[data-update-modal-cancel]");
  assert.ok(cancelFirst, "downloading render must produce a Cancel button");

  // Same-version progress events used to trigger re-render via
  // updateProgressDisplay paths in older revisions. Calling showAvailable
  // again (live backend often re-broadcasts on reconnect) used to clear
  // the modal too. Pin idempotency at the renderModalDownloading layer.
  controller.showAvailable("9.26.0");
  fixture.document.getElementById("update-cta").click();
  const cancelSecond = fixture.document.querySelector("[data-update-modal-cancel]");
  assert.strictEqual(
    cancelSecond,
    cancelFirst,
    "repeated downloading entry for same version must reuse the existing Cancel button",
  );
});

test("SPEC-2780: ready modal exposes 'View release notes' when openReleaseNotes is wired", () => {
  const fixture = createFixture();
  const releaseNotesCalls = [];
  const controller = createUpdateCtaController({
    ...fixture.options,
    openReleaseNotes: (version) => releaseNotesCalls.push(version),
  });
  controller.showAvailable("9.40.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.40.0", asset_path: "/x" });

  const link = fixture.document.querySelector(
    "[data-update-modal-release-notes]",
  );
  assert.ok(link, "release notes link must be present when wired");
  link.click();
  assert.deepEqual(releaseNotesCalls, ["9.40.0"]);
});

test("phase20: ready modal copy names gwt and keeps release notes out of the action group", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController({
    ...fixture.options,
    openReleaseNotes: () => {},
  });
  controller.showAvailable("9.48.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.48.0", asset_path: "/x" });

  const modal = fixture.document.getElementById("update-modal");
  const version = modal.querySelector(".update-modal__version");
  const releaseNotes = modal.querySelector("[data-update-modal-release-notes]");
  const actions = modal.querySelector(".update-modal__actions");

  assert.equal(version.textContent, "gwt v9.48.0 is ready to install.");
  assert.ok(releaseNotes, "release notes action must be present");
  assert.equal(releaseNotes.className, "update-modal__link");
  assert.equal(
    actions.contains(releaseNotes),
    false,
    "release notes must not be grouped with Later / Restart now buttons",
  );
  assert.deepEqual(
    [...actions.children].map((node) => node.textContent),
    ["Later", "Restart now"],
  );
});

test("phase20: update modal css defines compact panel and link-style release notes action", () => {
  const modalRule = cssRule(".update-modal");
  const panelRule = cssRule(".update-modal__panel");
  const actionRule = cssRule(".update-modal__actions");
  const buttonRule = cssRule(".update-modal__btn");
  const linkRule = cssRule(".update-modal__link");

  assert.match(modalRule, /background:\s*var\(--color-scrim/);
  assert.match(panelRule, /width:\s*min\(420px,\s*calc\(100vw - 32px\)\)/);
  assert.match(panelRule, /padding:\s*var\(--space-6\)/);
  assert.match(actionRule, /align-items:\s*center/);
  assert.match(buttonRule, /min-height:\s*36px/);
  assert.match(buttonRule, /font-size:\s*var\(--type-sm\)/);
  assert.match(linkRule, /border:\s*0/);
  assert.match(linkRule, /background:\s*transparent/);
  assert.match(linkRule, /color:\s*var\(--color-link\)/);
});

test("SPEC-2780: ready modal omits release notes link when openReleaseNotes is absent", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.40.0");
  fixture.document.getElementById("update-cta").click();
  controller.handleUpdateReady({ version: "9.40.0", asset_path: "/x" });

  const link = fixture.document.querySelector(
    "[data-update-modal-release-notes]",
  );
  assert.equal(link, null);
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

// SPEC-2041 Phase 19 (FR-064 follow-up, CodeRabbit review on PR #2635):
// Later -> commit_update_later_pending can detect that the manifest persisted
// by ApplyUpdateStart's worker thread vanished (external cleanup, disk-full
// race). In that case backend emits update_apply_error AFTER the CTA has
// already morphed to "ready". The frontend must surface the failure modal
// even though status === "ready" so the user is not silently misled.
test("phase19: update_apply_error in ready state re-opens failed modal", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  // Simulate the post-Later state that the next-launch path lands in.
  controller.handleUpdateApplyPendingPersisted({ version: "9.26.0" });
  const ctaBefore = fixture.document.getElementById("update-cta");
  assert.equal(ctaBefore.dataset.status, "ready");

  // Backend reports the persisted manifest is gone.
  controller.handleUpdateApplyError({
    stage: "Persist pending",
    reason: "Pending update manifest is missing; download did not persist.",
    log_path: "/Users/x/.gwt/logs/update-2026-05-10.log",
  });

  const modal = fixture.document.getElementById("update-modal");
  assert.ok(modal, "modal must reopen so the failure is visible");
  assert.equal(modal.dataset.state, "failed");
  assert.match(
    modal.querySelector("[data-update-modal-stage]").textContent,
    /Persist pending/,
  );
  assert.match(
    modal.querySelector("[data-update-modal-reason]").textContent,
    /manifest is missing/,
  );

  const cta = fixture.document.getElementById("update-cta");
  assert.equal(
    cta.dataset.status,
    "applying",
    "CTA flips to applying so the modal sits visually on top",
  );
});

test("phase19: controller exposes the Phase 19 event handlers required by app.js wiring", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  assert.equal(typeof controller.handleUpdateProgress, "function");
  assert.equal(typeof controller.handleUpdateReady, "function");
  assert.equal(typeof controller.handleUpdateApplyError, "function");
  assert.equal(typeof controller.handleUpdateApplyPendingPersisted, "function");
});

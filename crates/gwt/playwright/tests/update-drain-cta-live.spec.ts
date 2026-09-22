/* Issue #3906 AC-12 — drain-and-apply update CTA (live backend).
 *
 * Drives the update CTA with `update_apply_pending_persisted` and
 * `issue_monitor_status` payloads injected through the test bridge, so the
 * draining state renders in a real Chromium against the live frontend without
 * waiting for a real release or a real drain. Runs in both theme projects and
 * fails on any console / page error. Skipped when `GWT_PLAYWRIGHT_BASE_URL`
 * is unset, like the other live specs.
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Update drain CTA", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("issue_monitor_status.update_drain renders the draining CTA and clears back to ready", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(error.message));

    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressUpdateApplyStart: true,
    });

    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);

    await inject({ kind: "update_apply_pending_persisted", version: "9.99.0" });
    const cta = page.locator("#update-cta");
    await expect(cta).toHaveText(/Update v9\.99\.0 ready/);

    const since = new Date(Date.now() - 12 * 60 * 1000).toISOString();
    await inject({
      kind: "issue_monitor_status",
      status: {
        enabled: true,
        state: "update_drain",
        update_drain: {
          version: "9.99.0",
          since,
          reason: "auto",
          blocking: [
            { kind: "active_pane", window_id: "w1", label: "work/issue-1", state: "running" },
            { kind: "pending_acquire_claim", issue_number: 42 },
          ],
        },
      },
    });
    await expect(cta).toHaveAttribute("data-status", "draining");
    await expect(cta).toHaveText("Update v9.99.0 pending — draining 2 agents (12 min)");
    await expect(cta).toHaveClass(/is-draining/);
    await expect(cta).toBeEnabled();
    await expect(page.locator("[data-update-cta-dismiss]")).toBeVisible();

    // The CTA colour comes from the Operator needs-input token, not a literal.
    const color = await cta.evaluate((node) => getComputedStyle(node).color);
    const token = await page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--color-state-needs-input").trim(),
    );
    expect(token).not.toEqual("");
    expect(color).not.toEqual("");

    await inject({
      kind: "issue_monitor_status",
      status: { enabled: true, state: "idle" },
    });
    await expect(cta).toHaveAttribute("data-status", "ready");
    await expect(cta).toHaveText(/Update v9\.99\.0 ready/);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // Issue #3906 AC-7 / #4076 AC-5: the backend's `update_auto_apply` phases
  // turn the CTA into the cancel control for the grace, withdraw it, and
  // announce the apply. The click during the grace sends the cancel request.
  test("update_auto_apply phases drive the CTA and clicking during the grace cancels", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await page.addInitScript(() => {
      (window as any).__gwtSentKinds = [];
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        try {
          const payload = typeof data === "string" ? JSON.parse(data) : null;
          if (payload && typeof payload.kind === "string") {
            (window as any).__gwtSentKinds.push(payload.kind);
          }
        } catch {
          /* no-op */
        }
        return originalSend.call(this, data);
      };
    });

    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressUpdateApplyStart: true,
    });

    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);

    await inject({ kind: "update_apply_pending_persisted", version: "9.99.0" });
    const cta = page.locator("#update-cta");
    await expect(cta).toHaveText(/Update v9\.99\.0 ready/);

    await inject({ kind: "update_auto_apply", version: "9.99.0", phase: "scheduled", grace_secs: 60 });
    await expect(cta).toHaveAttribute("data-status", "scheduled");
    await expect(cta).toHaveClass(/is-scheduled/);
    await expect(cta).toHaveText("Update v9.99.0 applies in 60 s — click to cancel");
    await expect(cta).toBeEnabled();
    await expect(page.locator("#update-modal")).toHaveCount(0);

    await cta.click();
    await expect(cta).toHaveText("Cancelling automatic update…");
    const sentKinds = await page.evaluate(() => (window as any).__gwtSentKinds as string[]);
    expect(sentKinds).toContain("cancel_update_auto_apply");

    await inject({ kind: "update_auto_apply", version: "9.99.0", phase: "cancelled" });
    await expect(cta).toHaveAttribute("data-status", "ready");
    await expect(cta).toHaveText(/Update v9\.99\.0 ready/);

    await inject({ kind: "update_auto_apply", version: "9.99.0", phase: "applying" });
    await expect(cta).toHaveAttribute("data-status", "applying");
    await expect(cta).toHaveText("Applying update v9.99.0…");
    await expect(cta).toBeDisabled();

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // Issue #4376 AC-5 / AC-6 / AC-8: the manual route. The user clicks the
  // update CTA, the download lands while agents are running, and the backend
  // raises the drain instead of inviting Restart now. The CTA must read as
  // waiting, name what it waits for, and offer both the immediate override
  // and a way to stop waiting.
  test("a manual update click that lands on running agents drains, names its blockers and can stop waiting", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(error.message));
    await page.addInitScript(() => {
      (window as any).__gwtSentKinds = [];
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        try {
          const payload = typeof data === "string" ? JSON.parse(data) : null;
          if (payload && typeof payload.kind === "string") {
            (window as any).__gwtSentKinds.push(payload.kind);
          }
        } catch {
          /* no-op */
        }
        return originalSend.call(this, data);
      };
    });

    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      suppressUpdateApplyStart: true,
    });

    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);

    await inject({ kind: "update_state", state: "available", current: "9.98.0", latest: "9.99.0" });
    const cta = page.locator("#update-cta");
    await expect(cta).toHaveText("Update available: v9.99.0 - Click to update");

    // The manual click: the download modal opens and the request is sent.
    await cta.click();
    const modal = page.locator("#update-modal");
    await expect(modal).toHaveAttribute("data-state", "downloading");
    await inject({ kind: "update_ready", version: "9.99.0", asset_path: "/tmp/gwt-9.99.0" });
    await expect(modal).toHaveAttribute("data-state", "ready");

    // The backend found running agents at staging time and raised the drain:
    // the ready modal gives way to the draining CTA.
    const since = new Date(Date.now() - 3 * 60 * 1000).toISOString();
    await inject({
      kind: "issue_monitor_status",
      status: {
        enabled: true,
        autonomous_mode: false,
        state: "update_drain",
        update_drain: {
          version: "9.99.0",
          since,
          reason: "auto",
          blocking: [
            { kind: "active_pane", window_id: "w1", label: "work/issue-4376", state: "running" },
            { kind: "pending_acquire_claim", issue_number: 42 },
          ],
        },
      },
    });
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveAttribute("data-status", "draining");
    await expect(cta).toHaveText("Update v9.99.0 pending — draining 2 agents (3 min)");
    await expect(cta).toHaveAttribute(
      "title",
      "Update v9.99.0 pending — draining 2 agents (3 min). Waiting for: work/issue-4376 (running), claim for #42. New launches are held; agents are never stopped. Click to apply now anyway or stop waiting.",
    );
    await expect(cta).toBeEnabled();

    // Clicking the draining CTA: the modal names the blockers and offers both
    // the override and Stop waiting.
    await cta.click();
    await expect(modal).toHaveAttribute("data-state", "ready");
    await expect(modal).toHaveAttribute("data-variant", "anyway");
    await expect(modal).toContainText("Waiting for: work/issue-4376 (running), claim for #42");
    await expect(modal.locator("[data-update-modal-restart-now]")).toHaveText("Apply now anyway");
    await expect(modal.locator("[data-update-modal-later]")).toHaveCount(0);
    const stopWaiting = modal.locator("[data-update-modal-stop-waiting]");
    await expect(stopWaiting).toHaveText("Stop waiting");

    await stopWaiting.click();
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveText("Stopping the wait…");
    const sentKinds = await page.evaluate(() => (window as any).__gwtSentKinds as string[]);
    expect(sentKinds).toContain("cancel_update_auto_apply");
    expect(sentKinds).not.toContain("apply_update_restart_now");

    await inject({ kind: "update_auto_apply", version: "9.99.0", phase: "cancelled" });
    await expect(cta).toHaveAttribute("data-status", "ready");
    await expect(cta).toHaveText(/Update v9\.99\.0 ready/);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });
});

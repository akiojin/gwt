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

  test("automatic staging reaches drain and scheduled apply without a click or modal", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("pageerror", (error) => errors.push(error.message));
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressUpdateApplyStart: true });
    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);

    await inject({ kind: "update_state", state: "available", current: "9.105.0", latest: "9.106.0" });
    await inject({ kind: "update_progress", version: "9.106.0", downloaded: 100, total: 100 });
    await inject({ kind: "update_ready", version: "9.106.0", asset_path: "staged-test-payload" });
    await expect(page.locator("#update-modal")).toHaveCount(0);
    await inject({
      kind: "issue_monitor_status",
      status: {
        enabled: true, state: "update_drain",
        update_drain: {
          version: "9.106.0", since: new Date().toISOString(), reason: "auto",
          blocking: [{ kind: "active_pane", window_id: "agent-4801", label: "work/issue-4801", state: "running" }],
        },
      },
    });
    const cta = page.locator("#update-cta");
    await expect(cta).toHaveAttribute("data-status", "draining");
    await expect(cta).toContainText("Update v9.106.0 pending");
    await expect(page.locator("#update-modal")).toHaveCount(0);
    await inject({ kind: "update_auto_apply", version: "9.106.0", phase: "scheduled", grace_secs: 60 });
    await expect(cta).toHaveText("Update v9.106.0 applies in 60 s — click to cancel");
    await expect(page.locator("#update-modal")).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test("administrator-required staging keeps a manual CTA and the reason in notifications", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("pageerror", (error) => errors.push(error.message));
    // The staged release is injected, so Later must not persist a nonexistent
    // manifest. Its acknowledgement is injected below, as in update-modal.spec.
    await page.addInitScript(() => {
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string" && JSON.parse(data).kind === "apply_update_later") return;
        return originalSend.call(this, data);
      };
    });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressUpdateApplyStart: true });
    const inject = (detail: Record<string, unknown>) =>
      page.evaluate((payload) => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
      }, detail);

    await inject({ kind: "update_state", state: "available", current: "9.105.0", latest: "9.106.0" });
    await inject({ kind: "update_ready", version: "9.106.0", asset_path: "staged-test-payload" });
    await inject({ kind: "update_auto_apply", version: "9.106.0", phase: "elevation_required" });
    await inject({
      kind: "issue_monitor_toast", level: "warn", issue_number: 0,
      message: "Update v9.106.0: Administrator privileges required; the update stays staged. Use the update button to apply manually.",
    });

    const cta = page.locator("#update-cta");
    await expect(cta).toHaveAttribute("data-status", "ready");
    await expect(cta).toContainText("Administrator privileges required");
    await expect(cta).toContainText("Restart now");
    await expect(cta).toBeEnabled();
    await expect(page.locator("#update-modal")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("update-elevation-required.png") });

    await page.locator("#op-notifications-button").click();
    const notice = page.locator("#notification-center .notification-center__item[data-level='warn']")
      .filter({ hasText: "Administrator privileges required" }).first();
    await expect(notice).toBeVisible();
    await expect(notice).toHaveAttribute("data-level", "warn");
    await expect(notice).toContainText("Use the update button to apply manually");
    await page.getByRole("button", { name: "Close notifications", exact: true }).click();

    await cta.click();
    const modal = page.locator("#update-modal");
    await expect(modal).toHaveAttribute("data-state", "ready");
    await expect(modal).toHaveAttribute("data-version", "9.106.0");
    // Observe the manual action without invoking an actual installer/restart.
    await expect(modal.locator("[data-update-modal-restart-now]")).toHaveText("Restart now");
    await modal.locator("[data-update-modal-later]").click();
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveAttribute("data-status", "ready");
    await expect(cta).toContainText("Administrator privileges required");
    await inject({ kind: "update_apply_pending_persisted", version: "9.106.0" });
    await expect(cta).toContainText("Administrator privileges required");
    expect(errors).toEqual([]);
  });

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
    // The Hub socket may still be opening; observe its queued send flushing.
    await expect.poll(() => page.evaluate(() => (window as any).__gwtSentKinds as string[]))
      .toContain("cancel_update_auto_apply");

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
  }, testInfo) => {
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
            { kind: "active_pane", window_id: "llmlb::agent-14", label: "llmlb: SPEC #821 T012c", state: "running" },
          ],
        },
      },
    });
    await expect(modal).toHaveCount(0);
    await expect(cta).toHaveAttribute("data-status", "draining");
    await expect(cta).toHaveText("Update v9.99.0 pending — draining 1 agents (3 min)");
    await expect(cta).toHaveAttribute(
      "title",
      "Update v9.99.0 pending — draining 1 agents (3 min). Waiting for: llmlb: SPEC #821 T012c (running). New launches are held; agents are never stopped. Click to apply now anyway or stop waiting.",
    );
    await expect(cta).toBeEnabled();

    // Clicking the draining CTA: the modal names the blockers and offers both
    // the override and Stop waiting.
    await cta.click();
    await expect(modal).toHaveAttribute("data-state", "ready");
    await expect(modal).toHaveAttribute("data-variant", "anyway");
    await expect(modal).toContainText("Waiting for: llmlb: SPEC #821 T012c (running)");
    await expect(modal.locator("[data-update-modal-restart-now]")).toHaveText("Apply now anyway");
    await expect(modal.locator("[data-update-modal-later]")).toHaveCount(0);
    const stopWaiting = modal.locator("[data-update-modal-stop-waiting]");
    await expect(stopWaiting).toHaveText("Stop waiting");
    await page.screenshot({ path: testInfo.outputPath("update-drain-other-project.png") });

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

  test("Issue #5646: immediate restart previews all active work and requires a safe explicit choice", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      (window as any).__gwtRestartRequests = [];
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (["get_update_restart_impact", "apply_update_restart_now", "apply_update_when_idle"].includes(payload?.kind)) {
          (window as any).__gwtRestartRequests.push(payload);
          // These actions are observed locally. No installer, restart, or
          // nonexistent staged manifest reaches the isolated live backend.
          return;
        }
        return originalSend.call(this, data);
      };
    });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressUpdateApplyStart: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
    const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }, detail);
    const requests = () => page.evaluate(() =>
      (window as any).__gwtRestartRequests as Array<{ kind: string; request_id?: string }>,
    );
    const blocking = [
      ...Array.from({ length: 12 }, (_, index) => ({
        kind: "active_pane", window_id: `restart-agent-${index + 1}`,
        label: `work/issue-${5601 + index}`, state: index === 11 ? "starting" : "running",
      })),
      { kind: "pending_acquire_claim", issue_number: 5644 },
      { kind: "pending_acquire_claim", issue_number: 5645 },
      { kind: "held_verification_lease", lease_id: "restart-verification" },
    ];
    const issueNumbers = [5601, 5602, 5603, 5604, 5605, 5606, 5607, 5608, 5609, 5610, 5611, 5612, 5644, 5645, 5646];
    await inject({ kind: "update_apply_pending_persisted", version: "9.99.0" });
    const cta = page.locator("#update-cta");
    const modal = page.locator("#update-modal");
    const wait = modal.locator("[data-update-modal-wait-for-idle]");
    const confirm = modal.locator("[data-update-modal-confirm-restart]");
    const cancel = modal.locator("[data-update-modal-cancel-restart]");
    const preview = async () => {
      const previous = (await requests()).filter((request) => request.kind === "get_update_restart_impact").length;
      await cta.click();
      await modal.locator("[data-update-modal-restart-now]").click();
      await expect(modal).toHaveAttribute("data-state", "restart-preview");
      await expect(wait).toBeDisabled();
      await expect(confirm).toBeDisabled();
      await expect.poll(async () => (await requests()).filter((request) => request.kind === "get_update_restart_impact").length)
        .toBe(previous + 1);
      const request = (await requests()).filter((request) => request.kind === "get_update_restart_impact").at(-1)!;
      await inject({ kind: "update_restart_impact", request_id: request.request_id, blocking, issue_numbers: issueNumbers });
      await expect(modal).toHaveAttribute("data-state", "restart-confirm");
    };

    await preview();
    await expect(modal.locator(".modal-shell")).toHaveAttribute("role", "dialog");
    await expect(modal.locator(".modal-shell")).toHaveAttribute("aria-modal", "true");
    await expect(modal).toContainText("Restart gwt and stop active work?");
    await expect(modal).toContainText("Running / Starting agents: 12. Pending claims: 2. Verification leases: 1.");
    for (const number of issueNumbers) await expect(modal).toContainText(`#${number}`);
    await expect(modal).toContainText("Restart now stops these agents and interrupts pending claims and verification.");
    await expect(wait).toHaveText("Apply after all agents finish");
    await expect(wait).toHaveClass(/update-modal__btn--primary/);
    await expect(wait).toBeFocused();
    await expect(confirm).toHaveText("Restart now");
    expect((await requests()).filter((request) => request.kind === "apply_update_restart_now")).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath("update-restart-impact.png") });
    await cancel.click();
    await expect(modal).toHaveCount(0);
    expect((await requests()).filter((request) => request.kind !== "get_update_restart_impact")).toEqual([]);

    await preview();
    await wait.click();
    await expect.poll(async () => (await requests()).map((request) => request.kind)).toContain("apply_update_when_idle");
    expect((await requests()).filter((request) => request.kind === "apply_update_restart_now")).toEqual([]);
    // Complete the synthetic pending apply before testing its explicit override.
    await inject({ kind: "update_apply_pending_persisted", version: "9.99.0" });
    await inject({ kind: "issue_monitor_status", status: { enabled: true, state: "update_drain", update_drain: {
      version: "9.99.0", since: new Date().toISOString(), reason: "auto", blocking,
    } } });
    await expect(cta).toHaveAttribute("data-status", "draining");
    await preview();
    await expect(wait).toBeFocused();
    expect((await requests()).filter((request) => request.kind === "apply_update_restart_now")).toEqual([]);
    await confirm.click();
    await expect.poll(async () => (await requests()).filter((request) => request.kind === "apply_update_restart_now").length).toBe(1);
    expect(errors).toEqual([]);
  });
});

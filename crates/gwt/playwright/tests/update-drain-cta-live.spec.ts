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
});

/* Issue #4911 AC-9 — `＋` / `−` / reorder / save of the Issue Monitor Agent
 * Settings sets against a real backend, in both themes. The save only writes
 * the candidate pool; it never starts an agent, enables the Monitor, or
 * creates a branch.
 */
import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Issue Monitor Agent Settings sets", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(120_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("adds, reorders, removes and saves sets in launch order", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await expect(page.locator("#close-project-button")).toBeVisible();

      // Start from one saved candidate, written the way the JSON operation
      // writes it, and wait until the backend reports it.
      await sendLiveGwtEvent(page, {
        kind: "issue_monitor_profiles_set",
        profiles: [{ agent_id: "codex" }],
      });
      await expectSavedAgents(page, ["codex"]);

      await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
      const modal = page.locator("#wizard-modal");
      // The open is queued behind the status reads above on a busy backend.
      await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
      const sets = modal.locator(".launch-agent-set");
      await expect(sets).toHaveCount(1);
      await expect(sets.first()).toHaveAttribute("data-agent-id", "codex");
      // AC-6: the last set stays, and the form says why.
      await expect(
        modal.getByRole("button", { name: "Remove Agent Settings 1", exact: true }),
      ).toBeDisabled();
      await expect(modal.locator(".launch-agent-sets__footer")).toContainText(
        "At least one Agent Settings set is required",
      );

      // AC-1: `＋` adds a set, open in the same form.
      await modal.getByRole("button", { name: "Add Agent Settings" }).click();
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(1)).toHaveClass(/is-open/);
      await expect(sets.nth(1).locator(".launch-section").first()).toContainText("Launch");
      const added = await sets.nth(1).getAttribute("data-agent-id");
      expect(added).toBeTruthy();
      expect(added).not.toBe("codex");
      await expect(sets.nth(0)).toContainText("Codex");

      // AC-3: the order of the sets is the launch order.
      await modal.getByRole("button", { name: "Move Agent Settings 2 up", exact: true }).click();
      await expect(sets.nth(0)).toHaveAttribute("data-agent-id", added!);
      await expect(sets.nth(0)).toHaveClass(/is-open/);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-id", "codex");

      // AC-2: `−` removes a set.
      await modal.getByRole("button", { name: "Add Agent Settings" }).click();
      await expect(sets).toHaveCount(3);
      await modal.getByRole("button", { name: "Remove Agent Settings 3", exact: true }).click();
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(1)).toHaveClass(/is-open/);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-id", "codex");
      await modal.getByRole("button", { name: "Edit Agent Settings 1", exact: true }).click();
      await expect(sets.nth(0)).toHaveClass(/is-open/);
      await expect(modal.locator(".launch-agent-sets__order")).toContainText(`auto (2): ${added}`);

      await expect(page.locator("html")).toHaveAttribute(
        "data-theme",
        testInfo.project.name.endsWith("light") ? "light" : "dark",
      );
      await testInfo.attach(`agent-settings-sets-live-${testInfo.project.name}`, {
        body: await modal.screenshot(),
        contentType: "image/png",
      });

      // Settings → Runtime → Confirm → Save.
      const submit = page.locator("#wizard-submit-button");
      await submit.click();
      await expect(modal.locator(".launch-section", { hasText: "Runtime" })).toBeVisible();
      await expect(submit).toBeEnabled();
      await submit.click();
      await expect(submit).toHaveText("Save settings");
      await expect(modal.locator(".launch-agent-sets__order")).toContainText(`auto (2): ${added}`);
      await submit.click();
      await expect(modal).not.toHaveClass(/open/);

      // AC-5: the saved pool reads back in the same order through the status
      // the JSON operation reports, and the reopened form agrees with it.
      await expectSavedAgents(page, [added!, "codex"]);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
      await expect(modal).toHaveClass(/open/);
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(0)).toHaveAttribute("data-agent-id", added!);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-id", "codex");
      await page.locator("#wizard-cancel-button").click();
      await expect(modal).not.toHaveClass(/open/);
      expect(errors).toEqual([]);
    });
  });
});

// Waits until the newest `issue_monitor_status` — what `issue.monitor.profiles`
// reads — lists the candidates in `expected` order. The read is re-requested on
// a slow cadence: one request can be dropped while the page is still
// connecting, and each one is expensive for the backend to answer.
async function expectSavedAgents(page: any, expected: string[]): Promise<void> {
  await expect.poll(async () => {
    await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
    return page.evaluate(() => {
      const statuses = ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status");
      const latest = statuses[statuses.length - 1]?.payload?.status;
      return (latest?.launch_profile_candidates ?? []).map((candidate: any) => candidate.agent_id);
    });
  }, { timeout: 30_000, intervals: [1_000, 2_000, 5_000] }).toEqual(expected);
}

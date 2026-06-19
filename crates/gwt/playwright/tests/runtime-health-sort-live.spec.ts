import { expect, test } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Runtime health sort controls", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test.beforeEach(async ({ page }) => {
    await gotoLiveGwt(page, BASE);
    await page.waitForFunction(() => Boolean((window as any).__operatorShell?.applyRuntimeHealth));
  });

  test("sort buttons keep the process detail open after click", async ({ page }) => {
    await page.evaluate(() => {
      (window as any).__operatorShell.applyRuntimeHealth({
        state: "hot",
        cpu_percent: 122,
        memory_bytes: 5 * 1024 * 1024 * 1024,
        process_count: 3,
        runner_count: 1,
        queue: {
          client_count: 1,
          queued_entries: 0,
          dirty_panes: 0,
          dropped_lossy_delta: 0,
        },
        processes: [
          {
            pid: 701,
            parent_pid: null,
            role: "runner",
            name: "memory-hog",
            cpu_percent: 2,
            memory_bytes: 3 * 1024 * 1024 * 1024,
          },
          {
            pid: 702,
            parent_pid: null,
            role: "gwt",
            name: "cpu-burn",
            cpu_percent: 90,
            memory_bytes: 128 * 1024 * 1024,
          },
          {
            pid: 703,
            parent_pid: null,
            role: "gwtd",
            name: "balanced",
            cpu_percent: 20,
            memory_bytes: 768 * 1024 * 1024,
          },
        ],
      });
    });

    const perfCell = page.locator("#op-strip-runtime-health");
    const detail = page.locator("#op-runtime-health-detail");

    await perfCell.hover();
    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by Load",
    );

    await detail.getByRole("button", { name: "CPU" }).click();
    await page.waitForTimeout(200);

    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by CPU",
    );

    await detail.getByRole("button", { name: "Mem" }).click();
    await page.waitForTimeout(200);

    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by Mem",
    );
  });
});

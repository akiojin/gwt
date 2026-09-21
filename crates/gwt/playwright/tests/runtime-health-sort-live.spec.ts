import { expect, test } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";
const CPU_UNITS =
  /Aggregate CPU is logical-core-normalized host share \(0–100%\); process rows use 1 core = 100%\./;
const browserErrors = new WeakMap<object, string[]>();

test.describe("Runtime health sort controls", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test.beforeEach(async ({ page }) => {
    const errors: string[] = [];
    browserErrors.set(page, errors);
    page.on("console", (message) => {
      if (message.type() === "error") {
        const location = message.location();
        const source = location.url
          ? ` @ ${location.url}:${location.lineNumber}:${location.columnNumber}`
          : "";
        errors.push(`console: ${message.text()}${source}`);
      }
    });
    page.on("pageerror", (error) => errors.push(`page: ${error.message}`));
    await gotoLiveGwt(page, BASE);
    await page.waitForFunction(() => Boolean((window as any).__operatorShell?.applyRuntimeHealth));
  });

  test.afterEach(async ({ page }) => {
    expect(browserErrors.get(page) ?? []).toEqual([]);
  });

  test("sort buttons keep the process detail open after click", async ({ page }) => {
    await page.evaluate(() => {
      const operatorShell = (window as any).__operatorShell;
      operatorShell.applyRuntimeHealth({
        state: "hot",
        cpu_percent: 501.2,
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
            cpu_percent: 118.4,
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
      operatorShell.applyRuntimeHealth = () => {};
    });

    const perfCell = page.locator("#op-strip-runtime-health");
    const perfValue = page.locator("#op-strip-runtime-health-value");
    const detail = page.locator("#op-runtime-health-detail");

    await expect(perfValue).toHaveText("HOT 100% 5.0G");
    await expect(perfCell).toHaveAttribute("title", CPU_UNITS);
    await expect(perfCell).toHaveAttribute("aria-label", CPU_UNITS);

    await perfCell.hover();
    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__units")).toHaveText(CPU_UNITS);
    await expect(detail).toContainText("118%");
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by Load",
    );

    await perfCell.focus();
    await expect(detail).toBeVisible();

    for (const theme of ["dark", "light"]) {
      await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await perfCell.focus();
      await expect(detail).toBeVisible();
      await expect(perfValue).toHaveText("HOT 100% 5.0G");
    }

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

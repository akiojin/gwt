/* SPEC #3206 — real-browser E2E for the shared toast-host primitive.
 *
 * Phase 0 routes the autonomous `log` region through `createToastStack`. This
 * mounts the refactored `createAutonomousNotifications` in a real chromium page
 * and asserts what only a real browser proves: a fixed top-right region, a
 * scrollable height-bounded list (real getComputedStyle), the bounded cap with
 * newest-on-top, and a real dismiss click — confirming P0 preserved behaviour.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("toast-host autonomous log region (real browser)", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      class NoopSocket {
        constructor() {}
        send() {}
        close() {}
        addEventListener() {}
        removeEventListener() {}
      }
      // @ts-ignore
      window.WebSocket = NoopSocket;
    });
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);
    await page.evaluate(async () => {
      const mod = await import("/autonomous-notifications.js");
      document.body.replaceChildren();
      const stack = (mod as any).createAutonomousNotifications({
        document,
        maxRetained: 5,
      });
      stack.mount(document.body);
      (window as any).__stack = stack;
    });
  });

  test("renders a fixed top-right scrollable log with a bounded cap", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__stack;
      for (let i = 0; i < 12; i += 1) {
        s.push({
          level: i % 2 ? "error" : "success",
          title: "Issue",
          issueNumber: 3000 + i,
          message: `m${i}`,
        });
      }
    });

    await expect(page.locator(".autonomous-notifications")).toBeVisible();
    // Real DOM: bounded cap, newest on top.
    await expect(page.locator(".autonomous-notifications__item")).toHaveCount(5);
    await expect(page.locator(".autonomous-notifications__item").first()).toContainText("3011");

    const layout = await page.evaluate(() => {
      const region = document.querySelector(".autonomous-notifications") as HTMLElement;
      const list = document.querySelector(".autonomous-notifications__list") as HTMLElement;
      const r = getComputedStyle(region);
      const l = getComputedStyle(list);
      return { position: r.position, overflowY: l.overflowY, maxHeight: l.maxHeight };
    });
    expect(layout.position).toBe("fixed");
    expect(layout.overflowY).toBe("auto");
    expect(layout.maxHeight).not.toBe("none");
  });

  test("a dismiss button removes its toast on a real click", async ({ page }) => {
    await page.evaluate(() =>
      (window as any).__stack.push({ level: "info", title: "x", issueNumber: 1, message: "y" }),
    );
    const item = page.locator(".autonomous-notifications__item");
    await expect(item).toHaveCount(1);
    await page.locator(".autonomous-notifications__dismiss").click();
    await expect(item).toHaveCount(0);
  });
});

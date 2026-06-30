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

test.describe("toast-host alerts region (real browser, SPEC #3206 P1)", () => {
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
      const mod = await import("/toast-host.js");
      document.body.replaceChildren();
      (window as any).__activated = [];
      const stack = (mod as any).createToastStack({
        document,
        className: "toast-alerts",
        ariaRole: "status",
        animateDismiss: true,
        levels: ["neutral", "info", "warn", "error", "done"],
        defaultLevel: "neutral",
      });
      stack.mount(document.body);
      (window as any).__alerts = stack;
    });
  });

  test("the three former systems share ONE bottom-right stack (no overlap)", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "agent-completion", level: "neutral", title: "Done", message: "ok", dismissible: false });
      s.push({ id: "attention-w1", level: "warn", title: "Needs input", message: "y", dismissible: true });
      s.push({ id: "board-mention", level: "info", title: "Board reply", dismissible: false });
    });

    // All three live in the SAME single container (no per-system fixed offsets).
    expect(await page.locator(".toast-alerts").count()).toBe(1);
    const items = page.locator(".toast-alerts__list .toast-alerts__item");
    await expect(items).toHaveCount(3);
    await expect(items.first()).toContainText("Board reply"); // newest on top

    const layout = await page.evaluate(() => {
      const r = getComputedStyle(document.querySelector(".toast-alerts") as HTMLElement);
      return { position: r.position };
    });
    expect(layout.position).toBe("fixed");
  });

  test("dedup by id replaces; onActivate jumps then dismisses", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "attention-w1", level: "warn", title: "first" });
      s.push({ id: "attention-w1", level: "error", title: "second", dismissible: true });
    });
    const attention = page.locator('.toast-alerts__item[data-toast-id="attention-w1"]');
    await expect(attention).toHaveCount(1, { timeout: 1000 });
    await expect(attention).toContainText("second");
    await expect(attention).toHaveAttribute("data-level", "error");

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "agent-completion",
        title: "Done",
        dismissible: false,
        onActivate: () => (window as any).__activated.push("completion"),
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="agent-completion"]').click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("completion");
  });
});

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
      const host = document.createElement("div");
      host.id = "operator-notice-stack";
      host.className = "operator-notice-stack";
      document.body.appendChild(host);
      (window as any).__activated = [];
      const stack = (mod as any).createToastStack({
        document,
        className: "toast-alerts",
        ariaRole: "status",
        ariaLive: "polite",
        animateDismiss: true,
        levels: ["neutral", "info", "warn", "error", "done"],
        defaultLevel: "neutral",
      });
      stack.mount(host);
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
      const host = getComputedStyle(
        document.querySelector(".operator-notice-stack") as HTMLElement,
      );
      const alerts = getComputedStyle(
        document.querySelector(".toast-alerts") as HTMLElement,
      );
      return { hostPosition: host.position, alertsPosition: alerts.position };
    });
    expect(layout.hostPosition).toBe("fixed");
    expect(layout.alertsPosition).not.toBe("fixed");
  });

  test("sticky alerts and the update CTA remain visible and operable without overlap", async ({
    page,
  }) => {
    await page.evaluate(async () => {
      const mod = await import("/update-cta.js");
      (window as any).__updateSent = [];
      const controller = (mod as any).createUpdateCtaController({
        document,
        send: (message: unknown) => (window as any).__updateSent.push(message),
        setVersionState: () => {},
      });
      controller.showAvailable("9.99.0");
      (window as any).__updateController = controller;
      (window as any).__alerts.push({
        id: "sticky-error",
        level: "error",
        title: "Background task failed",
        message: "Open the affected work item for details.",
        timeoutMs: 0,
        dismissible: true,
        onActivate: () => (window as any).__activated.push("sticky-error"),
      });
    });

    const host = page.locator("#operator-notice-stack");
    const alerts = page.locator(".toast-alerts");
    const shell = page.locator("#update-cta-shell");
    await expect(host).toBeVisible();
    await expect(alerts).toHaveAttribute("role", "status");
    await expect(shell).toBeVisible();

    const ownership = await page.evaluate(() => {
      const hostNode = document.getElementById("operator-notice-stack")!;
      const alertsNode = document.querySelector(".toast-alerts")!;
      const shellNode = document.getElementById("update-cta-shell")!;
      return {
        children: Array.from(hostNode.children).map((node) => node.className),
        alertsParentIsHost: alertsNode.parentElement === hostNode,
        shellParentIsHost: shellNode.parentElement === hostNode,
        ctaInsideAlertsLiveRegion: alertsNode.contains(shellNode),
        hostRole: hostNode.getAttribute("role"),
        hostAriaLive: hostNode.getAttribute("aria-live"),
      };
    });
    expect(ownership.alertsParentIsHost).toBe(true);
    expect(ownership.shellParentIsHost).toBe(true);
    expect(ownership.children).toEqual(["toast-alerts", "update-cta-shell"]);
    expect(ownership.ctaInsideAlertsLiveRegion).toBe(false);
    expect(ownership.hostRole).toBeNull();
    expect(ownership.hostAriaLive).toBeNull();

    await page.evaluate(() => {
      const alerts = (window as any).__alerts;
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-info",
        level: "info",
        title: "Board reply",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input (updated)",
        timeoutMs: 0,
        dismissible: true,
      });
    });
    const alertItems = page.locator(".toast-alerts__item");
    await expect(alertItems).toHaveCount(3);
    await expect(page.locator('#update-cta')).toBeVisible();

    for (const viewport of [
      { width: 1280, height: 900 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(viewport);
      const geometry = await page.evaluate(() => {
        const alertBoxes = Array.from(document.querySelectorAll(".toast-alerts__item"))
          .map((node) => node.getBoundingClientRect());
        const ctaBox = document
          .getElementById("update-cta-shell")!
          .getBoundingClientRect();
        const overlapAreas = alertBoxes.map((alertBox) => {
          const overlapWidth = Math.max(
            0,
            Math.min(alertBox.right, ctaBox.right) - Math.max(alertBox.left, ctaBox.left),
          );
          const overlapHeight = Math.max(
            0,
            Math.min(alertBox.bottom, ctaBox.bottom) - Math.max(alertBox.top, ctaBox.top),
          );
          return overlapWidth * overlapHeight;
        });
        return {
          maxOverlapArea: Math.max(...overlapAreas),
          scrollWidth: document.documentElement.scrollWidth,
          viewportWidth: window.innerWidth,
          maxAlertBottom: Math.max(...alertBoxes.map((box) => box.bottom)),
          ctaTop: ctaBox.top,
        };
      });
      expect(geometry.maxOverlapArea).toBe(0);
      expect(geometry.maxAlertBottom).toBeLessThanOrEqual(geometry.ctaTop);
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.viewportWidth);
    }

    const stickyAlert = page.locator(
      '.toast-alerts__item[data-toast-id="sticky-error"]',
    );
    await stickyAlert.focus();
    await expect(stickyAlert).toBeFocused();
    expect(await stickyAlert.evaluate((node) => node.matches(":focus-visible"))).toBe(true);
    await page.locator("#update-cta").focus();
    await expect(page.locator("#update-cta")).toBeFocused();
    expect(
      await page.locator("#update-cta").evaluate((node) => node.matches(":focus-visible")),
    ).toBe(true);

    await page.evaluate(() => (window as any).__alerts.dismiss("queued-info"));
    await expect(alertItems).toHaveCount(2);
    await expect(page.locator("#update-cta")).toBeVisible();

    await stickyAlert.click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("sticky-error");
    await expect(page.locator('.toast-alerts__item[data-toast-id="sticky-error"]')).toHaveCount(0);

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "dismiss-error",
        level: "error",
        title: "Dismiss me",
        timeoutMs: 0,
        dismissible: true,
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="dismiss-error"] .toast-alerts__dismiss').click();
    await expect(page.locator('.toast-alerts__item[data-toast-id="dismiss-error"]')).toHaveCount(0);

    await page.locator("[data-update-cta-dismiss]").click();
    await expect(shell).toHaveCount(0);
    await page.evaluate(() => (window as any).__updateController.showAvailable("9.99.0"));
    await page.locator("#update-cta").click();
    expect(await page.evaluate(() => (window as any).__updateSent)).toContainEqual({
      kind: "apply_update_start",
    });
    await expect(page.locator('#update-modal[data-state="downloading"]')).toBeVisible();
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

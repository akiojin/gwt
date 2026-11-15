import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Issue #4665: typed state notices, without treating inactivity as completion.
const browserErrors = new WeakMap<Page, string[]>();
const controllerTestTitle = "server desktop intents respect permission, dedupe transitions, and focus and close on click";

test.beforeEach(async ({ page }, testInfo) => {
  const errors: string[] = [];
  browserErrors.set(page, errors);
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript((theme) => {
    localStorage.setItem("gwt:ui:theme", theme);
  }, theme);
  await page.addInitScript(() => {
    const state = window as any;
    state.__notificationSent = [];
    state.__notificationInstances = [];
    state.__notificationPermissionRequests = 0;
    state.__notificationFocusCalls = 0;
    state.__notificationAttention = { visible: true, focused: false };
    document.hasFocus = () => state.__notificationAttention.focused;
    Object.defineProperty(document, "visibilityState", { get: () => state.__notificationAttention.visible ? "visible" : "hidden" });
    window.focus = () => { state.__notificationFocusCalls++; };
    class NotificationProbe {
      static permission = "default";
      static requestPermission() { state.__notificationPermissionRequests++; return Promise.resolve("granted"); }
      closed = false;
      onclick: (() => void) | null = null;
      constructor(public title: string, public options: any) { state.__notificationInstances.push(this); }
      close() { this.closed = true; }
    }
    Object.defineProperty(window, "Notification", { configurable: true, value: NotificationProbe });
    state.__notificationPermissionStatus = new EventTarget();
    Object.defineProperty(navigator, "permissions", { configurable: true, value: { query: async () => state.__notificationPermissionStatus } });
    class FixtureSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public readonly url: string) {
        super();
        if (new URL(url).searchParams.get("repo_hash")) state.__notificationSocket = this;
        setTimeout(() => {
          this.readyState = FixtureSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      emit(payload: any) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })); }
      send(raw: string) {
        const message = JSON.parse(raw);
        state.__notificationSent.push(message);
        if (message.kind !== "frontend_ready") return;
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({
          kind: "workspace_state",
          workspace: {
            app_version: "playwright",
            tabs: [{
              id: "notification-project", project_key: "0123456789abcdef", title: "Notification project",
              project_root: "/fixture", kind: "git",
              workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] },
            }],
            active_tab_id: "notification-project", recent_projects: [],
          },
        }) }));
      }
      close() { this.readyState = FixtureSocket.CLOSED; }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureSocket });
  });
  await installEmbeddedRoutes(page);
  await page.goto(APP_URL);
  await expect(page.locator("#op-notifications-button")).toBeVisible();
  await expect(page.locator("#close-project-button")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
});

test.afterEach(({ page }) => {
  expect(browserErrors.get(page), "console and page errors").toEqual([]);
});

test("windowless NeedsHuman uses one notification history entry, never the inbox snapshot", async ({ page }, testInfo) => {
  const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
  }, detail);
  await inject({
    kind: "issue_monitor_toast", level: "warn", issue_number: 4665,
    notification_transition: "needs_human", message: "Issue #4665 needs human attention.",
  });
  const bell = page.locator("#op-notifications-button");
  await expect(bell.locator(".op-rail__badge")).toHaveText("1");
  await bell.click();
  const rows = page.locator("#notification-center .notification-center__item");
  await expect(rows).toHaveCount(1);
  await expect(rows.first().locator(".notification-center__title")).toContainText("Needs human");
  await expect(rows.first()).toContainText("#4665");
  await expect(rows.first()).not.toContainText(/completed|finished/i);
  await inject({ kind: "issue_monitor_inbox", items: [{
    issue: { number: 4665, title: "Typed state notifications", labels: [], state: "open" },
    state: "needs_human", claim_id: null, blocked_by_owner: null,
    claim_expires_at: null, launched_window_id: null,
  }] });
  await inject({ kind: "project_notification", notification: {
    transition_id: "monitor:4665", project_key: "0123456789abcdef", window_id: null,
    category: "needs_human", title: "Needs human", body: "Issue #4665 needs human attention.",
  } });
  await expect(rows).toHaveCount(1);
  await expect(page.locator(".toast-alerts__item")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("needs-human-history.png"), fullPage: true });
});

test("Issue #4923: review launch failure appears once in notification history", async ({ page }, testInfo) => {
  const message = "independent review for PR #99 at abc123 failed (attempt 1/3): review window launch failure: Process exited with status 1";
  const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
  }, detail);
  await inject({ kind: "issue_monitor_toast", level: "warn", issue_number: 4923, message });
  await inject({ kind: "issue_monitor_inbox", items: [{
    issue: { number: 4923, title: "Review launch failure", labels: [], state: "open" },
    state: "launched", launched_window_id: "tab-1::implementation", error_message: message,
  }] });
  await page.locator("#op-notifications-button").click();
  const rows = page.locator("#notification-center .notification-center__item");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("launch failure");
  await expect(rows.first()).toContainText("PR #99");
  await expect(rows.first()).not.toContainText(/PTY handoff complete|never appeared|completed|finished/i);
  await page.screenshot({ path: testInfo.outputPath("review-launch-failure.png"), fullPage: true });
});

test("Issue #5016 AC-HS3: identical Issues notices aggregate into one counted row", async ({ page }, testInfo) => {
  const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
  }, detail);
  const steering = {
    kind: "issue_monitor_toast", level: "warn", issue_number: 4499,
    message: "Issue #4499 steering requested (1): resolve the review before closing",
  };
  await inject(steering);
  await inject({ ...steering, issue_number: 4500, message: "Issue #4500 steering requested (1): resolve the review before closing" });
  await inject(steering);
  await inject(steering);
  const bell = page.locator("#op-notifications-button");
  await expect(bell.locator(".op-rail__badge")).toHaveText("2");
  await bell.click();
  const rows = page.locator("#notification-center .notification-center__item");
  await expect(rows).toHaveCount(2);
  await expect(rows.first()).toContainText("#4499");
  await expect(rows.first().locator(".notification-center__count")).toHaveText("×3");
  await expect(rows.nth(1)).toContainText("#4500");
  await expect(rows.nth(1).locator(".notification-center__count")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("aggregated-history.png"), fullPage: true });
});

test(controllerTestTitle, async ({ page }, testInfo) => {
  const observed = await page.evaluate(() => {
    const state = window as any;
    const notify = (transitionId: string, overrides: Record<string, unknown> = {}) => state.__notificationSocket.emit({
      kind: "project_notification", notification: {
        transition_id: transitionId, project_key: "0123456789abcdef", window_id: null,
        category: "stopped", title: "Agent stopped", body: "Fixture agent stopped in Notification project.", ...overrides,
      },
    });
    for (const permission of ["default", "denied"]) {
      (Notification as any).permission = permission;
      notify(permission);
    }
    (Notification as any).permission = "granted";
    state.__notificationAttention = { visible: false, focused: false };
    document.dispatchEvent(new Event("visibilitychange"));
    // Hidden tabs can suspend rendering frames. Desktop intents must still run immediately.
    state.__notificationRestoreRaf = window.requestAnimationFrame;
    window.requestAnimationFrame = () => 0;
    notify("wrong-project", { project_key: "fedcba9876543210" });
    notify("idle", { category: "idle" });
    notify("waiting", { category: "waiting" });
    notify("granted");
    notify("granted");
    const notice = state.__notificationInstances[0];
    notice?.onclick();
    window.requestAnimationFrame = state.__notificationRestoreRaf;
    return {
      titles: state.__notificationInstances.map((item: any) => item.title),
      body: notice?.options.body, closed: notice?.closed,
      focusCalls: state.__notificationFocusCalls, permissionRequests: state.__notificationPermissionRequests,
      initialAttention: state.__notificationSent.find((event: any) => event.kind === "project_client_attention"),
      latestAttention: state.__notificationSent.filter((event: any) => event.kind === "project_client_attention").at(-1),
    };
  });
  expect(observed).toEqual({
    titles: ["Agent stopped"], body: "Fixture agent stopped in Notification project.", closed: true,
    focusCalls: 1, permissionRequests: 0,
    initialAttention: { kind: "project_client_attention", visible: true, focused: false, notification_permission: "default" },
    latestAttention: { kind: "project_client_attention", visible: false, focused: false, notification_permission: "granted" },
  });
  await expect(page.locator(".toast-alerts__item")).toHaveCount(0);
  await page.locator("#op-notifications-button").click();
  await expect(page.locator("#notification-center .notification-center__item")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("server-notification-intent.png"), fullPage: true });
});

// SPEC #6476 T-005nc (AC-1): on a phone the drawer kept its desktop sizes
// (dismiss 20px, close 28px, Clear all 24px).
test.describe("Notification center touch targets", () => {
  test.use({ hasTouch: true });
  test("drawer controls are 44px tap targets on a phone", async ({ page }, testInfo) => {
    const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }, detail);
    await inject({
      kind: "issue_monitor_toast", level: "warn", issue_number: 4665,
      notification_transition: "needs_human", message: "Issue #4665 needs human attention.",
    });
    await inject({ kind: "window_command_error", id: "tab-1::agent", message: "spawn failed: permission denied" });
    await page.setViewportSize({ width: 601, height: 900 });
    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    await expect(rows).toHaveCount(2);
    const controls = "#notification-center :is(button, summary):visible";
    const measure = () => page.locator(controls).evaluateAll(nodes => nodes.map(node => {
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return { name: node.getAttribute("aria-label") || node.textContent!.trim().slice(0, 24) || node.className,
        left: box.left, right: box.right, width: box.width, height: box.height,
        minSize: `${style.minWidth} ${style.minHeight}` };
    }));
    const desktop = (await measure()).map(control => control.minSize);
    expect(desktop.join(" "), "601px keeps the desktop control sizes").not.toContain("44px");
    for (const [width, height] of [[320, 568], [390, 844]]) {
      await page.setViewportSize({ width, height });
      await expect(async () => {
        const measured = await measure();
        expect(measured.map(control => control.name)).toEqual(expect.arrayContaining(
          ["Clear all", "Close notifications", "Dismiss notification", "Details"]));
        for (const control of measured) {
          expect(control.height, `${control.name} height at ${width}px`).toBeGreaterThanOrEqual(44);
          if (control.name !== "Details") {
            expect(control.width, `${control.name} width at ${width}px`).toBeGreaterThanOrEqual(44);
          }
          expect(control.left, `${control.name} is on screen at ${width}px`).toBeGreaterThanOrEqual(0);
          expect(control.right, `${control.name} is on screen at ${width}px`).toBeLessThanOrEqual(width);
        }
        for (const target of [drawer, drawer.locator(".notification-center__body")]) {
          expect(await target.evaluate(node => node.scrollWidth - node.clientWidth),
            `the drawer must not scroll sideways at ${width}px`).toBeLessThanOrEqual(0);
        }
        expect(await drawer.locator(".notification-center__details > summary")
          .evaluate(node => getComputedStyle(node).display), "Details keeps its disclosure marker").toBe("list-item");
      }).toPass({ timeout: 5_000 });
      await page.screenshot({ path: testInfo.outputPath(`notifications-${width}.png`) });
    }
    await rows.filter({ hasText: "#4665" }).getByRole("button", { name: "Dismiss notification" }).tap();
    await expect(rows).toHaveCount(1);
    await page.setViewportSize({ width: 601, height: 900 });
    await expect.poll(async () => (await measure()).map(control => control.minSize).join(" "),
      { message: "601px returns to the desktop control sizes" }).not.toContain("44px");
  });
});

// SPEC #6476 T-005md (AC-1): the shared modal primitive kept its desktop sizes
// on a phone (Cancel text button 19px, footer buttons 36px).
test.describe("Shared modal touch targets", () => {
  test.use({ hasTouch: true });
  test("close project and migration modals use 44px tap targets on a phone", async ({ page }, testInfo) => {
    const inject = (detail: Record<string, unknown>) => page.evaluate((payload) => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }, detail);
    const openModal = async (name: string) => {
      if (name === "close") {
        await page.locator("#close-project-button").click();
        await inject({
          kind: "close_project_preview", token: { project_key: "0123456789abcdef" }, title: "Notification project",
          running_agents: [{ display_name: "Aster", branch: "work/issue-1" }], choice_required: true, project_wide: false, view_count: 1,
        });
      } else {
        await inject({ kind: "migration_detected", tab_id: "notification-project", project_root: "/fixture", branch: "develop" });
      }
      await expect(page.locator(".modal-backdrop.open .modal-shell")).toBeVisible();
    };
    const controls = ".modal-backdrop.open .modal-shell :is(button, select, textarea, summary, input):visible";
    const measure = () => page.locator(controls).evaluateAll(nodes => nodes.map(node => {
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return { name: node.getAttribute("aria-label") || node.textContent!.trim().slice(0, 24) || node.className,
        left: box.left, right: box.right, width: box.width, height: box.height,
        minSize: `${style.minWidth} ${style.minHeight}` };
    }));
    for (const name of ["close", "migration"]) {
      await page.setViewportSize({ width: 601, height: 900 });
      await openModal(name);
      const desktop = (await measure()).map(control => control.minSize);
      expect(desktop.join(" "), `601px keeps the ${name} control sizes`).not.toContain("44px");
      for (const [width, height] of [[320, 568], [390, 844]]) {
        await page.setViewportSize({ width, height });
        await expect(async () => {
          const measured = await measure();
          expect(measured.length, `${name} has controls`).toBeGreaterThan(0);
          for (const control of measured) {
            expect(control.height, `${name} ${control.name} height at ${width}px`).toBeGreaterThanOrEqual(44);
            expect(control.width, `${name} ${control.name} width at ${width}px`).toBeGreaterThanOrEqual(44);
            expect(control.left, `${name} ${control.name} is on screen at ${width}px`).toBeGreaterThanOrEqual(0);
            expect(control.right, `${name} ${control.name} is on screen at ${width}px`).toBeLessThanOrEqual(width);
          }
          expect(await page.locator(".modal-backdrop.open .modal-shell").evaluate(node => node.scrollWidth - node.clientWidth),
            `the ${name} modal must not scroll sideways at ${width}px`).toBeLessThanOrEqual(0);
        }).toPass({ timeout: 5_000 });
        await page.screenshot({ path: testInfo.outputPath(`modal-${name}-${width}.png`) });
      }
      if (name === "close") {
        await page.getByRole("button", { name: "Cancel" }).tap();
        await expect(page.locator(".modal-backdrop.open .modal-shell")).toHaveCount(0);
      }
    }
    await page.setViewportSize({ width: 601, height: 900 });
    await expect.poll(async () => (await measure()).map(control => control.minSize).join(" "),
      { message: "601px returns to the desktop control sizes" }).not.toContain("44px");
  });
});

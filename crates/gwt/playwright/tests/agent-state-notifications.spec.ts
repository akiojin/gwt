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

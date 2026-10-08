import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Issue #4665: typed state notices, without treating inactivity as completion.
const browserErrors = new WeakMap<Page, string[]>();
const controllerTestTitle = "sustained Stopped says stopped; Idle and ungranted permission cannot produce completion or desktop notices";

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
  // The controller case uses assets served by the isolated checkout binary
  // when supplied. Only clock, permission, and output rendering are injected;
  // no source route or WebSocket is replaced on this path.
  const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
  if (testInfo.title === controllerTestTitle && liveBase) {
    const response = await page.goto(new URL("/", liveBase).href);
    expect(response?.ok(), "isolated checkout Hub response").toBe(true);
    await expect(page.locator(".gwt-hub")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    return;
  }
  // The Monitor wiring case always uses deterministic backend events. The
  // controller case also uses this source fixture in CI without a live URL.
  await page.addInitScript(() => {
    class FixtureSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public readonly url: string) {
        super();
        setTimeout(() => {
          this.readyState = FixtureSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        if (JSON.parse(raw).kind !== "frontend_ready") return;
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({
          kind: "workspace_state",
          workspace: {
            app_version: "playwright",
            tabs: [{
              id: "notification-project", title: "Notification project",
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
  await expect(rows).toHaveCount(1);
  await expect(page.locator(".toast-alerts__item")).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath("needs-human-history.png"), fullPage: true });
});

test(controllerTestTitle, async ({ page }, testInfo) => {
  const observed = await page.evaluate(async () => {
    const { createAgentCompletionNotifier, createAgentAttentionToaster } =
      await import("/agent-completion-notifications.js");
    const { createToastStack } = await import("/toast-host.js");
    const host = document.createElement("div");
    host.className = "operator-notice-stack";
    document.body.append(host);
    const stack = createToastStack({ document, className: "toast-alerts", ariaRole: "status", ariaLive: "polite" });
    stack.mount(host);
    let timestamp = 0;
    let desktopCount = 0;
    let permissionRequests = 0;
    class NotificationProbe {
      static permission = "default";
      static requestPermission() { permissionRequests++; return Promise.resolve("granted"); }
      constructor() { desktopCount++; }
    }
    const titles: string[] = [];
    const notifier = createAgentCompletionNotifier({
      window: { Notification: NotificationProbe },
      now: () => timestamp, isAttentionAway: () => true,
      showToast: (notice: { windowId: string; title: string; body: string }) => {
        titles.push(notice.title);
        stack.push({ id: notice.windowId, title: notice.title, message: notice.body, timeoutMs: 0 });
      },
    });
    const state = (windowId: string, runtimeState: string) => notifier.handleRuntimeState({
      windowId, runtimeState, windowData: { title: "Fixture agent" },
      projectTab: { id: "notification-project", title: "Notification project" },
    });
    state("idle", "running");
    timestamp += 300_000;
    const idle = state("idle", "idle");
    const attentionNotices: string[] = [];
    const attention = createAgentAttentionToaster({ showToast: (notice: { title: string }) => attentionNotices.push(notice.title) });
    attention.handleRuntimeState({ windowId: "idle", runtimeState: "idle" });
    state("short", "running");
    timestamp += 299_999;
    const short = state("short", "stopped");
    for (const permission of ["default", "denied"]) {
      NotificationProbe.permission = permission;
      state(permission, "running");
      timestamp += 300_000;
      state(permission, "stopped");
      state(permission, "stopped"); // Duplicate state frames stay quiet.
    }
    return { idle, short, attentionNotices, titles, desktopCount, permissionRequests };
  });
  expect(observed).toEqual({
    idle: null, short: null, attentionNotices: [],
    titles: ["Agent stopped", "Agent stopped"], desktopCount: 0, permissionRequests: 0,
  });
  const notices = page.locator(".toast-alerts__item");
  await expect(notices).toHaveCount(2);
  await expect(notices.first()).toContainText("Agent stopped");
  await expect(notices.first()).toContainText("Fixture agent stopped in Notification project.");
  await expect(notices.first()).not.toContainText(/completed|finished/i);
  await page.screenshot({ path: testInfo.outputPath("agent-stopped-notices.png"), fullPage: true });
});

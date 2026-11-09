// Uses browser-check's checkout binary and fresh HOME. Browser notification
// construction is probed; this never claims that the host OS displayed a notice.
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test("live Project reports ephemeral capability and consumes Interrupted desktop intents without duplicate history", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires browser-check isolated checkout");
  test.setTimeout(90_000);
  const errors: string[] = [];
  const sent: any[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  page.on("websocket", (socket) => socket.on("framesent", ({ payload }) => {
    try { sent.push(JSON.parse(String(payload))); } catch { /* non-JSON frame */ }
  }));
  await page.addInitScript(() => {
    const state = window as any;
    state.__desktopIntents = [];
    state.__desktopFocus = 0;
    state.__permissionRequests = 0;
    window.focus = () => { state.__desktopFocus++; };
    class NotificationProbe {
      static permission = "default";
      static requestPermission() { state.__permissionRequests++; }
      closed = false;
      onclick: (() => void) | null = null;
      constructor(public title: string, public options: any) { state.__desktopIntents.push(this); }
      close() { this.closed = true; }
    }
    Object.defineProperty(window, "Notification", { configurable: true, value: NotificationProbe });
  });
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    await expect.poll(() => sent.filter((event) => event.kind === "project_client_attention").length).toBeGreaterThan(0);
    expect(sent.find((event) => event.kind === "project_client_attention").notification_permission).toBe("default");
    // The real server must accept the new report without losing its Project scope.
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });
    const key = new URL(page.url()).pathname.split("/")[2];
    expect(key).toMatch(/^[a-f0-9]{16}$/);
    // Desktop intents must not enter history; unrelated capacity diagnostics may
    // arrive asynchronously while this test runs.
    await page.locator("#op-notifications-button").click();
    const history = page.locator("#notification-center .notification-center__item");
    const result = await page.evaluate((projectKey) => {
      const state = window as any;
      const emit = (id: string, key = projectKey, category = "needs_human") => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "project_notification", notification: {
          transition_id: id, project_key: key, window_id: null,
          category, title: category === "interrupted" ? "gwt — Agent interrupted" : "Needs human",
          body: category === "interrupted"
            ? "An agent was interrupted after running for at least five minutes."
            : "A monitored issue needs human attention.",
        } },
      }));
      emit("default", projectKey, "interrupted");
      (Notification as any).permission = "granted";
      window.dispatchEvent(new Event("blur"));
      emit("foreign", "0000000000000000", "interrupted");
      emit("granted"); emit("granted");
      emit("interrupted", projectKey, "interrupted"); emit("interrupted", projectKey, "interrupted");
      const notice = state.__desktopIntents[0];
      notice?.onclick();
      (Notification as any).permission = "denied";
      window.dispatchEvent(new Event("focus"));
      emit("denied", projectKey, "interrupted");
      return { titles: state.__desktopIntents.map((item: any) => item.title),
        interruptedBody: state.__desktopIntents[1]?.options.body, closed: notice?.closed,
        focus: state.__desktopFocus, requests: state.__permissionRequests };
    }, key);
    expect(result).toEqual({ titles: ["Needs human", "gwt — Agent interrupted"],
      interruptedBody: "An agent was interrupted after running for at least five minutes.",
      closed: true, focus: 1, requests: 0 });
    await expect.poll(() => sent.filter((event) => event.kind === "project_client_attention").at(-1)?.notification_permission).toBe("denied");
    const noticeText = /Needs human|Agent interrupted|A monitored issue needs human attention\.|An agent was interrupted after running for at least five minutes\./i;
    await expect(history.filter({ hasText: noticeText })).toHaveCount(0);
    await expect(page.locator(".toast-alerts__item").filter({ hasText: noticeText })).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("live-project-notifications.png") });
    expect(errors).toEqual([]);
  });
});

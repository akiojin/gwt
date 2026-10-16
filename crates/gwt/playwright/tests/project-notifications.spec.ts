// Uses browser-check's checkout binary and fresh HOME. Browser notification
// construction is probed; this never claims that the host OS displayed a notice.
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test("live Project reports ephemeral capability and consumes desktop intents without duplicate history", async ({ page }, testInfo) => {
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
    // The stopped-Agent metadata fixture has its existing in-app attention row.
    // Desktop intents must preserve that history without adding another row.
    await page.locator("#op-notifications-button").click();
    const history = page.locator("#notification-center .notification-center__item");
    await expect(history).toHaveCount(1);
    await expect(history.first()).toContainText("Metadata fixture");
    const result = await page.evaluate((projectKey) => {
      const state = window as any;
      const emit = (id: string, key = projectKey) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "project_notification", notification: {
          transition_id: id, project_key: key, window_id: null,
          category: "needs_human", title: "Needs human", body: "A monitored issue needs human attention.",
        } },
      }));
      emit("default");
      (Notification as any).permission = "granted";
      window.dispatchEvent(new Event("blur"));
      emit("foreign", "0000000000000000");
      emit("granted"); emit("granted");
      const notice = state.__desktopIntents[0];
      notice?.onclick();
      (Notification as any).permission = "denied";
      window.dispatchEvent(new Event("focus"));
      emit("denied");
      return { count: state.__desktopIntents.length, closed: notice?.closed,
        focus: state.__desktopFocus, requests: state.__permissionRequests };
    }, key);
    expect(result).toEqual({ count: 1, closed: true, focus: 1, requests: 0 });
    await expect.poll(() => sent.filter((event) => event.kind === "project_client_attention").at(-1)?.notification_permission).toBe("denied");
    await expect(history).toHaveCount(1);
    await expect(history.first()).toContainText("Metadata fixture");
    await page.screenshot({ path: testInfo.outputPath("live-project-notifications.png") });
    expect(errors).toEqual([]);
  });
});

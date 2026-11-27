import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { liveGwtRouteUrl } from "./_helpers/live-gwt";

// SPEC #6943 Phase 1: the workspace state notice is the first React surface.
// It mounts as an island inside the unmigrated shell, renders from the shared
// UI state store, and loads nothing from outside the embedded origin.

declare global {
  interface Window {
    __gwtReactNoticeSocket?: { emit(payload: unknown): void; sent: unknown[] };
  }
}

async function installNoticeBackend(page: any) {
  await page.addInitScript(() => {
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      sent: unknown[] = [];
      readyState = 0;
      url: string;

      constructor(url: string) {
        super();
        this.url = url;
        (window as any).__gwtReactNoticeSocket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(data: string) {
        try { this.sent.push(JSON.parse(data)); } catch { this.sent.push(data); }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
        setTimeout(() => {
          this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
        }, 0);
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  });
}

test.describe("React workspace state notice", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });

  test("renders from the store, retries, clears, and loads only same-origin assets", async ({ page }, testInfo) => {
    const errors: string[] = [];
    const foreignRequests: string[] = [];
    const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
    const appUrl = liveBase ? await liveGwtRouteUrl(page, liveBase) : APP_URL;
    const origin = new URL(appUrl).origin;
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("request", request => {
      const url = new URL(request.url());
      if (["http:", "https:"].includes(url.protocol) && url.origin !== origin) foreignRequests.push(request.url());
    });
    if (!liveBase) await installEmbeddedRoutes(page);
    await installNoticeBackend(page);
    const islandResponse = page.waitForResponse(response => response.url().endsWith("/react/islands.js"));
    await page.goto(appUrl);
    expect((await islandResponse).status()).toBe(200);
    await page.waitForFunction(() => Boolean(window.__gwtReactNoticeSocket), undefined, { timeout: 10_000 });
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    await expect(page.locator("#app > .react-island")).toHaveCount(1);

    const notice = { kind: "load_error", path: "/state/<img src=x>/current.json", message: "<b>bad JSON</b>" };
    await page.evaluate(payload => window.__gwtReactNoticeSocket!.emit({ kind: "workspace_state_notice", notice: payload }), notice);
    const banner = page.locator("#app > .react-island > .workspace-state-notice[role='alert']");
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await expect(banner).toContainText(`${notice.path}: ${notice.message}`);
    await expect(banner.locator("img, b")).toHaveCount(0);

    // The island is display: contents, so the banner keeps its full-width
    // grid area in the existing shell.
    const appBox = await page.locator("#app").boundingBox();
    const bannerBox = await banner.boundingBox();
    expect(bannerBox!.width).toBeGreaterThan(appBox!.width * 0.9);

    await banner.getByRole("button", { name: "Retry" }).click();
    await expect.poll(() => page.evaluate(() => window.__gwtReactNoticeSocket!.sent))
      .toContainEqual({ kind: "retry_workspace_state_load" });
    await expect(banner).toBeVisible();

    await page.evaluate(() => window.__gwtReactNoticeSocket!.emit({ kind: "workspace_state_notice", notice: null }));
    await expect(page.locator(".workspace-state-notice")).toHaveCount(0, { timeout: 10_000 });

    expect(foreignRequests).toEqual([]);
    expect(errors).toEqual([]);
  });
});

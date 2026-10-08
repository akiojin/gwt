/* Issue #4540 AC-2 — project metadata and server-owned unread in both themes. */
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";
import { APP_URL, HUB_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

async function installMetadataBackend(page: Page) {
  await page.addInitScript(() => {
    const state = window as any;
    state.__metadataSent = [];
    // Playwright forces each page focused even with another tab foregrounded.
    // Control attention inputs in the backend fixture; rendering is real Chromium.
    state.__metadataAttention = { focused: true, visible: true };
    document.hasFocus = () => state.__metadataAttention.focused;
    Object.defineProperty(document, "visibilityState", { get: () => state.__metadataAttention.visible ? "visible" : "hidden" });
    state.__metadataPermissionRequests = 0;
    Object.defineProperty(window, "Notification", { configurable: true, value: class {
      static permission = "denied";
      static requestPermission() { state.__metadataPermissionRequests++; }
    } });
    const project = { id: "tab-a", project_key: "0123456789abcdef", title: "Alpha", kind: "git" };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      url: string;
      projectKey: string | null;
      aggregate = { running_count: 0, block_count: 0, error_count: 0, unread: false, revision: 1 };
      constructor(url: string) {
        super();
        this.url = url;
        this.projectKey = new URL(url).searchParams.get("repo_hash");
        if (this.projectKey) state.__metadataBackend = this;
        setTimeout(() => { this.readyState = 1; this.dispatchEvent(new Event("open")); }, 0);
      }
      emit(event: any) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) })); }
      update(aggregate: any) { this.aggregate = aggregate; this.emit({ kind: "project_agent_aggregate", aggregate }); }
      send(raw: string) {
        const message = JSON.parse(raw);
        state.__metadataSent.push(message);
        if (message.kind === "frontend_ready") {
          setTimeout(() => {
            if (!this.projectKey) {
              this.emit({ kind: "hub_state", hub: { app_version: "e2e", projects: [project], recent_projects: [] } });
              return;
            }
            this.emit({ kind: "workspace_state", workspace: { app_version: "e2e", tabs: [{ ...project, project_root: "/fixture", workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] } }], active_tab_id: project.id, recent_projects: [] } });
            this.update(this.aggregate);
          }, 0);
        }
        if (message.kind === "project_aggregate_ack" && message.visible && message.focused && message.revision === this.aggregate.revision && this.aggregate.unread) {
          setTimeout(() => this.update({ ...this.aggregate, unread: false, revision: this.aggregate.revision + 1 }), 0);
        }
      }
      close() { this.readyState = 3; this.dispatchEvent(new CloseEvent("close")); }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  });
}

function collectErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

test("aggregate updates hidden project title/favicon, focus acknowledges unread, Hub stays fixed", async ({ page, context }, testInfo) => {
  const errors = collectErrors(page);
  await installEmbeddedRoutes(page);
  await installMetadataBackend(page);
  await page.goto(APP_URL);
  await expect(page).toHaveTitle("Alpha — RUN 0 · BLOCK 0 — gwt");
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const hub = await context.newPage();
  const hubErrors = collectErrors(hub);
  await installEmbeddedRoutes(hub);
  await installMetadataBackend(hub);
  await hub.goto(HUB_URL);
  await hub.bringToFront();
  await page.evaluate(() => {
    // Hold every newly scheduled animation frame, as background tabs do.
    // The existing websocket dispatcher also resolves rAF at dispatch time.
    (window as any).__metadataRestoreRaf = window.requestAnimationFrame;
    window.requestAnimationFrame = () => 0;
    (window as any).__metadataAttention = { focused: false, visible: false };
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await page.evaluate(() => (window as any).__metadataBackend.update({ running_count: 2, block_count: 1, error_count: 1, unread: true, revision: 5 }));
  await expect(page).toHaveTitle("● Alpha — RUN 2 · BLOCK 1 — gwt");
  const icon = page.locator('link[rel="icon"]');
  await expect(icon).toHaveAttribute("data-state", "error");
  await expect(icon).toHaveAttribute("data-unread", "true");
  expect(await page.evaluate(() => {
    const svg = decodeURIComponent(document.querySelector<HTMLLinkElement>('link[rel="icon"]')!.href);
    return svg.includes(getComputedStyle(document.documentElement).getPropertyValue("--color-state-blocked").trim());
  })).toBe(true);
  await expect(hub).toHaveTitle("gwt — Hub");
  await expect(hub.locator('link[rel="icon"]')).toHaveAttribute("href", "data:,");
  expect(await page.evaluate(() => (window as any).__metadataSent.filter((event: any) => event.kind === "project_aggregate_ack"))).toEqual([]);
  await page.bringToFront();
  await page.evaluate(() => {
    (window as any).__metadataAttention = { focused: true, visible: true };
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page).toHaveTitle("Alpha — RUN 2 · BLOCK 1 — gwt");
  await expect(icon).toHaveAttribute("data-unread", "false");
  await page.evaluate(() => (window as any).__metadataBackend.update({ running_count: 1, block_count: 0, error_count: 0, unread: false, revision: 7 }));
  await expect(icon).toHaveAttribute("data-state", "running");
  await expect(page).toHaveTitle("Alpha — RUN 1 · BLOCK 0 — gwt");
  expect(await page.evaluate(() => (window as any).__metadataPermissionRequests)).toBe(0);
  await page.evaluate(() => { window.requestAnimationFrame = (window as any).__metadataRestoreRaf; });
  await page.screenshot({ path: testInfo.outputPath(`metadata-${theme}.png`) });
  expect([...errors, ...hubErrors]).toEqual([]);
  await hub.close();
});


test("live server projects the seeded agent aggregate and keeps Hub metadata fixed", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires browser-check isolated checkout server");
  const errors = collectErrors(page);
  const aggregates: any[] = [];
  page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
    try {
      const event = JSON.parse(String(payload));
      if (event.kind === "project_agent_aggregate") aggregates.push(event.aggregate);
    } catch { /* unrelated websocket payload */ }
  }));
  await gotoLiveGwt(page, base!, { projectKey: process.env.GWT_PLAYWRIGHT_PROJECT_KEY || "99a8660247f5bc49" });
  await expect.poll(() => aggregates.length).toBeGreaterThan(0);
  expect(aggregates.at(-1)).toMatchObject({ running_count: 0, block_count: 1 });
  await expect(page).toHaveTitle("Issue 4540 Check — RUN 0 · BLOCK 1 — gwt");
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute("data-state", "blocked");
  await page.screenshot({ path: testInfo.outputPath("live-project-metadata.png") });
  await gotoLiveGwt(page, base!, { hub: true });
  await expect(page).toHaveTitle("gwt — Hub");
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute("href", "data:,");
  expect(errors).toEqual([]);
});

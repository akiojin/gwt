/* Issue #4538 AC-6 — Project-per-Browser-Tab against a live isolated gwt.
 *
 * One browser tab is one Project URL (`/p/<repo-hash>`):
 * - A/B: a mutation in Project A never reaches a tab bound to Project B.
 * - A/A: two tabs on Project A mirror the same state.
 * - reconnect: a dropped Project socket rebinds from the URL and only that
 *   client receives the full sync.
 * - URL stability: workspace actions and cross-project navigation never
 *   change a Project tab's URL; the Hub and other Projects open new tabs.
 * - routing: `/` is the picker, an unknown hash answers not found, and a
 *   known Recent Project auto-opens on direct access.
 *
 * Requires the browser-check isolated checkout server seeded with at least
 * two Projects (GWT_PLAYWRIGHT_BASE_URL). Runs in both Operator themes.
 */
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import {
  gotoLiveGwt,
  liveGwtProjectUrl,
  readLiveHubCatalog,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

type Frame = { kind: string; scope: string | null; payload: any };

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

function recordFrames(page: Page): Frame[] {
  const frames: Frame[] = [];
  page.on("websocket", (socket) => {
    const scope = new URL(socket.url()).searchParams.get("repo_hash");
    socket.on("framereceived", ({ payload }) => {
      try {
        const event = JSON.parse(String(payload));
        frames.push({ kind: event.kind, scope, payload: event });
      } catch {
        /* non-JSON frames are irrelevant */
      }
    });
  });
  return frames;
}

async function createLiveShell(page: Page): Promise<string> {
  const shells = page.locator('.workspace-window[data-preset="shell"]');
  const ids = async () => shells.evaluateAll((nodes) =>
    nodes.map((node) => (node as HTMLElement).dataset.id!));
  const before = new Set(await ids());
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset: "shell",
    bounds: { x: 80, y: 80, width: 640, height: 380 },
  });
  const created = async () => (await ids()).filter((id) => !before.has(id));
  await expect.poll(created, { timeout: 30_000 }).toHaveLength(1);
  return (await created())[0];
}

async function expectTheme(page: Page, theme: string) {
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

test.describe("Project-per-Browser-Tab (live)", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("A/B isolation, A/A mirror, scoped reconnect, and URL stability", async ({ page, context }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base, "requires browser-check isolated checkout server with two projects");
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await withLiveGwtBackendLock(base!, testInfo, async () => {
      const catalog = await readLiveHubCatalog(page, base!);
      expect(catalog.projects.length, "isolated session must seed projects A and B").toBeGreaterThanOrEqual(2);
      const [projectA, projectB] = catalog.projects;
      const other = await context.newPage();
      const mirror = await context.newPage();
      const pages = [page, other, mirror];
      const keys = [projectA.project_key, projectB.project_key, projectA.project_key];
      const errors = pages.map(collectBrowserErrors);
      const frames = pages.map(recordFrames);
      const created: Array<{ page: Page; id: string }> = [];
      try {
        for (const [index, current] of pages.entries()) {
          await gotoLiveGwt(current, base!, { enableTestBridge: true, projectKey: keys[index] });
          await expect.poll(() => frames[index].some((frame) =>
            frame.kind === "workspace_state" && frame.scope === keys[index])).toBe(true);
          await expectTheme(current, theme);
          expect(current.url()).toBe(liveGwtProjectUrl(base!, keys[index]));
        }

        // A/B isolation and A/A mirror.
        const aWindow = await createLiveShell(page);
        created.push({ page, id: aWindow });
        await expect(mirror.locator(`.workspace-window[data-id="${aWindow}"]`)).toBeVisible();
        await expect(other.locator(`.workspace-window[data-id="${aWindow}"]`)).toHaveCount(0);
        const bWindow = await createLiveShell(other);
        created.push({ page: other, id: bWindow });
        await expect(page.locator(`.workspace-window[data-id="${bWindow}"]`)).toHaveCount(0);
        await expect(mirror.locator(`.workspace-window[data-id="${bWindow}"]`)).toHaveCount(0);
        for (const [index, received] of frames.entries()) {
          const snapshots = received.filter((frame) => frame.kind === "workspace_state");
          expect(snapshots.every((frame) => frame.scope === keys[index]
            && frame.payload.workspace.tabs.length === 1
            && frame.payload.workspace.tabs[0].project_key === keys[index])).toBe(true);
        }
        await page.screenshot({ path: testInfo.outputPath(`project-a-${theme}.png`) });
        await other.screenshot({ path: testInfo.outputPath(`project-b-${theme}.png`) });

        // Reconnect: only the reconnecting client receives its full sync.
        // A Project full sync is a client-only reply that starts with the Hub
        // catalog; Project broadcasts never carry `hub_state`, so a
        // `hub_state` frame on a Project socket marks exactly one full sync.
        const fullSyncs = () => frames.map((received) =>
          received.filter((frame) => frame.kind === "hub_state" && frame.scope).length);
        const before = fullSyncs();
        await page.evaluate(() => {
          for (const socket of (window as any).__gwtPlaywrightSockets ?? []) {
            if (socket.url.includes("repo_hash=") && socket.readyState === WebSocket.OPEN) socket.close();
          }
        });
        await expect.poll(() => fullSyncs()[0], { timeout: 15_000 }).toBe(before[0] + 1);
        await expect.poll(() => frames[0].filter((frame) => frame.kind === "workspace_state").at(-1)?.scope)
          .toBe(projectA.project_key);
        expect(fullSyncs().slice(1)).toEqual(before.slice(1));
        await expect(page.locator(`.workspace-window[data-id="${aWindow}"]`)).toBeVisible();

        // URL stability: workspace actions keep the URL; another Project and
        // the Hub open in new browser tabs.
        await sendLiveGwtEvent(page, { kind: "close_window", id: aWindow });
        created.shift();
        await expect(mirror.locator(`.workspace-window[data-id="${aWindow}"]`)).toHaveCount(0);
        expect(page.url()).toBe(liveGwtProjectUrl(base!, projectA.project_key));
        const projectPopup = page.waitForEvent("popup");
        await page.locator(`[data-project-tab-id="${projectB.id}"]`).click();
        const projectTab = await projectPopup;
        await expect.poll(() => projectTab.url()).toBe(liveGwtProjectUrl(base!, projectB.project_key));
        await projectTab.close();
        const hubPopup = page.waitForEvent("popup");
        await page.locator("#project-home-link").click();
        const hubTab = await hubPopup;
        await expect(hubTab.locator("[data-hub]")).toBeVisible();
        await hubTab.close();
        expect(page.url()).toBe(liveGwtProjectUrl(base!, projectA.project_key));
      } finally {
        for (const window of created) {
          await sendLiveGwtEvent(window.page, { kind: "close_window", id: window.id }).catch(() => {});
        }
        await other.close();
        await mirror.close();
      }
      expect(errors.flat()).toEqual([]);
    });
  });

  test("Hub picker, not-found routes, and Recent auto-open", async ({ page, context }, testInfo) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base, "requires browser-check isolated checkout server");
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await withLiveGwtBackendLock(base!, testInfo, async () => {
      const errors = collectBrowserErrors(page);
      const recentRoot = await mkdtemp(join(tmpdir(), "gwt-4538-recent-"));
      let recentTabId: string | undefined;
      try {
        // Hub: picker only, path-free new-tab links.
        await gotoLiveGwt(page, base!, { hub: true });
        const hub = page.locator("[data-hub]");
        await expect(hub).toBeVisible();
        await expectTheme(page, theme);
        await expect(page).toHaveTitle("gwt — Hub");
        for (const selector of ["#app", "#project-tabs", "#op-rail", ".workspace-window"]) {
          await expect(page.locator(selector)).toHaveCount(0);
        }
        await expect(page.locator('[data-hub-action="open-folder"]')).toBeVisible();
        await expect(page.locator('[data-hub-action="clone"]')).toBeVisible();
        const links = hub.locator('[data-hub-list="open"] a');
        await expect(links.first()).toBeVisible();
        for (const link of await links.all()) {
          await expect(link).toHaveAttribute("href", /^\/p\/[0-9a-f]{16}$/);
          await expect(link).toHaveAttribute("target", "_blank");
          await expect(link).toHaveAttribute("rel", "noopener");
        }
        await page.screenshot({ path: testInfo.outputPath(`hub-${theme}.png`) });
        const popup = page.waitForEvent("popup");
        await links.first().click();
        const opened = await popup;
        await expect(opened.locator(".project-tab[aria-current='page']")).toBeVisible();
        expect(new URL(opened.url()).pathname).toMatch(/^\/p\/[0-9a-f]{16}$/);
        await opened.close();

        // Invalid hash: HTTP 404, path-free.
        const invalid = await page.request.get(new URL("/p/zzz", base!).toString());
        expect(invalid.status()).toBe(404);
        expect(await invalid.text()).toContain("Project not found");

        // Valid but unknown hash: deterministic not-found view.
        await page.goto(liveGwtProjectUrl(base!, "cccccccccccccccc"));
        await expect(page.locator("[data-route-not-found]")).toBeVisible();
        await expect(page.locator("[data-route-not-found] a")).toHaveAttribute("href", "/");

        // Known Recent, currently closed: direct access auto-opens it.
        const withRecent = await readLiveHubCatalog(page, base!, {
          send: { kind: "reopen_recent_project", path: recentRoot },
          until: (catalog) => catalog.recent_projects.some((entry) => entry.path.includes("gwt-4538-recent-")),
        });
        const recent = withRecent.recent_projects.find((entry) => entry.path.includes("gwt-4538-recent-"))!;
        const openEntry = await readLiveHubCatalog(page, base!, {
          until: (catalog, key) => catalog.projects.some((entry) => entry.project_key === key), arg: recent.project_key,
        });
        recentTabId = openEntry.projects.find((entry) => entry.project_key === recent.project_key)!.id;
        await readLiveHubCatalog(page, base!, {
          send: { kind: "close_project_tab", tab_id: recentTabId },
          until: (catalog, key) => !catalog.projects.some((entry) => entry.project_key === key), arg: recent.project_key,
        });
        recentTabId = undefined;
        await page.goto(liveGwtProjectUrl(base!, recent.project_key!));
        await expect(page.locator(".project-tab[aria-current='page']")).toBeVisible({ timeout: 30_000 });
        const reopened = await readLiveHubCatalog(page, base!, {
          until: (catalog, key) => catalog.projects.some((entry) => entry.project_key === key), arg: recent.project_key,
        });
        recentTabId = reopened.projects.find((entry) => entry.project_key === recent.project_key)!.id;
      } finally {
        if (recentTabId) {
          await readLiveHubCatalog(page, base!, {
            send: { kind: "close_project_tab", tab_id: recentTabId },
          }).catch(() => {});
        }
        await rm(recentRoot, { recursive: true, force: true });
      }
      expect(errors).toEqual([]);
    });
  });
});

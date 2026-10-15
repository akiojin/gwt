import { expect, test } from "@playwright/test";
import {
  APP_URL,
  installEmbeddedRoutes,
} from "./_helpers/embedded-frontend";

import {
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";


test.describe("surface reopen z-order", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("reopening an existing surface raises it locally before backend focus ack", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    const board = page.locator('.workspace-window[data-preset="board"]');
    const index = page.locator('.workspace-window[data-preset="index"]');
    await expect(board).toBeVisible();
    await expect(index).toBeVisible();
    await expect(async () => {
      expect(await zIndex(board)).toBeLessThan(await zIndex(index));
    }).toPass();

    await page.locator("#add-button").click();
    await expect(page.locator("#preset-modal")).not.toHaveAttribute(
      "aria-hidden",
      "true",
    );
    await page.locator('#preset-modal [data-preset="board"]').click();

    await expect(board).toHaveClass(/focused/);
    await expect(async () => {
      expect(await zIndex(board)).toBeGreaterThan(await zIndex(index));
    }).toPass({ timeout: 500 });

    await expect
      .poll(async () => {
        return page.evaluate(() => (window as any).__focusRequests ?? []);
      })
      .toEqual(["tab-1::board-1"]);
  });
});

async function zIndex(locator: any): Promise<number> {
  const raw = await locator.evaluate((node: HTMLElement) => {
    return Number.parseInt(node.style.zIndex || "0", 10);
  });
  return Number.isFinite(raw) ? raw : 0;
}

async function installWorkspaceFixture(page: any): Promise<void> {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "tab-1::board-1",
                  title: "Board",
                  preset: "board",
                  status: "running",
                  geometry: { x: 160, y: 120, width: 720, height: 420 },
                  z_index: 1,
                  placement: { kind: "canvas" },
                },
                {
                  id: "tab-1::index-1",
                  title: "Index",
                  preset: "index",
                  status: "running",
                  geometry: { x: 220, y: 170, width: 720, height: 420 },
                  z_index: 2,
                  placement: { kind: "canvas" },
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    (window as any).__focusRequests = [];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message.kind === "frontend_ready") {
          (this as any).emit(workspaceState);
          return;
        }
        if (message.kind === "focus_window") {
          (window as any).__focusRequests.push(message.id);
        }
      }

      close(): void {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: any): void {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PRESETS = ["logs", "board", "issue", "index", "settings", "profile"];

test("Issue #4174: six supported surfaces retain keyboard navigation without retired GUI windows", async ({ page }, info) => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is required for the live backend");
  await withLiveGwtBackendLock(BASE, info, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, keepPresetModal: true });
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const beforeIds = await page.locator(".workspace-window").evaluateAll(nodes =>
      nodes.map(node => (node as HTMLElement).dataset.id));

    try {
      const add = page.locator("#add-button");
      await add.click();
      const modal = page.locator("#preset-modal");
      await expect(modal).toBeVisible();
      const buttons = modal.locator(".preset-button");
      await expect(buttons).toHaveCount(6);
      expect(await buttons.evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.preset)))
        .toEqual(PRESETS);
      const preset = (name: string) => modal.locator(`[data-preset="${name}"]`);
      await expect(preset("logs")).toBeFocused();
      for (const name of ["board", "issue", "index", "settings"]) {
        await page.keyboard.press("ArrowRight");
        await expect(preset(name)).toBeFocused();
      }
      await page.keyboard.press("ArrowDown");
      await expect(preset("profile")).toBeFocused();
      await page.keyboard.press("ArrowUp");
      await expect(preset("settings")).toBeFocused();
      await info.attach(`supported-surfaces-${theme}`, {
        body: await page.screenshot({ path: info.outputPath(`supported-surfaces-${theme}.png`) }),
        contentType: "image/png",
      });
      await page.keyboard.press("Escape");
      await expect(modal).toBeHidden();
      await expect(add).toBeFocused();

      await add.click();
      await expect(preset("logs")).toBeFocused();
      await page.keyboard.press("ArrowRight");
      await expect(preset("board")).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(modal).toBeHidden();
      await expect(page.locator('.workspace-window[data-preset="board"]').first()).toBeVisible();
      await expect(page.locator([
        '.workspace-window[data-preset="file_tree"]',
        '.workspace-window[data-preset="pr"]',
        '.workspace-window[data-preset="console"]',
      ].join(","))).toHaveCount(0);
      expect(errors, "zero console/page errors").toEqual([]);
    } finally {
      const created = await page.locator('.workspace-window[data-preset="board"]').evaluateAll(
        (nodes, before) => nodes.map(node => (node as HTMLElement).dataset.id)
          .filter(id => id && !before.includes(id)), beforeIds);
      for (const id of created) await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
  });
});

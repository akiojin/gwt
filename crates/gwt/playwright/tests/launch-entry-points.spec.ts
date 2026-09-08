/* Issue #3571 / SPEC #3245 Stage E — the deprecated Intake launch route is
 * absent from every frontend entry surface. Normal Workspace and Add Window
 * actions remain available, and even a stale internal command cannot emit the
 * removed open_intake_session wire.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Launch entry points", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("deprecated Intake surfaces are absent while Workspace and Add Window remain", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installOpenProjectBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.op-rail [data-cmd="intake-session"]')).toHaveCount(0);
    await expect(page.locator("#canvas-empty-intake")).toHaveCount(0);
    await expect(page.locator("#op-workspace-overview-entry")).toBeVisible();
    await expect(page.locator("#canvas-empty-open-workspace")).toBeVisible();
    await expect(page.locator("#canvas-empty-add-window")).toBeVisible();
    expect(pageErrors).toEqual([]);
  });

  test("stale Intake command emits no removed wire or pending surface", async ({ page }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installOpenProjectBackend(page);
    await page.goto(APP_URL);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await page.evaluate(async () => {
      document.dispatchEvent(
        new CustomEvent("op:command", { detail: { id: "intake-session" } }),
      );
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });

    const sentKinds = await page.evaluate(() => (window as any).__sentKinds as string[]);
    expect.soft(sentKinds).not.toContain("open_intake_session");
    await expect.soft(page.locator("#wizard-modal")).not.toHaveClass(/\bopen\b/);
    await expect.soft(page.locator("#wizard-modal")).toHaveAttribute("aria-hidden", "true");
    expect(pageErrors).toEqual([]);
  });
});

async function installOpenProjectBackend(page: any): Promise<void> {
  await page.addInitScript(() => {
    (window as any).__sentKinds = [];
    try {
      // Suppress the first-run briefing overlay so it can't intercept clicks.
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }

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
              windows: [],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

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

      send(raw: string) {
        let message: any = null;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message?.kind) (window as any).__sentKinds.push(message.kind);
        if (message && message.kind === "frontend_ready") {
          this.emit(workspaceState);
        }
      }

      close() {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
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

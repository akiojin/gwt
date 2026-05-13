/* Issue #2684 — top-toolbar Open Project split-button exposes Clone from
 * GitHub while a project tab is active. The picker overlay only shows when
 * no tab is open, so the split-button is the canonical path back to the
 * clone modal for users mid-session.
 *
 * The fixture pattern follows `tests/index-status.spec.ts`: serve the
 * embedded frontend via `installEmbeddedRoutes` and replace WebSocket with a
 * deterministic backend that emits one workspace_state with a single tab.
 */
import { expect, test } from "@playwright/test";
import {
  APP_URL,
  installEmbeddedRoutes,
} from "./_helpers/embedded-frontend";

test.describe("Open Project split-button", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("caret reveals dropdown with Open / Clone / Recent items", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    // Picker overlay must NOT be visible while a tab is active — that is the
    // gap this Issue addresses.
    await expect(page.locator("#project-picker")).not.toHaveClass(/visible/);

    const caret = page.locator("#open-project-menu-button");
    const menu = page.locator("#open-project-menu");
    await expect(caret).toHaveAttribute("aria-expanded", "false");

    await caret.click();
    await expect(menu).toHaveClass(/open/);
    await expect(caret).toHaveAttribute("aria-expanded", "true");

    await expect(page.locator("#open-project-menu-open")).toBeVisible();
    await expect(page.locator("#open-project-menu-clone")).toBeVisible();
    await expect(
      page.locator("#open-project-menu .split-button-menu-section-label"),
    ).toHaveText("Recent");
  });

  test("Clone from GitHub menu item opens the clone modal", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await page.locator("#open-project-menu-button").click();
    await page.locator("#open-project-menu-clone").click();

    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);
    await expect(page.locator("#clone-project-modal")).not.toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  test("Escape closes the menu and returns focus to the caret", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    const caret = page.locator("#open-project-menu-button");
    const menu = page.locator("#open-project-menu");

    await caret.click();
    await expect(menu).toHaveClass(/open/);

    await page.keyboard.press("Escape");
    await expect(menu).not.toHaveClass(/open/);
    await expect(caret).toHaveAttribute("aria-expanded", "false");
    await expect(caret).toBeFocused();
  });
});

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
              windows: [],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [
          {
            title: "Recent A",
            path: "/recent/a",
            kind: "git",
          },
        ],
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

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch (e) {
          return;
        }
        if (message.kind === "frontend_ready") {
          (this as any).emit(workspaceState);
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

/* SPEC-2013 Phase 8 — the top-toolbar Open Project split-button (Issue #2684)
 * was retired. The consolidated `Projects ▾` switcher now owns switching
 * (OPEN / RECENT) plus project intake (Open Folder… / Clone from GitHub…),
 * and the picker overlay only shows when no tab is open.
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

test.describe("Projects ▾ consolidated chrome", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("the Open Project split-button is removed from the top bar", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await expect(page.locator("#open-project-group")).toHaveCount(0);
    await expect(page.locator("#open-project-menu")).toHaveCount(0);
    await expect(page.locator(".split-button-group")).toHaveCount(0);
    await expect(page.locator("#project-switcher-button")).toBeVisible();
  });

  test("Projects ▾ lists OPEN + RECENT and the Open Folder / Clone actions", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });
    // Picker overlay must NOT be visible while a tab is active.
    await expect(page.locator("#project-picker")).not.toHaveClass(/visible/);

    await page.locator("#project-switcher-button").click();
    const panel = page.locator("#project-switcher-panel");
    await expect(panel).toHaveClass(/open/);

    await expect(panel.getByText("Open Projects")).toBeVisible();
    await expect(panel.getByText("Fixture Project")).toBeVisible();
    await expect(panel.getByText("Recent A")).toBeVisible();
    await expect(
      panel.locator("[data-action='open-folder']"),
    ).toHaveText(/Open Folder/);
    await expect(
      panel.locator("[data-action='clone-from-github']"),
    ).toHaveText(/Clone from GitHub/);
  });

  test("Clone from GitHub action opens the clone modal", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await page.locator("#project-switcher-button").click();
    await page.locator("[data-action='clone-from-github']").click();

    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);
    await expect(page.locator("#clone-project-modal")).not.toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  test("Escape closes the Projects switcher", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    const panel = page.locator("#project-switcher-panel");
    await page.locator("#project-switcher-button").click();
    await expect(panel).toHaveClass(/open/);

    await page.keyboard.press("Escape");
    await expect(panel).not.toHaveClass(/open/);
    await expect(page.locator("#project-switcher-button")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });
});

async function installWorkspaceFixture(
  page: any,
  options: { recentProjects?: Array<{ title: string; path: string; kind: string }> } = {},
): Promise<void> {
  const recentProjects = options.recentProjects ?? [
    { title: "Recent A", path: "/recent/a", kind: "git" },
  ];
  await page.addInitScript((fixture: any) => {
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
        recent_projects: fixture.recentProjects,
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
  }, { recentProjects });
}

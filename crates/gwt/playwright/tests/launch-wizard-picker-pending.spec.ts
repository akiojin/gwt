import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC-3214 T-042 (FR-010): the standalone existing-branch picker keeps the
// local pending-wizard UX the removed Start Work entry used to provide.
test.describe("Launch Wizard existing-branch picker pending state", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("Open existing branch opens the local pending wizard before backend state arrives", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => {
      pageErrors.push(error.message);
    });

    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await page.locator("#canvas-empty-open-branch").click();

    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toHaveClass(/open/);
    await expect(wizard).not.toHaveAttribute("aria-hidden", "true");
    await expect(wizard).toContainText("Fetching remote branches...");
    await expect(wizard.locator("#wizard-submit-button")).toBeHidden();

    await expect
      .poll(() => page.evaluate(() => (window as any).__sentKinds))
      .toContain("open_existing_branch");
    expect(pageErrors).toEqual([]);
  });
});

async function keepLaunchWizardModalVisible(page: any): Promise<void> {
  await page.addStyleTag({
    content: `
      #wizard-modal[aria-hidden="false"],
      #wizard-modal.open {
        display: flex !important;
        pointer-events: auto !important;
      }
      #wizard-modal[aria-hidden="true"] {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });
}

async function installWorkspaceFixture(page: any): Promise<void> {
  await page.addInitScript(() => {
    (window as any).__sent = [];
    (window as any).__sentKinds = [];

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

      url: string;
      readyState: number;

      constructor(url: string) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        (window as any).__sent.push(message);
        (window as any).__sentKinds.push(message.kind);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
        }
      }

      close(): void {
        this.readyState = FixtureWebSocket.CLOSED;
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

import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Launch Wizard states", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("Intake opens the local pending wizard before backend state arrives", async ({
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

    await page.locator('.op-rail__item[data-cmd="intake-session"]').click();

    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toHaveClass(/open/);
    await expect(wizard).not.toHaveAttribute("aria-hidden", "true");
    await expect(wizard.locator("#wizard-title")).toHaveText("Intake");
    await expect(wizard).toContainText("Preparing Intake session...");
    await expect(wizard.locator("#wizard-submit-button")).toBeHidden();

    await expect
      .poll(() => page.evaluate(() => (window as any).__sentKinds))
      .toContain("open_intake_session");
    expect(pageErrors).toEqual([]);
  });

  test("exact holder decision renders safe actions without browser errors", async ({
    page,
  }, testInfo) => {
    const browserErrors: string[] = [];
    page.on("pageerror", (error) => {
      browserErrors.push(error.message);
    });
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(message.text());
    });

    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "launch_wizard_state",
            wizard: {
              title: "Launch Agent",
              branch_name: "work/holder-decision",
              selected_branch_name: "work/holder-decision",
              branch_mode: "use_selected",
              show_back_button: true,
              show_branch_controls: true,
              show_manual_setup: false,
              show_runtime_confirmation: false,
              show_confirm: false,
              show_start_methods: false,
              runtime_context_resolved: true,
              primary_action_enabled: false,
              launch_summary: [],
              progress_steps: [],
              holder_decision: {
                fingerprint: "fp-live-holder",
                holder_session_id: "session-live-holder",
                holder_window_id: "window-live-holder",
                holder_summary: "Codex · local active holder",
                stop_available: true,
                stop_unavailable_reason: null,
                move_available: true,
                move_unavailable_reason: null,
              },
            },
          },
        }),
      );
    });

    const modal = page.locator("#wizard-modal");
    const dialog = modal.getByRole("dialog");
    await expect(modal).toHaveClass(/open/);
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(dialog).toHaveAttribute("aria-labelledby", "wizard-title");
    await expect(modal.locator(".modal-header")).toBeVisible();
    await expect(modal.locator("#wizard-body")).toContainText(
      "Codex · local active holder",
    );

    const actions = modal.locator(".wizard-actions > button:visible");
    await expect(actions).toHaveText([
      "Move existing pane",
      "Cancel",
      "Stop and start successor",
    ]);
    await expect(actions.nth(0)).toBeEnabled();
    await expect(actions.nth(2)).toBeEnabled();
    await expect(actions.nth(2)).toHaveClass(/destructive/);
    await expect(actions.nth(0)).not.toHaveClass(/destructive/);

    await testInfo.attach("holder-theme", {
      body: Buffer.from(testInfo.project.name),
      contentType: "text/plain",
    });
    expect(browserErrors).toEqual([]);
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

/* Issue #3192 / SPEC #3214 — clicking the left-rail "Intake" button must open
 * the Launch Wizard pending modal ("Preparing Intake session...").
 *
 * Regression of commit 1fdbe25c0 ("fix: show launch materialization
 * progress"): renderLaunchWizard() dereferenced `launchWizard` before the
 * opening/error early-returns, so the Start Work pending state (launchWizard
 * === null, launchWizardOpening set) threw a TypeError and the modal never
 * received its `.open` class. The crash is synchronous inside the click
 * handler, so it also blocked the `open_intake_session` WS send — the button
 * silently did nothing.
 *
 * This spec boots the embedded frontend with a deterministic WebSocket stub
 * (no live gwt process), clicks the rail button, and asserts the pending
 * modal opens without a page error. It needs no backend round-trip because
 * the pending modal is rendered client-side before any send.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Intake launch-pending modal", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("clicking Intake opens the Preparing Intake modal", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installOpenProjectBackend(page);
    await page.goto(APP_URL);

    // App boots to an open git project; the operator rail is interactive.
    const intake = page.locator('.op-rail [data-cmd="intake-session"]');
    await expect(intake).toBeVisible({ timeout: 10_000 });

    await intake.click();

    const wizard = page.locator("#wizard-modal");
    // The `.open` class is added at the end of renderLaunchWizard(); the
    // regression threw before reaching it.
    await expect(wizard).toHaveClass(/\bopen\b/, { timeout: 10_000 });
    await expect(wizard.locator(".launch-pending-note")).toHaveText(
      "Preparing Intake session...",
    );

    // Pin the exact regression: no null dereference of the wizard view model.
    const wizardErrors = pageErrors.filter((message) =>
      message.includes("launch_materialization_pending"),
    );
    expect(
      wizardErrors,
      `unexpected wizard page error(s): ${pageErrors.join("\n")}`,
    ).toEqual([]);
  });
});

async function installOpenProjectBackend(page: any): Promise<void> {
  await page.addInitScript(() => {
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
        if (message && message.kind === "frontend_ready") {
          this.emit(workspaceState);
        }
        // open_intake_session and other sends are intentionally ignored: the
        // pending modal is rendered client-side before this point.
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

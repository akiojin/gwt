/* SPEC-3214 T-050 (FR-009/FR-010) — the Start Work entry points are removed
 * and the autonomous-era launch entries take their place:
 *
 *   - the operator rail exposes Intake session (no Start Work item)
 *   - the empty-canvas call to action offers Quick issue / Intake session /
 *     Shell / Open existing branch
 *   - clicking Open existing branch renders the client-side pending wizard
 *     ("Fetching remote branches...") before any backend round-trip
 *
 * This spec boots the embedded frontend with a deterministic WebSocket stub
 * (no live gwt process). The pending modal is rendered client-side before the
 * `open_existing_branch` WS send, so no backend round-trip is required.
 * (The pending-render-before-send contract itself was pinned by Issue #3192.)
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("launch entry points (SPEC-3214)", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("rail and empty canvas expose the intake-era entries without Start Work", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installOpenProjectBackend(page);
    await page.goto(APP_URL);

    // App boots to an open git project; the operator rail is interactive.
    const intakeSession = page.locator('.op-rail [data-cmd="intake-session"]');
    await expect(intakeSession).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('.op-rail [data-cmd="start-work"]')).toHaveCount(0);

    // The empty canvas offers the autonomous-era call to action.
    const empty = page.locator("#canvas-empty-state");
    await expect(empty).toBeVisible();
    await expect(empty.locator("#canvas-empty-start-work")).toHaveCount(0);
    for (const id of [
      "canvas-empty-quick-issue",
      "canvas-empty-intake-session",
      "canvas-empty-shell",
      "canvas-empty-open-branch",
      "canvas-empty-add-window",
    ]) {
      await expect(empty.locator(`#${id}`)).toBeVisible();
    }

    // Open existing branch renders the pending wizard client-side.
    await empty.locator("#canvas-empty-open-branch").click();
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toHaveClass(/\bopen\b/, { timeout: 10_000 });
    await expect(wizard.locator(".launch-pending-note")).toHaveText(
      "Fetching remote branches...",
    );

    expect(pageErrors, `unexpected page error(s): ${pageErrors.join("\n")}`).toEqual(
      [],
    );
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
        // open_existing_branch and other sends are intentionally ignored:
        // the pending modal is rendered client-side before this point.
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

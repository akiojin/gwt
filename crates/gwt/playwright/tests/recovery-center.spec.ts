/* SPEC #1921 T460 — real-browser contract for the public-safe Recovery Center.
 *
 * This fixture exercises the full embedded frontend with a deterministic
 * WebSocket peer. The backend intentionally includes private-looking extra
 * properties so the screenshot also proves that the browser projection is an
 * allowlist rather than a generic object renderer.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Recovery Center", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => { throw error; });
    page.on("console", (message) => {
      if (message.type() === "error") throw new Error(message.text());
    });
    await installEmbeddedRoutes(page);
    await installRecoveryCenterBackend(page);
    await page.goto(APP_URL);
  });

  test("filters independently and resets both filters when reopened", async ({ page }) => {
    await runPaletteCommand(page, "Open Recovery Center");
    const modal = page.locator("#recovery-center-modal");
    const state = modal.locator("#recovery-center-state-filter");
    const form = modal.locator("#recovery-center-worktree-filter");
    await state.selectOption("all");
    await form.selectOption("branch-backed");
    await expect(modal.locator(".recovery-center-row")).toHaveCount(1);
    await expect(modal.locator(".recovery-center-row")).toContainText("Acknowledged");
    await page.keyboard.press("Escape");
    await expect(modal).not.toHaveClass(/\bopen\b/);
    await runPaletteCommand(page, "Open Recovery Center");
    await expect(state).toHaveValue("attention");
    await expect(form).toHaveValue("all");
    await expect(modal.locator(".recovery-center-row")).toHaveCount(2);
  });

  test("renders all public facets and reuses the Board deep-link path", async ({
    page,
  }, testInfo) => {
    await runPaletteCommand(page, "Open Recovery Center");

    const modal = page.locator("#recovery-center-modal");
    const dialog = modal.getByRole("dialog", { name: "Recovery Center" });
    await expect(modal).toHaveClass(/\bopen\b/);
    await expect(dialog).toBeVisible();

    // The default Attention view contains pending + conflicted only.
    const rows = modal.locator(".recovery-center-row");
    await expect(rows).toHaveCount(2);
    await expect(modal.locator(".recovery-center-count")).toHaveText(
      "2 shown · 3 total",
    );

    // The explicit All states filter is the only view that reveals the
    // acknowledged delivery. Keep all three rows visible for the visual
    // evidence requested by T460.
    await modal.locator("#recovery-center-state-filter").selectOption("all");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("Pending");
    await expect(rows.nth(0)).toContainText("Ephemeral");
    await expect(rows.nth(1)).toContainText("Acknowledged");
    await expect(rows.nth(1)).toContainText("Branch-Backed");
    await expect(rows.nth(2)).toContainText("Conflicted");
    await expect(rows.nth(2)).toContainText("Unknown");
    await expect(modal).not.toContainText("private-session-sentinel");
    await expect(modal).not.toContainText("/private/recovery/path");
    await expect(modal).not.toContainText(/\b(?:Intake|Execution)\b/);

    await page.evaluate(() => document.fonts.ready);
    await testInfo.attach("recovery-center-modal", {
      body: await dialog.screenshot(),
      contentType: "image/png",
    });

    // Only the acknowledged row has a Board action. The backend resolves its
    // opaque handle to a public Board entry id, then the existing Board focus
    // route creates/focuses the singleton Board surface.
    await expect(
      rows.nth(0).getByRole("button", { name: "Open Board history" }),
    ).toHaveCount(0);
    await expect(
      rows.nth(2).getByRole("button", { name: "Open Board history" }),
    ).toHaveCount(0);
    await rows.nth(1).getByRole("button", { name: "Open Board history" }).click();
    await expect(modal).not.toHaveClass(/\bopen\b/);
    await expect
      .poll(() => fixtureMessages(page))
      .toContainEqual(expect.objectContaining({ kind: "create_window", preset: "board" }));
  });
});

async function runPaletteCommand(page: any, query: string): Promise<void> {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute(
    "data-open",
    "true",
  );
}

async function fixtureMessages(page: any): Promise<Array<Record<string, unknown>>> {
  return page.evaluate(() => (window as any).__recoveryCenterFixtureMessages || []);
}

async function installRecoveryCenterBackend(page: any): Promise<void> {
  await page.addInitScript(() => {
    try {
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
            id: "recovery-tab",
            title: "Recovery Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          },
        ],
        active_tab_id: "recovery-tab",
        recent_projects: [],
      },
    };

    const recoveryItems = [
      {
        action_handle: "opaque-pending",
        state: "pending",
        worktree_form: "ephemeral",
        title: "Unfinished delivery",
        summary: "A sanitized durable update is waiting for acknowledgement.",
        updated_at: "2026-08-10T08:20:00Z",
        session_id: "private-session-sentinel",
        path: "/private/recovery/path",
      },
      {
        action_handle: "opaque-acknowledged",
        state: "acknowledged",
        worktree_form: "branch-backed",
        title: "Delivered Board update",
        summary: "This delivery already has a public Board history entry.",
        updated_at: "2026-08-10T08:10:00Z",
        recovery_id: "private-recovery-sentinel",
      },
      {
        action_handle: "opaque-conflicted",
        state: "conflicted",
        worktree_form: "unknown",
        title: "Delivery needs attention",
        summary: "The durable update could not be applied without review.",
        updated_at: "2026-08-10T08:00:00Z",
        private_error: "private-error-sentinel",
      },
    ];

    (window as any).__recoveryCenterFixtureMessages = [];

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
        (window as any).__recoveryCenterFixtureMessages.push(message);
        if (message?.kind === "frontend_ready") {
          this.emit(workspaceState);
        } else if (message?.kind === "load_recovery_center") {
          this.emit({
            kind: "recovery_center_state",
            request_id: message.request_id,
            generation: 41,
            status: "ready",
            items: recoveryItems,
          });
        } else if (message?.kind === "open_recovery_center_board_entry") {
          this.emit({
            kind: "recovery_center_board_entry",
            request_id: message.request_id,
            generation: message.generation,
            board_entry_id: "board-entry-public-42",
          });
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

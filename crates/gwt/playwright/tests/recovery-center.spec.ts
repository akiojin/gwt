import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Recovery Center", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("auto-opens only attention candidates and manually exposes full recovery inventory", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installRecoveryBackend(page);
    await page.goto(APP_URL);

    const modal = page.locator("#recovery-center-modal");
    await expect(modal).toHaveClass(/\bopen\b/);
    await expect(modal.getByRole("dialog", { name: "Recovery Center" })).toBeVisible();
    await expect(modal.locator(".recovery-center-row")).toHaveCount(2);
    await expect(modal).toContainText("2 need attention · 3 recoverable total");
    await expect(modal).toContainText("Intake recovery");
    await expect(modal).not.toContainText("Already restored exactly");

    await modal.getByRole("button", { name: "Close" }).click();
    await expect(modal).not.toHaveClass(/\bopen\b/);

    await page.locator("#op-palette-button").click();
    const paletteInput = page.locator("#op-palette-input");
    await expect(paletteInput).toBeVisible();
    await paletteInput.fill("Recovery Center");
    await page.getByRole("option", { name: /Recovery Center/ }).click();

    await expect(modal).toHaveClass(/\bopen\b/);
    await expect(modal.locator(".recovery-center-row")).toHaveCount(3);
    await expect(modal).toContainText("3 recoverable sessions");
    await expect(modal).toContainText("Intake recovery");

    const executionRow = modal.locator('[data-action-handle="rc1_execution_opaque"]');
    await executionRow.focus();
    await page.keyboard.press("Enter");
    await expect(executionRow).toBeFocused();

    const intakeRow = modal.locator('[data-action-handle="rc1_intake_opaque"]');
    await intakeRow.focus();
    await page.keyboard.press("Enter");
    await expect(intakeRow).toBeFocused();

    const roots = modal.locator('[name="recovery-provider-choice"]');
    await expect(roots).toHaveCount(2);
    await roots.nth(1).focus();
    await page.keyboard.press("Space");
    await expect(roots.nth(1)).toBeChecked();
    await expect(roots.nth(1)).toBeFocused();

    const confirmResume = modal.locator('[data-recovery-action="confirm_resume"]');
    await page.keyboard.press("Tab");
    await expect(confirmResume).toBeFocused();
    await page.keyboard.press("Enter");

    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__recoveryRequests?.at(-1) ?? null),
      )
      .toMatchObject({
        kind: "recovery_center_action",
        request: {
          action_handle: "rc1_intake_opaque",
          action: "confirm_resume",
          provider_choice_handle: "rp1_choice_b_opaque",
        },
      });
    expect(pageErrors).toEqual([]);
  });
});

async function installRecoveryBackend(page: any): Promise<void> {
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
    const recoveryState = {
      kind: "recovery_center_state",
      center: {
        candidates: [
          {
            action_handle: "rc1_intake_opaque",
            attention_required: true,
            purpose_preview: "Intake recovery",
            kind: "intake",
            provider: "Codex",
            worktree_name: ".intake-5",
            last_checkpoint_at: "2026-07-16 12:00",
            coverage: "current structured milestone",
            capture_health: "degraded",
            board_pending: 1,
            exact_available: false,
            exact_ambiguous: true,
            available_actions: ["confirm_resume", "details", "open_board"],
            provider_choices: [
              {
                choice_handle: "rp1_choice_a_opaque",
                label: "Candidate 1",
                evidence_count: 1,
              },
              {
                choice_handle: "rp1_choice_b_opaque",
                label: "Candidate 2",
                evidence_count: 1,
              },
            ],
          },
          {
            action_handle: "rc1_execution_opaque",
            attention_required: true,
            purpose_preview: "Execution recovery",
            kind: "execution",
            provider: "Claude Code",
            worktree_name: "feature",
            capture_health: "healthy",
            board_pending: 0,
            exact_available: true,
            available_actions: ["confirm_resume", "continue_checkpoint", "details"],
          },
          {
            action_handle: "rc1_restored_opaque",
            attention_required: false,
            purpose_preview: "Intake recovery",
            kind: "intake",
            provider: "Codex",
            worktree_name: ".intake-3",
            capture_health: "healthy",
            board_pending: 0,
            exact_available: true,
            available_actions: ["focus", "details"],
          },
        ],
      },
    };

    (window as any).__recoveryRequests = [];
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      readyState = FixtureWebSocket.CONNECTING;
      url: string;

      constructor(url: string) {
        super();
        this.url = url;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message?.kind === "frontend_ready") this.emit(workspaceState);
        if (message?.kind === "list_recovery_center") this.emit(recoveryState);
        if (message?.kind === "recovery_center_action") {
          (window as any).__recoveryRequests.push(message);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
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

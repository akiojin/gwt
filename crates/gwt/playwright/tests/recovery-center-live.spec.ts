/* SPEC #1921 T460 — live-backend Recovery Center privacy contract.
 *
 * The fixture visual test covers deterministic states. This companion test
 * uses a fresh live gwt server when GWT_PLAYWRIGHT_BASE_URL is supplied and
 * inspects the actual WebSocket response before asserting the rendered empty
 * or public-safe state.
 */
import { expect, test } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  gotoLiveGwt,
  openLiveGwtProject,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
let releaseLiveBackendLock: (() => Promise<void>) | null = null;

test.describe.serial("Recovery Center live backend", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.setTimeout(90_000);

  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "Recovery Center live E2E runs once against the shared backend",
    );
    releaseLiveBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    await installRecoveryCenterWireCapture(page);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await openLiveGwtProject(page);
  });

  test.afterEach(async () => {
    if (releaseLiveBackendLock) {
      await releaseLiveBackendLock();
      releaseLiveBackendLock = null;
    }
  });

  test("manual open receives only the public read projection", async ({ page }) => {
    await runPaletteCommand(page, "Open Recovery Center");

    const modal = page.locator("#recovery-center-modal");
    await expect(modal.getByRole("dialog", { name: "Recovery Center" })).toBeVisible();
    await page.waitForFunction(
      () => (window as any).__recoveryCenterWireEvents?.length > 0,
      undefined,
      { timeout: 30_000 },
    );

    const event = await page.evaluate(() =>
      (window as any).__recoveryCenterWireEvents.at(-1),
    );
    expect(event.kind).toBe("recovery_center_state");
    expect(Object.keys(event).sort()).toEqual([
      "generation",
      "items",
      "kind",
      "request_id",
      "status",
    ]);
    expect(["ready", "error"]).toContain(event.status);
    expect(Array.isArray(event.items)).toBe(true);

    for (const item of event.items) {
      const publicKeys = [
        "action_handle", "state", "summary", "updated_at", "worktree_form",
      ];
      if (Object.hasOwn(item, "title")) publicKeys.push("title");
      expect(Object.keys(item).sort()).toEqual(publicKeys.sort());
    }

    const modalText = await modal.innerText();
    expect(modalText).not.toMatch(/\/Users\/|session[_ -]?id|recovery[_ -]?id/i);
    expect(modalText).not.toMatch(/\b(?:Intake|Execution)\b/);
    if (event.status === "ready" && event.items.length === 0) {
      await expect(modal).toContainText("No recovery deliveries are available.");
    } else if (event.status === "error") {
      await expect(modal).toContainText("Recovery deliveries could not be loaded.");
    }
  });
});

async function runPaletteCommand(page: any, query: string): Promise<void> {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
}

async function installRecoveryCenterWireCapture(page: any): Promise<void> {
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    (window as any).__recoveryCenterWireEvents = [];
    window.WebSocket = class RecoveryCenterCaptureSocket extends NativeWebSocket {
      constructor(url, protocols) {
        super(url, protocols);
        this.addEventListener("message", (event) => {
          try {
            const payload = JSON.parse(String(event.data));
            if (payload?.kind === "recovery_center_state") {
              (window as any).__recoveryCenterWireEvents.push(payload);
            }
          } catch {
            /* no-op */
          }
        });
      }
    };
    Object.assign(window.WebSocket, NativeWebSocket);
  });
}

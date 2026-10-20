/* SPEC #1921 T461 — real Store -> Recovery Center -> Board acceptance.
 *
 * The fixture visual test covers deterministic states. This companion test
 * uses a fresh live gwt server when GWT_PLAYWRIGHT_BASE_URL is supplied and
 * inspects the actual WebSocket response and all three seeded Store states.
 * The Rust Board CLI fixture supplies the expected public Board article id.
 */
import { readFileSync } from "node:fs";
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
    page.on("pageerror", (error) => { throw error; });
    page.on("console", (message) => {
      if (message.type() === "error") throw new Error(message.text());
    });
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

  test("three real Store states filter independently and open the exact acknowledged Board article", async ({ page }, testInfo) => {
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
    // Empty/missing storage must never pass this positive acceptance.
    expect(event.items.map((item: any) => item.state).sort()).toEqual([
      "acknowledged", "conflicted", "pending",
    ]);
    expect(event.status).toBe("ready");
    const expected = JSON.parse(readFileSync(
      process.env.GWT_RECOVERY_FIXTURE_METADATA!, "utf8",
    ));
    expect(event.items.map(({ state, worktree_form }: any) => ({ state, worktree_form })))
      .toEqual(expect.arrayContaining([
        { state: "pending", worktree_form: "ephemeral" },
        { state: "conflicted", worktree_form: "unknown" },
        { state: "acknowledged", worktree_form: "branch-backed" },
      ]));

    for (const item of event.items) {
      const publicKeys = [
        "action_handle", "state", "summary", "updated_at", "worktree_form",
      ];
      if (Object.hasOwn(item, "title")) publicKeys.push("title");
      expect(Object.keys(item).sort()).toEqual(publicKeys.sort());
    }

    const rows = modal.locator(".recovery-center-row");
    const state = modal.locator("#recovery-center-state-filter");
    const form = modal.locator("#recovery-center-worktree-filter");
    await expect(state).toHaveValue("attention");
    await expect(rows).toHaveCount(2);
    await expect(rows.getByRole("button", { name: "Open Board history" })).toHaveCount(0);
    // Both unpublished states must reject the actual keyboard action path.
    const pending = modal.locator('.recovery-center-row[data-state="pending"]');
    const conflicted = modal.locator('.recovery-center-row[data-state="conflicted"]');
    await pending.focus();
    await page.keyboard.press("Enter");
    await conflicted.focus();
    await page.keyboard.press("Enter");
    await expect(modal).toHaveClass(/\bopen\b/);
    expect(await page.evaluate(() => (window as any).__recoveryCenterActionReplies)).toEqual([]);

    await state.selectOption("all");
    await form.selectOption("branch-backed");
    await expect(rows).toHaveCount(1);
    await expect(rows).toHaveAttribute("data-state", "acknowledged");
    await state.selectOption("pending");
    await expect(form).toHaveValue("branch-backed");
    await expect(rows).toHaveCount(0);
    await form.selectOption("ephemeral");
    await expect(state).toHaveValue("pending");
    await expect(rows).toHaveCount(1);
    await expect(rows).toHaveAttribute("data-state", "pending");
    await state.selectOption("attention");
    await form.selectOption("unknown");
    await expect(rows).toHaveCount(1);
    await expect(rows).toHaveAttribute("data-state", "conflicted");
    await state.selectOption("all");
    await form.selectOption("all");
    await expect(rows).toHaveCount(3);
    await expect(modal.locator(".recovery-center-count")).toHaveText("3 shown · 3 total");
    const modalText = await modal.innerText();
    expect(modalText).not.toMatch(/\/Users\/|session[_ -]?id|recovery[_ -]?id/i);
    expect(modalText).not.toMatch(/\b(?:Intake|Execution)\b/);
    await testInfo.attach("real-store-three-states", {
      body: await modal.screenshot({ path: testInfo.outputPath("real-store-three-states.png") }),
      contentType: "image/png",
    });
    await modal.getByRole("button", { name: "Open Board history" }).click();
    await expect(modal).not.toHaveClass(/\bopen\b/);
    const board = page.locator('.workspace-window[data-preset="board"]');
    await expect(board).toBeVisible();
    const article = board.locator(`[data-board-entry-id="${expected.entry_id}"]`);
    await expect(article).toContainText(expected.title);
    await expect(article).toContainText(expected.body);
    await expect(article).toHaveClass(/\bfocus-target\b/);
    await expect(article).toBeFocused();
    await expect(board.locator("[data-board-entry-id]")).toHaveCount(1);
    const replies = await page.evaluate(() => (window as any).__recoveryCenterActionReplies);
    expect(replies).toHaveLength(1);
    expect(replies[0].board_entry_id).toBe(expected.entry_id);
    expect(replies[0].error).toBeUndefined();
    await testInfo.attach("exact-acknowledged-board-article", {
      body: await board.screenshot({ path: testInfo.outputPath("exact-acknowledged-board-article.png") }),
      contentType: "image/png",
    });
  });

  test("invalid action receives an explicit handle rejection from the live backend", async ({ page }) => {
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_send", {
      detail: { kind: "open_recovery_center_board_entry", request_id: "invalid-recovery-action",
        generation: 1, action_handle: "rc-invalid-client-handle" },
    })));
    await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages.some(
      ({ payload }) => payload.kind === "recovery_center_board_entry"
        && payload.request_id === "invalid-recovery-action",
    ));
    const reply = await page.evaluate(() => (window as any).__gwtPlaywrightMessages.find(
      ({ payload }) => payload.kind === "recovery_center_board_entry"
        && payload.request_id === "invalid-recovery-action",
    ).payload);
    expect(reply.board_entry_id).toBeUndefined();
    expect(reply.error).toContain("rc-invalid-client-handle");
    expect(reply.error).toContain("unknown or expired handle");
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
    (window as any).__recoveryCenterActionReplies = [];
    window.WebSocket = class RecoveryCenterCaptureSocket extends NativeWebSocket {
      constructor(url, protocols) {
        super(url, protocols);
        this.addEventListener("message", (event) => {
          try {
            const payload = JSON.parse(String(event.data));
            if (payload?.kind === "recovery_center_state") {
              (window as any).__recoveryCenterWireEvents.push(payload);
            }
            if (payload?.kind === "recovery_center_board_entry") {
              (window as any).__recoveryCenterActionReplies.push(payload);
            }
          } catch {
            /* no-op */
          }
        });
      }
    };
  });
}

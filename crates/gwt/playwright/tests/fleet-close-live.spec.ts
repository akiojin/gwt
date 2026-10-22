import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

test("fleet close refuses an unauthenticated browser without removing its pane", async ({ page }, testInfo) => {
  test.skip(!BASE, "requires browser-check isolated checkout instance");
  test.setTimeout(120_000);
  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
    const before = await page.locator(".workspace-window").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id));
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell", bounds: { x: 64, y: 64, width: 720, height: 420 } });
    const id = await page.waitForFunction(before => Array.from(document.querySelectorAll(".workspace-window"))
      .map(node => (node as HTMLElement).dataset.id).find(id => id && !before.includes(id)), before)
      .then(handle => handle.jsonValue());
    expect(id).toBeTruthy();
    const pane = page.locator(`.workspace-window[data-id="${id}"]`);
    try {
      const requestId = "41790000-0000-4000-8000-000000000001";
      await sendLiveGwtEvent(page, { kind: "close_fleet_pane", request_id: requestId, id,
        session_id: "untrusted-browser", child_pid: null, child_started_at: null });
      const receipt = await page.waitForFunction(requestId => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ payload }: any) => payload.kind === "fleet_pane_close_result" && payload.request_id === requestId)?.payload,
      requestId).then(handle => handle.jsonValue());
      expect(receipt.ok).toBe(false);
      expect(receipt.reason).toContain("authenticated agent");
      await expect(pane).toHaveCount(1);
    } finally {
      await sendLiveGwtEvent(page, { kind: "close_window", id });
      await expect(pane).toHaveCount(0);
    }
    expect(errors).toEqual([]);
  });
});

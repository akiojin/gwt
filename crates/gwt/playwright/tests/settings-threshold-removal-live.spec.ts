/** Issue #5628: retired threshold never appears or persists, in both themes. */
import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const preferences = process.env.GWT_PLAYWRIGHT_MONITOR_PREFS ?? "";

test("Settings uses refusal-based candidates and ignores the legacy threshold", async ({ page }, info) => {
  test.skip(!base || !preferences, "requires isolated checkout browser-check runner");
  test.setTimeout(180_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const errors: string[] = [], saves: any[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      try {
        const frame = JSON.parse(String(payload));
        if (frame.kind === "issue_monitor_profiles_set") saves.push(frame);
      } catch { /* non-JSON terminal frames */ }
    }));
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    const settings = page.locator('.workspace-window[data-preset="settings"]');
    // This runner shares one backend: light restores the window saved by dark.
    // Wait for that hydration before opening, as in settings-issues-live.
    if (theme === "light") await expect(settings).toHaveCount(1, { timeout: 30_000 });
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
      detail: { section: "agents" },
    })));
    await expect(settings).toHaveCount(1);
    const panel = settings.locator("[data-settings-candidates]");
    await expect(panel).toBeVisible();
    await expect(panel.getByLabel("Model codex", { exact: true })).toBeVisible();
    await expect(panel.getByText("Usage threshold (%)", { exact: true })).toHaveCount(0);
    await expect(panel.locator('[data-profile-field="threshold"]')).toHaveCount(0);
    await expect(panel).toContainText("next candidate when a provider rejects a launch due to a rate limit");
    await panel.getByLabel("Model codex", { exact: true }).fill(`threshold-free-${info.project.name}`);
    await panel.getByLabel("Model codex", { exact: true }).press("Tab");
    await expect.poll(() => saves.length).toBe(1);
    expect(saves[0]).not.toHaveProperty("usage_threshold_percent");
    await page.waitForFunction(requestId => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "issue_monitor_profiles" && payload.request_id === requestId),
    saves[0].request_id);
    await expect(panel).toHaveAttribute("data-saving", "false", { timeout: 90_000 });
    await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")))
      .not.toHaveProperty("launch_usage_threshold_percent");
    // An older frontend may still send the retired field. It cannot block a save.
    const legacyModel = `legacy-threshold-ignored-${info.project.name}`;
    await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set",
      profiles: [{ agent_id: "codex", model: legacyModel }], usage_threshold_percent: 0 });
    await expect.poll(async () => JSON.parse(await readFile(preferences, "utf8")).launch_profile.model,
      { timeout: 90_000 }).toBe(legacyModel);
    expect(JSON.parse(await readFile(preferences, "utf8"))).not.toHaveProperty("launch_usage_threshold_percent");
    await expect.poll(() => page.evaluate(() => {
      const messages = (window as any).__gwtPlaywrightMessages ?? [];
      return messages.findLast(({ payload }: any) => payload.kind === "issue_monitor_status")?.payload.status;
    })).toHaveProperty("launch_profile_candidates");
    const status = await page.evaluate(() => (window as any).__gwtPlaywrightMessages
      .findLast(({ payload }: any) => payload.kind === "issue_monitor_status").payload.status);
    expect(status).not.toHaveProperty("usage_threshold_percent");
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await panel.locator(".settings-help").scrollIntoViewIfNeeded();
    await expect(panel.locator(".settings-help")).toBeInViewport();
    await info.attach(`settings-threshold-${theme}`, {
      body: await page.screenshot({ path: info.outputPath(`settings-threshold-${theme}.png`) }), contentType: "image/png",
    });
    expect(errors).toEqual([]);
  });
});

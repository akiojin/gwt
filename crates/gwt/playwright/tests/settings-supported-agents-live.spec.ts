/** SPEC #1921 L3: the Settings catalog uses real, isolated detection results. */
import { expect, test } from "@playwright/test";
import { join } from "node:path";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const MISSING_AGENT = process.env.GWT_L3_MISSING_AGENT ?? "agy";

test.describe("Supported Agents Settings (isolated live backend)", () => {
  test.skip(!BASE || !process.env.GWT_L3_AGENT_FIXTURES,
    "requires checkout/fresh HOME with the L3 version-probe fixtures");
  test.use({ viewport: { width: 1440, height: 900 } });
  test.setTimeout(120_000);

  test("shows the full catalog and distinguishes unknown from uninstalled versions", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page);
      await page.evaluate(() => document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "supported-agents" } }),
      ));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      await expect(settings).toBeVisible();
      const tab = settings.getByRole("tab", { name: "Supported Agents", exact: true });
      await expect(tab).toHaveAttribute("aria-selected", "true");
      const panel = settings.locator('[data-settings-panel="supported-agents"]');
      await expect(panel).toBeVisible();
      const rows = panel.locator("tbody [data-agent-id]");
      await expect(rows).toHaveCount(8);
      await expect(panel.locator('[data-agent-id="claude"]')).toContainText("Installed");
      await expect(panel.locator('[data-agent-id="claude"]')).toContainText("2.1.0");
      const unknown = panel.locator('[data-agent-id="codex"]');
      await expect(unknown).toContainText("Installed");
      await expect(unknown).toContainText("Unknown (version unavailable)");
      const missing = panel.locator(`[data-agent-id="${MISSING_AGENT}"]`);
      await expect(missing).toContainText("Not installed");
      await expect(missing).not.toContainText("Unknown");
      const catalog = await page.evaluate(() => (window as any).__gwtPlaywrightMessages
        .findLast((entry: any) => entry.payload.kind === "supported_agent_list")?.payload.agents);
      expect(catalog.map((agent: any) => agent.id)).toEqual([
        "claude", "codex", "grok", "agy", "opencode", "openclaw", "hermes", "gh",
      ]);
      await settings.getByRole("tab", { name: "System", exact: true }).click();
      await expect(panel).toBeHidden();
      await tab.click();
      await expect(rows).toHaveCount(8);
      const theme = testInfo.project.name.endsWith("light") ? "light" : "dark";
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await testInfo.attach(`supported-agents-${theme}`, {
        body: await settings.screenshot(), contentType: "image/png",
      });
      if (process.env.GWT_L3_SCREENSHOT_DIR) {
        await settings.screenshot({
          path: join(process.env.GWT_L3_SCREENSHOT_DIR, `supported-agents-${theme}.png`),
        });
      }
      expect(errors, "console and page errors across Settings navigation").toEqual([]);
    });
  });
});

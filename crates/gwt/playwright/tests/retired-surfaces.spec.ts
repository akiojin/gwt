/** Issue #4174: the remaining Add Window surfaces work with keyboard input. */
import { expect, test } from "@playwright/test";
import {
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PRESETS = ["logs", "board", "issue", "index", "settings", "profile"];

test("Issue #4174: six supported surfaces retain keyboard navigation without retired GUI windows", async ({ page }, info) => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is required for the live backend");
  await withLiveGwtBackendLock(BASE, info, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, keepPresetModal: true });
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const beforeIds = await page.locator(".workspace-window").evaluateAll(nodes =>
      nodes.map(node => (node as HTMLElement).dataset.id));

    try {
      const add = page.locator("#add-button");
      await add.click();
      const modal = page.locator("#preset-modal");
      await expect(modal).toBeVisible();
      const buttons = modal.locator(".preset-button");
      await expect(buttons).toHaveCount(6);
      expect(await buttons.evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.preset)))
        .toEqual(PRESETS);
      const preset = (name: string) => modal.locator(`[data-preset="${name}"]`);
      await expect(preset("logs")).toBeFocused();
      for (const name of ["board", "issue", "index", "settings"]) {
        await page.keyboard.press("ArrowRight");
        await expect(preset(name)).toBeFocused();
      }
      await page.keyboard.press("ArrowDown");
      await expect(preset("profile")).toBeFocused();
      await page.keyboard.press("ArrowUp");
      await expect(preset("settings")).toBeFocused();
      await info.attach(`supported-surfaces-${theme}`, {
        body: await page.screenshot(), contentType: "image/png",
      });
      await page.keyboard.press("Escape");
      await expect(modal).toBeHidden();
      await expect(add).toBeFocused();

      await add.click();
      await expect(preset("logs")).toBeFocused();
      await page.keyboard.press("ArrowRight");
      await expect(preset("board")).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(modal).toBeHidden();
      await expect(page.locator('.workspace-window[data-preset="board"]').first()).toBeVisible();
      await expect(page.locator([
        '.workspace-window[data-preset="file_tree"]',
        '.workspace-window[data-preset="pr"]',
        '.workspace-window[data-preset="console"]',
      ].join(","))).toHaveCount(0);
      expect(errors, "zero console/page errors").toEqual([]);
    } finally {
      const created = await page.locator('.workspace-window[data-preset="board"]').evaluateAll(
        (nodes, before) => nodes.map(node => (node as HTMLElement).dataset.id)
          .filter(id => id && !before.includes(id)), beforeIds);
      for (const id of created) await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
  });
});

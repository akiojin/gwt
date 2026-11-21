/**
 * Issue #7766: the shared live helper must open the Launch Wizard whichever
 * window is frontmost. The backend refuses `open_launch_wizard` for an Issues
 * window, and Issues is the only knowledge window a current backend creates.
 * Runs this checkout's real backend on a throwaway project and fresh HOME; it
 * never submits a launch.
 */
import { expect, test } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } from "./_helpers/live-gwt";

test("opens the Launch Wizard while an Issues window is frontmost", async ({ page }, info) => {
  test.skip(process.env.GWT_PLAYWRIGHT_LAUNCH_WIZARD_HELPER !== "1",
    "GWT_PLAYWRIGHT_LAUNCH_WIZARD_HELPER=1 is not set; requires freshly built checkout binaries");
  test.setTimeout(180_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const fixture = await startExactRelaunchFixture(info);
  try {
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue" });
    const issues = page.locator('.workspace-window[data-preset="issue"]');
    await expect(issues).toBeVisible({ timeout: 30_000 });
    // Frontmost: no other visible workspace window is stacked above Issues.
    expect(await page.locator(".workspace-window").evaluateAll(nodes => nodes
      .filter(node => !(node as HTMLElement).hidden)
      .sort((left, right) => (Number.parseInt((right as HTMLElement).style.zIndex || "0", 10) || 0)
        - (Number.parseInt((left as HTMLElement).style.zIndex || "0", 10) || 0))
      .map(node => (node as HTMLElement).dataset.preset)[0])).toBe("issue");

    const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
    await openLiveLaunchWizardForBranch(page, fixture.branch);
    const reply = await page.waitForFunction(after => {
      const frame = ((window as any).__gwtPlaywrightMessages ?? []).find((entry: any) =>
        entry.sequence > after && (entry.payload?.kind === "launch_wizard_open_error"
          || (entry.payload?.kind === "launch_wizard_state" && entry.payload.wizard)));
      return frame?.payload;
    }, after, { timeout: 60_000 }).then(handle => handle.jsonValue());
    expect(reply.kind, JSON.stringify(reply)).toBe("launch_wizard_state");
    await expect(page.locator("#wizard-modal")).toBeVisible();
    expect(await fixture.launches(), "opening the wizard never spawns a provider").toEqual([]);
    expect(errors, "console and page errors").toEqual([]);
    await info.attach(`launch-wizard-helper-${theme}`, {
      body: await page.screenshot({ fullPage: true }), contentType: "image/png",
    });
  } finally {
    try { await page.close(); } finally { await fixture.stop(); }
  }
});

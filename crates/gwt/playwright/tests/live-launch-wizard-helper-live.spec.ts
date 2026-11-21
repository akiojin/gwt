/**
 * Issue #7766: the shared live helper must open the Launch Wizard whichever
 * window is frontmost, so it addresses no window (backends before Issue #7768
 * refused `open_launch_wizard` for an Issues window).
 * Runs this checkout's real backend on a throwaway project and fresh HOME; it
 * never submits a launch.
 */
import { spawnSync } from "node:child_process";
import { join } from "node:path";
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

// Issue #7768: the Workspace row ▶ and the detail `Launch Agent` live in the
// Issues window and address it by id. Both must open the wizard on a real
// backend; neither submits a launch.
test("opens the Launch Wizard from the Issues window's Workspace row and Launch Agent", async ({ page }, info) => {
  test.skip(process.env.GWT_PLAYWRIGHT_LAUNCH_WIZARD_HELPER !== "1",
    "GWT_PLAYWRIGHT_LAUNCH_WIZARD_HELPER=1 is not set; requires freshly built checkout binaries");
  test.setTimeout(180_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const remoteBranch = "feature/issue-7768";
  const fixture = await startExactRelaunchFixture(info, {
    // An origin branch nobody works on yet is listed as a startable Workspace row.
    prepare: async ({ home, project, env }) => {
      const origin = join(home, "origin.git");
      for (const args of [["init", "-q", "--bare", origin], ["remote", "add", "origin", origin],
        ["push", "-q", "origin", `HEAD:refs/heads/${remoteBranch}`], ["fetch", "-q", "origin"]]) {
        const result = spawnSync("git", args, { cwd: project, env, encoding: "utf8", windowsHide: true });
        if (result.status !== 0) throw new Error(`Fixture origin git ${args[0]} failed: ${result.stderr}`);
      }
    },
  });
  try {
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue" });
    const issues = page.locator('.workspace-window[data-preset="issue"]');
    await expect(issues).toBeVisible({ timeout: 30_000 });
    const other = issues.locator("details.issue-other-group");
    const row = other.locator(`.workspace-overview-row[data-workspace-id="remote-start:${remoteBranch}"]`);
    await expect(row).toHaveCount(1, { timeout: 60_000 });
    if (!await other.evaluate(node => (node as HTMLDetailsElement).open)) await other.locator("summary").click();
    await row.click();

    for (const action of ["launch-workspace-row", "launch-workspace"]) {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
      await other.locator(`[data-action="${action}"]`).click();
      const reply = await page.waitForFunction(after => {
        const frame = ((window as any).__gwtPlaywrightMessages ?? []).find((entry: any) =>
          entry.sequence > after && (entry.payload?.kind === "launch_wizard_open_error"
            || (entry.payload?.kind === "launch_wizard_state" && entry.payload.wizard)));
        return frame?.payload;
      }, after, { timeout: 60_000 }).then(handle => handle.jsonValue());
      expect(reply.kind, `${action}: ${JSON.stringify(reply)}`).toBe("launch_wizard_state");
      await expect(page.locator("#wizard-modal")).toBeVisible();
      await info.attach(`${action}-${theme}`, {
        body: await page.screenshot({ fullPage: true }), contentType: "image/png",
      });
      await page.keyboard.press("Escape");
      await expect(page.locator("#wizard-modal")).toBeHidden();
      // An open wizard moves the view to Works; go back to the Issues window.
      await page.locator('.op-rail__surface[data-surface="issues"]').click();
      await expect(issues).toBeVisible();
    }
    expect(await fixture.launches(), "opening the wizard never spawns a provider").toEqual([]);
    expect(errors, "console and page errors").toEqual([]);
  } finally {
    try { await page.close(); } finally { await fixture.stop(); }
  }
});

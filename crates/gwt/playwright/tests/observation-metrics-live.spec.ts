import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";
import { openWorks } from "./_helpers/works-entry";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test("Issue #7524 displays measured verification slots and only connected Works metrics", async ({ page }, info) => {
  test.skip(!base, "requires browser-check's isolated checkout instance");
  test.setTimeout(120_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME!;
    expect(await realpath(home)).not.toBe(await realpath(homedir()));
    const marker = JSON.parse(await readFile(join(home, "issue-7524-isolated.json"), "utf8"));
    expect(new URL(marker.url).origin).toBe(new URL(base).origin);
    const state = join(home, ".gwt/projects", marker.repo_hash, "project-state");
    expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
    expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.getByRole("button", { name: "Issues", exact: true }).click();
    await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
    const measured = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .filter(({ payload }: any) => payload.kind === "issue_monitor_status")
      .at(-1)?.payload.status.verification_capacity).then(handle => handle.jsonValue());
    expect(measured.capacity).toBeGreaterThan(0);
    expect(measured.running).toBeGreaterThanOrEqual(0);
    expect(measured.available).toBeGreaterThanOrEqual(0);
    const metric = page.locator('.issue-bridge-root [data-metric="verification"]').first();
    await expect(metric).toBeVisible();
    await expect.poll(() => page.evaluate(() => {
      const slots = ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status")
        .at(-1)?.payload.status.verification_capacity;
      return slots && document.querySelector('.issue-bridge-root [data-metric="verification"]')?.textContent
        === `${slots.running} of ${slots.capacity} · ${slots.available} available`;
    })).toBe(true);
    await page.screenshot({ path: info.outputPath(`verification-${theme}.png`) });
    await openWorks(page);
    const today = page.locator(".works-surface:not(.works-surface--preview) .works-today");
    await expect(today).toBeVisible();
    await expect(today).toContainText("Tokens");
    await expect(today).not.toContainText(/Cost|Landed PRs/);
    await expect(today.locator('[data-metric="today-cost"], [data-metric="today-landed"], .works-source-note')).toHaveCount(0);
    await page.screenshot({ path: info.outputPath(`works-${theme}.png`) });
    expect(errors, "zero console/page errors").toEqual([]);
  });
});

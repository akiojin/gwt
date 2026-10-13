import { expect, test } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_EXACT_RELAUNCH !== "1", "requires checkout gwt/gwtd binaries");
test.skip(process.platform === "win32", "the isolated fixture requires POSIX executables");

test("no-tray exits after the last browser closes and tolerates reload", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const fixture = await startExactRelaunchFixture(testInfo);
  try {
    const pid = fixture.ownedPid!;
    expect(pid, "fresh owned gwt PID").toBeGreaterThan(0);
    const alive = () => { try { process.kill(pid, 0); return true; } catch { return false; } };
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, fixture.url, { hub: true, enableTestBridge: true });
    await page.reload();
    await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages?.some(
      (entry: any) => entry.payload.kind === "hub_state"));
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await testInfo.attach(`no-tray-${theme}`, {
      body: await page.screenshot({ path: `${fixture.home}/no-tray-${theme}.png` }), contentType: "image/png",
    });
    expect(alive(), "reload must preserve the transient server").toBe(true);
    expect(errors, "console/page errors").toEqual([]);
    await page.close();
    await expect.poll(alive, { timeout: 20_000, message: `no-tray PID ${pid} must exit on browser close` }).toBe(false);
  } finally {
    await fixture.stop();
  }
});

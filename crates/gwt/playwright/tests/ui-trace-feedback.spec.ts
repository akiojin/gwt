import { readFile, realpath } from "node:fs/promises";
import { relative, isAbsolute } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

async function paletteCommand(page: Page, label: string) {
  await page.locator("#op-palette-button").click();
  await page.locator("#op-palette-input").fill(label);
  await page.getByRole("option", { name: label, exact: true }).click();
}

test("UI trace feedback follows live save, repeated errors, and recovery", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !checkHome, "requires browser-check isolated checkout and HOME");
  const errors: string[] = [];
  const alerts: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("dialog", async dialog => {
    expect(dialog.type()).toBe("alert");
    alerts.push(dialog.message());
    await dialog.accept();
  });
  await gotoLiveGwt(page, base!, { enableTestBridge: true });
  await expect(page.locator("#op-palette-button")).toBeVisible();

  await paletteCommand(page, "Stop UI Trace");
  await expect.poll(() => alerts.slice()).toEqual(["UI trace is not running."]);
  await paletteCommand(page, "Start UI Trace");
  await paletteCommand(page, "Stop UI Trace");
  await expect.poll(() => alerts.length).toBe(2);
  const saved = await page.evaluate(() => (window as any).__gwtPlaywrightMessages
    .findLast((entry: any) => entry.payload.kind === "ui_trace_saved")?.payload);
  expect(saved).toBeTruthy();
  expect(alerts[1]).toBe(`UI trace saved\n${saved.path}\nentries: ${saved.entries}`);
  const insideHome = relative(await realpath(checkHome!), await realpath(saved.path));
  expect(isAbsolute(insideHome) || insideHome.startsWith("..")).toBe(false);
  const entries = (await readFile(saved.path, "utf8")).trim().split("\n").map(line => JSON.parse(line));
  expect(entries).toHaveLength(saved.entries);
  expect(entries.some(entry => entry.kind === "trace_start")).toBe(true);

  // Exercise real backend validation; no write permissions or shared state change.
  for (let index = 0; index < 2; index += 1) {
    await sendLiveGwtEvent(page, { kind: "save_ui_trace", trace: { session_id: "missing-entries" } });
    await expect.poll(() => alerts.length).toBe(3 + index);
    expect(alerts[2 + index]).toMatch(/^UI trace error: /);
  }
  expect(alerts[2]).toBe(alerts[3]);
  await paletteCommand(page, "Start UI Trace");
  await paletteCommand(page, "Stop UI Trace");
  await expect.poll(() => alerts.length).toBe(5);
  expect(alerts[4]).toMatch(/^UI trace saved\n.+\nentries: \d+$/);

  await page.locator("#op-palette-button").click();
  await page.locator("#op-palette-input").fill("UI Trace");
  await expect(page.getByRole("option", { name: "Start UI Trace", exact: true })).toBeVisible();
  await expect(page.getByRole("option", { name: "Stop UI Trace", exact: true })).toBeVisible();
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await testInfo.attach(`ui-trace-feedback-${theme}`, {
    body: await page.screenshot({ fullPage: true }), contentType: "image/png",
  });
  expect(errors).toEqual([]);
});
